//! The virtual filesystem: which provider owns a location, and the
//! operations that span providers.
//!
//! [`Mounts`] maps a [`ProviderId`] to its [`Provider`]; a [`Location`]
//! carries the id, so routing is a lookup. [`copy`] and [`move_entry`] are
//! what `ops` runs: within one provider they use the provider's own fast
//! path (`Provider::copy`, `Provider::rename`); across providers, or when a
//! rename crosses devices, they stream through `open_read` and `open_write`,
//! walking directories with `list` and `create_dir`, reporting bytes and
//! stopping within one buffer of cancellation.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use ext::{CopyOptions, Depth, ListOptions, Provider, Registry, RemoveMode, WriteOptions};
use futures::StreamExt;
use runtime::{PauseGate, ProgressSink};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use types::{CancelToken, Entry, EntryKind, Error, Location, Progress, ProviderId, VPath};

/// Bytes moved per read when streaming between providers.
const BUFFER: usize = 1 << 20;

/// The provider table.
#[derive(Default)]
pub struct Mounts {
    providers: RwLock<HashMap<ProviderId, Arc<dyn Provider>>>,
}

impl Mounts {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A table holding every provider the registry collected.
    #[must_use]
    pub fn from_registry(registry: &Registry) -> Self {
        let mounts = Self::new();
        for provider in registry.providers() {
            mounts.mount(Arc::clone(provider));
        }
        mounts
    }

    fn table(&self) -> std::sync::RwLockReadGuard<'_, HashMap<ProviderId, Arc<dyn Provider>>> {
        // The map holds plain handles; a panic elsewhere cannot corrupt it.
        self.providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Adds a provider under its own id, replacing one with the same id.
    pub fn mount(&self, provider: Arc<dyn Provider>) {
        self.providers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(provider.id(), provider);
    }

    /// The provider with this id, if mounted.
    #[must_use]
    pub fn provider(&self, id: ProviderId) -> Option<Arc<dyn Provider>> {
        self.table().get(&id).cloned()
    }

    /// The provider that owns `loc`.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when no provider with the location's id is
    /// mounted.
    pub fn resolve(&self, loc: &Location) -> Result<Arc<dyn Provider>, Error> {
        self.provider(loc.provider)
            .ok_or(Error::Unsupported("no provider mounted for this location"))
    }

    /// Ids of the mounted providers, in no particular order.
    #[must_use]
    pub fn ids(&self) -> Vec<ProviderId> {
        self.table().keys().copied().collect()
    }
}

impl std::fmt::Debug for Mounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mounts").field("ids", &self.ids()).finish()
    }
}

/// Copies an entry, recursively for directories, into `to`.
///
/// Within one provider every file goes through the provider's own `copy`,
/// which uses the native fast path. Across providers bytes are streamed
/// through `open_read` and `open_write`. Progress is reported in bytes over
/// the whole tree; cancellation ends the copy within one buffer, leaving
/// what was copied so far in place; a paused `pause` gate holds the copy
/// before the next directory, file or streamed buffer.
///
/// # Errors
///
/// `Error::Cancelled` when cancelled; the provider's error otherwise. A
/// target that exists is an `AlreadyExists` I/O error unless
/// `opts.overwrite` is set.
pub async fn copy(
    mounts: &Mounts,
    from: &Location,
    to: &Location,
    opts: CopyOptions,
    progress: ProgressSink,
    cancel: CancelToken,
    pause: &PauseGate,
) -> Result<(), Error> {
    let src = mounts.resolve(from)?;
    let dst = mounts.resolve(to)?;
    let root = src.stat(from).await?;
    let plan = Plan::walk(&src, from, &root, &cancel).await?;
    tracing::debug!(files = plan.files.len(), bytes = plan.total, "copy planned");
    let mut done = 0u64;
    progress.report(Progress::Bytes {
        done,
        total: Some(plan.total),
    });

    for dir in &plan.dirs {
        pause.wait().await;
        check(&cancel)?;
        let target = Location::new(to.provider, relocate(&dir.path, &from.path, &to.path));
        match dst.create_dir(&target).await {
            Ok(_) => {}
            Err(Error::Io(e))
                if e.kind() == std::io::ErrorKind::AlreadyExists && opts.overwrite => {}
            Err(e) => return Err(e),
        }
    }
    for file in &plan.files {
        pause.wait().await;
        check(&cancel)?;
        let source = Location::new(from.provider, file.path.clone());
        let target = Location::new(to.provider, relocate(&file.path, &from.path, &to.path));
        let size = file.size.unwrap_or(0);
        match &file.kind {
            EntryKind::Symlink {
                target: link_target,
                ..
            } => {
                dst.link(
                    &Location::new(to.provider, link_target.clone()),
                    &target,
                    ext::LinkKind::Symbolic,
                )
                .await?;
            }
            _ if Arc::ptr_eq(&src, &dst) || src.id() == dst.id() => {
                copy_native(
                    &src, &source, &target, opts, &progress, &cancel, done, plan.total,
                )
                .await?;
            }
            _ => {
                stream_copy(
                    &src, &dst, &source, &target, opts, &progress, &cancel, pause, done, plan.total,
                )
                .await?;
            }
        }
        done += size;
        progress.report(Progress::Bytes {
            done,
            total: Some(plan.total),
        });
    }
    Ok(())
}

/// Moves an entry into `to`.
///
/// Within one provider this is `rename`; when the rename crosses devices, or
/// the providers differ, it is a [`copy`] followed by permanent removal of
/// the source.
///
/// # Errors
///
/// As for [`copy`]; a failed copy leaves the source in place.
pub async fn move_entry(
    mounts: &Mounts,
    from: &Location,
    to: &Location,
    opts: CopyOptions,
    progress: ProgressSink,
    cancel: CancelToken,
    pause: &PauseGate,
) -> Result<(), Error> {
    let src = mounts.resolve(from)?;
    if from.provider == to.provider {
        match src.rename(from, to).await {
            Ok(_) => return Ok(()),
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::CrossesDevices => {
                tracing::debug!("rename crosses devices; copying instead");
            }
            Err(e) => return Err(e),
        }
    }
    copy(mounts, from, to, opts, progress, cancel.clone(), pause).await?;
    check(&cancel)?;
    src.remove(from, RemoveMode::Permanent).await
}

fn check(cancel: &CancelToken) -> Result<(), Error> {
    if cancel.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// `path` with the `from` prefix swapped for `to`.
fn relocate(path: &VPath, from: &VPath, to: &VPath) -> VPath {
    match path.as_bytes().strip_prefix(from.as_bytes()) {
        Some(rest) => {
            let rest = rest
                .strip_prefix(b"/")
                .or_else(|| rest.strip_prefix(b"\\"))
                .unwrap_or(rest);
            if rest.is_empty() {
                to.clone()
            } else {
                to.join(rest)
            }
        }
        None => to.join(path.file_name().unwrap_or_default()),
    }
}

/// Everything under the source, decided before anything is written.
struct Plan {
    /// Directories in creation order, parents first.
    dirs: Vec<Entry>,
    /// Files and links, in listing order.
    files: Vec<Entry>,
    total: u64,
}

impl Plan {
    async fn walk(
        src: &Arc<dyn Provider>,
        from: &Location,
        root: &Entry,
        cancel: &CancelToken,
    ) -> Result<Self, Error> {
        let mut plan = Self {
            dirs: Vec::new(),
            files: Vec::new(),
            total: 0,
        };
        let mut pending = vec![root.clone()];
        while let Some(entry) = pending.pop() {
            check(cancel)?;
            match entry.kind {
                EntryKind::Dir => {
                    let loc = Location::new(from.provider, entry.path.clone());
                    let mut batches = src.list(
                        &loc,
                        ListOptions {
                            hidden: true,
                            system: true,
                            depth: Depth::One,
                        },
                        cancel.clone(),
                    );
                    let mut children = Vec::new();
                    while let Some(batch) = batches.next().await {
                        children.extend(batch?);
                    }
                    plan.dirs.push(entry);
                    // Reverse so siblings come off the stack in listing order.
                    pending.extend(children.into_iter().rev());
                }
                EntryKind::File | EntryKind::Symlink { .. } => {
                    plan.total += entry.size.unwrap_or(0);
                    plan.files.push(entry);
                }
                EntryKind::Other => {
                    return Err(Error::Unsupported("cannot copy a special file"));
                }
            }
        }
        Ok(plan)
    }
}

#[allow(clippy::too_many_arguments)]
async fn copy_native(
    provider: &Arc<dyn Provider>,
    from: &Location,
    to: &Location,
    opts: CopyOptions,
    progress: &ProgressSink,
    cancel: &CancelToken,
    base: u64,
    total: u64,
) -> Result<(), Error> {
    let (sink, mut sub) = ProgressSink::channel();
    let forward = progress.clone();
    let relay = runtime::handle().spawn(async move {
        while sub.changed().await.is_ok() {
            if let Progress::Bytes { done, .. } = *sub.borrow_and_update() {
                forward.report(Progress::Bytes {
                    done: base + done,
                    total: Some(total),
                });
            }
        }
    });
    let result = provider.copy(from, to, opts, sink, cancel.clone()).await;
    relay.abort();
    result
}

#[allow(clippy::too_many_arguments)]
async fn stream_copy(
    src: &Arc<dyn Provider>,
    dst: &Arc<dyn Provider>,
    from: &Location,
    to: &Location,
    opts: CopyOptions,
    progress: &ProgressSink,
    cancel: &CancelToken,
    pause: &PauseGate,
    base: u64,
    total: u64,
) -> Result<(), Error> {
    let mut reader = src.open_read(from).await?;
    let mut writer = dst
        .open_write(
            to,
            WriteOptions {
                overwrite: opts.overwrite,
            },
        )
        .await?;
    let mut buffer = vec![0u8; BUFFER];
    let mut done = 0u64;
    loop {
        pause.wait().await;
        check(cancel)?;
        let read = reader.read(&mut buffer).await.map_err(Error::from_io)?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .await
            .map_err(Error::from_io)?;
        done += read as u64;
        progress.report(Progress::Bytes {
            done: base + done,
            total: Some(total),
        });
    }
    writer.shutdown().await.map_err(Error::from_io)?;
    Ok(())
}

#[cfg(test)]
mod tests;
