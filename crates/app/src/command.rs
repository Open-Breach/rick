//! The two types that cross the UI boundary.

use std::fmt;

use std::path::PathBuf;
use std::sync::Arc;

use ext::{ActionCtx, ActionId, Continuation, Decoration, FsChange, Level, PreviewDoc, Values};
use model::{
    Column, Dock, DockTarget, Geometry, PaneId, Place, SelectionChange, SettingsChange, Side,
    SidePanelChange, SortDir, SplitDir, TabId, ViewMode, WindowId, WorkspaceId,
};
use ops::PermissionRequest;
use platform::{FilePermissions, Principals};
use types::{
    Answer, Conflict, ConflictAnswer, ConflictId, Entry, EntryId, Error, JobId, Location, Progress,
    ProviderId, Question, QuestionId,
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

/// What the context menu of a connection row in the side panel does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionOp {
    /// Open or verify the connection now, as a job, so a wrong password is
    /// reported at once.
    Connect,
    /// Drop its session; the next operation connects again.
    Disconnect,
    /// Forget it: out of `rick.toml`, its secrets out of the keychain.
    Remove,
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
    /// Open an entry of the pane's active tab, as *Opening files* says: a
    /// folder navigates the pane to it; a local file the archive provider
    /// can browse navigates into it as a location (`archive:` with the
    /// archive as a breadcrumb segment) unless the setting
    /// `archives.on_open` is `system`; every other file opens with the
    /// system's default application. Nothing opens a viewer: the preview
    /// pane is the only one. The archive check reads the file as a job, so
    /// the result follows a moment later.
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
    /// Narrow a pane to the entries of these file types, as the Type
    /// column names them, from the view options menu; `None` shows every
    /// type again. The filter is per pane and clears on navigation.
    TypeFilter {
        /// The pane.
        pane: PaneId,
        /// The types to show, or every type.
        types: Option<std::collections::BTreeSet<String>>,
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
    /// Add a connection of a network or cloud provider, as its *Add
    /// connection…* dialogue filled it in: the fields of
    /// [`ConnectionFieldKind::Secret`](ext::ConnectionFieldKind::Secret) go into the platform
    /// keychain (as a job, since a keychain may ask to be unlocked), the
    /// rest into `rick.toml`, and the provider's connector is told.
    AddConnection {
        /// The provider.
        provider: ProviderId,
        /// The connection's name, unique within the provider.
        name: String,
        /// The field values by key, secrets included; an empty value is
        /// left out.
        values: Vec<(String, String)>,
    },
    /// Change a connection of a network or cloud provider, as its *Edit…*
    /// dialogue filled it in: the non-secret fields replace the ones in
    /// `rick.toml`, a secret given replaces the one in the keychain and a
    /// secret left out keeps it.
    EditConnection {
        /// The provider.
        provider: ProviderId,
        /// The connection's name, which the dialogue cannot change.
        name: String,
        /// The field values by key, secrets included; an empty value is
        /// left out.
        values: Vec<(String, String)>,
    },
    /// Connect, disconnect or remove a connection of a network or cloud
    /// provider.
    Connection {
        /// The provider.
        provider: ProviderId,
        /// The connection's name.
        name: String,
        /// What to do.
        op: ConnectionOp,
    },
    /// Run an action on a context, usually from
    /// [`App::action_ctx`](crate::App::action_ctx).
    Run {
        /// Which action.
        action: ActionId,
        /// What it runs on.
        ctx: ActionCtx,
    },
    /// Run the rest of an action with the values of the form it asked for
    /// once a job of its finished ([`ext::UiHost::form_later`]): what the
    /// UI sends when the user confirms or dismisses that form.
    Continue {
        /// The pane the action ran in.
        pane: PaneId,
        /// The rest of the action.
        then: Continuation,
        /// The form's values, or `None` when the dialogue was dismissed.
        values: Option<Values>,
    },
    /// Open a new workspace tab with one pane at the location, in the
    /// focused pane's window, right after the shown workspace, and show it
    /// (SPEC.md *Workspace tabs*).
    NewTab(Location),
    /// Close a tab with its pane; a workspace left empty closes with it.
    CloseTab(TabId),
    /// Give a tab's pane the keyboard focus, showing its workspace.
    ActivateTab(TabId),
    /// Give a pane the keyboard focus, showing its workspace.
    FocusPane(PaneId),
    /// Show a workspace tab of the title row.
    ActivateWorkspace(WorkspaceId),
    /// Close a workspace with every pane in it; the neighbour shows.
    CloseWorkspace(WorkspaceId),
    /// Move a workspace tab to a position in its window's title row.
    MoveWorkspace {
        /// The workspace.
        id: WorkspaceId,
        /// The index it takes, clamped to the row.
        to: usize,
    },
    /// Dock the one pane of a workspace beside a pane of any layout, the
    /// workspace's tab leaving its row: a tab holding exactly one pane
    /// dropped on a dock target. The model refuses a workspace of several
    /// panes.
    DockWorkspace {
        /// The workspace whose pane moves.
        id: WorkspaceId,
        /// The pane it docks beside.
        pane: PaneId,
        /// On which side of that pane.
        side: Side,
    },
    /// Move a workspace to the end of another window's title row, or into a
    /// window of its own for `None`.
    WorkspaceToWindow {
        /// The workspace.
        id: WorkspaceId,
        /// The window, or `None` for a new one.
        window: Option<WindowId>,
    },
    /// Split a pane, opening a new pane with a tab at the same location.
    Split {
        /// The pane.
        pane: PaneId,
        /// To the right or below.
        dir: SplitDir,
    },
    /// Dock a tab with its pane somewhere else: beside a pane of any
    /// window, in a new workspace or in a new window.
    MoveTab {
        /// The tab.
        tab: TabId,
        /// Where it goes.
        to: DockTarget,
    },
    /// Open the window's Preview tab, in a new pane right of every other
    /// pane of the shown workspace, or close it when it is open there. The
    /// preview is one tab per window that follows the focus; toggled on in
    /// another workspace it moves there; it docks and tears out like any
    /// other tab.
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
    /// Answer the question dialogue of a waiting extension: the host key to
    /// trust, the passphrase or password typed, or the dismissal.
    AnswerQuestion {
        /// The question.
        id: QuestionId,
        /// The answer.
        answer: Answer,
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
    /// A remote file shown as a prefix (`Event::PreviewPrefix`): serve the
    /// next chunk too and open the preview again.
    PreviewMore(PaneId),
    /// A remote file held behind the size cap (`Event::PreviewCapped`):
    /// open the preview whatever its size.
    PreviewAnyway(PaneId),
    /// The cap message's *Always preview on <source>*: switch the cap off
    /// for the source (one SSH host, one cloud account) that holds the
    /// pane's file, `preview_always = true` in its connection, then open
    /// the file.
    PreviewAlways(PaneId),
    /// Turn a source's *Always preview* on or off: the connection row's
    /// context menu in the Network or Cloud section. A source Rick cannot
    /// edit (a host of `~/.ssh/config`) gets a connection entry that
    /// carries the switch alone.
    SetPreviewAlways {
        /// The provider.
        provider: ProviderId,
        /// The connection's name.
        name: String,
        /// On or off.
        on: bool,
    },
    /// Open the pane's preview again, as it is: *Retry* on a missing
    /// third-party library, after it was installed or its setting changed.
    PreviewRetry(PaneId),
    /// The pane's *Copy*: put the whole text of the file the pane previews
    /// on the clipboard as text, through `platform::Clipboard`: the whole
    /// file (read again as a job, not the window the pane has loaded), the
    /// raw source for a rendered format, never the file itself.
    PreviewCopy(PaneId),
    /// Open Properties on the pane's selected entry: the general rows and,
    /// for a local entry, its owner, group, mode bits and ACL; answered by
    /// `Event::Properties`, and again after a change while the dialogue is
    /// open.
    ShowProperties(PaneId),
    /// The Properties dialogue closed; a later change does not report back.
    PropertiesClosed,
    /// The Hash dialogue's format toggle changed: remember the format as
    /// `[hash] format` in `core.toml`, so the next dialogue opens with it.
    HashFormat(ops::hash::Format),
    /// Open the platform's own properties dialogue (Windows Properties,
    /// macOS Get Info) on the pane's selected entry.
    SystemProperties(PaneId),
    /// Change the ownership, mode bits or ACL of entries, as the Properties
    /// dialogue asks: one job with progress, one undo step, and the
    /// privileged helper for what the user may not change by themselves.
    SetPermissions {
        /// The pane the entries are listed in, refreshed when the job ends.
        pane: PaneId,
        /// What to change, on which entries, and how deep.
        request: PermissionRequest,
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
    /// The shown workspace's split fractions, as the dock shows them:
    /// `dock` has the workspace's panes in its splits.
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
    /// The widths the user gave a tab's details columns, by column id
    /// (SPEC.md *File list*); an empty map restores the defaults.
    ColumnWidths {
        /// The tab.
        tab: TabId,
        /// The widths in logical points.
        columns: std::collections::BTreeMap<String, f32>,
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
    /// Open a window of its own with one tab at the location, focused:
    /// `rick --new-window <path>` (specs/system-integration.md §3).
    NewWindowAt(Location),
    /// `rick reveal <file>…`: open each file's folder in a new tab of the
    /// focused window — a window of its own for the first with
    /// `new_window` — and select the files there once they are listed.
    Reveal {
        /// The files.
        items: Vec<Location>,
        /// The first folder's tab opens in a new window.
        new_window: bool,
    },
    /// Raise and focus the focused window, as a second launch's command
    /// line asks: the UI gets [`Event::Activate`] with the window and the
    /// launcher's activation token, when the launch had one.
    Activate {
        /// `XDG_ACTIVATION_TOKEN` or `DESKTOP_STARTUP_ID` of the launch.
        token: Option<String>,
    },
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
    /// System integration (specs/system-integration.md §6): run the
    /// check, apply the dialog's rows, add or restore one item from
    /// Settings → Integration, reset the choices.
    Integration(crate::IntegrationCommand),
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
    /// No preview could be opened for the pane's selection: a previewer
    /// claimed the file and failed to open it.
    PreviewFailed {
        /// The pane.
        pane: PaneId,
        /// Why.
        error: Error,
    },
    /// No previewer claims the pane's selection; the pane shows the
    /// placeholder of SPEC.md's *No previewer*: the file's facts, its type
    /// icon and one line saying whether a plugin for the type is there.
    PreviewNone {
        /// The pane.
        pane: PaneId,
        /// A previewer plugin in the plugins directory whose manifest
        /// claims the file's type but which is disabled, by id: *Plugin
        /// available*. `None`: *No plugin available*.
        plugin: Option<String>,
    },
    /// The preview just opened for the pane shows a prefix of a file on a
    /// remote provider; the pane offers *Load more*
    /// (`specs/ext-ssh.md`, "Previews over SSH").
    PreviewPrefix {
        /// The pane.
        pane: PaneId,
        /// How many bytes the previewer was served.
        served: u64,
        /// The file's size, when known.
        total: Option<u64>,
    },
    /// The pane's selection is a file on a remote provider above the size
    /// cap for whole-file previews; the pane shows the file's facts and
    /// *Preview anyway*.
    PreviewCapped {
        /// The pane.
        pane: PaneId,
        /// The file's size.
        size: u64,
        /// The cap it is above.
        cap: u64,
        /// The source the file is on, when its provider keeps connections:
        /// the host or account *Always preview on <source>* names.
        source: Option<String>,
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
    /// An extension asked the user something and waits for the answer; the
    /// question is in the model until it is answered.
    Question(Question),
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
    /// Raise and focus a window through the windowing backend and the
    /// platform's activation (`Command::Activate`).
    Activate {
        /// The window.
        window: WindowId,
        /// The launcher's activation token, for the platform.
        token: Option<String>,
    },
    /// What `Command::ShowProperties` found: the dialogue's contents.
    Properties(Box<PropertiesInfo>),
    /// Compute hash started on the pane's selection: open the Hash
    /// dialogue on the slot, which the job fills file by file.
    HashesOpened {
        /// The pane.
        pane: PaneId,
        /// The files' names and, as they arrive, their digests.
        hashes: Arc<ext::Hashes>,
        /// The format remembered in `core.toml`, for the toggle.
        format: ops::hash::Format,
    },
    /// An integration check finished
    /// ([`IntegrationCommand::Check`](crate::IntegrationCommand::Check));
    /// the model's `integration` holds what it found.
    IntegrationChecked {
        /// The check at startup found an item missing or drifted and the
        /// user has not said *Don't ask again*: open the dialog.
        ask: bool,
    },
    /// The rows of an apply, an add or a restore were performed; the
    /// records hold what each wrote, and a check of what is there now
    /// follows.
    IntegrationApplied {
        /// The rows that could not be set up; each was reported through
        /// [`Event::Notify`].
        failed: Vec<platform::integration::Item>,
        /// The binary of the copy Rick was installed to in this apply,
        /// through its `current` link: the UI saves the session and
        /// continues from there, the running process exiting once the new
        /// one answers.
        relaunch: Option<PathBuf>,
    },
}

/// What the Properties dialogue shows for one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertiesInfo {
    /// The pane the entry is listed in.
    pub pane: PaneId,
    /// The entry.
    pub entry: Entry,
    /// The entry as a path on this machine, for a local entry: what the
    /// permissions and the platform's dialogue work on.
    pub path: Option<PathBuf>,
    /// The general rows: name, type, location, size, dates, link target.
    pub rows: Vec<(String, String)>,
    /// The owner, group, mode bits and ACL, for a local entry the platform
    /// could read them of.
    pub permissions: Option<FilePermissions>,
    /// The users and groups to choose from.
    pub principals: Principals,
    /// Whether the platform has a dialogue of its own to offer
    /// (`Command::SystemProperties`).
    pub native: bool,
}

/// A `debug_struct` of `name` with the pane and one more field: the shape
/// of most pane events.
fn pane_struct(
    f: &mut fmt::Formatter<'_>,
    name: &str,
    pane: PaneId,
    field: &str,
    value: &dyn fmt::Debug,
) -> fmt::Result {
    f.debug_struct(name)
        .field("pane", &pane)
        .field(field, value)
        .finish()
}

impl fmt::Debug for Event {
    // One arm per event, each naming what the UI would want in a log.
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Listed { pane, batch, done } => f
                .debug_struct("Listed")
                .field("pane", pane)
                .field("batch", &batch.len())
                .field("done", done)
                .finish(),
            Self::Changed { pane, change } => pane_struct(f, "Changed", *pane, "change", change),
            Self::Decorated { pane, items } => {
                pane_struct(f, "Decorated", *pane, "items", &items.len())
            }
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
            Self::SelectionChanged { pane, ids } => {
                pane_struct(f, "SelectionChanged", *pane, "ids", ids)
            }
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
            Self::PreviewFailed { pane, error } => {
                pane_struct(f, "PreviewFailed", *pane, "error", error)
            }
            Self::PreviewNone { pane, plugin } => {
                pane_struct(f, "PreviewNone", *pane, "plugin", plugin)
            }
            Self::PreviewPrefix {
                pane,
                served,
                total,
            } => f
                .debug_struct("PreviewPrefix")
                .field("pane", pane)
                .field("served", served)
                .field("total", total)
                .finish(),
            Self::PreviewCapped {
                pane,
                size,
                cap,
                source,
            } => f
                .debug_struct("PreviewCapped")
                .field("pane", pane)
                .field("size", size)
                .field("cap", cap)
                .field("source", source)
                .finish(),
            Self::Conflict(conflict) => f.debug_tuple("Conflict").field(&conflict.id).finish(),
            Self::Question(question) => f.debug_tuple("Question").field(&question.id).finish(),
            Self::Properties(info) => f
                .debug_struct("Properties")
                .field("entry", &info.entry.name)
                .field("permissions", &info.permissions.is_some())
                .finish(),
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
            Self::Activate { window, token } => f
                .debug_struct("Activate")
                .field("window", window)
                .field("token", token)
                .finish(),
            Self::HashesOpened {
                pane,
                hashes,
                format,
            } => f
                .debug_struct("HashesOpened")
                .field("pane", pane)
                .field("hashes", hashes)
                .field("format", format)
                .finish(),
            Self::IntegrationChecked { ask } => f
                .debug_struct("IntegrationChecked")
                .field("ask", ask)
                .finish(),
            Self::IntegrationApplied { failed, relaunch } => f
                .debug_struct("IntegrationApplied")
                .field("failed", failed)
                .field("relaunch", relaunch)
                .finish(),
        }
    }
}
