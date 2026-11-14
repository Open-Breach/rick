//! The Installed Apps view, `rick://apps`: one row per install record, the
//! app folder browsable below it, orphaned staging folders as rows of their
//! own, and the decorator that adds the version, kind, scope, state and
//! commands columns.

use ext::t;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ext::Text;
use ext::platform::Platform;
use ext::{
    Capabilities, CellValue, ChangeStream, ColumnDef, ColumnId, CopyOptions, DecorateCtx,
    Decoration, Decorator, EntryStream, LinkKind, ListOptions, PanelSection, ProgressSink,
    Provider, ReadHandle, RemoveMode, WriteHandle, WriteOptions, async_trait,
};
use futures::{StreamExt, stream};
use types::{
    Attrs, CancelToken, Entry, EntryId, EntryKind, Error, ExtId, Location, ProviderId, VPath,
};

use crate::error::{InstallError, err};
use crate::record::{self, Installed, Layout, Orphan, Scan, Scope};
use crate::{ID, VIEW, is_view, location, view_path};

/// The *Version* column.
pub const VERSION_COLUMN: ColumnId = ColumnId("installer.version");
/// The *Kind* column.
pub const KIND_COLUMN: ColumnId = ColumnId("installer.kind");
/// The *Scope* column.
pub const SCOPE_COLUMN: ColumnId = ColumnId("installer.scope");
/// The *State* column.
pub const STATE_COLUMN: ColumnId = ColumnId("installer.state");
/// The *Commands* column.
pub const COMMANDS_COLUMN: ColumnId = ColumnId("installer.commands");

const COLUMNS: [ColumnDef; 5] = [
    ColumnDef {
        id: VERSION_COLUMN,
        label: Text("installer-version"),
    },
    ColumnDef {
        id: KIND_COLUMN,
        label: Text("installer-kind"),
    },
    ColumnDef {
        id: SCOPE_COLUMN,
        label: Text("installer-scope"),
    },
    ColumnDef {
        id: STATE_COLUMN,
        label: Text("installer-state"),
    },
    ColumnDef {
        id: COMMANDS_COLUMN,
        label: Text("installer-commands"),
    },
];

/// FNV-1a over `bytes`: a stable key for a path.
fn key(bytes: &[u8]) -> u128 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    u128::from(hash)
}

fn root_id() -> EntryId {
    EntryId::new(ProviderId::RICK, key(VIEW.as_bytes()) << 64)
}

fn path_id(path: &Path) -> EntryId {
    EntryId::new(ProviderId::RICK, key(path.as_os_str().as_encoded_bytes()))
}

/// The path component a row has under the view: the app id, with
/// `@system` for a system-wide install so the two scopes never clash.
fn row_name(installed: &Installed) -> String {
    match installed.layout.scope {
        Scope::User => installed.record.id.clone(),
        Scope::System => format!("{}@system", installed.record.id),
    }
}

fn orphan_row_name(orphan: &Orphan) -> String {
    format!("{}.staging-{}", orphan.app_id, orphan.version)
}

fn app_entry(installed: &Installed) -> Entry {
    Entry {
        id: path_id(&installed.record_path()),
        name: installed.record.name.clone(),
        path: location().path.join(row_name(installed).as_bytes()),
        kind: EntryKind::Dir,
        size: None,
        created: None,
        modified: installed.record.installed_at_time(),
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    }
}

fn orphan_entry(orphan: &Orphan) -> Entry {
    Entry {
        id: path_id(&orphan.dir),
        name: format!("{} (unfinished {})", orphan.app_id, orphan.version),
        path: location().path.join(orphan_row_name(orphan).as_bytes()),
        kind: EntryKind::Dir,
        size: None,
        created: None,
        modified: std::fs::metadata(&orphan.dir)
            .and_then(|m| m.modified())
            .ok(),
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    }
}

/// An entry of the local filesystem below a row.
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

/// What a location under the view points at.
enum Target {
    Root,
    Row { name: String, below: Vec<String> },
}

fn target(loc: &Location) -> Result<Target, Error> {
    if loc.provider != ProviderId::RICK {
        return Err(Error::Unsupported("location belongs to another provider"));
    }
    let rest = view_path(loc).ok_or(Error::NotFound)?;
    let mut parts = rest
        .split(|b| matches!(b, b'/' | b'\\'))
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned());
    let Some(name) = parts.next() else {
        return Ok(Target::Root);
    };
    Ok(Target::Row {
        name,
        below: parts.collect(),
    })
}

/// The last scan, shared with the decorator and the actions.
type Cache = Arc<Mutex<Scan>>;

fn lock(cache: &Cache) -> MutexGuard<'_, Scan> {
    cache.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The view: the installed apps as entries.
pub struct AppsProvider {
    platform: Arc<dyn Platform>,
    cache: Cache,
    /// The staging folders jobs are extracting into right now, which the
    /// scan does not list as orphans.
    busy: Arc<Mutex<Vec<PathBuf>>>,
}

impl std::fmt::Debug for AppsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppsProvider")
            .field("apps", &self.scan().apps.len())
            .finish_non_exhaustive()
    }
}

impl AppsProvider {
    /// A view over the records under the platform's home and `/opt`.
    #[must_use]
    pub fn new(platform: Arc<dyn Platform>) -> Self {
        Self {
            platform,
            cache: Arc::default(),
            busy: Arc::default(),
        }
    }

    /// The platform the view and the actions use.
    #[must_use]
    pub fn platform(&self) -> &Arc<dyn Platform> {
        &self.platform
    }

    /// The user-space and system-wide layouts.
    #[must_use]
    pub fn layouts(&self) -> [Layout; 2] {
        [
            Layout::user(&*self.platform),
            Layout::system(&*self.platform),
        ]
    }

    /// The user-space layout.
    #[must_use]
    pub fn user_layout(&self) -> Layout {
        Layout::user(&*self.platform)
    }

    /// The last scan.
    #[must_use]
    pub fn scan(&self) -> Scan {
        lock(&self.cache).clone()
    }

    /// Scans the roots now, on the caller's thread, and remembers the
    /// result.
    pub fn rescan(&self) -> Scan {
        let busy = self
            .busy
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let scan = record::scan(&self.layouts(), &busy);
        *lock(&self.cache) = scan.clone();
        scan
    }

    /// Marks a staging folder as in use by a job, or releases it.
    pub fn set_busy(&self, staging: &Path, busy: bool) {
        let mut list = self.busy.lock().unwrap_or_else(PoisonError::into_inner);
        list.retain(|p| p != staging);
        if busy {
            list.push(staging.to_path_buf());
        }
    }

    /// The installed app behind `entry`, as the last scan knew it.
    #[must_use]
    pub fn installed_of(&self, entry: &Entry) -> Option<Installed> {
        lock(&self.cache)
            .apps
            .iter()
            .find(|a| path_id(&a.record_path()) == entry.id)
            .cloned()
    }

    /// The orphaned staging folder behind `entry`, if it is one.
    #[must_use]
    pub fn orphan_of(&self, entry: &Entry) -> Option<Orphan> {
        lock(&self.cache)
            .orphans
            .iter()
            .find(|o| path_id(&o.dir) == entry.id)
            .cloned()
    }

    /// The app with this row name, scanning again when the cache does not
    /// know it.
    fn row(&self, name: &str) -> Result<Installed, Error> {
        if let Some(found) = lock(&self.cache).apps.iter().find(|a| row_name(a) == name) {
            return Ok(found.clone());
        }
        self.rescan()
            .apps
            .into_iter()
            .find(|a| row_name(a) == name)
            .ok_or(Error::NotFound)
    }

    fn orphan_row(&self, name: &str) -> Option<Orphan> {
        lock(&self.cache)
            .orphans
            .iter()
            .find(|o| orphan_row_name(o) == name)
            .cloned()
    }

    fn root_entry(scan: &Scan) -> Entry {
        Entry {
            id: root_id(),
            name: "Installed Apps".to_owned(),
            path: location().path,
            kind: EntryKind::Dir,
            size: Some(scan.apps.len() as u64),
            created: None,
            modified: scan
                .apps
                .iter()
                .filter_map(|a| a.record.installed_at_time())
                .max(),
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        }
    }
}

fn dir_below(base: &Path, below: &[String]) -> PathBuf {
    let mut path = base.to_path_buf();
    for part in below {
        path.push(part);
    }
    path
}

#[async_trait]
impl Provider for AppsProvider {
    fn id(&self) -> ProviderId {
        ProviderId::RICK
    }

    /// Nothing is created, written or moved in the view; the actions do the
    /// installing and uninstalling.
    fn capabilities(&self, _loc: &Location) -> Capabilities {
        Capabilities::empty()
    }

    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken) -> EntryStream {
        if !crate::supported() {
            return stream::once(async { Err(err(InstallError::Unsupported)) }).boxed();
        }
        let target = match target(loc) {
            Ok(target) => target,
            Err(error) => return stream::once(async move { Err(error) }).boxed(),
        };
        let cache = Arc::clone(&self.cache);
        let busy = Arc::clone(&self.busy);
        let layouts = self.layouts();
        let loc = loc.clone();
        stream::once(async move {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            match target {
                Target::Root => {
                    let busy = busy.lock().unwrap_or_else(PoisonError::into_inner).clone();
                    let scan = ext::blocking(move || Ok(record::scan(&layouts, &busy))).await?;
                    let mut entries: Vec<Entry> = scan.apps.iter().map(app_entry).collect();
                    entries.extend(scan.orphans.iter().map(orphan_entry));
                    *lock(&cache) = scan;
                    Ok(entries)
                }
                Target::Row { name, below } => {
                    let found = lock(&cache)
                        .apps
                        .iter()
                        .find(|a| row_name(a) == name)
                        .map(|a| a.dir.clone())
                        .or_else(|| {
                            lock(&cache)
                                .orphans
                                .iter()
                                .find(|o| orphan_row_name(o) == name)
                                .map(|o| o.dir.clone())
                        });
                    let base = if let Some(dir) = found {
                        dir
                    } else {
                        let busy = busy.lock().unwrap_or_else(PoisonError::into_inner).clone();
                        let scan = ext::blocking(move || Ok(record::scan(&layouts, &busy))).await?;
                        let dir = scan
                            .apps
                            .iter()
                            .find(|a| row_name(a) == name)
                            .map(|a| a.dir.clone())
                            .or_else(|| {
                                scan.orphans
                                    .iter()
                                    .find(|o| orphan_row_name(o) == name)
                                    .map(|o| o.dir.clone())
                            });
                        *lock(&cache) = scan;
                        dir.ok_or(Error::NotFound)?
                    };
                    let dir = dir_below(&base, &below);
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
        if !crate::supported() {
            return Err(err(InstallError::Unsupported));
        }
        match target(loc)? {
            Target::Root => {
                let layouts = self.layouts();
                let busy = self
                    .busy
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                let scan = ext::blocking(move || Ok(record::scan(&layouts, &busy))).await?;
                let entry = Self::root_entry(&scan);
                *lock(&self.cache) = scan;
                Ok(entry)
            }
            Target::Row { name, below } => {
                if let Some(orphan) = self.orphan_row(&name) {
                    if below.is_empty() {
                        return Ok(orphan_entry(&orphan));
                    }
                    let path = dir_below(&orphan.dir, &below);
                    let vpath = loc.path.clone();
                    return ext::blocking(move || entry_below(&path, vpath)).await;
                }
                let installed = self.row(&name)?;
                if below.is_empty() {
                    return Ok(app_entry(&installed));
                }
                let path = dir_below(&installed.dir, &below);
                let vpath = loc.path.clone();
                ext::blocking(move || entry_below(&path, vpath)).await
            }
        }
    }

    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
        let Target::Row { name, below } = target(loc)? else {
            return Err(Error::Unsupported(
                "the view itself has no contents to read",
            ));
        };
        let base = match self.orphan_row(&name) {
            Some(orphan) => orphan.dir,
            None => self.row(&name)?.dir,
        };
        let path = dir_below(&base, &below);
        let file = tokio::fs::File::open(path).await.map_err(Error::from_io)?;
        Ok(Box::new(file))
    }

    async fn open_write(&self, _loc: &Location, _opts: WriteOptions) -> Result<WriteHandle, Error> {
        Err(Error::Unsupported(
            "nothing can be written into the Installed Apps view; Install as application does the installing",
        ))
    }

    async fn create_dir(&self, _loc: &Location) -> Result<Entry, Error> {
        Err(Error::Unsupported(
            "no folders can be made in the Installed Apps view",
        ))
    }

    async fn rename(&self, _from: &Location, _to: &Location) -> Result<Entry, Error> {
        Err(Error::Unsupported(
            "installed apps cannot be renamed here; Edit launcher changes the name",
        ))
    }

    async fn remove(&self, _loc: &Location, _mode: RemoveMode) -> Result<(), Error> {
        Err(Error::Unsupported(
            "use Uninstall to remove an installed app",
        ))
    }

    async fn link(
        &self,
        _target: &Location,
        _at: &Location,
        _kind: LinkKind,
    ) -> Result<Entry, Error> {
        Err(Error::Unsupported(
            "no links can be made in the Installed Apps view",
        ))
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
            "nothing can be copied within the Installed Apps view",
        ))
    }

    fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
        Err(Error::Unsupported(
            "the install records are polled, not watched",
        ))
    }
}

/// The decorator: the view's five columns, and the record's facts in the
/// info panel.
#[derive(Debug)]
pub struct AppsDecorator {
    provider: Arc<AppsProvider>,
}

impl AppsDecorator {
    /// A decorator over the view's provider.
    #[must_use]
    pub fn new(provider: Arc<AppsProvider>) -> Self {
        Self { provider }
    }
}

impl Decorator for AppsDecorator {
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
                if let Some(installed) = self.provider.installed_of(entry) {
                    let record = &installed.record;
                    let commands = match record.path_mode {
                        Some(record::PathMode::Path) => {
                            format!("{} on PATH", record.bin_dir.as_deref().unwrap_or("bin"))
                        }
                        _ => record.commands().join(", "),
                    };
                    decoration
                        .cells
                        .push((VERSION_COLUMN, CellValue::Text(record.version.clone())));
                    decoration
                        .cells
                        .push((KIND_COLUMN, CellValue::Text(record.kind.label().to_owned())));
                    decoration.cells.push((
                        SCOPE_COLUMN,
                        CellValue::Text(record.scope.label().to_owned()),
                    ));
                    decoration
                        .cells
                        .push((STATE_COLUMN, CellValue::Text(installed.state())));
                    decoration
                        .cells
                        .push((COMMANDS_COLUMN, CellValue::Text(commands)));
                } else if let Some(orphan) = self.provider.orphan_of(entry) {
                    decoration
                        .cells
                        .push((VERSION_COLUMN, CellValue::Text(orphan.version.clone())));
                    decoration.cells.push((
                        STATE_COLUMN,
                        CellValue::Text("orphaned staging folder".to_owned()),
                    ));
                }
                (entry.id, decoration)
            })
            .collect()
    }

    fn panel(&self, entry: &Entry) -> Option<PanelSection> {
        let installed = self.provider.installed_of(entry)?;
        let record = &installed.record;
        let mut rows = vec![
            ("Version".to_owned(), record.version.clone()),
            ("Kind".to_owned(), record.kind.label().to_owned()),
            ("Scope".to_owned(), record.scope.label().to_owned()),
            ("Folder".to_owned(), installed.dir.display().to_string()),
            ("Source".to_owned(), record.source_archive.clone()),
            ("State".to_owned(), installed.state()),
        ];
        if let Some(previous) = &record.previous_version {
            rows.push(("Previous version".to_owned(), previous.clone()));
        }
        if let Some(class) = &record.startup_wm_class {
            rows.push(("StartupWMClass".to_owned(), class.clone()));
        }
        Some(PanelSection {
            title: t!("installer-installed-app"),
            rows,
        })
    }

    fn invalidates_on(&self, _loc: &Location) -> Vec<Location> {
        Vec::new()
    }

    fn invalidate(&self, _loc: &Location) {}
}
