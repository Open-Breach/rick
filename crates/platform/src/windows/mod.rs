//! Windows: the Recycle Bin through `IFileOperation`, `ShellExecute`, the
//! shell Properties dialogue, `%APPDATA%\Rick`, the drives from
//! `GetLogicalDrives` with `GetVolumeInformationW` and `GetDiskFreeSpaceExW`
//! and ejected through `DeviceIoControl`, the known folders through
//! `SHGetKnownFolderPath`, the Recent folder's shortcuts resolved with
//! `IShellLinkW`, and drag and drop through OLE's `DoDragDrop` and the
//! cursor position.

mod drag;

use std::ffi::c_void;
use std::fs;
use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::SystemTime;

use types::Error;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Foundation::{HWND, PROPERTYKEY, RPC_E_CHANGED_MODE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDiskFreeSpaceExW,
    GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, OPEN_EXISTING,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IDataObject, IPersistFile,
    STGM_READ,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, IOCTL_STORAGE_EJECT_MEDIA,
    IOCTL_STORAGE_MEDIA_REMOVAL, PREVENT_MEDIA_REMOVAL,
};
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows::Win32::UI::Shell::{
    ASSOC_FILTER_RECOMMENDED, BHID_DataObject, BHID_EnumItems, FOF_ALLOWUNDO, FOF_NO_UI,
    FOFX_RECYCLEONDELETE, FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music,
    FOLDERID_Pictures, FOLDERID_Recent, FOLDERID_RecycleBinFolder, FOLDERID_Videos, FileOperation,
    IAssocHandler, IEnumAssocHandlers, IEnumShellItems, IFileOperation, IShellItem, IShellItem2,
    IShellLinkW, KF_FLAG_DEFAULT, SEE_MASK_INVOKEIDLIST, SHAssocEnumHandlers,
    SHCreateItemFromParsingName, SHELLEXECUTEINFOW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI,
    SHERB_NOSOUND, SHEmptyRecycleBinW, SHGetKnownFolderItem, SHGetKnownFolderPath,
    SIGDN_FILESYSPATH, SIGDN_NORMALDISPLAY, ShellExecuteExW, ShellExecuteW, ShellLink,
};
use windows::Win32::UI::WindowsAndMessaging::{SW_SHOW, SW_SHOWNORMAL};
use windows::core::{GUID, Interface, PCWSTR, PWSTR, w};

use crate::Native;
use crate::fs::windows::{filetime, io_error, wide};
use crate::traits::{
    Application, Chrome, ConfigDir, ControlsSide, DragSource, DropTarget, Elevate, ElevatedOutput,
    ImageFormat, IncomingFiles, Mount, OpenWith, Point, Properties, PropertiesOutcome, RecentItem,
    RecentlyUsed, StandardFolder, StandardFolders, Terminal, Trash, TrashedItem, Volume,
    VolumeKind, Volumes, WindowChrome, WindowRef,
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
        let recent = started - std::time::Duration::from_mins(2);
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

/// `GetDriveTypeW` answers, from `winbase.h` (the `windows` crate keeps
/// them in `Win32::System::WindowsProgramming`, a feature not otherwise
/// needed).
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;

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
    /// Every drive letter with the volume label, the drive type, the file
    /// system and the free space; the system drive first.
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
            let root = format!("{letter}:\\");
            let root_wide = wide(Path::new(&root));
            // SAFETY: the root string is NUL-terminated and outlives the
            // calls; the out-pointers point at locals of the right type.
            let (kind, label, fs_type, space) = unsafe {
                let drive_type = GetDriveTypeW(PCWSTR(root_wide.as_ptr()));
                let kind = match drive_type {
                    DRIVE_REMOVABLE => VolumeKind::Removable,
                    DRIVE_REMOTE => VolumeKind::Network,
                    DRIVE_CDROM => VolumeKind::Optical,
                    DRIVE_FIXED => VolumeKind::Fixed,
                    _ => continue,
                };
                let mut label = [0u16; 261];
                let mut fs_name = [0u16; 261];
                let labelled = GetVolumeInformationW(
                    PCWSTR(root_wide.as_ptr()),
                    Some(&mut label),
                    None,
                    None,
                    None,
                    Some(&mut fs_name),
                )
                .is_ok();
                let text = |buffer: &[u16]| {
                    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
                    String::from_utf16_lossy(&buffer[..end])
                };
                let label = labelled.then(|| text(&label)).filter(|l| !l.is_empty());
                let fs_type = if labelled {
                    text(&fs_name)
                } else {
                    String::new()
                };
                let (mut free, mut total) = (0u64, 0u64);
                let space = GetDiskFreeSpaceExW(
                    PCWSTR(root_wide.as_ptr()),
                    Some(&raw mut free),
                    Some(&raw mut total),
                    None,
                )
                .is_ok()
                .then_some((total, free));
                (kind, label, fs_type, space)
            };
            let name = match label {
                Some(label) => label,
                None if Some(letter) == system => "System".to_owned(),
                None => format!("{letter}:"),
            };
            let (total, free) = space.map_or((None, None), |(t, f)| (Some(t), Some(f)));
            out.push(Volume {
                name: format!("{name} ({letter}:)"),
                mount_point: PathBuf::from(root),
                kind,
                fs_type,
                device: None,
                total,
                free,
            });
        }
        // The system drive first, the rest by letter.
        out.sort_by_key(|v| {
            let letter = v.mount_point.to_string_lossy().chars().next();
            (letter != system, letter)
        });
        Ok(out)
    }

    /// Locks and dismounts the volume, then asks the device to eject its
    /// medium: what the shell's Eject does for a USB stick or a disc.
    fn eject(&self, volume: &Volume) -> Result<(), Error> {
        if !volume.is_removable() {
            return Err(Error::Unsupported("only removable media can be ejected"));
        }
        let letter = volume
            .mount_point
            .to_string_lossy()
            .chars()
            .next()
            .ok_or(Error::Unsupported("the volume has no drive letter"))?;
        let device = wide(Path::new(&format!("\\\\.\\{letter}:")));
        // SAFETY: the device name is NUL-terminated and outlives the open;
        // the handle it returns is used only until `CloseHandle` below.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(device.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }
        .map_err(|e| Error::Io(io_error(e)))?;
        let allow = PREVENT_MEDIA_REMOVAL {
            PreventMediaRemoval: false,
        };
        let size = u32::try_from(std::mem::size_of::<PREVENT_MEDIA_REMOVAL>()).unwrap_or(1);
        // SAFETY: the handle is open; each control code is passed the
        // buffer its documentation names, which outlives the call, or none.
        let result = unsafe {
            control(handle, FSCTL_LOCK_VOLUME, None)
                .and_then(|()| control(handle, FSCTL_DISMOUNT_VOLUME, None))
                .and_then(|()| {
                    control(
                        handle,
                        IOCTL_STORAGE_MEDIA_REMOVAL,
                        Some((std::ptr::from_ref(&allow).cast::<c_void>(), size)),
                    )
                })
                .and_then(|()| control(handle, IOCTL_STORAGE_EJECT_MEDIA, None))
        };
        // SAFETY: balances the successful `CreateFileW` above.
        let _ = unsafe { CloseHandle(handle) };
        result
    }
}

/// One `DeviceIoControl` with an optional input buffer and no output.
///
/// # Safety
///
/// `handle` must be open and `input`, when given, must point at `size`
/// readable bytes for the duration of the call.
unsafe fn control(
    handle: HANDLE,
    code: u32,
    input: Option<(*const c_void, u32)>,
) -> Result<(), Error> {
    let mut returned = 0u32;
    let (buffer, size) = input.unwrap_or((std::ptr::null(), 0));
    // SAFETY: the caller's contract.
    unsafe {
        DeviceIoControl(
            handle,
            code,
            (!buffer.is_null()).then_some(buffer),
            size,
            None,
            0,
            Some(&raw mut returned),
            None,
        )
    }
    .map_err(|e| Error::Io(io_error(e)))
}

/// The path of a known folder, when the shell knows it.
fn known_folder(id: &GUID) -> Option<PathBuf> {
    // SAFETY: the id is a static GUID and the returned string is freed
    // exactly once by `take_string`.
    let path = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.ok()?;
    take_string(path).map(PathBuf::from)
}

impl Mount for Native {
    /// ISO, VHD and VHDX, through `Mount-DiskImage`.
    fn mountable(&self) -> &'static [ImageFormat] {
        crate::NATIVE_MOUNTABLE
    }

    /// `Mount-DiskImage` through PowerShell: an ISO becomes a virtual DVD
    /// drive, a VHD or VHDX a disk whose partitions get drive letters; the
    /// first drive letter is the volume returned.
    fn mount_image(&self, path: &Path) -> Result<Volume, Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let format = ImageFormat::for_name(&name)
            .filter(|f| crate::NATIVE_MOUNTABLE.contains(f))
            .ok_or(Error::Unsupported(
                "Windows mounts ISO, VHD and VHDX images only",
            ))?;
        // A single-quoted PowerShell string: the quote doubled, nothing
        // else interpreted. No double quotes anywhere in the script, so the
        // command line reaches PowerShell as written.
        let quoted = path.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$ErrorActionPreference = 'Stop'; \
             $image = Mount-DiskImage -ImagePath '{quoted}' -PassThru; \
             $volumes = if ($image.StorageType -eq 1) {{ $image | Get-Volume }} \
             else {{ $image | Get-Disk | Get-Partition | Get-Volume }}; \
             $volumes | Where-Object {{ $_.DriveLetter }} | ForEach-Object {{ $_.DriveLetter.ToString() }}"
        );
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &script,
            ])
            .stdin(Stdio::null())
            .output()
            .map_err(Error::from_io)?;
        if !output.status.success() {
            let said = String::from_utf8_lossy(&output.stderr);
            let said = said.trim();
            return Err(Error::Io(io::Error::other(if said.is_empty() {
                format!("Mount-DiskImage exited with {}", output.status)
            } else {
                said.to_owned()
            })));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let letter = stdout
            .lines()
            .map(str::trim)
            .filter_map(|line| {
                let mut chars = line.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase()),
                    _ => None,
                }
            })
            .next()
            .ok_or_else(|| {
                Error::Io(io::Error::other(
                    "Mount-DiskImage gave the image no drive letter",
                ))
            })?;
        let mount_point = PathBuf::from(format!("{letter}:\\"));
        if let Ok(volumes) = self.volumes()
            && let Some(volume) = volumes.into_iter().find(|v| v.mount_point == mount_point)
        {
            return Ok(volume);
        }
        Ok(Volume {
            name: format!("{letter}:"),
            mount_point,
            kind: if format == ImageFormat::Iso {
                VolumeKind::Optical
            } else {
                VolumeKind::Removable
            },
            fs_type: String::new(),
            device: None,
            total: None,
            free: None,
        })
    }
}

impl StandardFolders for Native {
    fn standard_folder(&self, folder: StandardFolder) -> Option<PathBuf> {
        let path = match folder {
            StandardFolder::Home => self.home_dir(),
            StandardFolder::Documents => known_folder(&FOLDERID_Documents)?,
            StandardFolder::Pictures => known_folder(&FOLDERID_Pictures)?,
            StandardFolder::Music => known_folder(&FOLDERID_Music)?,
            StandardFolder::Video => known_folder(&FOLDERID_Videos)?,
            StandardFolder::Downloads => known_folder(&FOLDERID_Downloads)?,
            StandardFolder::Desktop => known_folder(&FOLDERID_Desktop)?,
        };
        path.is_dir().then_some(path)
    }
}

/// The target of a `.lnk` shortcut, through the shell link object.
fn shortcut_target(link: &IShellLinkW, shortcut: &Path) -> Option<PathBuf> {
    let file = wide(shortcut);
    // SAFETY: COM is initialised by the caller; the file name is
    // NUL-terminated and outlives the load; the path buffer is valid for
    // its whole length and `GetPath` writes at most that many code units.
    unsafe {
        let persist: IPersistFile = link.cast().ok()?;
        persist.Load(PCWSTR(file.as_ptr()), STGM_READ).ok()?;
        let mut buffer = [0u16; 1024];
        link.GetPath(&mut buffer, std::ptr::null_mut(), 0).ok()?;
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        if end == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&buffer[..end])))
    }
}

impl RecentlyUsed for Native {
    /// The shortcuts of the Recent folder, each resolved to its target;
    /// the shortcut's modification time is when the item was last used.
    fn recently_used(&self) -> Result<Vec<RecentItem>, Error> {
        let Some(recent) = known_folder(&FOLDERID_Recent) else {
            return Ok(Vec::new());
        };
        let entries = match fs::read_dir(&recent) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::from_io(e)),
        };
        let _com = Com::new()?;
        // SAFETY: COM is initialised on this thread and the class id is a
        // static GUID.
        let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| Error::Io(io_error(e)))?;
        let mut items: Vec<RecentItem> = Vec::new();
        for entry in entries.flatten() {
            let shortcut = entry.path();
            if !shortcut
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
            {
                continue;
            }
            let Some(target) = shortcut_target(&link, &shortcut) else {
                continue;
            };
            if fs::symlink_metadata(&target).is_err() {
                continue;
            }
            let used_at = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            items.retain(|item| !same_path(&item.path, &target));
            items.push(RecentItem {
                path: target,
                used_at,
            });
        }
        items.sort_by_key(|item| std::cmp::Reverse(item.used_at));
        Ok(items)
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

/// The shell's handlers for a file name's extension (`.txt`), the
/// recommended ones, each with its display name and executable; empty for
/// a name without an extension. COM must be initialised.
fn assoc_handlers(path: &Path) -> Result<Vec<(String, String, IAssocHandler)>, Error> {
    let Some(ext) = path.extension() else {
        return Ok(Vec::new());
    };
    let ext: Vec<u16> = std::iter::once(u16::from(b'.'))
        .chain(ext.encode_wide())
        .chain(Some(0))
        .collect();
    // SAFETY: the extension is NUL-terminated and outlives the call.
    let enumerator: IEnumAssocHandlers =
        unsafe { SHAssocEnumHandlers(PCWSTR(ext.as_ptr()), ASSOC_FILTER_RECOMMENDED) }
            .map_err(|e| Error::Io(io_error(e)))?;
    let mut out = Vec::new();
    loop {
        let mut handlers: [Option<IAssocHandler>; 8] = Default::default();
        let mut fetched = 0u32;
        // SAFETY: the slice and the count are valid for the call, which
        // fills at most the slice's length and reports how many.
        let result = unsafe { enumerator.Next(&mut handlers, Some(&raw mut fetched)) };
        if result.is_err() || fetched == 0 {
            break;
        }
        for handler in handlers.iter_mut().take(fetched as usize) {
            let Some(handler) = handler.take() else {
                continue;
            };
            // SAFETY: both calls return task-allocated strings that
            // `take_string` reads once and frees once.
            let (name, exe) = unsafe {
                (
                    handler.GetUIName().ok().and_then(take_string),
                    handler.GetName().ok().and_then(take_string),
                )
            };
            let (Some(name), Some(exe)) = (name, exe) else {
                continue;
            };
            if out.iter().any(|(_, e, _): &(String, String, _)| *e == exe) {
                continue;
            }
            out.push((name, exe, handler));
        }
    }
    Ok(out)
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

    /// The shell's recommended handlers for the extension; File Explorer
    /// for a folder.
    fn applications(&self, path: &Path) -> Result<Vec<Application>, Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        let meta = fs::symlink_metadata(&path).map_err(Error::from_io)?;
        if meta.is_dir() {
            return Ok(vec![Application {
                name: "File Explorer".to_owned(),
                id: "explorer.exe".to_owned(),
            }]);
        }
        let _com = Com::new()?;
        Ok(assoc_handlers(&path)?
            .into_iter()
            .map(|(name, id, _)| Application { name, id })
            .collect())
    }

    /// A handler of the extension is invoked through the shell, so store
    /// applications work too; anything else runs with the path as its
    /// argument.
    fn open_with(&self, path: &Path, app: &Application) -> Result<(), Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        {
            let _com = Com::new()?;
            if let Some((_, _, handler)) = assoc_handlers(&path)?
                .into_iter()
                .find(|(_, exe, _)| *exe == app.id)
            {
                let wide = wide(&path);
                // SAFETY: the path is NUL-terminated and outlives the call;
                // the item and the data object are owned COM pointers.
                let invoked = unsafe {
                    let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)
                        .map_err(|e| Error::Io(io_error(e)))?;
                    let data: IDataObject = item
                        .BindToHandler(None, &BHID_DataObject)
                        .map_err(|e| Error::Io(io_error(e)))?;
                    handler.Invoke(&data)
                };
                return invoked.map_err(|e| Error::Io(io_error(e)));
            }
        }
        let mut words = app.id.split_whitespace();
        let program = words
            .next()
            .ok_or(Error::Unsupported("no application named"))?;
        let args: Vec<&str> = words.collect();
        std::process::Command::new(program)
            .args(args)
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(drop)
            .map_err(Error::from_io)
    }
}

/// Whether `program` is on the `PATH`.
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

impl Elevate for Native {
    /// Through User Account Control: `ShellExecuteEx` with the `runas` verb
    /// shows the consent prompt and starts the program elevated; the call
    /// waits for it to end. An elevated process shares no pipes with this
    /// one, so the output comes back through a file the helper is told to
    /// write (`--output <file>` appended to the arguments) when the caller
    /// asked for one, else empty.
    fn run_elevated(
        &self,
        program: &Path,
        args: &[std::ffi::OsString],
    ) -> Result<ElevatedOutput, Error> {
        use windows::Win32::System::Threading::{
            GetExitCodeProcess, INFINITE, WaitForSingleObject,
        };
        use windows::Win32::UI::Shell::{SEE_MASK_NO_CONSOLE, SEE_MASK_NOCLOSEPROCESS};
        use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

        let program = std::path::absolute(program).map_err(Error::from_io)?;
        let output_file = tempfile::Builder::new()
            .prefix("rick-elevated-")
            .suffix(".out")
            .tempfile()
            .map_err(Error::from_io)?;
        let mut words: Vec<String> = args
            .iter()
            .map(|a| quote_argument(&a.to_string_lossy()))
            .collect();
        words.push("--output".to_owned());
        words.push(quote_argument(&output_file.path().to_string_lossy()));
        let parameters = wide_text(&words.join(" "));
        let file = wide(&program);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: u32::try_from(std::mem::size_of::<SHELLEXECUTEINFOW>()).unwrap_or(0),
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NO_CONSOLE,
            hwnd: HWND::default(),
            lpVerb: w!("runas"),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(parameters.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        // SAFETY: `info` is fully initialised and the strings outlive the call.
        if let Err(error) = unsafe { ShellExecuteExW(&raw mut info) } {
            // ERROR_CANCELLED: the user said no to the prompt.
            if error.code().0.cast_unsigned() & 0xFFFF == 1223 {
                return Err(Error::PermissionDenied);
            }
            return Err(Error::Io(io_error(error)));
        }
        let process = info.hProcess;
        if process.is_invalid() {
            return Err(Error::Io(io::Error::other("no process started")));
        }
        // SAFETY: `process` is the handle `ShellExecuteExW` returned, closed
        // below.
        let code = unsafe {
            let _ = WaitForSingleObject(process, INFINITE);
            let mut code: u32 = 0;
            let got = GetExitCodeProcess(process, &raw mut code);
            let _ = CloseHandle(process);
            got.map(|()| code)
        }
        .map_err(|e| Error::Io(io_error(e)))?;
        let stdout = fs::read(output_file.path()).unwrap_or_default();
        Ok(ElevatedOutput {
            code: i32::try_from(code).ok(),
            stdout,
            stderr: Vec::new(),
        })
    }
}

/// `text` as one argument of a Windows command line, quoted the way
/// `CommandLineToArgvW` undoes it.
fn quote_argument(text: &str) -> String {
    if !text.is_empty() && !text.contains([' ', '\t', '"']) {
        return text.to_owned();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in text.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            other => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(other);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

fn wide_text(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

impl Terminal for Native {
    /// Windows Terminal when it is installed, `cmd` in a new window
    /// otherwise.
    fn open_terminal(&self, dir: &Path, command: Option<&[String]>) -> Result<(), Error> {
        let dir = std::path::absolute(dir).map_err(Error::from_io)?;
        let command = command.filter(|c| !c.is_empty());
        let mut process = if on_path("wt.exe") {
            let mut process = std::process::Command::new("wt.exe");
            process.arg("-d").arg(&dir);
            if let Some(command) = command {
                process.arg("cmd.exe").arg("/k").args(command);
            }
            process
        } else {
            let mut process = std::process::Command::new("cmd.exe");
            process
                .args(["/c", "start", "", "/D"])
                .arg(&dir)
                .arg("cmd.exe");
            if let Some(command) = command {
                process.arg("/k").args(command);
            }
            process
        };
        process
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(drop)
            .map_err(Error::from_io)
    }
}

/// A `.lnk` shortcut at `at` pointing to `target`, through the shell link
/// object.
pub(crate) fn create_shortcut(target: &Path, at: &Path) -> io::Result<()> {
    let target = std::path::absolute(target)?;
    let _com = Com::new().map_err(|e| io::Error::other(e.to_string()))?;
    // SAFETY: COM is initialised on this thread, the class id is a static
    // GUID, and every string is NUL-terminated and outlives its call.
    unsafe {
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(io_error)?;
        let target_w = wide(&target);
        link.SetPath(PCWSTR(target_w.as_ptr())).map_err(io_error)?;
        if let Some(dir) = target.parent() {
            let dir_w = wide(dir);
            link.SetWorkingDirectory(PCWSTR(dir_w.as_ptr()))
                .map_err(io_error)?;
        }
        let persist: IPersistFile = link.cast().map_err(io_error)?;
        let at_w = wide(at);
        persist
            .Save(PCWSTR(at_w.as_ptr()), true)
            .map_err(io_error)?;
    }
    Ok(())
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
    /// Nothing to prepare: OLE is initialised for the window's thread by
    /// winit.
    fn attach(&self, _window: &WindowRef, _wake: std::sync::Arc<dyn Fn() + Send + Sync>) {}

    /// `DoDragDrop` with the files as a `CF_HDROP`, offering copy, move and
    /// link; returns when the drag has ended, since OLE runs its loop here.
    fn start_drag(&self, window: &WindowRef, paths: &[PathBuf]) -> Result<(), Error> {
        drag::start_drag(window, paths)
    }

    fn drag_in_progress(&self) -> bool {
        drag::drag_in_progress()
    }
}

impl DropTarget for Native {
    /// winit's drop target receives the drops and hands them to egui.
    fn incoming_files(&self) -> IncomingFiles {
        IncomingFiles::default()
    }

    /// The cursor over the window's client area, at the window's DPI.
    fn drop_position(&self, window: &WindowRef) -> Option<Point> {
        drag::drop_position(window)
    }
}
