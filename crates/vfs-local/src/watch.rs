//! Change notification: `notify` events translated into `FsChange`.
//!
//! `notify` reports paths; `FsChange` speaks in entries and ids. The watch
//! keeps a map from path to id, seeded from a listing on the first event, so
//! a removal can name the id of what disappeared and a rename can pair the
//! old id with the new entry. Anything it cannot translate becomes
//! `FsChange::Rescan`.
//!
//! The OS may spell the watched directory differently from how it was
//! given: `FSEvents` reports canonical paths (`/private/var/…` for
//! `/var/…`), and may report the directory itself. Every reported path is
//! therefore reduced to a child name under one of the directory's known
//! spellings, and keyed under the directory as given.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
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

    // The canonical spelling is what FSEvents reports; the given one is
    // what inotify and ReadDirectoryChangesW report.
    let mut aliases = vec![dir.clone()];
    if let Ok(canonical) = std::fs::canonicalize(&dir)
        && canonical != dir
    {
        aliases.push(canonical);
    }
    let state = State::new(watcher, rx, dir, loc.path.clone(), aliases);
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
    /// The watched directory as given; ids are keyed under it.
    dir: PathBuf,
    /// Every spelling of `dir` the OS may report, `dir` itself first.
    aliases: Vec<PathBuf>,
    dir_vpath: VPath,
    ids: HashMap<PathBuf, EntryId>,
    seeded: bool,
    pending_rename: Option<(PathBuf, Option<EntryId>)>,
    queue: VecDeque<FsChange>,
}

/// A reported path reduced to the watched directory's child.
struct Child {
    /// `dir` joined with the name: the key in the id map.
    key: PathBuf,
    /// The path as reported, for `stat`.
    reported: PathBuf,
}

impl State {
    fn new(
        watcher: notify::RecommendedWatcher,
        rx: mpsc::Receiver<notify::Result<notify::Event>>,
        dir: PathBuf,
        dir_vpath: VPath,
        aliases: Vec<PathBuf>,
    ) -> Self {
        Self {
            _watcher: watcher,
            rx,
            dir,
            aliases,
            dir_vpath,
            ids: HashMap::new(),
            seeded: false,
            pending_rename: None,
            queue: VecDeque::new(),
        }
    }

    /// The name of a direct child of the watched directory, however the OS
    /// spelt the directory; `None` for the directory itself or anything
    /// outside it.
    fn child_name(&self, path: &Path) -> Option<OsString> {
        let parent = path.parent()?;
        if !self.aliases.iter().any(|alias| alias == parent) {
            return None;
        }
        path.file_name().map(ToOwned::to_owned)
    }

    fn child(&self, path: &Path) -> Option<Child> {
        let name = self.child_name(path)?;
        Some(Child {
            key: self.dir.join(name),
            reported: path.to_path_buf(),
        })
    }

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
        let paths: Vec<Child> = event.paths.iter().filter_map(|p| self.child(p)).collect();
        match event.kind {
            EventKind::Create(_) => {
                for path in paths {
                    self.created(&path).await;
                }
            }
            EventKind::Remove(_) => {
                for path in paths {
                    self.removed(&path.key);
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                if let Some(from) = paths.first() {
                    let id = self.ids.remove(&from.key);
                    self.pending_rename = Some((from.key.clone(), id));
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
                    let id = self.ids.remove(&from.key);
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

    async fn created(&mut self, path: &Child) {
        if let Some(entry) = self.stat(&path.reported).await {
            self.ids.insert(path.key.clone(), entry.id);
            self.queue.push_back(FsChange::Created(entry));
        }
    }

    fn removed(&mut self, key: &Path) {
        match self.ids.remove(key) {
            Some(id) => self.queue.push_back(FsChange::Removed(id)),
            None => self.queue.push_back(FsChange::Rescan),
        }
    }

    async fn modified(&mut self, path: &Child) {
        match (
            self.stat(&path.reported).await,
            self.ids.get(&path.key).copied(),
        ) {
            (Some(entry), Some(_)) => {
                self.ids.insert(path.key.clone(), entry.id);
                self.queue.push_back(FsChange::Modified(entry));
            }
            (Some(entry), None) => {
                self.ids.insert(path.key.clone(), entry.id);
                self.queue.push_back(FsChange::Created(entry));
            }
            (None, Some(id)) => {
                self.ids.remove(&path.key);
                self.queue.push_back(FsChange::Removed(id));
            }
            (None, None) => {}
        }
    }

    async fn renamed(&mut self, from: Option<EntryId>, to: &Child) {
        let Some(entry) = self.stat(&to.reported).await else {
            if let Some(id) = from {
                self.queue.push_back(FsChange::Removed(id));
            }
            return;
        };
        self.ids.insert(to.key.clone(), entry.id);
        match from {
            Some(from) => self.queue.push_back(FsChange::Renamed { from, to: entry }),
            None => self.queue.push_back(FsChange::Created(entry)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use notify::event::{CreateKind, RemoveKind};

    use super::*;

    /// Events may name the directory the way the OS spells it, not the way
    /// it was watched; `FSEvents` does, and also reports the directory itself.
    #[test]
    fn events_spelt_with_another_form_of_the_directory_map_to_its_children() {
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let real = tmp.path().join("real");
        fs::create_dir(&real).unwrap_or_else(|e| panic!("{e}"));
        fs::write(real.join("a.txt"), "a").unwrap_or_else(|e| panic!("{e}"));
        // How the directory was asked for; it need not exist for this test,
        // because every reported path uses the other spelling.
        let watched = tmp.path().join("watched");

        let (_tx, rx) = mpsc::channel(1);
        let noop = notify::recommended_watcher(|_| {}).unwrap_or_else(|e| panic!("{e}"));
        let mut state = State::new(
            noop,
            rx,
            watched.clone(),
            VPath::from(watched.as_path()),
            vec![watched.clone(), real.clone()],
        );
        state.seeded = true;

        runtime::block_on(async {
            let dir_itself =
                notify::Event::new(EventKind::Create(CreateKind::Folder)).add_path(real.clone());
            state.translate(Ok(dir_itself)).await;
            assert_eq!(
                state.queue,
                VecDeque::new(),
                "the directory is not its own child"
            );

            let child = notify::Event::new(EventKind::Create(CreateKind::File))
                .add_path(real.join("a.txt"));
            state.translate(Ok(child)).await;
            let id = match state.queue.pop_front() {
                Some(FsChange::Created(entry)) => {
                    assert_eq!(entry.name, "a.txt");
                    assert_eq!(
                        entry.path,
                        VPath::from(watched.as_path()).join("a.txt"),
                        "the entry is spelt the way the directory was watched"
                    );
                    entry.id
                }
                other => panic!("{other:?}"),
            };
            assert_eq!(state.ids.get(&watched.join("a.txt")), Some(&id));

            let gone = notify::Event::new(EventKind::Remove(RemoveKind::File))
                .add_path(real.join("a.txt"));
            state.translate(Ok(gone)).await;
            assert_eq!(state.queue.pop_front(), Some(FsChange::Removed(id)));
            assert_eq!(state.queue, VecDeque::new());
        });
    }
}
