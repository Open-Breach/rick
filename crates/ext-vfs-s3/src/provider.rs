//! The provider: accounts at the root, buckets below each, key prefixes as
//! folders below those.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::SystemTime;

use bytes::Bytes;
use ext::{
    Capabilities, ChangeStream, CopyOptions, Depth, EntryStream, LinkKind, ListOptions,
    ProgressSink, Provider, ReadHandle, RemoveMode, TransferMode, WriteHandle, WriteOptions,
    async_trait, t,
};
use futures::{StreamExt, stream};
use tokio::io::{AsyncReadExt, AsyncWrite};
use tracing::{debug, warn};
use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, Location, Progress, VPath};

use crate::client::{Object, S3Client, message};
use crate::connection::S3Connections;
use crate::{PROVIDER_ID, Place};

/// The size of one part of a multipart upload; an object smaller than
/// this is written with one `PUT`.
const PART: usize = 8 * 1024 * 1024;

/// How much the upload pipe holds before the writer waits.
const PIPE: usize = 4 * 1024 * 1024;

/// What the provider can do inside a bucket: folders and files at every
/// depth, everything beneath a prefix in one listing, and downloads.
const IN_BUCKET: Capabilities = Capabilities::REMOTE
    .union(Capabilities::WRITE)
    .union(Capabilities::NATIVE_TRANSFER)
    .union(Capabilities::CREATE_DIR)
    .union(Capabilities::CREATE_FILE)
    .union(Capabilities::SEARCH_SUBTREE)
    .union(Capabilities::DOWNLOAD);

/// What the provider can do at an account: nothing is created there, a
/// bucket needing more than the connection holds (SPEC.md, *Object
/// storage*).
const AT_ACCOUNT: Capabilities = Capabilities::REMOTE
    .union(Capabilities::WRITE)
    .union(Capabilities::SEARCH_SUBTREE)
    .union(Capabilities::DOWNLOAD);

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

/// The last segment of a key or prefix, without a trailing slash.
fn leaf(key: &str) -> &str {
    key.trim_end_matches('/').rsplit('/').next().unwrap_or(key)
}

/// A folder entry at `place`, a bucket or a prefix.
fn folder(place: &Place, name: &str, created: Option<SystemTime>) -> Entry {
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
        created,
        modified: None,
        accessed: None,
        attrs,
        mime: None,
    }
}

/// A file entry for an object at `place`.
fn file(place: &Place, object: &Object) -> Entry {
    let loc = place.location();
    let name = leaf(&object.key).to_owned();
    let mut attrs = Attrs::empty();
    if name.starts_with('.') {
        attrs |= Attrs::HIDDEN;
    }
    Entry {
        id: EntryId::new(PROVIDER_ID, key_of(loc.path.as_bytes())),
        name,
        path: loc.path,
        kind: EntryKind::File,
        size: Some(object.size),
        created: None,
        modified: object.modified,
        accessed: None,
        attrs,
        mime: None,
    }
}

fn root_entry() -> Entry {
    Entry {
        id: EntryId::new(PROVIDER_ID, 0),
        name: "S3".to_owned(),
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

/// The prefix the children of a bucket or a key folder share: empty for
/// the bucket, `key/` for a folder.
fn prefix_of(place: &Place) -> Option<(&str, &str, String)> {
    match place {
        Place::Bucket { account, bucket } => Some((account, bucket, String::new())),
        Place::Key {
            account,
            bucket,
            key,
        } => Some((account, bucket, format!("{key}/"))),
        _ => None,
    }
}

/// The states of a streamed listing.
enum Listing {
    Page {
        client: Arc<S3Client>,
        account: String,
        bucket: String,
        prefix: String,
        token: Option<String>,
    },
    Done,
}

/// The `s3` provider.
pub struct S3Provider {
    conns: Arc<S3Connections>,
}

impl std::fmt::Debug for S3Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Provider")
            .field("connections", &self.conns.names())
            .finish()
    }
}

/// The states of a listing of everything beneath a location
/// ([`Depth::Unlimited`]): the buckets still to go through, each from its
/// prefix, page by page.
struct Flat {
    conns: Arc<S3Connections>,
    client: Option<Arc<S3Client>>,
    account: String,
    /// At an account every bucket is listed; `false` once they are known.
    whole_account: bool,
    /// The buckets still to list, each with the prefix to list under.
    todo: VecDeque<(String, String)>,
    token: Option<String>,
    /// The folders already reported in the bucket being listed.
    seen: HashSet<String>,
    hidden: bool,
    cancel: CancelToken,
}

/// Whether a path below a listing's root has a hidden component.
fn hidden_below(rest: &str) -> bool {
    rest.split('/').any(|part| part.starts_with('.'))
}

/// What a key stands for on the service.
enum Kind {
    Object(Object),
    Folder,
}

impl S3Provider {
    /// A provider over `conns`.
    #[must_use]
    pub fn new(conns: Arc<S3Connections>) -> Self {
        Self { conns }
    }

    /// Whether `key` is an object or a folder of keys in `bucket`.
    async fn kind(client: &S3Client, bucket: &str, key: &str) -> Result<Kind, Error> {
        match client.head(bucket, key).await {
            Ok(object) => Ok(Kind::Object(object)),
            Err(Error::NotFound) => {
                let page = client
                    .list(bucket, &format!("{key}/"), None, Some("1"))
                    .await?;
                if page.prefixes.is_empty() && page.objects.is_empty() {
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
        bucket: &str,
        prefix: &str,
        page: crate::client::Page,
        hidden: bool,
    ) -> Vec<Entry> {
        let mut out = Vec::with_capacity(page.prefixes.len() + page.objects.len());
        for common in page.prefixes {
            let key = common.trim_end_matches('/').to_owned();
            let place = Place::Key {
                account: account.to_owned(),
                bucket: bucket.to_owned(),
                key,
            };
            let entry = folder(&place, leaf(&common), None);
            if hidden || !entry.attrs.contains(Attrs::HIDDEN) {
                out.push(entry);
            }
        }
        for object in page.objects {
            // The folder's own marker, and any other key ending in a
            // slash, is not a file.
            if object.key == prefix || object.key.ends_with('/') {
                continue;
            }
            let place = Place::Key {
                account: account.to_owned(),
                bucket: bucket.to_owned(),
                key: object.key.clone(),
            };
            let entry = file(&place, &object);
            if hidden || !entry.attrs.contains(Attrs::HIDDEN) {
                out.push(entry);
            }
        }
        out
    }

    /// The entries of one page of a listing without a delimiter: every
    /// object below `prefix` as a file, and every folder on the way to
    /// one, once — a folder being a prefix objects share, or its marker.
    pub(crate) fn flat_entries(
        account: &str,
        bucket: &str,
        prefix: &str,
        page: crate::client::Page,
        hidden: bool,
        seen: &mut HashSet<String>,
    ) -> Vec<Entry> {
        let mut out = Vec::with_capacity(page.objects.len());
        let place_of = |name: &str| Place::Key {
            account: account.to_owned(),
            bucket: bucket.to_owned(),
            key: name.to_owned(),
        };
        for object in page.objects {
            let Some(rest) = object.key.strip_prefix(prefix) else {
                continue;
            };
            let is_folder = rest.ends_with('/');
            let rest = rest.trim_end_matches('/');
            if rest.is_empty() || (!hidden && hidden_below(rest)) {
                continue;
            }
            // The folders above the object, and the object itself when it
            // is a folder's marker.
            let mut end = 0;
            for part in rest.split('/') {
                end += part.len();
                let last = end == rest.len();
                if !last || is_folder {
                    let name = format!("{prefix}{}", &rest[..end]);
                    if seen.insert(name.clone()) {
                        out.push(folder(&place_of(&name), part, None));
                    }
                }
                end += 1;
            }
            if !is_folder {
                out.push(file(&place_of(&object.key), &object));
            }
        }
        out
    }

    /// Everything beneath an account, a bucket or a folder, streamed page
    /// by page: what the search in subfolders and the flattened view list
    /// (`SEARCH_SUBTREE`).
    fn list_below(&self, place: &Place, hidden: bool, cancel: CancelToken) -> EntryStream {
        let (account, whole_account, todo) = match place {
            Place::Root => return stream::once(async { Err(Error::NotFound) }).boxed(),
            Place::Account(account) => (account.clone(), true, VecDeque::new()),
            Place::Bucket { account, bucket } => (
                account.clone(),
                false,
                VecDeque::from([(bucket.clone(), String::new())]),
            ),
            Place::Key {
                account,
                bucket,
                key,
            } => (
                account.clone(),
                false,
                VecDeque::from([(bucket.clone(), format!("{key}/"))]),
            ),
        };
        let start = Flat {
            conns: Arc::clone(&self.conns),
            client: None,
            account,
            whole_account,
            todo,
            token: None,
            seen: HashSet::new(),
            hidden,
            cancel,
        };
        stream::unfold(Some(start), |state| async move {
            let mut state = state?;
            if state.cancel.is_cancelled() {
                return Some((Err(Error::Cancelled), None));
            }
            let client = match &state.client {
                Some(client) => Arc::clone(client),
                None => match state.conns.client(&state.account).await {
                    Ok(client) => {
                        state.client = Some(Arc::clone(&client));
                        client
                    }
                    Err(error) => return Some((Err(error), None)),
                },
            };
            if state.whole_account {
                state.whole_account = false;
                return match client.list_buckets().await {
                    Ok(buckets) => {
                        let mut entries = Vec::with_capacity(buckets.len());
                        for b in buckets {
                            let place = Place::Bucket {
                                account: state.account.clone(),
                                bucket: b.name.clone(),
                            };
                            let entry = folder(&place, &b.name, b.created);
                            if state.hidden || !entry.attrs.contains(Attrs::HIDDEN) {
                                entries.push(entry);
                                state.todo.push_back((b.name, String::new()));
                            }
                        }
                        Some((Ok(entries), Some(state)))
                    }
                    Err(error) => Some((Err(error), None)),
                };
            }
            let (bucket, prefix) = state.todo.front()?.clone();
            match client
                .list_flat(&bucket, &prefix, state.token.as_deref())
                .await
            {
                Ok(page) => {
                    let next = page.next.clone();
                    let entries = Self::flat_entries(
                        &state.account,
                        &bucket,
                        &prefix,
                        page,
                        state.hidden,
                        &mut state.seen,
                    );
                    state.token = next;
                    if state.token.is_none() {
                        state.todo.pop_front();
                        state.seen.clear();
                    }
                    Some((Ok(entries), Some(state)))
                }
                Err(error) => Some((Err(error), None)),
            }
        })
        .boxed()
    }

    /// Every object under an item: the object itself, or everything below
    /// a folder, with the key each lands at under `to_prefix`.
    async fn plan(
        client: &S3Client,
        bucket: &str,
        key: &str,
        to_prefix: &str,
    ) -> Result<Vec<(Object, String)>, Error> {
        let name = leaf(key);
        match Self::kind(client, bucket, key).await? {
            Kind::Object(object) => Ok(vec![(object, format!("{to_prefix}{name}"))]),
            Kind::Folder => {
                let below = client.list_all(bucket, &format!("{key}/")).await?;
                Ok(below
                    .into_iter()
                    .map(|object| {
                        let rest = &object.key[key.len()..];
                        let target = format!("{to_prefix}{name}{rest}");
                        (object, target)
                    })
                    .collect())
            }
        }
    }
}

#[async_trait]
impl Provider for S3Provider {
    fn id(&self) -> types::ProviderId {
        PROVIDER_ID
    }

    fn capabilities(&self, loc: &Location) -> Capabilities {
        match Place::of(loc) {
            Place::Root => Capabilities::REMOTE,
            Place::Account(_) => AT_ACCOUNT,
            Place::Bucket { .. } | Place::Key { .. } => IN_BUCKET,
        }
    }

    fn unavailable_reason(&self, loc: &Location, capability: Capabilities) -> Option<String> {
        match Place::of(loc) {
            Place::Root
                if capability.intersects(Capabilities::CREATE_DIR | Capabilities::CREATE_FILE) =>
            {
                Some(t!("s3-accounts-come-from-the-side-panel"))
            }
            Place::Account(_) if capability.contains(Capabilities::CREATE_FILE) => {
                Some(t!("s3-files-live-in-a-bucket"))
            }
            Place::Account(_) if capability.contains(Capabilities::CREATE_DIR) => {
                Some(t!("s3-buckets-come-from-the-console"))
            }
            _ => None,
        }
    }

    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken) -> EntryStream {
        let place = Place::of(loc);
        let conns = Arc::clone(&self.conns);
        let hidden = opts.hidden;
        if opts.depth == Depth::Unlimited && place != Place::Root {
            return self.list_below(&place, hidden, cancel);
        }
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
                let buckets = client.list_buckets().await?;
                Ok(buckets
                    .into_iter()
                    .map(|b| {
                        folder(
                            &Place::Bucket {
                                account: account.clone(),
                                bucket: b.name.clone(),
                            },
                            &b.name,
                            b.created,
                        )
                    })
                    .collect())
            })
            .boxed(),
            Place::Bucket { .. } | Place::Key { .. } => {
                let Some((account, bucket, prefix)) = prefix_of(&place) else {
                    return stream::once(async { Err(Error::NotFound) }).boxed();
                };
                let (account, bucket) = (account.to_owned(), bucket.to_owned());
                stream::once(async move {
                    let client = conns.client(&account).await?;
                    Ok(Listing::Page {
                        client,
                        account,
                        bucket,
                        prefix,
                        token: None,
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
                                bucket,
                                prefix,
                                token,
                            } = state
                            else {
                                return None;
                            };
                            if cancel.is_cancelled() {
                                return Some((Err(Error::Cancelled), Listing::Done));
                            }
                            match client.list(&bucket, &prefix, token.as_deref(), None).await {
                                Ok(page) => {
                                    let next = page.next.clone();
                                    let entries = Self::page_entries(
                                        &account, &bucket, &prefix, page, hidden,
                                    );
                                    let state = match next {
                                        Some(token) => Listing::Page {
                                            client,
                                            account,
                                            bucket,
                                            prefix,
                                            token: Some(token),
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
            Place::Bucket { account, bucket } => {
                let client = self.conns.client(account).await?;
                client.head_bucket(bucket).await?;
                Ok(folder(&place, bucket, None))
            }
            Place::Key {
                account,
                bucket,
                key,
            } => {
                let client = self.conns.client(account).await?;
                match Self::kind(&client, bucket, key).await? {
                    Kind::Object(object) => Ok(file(&place, &object)),
                    Kind::Folder => Ok(folder(&place, leaf(key), None)),
                }
            }
        }
    }

    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
        let Place::Key {
            account,
            bucket,
            key,
        } = Place::of(loc)
        else {
            return Err(Error::Unsupported("only an object can be read"));
        };
        let client = self.conns.client(&account).await?;
        client.get(&bucket, &key).await
    }

    async fn open_write(&self, loc: &Location, opts: WriteOptions) -> Result<WriteHandle, Error> {
        let Place::Key {
            account,
            bucket,
            key,
        } = Place::of(loc)
        else {
            return Err(Error::Unsupported("only an object can be written"));
        };
        let client = self.conns.client(&account).await?;
        if !opts.overwrite && client.head(&bucket, &key).await.is_ok() {
            return Err(already_exists());
        }
        Ok(Box::new(Upload::start(client, bucket, key)))
    }

    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error> {
        let place = Place::of(loc);
        match &place {
            Place::Root | Place::Account(_) => Err(Error::Unsupported(
                "an account is added from the side panel",
            )),
            Place::Bucket { account, bucket } => {
                let client = self.conns.client(account).await?;
                if client.head_bucket(bucket).await.is_ok() {
                    return Err(already_exists());
                }
                client.create_bucket(bucket).await?;
                Ok(folder(&place, bucket, Some(SystemTime::now())))
            }
            Place::Key {
                account,
                bucket,
                key,
            } => {
                let client = self.conns.client(account).await?;
                if Self::kind(&client, bucket, key).await.is_ok() {
                    return Err(already_exists());
                }
                client.put(bucket, &format!("{key}/"), Bytes::new()).await?;
                Ok(folder(&place, leaf(key), None))
            }
        }
    }

    async fn rename(&self, _from: &Location, _to: &Location) -> Result<Entry, Error> {
        Err(Error::Unsupported(
            "an object store has no rename; a move copies then deletes",
        ))
    }

    async fn remove(&self, loc: &Location, mode: RemoveMode) -> Result<(), Error> {
        if mode == RemoveMode::Trash {
            return Err(Error::Unsupported("an object store has no trash"));
        }
        match Place::of(loc) {
            Place::Root | Place::Account(_) => Err(Error::Unsupported(
                "an account is removed from the side panel",
            )),
            Place::Bucket { account, bucket } => {
                let client = self.conns.client(&account).await?;
                for object in client.list_all(&bucket, "").await? {
                    client.delete(&bucket, &object.key).await?;
                }
                client.delete_bucket(&bucket).await
            }
            Place::Key {
                account,
                bucket,
                key,
            } => {
                let client = self.conns.client(&account).await?;
                match Self::kind(&client, &bucket, &key).await? {
                    Kind::Object(_) => client.delete(&bucket, &key).await,
                    Kind::Folder => {
                        for object in client.list_all(&bucket, &format!("{key}/")).await? {
                            client.delete(&bucket, &object.key).await?;
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
        Err(Error::Unsupported("an object store has no links"))
    }

    /// One object copied on the service, within the account.
    async fn copy(
        &self,
        from: &Location,
        to: &Location,
        opts: CopyOptions,
        progress: ProgressSink,
        _cancel: CancelToken,
    ) -> Result<(), Error> {
        let (
            Place::Key {
                account,
                bucket,
                key,
            },
            Place::Key {
                account: to_account,
                bucket: to_bucket,
                key: to_key,
            },
        ) = (Place::of(from), Place::of(to))
        else {
            return Err(Error::Unsupported("only objects can be copied"));
        };
        if account != to_account {
            return Err(Error::Unsupported(
                "a copy across accounts streams through the kernel",
            ));
        }
        let client = self.conns.client(&account).await?;
        if !opts.overwrite && client.head(&to_bucket, &to_key).await.is_ok() {
            return Err(already_exists());
        }
        let size = client.head(&bucket, &key).await?.size;
        client.copy(&bucket, &key, &to_bucket, &to_key).await?;
        progress.report(Progress::Bytes {
            done: size,
            total: Some(size),
        });
        Ok(())
    }

    /// Moves or copies objects and folders within the account, across
    /// buckets too, through `CopyObject`: nothing streams through the
    /// machine. Progress counts the bytes the service copied; cancelling
    /// stops before the next object.
    async fn transfer(
        &self,
        items: &[Location],
        to: &Location,
        mode: TransferMode,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        let to_place = Place::of(to);
        let Some((account, to_bucket, to_prefix)) = prefix_of(&to_place) else {
            return Err(Error::Unsupported(
                "objects land in a bucket or a folder of one",
            ));
        };
        let client = self.conns.client(account).await?;
        let mut plan: Vec<(String, Object, String)> = Vec::new();
        for item in items {
            let Place::Key {
                account: item_account,
                bucket,
                key,
            } = Place::of(item)
            else {
                return Err(Error::Unsupported("only objects and folders move"));
            };
            if item_account != account {
                return Err(Error::Unsupported(
                    "a transfer across accounts streams through the kernel",
                ));
            }
            for (object, target) in Self::plan(&client, &bucket, &key, &to_prefix).await? {
                plan.push((bucket.clone(), object, target));
            }
        }
        let total: u64 = plan.iter().map(|(_, o, _)| o.size).sum();
        let mut done = 0u64;
        progress.report(Progress::Bytes {
            done,
            total: Some(total),
        });
        for (bucket, object, target) in plan {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            debug!(from = %object.key, %target, ?mode, "server-side copy");
            client
                .copy(&bucket, &object.key, to_bucket, &target)
                .await?;
            if mode == TransferMode::Move {
                client.delete(&bucket, &object.key).await?;
            }
            done += object.size;
            progress.report(Progress::Bytes {
                done,
                total: Some(total),
            });
        }
        Ok(())
    }

    fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
        Err(Error::Unsupported(
            "an object store sends no change notifications; the kernel polls",
        ))
    }
}

/// The writer `open_write` hands out: one end of a pipe whose other end
/// an upload task drains into a multipart upload, part by part. Shutting
/// the writer down finishes the upload and waits for the service's
/// answer; dropping it without shutdown makes the task abort the upload,
/// so a cancelled copy leaves no object behind.
struct Upload {
    pipe: tokio::io::DuplexStream,
    finished: Arc<AtomicBool>,
    /// The bytes the pipe accepted from the writer, for the task to
    /// compare with what it read out.
    accepted: Arc<AtomicU64>,
    done: tokio::sync::oneshot::Receiver<Result<(), Error>>,
    closed: bool,
}

impl Upload {
    fn start(client: Arc<S3Client>, bucket: String, key: String) -> Self {
        let (ours, theirs) = tokio::io::duplex(PIPE);
        let finished = Arc::new(AtomicBool::new(false));
        let accepted = Arc::new(AtomicU64::new(0));
        let (tx, done) = tokio::sync::oneshot::channel();
        let flag = Arc::clone(&finished);
        let count = Arc::clone(&accepted);
        ext::runtime::handle().spawn(async move {
            let result = upload(&client, &bucket, &key, theirs, &flag, &count).await;
            let _ = tx.send(result);
        });
        Self {
            pipe: ours,
            finished,
            accepted,
            done,
            closed: false,
        }
    }
}

/// The size of the part just recorded, for the log line.
fn data_len(sizes: &[usize]) -> usize {
    sizes.last().copied().unwrap_or(0)
}

/// The number of the part after `done` parts; the service takes 10 000.
fn part_number(done: usize) -> Result<u32, Error> {
    u32::try_from(done + 1)
        .ok()
        .filter(|n| *n <= 10_000)
        .ok_or(Error::Unsupported("an object has at most 10 000 parts"))
}

/// Reads the pipe into parts and uploads them; a pipe that closes before
/// `finished` was set is a dropped writer, and the upload is abandoned.
/// Once the service has assembled the object, its size is checked against
/// the bytes that went in: a part the service lost on the way is an error
/// and the short object is removed, never left behind as the copy.
async fn upload(
    client: &S3Client,
    bucket: &str,
    key: &str,
    mut pipe: tokio::io::DuplexStream,
    finished: &AtomicBool,
    accepted: &AtomicU64,
) -> Result<(), Error> {
    let mut buffer: Vec<u8> = Vec::with_capacity(PART.min(PIPE));
    let mut upload_id: Option<String> = None;
    let mut parts: Vec<(u32, String)> = Vec::new();
    let mut sizes: Vec<usize> = Vec::new();
    let mut streamed: u64 = 0;
    let mut chunk = vec![0u8; 256 * 1024];
    let outcome: Result<(), Error> = async {
        loop {
            let read = pipe.read(&mut chunk).await.map_err(Error::from_io)?;
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            streamed += read as u64;
            if buffer.len() >= PART {
                let id = if let Some(id) = &upload_id {
                    id.clone()
                } else {
                    let id = client.create_multipart(bucket, key).await?;
                    upload_id = Some(id.clone());
                    id
                };
                let data = Bytes::from(std::mem::take(&mut buffer));
                let number = part_number(parts.len())?;
                sizes.push(data.len());
                let etag = client.upload_part(bucket, key, &id, number, data).await?;
                debug!(%key, number, size = data_len(&sizes), %etag, "part uploaded");
                parts.push((number, etag));
            }
        }
        if !finished.load(Ordering::SeqCst) {
            return Err(Error::Cancelled);
        }
        let written = accepted.load(Ordering::SeqCst);
        if written != streamed {
            return Err(message(format!(
                "{key}: the pipe accepted {written} bytes from the writer but handed {streamed} to the upload"
            )));
        }
        match &upload_id {
            None => {
                client
                    .put(bucket, key, Bytes::from(std::mem::take(&mut buffer)))
                    .await
            }
            Some(id) => {
                if !buffer.is_empty() {
                    let number = part_number(parts.len())?;
                    sizes.push(buffer.len());
                    let etag = client
                        .upload_part(
                            bucket,
                            key,
                            id,
                            number,
                            Bytes::from(std::mem::take(&mut buffer)),
                        )
                        .await?;
                    debug!(%key, number, size = data_len(&sizes), %etag, "last part uploaded");
                    parts.push((number, etag));
                }
                client.complete_multipart(bucket, key, id, &parts).await?;
                let kept = client.head(bucket, key).await?.size;
                if kept != streamed {
                    let described: Vec<String> = parts
                        .iter()
                        .zip(&sizes)
                        .map(|((number, etag), size)| format!("{number}: {size} bytes, {etag}"))
                        .collect();
                    warn!(
                        %key,
                        kept,
                        streamed,
                        parts = %described.join("; "),
                        "the service assembled a shorter object than was sent; removing it"
                    );
                    let _ = client.delete(bucket, key).await;
                    return Err(message(format!(
                        "{key}: the service kept {kept} of {streamed} bytes from {} parts ({})",
                        parts.len(),
                        described.join("; ")
                    )));
                }
                Ok(())
            }
        }
    }
    .await;
    if outcome.is_err()
        && let Some(id) = &upload_id
    {
        debug!(%key, "upload abandoned; aborting the multipart upload");
        let _ = client.abort_multipart(bucket, key, id).await;
    }
    outcome
}

impl AsyncWrite for Upload {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let polled = std::pin::Pin::new(&mut self.pipe).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(n)) = &polled {
            self.accepted.fetch_add(*n as u64, Ordering::SeqCst);
        }
        polled
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
