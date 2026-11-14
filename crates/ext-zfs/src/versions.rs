//! The versions of one file: `/~versions/<live path>`.
//!
//! The snapshots of the file's dataset are walked newest first and the file
//! is looked up in each; a snapshot is listed when it holds the file and its
//! size or modification time differs from the version listed before it, so
//! runs of unchanged copies collapse into the newest one. Each entry's path
//! is the file's location in that snapshot, so the preview, copy and
//! Restore treat it like any file in a snapshot; its name is
//! `<snapshot> — <file name>`. Every look-up may mount a snapshot, so the
//! walk runs on the blocking pool, sends what it found in batches and stops
//! when the token fires.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use ext::EntryStream;
use tokio::sync::mpsc;
use types::{CancelToken, Entry, Error};

use crate::datasets::Datasets;
use crate::provider::{entry_from_raw, receive, snapshot_location};

/// Snapshots looked up between two batches.
const STEP: usize = 16;

/// The versions of the file at the live path `live`, streamed.
pub(crate) fn stream(datasets: Arc<Datasets>, live: PathBuf, cancel: CancelToken) -> EntryStream {
    let (tx, rx) = mpsc::channel::<Result<Vec<Entry>, Error>>(4);
    let walker_cancel = cancel.clone();
    ext::runtime::handle().spawn_blocking(move || {
        if let Err(error) = walk(&datasets, &live, &walker_cancel, &mut |batch| {
            tx.blocking_send(Ok(batch)).is_ok()
        }) {
            let _ = tx.blocking_send(Err(error));
        }
    });
    receive(rx, cancel)
}

/// Walks the snapshots and hands each batch of versions to `send`, which
/// answers whether to go on.
///
/// # Errors
///
/// `Error::NotFound` when `live` lies in no dataset; the error listing the
/// snapshots gave.
pub(crate) fn walk(
    datasets: &Datasets,
    live: &Path,
    cancel: &CancelToken,
    send: &mut dyn FnMut(Vec<Entry>) -> bool,
) -> Result<(), Error> {
    let (fs, rel) = datasets.split(live).ok_or(Error::NotFound)?;
    let fs = fs.clone();
    if rel.as_os_str().is_empty() {
        return Ok(());
    }
    let file_name = live
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut last: Option<(Option<u64>, Option<SystemTime>)> = None;
    let mut batch = Vec::new();
    for (i, snapshot) in datasets.snapshots(&fs.name)?.iter().enumerate() {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let path = Datasets::snapshot_dir(&fs, &snapshot.name).join(&rel);
        if let Ok(raw) = ext::platform::fs::stat(&path) {
            let key = (raw.size, raw.modified);
            if last != Some(key) {
                last = Some(key);
                let loc = snapshot_location(&fs.name, &snapshot.name, &rel);
                let dir = path.parent().unwrap_or(&path).to_path_buf();
                let mut entry = entry_from_raw(&dir, loc.path, raw);
                entry.name = format!("{} — {file_name}", snapshot.name);
                batch.push(entry);
            }
        }
        if (i + 1) % STEP == 0 && !batch.is_empty() && !send(std::mem::take(&mut batch)) {
            return Ok(());
        }
    }
    if !batch.is_empty() {
        send(batch);
    }
    Ok(())
}
