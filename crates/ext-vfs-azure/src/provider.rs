//! The provider: accounts at the root, containers below each, blob name
//! prefixes as folders below those.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use bytes::Bytes;
use ext::{
    Capabilities, ChangeStream, CopyOptions, EntryStream, LinkKind, ListOptions, ProgressSink,
    Provider, ReadHandle, RemoveMode, TransferMode, WriteHandle, WriteOptions, async_trait,
};
use futures::{StreamExt, stream};
use tokio::io::{AsyncReadExt, AsyncWrite};
use tracing::debug;
use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, Location, Progress, VPath};

use crate::client::{AzureClient, Blob, CopyState};
use crate::connection::AzureConnections;
use crate::{PROVIDER_ID, Place};

/// The size of one block of an upload; a blob smaller than this is
/// written with one `PUT`.
const BLOCK: usize = 8 * 1024 * 1024;

/// How much the upload pipe holds before the writer waits.
const PIPE: usize = 4 * 1024 * 1024;

/// How often a pending copy is polled.
const POLL: Duration = Duration::from_millis(500);

/// What the provider can do inside a container.
const IN_CONTAINER: Capabilities = Capabilities::REMOTE
    .union(Capabilities::WRITE)
    .union(Capabilities::NATIVE_TRANSFER);

/// FNV-1a over `bytes`: the key of an entry, stable across listings and
/// processes.
fn key_of(bytes: &[u8]) -> u128 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    u128::from(hash)
}

fn already_exists() -> Error {
    Error::Io(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
}

/// The last segment of a name or prefix, without a trailing slash.
fn leaf(name: &str) -> &str {
    name.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(name)
}

/// A folder entry at `place`, a container or a prefix.
fn folder(place: &Place, name: &str, modified: Option<SystemTime>) -> Entry {
    let loc = place.location();
    let mut attrs = Attrs::empty();
    if name.starts_with('.') {
        attrs |= Attrs::HIDDEN;
    }
    Entry {
        id: EntryId::new(PROVIDER_ID, key_of(loc.path.as_bytes())),
        name: name.to_owned(),
        path: loc.path,
        kind: EntryKind::Dir,
        size: None,
        created: None,
        modified,
        accessed: None,
        attrs,
        mime: None,
    }
}

/// A file entry for a blob at `place`.
fn file(place: &Place, blob: &Blob) -> Entry {
    let loc = place.location();
    let name = leaf(&blob.name).to_owned();
    let mut attrs = Attrs::empty();
    if name.starts_with('.') {
        attrs |= Attrs::HIDDEN;
    }
    Entry {
        id: EntryId::new(PROVIDER_ID, key_of(loc.path.as_bytes())),
        name,
        path: loc.path,
        kind: EntryKind::File,
        size: Some(blob.size),
        created: None,
        modified: blob.modified,
        accessed: None,
        attrs,
        mime: None,
    }
}

fn root_entry() -> Entry {
    Entry {
        id: EntryId::new(PROVIDER_ID, 0),
        name: "Azure Blob".to_owned(),
        path: VPath::from("/"),
        kind: EntryKind::Dir,
        size: None,
        created: None,
        modified: None,
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    }
}

/// The prefix the children of a container or a folder share: empty for
/// the container, `name/` for a folder.
fn prefix_of(place: &Place) -> Option<(&str, &str, String)> {
    match place {
        Place::Container { account, container } => Some((account, container, String::new())),
        Place::Blob {
            account,
            container,
            name,
        } => Some((account, container, format!("{name}/"))),
        _ => None,
    }
}

/// The states of a streamed listing.
enum Listing {
    Page {
        client: Arc<AzureClient>,
        account: String,
        container: String,
        prefix: String,
        marker: Option<String>,
    },
    Done,
}

/// What a name stands for on the service.
enum Kind {
    Blob(Blob),
    Folder,
}

/// The `azure` provider.
pub struct AzureProvider {
    conns: Arc<AzureConnections>,
}

impl std::fmt::Debug for AzureProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AzureProvider")
            .field("connections", &self.conns.names())
            .finish()
    }
}

impl AzureProvider {
    /// A provider over `conns`.
    #[must_use]
    pub fn new(conns: Arc<AzureConnections>) -> Self {
        Self { conns }
    }

    /// Whether `name` is a blob or a folder of blobs in `container`.
    async fn kind(client: &AzureClient, container: &str, name: &str) -> Result<Kind, Error> {
        match client.head(container, name).await {
            Ok((blob, _)) => Ok(Kind::Blob(blob)),
            Err(Error::NotFound) => {
                let page = client
                    .list(container, &format!("{name}/"), None, Some("1"))
                    .await?;
                if page.prefixes.is_empty() && page.blobs.is_empty() {
                    Err(Error::NotFound)
                } else {
                    Ok(Kind::Folder)
                }
            }
            Err(e) => Err(e),
        }
    }

    /// The entries of one listing page.
    pub(crate) fn page_entries(
        account: &str,
        container: &str,
        prefix: &str,
        page: crate::client::Page,
        hidden: bool,
    ) -> Vec<Entry> {
        let mut out = Vec::with_capacity(page.prefixes.len() + page.blobs.len());
        for common in page.prefixes {
            let place = Place::Blob {
                account: account.to_owned(),
                container: container.to_owned(),
                name: common.trim_end_matches('/').to_owned(),
            };
            let entry = folder(&place, leaf(&common), None);
            if hidden || !entry.attrs.contains(Attrs::HIDDEN) {
                out.push(entry);
            }
        }
        for blob in page.blobs {
            if blob.name == prefix || blob.name.ends_with('/') {
                continue;
            }
            let place = Place::Blob {
                account: account.to_owned(),
                container: container.to_owned(),
                name: blob.name.clone(),
            };
            let entry = file(&place, &blob);
            if hidden || !entry.attrs.contains(Attrs::HIDDEN) {
                out.push(entry);
            }
        }
        out
    }

    /// Every blob under an item, with the name each lands at under
    /// `to_prefix`.
    async fn plan(
        client: &AzureClient,
        container: &str,
        name: &str,
        to_prefix: &str,
    ) -> Result<Vec<(Blob, String)>, Error> {
        let last = leaf(name);
        match Self::kind(client, container, name).await? {
            Kind::Blob(blob) => Ok(vec![(blob, format!("{to_prefix}{last}"))]),
            Kind::Folder => {
                let below = client.list_all(container, &format!("{name}/")).await?;
                Ok(below
                    .into_iter()
                    .map(|blob| {
                        let rest = &blob.name[name.len()..];
                        let target = format!("{to_prefix}{last}{rest}");
                        (blob, target)
                    })
                    .collect())
            }
        }
    }

    /// Copies one blob on the service and follows it to the end: a
    /// pending copy is polled for its progress, reported on top of `base`,
    /// and aborted when `cancel` fires.
    async fn copy_blob(
        client: &AzureClient,
        from: (&str, &str),
        to: (&str, &str),
        base: u64,
        total: u64,
        progress: &ProgressSink,
        cancel: &CancelToken,
    ) -> Result<(), Error> {
        let mut state: CopyState = client.copy(from.0, from.1, to.0, to.1).await?;
        loop {
            match state.status.as_str() {
                "success" => return Ok(()),
                "pending" => {}
                other => {
                    return Err(crate::client::message(format!(
                        "the copy of {} ended {other}",
                        from.1
                    )));
                }
            }
            if cancel.is_cancelled() {
                let _ = client.abort_copy(to.0, to.1, &state.id).await;
                let _ = client.delete(to.0, to.1).await;
                return Err(Error::Cancelled);
            }
            if let Some((done, _)) = state.progress {
                progress.report(Progress::Bytes {
                    done: base + done,
                    total: Some(total),
                });
            }
            tokio::time::sleep(POLL).await;
            let (_, copy) = client.head(to.0, to.1).await?;
            state = copy.unwrap_or(CopyState {
                id: state.id,
                status: "success".to_owned(),
                progress: None,
            });
        }
    }
}

#[async_trait]
impl Provider for AzureProvider {
    fn id(&self) -> types::ProviderId {
        PROVIDER_ID
    }

    fn capabilities(&self, loc: &Location) -> Capabilities {
        match Place::of(loc) {
            Place::Root => Capabilities::REMOTE,
            Place::Account(_) => Capabilities::REMOTE | Capabilities::WRITE,
            Place::Container { .. } | Place::Blob { .. } => IN_CONTAINER,
        }
    }

    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken) -> EntryStream {
        let place = Place::of(loc);
        let conns = Arc::clone(&self.conns);
        let hidden = opts.hidden;
        match place {
            Place::Root => {
                let entries: Vec<Entry> = conns
                    .names()
                    .iter()
                    .map(|name| folder(&Place::Account(name.clone()), name, None))
                    .collect();
                stream::once(async move { Ok(entries) }).boxed()
            }
            Place::Account(account) => stream::once(async move {
                let client = conns.client(&account).await?;
                let containers = client.list_containers().await?;
                Ok(containers
                    .into_iter()
                    .map(|c| {
                        folder(
                            &Place::Container {
                                account: account.clone(),
                                container: c.name.clone(),
                            },
                            &c.name,
                            c.modified,
                        )
                    })
                    .collect())
            })
            .boxed(),
            Place::Container { .. } | Place::Blob { .. } => {
                let Some((account, container, prefix)) = prefix_of(&place) else {
                    return stream::once(async { Err(Error::NotFound) }).boxed();
                };
                let (account, container) = (account.to_owned(), container.to_owned());
                stream::once(async move {
                    let client = conns.client(&account).await?;
                    Ok(Listing::Page {
                        client,
                        account,
                        container,
                        prefix,
                        marker: None,
                    })
                })
                .flat_map(move |start| {
                    let cancel = cancel.clone();
                    let start = match start {
                        Ok(state) => state,
                        Err(error) => {
                            return stream::once(async move { Err(error) }).boxed();
                        }
                    };
                    stream::unfold(start, move |state| {
                        let cancel = cancel.clone();
                        async move {
                            let Listing::Page {
                                client,
                                account,
                                container,
                                prefix,
                                marker,
                            } = state
                            else {
                                return None;
                            };
                            if cancel.is_cancelled() {
                                return Some((Err(Error::Cancelled), Listing::Done));
                            }
                            match client
                                .list(&container, &prefix, marker.as_deref(), None)
                                .await
                            {
                                Ok(page) => {
                                    let next = page.next.clone();
                                    let entries = Self::page_entries(
                                        &account, &container, &prefix, page, hidden,
                                    );
                                    let state = match next {
                                        Some(marker) => Listing::Page {
                                            client,
                                            account,
                                            container,
                                            prefix,
                                            marker: Some(marker),
                                        },
                                        None => Listing::Done,
                                    };
                                    Some((Ok(entries), state))
                                }
                                Err(error) => Some((Err(error), Listing::Done)),
                            }
                        }
                    })
                    .boxed()
                })
                .boxed()
            }
        }
    }

    async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
        let place = Place::of(loc);
        match &place {
            Place::Root => Ok(root_entry()),
            Place::Account(account) => {
                if self.conns.names().contains(account) {
                    Ok(folder(&place, account, None))
                } else {
                    Err(Error::NotFound)
                }
            }
            Place::Container { account, container } => {
                let client = self.conns.client(account).await?;
                client.head_container(container).await?;
                Ok(folder(&place, container, None))
            }
            Place::Blob {
                account,
                container,
                name,
            } => {
                let client = self.conns.client(account).await?;
                match Self::kind(&client, container, name).await? {
                    Kind::Blob(blob) => Ok(file(&place, &blob)),
                    Kind::Folder => Ok(folder(&place, leaf(name), None)),
                }
            }
        }
    }

    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
        let Place::Blob {
            account,
            container,
            name,
        } = Place::of(loc)
        else {
            return Err(Error::Unsupported("only a blob can be read"));
        };
        let client = self.conns.client(&account).await?;
        client.get(&container, &name).await
    }

    async fn open_write(&self, loc: &Location, opts: WriteOptions) -> Result<WriteHandle, Error> {
        let Place::Blob {
            account,
            container,
            name,
        } = Place::of(loc)
        else {
            return Err(Error::Unsupported("only a blob can be written"));
        };
        let client = self.conns.client(&account).await?;
        if !opts.overwrite && client.head(&container, &name).await.is_ok() {
            return Err(already_exists());
        }
        Ok(Box::new(Upload::start(client, container, name)))
    }

    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error> {
        let place = Place::of(loc);
        match &place {
            Place::Root | Place::Account(_) => Err(Error::Unsupported(
                "an account is added from the side panel",
            )),
            Place::Container { account, container } => {
                let client = self.conns.client(account).await?;
                if client.head_container(container).await.is_ok() {
                    return Err(already_exists());
                }
                client.create_container(container).await?;
                Ok(folder(&place, container, Some(SystemTime::now())))
            }
            Place::Blob {
                account,
                container,
                name,
            } => {
                let client = self.conns.client(account).await?;
                if Self::kind(&client, container, name).await.is_ok() {
                    return Err(already_exists());
                }
                client
                    .put(container, &format!("{name}/"), Bytes::new())
                    .await?;
                Ok(folder(&place, leaf(name), None))
            }
        }
    }

    async fn rename(&self, _from: &Location, _to: &Location) -> Result<Entry, Error> {
        Err(Error::Unsupported(
            "blob storage has no rename; a move copies then deletes",
        ))
    }

    async fn remove(&self, loc: &Location, mode: RemoveMode) -> Result<(), Error> {
        if mode == RemoveMode::Trash {
            return Err(Error::Unsupported("blob storage has no trash"));
        }
        match Place::of(loc) {
            Place::Root | Place::Account(_) => Err(Error::Unsupported(
                "an account is removed from the side panel",
            )),
            Place::Container { account, container } => {
                let client = self.conns.client(&account).await?;
                client.delete_container(&container).await
            }
            Place::Blob {
                account,
                container,
                name,
            } => {
                let client = self.conns.client(&account).await?;
                match Self::kind(&client, &container, &name).await? {
                    Kind::Blob(_) => client.delete(&container, &name).await,
                    Kind::Folder => {
                        for blob in client.list_all(&container, &format!("{name}/")).await? {
                            client.delete(&container, &blob.name).await?;
                        }
                        Ok(())
                    }
                }
            }
        }
    }

    async fn link(
        &self,
        _target: &Location,
        _at: &Location,
        _kind: LinkKind,
    ) -> Result<Entry, Error> {
        Err(Error::Unsupported("blob storage has no links"))
    }

    /// One blob copied on the service, within the account.
    async fn copy(
        &self,
        from: &Location,
        to: &Location,
        opts: CopyOptions,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        let (
            Place::Blob {
                account,
                container,
                name,
            },
            Place::Blob {
                account: to_account,
                container: to_container,
                name: to_name,
            },
        ) = (Place::of(from), Place::of(to))
        else {
            return Err(Error::Unsupported("only blobs can be copied"));
        };
        if account != to_account {
            return Err(Error::Unsupported(
                "a copy across accounts streams through the kernel",
            ));
        }
        let client = self.conns.client(&account).await?;
        if !opts.overwrite && client.head(&to_container, &to_name).await.is_ok() {
            return Err(already_exists());
        }
        let (blob, _) = client.head(&container, &name).await?;
        Self::copy_blob(
            &client,
            (&container, &name),
            (&to_container, &to_name),
            0,
            blob.size,
            &progress,
            &cancel,
        )
        .await?;
        progress.report(Progress::Bytes {
            done: blob.size,
            total: Some(blob.size),
        });
        Ok(())
    }

    /// Moves or copies blobs and folders within the account, across
    /// containers too, through *Copy Blob*: nothing streams through the
    /// machine. Progress follows the service's own copy progress;
    /// cancelling aborts a pending copy and stops before the next blob.
    async fn transfer(
        &self,
        items: &[Location],
        to: &Location,
        mode: TransferMode,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        let to_place = Place::of(to);
        let Some((account, to_container, to_prefix)) = prefix_of(&to_place) else {
            return Err(Error::Unsupported(
                "blobs land in a container or a folder of one",
            ));
        };
        let client = self.conns.client(account).await?;
        let mut plan: Vec<(String, Blob, String)> = Vec::new();
        for item in items {
            let Place::Blob {
                account: item_account,
                container,
                name,
            } = Place::of(item)
            else {
                return Err(Error::Unsupported("only blobs and folders move"));
            };
            if item_account != account {
                return Err(Error::Unsupported(
                    "a transfer across accounts streams through the kernel",
                ));
            }
            for (blob, target) in Self::plan(&client, &container, &name, &to_prefix).await? {
                plan.push((container.clone(), blob, target));
            }
        }
        let total: u64 = plan.iter().map(|(_, b, _)| b.size).sum();
        let mut done = 0u64;
        progress.report(Progress::Bytes {
            done,
            total: Some(total),
        });
        for (container, blob, target) in plan {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            debug!(from = %blob.name, %target, ?mode, "copy blob on the service");
            Self::copy_blob(
                &client,
                (&container, &blob.name),
                (to_container, &target),
                done,
                total,
                &progress,
                &cancel,
            )
            .await?;
            if mode == TransferMode::Move {
                client.delete(&container, &blob.name).await?;
            }
            done += blob.size;
            progress.report(Progress::Bytes {
                done,
                total: Some(total),
            });
        }
        Ok(())
    }

    fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
        Err(Error::Unsupported(
            "blob storage sends no change notifications; the kernel polls",
        ))
    }
}

/// The writer `open_write` hands out: one end of a pipe whose other end
/// an upload task drains into blocks, committed as the blob when the
/// writer is shut down. A writer dropped before that leaves the blocks
/// uncommitted, which the service forgets by itself, and no blob.
struct Upload {
    pipe: tokio::io::DuplexStream,
    finished: Arc<AtomicBool>,
    done: tokio::sync::oneshot::Receiver<Result<(), Error>>,
    closed: bool,
}

impl Upload {
    fn start(client: Arc<AzureClient>, container: String, name: String) -> Self {
        let (ours, theirs) = tokio::io::duplex(PIPE);
        let finished = Arc::new(AtomicBool::new(false));
        let (tx, done) = tokio::sync::oneshot::channel();
        let flag = Arc::clone(&finished);
        ext::runtime::handle().spawn(async move {
            let result = upload(&client, &container, &name, theirs, &flag).await;
            let _ = tx.send(result);
        });
        Self {
            pipe: ours,
            finished,
            done,
            closed: false,
        }
    }
}

/// The id of block `index`: the service wants base64 of equal lengths.
fn block_id(index: usize) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!("rick-{index:08}"))
}

/// Reads the pipe into blocks and uploads them; a pipe that closes before
/// `finished` was set is a dropped writer, and nothing is committed.
async fn upload(
    client: &AzureClient,
    container: &str,
    name: &str,
    mut pipe: tokio::io::DuplexStream,
    finished: &AtomicBool,
) -> Result<(), Error> {
    let mut buffer: Vec<u8> = Vec::with_capacity(BLOCK.min(PIPE));
    let mut blocks: Vec<String> = Vec::new();
    let mut chunk = vec![0u8; 256 * 1024];
    loop {
        let read = pipe.read(&mut chunk).await.map_err(Error::from_io)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() >= BLOCK {
            let id = block_id(blocks.len());
            client
                .put_block(
                    container,
                    name,
                    &id,
                    Bytes::from(std::mem::take(&mut buffer)),
                )
                .await?;
            blocks.push(id);
        }
    }
    if !finished.load(Ordering::SeqCst) {
        debug!(%name, "upload abandoned; the blocks stay uncommitted");
        return Err(Error::Cancelled);
    }
    if blocks.is_empty() {
        return client
            .put(container, name, Bytes::from(std::mem::take(&mut buffer)))
            .await;
    }
    if !buffer.is_empty() {
        let id = block_id(blocks.len());
        client
            .put_block(
                container,
                name,
                &id,
                Bytes::from(std::mem::take(&mut buffer)),
            )
            .await?;
        blocks.push(id);
    }
    client.put_block_list(container, name, &blocks).await
}

impl AsyncWrite for Upload {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.pipe).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.pipe).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        use std::future::Future as _;
        use std::task::Poll;
        if !self.closed {
            self.finished.store(true, Ordering::SeqCst);
            match std::pin::Pin::new(&mut self.pipe).poll_shutdown(cx) {
                Poll::Ready(Ok(())) => self.closed = true,
                other => return other,
            }
        }
        match std::pin::Pin::new(&mut self.done).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(Ok(()))) => Poll::Ready(Ok(())),
            Poll::Ready(Ok(Err(error))) => Poll::Ready(Err(std::io::Error::other(error))),
            Poll::Ready(Err(_)) => Poll::Ready(Err(std::io::Error::other(
                "the upload ended without a word",
            ))),
        }
    }
}
