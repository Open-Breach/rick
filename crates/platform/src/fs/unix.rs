//! Pieces shared by the Linux and macOS filesystem code.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::Path;
use std::time::{Duration, SystemTime};

use rustix::fs::SeekFrom;
use types::Attrs;

use super::{RawEntry, RawKind};

/// A `timespec`-like pair as a `SystemTime`; `None` for times before 1970.
pub(super) fn timestamp(secs: i64, nanos: i64) -> Option<SystemTime> {
    let secs = u64::try_from(secs).ok()?;
    let nanos = u32::try_from(nanos).ok()?;
    SystemTime::UNIX_EPOCH.checked_add(Duration::new(secs, nanos))
}

/// Attribute flags from a Unix mode and name. `extra` carries platform
/// flags already decided (macOS `UF_HIDDEN`, for instance).
pub(super) fn attrs(name: &OsStr, mode: u32, kind: RawKind, extra: Attrs) -> Attrs {
    let mut attrs = extra;
    if name.as_encoded_bytes().first() == Some(&b'.') {
        attrs |= Attrs::HIDDEN;
    }
    if mode & 0o222 == 0 {
        attrs |= Attrs::READONLY;
    }
    if kind == RawKind::File && mode & 0o111 != 0 {
        attrs |= Attrs::EXECUTABLE;
    }
    attrs
}

pub(super) fn stat(path: &Path) -> io::Result<RawEntry> {
    let meta = std::fs::symlink_metadata(path)?;
    let name: OsString = path.file_name().map_or_default(ToOwned::to_owned);
    let file_type = meta.file_type();
    let kind = if file_type.is_symlink() {
        RawKind::Symlink
    } else if file_type.is_dir() {
        RawKind::Dir
    } else if file_type.is_file() {
        RawKind::File
    } else {
        RawKind::Other
    };
    Ok(RawEntry {
        id: (u128::from(meta.dev()) << 64) | u128::from(meta.ino()),
        kind,
        size: matches!(kind, RawKind::File | RawKind::Symlink).then_some(meta.len()),
        created: meta.created().ok(),
        modified: meta.modified().ok(),
        accessed: meta.accessed().ok(),
        attrs: attrs(&name, meta.mode(), kind, platform_flags(&meta)),
        name,
    })
}

#[cfg(target_os = "macos")]
fn platform_flags(meta: &std::fs::Metadata) -> Attrs {
    use std::os::macos::fs::MetadataExt as _;
    const UF_HIDDEN: u32 = 0x8000;
    if meta.st_flags() & UF_HIDDEN != 0 {
        Attrs::HIDDEN
    } else {
        Attrs::empty()
    }
}

#[cfg(not(target_os = "macos"))]
fn platform_flags(_meta: &std::fs::Metadata) -> Attrs {
    Attrs::empty()
}

pub(super) fn symlink(target: &Path, at: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}

pub(super) fn unix_mode(path: &Path) -> io::Result<Option<u32>> {
    use std::os::unix::fs::PermissionsExt as _;
    Ok(Some(
        std::fs::symlink_metadata(path)?.permissions().mode() & 0o7777,
    ))
}

pub(super) fn set_unix_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777))
}

pub(super) fn allocated_size(path: &Path) -> io::Result<u64> {
    Ok(std::fs::symlink_metadata(path)?.blocks() * 512)
}

/// Opens the copy target according to `overwrite`.
pub(super) fn open_target(to: &Path, overwrite: bool) -> io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).truncate(true);
    if overwrite {
        options.create(true);
    } else {
        options.create_new(true);
    }
    options.open(to)
}

/// Copies `[offset, length)` of the source into the target, calling
/// `report(done)` as bytes land; `report` returns whether to keep going.
pub(super) type RangeCopier<'a> =
    dyn FnMut(u64, u64, &mut dyn FnMut(u64) -> bool) -> io::Result<()> + 'a;

/// Copies the data segments of `src` into `dst`, leaving holes where `src`
/// has them, through `copy_range(offset, length, report)`. `report(done)`
/// returns whether to keep going. Filesystems without `SEEK_DATA` are copied
/// as one segment.
pub(super) fn copy_sparse(
    src: &File,
    dst: &File,
    total: u64,
    progress: &mut dyn FnMut(u64, u64) -> bool,
    copy_range: &mut RangeCopier<'_>,
) -> io::Result<()> {
    let mut offset = 0u64;
    let mut segments = 0usize;
    while offset < total {
        let data = match rustix::fs::seek(src, SeekFrom::Data(offset)) {
            Ok(pos) => pos.min(total),
            // No more data before the end of the file: the rest is one hole.
            Err(rustix::io::Errno::NXIO) => break,
            // The filesystem cannot tell holes from data: everything is data.
            Err(_) if segments == 0 => {
                copy_range(0, total, &mut |done| progress(done, total))?;
                break;
            }
            Err(e) => return Err(e.into()),
        };
        let hole = rustix::fs::seek(src, SeekFrom::Hole(data)).map_or(total, |pos| pos.min(total));
        if hole > data {
            copy_range(data, hole - data, &mut |done| progress(done, total))?;
        }
        segments += 1;
        offset = hole;
    }
    // Extend the target to the full length so a trailing hole is kept.
    if dst.metadata()?.len() < total {
        dst.set_len(total)?;
    }
    if !progress(total, total) {
        return Err(cancelled());
    }
    Ok(())
}

/// Copies `[offset, offset + len)` with positioned reads and writes.
pub(super) fn copy_range_buffered(
    src: &File,
    dst: &File,
    offset: u64,
    len: u64,
    report: &mut dyn FnMut(u64) -> bool,
) -> io::Result<()> {
    let mut buffer = vec![0u8; 1 << 20];
    let mut done = offset;
    let end = offset + len;
    while done < end {
        let want = usize::try_from((end - done).min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = src.read_at(&mut buffer[..want], done)?;
        if read == 0 {
            break; // the source shrank under us
        }
        dst.write_all_at(&buffer[..read], done)?;
        done += read as u64;
        if !report(done) {
            return Err(cancelled());
        }
    }
    Ok(())
}

pub(super) fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "copy cancelled")
}

/// Finishes a copy: keeps the source's modification time, or on failure
/// removes the partial target.
pub(super) fn finish(
    result: io::Result<super::CopyMethod>,
    src: &File,
    to: &Path,
) -> io::Result<super::CopyMethod> {
    match result {
        Ok(method) => {
            if let Ok(modified) = src.metadata().and_then(|m| m.modified())
                && let Ok(dst) = File::options().write(true).open(to)
            {
                let _ = dst.set_modified(modified);
            }
            Ok(method)
        }
        Err(e) => {
            let _ = std::fs::remove_file(to);
            Err(e)
        }
    }
}
