//! The two types that cross the UI boundary.

use std::fmt;

use std::path::PathBuf;

use ext::{ActionCtx, ActionId, Decoration, FsChange, Level, PreviewDoc};
use model::{
    Column, Dock, DockTarget, Geometry, PaneId, Place, SelectionChange, SettingsChange,
    SidePanelChange, SortDir, SplitDir, TabId, ViewMode, WindowId,
};
use types::{
    Conflict, ConflictAnswer, ConflictId, Entry, EntryId, Error, JobId, Location, Progress,
};

/// Whether a transfer moves or copies, or asks on drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMode {
    /// Move: a rename within one provider, copy and remove across providers.
    Move,
    /// Copy through the provider's fast path.
    Copy,
    /// Ask with the move/copy/cancel prompt first; the UI answers it and
    /// sends `Move` or `Copy` in its place.
    Ask,
}

/// What the UI asks the application to do. The UI never touches the model;
/// it sends one of these and re-renders from the model afterwards.
#[derive(Debug, Clone)]
pub enum Command {
    /// Show `to` in the pane's active tab, pushing the current location on
    /// the back stack.
    Navigate {
        /// The pane.
        pane: PaneId,
        /// Where to go.
        to: Location,
    },
    /// Open an entry of the pane's active tab: a folder navigates the pane
    /// to it; a local file the archive provider can browse navigates into
    /// it as a location (`archive:` with the archive as a breadcrumb
    /// segment); any other file opens in a viewer tab of the pane when a
    /// previewer claims it, and with the system's default application
    /// otherwise or when its type is listed in `[viewer] external`. The
    /// archive check and the scoring read the file as a job, so the result
    /// follows a moment later.
    Open {
        /// The pane.
        pane: PaneId,
        /// The entry.
        id: EntryId,
    },
    /// One step back in the pane's history.
    Back(PaneId),
    /// One step forward in the pane's history.
    Forward(PaneId),
    /// List the pane's location again.
    Refresh(PaneId),
    /// Change the pane's selection.
    Select {
        /// The pane.
        pane: PaneId,
        /// The change.
        change: SelectionChange,
    },
    /// Change how the pane shows its entries.
    SetView {
        /// The pane.
        pane: PaneId,
        /// The new mode.
        mode: ViewMode,
    },
    /// Sort the pane's entries.
    Sort {
        /// The pane.
        pane: PaneId,
        /// The column.
        by: Column,
        /// The direction.
        dir: SortDir,
    },
    /// Group the pane's entries under headers, or not.
    GroupBy {
        /// The pane.
        pane: PaneId,
        /// The column, or `None` for a flat list.
        by: Option<Column>,
    },
    /// Set the pane's type-to-filter text.
    Filter {
        /// The pane.
        pane: PaneId,
        /// The text; empty clears the filter.
        text: String,
    },
    /// Flip whether the pane shows hidden entries.
    ToggleHidden(PaneId),
    /// Flatten the pane's subfolders into one list, or not.
    Flatten {
        /// The pane.
        pane: PaneId,
        /// On or off.
        on: bool,
    },
    /// Search in subdirectories: the pane's filter text matches the whole
    /// subtree, the matches streaming in from a cancellable walk, or only
    /// the location's own entries.
    Subtree {
        /// The pane.
        pane: PaneId,
        /// On or off.
        on: bool,
    },
    /// The title-row search box: query every available search index for
    /// `text` by name, after a short pause for the next keystroke, and
    /// stream the hits into the model's [`IndexSearch`](model::IndexSearch)
    /// with their paths; an empty text ends the search. Without an
    /// available index the box searches the current location alone, as in
    /// 0.1 (QUESTIONS.md 118).
    IndexSearch {
        /// What to look for; empty clears.
        text: String,
    },
    /// Tree view: expand a folder of the pane's active tab in place; its
    /// children are listed lazily on the first expand.
    TreeExpand {
        /// The pane.
        pane: PaneId,
        /// The folder.
        id: EntryId,
    },
    /// Tree view: collapse an expanded folder.
    TreeCollapse {
        /// The pane.
        pane: PaneId,
        /// The folder.
        id: EntryId,
    },
    /// Add a row to the side panel's Places list at `at`, or at the end: a
    /// folder dropped on the list, or a location to keep one click away. A
    /// location already listed stays where it is.
    PlaceAdd {
        /// The row.
        place: Place,
        /// The index to insert at; `None` appends.
        at: Option<usize>,
    },
    /// Remove the Places row at this index.
    PlaceRemove(usize),
    /// Move a Places row, by its index, so that it sits at `to`.
    PlaceMove {
        /// The row's index now.
        from: usize,
        /// Its index afterwards.
        to: usize,
    },
    /// Change a Places row's title and icon.
    PlaceEdit {
        /// The row's index.
        index: usize,
        /// The new title; empty keeps the old one.
        title: String,
        /// The new Phosphor icon name; empty keeps the old one.
        icon: String,
    },
    /// Eject the removable volume mounted at this path, as a job.
    Eject(PathBuf),
    /// Run an action on a context, usually from
    /// [`App::action_ctx`](crate::App::action_ctx).
    Run {
        /// Which action.
        action: ActionId,
        /// What it runs on.
        ctx: ActionCtx,
    },
    /// Open a tab at the location in the focused pane.
    NewTab(Location),
    /// Close a tab.
    CloseTab(TabId),
    /// Make a tab its pane's active one.
    ActivateTab(TabId),
    /// Give a pane the keyboard focus.
    FocusPane(PaneId),
    /// Split a pane, opening a new pane with a tab at the same location.
    Split {
        /// The pane.
        pane: PaneId,
        /// To the right or below.
        dir: SplitDir,
    },
    /// Dock a tab somewhere else.
    MoveTab {
        /// The tab.
        tab: TabId,
        /// Where it goes.
        to: DockTarget,
    },
    /// Open the window's Preview tab, in a new pane right of every other,
    /// or close it when it is open. The preview is one tab per window that
    /// follows the focus; it docks and tears out like any other tab.
    TogglePreview {
        /// The window.
        window: WindowId,
        /// The share of the width the existing panes keep when the tab
        /// opens; display state the UI works out from its size.
        fraction: f32,
    },
    /// Cancel a running job.
    CancelJob(JobId),
    /// Pause a running job that supports it.
    PauseJob(JobId),
    /// Resume a paused job.
    ResumeJob(JobId),
    /// Forget a finished job.
    DismissJob(JobId),
    /// Answer the conflict dialogue of a waiting job.
    ResolveConflict {
        /// The conflict.
        id: ConflictId,
        /// Skip, overwrite, rename, cancel; for this item or all remaining.
        answer: ConflictAnswer,
    },
    /// Enumerate the mounted volumes again.
    RefreshVolumes,
    /// Revert the last reversible operation.
    Undo,
    /// Apply the last undone operation again.
    Redo,
    /// Change one setting and save the settings file.
    Settings(SettingsChange),
    /// Change one setting an extension declared and save its TOML file.
    ExtSetting {
        /// The extension's id.
        ext: String,
        /// The setting's key.
        key: String,
        /// The new value.
        value: ext::SettingValue,
    },
    /// Render a thumbnail of an entry of the pane's listing at `size`
    /// pixels square, through the previewer that claims it; answered by
    /// [`Event::Thumbnail`].
    Thumbnail {
        /// The pane whose listing holds the entry.
        pane: PaneId,
        /// The entry.
        id: EntryId,
        /// Width and height in pixels.
        size: u32,
    },
    /// Drag-and-drop (and later Move to / Copy to): move or copy `items`
    /// into `to`. `Ask` opens the move/copy prompt first. Runs as one job;
    /// a move within a provider is a rename, a copy uses the provider's
    /// fast path.
    Transfer {
        /// The entries, from the listing of `from`.
        items: Vec<EntryId>,
        /// The pane the entries were dragged out of.
        from: PaneId,
        /// The folder they go into.
        to: Location,
        /// Move, copy, or ask.
        mode: TransferMode,
    },
    /// A drop from outside the process: the same as [`Command::Transfer`],
    /// but the items are locations, since they are in no pane's listing.
    Receive {
        /// The dropped entries.
        items: Vec<Location>,
        /// The folder they go into.
        to: Location,
        /// Move or copy; `Ask` is answered by the UI before it gets here.
        mode: TransferMode,
    },
    /// Spring-loading step while a drag is in progress: navigates like
    /// `Navigate` but records history only on the first step of a drag
    /// (`first: true`), with the selection of the starting location, so one
    /// Back returns there.
    SpringLoad {
        /// The pane.
        pane: PaneId,
        /// The folder to drill into.
        to: Location,
        /// Whether this is the drag's first step in the pane.
        first: bool,
    },
    /// Pin to the side panel, undoable; `at` is the position in the Pinned
    /// list, `None` the end.
    Pin {
        /// The folders and files to pin, in order.
        items: Vec<Location>,
        /// Where to insert them.
        at: Option<usize>,
    },
    /// Unpin from the side panel, undoable.
    Unpin(Location),
    /// The strip order of a pane's tabs, as the dock shows them; display
    /// state the session keeps.
    TabOrder {
        /// The pane.
        pane: PaneId,
        /// Its tabs, left to right.
        order: Vec<TabId>,
    },
    /// A window's split fractions, as the dock shows them: `dock` has the
    /// window's panes in the window's splits.
    Layout {
        /// The window.
        window: WindowId,
        /// The tree with its fractions.
        dock: Dock,
    },
    /// A tab's list scroll offset, as the UI shows it.
    Scroll {
        /// The tab.
        tab: TabId,
        /// The vertical offset in logical points.
        offset: f32,
    },
    /// Where a window is on the screen.
    WindowGeometry {
        /// The window.
        window: WindowId,
        /// Its position, size and maximised state.
        geometry: Geometry,
    },
    /// Change a window's side panel.
    SidePanel {
        /// The window.
        window: WindowId,
        /// The change.
        change: SidePanelChange,
    },
    /// Give a window the focus, as the system did.
    FocusWindow(WindowId),
    /// Open a window with the start layout.
    NewWindow,
    /// Close a window; closing the last one asks the application to exit
    /// ([`App::exit_requested`](crate::App::exit_requested)).
    CloseWindow(WindowId),
    /// Store the current session as the layout of this name.
    SaveLayout(String),
    /// Replace every window and tab with a saved layout, or with the last
    /// session for `None` ("Continue where I left off").
    SwitchLayout(Option<String>),
    /// Delete a saved layout.
    DeleteLayout(String),
    /// Rename a saved layout.
    RenameLayout {
        /// The current name.
        from: String,
        /// The new name.
        to: String,
    },
}

/// What the application tells the UI happened. The model already reflects
/// it when the event is delivered.
pub enum Event {
    /// A batch of a listing arrived, or the listing finished.
    Listed {
        /// The pane.
        pane: PaneId,
        /// The entries, already in the model.
        batch: Vec<Entry>,
        /// Whether the listing is complete.
        done: bool,
    },
    /// The filesystem changed under a pane.
    Changed {
        /// The pane.
        pane: PaneId,
        /// What changed, already applied to the model.
        change: FsChange,
    },
    /// A decorator produced badges or cells.
    Decorated {
        /// The pane.
        pane: PaneId,
        /// The decorations by entry.
        items: Vec<(EntryId, Decoration)>,
    },
    /// A job reported progress.
    JobProgress {
        /// The job.
        id: JobId,
        /// Its latest progress.
        progress: Progress,
    },
    /// A job ended.
    JobDone {
        /// The job.
        id: JobId,
        /// How it ended.
        result: Result<(), Error>,
    },
    /// Hits of the title-row search box's index search arrived, or the
    /// search finished; the model's `index_search` already holds them.
    IndexSearched {
        /// The new hits, each listed once across the indexes.
        batch: Vec<Entry>,
        /// Whether every index has answered.
        done: bool,
    },
    /// A pane's selection changed.
    SelectionChanged {
        /// The pane.
        pane: PaneId,
        /// The selected ids in view order.
        ids: Vec<EntryId>,
    },
    /// A preview finished opening.
    PreviewReady {
        /// The pane.
        pane: PaneId,
        /// The document.
        doc: Box<dyn PreviewDoc>,
    },
    /// No preview could be opened for the pane's selection: no previewer
    /// claimed the file, or opening it failed.
    PreviewFailed {
        /// The pane.
        pane: PaneId,
        /// Why.
        error: Error,
    },
    /// A thumbnail finished: the frame, or `None` when no previewer claims
    /// the entry or the render failed.
    Thumbnail {
        /// The entry.
        id: EntryId,
        /// The size that was asked for.
        size: u32,
        /// The pixels, when there are any.
        frame: Option<ext::Frame>,
    },
    /// A job met an existing name and waits for the conflict dialogue's
    /// answer; the conflict is in the model until it is resolved.
    Conflict(Conflict),
    /// An action opened a view in a new tab of the pane, which the model
    /// already holds; the UI draws `view` in it.
    ViewOpened {
        /// The pane.
        pane: PaneId,
        /// The new tab.
        tab: TabId,
        /// What to draw.
        view: ext::View,
    },
    /// Something to show the user.
    Notify {
        /// How serious.
        level: Level,
        /// The text.
        text: String,
    },
}

impl fmt::Debug for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Listed { pane, batch, done } => f
                .debug_struct("Listed")
                .field("pane", pane)
                .field("batch", &batch.len())
                .field("done", done)
                .finish(),
            Self::Changed { pane, change } => f
                .debug_struct("Changed")
                .field("pane", pane)
                .field("change", change)
                .finish(),
            Self::Decorated { pane, items } => f
                .debug_struct("Decorated")
                .field("pane", pane)
                .field("items", &items.len())
                .finish(),
            Self::JobProgress { id, progress } => f
                .debug_struct("JobProgress")
                .field("id", id)
                .field("progress", progress)
                .finish(),
            Self::JobDone { id, result } => f
                .debug_struct("JobDone")
                .field("id", id)
                .field("result", result)
                .finish(),
            Self::IndexSearched { batch, done } => f
                .debug_struct("IndexSearched")
                .field("batch", &batch.len())
                .field("done", done)
                .finish(),
            Self::SelectionChanged { pane, ids } => f
                .debug_struct("SelectionChanged")
                .field("pane", pane)
                .field("ids", ids)
                .finish(),
            Self::PreviewReady { pane, .. } => f
                .debug_struct("PreviewReady")
                .field("pane", pane)
                .finish_non_exhaustive(),
            Self::Thumbnail { id, size, frame } => f
                .debug_struct("Thumbnail")
                .field("id", id)
                .field("size", size)
                .field("frame", &frame.is_some())
                .finish(),
            Self::PreviewFailed { pane, error } => f
                .debug_struct("PreviewFailed")
                .field("pane", pane)
                .field("error", error)
                .finish(),
            Self::Conflict(conflict) => f.debug_tuple("Conflict").field(&conflict.id).finish(),
            Self::ViewOpened { pane, tab, view } => f
                .debug_struct("ViewOpened")
                .field("pane", pane)
                .field("tab", tab)
                .field("view", view)
                .finish(),
            Self::Notify { level, text } => f
                .debug_struct("Notify")
                .field("level", level)
                .field("text", text)
                .finish(),
        }
    }
}
