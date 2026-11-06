//! The command palette: every action and UI intent, fuzzy-matched, recently
//! used first.

use std::sync::Arc;

use app::App;
use ext::{Action, Availability, Invocation};
use model::PaneId;

use crate::keys::{Intent, KeyMap, UI_INTENTS};

/// The palette's state.
#[derive(Debug, Default)]
pub struct Palette {
    /// Whether it is shown.
    pub open: bool,
    /// The search text.
    pub query: String,
    /// The highlighted row.
    pub selected: usize,
    /// Focus the search box on the next frame.
    focus: bool,
}

/// One row of the palette.
#[derive(Debug, Clone)]
pub struct Item {
    /// What Enter does.
    pub intent: Intent,
    /// The text.
    pub label: String,
    /// The binding, if any.
    pub shortcut: Option<String>,
    /// Greyed out with this reason when it cannot run now.
    pub disabled: Option<&'static str>,
    /// Lower is more recently used; `None` never used.
    pub recency: Option<usize>,
}

impl Palette {
    /// Opens with an empty query.
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.focus = true;
    }

    /// Everything the palette offers for `pane`, unsorted.
    #[must_use]
    pub fn items(app: &App, keys: &KeyMap, pane: PaneId, recents: &[Intent]) -> Vec<Item> {
        let ctx = app.action_ctx(pane, Invocation::Palette);
        let recency = |intent: Intent| recents.iter().position(|r| *r == intent);
        let mut items: Vec<Item> = app
            .registry()
            .actions()
            .iter()
            .filter_map(|action: &Arc<dyn Action>| {
                let intent = Intent::Action(action.id());
                let disabled = match ctx.as_ref().map(|c| action.availability(c)) {
                    Some(Availability::Enabled) => None,
                    Some(Availability::Disabled(why)) => Some(why),
                    Some(Availability::Hidden) | None => return None,
                };
                Some(Item {
                    intent,
                    label: action.label().to_owned(),
                    shortcut: keys.text_for(intent),
                    disabled,
                    recency: recency(intent),
                })
            })
            .collect();
        items.extend(
            UI_INTENTS
                .iter()
                .filter(|spec| spec.group != "List")
                .map(|spec| Item {
                    intent: spec.intent,
                    label: spec.label.to_owned(),
                    shortcut: keys.text_for(spec.intent),
                    disabled: None,
                    recency: recency(spec.intent),
                }),
        );
        items
    }

    /// The rows matching the query, best first: by score, then recency,
    /// then label.
    #[must_use]
    pub fn ranked(&self, mut items: Vec<Item>) -> Vec<Item> {
        let query = self.query.trim().to_lowercase();
        let mut scored: Vec<(u32, Item)> = items
            .drain(..)
            .filter_map(|item| score(&query, &item.label).map(|s| (s, item)))
            .collect();
        scored.sort_by(|(sa, a), (sb, b)| {
            sb.cmp(sa)
                .then_with(|| match (a.recency, b.recency) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
                .then_with(|| a.label.cmp(&b.label))
        });
        scored.into_iter().map(|(_, item)| item).collect()
    }

    /// Draws the palette; returns the intent to run when the user picked one.
    pub fn show(&mut self, ctx: &egui::Context, rows: &[Item]) -> Option<Intent> {
        if !self.open {
            return None;
        }
        let mut picked = None;
        let (up, down, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            )
        });
        if escape {
            self.open = false;
            return None;
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        if down && self.selected + 1 < rows.len() {
            self.selected += 1;
        }
        self.selected = self.selected.min(rows.len().saturating_sub(1));
        if enter
            && let Some(item) = rows.get(self.selected)
            && item.disabled.is_none()
        {
            picked = Some(item.intent);
            self.open = false;
        }

        let modal = egui::Modal::new(egui::Id::new("command-palette")).show(ctx, |ui| {
            ui.set_width(480.0);
            let edit = egui::TextEdit::singleline(&mut self.query)
                .hint_text("Type a command")
                .desired_width(f32::INFINITY)
                .show(ui);
            if self.focus {
                edit.response.request_focus();
                self.focus = false;
            }
            if edit.response.changed() {
                self.selected = 0;
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .show(ui, |ui| {
                    if rows.is_empty() {
                        ui.weak("No matching command");
                    }
                    for (i, item) in rows.iter().enumerate() {
                        let selected = i == self.selected;
                        let text = egui::RichText::new(&item.label);
                        let text = if item.disabled.is_some() {
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
                                        if let Some(shortcut) = &item.shortcut {
                                            ui.weak(shortcut);
                                        }
                                    },
                                );
                                r
                            })
                            .inner;
                        if selected {
                            response.scroll_to_me(None);
                        }
                        if let Some(why) = item.disabled {
                            response.clone().on_hover_text(why);
                        }
                        if response.clicked() && item.disabled.is_none() {
                            picked = Some(item.intent);
                            self.open = false;
                        }
                        if response.hovered() {
                            self.selected = i;
                        }
                    }
                });
        });
        if modal.should_close() {
            self.open = false;
        }
        picked
    }
}

/// How well `query` matches `label` as a case-insensitive subsequence;
/// `None` when it does not. Word starts and runs score higher.
#[must_use]
pub fn score(query: &str, label: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let label_lower = label.to_lowercase();
    let mut score = 0u32;
    let mut pos = 0usize;
    let mut last_match: Option<usize> = None;
    for q in query.chars() {
        let rest = &label_lower[pos..];
        let found = rest.char_indices().find(|(_, c)| *c == q)?.0;
        let at = pos + found;
        score += 10;
        if at == 0 || label_lower[..at].ends_with(' ') {
            score += 15;
        }
        if last_match.is_some_and(|l| l + 1 == at) {
            score += 10;
        }
        last_match = Some(at);
        pos = at + q.len_utf8();
    }
    if label_lower.starts_with(query) {
        score += 30;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_scores_prefer_prefixes_and_word_starts() {
        assert!(score("", "Anything").is_some());
        assert_eq!(score("xyz", "Copy path"), None);
        let prefix = score("cop", "Copy").unwrap_or(0);
        let scattered = score("cop", "Clear grouping of pane").unwrap_or(0);
        assert!(prefix > scattered, "{prefix} vs {scattered}");
        let words = score("nf", "New folder").unwrap_or(0);
        let inner = score("nf", "Inform").unwrap_or(0);
        assert!(words > inner, "{words} vs {inner}");
    }

    #[test]
    fn ranking_puts_recent_items_first_when_the_query_is_empty() {
        let mut palette = Palette::default();
        let item = |label: &str, recency| Item {
            intent: Intent::Refresh,
            label: label.to_owned(),
            shortcut: None,
            disabled: None,
            recency,
        };
        let ranked = palette.ranked(vec![
            item("Zed", None),
            item("Alpha", Some(1)),
            item("Beta", Some(0)),
        ]);
        let labels: Vec<&str> = ranked.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Beta", "Alpha", "Zed"]);
        palette.query = "ze".into();
        let ranked = palette.ranked(vec![item("Zed", None), item("Alpha", Some(1))]);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].label, "Zed");
    }
}
