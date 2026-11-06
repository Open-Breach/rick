//! Change notification: `notify` events translated into `FsChange`.
//!
//! `notify` reports paths; `FsChange` speaks in entries and ids. The watch
//! keeps a map from path to id, seeded from a listing on the first event, so
//! a removal can name the id of what disappeared and a rename can pair the
//! old id with the new entry. Anything it cannot translate becomes
//! `FsChange::Rescan`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use ext::{ChangeStream, FsChange};
use futures::StreamExt;
use futures::stream;
use notify::event::{ModifyKind, RenameMode};
use notify::{EventKind, RecursiveMode, Watcher};
use platform::fs::DirReader;
use tokio::sync::mpsc;
use types::{Entry, EntryId, Error, Location, VPath};

use crate::{BATCH, PROVIDER_ID, entry_from_raw, path_of};

/// Events buffered between the watcher thread and the consumer.
const CHANNEL_DEPTH: usize = 1024;

pub(crate) fn stream(loc: &Location) -> Result<ChangeStream, Error> {
    let dir = path_of(loc)?;
    std::fs::symlink_metadata(&dir).map_err(Error::from_io)?;
    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>(CHANNEL_DEPTH);
    let mut watcher = notify::recommended_watcher(move |event| {
        // The watcher thread blocks while the consumer catches up; when the
        // consumer is gone the send fails and the watcher is dropped soon.
        let _ = tx.blocking_send(event);
    })
    .map_err(notify_error)?;
    watcher
        .watch(&dir, RecursiveMode::NonRecursive)
        .map_err(notify_error)?;
    tracing::debug!(dir = %dir.display(), "watching");

    let state = State {
        _watcher: watcher,
        rx,
        dir,
        dir_vpath: loc.path.clone(),
        ids: HashMap::new(),
        seeded: false,
        pending_rename: None,
        queue: VecDeque::new(),
    };
    Ok(stream::unfold(state, |mut state| async move {
        loop {
            if let Some(change) = state.queue.pop_front() {
                return Some((change, state));
            }
            let event = state.rx.recv().await?;
            state.translate(event).await;
        }
    })
    .boxed())
}

fn notify_error(error: notify::Error) -> Error {
    match error.kind {
        notify::ErrorKind::Io(io) => Error::from_io(io),
        notify::ErrorKind::PathNotFound => Error::NotFound,
        notify::ErrorKind::MaxFilesWatch => Error::Unsupported("too many watches"),
        notify::ErrorKind::InvalidConfig(_) | notify::ErrorKind::WatchNotFound => {
            Error::Unsupported("cannot watch this location")
        }
        notify::ErrorKind::Generic(text) => Error::Io(std::io::Error::other(text)),
    }
}

struct State {
    _watcher: notify::RecommendedWatcher,
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    dir: PathBuf,
    dir_vpath: VPath,
    ids: HashMap<PathBuf, EntryId>,
    seeded: bool,
    pending_rename: Option<(PathBuf, Option<EntryId>)>,
    queue: VecDeque<FsChange>,
}

impl State {
    async fn translate(&mut self, event: notify::Result<notify::Event>) {
        if !self.seeded {
            self.seed().await;
        }
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(%error, "watch error; asking for a rescan");
                self.queue.push_back(FsChange::Rescan);
                return;
            }
        };
        if event.need_rescan() {
            self.queue.push_back(FsChange::Rescan);
            return;
        }
        let paths: Vec<PathBuf> = event
            .paths
            .iter()
            .filter(|p| p.as_path() != self.dir.as_path())
            .cloned()
            .collect();
        match event.kind {
            EventKind::Create(_) => {
                for path in paths {
                    self.created(&path).await;
                }
            }
            EventKind::Remove(_) => {
                for path in paths {
                    self.removed(&path);
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                if let Some(from) = paths.first() {
                    let id = self.ids.remove(from);
                    self.pending_rename = Some((from.clone(), id));
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                if let Some(to) = paths.first() {
                    let from = self.pending_rename.take().and_then(|(_, id)| id);
                    self.renamed(from, to).await;
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                if let [from, to] = paths.as_slice() {
                    let id = self.ids.remove(from);
                    self.renamed(id, to).await;
                }
            }
            EventKind::Modify(_) => {
                for path in paths {
                    self.modified(&path).await;
                }
            }
            EventKind::Access(_) => {}
            EventKind::Any | EventKind::Other => self.queue.push_back(FsChange::Rescan),
        }
    }

    /// Fills the path-to-id map from a listing so removals can be named.
    async fn seed(&mut self) {
        self.seeded = true;
        let dir = self.dir.clone();
        let listed = runtime::handle()
            .spawn_blocking(move || -> std::io::Result<Vec<(PathBuf, EntryId)>> {
                let mut reader = DirReader::open(&dir)?;
                let mut out = Vec::new();
                loop {
                    let batch = reader.next_batch(BATCH)?;
                    if batch.is_empty() {
                        break;
                    }
                    out.extend(
                        batch
                            .into_iter()
                            .map(|raw| (dir.join(&raw.name), EntryId::new(PROVIDER_ID, raw.id))),
                    );
                }
                Ok(out)
            })
            .await;
        match listed {
            Ok(Ok(pairs)) => self.ids.extend(pairs),
            Ok(Err(error)) => tracing::warn!(%error, "could not seed watch ids"),
            Err(join) => tracing::warn!(error = %join, "could not seed watch ids"),
        }
    }

    async fn stat(&self, path: &Path) -> Option<Entry> {
        let (dir, dir_vpath, path) = (self.dir.clone(), self.dir_vpath.clone(), path.to_path_buf());
        runtime::handle()
            .spawn_blocking(move || {
                platform::fs::stat(&path).map(|raw| entry_from_raw(&dir, &dir_vpath, raw))
            })
            .await
            .ok()?
            .ok()
    }

    async fn created(&mut self, path: &Path) {
        if let Some(entry) = self.stat(path).await {
            self.ids.insert(path.to_path_buf(), entry.id);
            self.queue.push_back(FsChange::Created(entry));
        }
    }

    fn removed(&mut self, path: &Path) {
        match self.ids.remove(path) {
            Some(id) => self.queue.push_back(FsChange::Removed(id)),
            None => self.queue.push_back(FsChange::Rescan),
        }
    }

    async fn modified(&mut self, path: &Path) {
        match (self.stat(path).await, self.ids.get(path).copied()) {
            (Some(entry), Some(_)) => {
                self.ids.insert(path.to_path_buf(), entry.id);
                self.queue.push_back(FsChange::Modified(entry));
            }
            (Some(entry), None) => {
                self.ids.insert(path.to_path_buf(), entry.id);
                self.queue.push_back(FsChange::Created(entry));
            }
            (None, Some(id)) => {
                self.ids.remove(path);
                self.queue.push_back(FsChange::Removed(id));
            }
            (None, None) => {}
        }
    }

    async fn renamed(&mut self, from: Option<EntryId>, to: &Path) {
        let Some(entry) = self.stat(to).await else {
            if let Some(id) = from {
                self.queue.push_back(FsChange::Removed(id));
            }
            return;
        };
        self.ids.insert(to.to_path_buf(), entry.id);
        match from {
            Some(from) => self.queue.push_back(FsChange::Renamed { from, to: entry }),
            None => self.queue.push_back(FsChange::Created(entry)),
        }
    }
}
