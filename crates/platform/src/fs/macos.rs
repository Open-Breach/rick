//! macOS: `getattrlistbulk` for listing, `clonefile` for copies, with the
//! shared Unix sparse copy as fallback.

use std::collections::VecDeque;
use std::ffi::{CString, OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use rustix::fs::{Mode, OFlags};
use types::Attrs;

use super::unix;
use super::{CopyMethod, RawEntry, RawKind};

// From <sys/attr.h>.
const ATTR_BIT_MAP_COUNT: u16 = 5;
const ATTR_CMN_NAME: u32 = 0x0000_0001;
const ATTR_CMN_DEVID: u32 = 0x0000_0002;
const ATTR_CMN_OBJTYPE: u32 = 0x0000_0008;
const ATTR_CMN_CRTIME: u32 = 0x0000_0200;
const ATTR_CMN_MODTIME: u32 = 0x0000_0400;
const ATTR_CMN_ACCTIME: u32 = 0x0000_1000;
const ATTR_CMN_ACCESSMASK: u32 = 0x0002_0000;
const ATTR_CMN_FLAGS: u32 = 0x0004_0000;
const ATTR_CMN_FILEID: u32 = 0x0200_0000;
const ATTR_CMN_RETURNED_ATTRS: u32 = 0x8000_0000;
const ATTR_FILE_DATALENGTH: u32 = 0x0000_0200;
// From <sys/vnode.h> and <sys/stat.h>.
const VREG: u32 = 1;
const VDIR: u32 = 2;
const VLNK: u32 = 5;
const UF_HIDDEN: u32 = 0x8000;

#[repr(C)]
struct AttrList {
    bitmapcount: u16,
    reserved: u16,
    commonattr: u32,
    volattr: u32,
    dirattr: u32,
    fileattr: u32,
    forkattr: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AttributeSet {
    commonattr: u32,
    volattr: u32,
    dirattr: u32,
    fileattr: u32,
    forkattr: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AttrReference {
    attr_dataoffset: i32,
    attr_length: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

unsafe extern "C" {
    fn getattrlistbulk(
        dirfd: libc::c_int,
        attr_list: *mut AttrList,
        attr_buf: *mut libc::c_void,
        attr_buf_size: libc::size_t,
        options: u64,
    ) -> libc::c_int;
    fn clonefile(src: *const libc::c_char, dst: *const libc::c_char, flags: u32) -> libc::c_int;
}

const BUFFER_SIZE: usize = 256 * 1024;

pub(super) struct DirReader {
    fd: OwnedFd,
    buffer: Vec<u8>,
    pending: VecDeque<RawEntry>,
    exhausted: bool,
}

impl DirReader {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let fd = rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(Self {
            fd,
            buffer: vec![0; BUFFER_SIZE],
            pending: VecDeque::new(),
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
        let mut list = AttrList {
            bitmapcount: ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: ATTR_CMN_RETURNED_ATTRS
                | ATTR_CMN_NAME
                | ATTR_CMN_DEVID
                | ATTR_CMN_OBJTYPE
                | ATTR_CMN_CRTIME
                | ATTR_CMN_MODTIME
                | ATTR_CMN_ACCTIME
                | ATTR_CMN_ACCESSMASK
                | ATTR_CMN_FLAGS
                | ATTR_CMN_FILEID,
            volattr: 0,
            dirattr: 0,
            fileattr: ATTR_FILE_DATALENGTH,
            forkattr: 0,
        };
        // SAFETY: `fd` is an open directory, `list` is a valid `attrlist`
        // with the bitmap count the kernel expects, and `buffer` is writable
        // for `BUFFER_SIZE` bytes, which is what we pass as its size.
        let count = unsafe {
            getattrlistbulk(
                self.fd.as_raw_fd(),
                &raw mut list,
                self.buffer.as_mut_ptr().cast::<libc::c_void>(),
                self.buffer.len(),
                0,
            )
        };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        if count == 0 {
            self.exhausted = true;
            return Ok(());
        }
        let mut offset = 0usize;
        for _ in 0..count {
            let Some(entry) = self.buffer.get(offset..) else {
                break;
            };
            let length = read::<u32>(entry, 0) as usize;
            if length < 4 || offset + length > self.buffer.len() {
                break;
            }
            if let Some(raw) = parse_entry(&entry[..length]) {
                self.pending.push_back(raw);
            }
            offset += length;
        }
        Ok(())
    }
}

/// Reads a `T` at `offset`, unaligned, as the packed attribute buffer needs.
fn read<T: Copy>(buffer: &[u8], offset: usize) -> T {
    let mut value = std::mem::MaybeUninit::<T>::uninit();
    let size = std::mem::size_of::<T>();
    // SAFETY: callers only pass offsets that the entry length and the
    // returned-attribute bitmap guarantee are inside `buffer`; the copy is
    // of `size_of::<T>()` bytes into a buffer of that size, and every `T`
    // used here is a plain `repr(C)` integer struct for which any bit
    // pattern is valid.
    unsafe {
        std::ptr::copy_nonoverlapping(
            buffer
                .as_ptr()
                .add(offset.min(buffer.len().saturating_sub(size))),
            value.as_mut_ptr().cast::<u8>(),
            size,
        );
        value.assume_init()
    }
}

fn parse_entry(entry: &[u8]) -> Option<RawEntry> {
    let returned = read::<AttributeSet>(entry, 4);
    let mut offset = 4 + std::mem::size_of::<AttributeSet>();
    let common = returned.commonattr;

    let name: OsString = if common & ATTR_CMN_NAME != 0 {
        let reference = read::<AttrReference>(entry, offset);
        let start =
            usize::try_from(i64::try_from(offset).ok()? + i64::from(reference.attr_dataoffset))
                .ok()?;
        let end = start + reference.attr_length as usize;
        let bytes = entry.get(start..end)?;
        let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
        offset += std::mem::size_of::<AttrReference>();
        OsStr::from_bytes(bytes).to_owned()
    } else {
        OsString::new()
    };
    let mut dev = 0u32;
    if common & ATTR_CMN_DEVID != 0 {
        dev = read::<u32>(entry, offset);
        offset += 4;
    }
    let mut kind = RawKind::Other;
    if common & ATTR_CMN_OBJTYPE != 0 {
        kind = match read::<u32>(entry, offset) {
            VREG => RawKind::File,
            VDIR => RawKind::Dir,
            VLNK => RawKind::Symlink,
            _ => RawKind::Other,
        };
        offset += 4;
    }
    let time = |flag: u32, offset: &mut usize| {
        if common & flag != 0 {
            let ts = read::<Timespec>(entry, *offset);
            *offset += std::mem::size_of::<Timespec>();
            unix::timestamp(ts.tv_sec, ts.tv_nsec)
        } else {
            None
        }
    };
    let created = time(ATTR_CMN_CRTIME, &mut offset);
    let modified = time(ATTR_CMN_MODTIME, &mut offset);
    let accessed = time(ATTR_CMN_ACCTIME, &mut offset);
    let mut mode = 0o777u32;
    if common & ATTR_CMN_ACCESSMASK != 0 {
        mode = read::<u32>(entry, offset);
        offset += 4;
    }
    let mut extra = Attrs::empty();
    if common & ATTR_CMN_FLAGS != 0 {
        if read::<u32>(entry, offset) & UF_HIDDEN != 0 {
            extra |= Attrs::HIDDEN;
        }
        offset += 4;
    }
    let mut file_id = 0u64;
    if common & ATTR_CMN_FILEID != 0 {
        file_id = read::<u64>(entry, offset);
        offset += 8;
    }
    let mut size = None;
    if returned.fileattr & ATTR_FILE_DATALENGTH != 0 {
        size = u64::try_from(read::<i64>(entry, offset)).ok();
    }
    if name == "." || name == ".." || name.is_empty() {
        return None;
    }
    Some(RawEntry {
        attrs: unix::attrs(&name, mode, kind, extra),
        name,
        id: (u128::from(dev) << 64) | u128::from(file_id),
        kind,
        size,
        created,
        modified,
        accessed,
    })
}

pub(super) fn copy_file(
    from: &Path,
    to: &Path,
    overwrite: bool,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> io::Result<CopyMethod> {
    let src = File::open(from)?;
    let meta = src.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let total = meta.len();
    if std::fs::symlink_metadata(to).is_ok() {
        if !overwrite {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        std::fs::remove_file(to)?;
    }

    let result = (|| {
        let (c_from, c_to) = (
            CString::new(from.as_os_str().as_bytes())?,
            CString::new(to.as_os_str().as_bytes())?,
        );
        // SAFETY: both are valid NUL-terminated paths that outlive the call.
        let rc = unsafe { clonefile(c_from.as_ptr(), c_to.as_ptr(), 0) };
        if rc == 0 {
            if !progress(total, total) {
                return Err(unix::cancelled());
            }
            return Ok(CopyMethod::Reflink);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            // Another volume, a filesystem without clones, or a file that
            // cannot be cloned: copy the bytes.
            Some(libc::ENOTSUP | libc::EXDEV | libc::EINVAL | libc::EPERM | libc::EACCES) => {}
            _ => return Err(error),
        }
        let dst = unix::open_target(to, true)?;
        unix::copy_sparse(&src, &dst, total, progress, &mut |offset, len, report| {
            unix::copy_range_buffered(&src, &dst, offset, len, report)
        })?;
        Ok(CopyMethod::Buffered)
    })();
    unix::finish(result, &src, to)
}

pub(super) fn stat(path: &Path) -> io::Result<RawEntry> {
    unix::stat(path)
}

pub(super) fn symlink(target: &Path, at: &Path) -> io::Result<()> {
    unix::symlink(target, at)
}

pub(super) fn junction(_target: &Path, _at: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "directory junctions exist on Windows only",
    ))
}

pub(super) fn shortcut(_target: &Path, _at: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        ".lnk shortcuts exist on Windows only",
    ))
}

pub(super) fn allocated_size(path: &Path) -> io::Result<u64> {
    unix::allocated_size(path)
}

// Same signature as the Linux version, which can fail.
#[allow(clippy::unnecessary_wraps)]
pub(super) fn shared_extents(_path: &Path) -> io::Result<Option<bool>> {
    Ok(None)
}
