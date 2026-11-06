//! Windows: `GetFileInformationByHandleEx` with `FileIdBothDirectoryInfo`
//! for listing, `CopyFileExW` for copies, `GetCompressedFileSizeW` for the
//! size on disk.

use std::ffi::{OsStr, OsString, c_void};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;
use std::time::{Duration, SystemTime};

use types::Attrs;
use windows::Win32::Foundation::{
    ERROR_FILE_EXISTS, ERROR_NO_MORE_FILES, ERROR_REQUEST_ABORTED, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    COPY_FILE_FAIL_IF_EXISTS, COPYFILE_FLAGS, COPYPROGRESSROUTINE_PROGRESS, CopyFileExW,
    CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_ID_BOTH_DIR_INFO, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo,
    GetCompressedFileSizeW, GetFileInformationByHandleEx, GetVolumeInformationByHandleW,
    INVALID_FILE_SIZE, LPPROGRESS_ROUTINE_CALLBACK_REASON, OPEN_EXISTING, PROGRESS_CANCEL,
    PROGRESS_CONTINUE,
};
use windows::core::PCWSTR;

use super::{CopyMethod, RawEntry, RawKind};

const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
/// Difference between the Windows epoch (1601) and the Unix epoch in 100 ns.
const EPOCH_DIFFERENCE: i64 = 116_444_736_000_000_000;
const BUFFER_SIZE: usize = 256 * 1024;

pub(crate) fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Turns a `windows` error into the `io::Error` with the same Win32 code.
pub(crate) fn io_error(error: windows::core::Error) -> io::Error {
    let code = error.code().0;
    if code.cast_unsigned() & 0xFFFF_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error(code & 0xFFFF)
    } else {
        io::Error::other(error)
    }
}

pub(crate) fn filetime(value: i64) -> Option<SystemTime> {
    let unix = value.checked_sub(EPOCH_DIFFERENCE)?;
    let unix = u64::try_from(unix).ok()?;
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_nanos(unix.checked_mul(100)?))
}

pub(super) struct DirReader {
    handle: OwnedHandle,
    serial: u32,
    buffer: Vec<u8>,
    pending: std::collections::VecDeque<RawEntry>,
    started: bool,
    exhausted: bool,
}

impl DirReader {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let wide = wide(path);
        // SAFETY: `wide` is a NUL-terminated path that outlives the call; the
        // other arguments are plain flags.
        let raw = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        }
        .map_err(io_error)?;
        // SAFETY: `raw` is a valid handle we own and nobody else closes.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw.0) };
        let mut serial = 0u32;
        // SAFETY: `raw` stays open (owned above) and `serial` is a valid
        // out-pointer for the call.
        unsafe {
            GetVolumeInformationByHandleW(raw, None, Some(&raw mut serial), None, None, None)
        }
        .map_err(io_error)?;
        Ok(Self {
            handle,
            serial,
            buffer: vec![0; BUFFER_SIZE],
            pending: std::collections::VecDeque::new(),
            started: false,
            exhausted: false,
        })
    }

    pub(super) fn next_batch(&mut self, max: usize) -> io::Result<Vec<RawEntry>> {
        while self.pending.len() < max && !self.exhausted {
            self.fill()?;
        }
        let take = self.pending.len().min(max);
        Ok(self.pending.drain(..take).collect())
    }

    fn fill(&mut self) -> io::Result<()> {
        let class = if self.started {
            FileIdBothDirectoryInfo
        } else {
            FileIdBothDirectoryRestartInfo
        };
        self.started = true;
        let handle = HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(
            &self.handle,
        ));
        // SAFETY: `handle` is our open directory handle and `buffer` is
        // writable for the length we pass.
        let result = unsafe {
            GetFileInformationByHandleEx(
                handle,
                class,
                self.buffer.as_mut_ptr().cast::<c_void>(),
                u32::try_from(self.buffer.len()).unwrap_or(u32::MAX),
            )
        };
        if let Err(error) = result {
            if error.code() == ERROR_NO_MORE_FILES.to_hresult() {
                self.exhausted = true;
                return Ok(());
            }
            return Err(io_error(error));
        }
        let mut offset = 0usize;
        while let Some(record) = self.buffer.get(offset..) {
            if record.len() < std::mem::size_of::<FILE_ID_BOTH_DIR_INFO>() {
                break;
            }
            // SAFETY: the kernel wrote a `FILE_ID_BOTH_DIR_INFO` at this
            // offset and we checked the buffer holds at least the fixed part.
            let info: FILE_ID_BOTH_DIR_INFO =
                unsafe { std::ptr::read_unaligned(record.as_ptr().cast()) };
            let name_offset = std::mem::offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
            let name_len = (info.FileNameLength as usize) / 2;
            let name: Vec<u16> = record
                .get(name_offset..)
                .map(|bytes| {
                    bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .take(name_len)
                        .map(|c| u16::from_le_bytes(*c))
                        .collect()
                })
                .unwrap_or_default();
            let name = OsString::from_wide(&name);
            if name != "." && name != ".." && !name.is_empty() {
                self.pending.push_back(entry_from(&info, name, self.serial));
            }
            if info.NextEntryOffset == 0 {
                break;
            }
            offset += info.NextEntryOffset as usize;
        }
        Ok(())
    }
}

fn entry_from(info: &FILE_ID_BOTH_DIR_INFO, name: OsString, serial: u32) -> RawEntry {
    let attributes = info.FileAttributes;
    let kind = if attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        match info.EaSize {
            IO_REPARSE_TAG_SYMLINK | IO_REPARSE_TAG_MOUNT_POINT => RawKind::Symlink,
            _ => RawKind::Other,
        }
    } else if attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
        RawKind::Dir
    } else {
        RawKind::File
    };
    let mut attrs = Attrs::empty();
    if attributes & FILE_ATTRIBUTE_HIDDEN.0 != 0 {
        attrs |= Attrs::HIDDEN;
    }
    if attributes & FILE_ATTRIBUTE_SYSTEM.0 != 0 {
        attrs |= Attrs::SYSTEM;
    }
    if attributes & FILE_ATTRIBUTE_READONLY.0 != 0 {
        attrs |= Attrs::READONLY;
    }
    if kind == RawKind::File && is_executable(&name) {
        attrs |= Attrs::EXECUTABLE;
    }
    RawEntry {
        id: (u128::from(serial) << 64) | u128::from(info.FileId.cast_unsigned()),
        kind,
        size: (kind == RawKind::File).then(|| u64::try_from(info.EndOfFile).unwrap_or(0)),
        created: filetime(info.CreationTime),
        modified: filetime(info.LastWriteTime),
        accessed: filetime(info.LastAccessTime),
        attrs,
        name,
    }
}

fn is_executable(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| {
            ext.eq_ignore_ascii_case("exe")
                || ext.eq_ignore_ascii_case("com")
                || ext.eq_ignore_ascii_case("bat")
                || ext.eq_ignore_ascii_case("cmd")
        })
}

struct ProgressState<'a> {
    callback: &'a mut dyn FnMut(u64, u64) -> bool,
}

unsafe extern "system" fn progress_routine(
    total: i64,
    transferred: i64,
    _stream_size: i64,
    _stream_transferred: i64,
    _stream: u32,
    _reason: LPPROGRESS_ROUTINE_CALLBACK_REASON,
    _source: HANDLE,
    _destination: HANDLE,
    data: *const c_void,
) -> COPYPROGRESSROUTINE_PROGRESS {
    // SAFETY: `data` is the `ProgressState` pointer `copy_file` passed to
    // `CopyFileExW`, which only calls this routine while that state is alive
    // and not otherwise borrowed.
    let state = unsafe { &mut *(data.cast_mut().cast::<ProgressState<'_>>()) };
    let keep_going = (state.callback)(
        u64::try_from(transferred).unwrap_or(0),
        u64::try_from(total).unwrap_or(0),
    );
    if keep_going {
        PROGRESS_CONTINUE
    } else {
        PROGRESS_CANCEL
    }
}

pub(super) fn copy_file(
    from: &Path,
    to: &Path,
    overwrite: bool,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> io::Result<CopyMethod> {
    let meta = std::fs::metadata(from)?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let total = meta.len();
    let (from_wide, to_wide) = (wide(from), wide(to));
    let mut state = ProgressState { callback: progress };
    let flags = if overwrite {
        COPYFILE_FLAGS(0)
    } else {
        COPY_FILE_FAIL_IF_EXISTS
    };
    // SAFETY: both paths are NUL-terminated and outlive the call; `state`
    // outlives the call and is only touched through the progress routine.
    let result = unsafe {
        CopyFileExW(
            PCWSTR(from_wide.as_ptr()),
            PCWSTR(to_wide.as_ptr()),
            Some(progress_routine),
            Some(std::ptr::from_mut(&mut state).cast::<c_void>()),
            None,
            flags,
        )
    };
    match result {
        Ok(()) => {
            if !(state.callback)(total, total) {
                let _ = std::fs::remove_file(to);
                return Err(io::Error::new(io::ErrorKind::Interrupted, "copy cancelled"));
            }
            Ok(CopyMethod::Kernel)
        }
        Err(error) if error.code() == ERROR_REQUEST_ABORTED.to_hresult() => {
            let _ = std::fs::remove_file(to);
            Err(io::Error::new(io::ErrorKind::Interrupted, "copy cancelled"))
        }
        Err(error) if error.code() == ERROR_FILE_EXISTS.to_hresult() => {
            Err(io::Error::from(io::ErrorKind::AlreadyExists))
        }
        Err(error) => Err(io_error(error)),
    }
}

pub(super) fn allocated_size(path: &Path) -> io::Result<u64> {
    let wide = wide(path);
    let mut high = 0u32;
    // SAFETY: `wide` is NUL-terminated and `high` is a valid out-pointer.
    let low = unsafe { GetCompressedFileSizeW(PCWSTR(wide.as_ptr()), Some(&raw mut high)) };
    if low == INVALID_FILE_SIZE {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(0) {
            return Err(error);
        }
    }
    Ok((u64::from(high) << 32) | u64::from(low))
}

// Same signature as the Linux version, which can fail.
#[allow(clippy::unnecessary_wraps)]
pub(super) fn shared_extents(_path: &Path) -> io::Result<Option<bool>> {
    Ok(None)
}
