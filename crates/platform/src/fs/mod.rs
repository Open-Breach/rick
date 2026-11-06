//! Filesystem fast paths the local provider needs and no portable API offers.
//!
//! - [`DirReader`] lists a directory in batches with every attribute the
//!   `Entry` type carries, from one bulk syscall per batch where the OS has
//!   one (`getattrlistbulk`, `GetFileInformationByHandleEx`) and from
//!   `getdents` plus `statx` on Linux.
//! - [`copy_file`] copies one regular file the fastest way the volume allows:
//!   a reflink (`FICLONE`, `clonefile`), an in-kernel copy
//!   (`copy_file_range`, `CopyFileExW`), or a buffered copy, skipping holes
//!   so sparse files stay sparse.
//! - [`allocated_size`] and [`shared_extents`] answer what a copy did.

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use types::Attrs;

#[cfg(unix)]
mod unix;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(windows)]
pub(crate) mod windows;
#[cfg(windows)]
use windows as imp;

/// What kind of thing a directory entry is, before any link is followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symbolic link (or, on Windows, a junction).
    Symlink,
    /// Anything else.
    Other,
}

/// One directory entry as the OS reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    /// The name within the directory.
    pub name: OsString,
    /// Provider-scoped identity: device and inode on Unix, volume serial
    /// and file id on Windows.
    pub id: u128,
    /// File, directory, link or other.
    pub kind: RawKind,
    /// Size in bytes, when known.
    pub size: Option<u64>,
    /// Creation time, when the filesystem records it.
    pub created: Option<SystemTime>,
    /// Modification time.
    pub modified: Option<SystemTime>,
    /// Access time.
    pub accessed: Option<SystemTime>,
    /// Attribute flags.
    pub attrs: Attrs,
}

/// Reads a directory in batches.
pub struct DirReader(imp::DirReader);

impl DirReader {
    /// Opens `dir` for listing.
    ///
    /// # Errors
    ///
    /// The I/O error from opening the directory.
    pub fn open(dir: &Path) -> io::Result<Self> {
        imp::DirReader::open(dir).map(Self)
    }

    /// The next batch of up to `max` entries, without `.` and `..`. An
    /// empty batch means the directory is exhausted. Entries that vanish
    /// between being listed and being examined are skipped.
    ///
    /// # Errors
    ///
    /// The I/O error from reading the directory.
    pub fn next_batch(&mut self, max: usize) -> io::Result<Vec<RawEntry>> {
        self.0.next_batch(max)
    }
}

impl fmt::Debug for DirReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DirReader")
    }
}

/// The entry at `path` itself, without following a final link, with the
/// same identity and attributes [`DirReader`] would report for it.
///
/// # Errors
///
/// The I/O error from examining the entry.
pub fn stat(path: &Path) -> io::Result<RawEntry> {
    imp::stat(path)
}

/// Creates a symbolic link at `at` pointing to `target`. On Windows the link
/// is a directory link when `target` is a directory.
///
/// # Errors
///
/// The I/O error from creating the link, including `PermissionDenied` on
/// Windows without developer mode or elevation.
pub fn symlink(target: &Path, at: &Path) -> io::Result<()> {
    imp::symlink(target, at)
}

/// Which path [`copy_file`] took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMethod {
    /// Extents are shared with the source; no bytes were copied.
    Reflink,
    /// The kernel copied the bytes without them passing through userspace.
    Kernel,
    /// Read and written through a buffer.
    Buffered,
}

/// Copies one regular file.
///
/// `progress(done, total)` is called as bytes land, with `done` counting
/// logical bytes including holes; returning `false` cancels the copy, which
/// then fails with `ErrorKind::Interrupted` and removes the partial target.
/// Holes in the source stay holes in the target. The target keeps the
/// source's modification time.
///
/// # Errors
///
/// `AlreadyExists` when `to` exists and `overwrite` is `false`;
/// `InvalidInput` when `from` is not a regular file; `Interrupted` when
/// cancelled; otherwise the I/O error.
pub fn copy_file(
    from: &Path,
    to: &Path,
    overwrite: bool,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> io::Result<CopyMethod> {
    imp::copy_file(from, to, overwrite, progress)
}

/// Bytes the entry occupies on disk, which is less than its size for sparse
/// and compressed files.
///
/// # Errors
///
/// The I/O error from examining the entry.
pub fn allocated_size(path: &Path) -> io::Result<u64> {
    imp::allocated_size(path)
}

/// Whether any extent of the file is shared with another file, as a reflink
/// copy on Btrfs or XFS leaves them. `Ok(None)` when the OS or filesystem
/// cannot say.
///
/// # Errors
///
/// The I/O error from examining the file.
pub fn shared_extents(path: &Path) -> io::Result<Option<bool>> {
    imp::shared_extents(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Seek, SeekFrom, Write};

    #[test]
    fn reader_lists_everything_once_with_attributes() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.path().join("a.txt"), b"hello").unwrap_or_else(|e| panic!("{e}"));
        fs::create_dir(dir.path().join("sub")).unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.path().join(".hidden"), b"").unwrap_or_else(|e| panic!("{e}"));

        let mut reader = DirReader::open(dir.path()).unwrap_or_else(|e| panic!("{e}"));
        let mut all = Vec::new();
        loop {
            let batch = reader.next_batch(2).unwrap_or_else(|e| panic!("{e}"));
            if batch.is_empty() {
                break;
            }
            assert!(batch.len() <= 2);
            all.extend(batch);
        }
        all.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<_> = all
            .iter()
            .map(|e| e.name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [".hidden", "a.txt", "sub"]);
        assert_eq!(all[1].kind, RawKind::File);
        assert_eq!(all[1].size, Some(5));
        assert!(all[1].modified.is_some());
        assert_eq!(all[2].kind, RawKind::Dir);
        assert_ne!(all[1].id, all[2].id);
        assert_eq!(all[1].id, all[1].id);
        let ids: std::collections::HashSet<u128> = all.iter().map(|e| e.id).collect();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn stat_matches_the_reader_and_links_are_links() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let file = dir.path().join("f.txt");
        fs::write(&file, b"data").unwrap_or_else(|e| panic!("{e}"));
        let link = dir.path().join("l");
        if let Err(e) = symlink(&file, &link) {
            // Windows without developer mode cannot create links; that is the
            // platform's answer, not a bug.
            assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
            return;
        }

        let mut reader = DirReader::open(dir.path()).unwrap_or_else(|e| panic!("{e}"));
        let listed = reader.next_batch(10).unwrap_or_else(|e| panic!("{e}"));
        let from_reader = listed
            .iter()
            .find(|e| e.name == "f.txt")
            .unwrap_or_else(|| panic!("f.txt"));
        let direct = stat(&file).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(direct.id, from_reader.id);
        assert_eq!(direct.kind, RawKind::File);
        assert_eq!(direct.size, Some(4));
        assert_eq!(direct.name, "f.txt");
        assert_eq!(stat(&link).map(|e| e.kind).ok(), Some(RawKind::Symlink));
        assert_eq!(stat(dir.path()).map(|e| e.kind).ok(), Some(RawKind::Dir));
    }

    #[test]
    fn copy_preserves_content_and_reports_progress() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let from = dir.path().join("src.bin");
        let to = dir.path().join("dst.bin");
        let payload: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(&from, &payload).unwrap_or_else(|e| panic!("{e}"));

        let mut reports = Vec::new();
        let method = copy_file(&from, &to, false, &mut |done, total| {
            reports.push((done, total));
            true
        })
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fs::read(&to).ok(), Some(payload.clone()));
        assert_eq!(reports.last(), Some(&(100_000, 100_000)), "{method:?}");

        let again = copy_file(&from, &to, false, &mut |_, _| true);
        assert!(matches!(again, Err(e) if e.kind() == io::ErrorKind::AlreadyExists));
        copy_file(&from, &to, true, &mut |_, _| true).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            fs::metadata(&to).and_then(|m| m.modified()).ok(),
            fs::metadata(&from).and_then(|m| m.modified()).ok()
        );
    }

    #[test]
    fn cancelled_copy_removes_the_partial_target() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let from = dir.path().join("src.bin");
        let to = dir.path().join("dst.bin");
        fs::write(&from, vec![7u8; 1 << 20]).unwrap_or_else(|e| panic!("{e}"));
        let result = copy_file(&from, &to, false, &mut |_, _| false);
        assert!(
            matches!(result, Err(ref e) if e.kind() == io::ErrorKind::Interrupted),
            "{result:?}"
        );
        assert!(!to.exists());
    }

    #[test]
    fn sparse_files_stay_sparse() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let from = dir.path().join("sparse.bin");
        let to = dir.path().join("copy.bin");
        {
            let mut file = fs::File::create(&from).unwrap_or_else(|e| panic!("{e}"));
            file.write_all(b"head").unwrap_or_else(|e| panic!("{e}"));
            file.seek(SeekFrom::Start(64 << 20))
                .unwrap_or_else(|e| panic!("{e}"));
            file.write_all(b"tail").unwrap_or_else(|e| panic!("{e}"));
            file.set_len(128 << 20).unwrap_or_else(|e| panic!("{e}"));
        }
        copy_file(&from, &to, false, &mut |_, _| true).unwrap_or_else(|e| panic!("{e}"));
        let meta = fs::metadata(&to).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(meta.len(), 128 << 20);
        assert_eq!(
            fs::read(&to)
                .map(|c| (c[..4].to_vec(), c[(64 << 20)..(64 << 20) + 4].to_vec()))
                .ok(),
            Some((b"head".to_vec(), b"tail".to_vec()))
        );
        let allocated = allocated_size(&to).unwrap_or_else(|e| panic!("{e}"));
        let source_allocated = allocated_size(&from).unwrap_or_else(|e| panic!("{e}"));
        // A filesystem without sparse support allocates everything for both;
        // one with it allocates little for both. Either way the copy is no
        // fatter than its source, within a block or two.
        assert!(
            allocated <= source_allocated + (1 << 16),
            "{allocated} vs {source_allocated}"
        );
        let _ = shared_extents(&to).unwrap_or_else(|e| panic!("{e}"));
    }
}
