//! The view: the trash's items as entries, a trashed folder browsable below
//! them, and the decorator that shows where each item came from.

use ext::t;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ext::Text;
use ext::platform::{Trash, TrashedItem};
use ext::{
    Capabilities, CellValue, ChangeStream, ColumnDef, ColumnId, CopyOptions, DecorateCtx,
    Decoration, Decorator, EntryStream, LinkKind, ListOptions, PanelSection, ProgressSink,
    Provider, ReadHandle, RemoveMode, WriteHandle, WriteOptions, async_trait,
};
use futures::{StreamExt, stream};
use types::{
    Attrs, CancelToken, Entry, EntryId, EntryKind, Error, ExtId, Location, ProviderId, VPath,
};

use crate::{ID, VIEW, is_view, location, view_path};

/// The decorator's column: the folder an item was trashed from.
pub const ORIGINAL_COLUMN: ColumnId = ColumnId("trash.original");

const COLUMNS: [ColumnDef; 1] = [ColumnDef {
    id: ORIGINAL_COLUMN,
    label: Text("trash-original-location"),
}];

/// FNV-1a over `bytes`: the key of an entry, stable across listings and
/// processes, which a path-derived key must be.
fn key(bytes: &[u8]) -> u128 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    u128::from(hash)
}

/// The id of the view's root, `rick://trash` itself: in the upper half, so
/// no item's key, which lives in the lower half, can equal it.
fn root_id() -> EntryId {
    EntryId::new(ProviderId::RICK, key(VIEW.as_bytes()) << 64)
}

/// The id of a trashed item or of an entry below one: from its OS path,
/// which does not change while the item is in the trash.
fn path_id(path: &Path) -> EntryId {
    EntryId::new(ProviderId::RICK, key(path.as_os_str().as_encoded_bytes()))
}

/// One trashed item as the view lists it.
#[derive(Debug, Clone)]
struct Listed {
    item: TrashedItem,
    /// The last component of the entry's path under the view: the trashed
    /// file's name, made unique with a suffix when two trash directories
    /// hold the same name.
    name: String,
    /// What the trashed path is now; `None` when it vanished meanwhile.
    meta: Option<std::fs::Metadata>,
}

impl Listed {
    /// The name the item had where it came from.
    fn original_name(&self) -> String {
        self.item.original.file_name().map_or_else(
            || self.item.original.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    fn entry(&self) -> Entry {
        let name = self.original_name();
        let attrs = if name.starts_with('.') {
            Attrs::HIDDEN
        } else {
            Attrs::empty()
        };
        Entry {
            id: path_id(&self.item.trashed),
            path: location().path.join(self.name.as_bytes()),
            kind: match &self.meta {
                Some(meta) if meta.is_dir() => EntryKind::Dir,
                _ => EntryKind::File,
            },
            size: self
                .meta
                .as_ref()
                .filter(|m| !m.is_dir())
                .map(std::fs::Metadata::len),
            created: None,
            modified: Some(self.item.deleted_at),
            accessed: None,
            attrs,
            mime: None,
            name,
        }
    }
}

/// Lists the trash and names every item: the trashed file's name, with
/// ` (2)`, ` (3)`, … added in trashed-path order when the same name comes
/// from two trash directories, so the names are the same on every call.
/// Newest first.
fn listed(trash: &dyn Trash) -> Result<Vec<Listed>, Error> {
    let mut items = trash.list()?;
    items.sort_by(|a, b| a.trashed.cmp(&b.trashed));
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let base = item.trashed.file_name().map_or_else(
            || item.trashed.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let count = seen.entry(base.clone()).or_insert(0);
        *count += 1;
        let name = if *count == 1 {
            base
        } else {
            format!("{base} ({count})")
        };
        let meta = std::fs::symlink_metadata(&item.trashed).ok();
        out.push(Listed { item, name, meta });
    }
    out.sort_by_key(|l| std::cmp::Reverse(l.item.deleted_at));
    Ok(out)
}

/// What a location under the view points at.
enum Target {
    /// The view itself.
    Root,
    /// A trashed item, and the path below it when the item is a folder.
    Item { name: String, below: Vec<Vec<u8>> },
}

fn target(loc: &Location) -> Result<Target, Error> {
    if loc.provider != ProviderId::RICK {
        return Err(Error::Unsupported("location belongs to another provider"));
    }
    let rest = view_path(loc).ok_or(Error::NotFound)?;
    let mut parts = rest
        .split(|b| matches!(b, b'/' | b'\\'))
        .filter(|p| !p.is_empty());
    let Some(name) = parts.next() else {
        return Ok(Target::Root);
    };
    let name = std::str::from_utf8(name)
        .map_err(|_| Error::NotFound)?
        .to_owned();
    Ok(Target::Item {
        name,
        below: parts.map(<[u8]>::to_vec).collect(),
    })
}

/// An entry of the local filesystem below a trashed folder.
fn entry_below(path: &Path, vpath: VPath) -> Result<Entry, Error> {
    let meta = std::fs::symlink_metadata(path).map_err(Error::from_io)?;
    let name = path
        .file_name()
        .map_or_default(|n| n.to_string_lossy().into_owned());
    let kind = if meta.is_dir() {
        EntryKind::Dir
    } else if meta.file_type().is_symlink() {
        EntryKind::Symlink {
            target: std::fs::read_link(path).map_or_default(|t| VPath::from_os_str(t.as_os_str())),
            broken: std::fs::metadata(path).is_err(),
        }
    } else if meta.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    Ok(Entry {
        id: path_id(path),
        attrs: if name.starts_with('.') {
            Attrs::HIDDEN
        } else {
            Attrs::empty()
        },
        name,
        path: vpath,
        size: (!meta.is_dir()).then_some(meta.len()),
        created: meta.created().ok(),
        modified: meta.modified().ok(),
        accessed: meta.accessed().ok(),
        kind,
        mime: None,
    })
}

/// The OS path of `below` inside the trashed item `item`. Names below the
/// view were produced by `to_string_lossy` of OS names read on this
/// machine; the lossy text is the best handle there is.
fn path_below(item: &TrashedItem, below: &[Vec<u8>]) -> PathBuf {
    let mut path = item.trashed.clone();
    for part in below {
        path.push(String::from_utf8_lossy(part).into_owned());
    }
    path
}

/// The last listing, shared with the listing tasks.
type Cache = Arc<Mutex<Vec<Listed>>>;

/// The trash as a tree of entries.
pub struct TrashProvider {
    trash: Arc<dyn Trash>,
    /// The last listing, for the decorator and the actions: the view's
    /// entries are decorated and acted on right after they were listed.
    cache: Cache,
}

impl std::fmt::Debug for TrashProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrashProvider")
            .field("cached", &self.cache().len())
            .finish_non_exhaustive()
    }
}

fn lock(cache: &Cache) -> MutexGuard<'_, Vec<Listed>> {
    // Plain data; a panic elsewhere leaves it usable.
    cache.lock().unwrap_or_else(PoisonError::into_inner)
}

impl TrashProvider {
    /// A view over `trash`.
    #[must_use]
    pub fn new(trash: Arc<dyn Trash>) -> Self {
        Self {
            trash,
            cache: Arc::default(),
        }
    }

    /// The platform trash the view shows.
    #[must_use]
    pub fn trash(&self) -> &Arc<dyn Trash> {
        &self.trash
    }

    fn cache(&self) -> MutexGuard<'_, Vec<Listed>> {
        lock(&self.cache)
    }

    /// Lists the trash now, off the caller's thread, and remembers the
    /// listing.
    async fn refresh(&self) -> Result<Vec<Listed>, Error> {
        let trash = Arc::clone(&self.trash);
        let items = ext::blocking(move || listed(&*trash)).await?;
        self.cache().clone_from(&items);
        Ok(items)
    }

    /// The trashed items the entries stand for, in the entries' order, as
    /// the last listing knew them; entries that are not items (or are gone)
    /// are left out.
    #[must_use]
    pub fn items_of(&self, entries: &[Entry]) -> Vec<TrashedItem> {
        let cache = self.cache();
        entries
            .iter()
            .filter_map(|entry| {
                cache
                    .iter()
                    .find(|l| path_id(&l.item.trashed) == entry.id)
                    .map(|l| l.item.clone())
            })
            .collect()
    }

    /// The folder the item behind `entry` was trashed from, as the last
    /// listing knew it.
    #[must_use]
    pub fn original_of(&self, entry: &Entry) -> Option<PathBuf> {
        self.cache()
            .iter()
            .find(|l| path_id(&l.item.trashed) == entry.id)
            .and_then(|l| l.item.original.parent().map(Path::to_path_buf))
    }

    /// Whether the last listing holds no items.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cache().is_empty()
    }

    /// The item named `name` in the view, listing the trash again when the
    /// cache does not know it.
    async fn item_named(&self, name: &str) -> Result<Listed, Error> {
        if let Some(found) = self.cache().iter().find(|l| l.name == name) {
            return Ok(found.clone());
        }
        self.refresh()
            .await?
            .into_iter()
            .find(|l| l.name == name)
            .ok_or(Error::NotFound)
    }

    /// The view's own entry: `size` counts the items and `modified` is the
    /// newest deletion, so a poll of `stat` notices a change.
    fn root_entry(items: &[Listed]) -> Entry {
        Entry {
            id: root_id(),
            name: "Trash".to_owned(),
            path: location().path,
            kind: EntryKind::Dir,
            size: Some(items.len() as u64),
            created: None,
            modified: items.iter().map(|l| l.item.deleted_at).max(),
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        }
    }
}

#[async_trait]
impl Provider for TrashProvider {
    fn id(&self) -> ProviderId {
        ProviderId::RICK
    }

    /// Nothing can be created, written, renamed or trashed in the view;
    /// `remove(Permanent)` works on every trashed item all the same.
    fn capabilities(&self, _loc: &Location) -> Capabilities {
        Capabilities::empty()
    }

    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken) -> EntryStream {
        let target = match target(loc) {
            Ok(target) => target,
            Err(error) => return stream::once(async move { Err(error) }).boxed(),
        };
        let trash = Arc::clone(&self.trash);
        let cache = Arc::clone(&self.cache);
        let loc = loc.clone();
        stream::once(async move {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            match target {
                Target::Root => {
                    let items = ext::blocking(move || listed(&*trash)).await?;
                    let entries = items
                        .iter()
                        .map(Listed::entry)
                        .filter(|e| opts.hidden || !e.attrs.contains(Attrs::HIDDEN))
                        .collect();
                    *lock(&cache) = items;
                    Ok(entries)
                }
                Target::Item { name, below } => {
                    let cached = lock(&cache).iter().find(|l| l.name == name).cloned();
                    let item = if let Some(item) = cached {
                        item
                    } else {
                        let items = ext::blocking(move || listed(&*trash)).await?;
                        let found = items.iter().find(|l| l.name == name).cloned();
                        *lock(&cache) = items;
                        found.ok_or(Error::NotFound)?
                    };
                    let dir = path_below(&item.item, &below);
                    let prefix = loc.path;
                    ext::blocking(move || {
                        let mut out = Vec::new();
                        for child in std::fs::read_dir(&dir).map_err(Error::from_io)? {
                            let child = child.map_err(Error::from_io)?;
                            let name = child.file_name();
                            let vpath = prefix.join(name.to_string_lossy().as_bytes());
                            let entry = entry_below(&child.path(), vpath)?;
                            if !opts.hidden && entry.attrs.contains(Attrs::HIDDEN) {
                                continue;
                            }
                            out.push(entry);
                        }
                        Ok(out)
                    })
                    .await
                }
            }
        })
        .boxed()
    }

    async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
        match target(loc)? {
            Target::Root => Ok(Self::root_entry(&self.refresh().await?)),
            Target::Item { name, below } => {
                let item = self.item_named(&name).await?;
                if below.is_empty() {
                    return Ok(item.entry());
                }
                let path = path_below(&item.item, &below);
                let vpath = loc.path.clone();
                ext::blocking(move || entry_below(&path, vpath)).await
            }
        }
    }

    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
        let Target::Item { name, below } = target(loc)? else {
            return Err(Error::Unsupported(
                "the trash itself has no contents to read",
            ));
        };
        let item = self.item_named(&name).await?;
        let path = path_below(&item.item, &below);
        let file = tokio::fs::File::open(path).await.map_err(Error::from_io)?;
        Ok(Box::new(file))
    }

    async fn open_write(&self, _loc: &Location, _opts: WriteOptions) -> Result<WriteHandle, Error> {
        Err(Error::Unsupported(
            "nothing can be written into the trash view",
        ))
    }

    async fn create_dir(&self, _loc: &Location) -> Result<Entry, Error> {
        Err(Error::Unsupported(
            "no folders can be made in the trash view",
        ))
    }

    async fn rename(&self, _from: &Location, _to: &Location) -> Result<Entry, Error> {
        Err(Error::Unsupported(
            "trashed items cannot be renamed; restore first",
        ))
    }

    /// Deletes a trashed item for good; `RemoveMode::Trash` and anything
    /// below an item are refused.
    async fn remove(&self, loc: &Location, mode: RemoveMode) -> Result<(), Error> {
        if mode == RemoveMode::Trash {
            return Err(Error::Unsupported("the item is in the trash already"));
        }
        let Target::Item { name, below } = target(loc)? else {
            return Err(Error::Unsupported("empty the trash with its own action"));
        };
        if !below.is_empty() {
            return Err(Error::Unsupported(
                "delete the trashed folder itself, not a part of it",
            ));
        }
        let item = self.item_named(&name).await?;
        let trash = Arc::clone(&self.trash);
        ext::blocking(move || trash.remove(&item.item)).await?;
        self.cache().retain(|l| l.name != name);
        Ok(())
    }

    async fn link(
        &self,
        _target: &Location,
        _at: &Location,
        _kind: LinkKind,
    ) -> Result<Entry, Error> {
        Err(Error::Unsupported("no links can be made in the trash view"))
    }

    async fn copy(
        &self,
        _from: &Location,
        _to: &Location,
        _opts: CopyOptions,
        _progress: ProgressSink,
        _cancel: CancelToken,
    ) -> Result<(), Error> {
        Err(Error::Unsupported(
            "nothing can be copied within the trash view",
        ))
    }

    fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
        Err(Error::Unsupported("the trash is polled, not watched"))
    }
}

/// The decorator: the *Original location* column of the view, and the
/// same fact in the info panel.
#[derive(Debug)]
pub struct TrashDecorator {
    provider: Arc<TrashProvider>,
}

impl TrashDecorator {
    /// A decorator over the view's provider.
    #[must_use]
    pub fn new(provider: Arc<TrashProvider>) -> Self {
        Self { provider }
    }
}

impl Decorator for TrashDecorator {
    fn id(&self) -> ExtId {
        ID
    }

    fn applies_to(&self, loc: &Location) -> bool {
        is_view(loc)
    }

    fn columns(&self) -> &[ColumnDef] {
        &COLUMNS
    }

    fn decorate(&self, entries: &[Entry], _ctx: &DecorateCtx) -> Vec<(EntryId, Decoration)> {
        entries
            .iter()
            .map(|entry| {
                let mut decoration = Decoration::default();
                let text = self
                    .provider
                    .original_of(entry)
                    .map_or_default(|p| p.display().to_string());
                decoration
                    .cells
                    .push((ORIGINAL_COLUMN, CellValue::Text(text)));
                (entry.id, decoration)
            })
            .collect()
    }

    fn panel(&self, entry: &Entry) -> Option<PanelSection> {
        let original = self.provider.original_of(entry)?;
        Some(PanelSection {
            title: t!("trash-trash"),
            rows: vec![(
                "Original location".to_owned(),
                original.display().to_string(),
            )],
        })
    }

    fn invalidates_on(&self, _loc: &Location) -> Vec<Location> {
        Vec::new()
    }

    fn invalidate(&self, _loc: &Location) {}
}
