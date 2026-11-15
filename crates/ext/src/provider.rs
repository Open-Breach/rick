//! The Provider extension point: a tree of entries.
//!
//! A provider answers where files come from: the local disk, an archive, an
//! SFTP server, an object store. It streams listings in batches so the UI
//! paints the first screen before the walk finishes, and it implements the
//! native fast path for same-provider copies; cross-provider copies are
//! composed by `vfs` from [`Provider::open_read`] and [`Provider::open_write`].
//! A provider whose service can move or copy a selection by itself (an
//! object store's copy API, a remote rename) sets
//! [`Capabilities::NATIVE_TRANSFER`] and answers [`Provider::transfer`];
//! the default answers `Unsupported` and the kernel does the work.
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
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite};
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
        /// happen on the provider's side (server-side copy, remote rename):
        /// nothing streams through the machine and the job only tracks
        /// progress. `ops` calls [`Provider::transfer`] first where it is
        /// set.
        const NATIVE_TRANSFER = 256;
        /// `link` with [`LinkKind::Junction`] is available (NTFS).
        const JUNCTION = 512;
        /// `link` with [`LinkKind::Shortcut`] is available (Windows `.lnk`).
        const SHORTCUT = 1024;
        /// The entries live behind a network: the kernel treats the
        /// location as a remote-latency mount (SPEC.md "Network file
        /// systems"), with no eager stat and no thumbnails by default.
        const REMOTE = 2048;
        /// A folder can be created in the location with
        /// [`Provider::create_dir`]: what *New folder* asks for. `WRITE`
        /// alone does not say so: an object store's account takes a
        /// container and no file.
        const CREATE_DIR = 4096;
        /// A file can be created in the location with
        /// [`Provider::open_write`]: what *New file* asks for.
        const CREATE_FILE = 8192;
        /// [`Provider::list`] with [`Depth::Unlimited`] answers for
        /// everything beneath the location in one pass, as an object store
        /// does with a prefix, so the search in subfolders can run on it:
        /// the kernel lists and matches the names itself.
        const SEARCH_SUBTREE = 16384;
        /// The location's files and folders can be brought down to the
        /// machine: *Download…*, and what Enter does on a remote file.
        const DOWNLOAD = 32768;
    }
}

/// Whether a [`Provider::transfer`] moves or copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferMode {
    /// The items end up at the target and are gone from the source.
    Move,
    /// The items are duplicated at the target.
    Copy,
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
    /// The name shown in menus and dialogues, in the language in use.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Symbolic => crate::t!("link-symbolic"),
            Self::Hard => crate::t!("link-hard"),
            Self::Junction => crate::t!("link-junction"),
            Self::Shortcut => crate::t!("link-shortcut"),
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

    /// Why `loc` lacks `capability`, in the language in use: the tooltip
    /// of the control that is disabled for it (*Files live inside a
    /// container*). `None`, the default, leaves the kernel's own sentence
    /// (SPEC.md, *Capabilities decide what is offered*).
    fn unavailable_reason(&self, loc: &Location, capability: Capabilities) -> Option<String> {
        let _ = (loc, capability);
        None
    }

    /// What is wrong with the name of the folder [`create_dir`] would make
    /// at `loc`, in the language in use, when the provider's own naming
    /// rules rule it out: an object store's rules for a container. The
    /// kernel asks before any request goes out and refuses the name in
    /// place with this text. `None`, the default, is a name the provider
    /// has nothing against.
    ///
    /// [`create_dir`]: Provider::create_dir
    fn name_problem(&self, loc: &Location) -> Option<String> {
        let _ = loc;
        None
    }

    /// The path on this machine's file system that holds the file at
    /// `loc` as it is, for a provider whose files are local files under
    /// another name: a ZFS snapshot's file in the dataset's
    /// `.zfs/snapshot` directory. The kernel hands it to the system to
    /// open it, with an application or in the editor, where it would
    /// otherwise offer nothing. `None`, the default, is a file that has no
    /// such path.
    fn local_path(&self, loc: &Location) -> Option<std::path::PathBuf> {
        let _ = loc;
        None
    }

    /// Streams entries in batches. The first batch must arrive within the
    /// first-screen budget. Cancelling the token ends the stream within one
    /// batch.
    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken) -> EntryStream;

    /// The entry at `loc`.
    async fn stat(&self, loc: &Location) -> Result<Entry, Error>;

    /// Opens the entry for reading from the start.
    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error>;

    /// Opens the entry for reading from byte `offset` on: what `SEEK`
    /// promises. The default opens from the start and discards the first
    /// `offset` bytes; a provider with a cheaper way (a `tail -c +N` on a
    /// remote host) overrides it.
    ///
    /// # Errors
    ///
    /// As [`open_read`](Self::open_read), or the error of discarding.
    async fn open_read_at(&self, loc: &Location, offset: u64) -> Result<ReadHandle, Error> {
        let mut reader = self.open_read(loc).await?;
        if offset > 0 {
            tokio::io::copy(&mut (&mut reader).take(offset), &mut tokio::io::sink())
                .await
                .map_err(Error::from_io)?;
        }
        Ok(reader)
    }

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

    /// Moves or copies a whole selection within this provider instance on
    /// the provider's side, if `NATIVE_TRANSFER` is set: the object store's
    /// copy API across buckets of one account, a remote rename over SSH.
    /// Every item lands in `to` under its own name. The provider submits
    /// the operation, reports progress as the service exposes it and
    /// honours `cancel` where the service allows. `ops` calls this first
    /// when source and target share a provider instance with the
    /// capability and falls back to `rename`, `copy` and the streamed path
    /// when it answers `Error::Unsupported`, which the default does, and
    /// which a provider answers for a mode it has no native path for (an
    /// SFTP server renames but cannot copy).
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when there is no native path for `mode` here;
    /// `Error::Cancelled` when cancelled; the service's error otherwise.
    async fn transfer(
        &self,
        items: &[Location],
        to: &Location,
        mode: TransferMode,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        let _ = (items, to, mode, progress, cancel);
        Err(Error::Unsupported(
            "this provider moves and copies through the kernel",
        ))
    }

    /// The size in bytes above which a whole-file previewer is held behind
    /// *Preview anyway* for an entry at `loc`, for a provider with
    /// [`Capabilities::REMOTE`]: a per-host override of the kernel's
    /// default, 4 MiB (`specs/ext-ssh.md`, "Previews over SSH"). `None`,
    /// the default, leaves the kernel's default in force.
    fn remote_preview_cap(&self, loc: &Location) -> Option<u64> {
        let _ = loc;
        None
    }

    /// Whether the source holding `loc` (one SSH host, one cloud account)
    /// has *Always preview* on: its `preview_always` switch, set from the
    /// cap message or the source's editor, after which no whole-file
    /// preview there is held behind the cap (SPEC.md, Preview, *Remote
    /// sources*). `false`, the default, leaves the cap in force.
    fn remote_preview_always(&self, loc: &Location) -> bool {
        let _ = loc;
        false
    }

    /// Change notifications for one location, if `WATCH` is set.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when the provider cannot watch `loc`, or the
    /// error the platform watcher reported while being set up.
    fn watch(&self, loc: &Location) -> Result<ChangeStream, Error>;
}
