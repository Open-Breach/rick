//! The title row's search box, which is also the command palette: plain
//! text searches the current location by name and, when the platform has a
//! search index the connectors found, every available index after it, the
//! hits listed with their paths (QUESTIONS.md 118); a leading `>` lists
//! every Action and UI intent fuzzy-matched, recently used first. Mod+P
//! focuses it empty, Mod+Shift+P with `>` typed; Escape clears it and gives
//! the keyboard back to the list.

use app::App;
use ext::t;
use model::PaneId;
use types::{Entry, EntryId};

use crate::keys::{Intent, KeyMap};
use crate::palette::{self, Item};
use crate::theme::{self, ICON, TEXT_BODY, TEXT_SECONDARY};
use crate::widgets;

/// What the user picked from the dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// An entry of the current location, by row index and id.
    Entry {
        /// Its row in the listing.
        index: usize,
        /// Its id.
        id: EntryId,
    },
    /// A hit of a search index, anywhere on the machine: the pane goes to
    /// its folder and selects it there.
    Hit(Entry),
    /// A command.
    Intent(Intent),
}

/// The box's state.
#[derive(Debug, Default)]
pub struct SearchBox {
    /// The text, `>`-prefixed in command mode.
    pub text: String,
    /// The highlighted row of the dropdown.
    selected: usize,
    /// Take the keyboard on the next frame.
    focus: bool,
    /// The box has the keyboard.
    active: bool,
    /// The text to hand the search indexes next, when it changed since the
    /// last frame; the shell takes it with [`take_query`](Self::take_query).
    query: Option<String>,
    /// An index was available when the dropdown was last drawn, so an empty
    /// dropdown says so.
    searching_indexes: bool,
    /// The dropdown of the last frame, `None` while it is closed.
    drawn: Option<Drawn>,
}

/// How many name matches the dropdown lists.
const MAX_ROWS: usize = 40;
/// The dropdown's width where the window has the room: the box's own
/// width is no measure for it, since the box gives way to the tabs down to
/// 120 px, and a name with its folder beside it needs the room.
pub const DROPDOWN_WIDTH: f32 = 560.0;
/// The dropdown keeps this far from the window's edges.
const DROPDOWN_MARGIN: f32 = 8.0;
/// The dropdown scrolls beyond this height: twelve rows.
const DROPDOWN_HEIGHT: f32 = 360.0;
/// A row's height, a menu entry's.
const ROW_HEIGHT: f32 = 30.0;
/// A row's padding at each side.
const ROW_PAD: f32 = 10.0;
/// The gap between a row's icon and its label.
const ICON_GAP: f32 = 10.0;
/// The least room between a row's label and the text at its right.
const HINT_GAP: f32 = 24.0;
/// The share of a row the text at its right may take from a label that
/// would fill the row by itself.
const HINT_SHARE: f32 = 0.5;

/// Where the dropdown and the texts of its rows were drawn in the last
/// frame, for tests.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    /// The dropdown's card.
    pub card: egui::Rect,
    /// Every row's texts, in order.
    pub rows: Vec<DrawnRow>,
}

/// The texts of one row of the dropdown as they were drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawnRow {
    /// The whole label, cut or not.
    pub label: String,
    /// Where the label was painted.
    pub label_rect: egui::Rect,
    /// Whether the label was cut with an ellipsis.
    pub label_cut: bool,
    /// Where the text at the right was painted, when the row has one.
    pub hint_rect: Option<egui::Rect>,
    /// Whether the text at the right was cut with an ellipsis.
    pub hint_cut: bool,
}

impl SearchBox {
    /// Puts `text` in the box and focuses it.
    pub fn focus_with(&mut self, text: &str) {
        text.clone_into(&mut self.text);
        self.selected = 0;
        self.focus = true;
        self.active = true;
    }

    /// Whether the box has the keyboard.
    #[must_use]
    pub fn active(&self) -> bool {
        self.active
    }

    /// The dropdown as the last frame drew it, `None` while it is closed.
    #[must_use]
    pub fn drawn(&self) -> Option<&Drawn> {
        self.drawn.as_ref()
    }

    /// Whether the text asks for commands.
    #[must_use]
    pub fn command_mode(&self) -> bool {
        self.text.starts_with('>')
    }

    /// The text the search indexes should be asked for, when it changed
    /// since the shell last took it: the trimmed search text, or empty when
    /// the box was cleared or switched to commands.
    pub fn take_query(&mut self) -> Option<String> {
        self.query.take()
    }

    /// Notes the current text as the next query for the indexes.
    fn note_query(&mut self) {
        let text = if self.command_mode() {
            String::new()
        } else {
            self.text.trim().to_owned()
        };
        self.query = Some(text);
    }

    /// Draws the box in `rect` and its dropdown below; returns what the user
    /// picked, if anything. `salt` tells one window's box from another's.
    ///
    /// The box sits on the title row's band (SPEC.md *Title row*): a field
    /// in the band's search fill with no border at rest — `bg/pane` on a
    /// band that agrees with the scheme — its text and its placeholder in
    /// the band's colours for them; the focus ring shows while it has the
    /// focus.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        app: &App,
        keys: &KeyMap,
        pane: PaneId,
        recents: &[Intent],
        salt: u64,
    ) -> Option<Pick> {
        let id = egui::Id::new(("title-search", salt));
        let band = crate::theme::band(ui);
        let hint = format!("{}  Search, or > for commands", crate::icons::SEARCH);
        let edit = egui::TextEdit::singleline(&mut self.text)
            .id(id)
            .hint_text(egui::RichText::new(hint).color(band.search_placeholder))
            .text_color(band.search_text)
            .desired_width(rect.width())
            .vertical_align(egui::Align::Center);
        let output = ui
            .scope(|ui| {
                let visuals = ui.visuals_mut();
                visuals.text_edit_bg_color = Some(band.search_fill);
                visuals.extreme_bg_color = band.search_fill;
                visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                ui.put(rect, edit)
            })
            .inner;
        if self.focus {
            output.request_focus();
            // Place the caret after the text, so `>` stays in front.
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let end = egui::text::CCursor::new(self.text.chars().count());
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ui.ctx(), id);
            }
            self.focus = false;
        }
        if output.changed() {
            self.selected = 0;
            self.note_query();
        }
        // Enter and Escape make the text edit give up the focus in the same
        // frame, so they count while the box had it a moment ago.
        let was_active = self.active;
        let has_focus = output.has_focus();
        self.drawn = None;
        let (enter, escape) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        let finishing = was_active && output.lost_focus() && (enter || escape);
        self.active = has_focus;
        if !has_focus && !finishing {
            return None;
        }
        let (up, down) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            )
        });
        if escape {
            self.text.clear();
            self.active = false;
            self.note_query();
            ui.memory_mut(|m| m.surrender_focus(id));
            return None;
        }

        self.searching_indexes = !app.model().search_indexes.is_empty();
        let rows = self.rows(app, keys, pane, recents);
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        if down && self.selected + 1 < rows.len() {
            self.selected += 1;
        }
        self.selected = self.selected.min(rows.len().saturating_sub(1));
        let mut picked = None;
        if enter && let Some(row) = rows.get(self.selected) {
            picked.clone_from(&row.pick);
        }

        let show_dropdown = self.command_mode() || !self.text.trim().is_empty();
        if show_dropdown {
            self.dropdown(ui, id, rect, &rows, &mut picked);
        }

        if picked.is_some() {
            self.text.clear();
            self.active = false;
            self.note_query();
            ui.memory_mut(|m| m.surrender_focus(id));
        }
        picked
    }

    /// The rows below the box, on a card of its own width: the box's left
    /// edge where the window has the room, moved left where it has not,
    /// [`DROPDOWN_WIDTH`] wide or as wide as the box where that is wider,
    /// and never past the window's edges. The card is as tall as its rows,
    /// growing and shrinking with them, and scrolls beyond
    /// [`DROPDOWN_HEIGHT`]. Each row is a line of its own ([`row`]).
    fn dropdown(
        &mut self,
        ui: &mut egui::Ui,
        id: egui::Id,
        rect: egui::Rect,
        rows: &[Row],
        picked: &mut Option<Pick>,
    ) {
        let screen = ui.ctx().content_rect();
        let width = rect
            .width()
            .max(DROPDOWN_WIDTH)
            .min(screen.width() - 2.0 * DROPDOWN_MARGIN);
        let left = rect
            .left()
            .min(screen.right() - DROPDOWN_MARGIN - width)
            .max(screen.left() + DROPDOWN_MARGIN);
        let frame = egui::Frame::popup(ui.style());
        let inner = width - frame.total_margin().sum().x;
        // The height the rows want, up to the height the list scrolls at.
        // An area remembers the size it last had and offers its content no
        // more, and a scroll area takes what it is offered: left to them,
        // the list would keep the height of the rows it first opened with
        // and scroll the hits that the index sends a moment later.
        #[allow(clippy::cast_precision_loss)]
        let wanted = (rows.len() as f32 * ROW_HEIGHT).min(DROPDOWN_HEIGHT);
        let mut drawn = Drawn {
            card: egui::Rect::NOTHING,
            rows: Vec::new(),
        };
        let area = egui::Area::new(id.with("dropdown"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(left, rect.bottom() + 4.0))
            .interactable(true);
        let shown = area.show(ui.ctx(), |ui| {
            frame.show(ui, |ui| {
                ui.set_width(inner);
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .max_height(DROPDOWN_HEIGHT)
                    .min_scrolled_height(wanted)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if rows.is_empty() {
                            ui.weak(if self.command_mode() {
                                "No matching command"
                            } else if self.searching_indexes {
                                "Nothing by that name here or in the index"
                            } else {
                                "Nothing by that name here"
                            });
                        }
                        for (i, entry) in rows.iter().enumerate() {
                            let selected = i == self.selected;
                            let response = row(ui, entry, selected, &mut drawn.rows);
                            if selected {
                                response.scroll_to_me(None);
                            }
                            if let Some(why) = &entry.disabled {
                                response.clone().on_hover_text(why);
                            }
                            if response.clicked() {
                                picked.clone_from(&entry.pick);
                            }
                            if response.hovered() {
                                self.selected = i;
                            }
                        }
                    });
            });
        });
        drawn.card = shown.response.rect;
        self.drawn = Some(drawn);
    }

    /// The dropdown's rows for the current text.
    fn rows(&self, app: &App, keys: &KeyMap, pane: PaneId, recents: &[Intent]) -> Vec<Row> {
        if self.command_mode() {
            let query = self.text[1..].trim();
            let items = palette::items(app, keys, pane, recents);
            return palette::ranked(query, items)
                .into_iter()
                .map(|item: Item| Row {
                    icon: None,
                    label: item.label,
                    hint: item.shortcut.map(Hint::Text),
                    pick: item.disabled.is_none().then_some(Pick::Intent(item.intent)),
                    disabled: item.disabled,
                })
                .collect();
        }
        let needle = self.text.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let Some((_, tab)) = app.model().active_tab(pane) else {
            return Vec::new();
        };
        let mut rows: Vec<Row> = tab
            .listing
            .rows()
            .enumerate()
            .filter(|(_, e)| e.name.to_lowercase().contains(&needle))
            .take(MAX_ROWS + 1)
            .map(|(index, e)| Row {
                icon: Some(crate::icons::for_entry(e)),
                label: e.name.clone(),
                hint: Some(Hint::Text(model::type_of(e))),
                disabled: None,
                pick: Some(Pick::Entry { index, id: e.id }),
            })
            .collect();
        // The platform's indexes, after the current location: each hit
        // with its folder, so the same name in two places tells apart.
        let search = &app.model().index_search;
        if search.text == needle_text(&self.text) && !app.model().search_indexes.is_empty() {
            for entry in &search.hits {
                if rows.len() > MAX_ROWS {
                    break;
                }
                let folder = entry
                    .path
                    .parent()
                    .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
                rows.push(Row {
                    icon: Some(crate::icons::for_entry(entry)),
                    label: entry.name.clone(),
                    hint: Some(Hint::Folder(folder)),
                    disabled: None,
                    pick: Some(Pick::Hit(entry.clone())),
                });
            }
            for error in &search.errors {
                rows.push(Row {
                    icon: None,
                    label: error.clone(),
                    hint: None,
                    disabled: Some(t!("search-index-did-not-answer")),
                    pick: None,
                });
            }
            if !search.done {
                rows.push(Row {
                    icon: None,
                    label: "Searching the index…".to_owned(),
                    hint: None,
                    disabled: Some(t!("search-hits-appear-as-the-index-answers")),
                    pick: None,
                });
            }
        }
        if rows.len() > MAX_ROWS {
            rows.truncate(MAX_ROWS);
            rows.push(Row {
                icon: None,
                label: "More matches: keep typing".to_owned(),
                hint: None,
                disabled: Some(t!("search-narrow-the-search")),
                pick: None,
            });
        }
        rows
    }
}

/// The search text as the indexes are asked for it.
fn needle_text(text: &str) -> &str {
    text.trim()
}

/// One line of the dropdown.
struct Row {
    /// The glyph before the label: an entry's icon. A command has none.
    icon: Option<&'static str>,
    label: String,
    hint: Option<Hint>,
    disabled: Option<String>,
    pick: Option<Pick>,
}

/// The text at a row's right.
enum Hint {
    /// A shortcut or a type: cut at its end where it has to be.
    Text(String),
    /// The folder of an index hit: cut at its start, so that the folder
    /// itself, the end of the path, stays.
    Folder(String),
}

impl Hint {
    /// The whole text.
    fn text(&self) -> &str {
        match self {
            Self::Text(text) | Self::Folder(text) => text,
        }
    }
}

/// One row of the dropdown, [`ROW_HEIGHT`] tall over the card's width: the
/// icon, the label, and at the right the hint in `text/secondary`. The two
/// texts never share a pixel: the hint takes what it needs, up to
/// [`HINT_SHARE`] of the row where the label wants the row for itself, the
/// label has the rest less [`HINT_GAP`], and what does not fit ends in an
/// ellipsis, a folder at its start. A text that was cut shows whole as the
/// tooltip. The selected row is `bg/selected-soft`; a row that cannot be
/// picked is `text/placeholder`. Its accessible name is the label; the
/// hint is a label of its own.
fn row(ui: &mut egui::Ui, row: &Row, selected: bool, drawn: &mut Vec<DrawnRow>) -> egui::Response {
    let scheme = theme::scheme(ui);
    let metrics = theme::metrics(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click(),
    );
    if selected {
        ui.painter()
            .rect_filled(rect, metrics.corner_radius(), scheme.bg_selected_soft);
    }
    let (label_colour, icon_colour, hint_colour) = if row.disabled.is_some() {
        (
            scheme.text_placeholder,
            scheme.text_placeholder,
            scheme.text_placeholder,
        )
    } else {
        (
            scheme.text_primary,
            scheme.icon_default,
            scheme.text_secondary,
        )
    };
    let middle = rect.center().y;
    let mut left = rect.left() + ROW_PAD;
    let right = rect.right() - ROW_PAD;
    if let Some(icon) = row.icon {
        ui.painter().text(
            egui::pos2(left + ICON / 2.0, middle),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(ICON),
            icon_colour,
        );
        left += ICON + ICON_GAP;
    }
    let room = (right - left).max(0.0);
    let label_font = egui::FontId::proportional(TEXT_BODY);
    let mut label_room = room;
    let mut hint_rect = None;
    let mut hint_cut = false;
    if let Some(hint) = &row.hint {
        let font = egui::FontId::proportional(TEXT_SECONDARY);
        // What the label leaves, and its share where the label would take
        // the row.
        let wanted = ui
            .painter()
            .layout_no_wrap(row.label.clone(), label_font.clone(), label_colour)
            .size()
            .x;
        let most = (room - wanted - HINT_GAP).max(room * HINT_SHARE);
        let (galley, cut) = match hint {
            Hint::Text(text) => {
                let galley = widgets::elided(ui, text, font, hint_colour, most);
                let cut = galley.elided;
                (galley, cut)
            }
            Hint::Folder(text) => widgets::elided_at_start(ui, text, &font, hint_colour, most),
        };
        let size = galley.size();
        let at = egui::Rect::from_min_size(egui::pos2(right - size.x, middle - size.y / 2.0), size);
        ui.painter().galley(at.min, galley, hint_colour);
        let spoken = hint.text().to_owned();
        let over = ui.interact(at, response.id.with("hint"), egui::Sense::hover());
        over.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, spoken.clone())
        });
        if cut {
            over.on_hover_text(hint.text());
        }
        label_room = (room - size.x - HINT_GAP).max(0.0);
        hint_rect = Some(at);
        hint_cut = cut;
    }
    let galley = widgets::elided(ui, &row.label, label_font, label_colour, label_room);
    let size = galley.size();
    let label_cut = galley.elided;
    let label_rect = egui::Rect::from_min_size(egui::pos2(left, middle - size.y / 2.0), size);
    ui.painter().galley(label_rect.min, galley, label_colour);
    let (spoken, enabled) = (row.label.clone(), row.pick.is_some());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, spoken.clone())
    });
    drawn.push(DrawnRow {
        label: row.label.clone(),
        label_rect,
        label_cut,
        hint_rect,
        hint_cut,
    });
    if label_cut && row.disabled.is_none() {
        return response.on_hover_text(&row.label);
    }
    response
}
