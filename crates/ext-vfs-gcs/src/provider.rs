//! The provider: projects at the root, buckets below each, object name
//! prefixes as folders below those.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use bytes::Bytes;
use ext::{
    Capabilities, ChangeStream, CopyOptions, Depth, EntryStream, LinkKind, ListOptions,
    ProgressSink, Provider, ReadHandle, RemoveMode, TransferMode, WriteHandle, WriteOptions,
    async_trait, t,
};
use futures::{StreamExt, stream};
use tokio::io::{AsyncReadExt, AsyncWrite};
use tracing::debug;
use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, Location, Progress, VPath};

use crate::client::{GcsClient, Object};
use crate::connection::GcsConnections;
use crate::{PROVIDER_ID, Place};

/// The size of one chunk of a resumable upload, a multiple of the 256 KiB
/// the service wants; an object smaller than this is written at once.
const CHUNK: usize = 8 * 1024 * 1024;

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

/// What the provider can do at a project: nothing is created there, a
/// bucket needing more than the connection holds (SPEC.md, *Object
/// storage*).
const AT_PROJECT: Capabilities = Capabilities::REMOTE
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

/// The last segment of a name or prefix, without a trailing slash.
fn leaf(name: &str) -> &str {
    name.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(name)
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
    let name = leaf(&object.name).to_owned();
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
        name: "Google Cloud Storage".to_owned(),
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

/// The prefix the children of a bucket or a folder share: empty for the
/// bucket, `name/` for a folder.
fn prefix_of(place: &Place) -> Option<(&str, &str, String)> {
    match place {
        Place::Bucket { project, bucket } => Some((project, bucket, String::new())),
        Place::Object {
            project,
            bucket,
            name,
        } => Some((project, bucket, format!("{name}/"))),
        _ => None,
    }
}

/// The states of a streamed listing.
enum Listing {
    Page {
        client: Arc<GcsClient>,
        project: String,
        bucket: String,
        prefix: String,
        token: Option<String>,
    },
    Done,
}

/// The states of a listing of everything beneath a location
/// ([`Depth::Unlimited`]): the buckets still to go through, each from its
/// prefix, page by page.
struct Flat {
    conns: Arc<GcsConnections>,
    client: Option<Arc<GcsClient>>,
    account: String,
    /// At a project every bucket is listed; `false` once they are known.
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

/// What a name stands for on the service.
enum Kind {
    Object(Object),
    Folder,
}

/// The `gcs` provider.
pub struct GcsProvider {
    conns: Arc<GcsConnections>,
}

impl std::fmt::Debug for GcsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcsProvider")
            .field("connections", &self.conns.names())
            .finish()
    }
}

impl GcsProvider {
    /// A provider over `conns`.
    #[must_use]
    pub fn new(conns: Arc<GcsConnections>) -> Self {
        Self { conns }
    }

    /// Whether `name` is an object or a folder of objects in `bucket`.
    async fn kind(client: &GcsClient, bucket: &str, name: &str) -> Result<Kind, Error> {
        match client.get(bucket, name).await {
            Ok(object) => Ok(Kind::Object(object)),
            Err(Error::NotFound) => {
                let page = client
                    .list(bucket, &format!("{name}/"), None, Some("1"))
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
        project: &str,
        bucket: &str,
        prefix: &str,
        page: crate::client::Page,
        hidden: bool,
    ) -> Vec<Entry> {
        let mut out = Vec::with_capacity(page.prefixes.len() + page.objects.len());
        for common in page.prefixes {
            let place = Place::Object {
                project: project.to_owned(),
                bucket: bucket.to_owned(),
                name: common.trim_end_matches('/').to_owned(),
            };
            let entry = folder(&place, leaf(&common), None);
            if hidden || !entry.attrs.contains(Attrs::HIDDEN) {
                out.push(entry);
            }
        }
        for object in page.objects {
            if object.name == prefix || object.name.ends_with('/') {
                continue;
            }
            let place = Place::Object {
                project: project.to_owned(),
                bucket: bucket.to_owned(),
                name: object.name.clone(),
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
        let place_of = |name: &str| Place::Object {
            project: account.to_owned(),
            bucket: bucket.to_owned(),
            name: name.to_owned(),
        };
        for object in page.objects {
            let Some(rest) = object.name.strip_prefix(prefix) else {
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
                out.push(file(&place_of(&object.name), &object));
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
            Place::Project(account) => (account.clone(), true, VecDeque::new()),
            Place::Bucket {
                project: account,
                bucket,
            } => (
                account.clone(),
                false,
                VecDeque::from([(bucket.clone(), String::new())]),
            ),
            Place::Object {
                project: account,
                bucket,
                name,
            } => (
                account.clone(),
                false,
                VecDeque::from([(bucket.clone(), format!("{name}/"))]),
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
                                project: state.account.clone(),
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

    /// Every object under an item, with the name each lands at under
    /// `to_prefix`.
    async fn plan(
        client: &GcsClient,
        bucket: &str,
        name: &str,
        to_prefix: &str,
    ) -> Result<Vec<(Object, String)>, Error> {
        let last = leaf(name);
        match Self::kind(client, bucket, name).await? {
            Kind::Object(object) => Ok(vec![(object, format!("{to_prefix}{last}"))]),
            Kind::Folder => {
                let below = client.list_all(bucket, &format!("{name}/")).await?;
                Ok(below
                    .into_iter()
                    .map(|object| {
                        let rest = &object.name[name.len()..];
                        let target = format!("{to_prefix}{last}{rest}");
                        (object, target)
                    })
                    .collect())
            }
        }
    }

    /// Rewrites one object on the service, step by step, the bytes
    /// rewritten reported on top of `base`; cancelling stops between
    /// steps and removes the half-written target.
    async fn rewrite_object(
        client: &GcsClient,
        from: (&str, &str),
        to: (&str, &str),
        base: u64,
        total: u64,
        progress: &ProgressSink,
        cancel: &CancelToken,
    ) -> Result<(), Error> {
        let mut token: Option<String> = None;
        loop {
            if cancel.is_cancelled() {
                let _ = client.delete(to.0, to.1).await;
                return Err(Error::Cancelled);
            }
            let step = client
                .rewrite(from.0, from.1, to.0, to.1, token.as_deref())
                .await?;
            progress.report(Progress::Bytes {
                done: base + step.written,
                total: Some(total),
            });
            if step.done {
                return Ok(());
            }
            match step.token {
                Some(next) => token = Some(next),
                None => {
                    return Err(crate::client::message(format!(
                        "the rewrite of {} stopped without a token",
                        from.1
                    )));
                }
            }
        }
    }
}

#[async_trait]
impl Provider for GcsProvider {
    fn id(&self) -> types::ProviderId {
        PROVIDER_ID
    }

    fn capabilities(&self, loc: &Location) -> Capabilities {
        match Place::of(loc) {
            Place::Root => Capabilities::REMOTE,
            Place::Project(_) => AT_PROJECT,
            Place::Bucket { .. } | Place::Object { .. } => IN_BUCKET,
        }
    }

    fn unavailable_reason(&self, loc: &Location, capability: Capabilities) -> Option<String> {
        match Place::of(loc) {
            Place::Root
                if capability.intersects(Capabilities::CREATE_DIR | Capabilities::CREATE_FILE) =>
            {
                Some(t!("gcs-projects-come-from-the-side-panel"))
            }
            Place::Project(_) if capability.contains(Capabilities::CREATE_FILE) => {
                Some(t!("gcs-files-live-in-a-bucket"))
            }
            Place::Project(_) if capability.contains(Capabilities::CREATE_DIR) => {
                Some(t!("gcs-buckets-come-from-the-console"))
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
                    .map(|name| folder(&Place::Project(name.clone()), name, None))
                    .collect();
                stream::once(async move { Ok(entries) }).boxed()
            }
            Place::Project(project) => stream::once(async move {
                let client = conns.client(&project).await?;
                let buckets = client.list_buckets().await?;
                Ok(buckets
                    .into_iter()
                    .map(|b| {
                        folder(
                            &Place::Bucket {
                                project: project.clone(),
                                bucket: b.name.clone(),
                            },
                            &b.name,
                            b.created,
                        )
                    })
                    .collect())
            })
            .boxed(),
            Place::Bucket { .. } | Place::Object { .. } => {
                let Some((project, bucket, prefix)) = prefix_of(&place) else {
                    return stream::once(async { Err(Error::NotFound) }).boxed();
                };
                let (project, bucket) = (project.to_owned(), bucket.to_owned());
                stream::once(async move {
                    let client = conns.client(&project).await?;
                    Ok(Listing::Page {
                        client,
                        project,
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
                                project,
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
                                        &project, &bucket, &prefix, page, hidden,
                                    );
                                    let state = match next {
                                        Some(token) => Listing::Page {
                                            client,
                                            project,
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
            Place::Project(project) => {
                if self.conns.names().contains(project) {
                    Ok(folder(&place, project, None))
                } else {
                    Err(Error::NotFound)
                }
            }
            Place::Bucket { project, bucket } => {
                let client = self.conns.client(project).await?;
                client.get_bucket(bucket).await?;
                Ok(folder(&place, bucket, None))
            }
            Place::Object {
                project,
                bucket,
                name,
            } => {
                let client = self.conns.client(project).await?;
                match Self::kind(&client, bucket, name).await? {
                    Kind::Object(object) => Ok(file(&place, &object)),
                    Kind::Folder => Ok(folder(&place, leaf(name), None)),
                }
            }
        }
    }

    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
        let Place::Object {
            project,
            bucket,
            name,
        } = Place::of(loc)
        else {
            return Err(Error::Unsupported("only an object can be read"));
        };
        let client = self.conns.client(&project).await?;
        client.download(&bucket, &name).await
    }

    async fn open_write(&self, loc: &Location, opts: WriteOptions) -> Result<WriteHandle, Error> {
        let Place::Object {
            project,
            bucket,
            name,
        } = Place::of(loc)
        else {
            return Err(Error::Unsupported("only an object can be written"));
        };
        let client = self.conns.client(&project).await?;
        if !opts.overwrite && client.get(&bucket, &name).await.is_ok() {
            return Err(already_exists());
        }
        Ok(Box::new(Upload::start(client, bucket, name)))
    }

    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error> {
        let place = Place::of(loc);
        match &place {
            Place::Root | Place::Project(_) => {
                Err(Error::Unsupported("a project is added from the side panel"))
            }
            Place::Bucket { project, bucket } => {
                let client = self.conns.client(project).await?;
                if client.get_bucket(bucket).await.is_ok() {
                    return Err(already_exists());
                }
                client.create_bucket(bucket).await?;
                Ok(folder(&place, bucket, Some(SystemTime::now())))
            }
            Place::Object {
                project,
                bucket,
                name,
            } => {
                let client = self.conns.client(project).await?;
                if Self::kind(&client, bucket, name).await.is_ok() {
                    return Err(already_exists());
                }
                client
                    .upload(bucket, &format!("{name}/"), Bytes::new())
                    .await?;
                Ok(folder(&place, leaf(name), None))
            }
        }
    }

    async fn rename(&self, _from: &Location, _to: &Location) -> Result<Entry, Error> {
        Err(Error::Unsupported(
            "an object store has no rename; a move rewrites then deletes",
        ))
    }

    async fn remove(&self, loc: &Location, mode: RemoveMode) -> Result<(), Error> {
        if mode == RemoveMode::Trash {
            return Err(Error::Unsupported("an object store has no trash"));
        }
        match Place::of(loc) {
            Place::Root | Place::Project(_) => Err(Error::Unsupported(
                "a project is removed from the side panel",
            )),
            Place::Bucket { project, bucket } => {
                let client = self.conns.client(&project).await?;
                for object in client.list_all(&bucket, "").await? {
                    client.delete(&bucket, &object.name).await?;
                }
                client.delete_bucket(&bucket).await
            }
            Place::Object {
                project,
                bucket,
                name,
            } => {
                let client = self.conns.client(&project).await?;
                match Self::kind(&client, &bucket, &name).await? {
                    Kind::Object(_) => client.delete(&bucket, &name).await,
                    Kind::Folder => {
                        for object in client.list_all(&bucket, &format!("{name}/")).await? {
                            client.delete(&bucket, &object.name).await?;
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

    /// One object rewritten on the service, within the project.
    async fn copy(
        &self,
        from: &Location,
        to: &Location,
        opts: CopyOptions,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        let (
            Place::Object {
                project,
                bucket,
                name,
            },
            Place::Object {
                project: to_project,
                bucket: to_bucket,
                name: to_name,
            },
        ) = (Place::of(from), Place::of(to))
        else {
            return Err(Error::Unsupported("only objects can be copied"));
        };
        if project != to_project {
            return Err(Error::Unsupported(
                "a copy across projects streams through the kernel",
            ));
        }
        let client = self.conns.client(&project).await?;
        if !opts.overwrite && client.get(&to_bucket, &to_name).await.is_ok() {
            return Err(already_exists());
        }
        let size = client.get(&bucket, &name).await?.size;
        Self::rewrite_object(
            &client,
            (&bucket, &name),
            (&to_bucket, &to_name),
            0,
            size,
            &progress,
            &cancel,
        )
        .await
    }

    /// Moves or copies objects and folders within the project, across
    /// buckets too, through `Rewrite`: nothing streams through the
    /// machine. Progress is the bytes the service rewrote; cancelling
    /// stops between rewrite steps.
    async fn transfer(
        &self,
        items: &[Location],
        to: &Location,
        mode: TransferMode,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        let to_place = Place::of(to);
        let Some((project, to_bucket, to_prefix)) = prefix_of(&to_place) else {
            return Err(Error::Unsupported(
                "objects land in a bucket or a folder of one",
            ));
        };
        let client = self.conns.client(project).await?;
        let mut plan: Vec<(String, Object, String)> = Vec::new();
        for item in items {
            let Place::Object {
                project: item_project,
                bucket,
                name,
            } = Place::of(item)
            else {
                return Err(Error::Unsupported("only objects and folders move"));
            };
            if item_project != project {
                return Err(Error::Unsupported(
                    "a transfer across projects streams through the kernel",
                ));
            }
            for (object, target) in Self::plan(&client, &bucket, &name, &to_prefix).await? {
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
            debug!(from = %object.name, %target, ?mode, "rewrite on the service");
            Self::rewrite_object(
                &client,
                (&bucket, &object.name),
                (to_bucket, &target),
                done,
                total,
                &progress,
                &cancel,
            )
            .await?;
            if mode == TransferMode::Move {
                client.delete(&bucket, &object.name).await?;
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
/// an upload task drains into a resumable upload, chunk by chunk. Shutting
/// the writer down sends the last chunk and waits for the service;
/// dropping it before that abandons the session, which holds no object.
struct Upload {
    pipe: tokio::io::DuplexStream,
    finished: Arc<AtomicBool>,
    done: tokio::sync::oneshot::Receiver<Result<(), Error>>,
    closed: bool,
}

impl Upload {
    fn start(client: Arc<GcsClient>, bucket: String, name: String) -> Self {
        let (ours, theirs) = tokio::io::duplex(PIPE);
        let finished = Arc::new(AtomicBool::new(false));
        let (tx, done) = tokio::sync::oneshot::channel();
        let flag = Arc::clone(&finished);
        ext::runtime::handle().spawn(async move {
            let result = upload(&client, &bucket, &name, theirs, &flag).await;
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

/// Reads the pipe into chunks and uploads them; a pipe that closes before
/// `finished` was set is a dropped writer, and the session is abandoned.
async fn upload(
    client: &GcsClient,
    bucket: &str,
    name: &str,
    mut pipe: tokio::io::DuplexStream,
    finished: &AtomicBool,
) -> Result<(), Error> {
    let mut buffer: Vec<u8> = Vec::with_capacity(CHUNK.min(PIPE));
    let mut session: Option<url::Url> = None;
    let mut offset = 0u64;
    let mut chunk = vec![0u8; 256 * 1024];
    let outcome: Result<(), Error> = async {
        loop {
            let read = pipe.read(&mut chunk).await.map_err(Error::from_io)?;
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if buffer.len() >= CHUNK {
                let url = if let Some(url) = &session {
                    url.clone()
                } else {
                    let url = client.start_resumable(bucket, name).await?;
                    session = Some(url.clone());
                    url
                };
                let data = Bytes::from(std::mem::take(&mut buffer));
                let len = data.len() as u64;
                client.upload_chunk(&url, offset, data, None).await?;
                offset += len;
            }
        }
        if !finished.load(Ordering::SeqCst) {
            return Err(Error::Cancelled);
        }
        match &session {
            None => {
                client
                    .upload(bucket, name, Bytes::from(std::mem::take(&mut buffer)))
                    .await
            }
            Some(url) => {
                let data = Bytes::from(std::mem::take(&mut buffer));
                let total = offset + data.len() as u64;
                client.upload_chunk(url, offset, data, Some(total)).await
            }
        }
    }
    .await;
    if outcome.is_err()
        && let Some(url) = &session
    {
        debug!(%name, "upload abandoned; the session is dropped");
        client.abandon_resumable(url).await;
    }
    outcome
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
