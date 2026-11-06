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
//!     PanelSection,
//! };
//! use runtime::JobRunner;
//! use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, ExtId, Location, ProviderId};
//!
//! const EXTENSION: ColumnId = ColumnId("docs.extension");
//! const COLUMNS: [ColumnDef; 1] = [ColumnDef { id: EXTENSION, label: "Extension" }];
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

use std::fmt;
use std::sync::Arc;

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
    /// The header text.
    pub label: &'static str,
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
}
