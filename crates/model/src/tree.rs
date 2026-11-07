//! Tree state: the expandable folders of a tab in Tree view, and the side
//! panel's folder tree.

use std::collections::{BTreeSet, HashMap};

use types::{Entry, EntryId, EntryKind, Location, VPath};

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

/// The side panel's folder tree: folders only, lazily listed from the root
/// of the focused pane's volume, following the pane's location unless
/// unlinked.
#[derive(Debug, Clone, Default)]
pub struct FolderTree {
    /// The tree's root, the top of the current volume or provider.
    pub root: Option<Location>,
    /// The folders listed so far, by path.
    nodes: HashMap<VPath, Vec<Entry>>,
    /// Folders whose listing is running.
    pub loading: BTreeSet<VPath>,
    /// Folders shown open.
    pub expanded: BTreeSet<VPath>,
    /// Whether the tree follows the focused pane's location.
    pub linked: bool,
}

impl FolderTree {
    pub(crate) fn new() -> Self {
        Self {
            linked: true,
            ..Self::default()
        }
    }

    /// The folders directly under `path`, in name order, once listed.
    #[must_use]
    pub fn children(&self, path: &VPath) -> Option<&[Entry]> {
        self.nodes.get(path).map(Vec::as_slice)
    }

    /// Whether `path` has been listed.
    #[must_use]
    pub fn is_listed(&self, path: &VPath) -> bool {
        self.nodes.contains_key(path)
    }

    /// Records the folders under `path`.
    pub(crate) fn set_children(&mut self, path: VPath, mut folders: Vec<Entry>) {
        folders.retain(|e| e.kind == EntryKind::Dir);
        folders.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.name.cmp(&b.name))
        });
        self.loading.remove(&path);
        self.nodes.insert(path, folders);
    }

    /// Points the tree at a new root, forgetting what it listed.
    pub(crate) fn set_root(&mut self, root: Location) {
        if self.root.as_ref() == Some(&root) {
            return;
        }
        self.root = Some(root.clone());
        self.nodes.clear();
        self.loading.clear();
        self.expanded.clear();
        self.expanded.insert(root.path);
    }

    /// Opens every folder from the root down to `path`, so the path's
    /// folder is visible. Returns the folders that still need listing.
    pub(crate) fn reveal(&mut self, path: &VPath) -> Vec<VPath> {
        let Some(root) = &self.root else {
            return Vec::new();
        };
        let mut chain = Vec::new();
        let mut current = Some(path.clone());
        while let Some(p) = current {
            let at_root = p == root.path;
            chain.push(p.clone());
            if at_root {
                break;
            }
            current = p.parent();
        }
        // Everything above the folder itself opens; the folder shows closed.
        let mut needed = Vec::new();
        for (i, p) in chain.iter().rev().enumerate() {
            let is_last = i + 1 == chain.len();
            if !is_last {
                self.expanded.insert(p.clone());
            }
            if !self.nodes.contains_key(p) && !self.loading.contains(p) {
                self.loading.insert(p.clone());
                needed.push(p.clone());
            }
        }
        needed
    }

    /// Opens or closes one folder. Returns whether it needs listing.
    pub(crate) fn toggle(&mut self, path: &VPath) -> bool {
        if self.expanded.remove(path) {
            return false;
        }
        self.expanded.insert(path.clone());
        if self.nodes.contains_key(path) || self.loading.contains(path) {
            return false;
        }
        self.loading.insert(path.clone());
        true
    }
}
