//! The Keyboard shortcuts dialogue (SPEC.md *Keyboard shortcuts
//! dialogue*): the live shortcut table on the dialogue shell.
//!
//! On the left the categories with their counts — *All*, Rick's own, then
//! one per plugin that contributes actions, so a plugin's actions are
//! filed under the plugin and nowhere else. On the right a search field
//! that narrows the list by name, or by a chord pressed in it; a grey
//! column header; and the rows, grouped under their category's heading.
//! A shortcut is drawn as key caps. A click on a row's shortcut cell
//! records: the next chord becomes the row's shortcut, Backspace unbinds
//! it and Escape leaves it. A row that differs from its default carries a
//! dot and a live reset button; a chord bound to two rows is allowed and
//! shown by an amber *also:* chip on both. Every change is a
//! `SettingsChange::Shortcut`, applied at once.

use app::{App, Command};
use egui::{Align, Align2, FontId, Rect, Sense, pos2, vec2};
use ext::{Key, Modifiers, Shortcut, t};
use model::SettingsChange;

use crate::keys::{self, Category, Intent, KeyMap, Row, from_egui_key, from_egui_modifiers};
use crate::shell::Request;
use crate::theme::{self, ICON, TEXT_BODY, TEXT_SECONDARY};
use crate::{dialog, icons, widgets};

/// The dialogue's width.
const WIDTH: f32 = 760.0;
/// The body's height where the window has the room.
const BODY: f32 = 520.0;
/// The category column's width.
const CATEGORIES: f32 = 190.0;
/// A category row's height.
const CATEGORY_ROW: f32 = 30.0;
/// The band that holds the search field.
const SEARCH_ROW: f32 = 54.0;
/// The search field's height.
const SEARCH_FIELD: f32 = 32.0;
/// The grey column header's height.
const COLUMNS: f32 = 28.0;
/// A category heading's height in the list.
const HEADING: f32 = 29.0;
/// A row's height.
const ROW: f32 = 34.0;
/// The shortcut cell's width and height.
const CELL: egui::Vec2 = vec2(200.0, 24.0);
/// A key cap's height.
const CAP: f32 = 22.0;
/// The list's padding at each side.
const PAD: f32 = 18.0;
/// The gap between a row's name, its cell and its reset button.
const GAP: f32 = 12.0;
/// The reset button's side.
const RESET: f32 = 24.0;
/// The dot of a changed row and of a recording cell.
const DOT: f32 = 6.0;
/// The text size of key caps and chips.
const SMALL: f32 = theme::TEXT_CAPS;

/// What the dialogue is doing.
#[derive(Debug, Default)]
pub struct ShortcutsScreen {
    /// Shown.
    pub open: bool,
    /// The row whose shortcut cell is recording.
    pub rebinding: Option<Intent>,
    /// The search field's text.
    pub filter: String,
    /// The chord pressed in the search field, which narrows the list to
    /// what it is bound to.
    pub chord: Option<Shortcut>,
    /// The chosen category; `None` is *All*.
    pub category: Option<Category>,
    /// The search field takes the focus in the next frame: the dialogue
    /// has just opened.
    focus: bool,
}

/// What a frame of the dialogue asked for.
enum Ask {
    Close,
    Record(Intent),
    Reset(Intent),
    ResetAll,
}

impl ShortcutsScreen {
    /// Opens the dialogue on *All*, its search field empty and focused.
    pub fn open(&mut self) {
        *self = Self {
            open: true,
            focus: true,
            ..Self::default()
        };
    }

    /// The dialogue, when it is open.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        keys: &KeyMap,
        app: &App,
        requests: &mut Vec<Request>,
    ) {
        if !self.open {
            return;
        }
        let rows = keys::rows_of(app);
        self.take_keys(ctx, requests);
        let mut asked = None;
        let modal = dialog::modal(ctx, egui::Id::new("shortcuts"), WIDTH, |ui| {
            let scheme = theme::scheme(ui);
            if dialog::band(
                ui,
                None,
                &t!("keyboard-shortcuts"),
                &t!("shortcuts-subtitle"),
            ) {
                asked = Some(Ask::Close);
            }
            // Header and footer stay in view in a low window.
            let room = ui.ctx().content_rect().height() - 64.0 - 68.0 - 48.0;
            let height = BODY.min(room).max(ROW * 4.0);
            let (body, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
            let split = body.left() + CATEGORIES;
            let left = Rect::from_min_max(body.min, pos2(split, body.bottom()));
            let right = Rect::from_min_max(pos2(split, body.top()), body.max);
            self.categories(ui, left, &rows);
            ui.painter()
                .vline(split - 0.5, body.y_range(), scheme.hairline());
            self.list(ui, right, &rows, keys, &mut asked);
            let footer = std::cell::Cell::new(None);
            dialog::footer(
                ui,
                |left| {
                    if widgets::button(left, &t!("shortcuts-reset-all"), false, true).clicked() {
                        footer.set(Some(Ask::ResetAll));
                    }
                },
                |right| {
                    if widgets::button(right, &t!("close"), true, true).clicked() {
                        footer.set(Some(Ask::Close));
                    }
                },
            );
            if let Some(ask) = footer.into_inner() {
                asked = Some(ask);
            }
        });
        if modal.should_close() {
            asked = Some(Ask::Close);
        }
        let reset = |action: &str| {
            Request::Command(Command::Settings(SettingsChange::Shortcut {
                action: action.to_owned(),
                binding: None,
            }))
        };
        match asked {
            None => {}
            Some(Ask::Close) => *self = Self::default(),
            Some(Ask::Record(intent)) => {
                self.rebinding = Some(intent);
                ctx.memory_mut(|m| m.surrender_focus(search_id()));
            }
            Some(Ask::Reset(intent)) => {
                self.rebinding = None;
                requests.push(reset(intent.id()));
            }
            Some(Ask::ResetAll) => {
                self.rebinding = None;
                requests.extend(app.model().settings.shortcuts.keys().map(|id| reset(id)));
            }
        }
    }

    /// This frame's key presses, before any widget sees them. While a
    /// cell records, the next chord is the row's new shortcut — Backspace
    /// unbinds, Escape leaves it. Otherwise, in the search field, a chord
    /// — a key with Mod or Alt, or a function key — narrows the list to
    /// what it is bound to, and Backspace or typing takes it away again.
    fn take_keys(&mut self, ctx: &egui::Context, requests: &mut Vec<Request>) {
        if let Some(intent) = self.rebinding {
            let Some(pressed) = ctx.input_mut(|input| take_chord(input, |_| true)) else {
                return;
            };
            self.rebinding = None;
            let bare = pressed.modifiers.is_empty();
            let binding = match pressed.key {
                Key::Escape if bare => return,
                Key::Backspace if bare => String::new(),
                _ => pressed.text(),
            };
            requests.push(Request::Command(Command::Settings(
                SettingsChange::Shortcut {
                    action: intent.id().to_owned(),
                    binding: Some(binding),
                },
            )));
        } else if ctx.memory(|m| m.has_focus(search_id())) {
            let is_chord = |shortcut: Shortcut| {
                shortcut
                    .modifiers
                    .intersects(Modifiers::MOD | Modifiers::ALT)
                    || matches!(shortcut.key, Key::F(_))
            };
            if let Some(chord) = ctx.input_mut(|input| take_chord(input, is_chord)) {
                self.chord = Some(chord);
                self.filter.clear();
            } else if self.chord.is_some() {
                let (erased, typed) = ctx.input_mut(|input| {
                    let erased = input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace);
                    let typed = input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Text(_)));
                    (erased, typed)
                });
                if erased || typed {
                    self.chord = None;
                }
            }
        }
    }

    /// The category column: *All*, Rick's own categories and one per
    /// plugin, each with its icon, its name and the number of its actions.
    fn categories(&mut self, ui: &mut egui::Ui, rect: Rect, rows: &[Row]) {
        let scheme = theme::scheme(ui);
        let radius = theme::metrics(ui.ctx()).corner_radius();
        let mut entries: Vec<(Option<Category>, usize)> = vec![(None, 0)];
        for row in rows.iter().filter(|row| row.category != Category::List) {
            entries[0].1 += 1;
            match entries.iter_mut().find(|e| e.0 == Some(row.category)) {
                Some(entry) => entry.1 += 1,
                None => entries.push((Some(row.category), 1)),
            }
        }
        ui.painter().rect_filled(rect, 0.0, scheme.bg_panel);
        let mut column = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        column.set_clip_rect(rect.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical()
            .id_salt("shortcut-categories")
            .auto_shrink([false, false])
            .show(&mut column, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                ui.add_space(12.0);
                for (category, count) in entries {
                    let (line, response) = ui.allocate_exact_size(
                        vec2(ui.available_width(), CATEGORY_ROW),
                        Sense::click(),
                    );
                    let inner = Rect::from_min_max(
                        pos2(line.left() + 10.0, line.top()),
                        pos2(line.right() - 11.0, line.bottom()),
                    );
                    let chosen = self.category == category;
                    if chosen {
                        ui.painter()
                            .rect_filled(inner, radius, scheme.bg_selected_soft);
                    } else if response.hovered() {
                        ui.painter().rect_filled(inner, radius, scheme.bg_subtle);
                    }
                    if let Some(category) = category {
                        ui.painter().text(
                            pos2(inner.left() + 10.0 + ICON / 2.0, inner.center().y),
                            Align2::CENTER_CENTER,
                            category_icon(category),
                            FontId::proportional(ICON),
                            scheme.icon_default,
                        );
                    }
                    let number = ui.painter().text(
                        pos2(inner.right() - 8.0, inner.center().y),
                        Align2::RIGHT_CENTER,
                        count.to_string(),
                        FontId::proportional(TEXT_SECONDARY),
                        scheme.text_secondary,
                    );
                    let name = category.map_or_else(|| t!("shortcuts-all"), category_name);
                    let galley = widgets::elided(
                        ui,
                        &name,
                        FontId::proportional(TEXT_BODY),
                        scheme.text_primary,
                        number.left() - 8.0 - (inner.left() + 34.0),
                    );
                    ui.painter().galley(
                        pos2(
                            inner.left() + 34.0,
                            inner.center().y - galley.size().y / 2.0,
                        ),
                        galley,
                        scheme.text_primary,
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::RadioButton,
                            true,
                            chosen,
                            name.clone(),
                        )
                    });
                    if response.clicked() {
                        self.category = category;
                    }
                }
                ui.add_space(12.0);
            });
    }

    /// The right column: the search field, the column header and the rows
    /// of the chosen category that the search leaves, under their
    /// category's heading.
    fn list(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        rows: &[Row],
        keys: &KeyMap,
        asked: &mut Option<Ask>,
    ) {
        let scheme = theme::scheme(ui);
        let search = Rect::from_min_size(rect.min, vec2(rect.width(), SEARCH_ROW));
        self.search_field(ui, search);
        let head = Rect::from_min_size(search.left_bottom(), vec2(rect.width(), COLUMNS));
        ui.painter().rect_filled(head, 0.0, scheme.bg_panel);
        for y in [head.top() + 0.5, head.bottom() - 0.5] {
            ui.painter().hline(head.x_range(), y, scheme.hairline());
        }
        let cell_left = head.right() - PAD - RESET - GAP - CELL.x;
        for (x, title) in [
            (head.left() + PAD, t!("shortcuts-action")),
            (cell_left, t!("shortcuts-shortcut")),
        ] {
            widgets::line(
                ui,
                pos2(x, head.center().y),
                &title,
                FontId::proportional(TEXT_SECONDARY),
                scheme.text_secondary,
                CELL.x,
            );
        }

        let needle = self.filter.trim().to_lowercase();
        let shown: Vec<&Row> = rows
            .iter()
            .filter(|row| row.category != Category::List)
            .filter(|row| self.category.is_none_or(|chosen| row.category == chosen))
            .filter(|row| match self.chord {
                Some(chord) => keys.shortcuts_for(row.intent).any(|bound| bound == chord),
                None => needle.is_empty() || row.label.to_lowercase().contains(&needle),
            })
            .collect();
        let below = Rect::from_min_max(head.left_bottom(), rect.max);
        let mut area = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(below)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        area.set_clip_rect(below.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical()
            .id_salt(("shortcut-rows", self.category))
            .auto_shrink([false, false])
            .show(&mut area, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                if shown.is_empty() {
                    let (line, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                    widgets::line(
                        ui,
                        pos2(line.left() + PAD, line.center().y),
                        &t!("shortcuts-none"),
                        FontId::proportional(TEXT_BODY),
                        scheme.text_secondary,
                        line.width() - 2.0 * PAD,
                    );
                }
                let mut heading = None;
                for row in shown {
                    if heading != Some(row.category) {
                        heading = Some(row.category);
                        let (line, _) = ui.allocate_exact_size(
                            vec2(ui.available_width(), HEADING),
                            Sense::hover(),
                        );
                        let mut at =
                            ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(
                                pos2(line.left() + PAD, line.top() + 12.0),
                                line.max,
                            )));
                        widgets::caps_heading(&mut at, &heading_text(row.category));
                    }
                    let recording = self.rebinding == Some(row.intent);
                    row_ui(ui, row, rows, keys, recording, asked);
                }
            });
    }

    /// The search field: a magnifier, then the text — or the chord
    /// pressed in the field, as key caps.
    fn search_field(&mut self, ui: &mut egui::Ui, band: Rect) {
        let scheme = theme::scheme(ui);
        let field = Rect::from_min_max(
            pos2(band.left() + PAD, band.top() + 12.0),
            pos2(band.right() - PAD, band.top() + 12.0 + SEARCH_FIELD),
        );
        ui.painter().rect(
            field,
            theme::CARD_RADIUS,
            scheme.bg_card,
            egui::Stroke::new(1.0, scheme.border_strong),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            pos2(field.left() + 11.0 + ICON / 2.0, field.center().y),
            Align2::CENTER_CENTER,
            icons::SEARCH,
            FontId::proportional(ICON),
            scheme.icon_default,
        );
        let mut left = field.left() + 35.0;
        if let Some(chord) = self.chord {
            let caps = Rect::from_min_max(pos2(left, field.top()), field.right_bottom());
            left += key_caps(ui, caps, &[chord]) + 8.0;
        }
        let mut inner = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(left, field.top()),
                    pos2(field.right() - 10.0, field.bottom()),
                ))
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        let font = FontId::proportional(TEXT_BODY);
        let hint = if self.chord.is_some() {
            String::new()
        } else {
            t!("shortcuts-search-hint")
        };
        let response = inner.add(
            egui::TextEdit::singleline(&mut self.filter)
                .id(search_id())
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .font(font.clone())
                .text_color(scheme.text_primary)
                .hint_text(
                    egui::RichText::new(hint)
                        .font(font)
                        .color(scheme.text_placeholder),
                )
                .desired_width(f32::INFINITY)
                .vertical_align(Align::Center),
        );
        if std::mem::take(&mut self.focus) {
            response.request_focus();
        }
        if response.changed() && !self.filter.is_empty() {
            self.chord = None;
        }
    }
}

/// The id of the search field, whose focus decides who takes a chord.
fn search_id() -> egui::Id {
    egui::Id::new("shortcut-search")
}

/// Takes the first chord pressed in this frame that `wanted` accepts out
/// of the input, so no widget acts on it. The windowing glue reports the
/// clipboard chords as copy, cut and paste events, not as key presses:
/// they are Mod+C, Mod+X and Mod+V here. A key is taken as the layout
/// gives it, or as the physical key where the layout gives a character no
/// shortcut names (Shift+1).
fn take_chord(input: &mut egui::InputState, wanted: impl Fn(Shortcut) -> bool) -> Option<Shortcut> {
    let mut found = None;
    input.events.retain(|event| {
        if found.is_some() {
            return true;
        }
        let pressed = match event {
            egui::Event::Key {
                key,
                physical_key,
                modifiers,
                pressed: true,
                ..
            } => from_egui_key(*key)
                .or_else(|| physical_key.and_then(from_egui_key))
                .map(|key| Shortcut {
                    modifiers: from_egui_modifiers(*modifiers),
                    key,
                }),
            egui::Event::Copy => Some(mod_with('c')),
            egui::Event::Cut => Some(mod_with('x')),
            egui::Event::Paste(_) => Some(mod_with('v')),
            _ => None,
        };
        match pressed {
            Some(shortcut) if wanted(shortcut) => {
                found = Some(shortcut);
                false
            }
            _ => true,
        }
    });
    found
}

/// Mod with a letter.
fn mod_with(letter: char) -> Shortcut {
    Shortcut {
        modifiers: Modifiers::MOD,
        key: Key::Char(letter),
    }
}

/// A category's icon.
fn category_icon(category: Category) -> &'static str {
    match category {
        Category::Navigation => icons::FOLDER,
        Category::FileOperations => icons::COPY,
        Category::View => icons::HIDDEN_SHOWN,
        Category::Selection => icons::LIST_CHECKS,
        Category::Panes | Category::List => icons::COLUMNS,
        Category::Search => icons::SEARCH,
        Category::Preview => icons::PREVIEW,
        Category::Plugin { id, .. } => icons::for_plugin(id.0),
    }
}

/// A category's name: Rick's own, or the plugin's as its manifest gives
/// it.
fn category_name(category: Category) -> String {
    match category {
        Category::Navigation => t!("shortcuts-navigation"),
        Category::FileOperations => t!("shortcuts-file-operations"),
        Category::View => t!("shortcuts-view"),
        Category::Selection => t!("shortcuts-selection"),
        Category::Panes | Category::List => t!("shortcuts-panes"),
        Category::Search => t!("shortcuts-search"),
        Category::Preview => t!("shortcuts-preview"),
        Category::Plugin { name, .. } => name.to_owned(),
    }
}

/// The heading of a category's rows in the list; a plugin's says that it
/// is one.
fn heading_text(category: Category) -> String {
    match category {
        Category::Plugin { name, .. } => t!("shortcuts-plugin", name = name),
        other => category_name(other),
    }
}

/// A row's chords as the user reads them, `Ctrl+C, F5`; *Not set* for
/// none.
fn chords_text(chords: &[Shortcut]) -> String {
    if chords.is_empty() {
        t!("shortcuts-not-set")
    } else {
        let spelled: Vec<String> = chords.iter().copied().map(keys::spelled).collect();
        spelled.join(", ")
    }
}

/// One row, 34 px: the action's name, its shortcut cell and its reset
/// button.
fn row_ui(
    ui: &mut egui::Ui,
    row: &Row,
    rows: &[Row],
    keys: &KeyMap,
    recording: bool,
    asked: &mut Option<Ask>,
) {
    let scheme = theme::scheme(ui);
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    if recording {
        ui.painter().rect_filled(line, 0.0, scheme.bg_selected_soft);
    }
    let reset = Rect::from_center_size(
        pos2(line.right() - PAD - RESET / 2.0, line.center().y),
        vec2(RESET, RESET),
    );
    let cell = Rect::from_center_size(
        pos2(reset.left() - GAP - CELL.x / 2.0, line.center().y),
        CELL,
    );
    widgets::line(
        ui,
        pos2(line.left() + PAD, line.center().y),
        &row.label,
        FontId::proportional(TEXT_BODY),
        scheme.text_primary,
        cell.left() - GAP - (line.left() + PAD),
    );
    let chords: Vec<Shortcut> = keys.shortcuts_for(row.intent).collect();
    let changed = chords != row.defaults;
    let bound = chords_text(&chords);
    // What the cell says: the chords, or what to press while it records.
    let said = if recording {
        t!("shortcuts-recording")
    } else {
        bound.clone()
    };

    let response = ui
        .interact(
            cell,
            ui.id().with(("cell", row.intent.id())),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            t!(
                "shortcuts-cell",
                action = row.label.as_str(),
                keys = said.as_str()
            ),
        )
    });
    if response.clicked() {
        *asked = Some(Ask::Record(row.intent));
    }
    if recording {
        recording_cell(ui, cell);
    } else {
        let others: Vec<&str> = keys
            .conflicts(row.intent)
            .into_iter()
            .filter_map(|other| rows.iter().find(|r| r.intent == other))
            .map(|r| r.label.as_str())
            .collect();
        if bound_cell(ui, cell, &chords, changed, &others) {
            response.on_hover_text(bound);
        }
    }
    reset_button(ui, reset, row, changed, asked);
}

/// The cell while it records: a `bg/card` field with the focus ring, the
/// dot and what to press.
fn recording_cell(ui: &egui::Ui, cell: Rect) {
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    ui.painter().rect(
        cell,
        radius,
        scheme.bg_card,
        egui::Stroke::new(1.5, scheme.accent_focus),
        egui::StrokeKind::Inside,
    );
    ui.painter().circle_filled(
        pos2(cell.left() + 9.5 + DOT / 2.0, cell.center().y),
        DOT / 2.0,
        scheme.accent_focus,
    );
    ui.painter().text(
        pos2(cell.left() + 21.5, cell.center().y),
        Align2::LEFT_CENTER,
        t!("shortcuts-recording"),
        FontId::proportional(TEXT_SECONDARY),
        scheme.text_secondary,
    );
}

/// The cell at rest: the chords as key caps from the left, or *Not set*;
/// from the right the dot of a changed row and, before it, the *also:*
/// chip that names the first of `others`, the actions bound to the same
/// chord. Returns whether the caps were cut for want of room.
fn bound_cell(
    ui: &mut egui::Ui,
    cell: Rect,
    chords: &[Shortcut],
    changed: bool,
    others: &[&str],
) -> bool {
    let scheme = theme::scheme(ui);
    let mut right = cell.right();
    if changed {
        ui.painter().circle_filled(
            pos2(right - DOT / 2.0, cell.center().y),
            DOT / 2.0,
            scheme.accent_focus,
        );
        right -= DOT + 8.0;
    }
    let wanted = if chords.is_empty() {
        0.0
    } else {
        caps_width(ui, chords)
    };
    if let Some(first) = others.first() {
        let text = t!("shortcuts-also", action = *first);
        let room = (right - cell.left() - wanted - 8.0).max(64.0) - 12.0;
        let galley = widgets::elided(
            ui,
            &text,
            FontId::proportional(SMALL),
            scheme.trash_mid_text,
            room,
        );
        let chip = Rect::from_min_max(
            pos2(right - galley.size().x - 12.0, cell.center().y - 8.5),
            pos2(right, cell.center().y + 8.5),
        );
        ui.painter().rect_filled(chip, 4.0, scheme.trash_mid_bg);
        ui.painter().galley(
            pos2(chip.left() + 6.0, chip.center().y - galley.size().y / 2.0),
            galley,
            scheme.trash_mid_text,
        );
        let all: Vec<String> = others
            .iter()
            .map(|other| t!("shortcuts-also", action = *other))
            .collect();
        let response = ui.interact(
            chip,
            ui.id().with(("also", chip.top().to_bits())),
            Sense::hover(),
        );
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text.clone()));
        response.on_hover_text(all.join("\n"));
        right = chip.left() - 8.0;
    }
    let room = Rect::from_min_max(cell.min, pos2(right.max(cell.left()), cell.bottom()));
    if chords.is_empty() {
        ui.painter().text(
            room.left_center(),
            Align2::LEFT_CENTER,
            t!("shortcuts-not-set"),
            FontId::proportional(TEXT_SECONDARY),
            scheme.text_placeholder,
        );
        return false;
    }
    key_caps(ui, room, chords) > room.width()
}

/// The reset button: the undo arrow, live on a changed row — it names the
/// default in its tooltip and takes the row back to it — and faint and
/// inert on a row at its default.
fn reset_button(ui: &mut egui::Ui, rect: Rect, row: &Row, changed: bool, asked: &mut Option<Ask>) {
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    let default = chords_text(&row.defaults);
    let sense = if changed {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, ui.id().with(("reset", row.intent.id())), sense);
    if changed && response.hovered() {
        ui.painter().rect_filled(rect, radius, scheme.bg_control);
    }
    let colour = if changed {
        scheme.icon_default
    } else {
        scheme.icon_default.gamma_multiply(0.35)
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icons::UNDO,
        FontId::proportional(14.0),
        colour,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            changed,
            t!(
                "shortcuts-reset-of",
                action = row.label.as_str(),
                keys = default.as_str()
            ),
        )
    });
    if changed {
        if response.clicked() {
            *asked = Some(Ask::Reset(row.intent));
        }
        response.on_hover_text(t!("shortcuts-reset", keys = default.as_str()));
    }
}

/// The names of a chord's keys, Mod as the platform's key.
fn key_names(chord: Shortcut) -> Vec<String> {
    keys::spelled(chord).split('+').map(str::to_owned).collect()
}

/// The width the key caps of `chords` take.
fn caps_width(ui: &egui::Ui, chords: &[Shortcut]) -> f32 {
    let font = FontId::proportional(SMALL);
    let measure = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
            .size()
            .x
    };
    let plus = ui
        .painter()
        .layout_no_wrap(
            "+".to_owned(),
            FontId::proportional(TEXT_SECONDARY),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        .x;
    let mut width = 0.0;
    for (index, chord) in chords.iter().enumerate() {
        if index > 0 {
            width += 10.0;
        }
        for (place, name) in key_names(*chord).iter().enumerate() {
            if place > 0 {
                width += 3.0 + plus + 3.0;
            }
            width += (measure(name) + 16.0).ceil();
        }
    }
    width
}

/// Draws `chords` as key caps from the left of `rect`, one chord after
/// the other: each key a 22 px `bg/card` cap with a `border/strong`
/// border, 2 px at its lower edge, and *+* between the caps of a chord.
/// Where the room ends before the caps do, an ellipsis stands for the
/// caps that do not fit whole. Returns the width the caps take uncut.
fn key_caps(ui: &egui::Ui, rect: Rect, chords: &[Shortcut]) -> f32 {
    let scheme = theme::scheme(ui);
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let font = FontId::proportional(SMALL);
    let top = rect.center().y - CAP / 2.0;
    let whole = caps_width(ui, chords);
    let ellipsis = painter.layout_no_wrap("…".to_owned(), font.clone(), scheme.text_secondary);
    // The caps end here when they do not all fit: the ellipsis follows.
    let limit = if whole > rect.width() {
        rect.right() - ellipsis.size().x - 4.0
    } else {
        rect.right()
    };
    let mut x = rect.left();
    'chords: for (index, chord) in chords.iter().enumerate() {
        if index > 0 {
            x += 10.0;
        }
        for (place, name) in key_names(*chord).iter().enumerate() {
            let galley = painter.layout_no_wrap(name.clone(), font.clone(), scheme.text_primary);
            let plus = painter.layout_no_wrap(
                "+".to_owned(),
                FontId::proportional(TEXT_SECONDARY),
                scheme.text_secondary,
            );
            let lead = if place > 0 {
                3.0 + plus.size().x + 3.0
            } else {
                0.0
            };
            let width = (galley.size().x + 16.0).ceil();
            if x + lead + width > limit + 0.5 {
                painter.galley(
                    pos2(x + 4.0, rect.center().y - ellipsis.size().y / 2.0),
                    ellipsis,
                    scheme.text_secondary,
                );
                break 'chords;
            }
            if place > 0 {
                painter.galley(
                    pos2(x + 3.0, rect.center().y - plus.size().y / 2.0),
                    plus,
                    scheme.text_secondary,
                );
                x += lead;
            }
            let cap = Rect::from_min_size(pos2(x, top), vec2(width, CAP));
            // The lower edge is 2 px: the border colour under a cap that
            // ends a pixel higher.
            painter.rect_filled(cap, 5.0, scheme.border_strong);
            painter.rect(
                Rect::from_min_max(cap.min, pos2(cap.right(), cap.bottom() - 1.0)),
                5.0,
                scheme.bg_card,
                egui::Stroke::new(1.0, scheme.border_strong),
                egui::StrokeKind::Inside,
            );
            painter.galley(
                pos2(
                    cap.left() + 8.0,
                    cap.center().y - 0.5 - galley.size().y / 2.0,
                ),
                galley,
                scheme.text_primary,
            );
            x = cap.right();
        }
    }
    whole
}
