//! The archive provider: an archive on the local disk browsed in place as
//! a location of the provider [`ProviderId::ARCHIVE`].
//!
//! The location's path is the archive's OS path followed by the path inside
//! it, `/home/me/a.zip/docs/readme.txt`, so the breadcrumb shows the
//! archive as a segment; [`split`] asks the filesystem which prefix is the
//! archive. Every archive is read into an [`Index`] once and kept while its
//! size and modification time hold; listings answer from it in one batch,
//! `open_read` decompresses the one entry on the blocking pool and streams
//! it through [`bridge`], and copying out of an archive goes through the
//! kernel's streamed path. Zip, the tar family and 7z take writes: each
//! `open_write`, `create_dir`, `rename`, `remove` or same-archive `copy`
//! rewrites the archive into a temporary file beside it and renames it over
//! the original, under a lock per archive, inside the job that asked.
//! RAR and the disc images are read-only and their capabilities say so;
//! `watch` is `Unsupported` and the kernel polls the archive file.
//!
//! An archive inside an archive is a file, not a location (QUESTIONS.md
//! 89). The crate builds as a `cdylib` the plugin host loads from
//! `plugins/vfs-archive/` and as an `rlib` for the binary's opt-in
//! `vfs-archive` feature and the tests.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

pub mod bridge;
pub mod format;
mod image;
pub mod index;
mod rar;
mod rewrite;
mod sevenz;
mod single;
mod tarfmt;
mod zipfmt;

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::SystemTime;

use ext::{
    API_VERSION, Capabilities, ChangeStream, CopyOptions, Depth, EntryStream, Extension, LinkKind,
    ListOptions, Manifest, Platforms, Points, ProgressSink, Provider, ReadHandle, Registry,
    RemoveMode, Version, WriteHandle, WriteOptions, async_trait,
};
use futures::{StreamExt, stream};
use tokio::io::AsyncWrite;
use types::{
    Attrs, CancelToken, Entry, EntryId, EntryKind, Error, ExtId, Location, ProviderId, VPath,
};

pub use format::{Codec, Format};
pub use index::{Index, Item};
use rewrite::{Edit, Plan};

/// The extension id.
pub const ID: ExtId = ExtId("vfs-archive");

/// The provider id every archive location carries.
pub const PROVIDER_ID: ProviderId = ProviderId::ARCHIVE;

/// An archive library's error as the kernel's.
pub(crate) fn ext_error(error: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::Extension {
        ext: ID,
        source: Box::new(error),
    }
}

/// A message as the kernel's error.
pub(crate) fn ext_error_text(text: String) -> Error {
    Error::Extension {
        ext: ID,
        source: text.into(),
    }
}

/// An I/O error as the kernel's.
pub(crate) fn io_error(error: io::Error) -> Error {
    Error::from_io(error)
}

/// Where an archive location points: the archive file and the path inside
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    /// The archive's OS path.
    pub archive: PathBuf,
    /// The archive's own location path, the root of the locations inside it.
    pub root: VPath,
    /// The normalised path inside the archive; empty for the archive itself.
    pub inner: Vec<u8>,
}

impl Split {
    /// The location of the entry at `inner` in this archive.
    #[must_use]
    pub fn location_of(&self, inner: &[u8]) -> Location {
        if inner.is_empty() {
            Location::new(PROVIDER_ID, self.root.clone())
        } else {
            Location::new(PROVIDER_ID, self.root.join(inner))
        }
    }
}

/// Finds the archive in a location's path: the shortest prefix that is a
/// file on the local disk; what follows is the path inside it.
///
/// # Errors
///
/// `Error::Unsupported` for a location of another provider;
/// `Error::NotFound` when no prefix of the path is an existing file.
pub fn split(loc: &Location) -> Result<Split, Error> {
    if loc.provider != PROVIDER_ID {
        return Err(Error::Unsupported("location belongs to another provider"));
    }
    let bytes = loc.path.as_bytes();
    let mut boundaries: Vec<usize> = bytes
        .iter()
        .enumerate()
        .filter(|(_, b)| matches!(b, b'/' | b'\\'))
        .map(|(i, _)| i)
        .collect();
    boundaries.push(bytes.len());
    for end in boundaries {
        if end == 0 {
            continue;
        }
        let prefix = VPath::from_bytes(bytes[..end].to_vec());
        let Ok(path) = ext::platform::paths::to_path(&prefix) else {
            continue;
        };
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Ok(Split {
                    archive: path,
                    root: prefix,
                    inner: index::normalize(&bytes[end..]),
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(Error::from_io(error)),
        }
    }
    Err(Error::NotFound)
}

/// FNV-1a over `bytes`, for entry ids: stable across listings and
/// processes.
fn fnv(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// The id of an entry: the archive in the upper half, the path inside it
/// in the lower.
fn id_of(archive: &Path, inner: &[u8]) -> EntryId {
    let archive = fnv(archive.as_os_str().as_encoded_bytes());
    EntryId::new(
        PROVIDER_ID,
        (u128::from(archive) << 64) | u128::from(fnv(inner)),
    )
}

fn attrs_of(name: &str) -> Attrs {
    if name.starts_with('.') {
        Attrs::HIDDEN
    } else {
        Attrs::empty()
    }
}

/// An index with what it was read from, so a changed file is read again.
struct Cached {
    len: u64,
    modified: Option<SystemTime>,
    index: Arc<Index>,
}

/// The provider's state, shared with the writers it hands out.
#[derive(Default)]
struct Inner {
    cache: Mutex<HashMap<PathBuf, Cached>>,
    /// One lock per archive: rewrites of one file go one after the other.
    locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Plain data; a panic elsewhere leaves it usable.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The entries of the archive at `path`, read by its format.
fn read_index(path: &Path) -> Result<Index, Error> {
    let format = Format::detect(path)?;
    let items = match format {
        Format::Zip => zipfmt::index(path)?,
        Format::Tar(codec) => tarfmt::index(path, codec)?,
        Format::SevenZ => sevenz::index(path)?,
        Format::Rar => rar::index(path)?,
        Format::Image => image::index(path)?,
        Format::Single(codec) => single::index(path, codec),
    };
    Ok(Index::new(format, items))
}

/// Writes the entry at `inner` of the archive at `path`, decompressed,
/// into `sink`.
fn copy_out(path: &Path, format: Format, inner: &[u8], sink: &mut dyn Write) -> Result<(), Error> {
    match format {
        Format::Zip => zipfmt::copy_out(path, inner, sink),
        Format::Tar(codec) => tarfmt::copy_out(path, codec, inner, sink),
        Format::SevenZ => sevenz::copy_out(path, inner, sink),
        Format::Rar => rar::copy_out(path, inner, sink),
        Format::Image => image::copy_out(path, inner, sink),
        Format::Single(codec) => single::copy_out(path, codec, inner, sink),
    }
}

impl Inner {
    fn lock_for(&self, archive: &Path) -> Arc<Mutex<()>> {
        Arc::clone(lock(&self.locks).entry(archive.to_path_buf()).or_default())
    }

    /// The cached index of `archive`, when the file has not changed since.
    fn cached(&self, archive: &Path) -> Option<Arc<Index>> {
        let meta = std::fs::metadata(archive).ok()?;
        let cache = lock(&self.cache);
        let cached = cache.get(archive)?;
        (cached.len == meta.len() && cached.modified == meta.modified().ok())
            .then(|| Arc::clone(&cached.index))
    }

    fn invalidate(&self, archive: &Path) {
        lock(&self.cache).remove(archive);
    }

    /// The index of `archive`, read now when the cache has none or the
    /// file changed. Blocking: call on the blocking pool.
    fn index_sync(&self, archive: &Path) -> Result<Arc<Index>, Error> {
        if let Some(index) = self.cached(archive) {
            return Ok(index);
        }
        let meta = std::fs::metadata(archive).map_err(Error::from_io)?;
        let span = tracing::info_span!("vfs_archive.index", path = %archive.display());
        let index = {
            let _enter = span.enter();
            Arc::new(read_index(archive)?)
        };
        lock(&self.cache).insert(
            archive.to_path_buf(),
            Cached {
                len: meta.len(),
                modified: meta.modified().ok(),
                index: Arc::clone(&index),
            },
        );
        Ok(index)
    }

    /// Rewrites the archive as `plan` says, under its lock, into a
    /// temporary file beside it that then takes its place. Blocking.
    fn rewrite_sync(&self, split: &Split, plan: &Plan) -> Result<(), Error> {
        let guard = self.lock_for(&split.archive);
        let _held = lock(&guard);
        let index = self.index_sync(&split.archive)?;
        let format = index.format();
        if !format.writable() {
            return Err(Error::Unsupported("this archive format is read-only"));
        }
        let span = tracing::info_span!("vfs_archive.rewrite", path = %split.archive.display());
        let _enter = span.enter();
        let dir = split.archive.parent().unwrap_or_else(|| Path::new("."));
        let temp = tempfile::Builder::new()
            .prefix(".rick-")
            .tempfile_in(dir)
            .map_err(Error::from_io)?;
        {
            let mut out = temp.as_file().try_clone().map_err(Error::from_io)?;
            match format {
                Format::Zip => zipfmt::rewrite(&split.archive, &mut out, plan)?,
                Format::Tar(codec) => tarfmt::rewrite(&split.archive, codec, &mut out, plan)?,
                Format::SevenZ => sevenz::rewrite(&split.archive, &mut out, plan)?,
                Format::Rar | Format::Image | Format::Single(_) => {
                    return Err(Error::Unsupported("this archive format is read-only"));
                }
            }
            out.sync_all().map_err(Error::from_io)?;
        }
        if let Ok(meta) = std::fs::metadata(&split.archive) {
            let _ = std::fs::set_permissions(temp.path(), meta.permissions());
        }
        temp.persist(&split.archive)
            .map_err(|error| Error::from_io(error.error))?;
        self.invalidate(&split.archive);
        Ok(())
    }
}

/// Archives browsed in place.
#[derive(Default)]
pub struct ArchiveProvider {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ArchiveProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveProvider")
            .field("cached", &lock(&self.inner.cache).len())
            .finish()
    }
}

impl ArchiveProvider {
    /// A provider with an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The index of an archive, read on the blocking pool when needed.
    async fn index_of(&self, archive: &Path) -> Result<Arc<Index>, Error> {
        let inner = Arc::clone(&self.inner);
        let archive = archive.to_path_buf();
        ext::blocking(move || inner.index_sync(&archive)).await
    }

    /// Rewrites an archive on the blocking pool.
    async fn rewrite(&self, split: &Split, plan: Plan) -> Result<(), Error> {
        let inner = Arc::clone(&self.inner);
        let split = split.clone();
        ext::blocking(move || inner.rewrite_sync(&split, &plan)).await
    }

    /// The entry for an item of an archive.
    fn entry_of(split: &Split, item: &Item) -> Entry {
        let name = String::from_utf8_lossy(item.name()).into_owned();
        Entry {
            id: id_of(&split.archive, &item.path),
            path: split.location_of(&item.path).path,
            kind: item.kind.clone(),
            size: item.size,
            created: None,
            modified: item.modified,
            accessed: None,
            attrs: attrs_of(&name),
            mime: None,
            name,
        }
    }

    /// The entry for the archive itself: a folder whose size and time are
    /// the file's, so a poll of `stat` notices a rewrite.
    fn root_entry(split: &Split) -> Result<Entry, Error> {
        let meta = std::fs::metadata(&split.archive).map_err(Error::from_io)?;
        let name = split
            .archive
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Entry {
            id: id_of(&split.archive, b""),
            path: split.root.clone(),
            kind: EntryKind::Dir,
            size: Some(meta.len()),
            created: meta.created().ok(),
            modified: meta.modified().ok(),
            accessed: meta.accessed().ok(),
            attrs: attrs_of(&name),
            mime: None,
            name,
        })
    }

    /// The format of the archive at a location, from the cache; `None`
    /// before the archive was ever listed.
    fn cached_format(&self, loc: &Location) -> Option<Format> {
        let split = split(loc).ok()?;
        self.inner.cached(&split.archive).map(|i| i.format())
    }

    /// The split and index of a location whose archive takes writes.
    async fn writable(&self, loc: &Location) -> Result<(Split, Arc<Index>), Error> {
        let split = split(loc)?;
        let index = self.index_of(&split.archive).await?;
        if !index.format().writable() {
            return Err(Error::Unsupported("this archive format is read-only"));
        }
        if split.inner.is_empty() {
            return Err(Error::Unsupported(
                "the archive itself cannot be changed here",
            ));
        }
        Ok((split, index))
    }
}

fn already_exists() -> Error {
    Error::Io(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "an entry with that name is in the archive",
    ))
}

#[async_trait]
impl Provider for ArchiveProvider {
    fn id(&self) -> ProviderId {
        PROVIDER_ID
    }

    /// Writable for zip, the tar family and 7z once the archive has been
    /// listed; nothing for RAR, disc images and single files.
    fn capabilities(&self, loc: &Location) -> Capabilities {
        match self.cached_format(loc) {
            Some(format) if format.writable() => Capabilities::WRITE | Capabilities::RENAME,
            _ => Capabilities::empty(),
        }
    }

    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken) -> EntryStream {
        let split = match split(loc) {
            Ok(split) => split,
            Err(error) => return stream::once(async move { Err(error) }).boxed(),
        };
        let inner = Arc::clone(&self.inner);
        stream::once(async move {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let archive = split.archive.clone();
            let index = ext::blocking(move || inner.index_sync(&archive)).await?;
            if !split.inner.is_empty()
                && index
                    .get(&split.inner)
                    .is_none_or(|i| i.kind != EntryKind::Dir)
            {
                return Err(if index.contains(&split.inner) {
                    Error::Unsupported("not a folder")
                } else {
                    Error::NotFound
                });
            }
            let items = match opts.depth {
                Depth::One => index.children(&split.inner),
                Depth::Unlimited => index.descendants(&split.inner),
            };
            Ok(items
                .into_iter()
                .map(|item| Self::entry_of(&split, item))
                .filter(|entry| opts.hidden || !entry.attrs.contains(Attrs::HIDDEN))
                .collect())
        })
        .boxed()
    }

    async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
        let split = split(loc)?;
        let index = self.index_of(&split.archive).await?;
        if split.inner.is_empty() {
            return Self::root_entry(&split);
        }
        index
            .get(&split.inner)
            .map(|item| Self::entry_of(&split, item))
            .ok_or(Error::NotFound)
    }

    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
        let split = split(loc)?;
        let index = self.index_of(&split.archive).await?;
        let item = index.get(&split.inner).ok_or(Error::NotFound)?;
        if item.kind == EntryKind::Dir {
            return Err(Error::Unsupported("a folder has no contents to read"));
        }
        let format = index.format();
        Ok(bridge::reader(move |sink| {
            copy_out(&split.archive, format, &split.inner, sink)
                .map_err(|error| io::Error::other(error.to_string()))
        }))
    }

    /// A writer whose bytes go into the archive when it is shut down: the
    /// entry is spooled to a temporary file and the archive rewritten with
    /// it, replacing an existing entry only with `opts.overwrite`.
    async fn open_write(&self, loc: &Location, opts: WriteOptions) -> Result<WriteHandle, Error> {
        let (split, index) = self.writable(loc).await?;
        match index.get(&split.inner) {
            Some(item) if item.kind == EntryKind::Dir => {
                return Err(Error::Unsupported("a folder cannot be written as a file"));
            }
            Some(_) if !opts.overwrite => return Err(already_exists()),
            _ => {}
        }
        let temp = tempfile::NamedTempFile::new().map_err(Error::from_io)?;
        let file = temp.reopen().map_err(Error::from_io)?;
        Ok(Box::new(ArchiveWriter {
            file: Some(tokio::fs::File::from_std(file)),
            temp: Some(temp.into_temp_path()),
            inner: Arc::clone(&self.inner),
            split,
            finishing: None,
            done: false,
        }))
    }

    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error> {
        let (split, index) = self.writable(loc).await?;
        if index.contains(&split.inner) {
            return Err(already_exists());
        }
        self.rewrite(
            &split,
            Plan::one(Edit::AddDir {
                path: split.inner.clone(),
            }),
        )
        .await?;
        self.stat(loc).await
    }

    async fn rename(&self, from: &Location, to: &Location) -> Result<Entry, Error> {
        let (source, index) = self.writable(from).await?;
        let target = split(to)?;
        if target.archive != source.archive {
            return Err(Error::Unsupported(
                "an entry moves within its archive; copy it out otherwise",
            ));
        }
        if target.inner.is_empty() {
            return Err(Error::Unsupported("the archive root is not an entry"));
        }
        if !index.contains(&source.inner) {
            return Err(Error::NotFound);
        }
        if index.contains(&target.inner) {
            return Err(already_exists());
        }
        self.rewrite(
            &source,
            Plan::one(Edit::Rename {
                from: source.inner.clone(),
                to: target.inner.clone(),
            }),
        )
        .await?;
        self.stat(to).await
    }

    /// Removes an entry for good; an archive has no trash.
    async fn remove(&self, loc: &Location, mode: RemoveMode) -> Result<(), Error> {
        if mode == RemoveMode::Trash {
            return Err(Error::Unsupported("an archive has no trash"));
        }
        let (split, index) = self.writable(loc).await?;
        if !index.contains(&split.inner) {
            return Err(Error::NotFound);
        }
        self.rewrite(&split, Plan::one(Edit::Remove(split.inner.clone())))
            .await
    }

    async fn link(
        &self,
        _target: &Location,
        _at: &Location,
        _kind: LinkKind,
    ) -> Result<Entry, Error> {
        Err(Error::Unsupported("links are not made inside archives"))
    }

    /// Copies one file within an archive: out to a temporary file, then
    /// back in under the new name in one rewrite.
    async fn copy(
        &self,
        from: &Location,
        to: &Location,
        opts: CopyOptions,
        _progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        let (source, index) = self.writable(from).await?;
        let target = split(to)?;
        if target.archive != source.archive {
            return Err(Error::Unsupported(
                "copies between archives stream through the kernel",
            ));
        }
        let item = index.get(&source.inner).ok_or(Error::NotFound)?;
        if item.kind != EntryKind::File {
            return Err(Error::Unsupported("copy folders through vfs::copy"));
        }
        if index.contains(&target.inner) && !opts.overwrite {
            return Err(already_exists());
        }
        let modified = item.modified.unwrap_or_else(SystemTime::now);
        let format = index.format();
        let inner = Arc::clone(&self.inner);
        ext::blocking(move || {
            let mut temp = tempfile::NamedTempFile::new().map_err(Error::from_io)?;
            copy_out(&source.archive, format, &source.inner, temp.as_file_mut())?;
            temp.as_file_mut().flush().map_err(Error::from_io)?;
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            inner.rewrite_sync(
                &target,
                &Plan::one(Edit::AddFile {
                    path: target.inner.clone(),
                    source: temp.path().to_path_buf(),
                    modified,
                }),
            )
        })
        .await
    }

    fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
        Err(Error::Unsupported("archives are polled, not watched"))
    }
}

/// The writer `open_write` hands out: a temporary file while bytes arrive,
/// the rewrite of the archive when it is shut down.
struct ArchiveWriter {
    file: Option<tokio::fs::File>,
    temp: Option<tempfile::TempPath>,
    inner: Arc<Inner>,
    split: Split,
    finishing: Option<tokio::task::JoinHandle<Result<(), Error>>>,
    done: bool,
}

impl AsyncWrite for ArchiveWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.file.as_mut() {
            Some(file) => Pin::new(file).poll_write(cx, buf),
            None => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the archive entry was already written",
            ))),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.file.as_mut() {
            Some(file) => Pin::new(file).poll_flush(cx),
            None => Poll::Ready(Ok(())),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.done {
            return Poll::Ready(Ok(()));
        }
        if self.finishing.is_none() {
            if let Some(file) = self.file.as_mut() {
                match Pin::new(file).poll_shutdown(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(())) => {}
                }
            }
            self.file = None;
            let Some(temp) = self.temp.take() else {
                self.done = true;
                return Poll::Ready(Ok(()));
            };
            let inner = Arc::clone(&self.inner);
            let split = self.split.clone();
            self.finishing = Some(ext::runtime::handle().spawn_blocking(move || {
                let plan = Plan::one(Edit::AddFile {
                    path: split.inner.clone(),
                    source: temp.to_path_buf(),
                    modified: SystemTime::now(),
                });
                let outcome = inner.rewrite_sync(&split, &plan);
                drop(temp);
                outcome
            }));
        }
        let Some(finishing) = self.finishing.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        match Pin::new(finishing).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(joined) => {
                self.done = true;
                Poll::Ready(match joined {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(error)) => Err(io::Error::other(error.to_string())),
                    Err(join) => Err(io::Error::other(join)),
                })
            }
        }
    }
}

/// The extension: one provider.
#[derive(Debug)]
pub struct ArchiveExtension {
    manifest: Manifest,
}

impl Default for ArchiveExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl ArchiveExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Archives",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PROVIDER,
                platforms: Platforms::ALL,
            },
        }
    }
}

impl Extension for ArchiveExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.provider(Arc::new(ArchiveProvider::new()));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(ArchiveExtension::new()) as Arc<dyn Extension>);

#[cfg(test)]
mod tests;
