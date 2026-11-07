//! Rick's own record of what it trashed: the [`TrashStore`] the trash
//! actions see, written to `trash.toml` in the config directory so that on
//! macOS, where the platform cannot list original paths, the Trash section
//! still shows what Rick trashed in this and earlier sessions.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use ops::TrashStore;
use platform::TrashedItem;
use serde::{Deserialize, Serialize};
use types::Error;

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

/// The record, shared with the actions.
#[derive(Debug, Default)]
pub struct TrashBridge {
    items: Mutex<Vec<TrashedItem>>,
    dirty: AtomicBool,
}

impl TrashBridge {
    fn lock(&self) -> MutexGuard<'_, Vec<TrashedItem>> {
        // Plain data; a panic elsewhere leaves it usable.
        self.items.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the record with what a file held.
    pub fn set(&self, items: Vec<TrashedItem>) {
        *self.lock() = items;
    }

    /// Whether the record changed since the last [`take_dirty`](Self::take_dirty).
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::SeqCst)
    }

    /// Drops recorded items whose trashed path no longer exists, after the
    /// platform listed its trash and knows better.
    pub fn prune(&self) {
        let mut items = self.lock();
        let before = items.len();
        items.retain(|i| i.trashed.exists());
        if items.len() != before {
            self.dirty.store(true, Ordering::SeqCst);
        }
    }
}

impl TrashStore for TrashBridge {
    fn items(&self) -> Vec<TrashedItem> {
        self.lock().clone()
    }

    fn record(&self, items: &[TrashedItem]) {
        let mut known = self.lock();
        for item in items {
            known.retain(|i| i.trashed != item.trashed);
            known.push(item.clone());
        }
        known.sort_by_key(|i| std::cmp::Reverse(i.deleted_at));
        self.dirty.store(true, Ordering::SeqCst);
    }

    fn forget(&self, items: &[TrashedItem]) {
        let mut known = self.lock();
        let before = known.len();
        known.retain(|i| !items.iter().any(|gone| gone.trashed == i.trashed));
        if known.len() != before {
            self.dirty.store(true, Ordering::SeqCst);
        }
    }
}

/// Writes the record.
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
    let text = toml::to_string_pretty(&file)
        .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
    crate::session::write_atomically(path, &text)
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
    let file: File = toml::from_str(&text).map_err(|error| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        ))
    })?;
    Ok(file
        .items
        .into_iter()
        .map(|r| TrashedItem {
            original: PathBuf::from(r.original),
            trashed: PathBuf::from(r.trashed),
            deleted_at: SystemTime::UNIX_EPOCH + Duration::from_secs(r.deleted_at),
        })
        .filter(|i| i.trashed.exists())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_round_trips_and_forgets() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let trashed = dir.path().join("t1");
        std::fs::write(&trashed, "x").unwrap_or_else(|e| panic!("{e}"));
        let item = TrashedItem {
            original: PathBuf::from("/home/me/a.txt"),
            trashed: trashed.clone(),
            deleted_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        };
        let gone = TrashedItem {
            trashed: dir.path().join("nope"),
            ..item.clone()
        };
        let bridge = TrashBridge::default();
        bridge.record(&[item.clone(), gone.clone()]);
        assert!(bridge.take_dirty());
        assert!(!bridge.take_dirty());
        assert_eq!(bridge.items().len(), 2);
        let file = dir.path().join("cfg").join(TRASH_FILE);
        save(&file, &bridge.items()).unwrap_or_else(|e| panic!("{e}"));
        let loaded = load(&file).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            loaded,
            std::slice::from_ref(&item),
            "the vanished item is dropped"
        );
        bridge.prune();
        assert!(bridge.take_dirty());
        bridge.forget(&[item]);
        assert_eq!(bridge.items().len(), 0, "the record is empty");
        assert!(load(&dir.path().join("missing")).is_ok_and(|l| l.is_empty()));
    }
}
