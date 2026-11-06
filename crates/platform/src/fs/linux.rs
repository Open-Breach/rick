//! Linux: `getdents` + `statx` through a directory fd, `FICLONE`,
//! `copy_file_range`, `FIEMAP`.

use std::ffi::{CStr, OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use rayon::prelude::*;
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, Statx, StatxFlags};
use types::Attrs;

use super::unix;
use super::{CopyMethod, RawEntry, RawKind};

/// Batches at least this large are stat'ed on the compute pool; smaller
/// ones are not worth the hand-off.
const PARALLEL_STAT_FROM: usize = 64;

pub(super) struct DirReader {
    fd: OwnedFd,
    dir: Dir,
    dev: u64,
}

impl DirReader {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let fd = rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let stat = rustix::fs::fstat(&fd)?;
        let dir = Dir::read_from(&fd)?;
        Ok(Self {
            fd,
            dir,
            dev: stat.st_dev,
        })
    }

    pub(super) fn next_batch(&mut self, max: usize) -> io::Result<Vec<RawEntry>> {
        let mut names: Vec<(Box<CStr>, RawKind, u64)> = Vec::with_capacity(max);
        while names.len() < max {
            match self.dir.next() {
                None => break,
                Some(Err(e)) => return Err(e.into()),
                Some(Ok(entry)) => {
                    let name = entry.file_name();
                    if name == c"." || name == c".." {
                        continue;
                    }
                    names.push((name.into(), kind_of(entry.file_type()), entry.ino()));
                }
            }
        }
        let (fd, dev) = (self.fd.as_fd(), self.dev);
        let stat =
            |(name, kind, ino): &(Box<CStr>, RawKind, u64)| stat_one(fd, dev, name, *kind, *ino);
        let entries = if names.len() >= PARALLEL_STAT_FROM {
            runtime::compute::pool().install(|| names.par_iter().map(stat).collect())
        } else {
            names.iter().map(stat).collect()
        };
        Ok(entries)
    }
}

fn kind_of(file_type: FileType) -> RawKind {
    match file_type {
        FileType::RegularFile => RawKind::File,
        FileType::Directory => RawKind::Dir,
        FileType::Symlink => RawKind::Symlink,
        _ => RawKind::Other,
    }
}

fn id(dev: u64, ino: u64) -> u128 {
    (u128::from(dev) << 64) | u128::from(ino)
}

fn stat_one(dirfd: BorrowedFd<'_>, dev: u64, name: &CStr, dkind: RawKind, ino: u64) -> RawEntry {
    let os_name: OsString = OsStr::from_bytes(name.to_bytes()).to_owned();
    match rustix::fs::statx(
        dirfd,
        name,
        AtFlags::SYMLINK_NOFOLLOW | AtFlags::STATX_DONT_SYNC,
        StatxFlags::BASIC_STATS | StatxFlags::BTIME,
    ) {
        Ok(stx) => from_statx(os_name, dev, &stx),
        Err(_) => RawEntry {
            attrs: unix::attrs(&os_name, 0o777, dkind, Attrs::empty()),
            name: os_name,
            id: id(dev, ino),
            kind: dkind,
            size: None,
            created: None,
            modified: None,
            accessed: None,
        },
    }
}

fn from_statx(name: OsString, dev: u64, stx: &Statx) -> RawEntry {
    let mode = u32::from(stx.stx_mode);
    let kind = kind_of(FileType::from_raw_mode(mode));
    let time = |t: rustix::fs::StatxTimestamp| unix::timestamp(t.tv_sec, i64::from(t.tv_nsec));
    let has = |flag: StatxFlags| StatxFlags::from_bits_retain(stx.stx_mask).contains(flag);
    RawEntry {
        attrs: unix::attrs(&name, mode, kind, Attrs::empty()),
        name,
        id: id(dev, stx.stx_ino),
        kind,
        size: has(StatxFlags::SIZE).then_some(stx.stx_size),
        created: has(StatxFlags::BTIME)
            .then(|| time(stx.stx_btime))
            .flatten(),
        modified: has(StatxFlags::MTIME)
            .then(|| time(stx.stx_mtime))
            .flatten(),
        accessed: has(StatxFlags::ATIME)
            .then(|| time(stx.stx_atime))
            .flatten(),
    }
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
    let dst = unix::open_target(to, overwrite)?;

    let result = (|| {
        match rustix::fs::ioctl_ficlone(&dst, &src) {
            Ok(()) => {
                if !progress(total, total) {
                    return Err(unix::cancelled());
                }
                return Ok(CopyMethod::Reflink);
            }
            // Not the same filesystem, or one without reflinks: copy bytes.
            Err(
                rustix::io::Errno::XDEV
                | rustix::io::Errno::INVAL
                | rustix::io::Errno::OPNOTSUPP
                | rustix::io::Errno::NOTTY
                | rustix::io::Errno::NOSYS
                | rustix::io::Errno::PERM
                | rustix::io::Errno::TXTBSY
                | rustix::io::Errno::BADF,
            ) => {}
            Err(e) => return Err(e.into()),
        }
        let mut method = CopyMethod::Kernel;
        unix::copy_sparse(&src, &dst, total, progress, &mut |offset, len, report| {
            copy_range(&src, &dst, offset, len, report, &mut method)
        })?;
        Ok(method)
    })();
    unix::finish(result, &src, to)
}

/// Copies a range with `copy_file_range`, falling back to buffered I/O when
/// the kernel or filesystem refuses.
fn copy_range(
    src: &File,
    dst: &File,
    offset: u64,
    len: u64,
    report: &mut dyn FnMut(u64) -> bool,
    method: &mut CopyMethod,
) -> io::Result<()> {
    let end = offset + len;
    let mut done = offset;
    while done < end {
        let (mut off_in, mut off_out) = (done, done);
        let want = usize::try_from(end - done).unwrap_or(usize::MAX);
        match rustix::fs::copy_file_range(src, Some(&mut off_in), dst, Some(&mut off_out), want) {
            Ok(0) => break, // the source shrank under us
            Ok(n) => {
                done += n as u64;
                if !report(done) {
                    return Err(unix::cancelled());
                }
            }
            Err(
                rustix::io::Errno::XDEV
                | rustix::io::Errno::INVAL
                | rustix::io::Errno::NOSYS
                | rustix::io::Errno::OPNOTSUPP
                | rustix::io::Errno::TXTBSY,
            ) => {
                *method = CopyMethod::Buffered;
                return unix::copy_range_buffered(src, dst, done, end - done, report);
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub(super) fn allocated_size(path: &Path) -> io::Result<u64> {
    unix::allocated_size(path)
}

// The FIEMAP ioctl, which the libc crate does not expose.

// Field names mirror `struct fiemap_extent` and `struct fiemap` in
// `linux/fiemap.h` so the layout can be checked against the header.
#[allow(clippy::struct_field_names)]
#[repr(C)]
#[derive(Clone, Copy)]
struct FiemapExtent {
    fe_logical: u64,
    fe_physical: u64,
    fe_length: u64,
    fe_reserved64: [u64; 2],
    fe_flags: u32,
    fe_reserved: [u32; 3],
}

#[allow(clippy::struct_field_names)]
#[repr(C)]
struct Fiemap {
    fm_start: u64,
    fm_length: u64,
    fm_flags: u32,
    fm_mapped_extents: u32,
    fm_extent_count: u32,
    fm_reserved: u32,
}

const EXTENTS_PER_CALL: usize = 256;

#[repr(C)]
struct FiemapBuffer {
    header: Fiemap,
    extents: [FiemapExtent; EXTENTS_PER_CALL],
}

const FS_IOC_FIEMAP: u64 = 0xC020_660B;
const FIEMAP_FLAG_SYNC: u32 = 0x1;
const FIEMAP_EXTENT_LAST: u32 = 0x1;
const FIEMAP_EXTENT_SHARED: u32 = 0x2000;

pub(super) fn shared_extents(path: &Path) -> io::Result<Option<bool>> {
    let file = File::open(path)?;
    let zero_extent = FiemapExtent {
        fe_logical: 0,
        fe_physical: 0,
        fe_length: 0,
        fe_reserved64: [0; 2],
        fe_flags: 0,
        fe_reserved: [0; 3],
    };
    let mut buffer = Box::new(FiemapBuffer {
        header: Fiemap {
            fm_start: 0,
            fm_length: 0,
            fm_flags: 0,
            fm_mapped_extents: 0,
            fm_extent_count: 0,
            fm_reserved: 0,
        },
        extents: [zero_extent; EXTENTS_PER_CALL],
    });
    let mut start = 0u64;
    loop {
        buffer.header = Fiemap {
            fm_start: start,
            fm_length: u64::MAX - start,
            fm_flags: FIEMAP_FLAG_SYNC,
            fm_mapped_extents: 0,
            fm_extent_count: u32::try_from(EXTENTS_PER_CALL).unwrap_or(u32::MAX),
            fm_reserved: 0,
        };
        // SAFETY: `FS_IOC_FIEMAP` takes a pointer to a `struct fiemap`
        // followed by `fm_extent_count` `struct fiemap_extent`s, which is
        // exactly the layout of `FiemapBuffer` (`repr(C)`, extents directly
        // after the header). The buffer outlives the call and the kernel
        // writes at most `fm_extent_count` extents.
        let rc = unsafe {
            libc::ioctl(
                file.as_raw_fd(),
                FS_IOC_FIEMAP as _,
                std::ptr::from_mut::<FiemapBuffer>(&mut buffer).cast::<libc::c_void>(),
            )
        };
        if rc != 0 {
            let error = io::Error::last_os_error();
            return match error.raw_os_error() {
                Some(libc::ENOTTY | libc::EOPNOTSUPP | libc::EINVAL | libc::ENOSYS) => Ok(None),
                _ => Err(error),
            };
        }
        let mapped = (buffer.header.fm_mapped_extents as usize).min(EXTENTS_PER_CALL);
        if mapped == 0 {
            return Ok(Some(false));
        }
        for extent in &buffer.extents[..mapped] {
            if extent.fe_flags & FIEMAP_EXTENT_SHARED != 0 {
                return Ok(Some(true));
            }
            if extent.fe_flags & FIEMAP_EXTENT_LAST != 0 {
                return Ok(Some(false));
            }
        }
        let last = buffer.extents[mapped - 1];
        start = last.fe_logical + last.fe_length;
    }
}
