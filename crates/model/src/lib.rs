//! The directory model: the single source of truth for everything a window
//! shows, as plain data.
//!
//! A [`Model`] holds every [`Window`] with its [`Dock`] tree of panes, each
//! [`Pane`]'s tabs, and per [`Tab`] the location with its [`History`], the
//! [`Listing`] with its sort, group, filter and selection, the [`ViewMode`]
//! with the [`Tree`] state of expanded folders and the scroll offset, plus
//! the job table, the side panel's data (the [`Places`], the volumes, the
//! cloud and network rows, the [`TrashView`]), the name conflicts jobs wait on, the
//! [`Settings`] snapshot and the [`Integration`] items as the last check
//! left them. All windows share the one model, so a job
//! started in one shows in every other. It applies listing batches and
//! filesystem changes and derives the visible rows; it performs no I/O.
//! `app` mutates it in response to `Command`s; the UI only reads it.
//! [`Model::session`] turns it into the [`Session`] that `session.toml` and
//! the saved layouts hold.

#![forbid(unsafe_code)]

mod integration;
mod listing;
mod panel;
mod session;
mod settings;
mod tree;

use std::collections::hash_map::Entry as Slot;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub use ext::PaneId;
use ext::{CellValue, Decoration, FsChange};
pub use integration::{
    Applying, Integration, IntegrationItem, IntegrationSettings, ItemRecord, ItemState, ItemStatus,
    day_of, decline, fold_check, reset_choices, set_up,
};
pub use listing::{
    Column, FOLDER_TYPE, Group, IconSize, Listing, Sort, SortDir, ViewMode, ViewSpec, type_of,
};
pub use panel::{DEFAULT_PLACE_ICON, Place, Places, TrashView};
pub use platform::{TrashedItem, Volume, VolumeKind};
use serde::{Deserialize, Serialize};
pub use session::{
    DockSession, Geometry, PaneSession, Session, TabSession, WindowSession, WorkspaceSession,
    parse_entry_id,
};
pub use settings::{
    AfterDrag, Archives, ConnectionEntry, Density, Drag, LAST_SESSION, OnOpen, PanelSide, Plugins,
    Preview, Rgb, Search, Section, SessionSettings, Settings, SettingsChange, SidePanel,
    SidePanelChange, TextPreview, Theme, TitlebarColour, WindowFrame, WindowSettings,
    parse_location,
};
pub use tree::{Tree, TreeRow};
use types::{
    Conflict, ConflictId, Entry, EntryId, ExtId, JobId, Location, Progress, ProviderId, Question,
    QuestionId,
};

/// Identity of a tab, unique within one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

/// Identity of a window, unique within one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u64);

/// Identity of a workspace: one tab of the title row, holding a whole layout
/// of panes (SPEC.md *Workspace tabs*). Unique within one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WorkspaceId(pub u64);

/// What a tab shows. Every kind is a tab of the dock: split, docked and
/// torn out alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TabKind {
    /// An explorer view of a location: toolbar, file list, status line.
    #[default]
    Explorer,
    /// The preview, one per window: it shows the selection of the explorer
    /// tab that was last focused ([`Model::preview_follows`]) and holds no
    /// listing of its own.
    Preview,
    /// A terminal session an action opened at the tab's location; drawn
    /// by the UI from the view the action returned, never restored with a
    /// session.
    Console,
    /// A comparison of two files or two folders an action opened; drawn by
    /// the UI from the view the action returned, never restored with a
    /// session.
    Compare,
}

impl TabKind {
    /// Whether a tab of this kind lists entries of its own: an explorer.
    #[must_use]
    pub fn lists(self) -> bool {
        matches!(self, Self::Explorer)
    }

    /// Whether a tab of this kind shows a view an action handed the UI,
    /// which cannot come back with a session.
    #[must_use]
    pub fn is_view(self) -> bool {
        matches!(self, Self::Console | Self::Compare)
    }
}

/// Where a split puts the new pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// Into a new pane split off `pane` on `side`.
    Beside {
        /// The pane to split.
        pane: PaneId,
        /// Which side the new pane takes.
        side: Side,
    },
    /// Into a new workspace tab of the same window, right after the shown
    /// one (a pane dropped on the centre target or on the tab row).
    NewWorkspace,
    /// Into a new window of its own (a pane dragged outside every window).
    NewWindow,
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
#[derive(Debug, Clone, PartialEq)]
pub enum Dock {
    /// A leaf holding one pane.
    Pane(PaneId),
    /// Two subtrees side by side (`Right`) or stacked (`Down`).
    Split {
        /// Which way the split goes.
        dir: SplitDir,
        /// How much of the room the first part takes, 0 to 1; display state
        /// the UI reports after the user drags the divider.
        fraction: f32,
        /// Left or top.
        first: Box<Dock>,
        /// Right or bottom.
        second: Box<Dock>,
    },
}

impl Dock {
    /// An even split.
    #[must_use]
    pub fn split_of(dir: SplitDir, first: Dock, second: Dock) -> Self {
        Self::Split {
            dir,
            fraction: 0.5,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// Whether the two trees have the same panes in the same splits,
    /// whatever their fractions.
    #[must_use]
    pub fn same_shape(&self, other: &Dock) -> bool {
        match (self, other) {
            (Dock::Pane(a), Dock::Pane(b)) => a == b,
            (
                Dock::Split {
                    dir: d1,
                    first: f1,
                    second: s1,
                    ..
                },
                Dock::Split {
                    dir: d2,
                    first: f2,
                    second: s2,
                    ..
                },
            ) => d1 == d2 && f1.same_shape(f2) && s1.same_shape(s2),
            _ => false,
        }
    }

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
                *self = Dock::split_of(dir, Dock::Pane(first), Dock::Pane(second));
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
            Dock::Split {
                dir,
                fraction,
                first,
                second,
            } => match (first.without(pane), second.without(pane)) {
                (Some(first), Some(second)) => Some(Dock::Split {
                    dir,
                    fraction,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(rest), None) | (None, Some(rest)) => Some(rest),
                (None, None) => None,
            },
        }
    }
}

/// A pane of a workspace's dock. It holds one tab: panes have no tab strips
/// (SPEC.md *Workspace tabs*); the accessors keep the shape of a list so the
/// callers written for strips still read.
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

/// One tab: a location and everything shown for it. An explorer shows the
/// location's entries; the preview tab carries a location for form's sake
/// and lists nothing.
#[derive(Debug, Clone)]
pub struct Tab {
    /// What the tab shows.
    pub kind: TabKind,
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
    /// The file types the list is narrowed to from the view options menu
    /// (as the Type column names them); `None` shows every type. Cleared
    /// on navigation, never saved.
    pub types: Option<BTreeSet<String>>,
    /// Hidden entries shown in this tab.
    pub show_hidden: bool,
    /// Subfolders flattened into the list.
    pub flatten: bool,
    /// The filter text searches the whole subtree: the listing then holds
    /// the matches the walk streams in, each with its full path.
    pub subtree: bool,
    /// The entries and the view over them.
    pub listing: Listing,
    /// Selected entry ids.
    pub selection: BTreeSet<EntryId>,
    /// What decorators said about the entries: badges, tints, cells.
    pub decorations: HashMap<EntryId, Decoration>,
    /// The expanded folders and their children, in [`ViewMode::Tree`].
    pub tree: Tree,
    /// The list's vertical scroll offset in logical points, as the UI last
    /// reported it; restored with the session.
    pub scroll: f32,
    /// The widths the user gave the details list's columns, in logical
    /// points by column id (`name`, `size`, `type`, `modified`, a decorator
    /// column's id); a column not in the map takes its default width.
    /// Restored with the session (SPEC.md *File list*).
    pub columns: BTreeMap<String, f32>,
    /// A note for the status line: why the tab is not where the session
    /// left it. Cleared by the next navigation.
    pub note: Option<String>,
    /// Why the location could not be listed, shown as a banner with
    /// *Retry* over the empty list (a connection that failed, a folder that
    /// refused); cleared by the next listing that works.
    pub failure: Option<String>,
    /// The title of a console or compare tab, as the action named it; the
    /// other kinds are titled from their location.
    pub title: Option<String>,
}

impl Tab {
    fn new(location: Location, settings: &Settings) -> Self {
        Self {
            kind: TabKind::Explorer,
            location,
            history: History::default(),
            view: ViewMode::Details,
            sort: Sort {
                by: settings.sort_by,
                dir: settings.sort_dir,
            },
            group: None,
            filter: String::new(),
            types: None,
            show_hidden: settings.show_hidden,
            flatten: false,
            subtree: false,
            listing: Listing::new(),
            selection: BTreeSet::new(),
            decorations: HashMap::new(),
            tree: Tree::default(),
            scroll: 0.0,
            columns: BTreeMap::new(),
            note: None,
            failure: None,
            title: None,
        }
    }

    /// Whether the rows come from the tree: the listing plus the children
    /// of every expanded folder.
    fn is_tree(&self) -> bool {
        self.view == ViewMode::Tree
    }

    /// Whether the listing holds subtree search results rather than the
    /// location's own entries.
    #[must_use]
    pub fn is_searching(&self) -> bool {
        self.subtree && !self.filter.is_empty()
    }

    /// Number of visible rows, children of expanded folders included in
    /// Tree view.
    #[must_use]
    pub fn row_count(&self) -> usize {
        if self.is_tree() {
            self.tree.rows().len()
        } else {
            self.listing.row_count()
        }
    }

    /// The visible row at `index`.
    #[must_use]
    pub fn row(&self, index: usize) -> Option<&Entry> {
        if self.is_tree() {
            let row = self.tree.rows().get(index)?;
            self.entry(row.id)
        } else {
            self.listing.row(index)
        }
    }

    /// The visible rows in display order.
    // Clippy 1.97 wants the attribute, 1.99 wants a reason with it.
    #[must_use = "the rows are the listing as the user sees it"]
    pub fn rows(&self) -> Box<dyn Iterator<Item = &Entry> + '_> {
        if self.is_tree() {
            Box::new(
                self.tree
                    .rows()
                    .iter()
                    .filter_map(move |row| self.entry(row.id)),
            )
        } else {
            Box::new(self.listing.rows())
        }
    }

    /// Ids of the visible rows in display order.
    #[must_use]
    pub fn row_ids(&self) -> Vec<EntryId> {
        if self.is_tree() {
            self.tree.rows().iter().map(|row| row.id).collect()
        } else {
            self.listing.row_ids()
        }
    }

    /// The depth of the visible row at `index`: 0 outside Tree view.
    #[must_use]
    pub fn depth(&self, index: usize) -> u16 {
        if self.is_tree() {
            self.tree.rows().get(index).map_or(0, |row| row.depth)
        } else {
            0
        }
    }

    /// The entry with this id, in the listing or under an expanded folder.
    #[must_use]
    pub fn entry(&self, id: EntryId) -> Option<&Entry> {
        self.listing.get(id).or_else(|| self.tree.child(id))
    }

    /// The location of an entry of this tab: on the tab's provider, or on
    /// the entry's own where the two differ, as in a virtual view that lists
    /// entries of other providers.
    #[must_use]
    pub fn location_of(&self, entry: &Entry) -> Location {
        Location::new(entry.id.provider, entry.path.clone())
    }

    /// Rebuilds the tree rows when the tab is in Tree view.
    fn rebuild_tree(&mut self) {
        if self.is_tree() {
            self.tree.rebuild(&self.listing);
        }
    }

    /// Forgets the listing, selection, decorations, expanded folders, scroll
    /// offset and note before the tab shows another location.
    fn leave(&mut self) {
        self.filter.clear();
        self.types = None;
        self.listing.clear();
        self.selection.clear();
        self.decorations.clear();
        self.tree.clear();
        self.scroll = 0.0;
        self.note = None;
    }

    /// The view specification this tab's listing follows.
    #[must_use]
    pub fn view_spec<'a>(&'a self, settings: &Settings) -> ViewSpec<'a> {
        ViewSpec {
            sort: self.sort,
            folders_first: settings.folders_first,
            filter: &self.filter,
            types: self.types.as_ref(),
            show_hidden: self.show_hidden,
            show_system: settings.show_system,
        }
    }

    /// The selected entries, in view order.
    pub fn selected(&self) -> impl Iterator<Item = &Entry> + '_ {
        self.rows().filter(|e| self.selection.contains(&e.id))
    }

    /// The text the view filters on: nothing while searching, since the
    /// results are the matches themselves.
    fn filter_text(&self) -> String {
        if self.is_searching() {
            String::new()
        } else {
            self.filter.clone()
        }
    }

    /// The view specification over `filter` and `types`, borrowing nothing
    /// of the tab, so the listing can change under it.
    fn spec_with<'a>(
        &self,
        settings: &Settings,
        filter: &'a str,
        types: Option<&'a BTreeSet<String>>,
    ) -> ViewSpec<'a> {
        ViewSpec {
            sort: self.sort,
            folders_first: settings.folders_first,
            filter,
            types,
            show_hidden: self.show_hidden,
            show_system: settings.show_system,
        }
    }

    fn refresh_view(&mut self, settings: &Settings) {
        let filter = self.filter_text();
        let types = self.types.clone();
        let spec = self.spec_with(settings, &filter, types.as_ref());
        self.listing.refresh_view(&spec);
        self.tree.refresh_views(&spec);
        self.rebuild_tree();
    }
}

/// What the job table knows about one job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobInfo {
    /// The name shown in the operations area.
    pub name: String,
    /// Running with progress, or how it ended.
    pub state: JobState,
    /// Whether the job can be paused.
    pub pausable: bool,
    /// Whether the job is paused right now.
    pub paused: bool,
}

/// Lifecycle of a job in the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobState {
    /// Queued behind another job on a volume it shares with it; the text is
    /// that job's name.
    Waiting(String),
    /// Still running, with its latest progress.
    Running(Progress),
    /// Finished well.
    Done,
    /// Finished with this error text.
    Failed(String),
    /// Cancelled.
    Cancelled,
}

/// One window: its pane tree, the pane it focuses, where it is on the
/// screen and its side panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    /// The workspaces, in the order of the title row's tabs; never empty.
    pub workspaces: Vec<Workspace>,
    /// The workspace shown.
    pub active: WorkspaceId,
    /// Position, size and maximised state, once the UI reported them.
    pub geometry: Option<Geometry>,
    /// The side panel: side, width, collapsed state, folded sections.
    pub side_panel: SidePanel,
}

impl Window {
    /// A window with one workspace.
    fn one(workspace: Workspace, geometry: Option<Geometry>, side_panel: SidePanel) -> Self {
        Self {
            active: workspace.id,
            workspaces: vec![workspace],
            geometry,
            side_panel,
        }
    }

    /// The workspace shown.
    #[must_use]
    pub fn workspace(&self) -> &Workspace {
        self.workspaces
            .iter()
            .find(|w| w.id == self.active)
            .or_else(|| self.workspaces.first())
            .unwrap_or(&EMPTY_WORKSPACE)
    }

    fn workspace_mut(&mut self, id: WorkspaceId) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|w| w.id == id)
    }

    /// The shown workspace's pane tree.
    #[must_use]
    pub fn dock(&self) -> &Dock {
        &self.workspace().dock
    }

    /// The pane keyboard input goes to when the window has it: the shown
    /// workspace's focused pane.
    #[must_use]
    pub fn focused(&self) -> PaneId {
        self.workspace().focused
    }

    /// The workspace holding this pane.
    #[must_use]
    pub fn workspace_of(&self, pane: PaneId) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .find(|w| w.dock.panes().contains(&pane))
    }

    /// Every pane of every workspace, workspace by workspace.
    #[must_use]
    pub fn panes(&self) -> Vec<PaneId> {
        self.workspaces
            .iter()
            .flat_map(|w| w.dock.panes())
            .collect()
    }

    /// The index of a workspace in the tab row.
    #[must_use]
    pub fn position(&self, id: WorkspaceId) -> Option<usize> {
        self.workspaces.iter().position(|w| w.id == id)
    }

    /// The workspace after `at`, else the one before it: what shows when
    /// the one at `at` goes.
    fn neighbour(&self, at: usize) -> Option<WorkspaceId> {
        self.workspaces
            .get(at)
            .or_else(|| self.workspaces.get(at.saturating_sub(1)))
            .map(|ws| ws.id)
    }
}

/// One workspace of a window: a layout of panes and the pane with the focus.
#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    /// The id, stable while the workspace exists.
    pub id: WorkspaceId,
    /// The pane tree.
    pub dock: Dock,
    /// The pane keyboard input goes to when the workspace is shown.
    pub focused: PaneId,
}

/// The workspace a window without one reads as; never shown.
static EMPTY_WORKSPACE: Workspace = Workspace {
    id: WorkspaceId(0),
    dock: Dock::Pane(PaneId(0)),
    focused: PaneId(0),
};

/// The title-row search box's query over the platform's search indexes and
/// what came back so far (QUESTIONS.md 118). The box lists the current
/// location's matches first and these hits, with their paths, after them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexSearch {
    /// The text the hits answer; empty when nothing is being searched.
    pub text: String,
    /// The hits in arrival order, each entry at most once.
    pub hits: Vec<Entry>,
    /// Every index has answered, or there was nothing to ask.
    pub done: bool,
    /// What the indexes that failed said, one line each.
    pub errors: Vec<String>,
}

impl Default for IndexSearch {
    /// Nothing searched, so nothing outstanding.
    fn default() -> Self {
        Self {
            text: String::new(),
            hits: Vec::new(),
            done: true,
            errors: Vec::new(),
        }
    }
}

impl IndexSearch {
    /// Starts over for `text`: no hits yet; done at once when the text is
    /// empty.
    pub fn start(&mut self, text: String) {
        self.done = text.is_empty();
        self.text = text;
        self.hits.clear();
        self.errors.clear();
    }

    /// Takes a batch of hits, skipping entries already listed (two indexes
    /// may know the same file); returns the ones that were new.
    pub fn extend(&mut self, batch: Vec<Entry>) -> Vec<Entry> {
        let mut new = Vec::new();
        for entry in batch {
            if self.hits.iter().all(|h| h.id != entry.id) {
                self.hits.push(entry.clone());
                new.push(entry);
            }
        }
        new
    }
}

/// The whole model: every window, their panes and tabs, the job table and
/// the settings snapshot.
#[derive(Debug, Clone)]
pub struct Model {
    windows: BTreeMap<WindowId, Window>,
    panes: BTreeMap<PaneId, Pane>,
    tabs: BTreeMap<TabId, Tab>,
    /// The pane keyboard input goes to: the focused window's focused pane.
    pub focused: PaneId,
    /// The window that has the focus.
    pub focused_window: WindowId,
    /// Jobs running or recently finished; shared by every window.
    pub jobs: BTreeMap<JobId, JobInfo>,
    /// Name conflicts jobs wait on, oldest first; the UI asks about the
    /// first.
    pub conflicts: Vec<Conflict>,
    /// Questions extensions wait on (a host key to trust, a passphrase),
    /// oldest first; the UI asks the first.
    pub questions: Vec<Question>,
    /// The side panel's Places list.
    pub places: Places,
    /// The mounted volumes: the machine's drives and the network mounts.
    pub volumes: Vec<Volume>,
    /// The cloud providers mounted, sorted; the Cloud section shows while
    /// there is one.
    pub cloud_providers: Vec<ProviderId>,
    /// The cloud rows: what the cloud providers list at their roots, their
    /// accounts or containers.
    pub cloud: Vec<Entry>,
    /// The network providers mounted, sorted; the Network section lists
    /// what they list at their roots.
    pub network_providers: Vec<ProviderId>,
    /// The network rows: what the network providers list at their roots,
    /// their connections; the mounted network volumes are in `volumes`.
    pub network: Vec<Entry>,
    /// The trash's contents.
    pub trash: TrashView,
    /// The settings snapshot.
    pub settings: Settings,
    /// The integration items as the last check left them
    /// (specs/system-integration.md §6): what the dialog and Settings →
    /// Integration show.
    pub integration: Integration,
    /// The title-row search box's query over the search indexes.
    pub index_search: IndexSearch,
    /// The search indexes available right now, by extension id: the ones
    /// the title-row search box queries. Empty when the platform has none;
    /// the box then searches the current location alone, as in 0.1.
    pub search_indexes: Vec<ExtId>,
    /// A subdirectory search index is registered, so the filter bar offers
    /// *Search in subdirectories*.
    pub subtree_search: bool,
    /// The explorer tab the preview follows: the last one focused.
    preview_follows: Option<TabId>,
    next_pane: u64,
    next_tab: u64,
    next_window: u64,
    next_workspace: u64,
}

impl Model {
    /// A model with one window, one pane and one tab at `start`.
    #[must_use]
    pub fn new(settings: Settings, start: Location) -> Self {
        let mut model = Self::empty(settings);
        model.new_window(start);
        model
    }

    /// A model without a window; one must be added before use.
    fn empty(settings: Settings) -> Self {
        Self {
            windows: BTreeMap::new(),
            panes: BTreeMap::new(),
            tabs: BTreeMap::new(),
            focused: PaneId(0),
            focused_window: WindowId(0),
            jobs: BTreeMap::new(),
            conflicts: Vec::new(),
            questions: Vec::new(),
            places: Places::default(),
            volumes: Vec::new(),
            cloud_providers: Vec::new(),
            cloud: Vec::new(),
            network_providers: Vec::new(),
            network: Vec::new(),
            trash: TrashView::default(),
            settings,
            integration: Integration::default(),
            index_search: IndexSearch::default(),
            search_indexes: Vec::new(),
            subtree_search: false,
            preview_follows: None,
            next_pane: 1,
            next_tab: 1,
            next_window: 1,
            next_workspace: 1,
        }
    }

    /// Keeps [`preview_follows`](Self::preview_follows) true to the focus:
    /// the focused pane's active tab when that is an explorer, else the
    /// explorer followed so far while it exists.
    fn refollow(&mut self) {
        if self
            .preview_follows
            .is_some_and(|tab| !self.tabs.contains_key(&tab))
        {
            self.preview_follows = None;
        }
        if let Some((id, tab)) = self.active_tab(self.focused)
            && tab.kind == TabKind::Explorer
        {
            self.preview_follows = Some(id);
        }
    }

    /// The explorer tab the preview shows the selection of: the one last
    /// focused, in any window. `None` until an explorer tab has had the
    /// focus, or once the last one was closed.
    #[must_use]
    pub fn preview_follows(&self) -> Option<TabId> {
        self.preview_follows
    }

    /// The window's Preview tab, if it is open.
    #[must_use]
    pub fn preview_tab(&self, window: WindowId) -> Option<TabId> {
        let w = self.windows.get(&window)?;
        w.panes()
            .into_iter()
            .filter_map(|pane| self.panes.get(&pane))
            .flat_map(|pane| pane.tabs.iter().copied())
            .find(|tab| {
                self.tabs
                    .get(tab)
                    .is_some_and(|t| t.kind == TabKind::Preview)
            })
    }

    /// Opens the window's Preview tab in a new pane right of every other
    /// pane of the shown workspace, the others keeping `fraction` of the
    /// width, or returns the one it has; one the window has in another
    /// workspace closes there and opens here. The focus stays where it is:
    /// the preview follows it.
    pub fn open_preview(&mut self, window: WindowId, fraction: f32) -> Option<TabId> {
        if let Some(tab) = self.preview_tab(window) {
            let shown = self.windows.get(&window)?.active;
            match self.pane_of(tab).and_then(|pane| self.workspace_of(pane)) {
                Some((_, ws)) if ws == shown => return Some(tab),
                _ => {
                    self.close_tab(tab);
                }
            }
        }
        let w = self.windows.get(&window)?;
        // A location for form's sake: the preview lists nothing.
        let location = self
            .active_tab(w.focused())
            .map(|(_, tab)| tab.location.clone())
            .or_else(|| {
                w.dock()
                    .panes()
                    .into_iter()
                    .filter_map(|pane| self.active_tab(pane))
                    .map(|(_, tab)| tab.location.clone())
                    .next()
            })?;
        let pane = PaneId(self.next_pane);
        self.next_pane += 1;
        let tab = TabId(self.next_tab);
        self.next_tab += 1;
        let mut state = Tab::new(location, &self.settings);
        state.kind = TabKind::Preview;
        self.tabs.insert(tab, state);
        self.panes.insert(
            pane,
            Pane {
                tabs: vec![tab],
                active: Some(tab),
            },
        );
        let w = self.windows.get_mut(&window)?;
        let active = w.active;
        let ws = w.workspace_mut(active)?;
        let rest = std::mem::replace(&mut ws.dock, Dock::Pane(pane));
        ws.dock = Dock::Split {
            dir: SplitDir::Right,
            fraction: if fraction.is_finite() {
                fraction.clamp(0.05, 0.95)
            } else {
                0.72
            },
            first: Box::new(rest),
            second: Box::new(Dock::Pane(pane)),
        };
        Some(tab)
    }

    /// Opens a window with one pane and one tab at `location`, focused.
    /// Returns the window, the pane and the tab.
    pub fn new_window(&mut self, location: Location) -> (WindowId, PaneId, TabId) {
        let window = WindowId(self.next_window);
        self.next_window += 1;
        let pane = PaneId(self.next_pane);
        self.next_pane += 1;
        let tab = TabId(self.next_tab);
        self.next_tab += 1;
        self.panes.insert(
            pane,
            Pane {
                tabs: vec![tab],
                active: Some(tab),
            },
        );
        self.tabs.insert(tab, Tab::new(location, &self.settings));
        let workspace = self.workspace_for(pane);
        self.windows
            .insert(window, Window::one(workspace, None, SidePanel::default()));
        self.focused_window = window;
        self.focused = pane;
        self.refollow();
        (window, pane, tab)
    }

    /// A workspace holding one pane, with the next id.
    fn workspace_for(&mut self, pane: PaneId) -> Workspace {
        let id = WorkspaceId(self.next_workspace);
        self.next_workspace += 1;
        Workspace {
            id,
            dock: Dock::Pane(pane),
            focused: pane,
        }
    }

    /// A pane holding one new tab at `location`, with the next ids.
    fn pane_with_tab(&mut self, location: Location) -> (PaneId, TabId) {
        let pane = PaneId(self.next_pane);
        self.next_pane += 1;
        let tab = TabId(self.next_tab);
        self.next_tab += 1;
        self.tabs.insert(tab, Tab::new(location, &self.settings));
        self.panes.insert(
            pane,
            Pane {
                tabs: vec![tab],
                active: Some(tab),
            },
        );
        (pane, tab)
    }

    /// The workspaces of a window, in the order of the title row.
    #[must_use]
    pub fn workspaces(&self, window: WindowId) -> &[Workspace] {
        self.windows.get(&window).map_or(&[], |w| &w.workspaces)
    }

    /// The window and the workspace holding this pane.
    #[must_use]
    pub fn workspace_of(&self, pane: PaneId) -> Option<(WindowId, WorkspaceId)> {
        self.windows
            .iter()
            .find_map(|(id, w)| w.workspace_of(pane).map(|ws| (*id, ws.id)))
    }

    /// The window holding this workspace.
    #[must_use]
    pub fn window_of_workspace(&self, id: WorkspaceId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, w)| w.position(id).is_some())
            .map(|(window, _)| *window)
    }

    /// The pane of a workspace that holds exactly one, whatever the pane
    /// shows — the preview, a console and a comparison count like an
    /// explorer (QUESTIONS.md 270); `None` for a workspace of several panes
    /// and for one the model does not have. Only the tab of such a
    /// workspace can be docked into another layout (SPEC.md *Workspace
    /// tabs*, Dragging a tab).
    #[must_use]
    pub fn sole_pane(&self, id: WorkspaceId) -> Option<PaneId> {
        let window = self.window_of_workspace(id)?;
        let workspace = self.workspaces(window).iter().find(|ws| ws.id == id)?;
        match workspace.dock {
            Dock::Pane(pane) => Some(pane),
            Dock::Split { .. } => None,
        }
    }

    /// Docks the one pane of workspace `id` beside `pane` on `side`, in
    /// whichever workspace and window `pane` is: the reverse of a pane
    /// dropped on the tab row. The workspace, emptied, leaves its row, and
    /// the layout that took the pane is shown with the pane focused.
    /// Refused for a workspace of several panes — its panes move one at a
    /// time by their handles — and for a `pane` that is the workspace's
    /// own. Returns whether the pane was docked.
    pub fn dock_workspace(&mut self, id: WorkspaceId, pane: PaneId, side: Side) -> bool {
        let Some(sole) = self.sole_pane(id) else {
            return false;
        };
        if sole == pane {
            return false;
        }
        let Some(tab) = self.pane(sole).and_then(Pane::active) else {
            return false;
        };
        self.move_tab(tab, DockTarget::Beside { pane, side })
    }

    /// Opens a new workspace in `window`, right after the shown one, with one
    /// pane at `location`, and shows it. Returns the workspace, the pane and
    /// the tab.
    pub fn new_workspace(
        &mut self,
        window: WindowId,
        location: Location,
    ) -> Option<(WorkspaceId, PaneId, TabId)> {
        if !self.windows.contains_key(&window) {
            return None;
        }
        let (pane, tab) = self.pane_with_tab(location);
        let workspace = self.workspace_for(pane);
        let id = workspace.id;
        let w = self.windows.get_mut(&window)?;
        let at = w.position(w.active).map_or(w.workspaces.len(), |i| i + 1);
        w.workspaces.insert(at, workspace);
        self.activate_workspace(id);
        Some((id, pane, tab))
    }

    /// Shows a workspace: its window gets the focus, its focused pane the
    /// keyboard. Returns whether it exists.
    pub fn activate_workspace(&mut self, id: WorkspaceId) -> bool {
        let Some(window) = self.window_of_workspace(id) else {
            return false;
        };
        let Some(w) = self.windows.get_mut(&window) else {
            return false;
        };
        w.active = id;
        let focused = w.focused();
        self.focused_window = window;
        self.focused = focused;
        self.refollow();
        true
    }

    /// Closes a workspace with every pane and tab in it; the neighbour
    /// after it, else before it, is shown. A window left without a
    /// workspace closes unless it is the last one, whose last workspace
    /// stays. Returns whether it closed.
    pub fn close_workspace(&mut self, id: WorkspaceId) -> bool {
        let Some(window) = self.window_of_workspace(id) else {
            return false;
        };
        let Some(w) = self.windows.get(&window) else {
            return false;
        };
        if w.workspaces.len() < 2 {
            return self.windows.len() > 1 && self.close_window(window);
        }
        let panes: Vec<PaneId> = w
            .workspaces
            .iter()
            .find(|ws| ws.id == id)
            .map(|ws| ws.dock.panes())
            .unwrap_or_default();
        for pane in panes {
            self.remove_pane(pane);
        }
        let Some(w) = self.windows.get_mut(&window) else {
            return false;
        };
        let Some(at) = w.position(id) else {
            return false;
        };
        w.workspaces.remove(at);
        if w.active == id
            && let Some(next) = w.neighbour(at)
        {
            w.active = next;
        }
        if self.focused_window == window {
            self.focused = w.focused();
        }
        self.refollow();
        true
    }

    /// Moves a workspace to position `to` in its window's tab row. Returns
    /// whether it moved.
    pub fn move_workspace(&mut self, id: WorkspaceId, to: usize) -> bool {
        let Some(window) = self.window_of_workspace(id) else {
            return false;
        };
        let Some(w) = self.windows.get_mut(&window) else {
            return false;
        };
        let Some(from) = w.position(id) else {
            return false;
        };
        let to = to.min(w.workspaces.len() - 1);
        if from == to {
            return false;
        }
        let workspace = w.workspaces.remove(from);
        w.workspaces.insert(to, workspace);
        true
    }

    /// Moves a workspace to the end of another window's tab row, or into a
    /// new window of its own, and shows it there. A window emptied by the
    /// move closes. Returns whether it moved.
    pub fn workspace_to_window(&mut self, id: WorkspaceId, target: Option<WindowId>) -> bool {
        let Some(from) = self.window_of_workspace(id) else {
            return false;
        };
        if target == Some(from) {
            return false;
        }
        let alone = self
            .windows
            .get(&from)
            .is_some_and(|w| w.workspaces.len() == 1);
        if target.is_none() && alone {
            // A window with this one workspace is one already.
            return true;
        }
        if target.is_some_and(|t| !self.windows.contains_key(&t)) {
            return false;
        }
        let geometry = self.offset_geometry(from);
        let Some(w) = self.windows.get_mut(&from) else {
            return false;
        };
        let Some(at) = w.position(id) else {
            return false;
        };
        let workspace = w.workspaces.remove(at);
        if w.active == id
            && let Some(next) = w.neighbour(at)
        {
            w.active = next;
        }
        if let Some(window) = target {
            if let Some(w) = self.windows.get_mut(&window) {
                w.workspaces.push(workspace);
            }
        } else {
            let window = WindowId(self.next_window);
            self.next_window += 1;
            self.windows.insert(
                window,
                Window::one(workspace, geometry, SidePanel::default()),
            );
        }
        self.drop_empty_window(from);
        self.activate_workspace(id);
        true
    }

    /// The geometry for a window torn off `window`: the same size, a little
    /// down and right, not maximised.
    fn offset_geometry(&self, window: WindowId) -> Option<Geometry> {
        self.windows
            .get(&window)
            .and_then(|w| w.geometry)
            .map(|g| Geometry {
                x: g.x + 40.0,
                y: g.y + 40.0,
                maximised: false,
                ..g
            })
    }

    /// Forgets a window left without a workspace; the focus moves to another
    /// window.
    fn drop_empty_window(&mut self, window: WindowId) {
        if self
            .windows
            .get(&window)
            .is_some_and(|w| w.workspaces.is_empty())
        {
            self.windows.remove(&window);
            if self.focused_window == window
                && let Some(next) = self.windows.keys().next().copied()
            {
                self.focused_window = next;
                self.focused = self.windows[&next].focused();
            }
        }
    }

    /// Opens a window from a saved one: its workspaces with their panes,
    /// tabs and docks, its side panel and geometry. A pane without tabs is
    /// dropped, so is a workspace without panes; a window without
    /// workspaces gets one at `fallback`. A pane saved with several tabs (a
    /// session of 0.2, whose panes had tab strips) keeps its active tab and
    /// opens the others as workspaces of their own, after every saved
    /// workspace. Returns the window, now focused.
    pub fn add_window(&mut self, saved: &WindowSession, fallback: &Location) -> WindowId {
        let mut workspaces: Vec<Workspace> = Vec::new();
        let mut extra: Vec<Workspace> = Vec::new();
        let mut active: Option<WorkspaceId> = None;
        for (index, saved_ws) in saved.workspaces().iter().enumerate() {
            let Some((workspace, more)) = self.add_workspace(saved_ws) else {
                continue;
            };
            if index == saved.active_workspace {
                active = Some(workspace.id);
            }
            workspaces.push(workspace);
            extra.extend(more);
        }
        workspaces.extend(extra);
        if workspaces.is_empty() {
            let (pane, _) = self.pane_with_tab(fallback.clone());
            workspaces.push(self.workspace_for(pane));
        }
        let active = active
            .filter(|id| workspaces.iter().any(|w| w.id == *id))
            .unwrap_or(workspaces[0].id);
        let window = WindowId(self.next_window);
        self.next_window += 1;
        self.windows.insert(
            window,
            Window {
                workspaces,
                active,
                geometry: saved.geometry,
                side_panel: saved.side_panel.clone(),
            },
        );
        self.activate_workspace(active);
        window
    }

    /// Builds a workspace from a saved one: its panes with their active tab
    /// each and its dock. Every further tab of a pane comes back in a
    /// workspace of its own, returned after it. `None` for a workspace with
    /// no tab at all.
    fn add_workspace(&mut self, saved: &WorkspaceSession) -> Option<(Workspace, Vec<Workspace>)> {
        let mut ids: Vec<PaneId> = Vec::new();
        let mut focused: Option<PaneId> = None;
        let mut extra: Vec<Workspace> = Vec::new();
        for (index, saved_pane) in saved.panes.iter().enumerate() {
            if saved_pane.tabs.is_empty() {
                // Keep the indices the dock refers to aligned.
                ids.push(PaneId(0));
                continue;
            }
            let kept = saved_pane
                .active
                .filter(|i| *i < saved_pane.tabs.len())
                .unwrap_or(0);
            let mut pane_id = None;
            for (i, saved_tab) in saved_pane.tabs.iter().enumerate() {
                let (pane, tab) = self.pane_with_tab(saved_tab.location.clone());
                if let Some(state) = self.tabs.get_mut(&tab) {
                    // A hand-edited file naming a console or a comparison:
                    // the view is gone, the location stays.
                    state.kind = if saved_tab.kind.is_view() {
                        TabKind::Explorer
                    } else {
                        saved_tab.kind
                    };
                    state.view = saved_tab.view;
                    state.sort = saved_tab.sort;
                    state.group = saved_tab.group;
                    state.filter.clone_from(&saved_tab.filter);
                    state.show_hidden = saved_tab.show_hidden;
                    state.flatten = saved_tab.flatten;
                    state.selection = saved_tab.selection.iter().copied().collect();
                    state.scroll = saved_tab.scroll.max(0.0);
                    state.columns = saved_tab
                        .columns
                        .iter()
                        .filter(|(_, w)| w.is_finite() && **w > 0.0)
                        .map(|(id, w)| (id.clone(), *w))
                        .collect();
                    state.note.clone_from(&saved_tab.note);
                }
                if i == kept {
                    pane_id = Some(pane);
                } else {
                    let workspace = self.workspace_for(pane);
                    extra.push(workspace);
                }
            }
            let pane = pane_id?;
            if index == saved.focused_pane {
                focused = Some(pane);
            }
            ids.push(pane);
        }
        let real: Vec<PaneId> = ids.iter().copied().filter(|p| p.0 != 0).collect();
        let dock = saved
            .dock
            .to_dock(&ids)
            .and_then(|dock| {
                // A dock that names a dropped pane collapses around it.
                let mut dock = dock;
                for gone in ids.iter().filter(|p| p.0 == 0) {
                    dock = dock.without(*gone)?;
                }
                Some(dock)
            })
            .filter(|dock| {
                let named = dock.panes();
                named.len() == real.len() && real.iter().all(|p| named.contains(p))
            });
        let dock = if let Some(dock) = dock {
            dock
        } else {
            let (first, rest) = real.split_first()?;
            // The dock did not match the panes: lay them out side by side.
            rest.iter().fold(Dock::Pane(*first), |dock, pane| {
                Dock::split_of(SplitDir::Right, dock, Dock::Pane(*pane))
            })
        };
        let focused = focused
            .filter(|p| p.0 != 0)
            .unwrap_or_else(|| dock.panes().first().copied().unwrap_or(PaneId(0)));
        let id = WorkspaceId(self.next_workspace);
        self.next_workspace += 1;
        Some((Workspace { id, dock, focused }, extra))
    }

    /// A model rebuilt from a session: every window, pane and tab. A session
    /// without a window gets one at `fallback`. The notes the session's tabs
    /// carry come along for the status line.
    #[must_use]
    pub fn from_session(settings: Settings, session: &Session, fallback: &Location) -> Self {
        let mut model = Self::empty(settings);
        let mut windows = Vec::new();
        for saved in &session.windows {
            windows.push(model.add_window(saved, fallback));
        }
        if windows.is_empty() {
            model.new_window(fallback.clone());
            return model;
        }
        let focused = windows
            .get(session.focused_window)
            .copied()
            .unwrap_or(windows[0]);
        model.focus_window(focused);
        model
    }

    /// The session this model is: what `session.toml` and a saved layout
    /// hold.
    #[must_use]
    pub fn session(&self) -> Session {
        let mut windows = Vec::new();
        let mut focused_window = 0;
        for (index, (id, window)) in self.windows.iter().enumerate() {
            if *id == self.focused_window {
                focused_window = index;
            }
            windows.push(WindowSession {
                geometry: window.geometry,
                side_panel: window.side_panel.clone(),
                active_workspace: window.position(window.active).unwrap_or(0),
                workspaces: window
                    .workspaces
                    .iter()
                    .map(|ws| self.workspace_session(ws))
                    .collect(),
                ..WindowSession::default()
            });
        }
        Session {
            focused_window,
            windows,
        }
    }

    /// One workspace as the session holds it.
    fn workspace_session(&self, ws: &Workspace) -> WorkspaceSession {
        let panes = ws.dock.panes();
        let index_of = |pane: PaneId| panes.iter().position(|p| *p == pane).unwrap_or(0);
        WorkspaceSession {
            focused_pane: index_of(ws.focused),
            dock: DockSession::from_dock(&ws.dock, &index_of),
            panes: panes
                .iter()
                .map(|pane| {
                    let pane = self.panes.get(pane);
                    // A console or a comparison cannot come back with the
                    // session: its tab is left out.
                    let tabs: Vec<TabId> = pane
                        .map(|p| p.tabs.clone())
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|id| self.tabs.get(id).is_some_and(|t| !t.kind.is_view()))
                        .collect();
                    PaneSession {
                        active: pane
                            .and_then(Pane::active)
                            .and_then(|a| tabs.iter().position(|t| *t == a))
                            .or_else(|| (!tabs.is_empty()).then_some(0)),
                        tabs: tabs
                            .iter()
                            .filter_map(|id| self.tabs.get(id))
                            .map(|tab| TabSession {
                                kind: tab.kind,
                                location: tab.location.clone(),
                                view: tab.view,
                                sort: tab.sort,
                                group: tab.group,
                                filter: tab.filter.clone(),
                                show_hidden: tab.show_hidden,
                                flatten: tab.flatten,
                                selection: tab.selection.iter().copied().collect(),
                                scroll: tab.scroll,
                                columns: tab.columns.clone(),
                                note: None,
                            })
                            .collect(),
                    }
                })
                .collect(),
        }
    }

    /// The window with this id.
    #[must_use]
    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    /// Every window, by id, in creation order.
    pub fn windows(&self) -> impl Iterator<Item = (WindowId, &Window)> + '_ {
        self.windows.iter().map(|(id, window)| (*id, window))
    }

    /// The window holding this pane.
    #[must_use]
    pub fn window_of(&self, pane: PaneId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, w)| w.workspace_of(pane).is_some())
            .map(|(id, _)| *id)
    }

    /// The focused window's shown workspace's pane tree.
    #[must_use]
    pub fn dock(&self) -> &Dock {
        self.windows
            .get(&self.focused_window)
            .or_else(|| self.windows.values().next())
            .map_or(&EMPTY_DOCK, Window::dock)
    }

    /// Every pane of every window, window by window and workspace by
    /// workspace, left-to-right and top-to-bottom within each.
    #[must_use]
    pub fn all_panes(&self) -> Vec<PaneId> {
        self.windows.values().flat_map(Window::panes).collect()
    }

    /// Gives `window` the focus; its focused pane becomes the focused one.
    /// Returns whether it exists.
    pub fn focus_window(&mut self, window: WindowId) -> bool {
        match self.windows.get(&window) {
            Some(w) => {
                self.focused_window = window;
                self.focused = w.focused();
                self.refollow();
                true
            }
            None => false,
        }
    }

    /// Closes a window with every pane and tab in it, unless it is the last
    /// one. Returns whether it closed.
    pub fn close_window(&mut self, window: WindowId) -> bool {
        if self.windows.len() < 2 {
            return false;
        }
        let Some(gone) = self.windows.remove(&window) else {
            return false;
        };
        for pane in gone.panes() {
            if let Some(pane) = self.panes.remove(&pane) {
                for tab in pane.tabs {
                    self.tabs.remove(&tab);
                }
            }
        }
        if self.focused_window == window
            && let Some(next) = self.windows.keys().next().copied()
        {
            self.focus_window(next);
        }
        self.refollow();
        true
    }

    /// Records where a window is on the screen.
    pub fn set_geometry(&mut self, window: WindowId, geometry: Geometry) -> bool {
        match self.windows.get_mut(&window) {
            Some(w) => {
                w.geometry = Some(geometry);
                true
            }
            None => false,
        }
    }

    /// Applies a change to a window's side panel.
    pub fn side_panel(&mut self, window: WindowId, change: SidePanelChange) -> bool {
        match self.windows.get_mut(&window) {
            Some(w) => {
                w.side_panel.apply(change);
                true
            }
            None => false,
        }
    }

    /// Takes a window's split fractions from `dock`, which must have the
    /// same panes in the same splits. Returns whether it did.
    /// Records the split fractions of the window's shown workspace, as the
    /// UI shows them; `dock` must have the workspace's shape. Returns
    /// whether it did.
    pub fn set_layout(&mut self, window: WindowId, dock: Dock) -> bool {
        let Some(w) = self.windows.get_mut(&window) else {
            return false;
        };
        let active = w.active;
        match w.workspace_mut(active) {
            Some(ws) if ws.dock.same_shape(&dock) => {
                ws.dock = dock;
                true
            }
            _ => false,
        }
    }

    /// Records a tab's scroll offset.
    pub fn set_scroll(&mut self, tab: TabId, offset: f32) -> bool {
        match self.tabs.get_mut(&tab) {
            Some(t) => {
                t.scroll = offset.max(0.0);
                true
            }
            None => false,
        }
    }

    /// Records the widths the user gave a tab's columns, by column id; an
    /// empty map puts every column back to its default. A width that is
    /// not a positive number is dropped. `false` when the tab is unknown.
    pub fn set_column_widths(&mut self, tab: TabId, columns: BTreeMap<String, f32>) -> bool {
        match self.tabs.get_mut(&tab) {
            Some(t) => {
                t.columns = columns
                    .into_iter()
                    .filter(|(_, w)| w.is_finite() && *w > 0.0)
                    .collect();
                true
            }
            None => false,
        }
    }

    /// Sets or clears a tab's status-line note.
    pub fn set_note(&mut self, tab: TabId, note: Option<String>) -> bool {
        match self.tabs.get_mut(&tab) {
            Some(t) => {
                t.note = note;
                true
            }
            None => false,
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

    /// Makes `pane` the focused one, its workspace the shown one and its
    /// window the focused window. Returns whether it exists.
    pub fn focus(&mut self, pane: PaneId) -> bool {
        let Some((window, workspace)) = self.workspace_of(pane) else {
            return false;
        };
        if let Some(w) = self.windows.get_mut(&window) {
            w.active = workspace;
            if let Some(ws) = w.workspace_mut(workspace) {
                ws.focused = pane;
            }
        }
        self.focused_window = window;
        self.focused = pane;
        self.refollow();
        true
    }

    /// Makes `tab` the active tab of its pane and focuses that pane.
    pub fn activate(&mut self, tab: TabId) -> bool {
        let Some(pane) = self.pane_of(tab) else {
            return false;
        };
        if let Some(p) = self.panes.get_mut(&pane) {
            p.active = Some(tab);
        }
        self.focus(pane)
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
        tab.leave();
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
        tab.leave();
        Some(id)
    }

    /// Goes back one step in the pane's active tab, restoring the selection
    /// the step recorded. Returns the new location.
    pub fn back(&mut self, pane: PaneId) -> Option<Location> {
        let (_, tab) = self.active_tab_mut(pane)?;
        let to = tab.history.back.pop()?;
        let from = std::mem::replace(&mut tab.location, to.location.clone());
        tab.history.forward.push(HistoryEntry::at(from));
        tab.leave();
        tab.selection = to.selection.iter().map(|e| e.id).collect();
        Some(to.location)
    }

    /// Goes forward one step in the pane's active tab. Returns the new
    /// location.
    pub fn forward(&mut self, pane: PaneId) -> Option<Location> {
        let (_, tab) = self.active_tab_mut(pane)?;
        let to = tab.history.forward.pop()?;
        let from = std::mem::replace(&mut tab.location, to.location.clone());
        tab.history.back.push(HistoryEntry::at(from));
        tab.leave();
        tab.selection = to.selection.iter().map(|e| e.id).collect();
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

    /// Narrows the pane's active tab to the entries of `types`, as the Type
    /// column names them, or shows every type again for `None` or an empty
    /// set (SPEC.md *View options menu*). Returns whether the tab exists.
    pub fn set_types(&mut self, pane: PaneId, types: Option<BTreeSet<String>>) -> bool {
        let settings = self.settings.clone();
        let Some((_, tab)) = self.active_tab_mut(pane) else {
            return false;
        };
        tab.types = types.filter(|t| !t.is_empty());
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
                tab.tree.clear();
                true
            }
            _ => false,
        }
    }

    /// Sets whether the pane's active tab's filter searches the whole
    /// subtree. Returns whether it changed; the listing must then be redone
    /// when a filter text is set.
    pub fn set_subtree(&mut self, pane: PaneId, on: bool) -> bool {
        let settings = self.settings.clone();
        match self.active_tab_mut(pane) {
            Some((_, tab)) if tab.subtree != on => {
                tab.subtree = on;
                if !tab.filter.is_empty() {
                    tab.listing.clear();
                    tab.selection.clear();
                    tab.tree.clear();
                }
                tab.refresh_view(&settings);
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
                tab.rebuild_tree();
                true
            }
            None => false,
        }
    }

    /// Expands a folder of the pane's active tab in Tree view. Returns
    /// whether it was not expanded before, in which case its children must
    /// be listed into [`tree_batch`](Self::tree_batch).
    pub fn tree_expand(&mut self, pane: PaneId, folder: EntryId) -> bool {
        let Some((_, tab)) = self.active_tab_mut(pane) else {
            return false;
        };
        if tab
            .entry(folder)
            .is_none_or(|e| e.kind != types::EntryKind::Dir)
        {
            return false;
        }
        let fresh = tab.tree.expand(folder);
        tab.rebuild_tree();
        fresh
    }

    /// Collapses an expanded folder, dropping its cached children. Returns
    /// whether it was expanded.
    pub fn tree_collapse(&mut self, pane: PaneId, folder: EntryId) -> bool {
        let Some((_, tab)) = self.active_tab_mut(pane) else {
            return false;
        };
        let was = tab.tree.collapse(folder);
        if was {
            let listing = &tab.listing;
            let tree = &tab.tree;
            tab.selection
                .retain(|id| listing.get(*id).is_some() || tree.child(*id).is_some());
            tab.rebuild_tree();
        }
        was
    }

    /// Records a batch of an expanded folder's children.
    pub fn tree_batch(&mut self, pane: PaneId, folder: EntryId, batch: Vec<Entry>) -> bool {
        let settings = self.settings.clone();
        let Some((_, tab)) = self.active_tab_mut(pane) else {
            return false;
        };
        let filter = tab.filter_text();
        let types = tab.types.clone();
        let spec = tab.spec_with(&settings, &filter, types.as_ref());
        let took = tab.tree.extend(folder, batch, &spec);
        tab.rebuild_tree();
        took
    }

    /// Replaces an expanded folder's children in one step, as a refresh
    /// does.
    pub fn tree_replace(&mut self, pane: PaneId, folder: EntryId, entries: Vec<Entry>) -> bool {
        let settings = self.settings.clone();
        let Some((_, tab)) = self.active_tab_mut(pane) else {
            return false;
        };
        let filter = tab.filter_text();
        let types = tab.types.clone();
        let spec = tab.spec_with(&settings, &filter, types.as_ref());
        let took = tab.tree.replace(folder, entries, &spec);
        if took {
            let listing = &tab.listing;
            let tree = &tab.tree;
            tab.selection
                .retain(|id| listing.get(*id).is_some() || tree.child(*id).is_some());
        }
        tab.rebuild_tree();
        took
    }

    /// Marks an expanded folder's listing complete.
    pub fn tree_done(&mut self, pane: PaneId, folder: EntryId) -> bool {
        match self.active_tab_mut(pane) {
            Some((_, tab)) => tab.tree.done(folder),
            None => false,
        }
    }

    /// The expanded folders of the pane's active tab that are still in its
    /// listing, with their locations, for listing them again after a
    /// refresh.
    #[must_use]
    pub fn tree_expanded(&self, pane: PaneId) -> Vec<(EntryId, Location)> {
        let Some((_, tab)) = self.active_tab(pane) else {
            return Vec::new();
        };
        let mut out: Vec<(EntryId, Location)> = tab
            .tree
            .expanded
            .iter()
            .filter_map(|id| tab.entry(*id).map(|e| (*id, tab.location_of(e))))
            .collect();
        // Parents before children, so a child's listing finds its folder.
        out.sort_by_key(|(_, loc)| loc.path.len());
        out
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
            SelectionChange::All => tab.selection = tab.row_ids().into_iter().collect(),
            SelectionChange::Invert => {
                let visible: BTreeSet<EntryId> = tab.row_ids().into_iter().collect();
                tab.selection = visible.difference(&before).copied().collect();
            }
            SelectionChange::Clear => tab.selection.clear(),
        }
        if tab.selection == before {
            return None;
        }
        Some(
            tab.row_ids()
                .into_iter()
                .filter(|id| tab.selection.contains(id))
                .collect(),
        )
    }

    /// Opens a tab of `kind` (a console or a comparison) titled `title` at
    /// `location`, next to the pane's active tab and active in its place;
    /// the UI draws it from the view the action returned. `None` when the
    /// pane does not exist.
    pub fn open_view(
        &mut self,
        pane: PaneId,
        kind: TabKind,
        location: Location,
        title: String,
    ) -> Option<TabId> {
        let window = self.window_of(pane)?;
        let (_, new_pane, id) = self.new_workspace(window, location)?;
        if let Some(state) = self.tabs.get_mut(&id) {
            state.kind = kind;
            state.title = Some(title);
        }
        self.focus(new_pane);
        Some(id)
    }

    /// Opens a new workspace with one pane at `location` in the window of
    /// `pane`, or of the focused pane, and shows it. Returns the new pane
    /// and its tab.
    pub fn new_tab(&mut self, pane: Option<PaneId>, location: Location) -> (PaneId, TabId) {
        let pane = pane
            .filter(|p| self.panes.contains_key(p))
            .unwrap_or(self.focused);
        let window = self
            .window_of(pane)
            .or_else(|| {
                self.windows
                    .contains_key(&self.focused_window)
                    .then_some(self.focused_window)
            })
            .or_else(|| self.windows.keys().next().copied());
        if let Some((_, pane, tab)) = window.and_then(|w| self.new_workspace(w, location.clone())) {
            (pane, tab)
        } else {
            // No window at all: one opens for the tab.
            let (_, pane, tab) = self.new_window(location);
            (pane, tab)
        }
    }

    /// Closes a tab with its pane: the pane leaves its workspace's dock, a
    /// workspace left empty closes with the neighbour shown, and a window
    /// left without a workspace closes unless it is the last one, whose
    /// last pane stays. Returns the pane the tab was in, `None` when
    /// nothing closed.
    pub fn close_tab(&mut self, tab: TabId) -> Option<PaneId> {
        let pane = self.pane_of(tab)?;
        if !self.drop_pane(pane) {
            return None;
        }
        self.refollow();
        Some(pane)
    }

    /// Removes a pane with its tab from its workspace; an emptied workspace
    /// closes, and a window emptied with it closes when other windows
    /// exist. The last pane of the last workspace of the last window stays.
    /// Returns whether the pane went.
    fn drop_pane(&mut self, pane: PaneId) -> bool {
        let Some((window, workspace)) = self.workspace_of(pane) else {
            return false;
        };
        let Some(w) = self.windows.get(&window) else {
            return false;
        };
        let Some(ws) = w.workspaces.iter().find(|ws| ws.id == workspace) else {
            return false;
        };
        match ws.dock.clone().without(pane) {
            Some(dock) => {
                let Some(w) = self.windows.get_mut(&window) else {
                    return false;
                };
                let Some(ws) = w.workspace_mut(workspace) else {
                    return false;
                };
                ws.dock = dock;
                if ws.focused == pane {
                    ws.focused = ws.dock.panes().first().copied().unwrap_or(pane);
                }
                let focused = w.focused();
                self.remove_pane(pane);
                if self.focused == pane {
                    self.focused = focused;
                }
                true
            }
            None => {
                if w.workspaces.len() > 1 || self.windows.len() > 1 {
                    self.close_workspace(workspace)
                } else {
                    false
                }
            }
        }
    }

    /// Forgets a pane and its tabs.
    fn remove_pane(&mut self, pane: PaneId) {
        if let Some(pane) = self.panes.remove(&pane) {
            for tab in pane.tabs {
                self.tabs.remove(&tab);
            }
        }
    }

    /// Splits `pane`, opening the same location in a new pane to the right
    /// or below. Returns the new pane and its tab.
    pub fn split(&mut self, pane: PaneId, dir: SplitDir) -> Option<(PaneId, TabId)> {
        let location = self.active_tab(pane)?.1.location.clone();
        let side = match dir {
            SplitDir::Right => Side::Right,
            SplitDir::Down => Side::Bottom,
        };
        let (new, tab) = self.pane_with_tab(location);
        if !self.place_beside(new, pane, side) {
            self.remove_pane(new);
            return None;
        }
        self.focus(new);
        Some((new, tab))
    }

    /// Puts `new` into the dock of `beside`'s workspace, on `side` of it.
    fn place_beside(&mut self, new: PaneId, beside: PaneId, side: Side) -> bool {
        let Some((window, workspace)) = self.workspace_of(beside) else {
            return false;
        };
        let Some(w) = self.windows.get_mut(&window) else {
            return false;
        };
        let Some(ws) = w.workspace_mut(workspace) else {
            return false;
        };
        ws.dock.split(beside, side, new)
    }

    /// Takes a pane out of its workspace's dock without forgetting it: the
    /// first step of a move. An emptied workspace closes, a window emptied
    /// with it too. Returns whether the pane was in a dock.
    fn detach_pane(&mut self, pane: PaneId) -> bool {
        let Some((window, workspace)) = self.workspace_of(pane) else {
            return false;
        };
        let Some(w) = self.windows.get_mut(&window) else {
            return false;
        };
        let Some(ws) = w.workspace_mut(workspace) else {
            return false;
        };
        if let Some(dock) = ws.dock.clone().without(pane) {
            ws.dock = dock;
            if ws.focused == pane {
                ws.focused = ws.dock.panes().first().copied().unwrap_or(pane);
            }
        } else {
            let Some(at) = w.position(workspace) else {
                return false;
            };
            w.workspaces.remove(at);
            if w.active == workspace
                && let Some(next) = w.neighbour(at)
            {
                w.active = next;
            }
            self.drop_empty_window(window);
        }
        true
    }

    /// Moves a tab with its pane: beside another pane (of any window), into
    /// a new workspace of its window, or into a window of its own. A
    /// workspace or window emptied by the move closes. Returns whether the
    /// move happened.
    pub fn move_tab(&mut self, tab: TabId, to: DockTarget) -> bool {
        let Some(from) = self.pane_of(tab) else {
            return false;
        };
        let Some((window, workspace)) = self.workspace_of(from) else {
            return false;
        };
        let alone_in_workspace = self
            .windows
            .get(&window)
            .and_then(|w| w.workspace_of(from))
            .is_some_and(|ws| ws.dock == Dock::Pane(from));
        let alone_in_window = alone_in_workspace
            && self
                .windows
                .get(&window)
                .is_some_and(|w| w.workspaces.len() == 1);
        match to {
            DockTarget::Beside { pane, side } => {
                if pane == from {
                    return true;
                }
                if !self.panes.contains_key(&pane) || !self.detach_pane(from) {
                    return false;
                }
                if !self.place_beside(from, pane, side) {
                    // The target is in no dock: a workspace of its own
                    // instead, so the pane is not lost.
                    let ws = self.workspace_for(from);
                    let id = ws.id;
                    if let Some(w) = self.windows.get_mut(&window) {
                        w.workspaces.push(ws);
                    }
                    self.activate_workspace(id);
                    return true;
                }
                self.focus(from);
                true
            }
            DockTarget::NewWorkspace => {
                if alone_in_workspace {
                    return true;
                }
                if !self.detach_pane(from) {
                    return false;
                }
                let ws = self.workspace_for(from);
                let id = ws.id;
                let Some(w) = self.windows.get_mut(&window) else {
                    return false;
                };
                let at = w.position(workspace).map_or(w.workspaces.len(), |i| i + 1);
                w.workspaces.insert(at.min(w.workspaces.len()), ws);
                self.activate_workspace(id);
                true
            }
            DockTarget::NewWindow => {
                if alone_in_window {
                    // A window alone with this pane is one already.
                    return true;
                }
                let geometry = self.offset_geometry(window);
                if !self.detach_pane(from) {
                    return false;
                }
                let ws = self.workspace_for(from);
                let id = ws.id;
                let new_window = WindowId(self.next_window);
                self.next_window += 1;
                self.windows
                    .insert(new_window, Window::one(ws, geometry, SidePanel::default()));
                self.activate_workspace(id);
                true
            }
        }
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
        let filter = tab.filter_text();
        let types = tab.types.clone();
        let spec = tab.spec_with(&settings, &filter, types.as_ref());
        tab.listing.extend(batch, &spec);
        tab.rebuild_tree();
        true
    }

    /// Empties the pane's active tab's listing before a fresh one streams
    /// in; the selection and filter stay, and so does the set of expanded
    /// folders, whose children are listed again. Returns whether a tab took
    /// it.
    pub fn clear_listing(&mut self, pane: PaneId) -> bool {
        match self.active_tab_mut(pane) {
            Some((_, tab)) => {
                tab.listing.clear();
                tab.tree.clear_children();
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
        let filter = tab.filter_text();
        let types = tab.types.clone();
        let spec = tab.spec_with(&settings, &filter, types.as_ref());
        tab.listing.clear();
        tab.listing.extend(entries, &spec);
        tab.listing.complete = true;
        // Folders that are gone leave the expanded set.
        let gone: Vec<EntryId> = tab
            .tree
            .expanded
            .iter()
            .copied()
            .filter(|id| tab.entry(*id).is_none())
            .collect();
        for id in gone {
            tab.tree.collapse(id);
        }
        let listing = &tab.listing;
        let tree = &tab.tree;
        tab.selection
            .retain(|id| listing.get(*id).is_some() || tree.child(*id).is_some());
        tab.rebuild_tree();
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
                tab.rebuild_tree();
                true
            }
            None => false,
        }
    }

    /// Records why the pane's active tab's listing failed, for the banner
    /// with *Retry*; `None` clears it.
    pub fn listing_failed(&mut self, pane: PaneId, why: Option<String>) -> bool {
        match self.active_tab_mut(pane) {
            Some((_, tab)) => {
                tab.failure = why;
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
        let filter = tab.filter_text();
        let types = tab.types.clone();
        let spec = tab.spec_with(&settings, &filter, types.as_ref());
        match change {
            FsChange::Created(entry) | FsChange::Modified(entry) => {
                tab.listing.upsert(entry, &spec);
            }
            FsChange::Removed(id) => {
                tab.listing.remove(id);
                tab.selection.remove(&id);
                tab.decorations.remove(&id);
                tab.tree.collapse(id);
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
        tab.rebuild_tree();
        true
    }

    /// Records a job.
    pub fn job_started(&mut self, id: JobId, name: String, pausable: bool) {
        self.jobs.insert(
            id,
            JobInfo {
                name,
                state: JobState::Running(Progress::Items {
                    done: 0,
                    total: None,
                }),
                pausable,
                paused: false,
            },
        );
    }

    /// Records a job's progress; unknown jobs are added without a name. A
    /// job that waits for its volume keeps waiting: its first report comes
    /// once it runs.
    pub fn job_progress(&mut self, id: JobId, progress: Progress) {
        let info = self.jobs.entry(id).or_insert_with(|| JobInfo {
            name: String::new(),
            state: JobState::Running(progress.clone()),
            pausable: false,
            paused: false,
        });
        if matches!(info.state, JobState::Running(_)) {
            info.state = JobState::Running(progress);
        }
    }

    /// Records that a job queued behind `waiting_for`, another job on a
    /// volume it shares, or that it runs now (`None`). A finished job is
    /// left alone.
    pub fn job_waiting(&mut self, id: JobId, waiting_for: Option<String>) -> bool {
        let Some(info) = self.jobs.get_mut(&id) else {
            return false;
        };
        match (&info.state, waiting_for) {
            (JobState::Running(_) | JobState::Waiting(_), Some(name)) => {
                info.state = JobState::Waiting(name);
                true
            }
            (JobState::Waiting(_), None) => {
                info.state = JobState::Running(Progress::Items {
                    done: 0,
                    total: None,
                });
                true
            }
            _ => false,
        }
    }

    /// Replaces the Places list; `true` when it differs from the old one.
    pub fn set_places(&mut self, rows: Vec<Place>) -> bool {
        if self.places.rows() == rows.as_slice() {
            return false;
        }
        self.places.set(rows);
        true
    }

    /// Records that a job was paused or resumed.
    pub fn job_paused(&mut self, id: JobId, paused: bool) -> bool {
        match self.jobs.get_mut(&id) {
            Some(info) if info.pausable && matches!(info.state, JobState::Running(_)) => {
                info.paused = paused;
                true
            }
            _ => false,
        }
    }

    /// Records how a job ended.
    pub fn job_finished(&mut self, id: JobId, state: JobState) {
        let info = self.jobs.entry(id).or_insert_with(|| JobInfo {
            name: String::new(),
            state: JobState::Done,
            pausable: false,
            paused: false,
        });
        info.state = state;
        info.paused = false;
        self.conflicts.retain(|c| c.job != id);
    }

    /// Records a conflict a job waits on.
    pub fn conflict_opened(&mut self, conflict: Conflict) {
        if !self.conflicts.iter().any(|c| c.id == conflict.id) {
            self.conflicts.push(conflict);
        }
    }

    /// Forgets an answered conflict. Returns whether it was open.
    pub fn conflict_closed(&mut self, id: ConflictId) -> bool {
        let before = self.conflicts.len();
        self.conflicts.retain(|c| c.id != id);
        before != self.conflicts.len()
    }

    /// Records a question an extension waits on.
    pub fn question_opened(&mut self, question: Question) {
        if !self.questions.iter().any(|q| q.id == question.id) {
            self.questions.push(question);
        }
    }

    /// Forgets an answered question. Returns whether it was open.
    pub fn question_closed(&mut self, id: QuestionId) -> bool {
        let before = self.questions.len();
        self.questions.retain(|q| q.id != id);
        before != self.questions.len()
    }

    /// Forgets a finished job.
    pub fn job_dismiss(&mut self, id: JobId) -> bool {
        self.jobs.remove(&id).is_some()
    }
}

/// What [`Model::dock`] answers for a model without a window.
static EMPTY_DOCK: Dock = Dock::Pane(PaneId(0));

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
    fn panes_split_move_and_close_through_the_dock() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        let window = model.focused_window;
        let pane = model.focused;
        let home = model.workspaces(window)[0].id;
        let (right, right_tab) = model
            .split(pane, SplitDir::Right)
            .unwrap_or_else(|| panic!("split"));
        assert_eq!(model.dock().panes(), [pane, right]);
        assert_eq!(model.focused, right);
        assert_eq!(
            model.tab(right_tab).map(|t| t.location.clone()),
            Some(loc("/a"))
        );

        // A new tab is a new workspace, shown, right after the one shown.
        let (extra_pane, extra) = model.new_tab(Some(pane), loc("/b"));
        assert_eq!(model.pane(pane).map(|p| p.tabs().len()), Some(1));
        assert_eq!(model.workspaces(window).len(), 2);
        let second = model.workspaces(window)[1].id;
        assert_eq!(model.window(window).map(|w| w.active), Some(second));
        assert_eq!(model.dock(), &Dock::Pane(extra_pane));
        assert_eq!(model.focused, extra_pane);

        // Its pane docks beside a pane of the other workspace; the emptied
        // workspace closes and the target's is shown.
        assert!(model.move_tab(
            extra,
            DockTarget::Beside {
                pane,
                side: Side::Top
            }
        ));
        assert_eq!(model.workspaces(window).len(), 1);
        assert_eq!(model.dock().panes().len(), 3);
        assert_eq!(model.focused, extra_pane);
        // And out again into a workspace of its own, after the shown one.
        assert!(model.move_tab(extra, DockTarget::NewWorkspace));
        assert_eq!(model.dock(), &Dock::Pane(extra_pane));
        assert_eq!(model.workspaces(window)[0].id, home);
        assert_eq!(model.workspaces(window)[0].dock.panes(), [pane, right]);
        // A pane alone in its workspace stays where it is.
        assert!(model.move_tab(extra, DockTarget::NewWorkspace));
        assert_eq!(model.workspaces(window).len(), 2);

        assert_eq!(model.close_tab(right_tab), Some(right));
        assert_eq!(
            model.workspaces(window)[0].dock,
            Dock::Pane(pane),
            "the emptied pane is gone"
        );
        assert!(model.pane(right).is_none());
        // Closing the one pane of a workspace closes the workspace; the
        // neighbour shows.
        assert_eq!(model.close_tab(extra), Some(extra_pane));
        assert_eq!(model.workspaces(window).len(), 1);
        assert_eq!(model.focused, pane);
        assert_eq!(*model.dock(), Dock::Pane(pane));
        // The last pane of the last workspace of the last window stays.
        assert_eq!(model.close_tab(TabId(1)), None);
        assert_eq!(model.pane(pane).map(|p| p.tabs().len()), Some(1));
        assert!(!model.move_tab(
            TabId(99),
            DockTarget::Beside {
                pane,
                side: Side::Left
            }
        ));
    }

    #[test]
    fn a_workspace_of_one_pane_docks_into_another_layout_and_one_of_several_does_not() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        let window = model.focused_window;
        let pane_a = model.focused;
        let first = model.workspaces(window)[0].id;
        let (second, pane_b, _) = model
            .new_workspace(window, loc("/b"))
            .unwrap_or_else(|| panic!("workspace"));
        let (third, pane_c, _) = model
            .new_workspace(window, loc("/c"))
            .unwrap_or_else(|| panic!("workspace"));
        let (pane_d, _) = model
            .split(pane_c, SplitDir::Right)
            .unwrap_or_else(|| panic!("split"));
        assert!(model.activate_workspace(first));

        // Exactly one pane, or none to speak of.
        assert_eq!(model.sole_pane(first), Some(pane_a));
        assert_eq!(model.sole_pane(second), Some(pane_b));
        assert_eq!(model.sole_pane(third), None, "two panes");
        assert_eq!(model.sole_pane(WorkspaceId(99)), None);

        // A workspace of several panes is not docked, nor one onto itself,
        // nor onto a pane that is not there.
        let before = model.clone();
        assert!(!model.dock_workspace(third, pane_a, Side::Left));
        assert!(!model.dock_workspace(second, pane_b, Side::Left));
        assert!(!model.dock_workspace(second, PaneId(99), Side::Left));
        assert!(!model.dock_workspace(WorkspaceId(99), pane_a, Side::Left));
        assert_eq!(model.workspaces(window), before.workspaces(window));

        // The one pane of the second workspace, docked left of the first's:
        // the second leaves the row, the first holds both and is shown, and
        // the docked pane has the keyboard.
        assert!(model.dock_workspace(second, pane_a, Side::Left));
        let order: Vec<WorkspaceId> = model.workspaces(window).iter().map(|w| w.id).collect();
        assert_eq!(order, [first, third]);
        assert_eq!(
            model.workspaces(window)[0].dock,
            Dock::split_of(SplitDir::Right, Dock::Pane(pane_b), Dock::Pane(pane_a))
        );
        assert_eq!(model.window(window).map(|w| w.active), Some(first));
        assert_eq!(model.focused, pane_b);
        assert_eq!(model.sole_pane(first), None, "it holds two now");

        // Into a layout that is not shown: it is shown afterwards. The
        // first workspace is down to one pane again after its other pane
        // goes below a pane of the third.
        assert_eq!(
            model.close_tab(
                model
                    .pane(pane_b)
                    .and_then(Pane::active)
                    .unwrap_or(TabId(0))
            ),
            Some(pane_b)
        );
        assert_eq!(model.sole_pane(first), Some(pane_a));
        assert!(model.dock_workspace(first, pane_d, Side::Bottom));
        let order: Vec<WorkspaceId> = model.workspaces(window).iter().map(|w| w.id).collect();
        assert_eq!(order, [third]);
        assert_eq!(model.dock().panes(), [pane_c, pane_d, pane_a]);
        assert_eq!(model.focused, pane_a);
    }

    #[test]
    fn workspaces_open_switch_reorder_and_close() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        let window = model.focused_window;
        let pane_a = model.focused;
        let first = model.workspaces(window)[0].id;
        let (second, pane_b, tab_b) = model
            .new_workspace(window, loc("/b"))
            .unwrap_or_else(|| panic!("workspace"));
        let (third, pane_c, _) = model
            .new_workspace(window, loc("/c"))
            .unwrap_or_else(|| panic!("workspace"));
        assert_eq!(model.workspaces(window).len(), 3);
        assert_eq!(model.focused, pane_c);
        assert_eq!(model.workspace_of(pane_b), Some((window, second)));
        assert_eq!(model.all_panes(), [pane_a, pane_b, pane_c]);
        assert_eq!(
            model.tab(tab_b).map(|t| t.location.clone()),
            Some(loc("/b"))
        );

        // Showing a workspace gives its pane the keyboard; focusing a pane
        // shows its workspace.
        assert!(model.activate_workspace(second));
        assert_eq!(model.focused, pane_b);
        assert_eq!(model.dock(), &Dock::Pane(pane_b));
        assert!(model.focus(pane_c));
        assert_eq!(model.window(window).map(|w| w.active), Some(third));
        assert!(!model.activate_workspace(WorkspaceId(99)));

        // Reordering.
        assert!(model.move_workspace(third, 0));
        let order: Vec<WorkspaceId> = model.workspaces(window).iter().map(|w| w.id).collect();
        assert_eq!(order, [third, first, second]);
        assert!(!model.move_workspace(third, 0), "already there");
        assert!(model.move_workspace(third, 9), "clamped to the end");
        let order: Vec<WorkspaceId> = model.workspaces(window).iter().map(|w| w.id).collect();
        assert_eq!(order, [first, second, third]);

        // Closing the shown one shows the one after it, else before it.
        assert!(model.activate_workspace(second));
        assert!(model.close_workspace(second));
        assert!(model.pane(pane_b).is_none());
        assert!(model.tab(tab_b).is_none());
        assert_eq!(model.window(window).map(|w| w.active), Some(third));
        assert_eq!(model.focused, pane_c);
        assert!(model.close_workspace(third));
        assert_eq!(model.window(window).map(|w| w.active), Some(first));
        assert_eq!(model.focused, pane_a);
        assert!(!model.close_workspace(first), "the last one stays");
        assert!(!model.close_workspace(second), "gone already");
    }

    #[test]
    fn windows_open_close_and_tear_out() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        let first = model.focused_window;
        let pane_a = model.focused;
        let (extra_pane, extra) = model.new_tab(Some(pane_a), loc("/b"));
        let (second, pane_b, tab_b) = model.new_window(loc("/c"));
        assert_eq!(model.focused_window, second);
        assert_eq!(model.focused, pane_b);
        assert_eq!(model.window_of(pane_a), Some(first));
        assert_eq!(model.window_of(pane_b), Some(second));
        assert_eq!(model.all_panes(), [pane_a, extra_pane, pane_b]);

        // A pane docks beside a pane of another window; the emptied window
        // closes.
        assert!(model.move_tab(
            tab_b,
            DockTarget::Beside {
                pane: pane_a,
                side: Side::Right
            }
        ));
        assert!(model.window(second).is_none(), "emptied window closed");
        assert_eq!(model.focused_window, first);
        assert_eq!(model.focused, pane_b);
        assert_eq!(model.dock().panes(), [pane_a, pane_b]);

        // Tear-out: a pane into a window of its own, as its one workspace.
        assert!(model.move_tab(extra, DockTarget::NewWindow));
        assert_eq!(model.windows().count(), 2);
        let torn = model.focused_window;
        assert_ne!(torn, first);
        assert_eq!(model.focused, extra_pane);
        assert_eq!(
            model.workspaces(first).len(),
            1,
            "its workspace went with it"
        );
        assert_eq!(model.workspaces(torn).len(), 1);
        // A pane alone in its window is torn out already.
        assert!(model.move_tab(extra, DockTarget::NewWindow));
        assert_eq!(model.windows().count(), 2);

        // A whole workspace moves between windows and into a window of its
        // own; an emptied window closes.
        let home = model.workspaces(first)[0].id;
        let torn_ws = model.workspaces(torn)[0].id;
        assert!(model.workspace_to_window(torn_ws, Some(first)));
        assert_eq!(model.windows().count(), 1);
        assert_eq!(model.workspaces(first).len(), 2);
        assert_eq!(model.window(first).map(|w| w.active), Some(torn_ws));
        assert!(
            !model.workspace_to_window(torn_ws, Some(first)),
            "there already"
        );
        assert!(model.workspace_to_window(home, None));
        assert_eq!(model.windows().count(), 2);
        assert_eq!(
            model.focused, pane_b,
            "the workspace keeps its focused pane"
        );
        assert!(model.workspace_to_window(home, None), "alone already");
        assert!(!model.workspace_to_window(WorkspaceId(99), None));

        // Closing the last tab of a window closes the window; the last
        // window never closes.
        assert!(model.close_tab(extra).is_some());
        assert_eq!(model.windows().count(), 1);
        assert!(
            !model.close_window(model.focused_window),
            "the last window stays"
        );
        assert!(model.focus(pane_a));
        assert!(!model.focus(PaneId(99)));
    }

    #[test]
    fn a_window_from_a_saved_layout_closes_and_gives_the_focus_back() {
        let mut model = Model::new(Settings::default(), loc("/home"));
        // The way the application opens: a tab at the start location, the
        // one at home closed.
        let (pane_1, _) = model.new_tab(None, loc("/a"));
        assert!(model.close_tab(TabId(1)).is_some());
        let first = model.focused_window;
        assert_eq!(model.focused, pane_1);

        let (second, pane_2, _) = model.new_window(loc("/a"));
        let (pane_3, _) = model.new_tab(None, loc("/a/x"));
        assert_eq!(model.window_of(pane_3), Some(second));
        let saved = model.session();
        let layout = saved.windows[saved.focused_window].clone();
        let third = model.add_window(&layout, &loc("/home"));
        assert_eq!(model.focused_window, third);
        assert_eq!(model.workspaces(third).len(), 2);
        assert!(model.close_window(third));
        assert_eq!(model.windows().count(), 2);
        assert!(model.focus_window(first));
        assert_eq!(model.focused, pane_1);
        assert!(model.focus_window(second));
        assert_eq!(model.focused, pane_3);
        assert!(model.focus(pane_2));
        assert_eq!(model.focused, pane_2);
    }

    #[test]
    fn the_preview_tab_is_one_per_window_and_follows_the_focused_explorer() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        let window = model.focused_window;
        let pane = model.focused;
        assert_eq!(model.preview_follows(), Some(TabId(1)));
        assert_eq!(model.preview_tab(window), None);

        let preview = model
            .open_preview(window, 0.7)
            .unwrap_or_else(|| panic!("preview"));
        assert_eq!(
            model.open_preview(window, 0.3),
            Some(preview),
            "one per window"
        );
        assert_eq!(model.preview_tab(window), Some(preview));
        assert_eq!(model.tab(preview).map(|t| t.kind), Some(TabKind::Preview));
        let preview_pane = model.pane_of(preview).unwrap_or_else(|| panic!("pane"));
        assert_ne!(preview_pane, pane);
        assert!(
            matches!(
                model.dock(),
                Dock::Split { dir: SplitDir::Right, fraction, first, second }
                    if (*fraction - 0.7).abs() < 1e-6
                        && **first == Dock::Pane(pane)
                        && **second == Dock::Pane(preview_pane)
            ),
            "{:?}",
            model.dock()
        );
        assert_eq!(model.focused, pane, "opening the preview moves no focus");

        // The preview pane focused: the explorer stays followed. Another
        // explorer focused: followed instead.
        assert!(model.focus(preview_pane));
        assert_eq!(model.preview_follows(), Some(TabId(1)));
        let (_, second) = model.new_tab(Some(pane), loc("/b"));
        assert_eq!(model.preview_follows(), Some(second));
        // Toggled on from the other workspace, the one preview of the
        // window moves there.
        assert_eq!(model.workspaces(window).len(), 2);
        let moved = model
            .open_preview(window, 0.7)
            .unwrap_or_else(|| panic!("preview"));
        assert_ne!(moved, preview);
        assert_eq!(model.preview_tab(window), Some(moved));
        assert_eq!(model.dock().panes().len(), 2);
        assert_eq!(
            model.workspaces(window)[0].dock,
            Dock::Pane(pane),
            "gone from the first workspace"
        );
        assert!(model.close_tab(moved).is_some());
        assert!(model.activate(TabId(1)));
        assert_eq!(model.preview_follows(), Some(TabId(1)));
        let preview = model
            .open_preview(window, 0.7)
            .unwrap_or_else(|| panic!("preview"));
        let preview_pane = model.pane_of(preview).unwrap_or_else(|| panic!("pane"));

        // The session keeps the kind and the follow comes back with the
        // focus.
        let session = model.session();
        let restored = Model::from_session(Settings::default(), &session, &loc("/home"));
        let restored_preview = restored
            .preview_tab(restored.focused_window)
            .unwrap_or_else(|| panic!("restored preview"));
        assert_eq!(
            restored.tab(restored_preview).map(|t| t.kind),
            Some(TabKind::Preview)
        );
        assert!(
            restored
                .preview_follows()
                .and_then(|t| restored.tab(t))
                .is_some_and(|t| t.kind == TabKind::Explorer && t.location == loc("/a"))
        );

        // Closing the tab collapses its split.
        assert_eq!(model.close_tab(preview), Some(preview_pane));
        assert_eq!(model.preview_tab(window), None);
        assert_eq!(model.dock(), &Dock::Pane(pane));

        // The follow moves with the focus across windows and never names a
        // closed tab.
        let (other, _, third) = model.new_window(loc("/c"));
        assert_eq!(model.preview_follows(), Some(third));
        assert!(model.close_window(other));
        assert_eq!(model.preview_follows(), Some(TabId(1)));
    }

    #[test]
    fn sessions_round_trip_every_detail() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        let pane = model.focused;
        model.listing_batch(pane, vec![entry(1, "x"), entry(2, "y")]);
        model.select(
            pane,
            SelectionChange::Set(vec![EntryId::new(ProviderId("t"), 2)]),
        );
        model.set_sort(pane, Column::Size, SortDir::Descending);
        model.set_filter(pane, "x");
        model.set_view(pane, ViewMode::Icons(IconSize::Large));
        model.set_group(pane, Some(Column::Type));
        model.toggle_hidden(pane);
        assert!(model.set_scroll(TabId(1), 120.5));
        let (right, right_tab) = model
            .split(pane, SplitDir::Right)
            .unwrap_or_else(|| panic!("split"));
        let (third_pane, _) = model.new_tab(Some(right), loc("/b"));
        let window = model.focused_window;
        assert_eq!(model.focused, third_pane, "the new workspace is shown");
        assert!(model.focus(pane));
        let mut layout = model.dock().clone();
        if let Dock::Split { fraction, .. } = &mut layout {
            *fraction = 0.3;
        }
        assert!(model.set_layout(window, layout));
        assert!(!model.set_layout(window, Dock::Pane(pane)), "shape differs");
        assert!(model.set_geometry(
            window,
            Geometry {
                x: 10.0,
                y: 20.0,
                width: 800.0,
                height: 600.0,
                maximised: false,
            }
        ));
        assert!(model.side_panel(window, SidePanelChange::Visible(false)));
        assert!(model.side_panel(
            window,
            SidePanelChange::Fold {
                section: Section::Network,
                folded: true
            }
        ));
        model.activate(right_tab);
        model.focus(pane);

        let session = model.session();
        let restored = Model::from_session(Settings::default(), &session, &loc("/home"));
        assert_eq!(restored.session(), session, "a round trip changes nothing");
        assert_eq!(restored.windows().count(), 1);
        let rwindow = restored.focused_window;
        let w = restored.window(rwindow).unwrap_or_else(|| panic!("window"));
        assert!(!w.side_panel.visible);
        assert!(w.side_panel.folded.contains(&Section::Network));
        assert_eq!(w.geometry.map(|g| g.width), Some(800.0));
        assert_eq!(w.workspaces.len(), 2);
        assert_eq!(
            w.active, w.workspaces[0].id,
            "the shown workspace is shown again"
        );
        assert!(matches!(w.dock(), Dock::Split { fraction, .. } if (fraction - 0.3).abs() < 1e-6));
        let panes = w.dock().panes();
        assert_eq!(panes.len(), 2);
        assert_eq!(restored.focused, panes[0]);
        let (_, tab) = restored
            .active_tab(panes[0])
            .unwrap_or_else(|| panic!("tab"));
        assert_eq!(tab.location, loc("/a"));
        assert_eq!(tab.sort.by, Column::Size);
        assert_eq!(tab.sort.dir, SortDir::Descending);
        assert_eq!(tab.filter, "x");
        assert_eq!(tab.view, ViewMode::Icons(IconSize::Large));
        assert_eq!(tab.group, Some(Column::Type));
        assert!(tab.show_hidden);
        assert!((tab.scroll - 120.5).abs() < f32::EPSILON);
        assert!(tab.selection.contains(&EntryId::new(ProviderId("t"), 2)));
        let right_pane = restored.pane(panes[1]).unwrap_or_else(|| panic!("pane"));
        assert_eq!(right_pane.tabs().len(), 1);
        let other = &w.workspaces[1];
        assert_eq!(
            restored
                .active_tab(other.focused)
                .map(|(_, t)| t.location.clone()),
            Some(loc("/b")),
            "the second workspace came back"
        );

        // An empty session gets one window at the fallback.
        let empty = Model::from_session(Settings::default(), &Session::default(), &loc("/home"));
        assert_eq!(empty.windows().count(), 1);
        assert_eq!(
            empty
                .active_tab(empty.focused)
                .map(|(_, t)| t.location.clone()),
            Some(loc("/home"))
        );
        // A dock naming a pane the session does not have is laid out anew.
        let mut odd = session.clone();
        odd.windows[0].workspaces[0].dock = DockSession::Pane(7);
        let odd = Model::from_session(Settings::default(), &odd, &loc("/home"));
        assert_eq!(odd.dock().panes().len(), 2);
    }

    #[test]
    fn a_session_of_0_2_opens_its_further_tabs_as_workspaces() {
        // 0.2 held the one layout in the window, and a pane had a tab strip.
        let tab = |path: &str| TabSession {
            location: loc(path),
            ..TabSession::default()
        };
        let old = Session {
            focused_window: 0,
            windows: vec![WindowSession {
                focused_pane: 1,
                dock: DockSession::Split {
                    dir: SplitDir::Right,
                    fraction: 0.4,
                    first: Box::new(DockSession::Pane(0)),
                    second: Box::new(DockSession::Pane(1)),
                },
                panes: vec![
                    PaneSession {
                        active: Some(1),
                        tabs: vec![tab("/a"), tab("/b"), tab("/c")],
                    },
                    PaneSession {
                        active: None,
                        tabs: vec![tab("/d")],
                    },
                ],
                ..WindowSession::default()
            }],
        };
        let model = Model::from_session(Settings::default(), &old, &loc("/home"));
        let window = model.focused_window;
        let workspaces = model.workspaces(window);
        assert_eq!(workspaces.len(), 3, "the layout and one per further tab");
        let at = |pane: PaneId| {
            model
                .active_tab(pane)
                .map_or_else(|| panic!("tab of {pane:?}"), |(_, t)| t.location.clone())
        };
        let first = workspaces[0].dock.panes();
        assert_eq!(first.len(), 2);
        assert_eq!(
            at(first[0]),
            loc("/b"),
            "the active tab stays in the layout"
        );
        assert_eq!(at(first[1]), loc("/d"));
        assert_eq!(model.focused, first[1]);
        assert_eq!(at(workspaces[1].dock.panes()[0]), loc("/a"));
        assert_eq!(at(workspaces[2].dock.panes()[0]), loc("/c"));
        assert!(
            matches!(workspaces[0].dock, Dock::Split { fraction, .. } if (fraction - 0.4).abs() < 1e-6)
        );
        // Written again, the file holds workspaces and none of the old keys.
        let again = model.session();
        assert_eq!(again.windows[0].panes, Vec::new());
        assert_eq!(again.windows[0].workspaces.len(), 3);
        assert_eq!(again.windows[0].active_workspace, 0);
    }

    #[test]
    fn jobs_and_settings() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        model.job_started(JobId(1), "copy".into(), true);
        assert!(model.job_paused(JobId(1), true));
        assert!(model.jobs.get(&JobId(1)).is_some_and(|j| j.paused));
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

    fn dir(id: u128, path: &str) -> Entry {
        Entry {
            kind: EntryKind::Dir,
            path: VPath::from(path),
            ..entry(id, path.rsplit('/').next().unwrap_or(path))
        }
    }

    #[test]
    fn the_tree_view_expands_folders_in_place_and_keeps_them_across_a_refresh() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        let pane = model.focused;
        model.listing_batch(
            pane,
            vec![dir(1, "/a/one"), entry(2, "file"), dir(3, "/a/two")],
        );
        model.listing_done(pane);
        model.set_view(pane, ViewMode::Tree);
        let one = EntryId::new(ProviderId("t"), 1);
        let two = EntryId::new(ProviderId("t"), 3);
        assert!(model.tree_expand(pane, one), "first expand lists");
        assert!(!model.tree_expand(pane, one), "already expanded");
        assert!(
            !model.tree_expand(pane, EntryId::new(ProviderId("t"), 2)),
            "files do not expand"
        );
        assert!(active(&model, pane).tree.is_loading(one));
        assert!(model.tree_batch(pane, one, vec![dir(11, "/a/one/inner"), entry(12, "leaf")]));
        assert!(model.tree_done(pane, one));
        let names = |m: &Model| -> Vec<(String, u16)> {
            let tab = active(m, pane);
            (0..tab.row_count())
                .filter_map(|i| tab.row(i).map(|e| (e.name.clone(), tab.depth(i))))
                .collect()
        };
        assert_eq!(
            names(&model),
            [
                ("one".to_owned(), 0),
                ("inner".to_owned(), 1),
                ("leaf".to_owned(), 1),
                ("two".to_owned(), 0),
                ("file".to_owned(), 0)
            ]
        );
        // A nested folder expands too; Select all takes the children.
        let inner = EntryId::new(ProviderId("t"), 11);
        assert!(model.tree_expand(pane, inner));
        model.tree_batch(pane, inner, vec![entry(111, "deep")]);
        assert_eq!(active(&model, pane).row_count(), 6);
        model.select(pane, SelectionChange::All);
        assert_eq!(active(&model, pane).selection.len(), 6);
        assert_eq!(model.tree_expanded(pane).len(), 2);

        // A refresh keeps the expanded set and relists the children.
        model.clear_listing(pane);
        assert!(active(&model, pane).tree.expanded.contains(&one));
        model.listing_batch(
            pane,
            vec![dir(1, "/a/one"), entry(2, "file"), dir(3, "/a/two")],
        );
        model.listing_done(pane);
        assert_eq!(
            model.tree_expanded(pane).len(),
            1,
            "the nested folder is gone with its parent's listing"
        );
        assert!(model.tree_replace(pane, one, vec![entry(12, "leaf")]));
        assert_eq!(names(&model).len(), 4);
        // Collapse drops the children and their selection; the set shrinks.
        assert!(model.tree_collapse(pane, one));
        assert!(!model.tree_collapse(pane, one));
        assert_eq!(names(&model).len(), 3);
        assert!(active(&model, pane).selection.iter().all(|id| *id != inner));
        assert!(model.tree_expand(pane, two));
        model.navigate(pane, loc("/b"));
        assert!(
            active(&model, pane).tree.expanded.is_empty(),
            "navigation clears the tree"
        );
        // Outside Tree view the rows are the listing's.
        model.set_view(pane, ViewMode::Details);
        model.listing_batch(pane, vec![dir(5, "/b/x")]);
        assert_eq!(active(&model, pane).row_count(), 1);
        assert_eq!(active(&model, pane).depth(0), 0);
    }

    #[test]
    fn subtree_search_and_conflicts() {
        let mut model = Model::new(Settings::default(), loc("/a"));
        let pane = model.focused;
        model.listing_batch(pane, vec![entry(1, "foo"), entry(2, "bar")]);
        model.set_filter(pane, "zzz");
        assert_eq!(active(&model, pane).row_count(), 0);
        assert!(model.set_subtree(pane, true));
        assert!(!model.set_subtree(pane, true));
        assert!(active(&model, pane).is_searching());
        assert!(
            active(&model, pane).listing.is_empty(),
            "search results replace the listing"
        );
        // Search results are not filtered again by the view.
        model.listing_batch(pane, vec![entry(3, "x/zzz-match")]);
        assert_eq!(active(&model, pane).row_count(), 1);
        assert!(model.set_subtree(pane, false));
        assert!(!active(&model, pane).is_searching());

        let conflict = Conflict {
            id: ConflictId(1),
            job: JobId(7),
            source: loc("/s"),
            target: loc("/t"),
            existing: types::ConflictSide::default(),
            incoming: types::ConflictSide::default(),
            proposed: "t (2)".into(),
            position: None,
            remaining: 0,
        };
        model.conflict_opened(conflict.clone());
        model.conflict_opened(conflict);
        assert_eq!(model.conflicts.len(), 1);
        model.job_finished(JobId(7), JobState::Cancelled);
        assert!(model.conflicts.is_empty(), "a finished job's conflicts go");
        assert!(!model.conflict_closed(ConflictId(1)));
    }
}
