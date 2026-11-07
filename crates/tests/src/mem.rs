//! An in-memory [`Provider`] for the integration tests: a tree of folders
//! and files under any provider id, with a settable delay per copied file,
//! so a test can mount a stand-in for a cloud provider (`s3`, `azure`) and
//! watch copies on disjoint volumes run side by side.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};

use ext::{
    API_VERSION, Capabilities, ChangeStream, CopyOptions, EntryStream, Extension, LinkKind,
    ListOptions, Manifest, Provider, ReadHandle, Registry, RemoveMode, Version, WriteHandle,
    WriteOptions, async_trait,
};
use futures::StreamExt;
use futures::stream;
use runtime::ProgressSink;
use types::{
    Attrs, CancelToken, Entry, EntryId, EntryKind, Error, ExtId, Location, Progress, ProviderId,
    VPath,
};

/// A node of the tree.
#[derive(Debug, Clone)]
enum Node {
    Dir,
    File(Vec<u8>),
}

/// The tree, by absolute path with `/` separators.
type Tree = BTreeMap<String, Node>;

/// The in-memory provider.
pub struct MemProvider {
    id: ProviderId,
    tree: Arc<Mutex<Tree>>,
    copy_delay: Duration,
}

impl std::fmt::Debug for MemProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemProvider")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

fn lock(tree: &Mutex<Tree>) -> MutexGuard<'_, Tree> {
    tree.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `/a/b` → `/a`; `/a` → `/`.
fn parent(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    }
}

fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn key(loc: &Location) -> String {
    let text = loc.path.to_string_lossy().into_owned();
    let trimmed = text.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

impl MemProvider {
    /// An empty tree under `id` whose copies take `copy_delay` per file.
    #[must_use]
    pub fn new(id: ProviderId, copy_delay: Duration) -> Self {
        let mut tree = BTreeMap::new();
        tree.insert("/".to_owned(), Node::Dir);
        Self {
            id,
            tree: Arc::new(Mutex::new(tree)),
            copy_delay,
        }
    }

    /// Adds a folder, its parents included.
    pub fn dir(&self, path: &str) {
        let mut tree = lock(&self.tree);
        let mut current = String::new();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            current.push('/');
            current.push_str(part);
            tree.entry(current.clone()).or_insert(Node::Dir);
        }
    }

    /// Adds a file with `bytes`, its parents included.
    pub fn file(&self, path: &str, bytes: &[u8]) {
        self.dir(parent(path));
        lock(&self.tree).insert(path.to_owned(), Node::File(bytes.to_vec()));
    }

    /// The paths in the tree, sorted.
    #[must_use]
    pub fn paths(&self) -> Vec<String> {
        lock(&self.tree).keys().cloned().collect()
    }

    fn entry(&self, path: &str, node: &Node) -> Entry {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(path, &mut hasher);
        let key = u128::from(std::hash::Hasher::finish(&hasher));
        Entry {
            id: EntryId::new(self.id, key),
            name: if path == "/" {
                self.id.0.to_owned()
            } else {
                name(path).to_owned()
            },
            path: VPath::from(path),
            kind: match node {
                Node::Dir => EntryKind::Dir,
                Node::File(_) => EntryKind::File,
            },
            size: match node {
                Node::Dir => None,
                Node::File(bytes) => Some(bytes.len() as u64),
            },
            created: None,
            modified: Some(SystemTime::UNIX_EPOCH),
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        }
    }
}

/// Collects what is written and stores it on shutdown.
struct MemWriter {
    tree: Arc<Mutex<Tree>>,
    path: String,
    bytes: Vec<u8>,
}

impl tokio::io::AsyncWrite for MemWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.bytes.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let bytes = std::mem::take(&mut self.bytes);
        let path = self.path.clone();
        lock(&self.tree).insert(path, Node::File(bytes));
        Poll::Ready(Ok(()))
    }
}

#[async_trait]
impl Provider for MemProvider {
    fn id(&self) -> ProviderId {
        self.id
    }

    fn capabilities(&self, _loc: &Location) -> Capabilities {
        Capabilities::WRITE | Capabilities::RENAME
    }

    fn list(&self, loc: &Location, _opts: ListOptions, _cancel: CancelToken) -> EntryStream {
        let dir = key(loc);
        let tree = lock(&self.tree);
        if !matches!(tree.get(&dir), Some(Node::Dir)) {
            return stream::once(async { Err(Error::NotFound) }).boxed();
        }
        let entries: Vec<Entry> = tree
            .iter()
            .filter(|(path, _)| path.as_str() != "/" && parent(path) == dir)
            .map(|(path, node)| self.entry(path, node))
            .collect();
        stream::once(async move { Ok(entries) }).boxed()
    }

    async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
        let path = key(loc);
        let tree = lock(&self.tree);
        tree.get(&path)
            .map(|node| self.entry(&path, node))
            .ok_or(Error::NotFound)
    }

    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
        let path = key(loc);
        match lock(&self.tree).get(&path) {
            Some(Node::File(bytes)) => Ok(Box::new(std::io::Cursor::new(bytes.clone()))),
            Some(Node::Dir) => Err(Error::Unsupported("a folder cannot be read")),
            None => Err(Error::NotFound),
        }
    }

    async fn open_write(&self, loc: &Location, opts: WriteOptions) -> Result<WriteHandle, Error> {
        let path = key(loc);
        if !opts.overwrite && lock(&self.tree).contains_key(&path) {
            return Err(Error::Io(std::io::Error::from(
                std::io::ErrorKind::AlreadyExists,
            )));
        }
        Ok(Box::new(MemWriter {
            tree: Arc::clone(&self.tree),
            path,
            bytes: Vec::new(),
        }))
    }

    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error> {
        let path = key(loc);
        {
            let mut tree = lock(&self.tree);
            if tree.contains_key(&path) {
                return Err(Error::Io(std::io::Error::from(
                    std::io::ErrorKind::AlreadyExists,
                )));
            }
            tree.insert(path.clone(), Node::Dir);
        }
        self.stat(loc).await
    }

    async fn rename(&self, from: &Location, to: &Location) -> Result<Entry, Error> {
        let (from_key, to_key) = (key(from), key(to));
        {
            let mut tree = lock(&self.tree);
            let moved: Vec<(String, Node)> = tree
                .iter()
                .filter(|(p, _)| **p == from_key || p.starts_with(&format!("{from_key}/")))
                .map(|(p, n)| (p.clone(), n.clone()))
                .collect();
            if moved.is_empty() {
                return Err(Error::NotFound);
            }
            for (p, _) in &moved {
                tree.remove(p);
            }
            for (p, n) in moved {
                let rest = &p[from_key.len()..];
                tree.insert(format!("{to_key}{rest}"), n);
            }
        }
        self.stat(to).await
    }

    async fn remove(&self, loc: &Location, _mode: RemoveMode) -> Result<(), Error> {
        let path = key(loc);
        let mut tree = lock(&self.tree);
        let before = tree.len();
        tree.retain(|p, _| *p != path && !p.starts_with(&format!("{path}/")));
        if tree.len() == before {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    async fn link(
        &self,
        _target: &Location,
        _at: &Location,
        _kind: LinkKind,
    ) -> Result<Entry, Error> {
        Err(Error::Unsupported("no links in memory"))
    }

    /// Copies one file after the configured delay, reporting its bytes.
    async fn copy(
        &self,
        from: &Location,
        to: &Location,
        opts: CopyOptions,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        let bytes = match lock(&self.tree).get(&key(from)) {
            Some(Node::File(bytes)) => bytes.clone(),
            Some(Node::Dir) => return Err(Error::Unsupported("copy folders through vfs::copy")),
            None => return Err(Error::NotFound),
        };
        let total = bytes.len() as u64;
        tokio::select! {
            () = cancel.cancelled() => return Err(Error::Cancelled),
            () = tokio::time::sleep(self.copy_delay) => {}
        }
        let to_key = key(to);
        let mut tree = lock(&self.tree);
        if !opts.overwrite && tree.contains_key(&to_key) {
            return Err(Error::Io(std::io::Error::from(
                std::io::ErrorKind::AlreadyExists,
            )));
        }
        tree.insert(to_key, Node::File(bytes));
        progress.report(Progress::Bytes {
            done: total,
            total: Some(total),
        });
        Ok(())
    }

    fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
        Err(Error::Unsupported("no watches in memory"))
    }
}

/// An extension that registers one [`MemProvider`].
pub struct MemExtension {
    manifest: Manifest,
    provider: Arc<MemProvider>,
}

impl MemExtension {
    /// An extension whose provider is `provider`; the manifest's id is the
    /// provider's id with `-test` appended.
    #[must_use]
    pub fn new(provider: Arc<MemProvider>) -> Self {
        let id: &'static str = Box::leak(format!("{}-test", provider.id).into_boxed_str());
        Self {
            manifest: Manifest {
                id: ExtId(id),
                name: "In-memory provider",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
            },
            provider,
        }
    }
}

impl Extension for MemExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.provider(Arc::clone(&self.provider) as Arc<dyn Provider>);
        Ok(())
    }
}
