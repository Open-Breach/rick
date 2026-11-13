//! The command loop: [`Command`]s in, model changes and [`Event`]s out.
//!
//! [`App`] owns the [`Model`], the extension [`Registry`] with the built-in
//! [`ops::Core`] loaded, the provider table, the [`JobRunner`] every
//! operation runs on, the undo and redo stacks and the settings file. The UI
//! calls [`App::handle`] with a command, then [`App::poll`] (or awaits
//! [`App::tick`]) to let listings, filesystem changes and job progress flow
//! into the model, and re-renders from the model. It never mutates the model
//! itself.
//!
//! Each pane shows its active tab's location. The app keeps that listing
//! live: when the location, the hidden flag, the flatten flag, the subtree
//! search or the system-files setting changes, it cancels the pane's
//! listing job, starts a new one that streams batches into the model, and
//! watches the location for changes until the next switch. Listing jobs do
//! not appear in the job table; their output is the [`Event::Listed`]
//! stream. A subtree search is the exception: its walk is a job the
//! operations area shows and cancels. Only a pane's active tab lists; the
//! other tabs list when they are first shown.
//!
//! The app also feeds the side panel: the Places list from `places.toml`
//! (seeded from the standard folders on the first start), the mounted
//! volumes polled through the platform, the cloud providers' rows, and the
//! trash's count after every operation; it mounts the `rick://` provider
//! whose views the panel opens, hands it the views extensions registered
//! and tells it when they changed; it routes the name conflicts copy and
//! move jobs ask about to the UI and back, and shows a job queued behind
//! another on its volume as *Waiting*. A location whose provider cannot
//! watch it (an archive) is polled instead: its `stat` every few seconds, a
//! change of size or time listing it again.
//!
//! The integration items (specs/system-integration.md §6) are checked by
//! [`IntegrationCommand::Check`]: a job off the loop's thread that reads
//! what the platform finds of every item the user has not declined, folds
//! it into the records of `rick.toml`'s `[integration]` table and into
//! [`model::Integration`], and answers [`Event::IntegrationChecked`]. The
//! dialog's *Apply* and Settings' *Add* and *Restore* perform their rows
//! through the platform and record what each wrote.
//!
//! The session (every window, pane and tab, see [`model::Session`]) is
//! written to `session.toml` after every change, at most once per second,
//! and once more by [`App::save_session`] on exit. A start restores it, or
//! the start layout the settings name; saved layouts live in `layouts/`.
//!
//! ```no_run
//! use std::sync::Arc;
//!
//! use app::{App, Command, Config, Event};
//! use ext::{Level, UiHost};
//! use types::Location;
//!
//! struct Headless;
//! impl UiHost for Headless {
//!     fn notify(&self, _: Level, _: &str) {}
//!     fn confirm(&self, _: &str, _: &str) -> bool { true }
//!     fn prompt(&self, _: &str, default: &str) -> Option<String> { Some(default.to_owned()) }
//!     fn show(&self, _: &str, _: &str) {}
//!     fn form(&self, form: ext::Form) -> Option<ext::Values> { Some(form.defaults()) }
//! }
//!
//! let platform = Arc::new(platform::Native::new());
//! let home = platform::paths::from_path(&std::env::home_dir().unwrap_or_default());
//! let mut app = App::new(Config {
//!     platform,
//!     ui: Arc::new(Headless),
//!     extensions: Vec::new(),
//!     config_dir: None,
//!     start: Some(Location::new(vfs_local::PROVIDER_ID, home)),
//! });
//! let mut events: Vec<Event> = Vec::new();
//! let pane = app.model().focused;
//! app.handle(Command::ToggleHidden(pane), &mut events);
//! runtime::block_on(app.tick(&mut events));
//! ```

#![forbid(unsafe_code)]

mod command;
mod ext_settings;
mod external;
mod integration;
mod places;
pub mod plugins;
mod session;
pub mod settings;

pub use command::{
    Command, ConnectionOp, Contents, Event, FolderContents, PropertiesInfo, TransferMode,
};
pub use ext_settings::ExtSettings;
pub use external::{External, ExternalReceiver, ExternalSender, external_channel};
pub use integration::{INSTALL_RICK_COMMAND, IntegrationCommand};
/// What the Hash dialogue draws with (the format, the rendering, the
/// Verify logic), re-exported for the UI.
pub use ops::hash;
/// What a `Command::SetPermissions` carries, re-exported for the UI.
pub use ops::{PermissionRequest, Recursion};
pub use places::{PLACES_FILE, defaults as default_places, virtual_icon};
pub use session::{LAYOUTS_DIR, SAVE_INTERVAL, SESSION_FILE};
pub use settings::SETTINGS_FILE;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use ext::{
    ActionCtx, ActionOutcome, Availability, Capabilities, CliCtx, CliOutcome, DecorateCtx,
    Decoration, Decorator, Depth, Extension, Filter, FormLater, FormLaterState, FsChange,
    Invocation, Level, ListOptions, Points, PreviewDoc, Previewer, Prompts, Query, QueryKind,
    Reads, Registry, SearchIndex, SearchScope, UiHost, UndoCommand,
};
use futures::StreamExt;
use model::{
    Column, ConnectionEntry, JobState, Model, PaneId, Place, SelectionChange, Session, Settings,
    SettingsChange, SortDir, TabId, Volume,
};
use ops::{Applied, Conflicts, PermissionsCommand, PinStore, Scheduler};
use platform::{Platform, TrashedItem};
use runtime::{JobEvent, JobRunner, mpsc};
use tracing::{debug, warn};
use types::{
    Answer, Attrs, CancelToken, Entry, EntryId, EntryKind, Error, ExtId, JobId, Location, Progress,
    ProviderId, QuestionKind, VPath,
};
use vfs::Mounts;
use vfs_local::LocalProvider;
use vfs_rick::{RickProvider, Virtual};

/// What became of a plugin the host found, for the Extensions dialogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginStatus {
    /// Loaded; its contributions are registered.
    Loaded,
    /// Listed in `rick.toml` as disabled: on disk, not loaded.
    Disabled,
    /// Not loaded, for the reason given.
    Refused(String),
    /// Unpacked in this session by *Install from file…*: on disk, and
    /// loaded or refused by the next start.
    Installed,
}

/// One plugin the host found, as the Extensions screen and
/// `rick --list-plugins` show it. The binary builds the rows from the
/// plugin host and hands them over after [`App::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    /// The extension id.
    pub id: String,
    /// The name its manifest gives.
    pub name: String,
    /// What its manifest says it does; empty when it says nothing.
    pub description: String,
    /// The version its manifest gives.
    pub version: String,
    /// The extension points its manifest lists.
    pub points: Points,
    /// The file name extensions its previewers claim, as its manifest
    /// lists them, lower case without the dot.
    pub previews: Vec<String>,
    /// Loaded, disabled or refused.
    pub status: PluginStatus,
    /// The plugin's directory.
    pub path: PathBuf,
    /// It ships beside the binary; a user's plugin sits in the per-user
    /// plugins folder and can be removed.
    pub built_in: bool,
    /// Its folder was deleted in this session by *Remove*: the next start
    /// no longer finds it.
    pub removed: bool,
}

/// What an [`App`] is built from.
pub struct Config {
    /// The platform services; also the trash behind the local provider.
    pub platform: Arc<dyn Platform>,
    /// Who answers actions' prompts and shows their notifications.
    pub ui: Arc<dyn UiHost>,
    /// Extensions to load besides the built-in [`ops::Core`].
    pub extensions: Vec<Arc<dyn Extension>>,
    /// Where `rick.toml`, `session.toml` and `layouts/` live; `None` for
    /// the platform's config directory.
    pub config_dir: Option<PathBuf>,
    /// A location to open in a new tab of the restored session's focused
    /// pane, as a path on the command line asks; with no session to restore
    /// it is the one tab's location, and `None` means Home.
    pub start: Option<Location>,
}

/// Called whenever a background task has left a message for the loop, so a
/// UI can ask for a repaint instead of polling.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// The loop's inbox: the channel plus the waker rung after every send.
#[derive(Clone)]
struct Mailbox {
    tx: mpsc::UnboundedSender<Msg>,
    waker: Arc<std::sync::Mutex<Option<Waker>>>,
}

impl Mailbox {
    /// Delivers `msg` and rings the waker; `false` once the loop is gone.
    fn send(&self, msg: Msg) -> bool {
        if self.tx.send(msg).is_err() {
            return false;
        }
        let waker = self
            .waker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(wake) = waker {
            wake();
        }
        true
    }
}

/// A message from a background task to the loop.
enum Msg {
    Batch {
        pane: PaneId,
        generation: u64,
        batch: Vec<Entry>,
    },
    ListDone {
        pane: PaneId,
        generation: u64,
        result: Result<(), Error>,
    },
    /// The wait before a lost connection is tried again is over.
    RetryLost {
        pane: PaneId,
        generation: u64,
    },
    Change {
        pane: PaneId,
        generation: u64,
        change: FsChange,
    },
    Settings(Result<Settings, Error>),
    Preview {
        pane: PaneId,
        generation: u64,
        result: Result<Box<dyn PreviewDoc>, Error>,
        /// The previewer that was asked.
        previewer: ExtId,
        /// The bytes served and the file's size, when the previewer got a
        /// prefix of a remote file.
        prefix: Option<(u64, Option<u64>)>,
    },
    /// A decorator finished a batch late.
    Decorated {
        pane: PaneId,
        generation: u64,
        items: Vec<(EntryId, Decoration)>,
    },
    /// Something a pane's decorators depend on changed; decorate again.
    Invalidate {
        pane: PaneId,
        generation: u64,
    },
    /// A thumbnail render finished.
    Thumbnail {
        id: EntryId,
        size: u32,
        frame: Option<ext::Frame>,
    },
    /// A batch of an expanded folder's children, in Tree view.
    TreeBatch {
        pane: PaneId,
        generation: u64,
        folder: EntryId,
        batch: Vec<Entry>,
    },
    /// An expanded folder's listing ended.
    TreeDone {
        pane: PaneId,
        generation: u64,
        folder: EntryId,
        result: Result<(), Error>,
    },
    /// A job asked about a name conflict.
    Conflicts,
    /// An extension asked the user a question.
    Prompts,
    /// `Command::ShowProperties` read the entry: the request's number and
    /// what was read.
    Properties(u64, Box<Result<PropertiesInfo, Error>>),
    /// The count of the folder the Properties dialogue shows ended.
    PropertiesCounted {
        pane: PaneId,
        path: PathBuf,
        contents: FolderContents,
    },
    /// A permission job ended.
    PermissionsApplied {
        pane: PaneId,
        label: String,
        request: PermissionRequest,
        result: Result<Applied, Error>,
    },
    /// No previewer claims the pane's file.
    PreviewNone {
        pane: PaneId,
        generation: u64,
        /// The file's mode string and `owner:group`, where they could be
        /// read.
        permissions: Option<String>,
    },
    /// The pane's *Copy* read the file: its text, for the clipboard.
    PreviewText {
        result: Result<String, Error>,
    },
    /// A remote file's preview was held behind the size cap.
    PreviewCapped {
        pane: PaneId,
        generation: u64,
        size: u64,
        cap: u64,
    },
    /// A whole remote file is being read for a preview.
    PreviewProgress {
        pane: PaneId,
        generation: u64,
        read: u64,
        total: Option<u64>,
    },
    /// A plugin package was unpacked, or was not.
    PluginInstalled(Result<Box<Plugin>, String>),
    /// A plugin's folder was deleted, or was not.
    PluginRemoved {
        path: PathBuf,
        result: Result<(), String>,
    },
    /// A job started waiting for its volume, or stopped.
    Schedule,
    /// The mounted volumes were enumerated.
    Volumes(Result<Vec<Volume>, Error>),
    /// Whether the system has a tool for formatting a drive, as looked up
    /// with the volumes: one that is installed while Rick runs is found.
    FormatTool(bool),
    /// *Run script?* was answered with Run, or with Edit.
    Script {
        pane: PaneId,
        entry: Entry,
        /// Run; Edit otherwise.
        run: bool,
    },
    /// Open's job answered: the archive provider browses the file as a
    /// location (navigate there), or does not (the system default).
    Opened {
        pane: PaneId,
        tab: TabId,
        entry: Entry,
        location: Option<Location>,
    },
    /// The pause after the search box's last keystroke is over: ask the
    /// indexes.
    IndexSearchStart {
        generation: u64,
    },
    /// A search index answered a batch of the title-row search.
    IndexHits {
        generation: u64,
        batch: Vec<Entry>,
    },
    /// A search index finished answering the title-row search.
    IndexDone {
        generation: u64,
        index: ExtId,
        result: Result<(), Error>,
    },
    /// The trash was listed.
    Trash(Result<(Vec<TrashedItem>, u64), Error>),
    /// The cloud providers' roots were listed.
    Cloud(Vec<Entry>),
    /// The network providers' roots were listed.
    Network(Vec<Entry>),
    /// A new connection's secrets are in the keychain: write the rest.
    ConnectionSaved(ConnectionEntry),
    /// A connection of this provider was connected or disconnected: its
    /// rows may read differently.
    ConnectionChanged(ProviderId),
    /// The integration check answered: the platform's items and what it
    /// found of each one that is not declined.
    IntegrationChecked {
        generation: u64,
        startup: bool,
        listed: platform::integration::Items,
        found: Vec<(platform::integration::Item, platform::integration::Check)>,
    },
    /// The integration apply starts on a row.
    IntegrationProgress {
        done: usize,
        total: usize,
        current: platform::integration::Item,
    },
    /// The integration apply ended: each row's receipt, or why it failed,
    /// and the copy Rick was installed to on the way.
    IntegrationApplied {
        results: Vec<(
            platform::integration::Item,
            Result<platform::integration::Receipt, Error>,
        )>,
        relaunch: Option<PathBuf>,
    },
}

/// How long changes are collected before the decorators run again.
const REDECORATE_AFTER: Duration = Duration::from_millis(150);
/// How often the mounted volumes are enumerated again, whatever the
/// platform reported: the net under its mount events, and all there is
/// where it has none.
const VOLUMES_EVERY: Duration = Duration::from_secs(2);
/// How long after a mount or unmount event the volumes are enumerated: the
/// events of one stick, one per partition, come in a burst.
const VOLUMES_AFTER_EVENT: Duration = Duration::from_millis(100);
/// How often a location its provider cannot watch is checked for a change.
const POLL_EVERY: Duration = Duration::from_secs(3);
/// When Rick tries a lost connection again by itself: after the loss, then
/// after each try that failed; after the last one it waits for *Retry*
/// (SPEC.md *Removed and unreachable locations*).
pub const RETRY_LOST_AFTER: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(15),
];
/// How long one [`App::poll`] may spend applying messages before it leaves
/// the rest to the next one.
const POLL_BUDGET: Duration = Duration::from_millis(8);
/// How long a reveal waits for its files to be listed before it selects
/// what is there.
const REVEAL_FOR: Duration = Duration::from_secs(5);

/// The line typed into a console to take its shell to `path`: quoted for
/// the shell, with a leading space so a shell that ignores such lines
/// keeps it out of its history, and Enter.
/// The embedded console's action, `ext-action-terminal`'s *Open console*.
const EMBEDDED_CONSOLE: ext::ActionId = ext::ActionId("terminal.console");
/// How much of a file with the execute bit is read to tell a binary from
/// a script.
const PROGRAM_PROBE: u64 = 4096;

/// Whether the file is a binary and no script: its first 4 KiB hold a NUL
/// byte (SPEC.md *Executables and scripts*).
fn is_binary(path: &Path) -> Result<bool, Error> {
    use std::io::Read as _;
    let mut head = Vec::new();
    std::fs::File::open(path)
        .map_err(Error::from_io)?
        .take(PROGRAM_PROBE)
        .read_to_end(&mut head)
        .map_err(Error::from_io)?;
    Ok(head.contains(&0))
}

/// The line that runs the script `name` of the console's folder, as it is
/// typed at the shell's prompt.
fn run_line(name: &str) -> Vec<u8> {
    let quoted = name.replace('\'', "'\\''");
    format!("'./{quoted}'\r").into_bytes()
}

fn cd_command(path: &Path) -> Vec<u8> {
    if ext::Platforms::current() == ext::Platforms::WINDOWS {
        format!(" cd /d \"{}\"\r", path.display()).into_bytes()
    } else {
        let quoted = path.to_string_lossy().replace('\'', "'\\''");
        format!(" cd '{quoted}'\r").into_bytes()
    }
}
/// How long the title-row search box's text must stand before the search
/// indexes are asked, so a typist does not start a query per keystroke.
const INDEX_SEARCH_AFTER: Duration = Duration::from_millis(150);
/// How many states of the settings file are remembered, its state at the
/// start and this process's own writes, so the watcher's report of each is
/// told from an edit.
const RECENT_SAVES: usize = 16;
/// How much of a file on a remote provider a prefix previewer is served at
/// first, and per *Load more* (`specs/ext-ssh.md`, "Previews over SSH").
pub const REMOTE_PREFIX_CHUNK: u64 = 256 * 1024;
/// The size above which a whole-file previewer waits for *Preview anyway*
/// on a remote provider, unless the provider names another cap.
pub const REMOTE_PREVIEW_CAP: u64 = 4 * 1024 * 1024;
/// The largest file the preview pane's *Copy* puts on the clipboard.
pub const COPY_LIMIT: usize = 256 * 1024 * 1024;
/// The connection field that switches the preview size cap off for one
/// source (SPEC.md, Preview, *Remote sources*): `preview_always = true`
/// in the source's `[[connections]]` table.
pub const PREVIEW_ALWAYS: &str = "preview_always";

/// The kernel's English messages, `locales/en.ftl` of the repository:
/// every key of the shell, the dialogues, the menus and the core actions.
pub const LOCALE_EN: &str = include_str!("../../../locales/en.ftl");
/// The kernel's German messages, `locales/de.ftl`: the Settings screen and
/// the context menus at least, the rest falling back to English.
pub const LOCALE_DE: &str = include_str!("../../../locales/de.ftl");

/// The pinned list as the pin actions see it. The list itself lives in the
/// model's settings, which `App` owns; the bridge keeps a copy for the
/// actions to read and queues the changes they ask for, which `App` applies
/// to the settings and writes to the file right after the action returns.
/// Shared state across the `ops` boundary, justified: an action runs with
/// nothing but the job runner and the UI host in hand, and the undo log
/// replays it later with the same.
#[derive(Default)]
struct PinBridge {
    pinned: RwLock<Vec<Location>>,
    pending: Mutex<Vec<SettingsChange>>,
    /// The command of *Open in editor*, as the settings have it.
    editor: RwLock<String>,
}

impl PinBridge {
    /// Replaces the copy with the settings' list.
    fn sync(&self, pinned: &[Location]) {
        pinned.clone_into(&mut self.pinned.write().unwrap_or_else(PoisonError::into_inner));
    }

    /// Takes what the open actions read of `settings`.
    fn sync_open(&self, settings: &Settings) {
        settings
            .editor
            .clone_into(&mut self.editor.write().unwrap_or_else(PoisonError::into_inner));
    }

    /// The changes asked for since the last call.
    fn take(&self) -> Vec<SettingsChange> {
        std::mem::take(&mut self.pending.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl ops::OpenSettings for PinBridge {
    fn editor(&self) -> String {
        self.editor
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl vfs_rick::Favorites for PinBridge {
    fn pinned(&self) -> Vec<Location> {
        PinStore::pinned(self)
    }
}

impl PinStore for PinBridge {
    fn pinned(&self) -> Vec<Location> {
        self.pinned
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn pin(&self, items: Vec<Location>, at: Option<usize>) {
        {
            // The copy follows at once, so a second pin in the same action
            // sees the first.
            let mut pinned = self.pinned.write().unwrap_or_else(PoisonError::into_inner);
            let mut index = at.map_or(pinned.len(), |at| at.min(pinned.len()));
            for item in &items {
                if !pinned.contains(item) {
                    pinned.insert(index, item.clone());
                    index += 1;
                }
            }
        }
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SettingsChange::Pin { items, at });
    }

    fn unpin(&self, item: &Location) {
        self.pinned
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|p| p != item);
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SettingsChange::Unpin(item.clone()));
    }
}

/// The document the Preview tab shows and the job that opens it, by the
/// pane of the explorer tab the preview follows.
struct PreviewJob {
    entry: EntryId,
    /// The entry itself, so the preview can open again with more served
    /// or the cap lifted.
    full: Entry,
    generation: u64,
    cancel: CancelToken,
}

/// What a pane's listing was started for; a different key means relist.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ListKey {
    tab: TabId,
    location: Location,
    opts: ListOptions,
    /// The subtree search text, when the filter searches the subtree.
    search: Option<String>,
}

/// The running listing of one expanded folder in Tree view.
struct Child {
    cancel: CancelToken,
    /// Set while a refresh collects the children to swap them in at once.
    buffer: Option<Vec<Entry>>,
}

/// The title-row search box's running index search: one job per index,
/// cancelled together when the text changes.
struct IndexJob {
    generation: u64,
    cancels: Vec<CancelToken>,
    /// Indexes still to answer.
    pending: usize,
}

/// A pane's running or finished listing and its watch.
struct Live {
    key: ListKey,
    generation: u64,
    cancel: CancelToken,
    complete: bool,
    /// Set while a refresh collects the whole listing to swap it in at once.
    buffer: Option<Vec<Entry>>,
    /// The decorators that apply to the location.
    decorators: Vec<Arc<dyn Decorator>>,
    /// A re-decoration is scheduled after the debounce.
    redecorate_scheduled: bool,
    /// The expanded folders' listings in flight.
    children: BTreeMap<EntryId, Child>,
}

/// A console tab and the explorer tab it was opened from
/// (specs/system-integration.md §4): the tab follows the shell's working
/// directory, and the shell is told `cd` when the tab navigates while it
/// is at its prompt.
struct ConsoleLink {
    /// The explorer's pane.
    pane: PaneId,
    /// The explorer tab, which must be its pane's active tab to be moved.
    explorer: TabId,
    /// The console.
    console: Arc<dyn ext::Console>,
    /// The working directory the console last reported, so a report is
    /// acted on once.
    cwd_seen: Option<PathBuf>,
    /// The explorer's location last seen, so a navigation is typed once,
    /// and one the console itself caused not at all.
    location_seen: Location,
    /// A line still to type into the console: the script it was opened to
    /// run, typed once the shell is at its prompt.
    line: Option<Vec<u8>>,
}

/// The application: model, registry, jobs, undo log and settings.
pub struct App {
    model: Model,
    mounts: Arc<Mounts>,
    registry: Registry,
    platform: Arc<dyn Platform>,
    ui: Arc<dyn UiHost>,
    pins: Arc<PinBridge>,
    conflicts: Arc<Conflicts>,
    /// Where extensions ask the user a question (a host key, a passphrase);
    /// the UI answers through [`Command::AnswerQuestion`].
    prompts: Arc<Prompts>,
    /// The per-volume queue the copy and move jobs go through.
    scheduler: Arc<Scheduler>,
    /// The jobs shown as *Waiting* right now.
    waiting: BTreeSet<JobId>,
    /// The `rick://` provider, told when a view's contents changed.
    rick: Arc<RickProvider>,
    jobs: JobRunner,
    job_events: mpsc::UnboundedReceiver<JobEvent>,
    /// Where decorators run their lookups: part of listing, so kept out of
    /// the job table like the listing jobs themselves.
    decor_jobs: JobRunner,
    mailbox: Mailbox,
    rx: mpsc::UnboundedReceiver<Msg>,
    live: BTreeMap<PaneId, Live>,
    /// Listing jobs, kept out of the job table.
    internal: BTreeSet<JobId>,
    /// Jobs an action or an undo started, by the pane to refresh when they
    /// finish.
    pane_jobs: BTreeMap<JobId, PaneId>,
    undo: Vec<Box<dyn UndoCommand>>,
    redo: Vec<Box<dyn UndoCommand>>,
    settings_path: PathBuf,
    settings_watch: CancelToken,
    /// Writes the settings file in order, one at a time.
    settings_saver: mpsc::UnboundedSender<Settings>,
    /// What this process wrote lately, after what the file held when it
    /// was read at the start: a reload that matches one is the watcher
    /// reporting our own write, or the file as it was before a change that
    /// is still on its way to it, not an edit to pick up.
    recent_saves: VecDeque<Settings>,
    /// The previews being opened or shown, by pane.
    preview: BTreeMap<PaneId, PreviewJob>,
    /// Numbers the preview jobs, so a late answer is told from the current.
    preview_generation: u64,
    /// How many times *Load more* was pressed for the entry a pane previews.
    preview_more: BTreeMap<PaneId, (EntryId, u32)>,
    /// The entry a pane previews whatever its size, after *Preview anyway*.
    preview_force: BTreeMap<PaneId, EntryId>,
    /// The entry a pane previews through one previewer whatever the
    /// scores say, after [`Command::PreviewWith`].
    preview_with: BTreeMap<PaneId, (EntryId, ExtId)>,
    /// Thumbnails being rendered, so a cell asked for twice renders once.
    thumbnails: BTreeSet<(EntryId, u32)>,
    /// Events from start-up, delivered on the first poll.
    pending: Vec<Event>,
    /// The forms actions will show once a job of theirs finishes
    /// ([`ActionOutcome::Later`]), with the pane each ran in; looked at on
    /// every poll.
    later: Vec<(PaneId, FormLater)>,
    /// The file names `Command::Reveal` selects in a pane once its folder
    /// is listed, with when it asked; a name that never shows is dropped
    /// after [`REVEAL_FOR`].
    reveal_pending: BTreeMap<PaneId, (Vec<Vec<u8>>, std::time::Instant)>,
    /// The pane whose reveal ends with the Properties dialogue on what it
    /// selected.
    reveal_properties: Option<PaneId>,
    /// The console tabs open and the explorer tabs they follow, by the
    /// console's tab.
    console_links: BTreeMap<TabId, ConsoleLink>,
    /// Writes `places.toml`, latest snapshot wins.
    places_saver: mpsc::UnboundedSender<Vec<Place>>,
    /// Stops the volume poll.
    volumes_watch: CancelToken,
    /// The line the console that is being opened is to type: the script
    /// *Run* starts, between asking for the console and its link.
    console_line: Option<Vec<u8>>,
    /// The platform's watch of mounts and unmounts, which makes the volume
    /// poll read the list at once; kept for as long as the application
    /// runs. `None` where the platform has no such events.
    mounts_watch: Option<platform::MountWatch>,
    /// Rung to make every polled location check for a change now; see
    /// [`poll_location`].
    poke: runtime::watch::Sender<u64>,
    /// The user's home, where a tab opens when its own location is gone.
    home: Location,
    /// Writes `session.toml`, debounced.
    session_saver: session::Saver,
    /// The session last handed to the saver, so an unchanged model sends
    /// nothing.
    last_sent: Option<Session>,
    /// `session.toml` as it was at start: "Continue where I left off" when
    /// the start was a layout.
    last_session: Option<Session>,
    /// The `layouts/` directory.
    layouts_dir: PathBuf,
    /// The saved layouts' names, refreshed on every change to the directory
    /// this process makes.
    layouts: Vec<String>,
    /// The last window was closed: the UI should exit.
    exit: bool,
    /// The plugins the host found, for the Extensions screen.
    plugins: Vec<Plugin>,
    /// The extension that registered each action, by the action's id; the
    /// kernel's own actions are not in it.
    action_owners: BTreeMap<&'static str, ExtId>,
    /// What each extension's providers serve, as the schemes of their
    /// locations: `azure://`, `rick://trash`.
    provider_schemes: BTreeMap<ExtId, Vec<String>>,
    /// The third-party library a plugin's previewer reported missing in
    /// this session, by the plugin's id.
    missing_libraries: BTreeMap<String, String>,
    /// The config directory, where the extensions' settings files live.
    config_dir: PathBuf,
    /// The values of the settings extensions declared, one TOML file each;
    /// shared with the extensions through the registry.
    ext_settings: Arc<ExtSettings>,
    /// The title-row search box's index search in flight.
    index_search: Option<IndexJob>,
    /// The entry the Properties dialogue shows, while it is open: a change
    /// to it reports the new state through `Event::Properties`.
    properties_shown: Option<(PaneId, PathBuf)>,
    /// The count of that entry's contents, when it is a folder: running,
    /// or done and kept for as long as the dialogue shows the folder.
    properties_count: Option<PropertiesCount>,
    /// The entries whose facts a background thread is still reading for
    /// the Properties dialogue: each ends in one `Msg::Properties`, and
    /// until then the application is busy.
    properties_reading: usize,
    /// Numbers the requests for Properties.
    properties_asked: u64,
    /// The request whose answer the dialogue waits for: the latest one,
    /// until it is answered or called off. Any other answer is dropped.
    properties_waiting: Option<u64>,
    /// Numbers the index searches, so a superseded one's hits are dropped.
    index_generation: u64,
    /// The integration check or apply in flight.
    integration: integration::IntegrationRun,
}

impl App {
    /// Builds the application: mounts the local provider, loads the
    /// built-in operations and `config.extensions`, mounts the `rick://`
    /// provider over them, reads the settings and places files and starts
    /// watching the settings, restores the session or the start layout,
    /// starts the side panel's feeds, and lists every active tab's location.
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self::with_prompts(config, Prompts::new())
    }

    /// [`App::new`] with the question queue the extensions ask the user
    /// through: the binary makes one and lends it to the plugins it loads
    /// before the application exists, so their questions reach the same
    /// dialogues.
    #[must_use]
    pub fn with_prompts(config: Config, prompts: Arc<Prompts>) -> Self {
        let dir = config
            .config_dir
            .clone()
            .unwrap_or_else(|| config.platform.config_dir());
        Self::with_services(config, prompts, Arc::new(ExtSettings::new(&dir)))
    }

    /// [`with_prompts`](Self::with_prompts) with the extension settings
    /// store the binary created before the plugin host ran, so the plugins
    /// loaded there and the extensions compiled in read and write the same
    /// values.
    // Start-up is one straight line: registry, settings, session, loop.
    #[allow(clippy::too_many_lines)]
    #[must_use]
    pub fn with_services(
        config: Config,
        prompts: Arc<Prompts>,
        ext_settings: Arc<ExtSettings>,
    ) -> Self {
        let Config {
            platform,
            ui,
            extensions,
            config_dir,
            start,
        } = config;
        let mut pending = Vec::new();

        let mut registry = Registry::with_platform(Arc::clone(&platform) as Arc<dyn Platform>)
            .with_prompts(Arc::clone(&prompts))
            .with_settings_store(Arc::clone(&ext_settings) as Arc<dyn ext::SettingsStore>);
        let mounts = Arc::new(Mounts::new());
        let trash: Arc<dyn platform::Trash> = Arc::clone(&platform) as Arc<dyn platform::Trash>;
        mounts.mount(Arc::new(LocalProvider::new(trash)));
        let pins = Arc::new(PinBridge::default());
        let (jobs, job_events) = JobRunner::with_events();
        // The name-conflict queue is the job runner's, so a plugin's job
        // asks through the same dialogue as the kernel's copies.
        let conflicts = Arc::clone(jobs.conflicts());
        let scheduler = Scheduler::new();
        let core = ops::Core::new(
            Arc::clone(&mounts),
            Arc::clone(&platform),
            Arc::clone(&pins) as Arc<dyn PinStore>,
            Arc::clone(&conflicts),
            Arc::clone(&scheduler),
        )
        .with_open_settings(Arc::clone(&pins) as Arc<dyn ops::OpenSettings>);
        // Which extension an action came from, the kernel's own apart: the
        // Keyboard shortcuts dialogue files a plugin's actions under the
        // plugin.
        let mut action_owners = BTreeMap::new();
        // And what its providers serve, for the Extensions dialogue.
        let mut provider_schemes: BTreeMap<ExtId, Vec<String>> = BTreeMap::new();
        let plugins = extensions.iter().map(|e| (&**e, true));
        for (ext, plugin) in std::iter::once((&core as &dyn Extension, false)).chain(plugins) {
            let id = ext.manifest().id;
            let before = (
                registry.actions().len(),
                registry.providers().len(),
                registry.views().len(),
            );
            match registry.load(ext) {
                Ok(()) if plugin => {
                    for action in &registry.actions()[before.0..] {
                        action_owners.insert(action.id().0, id);
                    }
                    let mut schemes: Vec<String> = registry.providers()[before.1..]
                        .iter()
                        .map(|provider| format!("{}://", provider.id().0))
                        .chain(
                            registry.views()[before.2..]
                                .iter()
                                .map(|(name, _)| format!("rick://{name}")),
                        )
                        .collect();
                    schemes.dedup();
                    if !schemes.is_empty() {
                        provider_schemes.insert(id, schemes);
                    }
                }
                Ok(()) => {}
                Err(error) => {
                    warn!(%error, ext = id.0, "extension refused");
                    pending.push(Event::Notify {
                        level: Level::Error,
                        text: format!("Extension {} refused: {error}", id.0),
                    });
                }
            }
        }
        for provider in registry.providers() {
            mounts.mount(Arc::clone(provider));
        }
        // The virtual locations go on top of every provider, since the
        // Network and Cloud overviews list the others' roots; the views
        // extensions registered (the Trash view of the trash plugin) are
        // served through it.
        let rick = Arc::new(RickProvider::new(
            Arc::clone(&platform),
            Arc::clone(&pins) as Arc<dyn vfs_rick::Favorites>,
            Arc::clone(&mounts),
        ));
        for (name, provider) in registry.views() {
            rick.add_view(name, Arc::clone(provider));
        }
        mounts.mount(Arc::clone(&rick) as Arc<dyn ext::Provider>);
        let mut cloud_providers: Vec<types::ProviderId> = mounts
            .ids()
            .into_iter()
            .filter(|id| vfs_rick::is_cloud_provider(*id))
            .collect();
        cloud_providers.sort();
        let mut network_providers: Vec<types::ProviderId> = mounts
            .ids()
            .into_iter()
            .filter(|id| vfs_rick::is_network_provider(*id))
            .collect();
        network_providers.sort();

        let config_dir = config_dir.unwrap_or_else(|| platform.config_dir());
        ext_settings.load(registry.settings_schemas());
        let settings_path = config_dir.join(SETTINGS_FILE);
        let places_path = config_dir.join(PLACES_FILE);
        let (places, seeded) = match places::load(&places_path) {
            Ok(Some(places)) => (places, false),
            Ok(None) => (places::defaults(&*platform), true),
            Err(error) => {
                warn!(%error, path = %places_path.display(), "places not read");
                pending.push(Event::Notify {
                    level: Level::Warning,
                    text: format!("Places file not read: {error}"),
                });
                (places::defaults(&*platform), false)
            }
        };
        let session_path = config_dir.join(SESSION_FILE);
        let layouts_dir = config_dir.join(LAYOUTS_DIR);
        // What the file holds now is no edit to pick up later: a reload
        // that reads it again before our first change has been written
        // must not undo that change.
        let mut recent_saves = VecDeque::new();
        let settings = match settings::load(&settings_path) {
            Ok(settings) => {
                recent_saves.push_back(settings.clone());
                settings
            }
            Err(error) => {
                warn!(%error, path = %settings_path.display(), "settings not read");
                pending.push(Event::Notify {
                    level: Level::Warning,
                    text: format!("Settings file not read: {error}"),
                });
                Settings::default()
            }
        };
        // The kernel's own translations, the user's overrides in the
        // configuration directory, and the language the settings name.
        ext::i18n::add_bundled("en", LOCALE_EN);
        ext::i18n::add_bundled("de", LOCALE_DE);
        ext::i18n::load_overrides(&config_dir);
        ext::i18n::set_language(settings.language.as_deref());
        pins.sync(&settings.pinned);
        pins.sync_open(&settings);
        for connector in registry.connectors() {
            connector.update(connections_of(&settings, connector.provider()));
        }

        let home = Location::new(
            vfs_local::PROVIDER_ID,
            platform::paths::from_path(&platform.home_dir()),
        );
        // The last session, whatever starts: it stays reachable as
        // "Continue where I left off".
        let last_session = match session::load(&session_path) {
            Ok(session) => Some(session),
            Err(Error::NotFound) => None,
            Err(error) => {
                warn!(%error, path = %session_path.display(), "session not read");
                pending.push(Event::Notify {
                    level: Level::Warning,
                    text: format!("Session file not read: {error}"),
                });
                None
            }
        };
        let start_layout = settings.session.start_layout().and_then(|name| {
            match session::load(&session::layout_path(&layouts_dir, name)) {
                Ok(layout) => Some(layout),
                Err(error) => {
                    warn!(%error, name, "start layout not read");
                    pending.push(Event::Notify {
                        level: Level::Warning,
                        text: format!("Start layout {name} not read: {error}"),
                    });
                    None
                }
            }
        });
        let restored = start_layout.or_else(|| last_session.clone());
        let mut model = match restored {
            Some(mut session) => {
                repair(&mounts, &home, &mut session);
                let mut model = Model::from_session(settings, &session, &home);
                if let Some(start) = start {
                    model.new_tab(None, start);
                }
                model
            }
            None => Model::new(settings, start.unwrap_or_else(|| home.clone())),
        };
        model.set_places(places);
        model.format_tool = platform.has_format_tool();
        model.cloud_providers = cloud_providers;
        model.network_providers = network_providers;
        // The search indexes: the platform's feed the title-row search
        // box; a walk searches beneath a location, which the filter bar's
        // toggle asks about per location (`subtree_search`).
        model.search_indexes = available_indexes(&registry)
            .iter()
            .map(|index| index.id())
            .collect();
        let (tx, rx) = mpsc::unbounded_channel();
        let settings_saver = settings::saver(settings_path.clone());
        let layouts = session::layouts(&layouts_dir);
        let places_saver = {
            let path = places_path;
            saver(move |places: &Vec<Place>| places::save(&path, places))
        };
        let mailbox = Mailbox {
            tx,
            waker: Arc::new(std::sync::Mutex::new(None)),
        };
        {
            let tx = mailbox.clone();
            conflicts.set_waker(Arc::new(move || {
                tx.send(Msg::Conflicts);
            }));
        }
        {
            let tx = mailbox.clone();
            scheduler.set_waker(Arc::new(move || {
                tx.send(Msg::Schedule);
            }));
        }
        {
            let tx = mailbox.clone();
            prompts.set_waker(Arc::new(move || {
                tx.send(Msg::Prompts);
            }));
        }
        let mut app = Self {
            model,
            mounts,
            registry,
            platform,
            ui,
            pins,
            conflicts,
            prompts,
            scheduler,
            waiting: BTreeSet::new(),
            rick,
            jobs,
            job_events,
            decor_jobs: JobRunner::new(),
            mailbox,
            rx,
            live: BTreeMap::new(),
            internal: BTreeSet::new(),
            pane_jobs: BTreeMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            settings_path,
            settings_watch: CancelToken::new(),
            settings_saver,
            recent_saves,
            preview: BTreeMap::new(),
            preview_generation: 0,
            preview_more: BTreeMap::new(),
            preview_force: BTreeMap::new(),
            preview_with: BTreeMap::new(),
            thumbnails: BTreeSet::new(),
            pending,
            later: Vec::new(),
            reveal_pending: BTreeMap::new(),
            reveal_properties: None,
            console_links: BTreeMap::new(),
            places_saver,
            volumes_watch: CancelToken::new(),
            console_line: None,
            mounts_watch: None,
            poke: runtime::watch::channel(0).0,
            home,
            session_saver: session::Saver::new(session_path),
            last_sent: None,
            last_session,
            layouts_dir,
            layouts,
            exit: false,
            plugins: Vec::new(),
            action_owners,
            provider_schemes,
            missing_libraries: BTreeMap::new(),
            config_dir: config_dir.clone(),
            ext_settings,
            index_search: None,
            properties_shown: None,
            properties_count: None,
            properties_reading: 0,
            properties_asked: 0,
            properties_waiting: None,
            index_generation: 0,
            integration: integration::IntegrationRun::default(),
        };
        app.watch_settings();
        if seeded {
            app.save_places();
        }
        app.poll_volumes();
        app.refresh_trash();
        app.refresh_cloud();
        app.refresh_network();
        let mut events = Vec::new();
        app.sync_panes(&mut events);
        app.sync_preview();
        app.pending.append(&mut events);
        app.sync_session();
        app
    }

    /// Where jobs ask about name conflicts; the UI answers through
    /// [`Command::ResolveConflict`].
    #[must_use]
    pub fn conflicts(&self) -> &Arc<Conflicts> {
        &self.conflicts
    }

    /// Where extensions ask the user a question; the UI answers through
    /// [`Command::AnswerQuestion`], a test through [`Prompts::answer_with`].
    #[must_use]
    pub fn prompts(&self) -> &Arc<Prompts> {
        &self.prompts
    }

    /// The session the model is right now.
    #[must_use]
    pub fn session(&self) -> Session {
        self.model.session()
    }

    /// Writes the session now and stops the continuous writer: the exit
    /// hook. Every window still in the model is in the file, so closing the
    /// last window leaves it there for the next start.
    ///
    /// # Errors
    ///
    /// The I/O error.
    pub fn save_session(&mut self) -> Result<(), Error> {
        let session = self.model.session();
        self.last_sent = Some(session.clone());
        self.session_saver.flush(&session)
    }

    /// Where the session is written.
    #[must_use]
    pub fn session_path(&self) -> &Path {
        self.session_saver.path()
    }

    /// How many times the continuous writer wrote the session file.
    #[must_use]
    pub fn session_writes(&self) -> u64 {
        self.session_saver.writes()
    }

    /// Records the plugins the host found, for the Extensions dialogue.
    pub fn set_plugins(&mut self, plugins: Vec<Plugin>) {
        self.plugins = plugins;
    }

    /// Unpacks a plugin package into the per-user plugins folder as a job
    /// (SPEC.md *Extensions dialogue*): nothing is checked and nothing is
    /// asked, and the plugin loads at the next start. Answered by
    /// [`Event::PluginInstalled`], or by an error notice.
    fn install_plugin(&mut self, package: PathBuf) {
        let dir = self.plugins_dir();
        let tx = self.mailbox.clone();
        let name = format!("Install {}", package.display());
        let handle = self.jobs.spawn_named(name, move |_job| async move {
            let result = tokio::task::spawn_blocking(move || {
                plugins::install(&package, &dir)
                    .map(|folder| Box::new(plugins::describe(&folder)))
                    .map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|join| Err(join.to_string()));
            tx.send(Msg::PluginInstalled(result));
            Ok(())
        });
        self.internal.insert(handle.id);
    }

    /// Deletes the folder of a user's plugin as a job, at once and
    /// without a question; a plugin that ships beside the binary is not
    /// removed. The plugin stays loaded until the next start.
    fn remove_plugin(&mut self, path: PathBuf) {
        let listed = self
            .plugins
            .iter()
            .any(|row| row.path == path && !row.built_in && !row.removed);
        if !listed {
            return;
        }
        let dir = self.plugins_dir();
        let tx = self.mailbox.clone();
        let name = format!("Remove {}", path.display());
        let handle = self.jobs.spawn_named(name, move |_job| async move {
            let folder = path.clone();
            let result = tokio::task::spawn_blocking(move || {
                plugins::remove(&folder, &dir).map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|join| Err(join.to_string()));
            tx.send(Msg::PluginRemoved { path, result });
            Ok(())
        });
        self.internal.insert(handle.id);
    }

    /// The plugins the host found, as the binary recorded them; empty when
    /// no host ran (the tests' apps).
    #[must_use]
    pub fn plugins(&self) -> &[Plugin] {
        &self.plugins
    }

    /// The extension an action came from, as its manifest describes it:
    /// the plugin the Keyboard shortcuts dialogue files the action under.
    /// `None` for the kernel's own actions, the file operations of
    /// [`ops::Core`].
    #[must_use]
    pub fn action_extension(&self, action: ext::ActionId) -> Option<&ext::Manifest> {
        let id = self.action_owners.get(action.0)?;
        self.registry.manifests().iter().find(|m| m.id == *id)
    }

    /// The per-user plugins folder: where *Install from file…* unpacks a
    /// package, and the only place *Remove* deletes from.
    #[must_use]
    pub fn plugins_dir(&self) -> PathBuf {
        self.config_dir.join(plugins::PLUGINS_DIR)
    }

    /// What the providers of the loaded plugin `id` serve, as the schemes
    /// of their locations (`azure://`, `rick://trash`); empty for a
    /// plugin without providers, and for one that is not loaded.
    #[must_use]
    pub fn plugin_schemes(&self, id: &str) -> &[String] {
        self.provider_schemes
            .iter()
            .find(|(ext, _)| ext.0 == id)
            .map_or(&[], |(_, schemes)| schemes.as_slice())
    }

    /// The settings file of the plugin `id`, when it declares settings:
    /// `<config dir>/<id>.toml`.
    #[must_use]
    pub fn plugin_config(&self, id: &str) -> Option<PathBuf> {
        self.registry
            .settings_schemas()
            .iter()
            .any(|schema| schema.ext.0 == id)
            .then(|| ext_settings::file_of(&self.config_dir, id))
    }

    /// The third-party library the plugin `id` reported missing in this
    /// session, by its name (`pdfium`, `LibreOffice`): its previewer
    /// answered a file with the missing-library card and has shown no
    /// file since.
    #[must_use]
    pub fn missing_library(&self, id: &str) -> Option<&str> {
        self.missing_libraries.get(id).map(String::as_str)
    }

    /// The id of a disabled plugin in the plugins directory whose manifest
    /// says its previewers claim files named like `name` (by any suffix
    /// after a dot, so `a.tar.gz` finds a plugin that claims `tar.gz` or
    /// `gz`): the answer to *Plugin available* in the preview pane's
    /// placeholder, from the manifests alone (SPEC.md, Preview, *No
    /// previewer*).
    #[must_use]
    pub fn disabled_plugin_for(&self, name: &str) -> Option<String> {
        let lower = name.to_ascii_lowercase();
        let suffixes: Vec<&str> = lower
            .char_indices()
            .filter(|(_, c)| *c == '.')
            .map(|(i, _)| &lower[i + 1..])
            .filter(|s| !s.is_empty())
            .collect();
        self.plugins
            .iter()
            .filter(|plugin| plugin.status == PluginStatus::Disabled)
            .find(|plugin| {
                plugin
                    .previews
                    .iter()
                    .any(|claimed| suffixes.iter().any(|s| claimed.eq_ignore_ascii_case(s)))
            })
            .map(|plugin| plugin.id.clone())
    }

    /// The values of the settings extensions declared
    /// ([`Registry::settings_schemas`]), which the Settings screen edits
    /// through [`Command::ExtSetting`].
    #[must_use]
    pub fn ext_settings(&self) -> &ExtSettings {
        &self.ext_settings
    }

    /// The saved layouts' names, sorted.
    #[must_use]
    pub fn layouts(&self) -> &[String] {
        &self.layouts
    }

    /// Whether the last window was closed and the UI should exit.
    #[must_use]
    pub fn exit_requested(&self) -> bool {
        self.exit
    }

    /// Asks the UI to exit, as closing the last window does: what the
    /// relaunch from the installed copy ends with, once the new instance
    /// has answered (specs/system-integration.md §6.1).
    pub fn request_exit(&mut self) {
        self.exit = true;
    }

    /// The user's home directory as a location.
    #[must_use]
    pub fn home(&self) -> &Location {
        &self.home
    }

    /// Hands the saver a snapshot when the session changed since the last
    /// one.
    fn sync_session(&mut self) {
        let session = self.model.session();
        if self.last_sent.as_ref() == Some(&session) {
            return;
        }
        self.session_saver.send(session.clone());
        self.last_sent = Some(session);
    }

    /// Opens a window with the start layout: the start layout's focused
    /// window when one is named and reads, otherwise one tab at the focused
    /// tab's location.
    fn new_window(&mut self, events: &mut Vec<Event>) {
        let layout = self
            .model
            .settings
            .session
            .start_layout()
            .map(str::to_owned)
            .and_then(|name| self.load_layout(&name, events));
        let window = layout.and_then(|mut layout| {
            repair(&self.mounts, &self.home, &mut layout);
            let index = layout
                .focused_window
                .min(layout.windows.len().saturating_sub(1));
            layout.windows.into_iter().nth(index)
        });
        if let Some(saved) = window {
            // A new window opens beside the one it was opened from,
            // whatever place the layout held for its own.
            let opener = self.model.focused_window;
            let home = self.home.clone();
            let new = self.model.add_window(&saved, &home);
            self.model.open_beside(new, opener);
        } else {
            let location = self
                .model
                .active_tab(self.model.focused)
                .map_or_else(|| self.home.clone(), |(_, tab)| tab.location.clone());
            self.model.new_window(location);
        }
    }

    /// Reads a layout file, reporting failure.
    fn load_layout(&self, name: &str, events: &mut Vec<Event>) -> Option<Session> {
        match session::load(&session::layout_path(&self.layouts_dir, name)) {
            Ok(layout) => Some(layout),
            Err(error) => {
                events.push(Event::Notify {
                    level: Level::Error,
                    text: format!("Layout {name}: {error}"),
                });
                None
            }
        }
    }

    /// Replaces every window and tab with `session`.
    fn switch_to(&mut self, mut session: Session) {
        repair(&self.mounts, &self.home, &mut session);
        let settings = self.model.settings.clone();
        let jobs = std::mem::take(&mut self.model.jobs);
        let mut model = Model::from_session(settings, &session, &self.home);
        model.jobs = jobs;
        self.model = model;
        // The new model numbers its panes and tabs from one again: a
        // listing of the old model under the same numbers would pass for
        // the new pane's and leave it empty, so every pane lists afresh.
        for (_, live) in std::mem::take(&mut self.live) {
            live.cancel.cancel();
            for child in live.children.values() {
                child.cancel.cancel();
            }
        }
    }

    fn save_layout(&mut self, name: &str, events: &mut Vec<Event>) {
        let name = name.trim();
        if name.is_empty() || name == model::LAST_SESSION {
            events.push(Event::Notify {
                level: Level::Warning,
                text: format!("\"{name}\" cannot name a layout"),
            });
            return;
        }
        let path = session::layout_path(&self.layouts_dir, name);
        match session::save(&path, &self.model.session()) {
            Ok(()) => events.push(Event::Notify {
                level: Level::Info,
                text: format!("Layout {name} saved"),
            }),
            Err(error) => events.push(Event::Notify {
                level: Level::Error,
                text: format!("Layout {name} not saved: {error}"),
            }),
        }
        self.layouts = session::layouts(&self.layouts_dir);
    }

    fn delete_layout(&mut self, name: &str, events: &mut Vec<Event>) {
        let path = session::layout_path(&self.layouts_dir, name);
        if let Err(error) = std::fs::remove_file(&path) {
            events.push(Event::Notify {
                level: Level::Error,
                text: format!("Layout {name} not deleted: {error}"),
            });
        }
        if self.model.settings.session.start_layout() == Some(name) {
            self.change_settings(SettingsChange::StartLayout(None));
        }
        self.layouts = session::layouts(&self.layouts_dir);
    }

    fn rename_layout(&mut self, from: &str, to: &str, events: &mut Vec<Event>) {
        let to = to.trim();
        if to.is_empty() || to == model::LAST_SESSION {
            events.push(Event::Notify {
                level: Level::Warning,
                text: format!("\"{to}\" cannot name a layout"),
            });
            return;
        }
        let source = session::layout_path(&self.layouts_dir, from);
        let target = session::layout_path(&self.layouts_dir, to);
        if source == target {
            return;
        }
        if let Err(error) = std::fs::rename(&source, &target) {
            events.push(Event::Notify {
                level: Level::Error,
                text: format!("Layout {from} not renamed: {error}"),
            });
        } else if self.model.settings.session.start_layout() == Some(from) {
            self.change_settings(SettingsChange::StartLayout(Some(to.to_owned())));
        }
        self.layouts = session::layouts(&self.layouts_dir);
    }

    /// The entry the preview shows: the single selected file of the
    /// explorer tab last focused ([`Model::preview_follows`]), with the pane
    /// holding that tab. The next [`Event::PreviewReady`] or
    /// [`Event::PreviewFailed`] for that pane is about it.
    #[must_use]
    pub fn preview_target(&self) -> Option<(PaneId, EntryId)> {
        let pane = self.preview_pane()?;
        self.preview.get(&pane).map(|job| (pane, job.entry))
    }

    /// The entry a pane is opening or showing: the Preview tab's file when
    /// `pane` holds the explorer tab the preview follows. The
    /// [`Event::PreviewReady`] and [`Event::PreviewFailed`] for the pane
    /// are about it.
    #[must_use]
    pub fn preview_entry(&self, pane: PaneId) -> Option<EntryId> {
        self.preview.get(&pane).map(|job| job.entry)
    }

    /// The pane holding the explorer tab the preview follows, whose
    /// selection the Preview tab shows; `None` while no explorer tab has
    /// had the focus.
    #[must_use]
    pub fn preview_pane(&self) -> Option<PaneId> {
        self.model
            .preview_follows()
            .and_then(|tab| self.model.pane_of(tab))
    }

    /// The model the UI renders from.
    #[must_use]
    pub fn model(&self) -> &Model {
        &self.model
    }

    /// The loaded extensions' contributions.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The provider table.
    #[must_use]
    pub fn mounts(&self) -> &Arc<Mounts> {
        &self.mounts
    }

    /// The platform services the application was built with.
    #[must_use]
    pub fn platform(&self) -> &Arc<dyn Platform> {
        &self.platform
    }

    /// Sets the function called when a listing, watch or settings message
    /// arrives, so a UI can request a repaint instead of polling. Job events
    /// are not announced this way; poll while [`is_busy`](Self::is_busy).
    pub fn set_waker(&self, waker: Waker) {
        *self
            .mailbox
            .waker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(waker);
    }

    /// The runner every operation runs on.
    #[must_use]
    pub fn jobs(&self) -> &JobRunner {
        &self.jobs
    }

    /// Where the settings are read from and written to.
    #[must_use]
    pub fn settings_path(&self) -> &Path {
        &self.settings_path
    }

    /// The label of the operation `Undo` would revert.
    #[must_use]
    pub fn undo_label(&self) -> Option<String> {
        self.undo.last().map(|c| c.label())
    }

    /// The label of the operation `Redo` would apply again.
    #[must_use]
    pub fn redo_label(&self) -> Option<String> {
        self.redo.last().map(|c| c.label())
    }

    /// Whether a listing is still streaming or a job still running, the
    /// decorators' jobs included (a Git status being read, for instance),
    /// or an entry's properties are still being read for the Properties
    /// dialogue: whether an event is still to come without a new command.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.live.values().any(|l| !l.complete)
            || !self.jobs.running().is_empty()
            || !self.decor_jobs.running().is_empty()
            || self.properties_reading > 0
    }

    /// The context an action gets for `pane`: its location and selection,
    /// plus the location of the window's other explorer pane when there is
    /// one. `None` when the pane shows the Preview tab, a console or a
    /// comparison, which have no entries.
    #[must_use]
    pub fn action_ctx(&self, pane: PaneId, invoked_from: Invocation) -> Option<ActionCtx> {
        let (_, tab) = self.model.active_tab(pane)?;
        if !tab.kind.lists() {
            return None;
        }
        Some(ActionCtx {
            pane,
            location: tab.location.clone(),
            selection: tab.selected().cloned().collect(),
            invoked_from,
            other: self.other_location(pane),
        })
    }

    /// The location of the first other pane of the pane's window whose
    /// active tab is an explorer: the other side of a two-pane layout.
    fn other_location(&self, pane: PaneId) -> Option<Location> {
        let (window, workspace) = self.model.workspace_of(pane)?;
        self.model
            .workspaces(window)
            .iter()
            .find(|ws| ws.id == workspace)?
            .dock
            .panes()
            .into_iter()
            .filter(|p| *p != pane)
            .find_map(|p| {
                let (_, tab) = self.model.active_tab(p)?;
                (tab.kind == model::TabKind::Explorer).then(|| tab.location.clone())
            })
    }

    /// Handles one command: mutates the model and appends the events it
    /// produced right away. Listings and jobs the command started report
    /// through [`poll`](Self::poll) and [`tick`](Self::tick).
    pub fn handle(&mut self, command: Command, events: &mut Vec<Event>) {
        debug!(?command, "command");
        self.dispatch(command, events);
        self.sync_panes(events);
        self.sync_consoles(events);
        self.sync_preview();
        self.sync_session();
    }

    // One arm per command: the enum is the table.
    #[allow(clippy::too_many_lines)]
    fn dispatch(&mut self, command: Command, events: &mut Vec<Event>) {
        match command {
            Command::Navigate { pane, to } => {
                // Where the tab is already, navigating is a refresh: the
                // Places row of the current folder lists it again rather
                // than emptying it.
                if self
                    .model
                    .active_tab(pane)
                    .is_some_and(|(_, tab)| tab.location == to)
                {
                    self.refresh(pane);
                    return;
                }
                let before = self.selection_len(pane);
                let recent = Virtual::of(&to) == Some(Virtual::Recent);
                if self.model.navigate(pane, to).is_some() {
                    Self::selection_cleared(pane, before, events);
                    if recent {
                        self.model
                            .set_sort(pane, Column::Accessed, SortDir::Descending);
                    }
                }
            }
            Command::Open { pane, id } => self.open(pane, id, events),
            Command::Back(pane) => {
                let before = self.selected_ids(pane);
                if self.model.back(pane).is_some() {
                    self.selection_restored(pane, &before, events);
                }
            }
            Command::Forward(pane) => {
                let before = self.selected_ids(pane);
                if self.model.forward(pane).is_some() {
                    self.selection_restored(pane, &before, events);
                }
            }
            Command::SpringLoad { pane, to, first } => {
                let before = self.selection_len(pane);
                if self.model.spring_load(pane, to, first).is_some() {
                    Self::selection_cleared(pane, before, events);
                }
            }
            Command::Transfer {
                items,
                from,
                to,
                mode,
            } => {
                let sources: Vec<Location> = items
                    .iter()
                    .filter_map(|id| self.locate(from, *id))
                    .collect();
                self.transfer(sources, &to, mode, from, events);
            }
            Command::Receive { items, to, mode } => {
                let pane = self.model.focused;
                self.transfer(items, &to, mode, pane, events);
            }
            Command::Pin { items, at } => {
                let command = ops::pin_command(self.pin_store(), items, at);
                self.apply_undoable(command, events);
            }
            Command::Unpin(item) => {
                let command = ops::unpin_command(self.pin_store(), item);
                self.apply_undoable(command, events);
            }
            Command::Refresh(pane) => self.refresh(pane),
            Command::Select { pane, change } => {
                // Select all selects what the list shows and nothing else;
                // the hidden entries it leaves out are named by their
                // number (SPEC.md *Select all*).
                let left_out = if change == SelectionChange::All {
                    self.model
                        .active_tab(pane)
                        .map_or(0, |(_, tab)| tab.hidden_left_out(&self.model.settings))
                } else {
                    0
                };
                if let Some(ids) = self.model.select(pane, change) {
                    events.push(Event::SelectionChanged { pane, ids });
                }
                if left_out > 0 {
                    events.push(Event::Notify {
                        level: Level::Info,
                        text: ext::t!("notice-hidden-not-selected", count = left_out),
                    });
                }
            }
            Command::SetView { pane, mode } => {
                self.model.set_view(pane, mode);
            }
            Command::Sort { pane, by, dir } => {
                self.model.set_sort(pane, by, dir);
            }
            Command::GroupBy { pane, by } => {
                self.model.set_group(pane, by);
            }
            Command::Filter { pane, text } => {
                self.model.set_filter(pane, &text);
            }
            Command::TypeFilter { pane, types } => {
                self.model.set_types(pane, types);
            }
            Command::ToggleHidden(pane) => {
                self.model.toggle_hidden(pane);
            }
            Command::Flatten { pane, on } => {
                let before = self.selection_len(pane);
                if self.model.set_flatten(pane, on) {
                    Self::selection_cleared(pane, before, events);
                }
            }
            Command::Subtree { pane, on } => {
                // Only where the location can be searched beneath: the
                // toggle is disabled elsewhere, and so is its shortcut.
                let on =
                    on && self.subtree_search(&self.pane_location(pane)) == Availability::Enabled;
                let before = self.selection_len(pane);
                if self.model.set_subtree(pane, on) {
                    Self::selection_cleared(pane, before, events);
                }
            }
            Command::IndexSearch { text } => self.index_search(&text, events),
            Command::TreeExpand { pane, id } => {
                if self.model.tree_expand(pane, id) {
                    let location = self
                        .model
                        .active_tab(pane)
                        .and_then(|(_, tab)| tab.entry(id).map(|e| tab.location_of(e)));
                    if let Some(location) = location {
                        self.list_child(pane, id, location, false);
                    }
                }
            }
            Command::TreeCollapse { pane, id } => {
                if let Some(live) = self.live.get_mut(&pane)
                    && let Some(child) = live.children.remove(&id)
                {
                    child.cancel.cancel();
                }
                let before = self.selected_ids(pane);
                if self.model.tree_collapse(pane, id) {
                    let after = self.selected_ids(pane);
                    if before != after {
                        events.push(Event::SelectionChanged { pane, ids: after });
                    }
                }
            }
            Command::PlaceAdd { place, at } => {
                if self.model.places.add(place, at) {
                    self.save_places();
                }
            }
            Command::PlaceRemove(index) => {
                if self.model.places.remove(index).is_some() {
                    self.save_places();
                }
            }
            Command::PlaceMove { from, to } => {
                if self.model.places.shift(from, to) {
                    self.save_places();
                }
            }
            Command::PlaceEdit { index, title, icon } => {
                if self.model.places.edit(index, &title, &icon) {
                    self.save_places();
                }
            }
            Command::Eject(mount_point) => self.eject(&mount_point, events),
            Command::Format(mount_point) => self.format(&mount_point, events),
            Command::AddConnection {
                provider,
                name,
                values,
            } => self.add_connection(provider, &name, values, false, events),
            Command::EditConnection {
                provider,
                name,
                values,
            } => self.add_connection(provider, &name, values, true, events),
            Command::Connection { provider, name, op } => {
                self.connection_op(provider, &name, op, events);
            }
            Command::Run { action, ctx } => self.run(action, ctx, events),
            Command::Continue { pane, then, values } => {
                let label = then.label().to_owned();
                let location = self.pane_location(pane);
                let result = then.run(values);
                self.outcome(pane, location, &label, result, events);
                self.check_later(events);
            }
            Command::NewTab(location) => {
                let recent = Virtual::of(&location) == Some(Virtual::Recent);
                let (pane, _) = self.model.new_tab(None, location);
                if recent {
                    self.model
                        .set_sort(pane, Column::Accessed, SortDir::Descending);
                }
            }
            Command::CloseTab(tab) => {
                self.model.close_tab(tab);
            }
            Command::ActivateTab(tab) => {
                self.model.activate(tab);
            }
            Command::FocusPane(pane) => {
                self.model.focus(pane);
            }
            Command::ActivateWorkspace(id) => {
                self.model.activate_workspace(id);
            }
            Command::CloseWorkspace(id) => {
                self.model.close_workspace(id);
            }
            Command::MoveWorkspace { id, to } => {
                self.model.move_workspace(id, to);
            }
            Command::WorkspaceToWindow { id, window } => {
                self.model.workspace_to_window(id, window);
            }
            Command::Split { pane, dir } => {
                self.model.split(pane, dir);
            }
            Command::MoveTab { tab, to } => {
                self.model.move_tab(tab, to);
            }
            Command::DockWorkspace { id, pane, side } => {
                self.model.dock_workspace(id, pane, side);
            }
            Command::TogglePreview { window, fraction } => {
                let shown = self.model.window(window).map(|w| w.active);
                let here = self.model.preview_tab(window).filter(|tab| {
                    self.model
                        .pane_of(*tab)
                        .and_then(|pane| self.model.workspace_of(pane))
                        .map(|(_, ws)| ws)
                        == shown
                });
                match here {
                    Some(tab) => {
                        self.model.close_tab(tab);
                    }
                    None => {
                        self.model.open_preview(window, fraction);
                    }
                }
            }
            Command::CancelJob(id) => {
                self.jobs.cancel(id);
            }
            Command::PauseJob(id) => {
                if self.jobs.pause(id) {
                    self.model.job_paused(id, true);
                }
            }
            Command::ResumeJob(id) => {
                if self.jobs.resume(id) {
                    self.model.job_paused(id, false);
                }
            }
            Command::DismissJob(id) => {
                self.model.job_dismiss(id);
            }
            Command::ResolveConflict { id, answer } => {
                if !self.conflicts.answer(id, answer) {
                    debug!(%id, "no job waits on this conflict");
                }
                self.model.conflict_closed(id);
            }
            Command::AnswerQuestion { id, answer } => {
                if !self.prompts.answer(id, answer) {
                    debug!(%id, "nobody waits on this question");
                }
                self.model.question_closed(id);
            }
            Command::ShowProperties(pane) => self.show_properties(pane, events),
            Command::PropertiesClosed => {
                self.properties_shown = None;
                // A request still on its way is called off with it.
                self.properties_waiting = None;
                if let Some(count) = self.properties_count.take() {
                    count.cancel.cancel();
                }
            }
            Command::HashFormat(format) => {
                if let Err(error) = ops::hash::remember(&*self.platform, format) {
                    events.push(Event::Notify {
                        level: Level::Error,
                        text: format!("Hash format: {error}"),
                    });
                }
            }
            Command::SystemProperties(pane) => self.system_properties(pane, events),
            Command::SetPermissions { pane, request } => self.set_permissions(pane, request),
            Command::RefreshVolumes => self.enumerate_volumes(),
            Command::Undo => self.undo(events),
            Command::Redo => self.redo(events),
            Command::Settings(change) => self.change_settings(change),
            Command::ExtSetting { ext, key, value } => {
                if let Err(error) = self.ext_settings.set(&ext, &key, value) {
                    events.push(Event::Notify {
                        level: Level::Error,
                        text: format!("Settings of {ext} not saved: {error}"),
                    });
                }
            }
            Command::Thumbnail { pane, id, size } => self.thumbnail(pane, id, size),
            Command::PreviewMore(pane) => {
                if let Some(job) = self.preview.get(&pane) {
                    let id = job.entry;
                    let count = self
                        .preview_more
                        .get(&pane)
                        .filter(|(e, _)| *e == id)
                        .map_or(0, |(_, n)| *n);
                    self.preview_more.insert(pane, (id, count + 1));
                    self.restart_preview(pane);
                }
            }
            Command::PreviewAnyway(pane) => {
                if let Some(job) = self.preview.get(&pane) {
                    self.preview_force.insert(pane, job.entry);
                    self.restart_preview(pane);
                }
            }
            Command::PreviewCancel(pane) => self.cancel_preview(pane, events),
            Command::InstallPlugin(package) => self.install_plugin(package),
            Command::RemovePlugin(path) => self.remove_plugin(path),
            Command::PreviewWith { pane, previewer } => {
                if let Some(job) = self.preview.get(&pane) {
                    self.preview_with.insert(pane, (job.entry, previewer));
                    self.restart_preview(pane);
                }
            }
            Command::PreviewAlways(pane) => self.preview_always(pane),
            Command::SetPreviewAlways { provider, name, on } => {
                self.set_preview_always(provider, &name, on);
            }
            Command::PreviewRetry(pane) => self.restart_preview(pane),
            Command::PreviewCopy(pane) => self.copy_preview_text(pane),
            Command::Layout { window, dock } => {
                self.model.set_layout(window, dock);
            }
            Command::Scroll { tab, offset } => {
                self.model.set_scroll(tab, offset);
            }
            Command::ColumnWidths { tab, columns } => {
                self.model.set_column_widths(tab, columns);
            }
            Command::WindowGeometry {
                window,
                geometry,
                monitor,
            } => {
                self.model.set_geometry(window, geometry, monitor);
            }
            Command::SidePanel { window, change } => {
                self.model.side_panel(window, change);
            }
            Command::FocusWindow(window) => {
                self.model.focus_window(window);
            }
            Command::NewWindow => self.new_window(events),
            Command::NewWindowAt(location) => {
                self.model.new_window(location);
            }
            Command::Reveal {
                items,
                new_window,
                properties,
            } => self.reveal(&items, new_window, properties),
            Command::Activate { token } => events.push(Event::Activate {
                window: self.model.focused_window,
                token,
            }),
            Command::CloseWindow(window) => {
                if !self.model.close_window(window) && self.model.window(window).is_some() {
                    self.exit = true;
                }
            }
            Command::SaveLayout(name) => self.save_layout(&name, events),
            Command::SwitchLayout(name) => {
                let session = match name {
                    Some(name) => self.load_layout(&name, events),
                    None => match self.last_session.clone() {
                        Some(session) => Some(session),
                        None => match session::load(self.session_saver.path()) {
                            Ok(session) => Some(session),
                            Err(error) => {
                                events.push(Event::Notify {
                                    level: Level::Warning,
                                    text: format!("No last session: {error}"),
                                });
                                None
                            }
                        },
                    },
                };
                if let Some(session) = session {
                    self.switch_to(session);
                }
            }
            Command::DeleteLayout(name) => self.delete_layout(&name, events),
            Command::RenameLayout { from, to } => self.rename_layout(&from, &to, events),
            Command::Integration(command) => self.integration(command, events),
        }
        self.sync_panes(events);
        self.sync_preview();
    }

    /// Processes what has arrived from listings, watches and jobs without
    /// waiting, for at most `POLL_BUDGET` (8 ms) so a stream that keeps
    /// producing (a search over thousands of matches) never holds a frame
    /// back; the rest is picked up by the next poll. Returns how many
    /// messages it handled.
    pub fn poll(&mut self, events: &mut Vec<Event>) -> usize {
        let started = std::time::Instant::now();
        events.append(&mut self.pending);
        self.check_later(events);
        self.sync_search_indexes();
        let mut handled = 0;
        loop {
            let before = handled;
            while let Ok(msg) = self.rx.try_recv() {
                self.on_msg(msg, events);
                handled += 1;
                if started.elapsed() >= POLL_BUDGET {
                    break;
                }
            }
            while let Ok(event) = self.job_events.try_recv() {
                self.on_job_event(event, events);
                handled += 1;
                if started.elapsed() >= POLL_BUDGET {
                    break;
                }
            }
            if handled == before || started.elapsed() >= POLL_BUDGET {
                break;
            }
        }
        self.sync_consoles(events);
        if handled > 0 {
            self.sync_preview();
            self.sync_session();
        }
        handled
    }

    /// Waits for the next message from a listing, watch or job, then
    /// processes it and everything else pending.
    pub async fn tick(&mut self, events: &mut Vec<Event>) {
        if !self.pending.is_empty() {
            events.append(&mut self.pending);
            self.poll(events);
            return;
        }
        tokio::select! {
            Some(msg) = self.rx.recv() => self.on_msg(msg, events),
            Some(event) = self.job_events.recv() => self.on_job_event(event, events),
        }
        self.poll(events);
    }

    fn undo(&mut self, events: &mut Vec<Event>) {
        let Some(command) = self.undo.pop() else {
            return;
        };
        match command.undo(&self.jobs) {
            Ok(()) => self.redo.push(command),
            Err(error) => events.push(Event::Notify {
                level: Level::Error,
                text: format!("Undo {}: {error}", command.label()),
            }),
        }
        self.drain_pins();
        self.attribute(self.model.focused, events);
    }

    fn redo(&mut self, events: &mut Vec<Event>) {
        let Some(command) = self.redo.pop() else {
            return;
        };
        match command.redo(&self.jobs) {
            Ok(()) => self.undo.push(command),
            Err(error) => events.push(Event::Notify {
                level: Level::Error,
                text: format!("Redo {}: {error}", command.label()),
            }),
        }
        self.drain_pins();
        self.attribute(self.model.focused, events);
    }

    /// Where the entry `id` dragged out of `from` is: in that pane's
    /// listing, in any other tab's, or in the selection a spring-loading
    /// step of that pane recorded before it moved on.
    fn locate(&self, from: PaneId, id: EntryId) -> Option<Location> {
        let location = |entry: &Entry| Location::new(entry.id.provider, entry.path.clone());
        if let Some((_, tab)) = self.model.active_tab(from)
            && let Some(entry) = tab.listing.get(id)
        {
            return Some(location(entry));
        }
        for (pane, _) in self.model.panes() {
            if let Some((_, tab)) = self.model.active_tab(pane)
                && let Some(entry) = tab.listing.get(id)
            {
                return Some(location(entry));
            }
        }
        let (_, tab) = self.model.active_tab(from)?;
        tab.history
            .back
            .iter()
            .rev()
            .flat_map(|step| step.selection.iter())
            .find(|entry| entry.id == id)
            .map(location)
    }

    /// The pinned list as the pin actions and commands see it.
    fn pin_store(&self) -> Arc<dyn PinStore> {
        Arc::clone(&self.pins) as Arc<dyn PinStore>
    }

    /// Applies the pin changes an action or an undo asked for to the
    /// settings, and writes the file.
    fn drain_pins(&mut self) {
        for change in self.pins.take() {
            self.change_settings(change);
        }
        let pinned = self.model.settings.pinned.clone();
        self.pins.sync(&pinned);
    }

    /// Makes every polled location check for a change now, after an
    /// operation that may have changed it.
    fn poll_now(&self) {
        self.poke.send_modify(|n| *n += 1);
    }

    /// Lists the trash again, off the loop's thread, with the bytes its
    /// items take, for the Places row's pill; a change makes the Trash view
    /// list again too.
    fn refresh_trash(&mut self) {
        let platform = Arc::clone(&self.platform);
        let tx = self.mailbox.clone();
        runtime::handle().spawn(async move {
            let result = runtime::handle()
                .spawn_blocking(move || {
                    platform.list().map(|items| {
                        let size = items.iter().map(|item| tree_size(&item.trashed)).sum();
                        (items, size)
                    })
                })
                .await
                .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))));
            tx.send(Msg::Trash(result));
        });
    }

    /// The share of its disk the trash takes: its size against the total of
    /// the volume whose mount point is the longest prefix of a trashed
    /// item's path.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn trash_share(&self, items: &[TrashedItem], size: u64) -> Option<f32> {
        let inside = items.first()?.trashed.as_path();
        let volume = self
            .model
            .volumes
            .iter()
            .filter(|v| inside.starts_with(&v.mount_point))
            .max_by_key(|v| v.mount_point.as_os_str().len())?;
        let total = volume.total.filter(|t| *t > 0)?;
        Some((size as f64 / total as f64) as f32)
    }

    /// Enumerates the mounted volumes once, off the loop's thread.
    fn enumerate_volumes(&self) {
        let platform = Arc::clone(&self.platform);
        let tx = self.mailbox.clone();
        runtime::handle().spawn(async move {
            let result = runtime::handle()
                .spawn_blocking(move || platform.volumes())
                .await
                .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))));
            tx.send(Msg::Volumes(result));
        });
    }

    /// Enumerates the volumes now, on every mount and unmount event of the
    /// platform and every [`VOLUMES_EVERY`], so media that come and go show
    /// up without a user action (SPEC.md *This Machine*): the list is never
    /// one read at start, and a partition that mounts a moment after its
    /// stick was plugged in is read like the first.
    fn poll_volumes(&mut self) {
        let platform = Arc::clone(&self.platform);
        let tx = self.mailbox.clone();
        let cancel = self.volumes_watch.clone();
        let mounted = Arc::new(tokio::sync::Notify::new());
        let ring = Arc::clone(&mounted);
        self.mounts_watch = self
            .platform
            .watch_mounts(Box::new(move || ring.notify_one()));
        runtime::handle().spawn(async move {
            loop {
                let platform = Arc::clone(&platform);
                let (result, format_tool) = runtime::handle()
                    .spawn_blocking(move || (platform.volumes(), platform.has_format_tool()))
                    .await
                    .unwrap_or_else(|join| (Err(Error::Io(std::io::Error::other(join))), false));
                if !tx.send(Msg::Volumes(result)) || !tx.send(Msg::FormatTool(format_tool)) {
                    break;
                }
                tokio::select! {
                    () = cancel.cancelled() => break,
                    () = tokio::time::sleep(VOLUMES_EVERY) => {}
                    () = mounted.notified() => {
                        tokio::select! {
                            () = cancel.cancelled() => break,
                            () = tokio::time::sleep(VOLUMES_AFTER_EVENT) => {}
                        }
                    }
                }
            }
        });
    }

    /// Brings the model's *Waiting* rows in line with the scheduler's queue.
    fn sync_waiting(&mut self) {
        let now: BTreeMap<JobId, String> = self.scheduler.waiting().into_iter().collect();
        for id in self.waiting.clone() {
            if !now.contains_key(&id) {
                self.model.job_waiting(id, None);
                self.waiting.remove(&id);
            }
        }
        for (id, for_name) in now {
            // A waiting job that has no row yet (its `Started` event is still
            // in the channel) is picked up on the next change.
            if self.model.job_waiting(id, Some(for_name)) {
                self.waiting.insert(id);
            }
        }
    }

    /// Hands the saver the Places list as it is now.
    fn save_places(&self) {
        let _ = self.places_saver.send(self.model.places.rows().to_vec());
    }

    /// Opens the system's own format tool on the volume at `mount_point`:
    /// GNOME Disks or the desktop's equivalent, Disk Utility, the format
    /// dialogue of Windows. Rick formats nothing itself.
    fn format(&mut self, mount_point: &Path, events: &mut Vec<Event>) {
        let Some(volume) = self
            .model
            .volumes
            .iter()
            .find(|v| v.mount_point == mount_point)
            .cloned()
        else {
            events.push(Event::Notify {
                level: Level::Warning,
                text: format!("{} is not mounted", mount_point.display()),
            });
            return;
        };
        // The entry is disabled where the system has no such tool; a
        // command that comes all the same is told what the tooltip says.
        if !self.platform.has_format_tool() {
            events.push(Event::Notify {
                level: Level::Warning,
                text: ext::t!("panel-no-disk-utility"),
            });
            return;
        }
        if let Err(error) = self.platform.open_format_tool(&volume) {
            events.push(Event::Notify {
                level: Level::Error,
                text: format!("Format {}: {error}", volume.name),
            });
        }
    }

    /// Ejects the removable volume at `mount_point` as a job, then lists
    /// the volumes again.
    fn eject(&mut self, mount_point: &Path, events: &mut Vec<Event>) {
        let Some(volume) = self
            .model
            .volumes
            .iter()
            .find(|v| v.mount_point == mount_point)
            .cloned()
        else {
            events.push(Event::Notify {
                level: Level::Warning,
                text: format!("{} is not mounted", mount_point.display()),
            });
            return;
        };
        // The panes on the drive leave it first, so that none points at a
        // drive that is gone and no listing keeps it busy while it is
        // unmounted.
        self.evacuate(
            &volume.name,
            &|location| on_volume(&volume.mount_point, location),
            events,
        );
        self.sync_panes(events);
        let platform = Arc::clone(&self.platform);
        let name = volume.name.clone();
        let handle = self
            .jobs
            .spawn_named(format!("Eject {name}"), move |job| async move {
                job.progress
                    .report(Progress::Message(format!("Ejecting {name}")));
                runtime::handle()
                    .spawn_blocking(move || platform.eject(&volume))
                    .await
                    .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))))
            });
        let id = handle.id;
        // Whatever happened, the list may have changed.
        let tx = self.mailbox.clone();
        let platform = Arc::clone(&self.platform);
        runtime::handle().spawn(async move {
            let _ = handle.result.await;
            let result = runtime::handle()
                .spawn_blocking(move || platform.volumes())
                .await
                .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))));
            tx.send(Msg::Volumes(result));
        });
        debug!(%id, "eject started");
        self.attribute(self.model.focused, events);
    }

    /// Lists the cloud providers' roots for the side panel's Cloud rows,
    /// as an internal job.
    fn refresh_cloud(&mut self) {
        if self.model.cloud_providers.is_empty() {
            return;
        }
        self.refresh_overview(Virtual::Cloud);
    }

    /// Lists the network providers' roots for the side panel's Network
    /// rows, as an internal job.
    fn refresh_network(&mut self) {
        if self.model.network_providers.is_empty() {
            return;
        }
        self.refresh_overview(Virtual::Network);
    }

    /// Lists an overview through the `rick://` provider and hands the rows
    /// to the model: the Cloud rows as they are, the Network rows without
    /// the mounted shares, which the volumes list already.
    fn refresh_overview(&mut self, which: Virtual) {
        let provider: Arc<dyn ext::Provider> = Arc::clone(&self.rick) as Arc<dyn ext::Provider>;
        let tx = self.mailbox.clone();
        let handle =
            self.jobs
                .spawn_named(format!("List {}", which.title()), move |job| async move {
                    let location = which.location();
                    let mut stream =
                        provider.list(&location, ListOptions::default(), job.cancel.clone());
                    let mut entries = Vec::new();
                    while let Some(batch) = stream.next().await {
                        match batch {
                            Ok(batch) => entries.extend(batch),
                            Err(error) => {
                                debug!(%error, overview = which.name(), "roots not listed");
                                break;
                            }
                        }
                    }
                    tx.send(match which {
                        Virtual::Network => Msg::Network(provider_rows(entries)),
                        _ => Msg::Cloud(entries),
                    });
                    Ok(())
                });
        self.internal.insert(handle.id);
    }

    /// Lists the rows of the section `provider` belongs to again.
    fn refresh_rows_of(&mut self, provider: ProviderId) {
        if vfs_rick::is_cloud_provider(provider) {
            self.rick.changed(Virtual::Cloud);
            self.refresh_cloud();
        } else {
            self.rick.changed(Virtual::Network);
            self.refresh_network();
        }
    }

    /// Adds a connection, or with `editing` replaces one: the secret fields
    /// go to the keychain as a job, the rest follow through
    /// [`Msg::ConnectionSaved`] once they are in. An edit that gives no
    /// value for a secret keeps the keychain's.
    fn add_connection(
        &mut self,
        provider: ProviderId,
        name: &str,
        values: Vec<(String, String)>,
        editing: bool,
        events: &mut Vec<Event>,
    ) {
        let Some(connector) = self.registry.connector_of(provider).cloned() else {
            events.push(Event::Notify {
                level: Level::Error,
                text: format!("No connections for provider {provider}"),
            });
            return;
        };
        let name = name.trim().to_owned();
        if name.is_empty() || name.contains(['/', '\\']) {
            events.push(Event::Notify {
                level: Level::Error,
                text: "A connection needs a name without slashes".to_owned(),
            });
            return;
        }
        let exists = connections_of(&self.model.settings, provider)
            .iter()
            .any(|c| c.name == name);
        if exists != editing {
            events.push(Event::Notify {
                level: Level::Error,
                text: if editing {
                    format!("No {} connection called {name} to edit", provider.0)
                } else {
                    format!("A {} connection called {name} exists already", provider.0)
                },
            });
            return;
        }
        let schema = connector.schema().clone();
        let mut entry = ConnectionEntry {
            provider: provider.0.to_owned(),
            name: name.clone(),
            settings: std::collections::BTreeMap::new(),
        };
        let mut secrets: Vec<(String, String)> = Vec::new();
        for (key, value) in values {
            if value.is_empty() {
                continue;
            }
            match schema.field(&key).map(|f| &f.kind) {
                Some(ext::ConnectionFieldKind::Secret) => {
                    secrets.push((ext::secret_key(provider, &name, &key), value));
                }
                Some(_) => {
                    entry.settings.insert(key, value);
                }
                None => {
                    debug!(key, "connection field the schema does not name; dropped");
                }
            }
        }
        if let Some(missing) = schema
            .fields
            .iter()
            .filter(|f| f.required)
            .find(|f| match f.kind {
                // An edit keeps the secret it has when none is typed.
                ext::ConnectionFieldKind::Secret => {
                    !editing
                        && !secrets
                            .iter()
                            .any(|(k, _)| k == &ext::secret_key(provider, &name, f.key))
                }
                _ => !entry.settings.contains_key(f.key),
            })
        {
            events.push(Event::Notify {
                level: Level::Error,
                text: format!("{} needs a value", missing.label),
            });
            return;
        }
        let platform = Arc::clone(&self.platform);
        let tx = self.mailbox.clone();
        let handle =
            self.jobs
                .spawn_named(format!("Add connection {name}"), move |_job| async move {
                    let stored = runtime::handle()
                        .spawn_blocking(move || {
                            for (key, value) in &secrets {
                                platform.store_secret(key, value)?;
                            }
                            Ok::<(), Error>(())
                        })
                        .await
                        .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))));
                    stored?;
                    tx.send(Msg::ConnectionSaved(entry));
                    Ok(())
                });
        self.pane_jobs.insert(handle.id, self.model.focused);
    }

    /// The secrets are in: writes the connection to the settings, tells
    /// the connector and lists the section's rows again.
    fn connection_saved(&mut self, entry: ConnectionEntry) {
        let provider = ProviderId::intern(&entry.provider);
        let mut connections = self.model.settings.connections.clone();
        connections.retain(|c| !(c.provider == entry.provider && c.name == entry.name));
        connections.push(entry);
        self.save_connections(connections);
        self.push_connections(provider);
        self.refresh_rows_of(provider);
    }

    /// Connect, disconnect or remove a connection.
    fn connection_op(
        &mut self,
        provider: ProviderId,
        name: &str,
        op: ConnectionOp,
        events: &mut Vec<Event>,
    ) {
        let Some(connector) = self.registry.connector_of(provider).cloned() else {
            events.push(Event::Notify {
                level: Level::Error,
                text: format!("No connections for provider {provider}"),
            });
            return;
        };
        match op {
            ConnectionOp::Connect => {
                let name = name.to_owned();
                let tx = self.mailbox.clone();
                let handle =
                    self.jobs
                        .spawn_named(format!("Connect {name}"), move |job| async move {
                            job.progress
                                .report(Progress::Message(format!("Connecting to {name}")));
                            let result = connector.connect(&name).await;
                            tx.send(Msg::ConnectionChanged(provider));
                            result
                        });
                self.pane_jobs.insert(handle.id, self.model.focused);
            }
            ConnectionOp::Disconnect => {
                connector.disconnect(name);
                self.refresh_rows_of(provider);
            }
            ConnectionOp::Remove => {
                let keys: Vec<String> = connector
                    .schema()
                    .secret_fields()
                    .map(|field| ext::secret_key(provider, name, field))
                    .collect();
                connector.disconnect(name);
                let mut connections = self.model.settings.connections.clone();
                connections.retain(|c| !(c.provider == provider.0 && c.name == name));
                self.save_connections(connections);
                self.push_connections(provider);
                let platform = Arc::clone(&self.platform);
                let label = name.to_owned();
                let handle = self.jobs.spawn_named(
                    format!("Remove connection {label}"),
                    move |_job| async move {
                        runtime::handle()
                            .spawn_blocking(move || {
                                for key in &keys {
                                    platform.remove_secret(key)?;
                                }
                                Ok::<(), Error>(())
                            })
                            .await
                            .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))))
                    },
                );
                self.pane_jobs.insert(handle.id, self.model.focused);
                self.refresh_rows_of(provider);
                // The panes on the account or the host leave with it.
                self.evacuate(
                    name,
                    &|location| {
                        location.provider == provider
                            && source_at(location).as_deref() == Some(name)
                    },
                    events,
                );
            }
        }
    }

    /// Sends every pane on a location `gone` names Home, at once, and says
    /// `<name> was removed` when one went: a medium that was pulled out or
    /// ejected, an account or a host that was removed (SPEC.md *Removed and
    /// unreachable locations*). It is no error, and none is shown.
    fn evacuate(&mut self, name: &str, gone: &dyn Fn(&Location) -> bool, events: &mut Vec<Event>) {
        let home = self.home.clone();
        let moved = self.model.evacuate(gone, &home);
        if moved > 0 {
            debug!(name, moved, "panes went home");
            events.push(Event::Notify {
                level: Level::Info,
                text: ext::t!("notice-was-removed", name = name.to_owned()),
            });
        }
    }

    /// The pane's listing failed because its location could not be
    /// reached: the tab stays where it is and shows the card with *Retry*,
    /// and Rick tries again by itself after each of [`RETRY_LOST_AFTER`].
    fn lost(&mut self, pane: PaneId, generation: u64, location: &Location, error: &Error) {
        let retries = self
            .model
            .active_tab(pane)
            .and_then(|(_, tab)| tab.lost.as_ref())
            .map_or(0, |lost| lost.retries);
        let connector = self.registry.connector_of(location.provider);
        let lost = model::ConnectionLost {
            source: connector
                .as_ref()
                .and_then(|_| source_at(location))
                .unwrap_or_else(|| location.provider.0.to_owned()),
            provider: connector.map_or_else(
                || location.provider.0.to_owned(),
                |connector| connector.schema().title.get(),
            ),
            // The provider's own words, without the kernel's prefix.
            message: match error {
                Error::Io(io) => io.to_string(),
                other => other.to_string(),
            },
            retries,
        };
        self.model.connection_lost(pane, Some(lost));
        self.model.listing_failed(pane, None);
        if let Some(after) = RETRY_LOST_AFTER.get(usize::from(retries)).copied() {
            let tx = self.mailbox.clone();
            runtime::handle().spawn(async move {
                tokio::time::sleep(after).await;
                tx.send(Msg::RetryLost { pane, generation });
            });
        }
    }

    /// Tries a lost connection again, when the pane is still where it was
    /// and still without its listing.
    fn retry_lost(&mut self, pane: PaneId, generation: u64) {
        let current = self.live.get(&pane).map(|live| live.generation);
        let Some(mut lost) = self
            .model
            .active_tab(pane)
            .and_then(|(_, tab)| tab.lost.clone())
        else {
            return;
        };
        if current != Some(generation) {
            return;
        }
        lost.retries += 1;
        debug!(
            ?pane,
            retries = lost.retries,
            "trying a lost connection again"
        );
        self.model.connection_lost(pane, Some(lost));
        self.refresh(pane);
    }

    /// The connection (one SSH host, one cloud account) an entry is on:
    /// the first component of its path, for a provider that keeps
    /// connections as the folders at its root.
    fn source_of(&self, entry: &Entry) -> Option<String> {
        self.registry.connector_of(entry.id.provider)?;
        let bytes = entry.path.as_bytes();
        let trimmed = bytes
            .strip_prefix(b"/")
            .or_else(|| bytes.strip_prefix(b"\\"))
            .unwrap_or(bytes);
        let first = trimmed
            .split(|b| matches!(b, b'/' | b'\\'))
            .next()
            .filter(|first| !first.is_empty())?;
        Some(String::from_utf8_lossy(first).into_owned())
    }

    /// The size-limit card's event for `entry`: its size against the cap,
    /// its source and the name of the provider the source is on.
    fn capped_event(&self, pane: PaneId, entry: &Entry, size: u64, cap: u64) -> Event {
        Event::PreviewCapped {
            pane,
            size,
            cap,
            source: self.source_of(entry),
            provider: self
                .registry
                .connector_of(entry.id.provider)
                .map(|connector| connector.schema().title.get()),
        }
    }

    /// Whether *Always preview* is on for the source `entry` is on: the
    /// switch in the source's connection, whichever provider keeps it.
    fn always_previewed(&self, entry: &Entry) -> bool {
        self.source_of(entry).is_some_and(|source| {
            self.model.settings.connections.iter().any(|c| {
                c.provider == entry.id.provider.0
                    && c.name == source
                    && c.settings
                        .get(PREVIEW_ALWAYS)
                        .is_some_and(|on| on == "true")
            })
        })
    }

    /// Escape during a download: the fetch stops, the file is no longer
    /// previewed whatever its size, and the size-limit card is back.
    fn cancel_preview(&mut self, pane: PaneId, events: &mut Vec<Event>) {
        let Some(job) = self.preview.get(&pane) else {
            return;
        };
        job.cancel.cancel();
        self.preview_force.remove(&pane);
        let entry = job.full.clone();
        let Ok(provider) = self
            .mounts
            .resolve(&Location::new(entry.id.provider, entry.path.clone()))
        else {
            return;
        };
        let loc = Location::new(entry.id.provider, entry.path.clone());
        let cap = provider
            .remote_preview_cap(&loc)
            .unwrap_or(REMOTE_PREVIEW_CAP);
        events.push(self.capped_event(pane, &entry, entry.size.unwrap_or(0), cap));
    }

    /// *Always preview on <source>*: the switch goes on for the source of
    /// the pane's file, and the file opens.
    fn preview_always(&mut self, pane: PaneId) {
        let Some(job) = self.preview.get(&pane) else {
            return;
        };
        let (provider, entry) = (job.full.id.provider, job.entry);
        let Some(source) = self.source_of(&job.full) else {
            return;
        };
        self.set_preview_always(provider, &source, true);
        self.preview_force.insert(pane, entry);
        self.restart_preview(pane);
    }

    /// Writes `preview_always` for the connection `name` of `provider`
    /// into `rick.toml` and tells the connector; a source without a
    /// connection entry (a host of `~/.ssh/config`) gets one that carries
    /// the switch alone (QUESTIONS.md 197).
    fn set_preview_always(&mut self, provider: ProviderId, name: &str, on: bool) {
        let mut connections = self.model.settings.connections.clone();
        let index = connections
            .iter()
            .position(|c| c.provider == provider.0 && c.name == name)
            .unwrap_or_else(|| {
                connections.push(ConnectionEntry {
                    provider: provider.0.to_owned(),
                    name: name.to_owned(),
                    settings: std::collections::BTreeMap::new(),
                });
                connections.len() - 1
            });
        if let Some(entry) = connections.get_mut(index) {
            entry
                .settings
                .insert(PREVIEW_ALWAYS.to_owned(), on.to_string());
        }
        self.save_connections(connections);
        self.push_connections(provider);
    }

    /// Replaces the connections in the settings and saves the file.
    fn save_connections(&mut self, connections: Vec<ConnectionEntry>) {
        let mut settings = self.model.settings.clone();
        settings.connections = connections;
        self.model.apply_settings(settings.clone());
        self.recent_saves.push_back(settings.clone());
        while self.recent_saves.len() > RECENT_SAVES {
            self.recent_saves.pop_front();
        }
        let _ = self.settings_saver.send(settings);
    }

    /// Hands `provider`'s connector the connections as the settings have
    /// them now.
    fn push_connections(&self, provider: ProviderId) {
        if let Some(connector) = self.registry.connector_of(provider) {
            connector.update(connections_of(&self.model.settings, provider));
        }
    }

    /// Lists the children of an expanded folder of a pane in Tree view,
    /// streaming them in, or collecting them to swap in at once when
    /// `reconcile`.
    fn list_child(&mut self, pane: PaneId, folder: EntryId, location: Location, reconcile: bool) {
        let Some(live) = self.live.get_mut(&pane) else {
            return;
        };
        if let Some(old) = live.children.remove(&folder) {
            old.cancel.cancel();
        }
        let generation = live.generation;
        let opts = live.key.opts;
        let Ok(provider) = self.mounts.resolve(&location) else {
            return;
        };
        let tx = self.mailbox.clone();
        let handle = self
            .jobs
            .spawn_named(format!("List {location}"), move |job| async move {
                let opts = ListOptions {
                    depth: Depth::One,
                    ..opts
                };
                let result = async {
                    let mut stream = provider.list(&location, opts, job.cancel.clone());
                    while let Some(batch) = stream.next().await {
                        let batch = batch?;
                        if !tx.send(Msg::TreeBatch {
                            pane,
                            generation,
                            folder,
                            batch,
                        }) {
                            break;
                        }
                    }
                    Ok(())
                }
                .await;
                tx.send(Msg::TreeDone {
                    pane,
                    generation,
                    folder,
                    result: result.as_ref().copied().map_err(Error::replicate),
                });
                result
            });
        self.internal.insert(handle.id);
        if let Some(live) = self.live.get_mut(&pane) {
            live.children.insert(
                folder,
                Child {
                    cancel: handle.cancel,
                    buffer: reconcile.then(Vec::new),
                },
            );
        }
    }

    /// Opens an entry as *Opening files* says (SPEC.md): a folder
    /// navigates; a local archive enters as a read-only location through
    /// the archive provider, which says in an internal job whether it
    /// browses the file, unless `archives.on_open` is `system`; every other
    /// file goes to the system default application. Nothing opens a
    /// viewer: the preview pane is the only one.
    fn open(&mut self, pane: PaneId, id: EntryId, events: &mut Vec<Event>) {
        let Some((tab_id, tab)) = self.model.active_tab(pane) else {
            return;
        };
        if !tab.kind.lists() {
            return;
        }
        let Some(entry) = tab.entry(id).cloned() else {
            return;
        };
        if entry.kind == EntryKind::Dir {
            let to = tab.location_of(&entry);
            self.dispatch(Command::Navigate { pane, to }, events);
            return;
        }
        // A local file with the execute bit is a program, where that bit
        // means one: it is started, not handed to the system (SPEC.md
        // *Executables and scripts*).
        if tab.location.provider == ProviderId::LOCAL
            && entry.attrs.contains(Attrs::EXECUTABLE)
            && self.platform.has_execute_bit()
        {
            self.launch(pane, entry);
            return;
        }
        let archive = (tab.location.provider == ProviderId::LOCAL
            && self.model.settings.archives.on_open == model::OnOpen::Enter)
            .then(|| self.mounts.provider(ProviderId::ARCHIVE))
            .flatten();
        let Some(archive) = archive else {
            self.open_default(pane, entry, events);
            return;
        };
        let tx = self.mailbox.clone();
        let inside = Location::new(ProviderId::ARCHIVE, entry.path.clone());
        let handle = self
            .jobs
            .spawn_named(format!("Open {}", entry.name), move |job| async move {
                let location = match archive.stat(&inside).await {
                    Ok(root) if root.kind == EntryKind::Dir => Some(inside),
                    Ok(_) => None,
                    Err(error) => {
                        debug!(%inside, %error, "not an archive to browse");
                        None
                    }
                };
                if job.cancel.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                tx.send(Msg::Opened {
                    pane,
                    tab: tab_id,
                    entry,
                    location,
                });
                Ok(())
            });
        self.internal.insert(handle.id);
    }

    /// Starts a program the user opened (SPEC.md *Executables and
    /// scripts*). A binary — its first 4 KiB hold a NUL byte — runs
    /// detached from Rick, with its folder as the working directory and
    /// without a question. An executable script is a readable file as
    /// well, so Rick asks first: *Run script?* with **Cancel**, **Edit**
    /// and **Run**; the answer comes back as [`Msg::Script`].
    fn launch(&mut self, pane: PaneId, entry: Entry) {
        let Ok(path) = platform::paths::to_path(&entry.path) else {
            return;
        };
        let platform = Arc::clone(&self.platform);
        let prompts = Arc::clone(&self.prompts);
        let tx = self.mailbox.clone();
        let name = entry.name.clone();
        let handle = self
            .jobs
            .spawn_named(format!("Open {name}"), move |_job| async move {
                let probe = path.clone();
                let binary = runtime::handle()
                    .spawn_blocking(move || is_binary(&probe))
                    .await
                    .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))))?;
                if binary {
                    let dir = path.parent().unwrap_or(&path).to_path_buf();
                    return platform.run_detached(&path, &[], &dir);
                }
                let answer = prompts
                    .ask(
                        ext::t!("run-script-title"),
                        name,
                        QuestionKind::Choice {
                            options: vec![
                                ext::t!("cancel"),
                                ext::t!("run-script-edit"),
                                ext::t!("run-script-run"),
                            ],
                            default: 2,
                        },
                        false,
                    )
                    .await;
                let run = match answer {
                    Answer::Choice(1) => false,
                    Answer::Choice(2) => true,
                    _ => return Ok(()),
                };
                tx.send(Msg::Script { pane, entry, run });
                Ok(())
            });
        self.internal.insert(handle.id);
    }

    /// What the user chose for an executable script: **Run** starts it in
    /// a console in its folder, so that its output is visible — the
    /// embedded console where `ext-action-terminal` is loaded, the
    /// system's terminal otherwise — and **Edit** opens it in the editor.
    fn script(&mut self, pane: PaneId, entry: Entry, run: bool, events: &mut Vec<Event>) {
        let Ok(path) = platform::paths::to_path(&entry.path) else {
            return;
        };
        let dir = path.parent().unwrap_or(&path).to_path_buf();
        let folder = Location::new(ProviderId::LOCAL, platform::paths::from_path(&dir));
        if !run {
            let ctx = ActionCtx {
                pane,
                location: folder,
                selection: vec![entry],
                invoked_from: Invocation::Shortcut,
                other: None,
            };
            self.run(ext::ActionId("core.open_in_editor"), ctx, events);
            return;
        }
        let embedded = self
            .registry
            .actions()
            .iter()
            .any(|action| action.id() == EMBEDDED_CONSOLE);
        if embedded {
            // The console the action opens is handed the line to type.
            self.console_line = Some(run_line(&entry.name));
            let ctx = ActionCtx {
                pane,
                location: folder,
                selection: Vec::new(),
                invoked_from: Invocation::Shortcut,
                other: None,
            };
            self.run(EMBEDDED_CONSOLE, ctx, events);
            self.console_line = None;
        } else if let Err(error) = self
            .platform
            .open_terminal(&dir, Some(&[path.to_string_lossy().into_owned()]))
        {
            events.push(Event::Notify {
                level: Level::Error,
                text: format!("{}: {error}", entry.name),
            });
        }
    }

    /// Opens an entry with the system default application, through the
    /// core action, from the pane's location.
    fn open_default(&mut self, pane: PaneId, entry: Entry, events: &mut Vec<Event>) {
        let Some((_, tab)) = self.model.active_tab(pane) else {
            return;
        };
        let ctx = ActionCtx {
            pane,
            location: tab.location.clone(),
            selection: vec![entry],
            invoked_from: Invocation::Shortcut,
            other: None,
        };
        self.run(ext::ActionId("core.open_default"), ctx, events);
    }

    /// Runs a freshly built undoable command and puts it on the undo log.
    fn apply_undoable(&mut self, command: Box<dyn UndoCommand>, events: &mut Vec<Event>) {
        match command.redo(&self.jobs) {
            Ok(()) => {
                self.undo.push(command);
                self.redo.clear();
            }
            Err(error) => events.push(Event::Notify {
                level: Level::Error,
                text: format!("{}: {error}", command.label()),
            }),
        }
        self.drain_pins();
        self.attribute(self.model.focused, events);
    }

    /// Moves or copies `sources` into `to` as one job, booked to `pane` for
    /// the refresh when it ends. Drops that would change nothing (an item
    /// into its own folder, a folder into itself or its subtree) are
    /// skipped without a word, as the spec asks; `Ask` is the UI's to
    /// answer and is reported when it gets this far.
    fn transfer(
        &mut self,
        sources: Vec<Location>,
        to: &Location,
        mode: TransferMode,
        pane: PaneId,
        events: &mut Vec<Event>,
    ) {
        let sources: Vec<Location> = sources
            .into_iter()
            .filter(|source| !transfer_is_noop(source, to))
            .collect();
        if sources.is_empty() {
            debug!(%to, "transfer with nothing to do");
            return;
        }
        let kind = match mode {
            TransferMode::Move => ops::TransferKind::Move,
            TransferMode::Copy => ops::TransferKind::Copy,
            TransferMode::Ask => {
                events.push(Event::Notify {
                    level: Level::Warning,
                    text: "Move or copy? The drop was not answered; nothing was done.".to_owned(),
                });
                return;
            }
        };
        match ops::transfer(
            &self.jobs,
            Arc::clone(&self.mounts),
            Arc::clone(&self.conflicts),
            Arc::clone(&self.scheduler),
            &sources,
            to,
            kind,
        ) {
            Ok(ActionOutcome::Undoable(command)) => {
                self.undo.push(command);
                self.redo.clear();
            }
            Ok(ActionOutcome::Started(id)) => {
                self.pane_jobs.insert(id, pane);
                let what = match sources.as_slice() {
                    [one] => one.path.file_name().map_or_else(
                        || one.path.to_string(),
                        |n| String::from_utf8_lossy(n).into_owned(),
                    ),
                    many => format!("{} items", many.len()),
                };
                events.push(Event::Notify {
                    level: Level::Info,
                    text: format!("Copying {what}; a copy cannot be undone"),
                });
            }
            // A transfer moves or copies; it selects, opens and navigates
            // nothing, and asks nothing later.
            Ok(
                ActionOutcome::Done
                | ActionOutcome::Select(_)
                | ActionOutcome::View(_)
                | ActionOutcome::Hashes(_)
                | ActionOutcome::Navigate(_)
                | ActionOutcome::Later(_),
            ) => {}
            Err(error) => events.push(Event::Notify {
                level: Level::Error,
                text: format!("Transfer: {error}"),
            }),
        }
        self.attribute(pane, events);
    }

    /// Applies one change and writes the file off the loop's thread; the
    /// writes land in order.
    fn change_settings(&mut self, change: model::SettingsChange) {
        let mut settings = self.model.settings.clone();
        let pinned_before = settings.pinned.clone();
        settings.apply(change);
        if settings.language != self.model.settings.language {
            ext::i18n::set_language(settings.language.as_deref());
        }
        self.pins.sync(&settings.pinned);
        self.pins.sync_open(&settings);
        self.model.apply_settings(settings.clone());
        if settings.pinned != pinned_before {
            self.rick.changed(Virtual::Favorites);
        }
        self.recent_saves.push_back(settings.clone());
        while self.recent_saves.len() > RECENT_SAVES {
            self.recent_saves.pop_front();
        }
        // The saver lives as long as the runtime; a closed channel means
        // the process is going down.
        let _ = self.settings_saver.send(settings);
    }

    /// Reads the selected entry's properties off the UI thread and answers
    /// with `Event::Properties`.
    fn show_properties(&mut self, pane: PaneId, events: &mut Vec<Event>) {
        let Some(ctx) = self.action_ctx(pane, Invocation::Shortcut) else {
            return;
        };
        let Some(entry) = ctx.selection.first().cloned() else {
            // Nothing selected: the folder's own properties (SPEC.md
            // *Context menu*, *Folder properties*).
            self.show_folder_properties(pane, ctx.location, events);
            return;
        };
        let path = (entry.id.provider == vfs_local::PROVIDER_ID)
            .then(|| platform::paths::to_path(&entry.path).ok())
            .flatten();
        self.properties_shown = path.clone().map(|path| (pane, path));
        let folder = path
            .as_deref()
            .filter(|_| entry.kind == types::EntryKind::Dir);
        let contents = self.count_contents(pane, &entry.name, folder);
        let platform = Arc::clone(&self.platform);
        let tx = self.mailbox.clone();
        let location = ctx.location.clone();
        let asked = self.properties_asking();
        runtime::handle().spawn(async move {
            // Every read is answered, one that failed on its thread too:
            // the application is busy until the answer is in.
            let info = runtime::handle()
                .spawn_blocking(move || {
                    read_properties(&*platform, pane, entry, path, &location, contents)
                })
                .await
                .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))));
            tx.send(Msg::Properties(asked, Box::new(info)));
        });
    }

    /// Numbers a request for Properties whose read is about to start: the
    /// dialogue waits for this one now, and the application is busy until
    /// it is answered.
    fn properties_asking(&mut self) -> u64 {
        self.properties_asked += 1;
        self.properties_waiting = Some(self.properties_asked);
        self.properties_reading += 1;
        self.properties_asked
    }

    /// Whether Properties was asked for and its dialogue has not opened
    /// yet, the entry still being read: Escape calls it off with
    /// [`Command::PropertiesClosed`].
    #[must_use]
    pub fn properties_pending(&self) -> bool {
        self.properties_waiting.is_some()
    }

    /// Starts the count of what is inside `folder` for the Properties
    /// dialogue, as a job the operations drawer does not list, and says
    /// where the count stands: a count of the same folder already running
    /// or done is kept, any other is cancelled. `None` counts nothing.
    fn count_contents(&mut self, pane: PaneId, name: &str, folder: Option<&Path>) -> Contents {
        if let (Some(count), Some(folder)) = (&self.properties_count, folder)
            && count.pane == pane
            && count.path == folder
        {
            return count.result.map_or(Contents::Counting, Contents::Counted);
        }
        if let Some(count) = self.properties_count.take() {
            count.cancel.cancel();
        }
        let Some(folder) = folder else {
            return Contents::None;
        };
        let path = folder.to_path_buf();
        let walked = path.clone();
        let tx = self.mailbox.clone();
        let handle = self
            .jobs
            .spawn_named(format!("Count {name}"), move |job| async move {
                let cancel = job.cancel.clone();
                let root = walked.clone();
                let contents = runtime::handle()
                    .spawn_blocking(move || count_tree(&root, &cancel))
                    .await
                    .map_err(|join| Error::Io(std::io::Error::other(join)))?
                    .ok_or(Error::Cancelled)?;
                tx.send(Msg::PropertiesCounted {
                    pane,
                    path: walked,
                    contents,
                });
                Ok(())
            });
        self.internal.insert(handle.id);
        self.properties_count = Some(PropertiesCount {
            pane,
            path,
            cancel: handle.cancel,
            result: None,
        });
        Contents::Counting
    }

    /// The Properties dialogue for the pane's folder itself: the location
    /// is read as an entry through its provider, then as any entry.
    fn show_folder_properties(
        &mut self,
        pane: PaneId,
        location: Location,
        events: &mut Vec<Event>,
    ) {
        let Some(provider) = self.mounts.provider(location.provider) else {
            events.push(Event::Notify {
                level: Level::Info,
                text: "Select an entry to inspect".to_owned(),
            });
            return;
        };
        let path = (location.provider == vfs_local::PROVIDER_ID)
            .then(|| platform::paths::to_path(&location.path).ok())
            .flatten();
        self.properties_shown = path.clone().map(|path| (pane, path));
        let name = runtime::name_of(&location);
        let contents = self.count_contents(pane, &name, path.as_deref());
        let platform = Arc::clone(&self.platform);
        let tx = self.mailbox.clone();
        let asked = self.properties_asking();
        runtime::handle().spawn(async move {
            let entry = match provider.stat(&location).await {
                Ok(entry) => entry,
                Err(error) => {
                    tx.send(Msg::Properties(asked, Box::new(Err(error))));
                    return;
                }
            };
            let info = runtime::handle()
                .spawn_blocking(move || {
                    read_properties(&*platform, pane, entry, path, &location, contents)
                })
                .await
                .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))));
            tx.send(Msg::Properties(asked, Box::new(info)));
        });
    }

    /// Opens the platform's own dialogue on the selected entry.
    fn system_properties(&mut self, pane: PaneId, events: &mut Vec<Event>) {
        let Some(ctx) = self.action_ctx(pane, Invocation::Shortcut) else {
            return;
        };
        let Some(entry) = ctx.selection.first() else {
            return;
        };
        let Ok(path) = platform::paths::to_path(&entry.path) else {
            return;
        };
        if let Err(error) = self.platform.show(&path) {
            events.push(Event::Notify {
                level: Level::Error,
                text: format!("Properties: {error}"),
            });
        }
    }

    /// Starts the permission job and waits for its outcome in the
    /// background.
    fn set_permissions(&mut self, pane: PaneId, request: PermissionRequest) {
        let label = ops::perms::label(&request);
        let handle = ops::perms::spawn(
            &self.jobs,
            Arc::clone(&self.platform),
            label.clone(),
            request.clone(),
        );
        self.pane_jobs.insert(handle.id, pane);
        let tx = self.mailbox.clone();
        runtime::handle().spawn(async move {
            let result = handle
                .result
                .await
                .unwrap_or_else(|_| Err(Error::Cancelled));
            tx.send(Msg::PermissionsApplied {
                pane,
                label,
                request,
                result,
            });
        });
    }

    /// A permission job ended: the step goes on the undo stack, what failed
    /// is reported, and an open dialogue on one of the entries sees the
    /// new state.
    fn permissions_applied(
        &mut self,
        pane: PaneId,
        label: String,
        request: &PermissionRequest,
        result: Result<Applied, Error>,
        events: &mut Vec<Event>,
    ) {
        match result {
            Ok(applied) => {
                if let Some((path, why)) = applied.failed.first() {
                    let name = path.file_name().map_or_else(
                        || path.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    let text = if applied.failed.len() == 1 {
                        format!("{label}: {name} was not changed: {why}")
                    } else {
                        format!(
                            "{label}: {} entries were not changed; {name}: {why}",
                            applied.failed.len()
                        )
                    };
                    events.push(Event::Notify {
                        level: Level::Warning,
                        text,
                    });
                }
                let command =
                    PermissionsCommand::new(Arc::clone(&self.platform), label, request, &applied);
                if !command.is_empty() {
                    self.undo.push(Box::new(command));
                    self.redo.clear();
                }
            }
            Err(Error::Cancelled) => {}
            Err(error) => events.push(Event::Notify {
                level: Level::Error,
                text: format!("{label}: {error}"),
            }),
        }
        if let Some((shown, path)) = &self.properties_shown
            && *shown == pane
            && request.targets.iter().any(|t| t == path)
        {
            self.show_properties(pane, events);
        }
    }

    fn selection_len(&self, pane: PaneId) -> usize {
        self.model
            .active_tab(pane)
            .map_or(0, |(_, tab)| tab.selection.len())
    }

    fn selection_cleared(pane: PaneId, before: usize, events: &mut Vec<Event>) {
        if before > 0 {
            events.push(Event::SelectionChanged {
                pane,
                ids: Vec::new(),
            });
        }
    }

    /// Announces the selection a history step brought back, if it differs
    /// from what was selected before. The listing is empty at this point,
    /// so the ids are the step's own; the rows show up as they stream in.
    fn selection_restored(&self, pane: PaneId, before: &[EntryId], events: &mut Vec<Event>) {
        let after: Vec<EntryId> = self
            .model
            .active_tab(pane)
            .map_or_default(|(_, tab)| tab.selection.iter().copied().collect());
        if after != before {
            events.push(Event::SelectionChanged { pane, ids: after });
        }
    }

    /// Runs the plugin subcommand `name` of the command line
    /// (specs/system-integration.md §4, [`ext::cli`]) with `args`, typed
    /// from the working directory `cwd`, on the focused pane: what an
    /// action would return is applied there like an action's outcome,
    /// text is handed back for the caller's terminal. `None` when no
    /// loaded extension provides the command; the command's own error
    /// otherwise, for the caller rather than a notification.
    pub fn run_cli(
        &mut self,
        name: &str,
        args: Vec<String>,
        cwd: PathBuf,
        events: &mut Vec<Event>,
    ) -> Option<Result<Option<String>, Error>> {
        let command = self
            .registry
            .commands()
            .iter()
            .find(|c| c.name() == name)
            .cloned()?;
        let pane = self.model.focused;
        let location = self.pane_location(pane);
        let ctx = CliCtx {
            args,
            cwd,
            pane,
            location: location.clone(),
        };
        let label = format!("rick {name}");
        Some(match command.run(ctx, &self.jobs, &*self.ui) {
            Ok(CliOutcome::Text(text)) => Ok(Some(text)),
            Ok(CliOutcome::Outcome(outcome)) => {
                self.outcome(pane, location, &label, Ok(outcome), events);
                // A form filled from an earlier look opens at once.
                self.check_later(events);
                Ok(None)
            }
            Err(error) => Err(error),
        })
    }

    fn run(&mut self, action: ext::ActionId, ctx: ActionCtx, events: &mut Vec<Event>) {
        let Some(action) = self
            .registry
            .actions()
            .iter()
            .find(|a| a.id() == action)
            .cloned()
        else {
            events.push(Event::Notify {
                level: Level::Error,
                text: format!("No action {}", action.0),
            });
            return;
        };
        match action.availability(&ctx) {
            Availability::Enabled => {}
            Availability::Disabled(why) => {
                events.push(Event::Notify {
                    level: Level::Warning,
                    text: format!("{}: {why}", action.label()),
                });
                return;
            }
            Availability::Hidden => return,
        }
        let pane = ctx.pane;
        let location = ctx.location.clone();
        let label = action.label();
        let result = action.run(ctx, &self.jobs, &*self.ui);
        self.outcome(pane, location, &label, result, events);
        // A detection kept from an earlier look fills the form at once.
        self.check_later(events);
    }

    /// The location of the pane's active tab, the home when it has none.
    fn pane_location(&self, pane: PaneId) -> Location {
        self.model
            .active_tab(pane)
            .map_or_else(|| self.home.clone(), |(_, tab)| tab.location.clone())
    }

    /// Looks at the forms actions asked for later: a form that arrived is
    /// shown through the host, which runs the rest of the action at once
    /// when it can block and otherwise sends `Command::Continue` later; a
    /// form that will never arrive (its job failed or was cancelled, which
    /// the job reported itself) is forgotten.
    fn check_later(&mut self, events: &mut Vec<Event>) {
        let mut ready = Vec::new();
        self.later
            .retain_mut(|(pane, later)| match later.try_take() {
                FormLaterState::Pending => true,
                FormLaterState::Gone => false,
                FormLaterState::Ready(form, then) => {
                    ready.push((*pane, form, then));
                    false
                }
            });
        for (pane, form, then) in ready {
            let label = then.label().to_owned();
            if let Some(result) = self.ui.form_later(pane, form, then) {
                let location = self.pane_location(pane);
                self.outcome(pane, location, &label, result, events);
            }
        }
    }

    /// Applies what an action, or the rest of one, produced for `pane`.
    fn outcome(
        &mut self,
        pane: PaneId,
        location: Location,
        label: &str,
        result: Result<ActionOutcome, Error>,
        events: &mut Vec<Event>,
    ) {
        match result {
            Ok(ActionOutcome::Done) => {}
            Ok(ActionOutcome::Started(id)) => {
                self.pane_jobs.insert(id, pane);
            }
            Ok(ActionOutcome::Later(later)) => {
                self.later.push((pane, later));
            }
            Ok(ActionOutcome::Undoable(command)) => {
                self.undo.push(command);
                self.redo.clear();
            }
            Ok(ActionOutcome::Select(selector)) => {
                let ids: Vec<EntryId> = self.model.active_tab(pane).map_or_default(|(_, tab)| {
                    tab.rows()
                        .filter(|entry| (selector.matches)(entry))
                        .map(|entry| entry.id)
                        .collect()
                });
                let change = if selector.replace {
                    SelectionChange::Set(ids)
                } else {
                    SelectionChange::Add(ids)
                };
                if let Some(ids) = self.model.select(pane, change) {
                    events.push(Event::SelectionChanged { pane, ids });
                }
            }
            Ok(ActionOutcome::View(view)) => {
                let kind = match &view {
                    ext::View::Compare(_) => model::TabKind::Compare,
                    ext::View::Console(_) => model::TabKind::Console,
                };
                // The explorer tab a console follows: the pane's active
                // tab the action ran in.
                let explorer = self
                    .model
                    .active_tab(pane)
                    .filter(|(_, tab)| tab.kind == model::TabKind::Explorer)
                    .map(|(id, tab)| (id, tab.location.clone()));
                if let Some(tab) = self.model.open_view(pane, kind, location, view.title()) {
                    if let (ext::View::Console(console), Some((explorer, location_seen))) =
                        (&view, explorer)
                    {
                        self.console_links.insert(
                            tab,
                            ConsoleLink {
                                pane,
                                explorer,
                                console: Arc::clone(console),
                                cwd_seen: console.cwd(),
                                location_seen,
                                line: self.console_line.take(),
                            },
                        );
                    }
                    events.push(Event::ViewOpened { pane, tab, view });
                }
            }
            Ok(ActionOutcome::Hashes(hashes)) => events.push(Event::HashesOpened {
                pane,
                hashes,
                format: ops::hash::remembered(&*self.platform),
            }),
            Ok(ActionOutcome::Navigate(to)) => {
                self.dispatch(Command::Navigate { pane, to }, events);
            }
            Err(error) => events.push(Event::Notify {
                level: Level::Error,
                text: format!("{label}: {error}"),
            }),
        }
        self.drain_pins();
        self.attribute(pane, events);
    }

    /// Drains the job events published so far and books every job that
    /// just started to `pane`, so the pane is refreshed when it finishes.
    fn attribute(&mut self, pane: PaneId, events: &mut Vec<Event>) {
        while let Ok(event) = self.job_events.try_recv() {
            if let JobEvent::Started { id, .. } = &event
                && !self.internal.contains(id)
            {
                self.pane_jobs.entry(*id).or_insert(pane);
            }
            self.on_job_event(event, events);
        }
    }

    fn list_key(&self, pane: PaneId) -> Option<ListKey> {
        let (tab, state) = self.model.active_tab(pane)?;
        // The preview, a console and a comparison list nothing of their own.
        if !state.kind.lists() {
            return None;
        }
        Some(ListKey {
            tab,
            location: state.location.clone(),
            opts: ListOptions {
                hidden: state.show_hidden,
                system: self.model.settings.show_system,
                depth: if state.flatten {
                    Depth::Unlimited
                } else {
                    Depth::One
                },
            },
            search: state.is_searching().then(|| state.filter.clone()),
        })
    }

    /// Brings every pane's listing in line with what its active tab shows.
    fn sync_panes(&mut self, events: &mut Vec<Event>) {
        let panes = self.model.all_panes();
        let gone: Vec<PaneId> = self
            .live
            .keys()
            .filter(|p| !panes.contains(p))
            .copied()
            .collect();
        for pane in gone {
            if let Some(live) = self.live.remove(&pane) {
                live.cancel.cancel();
            }
        }
        for pane in panes {
            // A pane that moved to where nothing can be searched beneath
            // lists the folder again; the toggle goes off with it.
            let unsearchable = self.model.active_tab(pane).is_some_and(|(_, tab)| {
                tab.subtree && self.subtree_search(&tab.location) != Availability::Enabled
            });
            if unsearchable {
                self.model.set_subtree(pane, false);
            }
            let key = self.list_key(pane);
            let current = self.live.get(&pane).map(|l| &l.key);
            match key {
                Some(key) if current != Some(&key) => {
                    let before = self.selection_len(pane);
                    let stale = self
                        .model
                        .active_tab(pane)
                        .is_some_and(|(_, tab)| !tab.listing.is_empty());
                    self.relist(pane, key, stale);
                    if before != self.selection_len(pane) {
                        Self::selection_cleared(pane, before, events);
                    }
                }
                Some(_) => {}
                None => {
                    if let Some(live) = self.live.remove(&pane) {
                        live.cancel.cancel();
                    }
                }
            }
        }
    }

    /// Lists the pane's location again, swapping the result in when it is
    /// complete.
    fn refresh(&mut self, pane: PaneId) {
        if let Some(key) = self.list_key(pane) {
            // A refresh re-reads everything, what the decorators cached too.
            if let Some(live) = self.live.get(&pane) {
                for decorator in &live.decorators {
                    decorator.invalidate(&live.key.location);
                }
            }
            let stale = self
                .model
                .active_tab(pane)
                .is_some_and(|(_, tab)| !tab.listing.is_empty());
            self.relist(pane, key, stale);
        }
    }

    /// Cancels the pane's listing and watch and starts new ones. With
    /// `reconcile` the batches are collected and replace the listing in one
    /// step when done; otherwise the listing is cleared and streams in.
    #[allow(clippy::too_many_lines)]
    fn relist(&mut self, pane: PaneId, key: ListKey, reconcile: bool) {
        let generation = self.live.get(&pane).map_or(0, |l| l.generation) + 1;
        if let Some(old) = self.live.remove(&pane) {
            old.cancel.cancel();
            for child in old.children.values() {
                child.cancel.cancel();
            }
        }
        // Search results never reconcile: the walk streams in afresh.
        let reconcile = reconcile && key.search.is_none();
        if !reconcile {
            self.model.clear_listing(pane);
        }
        let provider = match self.mounts.resolve(&key.location) {
            Ok(provider) => provider,
            Err(error) => {
                self.pending.push(Event::Notify {
                    level: Level::Error,
                    text: format!("{}: {error}", key.location),
                });
                return;
            }
        };
        if let Some(text) = key.search.clone() {
            self.search(pane, key, generation, &text);
            return;
        }
        let tx = self.mailbox.clone();
        let location = key.location.clone();
        // A folder is listed with its hidden entries whatever the tab
        // shows: the view leaves them out, and Select all can say how many
        // it did not select (SPEC.md *Select all*). A flattened listing is
        // not, so that hidden folders are not walked.
        let opts = ListOptions {
            hidden: key.opts.hidden || key.opts.depth == Depth::One,
            ..key.opts
        };
        let poke = self.poke.clone();
        let handle = self
            .jobs
            .spawn_named(format!("List {location}"), move |job| async move {
                // Watch first so nothing that happens during the listing is
                // missed; the watch runs until the pane moves on.
                match provider.watch(&location) {
                    Ok(mut changes) => {
                        let tx = tx.clone();
                        let cancel = job.cancel.clone();
                        runtime::handle().spawn(async move {
                            loop {
                                tokio::select! {
                                    () = cancel.cancelled() => break,
                                    next = changes.next() => match next {
                                        Some(change) => {
                                            let msg = Msg::Change { pane, generation, change };
                                            if !tx.send(msg) {
                                                break;
                                            }
                                        }
                                        None => break,
                                    },
                                }
                            }
                        });
                    }
                    Err(error) => {
                        debug!(%location, %error, "no watch; polling");
                        let tx = tx.clone();
                        poll_location(
                            Arc::clone(&provider),
                            location.clone(),
                            job.cancel.clone(),
                            &poke,
                            move |change| {
                                tx.send(Msg::Change {
                                    pane,
                                    generation,
                                    change,
                                })
                            },
                        );
                    }
                }
                let result = async {
                    let mut stream = provider.list(&location, opts, job.cancel.clone());
                    let mut count = 0u64;
                    while let Some(batch) = stream.next().await {
                        let batch = batch?;
                        count += batch.len() as u64;
                        job.progress.report(Progress::Items {
                            done: count,
                            total: None,
                        });
                        let msg = Msg::Batch {
                            pane,
                            generation,
                            batch,
                        };
                        if !tx.send(msg) {
                            break;
                        }
                    }
                    Ok(())
                }
                .await;
                tx.send(Msg::ListDone {
                    pane,
                    generation,
                    result: result.as_ref().copied().map_err(Error::replicate),
                });
                result
            });
        self.internal.insert(handle.id);
        let decorators: Vec<Arc<dyn Decorator>> = self
            .registry
            .decorators()
            .iter()
            .filter(|d| d.applies_to(&key.location))
            .cloned()
            .collect();
        for decorator in &decorators {
            for watched in decorator.invalidates_on(&key.location) {
                self.watch_invalidation(pane, generation, handle.cancel.clone(), &watched);
            }
        }
        self.live.insert(
            pane,
            Live {
                key,
                generation,
                cancel: handle.cancel,
                complete: false,
                buffer: reconcile.then(Vec::new),
                decorators,
                redecorate_scheduled: false,
                children: BTreeMap::new(),
            },
        );
    }

    /// Whether `location` can be searched beneath (SPEC.md, *Footer*):
    /// through a search index whose scope is under it, as the local file
    /// system is through the walk, or by its provider's own listing of a
    /// whole subtree (`SEARCH_SUBTREE`). Where it cannot, the reason is
    /// the tooltip of the filter bar's disabled toggle.
    #[must_use]
    pub fn subtree_search(&self, location: &Location) -> Availability {
        if self.subtree_index(location).is_some() {
            return Availability::Enabled;
        }
        match self.mounts.resolve(location) {
            Ok(provider)
                if provider
                    .capabilities(location)
                    .contains(Capabilities::SEARCH_SUBTREE) =>
            {
                Availability::Enabled
            }
            Ok(provider) => Availability::Disabled(
                provider
                    .unavailable_reason(location, Capabilities::SEARCH_SUBTREE)
                    .unwrap_or_else(|| ext::t!("reason-not-available-here")),
            ),
            Err(_) => Availability::Disabled(ext::t!("reason-not-available-here")),
        }
    }

    /// The available search index whose scope is under `location`.
    fn subtree_index(&self, location: &Location) -> Option<Arc<dyn SearchIndex>> {
        self.registry
            .search_indexes()
            .iter()
            .find(|index| {
                let scope = index.scope();
                matches!(scope, SearchScope::Under(_))
                    && scope.covers(location)
                    && index.availability() == Availability::Enabled
            })
            .cloned()
    }

    /// Starts the subtree search of `key` as a job the operations area
    /// shows: through the search index whose scope is `Under` the location
    /// (the `ext-search-walk` plugin; QUESTIONS.md 118), or, on a provider
    /// with `SEARCH_SUBTREE`, by listing everything beneath the location
    /// and matching the names here. The matches stream into the pane's
    /// listing like a listing's batches, and a change of text or location
    /// cancels it. The pane's hidden toggle and the `[search] ignore` list
    /// narrow both.
    fn search(&mut self, pane: PaneId, key: ListKey, generation: u64, text: &str) {
        let location = key.location.clone();
        let Some(index) = self.subtree_index(&location) else {
            self.search_listing(pane, key, generation, text);
            return;
        };
        let tx = self.mailbox.clone();
        let mut filters = vec![Filter::SkipNames(self.model.settings.search.ignore.clone())];
        if !key.opts.hidden {
            filters.push(Filter::NoHidden);
        }
        let query = Query {
            text: text.to_owned(),
            root: Some(location.clone()),
            kind: QueryKind::Name,
            filters,
        };
        let handle = self.jobs.spawn_named(
            format!("Search {text} in {location}"),
            move |job| async move {
                let mut stream = index.query(&query, job.cancel.clone());
                let mut count = 0u64;
                let result = async {
                    while let Some(batch) = stream.next().await {
                        let batch: Vec<Entry> = batch?.into_iter().map(|hit| hit.entry).collect();
                        count += batch.len() as u64;
                        job.progress.report(Progress::Items {
                            done: count,
                            total: None,
                        });
                        if !tx.send(Msg::Batch {
                            pane,
                            generation,
                            batch,
                        }) {
                            break;
                        }
                    }
                    Ok(())
                }
                .await;
                tx.send(Msg::ListDone {
                    pane,
                    generation,
                    result: result.as_ref().copied().map_err(Error::replicate),
                });
                result
            },
        );
        self.live.insert(
            pane,
            Live {
                key,
                generation,
                cancel: handle.cancel,
                complete: false,
                buffer: None,
                decorators: Vec::new(),
                redecorate_scheduled: false,
                children: BTreeMap::new(),
            },
        );
    }

    /// The subtree search on a provider that lists a whole subtree itself
    /// (`SEARCH_SUBTREE`): one listing of everything beneath the location,
    /// the names matched here as the walk matches them — the text anywhere
    /// in the name, whatever the case — and what the ignore list names
    /// left out with everything under it. Where the location has neither
    /// an index nor the capability the search finds nothing and says
    /// nothing: the toggle that starts it is disabled there.
    fn search_listing(&mut self, pane: PaneId, key: ListKey, generation: u64, text: &str) {
        let location = key.location.clone();
        let provider = self.mounts.resolve(&location).ok().filter(|provider| {
            provider
                .capabilities(&location)
                .contains(Capabilities::SEARCH_SUBTREE)
        });
        let Some(provider) = provider else {
            debug!(%location, "nothing searches beneath this location");
            self.model.listing_done(pane);
            self.live.insert(
                pane,
                Live {
                    key,
                    generation,
                    cancel: CancelToken::new(),
                    complete: true,
                    buffer: None,
                    decorators: Vec::new(),
                    redecorate_scheduled: false,
                    children: BTreeMap::new(),
                },
            );
            return;
        };
        let tx = self.mailbox.clone();
        let needle = text.to_lowercase();
        let ignore = self.model.settings.search.ignore.clone();
        let opts = ListOptions {
            depth: Depth::Unlimited,
            ..key.opts
        };
        let root = location.path.clone();
        let handle = self.jobs.spawn_named(
            format!("Search {text} in {location}"),
            move |job| async move {
                let mut stream = provider.list(&location, opts, job.cancel.clone());
                let mut count = 0u64;
                let result = async {
                    while let Some(batch) = stream.next().await {
                        let batch: Vec<Entry> = batch?
                            .into_iter()
                            .filter(|entry| {
                                entry.name.to_lowercase().contains(&needle)
                                    && !below_ignored(&root, &entry.path, &ignore)
                            })
                            .collect();
                        if batch.is_empty() {
                            continue;
                        }
                        count += batch.len() as u64;
                        job.progress.report(Progress::Items {
                            done: count,
                            total: None,
                        });
                        if !tx.send(Msg::Batch {
                            pane,
                            generation,
                            batch,
                        }) {
                            break;
                        }
                    }
                    Ok(())
                }
                .await;
                tx.send(Msg::ListDone {
                    pane,
                    generation,
                    result: result.as_ref().copied().map_err(Error::replicate),
                });
                result
            },
        );
        self.live.insert(
            pane,
            Live {
                key,
                generation,
                cancel: handle.cancel,
                complete: false,
                buffer: None,
                decorators: Vec::new(),
                redecorate_scheduled: false,
                children: BTreeMap::new(),
            },
        );
    }

    /// Keeps the model's list of available search indexes current: a
    /// connector detects its service at start and when it appears or
    /// disappears, and the title-row search box follows.
    fn sync_search_indexes(&mut self) {
        let available: Vec<ExtId> = available_indexes(&self.registry)
            .iter()
            .map(|index| index.id())
            .collect();
        if available != self.model.search_indexes {
            debug!(?available, "search indexes available");
            self.model.search_indexes = available;
        }
    }

    /// The title-row search box's text changed: cancel the running index
    /// search, start over for `text` after [`INDEX_SEARCH_AFTER`], or end
    /// the search when the text is empty.
    fn index_search(&mut self, text: &str, events: &mut Vec<Event>) {
        if let Some(job) = self.index_search.take() {
            for cancel in job.cancels {
                cancel.cancel();
            }
        }
        self.index_generation += 1;
        let generation = self.index_generation;
        let text = text.trim().to_owned();
        let same = self.model.index_search.text == text;
        self.model.index_search.start(text.clone());
        if text.is_empty() {
            if !same {
                events.push(Event::IndexSearched {
                    batch: Vec::new(),
                    done: true,
                });
            }
            return;
        }
        if available_indexes(&self.registry).is_empty() {
            self.model.index_search.done = true;
            return;
        }
        self.index_search = Some(IndexJob {
            generation,
            cancels: Vec::new(),
            pending: 0,
        });
        let tx = self.mailbox.clone();
        runtime::handle().spawn(async move {
            tokio::time::sleep(INDEX_SEARCH_AFTER).await;
            tx.send(Msg::IndexSearchStart { generation });
        });
    }

    /// The pause after the last keystroke passed: one job per available
    /// index, each streaming its hits into the model.
    fn index_search_start(&mut self, generation: u64) {
        let Some(job) = self.index_search.as_mut() else {
            return;
        };
        if job.generation != generation {
            return;
        }
        let text = self.model.index_search.text.clone();
        let indexes = available_indexes(&self.registry);
        if indexes.is_empty() {
            self.index_search = None;
            self.model.index_search.done = true;
            return;
        }
        job.pending = indexes.len();
        let query = Query {
            text: text.clone(),
            root: None,
            kind: QueryKind::Name,
            filters: Vec::new(),
        };
        let mut cancels = Vec::new();
        let mut ids = Vec::new();
        for index in indexes {
            let tx = self.mailbox.clone();
            let query = query.clone();
            let id = index.id();
            let handle = self.jobs.spawn_named(
                format!("Search {text} in {}", id.0),
                move |job| async move {
                    let mut stream = index.query(&query, job.cancel.clone());
                    let result = async {
                        while let Some(batch) = stream.next().await {
                            let batch: Vec<Entry> =
                                batch?.into_iter().map(|hit| hit.entry).collect();
                            if !tx.send(Msg::IndexHits { generation, batch }) {
                                break;
                            }
                        }
                        Ok(())
                    }
                    .await;
                    tx.send(Msg::IndexDone {
                        generation,
                        index: id,
                        result: result.as_ref().copied().map_err(Error::replicate),
                    });
                    result
                },
            );
            ids.push(handle.id);
            cancels.push(handle.cancel);
        }
        self.internal.extend(ids);
        if let Some(job) = self.index_search.as_mut() {
            job.cancels = cancels;
        }
    }

    /// Watches `location` for a pane's decorators and asks for a
    /// re-decoration, debounced, when it changes; stops with `cancel`.
    fn watch_invalidation(
        &self,
        pane: PaneId,
        generation: u64,
        cancel: CancelToken,
        location: &Location,
    ) {
        let Ok(provider) = self.mounts.resolve(location) else {
            return;
        };
        let mut changes = match provider.watch(location) {
            Ok(changes) => changes,
            Err(error) => {
                debug!(%location, %error, "no invalidation watch");
                return;
            }
        };
        let tx = self.mailbox.clone();
        runtime::handle().spawn(async move {
            loop {
                tokio::select! {
                    () = cancel.cancelled() => break,
                    next = changes.next() => {
                        if next.is_none() {
                            break;
                        }
                        // Collect what follows within the window.
                        let window = tokio::time::sleep(REDECORATE_AFTER);
                        tokio::pin!(window);
                        loop {
                            tokio::select! {
                                () = cancel.cancelled() => return,
                                () = &mut window => break,
                                more = changes.next() => {
                                    if more.is_none() {
                                        break;
                                    }
                                }
                            }
                        }
                        if !tx.send(Msg::Invalidate { pane, generation }) {
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Runs the pane's decorators over `entries`, records what they answer at
    /// once and routes what they publish later through the mailbox.
    fn decorate(&mut self, pane: PaneId, entries: &[Entry], events: &mut Vec<Event>) {
        let Some(live) = self.live.get(&pane) else {
            return;
        };
        if live.decorators.is_empty() || entries.is_empty() {
            return;
        }
        let generation = live.generation;
        let mut items = Vec::new();
        for decorator in &live.decorators {
            let tx = self.mailbox.clone();
            let ctx = DecorateCtx::new(
                live.key.location.clone(),
                live.cancel.clone(),
                self.decor_jobs.clone(),
                move |late| {
                    tx.send(Msg::Decorated {
                        pane,
                        generation,
                        items: late,
                    });
                },
            );
            items.extend(decorator.decorate(entries, &ctx));
        }
        self.decorated(pane, items, events);
    }

    /// Records decorations in the model and tells the UI.
    fn decorated(
        &mut self,
        pane: PaneId,
        items: Vec<(EntryId, Decoration)>,
        events: &mut Vec<Event>,
    ) {
        if !items.is_empty() && self.model.decorate(pane, items.clone()) {
            events.push(Event::Decorated { pane, items });
        }
    }

    /// Asks for the pane to be decorated again once changes settle.
    fn schedule_redecorate(&mut self, pane: PaneId) {
        let Some(live) = self.live.get_mut(&pane) else {
            return;
        };
        if live.decorators.is_empty() || live.redecorate_scheduled {
            return;
        }
        live.redecorate_scheduled = true;
        let generation = live.generation;
        let tx = self.mailbox.clone();
        runtime::handle().spawn(async move {
            tokio::time::sleep(REDECORATE_AFTER).await;
            tx.send(Msg::Invalidate { pane, generation });
        });
    }

    /// Drops what the pane's decorators cached and decorates the whole
    /// listing again.
    fn redecorate(&mut self, pane: PaneId, events: &mut Vec<Event>) {
        let Some(live) = self.live.get_mut(&pane) else {
            return;
        };
        live.redecorate_scheduled = false;
        for decorator in &live.decorators {
            decorator.invalidate(&live.key.location);
        }
        let entries: Vec<Entry> = self
            .model
            .active_tab(pane)
            .map_or_default(|(_, tab)| tab.listing.entries().to_vec());
        self.decorate(pane, &entries, events);
    }

    // One arm per message: the enum is the table.
    #[allow(clippy::too_many_lines)]
    fn on_msg(&mut self, msg: Msg, events: &mut Vec<Event>) {
        match msg {
            Msg::Batch {
                pane,
                generation,
                batch,
            } => {
                let Some(live) = self.live.get_mut(&pane) else {
                    return;
                };
                if live.generation != generation {
                    return;
                }
                if let Some(buffer) = &mut live.buffer {
                    buffer.extend(batch);
                    return;
                }
                self.model.listing_batch(pane, batch.clone());
                self.decorate(pane, &batch, events);
                events.push(Event::Listed {
                    pane,
                    batch,
                    done: false,
                });
                self.resolve_reveal(pane, false, events);
            }
            Msg::ListDone {
                pane,
                generation,
                result,
            } => self.list_done(pane, generation, &result, events),
            Msg::RetryLost { pane, generation } => self.retry_lost(pane, generation),
            Msg::Change {
                pane,
                generation,
                change,
            } => self.change(pane, generation, change, events),
            Msg::IndexSearchStart { generation } => self.index_search_start(generation),
            Msg::IndexHits { generation, batch } => {
                if self.index_search.as_ref().map(|j| j.generation) != Some(generation) {
                    return;
                }
                let batch = self.model.index_search.extend(batch);
                if !batch.is_empty() {
                    events.push(Event::IndexSearched { batch, done: false });
                }
            }
            Msg::IndexDone {
                generation,
                index,
                result,
            } => {
                let Some(job) = self.index_search.as_mut() else {
                    return;
                };
                if job.generation != generation {
                    return;
                }
                if let Err(error) = result
                    && !matches!(error, Error::Cancelled)
                {
                    warn!(index = index.0, %error, "search index failed");
                    self.model
                        .index_search
                        .errors
                        .push(format!("{}: {error}", index.0));
                }
                job.pending = job.pending.saturating_sub(1);
                if job.pending == 0 {
                    self.index_search = None;
                    self.model.index_search.done = true;
                    events.push(Event::IndexSearched {
                        batch: Vec::new(),
                        done: true,
                    });
                }
            }
            Msg::Settings(Ok(settings)) => {
                if self.recent_saves.contains(&settings) {
                    // Our own write coming back through the watcher; an
                    // earlier one may have been written over by a later
                    // change already. Or the file as it was at the start,
                    // read again before our first change reached it.
                    return;
                }
                if settings != self.model.settings {
                    debug!("settings file changed");
                    if settings.language != self.model.settings.language {
                        ext::i18n::set_language(settings.language.as_deref());
                    }
                    self.pins.sync(&settings.pinned);
                    self.pins.sync_open(&settings);
                    let connections_changed =
                        settings.connections != self.model.settings.connections;
                    self.model.apply_settings(settings);
                    if connections_changed {
                        let providers: Vec<ProviderId> = self
                            .registry
                            .connectors()
                            .iter()
                            .map(|c| c.provider())
                            .collect();
                        for provider in providers {
                            self.push_connections(provider);
                            self.refresh_rows_of(provider);
                        }
                    }
                    self.sync_panes(events);
                }
            }
            Msg::Settings(Err(error)) => {
                warn!(%error, "settings file not read");
                events.push(Event::Notify {
                    level: Level::Warning,
                    text: format!("Settings file not read: {error}"),
                });
            }
            Msg::Preview {
                pane,
                generation,
                result,
                previewer,
                prefix,
            } => {
                if self
                    .preview
                    .get(&pane)
                    .is_none_or(|job| job.generation != generation)
                {
                    return;
                }
                let opened = result.is_ok();
                if let Ok(doc) = &result {
                    // What the Extensions dialogue calls needing attention.
                    match doc.content() {
                        Some(ext::Content::MissingLibrary(missing)) => {
                            self.missing_libraries
                                .insert(missing.setting.0.0.to_owned(), missing.name.clone());
                        }
                        _ => {
                            self.missing_libraries.remove(previewer.0);
                        }
                    }
                }
                events.push(match result {
                    Ok(doc) => Event::PreviewReady {
                        pane,
                        doc,
                        previewer,
                    },
                    Err(error) => Event::PreviewFailed { pane, error },
                });
                if let Some((served, total)) = prefix.filter(|_| opened) {
                    events.push(Event::PreviewPrefix {
                        pane,
                        served,
                        total,
                    });
                }
            }
            Msg::PreviewText { result } => {
                let outcome = result.and_then(|text| self.platform.set_text(&text).map(|()| text));
                match outcome {
                    Ok(text) => events.push(Event::Notify {
                        level: Level::Info,
                        text: ext::t!("preview-copied", count = text.chars().count()),
                    }),
                    Err(Error::Unsupported(_)) => events.push(Event::Notify {
                        level: Level::Error,
                        text: ext::t!(
                            "preview-copy-too-large",
                            limit = format!("{} MiB", COPY_LIMIT / (1024 * 1024))
                        ),
                    }),
                    Err(error) => events.push(Event::Notify {
                        level: Level::Error,
                        text: format!("Copy failed: {error}"),
                    }),
                }
            }
            Msg::PreviewNone {
                pane,
                generation,
                permissions,
            } => {
                if let Some(job) = self
                    .preview
                    .get(&pane)
                    .filter(|job| job.generation == generation)
                {
                    let plugin = self.disabled_plugin_for(&job.full.name);
                    events.push(Event::PreviewNone {
                        pane,
                        plugin,
                        permissions,
                    });
                }
            }
            Msg::PreviewCapped {
                pane,
                generation,
                size,
                cap,
            } => {
                if let Some(job) = self
                    .preview
                    .get(&pane)
                    .filter(|job| job.generation == generation)
                {
                    events.push(self.capped_event(pane, &job.full, size, cap));
                }
            }
            Msg::PreviewProgress {
                pane,
                generation,
                read,
                total,
            } => {
                if self
                    .preview
                    .get(&pane)
                    .is_some_and(|job| job.generation == generation)
                {
                    events.push(Event::PreviewProgress { pane, read, total });
                }
            }
            Msg::PluginInstalled(Ok(plugin)) => {
                // The row of what is on disk now: in place of the plugin
                // it replaced, or a new one.
                let path = plugin.path.clone();
                match self.plugins.iter_mut().find(|row| row.path == path) {
                    Some(row) => *row = *plugin,
                    None => self.plugins.push(*plugin),
                }
                events.push(Event::PluginInstalled { path });
            }
            Msg::PluginRemoved {
                path,
                result: Ok(()),
            } => {
                // A plugin unpacked in this session was never loaded: its
                // row goes with its folder. One the host loaded stays,
                // marked, until the next start.
                self.plugins
                    .retain(|row| row.path != path || row.status != PluginStatus::Installed);
                if let Some(row) = self.plugins.iter_mut().find(|row| row.path == path) {
                    row.removed = true;
                }
            }
            Msg::PluginInstalled(Err(text))
            | Msg::PluginRemoved {
                result: Err(text), ..
            } => events.push(Event::Notify {
                level: Level::Error,
                text,
            }),
            Msg::Decorated {
                pane,
                generation,
                items,
            } => {
                if self
                    .live
                    .get(&pane)
                    .is_some_and(|l| l.generation == generation)
                {
                    self.decorated(pane, items, events);
                }
            }
            Msg::Thumbnail { id, size, frame } => {
                self.thumbnails.remove(&(id, size));
                events.push(Event::Thumbnail { id, size, frame });
            }
            Msg::TreeBatch {
                pane,
                generation,
                folder,
                batch,
            } => {
                let Some(live) = self.live.get_mut(&pane) else {
                    return;
                };
                if live.generation != generation {
                    return;
                }
                if let Some(child) = live.children.get_mut(&folder)
                    && let Some(buffer) = &mut child.buffer
                {
                    buffer.extend(batch);
                    return;
                }
                self.model.tree_batch(pane, folder, batch.clone());
                self.decorate(pane, &batch, events);
                events.push(Event::Listed {
                    pane,
                    batch,
                    done: false,
                });
            }
            Msg::TreeDone {
                pane,
                generation,
                folder,
                result,
            } => {
                let Some(live) = self.live.get_mut(&pane) else {
                    return;
                };
                if live.generation != generation {
                    return;
                }
                let child = live.children.remove(&folder);
                match (child.and_then(|c| c.buffer), result) {
                    (Some(entries), Ok(())) => {
                        self.model.tree_replace(pane, folder, entries.clone());
                        self.decorate(pane, &entries, events);
                    }
                    (_, Err(error)) => {
                        events.push(Event::Notify {
                            level: Level::Warning,
                            text: format!("Listing folder: {error}"),
                        });
                        self.model.tree_done(pane, folder);
                    }
                    (None, Ok(())) => {
                        self.model.tree_done(pane, folder);
                    }
                }
                events.push(Event::Listed {
                    pane,
                    batch: Vec::new(),
                    done: true,
                });
            }
            Msg::Conflicts => {
                for conflict in self.conflicts.take_new() {
                    self.model.conflict_opened(conflict.clone());
                    events.push(Event::Conflict(conflict));
                }
            }
            Msg::Prompts => {
                for question in self.prompts.take_new() {
                    self.model.question_opened(question.clone());
                    events.push(Event::Question(question));
                }
            }
            Msg::PropertiesCounted {
                pane,
                path,
                contents,
            } => {
                // A count the dialogue no longer waits for is dropped.
                if let Some(count) = &mut self.properties_count
                    && count.pane == pane
                    && count.path == path
                {
                    count.result = Some(contents);
                    events.push(Event::PropertiesCounted {
                        pane,
                        path,
                        contents,
                    });
                }
            }
            Msg::Properties(asked, result) => {
                // The read is answered: nothing more comes of it.
                self.properties_reading = self.properties_reading.saturating_sub(1);
                // Only the request the dialogue waits for opens it: one
                // called off in the meantime, or overtaken by a later one,
                // is dropped with whatever it read.
                if self.properties_waiting != Some(asked) {
                    return;
                }
                self.properties_waiting = None;
                match *result {
                    Ok(mut info) => {
                        // The count may have ended while the entry was read.
                        if info.contents == Contents::Counting
                            && let Some(count) = &self.properties_count
                            && count.pane == info.pane
                            && Some(&count.path) == info.path.as_ref()
                            && let Some(contents) = count.result
                        {
                            info.contents = Contents::Counted(contents);
                        }
                        // A dialogue closed in the meantime takes no answer.
                        if self.properties_shown.as_ref().is_some_and(|(pane, path)| {
                            *pane == info.pane && Some(path) == info.path.as_ref()
                        }) || info.path.is_none()
                        {
                            events.push(Event::Properties(Box::new(info)));
                        }
                    }
                    Err(error) => events.push(Event::Notify {
                        level: Level::Error,
                        text: format!("Properties: {error}"),
                    }),
                }
            }
            Msg::PermissionsApplied {
                pane,
                label,
                request,
                result,
            } => self.permissions_applied(pane, label, &request, result, events),
            Msg::Schedule => self.sync_waiting(),
            Msg::Volumes(Ok(volumes)) => {
                if volumes != self.model.volumes {
                    debug!(count = volumes.len(), "volumes");
                    // A volume that is gone — pulled out, or unmounted by
                    // another program — sends the panes on it Home.
                    let removed: Vec<Volume> = self
                        .model
                        .volumes
                        .iter()
                        .filter(|old| !volumes.iter().any(|v| v.mount_point == old.mount_point))
                        .cloned()
                        .collect();
                    for volume in &removed {
                        self.evacuate(
                            &volume.name,
                            &|location| on_volume(&volume.mount_point, location),
                            events,
                        );
                    }
                    self.model.volumes = volumes;
                    // The trash's disk may be among them now.
                    let size = self.model.trash.size;
                    let share = self.trash_share(&self.model.trash.items, size);
                    self.model.trash.set_stats(size, share);
                    self.rick.changed(Virtual::Machine);
                    self.rick.changed(Virtual::Network);
                }
            }
            Msg::Volumes(Err(error)) => debug!(%error, "volumes not enumerated"),
            Msg::FormatTool(installed) => self.model.format_tool = installed,
            Msg::Script { pane, entry, run } => self.script(pane, entry, run, events),
            Msg::Opened {
                pane,
                tab,
                entry,
                location,
            } => {
                // The pane may have moved on since; then the answer is moot.
                if self.model.active_tab(pane).map(|(id, _)| id) != Some(tab) {
                    return;
                }
                match location {
                    Some(to) => {
                        self.dispatch(Command::Navigate { pane, to }, events);
                    }
                    None => self.open_default(pane, entry, events),
                }
            }
            Msg::Trash(Ok((items, size))) => {
                let changed = self.model.trash.items != items || !self.model.trash.listed;
                let share = self.trash_share(&items, size);
                self.model.trash.set(items, true);
                self.model.trash.set_stats(size, share);
                if changed {
                    self.rick.changed(Virtual::Trash);
                }
            }
            Msg::Trash(Err(error)) => {
                warn!(%error, "trash not listed");
                self.model.trash.error = Some(error.to_string());
            }
            Msg::Cloud(entries) => {
                self.model.cloud = entries;
            }
            Msg::Network(entries) => {
                self.model.network = entries;
            }
            Msg::ConnectionSaved(entry) => self.connection_saved(entry),
            Msg::ConnectionChanged(provider) => self.refresh_rows_of(provider),
            Msg::IntegrationChecked {
                generation,
                startup,
                listed,
                found,
            } => self.integration_checked(generation, startup, listed, &found, events),
            Msg::IntegrationProgress {
                done,
                total,
                current,
            } => self.integration_progress(done, total, current),
            Msg::IntegrationApplied { results, relaunch } => {
                self.integration_applied(results, relaunch, events);
            }
            Msg::Invalidate { pane, generation } => {
                if self
                    .live
                    .get(&pane)
                    .is_some_and(|l| l.generation == generation)
                {
                    self.redecorate(pane, events);
                }
            }
        }
    }

    /// Points the preview at the focused pane's single selected file,
    /// opening it through the best-scoring previewer as a job, and cancels
    /// the previous one when the target changes.
    /// Renders a thumbnail of `id` at `size` pixels through the previewer
    /// that claims it, as an internal job; the answer is
    /// [`Event::Thumbnail`], `None` when nothing can draw it.
    fn thumbnail(&mut self, pane: PaneId, id: EntryId, size: u32) {
        if !self.thumbnails.insert((id, size)) {
            return;
        }
        let entry = self
            .model
            .active_tab(pane)
            .and_then(|(_, tab)| tab.listing.get(id))
            .cloned();
        let loc = entry
            .as_ref()
            .map(|e| Location::new(id.provider, e.path.clone()));
        let provider = loc.as_ref().and_then(|l| self.mounts.resolve(l).ok());
        let (Some(entry), Some(loc), Some(provider)) = (entry, loc, provider) else {
            self.thumbnails.remove(&(id, size));
            self.pending.push(Event::Thumbnail {
                id,
                size,
                frame: None,
            });
            return;
        };
        // A remote-latency provider thumbnails nothing by default (SPEC.md
        // "Network file systems").
        if provider
            .capabilities(&loc)
            .contains(ext::Capabilities::REMOTE)
        {
            self.thumbnails.remove(&(id, size));
            self.pending.push(Event::Thumbnail {
                id,
                size,
                frame: None,
            });
            return;
        }
        let previewers: Vec<Arc<dyn Previewer>> = self.registry.previewers().to_vec();
        let dark = self.model.settings.theme != model::Theme::Light;
        let tx = self.mailbox.clone();
        let handle =
            self.jobs
                .spawn_named(format!("Thumbnail {}", entry.name), move |job| async move {
                    let frame = async {
                        let sniff =
                            ext::read_bytes(&*provider, &loc, Some(4096), &job.cancel).await?;
                        // What only the fallback takes has no picture of
                        // its own: the file-type icon stays.
                        let best = previewers
                            .iter()
                            .map(|p| (p.score(&entry, &sniff), p))
                            .filter(|(score, _)| *score > ext::FALLBACK_SCORE)
                            .max_by_key(|(score, _)| *score)
                            .map(|(_, p)| Arc::clone(p))
                            .ok_or(Error::Unsupported("no previewer"))?;
                        let mut doc = best.open(&loc, provider, job.cancel.clone()).await?;
                        let req = ext::RenderRequest {
                            width: size,
                            height: size,
                            scale: 1.0,
                            dark,
                        };
                        ext::blocking(move || doc.render(0, req)).await
                    }
                    .await;
                    let frame = match frame {
                        Ok(frame @ ext::Frame::Rgba { .. }) => Some(frame),
                        Ok(_) => None,
                        Err(error) => {
                            debug!(%error, "no thumbnail");
                            None
                        }
                    };
                    tx.send(Msg::Thumbnail { id, size, frame });
                    Ok(())
                });
        self.internal.insert(handle.id);
    }

    fn sync_preview(&mut self) {
        // The Preview tab's file is the single selected one of the explorer
        // tab it follows, shown in that tab's pane.
        let mut wanted: BTreeMap<PaneId, Entry> = BTreeMap::new();
        if let Some(tab) = self.model.preview_follows()
            && let Some(pane) = self.model.pane_of(tab)
            && let Some(entry) = self.model.tab(tab).and_then(single_file)
        {
            wanted.insert(pane, entry);
        }
        let stale: Vec<PaneId> = self
            .preview
            .iter()
            .filter(|(pane, job)| wanted.get(pane).is_none_or(|e| e.id != job.entry))
            .map(|(pane, _)| *pane)
            .collect();
        for pane in stale {
            if let Some(job) = self.preview.remove(&pane) {
                job.cancel.cancel();
            }
            // The named previewer was for that one look at the file.
            self.preview_with.remove(&pane);
        }
        for (pane, entry) in wanted {
            if !self.preview.contains_key(&pane) {
                self.start_preview(pane, entry);
            }
        }
    }

    /// Reads the whole file the pane previews and puts its text on the
    /// clipboard: a job, answered by [`Msg::PreviewText`]. The bytes are
    /// inflated when the file is gzip-compressed (an SVGZ: the text shown is
    /// the inflated one), then decoded by their byte-order mark (UTF-8,
    /// UTF-16) and as UTF-8 otherwise, lossily; a file above [`COPY_LIMIT`]
    /// is refused with a notification.
    fn copy_preview_text(&mut self, pane: PaneId) {
        let Some(job) = self.preview.get(&pane) else {
            return;
        };
        let entry = job.full.clone();
        let loc = Location::new(entry.id.provider, entry.path.clone());
        let Ok(provider) = self.mounts.resolve(&loc) else {
            return;
        };
        let tx = self.mailbox.clone();
        let handle = self
            .jobs
            .spawn_named(format!("Copy {}", entry.name), move |job| async move {
                let result = async {
                    let bytes =
                        ext::read_bytes(&*provider, &loc, Some(COPY_LIMIT + 1), &job.cancel)
                            .await?;
                    if bytes.len() > COPY_LIMIT {
                        return Err(Error::Unsupported("the file is larger than the copy limit"));
                    }
                    copied_text(&bytes)
                }
                .await;
                tx.send(Msg::PreviewText { result });
                Ok(())
            });
        self.internal.insert(handle.id);
    }

    /// Opens the pane's preview again for the entry it shows: after *Load
    /// more* or *Preview anyway*.
    fn restart_preview(&mut self, pane: PaneId) {
        if let Some(job) = self.preview.remove(&pane) {
            job.cancel.cancel();
            self.start_preview(pane, job.full);
        }
    }

    /// The entry's provider cannot be reached: the preview failed before
    /// it began, and the pane keeps the entry so *Retry* can try again.
    fn preview_unreachable(&mut self, pane: PaneId, entry: Entry, generation: u64, error: Error) {
        self.pending.push(Event::PreviewFailed { pane, error });
        self.preview.insert(
            pane,
            PreviewJob {
                entry: entry.id,
                full: entry,
                generation,
                cancel: CancelToken::new(),
            },
        );
    }

    /// Opens `entry` for `pane` through the best-scoring previewer as a
    /// job, answered by [`Event::PreviewReady`] or [`Event::PreviewFailed`].
    /// On a remote provider a prefix previewer is served the first chunk
    /// (and one more per *Load more*), reported through
    /// [`Event::PreviewPrefix`], and a whole-file previewer waits behind
    /// the size cap ([`Event::PreviewCapped`]) until *Preview anyway*.
    fn start_preview(&mut self, pane: PaneId, entry: Entry) {
        self.preview_generation += 1;
        let generation = self.preview_generation;
        let loc = Location::new(entry.id.provider, entry.path.clone());
        let job_entry = entry.clone();
        let provider = match self.mounts.resolve(&loc) {
            Ok(provider) => provider,
            Err(error) => {
                self.preview_unreachable(pane, job_entry, generation, error);
                return;
            }
        };
        let remote = provider.capabilities(&loc).contains(Capabilities::REMOTE);
        let cap = provider
            .remote_preview_cap(&loc)
            .unwrap_or(REMOTE_PREVIEW_CAP);
        // The source's *Always preview* switch takes the cap off there,
        // on every remote provider: the provider's own answer, or the
        // switch in the source's connection.
        let always =
            remote && (provider.remote_preview_always(&loc) || self.always_previewed(&entry));
        let more = self
            .preview_more
            .get(&pane)
            .filter(|(id, _)| *id == entry.id)
            .map_or(0, |(_, n)| *n);
        let force = self.preview_force.get(&pane) == Some(&entry.id);
        // One named previewer for this file, whatever the scores.
        let with = self
            .preview_with
            .get(&pane)
            .filter(|(id, _)| *id == entry.id)
            .map(|(_, previewer)| *previewer);
        let id = entry.id;
        let name = format!("Preview {}", entry.name);
        let run = PreviewRun {
            pane,
            generation,
            previewers: self.registry.previewers().to_vec(),
            with,
            platform: Arc::clone(&self.platform),
            local: local_path(&entry),
            entry,
            loc,
            provider,
            remote,
            // Held behind the size limit unless something lifts it.
            capped: remote && !force && !always,
            cap,
            more,
            tx: self.mailbox.clone(),
        };
        let handle = self.jobs.spawn_named(name, move |job| run.open(job.cancel));
        self.internal.insert(handle.id);
        self.preview.insert(
            pane,
            PreviewJob {
                entry: id,
                full: job_entry,
                generation,
                cancel: handle.cancel,
            },
        );
    }

    fn list_done(
        &mut self,
        pane: PaneId,
        generation: u64,
        result: &Result<(), Error>,
        events: &mut Vec<Event>,
    ) {
        let Some(live) = self.live.get_mut(&pane) else {
            return;
        };
        if live.generation != generation {
            return;
        }
        live.complete = true;
        let buffered = live.buffer.take();
        let location = live.key.location.clone();
        // A remote location that cannot be reached keeps the pane where it
        // is, with the card and *Retry* (SPEC.md *Removed and unreachable
        // locations*). Any other failure is the tab's banner with *Retry*,
        // never a dialogue (`specs/ext-ssh.md`, "Connections"); a listing
        // that works clears both.
        let remote = self.mounts.resolve(&location).is_ok_and(|provider| {
            provider
                .capabilities(&location)
                .contains(Capabilities::REMOTE)
        });
        match result {
            Err(error) if remote && error.is_unreachable() => {
                self.lost(pane, generation, &location, error);
            }
            _ => {
                self.model.connection_lost(pane, None);
                self.model.listing_failed(
                    pane,
                    result
                        .as_ref()
                        .err()
                        .map(|error| format!("{location} could not be listed: {error}")),
                );
            }
        }
        let batch = match buffered {
            Some(entries) if result.is_ok() => {
                let before = self.selected_ids(pane);
                self.model.replace_listing(pane, entries.clone());
                let after = self.selected_ids(pane);
                if before != after {
                    events.push(Event::SelectionChanged { pane, ids: after });
                }
                self.decorate(pane, &entries, events);
                entries
            }
            _ => {
                self.model.listing_done(pane);
                Vec::new()
            }
        };
        self.resolve_reveal(pane, true, events);
        // The Cloud overview's rows are the side panel's Cloud rows too, and
        // the Network overview's provider rows its Network rows (the
        // mounted shares are the volumes' own rows).
        match (result, Virtual::of(&location)) {
            (Ok(()), Some(Virtual::Cloud)) => {
                let rows: Vec<Entry> = self
                    .model
                    .active_tab(pane)
                    .map_or_default(|(_, tab)| tab.listing.entries().to_vec());
                self.model.cloud = rows;
            }
            (Ok(()), Some(Virtual::Network)) => {
                let rows: Vec<Entry> = self
                    .model
                    .active_tab(pane)
                    .map_or_default(|(_, tab)| tab.listing.entries().to_vec());
                self.model.network = provider_rows(rows);
            }
            _ => {}
        }
        // Tree view: the expanded folders list their children again, swapped
        // in at once where they were listed before.
        let expanded = self.model.tree_expanded(pane);
        for (folder, location) in expanded {
            let listed = self
                .model
                .active_tab(pane)
                .is_some_and(|(_, tab)| !tab.tree.is_loading(folder));
            let running = self
                .live
                .get(&pane)
                .is_some_and(|l| l.children.contains_key(&folder));
            if !running {
                self.list_child(pane, folder, location, listed);
            }
        }
        events.push(Event::Listed {
            pane,
            batch,
            done: true,
        });
    }

    fn change(&mut self, pane: PaneId, generation: u64, change: FsChange, events: &mut Vec<Event>) {
        let Some(live) = self.live.get(&pane) else {
            return;
        };
        if live.generation != generation {
            return;
        }
        let before = self.selected_ids(pane);
        if self.model.apply_change(pane, change.clone()) {
            let after = self.selected_ids(pane);
            if before != after {
                events.push(Event::SelectionChanged { pane, ids: after });
            }
            events.push(Event::Changed { pane, change });
            self.schedule_redecorate(pane);
        } else {
            debug!(?pane, "rescan");
            self.refresh(pane);
        }
    }

    /// `Command::Reveal`: one tab per folder of `items`, in the focused
    /// window (the first in a window of its own with `new_window`), each
    /// selecting its files once listed.
    fn reveal(&mut self, items: &[Location], new_window: bool, properties: bool) {
        let mut folders: Vec<(Location, Vec<Vec<u8>>)> = Vec::new();
        for item in items {
            let Some(parent) = item.path.parent() else {
                continue;
            };
            let Some(name) = item.path.file_name() else {
                continue;
            };
            let folder = Location::new(item.provider, parent);
            match folders.iter_mut().find(|(f, _)| *f == folder) {
                Some((_, names)) => names.push(name.to_vec()),
                None => folders.push((folder, vec![name.to_vec()])),
            }
        }
        for (index, (folder, names)) in folders.into_iter().enumerate() {
            let pane = if index == 0 && new_window {
                self.model.new_window(folder).1
            } else {
                self.model.new_tab(None, folder).0
            };
            self.reveal_pending
                .insert(pane, (names, std::time::Instant::now()));
            if index == 0 && properties {
                self.reveal_properties = Some(pane);
            }
        }
    }

    /// Selects what a reveal asked for in `pane` once its names are
    /// listed: all of them as soon as every name has a row, what is there
    /// when the listing is `complete` or the wait is over.
    fn resolve_reveal(&mut self, pane: PaneId, complete: bool, events: &mut Vec<Event>) {
        let Some((names, since)) = self.reveal_pending.get(&pane) else {
            return;
        };
        let ids: Vec<EntryId> = self.model.active_tab(pane).map_or_default(|(_, tab)| {
            tab.listing
                .rows()
                .filter(|e| {
                    e.path
                        .file_name()
                        .is_some_and(|n| names.iter().any(|w| w == n))
                })
                .map(|e| e.id)
                .collect()
        });
        let all = ids.len() >= names.len();
        if !all && !complete && since.elapsed() < REVEAL_FOR {
            return;
        }
        self.reveal_pending.remove(&pane);
        let properties = self.reveal_properties.take_if(|p| *p == pane).is_some();
        if ids.is_empty() {
            return;
        }
        if let Some(ids) = self.model.select(pane, SelectionChange::Set(ids)) {
            events.push(Event::SelectionChanged { pane, ids });
        }
        if properties {
            self.show_properties(pane, events);
        }
    }

    /// Keeps each console and the explorer tab it was opened from in step
    /// (specs/system-integration.md §4): a working directory the shell
    /// reported (OSC 7) since the last look moves the explorer tab there,
    /// when it is its pane's active tab; the explorer's own navigation
    /// types `cd` into the console while the shell is at its prompt, never
    /// while a job has the terminal or the session cannot tell. A console
    /// or explorer tab that closed ends the link.
    fn sync_consoles(&mut self, events: &mut Vec<Event>) {
        let model = &self.model;
        self.console_links
            .retain(|tab, link| model.tab(*tab).is_some() && model.tab(link.explorer).is_some());
        let tabs: Vec<TabId> = self.console_links.keys().copied().collect();
        for tab in tabs {
            // A script the console was opened to run is typed once the
            // shell is at its prompt, or at once where the session cannot
            // tell: a new console has nothing running that could take it.
            if let Some(link) = self.console_links.get_mut(&tab)
                && link.line.is_some()
                && link.console.foreground_job() != Some(true)
                && let Some(line) = link.line.take()
            {
                link.console.write(&line);
            }
            let Some(link) = self.console_links.get(&tab) else {
                continue;
            };
            let Some(location) = self
                .model
                .active_tab(link.pane)
                .filter(|(id, _)| *id == link.explorer)
                .map(|(_, t)| t.location.clone())
            else {
                continue;
            };
            let pane = link.pane;
            let cwd = link.console.cwd();
            if !self.model.settings.integration.shell_integration {
                // Switched off in Settings → Integration: neither follows
                // the other, and what happened meanwhile is not caught up
                // on when it is switched on again.
                if let Some(link) = self.console_links.get_mut(&tab) {
                    link.cwd_seen = cwd;
                    link.location_seen = location;
                }
                continue;
            }
            if cwd != link.cwd_seen {
                let to = cwd
                    .as_deref()
                    .map(|path| Location::new(ProviderId::LOCAL, platform::paths::from_path(path)));
                if let Some(link) = self.console_links.get_mut(&tab) {
                    link.cwd_seen = cwd;
                    if let Some(to) = &to {
                        link.location_seen = to.clone();
                    }
                }
                if let Some(to) = to
                    && to != location
                {
                    self.dispatch(Command::Navigate { pane, to }, events);
                }
                continue;
            }
            if location != link.location_seen {
                let path = (location.provider == ProviderId::LOCAL)
                    .then(|| platform::paths::to_path(&location.path).ok())
                    .flatten();
                if let Some(path) = path
                    && link.console.foreground_job() == Some(false)
                {
                    link.console.write(&cd_command(&path));
                }
                if let Some(link) = self.console_links.get_mut(&tab) {
                    link.location_seen = location;
                }
            }
        }
    }

    fn selected_ids(&self, pane: PaneId) -> Vec<EntryId> {
        self.model
            .active_tab(pane)
            .map_or_default(|(_, tab)| tab.selected().map(|e| e.id).collect())
    }

    fn on_job_event(&mut self, event: JobEvent, events: &mut Vec<Event>) {
        match event {
            JobEvent::Started { id, name, pausable } => {
                if !self.internal.contains(&id) {
                    self.model.job_started(id, name, pausable);
                }
            }
            JobEvent::Progress { id, progress } => {
                if !self.internal.contains(&id) {
                    self.model.job_progress(id, progress.clone());
                    events.push(Event::JobProgress { id, progress });
                }
            }
            JobEvent::Finished { id, result } => {
                if self.internal.remove(&id) {
                    return;
                }
                let state = match &result {
                    Ok(()) => JobState::Done,
                    Err(Error::Cancelled) => JobState::Cancelled,
                    Err(error) => JobState::Failed(error.to_string()),
                };
                if let Err(error) = &result
                    && !matches!(error, Error::Cancelled)
                {
                    let name = self
                        .model
                        .jobs
                        .get(&id)
                        .map_or_default(|job| job.name.clone());
                    // Where the user acted, then what went wrong: the
                    // notice is shown in that pane's window.
                    events.push(Event::JobFailed {
                        id,
                        pane: self.pane_jobs.get(&id).copied(),
                    });
                    events.push(Event::Notify {
                        level: Level::Error,
                        text: format!("{name}: {error}"),
                    });
                }
                self.waiting.remove(&id);
                self.model.job_finished(id, state);
                self.integration_job_finished(id, &result, events);
                events.push(Event::JobDone { id, result });
                if let Some(pane) = self.pane_jobs.remove(&id) {
                    self.refresh(pane);
                    // An operation may have trashed, restored or emptied:
                    // the Places row's count and the Trash view follow, and
                    // every pane on a polled location checks again now.
                    self.refresh_trash();
                    self.poll_now();
                }
            }
        }
    }

    /// Watches the settings file's directory and reloads the file when it
    /// changes.
    fn watch_settings(&mut self) {
        let Some(dir) = self.settings_path.parent().map(Path::to_path_buf) else {
            return;
        };
        if let Err(error) = std::fs::create_dir_all(&dir) {
            warn!(%error, dir = %dir.display(), "config directory not created");
            return;
        }
        let Some(provider) = self.mounts.provider(vfs_local::PROVIDER_ID) else {
            return;
        };
        let location = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&dir));
        let mut changes = match provider.watch(&location) {
            Ok(changes) => changes,
            Err(error) => {
                warn!(%error, dir = %dir.display(), "settings not watched");
                return;
            }
        };
        let path = self.settings_path.clone();
        let tx = self.mailbox.clone();
        let cancel = self.settings_watch.clone();
        runtime::handle().spawn(async move {
            loop {
                let change = tokio::select! {
                    () = cancel.cancelled() => break,
                    next = changes.next() => match next {
                        Some(change) => change,
                        None => break,
                    },
                };
                let concerns_file = match &change {
                    FsChange::Created(e) | FsChange::Modified(e) => e.name == SETTINGS_FILE,
                    FsChange::Renamed { to, .. } => to.name == SETTINGS_FILE,
                    FsChange::Removed(_) | FsChange::Rescan => true,
                };
                if !concerns_file {
                    continue;
                }
                let path = path.clone();
                let result = runtime::handle()
                    .spawn_blocking(move || settings::load(&path))
                    .await
                    .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))));
                if !tx.send(Msg::Settings(result)) {
                    break;
                }
            }
        });
    }
}

/// Whether `location` is on the volume mounted at `mount_point`: a local
/// folder below it, or a location inside an archive or a disk image whose
/// file is.
fn on_volume(mount_point: &Path, location: &Location) -> bool {
    (location.provider == ProviderId::LOCAL || location.provider == ProviderId::ARCHIVE)
        && platform::paths::to_path(&location.path).is_ok_and(|path| path.starts_with(mount_point))
}

/// The connection a location is on, for a provider that keeps its
/// connections as the folders at its root: the first component of the
/// path.
fn source_at(location: &Location) -> Option<String> {
    location
        .path
        .as_bytes()
        .split(|b| matches!(b, b'/' | b'\\'))
        .find(|part| !part.is_empty())
        .map(|first| String::from_utf8_lossy(first).into_owned())
}

/// Whether `path`, below `root`, is or lies under an entry the ignore
/// list names: a component of its path from `root` on is one of `names`.
fn below_ignored(root: &VPath, path: &VPath, names: &[String]) -> bool {
    let (root, path) = (root.as_bytes(), path.as_bytes());
    let rest = path.strip_prefix(root).unwrap_or(path);
    rest.split(|b| matches!(b, b'/' | b'\\'))
        .any(|part| names.iter().any(|name| name.as_bytes() == part))
}

/// The search indexes the title-row search box queries: every registered
/// one whose service is there and whose scope is the whole machine. A walk
/// scoped `Under` a root answers the filter bar instead (QUESTIONS.md 118).
fn available_indexes(registry: &Registry) -> Vec<Arc<dyn SearchIndex>> {
    registry
        .search_indexes()
        .iter()
        .filter(|index| {
            index.scope() == SearchScope::Everywhere
                && index.availability() == Availability::Enabled
        })
        .cloned()
        .collect()
}

/// Polls a location its provider cannot watch: its `stat` every
/// [`POLL_EVERY`], and at once when `poke` is rung, reporting
/// [`FsChange::Rescan`] through `report` when the entry's size or times
/// changed (a view counts its items in `size`; an archive is its file).
/// Stops with `cancel`, or when `report` says the loop is gone.
fn poll_location(
    provider: Arc<dyn ext::Provider>,
    location: Location,
    cancel: CancelToken,
    poke: &runtime::watch::Sender<u64>,
    report: impl Fn(FsChange) -> bool + Send + 'static,
) {
    let mut poke = poke.subscribe();
    runtime::handle().spawn(async move {
        let marker = |entry: &Entry| (entry.size, entry.modified, entry.created);
        let mut last = provider.stat(&location).await.ok().map(|e| marker(&e));
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(POLL_EVERY) => {}
                changed = poke.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
            if cancel.is_cancelled() {
                break;
            }
            let now = provider.stat(&location).await.ok().map(|e| marker(&e));
            if now != last {
                last = now;
                if !report(FsChange::Rescan) {
                    break;
                }
            }
        }
    });
}

/// A writer that saves every snapshot sent to it, in order, off the
/// caller's thread; only the latest of a burst is written.
fn saver<T, F>(write: F) -> mpsc::UnboundedSender<T>
where
    T: Send + 'static,
    F: Fn(&T) -> Result<(), Error> + Send + Sync + 'static,
{
    let (tx, mut rx) = mpsc::unbounded_channel::<T>();
    let write = Arc::new(write);
    runtime::handle().spawn(async move {
        while let Some(mut value) = rx.recv().await {
            while let Ok(newer) = rx.try_recv() {
                value = newer;
            }
            let write = Arc::clone(&write);
            let written = runtime::handle()
                .spawn_blocking(move || write(&value))
                .await;
            match written {
                Ok(Ok(())) => {}
                Ok(Err(error)) => warn!(%error, "not saved"),
                Err(error) => warn!(%error, "writer stopped"),
            }
        }
    });
    tx
}

/// The text of a file's bytes for the clipboard: by its byte-order mark
/// when it has one (UTF-8, UTF-16), as UTF-8 otherwise, with anything that
/// is no UTF-8 replaced.
#[must_use]
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some((encoding, bom)) = encoding_rs::Encoding::for_bom(bytes) {
        let (text, _, _) = encoding.decode(&bytes[bom..]);
        return text.into_owned();
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// The text the preview pane's *Copy* puts on the clipboard for a file's
/// bytes: a gzip-compressed file (an SVGZ, whose previewer shows the
/// inflated XML) is inflated first, within [`COPY_LIMIT`]; then
/// [`decode_text`].
///
/// # Errors
///
/// `Error::Unsupported` when the file inflates beyond the limit, `Error::Io`
/// for gzip that does not inflate.
pub fn copied_text(bytes: &[u8]) -> Result<String, Error> {
    use std::io::Read as _;

    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(decode_text(bytes));
    }
    let mut inflated = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(COPY_LIMIT as u64 + 1)
        .read_to_end(&mut inflated)
        .map_err(Error::from_io)?;
    if inflated.len() > COPY_LIMIT {
        return Err(Error::Unsupported(
            "the file inflates beyond the copy limit",
        ));
    }
    Ok(decode_text(&inflated))
}

/// The tab's one selected entry, when exactly one is selected and it is not
/// a folder: what the preview shows.
fn single_file(tab: &model::Tab) -> Option<Entry> {
    let mut selected = tab.selected();
    let first = selected.next()?;
    if selected.next().is_some() || first.kind == EntryKind::Dir {
        return None;
    }
    Some(first.clone())
}

/// The top of the volume or provider `path` sits on: `/`, `C:\`, or the
/// path itself when it has no parent.
#[must_use]
pub fn top_of(path: &VPath) -> VPath {
    let mut current = path.clone();
    while let Some(parent) = current.parent() {
        current = parent;
    }
    current
}

/// Puts every tab of a restored session somewhere that exists: a location
/// whose provider is not mounted opens at Home; a local folder that is gone
/// opens at its nearest existing
/// ancestor, and so does an archive location whose archive is gone, as a
/// local folder; each with a note for the status line. Locations on other
/// providers are taken as they are.
fn repair(mounts: &Mounts, home: &Location, session: &mut Session) {
    for tab in session
        .windows
        .iter_mut()
        .flat_map(|w| {
            w.panes
                .iter_mut()
                .chain(w.workspaces.iter_mut().flat_map(|ws| ws.panes.iter_mut()))
        })
        .flat_map(|p| p.tabs.iter_mut())
    {
        if mounts.provider(tab.location.provider).is_none() {
            tab.note = Some(format!(
                "{} is not available; opened Home instead of {}",
                tab.location.provider, tab.location.path
            ));
            tab.location = home.clone();
            continue;
        }
        let archive = tab.location.provider == ProviderId::ARCHIVE;
        if tab.location.provider != ProviderId::LOCAL && !archive {
            continue;
        }
        let Ok(path) = platform::paths::to_path(&tab.location.path) else {
            continue;
        };
        if path.is_dir() && !archive {
            continue;
        }
        if archive && path.ancestors().any(std::path::Path::is_file) {
            // The archive is still there; what is inside it is its own
            // listing's business.
            continue;
        }
        let mut ancestor = path.parent();
        while let Some(dir) = ancestor {
            if dir.is_dir() {
                tab.note = Some(format!(
                    "{} no longer exists; opened {}",
                    tab.location.path,
                    dir.display()
                ));
                tab.location =
                    Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(dir));
                break;
            }
            ancestor = dir.parent();
        }
        if ancestor.is_none() {
            tab.note = Some(format!(
                "{} no longer exists; opened Home",
                tab.location.path
            ));
            tab.location = home.clone();
        }
    }
}

/// The contents of the Properties dialogue for `entry`: where it is, what
/// it occupies, what a link resolves to, and the permissions of a local
/// entry the platform can read them of. `contents` is where the count of
/// a folder stands.
fn read_properties(
    platform: &dyn Platform,
    pane: PaneId,
    entry: Entry,
    path: Option<PathBuf>,
    location: &Location,
    contents: Contents,
) -> Result<PropertiesInfo, Error> {
    let shown = path
        .as_ref()
        .and_then(|p| p.parent())
        .map_or_else(|| location.to_string(), |p| p.display().to_string());
    let on_disk = path
        .as_ref()
        .and_then(|p| platform::fs::allocated_size(p).ok());
    let target = match (&entry.kind, &path) {
        (types::EntryKind::Symlink { target, .. }, Some(path)) => link_target(path, target),
        _ => None,
    };
    let permissions = match &path {
        Some(path) => match platform.permissions(path) {
            Ok(permissions) => Some(permissions),
            Err(Error::Unsupported(_)) => None,
            Err(error) => return Err(error),
        },
        None => None,
    };
    let principals = if permissions.is_some() {
        platform.principals().unwrap_or_default()
    } else {
        platform::Principals::default()
    };
    Ok(PropertiesInfo {
        pane,
        entry,
        path,
        location: shown,
        on_disk,
        target,
        contents,
        permissions,
        principals,
        native: platform::NATIVE_PROPERTIES,
    })
}

/// What the link at `path` resolves to: the entry at the end of the chain
/// of links, with the path the link holds — `held`, taken from the link's
/// folder when it is relative — as its path, so its folder is the one the
/// link names. `None` for a broken link.
fn link_target(path: &Path, held: &VPath) -> Option<Entry> {
    let real = std::fs::canonicalize(path).ok()?;
    let held = platform::paths::to_path(held).ok()?;
    let named = lexical(&path.parent()?.join(held));
    // `..` after a link in the path names another folder than the text
    // says: then the real path is the one to go to.
    let at = if std::fs::symlink_metadata(&named).is_ok() {
        named
    } else {
        real.clone()
    };
    vfs_local::entry_at(&real, &platform::paths::from_path(&at)).ok()
}

/// `path` without its `.` and `..` components, as the text reads; nothing
/// is looked up.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push(component);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Counts what is inside the folder at `path`: every file and folder below
/// it, the files' sizes and what all of it occupies on disk. Links count as
/// themselves and are never followed; an entry that cannot be read counts
/// nothing. `None` when `cancel` was set before the end.
fn count_tree(path: &Path, cancel: &CancelToken) -> Option<FolderContents> {
    let mut contents = FolderContents::default();
    let mut folders = vec![path.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            if cancel.is_cancelled() {
                return None;
            }
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            contents.items += 1;
            contents.on_disk += platform::fs::allocated_size(&path).unwrap_or(0);
            if meta.is_dir() {
                folders.push(path);
            } else {
                contents.bytes += meta.len();
            }
        }
    }
    Some(contents)
}

/// One opening of a preview: what [`App::start_preview`] gathered for the
/// job.
struct PreviewRun {
    pane: PaneId,
    generation: u64,
    entry: Entry,
    loc: Location,
    provider: Arc<dyn ext::Provider>,
    previewers: Vec<Arc<dyn Previewer>>,
    /// The previewer asked for whatever the scores say.
    with: Option<ExtId>,
    /// The entry is on a remote source.
    remote: bool,
    /// A whole-file preview above `cap` waits for the user.
    capped: bool,
    cap: u64,
    /// How many times *Load more* was pressed.
    more: u32,
    platform: Arc<dyn Platform>,
    local: Option<PathBuf>,
    tx: Mailbox,
}

impl PreviewRun {
    /// Sniffs the file, chooses its previewer and opens it, answering
    /// through the mailbox: a document or its error, nobody's file, or a
    /// file held behind the size limit.
    async fn open(self, cancel: CancelToken) -> Result<(), Error> {
        let Self {
            pane,
            generation,
            entry,
            loc,
            provider,
            previewers,
            with,
            remote,
            capped,
            cap,
            more,
            platform,
            local,
            tx,
        } = self;
        let sniff = ext::read_bytes(&*provider, &loc, Some(4096), &cancel).await?;
        let Some(previewer) = choose_previewer(&previewers, with, &entry, &sniff) else {
            let permissions = permissions_of(platform, local, entry.kind.clone()).await;
            tx.send(Msg::PreviewNone {
                pane,
                generation,
                permissions,
            });
            return Ok(());
        };
        let mut prefix = None;
        let result = if remote && previewer.reads() == Reads::Prefix {
            let limit = REMOTE_PREFIX_CHUNK * (u64::from(more) + 1);
            if entry.size.is_none_or(|size| size > limit) {
                prefix = Some((entry.size.map_or(limit, |s| s.min(limit)), entry.size));
            }
            let limited: Arc<dyn ext::Provider> = Arc::new(Prefixed {
                inner: provider,
                limit,
                meter: None,
            });
            previewer.open(&loc, limited, cancel.clone()).await
        } else if capped && entry.size.is_some_and(|size| size > cap) {
            tx.send(Msg::PreviewCapped {
                pane,
                generation,
                size: entry.size.unwrap_or(0),
                cap,
            });
            return Ok(());
        } else if remote {
            // A whole file from a remote source: its bytes are counted as
            // they arrive, for the pane's progress.
            let metered: Arc<dyn ext::Provider> = Arc::new(Prefixed {
                inner: provider,
                limit: u64::MAX,
                meter: Some(Meter::new(tx.clone(), pane, generation, entry.size)),
            });
            previewer.open(&loc, metered, cancel.clone()).await
        } else {
            previewer.open(&loc, provider, cancel.clone()).await
        };
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        tx.send(Msg::Preview {
            pane,
            generation,
            result,
            previewer: previewer.id(),
            prefix,
        });
        Ok(())
    }
}

/// The entry as a path on this machine, for an entry of the local
/// provider.
fn local_path(entry: &Entry) -> Option<PathBuf> {
    (entry.id.provider == vfs_local::PROVIDER_ID)
        .then(|| platform::paths::to_path(&entry.path).ok())
        .flatten()
}

/// The previewer that shows `entry`: the one `asked` for, whatever the
/// scores say, else the highest-scoring one that claims the entry at all.
fn choose_previewer(
    previewers: &[Arc<dyn Previewer>],
    asked: Option<ExtId>,
    entry: &Entry,
    sniff: &[u8],
) -> Option<Arc<dyn Previewer>> {
    match asked {
        Some(asked) => previewers.iter().find(|p| p.id() == asked).cloned(),
        None => previewers
            .iter()
            .map(|p| (p.score(entry, sniff), p))
            .filter(|(score, _)| *score > 0)
            .max_by_key(|(score, _)| *score)
            .map(|(_, p)| Arc::clone(p)),
    }
}

/// The [`permission_line`] of the local file at `path`, read off the UI
/// thread; `None` for an entry that is not on this machine.
async fn permissions_of(
    platform: Arc<dyn Platform>,
    path: Option<PathBuf>,
    kind: EntryKind,
) -> Option<String> {
    let path = path?;
    runtime::handle()
        .spawn_blocking(move || {
            let read = platform.permissions(&path).ok()?;
            permission_line(&read, &kind)
        })
        .await
        .ok()
        .flatten()
}

/// An entry's mode as `ls -l` writes it and its `owner:group`,
/// `-rw-r--r-- serge:serge`, for the File info card; `None` where the
/// platform has no mode bits.
fn permission_line(permissions: &platform::FilePermissions, kind: &EntryKind) -> Option<String> {
    let mode = permissions.mode?;
    let mut text = String::with_capacity(10);
    text.push(match kind {
        EntryKind::Dir => 'd',
        EntryKind::Symlink { .. } => 'l',
        EntryKind::File | EntryKind::Other => '-',
    });
    // The execute place of each triplet carries its special bit: setuid,
    // setgid, sticky.
    let special = [(0o4000, 's', 'S'), (0o2000, 's', 'S'), (0o1000, 't', 'T')];
    for (index, (bit, set, alone)) in special.into_iter().enumerate() {
        let shift = 6 - 3 * index;
        let triplet = (mode >> shift) & 7;
        text.push(if triplet & 4 == 0 { '-' } else { 'r' });
        text.push(if triplet & 2 == 0 { '-' } else { 'w' });
        text.push(match (triplet & 1 != 0, mode & bit != 0) {
            (true, true) => set,
            (false, true) => alone,
            (true, false) => 'x',
            (false, false) => '-',
        });
    }
    Some(format!(
        "{text} {}:{}",
        permissions.owner.display(),
        permissions.group.display()
    ))
}

/// The count the Properties dialogue waits for or holds.
struct PropertiesCount {
    /// The pane the dialogue was opened from.
    pane: PaneId,
    /// The folder counted.
    path: PathBuf,
    /// Stops the count.
    cancel: CancelToken,
    /// What it found, once it ended.
    result: Option<FolderContents>,
}

/// A provider whose reads stop after `limit` bytes: what a prefix
/// previewer is handed for a file on a remote provider, so a 1 GiB log
/// previews from its first chunk. Everything else goes to the provider
/// itself.
struct Prefixed {
    inner: Arc<dyn ext::Provider>,
    limit: u64,
    /// Counts what is read and tells the pane, for a whole file fetched
    /// from a remote source.
    meter: Option<Meter>,
}

/// How often a download tells how far it is.
const PROGRESS_STEP: u64 = 128 * 1024;

/// Counts the bytes a preview reads from a remote file and reports them
/// to the application's loop, every [`PROGRESS_STEP`] and at the end.
#[derive(Clone)]
struct Meter {
    tx: Mailbox,
    pane: PaneId,
    generation: u64,
    total: Option<u64>,
    read: Arc<std::sync::atomic::AtomicU64>,
    told: Arc<std::sync::atomic::AtomicU64>,
}

impl Meter {
    fn new(tx: Mailbox, pane: PaneId, generation: u64, total: Option<u64>) -> Self {
        Self {
            tx,
            pane,
            generation,
            total,
            read: Arc::default(),
            told: Arc::default(),
        }
    }

    /// `bytes` more were read; `end` says the reader is done, or waits.
    fn add(&self, bytes: u64, end: bool) {
        use std::sync::atomic::Ordering;
        let read = self.read.fetch_add(bytes, Ordering::Relaxed) + bytes;
        let told = self.told.load(Ordering::Relaxed);
        if read >= told + PROGRESS_STEP || (end && read != told) {
            self.told.store(read, Ordering::Relaxed);
            self.tx.send(Msg::PreviewProgress {
                pane: self.pane,
                generation: self.generation,
                read,
                total: self.total,
            });
        }
    }

    /// `reader` with its bytes counted.
    fn wrap(&self, reader: ext::ReadHandle) -> ext::ReadHandle {
        // Before the first byte: the pane shows the download has begun.
        self.tx.send(Msg::PreviewProgress {
            pane: self.pane,
            generation: self.generation,
            read: self.read.load(std::sync::atomic::Ordering::Relaxed),
            total: self.total,
        });
        Box::new(Metered {
            inner: reader,
            meter: self.clone(),
        })
    }
}

/// A reader whose bytes a [`Meter`] counts.
struct Metered {
    inner: ext::ReadHandle,
    meter: Meter,
}

impl tokio::io::AsyncRead for Metered {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let polled = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        match &polled {
            std::task::Poll::Ready(Ok(())) => {
                let bytes = (buf.filled().len() - before) as u64;
                self.meter.add(bytes, bytes == 0);
            }
            // The link stalls: what has arrived is told now.
            std::task::Poll::Pending => self.meter.add(0, true),
            std::task::Poll::Ready(Err(_)) => {}
        }
        polled
    }
}

impl std::fmt::Debug for Prefixed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prefixed")
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

#[ext::async_trait]
impl ext::Provider for Prefixed {
    fn id(&self) -> ProviderId {
        self.inner.id()
    }
    fn capabilities(&self, loc: &Location) -> Capabilities {
        self.inner.capabilities(loc)
    }
    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken) -> ext::EntryStream {
        self.inner.list(loc, opts, cancel)
    }
    async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
        self.inner.stat(loc).await
    }
    async fn open_read(&self, loc: &Location) -> Result<ext::ReadHandle, Error> {
        use tokio::io::AsyncReadExt as _;
        let reader = self.inner.open_read(loc).await?;
        let reader: ext::ReadHandle = Box::new(reader.take(self.limit));
        Ok(match &self.meter {
            Some(meter) => meter.wrap(reader),
            None => reader,
        })
    }
    async fn open_read_at(&self, loc: &Location, offset: u64) -> Result<ext::ReadHandle, Error> {
        use tokio::io::AsyncReadExt as _;
        let reader = self.inner.open_read_at(loc, offset).await?;
        let reader: ext::ReadHandle = Box::new(reader.take(self.limit.saturating_sub(offset)));
        Ok(match &self.meter {
            Some(meter) => meter.wrap(reader),
            None => reader,
        })
    }
    async fn open_write(
        &self,
        loc: &Location,
        opts: ext::WriteOptions,
    ) -> Result<ext::WriteHandle, Error> {
        self.inner.open_write(loc, opts).await
    }
    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error> {
        self.inner.create_dir(loc).await
    }
    async fn rename(&self, from: &Location, to: &Location) -> Result<Entry, Error> {
        self.inner.rename(from, to).await
    }
    async fn remove(&self, loc: &Location, mode: ext::RemoveMode) -> Result<(), Error> {
        self.inner.remove(loc, mode).await
    }
    async fn link(
        &self,
        target: &Location,
        at: &Location,
        kind: ext::LinkKind,
    ) -> Result<Entry, Error> {
        self.inner.link(target, at, kind).await
    }
    async fn copy(
        &self,
        from: &Location,
        to: &Location,
        opts: ext::CopyOptions,
        progress: ext::ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        self.inner.copy(from, to, opts, progress, cancel).await
    }
    fn remote_preview_cap(&self, loc: &Location) -> Option<u64> {
        self.inner.remote_preview_cap(loc)
    }
    fn remote_preview_always(&self, loc: &Location) -> bool {
        self.inner.remote_preview_always(loc)
    }
    fn watch(&self, loc: &Location) -> Result<ext::ChangeStream, Error> {
        self.inner.watch(loc)
    }
}

/// The connections of `provider` as the settings hold them, for its
/// connector.
fn connections_of(settings: &Settings, provider: ProviderId) -> Vec<ext::Connection> {
    settings
        .connections
        .iter()
        .filter(|c| c.provider == provider.0)
        .map(ConnectionEntry::to_connection)
        .collect()
}

/// The rows of the Network overview that are a provider's, not a mounted
/// share's: the side panel draws the shares from the volumes.
fn provider_rows(rows: Vec<Entry>) -> Vec<Entry> {
    rows.into_iter()
        .filter(|e| e.id.provider != vfs_local::PROVIDER_ID)
        .collect()
}

/// Whether moving or copying `source` into `to` would change nothing: the
/// item is in `to` already, or `to` is the item itself or lies under it.
#[must_use]
pub fn transfer_is_noop(source: &Location, to: &Location) -> bool {
    if source.provider != to.provider {
        return false;
    }
    source.path.parent().as_ref() == Some(&to.path) || to.path.is_inside(&source.path)
}

impl Drop for App {
    fn drop(&mut self) {
        self.settings_watch.cancel();
        self.volumes_watch.cancel();
        for live in self.live.values() {
            live.cancel.cancel();
            for child in live.children.values() {
                child.cancel.cancel();
            }
        }
    }
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("panes", &self.model.all_panes())
            .field("live", &self.live.keys().collect::<Vec<_>>())
            .field("undo", &self.undo.len())
            .field("redo", &self.redo.len())
            .finish_non_exhaustive()
    }
}

/// The bytes under `path`: the file's size, or the sizes of everything in
/// the folder, links counted by their own size and never followed. An entry
/// that cannot be read counts nothing.
fn tree_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| tree_size(&entry.path()))
        .sum()
}

#[cfg(test)]
mod tests;
