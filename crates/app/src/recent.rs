//! The Recent feed: what fills the side panel's Recent section and how it
//! survives a restart.
//!
//! The list is fed by every change watch the application holds (the roots
//! below and the panes' own) and seeded by a bounded scan of the roots'
//! subtrees at start. It is written to `recent.toml` in the config
//! directory whenever it changes, so the rows come back on the next start.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use ext::{Depth, ListOptions, Provider};
use futures::StreamExt;
use model::RECENT_LIMIT;
use serde::{Deserialize, Serialize};
use types::{Attrs, CancelToken, Entry, EntryKind, Error, Location};

/// The file name under the config directory.
pub const RECENT_FILE: &str = "recent.toml";

/// How many entries the initial scan of one root looks at before it stops;
/// a home directory can hold millions and the newest 200 are what count.
const SCAN_CAP: u64 = 50_000;
/// How long the initial scan of one root may run.
const SCAN_FOR: Duration = Duration::from_secs(5);

/// One row of `recent.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    path: String,
    dir: bool,
    /// Milliseconds since the Unix epoch.
    modified: u64,
}

/// The whole file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
struct File {
    entries: Vec<Record>,
}

fn millis(time: Option<SystemTime>) -> u64 {
    time.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Writes the entries to `path`.
///
/// # Errors
///
/// The I/O error.
pub fn save(path: &Path, entries: &[Entry]) -> Result<(), Error> {
    let file = File {
        entries: entries
            .iter()
            .filter(|e| e.id.provider == vfs_local::PROVIDER_ID)
            .map(|e| Record {
                path: e.path.to_string_lossy().into_owned(),
                dir: e.kind == EntryKind::Dir,
                modified: millis(e.modified),
            })
            .collect(),
    };
    let text = toml::to_string_pretty(&file)
        .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
    crate::settings::write_atomically(path, &text)
}

/// Reads the entries back, examining each path again so the rows carry
/// current ids and times; paths that are gone are dropped. A missing file
/// is an empty list.
///
/// # Errors
///
/// The I/O error, or `InvalidData` when the file does not parse.
pub fn load(path: &Path) -> Result<Vec<Entry>, Error> {
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
    let mut entries = Vec::with_capacity(file.entries.len());
    for record in file.entries.into_iter().take(RECENT_LIMIT) {
        let os_path = PathBuf::from(&record.path);
        let vpath = platform::paths::from_path(&os_path);
        if let Ok(entry) = vfs_local::entry_at(&os_path, &vpath) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

/// Walks `root` through the provider's streamed listing and returns the
/// newest [`RECENT_LIMIT`] entries by modification time, hidden entries
/// left out. Bounded by [`SCAN_CAP`] entries and [`SCAN_FOR`].
pub async fn scan(provider: &dyn Provider, root: &Location, cancel: CancelToken) -> Vec<Entry> {
    let started = Instant::now();
    let opts = ListOptions {
        hidden: false,
        system: false,
        depth: Depth::Unlimited,
    };
    let mut stream = provider.list(root, opts, cancel.clone());
    let mut newest: Vec<Entry> = Vec::new();
    let mut seen = 0u64;
    while let Some(batch) = stream.next().await {
        let Ok(batch) = batch else { break };
        seen += batch.len() as u64;
        for entry in batch {
            if entry.attrs.contains(Attrs::HIDDEN) || entry.modified.is_none() {
                continue;
            }
            let at = newest.partition_point(|e| e.modified > entry.modified);
            if at < RECENT_LIMIT {
                newest.insert(at, entry);
                newest.truncate(RECENT_LIMIT);
            }
        }
        if seen >= SCAN_CAP || started.elapsed() >= SCAN_FOR || cancel.is_cancelled() {
            cancel.cancel();
            break;
        }
    }
    newest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_round_trips_existing_paths_and_drops_the_rest() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let kept = dir.path().join("kept.txt");
        std::fs::write(&kept, "x").unwrap_or_else(|e| panic!("{e}"));
        let gone = dir.path().join("gone.txt");
        std::fs::write(&gone, "y").unwrap_or_else(|e| panic!("{e}"));
        let entries: Vec<Entry> = [&kept, &gone]
            .iter()
            .map(|p| {
                vfs_local::entry_at(p, &platform::paths::from_path(p))
                    .unwrap_or_else(|e| panic!("{e}"))
            })
            .collect();
        let file = dir.path().join("cfg").join(RECENT_FILE);
        save(&file, &entries).unwrap_or_else(|e| panic!("{e}"));
        std::fs::remove_file(&gone).unwrap_or_else(|e| panic!("{e}"));
        let loaded = load(&file).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "kept.txt");
        assert_eq!(loaded[0].kind, EntryKind::File);
        assert!(load(&dir.path().join("missing.toml")).is_ok_and(|l| l.is_empty()));
        std::fs::write(&file, "entries = 3").unwrap_or_else(|e| panic!("{e}"));
        assert!(load(&file).is_err());
    }
}
