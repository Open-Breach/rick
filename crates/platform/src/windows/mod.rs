//! Windows: the Recycle Bin through `IFileOperation`, `ShellExecute`, the
//! shell Properties dialogue, `%APPDATA%\Rick`.

use std::ffi::c_void;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use types::Error;
use windows::Win32::Foundation::{HWND, PROPERTYKEY, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOF_ALLOWUNDO, FOF_NO_UI, FOFX_RECYCLEONDELETE, FOLDERID_RecycleBinFolder,
    FileOperation, IEnumShellItems, IFileOperation, IShellItem, IShellItem2, KF_FLAG_DEFAULT,
    SEE_MASK_INVOKEIDLIST, SHCreateItemFromParsingName, SHELLEXECUTEINFOW, SHGetKnownFolderItem,
    SIGDN_FILESYSPATH, SIGDN_NORMALDISPLAY, ShellExecuteExW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{SW_SHOW, SW_SHOWNORMAL};
use windows::core::{GUID, Interface, PCWSTR, PWSTR, w};

use crate::Native;
use crate::fs::windows::{filetime, io_error, wide};
use crate::traits::{ConfigDir, OpenWith, Properties, PropertiesOutcome, Trash, TrashedItem};

/// `System.Recycle.DeletedFrom`: the folder a recycled item came from.
const PKEY_RECYCLE_DELETED_FROM: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9B17_4B33_40FF_11D2_A27E_00C0_4FC3_0871),
    pid: 2,
};
/// `System.Recycle.DateDeleted`.
const PKEY_RECYCLE_DATE_DELETED: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9B17_4B33_40FF_11D2_A27E_00C0_4FC3_0871),
    pid: 3,
};
/// `System.ItemNameDisplay`: the full name, extension included, unlike
/// `SIGDN_NORMALDISPLAY`, which honours the "hide extensions" setting.
const PKEY_ITEM_NAME_DISPLAY: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xB725_F130_47EF_101A_A5F1_0260_8C9E_EBAC),
    pid: 10,
};

/// The absolute path with 8.3 short names expanded and no `\\?\` prefix, as
/// the shell reports paths.
fn long_path(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let Ok(canonical) = fs::canonicalize(&absolute) else {
        return absolute;
    };
    let text = canonical.to_string_lossy();
    if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
        PathBuf::from(format!("\\\\{unc}"))
    } else if let Some(plain) = text.strip_prefix("\\\\?\\") {
        PathBuf::from(plain)
    } else {
        canonical
    }
}

/// Initialises COM for the current thread and uninitialises it on drop.
struct Com {
    uninitialise: bool,
}

impl Com {
    fn new() -> Result<Self, Error> {
        // SAFETY: plain COM initialisation with no pointers involved.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if hr.is_ok() {
            return Ok(Self { uninitialise: true });
        }
        if hr == RPC_E_CHANGED_MODE {
            // Already initialised with another model by the host; use it.
            return Ok(Self {
                uninitialise: false,
            });
        }
        Err(Error::Io(io_error(windows::core::Error::from_hresult(hr))))
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.uninitialise {
            // SAFETY: balances the successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
        }
    }
}

fn take_string(pwstr: PWSTR) -> Option<String> {
    if pwstr.is_null() {
        return None;
    }
    // SAFETY: the shell returned a NUL-terminated string allocated with the
    // COM task allocator; we read it once and free it exactly once.
    let text = unsafe { pwstr.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(pwstr.0.cast::<c_void>().cast_const())) };
    text
}

fn recycle_bin_items() -> Result<Vec<TrashedItem>, Error> {
    let _com = Com::new()?;
    // SAFETY: COM is initialised on this thread; the known-folder id is a
    // static GUID and the enumeration interfaces are used as documented.
    unsafe {
        let bin: IShellItem =
            SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None)
                .map_err(|e| Error::Io(io_error(e)))?;
        let items: IEnumShellItems = bin
            .BindToHandler(None, &BHID_EnumItems)
            .map_err(|e| Error::Io(io_error(e)))?;
        let mut out = Vec::new();
        loop {
            let mut slot: [Option<IShellItem>; 1] = [None];
            let mut fetched = 0u32;
            let hr = items.Next(&mut slot, Some(&raw mut fetched));
            if hr.is_err() || fetched == 0 {
                break;
            }
            let Some(item) = slot[0].take() else { break };
            let Ok(item2) = item.cast::<IShellItem2>() else {
                continue;
            };
            let name = item2
                .GetString(&PKEY_ITEM_NAME_DISPLAY)
                .ok()
                .and_then(take_string)
                .or_else(|| {
                    item.GetDisplayName(SIGDN_NORMALDISPLAY)
                        .ok()
                        .and_then(take_string)
                });
            let deleted_from = item2
                .GetString(&PKEY_RECYCLE_DELETED_FROM)
                .ok()
                .and_then(take_string)
                .unwrap_or_default();
            let trashed = item
                .GetDisplayName(SIGDN_FILESYSPATH)
                .ok()
                .and_then(take_string)
                .unwrap_or_default();
            let deleted_at = item2
                .GetFileTime(&PKEY_RECYCLE_DATE_DELETED)
                .ok()
                .and_then(|ft| {
                    filetime((i64::from(ft.dwHighDateTime) << 32) | i64::from(ft.dwLowDateTime))
                })
                .unwrap_or(SystemTime::UNIX_EPOCH);
            if let Some(name) = name {
                // The shell keeps the display name of a recycled item, which
                // hides a known extension; the `$R` payload keeps the real one.
                let name = match (
                    Path::new(&name).extension(),
                    Path::new(&trashed).extension(),
                ) {
                    (None, Some(ext)) => format!("{name}.{}", ext.to_string_lossy()),
                    _ => name,
                };
                out.push(TrashedItem {
                    original: PathBuf::from(deleted_from).join(name),
                    trashed: PathBuf::from(trashed),
                    deleted_at,
                });
            }
        }
        Ok(out)
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let (a, b) = (a.to_string_lossy(), b.to_string_lossy());
    a.trim_end_matches('\\')
        .eq_ignore_ascii_case(b.trim_end_matches('\\'))
}

fn same_name(a: &Path, b: &Path) -> bool {
    match (a.file_name(), b.file_name()) {
        (Some(a), Some(b)) => a
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()),
        _ => false,
    }
}

impl Trash for Native {
    fn trash(&self, path: &Path) -> Result<TrashedItem, Error> {
        fs::symlink_metadata(path).map_err(Error::from_io)?;
        let original = long_path(path);
        let started = SystemTime::now();
        {
            let _com = Com::new()?;
            let wide = wide(&original);
            // SAFETY: COM is initialised; `wide` is NUL-terminated and
            // outlives the calls; the file operation is driven as documented.
            unsafe {
                let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)
                    .map_err(|e| Error::Io(io_error(e)))?;
                op.SetOperationFlags(FOF_ALLOWUNDO | FOF_NO_UI | FOFX_RECYCLEONDELETE)
                    .map_err(|e| Error::Io(io_error(e)))?;
                let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)
                    .map_err(|e| Error::Io(io_error(e)))?;
                op.DeleteItem(&item, None)
                    .map_err(|e| Error::Io(io_error(e)))?;
                op.PerformOperations().map_err(|e| Error::Io(io_error(e)))?;
                if op
                    .GetAnyOperationsAborted()
                    .map_err(|e| Error::Io(io_error(e)))?
                    .as_bool()
                {
                    return Err(Error::Unsupported("the shell did not recycle the item"));
                }
            }
        }
        if fs::symlink_metadata(&original).is_ok() {
            return Err(Error::Unsupported("the shell did not recycle the item"));
        }
        // Find what the shell made of it: by original path, or failing that
        // by name among the items recycled since we started.
        let recent = started - std::time::Duration::from_secs(120);
        let mut candidates: Vec<TrashedItem> = recycle_bin_items()?
            .into_iter()
            .filter(|item| {
                same_path(&item.original, &original)
                    || (same_name(&item.original, &original) && item.deleted_at >= recent)
            })
            .collect();
        candidates.sort_by_key(|item| item.deleted_at);
        let mut found = candidates
            .pop()
            .ok_or(Error::Unsupported("the volume has no Recycle Bin"))?;
        found.original = original;
        Ok(found)
    }

    fn restore(&self, item: &TrashedItem) -> Result<(), Error> {
        if fs::symlink_metadata(&item.trashed).is_err() {
            return Err(Error::NotFound);
        }
        if fs::symlink_metadata(&item.original).is_ok() {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "original path is taken",
            )));
        }
        if let Some(parent) = item.original.parent() {
            fs::create_dir_all(parent).map_err(Error::from_io)?;
        }
        fs::rename(&item.trashed, &item.original).map_err(Error::from_io)?;
        // The Recycle Bin keeps the metadata of `$R<id>` in `$I<id>` next to
        // it; without its payload the record is stale, so remove it.
        if let Some(name) = item.trashed.file_name().and_then(|n| n.to_str())
            && let Some(rest) = name.strip_prefix("$R")
        {
            let info = item.trashed.with_file_name(format!("$I{rest}"));
            let _ = fs::remove_file(info);
        }
        Ok(())
    }

    fn list(&self) -> Result<Vec<TrashedItem>, Error> {
        recycle_bin_items()
    }
}

fn shell_execute(file: PCWSTR, parameters: PCWSTR) -> Result<(), Error> {
    // SAFETY: both strings are NUL-terminated and outlive the call.
    let instance =
        unsafe { ShellExecuteW(None, w!("open"), file, parameters, None, SW_SHOWNORMAL) };
    if instance.0.addr() <= 32 {
        return Err(Error::from_io(io::Error::last_os_error()));
    }
    Ok(())
}

impl OpenWith for Native {
    fn open(&self, path: &Path) -> Result<(), Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        let wide = wide(&path);
        shell_execute(PCWSTR(wide.as_ptr()), PCWSTR::null())
    }

    fn reveal(&self, path: &Path) -> Result<(), Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        let select: Vec<u16> = format!("/select,\"{}\"", path.display())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        shell_execute(w!("explorer.exe"), PCWSTR(select.as_ptr()))
    }
}

impl Properties for Native {
    fn show(&self, path: &Path) -> Result<PropertiesOutcome, Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        fs::symlink_metadata(&path).map_err(Error::from_io)?;
        let wide = wide(&path);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: u32::try_from(std::mem::size_of::<SHELLEXECUTEINFOW>()).unwrap_or(0),
            fMask: SEE_MASK_INVOKEIDLIST,
            hwnd: HWND::default(),
            lpVerb: w!("properties"),
            lpFile: PCWSTR(wide.as_ptr()),
            nShow: SW_SHOW.0,
            ..Default::default()
        };
        // SAFETY: `info` is fully initialised and `wide` outlives the call.
        unsafe { ShellExecuteExW(&raw mut info) }.map_err(|e| Error::Io(io_error(e)))?;
        Ok(PropertiesOutcome::Native)
    }
}

impl ConfigDir for Native {
    fn config_dir(&self) -> PathBuf {
        std::env::var_os("APPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .map(|p| PathBuf::from(p).join("AppData").join("Roaming"))
            })
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Rick")
    }
}
