//! The preview pane: the content is the point of it, everything else is one
//! row. Top to bottom: a toolbar (the entry's file-type icon, the controls
//! for this kind of content and the previewer's own, info badges, close), the
//! pinned info section when one is pinned, the content, and a one-line
//! status bar in the file list's status size. The pane shows no heading: the word "Inspector" appears
//! nowhere in it.
//!
//! Info badges come from the previewer's sections and from the decorators'
//! panels. Hovering one shows its rows as an overlay; clicking pins it as a
//! panel above the content, one at a time, remembered per content kind in
//! the settings. Line numbers and wrap are settings too; pretty print is the
//! previewer's own control and starts off for every file.
//!
//! Nothing here asks for width: the pane is as wide as the dock makes it,
//! labels truncate, rows clip, and the content is fitted or scrolled.

use app::{App, Command};
use egui::{Align, Rect, pos2, vec2};
use ext::{ControlValue, PreviewControl};
use model::{PaneId, SettingsChange, TextPreview};
use types::{Entry, EntryId};

use crate::icons;
use crate::preview::Previews;
use crate::shell::Request;

/// The toolbar's height: a tab's.
pub const TOOLBAR_HEIGHT: f32 = 30.0;
/// Rows beyond this get a search box.
const SEARCHABLE_FROM: usize = 20;
/// A pinned panel takes at most this share of the pane's height.
const PANEL_SHARE: f32 = 0.45;
/// The least width the pane lays anything out in; what the pane asks the
/// dock for at most.
pub const MIN_WIDTH: f32 = 96.0;

/// Where the pane's parts were drawn in the last frame, for tests.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PaneRects {
    /// The toolbar row.
    pub toolbar: Option<Rect>,
    /// The pinned section's panel, when one is pinned.
    pub panel: Option<Rect>,
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

/// One info badge as drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadgeItem {
    /// The section id.
    pub id: String,
    /// The section title.
    pub title: String,
    /// Whether its panel is pinned.
    pub pinned: bool,
}

/// The pane's own state between frames.
#[derive(Debug, Default)]
pub struct PaneState {
    /// The pinned panel's search text.
    pub filter: String,
    /// What the last frame drew.
    pub rects: PaneRects,
    /// The toolbar's controls of the last frame, left to right.
    pub toolbar: Vec<ToolbarItem>,
    /// The badges of the last frame, left to right.
    pub badges: Vec<BadgeItem>,
    /// The status bar's text of the last frame.
    pub status: String,
}

/// An info section as the pane shows it, from either source.
struct Section {
    id: &'static str,
    title: String,
    glyph: &'static str,
    rows: Vec<(String, String)>,
}

/// What a toolbar control asked for.
enum Act {
    Setting(SettingsChange),
    Page(isize),
    ActualSize(bool),
    Control(ext::ControlId, ControlValue),
    Close,
}

/// Everything the pane draws from, gathered before drawing so the slot is
/// not borrowed while the widgets run.
struct View<'a> {
    target: Option<EntryId>,
    entry: Option<&'a Entry>,
    selected: usize,
    entries: Option<usize>,
    kind: &'static str,
    is_text: bool,
    is_raster: bool,
    actual_size: bool,
    pages: Option<usize>,
    shown_page: usize,
    text_settings: TextPreview,
    controls: Vec<PreviewControl>,
    sections: Vec<Section>,
    pinned: Option<usize>,
    status: Vec<String>,
}

/// Draws the pane for `pane`'s selection. Returns `true` when the close
/// button was pressed.
pub fn show(
    ui: &mut egui::Ui,
    app: &App,
    pane: PaneId,
    previews: &mut Previews,
    state: &mut PaneState,
    requests: &mut Vec<Request>,
) -> bool {
    let width = ui.available_width().max(MIN_WIDTH);
    ui.set_max_width(width);
    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
    ui.spacing_mut().item_spacing.y = 0.0;

    let view = collect(ui, app, pane, previews);

    // Toolbar.
    let (row, _) = ui.allocate_exact_size(vec2(width, TOOLBAR_HEIGHT), egui::Sense::hover());
    state.rects.toolbar = Some(row);
    state.toolbar.clear();
    state.badges.clear();
    let mut acts = Vec::new();
    toolbar(ui, row, &view, state, &mut acts);
    separator(ui, width);

    // Pinned panel.
    state.rects.panel = None;
    if let Some(section) = view.pinned.and_then(|i| view.sections.get(i)) {
        let max_h = (ui.available_height() * PANEL_SHARE).max(60.0);
        let before = ui.cursor().top();
        ui.scope(|ui| {
            ui.set_max_height(max_h);
            section_rows(ui, section, Some(&mut state.filter), max_h);
        });
        let after = ui.cursor().top();
        state.rects.panel = Some(Rect::from_min_max(
            pos2(row.left(), before),
            pos2(row.right(), after),
        ));
        separator(ui, width);
    }

    // Status bar at the foot, in the body size the file list's status line
    // uses; the content gets what is left.
    let status_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    let content_h = (ui.available_height() - status_h - 1.0).max(0.0);
    let (content, _) = ui.allocate_exact_size(vec2(width, content_h), egui::Sense::hover());
    state.rects.content = Some(content);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    inner.set_clip_rect(content.intersect(ui.clip_rect()));
    previews.content(&mut inner, pane, view.target, view.text_settings);
    separator(ui, width);
    status_bar(ui, width, status_h, &view, state);

    // What the toolbar asked for.
    let mut close = false;
    for act in acts {
        match act {
            Act::Setting(change) => requests.push(Request::Command(Command::Settings(change))),
            Act::Page(delta) => {
                let slot = previews.get_mut(pane);
                let last = slot.pages().unwrap_or(1).saturating_sub(1);
                slot.page = slot.page.saturating_add_signed(delta).min(last);
            }
            Act::ActualSize(on) => previews.get_mut(pane).actual_size = on,
            Act::Control(id, value) => previews.get_mut(pane).set_control(id, value),
            Act::Close => close = true,
        }
    }
    close
}

/// Gathers what the pane shows: the slot's facts, the selection and the
/// decorators' sections.
fn collect<'a>(ui: &egui::Ui, app: &'a App, pane: PaneId, previews: &mut Previews) -> View<'a> {
    let target = app
        .preview_target()
        .filter(|(p, _)| *p == pane)
        .map(|(_, id)| id);
    let tab = app.model().active_tab(pane).map(|(_, tab)| tab);
    let selected: Vec<&Entry> = tab.map(|t| t.selected().collect()).unwrap_or_default();
    let entry = match (target, selected.as_slice()) {
        (Some(id), [one]) if one.id == id => Some(*one),
        _ => None,
    };
    let settings = &app.model().settings;
    let slot = previews.sync(ui.ctx(), pane, target);
    let kind = slot.kind().unwrap_or("other");
    let mut sections: Vec<Section> = slot
        .sections()
        .iter()
        .map(|s| Section {
            id: s.id,
            title: s.title.to_owned(),
            glyph: icons::for_icon(s.icon),
            rows: s.rows.clone(),
        })
        .collect();
    if let Some(entry) = entry {
        for decorator in app.registry().decorators() {
            if let Some(panel) = decorator.panel(entry) {
                sections.push(Section {
                    id: decorator.id().0,
                    title: panel.title,
                    glyph: icons::for_decorator(decorator.id().0),
                    rows: panel.rows,
                });
            }
        }
    }
    let pinned = settings
        .preview
        .pinned
        .get(kind)
        .and_then(|id| sections.iter().position(|s| s.id == id));
    View {
        target,
        entry,
        selected: selected.len(),
        entries: tab.map(|t| t.listing.len()),
        kind,
        is_text: slot.is_text(),
        is_raster: slot.is_raster(),
        actual_size: slot.actual_size,
        pages: slot.pages(),
        shown_page: slot.page,
        text_settings: settings.preview.text,
        controls: slot.controls().to_vec(),
        sections,
        pinned,
        status: slot.status().to_vec(),
    }
}

/// One line of text, the file list's status size: the previewer's facts
/// joined with a dot, or what is known about the selection without one.
/// Never wraps; truncates.
fn status_bar(ui: &mut egui::Ui, width: f32, height: f32, view: &View<'_>, state: &mut PaneState) {
    let (status, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    state.rects.status = Some(status);
    let text = if view.status.is_empty() {
        match (view.entry, view.selected) {
            (Some(entry), _) => model::type_of(entry).clone(),
            (None, 0) => view
                .entries
                .map_or_else(String::new, |n| format!("{n} entries")),
            (None, n) => format!("{n} items selected"),
        }
    } else {
        view.status.join(" · ")
    };
    state.status.clone_from(&text);
    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(status.shrink2(vec2(6.0, 0.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    bar.set_clip_rect(status.intersect(ui.clip_rect()));
    bar.add(egui::Label::new(text).selectable(false).truncate());
}

fn separator(ui: &mut egui::Ui, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 1.0), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}

/// One row: the file icon, the controls, then the badges and the close
/// button from the right. Icons only, labels as tooltips.
fn toolbar(
    ui: &mut egui::Ui,
    row: Rect,
    view: &View<'_>,
    state: &mut PaneState,
    acts: &mut Vec<Act>,
) {
    // Right: close, then the badges, so they keep their place.
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    right.set_clip_rect(row.intersect(ui.clip_rect()));
    right.add_space(2.0);
    if icon_button(&mut right, icons::CLOSE, "Close", None).clicked() {
        acts.push(Act::Close);
    }
    badges(&mut right, view, state, acts);
    let right_edge = right.min_rect().left();

    // Left: the file icon, then the controls.
    let left_rect = Rect::from_min_max(row.min, pos2(right_edge.max(row.left()), row.bottom()));
    let mut left = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    left.set_clip_rect(left_rect.intersect(ui.clip_rect()));
    left.add_space(6.0);
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
}

/// The badges, right to left so the first section ends up leftmost.
fn badges(ui: &mut egui::Ui, view: &View<'_>, state: &mut PaneState, acts: &mut Vec<Act>) {
    for (i, section) in view.sections.iter().enumerate().rev() {
        let is_pinned = view.pinned == Some(i);
        let response = icon_button(ui, section.glyph, &section.title, Some(is_pinned));
        let response = response.on_hover_ui(|ui| {
            ui.set_max_width(360.0);
            ui.strong(&section.title);
            section_rows(ui, section, None, 320.0);
        });
        if response.clicked() {
            acts.push(Act::Setting(SettingsChange::PreviewPin {
                kind: view.kind.to_owned(),
                section: (!is_pinned).then(|| section.id.to_owned()),
            }));
        }
        state.badges.insert(
            0,
            BadgeItem {
                id: section.id.to_owned(),
                title: section.title.clone(),
                pinned: is_pinned,
            },
        );
    }
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

/// A section's rows as a key–value list: searchable when long, copy on
/// every row, scrolling past `max_h`. Values truncate to the width.
fn section_rows(ui: &mut egui::Ui, section: &Section, filter: Option<&mut String>, max_h: f32) {
    let width = ui.available_width();
    let needle = match filter {
        Some(filter) if section.rows.len() > SEARCHABLE_FROM => {
            ui.add_space(4.0);
            ui.add_sized(
                vec2(width - 8.0, ui.spacing().interact_size.y),
                egui::TextEdit::singleline(filter).hint_text("Search"),
            );
            ui.add_space(2.0);
            filter.trim().to_lowercase()
        }
        _ => String::new(),
    };
    let rows: Vec<&(String, String)> = section
        .rows
        .iter()
        .filter(|(k, v)| {
            needle.is_empty()
                || k.to_lowercase().contains(&needle)
                || v.to_lowercase().contains(&needle)
        })
        .collect();
    let key_w = (width * 0.35).clamp(48.0, 160.0);
    egui::ScrollArea::vertical()
        .id_salt(("section", section.id))
        .max_height(max_h)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_width(width);
            for (key, value) in rows {
                section_row(ui, width, key_w, key, value);
            }
            if section.rows.is_empty() {
                ui.weak("Nothing to show");
            }
        });
}

/// One key–value row with a copy button; both labels truncate.
fn section_row(ui: &mut egui::Ui, width: f32, key_w: f32, key: &str, value: &str) {
    let height = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    row.set_clip_rect(rect.intersect(ui.clip_rect()));
    row.add_space(6.0);
    row.add_sized(
        vec2(key_w, height),
        egui::Label::new(egui::RichText::new(key).weak())
            .selectable(false)
            .truncate(),
    );
    let copy_w = 22.0;
    let value_w = (rect.right() - row.cursor().left() - copy_w - 8.0).max(8.0);
    row.add_sized(
        vec2(value_w, height),
        egui::Label::new(value).selectable(false).truncate(),
    );
    if row
        .add(egui::Button::new(egui::RichText::new(icons::COPY).small()).frame(false))
        .on_hover_text("Copy")
        .clicked()
    {
        row.ctx().copy_text(value.to_owned());
    }
}
