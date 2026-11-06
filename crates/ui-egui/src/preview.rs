//! The preview pane's content: one slot per pane holding the opened
//! document, the texture it rendered to, and the render in flight.
//!
//! Rendering decodes pixels, so it never runs on the UI thread: the slot
//! hands its document to a blocking task on the runtime and takes it back
//! with the frame through a channel it polls each frame, showing the last
//! frame or a placeholder meanwhile. Textures are cached per pane and
//! replaced only when the page, the size or the theme changes.
//!
//! Text frames are laid out here, only the visible lines of them: with wrap
//! off each line is one row and the view scrolls both ways; with wrap on
//! lines break at the pane width (anywhere, so a monospace line's row count
//! follows from its length and the whole file never needs laying out), the
//! continuation rows start at the text column and the gutter marks them.
//!
//! The content never sets the pane's width: frames are asked for at the
//! pane's width and scaled down to fit, text scrolls inside a clipped area,
//! and nothing here asks for more than the width it was given.

use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontId, Rect, TextStyle, TextureHandle, TextureOptions, pos2, vec2};
use ext::{
    ControlId, ControlValue, Frame, InfoSection, PreviewControl, PreviewDoc, RenderRequest,
    TextBlock,
};
use model::{PaneId, TextPreview};
use runtime::oneshot;
use types::{EntryId, Error};

use crate::icons;

type Rendered = (Box<dyn PreviewDoc>, Result<Frame, Error>);

/// Requests are rounded up to this many pixels so a slow resize does not
/// re-render on every frame.
const QUANTUM: f32 = 64.0;

/// The largest frame asked for at actual size, per axis, within what GPUs
/// take as one texture.
const MAX_ACTUAL: u32 = 8192;

/// A tab counts as this many columns.
const TAB_COLUMNS: u32 = 4;

/// Content kind of a text document, for the pinned section setting.
pub const KIND_TEXT: &str = "text";
/// Content kind of a single raster, for the pinned section setting.
pub const KIND_IMAGE: &str = "image";
/// Content kind of a paged document, for the pinned section setting.
pub const KIND_PAGES: &str = "pages";

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
struct TextView {
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

/// One pane's preview.
#[derive(Default)]
pub struct Slot {
    entry: Option<EntryId>,
    doc: Option<Box<dyn PreviewDoc>>,
    pending: Option<oneshot::Receiver<Rendered>>,
    rendered_for: Option<(usize, u32, u32, bool)>,
    texture: Option<TextureHandle>,
    texture_size: (u32, u32),
    text: Option<TextView>,
    /// The page shown, zero-based.
    pub page: usize,
    pages: Option<usize>,
    has_text_layer: bool,
    sections: Vec<InfoSection>,
    status: Vec<String>,
    controls: Vec<PreviewControl>,
    error: Option<String>,
    /// Control changes made while the document was out rendering.
    queued: Vec<(ControlId, ControlValue)>,
    /// Rasters at actual pixels instead of fitted to the pane.
    pub actual_size: bool,
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
        self.has_text_layer = doc.text_layer().is_some();
        self.sections = doc.sections();
        self.status = doc.status();
        self.controls = doc.controls();
    }

    fn set_doc(&mut self, doc: Box<dyn PreviewDoc>) {
        self.pages = None;
        self.take_facts(&*doc);
        self.page = 0;
        self.rendered_for = None;
        self.error = None;
        self.texture = None;
        self.text = None;
        self.doc = Some(doc);
    }

    /// Whether a render is in flight.
    #[must_use]
    pub fn rendering(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether a frame has been shown for the current document.
    #[must_use]
    pub fn has_frame(&self) -> bool {
        self.texture.is_some() || self.text.is_some()
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
        Some(match (self.pages, self.has_text_layer) {
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

    /// Sets one of the previewer's controls; the next render reflects it.
    /// While the document is out rendering the change waits for it.
    pub fn set_control(&mut self, id: ControlId, value: ControlValue) {
        match self.doc.as_mut() {
            Some(doc) => {
                if let Err(error) = doc.set_control(id, value) {
                    tracing::warn!(%error, control = id.0, "control not set");
                }
                self.controls = doc.controls();
                self.sections = doc.sections();
                self.rendered_for = None;
            }
            None if self.pending.is_some() => self.queued.push((id, value)),
            None => {}
        }
    }

    /// Takes a finished render back, if any.
    fn poll(&mut self, ctx: &egui::Context, pane: PaneId) {
        let Some(rx) = self.pending.as_mut() else {
            return;
        };
        match rx.try_recv() {
            Ok((mut doc, result)) => {
                self.pending = None;
                let queued = std::mem::take(&mut self.queued);
                for (id, value) in queued {
                    if let Err(error) = doc.set_control(id, value) {
                        tracing::warn!(%error, control = id.0, "control not set");
                    }
                    self.rendered_for = None;
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
                        self.texture = Some(ctx.load_texture(
                            format!("preview-{}", pane.0),
                            image,
                            TextureOptions::LINEAR,
                        ));
                        self.texture_size = (width, height);
                        self.text = None;
                        self.error = None;
                    }
                    Ok(Frame::Text(block)) => {
                        self.text = Some(TextView::new(block));
                        self.texture = None;
                        self.error = None;
                    }
                    Ok(Frame::External(_)) => {
                        self.error = Some("external textures are not supported".to_owned());
                    }
                    Err(error) => self.error = Some(error.to_string()),
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

    /// Starts a render for `req` on the runtime's blocking pool when the
    /// shown frame is not for this page, size or theme.
    fn request(&mut self, ctx: &egui::Context, req: RenderRequest) {
        let key = (self.page, req.width, req.height, req.dark);
        if self.pending.is_some() || self.rendered_for == Some(key) {
            return;
        }
        // Text lays itself out; one render per document and theme is enough.
        if self.text.is_some()
            && self
                .rendered_for
                .is_some_and(|(_, _, _, dark)| dark == req.dark)
        {
            self.rendered_for = Some(key);
            return;
        }
        let Some(mut doc) = self.doc.take() else {
            return;
        };
        let page = self.page;
        let (tx, rx) = oneshot::channel();
        runtime::handle().spawn_blocking(move || {
            let result = doc.render(page, req);
            let _ = tx.send((doc, result));
        });
        self.pending = Some(rx);
        self.rendered_for = Some(key);
        ctx.request_repaint_after(Duration::from_millis(16));
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
        slot.error = Some(error.to_string());
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
    /// frame fitted to it, text scrolling inside it, or a placeholder.
    pub fn content(
        &mut self,
        ui: &mut egui::Ui,
        pane: PaneId,
        target: Option<EntryId>,
        text_settings: TextPreview,
    ) {
        let slot = self.slot(pane);
        let ctx = ui.ctx().clone();
        let body = ui.available_rect_before_wrap();
        let ppp = ctx.pixels_per_point();
        let quantise =
            |points: f32| ((points * ppp / QUANTUM).ceil() * QUANTUM).max(QUANTUM) as u32;
        let (width, height) = if slot.actual_size && slot.is_raster() {
            (MAX_ACTUAL, MAX_ACTUAL)
        } else {
            (quantise(body.width()), quantise(body.height()))
        };
        let req = RenderRequest {
            width,
            height,
            scale: ppp,
            dark: ui.visuals().dark_mode,
        };
        slot.request(&ctx, req);

        slot.drawn.clear();
        slot.h_scroll = false;
        slot.frame_rect = None;
        ui.set_clip_rect(body.intersect(ui.clip_rect()));
        if let Some(texture) = &slot.texture {
            let (w, h) = slot.texture_size;
            let native = vec2(w as f32 / ppp, h as f32 / ppp);
            if slot.actual_size && slot.is_raster() {
                let output = egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(egui::Image::from_texture(texture).fit_to_exact_size(native))
                    });
                slot.frame_rect = Some(output.inner.rect.intersect(output.inner_rect));
            } else {
                let scale = (body.width() / native.x)
                    .min(body.height() / native.y)
                    .min(1.0);
                let size = native * scale;
                let rect =
                    Rect::from_center_size(pos2(body.center().x, body.top() + size.y / 2.0), size);
                let response = ui.put(
                    rect,
                    egui::Image::from_texture(texture).fit_to_exact_size(size),
                );
                slot.frame_rect = Some(response.rect);
            }
        } else if let Some(view) = slot.text.as_mut() {
            let (drawn, h_scroll) =
                draw_text(ui, view, text_settings.wrap, text_settings.line_numbers);
            slot.drawn = drawn;
            slot.h_scroll = h_scroll;
            slot.frame_rect = Some(body);
        } else if let Some(error) = &slot.error {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("No preview: {error}"))
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
            ui.weak("Select a file to preview it");
        }
    }
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
}

impl Metrics {
    fn new(ui: &mut egui::Ui, lines: usize, numbers: bool) -> Self {
        let font = TextStyle::Monospace.resolve(ui.style());
        let (row_h, char_w) = ui.fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, '0')));
        let char_w = char_w.max(1.0);
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
fn draw_text(
    ui: &mut egui::Ui,
    view: &mut TextView,
    wrap: bool,
    numbers: bool,
) -> (Vec<DrawnLine>, bool) {
    let metrics = Metrics::new(ui, view.lines.len(), numbers);
    if wrap {
        (draw_wrapped(ui, view, &metrics, numbers), false)
    } else {
        draw_unwrapped(ui, view, &metrics, numbers)
    }
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
    numbers: bool,
) -> Vec<DrawnLine> {
    let avail = ui.available_rect_before_wrap();
    let text_w = (avail.width() - m.gutter_w).max(m.char_w);
    let cols = ((text_w / m.char_w).floor() as u32).max(1);
    let total_h = view.wrap_index(cols).total() as f32 * m.row_h;
    let mut drawn = Vec::new();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
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
                append_line(
                    &mut job,
                    &view.block,
                    &view.lines[line],
                    &m.font,
                    m.text_color,
                );
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                let rows = galley.rows.len().max(1);
                let x = rect.left() + m.gutter_w;
                painter.galley(pos2(x, y), galley.clone(), m.text_color);
                if numbers {
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
    numbers: bool,
) -> (Vec<DrawnLine>, bool) {
    let total_h = view.lines.len() as f32 * m.row_h;
    let content_w = m.gutter_w + (view.max_cells as f32 + 1.0) * m.char_w;
    let mut drawn = Vec::new();
    let output = egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
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
                append_line(
                    &mut job,
                    &view.block,
                    &view.lines[line],
                    &m.font,
                    m.text_color,
                );
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                let x = rect.left() + m.gutter_w;
                painter.galley(pos2(x, y), galley.clone(), m.text_color);
                if numbers {
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

fn format(font: &FontId, color: Color32, italics: bool) -> TextFormat {
    TextFormat {
        font_id: font.clone(),
        color,
        italics,
        ..TextFormat::default()
    }
}

/// Appends one line of the block with its spans; gaps take the default
/// colour.
fn append_line(
    job: &mut LayoutJob,
    block: &TextBlock,
    line: &Range<usize>,
    font: &FontId,
    default: Color32,
) {
    let text = &block.text;
    let mut cursor = line.start;
    // Spans are sorted; find the first that may touch this line.
    let first = block.spans.partition_point(|s| s.end <= line.start);
    for span in &block.spans[first..] {
        if span.start >= line.end {
            break;
        }
        let start = span.start.max(line.start);
        let end = span.end.min(line.end);
        if start > cursor {
            job.append(&text[cursor..start], 0.0, format(font, default, false));
        }
        let color = Color32::from_rgba_unmultiplied(
            span.color[0],
            span.color[1],
            span.color[2],
            span.color[3],
        );
        job.append(&text[start..end], 0.0, format(font, color, span.italic));
        cursor = end;
    }
    if cursor < line.end {
        job.append(&text[cursor..line.end], 0.0, format(font, default, false));
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
}
