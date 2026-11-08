//! The title row's search box, which is also the command palette: plain
//! text searches the current location by name and, when the platform has a
//! search index the connectors found, every available index after it, the
//! hits listed with their paths (QUESTIONS.md 118); a leading `>` lists
//! every Action and UI intent fuzzy-matched, recently used first. Mod+P
//! focuses it empty, Mod+Shift+P with `>` typed; Escape clears it and gives
//! the keyboard back to the list.

use app::App;
use model::PaneId;
use types::{Entry, EntryId};

use crate::keys::{Intent, KeyMap};
use crate::palette::{self, Item};

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
}

/// How many name matches the dropdown lists.
const MAX_ROWS: usize = 40;

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
        let hint = format!("{}  Search, or > for commands", crate::icons::SEARCH);
        let edit = egui::TextEdit::singleline(&mut self.text)
            .id(id)
            .hint_text(hint)
            .desired_width(rect.width())
            .vertical_align(egui::Align::Center);
        let output = ui.put(rect, edit);
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

    /// The rows below the box.
    fn dropdown(
        &mut self,
        ui: &mut egui::Ui,
        id: egui::Id,
        rect: egui::Rect,
        rows: &[Row],
        picked: &mut Option<Pick>,
    ) {
        {
            let area = egui::Area::new(id.with("dropdown"))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.left_bottom() + egui::vec2(0.0, 4.0))
                .interactable(true);
            area.show(ui.ctx(), |ui| {
                ui.set_width(rect.width());
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(rect.width() - 16.0);
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            if rows.is_empty() {
                                ui.weak(if self.command_mode() {
                                    "No matching command"
                                } else if self.searching_indexes {
                                    "Nothing by that name here or in the index"
                                } else {
                                    "Nothing by that name here"
                                });
                            }
                            for (i, row) in rows.iter().enumerate() {
                                let selected = i == self.selected;
                                let text = egui::RichText::new(&row.label);
                                let text = if row.disabled.is_some() {
                                    text.weak()
                                } else {
                                    text
                                };
                                let response = ui
                                    .horizontal(|ui| {
                                        let r = ui.selectable_label(selected, text);
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if let Some(hint) = &row.hint {
                                                    ui.weak(hint);
                                                }
                                            },
                                        );
                                        r
                                    })
                                    .inner;
                                if selected {
                                    response.scroll_to_me(None);
                                }
                                if let Some(why) = row.disabled {
                                    response.clone().on_hover_text(why);
                                }
                                if response.clicked() {
                                    picked.clone_from(&row.pick);
                                }
                                if response.hovered() {
                                    self.selected = i;
                                }
                            }
                        });
                });
            });
        }
    }

    /// The dropdown's rows for the current text.
    fn rows(&self, app: &App, keys: &KeyMap, pane: PaneId, recents: &[Intent]) -> Vec<Row> {
        if self.command_mode() {
            let query = self.text[1..].trim();
            let items = palette::items(app, keys, pane, recents);
            return palette::ranked(query, items)
                .into_iter()
                .map(|item: Item| Row {
                    label: item.label,
                    hint: item.shortcut,
                    disabled: item.disabled,
                    pick: item.disabled.is_none().then_some(Pick::Intent(item.intent)),
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
                label: format!("{} {}", crate::icons::for_entry(e), e.name),
                hint: Some(model::type_of(e)),
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
                    label: format!("{} {}", crate::icons::for_entry(entry), entry.name),
                    hint: Some(folder),
                    disabled: None,
                    pick: Some(Pick::Hit(entry.clone())),
                });
            }
            for error in &search.errors {
                rows.push(Row {
                    label: error.clone(),
                    hint: None,
                    disabled: Some("this index did not answer"),
                    pick: None,
                });
            }
            if !search.done {
                rows.push(Row {
                    label: "Searching the index…".to_owned(),
                    hint: None,
                    disabled: Some("hits appear as the index answers"),
                    pick: None,
                });
            }
        }
        if rows.len() > MAX_ROWS {
            rows.truncate(MAX_ROWS);
            rows.push(Row {
                label: "More matches: keep typing".to_owned(),
                hint: None,
                disabled: Some("narrow the search"),
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
    label: String,
    hint: Option<String>,
    disabled: Option<&'static str>,
    pick: Option<Pick>,
}
