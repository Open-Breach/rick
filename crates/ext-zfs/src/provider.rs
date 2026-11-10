//! The read-only provider `zfs`: the datasets with snapshots, each
//! dataset's snapshots, each snapshot's file tree, and the versions of one
//! file.
//!
//! | Path | Lists |
//! | --- | --- |
//! | `/` | the mounted datasets that have snapshots |
//! | `/<dataset>` | that dataset's snapshots, newest first |
//! | `/<dataset>/@<snapshot>/<rel>` | `<mountpoint>/.zfs/snapshot/<snapshot>/<rel>` |
//! | `/~versions/<live path>` | the versions of one file |
//!
//! The snapshot is the first path component that starts with `@` (dataset
//! names cannot hold one), so the parent of a snapshot is its dataset.
//! Nothing can be changed here: the capabilities are empty and every write
//! is `Unsupported`.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use ext::platform::fs::{DirReader, RawEntry, RawKind};
use ext::platform::paths;
use ext::{
    Capabilities, ChangeStream, CopyOptions, Depth, EntryStream, LinkKind, ListOptions,
    ProgressSink, Provider, ReadHandle, RemoveMode, WriteHandle, WriteOptions, async_trait,
};
use futures::{StreamExt, stream};
use tokio::sync::mpsc;
use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, Location, ProviderId, VPath};

use crate::datasets::Datasets;
use crate::parse::Snapshot;

/// The id of the provider's locations: `zfs:/tank/home/@b/docs`.
pub const PROVIDER_ID: ProviderId = ProviderId("zfs");

/// The first component of a versions location.
pub const VERSIONS: &str = "~versions";

/// Entries per batch of a snapshot listing.
const BATCH: usize = 256;
/// Batches buffered between the reader and a slow consumer.
const CHANNEL_DEPTH: usize = 4;

/// What a location of the provider stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `/`: the datasets that have snapshots.
    Root,
    /// `/<dataset>`: its snapshots.
    Dataset(String),
    /// `/<dataset>/@<snapshot>/<rel>`: a path inside a snapshot; `rel` is
    /// empty for the snapshot itself.
    Snapshot {
        /// The dataset's name.
        dataset: String,
        /// The snapshot's name, without `@`.
        snapshot: String,
        /// The path below the snapshot's root.
        rel: PathBuf,
    },
    /// `/~versions/<live path>`: the versions of the file at that live
    /// path.
    Versions(PathBuf),
}

/// Reads a location of the provider.
///
/// # Errors
///
/// `Error::Unsupported` for another provider's location; `Error::NotFound`
/// for a path that names nothing (an `@` without a name, a snapshot with
/// no dataset, a versions location without a path, a name that is not
/// UTF-8 where ZFS names must be).
pub fn target(loc: &Location) -> Result<Target, Error> {
    if loc.provider != PROVIDER_ID {
        return Err(Error::Unsupported("location belongs to another provider"));
    }
    let parts: Vec<&[u8]> = loc
        .path
        .as_bytes()
        .split(|b| *b == b'/')
        .filter(|p| !p.is_empty())
        .collect();
    let Some(first) = parts.first() else {
        return Ok(Target::Root);
    };
    if *first == VERSIONS.as_bytes() {
        if parts.len() == 1 {
            return Err(Error::NotFound);
        }
        return Ok(Target::Versions(join_path(&parts[1..], true)?));
    }
    let text = |part: &[u8]| {
        std::str::from_utf8(part)
            .map(str::to_owned)
            .map_err(|_| Error::NotFound)
    };
    let Some(at) = parts.iter().position(|p| p.first() == Some(&b'@')) else {
        let names: Result<Vec<String>, Error> = parts.iter().map(|p| text(p)).collect();
        return Ok(Target::Dataset(names?.join("/")));
    };
    let snapshot = text(&parts[at][1..])?;
    if at == 0 || snapshot.is_empty() {
        return Err(Error::NotFound);
    }
    let dataset: Result<Vec<String>, Error> = parts[..at].iter().map(|p| text(p)).collect();
    Ok(Target::Snapshot {
        dataset: dataset?.join("/"),
        snapshot,
        rel: join_path(&parts[at + 1..], false)?,
    })
}

/// The OS path of `parts` joined, absolute when asked.
fn join_path(parts: &[&[u8]], absolute: bool) -> Result<PathBuf, Error> {
    let mut bytes = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if absolute || i > 0 {
            bytes.push(b'/');
        }
        bytes.extend_from_slice(part);
    }
    paths::to_path(&VPath::from_bytes(bytes))
}

/// The location listing the snapshots of `dataset`.
#[must_use]
pub fn dataset_location(dataset: &str) -> Location {
    Location::new(PROVIDER_ID, format!("/{dataset}").as_str())
}

/// The location of `rel` inside the snapshot `snapshot` of `dataset`.
#[must_use]
pub fn snapshot_location(dataset: &str, snapshot: &str, rel: &Path) -> Location {
    let mut path = VPath::from(format!("/{dataset}/@{snapshot}").as_str());
    for part in rel.components() {
        if let std::path::Component::Normal(name) = part {
            path = path.join(name.as_encoded_bytes());
        }
    }
    Location::new(PROVIDER_ID, path)
}

/// The location listing the versions of the file at the live path `live`.
#[must_use]
pub fn versions_location(live: &Path) -> Location {
    let mut bytes = format!("/{VERSIONS}").into_bytes();
    let live = paths::from_path(live);
    if live.as_bytes().first() != Some(&b'/') {
        bytes.push(b'/');
    }
    bytes.extend_from_slice(live.as_bytes());
    Location::new(PROVIDER_ID, VPath::from_bytes(bytes))
}

/// FNV-1a over `bytes`, in the upper half of the key space so that no
/// device-and-inode key (a real device is never 0) can equal it.
fn synthetic_id(bytes: &[u8]) -> EntryId {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    EntryId::new(PROVIDER_ID, (1u128 << 127) | u128::from(hash))
}

/// A folder the provider makes up: the root, a dataset, a snapshot.
fn folder(name: String, path: VPath, modified: Option<SystemTime>, size: Option<u64>) -> Entry {
    Entry {
        id: synthetic_id(path.as_bytes()),
        name,
        path,
        kind: EntryKind::Dir,
        size,
        created: None,
        modified,
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    }
}

/// The entry of a snapshot at the dataset level.
#[must_use]
pub fn snapshot_entry(snapshot: &Snapshot) -> Entry {
    let loc = snapshot_location(&snapshot.dataset, &snapshot.name, Path::new(""));
    let mut entry = folder(
        snapshot.name.clone(),
        loc.path,
        Some(snapshot.created),
        Some(snapshot.used),
    );
    entry.created = Some(snapshot.created);
    entry
}

/// An entry read from inside a snapshot: `raw` lies in `dir`, its path in
/// the provider is `vpath`.
pub(crate) fn entry_from_raw(dir: &Path, vpath: VPath, raw: RawEntry) -> Entry {
    let RawEntry {
        name,
        id,
        kind,
        size,
        created,
        modified,
        accessed,
        attrs,
    } = raw;
    let full = dir.join(&name);
    let kind = match kind {
        RawKind::File => EntryKind::File,
        RawKind::Dir => EntryKind::Dir,
        RawKind::Other => EntryKind::Other,
        RawKind::Symlink => EntryKind::Symlink {
            target: std::fs::read_link(&full)
                .map(|t| paths::from_path(&t))
                .unwrap_or_default(),
            broken: std::fs::metadata(&full).is_err(),
        },
    };
    Entry {
        id: EntryId::new(PROVIDER_ID, id),
        name: name.to_string_lossy().into_owned(),
        path: vpath,
        kind,
        size,
        created,
        modified,
        accessed,
        attrs,
        mime: None,
    }
}

/// The entry at `path` inside a snapshot, as the provider names it.
fn stat_path(path: &Path, vpath: VPath) -> Result<Entry, Error> {
    let raw = ext::platform::fs::stat(path).map_err(Error::from_io)?;
    let dir = path.parent().unwrap_or(path).to_path_buf();
    Ok(entry_from_raw(&dir, vpath, raw))
}

/// The snapshots of the provider, read through the shared dataset cache.
#[derive(Debug)]
pub struct ZfsProvider {
    datasets: Arc<Datasets>,
}

impl ZfsProvider {
    /// A provider over `datasets`.
    #[must_use]
    pub fn new(datasets: Arc<Datasets>) -> Self {
        Self { datasets }
    }

    /// The dataset cache the provider reads.
    #[must_use]
    pub fn datasets(&self) -> &Arc<Datasets> {
        &self.datasets
    }

    /// The OS path a snapshot location stands for: under the dataset's
    /// mount point, `.zfs/snapshot/<snapshot>/<rel>`.
    ///
    /// # Errors
    ///
    /// `Error::NotFound` when the dataset is not mounted.
    pub fn os_path(&self, dataset: &str, snapshot: &str, rel: &Path) -> Result<PathBuf, Error> {
        let fs = self.datasets.named(dataset).ok_or(Error::NotFound)?;
        let mut path = Datasets::snapshot_dir(fs, snapshot);
        if !rel.as_os_str().is_empty() {
            path.push(rel);
        }
        Ok(path)
    }
}

/// The snapshot `snapshot` of `dataset`, as its (cached) list knows it.
fn find_snapshot(datasets: &Datasets, dataset: &str, snapshot: &str) -> Result<Snapshot, Error> {
    datasets
        .snapshots(dataset)?
        .into_iter()
        .find(|s| s.name == snapshot)
        .ok_or(Error::NotFound)
}

/// Streams the entries of the directory `root` (and below, for
/// `Depth::Unlimited`) from the blocking pool.
fn walk_stream(root: PathBuf, vpath: VPath, opts: ListOptions, cancel: CancelToken) -> EntryStream {
    let (tx, rx) = mpsc::channel::<Result<Vec<Entry>, Error>>(CHANNEL_DEPTH);
    let walker_cancel = cancel.clone();
    ext::runtime::handle().spawn_blocking(move || walk(root, vpath, opts, &walker_cancel, &tx));
    receive(rx, cancel)
}

/// The stream over what a blocking producer sends; batches still buffered
/// when the token fires are dropped.
pub(crate) fn receive(
    rx: mpsc::Receiver<Result<Vec<Entry>, Error>>,
    cancel: CancelToken,
) -> EntryStream {
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
                tracing::debug!(dir = %dir.display(), error = %e, "skipping unreadable directory");
                continue;
            }
        };
        is_root = false;
        loop {
            if cancel.is_cancelled() {
                return;
            }
            let raw = match reader.next_batch(BATCH) {
                Ok(raw) if raw.is_empty() => break,
                Ok(raw) => raw,
                Err(e) => {
                    let _ = tx.blocking_send(Err(Error::from_io(e)));
                    return;
                }
            };
            let mut batch = Vec::with_capacity(raw.len());
            for raw in raw {
                if (!opts.hidden && raw.attrs.contains(Attrs::HIDDEN))
                    || (!opts.system && raw.attrs.contains(Attrs::SYSTEM))
                {
                    continue;
                }
                let vpath = dir_vpath.join(raw.name.as_encoded_bytes());
                let child = (opts.depth == Depth::Unlimited).then(|| dir.join(&raw.name));
                let entry = entry_from_raw(&dir, vpath, raw);
                if let Some(child) = child
                    && entry.kind == EntryKind::Dir
                {
                    pending.push_back((child, entry.path.clone()));
                }
                batch.push(entry);
            }
            if !batch.is_empty() && tx.blocking_send(Ok(batch)).is_err() {
                return;
            }
        }
    }
}

/// A stream of one batch computed on the blocking pool.
fn once_blocking<F>(cancel: CancelToken, work: F) -> EntryStream
where
    F: FnOnce() -> Result<Vec<Entry>, Error> + Send + 'static,
{
    stream::once(async move {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        ext::blocking(work).await
    })
    .boxed()
}

const READ_ONLY: &str = "snapshots are read-only";

#[async_trait]
impl Provider for ZfsProvider {
    fn id(&self) -> ProviderId {
        PROVIDER_ID
    }

    /// Nothing can be written, renamed, trashed or linked anywhere here.
    fn capabilities(&self, _loc: &Location) -> Capabilities {
        Capabilities::empty()
    }

    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken) -> EntryStream {
        let target = match target(loc) {
            Ok(target) => target,
            Err(error) => return stream::once(async move { Err(error) }).boxed(),
        };
        let datasets = Arc::clone(&self.datasets);
        match target {
            Target::Root => once_blocking(cancel, move || {
                Ok(datasets
                    .with_snapshots()
                    .into_iter()
                    .map(|(fs, snapshots)| {
                        let loc = dataset_location(&fs.name);
                        let newest = snapshots.first().map(|s| s.created);
                        folder(fs.name, loc.path, newest, None)
                    })
                    .collect())
            }),
            Target::Dataset(name) => once_blocking(cancel, move || {
                datasets.named(&name).ok_or(Error::NotFound)?;
                Ok(datasets
                    .snapshots(&name)?
                    .iter()
                    .map(snapshot_entry)
                    .collect())
            }),
            Target::Snapshot {
                dataset,
                snapshot,
                rel,
            } => match self.os_path(&dataset, &snapshot, &rel) {
                Ok(dir) => walk_stream(dir, loc.path.clone(), opts, cancel),
                Err(error) => stream::once(async move { Err(error) }).boxed(),
            },
            Target::Versions(live) => crate::versions::stream(datasets, live, cancel),
        }
    }

    async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
        match target(loc)? {
            Target::Root => Ok(folder("ZFS".to_owned(), VPath::from("/"), None, None)),
            Target::Dataset(name) => {
                let datasets = Arc::clone(&self.datasets);
                let path = loc.path.clone();
                ext::blocking(move || {
                    datasets.named(&name).ok_or(Error::NotFound)?;
                    let newest = datasets.snapshots(&name)?.first().map(|s| s.created);
                    Ok(folder(name, path, newest, None))
                })
                .await
            }
            Target::Snapshot {
                dataset,
                snapshot,
                rel,
            } => {
                if rel.as_os_str().is_empty() {
                    let datasets = Arc::clone(&self.datasets);
                    let found =
                        ext::blocking(move || find_snapshot(&datasets, &dataset, &snapshot))
                            .await?;
                    return Ok(snapshot_entry(&found));
                }
                let path = self.os_path(&dataset, &snapshot, &rel)?;
                let vpath = loc.path.clone();
                ext::blocking(move || stat_path(&path, vpath)).await
            }
            Target::Versions(live) => {
                let name = live
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Ok(folder(
                    ext::t!("zfs-versions-of", name = name),
                    loc.path.clone(),
                    None,
                    None,
                ))
            }
        }
    }

    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
        let Target::Snapshot {
            dataset,
            snapshot,
            rel,
        } = target(loc)?
        else {
            return Err(Error::Unsupported("only files in a snapshot can be read"));
        };
        let path = self.os_path(&dataset, &snapshot, &rel)?;
        let file = tokio::fs::File::open(path).await.map_err(Error::from_io)?;
        Ok(Box::new(file))
    }

    async fn open_write(&self, _loc: &Location, _opts: WriteOptions) -> Result<WriteHandle, Error> {
        Err(Error::Unsupported(READ_ONLY))
    }

    async fn create_dir(&self, _loc: &Location) -> Result<Entry, Error> {
        Err(Error::Unsupported(READ_ONLY))
    }

    async fn rename(&self, _from: &Location, _to: &Location) -> Result<Entry, Error> {
        Err(Error::Unsupported(READ_ONLY))
    }

    async fn remove(&self, _loc: &Location, _mode: RemoveMode) -> Result<(), Error> {
        Err(Error::Unsupported(READ_ONLY))
    }

    async fn link(
        &self,
        _target: &Location,
        _at: &Location,
        _kind: LinkKind,
    ) -> Result<Entry, Error> {
        Err(Error::Unsupported(READ_ONLY))
    }

    async fn copy(
        &self,
        _from: &Location,
        _to: &Location,
        _opts: CopyOptions,
        _progress: ProgressSink,
        _cancel: CancelToken,
    ) -> Result<(), Error> {
        Err(Error::Unsupported(READ_ONLY))
    }

    fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
        Err(Error::Unsupported(
            "snapshots do not change; the lists are re-read",
        ))
    }
}
