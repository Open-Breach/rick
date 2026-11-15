//! The Decorator extension point: status on entries or a directory.
//!
//! A decorator adds badges, name tints, extra columns and info-panel sections
//! to entries it knows something about: Git status, sync state, a checksum.
//! `applies_to` is a cheap gate per location; `decorate` runs for each visible
//! batch and must return fast. Slow lookups return [`CellValue::Pending`] and
//! finish later through [`DecorateCtx::publish`], usually from a job spawned
//! on the context's [`JobRunner`].
//!
//! # Worked example
//!
//! A decorator that badges Markdown files and fills one extra column with the
//! file's extension.
//!
//! ```
//! use ext::{
//!     Badge, CellValue, Color, ColumnDef, ColumnId, DecorateCtx, Decoration, Decorator,
//!     PanelSection, Text,
//! };
//! use runtime::JobRunner;
//! use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, ExtId, Location, ProviderId};
//!
//! const EXTENSION: ColumnId = ColumnId("docs.extension");
//! const COLUMNS: [ColumnDef; 1] = [ColumnDef { id: EXTENSION, label: Text("column-extension") }];
//!
//! struct Docs;
//!
//! impl Decorator for Docs {
//!     fn id(&self) -> ExtId {
//!         ExtId("docs")
//!     }
//!
//!     fn applies_to(&self, _loc: &Location) -> bool {
//!         true
//!     }
//!
//!     fn columns(&self) -> &[ColumnDef] {
//!         &COLUMNS
//!     }
//!
//!     fn decorate(&self, entries: &[Entry], _ctx: &DecorateCtx) -> Vec<(EntryId, Decoration)> {
//!         entries
//!             .iter()
//!             .filter_map(|entry| {
//!                 let ext = entry.name.rsplit_once('.')?.1.to_owned();
//!                 let mut decoration = Decoration::default();
//!                 if ext == "md" {
//!                     decoration.badge = Some(Badge { glyph: "M", color: Color::rgb(0x2E, 0x7D, 0x32) });
//!                 }
//!                 decoration.cells.push((EXTENSION, CellValue::Text(ext)));
//!                 Some((entry.id, decoration))
//!             })
//!             .collect()
//!     }
//!
//!     fn panel(&self, entry: &Entry) -> Option<PanelSection> {
//!         entry.name.ends_with(".md").then(|| PanelSection {
//!             title: "Markdown".into(),
//!             rows: vec![("Flavour".into(), "CommonMark".into())],
//!         })
//!     }
//!
//!     fn invalidates_on(&self, _loc: &Location) -> Vec<Location> {
//!         Vec::new()
//!     }
//!
//!     fn invalidate(&self, _loc: &Location) {}
//! }
//!
//! let provider = ProviderId("local");
//! let entry = |id: u128, name: &str| Entry {
//!     id: EntryId::new(provider, id),
//!     name: name.into(),
//!     path: format!("/{name}").into(),
//!     kind: EntryKind::File,
//!     size: None,
//!     created: None,
//!     modified: None,
//!     accessed: None,
//!     attrs: Attrs::empty(),
//!     mime: None,
//! };
//! let entries = [entry(1, "SPEC.md"), entry(2, "main.rs"), entry(3, "Makefile")];
//! let ctx = DecorateCtx::new(
//!     Location::new(provider, "/"),
//!     CancelToken::new(),
//!     JobRunner::new(),
//!     |_late| {},
//! );
//!
//! let decorated = Docs.decorate(&entries, &ctx);
//! assert_eq!(decorated.len(), 2, "the Makefile has no extension");
//! assert!(decorated[0].1.badge.is_some());
//! assert!(decorated[1].1.badge.is_none());
//! assert_eq!(decorated[1].1.cells[0], (EXTENSION, CellValue::Text("rs".into())));
//! assert!(Docs.panel(&entries[0]).is_some());
//! ```
//!
//! # Offering a timeline
//!
//! A decorator that knows earlier states of a folder (filesystem snapshots,
//! backups) says so with [`Decorator::offers_timelines`] and answers
//! [`Decorator::timeline`] for one location. The kernel calls `timeline` in
//! a job, only while the pane's time slider is shown, and merges the points
//! of every decorator that answers, so it may block on a cold cache.
//!
//! ```
//! use std::time::{Duration, SystemTime};
//!
//! use ext::{
//!     ColumnDef, DecorateCtx, Decoration, Decorator, PanelSection, TimePoint, Timeline,
//! };
//! use types::{Entry, EntryId, ExtId, Location, ProviderId, VPath};
//!
//! /// Nightly backups of everything under `/data`, kept in `backup:/<day>/…`.
//! struct Nightly;
//!
//! impl Decorator for Nightly {
//!     fn id(&self) -> ExtId {
//!         ExtId("nightly")
//!     }
//!
//!     fn applies_to(&self, loc: &Location) -> bool {
//!         loc.path.to_string_lossy().starts_with("/data")
//!     }
//!
//!     fn columns(&self) -> &[ColumnDef] {
//!         &[]
//!     }
//!
//!     fn decorate(&self, _entries: &[Entry], _ctx: &DecorateCtx) -> Vec<(EntryId, Decoration)> {
//!         Vec::new()
//!     }
//!
//!     fn panel(&self, _entry: &Entry) -> Option<PanelSection> {
//!         None
//!     }
//!
//!     fn invalidates_on(&self, _loc: &Location) -> Vec<Location> {
//!         Vec::new()
//!     }
//!
//!     fn invalidate(&self, _loc: &Location) {}
//!
//!     fn offers_timelines(&self) -> bool {
//!         true
//!     }
//!
//!     fn timeline(&self, loc: &Location) -> Option<Timeline> {
//!         let day = Duration::from_secs(86_400);
//!         let points = (1..=2u32)
//!             .map(|n| TimePoint {
//!                 time: SystemTime::UNIX_EPOCH + day * n,
//!                 label: format!("day {n}"),
//!                 source: "Nightly".into(),
//!                 location: Location::new(
//!                     ProviderId("backup"),
//!                     format!("/{n}{}", loc.path),
//!                 ),
//!             })
//!             .collect();
//!         Some(Timeline { live: loc.clone(), points })
//!     }
//! }
//!
//! let docs = Location::new(ProviderId("local"), "/data/docs");
//! let timeline = Nightly.timeline(&docs).unwrap();
//! assert_eq!(timeline.points.len(), 2);
//! assert_eq!(timeline.points[1].location.path, VPath::from("/2/data/docs"));
//! assert_eq!(timeline.point_at(&docs), None, "the live folder is *Now*");
//! assert_eq!(timeline.point_at(&timeline.points[0].location.clone()), Some(0));
//! ```

use std::fmt;
use std::sync::Arc;
use std::time::SystemTime;

use runtime::JobRunner;
use smallvec::SmallVec;
use types::{CancelToken, Entry, EntryId, ExtId, Location};

/// Identity of an extra column, unique across decorators: `"git.status"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColumnId(pub &'static str);

/// An extra column a decorator can fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnDef {
    /// The column's id, used in [`Decoration::cells`].
    pub id: ColumnId,
    /// The header text: a Fluent key, read live.
    pub label: crate::Text,
}

/// One cell of an extra column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellValue {
    /// Text, shown as is.
    Text(String),
    /// A number, right-aligned and sortable.
    Number(i64),
    /// Not known yet; the decorator publishes the value later.
    Pending,
}

/// An sRGB colour with alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha, 255 is opaque.
    pub a: u8,
}

impl Color {
    /// An opaque colour.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}

/// A small glyph drawn in the icon's corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Badge {
    /// One character or short symbol.
    pub glyph: &'static str,
    /// The glyph's colour.
    pub color: Color,
}

/// What a decorator says about one entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decoration {
    /// Small glyph in the icon corner.
    pub badge: Option<Badge>,
    /// Name colour.
    pub tint: Option<Color>,
    /// Values for the decorator's extra columns.
    pub cells: SmallVec<[(ColumnId, CellValue); 2]>,
}

/// A section for the info panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelSection {
    /// Section heading.
    pub title: String,
    /// Label and value rows.
    pub rows: Vec<(String, String)>,
}

type Publish = dyn Fn(Vec<(EntryId, Decoration)>) + Send + Sync;

/// What a decorator gets with each batch.
#[derive(Clone)]
pub struct DecorateCtx {
    /// The location the batch belongs to.
    pub location: Location,
    /// Fires when the batch is no longer visible.
    pub cancel: CancelToken,
    /// For lookups too slow to answer inline.
    pub jobs: JobRunner,
    publish: Arc<Publish>,
}

impl DecorateCtx {
    /// Builds a context; `publish` receives decorations that finish late.
    pub fn new(
        location: Location,
        cancel: CancelToken,
        jobs: JobRunner,
        publish: impl Fn(Vec<(EntryId, Decoration)>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            location,
            cancel,
            jobs,
            publish: Arc::new(publish),
        }
    }

    /// Delivers decorations that were [`CellValue::Pending`] when `decorate`
    /// returned.
    pub fn publish(&self, items: Vec<(EntryId, Decoration)>) {
        (self.publish)(items);
    }
}

impl fmt::Debug for DecorateCtx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecorateCtx")
            .field("location", &self.location)
            .field("cancel", &self.cancel)
            .finish_non_exhaustive()
    }
}

/// One earlier state of a folder on a [`Timeline`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimePoint {
    /// When the state was taken.
    pub time: SystemTime,
    /// Its name as the source knows it: a snapshot's name.
    pub label: String,
    /// The source's name as the time slider shows it: "ZFS".
    pub source: String,
    /// The location that shows the folder as it was at `time`.
    pub location: Location,
}

/// The earlier states of a folder: what the time slider moves through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timeline {
    /// The folder now, the slider's right end.
    pub live: Location,
    /// The earlier states, oldest first once the kernel merged them.
    pub points: Vec<TimePoint>,
}

impl Timeline {
    /// Merges the answers of several sources into one timeline: every
    /// point, sorted by time (stable, so equal times keep the order of the
    /// answers), with the first answer's live folder. `None` without any
    /// point.
    #[must_use]
    pub fn merge(answers: impl IntoIterator<Item = Self>) -> Option<Self> {
        let mut answers = answers.into_iter();
        let mut merged = answers.next()?;
        for answer in answers {
            merged.points.extend(answer.points);
        }
        merged.points.sort_by_key(|point| point.time);
        (!merged.points.is_empty()).then_some(merged)
    }

    /// The index of the point that shows `location`; `None` for the live
    /// folder or a location that is no point of this timeline.
    #[must_use]
    pub fn point_at(&self, location: &Location) -> Option<usize> {
        self.points
            .iter()
            .position(|point| point.location == *location)
    }

    /// Whether `location` is the live folder or one of the points.
    #[must_use]
    pub fn covers(&self, location: &Location) -> bool {
        self.live == *location || self.point_at(location).is_some()
    }
}

/// Adds badges, columns and panel sections to entries.
pub trait Decorator: Send + Sync + 'static {
    /// The extension this decorator belongs to.
    fn id(&self) -> ExtId;

    /// Cheap check per location, e.g. "inside a Git work tree".
    fn applies_to(&self, loc: &Location) -> bool;

    /// Extra columns this decorator can fill.
    fn columns(&self) -> &[ColumnDef];

    /// Called for each visible batch. Must return fast; slow lookups return
    /// [`CellValue::Pending`] and complete later through
    /// [`DecorateCtx::publish`].
    fn decorate(&self, entries: &[Entry], ctx: &DecorateCtx) -> Vec<(EntryId, Decoration)>;

    /// Optional section for the info panel.
    fn panel(&self, entry: &Entry) -> Option<PanelSection>;

    /// Locations whose changes make this decorator's answers for `loc`
    /// stale, besides `loc` itself: a Git decorator names the `.git`
    /// directory. The kernel watches them and calls [`invalidate`](Self::invalidate).
    fn invalidates_on(&self, loc: &Location) -> Vec<Location>;

    /// The kernel saw a change at `loc` or at one of the locations named by
    /// [`invalidates_on`](Self::invalidates_on); cached answers for `loc` are
    /// stale and the next `decorate` must look again.
    fn invalidate(&self, loc: &Location);

    /// Whether this decorator can offer timelines at all. Read once, so it
    /// must not depend on a location; the kernel offers the time slider for
    /// a location when a decorator that offers timelines
    /// [`applies_to`](Self::applies_to) it.
    fn offers_timelines(&self) -> bool {
        false
    }

    /// The earlier states of the folder `loc`, or `None` when there are
    /// none. Runs in a job, never on the UI thread, and only while a time
    /// slider for `loc` is shown, so it may block; it should answer from a
    /// cache where it can. `loc` may be the live folder or one of the
    /// points of its timeline, and both answer with the same timeline.
    fn timeline(&self, loc: &Location) -> Option<Timeline> {
        let _ = loc;
        None
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use types::{Location, ProviderId};

    use super::{TimePoint, Timeline};

    fn point(day: u64, label: &str, source: &str) -> TimePoint {
        TimePoint {
            time: SystemTime::UNIX_EPOCH + Duration::from_secs(86_400 * day),
            label: label.into(),
            source: source.into(),
            location: Location::new(ProviderId("t"), format!("/{source}/{label}")),
        }
    }

    fn live() -> Location {
        Location::new(ProviderId("local"), "/docs")
    }

    #[test]
    fn two_sources_merge_by_time_and_keep_their_source() {
        let zfs = Timeline {
            live: live(),
            points: vec![point(1, "mon", "ZFS"), point(4, "thu", "ZFS")],
        };
        let backup = Timeline {
            live: Location::new(ProviderId("other"), "/"),
            points: vec![point(2, "tue", "Backup")],
        };
        let merged = Timeline::merge([zfs, backup]).unwrap_or_else(|| panic!("merged"));
        let order: Vec<(&str, &str)> = merged
            .points
            .iter()
            .map(|p| (p.label.as_str(), p.source.as_str()))
            .collect();
        assert_eq!(order, [("mon", "ZFS"), ("tue", "Backup"), ("thu", "ZFS")]);
        assert_eq!(merged.live, live(), "the first answer's live folder");
    }

    #[test]
    fn equal_times_keep_the_answers_order_and_nothing_merges_to_none() {
        let first = Timeline {
            live: live(),
            points: vec![point(3, "a", "ZFS")],
        };
        let second = Timeline {
            live: live(),
            points: vec![point(3, "b", "Backup")],
        };
        let merged = Timeline::merge([first, second]).unwrap_or_else(|| panic!("merged"));
        assert_eq!(merged.points[0].label, "a");
        assert_eq!(Timeline::merge([]), None);
        let empty = Timeline {
            live: live(),
            points: Vec::new(),
        };
        assert_eq!(Timeline::merge([empty]), None, "no point, no timeline");
    }

    #[test]
    fn point_at_and_covers() {
        let timeline = Timeline {
            live: live(),
            points: vec![point(1, "a", "ZFS"), point(2, "b", "ZFS")],
        };
        let b = timeline.points[1].location.clone();
        assert_eq!(timeline.point_at(&b), Some(1));
        assert_eq!(timeline.point_at(&live()), None);
        assert!(timeline.covers(&live()));
        assert!(timeline.covers(&b));
        assert!(!timeline.covers(&Location::new(ProviderId("local"), "/x")));
    }
}
