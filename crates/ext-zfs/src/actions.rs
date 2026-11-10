//! The "ZFS" action group: into the snapshots of the current folder and
//! back, a file's versions, Restore, and the snapshot management actions.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::platform::{Elevate, Platform, paths};
use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, JobRunner, Shortcut,
    UiHost, t,
};
use types::{Entry, EntryKind, Error, Location, ProviderId};

use crate::datasets::Datasets;
use crate::parse::FileSystem;
use crate::provider::{PROVIDER_ID, Target, snapshot_location, target, versions_location};

/// The group every action belongs to.
pub const GROUP: ActionGroup = ActionGroup::Extension("ZFS");

/// *Browse snapshots*.
pub const BROWSE: ActionId = ActionId("zfs.browse");
/// *Back to live folder*.
pub const LIVE: ActionId = ActionId("zfs.live");
/// *Versions*.
pub const VERSIONS: ActionId = ActionId("zfs.versions");

/// The local provider's id, whose locations are the live ones.
const LOCAL: ProviderId = ProviderId("local");

/// What every ZFS action shares.
pub struct Shared {
    /// The datasets and their snapshots.
    pub datasets: Arc<Datasets>,
    /// The platform the host lent: its trash for Restore, its elevation
    /// prompt for management; without one those actions are hidden.
    pub platform: Option<Arc<dyn Platform>>,
    /// The elevation prompt, when the platform has one here (`pkexec` is
    /// installed); without it a refused command is not offered elevated.
    pub elevate: Option<Arc<dyn Elevate>>,
    /// The privileged helper `rick-zfs` that runs a refused change under
    /// the prompt; without it a refused command is not offered elevated.
    pub helper: Option<PathBuf>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("datasets", &self.datasets)
            .field("platform", &self.platform.is_some())
            .field("elevate", &self.elevate.is_some())
            .field("helper", &self.helper)
            .finish()
    }
}

/// The OS path of a live (local) location; `None` for other providers.
#[must_use]
pub fn local_path(loc: &Location) -> Option<PathBuf> {
    (loc.provider == LOCAL)
        .then(|| paths::to_path(&loc.path).ok())
        .flatten()
}

/// The live location of an OS path.
#[must_use]
pub fn local(path: &Path) -> Location {
    Location::new(LOCAL, paths::from_path(path))
}

/// The location an entry of the pane at `at` stands for.
fn entry_location(at: &Location, entry: &Entry) -> Location {
    Location::new(at.provider, entry.path.clone())
}

/// The live path a location of the `zfs` provider corresponds to: the
/// mount point plus the path below the snapshot, or the file of a versions
/// view.
#[must_use]
pub fn live_path(datasets: &Datasets, loc: &Location) -> Option<PathBuf> {
    match target(loc).ok()? {
        Target::Snapshot { dataset, rel, .. } => {
            let fs = datasets.named(&dataset)?;
            Some(if rel.as_os_str().is_empty() {
                fs.mountpoint.clone()
            } else {
                fs.mountpoint.join(rel)
            })
        }
        Target::Versions(live) => Some(live),
        Target::Root | Target::Dataset(_) => None,
    }
}

/// The live folder of the pane and the dataset it lies in.
fn live_folder<'a>(datasets: &'a Datasets, ctx: &ActionCtx) -> Option<(PathBuf, &'a FileSystem)> {
    let dir = local_path(&ctx.location)?;
    let fs = datasets.dataset_of(&dir)?;
    Some((dir, fs))
}

/// The newest snapshot of `fs` that holds the folder `rel`.
fn newest_holding(datasets: &Datasets, fs: &FileSystem, rel: &Path) -> Option<String> {
    let snapshots = datasets.snapshots(&fs.name).ok()?;
    snapshots
        .into_iter()
        .find(|s| Datasets::snapshot_dir(fs, &s.name).join(rel).is_dir())
        .map(|s| s.name)
}

/// The nearest existing folder at or above `path`.
fn nearest_existing(path: &Path) -> PathBuf {
    let mut at = path;
    loop {
        if at.is_dir() {
            return at.to_path_buf();
        }
        match at.parent() {
            Some(parent) => at = parent,
            None => return at.to_path_buf(),
        }
    }
}

/// Takes the pane from a live folder in a dataset into the newest snapshot
/// that holds it.
#[derive(Debug)]
pub struct Browse(pub Arc<Shared>);

impl Browse {
    fn destination(&self, ctx: &ActionCtx) -> Result<Location, Availability> {
        let Some((dir, fs)) = live_folder(&self.0.datasets, ctx) else {
            return Err(Availability::Hidden);
        };
        let rel = dir.strip_prefix(&fs.mountpoint).unwrap_or(Path::new(""));
        match newest_holding(&self.0.datasets, fs, rel) {
            Some(snapshot) => Ok(snapshot_location(&fs.name, &snapshot, rel)),
            None => Err(Availability::Disabled(t!("zfs-no-snapshot-holds-folder"))),
        }
    }
}

impl Action for Browse {
    fn id(&self) -> ActionId {
        BROWSE
    }

    fn label(&self) -> String {
        t!("zfs-browse-snapshots")
    }

    fn group(&self) -> ActionGroup {
        GROUP
    }

    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }

    fn availability(&self, ctx: &ActionCtx) -> Availability {
        self.destination(ctx)
            .map_or_else(|why| why, |_| Availability::Enabled)
    }

    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        self.destination(&ctx)
            .map(ActionOutcome::Navigate)
            .map_err(|_| Error::Unsupported("no snapshot holds this folder"))
    }
}

/// Takes the pane from a snapshot or a versions view back to the live
/// folder, or its nearest existing parent.
#[derive(Debug)]
pub struct Live(pub Arc<Shared>);

impl Live {
    fn destination(&self, ctx: &ActionCtx) -> Option<PathBuf> {
        if ctx.location.provider != PROVIDER_ID {
            return None;
        }
        let live = live_path(&self.0.datasets, &ctx.location)?;
        let folder = match target(&ctx.location).ok()? {
            Target::Versions(_) => live.parent().map(Path::to_path_buf)?,
            _ => live,
        };
        Some(nearest_existing(&folder))
    }
}

impl Action for Live {
    fn id(&self) -> ActionId {
        LIVE
    }

    fn label(&self) -> String {
        t!("zfs-back-to-live-folder")
    }

    fn group(&self) -> ActionGroup {
        GROUP
    }

    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }

    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if self.destination(ctx).is_some() {
            Availability::Enabled
        } else {
            Availability::Hidden
        }
    }

    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        self.destination(&ctx)
            .map(|dir| ActionOutcome::Navigate(local(&dir)))
            .ok_or(Error::Unsupported("not inside a snapshot"))
    }
}

/// Opens the versions of one selected file, live or in a snapshot.
#[derive(Debug)]
pub struct Versions(pub Arc<Shared>);

impl Versions {
    /// Whether the pane is somewhere versions make sense: a live folder in
    /// a dataset or a folder inside a snapshot.
    fn applies(&self, ctx: &ActionCtx) -> bool {
        if ctx.location.provider == PROVIDER_ID {
            matches!(target(&ctx.location), Ok(Target::Snapshot { .. }))
        } else {
            live_folder(&self.0.datasets, ctx).is_some()
        }
    }

    /// The live path of the one selected file.
    fn file(&self, ctx: &ActionCtx) -> Option<PathBuf> {
        let [entry] = ctx.selection.as_slice() else {
            return None;
        };
        if entry.kind != EntryKind::File {
            return None;
        }
        let at = entry_location(&ctx.location, entry);
        if at.provider == PROVIDER_ID {
            live_path(&self.0.datasets, &at)
        } else {
            local_path(&at)
        }
    }
}

impl Action for Versions {
    fn id(&self) -> ActionId {
        VERSIONS
    }

    fn label(&self) -> String {
        t!("zfs-versions")
    }

    fn group(&self) -> ActionGroup {
        GROUP
    }

    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }

    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !self.applies(ctx) {
            Availability::Hidden
        } else if self.file(ctx).is_some() {
            Availability::Enabled
        } else {
            Availability::Disabled(t!("zfs-select-one-file"))
        }
    }

    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        self.file(&ctx)
            .map(|live| ActionOutcome::Navigate(versions_location(&live)))
            .ok_or(Error::Unsupported("select one file"))
    }
}

/// Every action of the group, in menu order.
#[must_use]
pub fn all(shared: &Arc<Shared>) -> Vec<Arc<dyn Action>> {
    vec![
        Arc::new(Browse(Arc::clone(shared))),
        Arc::new(Live(Arc::clone(shared))),
        Arc::new(Versions(Arc::clone(shared))),
        Arc::new(crate::restore::Restore(Arc::clone(shared))),
        Arc::new(crate::manage::Create(Arc::clone(shared))),
        Arc::new(crate::manage::Rename(Arc::clone(shared))),
        Arc::new(crate::manage::Destroy(Arc::clone(shared))),
    ]
}
