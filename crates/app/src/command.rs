//! The two types that cross the UI boundary.

use std::fmt;

use ext::{ActionCtx, ActionId, Decoration, FsChange, Level, PreviewDoc};
use model::{
    Column, DockTarget, PaneId, SelectionChange, SettingsChange, SortDir, SplitDir, TabId, ViewMode,
};
use types::{Entry, EntryId, Error, JobId, Location, Progress};

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
    /// Cancel a running job.
    CancelJob(JobId),
    /// Forget a finished job.
    DismissJob(JobId),
    /// Revert the last reversible operation.
    Undo,
    /// Apply the last undone operation again.
    Redo,
    /// Change one setting and save the settings file.
    Settings(SettingsChange),
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
            Self::SelectionChanged { pane, ids } => f
                .debug_struct("SelectionChanged")
                .field("pane", pane)
                .field("ids", ids)
                .finish(),
            Self::PreviewReady { pane, .. } => f
                .debug_struct("PreviewReady")
                .field("pane", pane)
                .finish_non_exhaustive(),
            Self::Notify { level, text } => f
                .debug_struct("Notify")
                .field("level", level)
                .field("text", text)
                .finish(),
        }
    }
}
