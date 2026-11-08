//! The documents the Preview tab shows: one slot per pane holding the
//! opened document, the frames it rendered as textures, and the render in
//! flight.
//!
//! Rendering decodes pixels, so it never runs on the UI thread: the slot
//! hands its document to a blocking task on the runtime and takes it back
//! with the frame through a channel it polls each frame, showing the last
//! frame or a placeholder meanwhile. Frames are cached per page, size and
//! theme ([`RenderKey`]), a bounded number of them, so paging back shows
//! the page at once; the pane asks for one frame at a time.
//!
//! Text frames are laid out here, only the visible lines of them: with wrap
//! off each line is one row and the view scrolls both ways; with wrap on
//! lines break at the pane width (anywhere, so a monospace line's row count
//! follows from its length and the whole file never needs laying out), the
//! continuation rows start at the text column and the gutter marks them.
//! Find's matches are drawn as backgrounds on the text, the current one
//! darker.
//!
//! The content never sets the pane's width: frames are asked for at the
//! pane's width and scaled down to fit, text scrolls inside a clipped area,
//! and nothing here asks for more than the width it was given.
//!
//! A document that claims a pointer gesture ([`PreviewControl::Pointer`]:
//! a 3D view's orbit by drag, zoom by wheel, reset by double-click) gets
//! the gesture through `set_control` instead of the pane's pan, scroll or
//! 1:1, and a control change keeps the last frame on screen, marked stale,
//! until the next render lands, so an orbit never drops to a spinner.

use ext::t;
use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontId, Rect, TextStyle, TextureHandle, TextureOptions, pos2, vec2};
use ext::{
    ControlId, ControlValue, Frame, Gesture, InfoSection, PreviewControl, PreviewDoc,
    RenderRequest, TextBlock, TextLayer, Transport,
};
use model::{PaneId, TextPreview};
use runtime::oneshot;
use types::{EntryId, Error};

use crate::icons;

type Rendered = (Box<dyn PreviewDoc>, Result<Frame, Error>);

/// Requests are rounded up to this many pixels so a slow resize does not
/// re-render on every frame.
pub const QUANTUM: f32 = 64.0;

/// The largest frame asked for at actual size, per axis, within what GPUs
/// take as one texture.
pub const MAX_ACTUAL: u32 = 8192;

/// How many rendered frames a slot keeps: the pages of a document paged
/// through, so going back costs nothing.
const CACHE: usize = 32;

/// A tab counts as this many columns.
const TAB_COLUMNS: u32 = 4;

/// Content kind of a text document, for the pinned section setting.
pub const KIND_TEXT: &str = "text";
/// Content kind of a single raster, for the pinned section setting.
pub const KIND_IMAGE: &str = "image";
/// Content kind of a paged document, for the pinned section setting.
pub const KIND_PAGES: &str = "pages";

/// What a frame was rendered for: the page, the requested size and the
/// theme. The same key never renders twice while the frame is cached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderKey {
    /// The page, zero-based.
    pub page: usize,
    /// The requested width in pixels.
    pub width: u32,
    /// The requested height in pixels.
    pub height: u32,
    /// Rendered for the dark theme.
    pub dark: bool,
}

/// A rendered frame kept as a texture.
struct Cached {
    key: RenderKey,
    texture: TextureHandle,
    /// The frame's size in pixels, which is at most the requested one.
    size: (u32, u32),
    /// The slot's tick when it was last drawn, for eviction.
    used: u64,
    /// Rendered before a control changed the document: shown until the
    /// next frame for its key lands, which `request` starts.
    stale: bool,
}

/// Wrapped row counts per line at one column width, with prefix sums so a
/// scroll offset maps to a line by binary search.
struct WrapIndex {
    cols: u32,
    rows: Vec<u32>,
    /// `starts[i]` is the first row of line `i`; one more entry holds the
    /// total.
    starts: Vec<u32>,
    dirty: bool,
}

impl WrapIndex {
    fn new(cells: &[u32], cols: u32) -> Self {
        let rows: Vec<u32> = cells.iter().map(|c| c.div_ceil(cols).max(1)).collect();
        let mut index = Self {
            cols,
            rows,
            starts: Vec::new(),
            dirty: true,
        };
        index.rebuild();
        index
    }

    fn rebuild(&mut self) {
        let mut starts = Vec::with_capacity(self.rows.len() + 1);
        let mut total = 0u32;
        for rows in &self.rows {
            starts.push(total);
            total = total.saturating_add(*rows);
        }
        starts.push(total);
        self.starts = starts;
        self.dirty = false;
    }

    fn total(&self) -> u32 {
        self.starts.last().copied().unwrap_or(0)
    }

    /// The line holding `row`.
    fn line_at(&self, row: u32) -> usize {
        self.starts.partition_point(|s| *s <= row).saturating_sub(1)
    }
}

/// A text frame prepared for drawing: the block, its line ranges, the
/// column count of each line, and the wrap index of the last width.
pub struct TextView {
    block: TextBlock,
    lines: Vec<Range<usize>>,
    cells: Vec<u32>,
    max_cells: u32,
    wrap: Option<WrapIndex>,
}

impl TextView {
    fn new(block: TextBlock) -> Self {
        let lines = line_ranges(&block.text);
        let cells: Vec<u32> = lines
            .iter()
            .map(|range| columns(&block.text[range.clone()]))
            .collect();
        let max_cells = cells.iter().copied().max().unwrap_or(0);
        Self {
            block,
            lines,
            cells,
            max_cells,
            wrap: None,
        }
    }

    /// The text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.block.text
    }

    /// The zero-based line holding byte `offset`.
    #[must_use]
    pub fn line_of(&self, offset: usize) -> usize {
        self.lines
            .partition_point(|range| range.start <= offset)
            .saturating_sub(1)
    }

    /// The wrap index for `cols` columns, built or rebuilt as needed.
    fn wrap_index(&mut self, cols: u32) -> &mut WrapIndex {
        let stale = self.wrap.as_ref().is_none_or(|w| w.cols != cols);
        if stale {
            self.wrap = Some(WrapIndex::new(&self.cells, cols));
        }
        let index = self
            .wrap
            .as_mut()
            .unwrap_or_else(|| unreachable!("just set"));
        if index.dirty {
            index.rebuild();
        }
        index
    }
}

/// How many columns `line` takes: a tab is [`TAB_COLUMNS`], anything else
/// one.
fn columns(line: &str) -> u32 {
    line.chars()
        .map(|c| if c == '\t' { TAB_COLUMNS } else { 1 })
        .sum()
}

/// What one line of text looked like when it was last drawn, for tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawnLine {
    /// Zero-based line number.
    pub line: usize,
    /// Rows the line took.
    pub rows: usize,
    /// Width of the laid-out text in points.
    pub width: f32,
    /// Where the text starts, in points from the left of the content.
    pub x: f32,
    /// Where the gutter starts: the content's left edge.
    pub gutter_x: f32,
}

/// How a text frame is drawn: the settings, find's matches and a line to
/// scroll into view.
#[derive(Debug, Clone, Copy, Default)]
pub struct TextOptions<'a> {
    /// Lines break at the width.
    pub wrap: bool,
    /// Line numbers in the gutter.
    pub numbers: bool,
    /// Byte ranges drawn with a highlight background.
    pub highlights: &'a [Range<usize>],
    /// The index into `highlights` drawn stronger: the current match.
    pub current: Option<usize>,
    /// A line to bring into view this frame.
    pub scroll_to_line: Option<usize>,
}

impl TextOptions<'_> {
    /// The settings alone: no highlights, no scrolling.
    #[must_use]
    pub fn from_settings(settings: TextPreview) -> Self {
        Self {
            wrap: settings.wrap,
            numbers: settings.line_numbers,
            ..Self::default()
        }
    }
}

/// One pane's document and its frames.
#[derive(Default)]
pub struct Slot {
    entry: Option<EntryId>,
    doc: Option<Box<dyn PreviewDoc>>,
    pending: Option<(RenderKey, oneshot::Receiver<Rendered>)>,
    textures: Vec<Cached>,
    tick: u64,
    /// The laid-out text frame and the theme it was rendered for.
    text: Option<(bool, TextView)>,
    /// The page shown, zero-based.
    pub page: usize,
    pages: Option<usize>,
    text_layer: Option<TextLayer>,
    sections: Vec<InfoSection>,
    status: Vec<String>,
    controls: Vec<PreviewControl>,
    error: Option<String>,
    /// The document shows a prefix of a remote file: the bytes served and
    /// the file's size; the pane offers *Load more*.
    prefix: Option<(u64, Option<u64>)>,
    /// A remote file above the size cap for whole-file previews: its size
    /// and the cap; the pane offers *Preview anyway*.
    capped: Option<(u64, u64)>,
    /// Control changes made while the document was out rendering.
    queued: Vec<(ControlId, ControlValue)>,
    /// Rasters at actual pixels instead of fitted to the pane.
    pub actual_size: bool,
    /// How far the picture is panned at actual size.
    pan: egui::Vec2,
    drawn: Vec<DrawnLine>,
    h_scroll: bool,
    frame_rect: Option<Rect>,
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slot")
            .field("entry", &self.entry)
            .field("open", &self.doc.is_some())
            .field("pending", &self.pending.is_some())
            .field("page", &self.page)
            .field("pages", &self.pages)
            .field("frames", &self.textures.len())
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl Slot {
    fn reset(&mut self, entry: Option<EntryId>) {
        *self = Self {
            entry,
            ..Self::default()
        };
    }

    fn take_facts(&mut self, doc: &dyn PreviewDoc) {
        self.pages = doc.pages().or(self.pages);
        self.sections = doc.sections();
        self.status = doc.status();
        self.controls = doc.controls();
        // The layer grows as pages render; copy it when it changed.
        let size = |layer: &TextLayer| {
            (
                layer.pages.len(),
                layer.pages.iter().map(String::len).sum::<usize>(),
            )
        };
        let fresh = doc.text_layer();
        if fresh.map(size) != self.text_layer.as_ref().map(size) {
            self.text_layer = fresh.cloned();
        }
    }

    fn set_doc(&mut self, doc: Box<dyn PreviewDoc>) {
        self.pages = None;
        self.text_layer = None;
        self.take_facts(&*doc);
        self.page = 0;
        self.error = None;
        self.capped = None;
        self.prefix = None;
        self.textures.clear();
        self.text = None;
        self.doc = Some(doc);
    }

    /// The prefix the document shows of a remote file, as the bytes served
    /// and the file's size, when it shows one.
    #[must_use]
    pub fn prefix(&self) -> Option<(u64, Option<u64>)> {
        self.prefix
    }

    /// The size and the cap of a remote file held behind *Preview anyway*.
    #[must_use]
    pub fn capped(&self) -> Option<(u64, u64)> {
        self.capped
    }

    /// Whether a render is in flight.
    #[must_use]
    pub fn rendering(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether a frame has been shown for the current document.
    #[must_use]
    pub fn has_frame(&self) -> bool {
        !self.textures.is_empty() || self.text.is_some()
    }

    /// Whether a document is open or on its way.
    #[must_use]
    pub fn has_doc(&self) -> bool {
        self.doc.is_some() || self.pending.is_some()
    }

    /// The document's page count, when it has pages.
    #[must_use]
    pub fn pages(&self) -> Option<usize> {
        self.pages
    }

    /// The content kind for the pinned-section setting, once a document is
    /// known: paged, text, or image.
    #[must_use]
    pub fn kind(&self) -> Option<&'static str> {
        if !self.has_doc() {
            return None;
        }
        Some(match (self.pages, self.text_layer.is_some()) {
            (Some(_), _) => KIND_PAGES,
            (None, true) => KIND_TEXT,
            (None, false) => KIND_IMAGE,
        })
    }

    /// Whether the content is text the pane lays out.
    #[must_use]
    pub fn is_text(&self) -> bool {
        self.kind() == Some(KIND_TEXT)
    }

    /// Whether the content is a raster without pages.
    #[must_use]
    pub fn is_raster(&self) -> bool {
        self.kind() == Some(KIND_IMAGE)
    }

    /// The document's info sections.
    #[must_use]
    pub fn sections(&self) -> &[InfoSection] {
        &self.sections
    }

    /// The document's status facts.
    #[must_use]
    pub fn status(&self) -> &[String] {
        &self.status
    }

    /// The previewer's own controls.
    #[must_use]
    pub fn controls(&self) -> &[PreviewControl] {
        &self.controls
    }

    /// The document's text, one string per page, for find; as far as the
    /// previewer has it (a paged document may fill it page by page).
    #[must_use]
    pub fn text_layer(&self) -> Option<&TextLayer> {
        self.text_layer.as_ref()
    }

    /// The laid-out text frame, when the content is text.
    #[must_use]
    pub fn text_view(&self) -> Option<&TextView> {
        self.text.as_ref().map(|(_, view)| view)
    }

    /// The laid-out text frame, for drawing it.
    pub fn text_view_mut(&mut self) -> Option<&mut TextView> {
        self.text.as_mut().map(|(_, view)| view)
    }

    /// The pixel size of the most recently drawn frame of `page`, if one is
    /// cached; it says the page's aspect.
    #[must_use]
    pub fn frame_size(&self, page: usize) -> Option<(u32, u32)> {
        self.textures
            .iter()
            .filter(|c| c.key.page == page)
            .max_by_key(|c| c.used)
            .map(|c| c.size)
    }

    /// The document's playback control, when it is media and the document
    /// is not out rendering.
    pub fn transport(&mut self) -> Option<&mut dyn Transport> {
        self.doc.as_mut()?.transport()
    }

    /// What went wrong, if anything.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The text lines drawn in the last frame.
    #[must_use]
    pub fn drawn_lines(&self) -> &[DrawnLine] {
        &self.drawn
    }

    /// Whether the last frame needed a horizontal scrollbar.
    #[must_use]
    pub fn h_scrollbar(&self) -> bool {
        self.h_scroll
    }

    /// Where the frame was drawn in the last frame.
    #[must_use]
    pub fn frame_rect(&self) -> Option<Rect> {
        self.frame_rect
    }

    /// How far a picture at actual size is panned, in points.
    #[must_use]
    pub fn pan_offset(&self) -> egui::Vec2 {
        self.pan
    }

    /// Sets one of the previewer's controls; the next render reflects it.
    /// While the document is out rendering the change waits for it. The
    /// frames rendered so far stay on screen, stale, until the next one.
    pub fn set_control(&mut self, id: ControlId, value: ControlValue) {
        match self.doc.as_mut() {
            Some(doc) => {
                if let Err(error) = doc.set_control(id, value) {
                    tracing::warn!(%error, control = id.0, "control not set");
                }
                self.controls = doc.controls();
                self.sections = doc.sections();
                self.outdate();
            }
            None if self.pending.is_some() => self.queued.push((id, value)),
            None => {}
        }
    }

    /// Marks every frame stale, to be rendered again, and drops the text
    /// layout, which the next request lays out anew.
    fn outdate(&mut self) {
        for cached in &mut self.textures {
            cached.stale = true;
        }
        self.text = None;
    }

    /// The id of the document's control for `gesture`, when it claims it.
    #[must_use]
    pub fn gesture(&self, gesture: Gesture) -> Option<ControlId> {
        self.controls.iter().find_map(|control| match control {
            PreviewControl::Pointer { id, gesture: g, .. } if *g == gesture => Some(*id),
            _ => None,
        })
    }

    /// Whether the document claims any pointer gesture, so the frame has
    /// to sense the pointer.
    #[must_use]
    pub fn claims_pointer(&self) -> bool {
        self.controls
            .iter()
            .any(|control| matches!(control, PreviewControl::Pointer { .. }))
    }

    /// Hands the pointer gestures the document claims to it: the drag's
    /// movement, the unmodified wheel's scroll (the modifier is the pane's
    /// zoom) and a double-click on `response`, the frame.
    pub fn forward_gestures(&mut self, ui: &egui::Ui, response: &egui::Response) {
        if let Some(id) = self.gesture(Gesture::Drag)
            && response.dragged_by(egui::PointerButton::Primary)
        {
            let delta = response.drag_delta();
            if delta != egui::Vec2::ZERO {
                self.set_control(
                    id,
                    ControlValue::Delta {
                        x: delta.x,
                        y: delta.y,
                    },
                );
            }
        }
        if let Some(id) = self.gesture(Gesture::Wheel)
            && response.hovered()
        {
            let delta = ui.input(|i| {
                if i.modifiers.command || i.modifiers.ctrl {
                    egui::Vec2::ZERO
                } else {
                    i.smooth_scroll_delta
                }
            });
            if delta != egui::Vec2::ZERO {
                self.set_control(
                    id,
                    ControlValue::Delta {
                        x: delta.x,
                        y: delta.y,
                    },
                );
            }
        }
        if let Some(id) = self.gesture(Gesture::DoubleClick)
            && response.double_clicked()
        {
            self.set_control(id, ControlValue::Pressed);
        }
    }

    /// The pane's scroll sources for a frame at actual size: all of them,
    /// less the drag and the wheel when the document claims them.
    #[must_use]
    pub fn scroll_source(&self) -> egui::scroll_area::ScrollSource {
        let mut source = egui::scroll_area::ScrollSource::ALL;
        if self.gesture(Gesture::Drag).is_some() {
            source.drag = egui::scroll_area::DragScroll::Never;
        }
        if self.gesture(Gesture::Wheel).is_some() {
            source.mouse_wheel = false;
        }
        source
    }

    /// Takes a finished render back, if any.
    fn poll(&mut self, ctx: &egui::Context, pane: PaneId) {
        let Some((key, rx)) = self.pending.as_mut() else {
            return;
        };
        let key = *key;
        match rx.try_recv() {
            Ok((mut doc, result)) => {
                self.pending = None;
                let queued = std::mem::take(&mut self.queued);
                let changed = !queued.is_empty();
                for (id, value) in queued {
                    if let Err(error) = doc.set_control(id, value) {
                        tracing::warn!(%error, control = id.0, "control not set");
                    }
                }
                self.take_facts(&*doc);
                self.doc = Some(doc);
                match result {
                    Ok(Frame::Rgba {
                        width,
                        height,
                        data,
                    }) => {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [width as usize, height as usize],
                            &data,
                        );
                        let texture = ctx.load_texture(
                            format!("preview-{}-{}-{}", pane.0, key.page, key.width),
                            image,
                            TextureOptions::LINEAR,
                        );
                        self.remember(key, texture, (width, height));
                        self.text = None;
                        self.error = None;
                    }
                    Ok(Frame::Text(block)) => {
                        self.text = Some((key.dark, TextView::new(block)));
                        self.textures.clear();
                        self.error = None;
                    }
                    Ok(Frame::External(_)) => {
                        self.error = Some("external textures are not supported".to_owned());
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
                if changed {
                    // The frame is of the document before the change: shown
                    // until the next request renders it again.
                    self.outdate();
                }
            }
            Err(oneshot::error::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
            Err(oneshot::error::TryRecvError::Closed) => {
                self.pending = None;
                self.error = Some("the render task ended without a frame".to_owned());
            }
        }
    }

    /// Keeps a frame, dropping the least recently drawn one past the cap.
    fn remember(&mut self, key: RenderKey, texture: TextureHandle, size: (u32, u32)) {
        self.textures.retain(|c| c.key != key);
        if self.textures.len() >= CACHE
            && let Some(oldest) = self
                .textures
                .iter()
                .enumerate()
                .min_by_key(|(_, c)| c.used)
                .map(|(i, _)| i)
        {
            self.textures.remove(oldest);
        }
        self.tick += 1;
        self.textures.push(Cached {
            key,
            texture,
            size,
            used: self.tick,
            stale: false,
        });
    }

    /// Starts a render for `key` on the runtime's blocking pool, unless a
    /// render is in flight, the frame is cached and not stale, or the
    /// content is text already laid out for the theme. `scale` is the
    /// display's.
    pub fn request(&mut self, ctx: &egui::Context, key: RenderKey, scale: f32) {
        if self.pending.is_some() || self.textures.iter().any(|c| c.key == key && !c.stale) {
            return;
        }
        // Text lays itself out; one render per document and theme is enough.
        if self
            .text
            .as_ref()
            .is_some_and(|(dark, _)| *dark == key.dark)
        {
            return;
        }
        let Some(mut doc) = self.doc.take() else {
            return;
        };
        let req = RenderRequest {
            width: key.width,
            height: key.height,
            scale,
            dark: key.dark,
        };
        let (tx, rx) = oneshot::channel();
        runtime::handle().spawn_blocking(move || {
            let result = doc.render(key.page, req);
            let _ = tx.send((doc, result));
        });
        self.pending = Some((key, rx));
        ctx.request_repaint_after(Duration::from_millis(16));
    }

    /// The cached frame for `key`, marked as drawn.
    pub fn texture(&mut self, key: RenderKey) -> Option<(TextureHandle, (u32, u32))> {
        self.tick += 1;
        let tick = self.tick;
        let cached = self.textures.iter_mut().find(|c| c.key == key)?;
        cached.used = tick;
        Some((cached.texture.clone(), cached.size))
    }

    /// The most recently drawn frame of `page` at any size, to show while
    /// the right size renders.
    pub fn any_texture(&mut self, page: usize) -> Option<(TextureHandle, (u32, u32))> {
        self.tick += 1;
        let tick = self.tick;
        let cached = self
            .textures
            .iter_mut()
            .filter(|c| c.key.page == page)
            .max_by_key(|c| c.used)?;
        cached.used = tick;
        Some((cached.texture.clone(), cached.size))
    }
}

/// Byte ranges of the lines of `text`, without their line ends.
fn line_ranges(text: &str) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            let end = if i > start && text.as_bytes()[i - 1] == b'\r' {
                i - 1
            } else {
                i
            };
            lines.push(start..end);
            start = i + 1;
        }
    }
    if start < text.len() || lines.is_empty() {
        lines.push(start..text.len());
    }
    lines
}

/// The previews of every pane.
#[derive(Debug, Default)]
pub struct Previews {
    slots: HashMap<PaneId, Slot>,
}

impl Previews {
    fn slot(&mut self, pane: PaneId) -> &mut Slot {
        self.slots.entry(pane).or_default()
    }

    /// The pane's slot, if it has one.
    #[must_use]
    pub fn get(&self, pane: PaneId) -> Option<&Slot> {
        self.slots.get(&pane)
    }

    /// The pane's slot, for the pane's controls.
    pub fn get_mut(&mut self, pane: PaneId) -> &mut Slot {
        self.slot(pane)
    }

    /// A document arrived for `pane`, about `entry`.
    pub fn ready(&mut self, pane: PaneId, doc: Box<dyn PreviewDoc>, entry: Option<EntryId>) {
        let slot = self.slot(pane);
        if slot.entry != entry {
            slot.reset(entry);
        }
        slot.set_doc(doc);
    }

    /// No document could be opened for `pane`'s selection.
    pub fn failed(&mut self, pane: PaneId, error: &Error, entry: Option<EntryId>) {
        let slot = self.slot(pane);
        if slot.entry != entry {
            slot.reset(entry);
        }
        slot.capped = None;
        slot.error = Some(error.to_string());
    }

    /// The document just opened for `pane` shows the first `served` bytes
    /// of a remote file of `total` bytes.
    pub fn prefix(&mut self, pane: PaneId, served: u64, total: Option<u64>) {
        self.slot(pane).prefix = Some((served, total));
    }

    /// `pane`'s selection is a remote file of `size` bytes above `cap`,
    /// about `entry`; nothing opens until *Preview anyway*.
    pub fn capped(&mut self, pane: PaneId, size: u64, cap: u64, entry: Option<EntryId>) {
        let slot = self.slot(pane);
        if slot.entry != entry {
            slot.reset(entry);
        }
        slot.doc = None;
        slot.error = None;
        slot.capped = Some((size, cap));
    }

    /// Brings `pane`'s slot up to date with the entry the app is following:
    /// clears it when that changed and takes back a finished render.
    pub fn sync(&mut self, ctx: &egui::Context, pane: PaneId, target: Option<EntryId>) -> &Slot {
        let slot = self.slot(pane);
        if slot.entry != target {
            slot.reset(target);
        }
        slot.poll(ctx, pane);
        slot
    }

    /// Draws `pane`'s content into the space the caller allocated: the
    /// frame fitted to it, text scrolling inside it, or a placeholder; a
    /// prefix of a remote file ends in a *Load more* bar and a file above
    /// the cap shows its facts and *Preview anyway*. Returns what a button
    /// of those asked for.
    pub fn content(
        &mut self,
        ui: &mut egui::Ui,
        pane: PaneId,
        target: Option<EntryId>,
        text_settings: TextPreview,
    ) -> Option<PreviewAct> {
        let slot = self.slot(pane);
        if let Some((size, cap)) = slot.capped.filter(|_| target.is_some()) {
            return capped_message(ui, size, cap);
        }
        let mut act = None;
        if let Some((served, total)) = slot.prefix.filter(|_| slot.doc.is_some()) {
            act = prefix_bar(ui, served, total);
        }
        let ctx = ui.ctx().clone();
        let body = ui.available_rect_before_wrap();
        let ppp = ctx.pixels_per_point();
        let (width, height) = if slot.actual_size && slot.is_raster() {
            (MAX_ACTUAL, MAX_ACTUAL)
        } else {
            (quantise(body.width(), ppp), quantise(body.height(), ppp))
        };
        let key = RenderKey {
            page: slot.page,
            width,
            height,
            dark: ui.visuals().dark_mode,
        };
        slot.request(&ctx, key, ppp);
        let shown = slot.texture(key).or_else(|| slot.any_texture(slot.page));

        slot.drawn.clear();
        slot.h_scroll = false;
        slot.frame_rect = None;
        ui.set_clip_rect(body.intersect(ui.clip_rect()));
        if let Some((texture, (w, h))) = shown {
            let native = vec2(w as f32 / ppp, h as f32 / ppp);
            // The gestures the document claims are sensed under one stable id,
            // so a drag survives the frame being rendered again under it.
            let gestures = slot
                .claims_pointer()
                .then(|| egui::Id::new(("preview-frame", pane)));
            if slot.actual_size && slot.is_raster() {
                // Actual pixels: pan by dragging the picture or with the
                // wheel, scrollbars either way; a document that claims the
                // drag or the wheel gets them instead.
                let source = slot.scroll_source();
                let output = egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .scroll_source(source)
                    .on_drag_cursor(egui::CursorIcon::Grabbing)
                    .show(ui, |ui| {
                        let image =
                            ui.add(egui::Image::from_texture(&texture).fit_to_exact_size(native));
                        match gestures {
                            Some(id) => ui.interact(image.rect, id, egui::Sense::click_and_drag()),
                            None => image,
                        }
                    });
                if output.inner.hovered()
                    && !output.inner.dragged()
                    && source.drag != egui::scroll_area::DragScroll::Never
                {
                    ctx.set_cursor_icon(egui::CursorIcon::Grab);
                }
                slot.pan = output.state.offset;
                slot.frame_rect = Some(output.inner.rect.intersect(output.inner_rect));
                slot.forward_gestures(ui, &output.inner);
            } else {
                let scale = (body.width() / native.x)
                    .min(body.height() / native.y)
                    .min(1.0);
                let size = native * scale;
                // Centred in the content both ways.
                let rect = Rect::from_center_size(body.center(), size);
                let response = ui.put(
                    rect,
                    egui::Image::from_texture(&texture).fit_to_exact_size(size),
                );
                let response = match gestures {
                    Some(id) => ui.interact(response.rect, id, egui::Sense::click_and_drag()),
                    None => response,
                };
                slot.frame_rect = Some(response.rect);
                slot.forward_gestures(ui, &response);
            }
        } else if let Some((_, view)) = slot.text.as_mut() {
            let (drawn, h_scroll) = draw_text(ui, view, TextOptions::from_settings(text_settings));
            slot.drawn = drawn;
            slot.h_scroll = h_scroll;
            slot.frame_rect = Some(body);
        } else if let Some(error) = &slot.error {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(t!("preview-failed", error = error.as_str()))
                        .color(ui.visuals().warn_fg_color),
                )
                .wrap(),
            );
        } else if target.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak(if slot.has_doc() {
                    "Rendering…"
                } else {
                    "Opening…"
                });
            });
        } else {
            ui.weak(t!("inspector-select-a-file-to-preview"));
        }
        act
    }
}

/// What a button of the preview's own asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewAct {
    /// Serve the next chunk of a remote file shown as a prefix.
    LoadMore,
    /// Open a remote file above the size cap whatever its size.
    PreviewAnyway,
}

/// The *Load more* bar at the foot of a prefix: takes its row off the
/// bottom of the space, so the content gets what is left above it.
pub fn prefix_bar(ui: &mut egui::Ui, served: u64, total: Option<u64>) -> Option<PreviewAct> {
    let whole = ui.available_rect_before_wrap();
    let bar_h = ui.text_style_height(&egui::TextStyle::Body) + 10.0;
    let bar = Rect::from_min_max(pos2(whole.left(), whole.bottom() - bar_h), whole.max);
    let mut foot = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    foot.add_space(6.0);
    foot.weak(match total {
        Some(total) => format!(
            "Showing the first {} of {}",
            crate::format::size(Some(served)),
            crate::format::size(Some(total))
        ),
        None => format!("Showing the first {}", crate::format::size(Some(served))),
    });
    let act = foot
        .button(t!("preview-load-more"))
        .clicked()
        .then_some(PreviewAct::LoadMore);
    ui.set_max_height((whole.height() - bar_h).max(0.0));
    act
}

/// The cap message: the file's size against the cap and *Preview anyway*.
pub fn capped_message(ui: &mut egui::Ui, size: u64, cap: u64) -> Option<PreviewAct> {
    let mut act = None;
    ui.add_space(12.0);
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} on a remote host, above the {} preview cap",
                crate::format::size(Some(size)),
                crate::format::size(Some(cap))
            ))
            .strong(),
        );
        ui.weak(t!("preview-the-whole-file-would-be"));
        ui.add_space(6.0);
        if ui.button(t!("preview-preview-anyway")).clicked() {
            act = Some(PreviewAct::PreviewAnyway);
        }
    });
    act
}

/// Points to pixels, rounded up to the [`QUANTUM`].
#[must_use]
pub fn quantise(points: f32, ppp: f32) -> u32 {
    ((points * ppp / QUANTUM).ceil() * QUANTUM).max(QUANTUM) as u32
}

/// The gutter's width in characters for `lines` lines: at least two digits
/// and a space.
fn gutter_chars(lines: usize) -> usize {
    lines.to_string().len().max(2) + 1
}

/// The measurements one text frame is drawn with.
struct Metrics {
    font: FontId,
    row_h: f32,
    char_w: f32,
    gutter_w: f32,
    text_color: Color32,
    gutter_color: Color32,
    /// Backgrounds of a match and of the current match.
    highlight: Color32,
    current: Color32,
}

impl Metrics {
    fn new(ui: &mut egui::Ui, lines: usize, numbers: bool) -> Self {
        let font = TextStyle::Monospace.resolve(ui.style());
        let (row_h, char_w) = ui.fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, '0')));
        let char_w = char_w.max(1.0);
        let selection = ui.visuals().selection.bg_fill;
        Self {
            gutter_w: if numbers {
                gutter_chars(lines) as f32 * char_w
            } else {
                0.0
            },
            font,
            row_h: row_h.max(1.0),
            char_w,
            text_color: ui.visuals().text_color(),
            gutter_color: ui.visuals().weak_text_color(),
            highlight: selection.linear_multiply(0.45),
            current: selection,
        }
    }

    /// Paints `text` right-aligned in the gutter of the row at `y`.
    fn gutter(&self, painter: &egui::Painter, left: f32, y: f32, text: &str) {
        painter.text(
            pos2(left + self.gutter_w - self.char_w, y),
            Align2::RIGHT_TOP,
            text,
            self.font.clone(),
            self.gutter_color,
        );
    }
}

/// Draws a text frame into the Ui's available rect, only the visible lines
/// of it. Returns what was drawn and whether a horizontal scrollbar showed.
pub fn draw_text(
    ui: &mut egui::Ui,
    view: &mut TextView,
    options: TextOptions<'_>,
) -> (Vec<DrawnLine>, bool) {
    let metrics = Metrics::new(ui, view.lines.len(), options.numbers);
    if options.wrap {
        (draw_wrapped(ui, view, &metrics, &options), false)
    } else {
        draw_unwrapped(ui, view, &metrics, &options)
    }
}

/// The offset that brings the row at `row_y` into the upper third of a
/// viewport `height` high.
fn offset_for(row_y: f32, height: f32) -> f32 {
    (row_y - height / 3.0).max(0.0)
}

/// Wrap on: lines break anywhere at the text width, the wrap index says
/// which rows each line takes, and only the lines in the viewport are laid
/// out. Continuation rows start at the text column with a mark in the
/// gutter. A line whose real row count differs from the estimate corrects
/// the index for the next frame.
fn draw_wrapped(
    ui: &mut egui::Ui,
    view: &mut TextView,
    m: &Metrics,
    options: &TextOptions<'_>,
) -> Vec<DrawnLine> {
    let avail = ui.available_rect_before_wrap();
    let text_w = (avail.width() - m.gutter_w).max(m.char_w);
    let cols = ((text_w / m.char_w).floor() as u32).max(1);
    let total_h = view.wrap_index(cols).total() as f32 * m.row_h;
    let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if let Some(line) = options.scroll_to_line {
        let row = view.wrap_index(cols).starts.get(line).copied().unwrap_or(0);
        area = area.vertical_scroll_offset(offset_for(row as f32 * m.row_h, avail.height()));
    }
    let mut drawn = Vec::new();
    area.show_viewport(ui, |ui, viewport| {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(
            vec2(width, total_h.max(viewport.height())),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(ui.clip_rect());
        let first_row = (viewport.min.y / m.row_h).floor().max(0.0) as u32;
        let mut line = view.wrap_index(cols).line_at(first_row);
        let mut fixes = Vec::new();
        while line < view.lines.len() {
            let start_row = view.wrap_index(cols).starts[line];
            let y = rect.top() + start_row as f32 * m.row_h;
            if y > rect.top() + viewport.max.y {
                break;
            }
            let mut job = LayoutJob::default();
            job.wrap.max_width = text_w;
            job.wrap.break_anywhere = true;
            append_line(&mut job, &view.block, &view.lines[line], m, options);
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let rows = galley.rows.len().max(1);
            let x = rect.left() + m.gutter_w;
            painter.galley(pos2(x, y), galley.clone(), m.text_color);
            if options.numbers {
                m.gutter(&painter, rect.left(), y, &(line + 1).to_string());
                for row in 1..rows {
                    let row_y = y + row as f32 * m.row_h;
                    m.gutter(&painter, rect.left(), row_y, icons::CONTINUATION);
                }
            }
            drawn.push(DrawnLine {
                line,
                rows,
                width: galley.rect.width(),
                x,
                gutter_x: rect.left(),
            });
            if view.wrap_index(cols).rows[line] as usize != rows {
                fixes.push((line, rows as u32));
            }
            line += 1;
        }
        if !fixes.is_empty() {
            let index = view.wrap_index(cols);
            for (line, rows) in fixes {
                index.rows[line] = rows;
            }
            index.dirty = true;
            ui.ctx().request_repaint();
        }
    });
    drawn
}

/// Wrap off: one row per line, the content as wide as the longest line, a
/// scrollbar each way, only the lines in the viewport laid out.
fn draw_unwrapped(
    ui: &mut egui::Ui,
    view: &mut TextView,
    m: &Metrics,
    options: &TextOptions<'_>,
) -> (Vec<DrawnLine>, bool) {
    let avail_h = ui.available_height();
    let total_h = view.lines.len() as f32 * m.row_h;
    let content_w = m.gutter_w + (view.max_cells as f32 + 1.0) * m.char_w;
    let mut area = egui::ScrollArea::both().auto_shrink([false, false]);
    if let Some(line) = options.scroll_to_line {
        area = area.vertical_scroll_offset(offset_for(line as f32 * m.row_h, avail_h));
    }
    let mut drawn = Vec::new();
    let output = area.show_viewport(ui, |ui, viewport| {
        let (rect, _) = ui.allocate_exact_size(
            vec2(
                content_w.max(viewport.width()),
                total_h.max(viewport.height()),
            ),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(ui.clip_rect());
        let first = (viewport.min.y / m.row_h).floor().max(0.0) as usize;
        let last = ((viewport.max.y / m.row_h).ceil().max(0.0) as usize).min(view.lines.len());
        for line in first..last {
            let y = rect.top() + line as f32 * m.row_h;
            let mut job = LayoutJob::default();
            job.wrap.max_width = f32::INFINITY;
            append_line(&mut job, &view.block, &view.lines[line], m, options);
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let x = rect.left() + m.gutter_w;
            painter.galley(pos2(x, y), galley.clone(), m.text_color);
            if options.numbers {
                m.gutter(&painter, rect.left(), y, &(line + 1).to_string());
            }
            drawn.push(DrawnLine {
                line,
                rows: galley.rows.len().max(1),
                width: galley.rect.width(),
                x,
                gutter_x: rect.left(),
            });
        }
    });
    let h_scroll = output.content_size.x > output.inner_rect.width() + 0.5;
    (drawn, h_scroll)
}

fn format(font: &FontId, color: Color32, italics: bool, background: Color32) -> TextFormat {
    TextFormat {
        font_id: font.clone(),
        color,
        italics,
        background,
        ..TextFormat::default()
    }
}

/// Appends one line of the block with its spans and the highlights that
/// touch it; gaps take the default colour. The text is cut at every span
/// and highlight edge so each run has one colour and one background.
fn append_line(
    job: &mut LayoutJob,
    block: &TextBlock,
    line: &Range<usize>,
    m: &Metrics,
    options: &TextOptions<'_>,
) {
    let text = &block.text;
    // Spans are sorted; find the first that may touch this line.
    let first_span = block.spans.partition_point(|s| s.end <= line.start);
    let spans = block.spans[first_span..]
        .iter()
        .take_while(|s| s.start < line.end);
    let highlights = options
        .highlights
        .iter()
        .enumerate()
        .filter(|(_, h)| h.start < line.end && h.end > line.start);
    let mut edges: Vec<usize> = vec![line.start, line.end];
    for span in spans.clone() {
        edges.push(span.start.clamp(line.start, line.end));
        edges.push(span.end.clamp(line.start, line.end));
    }
    for (_, h) in highlights.clone() {
        edges.push(h.start.clamp(line.start, line.end));
        edges.push(h.end.clamp(line.start, line.end));
    }
    edges.sort_unstable();
    edges.dedup();
    for pair in edges.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        let span = spans.clone().find(|s| s.start <= start && end <= s.end);
        let (color, italic) = span.map_or((m.text_color, false), |s| {
            (
                Color32::from_rgba_unmultiplied(s.color[0], s.color[1], s.color[2], s.color[3]),
                s.italic,
            )
        });
        let highlight = highlights
            .clone()
            .find(|(_, h)| h.start <= start && end <= h.end);
        let background = match highlight {
            Some((i, _)) if options.current == Some(i) => m.current,
            Some(_) => m.highlight,
            None => Color32::TRANSPARENT,
        };
        job.append(
            &text[start..end],
            0.0,
            format(&m.font, color, italic, background),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_on_newlines_and_keep_a_trailing_line() {
        assert_eq!(line_ranges("a\nbc\n"), [0..1, 2..4]);
        assert_eq!(line_ranges("a\r\nb"), [0..1, 3..4]);
        let empty = line_ranges("");
        assert_eq!((empty.len(), empty[0].clone()), (1, 0..0));
        let one = line_ranges("x");
        assert_eq!((one.len(), one[0].clone()), (1, 0..1));
        let view = TextView::new(TextBlock {
            text: "ab\ncd\nef".to_owned(),
            language: None,
            spans: Vec::new(),
        });
        assert_eq!(view.line_of(0), 0);
        assert_eq!(view.line_of(4), 1);
        assert_eq!(view.line_of(7), 2);
    }

    #[test]
    fn the_wrap_index_counts_rows_per_line_and_finds_lines_by_row() {
        let view = TextView::new(TextBlock {
            text: "short\n0123456789abcdef\n\n\tx\n".to_owned(),
            language: None,
            spans: Vec::new(),
        });
        assert_eq!(view.cells, [5, 16, 0, TAB_COLUMNS + 1]);
        assert_eq!(view.max_cells, 16);
        let index = WrapIndex::new(&view.cells, 6);
        assert_eq!(index.rows, [1, 3, 1, 1]);
        assert_eq!(index.starts, [0, 1, 4, 5, 6]);
        assert_eq!(index.total(), 6);
        assert_eq!(index.line_at(0), 0);
        assert_eq!(index.line_at(1), 1);
        assert_eq!(index.line_at(3), 1);
        assert_eq!(index.line_at(4), 2);
        assert_eq!(index.line_at(5), 3);
        assert_eq!(gutter_chars(7), 3);
        assert_eq!(gutter_chars(200_000), 7);
    }

    #[test]
    fn highlights_cut_a_line_into_runs_with_backgrounds() {
        let block = TextBlock {
            text: "one two three".to_owned(),
            language: None,
            spans: vec![ext::TextSpan {
                start: 4,
                end: 7,
                color: [255, 0, 0, 255],
                bold: false,
                italic: false,
            }],
        };
        let m = Metrics {
            font: FontId::monospace(12.0),
            row_h: 12.0,
            char_w: 6.0,
            gutter_w: 0.0,
            text_color: Color32::WHITE,
            gutter_color: Color32::GRAY,
            highlight: Color32::DARK_BLUE,
            current: Color32::BLUE,
        };
        let highlights = [2..5, 8..13];
        let options = TextOptions {
            highlights: &highlights,
            current: Some(1),
            ..TextOptions::default()
        };
        let mut job = LayoutJob::default();
        append_line(&mut job, &block, &(0..13), &m, &options);
        let runs: Vec<(String, Color32, Color32)> = job
            .sections
            .iter()
            .map(|s| {
                (
                    job.text[s.byte_range.start.0..s.byte_range.end.0].to_owned(),
                    s.format.color,
                    s.format.background,
                )
            })
            .collect();
        assert_eq!(
            runs,
            [
                ("on".to_owned(), Color32::WHITE, Color32::TRANSPARENT),
                ("e ".to_owned(), Color32::WHITE, Color32::DARK_BLUE),
                ("t".to_owned(), Color32::RED, Color32::DARK_BLUE),
                ("wo".to_owned(), Color32::RED, Color32::TRANSPARENT),
                (" ".to_owned(), Color32::WHITE, Color32::TRANSPARENT),
                ("three".to_owned(), Color32::WHITE, Color32::BLUE),
            ]
        );
    }
}
