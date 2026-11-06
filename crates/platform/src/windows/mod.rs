//! Windows: the Recycle Bin through `IFileOperation`, `ShellExecute`, the
//! shell Properties dialogue, `%APPDATA%\Rick`, and the drives from
//! `GetLogicalDrives` with `GetVolumeInformationW` and `GetDiskFreeSpaceExW`.

use std::ffi::c_void;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use types::Error;
use windows::Win32::Foundation::{HWND, PROPERTYKEY, RPC_E_CHANGED_MODE};
use windows::Win32::Storage::FileSystem::{
    DRIVE_CDROM, DRIVE_FIXED, DRIVE_REMOTE, DRIVE_REMOVABLE, GetDiskFreeSpaceExW, GetDriveTypeW,
    GetLogicalDrives, GetVolumeInformationW,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOF_ALLOWUNDO, FOF_NO_UI, FOFX_RECYCLEONDELETE, FOLDERID_RecycleBinFolder,
    FileOperation, IEnumShellItems, IFileOperation, IShellItem, IShellItem2, KF_FLAG_DEFAULT,
    SEE_MASK_INVOKEIDLIST, SHCreateItemFromParsingName, SHELLEXECUTEINFOW, SHERB_NOCONFIRMATION,
    SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW, SHGetKnownFolderItem, SIGDN_FILESYSPATH,
    SIGDN_NORMALDISPLAY, ShellExecuteExW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{SW_SHOW, SW_SHOWNORMAL};
use windows::core::{GUID, Interface, PCWSTR, PWSTR, w};

use crate::Native;
use crate::fs::windows::{filetime, io_error, wide};
use crate::traits::{
    Chrome, ConfigDir, ControlsSide, DragSource, OpenWith, Properties, PropertiesOutcome, Trash,
    TrashedItem, Volume, VolumeKind, Volumes, WindowChrome,
};

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

    /// Deletes the `$R<id>` payload and its `$I<id>` record.
    fn remove(&self, item: &TrashedItem) -> Result<(), Error> {
        let meta = fs::symlink_metadata(&item.trashed).map_err(|_| Error::NotFound)?;
        if meta.is_dir() {
            fs::remove_dir_all(&item.trashed)
        } else {
            fs::remove_file(&item.trashed)
        }
        .map_err(Error::from_io)?;
        if let Some(name) = item.trashed.file_name().and_then(|n| n.to_str())
            && let Some(rest) = name.strip_prefix("$R")
        {
            let info = item.trashed.with_file_name(format!("$I{rest}"));
            let _ = fs::remove_file(info);
        }
        Ok(())
    }

    fn empty(&self) -> Result<(), Error> {
        // SAFETY: no window, every drive's bin, no UI; plain flags.
        unsafe {
            SHEmptyRecycleBinW(
                None,
                PCWSTR::null(),
                SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND,
            )
        }
        .map_err(|e| Error::Io(io_error(e)))
    }
}

/// The drive letter of the Windows directory, upper case.
fn system_drive() -> Option<char> {
    let mut buffer = [0u16; 260];
    // SAFETY: the buffer is valid for its whole length and the call writes
    // at most that many code units.
    let written = usize::try_from(unsafe { GetWindowsDirectoryW(Some(&mut buffer)) }).unwrap_or(0);
    let text = String::from_utf16_lossy(&buffer[..written.min(buffer.len())]);
    text.chars().next().map(|c| c.to_ascii_uppercase())
}

impl Volumes for Native {
    /// Every drive letter but the system drive's, with the volume label, the
    /// drive type and the free space.
    fn volumes(&self) -> Result<Vec<Volume>, Error> {
        // SAFETY: a plain query with no pointers.
        let mask = unsafe { GetLogicalDrives() };
        if mask == 0 {
            return Err(Error::Io(io::Error::last_os_error()));
        }
        let system = system_drive();
        let mut out = Vec::new();
        for bit in 0..26u32 {
            if mask & (1 << bit) == 0 {
                continue;
            }
            let letter = char::from(b'A' + u8::try_from(bit).unwrap_or(0));
            if Some(letter) == system {
                continue;
            }
            let root = format!("{letter}:\\");
            let root_wide = wide(Path::new(&root));
            // SAFETY: the root string is NUL-terminated and outlives the
            // calls; the out-pointers point at locals of the right type.
            let (kind, label, space) = unsafe {
                let drive_type = GetDriveTypeW(PCWSTR(root_wide.as_ptr()));
                let kind = match drive_type {
                    DRIVE_REMOVABLE => VolumeKind::Removable,
                    DRIVE_REMOTE => VolumeKind::Network,
                    DRIVE_CDROM => VolumeKind::Optical,
                    DRIVE_FIXED => VolumeKind::Fixed,
                    _ => continue,
                };
                let mut label = [0u16; 261];
                let labelled = GetVolumeInformationW(
                    PCWSTR(root_wide.as_ptr()),
                    Some(&mut label),
                    None,
                    None,
                    None,
                    None,
                )
                .is_ok();
                let label = labelled
                    .then(|| {
                        let end = label.iter().position(|&c| c == 0).unwrap_or(label.len());
                        String::from_utf16_lossy(&label[..end])
                    })
                    .filter(|l| !l.is_empty());
                let (mut free, mut total) = (0u64, 0u64);
                let space = GetDiskFreeSpaceExW(
                    PCWSTR(root_wide.as_ptr()),
                    Some(&raw mut free),
                    Some(&raw mut total),
                    None,
                )
                .is_ok()
                .then_some((total, free));
                (kind, label, space)
            };
            let name = label.unwrap_or_else(|| format!("{letter}:"));
            let (total, free) = space.map_or((None, None), |(t, f)| (Some(t), Some(f)));
            out.push(Volume {
                name: format!("{name} ({letter}:)"),
                mount_point: PathBuf::from(root),
                kind,
                total,
                free,
            });
        }
        Ok(out)
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

    fn home_dir(&self) -> PathBuf {
        std::env::var_os("USERPROFILE")
            .filter(|v| !v.is_empty())
            .map_or_else(|| PathBuf::from("."), PathBuf::from)
    }
}

impl Chrome for Native {
    fn window_chrome(&self) -> WindowChrome {
        WindowChrome {
            controls: ControlsSide::Right,
            native_controls: false,
            native_controls_width: 0.0,
        }
    }
}

impl DragSource for Native {
    /// Not available yet: OLE's `DoDragDrop` must run on the thread that owns
    /// the window and pumps its messages while the drag lasts, with an
    /// `IDropSource` of Rick's own; `eframe` 0.36 owns that loop and gives
    /// Rick no hook into it, exposing drop reception only (QUESTIONS.md).
    fn start_drag(&self, paths: &[PathBuf]) -> Result<(), Error> {
        let uris: Vec<String> = paths.iter().map(|p| crate::paths::file_uri(p)).collect();
        tracing::debug!(?uris, "platform drag not started");
        Err(Error::Unsupported(
            "starting a drag needs OLE's DoDragDrop on the window's message loop, which eframe owns",
        ))
    }
}
