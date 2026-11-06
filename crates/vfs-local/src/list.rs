//! Listing: the bulk reader on the blocking pool, batches over a channel.

use std::collections::VecDeque;
use std::path::PathBuf;

use ext::{Depth, EntryStream, ListOptions};
use futures::StreamExt;
use futures::stream;
use platform::fs::DirReader;
use tokio::sync::mpsc;
use types::{Attrs, CancelToken, Entry, EntryKind, Error, Location, VPath};

use crate::{BATCH, entry_from_raw, path_of};

/// Batches buffered between the reader and a slow consumer.
const CHANNEL_DEPTH: usize = 4;

pub(crate) fn stream(loc: &Location, opts: ListOptions, cancel: CancelToken) -> EntryStream {
    let root = match path_of(loc) {
        Ok(root) => root,
        Err(e) => return stream::once(async move { Err(e) }).boxed(),
    };
    let root_vpath = loc.path.clone();
    let (tx, rx) = mpsc::channel::<Result<Vec<Entry>, Error>>(CHANNEL_DEPTH);

    let walker_cancel = cancel.clone();
    runtime::handle().spawn_blocking(move || {
        let span = tracing::info_span!("vfs_local.list", path = %root.display());
        let _enter = span.enter();
        walk(root, root_vpath, opts, &walker_cancel, &tx);
    });

    // Batches already buffered in the channel are dropped once cancelled, so
    // the consumer sees at most the batch it was awaiting.
    stream::unfold((rx, cancel), |(mut rx, cancel)| async move {
        if cancel.is_cancelled() {
            return None;
        }
        rx.recv().await.map(|item| (item, (rx, cancel)))
    })
    .boxed()
}

fn walk(
    root: PathBuf,
    root_vpath: VPath,
    opts: ListOptions,
    cancel: &CancelToken,
    tx: &mpsc::Sender<Result<Vec<Entry>, Error>>,
) {
    let mut pending: VecDeque<(PathBuf, VPath)> = VecDeque::from([(root, root_vpath)]);
    let mut is_root = true;
    while let Some((dir, dir_vpath)) = pending.pop_front() {
        let mut reader = match DirReader::open(&dir) {
            Ok(reader) => reader,
            Err(e) if is_root => {
                let _ = tx.blocking_send(Err(Error::from_io(e)));
                return;
            }
            Err(e) => {
                tracing::warn!(dir = %dir.display(), error = %e, "skipping unreadable directory");
                continue;
            }
        };
        is_root = false;
        loop {
            if cancel.is_cancelled() {
                tracing::debug!("listing cancelled");
                return;
            }
            let raw = match reader.next_batch(BATCH) {
                Ok(raw) => raw,
                Err(e) => {
                    let _ = tx.blocking_send(Err(Error::from_io(e)));
                    return;
                }
            };
            if raw.is_empty() {
                break;
            }
            let mut batch = Vec::with_capacity(raw.len());
            for entry in raw {
                if (!opts.hidden && entry.attrs.contains(Attrs::HIDDEN))
                    || (!opts.system && entry.attrs.contains(Attrs::SYSTEM))
                {
                    continue;
                }
                let child_dir = (opts.depth == Depth::Unlimited).then(|| dir.join(&entry.name));
                let entry = entry_from_raw(&dir, &dir_vpath, entry);
                if let Some(child_dir) = child_dir
                    && entry.kind == EntryKind::Dir
                {
                    pending.push_back((child_dir, entry.path.clone()));
                }
                batch.push(entry);
            }
            if !batch.is_empty() && tx.blocking_send(Ok(batch)).is_err() {
                // The consumer went away.
                return;
            }
        }
    }
}
