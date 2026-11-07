//! The viewer tab: one file of its location full size, with the folder
//! behind it as a slideshow. The tab is a copy of the explorer it was
//! opened from (listing, sort, filter), its selection the file shown, so
//! Left and Right step through the files in the list's order, the title
//! says "name · 12 of 240", and the Action registry works on the file as it
//! does in the list.
//!
//! Top to bottom: a toolbar (previous and next item, the zoom, the pager
//! for paged content, the text toggles, the previewer's own controls; on the
//! right fullscreen, the actions menu, the info panel and find), the find
//! bar when open, the body — the page strip on the left for paged content,
//! the content, the info panel on the right when open — the transport bar
//! when the document is media, and a status bar. Fullscreen draws the same
//! without the window's chrome.
//!
//! The documents come through the pane's [`Slot`], like the Preview tab's;
//! the viewer asks it for the frames it needs (the pages in view, the
//! strip's thumbnails) and the slot renders them one at a time. Zoom is
//! remembered per file type for the session.

use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use app::Command;
use egui::{Align, Rect, pos2, vec2};
use ext::{ControlId, ControlValue, PreviewControl, TextLayer};
use model::{SelectionChange, SettingsChange, Tab};
use types::{Entry, EntryId, EntryKind};

use crate::icons;
use crate::inspector::{self, MIN_WIDTH, TOOLBAR_HEIGHT, ToolbarItem};
use crate::menus;
use crate::preview::{MAX_ACTUAL, RenderKey, Slot, TextOptions, draw_text, quantise};
use crate::shell::{PaneCx, Request};

/// The page strip's width in points.
pub const STRIP_WIDTH: f32 = 96.0;
/// The info panel's width in points.
pub const INFO_WIDTH: f32 = 260.0;
/// The gap between pages in the continuous view.
const PAGE_GAP: f32 = 12.0;
/// The pixel size the strip's thumbnails are rendered at.
const THUMB_PX: u32 = 128;
/// A zoom step with the keys or the wheel buttons.
const ZOOM_STEP: f32 = 1.25;
/// How far J and L seek.
pub const SEEK_STEP: Duration = Duration::from_secs(10);
/// The aspect assumed for a page not yet rendered: A4.
const DEFAULT_ASPECT: f32 = 1.414;

/// How the viewer scales its content.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Zoom {
    /// Fitted into the content, never above 100% for a raster.
    #[default]
    Fit,
    /// As wide as the content.
    FitWidth,
    /// One pixel per pixel for a raster; a page is shown as wide as the
    /// content, since a document does not say how large its pages are.
    Actual,
    /// A factor on the fitted size: 2.0 is twice Fit.
    Free(f32),
}

/// A step through the folder's files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// So many files on.
    By(isize),
    /// The first file.
    First,
    /// The last file.
    Last,
}

/// Find in the text layer.
#[derive(Debug, Default)]
pub struct Find {
    /// The text looked for.
    pub query: String,
    /// The matches: the page and the byte range in it.
    pub matches: Vec<(usize, Range<usize>)>,
    /// The current match, an index into `matches`.
    pub current: Option<usize>,
    /// Focus the text field on the next frame.
    pub focus: bool,
    /// What the matches were made from: the query and the layer's size.
    searched: Option<(String, usize, usize)>,
    /// Bring the current match into view on the next frame.
    jump: bool,
}

impl Find {
    /// Searches the layer again when the query or the layer changed, the
    /// first match becoming current.
    fn refresh(&mut self, layer: Option<&TextLayer>) {
        let size = layer.map_or((0, 0), |l| {
            (l.pages.len(), l.pages.iter().map(String::len).sum())
        });
        let key = (self.query.clone(), size.0, size.1);
        if self.searched.as_ref() == Some(&key) {
            return;
        }
        let same_query = self.searched.as_ref().is_some_and(|k| k.0 == key.0);
        self.searched = Some(key);
        self.matches = match layer {
            Some(layer) if !self.query.is_empty() => search(layer, &self.query),
            _ => Vec::new(),
        };
        self.current = if self.matches.is_empty() {
            None
        } else if same_query {
            Some(self.current.unwrap_or(0).min(self.matches.len() - 1))
        } else {
            Some(0)
        };
        self.jump = !same_query && self.current.is_some();
    }

    /// Moves to the next or previous match, around the ends.
    fn step(&mut self, by: isize) {
        let n = self.matches.len();
        if n == 0 {
            return;
        }
        let at = self.current.unwrap_or(0) as isize;
        self.current = Some((at + by).rem_euclid(n as isize) as usize);
        self.jump = true;
    }

    /// The byte ranges of the matches on `page`.
    fn ranges_on(&self, page: usize) -> Vec<Range<usize>> {
        self.matches
            .iter()
            .filter(|(p, _)| *p == page)
            .map(|(_, r)| r.clone())
            .collect()
    }

    /// The index among the matches on `page` of the current match, when
    /// it is on that page.
    fn current_on(&self, page: usize) -> Option<usize> {
        let current = self.current?;
        if self.matches.get(current)?.0 != page {
            return None;
        }
        Some(
            self.matches[..current]
                .iter()
                .filter(|(p, _)| *p == page)
                .count(),
        )
    }
}

/// Every match of `query` in the layer, ignoring case where the lower-cased
/// text keeps its byte offsets (ASCII and most scripts), else exactly.
#[must_use]
pub fn search(layer: &TextLayer, query: &str) -> Vec<(usize, Range<usize>)> {
    let needle = query.to_lowercase();
    let mut out = Vec::new();
    for (page, text) in layer.pages.iter().enumerate() {
        let lowered = text.to_lowercase();
        let (hay, needle): (&str, &str) = if lowered.len() == text.len() {
            (&lowered, &needle)
        } else {
            (text, query)
        };
        if needle.is_empty() {
            continue;
        }
        let mut from = 0;
        while let Some(at) = hay[from..].find(needle) {
            let start = from + at;
            out.push((page, start..start + needle.len()));
            from = start + needle.len().max(1);
            if from >= hay.len() {
                break;
            }
        }
    }
    out
}

/// Where the viewer's parts were drawn in the last frame, for tests.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ViewerRects {
    /// The toolbar row.
    pub toolbar: Option<Rect>,
    /// The find bar, when open.
    pub find: Option<Rect>,
    /// The page strip, for paged content.
    pub strip: Option<Rect>,
    /// The content.
    pub content: Option<Rect>,
    /// The info panel, when open.
    pub info: Option<Rect>,
    /// The transport bar, for media.
    pub transport: Option<Rect>,
    /// The status bar.
    pub status: Option<Rect>,
}

/// One viewer tab's state between frames.
#[derive(Debug, Default)]
pub struct ViewerState {
    /// How the content is scaled.
    pub zoom: Zoom,
    /// Pages stacked in one scroll, or one at a time.
    pub continuous: bool,
    /// The info panel is open.
    pub info_open: bool,
    /// Find, while its bar is open.
    pub find: Option<Find>,
    /// The page counter's text while it is edited.
    pub page_entry: String,
    page_entry_focus: bool,
    /// Scroll the continuous view to this page on the next frame.
    scroll_to_page: Option<usize>,
    /// The page the strip last centred on.
    strip_centred: Option<usize>,
    /// The entry shown in the last frame, to notice a change.
    shown: Option<EntryId>,
    /// The index among the files of the file shown last, to pick a
    /// neighbour when it goes.
    pub last_index: Option<usize>,
    /// What the last frame drew.
    pub rects: ViewerRects,
    /// The toolbar's controls of the last frame, left to right.
    pub toolbar: Vec<ToolbarItem>,
    /// The status bar's text of the last frame.
    pub status: String,
    /// The pages whose strip tiles were drawn last frame.
    pub strip_pages: Vec<usize>,
    /// The pages whose strip tiles showed a thumbnail last frame.
    pub strip_thumbs: Vec<usize>,
    /// The size the frame was drawn at in the last frame, in points.
    pub frame_size: Option<egui::Vec2>,
    /// The content's text as drawn, for tests.
    pub drawn_highlights: usize,
    /// Scroll the text to this line on the next frame.
    scroll_to_line: Option<usize>,
}

/// What the viewer draws from besides the pane.
pub struct ViewerCx<'a> {
    /// This tab's state.
    pub state: &'a mut ViewerState,
    /// The pane's document and frames.
    pub slot: &'a mut Slot,
    /// The zoom remembered per file type, shared by every viewer.
    pub zoom_memory: &'a mut HashMap<String, Zoom>,
    /// Whether this viewer fills the screen.
    pub fullscreen: bool,
}

/// What a control asked for.
enum Act {
    Zoom(Zoom),
    Page(isize),
    GoToPage(usize),
    Continuous(bool),
    Info(bool),
    Find(bool),
    FindStep(isize),
    Fullscreen(bool),
    Setting(SettingsChange),
    Control(ControlId, ControlValue),
    Step(Step),
    PlayPause,
    Seek(Duration),
}

/// The files of the tab in view order: what the viewer steps through.
#[must_use]
pub fn files(tab: &Tab) -> Vec<EntryId> {
    tab.rows()
        .filter(|e| e.kind != EntryKind::Dir)
        .map(|e| e.id)
        .collect()
}

/// The entry the viewer shows: the tab's one selected entry.
#[must_use]
pub fn shown(tab: &Tab) -> Option<&Entry> {
    let mut selected = tab.selected();
    let first = selected.next()?;
    if selected.next().is_some() {
        return None;
    }
    Some(first)
}

/// The shown file's position among the folder's files, one-based, and
/// their count.
#[must_use]
pub fn position(tab: &Tab) -> Option<(usize, usize)> {
    let shown = shown(tab)?.id;
    let files = files(tab);
    let at = files.iter().position(|id| *id == shown)?;
    Some((at + 1, files.len()))
}

/// The file `step` reaches from the shown one; `None` when there is none
/// to go to.
#[must_use]
pub fn step(tab: &Tab, step: Step) -> Option<EntryId> {
    let files = files(tab);
    if files.is_empty() {
        return None;
    }
    let last = files.len() - 1;
    let target = match step {
        Step::First => 0,
        Step::Last => last,
        Step::By(by) => {
            let at = shown(tab).and_then(|e| files.iter().position(|id| *id == e.id))?;
            (at as isize + by).clamp(0, last as isize) as usize
        }
    };
    let id = files[target];
    (Some(id) != shown(tab).map(|e| e.id)).then_some(id)
}

/// The tab's title: the file's icon and name with its position.
#[must_use]
pub fn title(tab: &Tab) -> String {
    match shown(tab) {
        Some(entry) => match position(tab) {
            Some((at, of)) => format!("{} {} · {at} of {of}", icons::for_entry(entry), entry.name),
            None => format!("{} {}", icons::for_entry(entry), entry.name),
        },
        None => format!(
            "{} {}",
            icons::PREVIEW,
            crate::paths::title(&tab.location.path)
        ),
    }
}

/// The key the zoom is remembered under: the lower-case extension, or the
/// empty string for a name without one.
#[must_use]
pub fn type_key(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// The request to render a frame with, in pixels: the content for Fit, the
/// width with room below for Fit width, the native size of a raster for
/// Actual and the free factors, a page at its factor otherwise.
fn request_size(zoom: Zoom, content: egui::Vec2, ppp: f32, raster: bool) -> (u32, u32) {
    let q = |points: f32| quantise(points, ppp).min(MAX_ACTUAL);
    match zoom {
        Zoom::Fit => (q(content.x), q(content.y)),
        Zoom::Actual | Zoom::Free(_) if raster => (MAX_ACTUAL, MAX_ACTUAL),
        Zoom::FitWidth | Zoom::Actual => (q(content.x), MAX_ACTUAL),
        Zoom::Free(f) => (q(content.x * f), q(content.y * f)),
    }
}

/// The size to draw a frame of `native` points at.
fn display_size(zoom: Zoom, native: egui::Vec2, content: egui::Vec2, raster: bool) -> egui::Vec2 {
    let (nw, nh) = (native.x.max(1.0), native.y.max(1.0));
    let fit = (content.x / nw).min(content.y / nh);
    let fit = if raster { fit.min(1.0) } else { fit };
    match zoom {
        Zoom::Fit => native * fit,
        Zoom::Actual if raster => native,
        Zoom::FitWidth | Zoom::Actual => vec2(content.x, nh * content.x / nw),
        Zoom::Free(f) => native * fit * f,
    }
}

/// The zoom factor relative to Fit of a frame drawn at `size`.
fn factor_of(size: egui::Vec2, native: egui::Vec2, content: egui::Vec2, raster: bool) -> f32 {
    let fit = display_size(Zoom::Fit, native, content, raster);
    if fit.x > 0.0 { size.x / fit.x } else { 1.0 }
}

/// Draws the viewer. `cx` is the tab's context, `v` the viewer's own.
// The tab is one layout, top to bottom.
#[allow(clippy::too_many_lines)]
pub fn show(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, v: &mut ViewerCx<'_>) {
    let width = ui.available_width().max(MIN_WIDTH);
    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
    ui.spacing_mut().item_spacing.y = 0.0;
    let entry = shown(cx.tab).cloned();
    let current = entry.as_ref().map(|e| e.id);
    if v.state.shown != current {
        // Another file: its type's zoom, find anew, from the top.
        v.state.shown = current;
        v.state.zoom = entry
            .as_ref()
            .and_then(|e| v.zoom_memory.get(&type_key(&e.name)).copied())
            .unwrap_or_default();
        v.state.scroll_to_page = Some(0);
        v.state.strip_centred = None;
        v.state.page_entry.clear();
        if let Some(find) = &mut v.state.find {
            find.searched = None;
        }
    }
    match (&entry, position(cx.tab)) {
        (Some(_), Some((at, _))) => v.state.last_index = Some(at - 1),
        (None, _) if cx.tab.listing.complete => {
            // The file went (trashed, renamed away): show a neighbour.
            let files = files(cx.tab);
            if let Some(id) = v
                .state
                .last_index
                .and_then(|i| files.get(i.min(files.len().saturating_sub(1))))
                .or_else(|| files.first())
            {
                cx.requests.push(Request::Command(Command::Select {
                    pane: cx.pane,
                    change: SelectionChange::Set(vec![*id]),
                }));
            }
        }
        _ => {}
    }
    if let Some(find) = &mut v.state.find {
        find.refresh(v.slot.text_layer());
    }

    let mut acts = Vec::new();
    // Toolbar.
    let (row, _) = ui.allocate_exact_size(vec2(width, TOOLBAR_HEIGHT), egui::Sense::hover());
    v.state.rects.toolbar = Some(row);
    v.state.toolbar.clear();
    toolbar(ui, row, cx, v, entry.as_ref(), &mut acts);
    separator(ui, width);

    // Find bar.
    v.state.rects.find = None;
    if v.state.find.is_some() {
        let (bar, _) = ui.allocate_exact_size(vec2(width, TOOLBAR_HEIGHT), egui::Sense::hover());
        v.state.rects.find = Some(bar);
        find_bar(ui, bar, v, &mut acts);
        separator(ui, width);
    }

    // The foot: the status bar and, for media, the transport.
    let status_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    let has_transport = v.slot.transport().is_some();
    let transport_h = if has_transport { TOOLBAR_HEIGHT } else { 0.0 };
    let body_h = (ui.available_height() - status_h - transport_h - 2.0).max(0.0);
    let (body, _) = ui.allocate_exact_size(vec2(width, body_h), egui::Sense::hover());

    // The body: strip, content, info panel.
    let pages = v.slot.pages();
    let strip_w = if pages.is_some_and(|n| n > 1) {
        STRIP_WIDTH.min(body.width() * 0.3)
    } else {
        0.0
    };
    let info_w = if v.state.info_open {
        INFO_WIDTH.min(body.width() * 0.5)
    } else {
        0.0
    };
    let strip = Rect::from_min_max(body.min, pos2(body.left() + strip_w, body.bottom()));
    let info = Rect::from_min_max(pos2(body.right() - info_w, body.top()), body.max);
    let content = Rect::from_min_max(
        pos2(
            strip.right() + if strip_w > 0.0 { 1.0 } else { 0.0 },
            body.top(),
        ),
        pos2(
            info.left() - if info_w > 0.0 { 1.0 } else { 0.0 },
            body.bottom(),
        ),
    );
    v.state.rects.content = Some(content);
    v.state.rects.strip = (strip_w > 0.0).then_some(strip);
    v.state.rects.info = (info_w > 0.0).then_some(info);
    // The content first, so its frames render before the strip's.
    draw_content(ui, cx, v, entry.as_ref(), content, &mut acts);
    if let Some(pages) = pages.filter(|_| strip_w > 0.0) {
        vline(ui, strip.right(), strip.y_range());
        draw_strip(ui, strip, v, pages, &mut acts);
    }
    if info_w > 0.0 {
        vline(ui, info.left() - 1.0, info.y_range());
        draw_info(ui, cx, v, entry.as_ref(), info);
    }

    // Transport.
    v.state.rects.transport = None;
    if has_transport {
        separator(ui, width);
        let (bar, _) = ui.allocate_exact_size(vec2(width, TOOLBAR_HEIGHT), egui::Sense::hover());
        v.state.rects.transport = Some(bar);
        transport_bar(ui, bar, v, &mut acts);
    }

    // Status.
    separator(ui, width);
    let (status, _) = ui.allocate_exact_size(vec2(width, status_h), egui::Sense::hover());
    v.state.rects.status = Some(status);
    let text = if v.slot.status().is_empty() {
        entry.as_ref().map(model::type_of).unwrap_or_default()
    } else {
        v.slot.status().join(" · ")
    };
    v.state.status.clone_from(&text);
    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(status.shrink2(vec2(6.0, 0.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    bar.set_clip_rect(status.intersect(ui.clip_rect()));
    bar.add(egui::Label::new(text).selectable(false).truncate());

    apply(cx, v, entry.as_ref(), acts);
}

/// Carries out what the controls asked for.
fn apply(cx: &mut PaneCx<'_>, v: &mut ViewerCx<'_>, entry: Option<&Entry>, acts: Vec<Act>) {
    for act in acts {
        match act {
            Act::Zoom(zoom) => set_zoom(v, entry, zoom),
            Act::Page(delta) => go_to_page(v, v.slot.page.saturating_add_signed(delta)),
            Act::GoToPage(page) => go_to_page(v, page),
            Act::Continuous(on) => {
                v.state.continuous = on;
                v.state.scroll_to_page = Some(v.slot.page);
            }
            Act::Info(on) => v.state.info_open = on,
            Act::Find(true) => {
                let find = v.state.find.get_or_insert_with(Find::default);
                find.focus = true;
            }
            Act::Find(false) => v.state.find = None,
            Act::FindStep(by) => {
                if let Some(find) = &mut v.state.find {
                    find.step(by);
                }
            }
            Act::Fullscreen(on) => cx
                .requests
                .push(Request::ViewerFullscreen { tab: cx.tab_id, on }),
            Act::Setting(change) => cx
                .requests
                .push(Request::Command(Command::Settings(change))),
            Act::Control(id, value) => v.slot.set_control(id, value),
            Act::Step(by) => {
                if let Some(id) = step(cx.tab, by) {
                    cx.requests.push(Request::Command(Command::Select {
                        pane: cx.pane,
                        change: SelectionChange::Set(vec![id]),
                    }));
                }
            }
            Act::PlayPause => {
                if let Some(t) = v.slot.transport() {
                    if t.playing() {
                        t.pause();
                    } else {
                        t.play();
                    }
                }
            }
            Act::Seek(to) => {
                if let Some(t) = v.slot.transport() {
                    t.seek(to);
                }
            }
        }
    }
}

/// Sets the zoom and remembers it for the file's type.
pub fn set_zoom(v: &mut ViewerCx<'_>, entry: Option<&Entry>, zoom: Zoom) {
    v.state.zoom = zoom;
    if let Some(entry) = entry {
        v.zoom_memory.insert(type_key(&entry.name), zoom);
    }
}

/// Shows `page`, clamped to the document.
pub fn go_to_page(v: &mut ViewerCx<'_>, page: usize) {
    let last = v.slot.pages().unwrap_or(1).saturating_sub(1);
    v.slot.page = page.min(last);
    v.state.scroll_to_page = Some(v.slot.page);
}

/// The zoom a step in or out from the current one gives: the current
/// factor relative to Fit times the step.
#[must_use]
pub fn zoom_step(state: &ViewerState, by: f32) -> Zoom {
    let factor = match (state.zoom, state.frame_size) {
        (Zoom::Free(f), _) => f,
        (Zoom::FitWidth | Zoom::Actual, Some(size)) => {
            // Fit width or actual: measure against the fitted size.
            let content = state.rects.content.map_or(size, |r| r.size());
            let fit = display_size(Zoom::Fit, size, content, true);
            if fit.x > 0.0 { size.x / fit.x } else { 1.0 }
        }
        _ => 1.0,
    };
    Zoom::Free((factor * by).clamp(0.05, 32.0))
}

/// Pages: one on, one back, or a file when there is no page that way.
pub fn page_or_step(cx: &mut PaneCx<'_>, v: &mut ViewerCx<'_>, by: isize) {
    let pages = v.slot.pages().unwrap_or(1);
    let page = v.slot.page as isize + by;
    if pages > 1 && (0..pages as isize).contains(&page) {
        go_to_page(v, page as usize);
    } else if let Some(id) = step(cx.tab, Step::By(by)) {
        cx.requests.push(Request::Command(Command::Select {
            pane: cx.pane,
            change: SelectionChange::Set(vec![id]),
        }));
    }
}

/// Seeks media by `by` from where it is, within the duration.
pub fn seek_by(slot: &mut Slot, by: Duration, back: bool) {
    if let Some(t) = slot.transport() {
        let at = t.position();
        let to = if back {
            at.saturating_sub(by)
        } else {
            let to = at + by;
            t.duration().map_or(to, |d| to.min(d))
        };
        t.seek(to);
    }
}

fn separator(ui: &mut egui::Ui, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 1.0), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}

fn vline(ui: &egui::Ui, x: f32, range: egui::Rangef) {
    ui.painter()
        .vline(x, range, ui.visuals().widgets.noninteractive.bg_stroke);
}

/// A frameless icon button; a toggle is drawn in the fill weight and
/// highlighted when on.
fn icon_button(ui: &mut egui::Ui, glyph: &str, label: &str, on: Option<bool>) -> egui::Response {
    let text = match on {
        Some(true) => egui::RichText::new(glyph)
            .family(egui::FontFamily::Name(icons::FILL_FAMILY.into()))
            .size(15.0),
        _ => egui::RichText::new(glyph).size(15.0),
    };
    ui.add(
        egui::Button::new(text)
            .frame(false)
            .selected(on == Some(true))
            .min_size(vec2(24.0, 24.0)),
    )
    .on_hover_text(label)
}

/// A toggle in the toolbar: records itself for tests, returns whether it
/// was clicked.
fn toggle(ui: &mut egui::Ui, state: &mut ViewerState, glyph: &str, label: &str, on: bool) -> bool {
    state.toolbar.push(ToolbarItem {
        label: label.to_owned(),
        on: Some(on),
    });
    icon_button(ui, glyph, label, Some(on)).clicked()
}

/// A one-shot button in the toolbar, greyed out when not `enabled`.
fn button(
    ui: &mut egui::Ui,
    state: &mut ViewerState,
    glyph: &str,
    label: &str,
    enabled: bool,
) -> bool {
    state.toolbar.push(ToolbarItem {
        label: label.to_owned(),
        on: None,
    });
    ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(glyph).size(15.0))
            .frame(false)
            .min_size(vec2(24.0, 24.0)),
    )
    .on_hover_text(label)
    .clicked()
}

/// The toolbar: the file icon, the item steps, the zoom, the pager, the
/// text toggles and the previewer's controls from the left; fullscreen,
/// the actions menu, the info panel and find from the right.
// One row of controls, left to right.
#[allow(clippy::too_many_lines)]
fn toolbar(
    ui: &mut egui::Ui,
    row: Rect,
    cx: &mut PaneCx<'_>,
    v: &mut ViewerCx<'_>,
    entry: Option<&Entry>,
    acts: &mut Vec<Act>,
) {
    let state = &mut *v.state;
    let slot = &mut *v.slot;
    // Right: find, info, actions, fullscreen.
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    right.set_clip_rect(row.intersect(ui.clip_rect()));
    right.add_space(2.0);
    let fullscreen = v.fullscreen;
    if toggle(
        &mut right,
        state,
        if fullscreen {
            icons::LEAVE_FULLSCREEN
        } else {
            icons::FULLSCREEN
        },
        if fullscreen {
            "Leave fullscreen"
        } else {
            "Fullscreen"
        },
        fullscreen,
    ) {
        acts.push(Act::Fullscreen(!fullscreen));
    }
    if let Some(entry) = entry {
        state.toolbar.push(ToolbarItem {
            label: "Actions".to_owned(),
            on: None,
        });
        let action_ctx = menus::context_for(cx, Some(entry.clone()));
        right
            .menu_button(egui::RichText::new(icons::ACTIONS).size(15.0), |ui| {
                menus::context_menu(ui, cx, &action_ctx, Some(entry));
            })
            .response
            .on_hover_text("Actions");
    }
    if toggle(
        &mut right,
        state,
        icons::INFO_PANEL,
        "Info panel",
        state.info_open,
    ) {
        acts.push(Act::Info(!state.info_open));
    }
    let finding = state.find.is_some();
    if toggle(&mut right, state, icons::FIND, "Find", finding) {
        acts.push(Act::Find(!finding));
    }
    let right_edge = right.min_rect().left();

    // Left: the file icon, the steps, then the controls.
    let left_rect = Rect::from_min_max(row.min, pos2(right_edge.max(row.left()), row.bottom()));
    let mut left = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    left.set_clip_rect(left_rect.intersect(ui.clip_rect()));
    left.add_space(6.0);
    if let Some(entry) = entry {
        left.add(
            egui::Label::new(egui::RichText::new(icons::for_entry(entry)).size(16.0))
                .selectable(false),
        );
        state.toolbar.push(ToolbarItem {
            label: "File".to_owned(),
            on: None,
        });
        left.add_space(4.0);
    }
    let position = position(cx.tab);
    let (at, of) = position.unwrap_or((0, 0));
    if button(&mut left, state, icons::BACK, "Previous item", at > 1) {
        acts.push(Act::Step(Step::By(-1)));
    }
    if button(&mut left, state, icons::FORWARD, "Next item", at < of) {
        acts.push(Act::Step(Step::By(1)));
    }
    left.add_space(6.0);

    let settings = cx.app.model().settings.preview.text;
    if slot.is_text() {
        if toggle(
            &mut left,
            state,
            icons::LINE_NUMBERS,
            "Line numbers",
            settings.line_numbers,
        ) {
            acts.push(Act::Setting(SettingsChange::PreviewLineNumbers(
                !settings.line_numbers,
            )));
        }
        if toggle(&mut left, state, icons::WRAP, "Word wrap", settings.wrap) {
            acts.push(Act::Setting(SettingsChange::PreviewWrap(!settings.wrap)));
        }
    } else if slot.has_doc() {
        let zoom = state.zoom;
        if toggle(&mut left, state, icons::FIT, "Fit", zoom == Zoom::Fit) {
            acts.push(Act::Zoom(Zoom::Fit));
        }
        if toggle(
            &mut left,
            state,
            icons::FIT_WIDTH,
            "Fit width",
            zoom == Zoom::FitWidth,
        ) {
            acts.push(Act::Zoom(Zoom::FitWidth));
        }
        if toggle(
            &mut left,
            state,
            icons::ACTUAL_SIZE,
            "1:1",
            zoom == Zoom::Actual,
        ) {
            acts.push(Act::Zoom(Zoom::Actual));
        }
        if button(&mut left, state, icons::ZOOM_OUT, "Zoom out", true) {
            acts.push(Act::Zoom(zoom_step(state, 1.0 / ZOOM_STEP)));
        }
        let label = match zoom {
            Zoom::Fit => "Fit".to_owned(),
            Zoom::FitWidth => "Width".to_owned(),
            Zoom::Actual => "1:1".to_owned(),
            Zoom::Free(f) => format!("{:.0}%", f * 100.0),
        };
        left.add(egui::Label::new(label).selectable(false))
            .on_hover_text("Zoom, relative to Fit");
        if button(&mut left, state, icons::ZOOM_IN, "Zoom in", true) {
            acts.push(Act::Zoom(zoom_step(state, ZOOM_STEP)));
        }
    }
    if let Some(pages) = slot.pages().filter(|n| *n > 1) {
        left.add_space(6.0);
        if button(
            &mut left,
            state,
            icons::PREVIOUS,
            "Previous page",
            slot.page > 0,
        ) {
            acts.push(Act::Page(-1));
        }
        page_entry(&mut left, state, slot.page, pages, acts);
        if button(
            &mut left,
            state,
            icons::NEXT,
            "Next page",
            slot.page + 1 < pages,
        ) {
            acts.push(Act::Page(1));
        }
        let continuous = state.continuous;
        if toggle(
            &mut left,
            state,
            if continuous {
                icons::CONTINUOUS
            } else {
                icons::SINGLE_PAGE
            },
            if continuous {
                "Continuous scrolling"
            } else {
                "Single page"
            },
            continuous,
        ) {
            acts.push(Act::Continuous(!continuous));
        }
    }
    for control in slot.controls().to_vec() {
        match control {
            PreviewControl::Toggle {
                id,
                icon,
                label,
                on,
            } => {
                if toggle(&mut left, state, icons::for_icon(icon), label, on) {
                    acts.push(Act::Control(id, ControlValue::Bool(!on)));
                }
            }
            PreviewControl::Button {
                id,
                icon,
                label,
                enabled,
            } => {
                if button(&mut left, state, icons::for_icon(icon), label, enabled) {
                    acts.push(Act::Control(id, ControlValue::Pressed));
                }
            }
        }
    }
}

/// The page counter with direct entry: `150` in a field, `/ 300` after it;
/// Enter goes to the page typed.
fn page_entry(
    ui: &mut egui::Ui,
    state: &mut ViewerState,
    page: usize,
    pages: usize,
    acts: &mut Vec<Act>,
) {
    state.toolbar.push(ToolbarItem {
        label: "Page".to_owned(),
        on: None,
    });
    let mut text = if state.page_entry.is_empty() {
        (page + 1).to_string()
    } else {
        state.page_entry.clone()
    };
    let digits = pages.to_string().len();
    let width = ui.fonts_mut(|f| f.glyph_width(&egui::TextStyle::Body.resolve(ui.style()), '0'))
        * (digits as f32 + 1.5)
        + 8.0;
    let output = egui::TextEdit::singleline(&mut text)
        .desired_width(width)
        .horizontal_align(Align::Center)
        .show(ui);
    if state.page_entry_focus {
        output.response.request_focus();
        state.page_entry_focus = false;
    }
    if output.response.has_focus() {
        state.page_entry.clone_from(&text);
    } else {
        state.page_entry.clear();
    }
    let enter = output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if enter && let Ok(wanted) = text.trim().parse::<usize>() {
        acts.push(Act::GoToPage(wanted.saturating_sub(1)));
    }
    ui.add(egui::Label::new(format!("/ {pages}")).selectable(false))
        .on_hover_text("Pages");
}

/// The find bar: the text field, the match count, previous, next, close.
fn find_bar(ui: &mut egui::Ui, bar: Rect, v: &mut ViewerCx<'_>, acts: &mut Vec<Act>) {
    let Some(find) = v.state.find.as_mut() else {
        return;
    };
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    row.set_clip_rect(bar.intersect(ui.clip_rect()));
    row.add_space(6.0);
    let output = egui::TextEdit::singleline(&mut find.query)
        .hint_text("Find")
        .desired_width(220.0)
        .show(&mut row);
    if find.focus {
        output.response.request_focus();
        find.focus = false;
    }
    if output.response.changed() {
        find.refresh(v.slot.text_layer());
    }
    let focused = output.response.has_focus() || output.response.lost_focus();
    if focused && row.input(|i| i.key_pressed(egui::Key::Enter)) {
        let back = row.input(|i| i.modifiers.shift);
        acts.push(Act::FindStep(if back { -1 } else { 1 }));
        find.focus = true;
    }
    if focused && row.input(|i| i.key_pressed(egui::Key::Escape)) {
        acts.push(Act::Find(false));
    }
    row.add_space(6.0);
    let count = find.matches.len();
    let text = match (find.current, count) {
        (_, 0) if find.query.is_empty() => String::new(),
        (_, 0) => "No matches".to_owned(),
        (Some(at), n) => format!("{} of {n} matches", at + 1),
        (None, n) => format!("{n} matches"),
    };
    row.add(egui::Label::new(text).selectable(false));
    if icon_button(&mut row, icons::PREVIOUS, "Previous match", None).clicked() {
        acts.push(Act::FindStep(-1));
    }
    if icon_button(&mut row, icons::NEXT, "Next match", None).clicked() {
        acts.push(Act::FindStep(1));
    }
    if icon_button(&mut row, icons::CLOSE, "Close find", None).clicked() {
        acts.push(Act::Find(false));
    }
}

/// The content: text laid out with the matches highlighted, a raster or a
/// page at the zoom, the pages stacked in the continuous view, or what
/// stands in for them.
fn draw_content(
    ui: &mut egui::Ui,
    cx: &mut PaneCx<'_>,
    v: &mut ViewerCx<'_>,
    entry: Option<&Entry>,
    rect: Rect,
    acts: &mut Vec<Act>,
) {
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    inner.set_clip_rect(rect.intersect(ui.clip_rect()));
    let state = &mut *v.state;
    let slot = &mut *v.slot;
    state.frame_size = None;
    state.drawn_highlights = 0;
    if slot.is_text() {
        draw_text_content(&mut inner, cx, state, slot);
        return;
    }
    let pages = slot.pages();
    if slot.has_doc() && (slot.has_frame() || slot.rendering() || slot.error().is_none()) {
        if pages.is_some_and(|n| n > 1) && state.continuous {
            draw_continuous(&mut inner, state, slot, rect, acts);
        } else {
            draw_single(&mut inner, state, slot, rect, acts);
        }
        return;
    }
    if let Some(error) = slot.error() {
        inner.add_space(12.0);
        inner.vertical_centered(|ui| {
            ui.label(egui::RichText::new("No preview for this file").strong());
            if !error.contains("no previewer") {
                ui.weak(error);
            }
        });
    } else if entry.is_some() {
        inner.horizontal(|ui| {
            ui.spinner();
            ui.weak(if slot.has_doc() {
                "Rendering…"
            } else {
                "Opening…"
            });
        });
    } else {
        inner.weak("Nothing to show");
    }
}

/// Text: the whole file laid out line by line with the matches
/// highlighted, scrolled to the current match when it changed.
fn draw_text_content(
    ui: &mut egui::Ui,
    cx: &mut PaneCx<'_>,
    state: &mut ViewerState,
    slot: &mut Slot,
) {
    let dark = ui.visuals().dark_mode;
    let settings = cx.app.model().settings.preview.text;
    let key = RenderKey {
        page: 0,
        width: 0,
        height: 0,
        dark,
    };
    slot.request(&ui.ctx().clone(), key, ui.ctx().pixels_per_point());
    let highlights: Vec<Range<usize>> = state
        .find
        .as_ref()
        .map(|f| f.ranges_on(0))
        .unwrap_or_default();
    let current = state.find.as_ref().and_then(|f| f.current_on(0));
    let jump = state.find.as_mut().filter(|f| f.jump).and_then(|f| {
        f.jump = false;
        let (page, range) = f.matches.get(f.current?)?.clone();
        (page == 0).then_some(range.start)
    });
    let Some(view) = slot.text_view_mut() else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.weak("Rendering…");
        });
        return;
    };
    let scroll_to_line = jump
        .map(|offset| view.line_of(offset))
        .or(state.scroll_to_line.take());
    let options = TextOptions {
        wrap: settings.wrap,
        numbers: settings.line_numbers,
        highlights: &highlights,
        current,
        scroll_to_line,
    };
    let (_, _) = draw_text(ui, view, options);
    state.drawn_highlights = highlights.len();
    state.frame_size = Some(ui.min_rect().size());
}

/// Zooms by the wheel over `rect` with Mod held: egui reports it as a zoom.
fn wheel_zoom(ui: &egui::Ui, rect: Rect) -> Option<f32> {
    let delta = ui.input(|i| {
        if i.pointer.hover_pos().is_some_and(|p| rect.contains(p)) {
            i.zoom_delta()
        } else {
            1.0
        }
    });
    ((delta - 1.0).abs() > 1e-3).then_some(delta)
}

/// One frame, a raster or the current page, at the zoom: centred when it
/// fits, panned by dragging or scrolling otherwise.
fn draw_single(
    ui: &mut egui::Ui,
    state: &mut ViewerState,
    slot: &mut Slot,
    rect: Rect,
    acts: &mut Vec<Act>,
) {
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let dark = ui.visuals().dark_mode;
    let raster = slot.pages().is_none();
    let page = slot.page;
    let (width, height) = request_size(state.zoom, rect.size(), ppp, raster);
    let key = RenderKey {
        page,
        width,
        height,
        dark,
    };
    slot.request(&ctx, key, ppp);
    let Some((texture, (w, h))) = slot.texture(key).or_else(|| slot.any_texture(page)) else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.weak("Rendering…");
        });
        return;
    };
    let native = vec2(w as f32 / ppp, h as f32 / ppp);
    let size = display_size(state.zoom, native, rect.size(), raster);
    state.frame_size = Some(size);
    if let Some(delta) = wheel_zoom(ui, rect) {
        let factor = factor_of(size, native, rect.size(), raster);
        acts.push(Act::Zoom(Zoom::Free((factor * delta).clamp(0.05, 32.0))));
    }
    let pad = ((rect.size() - size) / 2.0).max(egui::Vec2::ZERO);
    let output = egui::ScrollArea::both()
        .id_salt(("viewer-frame", page))
        .auto_shrink([false, false])
        .scroll_source(egui::scroll_area::ScrollSource::ALL)
        .on_drag_cursor(egui::CursorIcon::Grabbing)
        .show(ui, |ui| {
            ui.add_space(pad.y);
            ui.horizontal(|ui| {
                ui.add_space(pad.x);
                ui.add(egui::Image::from_texture(&texture).fit_to_exact_size(size))
            })
            .inner
        });
    if output.inner.hovered()
        && !output.inner.dragged()
        && (size.x > rect.width() || size.y > rect.height())
    {
        ctx.set_cursor_icon(egui::CursorIcon::Grab);
    }
}

/// The layout of the continuous view: the pages' widths and heights from
/// the aspects known so far (the first rendered page's for the rest, A4
/// before any), and where each page starts.
struct Stack {
    page_w: f32,
    heights: Vec<f32>,
    offsets: Vec<f32>,
    total: f32,
}

impl Stack {
    fn new(slot: &Slot, zoom: Zoom, avail: egui::Vec2) -> Self {
        let pages = slot.pages().unwrap_or(1);
        let aspect = |(w, h): (u32, u32)| h as f32 / w.max(1) as f32;
        let aspect_default = (0..pages)
            .find_map(|p| slot.frame_size(p))
            .map_or(DEFAULT_ASPECT, aspect);
        let fit_w = (avail.y / aspect_default).min(avail.x).max(1.0);
        let page_w = match zoom {
            Zoom::Fit => fit_w,
            Zoom::FitWidth | Zoom::Actual => avail.x,
            Zoom::Free(f) => fit_w * f,
        }
        .max(16.0);
        let heights: Vec<f32> = (0..pages)
            .map(|p| page_w * slot.frame_size(p).map_or(aspect_default, aspect))
            .collect();
        let mut offsets = Vec::with_capacity(pages + 1);
        let mut total = 0.0;
        for h in &heights {
            offsets.push(total);
            total += h + PAGE_GAP;
        }
        offsets.push(total);
        Self {
            page_w,
            heights,
            offsets,
            total,
        }
    }

    /// The zoom factor relative to Fit of the stack's width.
    fn factor(&self, slot: &Slot, avail: egui::Vec2) -> f32 {
        let fit = Self::new(slot, Zoom::Fit, avail).page_w;
        if fit > 0.0 { self.page_w / fit } else { 1.0 }
    }
}

/// Every page stacked in one scroll, each rendered as it comes into view;
/// the page under the upper third of the viewport is the current one.
fn draw_continuous(
    ui: &mut egui::Ui,
    state: &mut ViewerState,
    slot: &mut Slot,
    rect: Rect,
    acts: &mut Vec<Act>,
) {
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let dark = ui.visuals().dark_mode;
    let pages = slot.pages().unwrap_or(1);
    let avail = rect.size() - vec2(16.0, 0.0);
    let stack = Stack::new(slot, state.zoom, avail);
    let Stack {
        page_w,
        heights,
        offsets,
        total,
    } = &stack;
    let (page_w, total) = (*page_w, *total);
    if let Some(delta) = wheel_zoom(ui, rect) {
        acts.push(Act::Zoom(Zoom::Free(
            (stack.factor(slot, avail) * delta).clamp(0.05, 32.0),
        )));
    }
    let mut area = egui::ScrollArea::both()
        .id_salt("viewer-pages")
        .auto_shrink([false, false])
        .scroll_source(egui::scroll_area::ScrollSource::ALL);
    if let Some(page) = state.scroll_to_page.take() {
        area = area.vertical_scroll_offset(offsets.get(page).copied().unwrap_or(0.0));
    }
    let content_w = page_w.max(avail.x);
    let mut seen = slot.page;
    let mut frame_size = None;
    area.show_viewport(ui, |ui, viewport| {
        let (full, _) = ui.allocate_exact_size(
            vec2(content_w, total.max(viewport.height())),
            egui::Sense::hover(),
        );
        let probe = viewport.min.y + viewport.height() / 3.0;
        seen = offsets
            .partition_point(|o| *o <= probe)
            .saturating_sub(1)
            .min(pages.saturating_sub(1));
        let x = full.left() + (content_w - page_w) / 2.0;
        for page in 0..pages {
            let y = full.top() + offsets[page];
            let page_rect = Rect::from_min_size(pos2(x, y), vec2(page_w, heights[page]));
            if page_rect.bottom() < full.top() + viewport.min.y - PAGE_GAP
                || page_rect.top() > full.top() + viewport.max.y + PAGE_GAP
            {
                continue;
            }
            let key = RenderKey {
                page,
                width: quantise(page_w, ppp).min(MAX_ACTUAL),
                height: quantise(heights[page], ppp).min(MAX_ACTUAL),
                dark,
            };
            slot.request(&ctx, key, ppp);
            if let Some((texture, (w, h))) = slot.texture(key).or_else(|| slot.any_texture(page)) {
                let size = vec2(page_w, page_w * h as f32 / w.max(1) as f32);
                ui.put(
                    Rect::from_min_size(page_rect.min, size),
                    egui::Image::from_texture(&texture).fit_to_exact_size(size),
                );
                if page == seen {
                    frame_size = Some(size);
                }
            } else {
                ui.painter().rect_stroke(
                    page_rect,
                    0.0,
                    ui.visuals().widgets.noninteractive.bg_stroke,
                    egui::StrokeKind::Inside,
                );
                ui.painter().text(
                    page_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    (page + 1).to_string(),
                    egui::TextStyle::Body.resolve(ui.style()),
                    ui.visuals().weak_text_color(),
                );
            }
        }
    });
    slot.page = seen;
    state.frame_size = frame_size;
}

/// The page strip: one tile per page with its thumbnail (rendered as it
/// comes into view) and number, the current one marked; a click goes
/// there, and the strip follows the current page.
fn draw_strip(
    ui: &mut egui::Ui,
    rect: Rect,
    v: &mut ViewerCx<'_>,
    pages: usize,
    acts: &mut Vec<Act>,
) {
    let state = &mut *v.state;
    let slot = &mut *v.slot;
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let dark = ui.visuals().dark_mode;
    let aspect = (0..pages)
        .find_map(|p| slot.frame_size(p))
        .map_or(DEFAULT_ASPECT, |(w, h)| h as f32 / w.max(1) as f32);
    let tile_w = (rect.width() - 20.0).max(24.0);
    let tile_h = (tile_w * aspect).clamp(24.0, tile_w * 2.0);
    let label_h = ui.text_style_height(&egui::TextStyle::Small) + 2.0;
    let pitch = tile_h + label_h + 10.0;
    let mut strip = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    strip.set_clip_rect(rect.intersect(ui.clip_rect()));
    let current = slot.page;
    let mut area = egui::ScrollArea::vertical()
        .id_salt("viewer-strip")
        .auto_shrink([false, false]);
    if state.strip_centred != Some(current) {
        state.strip_centred = Some(current);
        let offset = (current as f32 * pitch - (rect.height() - pitch) / 2.0).max(0.0);
        area = area.vertical_scroll_offset(offset);
    }
    state.strip_pages.clear();
    state.strip_thumbs.clear();
    let selection = ui.visuals().selection.stroke;
    area.show_viewport(&mut strip, |ui, viewport| {
        let (full, _) = ui.allocate_exact_size(
            vec2(
                ui.available_width(),
                (pages as f32 * pitch).max(viewport.height()),
            ),
            egui::Sense::hover(),
        );
        let first = (viewport.min.y / pitch).floor().max(0.0) as usize;
        let last = ((viewport.max.y / pitch).ceil().max(0.0) as usize + 1).min(pages);
        for page in first..last {
            let top = full.top() + page as f32 * pitch + 4.0;
            let tile = Rect::from_min_size(
                pos2(full.center().x - tile_w / 2.0, top),
                vec2(tile_w, tile_h),
            );
            let key = RenderKey {
                page,
                width: THUMB_PX,
                height: THUMB_PX,
                dark,
            };
            slot.request(&ctx, key, ppp);
            let response = ui.interact(
                tile.expand2(vec2(2.0, label_h + 2.0)),
                ui.id().with(("strip", page)),
                egui::Sense::click(),
            );
            if let Some((texture, (w, h))) = slot.texture(key) {
                let native = vec2(w as f32, h as f32);
                let scale = (tile_w / native.x).min(tile_h / native.y);
                let size = native * scale;
                ui.put(
                    Rect::from_center_size(tile.center(), size),
                    egui::Image::from_texture(&texture).fit_to_exact_size(size),
                );
                state.strip_thumbs.push(page);
            } else {
                ui.painter()
                    .rect_filled(tile, 0.0, ui.visuals().extreme_bg_color);
            }
            let stroke = if page == current {
                egui::Stroke::new(2.0, selection.color)
            } else {
                ui.visuals().widgets.noninteractive.bg_stroke
            };
            ui.painter()
                .rect_stroke(tile, 0.0, stroke, egui::StrokeKind::Outside);
            ui.painter().text(
                pos2(tile.center().x, tile.bottom() + 2.0),
                egui::Align2::CENTER_TOP,
                (page + 1).to_string(),
                egui::TextStyle::Small.resolve(ui.style()),
                if page == current {
                    ui.visuals().strong_text_color()
                } else {
                    ui.visuals().weak_text_color()
                },
            );
            state.strip_pages.push(page);
            if response.clicked() {
                acts.push(Act::GoToPage(page));
            }
        }
    });
}

/// The info panel: every section, the previewer's and the decorators',
/// as a collapsible list of its rows.
fn draw_info(
    ui: &mut egui::Ui,
    cx: &mut PaneCx<'_>,
    v: &mut ViewerCx<'_>,
    entry: Option<&Entry>,
    rect: Rect,
) {
    let sections = inspector::sections_for(cx.app, Some(&*v.slot), entry);
    let mut panel = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    panel.set_clip_rect(rect.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical()
        .id_salt("viewer-info")
        .auto_shrink([false, false])
        .show(&mut panel, |ui| {
            ui.set_width(rect.width());
            if sections.is_empty() {
                ui.add_space(8.0);
                ui.weak("No information");
            }
            for section in &sections {
                let header = format!("{} {}", section.glyph, section.title);
                egui::CollapsingHeader::new(header)
                    .id_salt(("viewer-section", section.id))
                    .default_open(true)
                    .show(ui, |ui| {
                        inspector::section_rows(ui, section, None, f32::INFINITY);
                    });
            }
        });
}

/// `m:ss` or `h:mm:ss`.
fn clock(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The transport: play or pause, a seek bar, elapsed and total time.
fn transport_bar(ui: &mut egui::Ui, bar: Rect, v: &mut ViewerCx<'_>, acts: &mut Vec<Act>) {
    let Some(transport) = v.slot.transport() else {
        return;
    };
    let playing = transport.playing();
    let position = transport.position();
    let duration = transport.duration();
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    row.set_clip_rect(bar.intersect(ui.clip_rect()));
    row.add_space(6.0);
    let glyph = if playing { icons::PAUSE } else { icons::PLAY };
    if icon_button(
        &mut row,
        glyph,
        if playing { "Pause" } else { "Play" },
        None,
    )
    .clicked()
    {
        acts.push(Act::PlayPause);
    }
    let total = duration.map_or(0.0, |d| d.as_secs_f64());
    let mut at = position.as_secs_f64().min(total.max(0.0));
    let width = (bar.width() - 170.0).max(40.0);
    row.spacing_mut().slider_width = width;
    let slider = row.add(
        egui::Slider::new(&mut at, 0.0..=total.max(0.001))
            .show_value(false)
            .trailing_fill(true),
    );
    if slider.changed() {
        acts.push(Act::Seek(Duration::from_secs_f64(at.max(0.0))));
    }
    let total_text = duration.map_or_else(|| "–:––".to_owned(), clock);
    row.add(egui::Label::new(format!("{} / {total_text}", clock(position))).selectable(false));
    if playing {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_every_match_ignoring_case() {
        let layer = TextLayer {
            pages: vec![
                "Hello hello HELLO".to_owned(),
                "nothing".to_owned(),
                "hello".to_owned(),
            ],
        };
        let matches = search(&layer, "hello");
        assert_eq!(matches, [(0, 0..5), (0, 6..11), (0, 12..17), (2, 0..5)]);
        assert!(search(&layer, "").is_empty());
        assert_eq!(search(&layer, "NOTH"), [(1, 0..4)]);
        let mut find = Find {
            query: "hello".to_owned(),
            ..Find::default()
        };
        find.refresh(Some(&layer));
        assert_eq!(find.current, Some(0));
        assert!(find.jump);
        find.step(-1);
        assert_eq!(find.current, Some(3));
        find.step(1);
        assert_eq!(find.current, Some(0));
        assert_eq!(find.ranges_on(0).len(), 3);
        find.current = Some(2);
        assert_eq!(find.current_on(0), Some(2));
        assert_eq!(find.current_on(2), None);
    }

    #[test]
    fn zoom_sizes_follow_the_mode() {
        let content = vec2(400.0, 300.0);
        let big = vec2(1600.0, 600.0);
        let small = vec2(100.0, 50.0);
        assert_eq!(
            display_size(Zoom::Fit, big, content, true),
            vec2(400.0, 150.0)
        );
        assert_eq!(
            display_size(Zoom::Fit, small, content, true),
            small,
            "a raster never grows past 100% to fit"
        );
        assert_eq!(
            display_size(Zoom::Fit, small, content, false),
            vec2(400.0, 200.0),
            "a page always fits"
        );
        assert_eq!(
            display_size(Zoom::FitWidth, small, content, true),
            vec2(400.0, 200.0)
        );
        assert_eq!(display_size(Zoom::Actual, big, content, true), big);
        assert_eq!(
            display_size(Zoom::Free(2.0), big, content, true),
            vec2(800.0, 300.0)
        );
        assert_eq!(request_size(Zoom::Fit, content, 1.0, true), (448, 320));
        assert_eq!(
            request_size(Zoom::Actual, content, 1.0, true),
            (MAX_ACTUAL, MAX_ACTUAL)
        );
        assert_eq!(
            request_size(Zoom::FitWidth, content, 2.0, false),
            (832, MAX_ACTUAL)
        );
        assert_eq!(type_key("Photo.JPG"), "jpg");
        assert_eq!(type_key(".bashrc"), "");
        assert_eq!(type_key("Makefile"), "");
        assert_eq!(clock(Duration::from_secs(65)), "1:05");
        assert_eq!(clock(Duration::from_secs(3725)), "1:02:05");
    }
}
