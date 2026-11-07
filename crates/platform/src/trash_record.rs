//! Rick's own record of what it trashed, for a platform whose trash cannot
//! list the original paths of its items (macOS, QUESTIONS.md 74 and 86):
//! `trash.toml` in the config directory, one row per item with its original
//! path, its path in the trash and the deletion time. The record is read
//! back pruned of items that have left the trash since, so a restore or a
//! deletion by another program is noticed on the next listing.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use types::Error;

use crate::traits::TrashedItem;

/// The file name under the config directory.
pub const TRASH_FILE: &str = "trash.toml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    original: String,
    trashed: String,
    /// Seconds since the Unix epoch.
    deleted_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
struct File {
    items: Vec<Record>,
}

fn invalid(error: &dyn std::fmt::Display) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        error.to_string(),
    ))
}

/// Reads the record; items whose trashed path is gone are dropped. A
/// missing file is an empty record.
///
/// # Errors
///
/// The I/O error, or `InvalidData` when the file does not parse.
pub fn load(path: &Path) -> Result<Vec<TrashedItem>, Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::from_io(error)),
    };
    let file: File = toml::from_str(&text).map_err(|e| invalid(&e))?;
    let mut items: Vec<TrashedItem> = file
        .items
        .into_iter()
        .map(|r| TrashedItem {
            original: PathBuf::from(r.original),
            trashed: PathBuf::from(r.trashed),
            deleted_at: SystemTime::UNIX_EPOCH + Duration::from_secs(r.deleted_at),
        })
        .filter(|i| i.trashed.symlink_metadata().is_ok())
        .collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.deleted_at));
    Ok(items)
}

/// Writes the record, through a temporary file and a rename so a crash
/// leaves the old file whole.
///
/// # Errors
///
/// The I/O error.
pub fn save(path: &Path, items: &[TrashedItem]) -> Result<(), Error> {
    let file = File {
        items: items
            .iter()
            .map(|i| Record {
                original: i.original.to_string_lossy().into_owned(),
                trashed: i.trashed.to_string_lossy().into_owned(),
                deleted_at: i
                    .deleted_at
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs()),
            })
            .collect(),
    };
    let text = toml::to_string_pretty(&file).map_err(|e| invalid(&e))?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(Error::from_io)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(Error::from_io)?;
    std::io::Write::write_all(&mut temp, text.as_bytes()).map_err(Error::from_io)?;
    temp.persist(path)
        .map_err(|error| Error::from_io(error.error))?;
    Ok(())
}

/// Adds `item` to the record, replacing an older row with the same trashed
/// path.
///
/// # Errors
///
/// The I/O error from reading or writing the file.
pub fn record(path: &Path, item: &TrashedItem) -> Result<(), Error> {
    let mut items = load(path)?;
    items.retain(|i| i.trashed != item.trashed);
    items.insert(0, item.clone());
    save(path, &items)
}

/// Drops the row whose trashed path is `trashed`: the item was restored or
/// deleted for good.
///
/// # Errors
///
/// The I/O error from reading or writing the file.
pub fn forget(path: &Path, trashed: &Path) -> Result<(), Error> {
    let mut items = load(path)?;
    let before = items.len();
    items.retain(|i| i.trashed != trashed);
    if items.len() == before {
        return Ok(());
    }
    save(path, &items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_round_trips_prunes_and_forgets() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let trashed = dir.path().join("t1");
        std::fs::write(&trashed, "x").unwrap_or_else(|e| panic!("{e}"));
        let item = TrashedItem {
            original: PathBuf::from("/home/me/a.txt"),
            trashed: trashed.clone(),
            deleted_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        };
        let gone = TrashedItem {
            original: PathBuf::from("/home/me/b.txt"),
            trashed: dir.path().join("nope"),
            deleted_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_001),
        };
        let file = dir.path().join("cfg").join(TRASH_FILE);
        assert!(load(&file).is_ok_and(|l| l.is_empty()), "no file, no items");
        save(&file, &[gone, item.clone()]).unwrap_or_else(|e| panic!("{e}"));
        let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{e}"));
        assert!(text.contains("a.txt") && text.contains("b.txt"), "{text}");
        assert_eq!(
            load(&file).unwrap_or_else(|e| panic!("{e}")),
            std::slice::from_ref(&item),
            "the vanished item is dropped on reading"
        );
        // Recording the same trashed path again replaces the row.
        record(&file, &item).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load(&file).map(|l| l.len()).ok(), Some(1));
        forget(&file, &item.trashed).unwrap_or_else(|e| panic!("{e}"));
        assert!(load(&file).is_ok_and(|l| l.is_empty()));
        forget(&file, &item.trashed).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(&file, "items = 3").unwrap_or_else(|e| panic!("{e}"));
        assert!(
            matches!(load(&file), Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::InvalidData)
        );
    }
}
