//! The local filesystem as a [`Provider`].
//!
//! [`LocalProvider`] serves locations whose provider id is [`PROVIDER_ID`]
//! and whose path is the OS path in [`VPath`] bytes. Listings come from the
//! platform's bulk directory reader in batches of [`BATCH`] entries on the
//! blocking pool, so the first batch of a 200 000-entry directory arrives
//! before the walk finishes. Copies take the native fast path through
//! [`platform::fs::copy_file`]. Trash goes through the platform's [`Trash`].
//! Change notification comes from the `notify` crate, translated into
//! [`FsChange`](ext::FsChange) with stable entry ids. [`search`] is the
//! subtree search: the `ignore` crate's parallel walker streaming matches.
//!
//! Entry ids are the device and inode on Unix and the volume serial and file
//! id on Windows, so a renamed entry keeps its id.

#![forbid(unsafe_code)]

mod list;
pub mod search;
mod watch;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::{
    Capabilities, ChangeStream, CopyOptions, EntryStream, LinkKind, ListOptions, Provider,
    ReadHandle, RemoveMode, WriteHandle, WriteOptions, async_trait,
};
use platform::Trash;
use platform::fs::{RawEntry, RawKind};
use runtime::ProgressSink;
use types::{CancelToken, Entry, EntryId, EntryKind, Error, Location, Progress, ProviderId, VPath};

/// The id of every local location.
pub const PROVIDER_ID: ProviderId = ProviderId("local");

/// Entries per listing batch.
pub const BATCH: usize = 1024;

/// The local filesystem.
pub struct LocalProvider {
    trash: Arc<dyn Trash>,
}

impl LocalProvider {
    /// A provider that trashes through `trash`.
    #[must_use]
    pub fn new(trash: Arc<dyn Trash>) -> Self {
        Self { trash }
    }

    /// A provider that trashes through the platform the binary runs on.
    #[must_use]
    pub fn native() -> Self {
        Self::new(Arc::new(platform::Native::new()))
    }
}

impl fmt::Debug for LocalProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LocalProvider")
    }
}

/// The OS path of a local location.
pub(crate) fn path_of(loc: &Location) -> Result<PathBuf, Error> {
    if loc.provider != PROVIDER_ID {
        return Err(Error::Unsupported("location belongs to another provider"));
    }
    platform::paths::to_path(&loc.path)
}

/// Builds the entry for `raw`, listed in `dir` whose location path is
/// `dir_vpath`. Symbolic links are read to fill their target.
pub(crate) fn entry_from_raw(dir: &Path, dir_vpath: &VPath, raw: RawEntry) -> Entry {
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
    let kind = match kind {
        RawKind::File => EntryKind::File,
        RawKind::Dir => EntryKind::Dir,
        RawKind::Other => EntryKind::Other,
        RawKind::Symlink => {
            let full = dir.join(&name);
            EntryKind::Symlink {
                target: std::fs::read_link(&full)
                    .map(|target| platform::paths::from_path(&target))
                    .unwrap_or_default(),
                broken: std::fs::metadata(&full).is_err(),
            }
        }
    };
    Entry {
        id: EntryId::new(PROVIDER_ID, id),
        path: dir_vpath.join(name.as_encoded_bytes()),
        name: name.to_string_lossy().into_owned(),
        kind,
        size,
        created,
        modified,
        accessed,
        attrs,
        mime: None,
    }
}

/// The entry at `path`, named by the location path `vpath`, without going
/// through a provider: what the application uses to bring persisted paths
/// (the Recent list) back as entries with current ids and times.
///
/// # Errors
///
/// The I/O error from examining the path.
pub fn entry_at(path: &Path, vpath: &VPath) -> io::Result<Entry> {
    let raw = platform::fs::stat(path)?;
    let parent = path.parent().unwrap_or(path);
    let mut entry = entry_from_raw(parent, &VPath::new(), raw);
    entry.path = vpath.clone();
    Ok(entry)
}

/// Runs a blocking filesystem call on the runtime's blocking pool.
async fn blocking<T, F>(work: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce() -> io::Result<T> + Send + 'static,
{
    runtime::handle()
        .spawn_blocking(work)
        .await
        .map_err(|join| Error::Io(io::Error::other(join)))?
        .map_err(Error::from_io)
}

#[async_trait]
impl Provider for LocalProvider {
    fn id(&self) -> ProviderId {
        PROVIDER_ID
    }

    fn capabilities(&self, _loc: &Location) -> Capabilities {
        Capabilities::WRITE
            | Capabilities::TRASH
            | Capabilities::RENAME
            | Capabilities::SYMLINK
            | Capabilities::REFLINK
            | Capabilities::WATCH
            | Capabilities::SEEK
            | Capabilities::PERMISSIONS
    }

    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken) -> EntryStream {
        list::stream(loc, opts, cancel)
    }

    async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
        let path = path_of(loc)?;
        let vpath = loc.path.clone();
        blocking(move || entry_at(&path, &vpath)).await
    }

    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
        let path = path_of(loc)?;
        let file = tokio::fs::File::open(path).await.map_err(Error::from_io)?;
        Ok(Box::new(file))
    }

    async fn open_write(&self, loc: &Location, opts: WriteOptions) -> Result<WriteHandle, Error> {
        let path = path_of(loc)?;
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).truncate(true);
        if opts.overwrite {
            options.create(true);
        } else {
            options.create_new(true);
        }
        let file = options.open(path).await.map_err(Error::from_io)?;
        Ok(Box::new(file))
    }

    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error> {
        let path = path_of(loc)?;
        tokio::fs::create_dir(&path).await.map_err(Error::from_io)?;
        self.stat(loc).await
    }

    async fn rename(&self, from: &Location, to: &Location) -> Result<Entry, Error> {
        let (from_path, to_path) = (path_of(from)?, path_of(to)?);
        tokio::fs::rename(&from_path, &to_path)
            .await
            .map_err(Error::from_io)?;
        self.stat(to).await
    }

    async fn remove(&self, loc: &Location, mode: RemoveMode) -> Result<(), Error> {
        let path = path_of(loc)?;
        match mode {
            RemoveMode::Permanent => {
                let meta = tokio::fs::symlink_metadata(&path)
                    .await
                    .map_err(Error::from_io)?;
                if meta.is_dir() {
                    tokio::fs::remove_dir_all(&path).await
                } else {
                    tokio::fs::remove_file(&path).await
                }
                .map_err(Error::from_io)
            }
            RemoveMode::Trash => {
                let trash = Arc::clone(&self.trash);
                runtime::handle()
                    .spawn_blocking(move || trash.trash(&path).map(drop))
                    .await
                    .map_err(|join| Error::Io(io::Error::other(join)))?
            }
        }
    }

    async fn link(&self, target: &Location, at: &Location, kind: LinkKind) -> Result<Entry, Error> {
        let (target_path, at_path) = (path_of(target)?, path_of(at)?);
        blocking(move || match kind {
            LinkKind::Symbolic => platform::fs::symlink(&target_path, &at_path),
            LinkKind::Hard => std::fs::hard_link(&target_path, &at_path),
        })
        .await?;
        self.stat(at).await
    }

    /// Copies one regular file with the native fast path; directories are
    /// walked by `vfs::copy`, which calls this once per file.
    async fn copy(
        &self,
        from: &Location,
        to: &Location,
        opts: CopyOptions,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        let (from_path, to_path) = (path_of(from)?, path_of(to)?);
        let outcome = runtime::handle()
            .spawn_blocking(move || {
                let span = tracing::info_span!("vfs_local.copy", from = %from_path.display());
                let _enter = span.enter();
                platform::fs::copy_file(&from_path, &to_path, opts.overwrite, &mut |done, total| {
                    progress.report(Progress::Bytes {
                        done,
                        total: Some(total),
                    });
                    !cancel.is_cancelled()
                })
            })
            .await
            .map_err(|join| Error::Io(io::Error::other(join)))?;
        match outcome {
            Ok(method) => {
                tracing::debug!(?method, "copied");
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => Err(Error::Cancelled),
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                Err(Error::Unsupported("copy directories through vfs::copy"))
            }
            Err(e) => Err(Error::from_io(e)),
        }
    }

    fn watch(&self, loc: &Location) -> Result<ChangeStream, Error> {
        watch::stream(loc)
    }
}

#[cfg(test)]
mod tests;
