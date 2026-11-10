//! The Preview tab's content: the content is the point of it, everything
//! else is one row. Top to bottom: a toolbar, the explorer toolbar's
//! height (the entry's file-type icon, the controls for this kind of
//! content and the previewer's own, the info button, close, a hairline and
//! the pane handle), the File info card when it is pinned, the content,
//! and a one-line status bar, the explorer footer's height under its
//! hairline, in the file list's status size. The pane shows
//! no heading: the dock tab's label is enough, and the word "Inspector"
//! appears nowhere in it. It shows the selection of the explorer tab the
//! preview follows ([`App::preview_pane`]), whichever window that is in.
//!
//! One card shows a file's facts ([`crate::info_card`]): the previewer's
//! sections in order, then the decorators' panels, with the entry's
//! *Modified* time after the *Size* row. While the pointer is on the info
//! button, or on the card that opened, the card hangs under the button; a
//! click on the button or on the card's pin pins it above the content, and
//! the pin is remembered per previewer in the settings. With no file
//! selected the pane shows a glyph and two lines, and its toolbar holds
//! only the info button, close and the handle. Line numbers and wrap are
//! settings too; pretty print is the previewer's own control and starts
//! off for every file.
//!
//! Nothing here asks for width: the pane is as wide as the dock makes it,
//! labels truncate, rows clip, and the content is fitted or scrolled.

use std::time::Duration;

use app::{App, Command};
use egui::{Align, Align2, FontId, Rect, pos2, vec2};
use ext::t;
use ext::{Content, ControlValue, Invocation, PreviewControl};
use model::{PaneId, SettingsChange, TextPreview};
use types::{Entry, EntryId, ExtId};

use crate::icons;
use crate::info_card::{self, Card, Row};
use crate::preview::{Around, PreviewAct, Previews, Slot};
use crate::shell::{PaneDrag, Request};
use crate::theme::{self, CARD_RADIUS, TEXT_SECONDARY};

/// The hex previewer, which *Open as hex* shows a file with.
const HEX_PREVIEWER: ExtId = ExtId("preview-hex");
/// The rows the card shows before *Show all n fields*: as a popover, and
/// pinned.
const ROWS_POPOVER: usize = 5;
const ROWS_PINNED: usize = 4;
/// The popover's greatest width.
const POPOVER_WIDTH: f32 = 360.0;
/// The content's margin around the pinned card.
const CARD_MARGIN: f32 = 12.0;
/// The least width the pane lays anything out in; the dock sizes it.
pub const MIN_WIDTH: f32 = 96.0;

/// Where the pane's parts were drawn in the last frame, for tests.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PaneRects {
    /// The toolbar row.
    pub toolbar: Option<Rect>,
    /// The pane handle at the toolbar's right end.
    pub handle: Option<Rect>,
    /// The File info card pinned above the content, when it is pinned.
    pub panel: Option<Rect>,
    /// The File info card hanging under the info button, while it is open.
    pub popover: Option<Rect>,
    /// The content area.
    pub content: Option<Rect>,
    /// The status bar.
    pub status: Option<Rect>,
}

/// One toolbar control as drawn: its tooltip and, for a toggle, its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolbarItem {
    /// The tooltip text.
    pub label: String,
    /// `Some(on)` for a toggle, `None` for a button or the file icon.
    pub on: Option<bool>,
}

/// The info button as drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Info {
    /// Nothing to show: no file, or the card stands centred already.
    #[default]
    Inert,
    /// A card to open, not pinned.
    Unpinned,
    /// The card is pinned above the content.
    Pinned,
}

/// The pane's own state between frames.
#[derive(Debug, Default)]
pub struct PaneState {
    /// What the last frame drew.
    pub rects: PaneRects,
    /// The toolbar's controls of the last frame, left to right.
    pub toolbar: Vec<ToolbarItem>,
    /// The info button of the last frame.
    pub info: Info,
    /// The File info card's rows of the last frame, as label and value,
    /// every section's in order.
    pub fields: Vec<(String, String)>,
    /// The status bar's text of the last frame.
    pub status: String,
    /// The folds of the File info card.
    card: info_card::State,
    /// The popover is open: the pointer is on the info button or on the
    /// card it opened.
    popover: bool,
}

/// The pane handle's wiring: the drag it starts and where the window keeps
/// the drag in progress (SPEC.md *Panes*).
pub struct Handle<'a> {
    /// The Preview tab and its pane, what the model moves.
    pub drag: PaneDrag,
    /// The window's pane drag, which the handle starts.
    pub slot: &'a mut Option<PaneDrag>,
}

/// An info section as the pane shows it, from either source: the
/// previewer's sections and the decorators' panels.
pub(crate) struct Section {
    pub(crate) title: String,
    pub(crate) rows: Vec<(String, String)>,
}

/// The info sections for a document and its entry: the previewer's, then
/// one per decorator with a panel for the entry.
pub(crate) fn sections_for(app: &App, slot: Option<&Slot>, entry: Option<&Entry>) -> Vec<Section> {
    let mut sections: Vec<Section> = slot
        .map(Slot::sections)
        .unwrap_or_default()
        .iter()
        .map(|s| Section {
            title: s.title.get(),
            rows: s.rows.clone(),
        })
        .collect();
    if let Some(entry) = entry {
        for decorator in app.registry().decorators() {
            if let Some(panel) = decorator.panel(entry) {
                sections.push(Section {
                    title: panel.title,
                    rows: panel.rows,
                });
            }
        }
    }
    sections
}

/// The File info card's rows (SPEC.md *File info card*, the fields): the
/// sections' rows in order, each section after the first beginning under
/// its title, with the entry's *Modified* time — which the pane supplies,
/// as `YYYY-MM-DD HH:MM` — after the first section's *Size* row, or at
/// that section's end when it has none.
pub(crate) fn card_rows(sections: &[Section], entry: Option<&Entry>) -> Vec<Row> {
    let modified = entry
        .and_then(|entry| entry.modified)
        .map(|time| Row::new(t!("preview-modified"), crate::format::stamp(time)));
    let mut rows = Vec::new();
    if sections.is_empty() {
        rows.extend(modified);
        return rows;
    }
    let size = t!("preview-size");
    let mut modified = modified;
    for (index, section) in sections.iter().enumerate() {
        for (at, (label, value)) in section.rows.iter().enumerate() {
            let mut row = Row::new(label, value);
            if index > 0 && at == 0 {
                row.section = Some(section.title.clone());
            }
            rows.push(row);
            // Previewers write the label in English.
            if index == 0 && (label == "Size" || *label == size) {
                rows.extend(modified.take());
            }
        }
        if index == 0 {
            rows.extend(modified.take());
        }
    }
    rows
}

/// What a toolbar control asked for.
enum Act {
    Setting(SettingsChange),
    Page(isize),
    ActualSize(bool),
    Control(ext::ControlId, ControlValue),
    /// The transport: play or pause.
    PlayPause,
    /// The transport: jump to a position.
    Seek(Duration),
    /// The transport: set the volume, 0.0 to 1.0.
    Volume(f32),
    /// Copy the whole file's text to the clipboard.
    Copy,
    Close,
}

/// The transport's facts for the toolbar, read from the document.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TransportView {
    playing: bool,
    position: Duration,
    duration: Option<Duration>,
    volume: f32,
}

/// Everything the pane draws from, gathered before drawing so the slot is
/// not borrowed while the widgets run.
// The kinds of content are independent flags, not states of one machine.
#[allow(clippy::struct_excessive_bools)]
struct View<'a> {
    /// The pane whose selection is shown, when an explorer tab is followed.
    pane: Option<PaneId>,
    target: Option<EntryId>,
    entry: Option<&'a Entry>,
    selected: usize,
    /// The previewer that shows the file, by its extension's id.
    previewer: Option<ExtId>,
    is_text: bool,
    is_raster: bool,
    actual_size: bool,
    pages: Option<usize>,
    shown_page: usize,
    text_settings: TextPreview,
    controls: Vec<PreviewControl>,
    /// The type and the size, as the card's head has them.
    line: String,
    /// The File info card's rows.
    rows: Vec<Row>,
    /// Whether the card is pinned for this previewer.
    pinned: bool,
    status: Vec<String>,
    /// Whether the preview has a text layer to copy: text, code, Markdown,
    /// HTML, CSV; never a file list or a paged document.
    copy: bool,
    /// The media transport, when the document is media.
    transport: Option<TransportView>,
}

impl View<'_> {
    /// The File info card for the file shown, with its pin button in the
    /// state `pin` and `first` rows before *Show all n fields*; `None`
    /// while there is no document to describe.
    fn card(&self, pin: Option<bool>, first: usize) -> Option<Card<'_>> {
        let entry = self.entry?;
        self.previewer?;
        Some(Card {
            glyph: icons::for_entry(entry),
            name: &entry.name,
            line: &self.line,
            rows: &self.rows,
            first: Some(first),
            pin,
            ..Card::default()
        })
    }

    /// Pins or unpins the card for the previewer that shows the file.
    fn pin(&self, pinned: bool) -> Act {
        Act::Setting(SettingsChange::PreviewPin {
            previewer: self
                .previewer
                .map_or_else(String::new, |id| id.0.to_owned()),
            pinned,
        })
    }
}

/// Draws the pane for the selection of the explorer tab the preview
/// follows. Returns `true` when the close button was pressed.
pub fn show(
    ui: &mut egui::Ui,
    app: &App,
    previews: &mut Previews,
    state: &mut PaneState,
    requests: &mut Vec<Request>,
    handle: &mut Handle<'_>,
) -> bool {
    let width = ui.available_width().max(MIN_WIDTH);
    ui.set_max_width(width);
    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
    ui.spacing_mut().item_spacing.y = 0.0;

    let view = collect(ui, app, previews);

    // Toolbar: the explorer toolbar's height (SPEC.md *Preview*).
    let toolbar_h = theme::metrics(ui.ctx()).toolbar_height();
    let (row, _) = ui.allocate_exact_size(vec2(width, toolbar_h), egui::Sense::hover());
    state.rects.toolbar = Some(row);
    state.toolbar.clear();
    state.fields = view
        .rows
        .iter()
        .map(|row| (row.label.clone(), row.value.clone()))
        .collect();
    let mut acts = Vec::new();
    let info = toolbar(ui, row, &view, state, &mut acts, handle);
    separator(ui, width);

    // Status bar at the foot, the explorer footer's height with its
    // hairline (SPEC.md *Footer*); the content gets what is left.
    let status_h = theme::metrics(ui.ctx()).footer_height();
    let content_h = (ui.available_height() - status_h).max(0.0);
    let (content, _) = ui.allocate_exact_size(vec2(width, content_h), egui::Sense::hover());
    state.rects.content = Some(content);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    inner.set_clip_rect(content.intersect(ui.clip_rect()));
    inner.spacing_mut().item_spacing.y = 0.0;

    // The pinned card: at the top of the content, inside its margin, and
    // the content below it.
    state.rects.panel = None;
    if view.pinned
        && let Some(card) = view.card(Some(true), ROWS_PINNED)
    {
        inner.add_space(10.0);
        let mut pinned = inner.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(
            pos2(content.left() + CARD_MARGIN, inner.cursor().top()),
            pos2(content.right() - CARD_MARGIN, content.bottom()),
        )));
        let outcome = info_card::show(&mut pinned, &card, &mut state.card, content.height() * 0.5);
        if outcome.pin {
            acts.push(view.pin(false));
        }
        state.rects.panel = outcome.rect;
        let used = outcome.rect.map_or(0.0, |rect| rect.height());
        inner.add_space(used + 10.0);
    }
    let body = inner.available_rect_before_wrap();
    let mut body_ui = inner.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    body_ui.set_clip_rect(body.intersect(ui.clip_rect()));
    match view.pane.filter(|_| view.target.is_some()) {
        Some(pane) => {
            let home = app.platform().home_dir();
            let plugins = app.platform().config_dir().join("plugins");
            let around = Around {
                home: &home,
                plugins: &plugins,
                hex: app
                    .registry()
                    .previewers()
                    .iter()
                    .any(|p| p.id() == HEX_PREVIEWER),
                still: app.platform().reduce_motion(),
                columns: &app.model().settings.preview.columns,
            };
            let act = previews.content(
                &mut body_ui,
                pane,
                view.target,
                view.entry,
                view.text_settings,
                &around,
            );
            // Either button of the size-limit card clears the card at
            // once: the download is what the pane shows from here.
            if matches!(
                act,
                Some(PreviewAct::PreviewAnyway | PreviewAct::PreviewAlways)
            ) {
                previews.fetching(pane);
            }
            match act {
                Some(PreviewAct::Columns(widths)) => {
                    if let Some(previewer) = view.previewer {
                        acts.push(Act::Setting(SettingsChange::PreviewColumns {
                            previewer: previewer.0.to_owned(),
                            widths,
                        }));
                    }
                }
                Some(act) => apply_preview_act(act, pane, app, ui.ctx(), requests),
                None => {}
            }
        }
        None => nothing_selected(&mut body_ui),
    }
    status_bar(ui, width, status_h, &view, state);
    popover(ui, &view, state, info, content, &mut acts);

    apply_acts(acts, view.pane, previews, requests)
}

/// What a button in the content asked for.
fn apply_preview_act(
    act: PreviewAct,
    pane: PaneId,
    app: &App,
    ctx: &egui::Context,
    requests: &mut Vec<Request>,
) {
    match act {
        PreviewAct::LoadMore => requests.push(Request::Command(Command::PreviewMore(pane))),
        PreviewAct::PreviewAnyway => {
            requests.push(Request::Command(Command::PreviewAnyway(pane)));
        }
        PreviewAct::PreviewAlways => {
            requests.push(Request::Command(Command::PreviewAlways(pane)));
        }
        PreviewAct::Retry => requests.push(Request::Command(Command::PreviewRetry(pane))),
        PreviewAct::Browse { setting, filters } => requests.push(Request::BrowseLibrary {
            pane,
            setting,
            filters,
        }),
        PreviewAct::OpenAsHex => requests.push(Request::Command(Command::PreviewWith {
            pane,
            previewer: HEX_PREVIEWER,
        })),
        PreviewAct::OpenWith => {
            if let Some(ctx) = app.action_ctx(pane, Invocation::ContextMenu) {
                requests.push(Request::RunAction {
                    action: ext::ActionId("core.open_with"),
                    ctx,
                });
            }
        }
        PreviewAct::Download(address) => ctx.open_url(egui::OpenUrl::new_tab(address)),
        // The pane itself stores a table's widths.
        PreviewAct::Columns(_) => {}
        PreviewAct::OpenFolder(folder) => {
            // The folder a library goes into is made when it is first shown.
            let shown = std::fs::create_dir_all(&folder)
                .map_err(types::Error::from_io)
                .and_then(|()| app.platform().open(&folder));
            if let Err(error) = shown {
                requests.push(Request::Toast(ext::Level::Error, error.to_string()));
            }
        }
    }
}

/// The empty state (SPEC.md *Nothing selected*): a 72 px `bg/subtle`
/// circle holding the file glyph, *No file selected* and, under it,
/// *Select a file to preview it here*, centred in the content.
fn nothing_selected(ui: &mut egui::Ui) {
    let scheme = theme::scheme(ui);
    let rect = ui.available_rect_before_wrap();
    let centre = rect.center();
    let circle = centre - vec2(0.0, 36.0);
    ui.painter().circle_filled(circle, 36.0, scheme.bg_subtle);
    ui.painter().text(
        circle,
        Align2::CENTER_CENTER,
        icons::FILE,
        FontId::proportional(32.0),
        scheme.icon_default,
    );
    let line = |text: String, y: f32, size: f32, colour: egui::Color32| {
        let galley = ui
            .painter()
            .layout_no_wrap(text.clone(), FontId::proportional(size), colour);
        let at = Rect::from_center_size(pos2(centre.x, y), galley.size());
        ui.painter().galley(at.min, galley, colour);
        ui.interact(
            at,
            ui.id().with(("empty", y.to_bits())),
            egui::Sense::hover(),
        )
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text.clone()));
    };
    line(
        t!("preview-no-file-selected"),
        centre.y + 18.0,
        14.0,
        scheme.text_secondary,
    );
    line(
        t!("inspector-select-a-file-to-preview"),
        centre.y + 43.0,
        TEXT_SECONDARY,
        scheme.text_placeholder,
    );
}

/// The card hanging under the info button (SPEC.md *File info card*, the
/// popover): open while the pointer is on the button or on the card that
/// opened, its right edge on the button's, at most 360 px wide, with a
/// soft shadow and a pin in its head. Leaving both closes it.
fn popover(
    ui: &mut egui::Ui,
    view: &View<'_>,
    state: &mut PaneState,
    info: Option<egui::Response>,
    content: Rect,
    acts: &mut Vec<Act>,
) {
    let last = state.rects.popover.take();
    let Some(info) = info.filter(|_| !view.pinned) else {
        state.popover = false;
        return;
    };
    let Some(card) = view.card(Some(false), ROWS_POPOVER) else {
        state.popover = false;
        return;
    };
    let pointer = ui.ctx().pointer_hover_pos();
    // The way from the button down to the card belongs to both.
    let over_card = state.popover
        && last.is_some_and(|rect| {
            let reach = rect.union(info.rect).expand(4.0);
            pointer.is_some_and(|at| reach.contains(at))
        });
    state.popover = info.hovered() || over_card;
    if !state.popover {
        return;
    }
    let scheme = theme::scheme(ui);
    let width = POPOVER_WIDTH.min(content.width() - 16.0).max(MIN_WIDTH);
    let right = info.rect.right().min(content.right() - 8.0);
    let at = pos2(
        (right - width).max(content.left() + 8.0),
        content.top() + 6.0,
    );
    let area = egui::Area::new(ui.id().with("file-info-popover"))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .constrain(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .corner_radius(CARD_RADIUS)
                .shadow(egui::epaint::Shadow {
                    offset: [0, 6],
                    blur: 20,
                    spread: 0,
                    color: scheme.shadow(),
                })
                .show(ui, |ui| {
                    ui.set_width(width);
                    info_card::show(ui, &card, &mut state.card, content.height() * 0.6)
                })
                .inner
        });
    if area.inner.pin {
        acts.push(view.pin(true));
    }
    state.rects.popover = area.inner.rect;
}

/// Applies what the toolbar asked for; returns whether the close button
/// was pressed.
fn apply_acts(
    acts: Vec<Act>,
    pane: Option<PaneId>,
    previews: &mut Previews,
    requests: &mut Vec<Request>,
) -> bool {
    let mut close = false;
    for act in acts {
        match (act, pane) {
            (Act::Setting(change), _) => {
                requests.push(Request::Command(Command::Settings(change)));
            }
            (Act::Page(delta), Some(pane)) => {
                let slot = previews.get_mut(pane);
                let last = slot.pages().unwrap_or(1).saturating_sub(1);
                slot.page = slot.page.saturating_add_signed(delta).min(last);
            }
            (Act::ActualSize(on), Some(pane)) => previews.get_mut(pane).actual_size = on,
            (Act::Control(id, value), Some(pane)) => {
                previews.get_mut(pane).set_control(id, value);
            }
            (Act::PlayPause, Some(pane)) => {
                if let Some(transport) = previews.get_mut(pane).transport() {
                    if transport.playing() {
                        transport.pause();
                    } else {
                        transport.play();
                    }
                }
            }
            (Act::Seek(to), Some(pane)) => {
                if let Some(transport) = previews.get_mut(pane).transport() {
                    transport.seek(to);
                }
            }
            (Act::Volume(volume), Some(pane)) => {
                if let Some(transport) = previews.get_mut(pane).transport() {
                    transport.set_volume(volume);
                }
            }
            (Act::Copy, Some(pane)) => {
                requests.push(Request::Command(Command::PreviewCopy(pane)));
            }
            (Act::Close, _) => close = true,
            (
                Act::Page(_)
                | Act::ActualSize(_)
                | Act::Control(..)
                | Act::PlayPause
                | Act::Seek(_)
                | Act::Volume(_)
                | Act::Copy,
                None,
            ) => {}
        }
    }
    close
}

/// Plays or pauses the media the pane shows, if any: Space while the
/// preview pane has the focus.
pub fn play_pause(previews: &mut Previews, pane: Option<PaneId>) {
    if let Some(transport) = pane.and_then(|pane| previews.get_mut(pane).transport()) {
        if transport.playing() {
            transport.pause();
        } else {
            transport.play();
        }
    }
}

/// Gathers what the pane shows: the followed explorer tab's selection, the
/// slot's facts and the decorators' sections.
fn collect<'a>(ui: &egui::Ui, app: &'a App, previews: &mut Previews) -> View<'a> {
    let pane = app.preview_pane();
    let target = app
        .preview_target()
        .filter(|(p, _)| Some(*p) == pane)
        .map(|(_, id)| id);
    let tab = app
        .model()
        .preview_follows()
        .and_then(|id| app.model().tab(id));
    let selected: Vec<&Entry> = tab.map(|t| t.selected().collect()).unwrap_or_default();
    let entry = match (target, selected.as_slice()) {
        (Some(id), [one]) if one.id == id => Some(*one),
        _ => None,
    };
    let settings = &app.model().settings;
    let slot: Option<&Slot> = pane.map(|pane| previews.sync(ui.ctx(), pane, target));
    // The card belongs to a document that is shown: a file no previewer
    // took, or one that failed, has its card centred in the content.
    let previewer = slot
        .filter(|s| s.has_doc() && !matches!(s.content(), Some(Content::MissingLibrary(_))))
        .and_then(Slot::previewer);
    let rows = if previewer.is_some() {
        card_rows(&sections_for(app, slot, entry), entry)
    } else {
        Vec::new()
    };
    let pinned = previewer.is_some_and(|id| settings.preview.pinned.contains_key(id.0));
    // The controls belong to a document that is shown; with the card
    // centred in the content there is nothing for them to act on.
    let shown = slot.filter(|_| previewer.is_some());
    let copy = shown.is_some_and(|s| {
        s.text_layer().is_some()
            && s.pages().is_none()
            && !matches!(s.content(), Some(Content::Listing(_)))
    });
    let mut view = View {
        pane,
        target,
        entry,
        selected: selected.len(),
        previewer,
        is_text: shown.is_some_and(Slot::is_text),
        is_raster: shown.is_some_and(Slot::is_raster),
        actual_size: shown.is_some_and(|s| s.actual_size),
        pages: shown.and_then(Slot::pages),
        shown_page: shown.map_or(0, |s| s.page),
        text_settings: settings.preview.text,
        controls: shown.map(|s| s.controls().to_vec()).unwrap_or_default(),
        line: entry.map_or_else(String::new, |e| crate::preview::type_and_size(e, false)),
        rows,
        pinned,
        status: slot.map(|s| s.status().to_vec()).unwrap_or_default(),
        copy,
        transport: None,
    };
    view.transport = pane
        .filter(|_| target.is_some())
        .and_then(|pane| previews.get_mut(pane).transport())
        .map(|t| TransportView {
            playing: t.playing(),
            position: t.position(),
            duration: t.duration(),
            volume: t.volume(),
        });
    view
}

/// `m:ss`, or `h:mm:ss` from an hour on.
fn clock(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The transport in the toolbar's middle group: play or pause, a seek bar
/// and the elapsed and total time (SPEC.md, Preview, *Toolbar*: "Media:
/// the transport").
fn transport_controls(
    ui: &mut egui::Ui,
    transport: TransportView,
    state: &mut PaneState,
    acts: &mut Vec<Act>,
) {
    let (glyph, label) = if transport.playing {
        (icons::PAUSE, t!("preview-pause"))
    } else {
        (icons::PLAY, t!("preview-play"))
    };
    if button(ui, state, glyph, &label, true) {
        acts.push(Act::PlayPause);
    }
    let total = transport.duration.map_or(0.0, |d| d.as_secs_f64());
    let mut at = transport.position.as_secs_f64().min(total.max(0.0));
    let width = (ui.available_width() - 160.0).clamp(40.0, 240.0);
    ui.spacing_mut().slider_width = width;
    let slider = ui.add(
        egui::Slider::new(&mut at, 0.0..=total.max(0.001))
            .show_value(false)
            .trailing_fill(true),
    );
    if slider.changed() {
        acts.push(Act::Seek(Duration::from_secs_f64(at.max(0.0))));
    }
    let total_text = transport.duration.map_or_else(|| "–:––".to_owned(), clock); // i18n: ok
    ui.add(
        egui::Label::new(
            egui::RichText::new(format!("{} / {total_text}", clock(transport.position))).small(),
        )
        .selectable(false),
    );
    // The volume, a short slider behind the speaker glyph.
    ui.add(egui::Label::new(egui::RichText::new(icons::VOLUME).size(15.0)).selectable(false))
        .on_hover_text(t!("preview-volume"));
    let mut volume = transport.volume.clamp(0.0, 1.0);
    ui.spacing_mut().slider_width = 56.0;
    if ui
        .add(egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false))
        .on_hover_text(t!("preview-volume"))
        .changed()
    {
        acts.push(Act::Volume(volume));
    }
    if transport.playing {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
}

/// One line of text, the file list's status size: the previewer's facts
/// joined with a dot, or what is known about the selection without one.
/// Never wraps; truncates.
/// The footer: the explorer footer's height, its hairline on top, the
/// previewer's facts in 12 px `text/secondary` with `space/toolbar-x` of
/// padding, truncated when the pane is narrow.
fn status_bar(ui: &mut egui::Ui, width: f32, height: f32, view: &View<'_>, state: &mut PaneState) {
    let (status, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    state.rects.status = Some(status);
    let scheme = crate::theme::scheme(ui);
    let metrics = crate::theme::metrics(ui.ctx());
    ui.painter()
        .hline(status.x_range(), status.top() + 0.5, scheme.hairline());
    let text = if view.status.is_empty() {
        match (view.entry, view.selected) {
            (Some(entry), _) => model::type_of(entry).clone(),
            // Nothing selected: the footer is empty.
            (None, 0) => String::new(),
            (None, n) => format!("{n} items selected"),
        }
    } else {
        view.status.join(" · ")
    };
    state.status.clone_from(&text);
    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(status.shrink2(vec2(metrics.space_toolbar_x, 0.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    bar.set_clip_rect(status.intersect(ui.clip_rect()));
    bar.add(
        egui::Label::new(
            egui::RichText::new(text)
                .size(crate::theme::TEXT_SECONDARY)
                .color(scheme.text_secondary),
        )
        .selectable(false)
        .truncate(),
    );
}

fn separator(ui: &mut egui::Ui, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 1.0), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}

/// One row with `space/toolbar-x` padding: the file icon, the controls,
/// then from the right the pane handle after a hairline, the close button
/// and the info button. Icons only, labels as tooltips. Returns the info
/// button's response while it has a card to open.
fn toolbar(
    ui: &mut egui::Ui,
    row: Rect,
    view: &View<'_>,
    state: &mut PaneState,
    acts: &mut Vec<Act>,
    handle: &mut Handle<'_>,
) -> Option<egui::Response> {
    let inner = row.shrink2(vec2(theme::metrics(ui.ctx()).space_toolbar_x, 0.0));
    // Right: the handle and its hairline, close, then the info button, so
    // they keep their place.
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    right.set_clip_rect(row.intersect(ui.clip_rect()));
    let grip = crate::widgets::pane_handle(&mut right);
    state.rects.handle = Some(grip.rect);
    if grip.drag_started() {
        *handle.slot = Some(handle.drag);
    }
    crate::widgets::divider(&mut right, 16.0);
    if icon_button(&mut right, icons::CLOSE, &t!("close"), None).clicked() {
        acts.push(Act::Close);
    }
    let info = info_button(&mut right, view, state, acts);
    let right_edge = right.min_rect().left();

    // Left: the file icon, then the controls.
    let left_rect = Rect::from_min_max(
        inner.min,
        pos2(right_edge.max(inner.left()), inner.bottom()),
    );
    let mut left = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    left.set_clip_rect(left_rect.intersect(ui.clip_rect()));
    if let Some(entry) = view.entry {
        left.add(
            egui::Label::new(egui::RichText::new(icons::for_entry(entry)).size(16.0))
                .selectable(false),
        );
        state.toolbar.push(ToolbarItem {
            label: "File".to_owned(),
            on: None,
        });
        left.add_space(6.0);
    }
    standard_controls(&mut left, view, state, acts);
    previewer_controls(&mut left, view, state, acts);
    if let Some(transport) = view.transport {
        transport_controls(&mut left, transport, state, acts);
    }
    // Copy closes the middle group of every text-based preview: the whole
    // file's text, the raw source of a rendered one, never the file.
    if view.copy && button(&mut left, state, icons::COPY, &t!("inspector-copy"), true) {
        acts.push(Act::Copy);
    }
    info
}

/// The info button: one for every file. With a document shown it opens
/// the File info card under itself while the pointer is on it, and a click
/// pins or unpins the card; pinned, it carries the tint of an active
/// toggle. With nothing to describe it is there and inert.
fn info_button(
    ui: &mut egui::Ui,
    view: &View<'_>,
    state: &mut PaneState,
    acts: &mut Vec<Act>,
) -> Option<egui::Response> {
    let name = t!("preview-file-info");
    let (rect, _) = ui.allocate_exact_size(vec2(26.0, 26.0), egui::Sense::hover());
    if view.card(None, 0).is_none() {
        state.info = Info::Inert;
        let scheme = theme::scheme(ui);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            icons::INFO,
            FontId::proportional(theme::ICON),
            scheme.icon_default,
        );
        ui.interact(rect, ui.id().with("file-info-inert"), egui::Sense::hover())
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, false, name.clone())
            });
        return None;
    }
    state.info = if view.pinned {
        Info::Pinned
    } else {
        Info::Unpinned
    };
    // No tooltip of its own: the card is what the pointer brings up.
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    let response = ui.interact(rect, ui.id().with("file-info"), egui::Sense::click());
    let open = state.popover;
    if view.pinned {
        ui.painter()
            .rect_filled(rect, radius, scheme.bg_selected_soft);
    } else if response.hovered() || open {
        ui.painter().rect_filled(rect, radius, scheme.bg_subtle);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icons::INFO,
        FontId::proportional(theme::ICON),
        if view.pinned {
            scheme.accent_focus
        } else {
            scheme.icon_default
        },
    );
    let pinned = view.pinned;
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, pinned, name.clone())
    });
    if response.clicked() {
        acts.push(view.pin(!view.pinned));
    }
    Some(response)
}

/// The controls the kind of document supplies: line numbers and wrap for
/// text, the pager for pages, Fit and 1:1 for a raster.
fn standard_controls(
    ui: &mut egui::Ui,
    view: &View<'_>,
    state: &mut PaneState,
    acts: &mut Vec<Act>,
) {
    if view.is_text {
        let numbers = view.text_settings.line_numbers;
        if toggle(ui, state, icons::LINE_NUMBERS, "Line numbers", numbers) {
            acts.push(Act::Setting(SettingsChange::PreviewLineNumbers(!numbers)));
        }
        let wrap = view.text_settings.wrap;
        if toggle(ui, state, icons::WRAP, "Word wrap", wrap) {
            acts.push(Act::Setting(SettingsChange::PreviewWrap(!wrap)));
        }
    }
    if let Some(pages) = view.pages {
        if button(ui, state, icons::BACK, "Previous page", view.shown_page > 0) {
            acts.push(Act::Page(-1));
        }
        ui.add(egui::Label::new(format!("{} / {pages}", view.shown_page + 1)).selectable(false));
        if button(
            ui,
            state,
            icons::FORWARD,
            "Next page",
            view.shown_page + 1 < pages,
        ) {
            acts.push(Act::Page(1));
        }
    }
    if view.is_raster {
        if toggle(ui, state, icons::FIT, "Fit", !view.actual_size) {
            acts.push(Act::ActualSize(false));
        }
        if toggle(ui, state, icons::ACTUAL_SIZE, "1:1", view.actual_size) {
            acts.push(Act::ActualSize(true));
        }
    }
}

/// The previewer's own controls, after the standard ones.
fn previewer_controls(
    ui: &mut egui::Ui,
    view: &View<'_>,
    state: &mut PaneState,
    acts: &mut Vec<Act>,
) {
    for control in &view.controls {
        match control {
            PreviewControl::Toggle {
                id,
                icon,
                label,
                on,
            } => {
                if toggle(ui, state, icons::for_icon(*icon), label, *on) {
                    acts.push(Act::Control(*id, ControlValue::Bool(!*on)));
                }
            }
            PreviewControl::Button {
                id,
                icon,
                label,
                enabled,
            } => {
                if button(ui, state, icons::for_icon(*icon), label, *enabled) {
                    acts.push(Act::Control(*id, ControlValue::Pressed));
                }
            }
            // Pointer gestures are the frame's, not the toolbar's.
            PreviewControl::Pointer { .. } => {}
        }
    }
}

/// A toggle in the toolbar: records itself for tests, returns whether it
/// was clicked.
fn toggle(ui: &mut egui::Ui, state: &mut PaneState, glyph: &str, label: &str, on: bool) -> bool {
    state.toolbar.push(ToolbarItem {
        label: label.to_owned(),
        on: Some(on),
    });
    icon_button(ui, glyph, label, Some(on)).clicked()
}

/// A one-shot button in the toolbar, greyed out when not `enabled`.
fn button(
    ui: &mut egui::Ui,
    state: &mut PaneState,
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
