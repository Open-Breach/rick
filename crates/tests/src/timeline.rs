//! A time slider source for the UI tests and screenshots: snapshots of a
//! folder kept as `.snaps/<label>/…` beside it, so every point is a real
//! folder the local provider lists, with times relative to now.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use ext::{
    API_VERSION, Capabilities, ColumnDef, DecorateCtx, Decoration, Decorator, Extension, Manifest,
    PanelSection, Platforms, Points, Registry, TimePoint, Timeline, Version,
};
use types::{Entry, EntryId, Error, ExtId, Location};

use crate::local;

/// One snapshot: how long ago it was taken and its name.
pub type Snap = (Duration, &'static str);

/// Snapshots of `root` as a ZFS pool keeps them: weekly, daily and hourly.
pub const SNAPSHOTS: [Snap; 9] = [
    (Duration::from_hours(24 * 20), "weekly-44"),
    (Duration::from_hours(24 * 13), "weekly-45"),
    (Duration::from_hours(24 * 6), "daily-mon"),
    (Duration::from_hours(24 * 5), "daily-tue"),
    (Duration::from_hours(24 * 4), "daily-wed"),
    (Duration::from_hours(24 * 3), "daily-thu"),
    (Duration::from_hours(24 * 2), "daily-fri"),
    (Duration::from_hours(5), "hourly-09"),
    (Duration::from_hours(2), "hourly-12"),
];

/// The source: a decorator over `root` that offers its `.snaps` as
/// timeline points.
pub struct Snapshots {
    root: PathBuf,
    snaps: Vec<Snap>,
    delay: Duration,
    calls: AtomicUsize,
}

impl Snapshots {
    /// A source over `root` with `snaps`, answering after `delay`.
    #[must_use]
    pub fn new(root: &Path, snaps: &[Snap], delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            root: root.to_path_buf(),
            snaps: snaps.to_vec(),
            delay,
            calls: AtomicUsize::new(0),
        })
    }

    /// How often the timeline was asked.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// The point `label`'s copy of `rel`.
    #[must_use]
    pub fn point(&self, label: &str, rel: &Path) -> Location {
        local(&joined(&self.root.join(".snaps").join(label), rel))
    }

    /// The folder `loc` shows relative to the root, live or in a point.
    fn rel(&self, loc: &Location) -> Option<PathBuf> {
        let path = platform::paths::to_path(&loc.path).ok()?;
        let rel = path.strip_prefix(&self.root).ok()?;
        let mut parts = rel.components();
        if parts.next().is_some_and(|c| c.as_os_str() == ".snaps") {
            parts.next()?;
            return Some(parts.as_path().to_path_buf());
        }
        Some(rel.to_path_buf())
    }

    /// The extension that registers this source, ready for a config.
    #[must_use]
    pub fn extension(self: &Arc<Self>) -> Arc<dyn Extension> {
        Arc::new(SnapshotsExtension {
            manifest: Manifest {
                id: ExtId("test-snapshots"),
                name: "Test snapshots",
                description: "",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::DECORATOR,
                platforms: Platforms::ALL,
                previews: &[],
                commands: &[],
            },
            source: Arc::clone(self),
        })
    }
}

impl Decorator for Snapshots {
    fn id(&self) -> ExtId {
        ExtId("test-snapshots")
    }
    fn applies_to(&self, loc: &Location) -> bool {
        self.rel(loc).is_some()
    }
    fn columns(&self) -> &[ColumnDef] {
        &[]
    }
    fn decorate(&self, _: &[Entry], _: &DecorateCtx) -> Vec<(EntryId, Decoration)> {
        Vec::new()
    }
    fn panel(&self, _: &Entry) -> Option<PanelSection> {
        None
    }
    fn invalidates_on(&self, _: &Location) -> Vec<Location> {
        Vec::new()
    }
    fn invalidate(&self, _: &Location) {}
    fn offers_timelines(&self) -> bool {
        true
    }
    fn timeline(&self, loc: &Location) -> Option<Timeline> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        let rel = self.rel(loc)?;
        let now = SystemTime::now();
        let points: Vec<TimePoint> = self
            .snaps
            .iter()
            .map(|(ago, label)| TimePoint {
                time: now - *ago,
                label: (*label).to_owned(),
                source: "ZFS".to_owned(),
                location: self.point(label, &rel),
            })
            .collect();
        (!points.is_empty()).then(|| Timeline {
            live: local(&joined(&self.root, &rel)),
            points,
        })
    }
}

/// `base` with `rel` below it; `base` itself for an empty `rel`, which
/// `join` would end with a separator.
fn joined(base: &Path, rel: &Path) -> PathBuf {
    if rel.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        base.join(rel)
    }
}

struct SnapshotsExtension {
    manifest: Manifest,
    source: Arc<Snapshots>,
}

impl Extension for SnapshotsExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.decorator(Arc::clone(&self.source) as Arc<dyn Decorator>);
        Ok(())
    }
}

/// Writes a folder with a few files and its snapshots under `root`: each
/// snapshot holds the files as they were, `lost-notes.md` only in the
/// older ones.
///
/// # Panics
///
/// When the temporary folder cannot be written.
pub fn write_tree(root: &Path, snaps: &[Snap]) {
    let write = |path: &Path, text: &str| {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{e}"));
        }
        std::fs::write(path, text).unwrap_or_else(|e| panic!("{e}"));
    };
    for name in ["report.odt", "budget.ods", "drafts/plan.md"] {
        write(&root.join(name), name);
    }
    for (i, (_, label)) in snaps.iter().enumerate() {
        let snap = root.join(".snaps").join(label);
        write(&snap.join("report.odt"), label);
        write(&snap.join("budget.ods"), label);
        if i + 2 < snaps.len() {
            write(&snap.join("lost-notes.md"), "the notes");
        }
        if i > 0 {
            write(&snap.join("drafts/plan.md"), label);
        }
    }
}
