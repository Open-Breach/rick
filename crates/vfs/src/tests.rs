//! Tests against an in-memory provider, so two providers can be mounted.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use ext::{
    Capabilities, ChangeStream, CopyOptions, EntryStream, LinkKind, ListOptions, Provider,
    ReadHandle, RemoveMode, WriteHandle, WriteOptions, async_trait,
};
use futures::{StreamExt, stream};
use runtime::{PauseGate, ProgressSink};
use tokio::io::AsyncWrite;
use types::{
    Attrs, CancelToken, Entry, EntryId, EntryKind, Error, Location, Progress, ProviderId, VPath,
};

use super::{Mounts, copy, move_entry};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    File(Vec<u8>),
    Dir,
}

type Tree = Arc<Mutex<BTreeMap<Vec<u8>, Node>>>;

fn lock(tree: &Tree) -> MutexGuard<'_, BTreeMap<Vec<u8>, Node>> {
    tree.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A provider over a map from path bytes to nodes, with `/` separators.
struct Memory {
    id: ProviderId,
    tree: Tree,
    native_copies: AtomicUsize,
}

impl Memory {
    fn new(id: &'static str) -> Arc<Self> {
        let mut tree = BTreeMap::new();
        tree.insert(b"/".to_vec(), Node::Dir);
        Arc::new(Self {
            id: ProviderId(id),
            tree: Arc::new(Mutex::new(tree)),
            native_copies: AtomicUsize::new(0),
        })
    }

    fn put(&self, path: &str, node: Node) {
        lock(&self.tree).insert(path.as_bytes().to_vec(), node);
    }

    fn get(&self, path: &str) -> Option<Node> {
        lock(&self.tree).get(path.as_bytes()).cloned()
    }

    fn entry(&self, path: &[u8], node: &Node) -> Entry {
        let vpath = VPath::from_bytes(path.to_vec());
        Entry {
            id: EntryId::new(self.id, u128::from(fxhash(path))),
            name: String::from_utf8_lossy(vpath.file_name().unwrap_or(b"/")).into_owned(),
            kind: match node {
                Node::File(_) => EntryKind::File,
                Node::Dir => EntryKind::Dir,
            },
            size: match node {
                Node::File(bytes) => Some(bytes.len() as u64),
                Node::Dir => None,
            },
            path: vpath,
            created: None,
            modified: None,
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        }
    }
}

fn fxhash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3)
    })
}

fn parent_of(path: &[u8]) -> &[u8] {
    match path.iter().rposition(|&b| b == b'/') {
        Some(0) | None => b"/",
        Some(i) => &path[..i],
    }
}

/// Collects writes and stores the file when shut down.
struct MemoryWriter {
    tree: Tree,
    path: Vec<u8>,
    buffer: Vec<u8>,
}

impl AsyncWrite for MemoryWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.buffer.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let bytes = std::mem::take(&mut self.buffer);
        lock(&self.tree).insert(self.path.clone(), Node::File(bytes));
        Poll::Ready(Ok(()))
    }
}

#[async_trait]
impl Provider for Memory {
    fn id(&self) -> ProviderId {
        self.id
    }

    fn capabilities(&self, _loc: &Location) -> Capabilities {
        Capabilities::WRITE | Capabilities::RENAME
    }

    fn list(&self, loc: &Location, _opts: ListOptions, _cancel: CancelToken) -> EntryStream {
        let parent = loc.path.as_bytes().to_vec();
        let children: Vec<Entry> = lock(&self.tree)
            .iter()
            .filter(|(path, _)| {
                path.as_slice() != parent.as_slice() && parent_of(path) == parent.as_slice()
            })
            .map(|(path, node)| self.entry(path, node))
            .collect();
        // Two batches, to exercise batch handling.
        let (a, b) = children.split_at(children.len() / 2);
        stream::iter(vec![Ok(a.to_vec()), Ok(b.to_vec())]).boxed()
    }

    async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
        let path = loc.path.as_bytes();
        lock(&self.tree)
            .get(path)
            .map(|node| self.entry(path, node))
            .ok_or(Error::NotFound)
    }

    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
        match lock(&self.tree).get(loc.path.as_bytes()) {
            Some(Node::File(bytes)) => Ok(Box::new(Cursor::new(bytes.clone()))),
            Some(Node::Dir) => Err(Error::Unsupported("cannot read a directory")),
            None => Err(Error::NotFound),
        }
    }

    async fn open_write(&self, loc: &Location, opts: WriteOptions) -> Result<WriteHandle, Error> {
        let path = loc.path.as_bytes().to_vec();
        let tree = lock(&self.tree);
        if !opts.overwrite && tree.contains_key(&path) {
            return Err(Error::Io(std::io::ErrorKind::AlreadyExists.into()));
        }
        if !tree.contains_key(parent_of(&path)) {
            return Err(Error::NotFound);
        }
        drop(tree);
        Ok(Box::new(MemoryWriter {
            tree: Arc::clone(&self.tree),
            path,
            buffer: Vec::new(),
        }))
    }

    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error> {
        let path = loc.path.as_bytes().to_vec();
        let mut tree = lock(&self.tree);
        if tree.contains_key(&path) {
            return Err(Error::Io(std::io::ErrorKind::AlreadyExists.into()));
        }
        tree.insert(path.clone(), Node::Dir);
        drop(tree);
        Ok(self.entry(&path, &Node::Dir))
    }

    async fn rename(&self, from: &Location, to: &Location) -> Result<Entry, Error> {
        let mut tree = lock(&self.tree);
        let node = tree.remove(from.path.as_bytes()).ok_or(Error::NotFound)?;
        tree.insert(to.path.as_bytes().to_vec(), node.clone());
        drop(tree);
        Ok(self.entry(to.path.as_bytes(), &node))
    }

    async fn remove(&self, loc: &Location, _mode: RemoveMode) -> Result<(), Error> {
        let prefix = loc.path.as_bytes().to_vec();
        let mut tree = lock(&self.tree);
        let before = tree.len();
        tree.retain(|path, _| {
            !(path == &prefix
                || (path.starts_with(&prefix) && path.get(prefix.len()) == Some(&b'/')))
        });
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

    async fn copy(
        &self,
        from: &Location,
        to: &Location,
        opts: CopyOptions,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        self.native_copies.fetch_add(1, Ordering::Relaxed);
        let mut tree = lock(&self.tree);
        let node = tree
            .get(from.path.as_bytes())
            .cloned()
            .ok_or(Error::NotFound)?;
        if !opts.overwrite && tree.contains_key(to.path.as_bytes()) {
            return Err(Error::Io(std::io::ErrorKind::AlreadyExists.into()));
        }
        let size = match &node {
            Node::File(b) => b.len() as u64,
            Node::Dir => 0,
        };
        tree.insert(to.path.as_bytes().to_vec(), node);
        progress.report(Progress::Bytes {
            done: size,
            total: Some(size),
        });
        Ok(())
    }

    fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
        Err(Error::Unsupported("no watching in memory"))
    }
}

fn loc(id: &'static str, path: &str) -> Location {
    Location::new(ProviderId(id), path)
}

fn tree_with_files(id: &'static str) -> Arc<Memory> {
    let p = Memory::new(id);
    p.put("/src", Node::Dir);
    p.put("/src/a.txt", Node::File(b"alpha".to_vec()));
    p.put("/src/sub", Node::Dir);
    p.put("/src/sub/b.bin", Node::File(vec![7u8; 3_000_000]));
    p.put("/src/sub/empty", Node::Dir);
    p
}

#[test]
fn mounts_route_by_provider_id() {
    let mounts = Mounts::new();
    let a = Memory::new("a");
    mounts.mount(a.clone());
    assert!(mounts.provider(ProviderId("a")).is_some());
    assert!(mounts.resolve(&loc("a", "/")).is_ok());
    assert!(matches!(
        mounts.resolve(&loc("b", "/")),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(mounts.ids(), [ProviderId("a")]);
    assert!(format!("{mounts:?}").contains("ids"));

    let mut registry = ext::Registry::new();
    registry.provider(Memory::new("c"));
    assert_eq!(Mounts::from_registry(&registry).ids(), [ProviderId("c")]);
}

#[test]
fn cross_provider_copy_streams_a_whole_tree_with_progress() {
    let (a, b) = (tree_with_files("a"), Memory::new("b"));
    let mounts = Mounts::new();
    mounts.mount(a.clone());
    mounts.mount(b.clone());
    let (sink, rx) = ProgressSink::channel();

    runtime::block_on(copy(
        &mounts,
        &loc("a", "/src"),
        &loc("b", "/dst"),
        CopyOptions::default(),
        sink,
        CancelToken::new(),
        &PauseGate::open(),
    ))
    .unwrap_or_else(|e| panic!("{e}"));

    assert_eq!(b.get("/dst"), Some(Node::Dir));
    assert_eq!(b.get("/dst/a.txt"), Some(Node::File(b"alpha".to_vec())));
    assert_eq!(
        b.get("/dst/sub/b.bin"),
        Some(Node::File(vec![7u8; 3_000_000]))
    );
    assert_eq!(b.get("/dst/sub/empty"), Some(Node::Dir));
    assert_eq!(a.native_copies.load(Ordering::Relaxed), 0);
    assert_eq!(
        *rx.borrow(),
        Progress::Bytes {
            done: 3_000_005,
            total: Some(3_000_005)
        }
    );
}

#[test]
fn same_provider_copy_uses_the_native_path() {
    let a = tree_with_files("a");
    let mounts = Mounts::new();
    mounts.mount(a.clone());
    let (sink, rx) = ProgressSink::channel();
    runtime::block_on(copy(
        &mounts,
        &loc("a", "/src"),
        &loc("a", "/copy"),
        CopyOptions::default(),
        sink,
        CancelToken::new(),
        &PauseGate::open(),
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(a.native_copies.load(Ordering::Relaxed), 2);
    assert_eq!(
        a.get("/copy/sub/b.bin"),
        Some(Node::File(vec![7u8; 3_000_000]))
    );
    assert_eq!(
        *rx.borrow(),
        Progress::Bytes {
            done: 3_000_005,
            total: Some(3_000_005)
        }
    );
}

#[test]
fn existing_targets_need_overwrite() {
    let (a, b) = (tree_with_files("a"), Memory::new("b"));
    b.put("/dst", Node::Dir);
    b.put("/dst/a.txt", Node::File(b"old".to_vec()));
    let mounts = Mounts::new();
    mounts.mount(a.clone());
    mounts.mount(b.clone());

    let refused = runtime::block_on(copy(
        &mounts,
        &loc("a", "/src"),
        &loc("b", "/dst"),
        CopyOptions::default(),
        ProgressSink::channel().0,
        CancelToken::new(),
        &PauseGate::open(),
    ));
    assert!(matches!(refused, Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists));

    runtime::block_on(copy(
        &mounts,
        &loc("a", "/src"),
        &loc("b", "/dst"),
        CopyOptions { overwrite: true },
        ProgressSink::channel().0,
        CancelToken::new(),
        &PauseGate::open(),
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(b.get("/dst/a.txt"), Some(Node::File(b"alpha".to_vec())));
}

#[test]
fn a_cancelled_copy_stops_and_says_so() {
    let (a, b) = (tree_with_files("a"), Memory::new("b"));
    let mounts = Mounts::new();
    mounts.mount(a.clone());
    mounts.mount(b.clone());
    let cancel = CancelToken::new();
    cancel.cancel();
    let result = runtime::block_on(copy(
        &mounts,
        &loc("a", "/src"),
        &loc("b", "/dst"),
        CopyOptions::default(),
        ProgressSink::channel().0,
        cancel,
        &PauseGate::open(),
    ));
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
    assert_eq!(b.get("/dst/a.txt"), None);
}

#[test]
fn move_renames_within_a_provider_and_copies_across() {
    let (a, b) = (tree_with_files("a"), Memory::new("b"));
    let mounts = Mounts::new();
    mounts.mount(a.clone());
    mounts.mount(b.clone());

    runtime::block_on(move_entry(
        &mounts,
        &loc("a", "/src/a.txt"),
        &loc("a", "/renamed.txt"),
        CopyOptions::default(),
        ProgressSink::channel().0,
        CancelToken::new(),
        &PauseGate::open(),
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(a.get("/src/a.txt"), None);
    assert_eq!(a.get("/renamed.txt"), Some(Node::File(b"alpha".to_vec())));

    runtime::block_on(move_entry(
        &mounts,
        &loc("a", "/src"),
        &loc("b", "/moved"),
        CopyOptions::default(),
        ProgressSink::channel().0,
        CancelToken::new(),
        &PauseGate::open(),
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(a.get("/src"), None);
    assert_eq!(a.get("/src/sub/b.bin"), None);
    assert_eq!(
        b.get("/moved/sub/b.bin"),
        Some(Node::File(vec![7u8; 3_000_000]))
    );
}

#[test]
fn relocate_swaps_the_prefix() {
    use super::relocate;
    assert_eq!(
        relocate(&"/src/x/y".into(), &"/src".into(), &"/dst".into()).to_string(),
        "/dst/x/y"
    );
    assert_eq!(
        relocate(&"/src".into(), &"/src".into(), &"/dst".into()).to_string(),
        "/dst"
    );
    assert_eq!(
        relocate(&"/elsewhere/z".into(), &"/src".into(), &"/dst".into()).to_string(),
        "/dst/z"
    );
}
