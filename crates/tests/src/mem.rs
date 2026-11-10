//! An in-memory [`Provider`] for the integration tests: a tree of folders
//! and files under any provider id, with a settable delay per copied file,
//! so a test can mount a stand-in for a cloud provider (`s3`, `azure`) and
//! watch copies on disjoint volumes run side by side. With
//! [`MemProvider::set_native`] on, it moves and copies through
//! [`Provider::transfer`] under `NATIVE_TRANSFER`, like a cloud provider,
//! and counts the bytes read through `open_read`, so a test can prove a
//! transfer streamed nothing.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use ext::{
    API_VERSION, Capabilities, ChangeStream, CopyOptions, EntryStream, Extension, LinkKind,
    ListOptions, Manifest, Platforms, Points, Provider, ReadHandle, Registry, RemoveMode,
    TransferMode, Version, WriteHandle, WriteOptions, async_trait,
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
    /// Moves and copies run through `transfer`, as a cloud provider's do.
    native: AtomicBool,
    /// Bytes handed out through `open_read`.
    read_bytes: Arc<AtomicU64>,
    /// Every `transfer` call: the mode, the items and the target folder.
    transfers: Mutex<Vec<(TransferMode, Vec<String>, String)>>,
    /// Where reads wait, for a test that looks at a download half way.
    gate: Arc<Gate>,
}

/// Holds every reader at a byte position until it is opened again.
struct Gate {
    /// Readers stop once this many bytes of their file are handed out;
    /// `u64::MAX` lets them run.
    limit: AtomicU64,
    /// The readers waiting at the gate.
    waiting: Mutex<Vec<std::task::Waker>>,
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
            native: AtomicBool::new(false),
            read_bytes: Arc::new(AtomicU64::new(0)),
            transfers: Mutex::new(Vec::new()),
            gate: Arc::new(Gate {
                limit: AtomicU64::new(u64::MAX),
                waiting: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Makes every read stop after `bytes` of its file, as a slow link
    /// would leave it for a while, until [`release_reads`](Self::release_reads).
    pub fn hold_reads_at(&self, bytes: u64) {
        self.gate.limit.store(bytes, Ordering::SeqCst);
    }

    /// Lets the reads held by [`hold_reads_at`](Self::hold_reads_at) run
    /// to their end.
    pub fn release_reads(&self) {
        self.gate.limit.store(u64::MAX, Ordering::SeqCst);
        let waiting = std::mem::take(
            &mut *self
                .gate
                .waiting
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for waker in waiting {
            waker.wake();
        }
    }

    /// Turns `NATIVE_TRANSFER` on or off: with it, moves and copies within
    /// the provider go through [`Provider::transfer`] and are recorded.
    pub fn set_native(&self, on: bool) {
        self.native.store(on, Ordering::SeqCst);
    }

    /// Bytes handed out through `open_read` so far.
    #[must_use]
    pub fn read_bytes(&self) -> u64 {
        self.read_bytes.load(Ordering::SeqCst)
    }

    /// Every `transfer` call so far: the mode, the items' paths and the
    /// target folder's path.
    #[must_use]
    pub fn transfers(&self) -> Vec<(TransferMode, Vec<String>, String)> {
        self.transfers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        let mut caps = Capabilities::WRITE | Capabilities::RENAME;
        if self.native.load(Ordering::SeqCst) {
            caps |= Capabilities::NATIVE_TRANSFER | Capabilities::REMOTE;
        }
        caps
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
            Some(Node::File(bytes)) => Ok(Box::new(Counted {
                inner: std::io::Cursor::new(bytes.clone()),
                count: Arc::clone(&self.read_bytes),
                gate: Arc::clone(&self.gate),
            })),
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

    /// Moves or copies the items into `to` inside the tree, recording the
    /// call; nothing is read through `open_read`.
    async fn transfer(
        &self,
        items: &[Location],
        to: &Location,
        mode: TransferMode,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        if !self.native.load(Ordering::SeqCst) {
            return Err(Error::Unsupported("native transfer is off"));
        }
        self.transfers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((mode, items.iter().map(key).collect(), key(to)));
        tokio::select! {
            () = cancel.cancelled() => return Err(Error::Cancelled),
            () = tokio::time::sleep(self.copy_delay) => {}
        }
        let dir = key(to);
        let mut tree = lock(&self.tree);
        let total = items.len() as u64;
        for (done, item) in items.iter().enumerate() {
            let from = key(item);
            let name = from.rsplit('/').next().unwrap_or(&from).to_owned();
            let target_root = if dir == "/" {
                format!("/{name}")
            } else {
                format!("{dir}/{name}")
            };
            let moved: Vec<(String, Node)> = tree
                .iter()
                .filter(|(p, _)| **p == from || p.starts_with(&format!("{from}/")))
                .map(|(p, n)| (p.clone(), n.clone()))
                .collect();
            if moved.is_empty() {
                return Err(Error::NotFound);
            }
            for (p, n) in moved {
                let rest = &p[from.len()..];
                tree.insert(format!("{target_root}{rest}"), n);
                if mode == TransferMode::Move {
                    tree.remove(&p);
                }
            }
            progress.report(Progress::Items {
                done: done as u64 + 1,
                total: Some(total),
            });
        }
        Ok(())
    }

    fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
        Err(Error::Unsupported("no watches in memory"))
    }
}

/// A reader that adds what it hands out to the provider's count.
struct Counted {
    inner: std::io::Cursor<Vec<u8>>,
    count: Arc<AtomicU64>,
    gate: Arc<Gate>,
}

impl tokio::io::AsyncRead for Counted {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // Up to the gate and no further while it is shut.
        let limit = self.gate.limit.load(Ordering::SeqCst);
        let room = limit.saturating_sub(self.inner.position());
        let left = (self.inner.get_ref().len() as u64).saturating_sub(self.inner.position());
        if room == 0 && left > 0 {
            self.gate
                .waiting
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(cx.waker().clone());
            // The gate may have opened while the waker was being left.
            if self.gate.limit.load(Ordering::SeqCst) == limit {
                return Poll::Pending;
            }
        }
        let unfilled = buf.initialize_unfilled();
        let take = usize::try_from(room.max(1))
            .unwrap_or(usize::MAX)
            .min(unfilled.len());
        let n = std::io::Read::read(&mut self.inner, &mut unfilled[..take])?;
        buf.advance(n);
        self.count.fetch_add(n as u64, Ordering::SeqCst);
        Poll::Ready(Ok(()))
    }
}

/// An extension that registers one [`MemProvider`] and, asked to, a
/// connector for it, so the folders at its root are accounts as a cloud
/// provider has them.
pub struct MemExtension {
    manifest: Manifest,
    provider: Arc<MemProvider>,
    /// The connector's title, a Fluent key, when it has one.
    accounts: Option<&'static str>,
}

/// The connector of a [`MemProvider`] with accounts: it keeps nothing and
/// every account is always connected.
struct MemConnector {
    provider: ProviderId,
    title: &'static str,
}

#[async_trait]
impl ext::Connector for MemConnector {
    fn provider(&self) -> ProviderId {
        self.provider
    }
    fn schema(&self) -> ext::ConnectionSchema {
        ext::ConnectionSchema {
            title: ext::Text(self.title),
            fields: Vec::new(),
        }
    }
    fn update(&self, _connections: Vec<ext::Connection>) {}
    async fn connect(&self, _name: &str) -> Result<(), Error> {
        Ok(())
    }
    fn disconnect(&self, _name: &str) {}
    fn is_connected(&self, _name: &str) -> bool {
        true
    }
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
                points: Points::PROVIDER,
                platforms: Platforms::ALL,
                previews: &[],
                commands: &[],
            },
            provider,
            accounts: None,
        }
    }

    /// [`new`](Self::new) with a connector whose connections are titled by
    /// the Fluent key `title`: the provider's root folders are its
    /// accounts, as *Always preview on ‹account›* needs them.
    #[must_use]
    pub fn with_accounts(provider: Arc<MemProvider>, title: &'static str) -> Self {
        Self {
            accounts: Some(title),
            ..Self::new(provider)
        }
    }
}

impl Extension for MemExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.provider(Arc::clone(&self.provider) as Arc<dyn Provider>);
        if let Some(title) = self.accounts {
            reg.connector(Arc::new(MemConnector {
                provider: self.provider.id,
                title,
            }));
        }
        Ok(())
    }
}
