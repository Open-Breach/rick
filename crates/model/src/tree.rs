//! Tree state: the expandable folders of a tab in Tree view.

use std::collections::{BTreeSet, HashMap};

use types::{Entry, EntryId, EntryKind};

use crate::listing::{Listing, ViewSpec};

/// One row of the tree view: an entry and how deep it sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeRow {
    /// The entry.
    pub id: EntryId,
    /// 0 for the tab's own listing, 1 for the children of an expanded
    /// folder, and so on.
    pub depth: u16,
}

/// The expandable folders of a tab shown as a tree: which are expanded,
/// their children while they are, and the flattened rows the list draws.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    /// Folders expanded in place. Survives a refresh; cleared by navigation.
    pub expanded: BTreeSet<EntryId>,
    /// The children of each expanded folder, cached while it is expanded.
    children: HashMap<EntryId, Listing>,
    /// The rows in display order, rebuilt when anything above changes.
    rows: Vec<TreeRow>,
}

impl Tree {
    /// Forgets everything.
    pub(crate) fn clear(&mut self) {
        self.expanded.clear();
        self.children.clear();
        self.rows.clear();
    }

    /// Drops the cached children but keeps the expanded set, so a refresh
    /// lists them again.
    pub(crate) fn clear_children(&mut self) {
        self.children.clear();
        self.rows.clear();
    }

    /// The rows in display order.
    #[must_use]
    pub fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    /// The children of an expanded folder, if listed.
    #[must_use]
    pub fn children(&self, folder: EntryId) -> Option<&Listing> {
        self.children.get(&folder)
    }

    /// Whether an expanded folder's children are still streaming in.
    #[must_use]
    pub fn is_loading(&self, folder: EntryId) -> bool {
        self.expanded.contains(&folder) && !self.children.get(&folder).is_some_and(|l| l.complete)
    }

    /// The entry with this id in any child listing.
    #[must_use]
    pub fn child(&self, id: EntryId) -> Option<&Entry> {
        self.children.values().find_map(|l| l.get(id))
    }

    pub(crate) fn expand(&mut self, folder: EntryId) -> bool {
        if !self.expanded.insert(folder) {
            return false;
        }
        self.children.entry(folder).or_default();
        true
    }

    pub(crate) fn collapse(&mut self, folder: EntryId) -> bool {
        if !self.expanded.remove(&folder) {
            return false;
        }
        self.children.remove(&folder);
        // Folders expanded underneath go with it.
        let orphaned: Vec<EntryId> = self
            .expanded
            .iter()
            .copied()
            .filter(|id| !self.reachable(*id))
            .collect();
        for id in orphaned {
            self.expanded.remove(&id);
            self.children.remove(&id);
        }
        true
    }

    /// Whether `id` is listed by some other expanded folder; the root's own
    /// entries are checked by the caller.
    fn reachable(&self, id: EntryId) -> bool {
        self.children.values().any(|l| l.get(id).is_some())
    }

    /// The listing of an expanded folder, created on first use; `None` for
    /// a folder that is not expanded.
    fn listing_of(&mut self, folder: EntryId) -> Option<&mut Listing> {
        if !self.expanded.contains(&folder) {
            return None;
        }
        Some(self.children.entry(folder).or_default())
    }

    pub(crate) fn extend(
        &mut self,
        folder: EntryId,
        batch: Vec<Entry>,
        spec: &ViewSpec<'_>,
    ) -> bool {
        match self.listing_of(folder) {
            Some(listing) => {
                listing.extend(batch, spec);
                true
            }
            None => false,
        }
    }

    pub(crate) fn replace(
        &mut self,
        folder: EntryId,
        entries: Vec<Entry>,
        spec: &ViewSpec<'_>,
    ) -> bool {
        match self.listing_of(folder) {
            Some(listing) => {
                listing.clear();
                listing.extend(entries, spec);
                listing.complete = true;
                true
            }
            None => false,
        }
    }

    pub(crate) fn done(&mut self, folder: EntryId) -> bool {
        match self.listing_of(folder) {
            Some(listing) => {
                listing.complete = true;
                true
            }
            None => false,
        }
    }

    pub(crate) fn refresh_views(&mut self, spec: &ViewSpec<'_>) {
        for listing in self.children.values_mut() {
            listing.refresh_view(spec);
        }
    }

    /// Rebuilds the rows from the root listing and the expanded children.
    pub(crate) fn rebuild(&mut self, root: &Listing) {
        let mut rows = Vec::with_capacity(root.row_count());
        self.push_rows(root, 0, &mut rows);
        self.rows = rows;
    }

    fn push_rows(&self, listing: &Listing, depth: u16, rows: &mut Vec<TreeRow>) {
        for entry in listing.rows() {
            rows.push(TreeRow {
                id: entry.id,
                depth,
            });
            if entry.kind == EntryKind::Dir
                && self.expanded.contains(&entry.id)
                && let Some(children) = self.children.get(&entry.id)
                && depth < 64
            {
                self.push_rows(children, depth + 1, rows);
            }
        }
    }
}
