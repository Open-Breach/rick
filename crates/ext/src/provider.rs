//! The Provider extension point: a tree of entries.
//!
//! A provider answers where files come from: the local disk, an archive, an
//! SFTP server, an object store. It streams listings in batches so the UI
//! paints the first screen before the walk finishes, and it implements the
//! native fast path for same-provider copies; cross-provider copies are
//! composed by `vfs` from [`Provider::open_read`] and [`Provider::open_write`].
//!
//! # Worked example
//!
//! A read-only provider over a fixed list of entries. Everything it cannot do
//! answers `Error::Unsupported`; `capabilities` tells the kernel so up front.
//!
//! ```
//! use std::time::SystemTime;
//!
//! use ext::{
//!     Capabilities, ChangeStream, CopyOptions, EntryStream, LinkKind, ListOptions, Provider,
//!     ReadHandle, RemoveMode, WriteHandle, WriteOptions, async_trait,
//! };
//! use futures::{StreamExt, stream};
//! use runtime::ProgressSink;
//! use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, Location, ProviderId};
//!
//! const ID: ProviderId = ProviderId("fixed");
//!
//! struct Fixed(Vec<Entry>);
//!
//! #[async_trait]
//! impl Provider for Fixed {
//!     fn id(&self) -> ProviderId {
//!         ID
//!     }
//!
//!     fn capabilities(&self, _loc: &Location) -> Capabilities {
//!         Capabilities::empty()
//!     }
//!
//!     fn list(&self, _loc: &Location, _opts: ListOptions, _cancel: CancelToken) -> EntryStream {
//!         stream::iter(vec![Ok(self.0.clone())]).boxed()
//!     }
//!
//!     async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
//!         self.0.iter().find(|e| e.path == loc.path).cloned().ok_or(Error::NotFound)
//!     }
//!
//!     async fn open_read(&self, _loc: &Location) -> Result<ReadHandle, Error> {
//!         Ok(Box::new(tokio::io::empty()))
//!     }
//!
//!     async fn open_write(&self, _loc: &Location, _opts: WriteOptions) -> Result<WriteHandle, Error> {
//!         Err(Error::Unsupported("fixed provider is read-only"))
//!     }
//!
//!     async fn create_dir(&self, _loc: &Location) -> Result<Entry, Error> {
//!         Err(Error::Unsupported("fixed provider is read-only"))
//!     }
//!
//!     async fn rename(&self, _from: &Location, _to: &Location) -> Result<Entry, Error> {
//!         Err(Error::Unsupported("fixed provider is read-only"))
//!     }
//!
//!     async fn remove(&self, _loc: &Location, _mode: RemoveMode) -> Result<(), Error> {
//!         Err(Error::Unsupported("fixed provider is read-only"))
//!     }
//!
//!     async fn link(&self, _target: &Location, _at: &Location, _kind: LinkKind) -> Result<Entry, Error> {
//!         Err(Error::Unsupported("fixed provider is read-only"))
//!     }
//!
//!     async fn copy(
//!         &self,
//!         _from: &Location,
//!         _to: &Location,
//!         _opts: CopyOptions,
//!         _progress: ProgressSink,
//!         _cancel: CancelToken,
//!     ) -> Result<(), Error> {
//!         Err(Error::Unsupported("fixed provider is read-only"))
//!     }
//!
//!     fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
//!         Err(Error::Unsupported("fixed provider does not watch"))
//!     }
//! }
//!
//! let readme = Entry {
//!     id: EntryId::new(ID, 1),
//!     name: "README.md".into(),
//!     path: "/README.md".into(),
//!     kind: EntryKind::File,
//!     size: Some(12),
//!     created: None,
//!     modified: Some(SystemTime::UNIX_EPOCH),
//!     accessed: None,
//!     attrs: Attrs::empty(),
//!     mime: None,
//! };
//! let provider = Fixed(vec![readme.clone()]);
//! let root = Location::new(ID, "/");
//!
//! runtime::block_on(async {
//!     let mut batches = provider.list(&root, ListOptions::default(), CancelToken::new());
//!     let first = batches.next().await.transpose().unwrap();
//!     assert_eq!(first, Some(vec![readme.clone()]));
//!     assert_eq!(provider.stat(&Location::new(ID, "/README.md")).await.unwrap(), readme);
//!     assert!(matches!(provider.stat(&Location::new(ID, "/nope")).await, Err(Error::NotFound)));
//! });
//! ```

use async_trait::async_trait;
use futures::stream::BoxStream;
use runtime::ProgressSink;
use tokio::io::{AsyncRead, AsyncWrite};
use types::{CancelToken, Entry, EntryId, Error, Location, ProviderId};

bitflags::bitflags! {
    /// What a provider can do at a location. The kernel greys out or hides
    /// operations the provider does not offer instead of trying and failing.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Capabilities: u32 {
        /// Entries can be created, written, renamed, removed.
        const WRITE = 1;
        /// `remove` with [`RemoveMode::Trash`] is available.
        const TRASH = 2;
        /// `rename` is available.
        const RENAME = 4;
        /// `link` with [`LinkKind::Symbolic`] is available.
        const SYMLINK = 8;
        /// `copy` can share extents instead of duplicating bytes.
        const REFLINK = 16;
        /// `watch` delivers change notifications.
        const WATCH = 32;
        /// Readers support seeking.
        const SEEK = 64;
        /// Entries carry permission bits the user may change.
        const PERMISSIONS = 128;
        /// Copy and move between two locations of this provider instance
        /// happen on the provider's side.
        const NATIVE_TRANSFER = 256;
        /// `link` with [`LinkKind::Junction`] is available (NTFS).
        const JUNCTION = 512;
        /// `link` with [`LinkKind::Shortcut`] is available (Windows `.lnk`).
        const SHORTCUT = 1024;
    }
}

/// How deep a listing goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Depth {
    /// The location's direct children only.
    #[default]
    One,
    /// Every descendant, for the flattened view.
    Unlimited,
}

/// What a listing includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListOptions {
    /// Include entries flagged `HIDDEN`.
    pub hidden: bool,
    /// Include entries flagged `SYSTEM`.
    pub system: bool,
    /// Direct children or the whole subtree.
    pub depth: Depth,
}

/// How to open an entry for writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WriteOptions {
    /// Replace an existing entry. When `false` an existing entry is an
    /// `AlreadyExists` I/O error.
    pub overwrite: bool,
}

/// What `remove` does with the entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveMode {
    /// Send it to the platform trash, restorable.
    Trash,
    /// Delete it for good.
    Permanent,
}

/// Which kind of link `link` creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// A symbolic link.
    Symbolic,
    /// A hard link to the same entry.
    Hard,
    /// An NTFS directory junction; Windows only, folders only.
    Junction,
    /// A classic `.lnk` shortcut file; Windows only.
    Shortcut,
}

impl LinkKind {
    /// The name shown in menus and dialogues.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Symbolic => "Symbolic link",
            Self::Hard => "Hard link",
            Self::Junction => "Directory junction",
            Self::Shortcut => "Shortcut (.lnk)",
        }
    }

    /// The capability a provider needs for this kind.
    #[must_use]
    pub fn capability(self) -> Capabilities {
        match self {
            Self::Symbolic | Self::Hard => Capabilities::SYMLINK,
            Self::Junction => Capabilities::JUNCTION,
            Self::Shortcut => Capabilities::SHORTCUT,
        }
    }
}

/// How to copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CopyOptions {
    /// Replace an existing target. When `false` an existing target is an
    /// `AlreadyExists` I/O error.
    pub overwrite: bool,
}

/// One change notification from [`Provider::watch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsChange {
    /// A new entry appeared.
    Created(Entry),
    /// An entry disappeared.
    Removed(EntryId),
    /// An entry's content or metadata changed.
    Modified(Entry),
    /// An entry was renamed within the watched location.
    Renamed {
        /// The entry before the rename.
        from: EntryId,
        /// The entry after it.
        to: Entry,
    },
    /// Too much changed to say what; list again.
    Rescan,
}

/// Batches of entries from a listing; the stream ends after the last batch.
pub type EntryStream = BoxStream<'static, Result<Vec<Entry>, Error>>;
/// Change notifications from a watch; the stream ends when the watch stops.
pub type ChangeStream = BoxStream<'static, FsChange>;
/// An open entry for reading.
pub type ReadHandle = Box<dyn AsyncRead + Send + Unpin>;
/// An open entry for writing.
pub type WriteHandle = Box<dyn AsyncWrite + Send + Unpin>;

/// A tree of entries: local, archive, SFTP, object storage.
///
/// Every method takes a [`Location`] whose `provider` is this provider's id;
/// the kernel routes by that id before calling.
#[async_trait]
pub trait Provider: Send + Sync + 'static {
    /// The id this provider's locations carry.
    fn id(&self) -> ProviderId;

    /// What the provider can do at `loc`.
    fn capabilities(&self, loc: &Location) -> Capabilities;

    /// Streams entries in batches. The first batch must arrive within the
    /// first-screen budget. Cancelling the token ends the stream within one
    /// batch.
    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken) -> EntryStream;

    /// The entry at `loc`.
    async fn stat(&self, loc: &Location) -> Result<Entry, Error>;

    /// Opens the entry for reading from the start.
    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error>;

    /// Opens or creates the entry for writing from the start.
    async fn open_write(&self, loc: &Location, opts: WriteOptions) -> Result<WriteHandle, Error>;

    /// Creates a directory and returns it.
    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error>;

    /// Renames or moves within the provider and returns the entry at `to`.
    async fn rename(&self, from: &Location, to: &Location) -> Result<Entry, Error>;

    /// Removes the entry, to the trash or for good.
    async fn remove(&self, loc: &Location, mode: RemoveMode) -> Result<(), Error>;

    /// Creates a link at `at` pointing to `target` and returns it.
    async fn link(&self, target: &Location, at: &Location, kind: LinkKind) -> Result<Entry, Error>;

    /// Same-provider copy with the native fast path (`copy_file_range`,
    /// reflink, server-side copy). Reports bytes through `progress` and stops
    /// with `Error::Cancelled` when `cancel` fires.
    async fn copy(
        &self,
        from: &Location,
        to: &Location,
        opts: CopyOptions,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error>;

    /// Change notifications for one location, if `WATCH` is set.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when the provider cannot watch `loc`, or the
    /// error the platform watcher reported while being set up.
    fn watch(&self, loc: &Location) -> Result<ChangeStream, Error>;
}
