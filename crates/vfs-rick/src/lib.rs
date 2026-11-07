//! Rick's virtual locations as a [`Provider`]: the `rick://` scheme.
//!
//! Six locations live under the provider id [`PROVIDER_ID`], one per
//! [`Virtual`]: `rick://favorites`, `rick://recent` and `rick://trash` are
//! the views the side panel's Places open; `rick://machine`,
//! `rick://network` and `rick://cloud` the overviews its titled sections
//! open. Each lists what the kernel knows already, read-only:
//!
//! | Location | Entries |
//! | --- | --- |
//! | Favorites | the pinned folders and files, with their own ids and paths |
//! | Recent | the system's recently-used list, the use time as `accessed` |
//! | Trash | the platform trash, or Rick's own record where it cannot be listed |
//! | This Machine | the drives of the machine, one folder entry per mount root |
//! | Network | the mounted network file systems and the network providers' roots |
//! | Cloud | the roots of the cloud providers mounted, as they list them |
//!
//! Because they are provider-backed paths they sit in a tab, the
//! breadcrumb, the history and the session like any folder. The provider
//! cannot write; every mutating method answers `Unsupported`. A view lists
//! again when the application calls [`RickProvider::changed`] for it, which
//! its watchers see as [`FsChange::Rescan`].
//!
//! ```
//! use vfs_rick::{PROVIDER_ID, Virtual, parse_uri};
//!
//! let trash = Virtual::Trash.location();
//! assert_eq!(trash.provider, PROVIDER_ID);
//! assert_eq!(trash.to_string(), "rick:trash");
//! assert_eq!(Virtual::Trash.uri(), "rick://trash");
//! assert_eq!(Virtual::of(&trash), Some(Virtual::Trash));
//! assert_eq!(parse_uri("rick://machine"), Some(Virtual::Machine.location()));
//! assert_eq!(parse_uri("/home/me"), None);
//! ```

#![forbid(unsafe_code)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

use ext::{
    Capabilities, ChangeStream, CopyOptions, Depth, EntryStream, FsChange, LinkKind, ListOptions,
    Provider, ReadHandle, RemoveMode, WriteHandle, WriteOptions, async_trait,
};
use futures::StreamExt;
use futures::stream;
use platform::{Platform, TrashedItem, VolumeKind};
use runtime::{ProgressSink, watch};
use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, Location, ProviderId, VPath};
use vfs::Mounts;

/// The id of every virtual location.
pub const PROVIDER_ID: ProviderId = ProviderId("rick");

/// The scheme the virtual locations are written with: `rick://trash`.
pub const SCHEME: &str = "rick://";

/// Provider ids that count as cloud providers: account-addressed object
/// stores, one plugin each (SPEC.md "Object storage"). The Cloud section
/// shows while one of them is mounted (QUESTIONS.md 90).
pub const CLOUD_PROVIDERS: &[&str] = &["s3", "azure", "gcs"];

/// Provider ids that count as network providers: host-addressed
/// connections, listed under Network beside the mounted shares.
pub const NETWORK_PROVIDERS: &[&str] = &["sftp"];

/// The six virtual locations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Virtual {
    /// The pinned folders and files.
    Favorites,
    /// The system's recently-used list.
    Recent,
    /// The platform trash.
    Trash,
    /// The drives of the machine.
    Machine,
    /// The mounted network file systems and connections.
    Network,
    /// The cloud providers' accounts and containers.
    Cloud,
}

impl Virtual {
    /// Every virtual location, Places first, then the section overviews.
    pub const ALL: [Self; 6] = [
        Self::Favorites,
        Self::Recent,
        Self::Trash,
        Self::Machine,
        Self::Network,
        Self::Cloud,
    ];

    /// The path under the scheme: `favorites`, `machine`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Favorites => "favorites",
            Self::Recent => "recent",
            Self::Trash => "trash",
            Self::Machine => "machine",
            Self::Network => "network",
            Self::Cloud => "cloud",
        }
    }

    /// The title a tab, the breadcrumb and the side panel show.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Favorites => "Favorites",
            Self::Recent => "Recent",
            Self::Trash => "Trash",
            Self::Machine => "This Machine",
            Self::Network => "Network",
            Self::Cloud => "Cloud",
        }
    }

    /// The location, `rick:<name>`.
    #[must_use]
    pub fn location(self) -> Location {
        Location::new(PROVIDER_ID, self.name())
    }

    /// The URI, `rick://<name>`.
    #[must_use]
    pub fn uri(self) -> String {
        format!("{SCHEME}{}", self.name())
    }

    /// Which virtual location `loc` is, if any. The path may carry a
    /// leading slash or the whole scheme, as a hand-edited file might.
    #[must_use]
    pub fn of(loc: &Location) -> Option<Self> {
        if loc.provider != PROVIDER_ID {
            return None;
        }
        let text = loc.path.to_string_lossy();
        let name = text.strip_prefix(SCHEME).unwrap_or(&text).trim_matches('/');
        Self::ALL.into_iter().find(|v| v.name() == name)
    }

    /// Whether this is one of the three section overviews.
    #[must_use]
    pub fn is_overview(self) -> bool {
        matches!(self, Self::Machine | Self::Network | Self::Cloud)
    }
}

/// The location a `rick://<name>` text names; `None` for anything else.
#[must_use]
pub fn parse_uri(text: &str) -> Option<Location> {
    let name = text.trim().strip_prefix(SCHEME)?.trim_matches('/');
    Virtual::ALL
        .into_iter()
        .find(|v| v.name() == name)
        .map(Virtual::location)
}

/// Whether a provider id names a cloud provider.
#[must_use]
pub fn is_cloud_provider(id: ProviderId) -> bool {
    CLOUD_PROVIDERS.contains(&id.0)
}

/// Whether a provider id names a network provider.
#[must_use]
pub fn is_network_provider(id: ProviderId) -> bool {
    NETWORK_PROVIDERS.contains(&id.0)
}

/// The pinned locations, as the application keeps them; the Favorites view
/// lists them.
pub trait Favorites: Send + Sync {
    /// The pinned locations in display order.
    fn pinned(&self) -> Vec<Location>;
}

/// Rick's own record of what it trashed, for platforms whose trash cannot
/// be listed (macOS); the Trash view falls back to it.
pub trait TrashRecord: Send + Sync {
    /// Every item known, newest first.
    fn items(&self) -> Vec<TrashedItem>;
}

/// A stable 128-bit key for a path or a name.
fn key_of(bytes: &[u8]) -> u128 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    let low = hasher.finish();
    let mut hasher = DefaultHasher::new();
    (bytes, 1u8).hash(&mut hasher);
    (u128::from(hasher.finish()) << 64) | u128::from(low)
}

/// The location the trash actions' context carries: `rick://trash`.
#[must_use]
pub fn trash_location() -> Location {
    Virtual::Trash.location()
}

/// Whether a location is the Trash view.
#[must_use]
pub fn is_trash(loc: &Location) -> bool {
    Virtual::of(loc) == Some(Virtual::Trash)
}

/// The entry that stands for a trashed item: the original name, the trashed
/// path, the deletion time as `modified`, an id of this provider hashed
/// from the trashed path.
#[must_use]
pub fn trash_entry(item: &TrashedItem) -> Entry {
    let is_dir = std::fs::symlink_metadata(&item.trashed).is_ok_and(|m| m.is_dir());
    let size = (!is_dir)
        .then(|| {
            std::fs::symlink_metadata(&item.trashed)
                .ok()
                .map(|m| m.len())
        })
        .flatten();
    Entry {
        id: EntryId::new(
            PROVIDER_ID,
            key_of(item.trashed.as_os_str().as_encoded_bytes()),
        ),
        name: original_name(item),
        path: VPath::from_os_str(item.trashed.as_os_str()),
        kind: if is_dir {
            EntryKind::Dir
        } else {
            EntryKind::File
        },
        size,
        created: None,
        modified: Some(item.deleted_at),
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    }
}

/// The name a trashed item had where it came from.
#[must_use]
pub fn original_name(item: &TrashedItem) -> String {
    item.original.file_name().map_or_else(
        || item.original.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The folder entry for a path on the local disk, with the local provider's
/// own id when the path can be examined and a hashed one otherwise.
fn local_dir_entry(path: &Path, name: Option<String>) -> Entry {
    let vpath = platform::paths::from_path(path);
    let mut entry = vfs_local::entry_at(path, &vpath).unwrap_or_else(|_| Entry {
        id: EntryId::new(vfs_local::PROVIDER_ID, key_of(vpath.as_bytes())),
        name: String::new(),
        path: vpath.clone(),
        kind: EntryKind::Dir,
        size: None,
        created: None,
        modified: None,
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    });
    if let Some(name) = name {
        entry.name = name;
    } else if entry.name.is_empty() {
        entry.name = path.to_string_lossy().into_owned();
    }
    entry
}

/// The entry for a pinned location: the real entry when it can be examined,
/// otherwise a folder entry with the pinned provider's id hashed from the
/// path, so Unpin still finds it.
async fn favorite_entry(mounts: &Mounts, pinned: &Location) -> Entry {
    let fallback = || Entry {
        id: EntryId::new(pinned.provider, key_of(pinned.path.as_bytes())),
        name: pinned.path.file_name().map_or_else(
            || pinned.path.to_string(),
            |n| String::from_utf8_lossy(n).into_owned(),
        ),
        path: pinned.path.clone(),
        kind: EntryKind::Dir,
        size: None,
        created: None,
        modified: None,
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    };
    if pinned.provider == PROVIDER_ID {
        return fallback();
    }
    let Ok(provider) = mounts.resolve(pinned) else {
        return fallback();
    };
    provider.stat(pinned).await.unwrap_or_else(|_| fallback())
}

/// The root entry of a provider other than the local one, for the Network
/// and Cloud overviews.
fn provider_root(id: ProviderId) -> Entry {
    Entry {
        id: EntryId::new(id, 0),
        name: id.0.to_owned(),
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

/// Runs a blocking platform call on the runtime's blocking pool.
async fn blocking<T, F>(work: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, Error> + Send + 'static,
{
    runtime::handle()
        .spawn_blocking(work)
        .await
        .map_err(|join| Error::Io(std::io::Error::other(join)))?
}

/// The virtual-location provider.
pub struct RickProvider {
    platform: Arc<dyn Platform>,
    favorites: Arc<dyn Favorites>,
    trash: Arc<dyn TrashRecord>,
    mounts: Arc<Mounts>,
    /// One change counter per virtual location; a watch stream yields a
    /// `Rescan` on every increment.
    changes: [watch::Sender<u64>; 6],
}

impl std::fmt::Debug for RickProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RickProvider")
    }
}

impl RickProvider {
    /// A provider over the platform's trash, volumes and recently-used
    /// list, the application's pinned locations and trash record, and the
    /// provider table for the network and cloud roots.
    #[must_use]
    pub fn new(
        platform: Arc<dyn Platform>,
        favorites: Arc<dyn Favorites>,
        trash: Arc<dyn TrashRecord>,
        mounts: Arc<Mounts>,
    ) -> Self {
        Self {
            platform,
            favorites,
            trash,
            mounts,
            changes: std::array::from_fn(|_| watch::channel(0).0),
        }
    }

    /// Tells every open view of `which` to list again: the pins, the
    /// trash or the volumes changed.
    pub fn changed(&self, which: Virtual) {
        self.changes[which as usize].send_modify(|n| *n += 1);
    }

    /// The entries of the Favorites view.
    async fn list_favorites(&self) -> Result<Vec<Entry>, Error> {
        let mut out = Vec::new();
        for pinned in self.favorites.pinned() {
            out.push(favorite_entry(&self.mounts, &pinned).await);
        }
        Ok(out)
    }

    /// The entries of the Recent view, newest first by `accessed`.
    async fn list_recent(&self) -> Result<Vec<Entry>, Error> {
        let platform = Arc::clone(&self.platform);
        let items = blocking(move || platform.recently_used()).await?;
        blocking(move || {
            Ok(items
                .into_iter()
                .filter_map(|item| {
                    let vpath = platform::paths::from_path(&item.path);
                    let mut entry = vfs_local::entry_at(&item.path, &vpath).ok()?;
                    entry.accessed = Some(item.used_at);
                    Some(entry)
                })
                .collect())
        })
        .await
    }

    /// The entries of the Trash view: the platform's list, or Rick's own
    /// record where the platform keeps the original paths to itself.
    async fn list_trash(&self) -> Result<Vec<Entry>, Error> {
        let platform = Arc::clone(&self.platform);
        let mut items = match blocking(move || platform.list()).await {
            Ok(items) => items,
            Err(Error::Unsupported(why)) => {
                tracing::debug!(why, "trash not listable; showing Rick's own record");
                self.trash.items()
            }
            Err(error) => return Err(error),
        };
        items.sort_by_key(|item| std::cmp::Reverse(item.deleted_at));
        blocking(move || Ok(items.iter().map(trash_entry).collect())).await
    }

    /// The volumes of the given kinds as folder entries at their mount
    /// roots.
    async fn list_volumes(&self, network: bool) -> Result<Vec<Entry>, Error> {
        let platform = Arc::clone(&self.platform);
        blocking(move || {
            let volumes = platform.volumes()?;
            Ok(volumes
                .into_iter()
                .filter(|v| (v.kind == VolumeKind::Network) == network)
                .map(|v| {
                    let mut entry = local_dir_entry(&v.mount_point, Some(v.name.clone()));
                    entry.size = v.total;
                    entry
                })
                .collect())
        })
        .await
    }

    /// The roots of the mounted providers `pick` accepts.
    fn provider_roots(&self, pick: fn(ProviderId) -> bool) -> Vec<ProviderId> {
        let mut ids: Vec<ProviderId> = self
            .mounts
            .ids()
            .into_iter()
            .filter(|id| pick(*id))
            .collect();
        ids.sort();
        ids
    }

    /// The Cloud overview: what each cloud provider lists at its root, its
    /// accounts or containers, with the provider's own ids and paths.
    async fn list_cloud(&self, cancel: CancelToken) -> Result<Vec<Entry>, Error> {
        let mut out = Vec::new();
        for id in self.provider_roots(is_cloud_provider) {
            let Some(provider) = self.mounts.provider(id) else {
                continue;
            };
            let root = Location::new(id, "/");
            let opts = ListOptions {
                hidden: false,
                system: false,
                depth: Depth::One,
            };
            let mut batches = provider.list(&root, opts, cancel.clone());
            let mut any = false;
            while let Some(batch) = batches.next().await {
                match batch {
                    Ok(batch) => {
                        any = true;
                        out.extend(batch);
                    }
                    Err(error) => {
                        tracing::debug!(%id, %error, "cloud root not listed");
                        break;
                    }
                }
            }
            if !any {
                out.push(provider_root(id));
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl Provider for RickProvider {
    fn id(&self) -> ProviderId {
        PROVIDER_ID
    }

    fn capabilities(&self, _loc: &Location) -> Capabilities {
        Capabilities::WATCH
    }

    fn list(&self, loc: &Location, _opts: ListOptions, cancel: CancelToken) -> EntryStream {
        let Some(which) = Virtual::of(loc) else {
            return stream::once(async { Err(Error::NotFound) }).boxed();
        };
        let this = Self {
            platform: Arc::clone(&self.platform),
            favorites: Arc::clone(&self.favorites),
            trash: Arc::clone(&self.trash),
            mounts: Arc::clone(&self.mounts),
            changes: self.changes.clone(),
        };
        stream::once(async move {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            match which {
                Virtual::Favorites => this.list_favorites().await,
                Virtual::Recent => this.list_recent().await,
                Virtual::Trash => this.list_trash().await,
                Virtual::Machine => this.list_volumes(false).await,
                Virtual::Network => {
                    let mut entries = this.list_volumes(true).await?;
                    entries.extend(
                        this.provider_roots(is_network_provider)
                            .into_iter()
                            .map(provider_root),
                    );
                    Ok(entries)
                }
                Virtual::Cloud => this.list_cloud(cancel).await,
            }
        })
        .boxed()
    }

    async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
        let which = Virtual::of(loc).ok_or(Error::NotFound)?;
        Ok(Entry {
            id: EntryId::new(PROVIDER_ID, key_of(which.name().as_bytes())),
            name: which.title().to_owned(),
            path: loc.path.clone(),
            kind: EntryKind::Dir,
            size: None,
            created: None,
            modified: None,
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        })
    }

    async fn open_read(&self, _loc: &Location) -> Result<ReadHandle, Error> {
        Err(Error::Unsupported(
            "a virtual location has no contents to read",
        ))
    }

    async fn open_write(&self, _loc: &Location, _opts: WriteOptions) -> Result<WriteHandle, Error> {
        Err(Error::Unsupported("a virtual location cannot be written"))
    }

    async fn create_dir(&self, _loc: &Location) -> Result<Entry, Error> {
        Err(Error::Unsupported(
            "a virtual location cannot hold a folder",
        ))
    }

    async fn rename(&self, _from: &Location, _to: &Location) -> Result<Entry, Error> {
        Err(Error::Unsupported("a virtual location cannot be renamed"))
    }

    async fn remove(&self, _loc: &Location, _mode: RemoveMode) -> Result<(), Error> {
        Err(Error::Unsupported("a virtual location cannot be removed"))
    }

    async fn link(
        &self,
        _target: &Location,
        _at: &Location,
        _kind: LinkKind,
    ) -> Result<Entry, Error> {
        Err(Error::Unsupported("a virtual location cannot hold a link"))
    }

    async fn copy(
        &self,
        _from: &Location,
        _to: &Location,
        _opts: CopyOptions,
        _progress: ProgressSink,
        _cancel: CancelToken,
    ) -> Result<(), Error> {
        Err(Error::Unsupported("a virtual location cannot be copied"))
    }

    /// A `Rescan` every time [`RickProvider::changed`] is called for the
    /// location.
    fn watch(&self, loc: &Location) -> Result<ChangeStream, Error> {
        let which = Virtual::of(loc).ok_or(Error::NotFound)?;
        let mut rx = self.changes[which as usize].subscribe();
        rx.mark_unchanged();
        Ok(stream::unfold(rx, |mut rx| async move {
            match rx.changed().await {
                Ok(()) => Some((FsChange::Rescan, rx)),
                Err(_) => None,
            }
        })
        .boxed())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::SystemTime;

    use platform::{Fake, RecentItem, StandardFolders as _, Trash as _, Volume};

    use super::*;

    struct Pins(Vec<Location>);
    impl Favorites for Pins {
        fn pinned(&self) -> Vec<Location> {
            self.0.clone()
        }
    }

    struct NoRecord;
    impl TrashRecord for NoRecord {
        fn items(&self) -> Vec<TrashedItem> {
            Vec::new()
        }
    }

    fn provider(fake: &Arc<Fake>, pins: Vec<Location>) -> (RickProvider, Arc<Mounts>) {
        let mounts = Arc::new(Mounts::new());
        mounts.mount(Arc::new(vfs_local::LocalProvider::new(
            Arc::clone(fake) as Arc<dyn platform::Trash>
        )));
        let provider = RickProvider::new(
            Arc::clone(fake) as Arc<dyn Platform>,
            Arc::new(Pins(pins)),
            Arc::new(NoRecord),
            Arc::clone(&mounts),
        );
        (provider, mounts)
    }

    async fn collect(provider: &RickProvider, which: Virtual) -> Vec<Entry> {
        let mut stream = provider.list(
            &which.location(),
            ListOptions::default(),
            CancelToken::new(),
        );
        let mut out = Vec::new();
        while let Some(batch) = stream.next().await {
            out.extend(batch.unwrap_or_else(|e| panic!("{e}")));
        }
        out
    }

    #[test]
    fn names_titles_and_uris_round_trip() {
        for which in Virtual::ALL {
            assert_eq!(Virtual::of(&which.location()), Some(which));
            assert_eq!(parse_uri(&which.uri()), Some(which.location()));
            assert_eq!(
                Virtual::of(&Location::new(PROVIDER_ID, format!("/{}", which.name()))),
                Some(which)
            );
        }
        assert_eq!(Virtual::of(&Location::new(PROVIDER_ID, "nope")), None);
        assert_eq!(
            Virtual::of(&Location::new(vfs_local::PROVIDER_ID, "trash")),
            None
        );
        assert!(is_trash(&trash_location()));
        assert!(Virtual::Machine.is_overview());
        assert!(!Virtual::Trash.is_overview());
        assert!(is_cloud_provider(ProviderId("s3")));
        assert!(!is_cloud_provider(vfs_local::PROVIDER_ID));
    }

    #[test]
    fn the_views_list_pins_recent_items_trash_and_volumes() {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let home = fake
            .standard_folder(platform::StandardFolder::Home)
            .unwrap_or_default();
        let docs = home.join("Documents");
        let file = home.join("note.txt");
        std::fs::write(&file, "x").unwrap_or_else(|e| panic!("{e}"));
        let gone = Location::new(vfs_local::PROVIDER_ID, "/nowhere/at/all");
        let (provider, _mounts) = provider(
            &fake,
            vec![
                Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&docs)),
                Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&file)),
                gone.clone(),
            ],
        );
        fake.set_recent(vec![RecentItem {
            path: file.clone(),
            used_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
        }]);
        fake.add_volume(Volume {
            name: "USB".into(),
            mount_point: docs.clone(),
            kind: VolumeKind::Removable,
            fs_type: "vfat".into(),
            device: None,
            total: Some(100),
            free: Some(40),
        });
        fake.add_volume(Volume {
            name: "nas".into(),
            mount_point: home.clone(),
            kind: VolumeKind::Network,
            fs_type: "cifs".into(),
            device: None,
            total: None,
            free: None,
        });
        let trashed = home.join("doomed.txt");
        std::fs::write(&trashed, "y").unwrap_or_else(|e| panic!("{e}"));
        let item = fake.trash(&trashed).unwrap_or_else(|e| panic!("{e}"));

        runtime::block_on(async {
            let favorites = collect(&provider, Virtual::Favorites).await;
            let names: Vec<&str> = favorites.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(names, ["Documents", "note.txt", "all"]);
            assert_eq!(favorites[0].kind, EntryKind::Dir);
            assert_eq!(favorites[1].kind, EntryKind::File);
            assert_eq!(favorites[1].id.provider, vfs_local::PROVIDER_ID);
            assert_eq!(
                Location::new(favorites[2].id.provider, favorites[2].path.clone()),
                gone,
                "a vanished pin keeps its location so Unpin finds it"
            );

            let recent = collect(&provider, Virtual::Recent).await;
            assert_eq!(recent.len(), 1);
            assert_eq!(recent[0].name, "note.txt");
            assert_eq!(
                recent[0].accessed,
                Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))
            );

            let trash = collect(&provider, Virtual::Trash).await;
            assert_eq!(trash.len(), 1);
            assert_eq!(trash[0].name, "doomed.txt");
            assert_eq!(trash[0].id.provider, PROVIDER_ID);
            assert_eq!(trash[0].path, VPath::from_os_str(item.trashed.as_os_str()));
            assert_eq!(trash[0], trash_entry(&item));

            let machine = collect(&provider, Virtual::Machine).await;
            assert_eq!(machine.len(), 1);
            assert_eq!(machine[0].name, "USB");
            assert_eq!(machine[0].size, Some(100));
            assert_eq!(machine[0].kind, EntryKind::Dir);
            assert_eq!(machine[0].path, platform::paths::from_path(&docs));
            assert_eq!(machine[0].id.provider, vfs_local::PROVIDER_ID);

            let network = collect(&provider, Virtual::Network).await;
            assert_eq!(network.len(), 1);
            assert_eq!(network[0].name, "nas");

            let cloud = collect(&provider, Virtual::Cloud).await;
            assert!(cloud.is_empty(), "no cloud provider is mounted");

            let root = provider
                .stat(&Virtual::Trash.location())
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(root.name, "Trash");
            assert!(matches!(
                provider.stat(&Location::new(PROVIDER_ID, "x")).await,
                Err(Error::NotFound)
            ));
            assert!(matches!(
                provider.create_dir(&Virtual::Trash.location()).await,
                Err(Error::Unsupported(_))
            ));
            let mut unknown = provider.list(
                &Location::new(PROVIDER_ID, "x"),
                ListOptions::default(),
                CancelToken::new(),
            );
            assert!(matches!(unknown.next().await, Some(Err(Error::NotFound))));
        });
    }

    #[test]
    fn a_change_reaches_the_watchers_as_a_rescan() {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let (provider, _mounts) = provider(&fake, Vec::new());
        runtime::block_on(async {
            let mut trash = provider
                .watch(&Virtual::Trash.location())
                .unwrap_or_else(|e| panic!("{e}"));
            let mut favorites = provider
                .watch(&Virtual::Favorites.location())
                .unwrap_or_else(|e| panic!("{e}"));
            provider.changed(Virtual::Trash);
            assert_eq!(trash.next().await, Some(FsChange::Rescan));
            let quiet =
                tokio::time::timeout(std::time::Duration::from_millis(50), favorites.next()).await;
            assert!(quiet.is_err(), "the favorites watch saw nothing");
            assert!(matches!(
                provider.watch(&Location::new(PROVIDER_ID, "x")),
                Err(Error::NotFound)
            ));
        });
        let _ = PathBuf::new();
    }
}
