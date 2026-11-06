//! The directory model: the single source of truth for everything a window
//! shows, as plain data.
//!
//! A [`Model`] holds the [`Dock`] tree of panes, each [`Pane`]'s tabs, and
//! per [`Tab`] the location with its [`History`], the [`Listing`] with its
//! sort, group, filter and selection, the [`ViewMode`], plus the job table
//! and the [`Settings`] snapshot. It applies listing batches and filesystem
//! changes and derives the visible rows; it performs no I/O. `app` mutates
//! it in response to `Command`s; the UI only reads it.

#![forbid(unsafe_code)]

mod listing;
mod settings;

use std::collections::hash_map::Entry as Slot;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub use ext::PaneId;
use ext::{CellValue, Decoration, FsChange};
pub use listing::{Column, Group, IconSize, Listing, Sort, SortDir, ViewMode, ViewSpec, type_of};
pub use settings::{
    AfterDrag, Density, Drag, PanelSide, Section, Settings, SettingsChange, SidePanel, Theme,
    parse_location,
};
use types::{Entry, EntryId, JobId, Location, Progress};

/// Identity of a tab, unique within one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

/// Where a split puts the new pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    /// New pane to the right.
    Right,
    /// New pane below.
    Down,
}

/// A side of a pane, for dropping a tab next to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Left of the pane.
    Left,
    /// Right of the pane.
    Right,
    /// Above the pane.
    Top,
    /// Below the pane.
    Bottom,
}

/// Where a moved tab goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockTarget {
    /// Into an existing pane's tab strip.
    Into(PaneId),
    /// Into a new pane split off `pane` on `side`.
    Beside {
        /// The pane to split.
        pane: PaneId,
        /// Which side the new pane takes.
        side: Side,
    },
}

/// How a selection changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionChange {
    /// Exactly these.
    Set(Vec<EntryId>),
    /// These as well.
    Add(Vec<EntryId>),
    /// These no longer.
    Remove(Vec<EntryId>),
    /// Flip one.
    Toggle(EntryId),
    /// Every visible row.
    All,
    /// Visible rows that were not selected, and nothing else.
    Invert,
    /// Nothing.
    Clear,
}

/// The tree of panes: `egui_dock`'s splits, as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dock {
    /// A leaf holding one pane.
    Pane(PaneId),
    /// Two subtrees side by side (`Right`) or stacked (`Down`).
    Split {
        /// Which way the split goes.
        dir: SplitDir,
        /// Left or top.
        first: Box<Dock>,
        /// Right or bottom.
        second: Box<Dock>,
    },
}

impl Dock {
    /// Every pane in the tree, left-to-right, top-to-bottom.
    #[must_use]
    pub fn panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<PaneId>) {
        match self {
            Dock::Pane(id) => out.push(*id),
            Dock::Split { first, second, .. } => {
                first.collect(out);
                second.collect(out);
            }
        }
    }

    /// Replaces the leaf `pane` with a split holding `pane` and `new`, `new`
    /// on `side`. Returns whether `pane` was found.
    fn split(&mut self, pane: PaneId, side: Side, new: PaneId) -> bool {
        match self {
            Dock::Pane(id) if *id == pane => {
                let (dir, first, second) = match side {
                    Side::Right => (SplitDir::Right, pane, new),
                    Side::Left => (SplitDir::Right, new, pane),
                    Side::Bottom => (SplitDir::Down, pane, new),
                    Side::Top => (SplitDir::Down, new, pane),
                };
                *self = Dock::Split {
                    dir,
                    first: Box::new(Dock::Pane(first)),
                    second: Box::new(Dock::Pane(second)),
                };
                true
            }
            Dock::Pane(_) => false,
            Dock::Split { first, second, .. } => {
                first.split(pane, side, new) || second.split(pane, side, new)
            }
        }
    }

    /// Removes the leaf `pane`, collapsing its split. Returns `None` when the
    /// tree would become empty.
    fn without(self, pane: PaneId) -> Option<Dock> {
        match self {
            Dock::Pane(id) if id == pane => None,
            leaf @ Dock::Pane(_) => Some(leaf),
            Dock::Split { dir, first, second } => {
                match (first.without(pane), second.without(pane)) {
                    (Some(first), Some(second)) => Some(Dock::Split {
                        dir,
                        first: Box::new(first),
                        second: Box::new(second),
                    }),
                    (Some(rest), None) | (None, Some(rest)) => Some(rest),
                    (None, None) => None,
                }
            }
        }
    }
}

/// A pane: a tab strip inside the dock.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pane {
    tabs: Vec<TabId>,
    active: Option<TabId>,
}

impl Pane {
    /// The tabs in strip order.
    #[must_use]
    pub fn tabs(&self) -> &[TabId] {
        &self.tabs
    }

    /// The tab shown, if the pane has any.
    #[must_use]
    pub fn active(&self) -> Option<TabId> {
        self.active
    }
}

/// One step of a tab's history: where it was, and what Back or Forward
/// restores there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// Where the tab was.
    pub location: Location,
    /// The entries to select again on return; empty for a plain navigation,
    /// the selection at the start of a drag for a spring-loaded one. Whole
    /// entries, so a drag can still name them once the pane has moved on.
    pub selection: Vec<Entry>,
}

impl HistoryEntry {
    /// A step that restores nothing but the location.
    #[must_use]
    pub fn at(location: Location) -> Self {
        Self {
            location,
            selection: Vec::new(),
        }
    }
}

impl PartialEq<Location> for HistoryEntry {
    fn eq(&self, other: &Location) -> bool {
        self.location == *other
    }
}

/// Where a tab has been.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct History {
    /// Steps behind the current one, oldest first.
    pub back: Vec<HistoryEntry>,
    /// Steps ahead, nearest last.
    pub forward: Vec<HistoryEntry>,
}

/// One explorer view: a location and everything shown for it.
#[derive(Debug, Clone)]
pub struct Tab {
    /// Where the tab is.
    pub location: Location,
    /// Back and forward stacks.
    pub history: History,
    /// How the entries are shown.
    pub view: ViewMode,
    /// Sort column and direction.
    pub sort: Sort,
    /// Group headers, if any.
    pub group: Option<Column>,
    /// The type-to-filter text.
    pub filter: String,
    /// Hidden entries shown in this tab.
    pub show_hidden: bool,
    /// Subfolders flattened into the list.
    pub flatten: bool,
    /// The entries and the view over them.
    pub listing: Listing,
    /// Selected entry ids.
    pub selection: BTreeSet<EntryId>,
    /// What decorators said about the entries: badges, tints, cells.
    pub decorations: HashMap<EntryId, Decoration>,
}

impl Tab {
    fn new(location: Location, settings: &Settings) -> Self {
        Self {
            location,
            history: History::default(),
            view: ViewMode::Details,
            sort: Sort {
                by: settings.sort_by,
                dir: settings.sort_dir,
            },
            group: None,
            filter: String::new(),
            show_hidden: settings.show_hidden,
            flatten: false,
            listing: Listing::new(),
            selection: BTreeSet::new(),
            decorations: HashMap::new(),
        }
    }

    /// The view specification this tab's listing follows.
    #[must_use]
    pub fn view_spec<'a>(&'a self, settings: &Settings) -> ViewSpec<'a> {
        ViewSpec {
            sort: self.sort,
            folders_first: settings.folders_first,
            filter: &self.filter,
            show_hidden: self.show_hidden,
            show_system: settings.show_system,
        }
    }

    /// The selected entries, in view order.
    pub fn selected(&self) -> impl Iterator<Item = &Entry> + '_ {
        self.listing
            .rows()
            .filter(|e| self.selection.contains(&e.id))
    }

    fn refresh_view(&mut self, settings: &Settings) {
        let spec = ViewSpec {
            sort: self.sort,
            folders_first: settings.folders_first,
            filter: &self.filter,
            show_hidden: self.show_hidden,
            show_system: settings.show_system,
        };
        self.listing.refresh_view(&spec);
    }
}

/// What the job table knows about one job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobInfo {
    /// The name shown in the operations area.
    pub name: String,
    /// Running with progress, or how it ended.
    pub state: JobState,
}

/// Lifecycle of a job in the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobState {
    /// Still running, with its latest progress.
    Running(Progress),
    /// Finished well.
    Done,
    /// Finished with this error text.
    Failed(String),
    /// Cancelled.
    Cancelled,
}

/// The whole model of one window.
#[derive(Debug, Clone)]
pub struct Model {
    /// The pane tree.
    pub dock: Dock,
    panes: BTreeMap<PaneId, Pane>,
    tabs: BTreeMap<TabId, Tab>,
    /// The pane keyboard input goes to.
    pub focused: PaneId,
    /// Jobs running or recently finished.
    pub jobs: BTreeMap<JobId, JobInfo>,
    /// The settings snapshot.
    pub settings: Settings,
    next_pane: u64,
    next_tab: u64,
}

impl Model {
    /// A model with one pane holding one tab at `start`.
    #[must_use]
    pub fn new(settings: Settings, start: Location) -> Self {
        let pane = PaneId(1);
        let tab = TabId(1);
        let mut panes = BTreeMap::new();
        panes.insert(
            pane,
            Pane {
                tabs: vec![tab],
                active: Some(tab),
            },
        );
        let mut tabs = BTreeMap::new();
        tabs.insert(tab, Tab::new(start, &settings));
        Self {
            dock: Dock::Pane(pane),
            panes,
            tabs,
            focused: pane,
            jobs: BTreeMap::new(),
            settings,
            next_pane: 2,
            next_tab: 2,
        }
    }

    /// The pane with this id.
    #[must_use]
    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.get(&id)
    }

    /// Every pane, by id.
    pub fn panes(&self) -> impl Iterator<Item = (PaneId, &Pane)> + '_ {
        self.panes.iter().map(|(id, pane)| (*id, pane))
    }

    /// The tab with this id.
    #[must_use]
    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.get(&id)
    }

    /// The pane holding this tab.
    #[must_use]
    pub fn pane_of(&self, tab: TabId) -> Option<PaneId> {
        self.panes
            .iter()
            .find(|(_, pane)| pane.tabs.contains(&tab))
            .map(|(id, _)| *id)
    }

    /// The active tab of a pane.
    #[must_use]
    pub fn active_tab(&self, pane: PaneId) -> Option<(TabId, &Tab)> {
        let id = self.panes.get(&pane)?.active?;
        self.tabs.get(&id).map(|tab| (id, tab))
    }

    fn active_tab_mut(&mut self, pane: PaneId) -> Option<(TabId, &mut Tab)> {
        let id = self.panes.get(&pane)?.active?;
        self.tabs.get_mut(&id).map(|tab| (id, tab))
    }

    /// Makes `pane` the focused one. Returns whether it exists.
    pub fn focus(&mut self, pane: PaneId) -> bool {
        if self.panes.contains_key(&pane) {
            self.focused = pane;
            true
        } else {
            false
        }
    }

    /// Makes `tab` the active tab of its pane and focuses that pane.
    pub fn activate(&mut self, tab: TabId) -> bool {
        let Some(pane) = self.pane_of(tab) else {
            return false;
        };
        if let Some(p) = self.panes.get_mut(&pane) {
            p.active = Some(tab);
        }
        self.focused = pane;
        true
    }

    /// Moves the pane's active tab to `to`, pushing the current location on
    /// its back stack and clearing the forward stack, the filter, the listing
    /// and the selection. Returns the tab.
    pub fn navigate(&mut self, pane: PaneId, to: Location) -> Option<TabId> {
        let (id, tab) = self.active_tab_mut(pane)?;
        if tab.location != to {
            let from = std::mem::replace(&mut tab.location, to);
            tab.history.back.push(HistoryEntry::at(from));
            tab.history.forward.clear();
        }
        tab.filter.clear();
        tab.listing.clear();
        tab.selection.clear();
        tab.decorations.clear();
        Some(id)
    }

    /// A spring-loading step of a drag: like [`navigate`](Self::navigate),
    /// but the back stack grows only on the first step, with the selection
    /// of the starting location, so one Back returns there with it. The
    /// later steps replace the location without a trace. Returns the tab.
    pub fn spring_load(&mut self, pane: PaneId, to: Location, first: bool) -> Option<TabId> {
        let (id, tab) = self.active_tab_mut(pane)?;
        if tab.location == to {
            return Some(id);
        }
        let from = std::mem::replace(&mut tab.location, to);
        if first {
            let selection = tab.selected().cloned().collect();
            tab.history.back.push(HistoryEntry {
                location: from,
                selection,
            });
            tab.history.forward.clear();
        }
        tab.filter.clear();
        tab.listing.clear();
        tab.selection.clear();
        tab.decorations.clear();
        Some(id)
    }

    /// Goes back one step in the pane's active tab, restoring the selection
    /// the step recorded. Returns the new location.
    pub fn back(&mut self, pane: PaneId) -> Option<Location> {
        let (_, tab) = self.active_tab_mut(pane)?;
        let to = tab.history.back.pop()?;
        let from = std::mem::replace(&mut tab.location, to.location.clone());
        tab.history.forward.push(HistoryEntry::at(from));
        tab.filter.clear();
        tab.listing.clear();
        tab.selection = to.selection.iter().map(|e| e.id).collect();
        tab.decorations.clear();
        Some(to.location)
    }

    /// Goes forward one step in the pane's active tab. Returns the new
    /// location.
    pub fn forward(&mut self, pane: PaneId) -> Option<Location> {
        let (_, tab) = self.active_tab_mut(pane)?;
        let to = tab.history.forward.pop()?;
        let from = std::mem::replace(&mut tab.location, to.location.clone());
        tab.history.back.push(HistoryEntry::at(from));
        tab.filter.clear();
        tab.listing.clear();
        tab.selection = to.selection.iter().map(|e| e.id).collect();
        tab.decorations.clear();
        Some(to.location)
    }

    /// Changes the sort of the pane's active tab and re-sorts its view.
    pub fn set_sort(&mut self, pane: PaneId, by: Column, dir: SortDir) -> bool {
        let settings = self.settings.clone();
        let Some((_, tab)) = self.active_tab_mut(pane) else {
            return false;
        };
        tab.sort = Sort { by, dir };
        tab.refresh_view(&settings);
        true
    }

    /// Groups the pane's active tab by `by`, or ungroups it.
    pub fn set_group(&mut self, pane: PaneId, by: Option<Column>) -> bool {
        match self.active_tab_mut(pane) {
            Some((_, tab)) => {
                tab.group = by;
                true
            }
            None => false,
        }
    }

    /// Sets the type-to-filter text of the pane's active tab and refreshes
    /// its view. Selected entries that vanish from view stay selected.
    pub fn set_filter(&mut self, pane: PaneId, text: &str) -> bool {
        let settings = self.settings.clone();
        let Some((_, tab)) = self.active_tab_mut(pane) else {
            return false;
        };
        text.clone_into(&mut tab.filter);
        tab.refresh_view(&settings);
        true
    }

    /// Flips hidden entries in the pane's active tab. Returns the new state.
    pub fn toggle_hidden(&mut self, pane: PaneId) -> Option<bool> {
        let settings = self.settings.clone();
        let (_, tab) = self.active_tab_mut(pane)?;
        tab.show_hidden = !tab.show_hidden;
        tab.refresh_view(&settings);
        Some(tab.show_hidden)
    }

    /// Sets whether the pane's active tab flattens subfolders. Returns
    /// whether it changed, in which case the listing must be redone.
    pub fn set_flatten(&mut self, pane: PaneId, on: bool) -> bool {
        match self.active_tab_mut(pane) {
            Some((_, tab)) if tab.flatten != on => {
                tab.flatten = on;
                tab.listing.clear();
                tab.selection.clear();
                true
            }
            _ => false,
        }
    }

    /// Sets the view mode of the pane's active tab.
    pub fn set_view(&mut self, pane: PaneId, mode: ViewMode) -> bool {
        match self.active_tab_mut(pane) {
            Some((_, tab)) => {
                tab.view = mode;
                true
            }
            None => false,
        }
    }

    /// Applies a selection change to the pane's active tab. Returns the new
    /// selection in view order when it changed.
    pub fn select(&mut self, pane: PaneId, change: SelectionChange) -> Option<Vec<EntryId>> {
        let (_, tab) = self.active_tab_mut(pane)?;
        let before = tab.selection.clone();
        match change {
            SelectionChange::Set(ids) => tab.selection = ids.into_iter().collect(),
            SelectionChange::Add(ids) => tab.selection.extend(ids),
            SelectionChange::Remove(ids) => {
                for id in ids {
                    tab.selection.remove(&id);
                }
            }
            SelectionChange::Toggle(id) => {
                if !tab.selection.remove(&id) {
                    tab.selection.insert(id);
                }
            }
            SelectionChange::All => tab.selection = tab.listing.row_ids().into_iter().collect(),
            SelectionChange::Invert => {
                let visible: BTreeSet<EntryId> = tab.listing.row_ids().into_iter().collect();
                tab.selection = visible.difference(&before).copied().collect();
            }
            SelectionChange::Clear => tab.selection.clear(),
        }
        if tab.selection == before {
            return None;
        }
        Some(
            tab.listing
                .row_ids()
                .into_iter()
                .filter(|id| tab.selection.contains(id))
                .collect(),
        )
    }

    /// Opens a tab at `location` in `pane`, or in the focused pane, and makes
    /// it active.
    pub fn new_tab(&mut self, pane: Option<PaneId>, location: Location) -> (PaneId, TabId) {
        let pane = pane
            .filter(|p| self.panes.contains_key(p))
            .unwrap_or(self.focused);
        let id = TabId(self.next_tab);
        self.next_tab += 1;
        self.tabs.insert(id, Tab::new(location, &self.settings));
        let p = self.panes.entry(pane).or_default();
        p.tabs.push(id);
        p.active = Some(id);
        self.focused = pane;
        (pane, id)
    }

    /// Closes a tab. The neighbour becomes active; a pane left empty is
    /// removed from the dock unless it is the last one. Returns the pane the
    /// tab was in.
    pub fn close_tab(&mut self, tab: TabId) -> Option<PaneId> {
        let pane = self.pane_of(tab)?;
        self.tabs.remove(&tab);
        let p = self.panes.get_mut(&pane)?;
        let at = p.tabs.iter().position(|t| *t == tab)?;
        p.tabs.remove(at);
        if p.active == Some(tab) {
            p.active = p.tabs.get(at.min(p.tabs.len().saturating_sub(1))).copied();
        }
        if p.tabs.is_empty() && self.panes.len() > 1 {
            self.panes.remove(&pane);
            if let Some(dock) = self.dock.clone().without(pane) {
                self.dock = dock;
            }
            if self.focused == pane {
                self.focused = self.dock.panes().first().copied().unwrap_or(pane);
            }
        }
        Some(pane)
    }

    /// Splits `pane`, opening the same location in a new pane to the right
    /// or below. Returns the new pane and its tab.
    pub fn split(&mut self, pane: PaneId, dir: SplitDir) -> Option<(PaneId, TabId)> {
        let location = self.active_tab(pane)?.1.location.clone();
        let side = match dir {
            SplitDir::Right => Side::Right,
            SplitDir::Down => Side::Bottom,
        };
        let new = self.add_pane(pane, side)?;
        let (_, tab) = self.new_tab(Some(new), location);
        Some((new, tab))
    }

    fn add_pane(&mut self, beside: PaneId, side: Side) -> Option<PaneId> {
        let new = PaneId(self.next_pane);
        if !self.dock.split(beside, side, new) {
            return None;
        }
        self.next_pane += 1;
        self.panes.insert(new, Pane::default());
        Some(new)
    }

    /// Moves a tab into another pane or into a new pane beside one. An
    /// emptied pane is removed. Returns whether the move happened.
    pub fn move_tab(&mut self, tab: TabId, to: DockTarget) -> bool {
        let Some(from) = self.pane_of(tab) else {
            return false;
        };
        let destination = match to {
            DockTarget::Into(pane) => {
                if !self.panes.contains_key(&pane) {
                    return false;
                }
                pane
            }
            DockTarget::Beside { pane, side } => match self.add_pane(pane, side) {
                Some(new) => new,
                None => return false,
            },
        };
        if destination == from {
            return true;
        }
        if let Some(p) = self.panes.get_mut(&from) {
            p.tabs.retain(|t| *t != tab);
            if p.active == Some(tab) {
                p.active = p.tabs.first().copied();
            }
        }
        let p = self.panes.entry(destination).or_default();
        p.tabs.push(tab);
        p.active = Some(tab);
        self.focused = destination;
        if self.panes.get(&from).is_some_and(|p| p.tabs.is_empty()) && self.panes.len() > 1 {
            self.panes.remove(&from);
            if let Some(dock) = self.dock.clone().without(from) {
                self.dock = dock;
            }
        }
        true
    }

    /// Replaces the settings and re-derives every tab's view.
    pub fn apply_settings(&mut self, settings: Settings) {
        self.settings = settings;
        let settings = self.settings.clone();
        for tab in self.tabs.values_mut() {
            tab.refresh_view(&settings);
        }
    }

    /// Records a batch of the pane's active tab's listing. Returns whether
    /// a tab took it.
    pub fn listing_batch(&mut self, pane: PaneId, batch: Vec<Entry>) -> bool {
        let settings = self.settings.clone();
        let Some((_, tab)) = self.active_tab_mut(pane) else {
            return false;
        };
        let spec = ViewSpec {
            sort: tab.sort,
            folders_first: settings.folders_first,
            filter: &tab.filter,
            show_hidden: tab.show_hidden,
            show_system: settings.show_system,
        };
        tab.listing.extend(batch, &spec);
        true
    }

    /// Empties the pane's active tab's listing before a fresh one streams
    /// in; the selection and filter stay. Returns whether a tab took it.
    pub fn clear_listing(&mut self, pane: PaneId) -> bool {
        match self.active_tab_mut(pane) {
            Some((_, tab)) => {
                tab.listing.clear();
                true
            }
            None => false,
        }
    }

    /// Replaces the pane's active tab's listing in one step, so a refresh
    /// never shows an empty list in between. Selected entries that are gone
    /// leave the selection. Returns whether a tab took it.
    pub fn replace_listing(&mut self, pane: PaneId, entries: Vec<Entry>) -> bool {
        let settings = self.settings.clone();
        let Some((_, tab)) = self.active_tab_mut(pane) else {
            return false;
        };
        let spec = ViewSpec {
            sort: tab.sort,
            folders_first: settings.folders_first,
            filter: &tab.filter,
            show_hidden: tab.show_hidden,
            show_system: settings.show_system,
        };
        tab.listing.clear();
        tab.listing.extend(entries, &spec);
        tab.listing.complete = true;
        let listing = &tab.listing;
        tab.selection.retain(|id| listing.get(*id).is_some());
        true
    }

    /// Records what a decorator said about entries of the pane's active tab.
    /// Returns whether a tab took it.
    ///
    /// A later decoration replaces the badge, the tint and the cells of the
    /// columns it names; cells of other columns stay. A decoration that is
    /// still [`CellValue::Pending`] only adds what was not there, so the
    /// badges do not blink while a decorator looks again.
    pub fn decorate(&mut self, pane: PaneId, items: Vec<(EntryId, Decoration)>) -> bool {
        match self.active_tab_mut(pane) {
            Some((_, tab)) => {
                for (id, decoration) in items {
                    match tab.decorations.entry(id) {
                        Slot::Vacant(slot) => {
                            slot.insert(decoration);
                        }
                        Slot::Occupied(mut slot) => merge(slot.get_mut(), decoration),
                    }
                }
                true
            }
            None => false,
        }
    }

    /// Marks the pane's active tab's listing complete.
    pub fn listing_done(&mut self, pane: PaneId) -> bool {
        match self.active_tab_mut(pane) {
            Some((_, tab)) => {
                tab.listing.complete = true;
                true
            }
            None => false,
        }
    }

    /// Applies a change notification to the pane's active tab. Returns
    /// `false` for [`FsChange::Rescan`], which needs a fresh listing.
    pub fn apply_change(&mut self, pane: PaneId, change: FsChange) -> bool {
        let settings = self.settings.clone();
        let Some((_, tab)) = self.active_tab_mut(pane) else {
            return true;
        };
        let spec = ViewSpec {
            sort: tab.sort,
            folders_first: settings.folders_first,
            filter: &tab.filter,
            show_hidden: tab.show_hidden,
            show_system: settings.show_system,
        };
        match change {
            FsChange::Created(entry) | FsChange::Modified(entry) => {
                tab.listing.upsert(entry, &spec);
            }
            FsChange::Removed(id) => {
                tab.listing.remove(id);
                tab.selection.remove(&id);
                tab.decorations.remove(&id);
            }
            FsChange::Renamed { from, to } => {
                tab.listing.remove(from);
                if tab.selection.remove(&from) {
                    tab.selection.insert(to.id);
                }
                tab.listing.upsert(to, &spec);
            }
            FsChange::Rescan => return false,
        }
        true
    }

    /// Records a job.
    pub fn job_started(&mut self, id: JobId, name: String) {
        self.jobs.insert(
            id,
            JobInfo {
                name,
                state: JobState::Running(Progress::Items {
                    done: 0,
                    total: None,
                }),
            },
        );
    }

    /// Records a job's progress; unknown jobs are added without a name.
    pub fn job_progress(&mut self, id: JobId, progress: Progress) {
        let info = self.jobs.entry(id).or_insert_with(|| JobInfo {
            name: String::new(),
            state: JobState::Running(progress.clone()),
        });
        if matches!(info.state, JobState::Running(_)) {
            info.state = JobState::Running(progress);
        }
    }

    /// Records how a job ended.
    pub fn job_finished(&mut self, id: JobId, state: JobState) {
        let info = self.jobs.entry(id).or_insert_with(|| JobInfo {
            name: String::new(),
            state: JobState::Done,
        });
        info.state = state;
    }

    /// Forgets a finished job.
    pub fn job_dismiss(&mut self, id: JobId) -> bool {
        self.jobs.remove(&id).is_some()
    }
}

/// Folds `new` into `old`; see [`Model::decorate`].
fn merge(old: &mut Decoration, new: Decoration) {
    let pending = new
        .cells
        .iter()
        .any(|(_, value)| matches!(value, CellValue::Pending));
    if !pending {
        old.badge = new.badge;
        old.tint = new.tint;
    }
    for (column, value) in new.cells {
        match old.cells.iter_mut().find(|(c, _)| *c == column) {
            Some(slot) => {
                if !matches!(value, CellValue::Pending) {
                    slot.1 = value;
                }
            }
            None => old.cells.push((column, value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Attrs, EntryKind, ProviderId, VPath};

    fn loc(path: &str) -> Location {
        Location::new(ProviderId("t"), path)
    }

    fn active(model: &Model, pane: PaneId) -> &Tab {
        model
            .active_tab(pane)
            .map_or_else(|| panic!("no active tab"), |(_, tab)| tab)
    }

    fn entry(id: u128, name: &str) -> Entry {
        Entry {
            id: EntryId::new(ProviderId("t"), id),
            name: name.to_owned(),
            path: VPath::from(format!("/{name}")),
            kind: EntryKind::File,
            size: Some(1),
            created: None,
            modified: None,
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        }
    }

    #[test]
    fn navigation_keeps_history_and_resets_the_view_state() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        let pane = model.focused;
        model.set_filter(pane, "x");
        assert_eq!(model.navigate(pane, loc("/b")), Some(TabId(1)));
        model.navigate(pane, loc("/c"));
        let tab = active(&model, pane);
        assert_eq!(tab.location, loc("/c"));
        assert_eq!(tab.history.back, [loc("/a"), loc("/b")]);
        assert_eq!(tab.filter, "");

        assert_eq!(model.back(pane), Some(loc("/b")));
        assert_eq!(model.back(pane), Some(loc("/a")));
        assert_eq!(model.back(pane), None);
        assert_eq!(model.forward(pane), Some(loc("/b")));
        model.navigate(pane, loc("/d"));
        assert_eq!(model.forward(pane), None, "navigating clears forward");
        let tab = active(&model, pane);
        assert_eq!(tab.history.back, [loc("/a"), loc("/b")]);
    }

    #[test]
    fn spring_loading_records_one_step_and_back_restores_the_selection() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        let pane = model.focused;
        model.listing_batch(pane, vec![entry(1, "x"), entry(2, "y")]);
        let picked = EntryId::new(ProviderId("t"), 2);
        model.select(pane, SelectionChange::Set(vec![picked]));

        assert_eq!(model.spring_load(pane, loc("/a/sub"), true), Some(TabId(1)));
        assert_eq!(
            model.spring_load(pane, loc("/a/sub/deeper"), false),
            Some(TabId(1))
        );
        let tab = active(&model, pane);
        assert_eq!(tab.location, loc("/a/sub/deeper"));
        assert_eq!(tab.history.back.len(), 1, "one step for the whole drag");
        assert_eq!(tab.history.back[0].location, loc("/a"));
        assert_eq!(tab.history.back[0].selection, [entry(2, "y")]);
        assert!(tab.selection.is_empty());

        assert_eq!(model.back(pane), Some(loc("/a")));
        let tab = active(&model, pane);
        assert!(tab.selection.contains(&picked), "the selection is back");
        assert_eq!(tab.history.forward, [loc("/a/sub/deeper")]);
        assert_eq!(model.forward(pane), Some(loc("/a/sub/deeper")));
        assert!(active(&model, pane).selection.is_empty());
    }

    #[test]
    fn listing_batches_selection_and_changes() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        let pane = model.focused;
        assert!(model.listing_batch(pane, vec![entry(1, "b"), entry(2, "a")]));
        assert!(model.listing_done(pane));
        let ids = model.select(pane, SelectionChange::All).unwrap_or_default();
        assert_eq!(
            ids,
            [
                EntryId::new(ProviderId("t"), 2),
                EntryId::new(ProviderId("t"), 1)
            ]
        );
        assert_eq!(model.select(pane, SelectionChange::All), None, "unchanged");
        let inverted = model.select(pane, SelectionChange::Invert);
        assert_eq!(inverted, Some(Vec::new()));

        assert!(model.apply_change(pane, FsChange::Created(entry(3, "c"))));
        let renamed = Entry {
            name: "z".into(),
            ..entry(3, "c")
        };
        model.select(
            pane,
            SelectionChange::Set(vec![EntryId::new(ProviderId("t"), 3)]),
        );
        assert!(model.apply_change(
            pane,
            FsChange::Renamed {
                from: EntryId::new(ProviderId("t"), 3),
                to: renamed
            }
        ));
        let tab = active(&model, pane);
        assert_eq!(
            tab.listing
                .rows()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "z"]
        );
        assert!(tab.selection.contains(&EntryId::new(ProviderId("t"), 3)));
        assert!(!model.apply_change(pane, FsChange::Rescan));
        assert!(model.apply_change(pane, FsChange::Removed(EntryId::new(ProviderId("t"), 3))));
        let tab = active(&model, pane);
        assert!(tab.selection.is_empty());
    }

    #[test]
    fn tabs_split_move_and_close_through_the_dock() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        let pane = model.focused;
        let (right, right_tab) = model
            .split(pane, SplitDir::Right)
            .unwrap_or_else(|| panic!("split"));
        assert_eq!(model.dock.panes(), [pane, right]);
        assert_eq!(model.focused, right);
        assert_eq!(
            model.tab(right_tab).map(|t| t.location.clone()),
            Some(loc("/a"))
        );

        let (_, extra) = model.new_tab(Some(pane), loc("/b"));
        assert_eq!(model.pane(pane).map(|p| p.tabs().len()), Some(2));
        assert!(model.move_tab(extra, DockTarget::Into(right)));
        assert_eq!(model.pane(right).map(Pane::active), Some(Some(extra)));
        assert!(model.move_tab(
            extra,
            DockTarget::Beside {
                pane,
                side: Side::Top
            }
        ));
        assert_eq!(model.dock.panes().len(), 3);

        assert_eq!(model.close_tab(right_tab), Some(right));
        assert_eq!(model.dock.panes().len(), 2, "the emptied pane is gone");
        assert!(model.pane(right).is_none());
        assert!(model.close_tab(extra).is_some());
        assert_eq!(model.dock, Dock::Pane(pane));
        assert_eq!(model.close_tab(TabId(1)), Some(pane));
        assert_eq!(
            model.pane(pane).map(|p| p.tabs().len()),
            Some(0),
            "the last pane stays"
        );
        assert!(!model.move_tab(TabId(99), DockTarget::Into(pane)));
    }

    #[test]
    fn jobs_and_settings() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        model.job_started(JobId(1), "copy".into());
        model.job_progress(
            JobId(1),
            Progress::Bytes {
                done: 1,
                total: Some(2),
            },
        );
        model.job_finished(JobId(1), JobState::Done);
        model.job_progress(
            JobId(1),
            Progress::Bytes {
                done: 2,
                total: Some(2),
            },
        );
        assert_eq!(
            model.jobs.get(&JobId(1)).map(|j| j.state.clone()),
            Some(JobState::Done)
        );
        assert!(model.job_dismiss(JobId(1)));
        assert!(!model.job_dismiss(JobId(1)));

        let pane = model.focused;
        model.listing_batch(
            pane,
            vec![
                entry(1, "file"),
                Entry {
                    kind: EntryKind::Dir,
                    ..entry(2, "zdir")
                },
            ],
        );
        let names = |m: &Model| {
            m.active_tab(pane)
                .map(|(_, t)| t.listing.rows().map(|e| e.name.clone()).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        assert_eq!(names(&model), ["zdir", "file"]);
        model.apply_settings(Settings {
            folders_first: false,
            ..Settings::default()
        });
        assert_eq!(names(&model), ["file", "zdir"]);
        assert_eq!(model.toggle_hidden(pane), Some(true));
        assert!(model.set_flatten(pane, true));
        assert!(!model.set_flatten(pane, true));
        assert!(model.set_view(pane, ViewMode::Icons(IconSize::Large)));
        assert!(model.set_group(pane, Some(Column::Type)));
    }
}
