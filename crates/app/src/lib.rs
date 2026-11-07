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
//! live: when the location, the hidden flag, the flatten flag or the
//! system-files setting changes, it cancels the pane's listing job, starts a
//! new one that streams batches into the model, and watches the location
//! for changes until the next switch. Listing jobs do not appear in the job
//! table; their output is the [`Event::Listed`] stream. Only a pane's active
//! tab lists; the other tabs list when they are first shown.
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
mod session;
mod settings;

pub use command::{Command, Event, TransferMode};
pub use session::{LAYOUTS_DIR, SAVE_INTERVAL, SESSION_FILE};
pub use settings::SETTINGS_FILE;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use ext::{
    ActionCtx, ActionOutcome, Availability, DecorateCtx, Decoration, Decorator, Depth, Extension,
    FsChange, Invocation, Level, ListOptions, PreviewDoc, Previewer, Registry, UiHost, UndoCommand,
};
use futures::StreamExt;
use model::{JobState, Model, PaneId, Session, Settings, SettingsChange, TabId};
use ops::PinStore;
use platform::Platform;
use runtime::{JobEvent, JobRunner, mpsc};
use tracing::{debug, warn};
use types::{CancelToken, Entry, EntryId, Error, JobId, Location, Progress};
use vfs::Mounts;
use vfs_local::LocalProvider;

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
}

/// How long changes are collected before the decorators run again.
const REDECORATE_AFTER: Duration = Duration::from_millis(150);
/// How many of this process's own settings writes are remembered, so the
/// watcher's report of each is told from an edit.
const RECENT_SAVES: usize = 16;

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
}

impl PinBridge {
    /// Replaces the copy with the settings' list.
    fn sync(&self, pinned: &[Location]) {
        pinned.clone_into(&mut self.pinned.write().unwrap_or_else(PoisonError::into_inner));
    }

    /// The changes asked for since the last call.
    fn take(&self) -> Vec<SettingsChange> {
        std::mem::take(&mut self.pending.lock().unwrap_or_else(PoisonError::into_inner))
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

/// Which selection the preview follows and the job opening it.
struct PreviewState {
    /// The focused pane and its single selected file, when there is one.
    target: Option<(PaneId, EntryId)>,
    generation: u64,
    cancel: CancelToken,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            target: None,
            generation: 0,
            cancel: CancelToken::new(),
        }
    }
}

/// What a pane's listing was started for; a different key means relist.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ListKey {
    tab: TabId,
    location: Location,
    opts: ListOptions,
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
}

/// The application: model, registry, jobs, undo log and settings.
pub struct App {
    model: Model,
    mounts: Arc<Mounts>,
    registry: Registry,
    platform: Arc<dyn Platform>,
    ui: Arc<dyn UiHost>,
    pins: Arc<PinBridge>,
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
    /// What this process wrote lately: a reload that matches one is the
    /// watcher reporting our own write, not an edit to pick up.
    recent_saves: VecDeque<Settings>,
    preview: PreviewState,
    /// Thumbnails being rendered, so a cell asked for twice renders once.
    thumbnails: BTreeSet<(EntryId, u32)>,
    /// Events from start-up, delivered on the first poll.
    pending: Vec<Event>,
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
}

impl App {
    /// Builds the application: mounts the local provider, loads the
    /// built-in operations and `config.extensions`, reads the settings file
    /// and starts watching it, restores the session or the start layout, and
    /// lists every active tab's location.
    // Start-up is one straight line: registry, settings, session, loop.
    #[allow(clippy::too_many_lines)]
    #[must_use]
    pub fn new(config: Config) -> Self {
        let Config {
            platform,
            ui,
            extensions,
            config_dir,
            start,
        } = config;
        let mut pending = Vec::new();

        let mut registry = Registry::new();
        let mounts = Arc::new(Mounts::new());
        let trash: Arc<dyn platform::Trash> = Arc::clone(&platform) as Arc<dyn platform::Trash>;
        mounts.mount(Arc::new(LocalProvider::new(trash)));
        let pins = Arc::new(PinBridge::default());
        let core = ops::Core::new(
            Arc::clone(&mounts),
            Arc::clone(&platform),
            Arc::clone(&pins) as Arc<dyn PinStore>,
        );
        for ext in std::iter::once(&core as &dyn Extension).chain(extensions.iter().map(|e| &**e)) {
            if let Err(error) = registry.load(ext) {
                let id = ext.manifest().id;
                warn!(%error, ext = id.0, "extension refused");
                pending.push(Event::Notify {
                    level: Level::Error,
                    text: format!("Extension {} refused: {error}", id.0),
                });
            }
        }
        for provider in registry.providers() {
            mounts.mount(Arc::clone(provider));
        }

        let config_dir = config_dir.unwrap_or_else(|| platform.config_dir());
        let settings_path = config_dir.join(SETTINGS_FILE);
        let session_path = config_dir.join(SESSION_FILE);
        let layouts_dir = config_dir.join(LAYOUTS_DIR);
        let settings = match settings::load(&settings_path) {
            Ok(settings) => settings,
            Err(error) => {
                warn!(%error, path = %settings_path.display(), "settings not read");
                pending.push(Event::Notify {
                    level: Level::Warning,
                    text: format!("Settings file not read: {error}"),
                });
                Settings::default()
            }
        };
        pins.sync(&settings.pinned);

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
        let model = match restored {
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
        let (jobs, job_events) = JobRunner::with_events();
        let (tx, rx) = mpsc::unbounded_channel();
        let settings_saver = settings::saver(settings_path.clone());
        let layouts = session::layouts(&layouts_dir);
        let mut app = Self {
            model,
            mounts,
            registry,
            platform,
            ui,
            pins,
            jobs,
            job_events,
            decor_jobs: JobRunner::new(),
            mailbox: Mailbox {
                tx,
                waker: Arc::new(std::sync::Mutex::new(None)),
            },
            rx,
            live: BTreeMap::new(),
            internal: BTreeSet::new(),
            pane_jobs: BTreeMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            settings_path,
            settings_watch: CancelToken::new(),
            settings_saver,
            recent_saves: VecDeque::new(),
            preview: PreviewState::default(),
            thumbnails: BTreeSet::new(),
            pending,
            home,
            session_saver: session::Saver::new(session_path),
            last_sent: None,
            last_session,
            layouts_dir,
            layouts,
            exit: false,
        };
        app.watch_settings();
        let mut events = Vec::new();
        app.sync_panes(&mut events);
        app.sync_preview();
        app.pending.append(&mut events);
        app.sync_session();
        app
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
            let home = self.home.clone();
            self.model.add_window(&saved, &home);
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

    /// The entry the preview follows: the focused pane's single selected
    /// file, if any. The next [`Event::PreviewReady`] or
    /// [`Event::PreviewFailed`] for that pane is about it.
    #[must_use]
    pub fn preview_target(&self) -> Option<(PaneId, EntryId)> {
        self.preview.target
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

    /// Whether a listing is still streaming or a job still running.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.live.values().any(|l| !l.complete) || !self.jobs.running().is_empty()
    }

    /// The context an action gets for `pane`: its location and selection.
    #[must_use]
    pub fn action_ctx(&self, pane: PaneId, invoked_from: Invocation) -> Option<ActionCtx> {
        let (_, tab) = self.model.active_tab(pane)?;
        Some(ActionCtx {
            pane,
            location: tab.location.clone(),
            selection: tab.selected().cloned().collect(),
            invoked_from,
        })
    }

    /// Handles one command: mutates the model and appends the events it
    /// produced right away. Listings and jobs the command started report
    /// through [`poll`](Self::poll) and [`tick`](Self::tick).
    pub fn handle(&mut self, command: Command, events: &mut Vec<Event>) {
        debug!(?command, "command");
        self.dispatch(command, events);
        self.sync_panes(events);
        self.sync_preview();
        self.sync_session();
    }

    // One arm per command: the enum is the table.
    #[allow(clippy::too_many_lines)]
    fn dispatch(&mut self, command: Command, events: &mut Vec<Event>) {
        match command {
            Command::Navigate { pane, to } => {
                let before = self.selection_len(pane);
                if self.model.navigate(pane, to).is_some() {
                    Self::selection_cleared(pane, before, events);
                }
            }
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
                if let Some(ids) = self.model.select(pane, change) {
                    events.push(Event::SelectionChanged { pane, ids });
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
            Command::ToggleHidden(pane) => {
                self.model.toggle_hidden(pane);
            }
            Command::Flatten { pane, on } => {
                let before = self.selection_len(pane);
                if self.model.set_flatten(pane, on) {
                    Self::selection_cleared(pane, before, events);
                }
            }
            Command::Run { action, ctx } => self.run(action, ctx, events),
            Command::NewTab(location) => {
                self.model.new_tab(None, location);
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
            Command::Split { pane, dir } => {
                self.model.split(pane, dir);
            }
            Command::MoveTab { tab, to } => {
                self.model.move_tab(tab, to);
            }
            Command::CancelJob(id) => {
                self.jobs.cancel(id);
            }
            Command::DismissJob(id) => {
                self.model.job_dismiss(id);
            }
            Command::Undo => self.undo(events),
            Command::Redo => self.redo(events),
            Command::Settings(change) => self.change_settings(change),
            Command::Thumbnail { pane, id, size } => self.thumbnail(pane, id, size),
            Command::TabOrder { pane, order } => {
                self.model.set_tab_order(pane, order);
            }
            Command::Layout { window, dock } => {
                self.model.set_layout(window, dock);
            }
            Command::Scroll { tab, offset } => {
                self.model.set_scroll(tab, offset);
            }
            Command::WindowGeometry { window, geometry } => {
                self.model.set_geometry(window, geometry);
            }
            Command::SidePanel { window, change } => {
                self.model.side_panel(window, change);
            }
            Command::FocusWindow(window) => {
                self.model.focus_window(window);
            }
            Command::NewWindow => self.new_window(events),
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
        }
        self.sync_panes(events);
        self.sync_preview();
    }

    /// Processes everything that has arrived from listings, watches and
    /// jobs without waiting. Returns how many messages it handled.
    pub fn poll(&mut self, events: &mut Vec<Event>) -> usize {
        events.append(&mut self.pending);
        let mut handled = 0;
        loop {
            let before = handled;
            while let Ok(msg) = self.rx.try_recv() {
                self.on_msg(msg, events);
                handled += 1;
            }
            while let Ok(event) = self.job_events.try_recv() {
                self.on_job_event(event, events);
                handled += 1;
            }
            if handled == before {
                break;
            }
        }
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
        match ops::transfer(&self.jobs, Arc::clone(&self.mounts), &sources, to, kind) {
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
            Ok(ActionOutcome::Done) => {}
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
        settings.apply(change);
        self.pins.sync(&settings.pinned);
        self.model.apply_settings(settings.clone());
        self.recent_saves.push_back(settings.clone());
        while self.recent_saves.len() > RECENT_SAVES {
            self.recent_saves.pop_front();
        }
        // The saver lives as long as the runtime; a closed channel means
        // the process is going down.
        let _ = self.settings_saver.send(settings);
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
            .map(|(_, tab)| tab.selection.iter().copied().collect())
            .unwrap_or_default();
        if after != before {
            events.push(Event::SelectionChanged { pane, ids: after });
        }
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
        match action.run(ctx, &self.jobs, &*self.ui) {
            Ok(ActionOutcome::Done) => {}
            Ok(ActionOutcome::Started(id)) => {
                self.pane_jobs.insert(id, pane);
            }
            Ok(ActionOutcome::Undoable(command)) => {
                self.undo.push(command);
                self.redo.clear();
            }
            Err(error) => events.push(Event::Notify {
                level: Level::Error,
                text: format!("{}: {error}", action.label()),
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
    fn relist(&mut self, pane: PaneId, key: ListKey, reconcile: bool) {
        let generation = self.live.get(&pane).map_or(0, |l| l.generation) + 1;
        if let Some(old) = self.live.remove(&pane) {
            old.cancel.cancel();
        }
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
        let tx = self.mailbox.clone();
        let location = key.location.clone();
        let opts = key.opts;
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
                    Err(error) => debug!(%location, %error, "no watch"),
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
            },
        );
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
            .map(|(_, tab)| tab.listing.entries().to_vec())
            .unwrap_or_default();
        self.decorate(pane, &entries, events);
    }

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
            }
            Msg::ListDone {
                pane,
                generation,
                result,
            } => self.list_done(pane, generation, &result, events),
            Msg::Change {
                pane,
                generation,
                change,
            } => self.change(pane, generation, change, events),
            Msg::Settings(Ok(settings)) => {
                if self.recent_saves.contains(&settings) {
                    // Our own write coming back through the watcher; an
                    // earlier one may have been written over by a later
                    // change already.
                    return;
                }
                if settings != self.model.settings {
                    debug!("settings file changed");
                    self.pins.sync(&settings.pinned);
                    self.model.apply_settings(settings);
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
            } => {
                if generation != self.preview.generation
                    || self.preview.target.map(|(p, _)| p) != Some(pane)
                {
                    return;
                }
                events.push(match result {
                    Ok(doc) => Event::PreviewReady { pane, doc },
                    Err(error) => Event::PreviewFailed { pane, error },
                });
            }
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
        let previewers: Vec<Arc<dyn Previewer>> = self.registry.previewers().to_vec();
        let dark = self.model.settings.theme != model::Theme::Light;
        let tx = self.mailbox.clone();
        let handle =
            self.jobs
                .spawn_named(format!("Thumbnail {}", entry.name), move |job| async move {
                    let frame = async {
                        let sniff =
                            ext::read_bytes(&*provider, &loc, Some(4096), &job.cancel).await?;
                        let best = previewers
                            .iter()
                            .map(|p| (p.score(&entry, &sniff), p))
                            .filter(|(score, _)| *score > 0)
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
        let focused = self.model.focused;
        let want = self.model.active_tab(focused).and_then(|(_, tab)| {
            let mut selected = tab.selected();
            let first = selected.next()?;
            if selected.next().is_some() || first.kind != types::EntryKind::File {
                return None;
            }
            Some((first.clone(), tab.location.provider))
        });
        let target = want.as_ref().map(|(entry, _)| (focused, entry.id));
        if target == self.preview.target {
            return;
        }
        self.preview.cancel.cancel();
        self.preview.generation += 1;
        self.preview.target = target;
        let Some((entry, provider_id)) = want else {
            return;
        };
        let loc = Location::new(provider_id, entry.path.clone());
        let provider = match self.mounts.resolve(&loc) {
            Ok(provider) => provider,
            Err(error) => {
                self.pending.push(Event::PreviewFailed {
                    pane: focused,
                    error,
                });
                return;
            }
        };
        let previewers: Vec<Arc<dyn Previewer>> = self.registry.previewers().to_vec();
        let tx = self.mailbox.clone();
        let generation = self.preview.generation;
        let handle =
            self.jobs
                .spawn_named(format!("Preview {}", entry.name), move |job| async move {
                    let sniff = ext::read_bytes(&*provider, &loc, Some(4096), &job.cancel).await?;
                    let best = previewers
                        .iter()
                        .map(|p| (p.score(&entry, &sniff), p))
                        .filter(|(score, _)| *score > 0)
                        .max_by_key(|(score, _)| *score)
                        .map(|(_, p)| Arc::clone(p));
                    let result = match best {
                        Some(previewer) => previewer.open(&loc, provider, job.cancel.clone()).await,
                        None => Err(Error::Unsupported("no previewer for this file")),
                    };
                    if job.cancel.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    tx.send(Msg::Preview {
                        pane: focused,
                        generation,
                        result,
                    });
                    Ok(())
                });
        self.internal.insert(handle.id);
        self.preview.cancel = handle.cancel;
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
        if let Err(error) = result {
            events.push(Event::Notify {
                level: Level::Error,
                text: format!("Listing {location}: {error}"),
            });
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

    fn selected_ids(&self, pane: PaneId) -> Vec<EntryId> {
        self.model
            .active_tab(pane)
            .map(|(_, tab)| tab.selected().map(|e| e.id).collect())
            .unwrap_or_default()
    }

    fn on_job_event(&mut self, event: JobEvent, events: &mut Vec<Event>) {
        match event {
            JobEvent::Started { id, name } => {
                if !self.internal.contains(&id) {
                    self.model.job_started(id, name);
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
                        .map(|job| job.name.clone())
                        .unwrap_or_default();
                    events.push(Event::Notify {
                        level: Level::Error,
                        text: format!("{name}: {error}"),
                    });
                }
                self.model.job_finished(id, state);
                events.push(Event::JobDone { id, result });
                if let Some(pane) = self.pane_jobs.remove(&id) {
                    self.refresh(pane);
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

/// Puts every tab of a restored session somewhere that exists: a location
/// whose provider is not mounted opens at Home, a local folder that is gone
/// opens at its nearest existing ancestor, each with a note for the status
/// line. Locations on other providers are taken as they are.
fn repair(mounts: &Mounts, home: &Location, session: &mut Session) {
    for tab in session
        .windows
        .iter_mut()
        .flat_map(|w| w.panes.iter_mut())
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
        if tab.location.provider != vfs_local::PROVIDER_ID {
            continue;
        }
        let Ok(path) = platform::paths::to_path(&tab.location.path) else {
            continue;
        };
        if path.is_dir() {
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
        for live in self.live.values() {
            live.cancel.cancel();
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

#[cfg(test)]
mod tests;
