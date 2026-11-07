//! The side panel's data: the recent entries and the trash's contents.

use std::time::SystemTime;

use platform::TrashedItem;
use types::{Entry, EntryId, VPath};

/// How many recent entries are kept.
pub const RECENT_LIMIT: usize = 200;

/// Files and folders changed recently, newest first.
#[derive(Debug, Clone, Default)]
pub struct Recent {
    entries: Vec<Entry>,
}

impl Recent {
    /// The entries, newest first.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Whether nothing is recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Records a change: the entry takes the place its modification time
    /// gives it (now, when it has none), replacing an older record of the
    /// same path or id; the list is cut to [`RECENT_LIMIT`]. Returns whether
    /// anything changed.
    pub fn touched(&mut self, mut entry: Entry) -> bool {
        if entry.modified.is_none() {
            entry.modified = Some(SystemTime::now());
        }
        if self
            .entries
            .first()
            .is_some_and(|first| first.path == entry.path && first.modified == entry.modified)
        {
            return false;
        }
        self.entries
            .retain(|e| e.path != entry.path && e.id != entry.id);
        let at = self
            .entries
            .partition_point(|e| e.modified > entry.modified);
        self.entries.insert(at, entry);
        self.entries.truncate(RECENT_LIMIT);
        true
    }

    /// Forgets an entry that was removed. Returns whether it was listed.
    pub fn removed(&mut self, id: EntryId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        before != self.entries.len()
    }

    /// Forgets every entry at or under `path`.
    pub fn removed_path(&mut self, path: &VPath) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|e| e.path != *path && !e.path.is_inside(path));
        before != self.entries.len()
    }

    /// Merges a scan's results in, keeping the newest 200 of both.
    pub fn merge(&mut self, found: Vec<Entry>) {
        for entry in found {
            self.touched(entry);
        }
    }
}

/// The trash as the side panel shows it.
#[derive(Debug, Clone, Default)]
pub struct TrashView {
    /// The items, newest first.
    pub items: Vec<TrashedItem>,
    /// `true` when the platform listed its trash; `false` when the items are
    /// Rick's own record of what it trashed, because the platform cannot
    /// list original paths (macOS).
    pub listed: bool,
    /// The platform's trash could not be read at all; the text says why.
    pub error: Option<String>,
}

impl TrashView {
    /// Replaces the items, newest first.
    pub fn set(&mut self, mut items: Vec<TrashedItem>, listed: bool) {
        items.sort_by_key(|item| std::cmp::Reverse(item.deleted_at));
        self.items = items;
        self.listed = listed;
        self.error = None;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use types::{Attrs, EntryKind, ProviderId};

    use super::*;

    fn entry(id: u128, path: &str, secs: u64) -> Entry {
        Entry {
            id: EntryId::new(ProviderId("local"), id),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            path: VPath::from(path),
            kind: EntryKind::File,
            size: Some(1),
            created: None,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)),
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        }
    }

    #[test]
    fn recent_keeps_the_newest_first_without_duplicates() {
        let mut recent = Recent::default();
        assert!(recent.touched(entry(1, "/a", 10)));
        assert!(recent.touched(entry(2, "/b", 30)));
        assert!(recent.touched(entry(3, "/c", 20)));
        let order: Vec<&str> = recent.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(order, ["b", "c", "a"]);
        // The same path again, newer: one row, at the top.
        assert!(recent.touched(entry(1, "/a", 40)));
        let order: Vec<&str> = recent.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(order, ["a", "b", "c"]);
        assert!(!recent.touched(entry(1, "/a", 40)), "unchanged");
        assert!(recent.removed(EntryId::new(ProviderId("local"), 2)));
        assert!(!recent.removed(EntryId::new(ProviderId("local"), 2)));
        recent.merge(vec![entry(9, "/sub/x", 5), entry(10, "/sub/y", 50)]);
        assert_eq!(recent.entries()[0].name, "y");
        assert!(recent.removed_path(&VPath::from("/sub")));
        assert_eq!(recent.entries().len(), 2);
        for i in 0..300u64 {
            recent.touched(entry(1000 + u128::from(i), &format!("/many/{i}"), 100 + i));
        }
        assert_eq!(recent.entries().len(), RECENT_LIMIT);
        assert_eq!(recent.entries()[0].name, "299");
    }

    #[test]
    fn the_trash_view_sorts_newest_first() {
        let mut view = TrashView::default();
        let item = |n: u64| TrashedItem {
            original: std::path::PathBuf::from(format!("/o/{n}")),
            trashed: std::path::PathBuf::from(format!("/t/{n}")),
            deleted_at: SystemTime::UNIX_EPOCH + Duration::from_secs(n),
        };
        view.set(vec![item(1), item(3), item(2)], true);
        let order: Vec<_> = view.items.iter().map(|i| i.deleted_at).collect();
        assert_eq!(
            order,
            [item(3).deleted_at, item(2).deleted_at, item(1).deleted_at]
        );
        assert!(view.listed);
    }
}
