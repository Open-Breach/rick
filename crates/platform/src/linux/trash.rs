//! The freedesktop.org trash specification.
//!
//! Entries go to `$XDG_DATA_HOME/Trash` when they live on the same device as
//! it, otherwise to `$topdir/.Trash/$uid` (an administrator-created sticky
//! directory) or `$topdir/.Trash-$uid` at the root of their own volume. Each
//! trashed entry has a `.trashinfo` file naming its original path and
//! deletion time.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use types::Error;

use super::percent_encode;
use crate::time::{iso8601, parse_iso8601};
use crate::traits::TrashedItem;

struct TrashDir {
    root: PathBuf,
    /// For a volume trash, paths in `.trashinfo` are relative to this.
    topdir: Option<PathBuf>,
}

fn home_trash() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("Trash")
}

/// Device of `path` or of its nearest existing ancestor.
fn device_of(path: &Path) -> io::Result<u64> {
    let mut current = Some(path);
    while let Some(p) = current {
        match fs::metadata(p) {
            Ok(meta) => return Ok(meta.dev()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => current = p.parent(),
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no existing ancestor",
    ))
}

/// The mount point of the volume `path` is on.
fn topdir_of(path: &Path, dev: u64) -> PathBuf {
    let mut top = path.to_path_buf();
    while let Some(parent) = top.parent() {
        match fs::metadata(parent) {
            Ok(meta) if meta.dev() == dev => top = parent.to_path_buf(),
            _ => break,
        }
    }
    top
}

fn trash_dir_for(path: &Path, dev: u64) -> Result<TrashDir, Error> {
    let home = home_trash();
    if device_of(&home).map_err(Error::from_io)? == dev {
        return Ok(TrashDir {
            root: home,
            topdir: None,
        });
    }
    let topdir = topdir_of(path, dev);
    let uid = rustix::process::getuid().as_raw();
    let shared = topdir.join(".Trash");
    let root = match fs::symlink_metadata(&shared) {
        Ok(meta) if meta.is_dir() && meta.permissions().mode() & 0o1000 != 0 => {
            shared.join(uid.to_string())
        }
        _ => topdir.join(format!(".Trash-{uid}")),
    };
    Ok(TrashDir {
        root,
        topdir: Some(topdir),
    })
}

fn with_suffix(name: &OsStr, attempt: u32, suffix: &str) -> OsString {
    let mut out = name.to_os_string();
    if attempt > 0 {
        out.push(format!(".{attempt}"));
    }
    out.push(suffix);
    out
}

pub(super) fn trash(path: &Path) -> Result<TrashedItem, Error> {
    let original = std::path::absolute(path).map_err(Error::from_io)?;
    let meta = fs::symlink_metadata(&original).map_err(Error::from_io)?;
    let name = original
        .file_name()
        .ok_or(Error::Unsupported("cannot trash a filesystem root"))?;
    let dir = trash_dir_for(&original, meta.dev())?;
    let files = dir.root.join("files");
    let info_dir = dir.root.join("info");
    fs::create_dir_all(&files).map_err(Error::from_io)?;
    fs::create_dir_all(&info_dir).map_err(Error::from_io)?;

    let recorded = match &dir.topdir {
        Some(topdir) => original.strip_prefix(topdir).unwrap_or(&original),
        None => &original,
    };
    let now = SystemTime::now();
    let body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        percent_encode(recorded.as_os_str().as_bytes()),
        iso8601(now)
    );

    // The `.trashinfo` file is created exclusively, which also reserves the
    // name under `files/`.
    let mut attempt = 0u32;
    let (trashed, info) = loop {
        let candidate = with_suffix(name, attempt, "");
        let info = info_dir.join(with_suffix(name, attempt, ".trashinfo"));
        match fs::File::create_new(&info) {
            Ok(mut file) => {
                file.write_all(body.as_bytes()).map_err(Error::from_io)?;
                break (files.join(candidate), info);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && attempt < 10_000 => {
                attempt += 1;
            }
            Err(e) => return Err(Error::from_io(e)),
        }
    };

    if let Err(e) = fs::rename(&original, &trashed) {
        let _ = fs::remove_file(&info);
        return Err(Error::from_io(e));
    }
    tracing::debug!(original = %original.display(), trashed = %trashed.display(), "trashed");
    Ok(TrashedItem {
        original,
        trashed,
        deleted_at: now,
    })
}

fn info_path_for(trashed: &Path) -> Option<PathBuf> {
    let name = trashed.file_name()?;
    let root = trashed.parent()?.parent()?;
    Some(root.join("info").join(with_suffix(name, 0, ".trashinfo")))
}

pub(super) fn restore(item: &TrashedItem) -> Result<(), Error> {
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
    if let Some(info) = info_path_for(&item.trashed) {
        let _ = fs::remove_file(info);
    }
    Ok(())
}

fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(value) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(value);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn parse_info(text: &str, trashed: PathBuf) -> Option<TrashedItem> {
    let mut path = None;
    let mut deleted_at = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("Path=") {
            path = Some(PathBuf::from(OsString::from_vec(percent_decode(value))));
        } else if let Some(value) = line.strip_prefix("DeletionDate=") {
            deleted_at = parse_iso8601(value);
        }
    }
    Some(TrashedItem {
        original: path?,
        trashed,
        deleted_at: deleted_at.unwrap_or(SystemTime::UNIX_EPOCH),
    })
}

/// Deletes a file or a whole directory; a missing path is fine.
fn remove_path(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Deletes one trashed entry and its `.trashinfo` record for good.
pub(super) fn remove(item: &TrashedItem) -> Result<(), Error> {
    if fs::symlink_metadata(&item.trashed).is_err() {
        return Err(Error::NotFound);
    }
    remove_path(&item.trashed).map_err(Error::from_io)?;
    if let Some(info) = info_path_for(&item.trashed) {
        let _ = fs::remove_file(info);
    }
    Ok(())
}

/// Deletes everything in the home trash for good: every payload under
/// `files/` and every record under `info/`.
pub(super) fn empty() -> Result<(), Error> {
    let root = home_trash();
    for sub in ["files", "info"] {
        let dir = root.join(sub);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(Error::from_io(e)),
        };
        for entry in entries {
            let entry = entry.map_err(Error::from_io)?;
            remove_path(&entry.path()).map_err(Error::from_io)?;
        }
    }
    Ok(())
}

/// The home trash. Volume trashes are not walked; they belong to a later
/// trash provider.
pub(super) fn list() -> Result<Vec<TrashedItem>, Error> {
    let root = home_trash();
    let info_dir = root.join("info");
    let entries = match fs::read_dir(&info_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::from_io(e)),
    };
    let mut items = Vec::new();
    for entry in entries {
        let entry = entry.map_err(Error::from_io)?;
        let info_name = entry.file_name();
        let Some(stem) = info_name.as_bytes().strip_suffix(b".trashinfo") else {
            continue;
        };
        let trashed = root.join("files").join(OsStr::from_bytes(stem));
        let Ok(text) = fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Some(item) = parse_info(&text, trashed) {
            items.push(item);
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_files_round_trip_paths_and_dates() {
        let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let text = format!(
            "[Trash Info]\nPath={}\nDeletionDate={}\n",
            percent_encode(b"/home/me/a b%.txt"),
            iso8601(now)
        );
        let item = parse_info(&text, PathBuf::from("/trash/files/a b%.txt"))
            .unwrap_or_else(|| panic!("parse"));
        assert_eq!(item.original, PathBuf::from("/home/me/a b%.txt"));
        assert_eq!(item.deleted_at, now);
        assert_eq!(
            info_path_for(Path::new("/t/files/x.txt")),
            Some(PathBuf::from("/t/info/x.txt.trashinfo"))
        );
        assert_eq!(percent_decode("a%2"), b"a%2");
    }

    #[test]
    fn suffixes_make_names_unique() {
        assert_eq!(
            with_suffix(OsStr::new("a.txt"), 0, ".trashinfo"),
            "a.txt.trashinfo"
        );
        assert_eq!(with_suffix(OsStr::new("a.txt"), 2, ""), "a.txt.2");
    }

    #[test]
    fn home_trash_ends_with_trash() {
        assert!(home_trash().ends_with("Trash"));
    }
}
