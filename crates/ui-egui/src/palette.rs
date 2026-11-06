//! The command list behind the search box's command mode: every action and
//! UI intent, fuzzy-matched, recently used first.

use std::sync::Arc;

use app::App;
use ext::{Action, Availability, Invocation};
use model::PaneId;

use crate::keys::{Intent, KeyMap, UI_INTENTS};

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

/// Everything the command mode offers for `pane`, unsorted.
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

/// The rows matching `query`, best first: by score, then recency, then
/// label.
#[must_use]
pub fn ranked(query: &str, mut items: Vec<Item>) -> Vec<Item> {
    let query = query.trim().to_lowercase();
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
        let item = |label: &str, recency| Item {
            intent: Intent::Refresh,
            label: label.to_owned(),
            shortcut: None,
            disabled: None,
            recency,
        };
        let by_recency = ranked(
            "",
            vec![
                item("Zed", None),
                item("Alpha", Some(1)),
                item("Beta", Some(0)),
            ],
        );
        let labels: Vec<&str> = by_recency.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Beta", "Alpha", "Zed"]);
        let by_query = ranked("ze", vec![item("Zed", None), item("Alpha", Some(1))]);
        assert_eq!(by_query.len(), 1);
        assert_eq!(by_query[0].label, "Zed");
    }
}
