//! The windows: for each one the title row, dock of tabs and panes (the
//! Preview tab among them), side panel and operations area; and, shared by
//! all of them, the dialogues, palette, toasts and the keyboard.
//!
//! One [`RickUi`] holds the `App` and every window's state; the first
//! window is eframe's root viewport and every further one an immediate
//! viewport drawn from the same state in the same frame (QUESTIONS.md 68).
//! All windows share one app, one job table and one undo log. Each window
//! reports its geometry, its side panel, its tab order and split fractions
//! and its tabs' scroll offsets to the app, which keeps them in the session.

use ext::t;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use app::{App, Command, Event};
use egui::{ViewportBuilder, ViewportCommand, ViewportId};
use egui_dock::{DockArea, NodePath, TabViewer};
use ext::{ActionCtx, ActionId, Invocation, Level};
use model::{
    Column, Density, DockTarget, Geometry, IconSize, JobState, PaneId, PanelSide, SelectionChange,
    SettingsChange, SidePanelChange, SortDir, Tab, TabId, TabKind, Theme, ViewMode, WindowId,
};
use platform::raw_window_handle::{HasDisplayHandle as _, HasWindowHandle as _};
use platform::{ControlsSide, WindowRef};
use types::{EntryId, Location};

use crate::console::ConsoleUi;
use crate::dnd::{Dnd, DragView, Source, Spring, Target, Zone};
use crate::dock::DockSync;
use crate::host::{Ask, EguiHost};
use crate::inline::{EditKind, RowEdit};
use crate::keys::{Direction, Intent, KeyMap};
use crate::operations::Operations;
use crate::preview::Previews;
use crate::search::{Pick, SearchBox};
use crate::settings_ui::ShortcutsScreen;
use crate::side_panel::SidePanelState;
use crate::thumbs::Thumbnails;
use crate::{compare, console};
use crate::{
    conflict, frame, icons, inspector, menus, pane, paths, question, settings_ui, side_panel,
};
use crate::{hash, properties};

/// Per-tab view state that is the UI's business, not the model's.
#[derive(Debug, Default)]
pub struct TabUi {
    /// The entry keyboard navigation moves from.
    pub cursor: Option<EntryId>,
    /// Where a range selection starts.
    pub anchor: Option<EntryId>,
    /// The path bar's text while it is being edited.
    pub path_edit: Option<String>,
    /// Focus the path bar on the next frame.
    pub focus_path: bool,
    /// Focus the filter on the next frame.
    pub focus_filter: bool,
    /// Scroll this row into view on the next frame.
    pub scroll_to: Option<usize>,
    /// The toolbar's widget rectangles of the last frame by group ("left",
    /// "middle", "right"), for tests that check the layout.
    pub toolbar_rects: Vec<(&'static str, egui::Rect)>,
    /// The toolbar row itself in the last frame, `None` before the first.
    pub toolbar_row: Option<egui::Rect>,
    /// Forget the column widths on the next frame.
    pub reset_columns: bool,
    /// How many rows the last frame could show, for Page Up and Down.
    pub visible_rows: usize,
    /// The list's vertical scroll offset in the last frame.
    pub scroll_offset: f32,
    /// Scroll to this offset on the next frame.
    pub pending_offset: Option<f32>,
    /// The scroll offset to restore when the tab is back at this location
    /// after a drag spring-loaded out of it.
    pub scroll_restore: Option<(Location, f32)>,
    /// The session's scroll offset, applied once the listing is complete.
    pub restore_scroll: Option<f32>,
    /// The location drawn last frame, to notice a change.
    pub last_location: Option<Location>,
    /// Select the entry with this name once it is listed, giving up after
    /// `SELECT_FOR` (5 s): the entry may still be on its way (a new folder's
    /// job, a renamed entry's change event).
    pub pending_select: Option<(Vec<u8>, Instant)>,
    /// The name being edited in place, if any.
    pub edit: Option<RowEdit>,
    /// The last plain click on a row and when, for the slow second click.
    pub last_click: Option<(EntryId, f64)>,
    /// The breadcrumbs of the location drawn last, so the filesystem is
    /// asked where an archive begins once per location, not per frame.
    pub crumbs: Option<(Location, Vec<paths::Crumb>)>,
}

impl TabUi {
    /// The breadcrumbs of `location`, computed when it is not the one
    /// cached.
    pub fn crumbs_of(&mut self, location: &Location) -> &[paths::Crumb] {
        if self.crumbs.as_ref().is_none_or(|(l, _)| l != location) {
            self.crumbs = Some((location.clone(), paths::crumbs(location)));
        }
        self.crumbs.as_ref().map_or(&[], |(_, c)| c.as_slice())
    }

    /// The view state of a tab the model holds: a restored scroll offset
    /// waits for the listing.
    fn for_tab(tab: &Tab) -> Self {
        Self {
            restore_scroll: (tab.scroll > 0.0).then_some(tab.scroll),
            ..Self::default()
        }
    }
}

/// What widgets ask the shell to do; applied after the frame's layout.
#[derive(Debug)]
pub enum Request {
    /// A command for the app.
    Command(Command),
    /// A shortcut-level intent on a pane.
    Intent {
        /// What.
        intent: Intent,
        /// Where.
        pane: PaneId,
    },
    /// Open an entry: a folder navigates, a file opens with the default.
    OpenEntry {
        /// The pane.
        pane: PaneId,
        /// The entry.
        id: EntryId,
    },
    /// Run an action on an explicit context, from a menu.
    RunAction {
        /// Which.
        action: ActionId,
        /// On what.
        ctx: ActionCtx,
    },
    /// A new tab at a location, in the focused pane.
    NewTabAt(Location),
    /// Give a pane the focus.
    FocusPane(PaneId),
    /// A notification.
    Toast(Level, String),
    /// Show the About window.
    About,
    /// Rows were dragged past the threshold: start a drag of them.
    StartDrag {
        /// The pane.
        pane: PaneId,
        /// Its tab.
        tab: TabId,
        /// The entries, in view order.
        items: Vec<EntryId>,
        /// With the right button, which asks on drop.
        right_button: bool,
    },
    /// Select the entry with this name in the pane once it is listed.
    SelectNamed {
        /// The pane.
        pane: PaneId,
        /// The entry's file name.
        name: Vec<u8>,
    },
    /// Start editing a name in the pane's list.
    StartEdit {
        /// The pane.
        pane: PaneId,
        /// Rename this entry, or name a new file or folder.
        kind: EditKind,
    },
    /// The inline editor committed a name: run the action with it.
    CommitEdit {
        /// The pane.
        pane: PaneId,
        /// What the name is for.
        kind: EditKind,
        /// The name.
        name: String,
    },
    /// Navigate the pane to the folder holding `entry` and select it there,
    /// in this tab or a new one: Open containing folder.
    OpenContaining {
        /// The pane.
        pane: PaneId,
        /// The entry.
        entry: types::Entry,
        /// In a new tab.
        new_tab: bool,
    },
    /// Ask for a new name for this layout.
    RenameLayout(String),
    /// Close a window; the last one closes the application.
    CloseWindow(WindowId),
    /// The preview pane's *Browse…* for a missing third-party library:
    /// open the platform's file dialogue, store the chosen file into the
    /// previewer's setting and open the preview again.
    BrowseLibrary {
        /// The pane whose preview asked.
        pane: PaneId,
        /// The extension and the key of the setting that holds the path.
        setting: (types::ExtId, &'static str),
        /// What the dialogue filters for.
        filters: Vec<platform::DialogFilter>,
    },
    /// Open the side panel's Edit… dialogue for a place.
    EditPlace {
        /// The row's index.
        index: usize,
        /// Its title now.
        title: String,
        /// Its icon now.
        icon: String,
    },
    /// Open the side panel's *Add connection…* dialogue of a provider.
    AddConnection(types::ProviderId),
    /// Open the side panel's *Edit…* dialogue of a connection.
    EditConnection(types::ProviderId, String),
}

/// What a tab's widgets see.
pub struct PaneCx<'a> {
    /// The application, read only.
    pub app: &'a App,
    /// The pane.
    pub pane: PaneId,
    /// The tab.
    pub tab_id: TabId,
    /// The tab's model.
    pub tab: &'a Tab,
    /// The tab's view state.
    pub tab_ui: &'a mut TabUi,
    /// Where widgets leave their requests.
    pub requests: &'a mut Vec<Request>,
    /// The live shortcut table, for menu hints.
    pub keys: &'a KeyMap,
    /// This frame's time.
    pub now: SystemTime,
    /// Whether the pane has the focus.
    pub focused: bool,
    /// The icon view's thumbnails.
    pub thumbnails: &'a mut Thumbnails,
    /// The drag in progress and its drop zones.
    pub dnd: &'a mut Dnd,
}

/// One dispatched intent, for the palette's recents and for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dispatched {
    /// What.
    pub intent: Intent,
    /// On which pane.
    pub pane: PaneId,
}

struct Toast {
    level: Level,
    text: String,
    until: Instant,
}

/// A text an action asked the host to show.
struct TextWindow {
    serial: u64,
    title: String,
    text: String,
}

struct Dialog {
    ask: Ask,
    origin: DialogOrigin,
    input: String,
    focus: bool,
    /// A form's values as typed so far.
    values: ext::Values,
    /// A form's preview rows for `values`.
    rows: Vec<ext::PreviewRow>,
    /// A folder dialogue a Browse… button opened, for the field named.
    browse: Option<(
        &'static str,
        runtime::oneshot::Receiver<Option<std::path::PathBuf>>,
    )>,
}

/// Where a dialogue's answer goes.
enum DialogOrigin {
    /// The action runs again with the answer: the host's protocol for a
    /// question asked on the app thread, which cannot block a frame.
    Rerun { action: ActionId, ctx: ActionCtx },
    /// The rest of an action whose job finished with the form: the values
    /// go to the application, which runs it (`Command::Continue`).
    Continue {
        pane: PaneId,
        then: ext::Continuation,
    },
}

/// The file dialogue a preview's *Browse…* opened for a missing library:
/// the pane that asked, the setting the choice goes into, the answer.
struct LibraryBrowse {
    pane: PaneId,
    setting: (types::ExtId, &'static str),
    answer: runtime::oneshot::Receiver<Vec<std::path::PathBuf>>,
}

/// What a name typed into the name prompt is for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NameFor {
    /// Save the session as a layout of this name.
    SaveLayout,
    /// Rename this layout.
    RenameLayout(String),
}

/// The prompt for a layout name.
struct NamePrompt {
    what: NameFor,
    input: String,
    focus: bool,
}

/// The state of one window: its dock, search box and drag.
pub struct WindowUi {
    /// The dock tree.
    dock: DockSync,
    search: SearchBox,
    /// Where the Preview tab's content was in the last frame, when the
    /// window showed it.
    preview_rect: Option<egui::Rect>,
    /// The tab the pointer was over in the frame being drawn.
    hovered_tab: Option<TabId>,
    /// Each tab's header button in the last frame, for tests that drop on
    /// one.
    tab_headers: HashMap<TabId, egui::Rect>,
    /// Rick's own window frame as the last frame drew it, if it did; the
    /// window is then cleared to transparent around it.
    window_frame: Option<frame::Frame>,
    /// Drag and drop of entries.
    dnd: Dnd,
    /// The tab a drag out of a tab strip carried in the last frame.
    tab_drag: Option<TabId>,
    /// The content rectangle of the last frame, window coordinates.
    content: Option<egui::Rect>,
    /// The window's inner rectangle on the screen, from the last frame.
    screen: Option<egui::Rect>,
    /// Each pane's tab strip in the last frame, window coordinates.
    strips: Vec<(PaneId, egui::Rect)>,
    /// The restored geometry has been handed to the backend.
    placed: bool,
}

impl WindowUi {
    fn new(model: &model::Model, window: WindowId) -> Self {
        Self {
            dock: DockSync::new(model, window),
            search: SearchBox::default(),
            preview_rect: None,
            hovered_tab: None,
            tab_headers: HashMap::new(),
            window_frame: None,
            dnd: Dnd::default(),
            tab_drag: None,
            content: None,
            screen: None,
            strips: Vec::new(),
            placed: false,
        }
    }
}

/// The eframe application: the app and every window.
// The open flags are what they are: one per screen.
#[allow(clippy::struct_excessive_bools)]
pub struct RickUi {
    app: App,
    host: Arc<EguiHost>,
    keys: KeyMap,
    keys_from: std::collections::BTreeMap<String, String>,
    tabs: HashMap<TabId, TabUi>,
    requests: Vec<Request>,
    dispatched: VecDeque<Dispatched>,
    toasts: Vec<Toast>,
    dialog: Option<Dialog>,
    name_prompt: Option<NamePrompt>,
    settings_open: bool,
    shortcuts: ShortcutsScreen,
    extensions_open: bool,
    about_open: bool,
    /// What the About window shows as the version: the binary's version
    /// and git hash, or this crate's version until the binary says.
    version: String,
    /// The Preview tab's own state: pinned-panel search, what it drew. One
    /// preview, shown in every window that opens the tab.
    inspector_state: inspector::PaneState,
    /// The width the Preview tab's pane opens at, in points.
    inspector_width: f32,
    /// The Properties dialogue, open or not.
    properties: properties::PropertiesDialog,
    /// The file dialogue a preview's *Browse…* opened for a missing
    /// library, while it is open.
    library_browse: Option<LibraryBrowse>,
    /// The Hash dialogue, open or not.
    hash: hash::HashDialog,
    /// Read-only texts actions asked to show, each in its own window.
    texts: Vec<TextWindow>,
    next_text: u64,
    /// The icon view's thumbnails.
    thumbnails: Thumbnails,
    /// The egui context, for uploading textures outside a frame callback.
    ctx: egui::Context,
    previews: Previews,
    /// The views actions opened (a console, a comparison), by their tab.
    views: HashMap<TabId, ext::View>,
    /// Each console tab's own state.
    consoles: HashMap<TabId, ConsoleUi>,
    closed_tabs: Vec<Location>,
    /// The operations area's rows: rates and when each job ended.
    operations: Operations,
    /// The side panel's own state: the keyboard focus, a reorder, the
    /// Edit… dialogue.
    side: SidePanelState,
    /// The conflict dialogue's text field and checkbox.
    conflict: conflict::DialogState,
    /// The question dialogue's secret field and checkbox.
    question: question::DialogState,
    /// Set while the inline editor commits through the action, so the shell
    /// runs the action instead of starting another edit.
    inline_commit: bool,
    applied: Option<(Theme, u8, Density)>,
    now: SystemTime,
    /// Each model window's state.
    windows: BTreeMap<WindowId, WindowUi>,
    /// The model window the root viewport shows.
    root: WindowId,
    /// The application is closing: the root's close is not a window close.
    quitting: bool,
    /// The viewports open right now, for the waker.
    viewports: Arc<Mutex<Vec<ViewportId>>>,
    /// The root viewport's window as the backend describes it, for the
    /// platform's drag source and drop target; `None` without a backend
    /// window (the test harness).
    window_ref: Option<WindowRef>,
    /// The platform has been told about `window_ref`.
    attached: bool,
}

/// How many dispatched intents are remembered.
const RECENTS: usize = 64;
/// The title row's height in points: that of a tab.
const TITLE_ROW_HEIGHT: f32 = 30.0;
/// A new window's inner size before the user resizes it.
const DEFAULT_SIZE: [f32; 2] = [1100.0, 720.0];
/// The smallest inner size.
const MIN_SIZE: [f32; 2] = [480.0, 320.0];

/// The tab strip's look: VS Code-like padding, rounded top corners, the
/// new-tab button right after the last tab, a scroll bar on overflow; the
/// strip on the window's background and the leaf on the pane's.
fn tab_strip_style(egui_style: &egui::Style, scheme: &crate::theme::Scheme) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(egui_style);
    style.tab_bar.bg_fill = scheme.bg_window;
    style.tab.tab_body.bg_fill = scheme.bg_pane;
    style.tab.tab_body.stroke = egui::Stroke::NONE;
    style.buttons.add_tab_align = egui_dock::TabAddAlign::Left;
    style.tab_bar.height = TITLE_ROW_HEIGHT;
    style.tab_bar.show_scroll_bar_on_overflow = true;
    style.tab_bar.fill_tab_bar = false;
    style.tab.spacing = 2.0;
    style.tab.minimum_width = Some(96.0);
    style.tab.hline_below_active_tab_name = false;
    let top = egui::CornerRadius {
        nw: 4,
        ne: 4,
        sw: 0,
        se: 0,
    };
    for interaction in [
        &mut style.tab.active,
        &mut style.tab.inactive,
        &mut style.tab.focused,
        &mut style.tab.hovered,
        &mut style.tab.inactive_with_kb_focus,
        &mut style.tab.active_with_kb_focus,
        &mut style.tab.focused_with_kb_focus,
    ] {
        interaction.corner_radius = top;
    }
    style
}

/// An icon-only toggle; the fill weight marks it active.
fn toggle_icon(glyph: &str, on: bool) -> egui::Button<'static> {
    let text = if on {
        egui::RichText::new(glyph).family(egui::FontFamily::Name(t!("shell-phosphor-fill").into()))
    } else {
        egui::RichText::new(glyph)
    };
    egui::Button::new(text).frame(false).selected(on)
}
/// How long a notification stays.
const TOAST_FOR: Duration = Duration::from_secs(5);
/// How long a `SelectNamed` waits for its entry to be listed.
const SELECT_FOR: Duration = Duration::from_secs(5);

/// The viewport of a model window other than the root's.
fn viewport_of(window: WindowId) -> ViewportId {
    ViewportId::from_hash_of(("rick-window", window.0))
}

/// The viewport builder for a window: Rick's own chrome, at the window's
/// restored geometry when it has one.
#[must_use]
pub fn viewport_builder(
    title: &str,
    chrome: platform::WindowChrome,
    geometry: Option<Geometry>,
) -> ViewportBuilder {
    let mut viewport = ViewportBuilder::default()
        .with_title(title)
        .with_min_inner_size(MIN_SIZE);
    viewport = match geometry {
        Some(g) => viewport
            .with_position([g.x, g.y])
            .with_inner_size([g.width.max(MIN_SIZE[0]), g.height.max(MIN_SIZE[1])])
            .with_maximized(g.maximised),
        None => viewport.with_inner_size(DEFAULT_SIZE),
    };
    // Without decorations Rick draws the frame itself (`frame`): the window
    // is transparent so the margin around the content shows the shadow.
    if chrome.native_controls {
        viewport
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_fullsize_content_view(true)
    } else {
        viewport.with_decorations(false).with_transparent(true)
    }
}

impl RickUi {
    /// The version the About window shows, as the binary reports it
    /// (`rick --version`).
    #[must_use]
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Wraps `app`; `host` is the `UiHost` the app was built with.
    #[must_use]
    pub fn new(app: App, host: Arc<EguiHost>, ctx: &egui::Context) -> Self {
        crate::fonts::install(ctx, &app.platform().system_fonts());
        let viewports: Arc<Mutex<Vec<ViewportId>>> = Arc::new(Mutex::new(vec![ViewportId::ROOT]));
        let wake = ctx.clone();
        let open = Arc::clone(&viewports);
        app.set_waker(Arc::new(move || {
            let ids = open.lock().unwrap_or_else(PoisonError::into_inner).clone();
            for id in ids {
                wake.request_repaint_of(id);
            }
        }));
        let keys = KeyMap::new(app.registry(), &app.model().settings);
        let keys_from = app.model().settings.shortcuts.clone();
        let root = app.model().focused_window;
        let inspector_width = 260.0;
        let mut windows = BTreeMap::new();
        for (id, _) in app.model().windows() {
            windows.insert(id, WindowUi::new(app.model(), id));
        }
        Self {
            app,
            host,
            keys,
            keys_from,
            tabs: HashMap::new(),
            requests: Vec::new(),
            dispatched: VecDeque::new(),
            toasts: Vec::new(),
            dialog: None,
            name_prompt: None,
            settings_open: false,
            shortcuts: ShortcutsScreen::default(),
            extensions_open: false,
            about_open: false,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            inspector_state: inspector::PaneState::default(),
            inspector_width,
            properties: properties::PropertiesDialog::default(),
            library_browse: None,
            hash: hash::HashDialog::default(),
            texts: Vec::new(),
            next_text: 0,
            thumbnails: Thumbnails::default(),
            ctx: ctx.clone(),
            previews: Previews::default(),
            views: HashMap::new(),
            consoles: HashMap::new(),
            closed_tabs: Vec::new(),
            operations: Operations::default(),
            side: SidePanelState::default(),
            conflict: conflict::DialogState::default(),
            question: question::DialogState::default(),
            inline_commit: false,
            applied: None,
            now: SystemTime::now(),
            windows,
            root,
            quitting: false,
            window_ref: None,
            attached: false,
            viewports,
        }
    }

    /// The application, for reading the model.
    #[must_use]
    pub fn app(&self) -> &App {
        &self.app
    }

    /// The application, for tests that drive it directly.
    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    /// The live shortcut table.
    #[must_use]
    pub fn keys(&self) -> &KeyMap {
        &self.keys
    }

    /// The intents dispatched so far, most recent last.
    #[must_use]
    pub fn dispatched(&self) -> &VecDeque<Dispatched> {
        &self.dispatched
    }

    /// Forgets the dispatched intents.
    pub fn clear_dispatched(&mut self) {
        self.dispatched.clear();
    }

    /// The model windows shown right now, the root's first.
    #[must_use]
    pub fn window_ids(&self) -> Vec<WindowId> {
        let mut ids = vec![self.root];
        ids.extend(self.windows.keys().copied().filter(|id| *id != self.root));
        ids
    }

    /// Whether the root window has its Preview tab open.
    #[must_use]
    pub fn inspector_open(&self) -> bool {
        self.app.model().preview_tab(self.root).is_some()
    }

    /// The width the Preview tab's pane opens at; for tests, before it
    /// opens.
    pub fn set_inspector_width(&mut self, width: f32) {
        self.inspector_width = width;
    }

    /// Where the root window's Preview tab drew its content in the last
    /// frame, if the tab is open and shown.
    #[must_use]
    pub fn inspector_rect(&self) -> Option<egui::Rect> {
        self.windows.get(&self.root).and_then(|w| w.preview_rect)
    }

    /// Where the preview pane's parts were in the last frame, for tests.
    #[must_use]
    /// The Properties dialogue, for the tests.
    pub fn properties(&self) -> &properties::PropertiesDialog {
        &self.properties
    }

    /// The Hash dialogue, for the tests.
    #[must_use]
    pub fn hash_dialog(&self) -> &hash::HashDialog {
        &self.hash
    }

    #[must_use]
    pub fn preview_rects(&self) -> inspector::PaneRects {
        self.inspector_state.rects
    }

    /// The preview pane's toolbar controls of the last frame, for tests.
    #[must_use]
    pub fn preview_toolbar(&self) -> &[inspector::ToolbarItem] {
        &self.inspector_state.toolbar
    }

    /// The preview pane's badges of the last frame, for tests.
    #[must_use]
    pub fn preview_badges(&self) -> &[inspector::BadgeItem] {
        &self.inspector_state.badges
    }

    /// The preview pane's status bar text of the last frame, for tests.
    #[must_use]
    pub fn preview_status(&self) -> &str {
        &self.inspector_state.status
    }

    /// Whether the root window's title-row search box has the keyboard.
    #[must_use]
    pub fn search_active(&self) -> bool {
        self.windows
            .get(&self.root)
            .is_some_and(|w| w.search.active())
    }

    /// The root window's search box's text.
    #[must_use]
    pub fn search_text(&self) -> &str {
        self.windows
            .get(&self.root)
            .map_or("", |w| w.search.text.as_str())
    }

    /// Whether the Settings screen is open.
    #[must_use]
    pub fn settings_open(&self) -> bool {
        self.settings_open
    }

    /// Opens the Settings window, as the menu does; for the tests.
    pub fn open_settings(&mut self) {
        self.settings_open = true;
    }

    /// Whether the Keyboard shortcuts screen is open.
    #[must_use]
    pub fn shortcuts_open(&self) -> bool {
        self.shortcuts.open
    }

    /// Whether the Extensions screen is open.
    #[must_use]
    pub fn extensions_open(&self) -> bool {
        self.extensions_open
    }

    /// The question a dialogue is showing, if any.
    #[must_use]
    pub fn asking(&self) -> Option<&Ask> {
        self.dialog.as_ref().map(|d| &d.ask)
    }

    /// The inline edit in progress in a tab, for tests.
    #[must_use]
    pub fn editing(&self, tab: TabId) -> Option<&RowEdit> {
        self.tabs.get(&tab).and_then(|t| t.edit.as_ref())
    }

    /// The side panel's state, for tests.
    #[must_use]
    pub fn side_panel_state(&self) -> &SidePanelState {
        &self.side
    }

    /// The operations area, for tests: how often it was drawn.
    #[must_use]
    pub fn operations_paints(&self) -> u64 {
        self.operations.paints()
    }

    /// Whether the layout name prompt is up, for tests.
    #[must_use]
    pub fn name_prompt_open(&self) -> bool {
        self.name_prompt.is_some()
    }

    /// The notifications currently shown.
    #[must_use]
    pub fn toasts(&self) -> Vec<(Level, String)> {
        self.toasts
            .iter()
            .map(|t| (t.level, t.text.clone()))
            .collect()
    }

    /// The tab order of a pane as the dock shows it.
    #[must_use]
    pub fn tab_order(&self, pane: PaneId) -> Vec<TabId> {
        self.order_of(pane)
    }

    /// Queues an intent as a key press would.
    pub fn queue(&mut self, intent: Intent, pane: PaneId) {
        self.requests.push(Request::Intent { intent, pane });
    }

    /// The icon view's thumbnail cache, for tests.
    #[must_use]
    pub fn thumbnails(&self) -> &Thumbnails {
        &self.thumbnails
    }

    /// The toolbar rectangles of a tab in the last frame, by group, for
    /// tests that check the layout.
    #[must_use]
    pub fn toolbar_rects(&self, tab: TabId) -> Vec<(&'static str, egui::Rect)> {
        self.tabs
            .get(&tab)
            .map(|t| t.toolbar_rects.clone())
            .unwrap_or_default()
    }

    /// The toolbar row of a tab in the last frame, for tests.
    #[must_use]
    pub fn toolbar_row(&self, tab: TabId) -> Option<egui::Rect> {
        self.tabs.get(&tab).and_then(|t| t.toolbar_row)
    }

    /// The content area inside Rick's own window frame in the root window's
    /// last frame, `None` when the system draws the frame or the window is
    /// maximised.
    #[must_use]
    pub fn framed_content(&self) -> Option<egui::Rect> {
        self.windows
            .get(&self.root)
            .and_then(|w| w.window_frame.map(|f| f.content()))
    }

    /// Each pane's rectangle in the root window's last frame, for tests
    /// that drag tabs.
    #[must_use]
    pub fn pane_rects(&self) -> Vec<(PaneId, egui::Rect)> {
        self.windows
            .get(&self.root)
            .map(|w| w.dock.pane_rects(self.app.model()))
            .unwrap_or_default()
    }

    /// The texts shown right now, as (title, text), for tests.
    #[must_use]
    pub fn shown_texts(&self) -> Vec<(String, String)> {
        self.texts
            .iter()
            .map(|w| (w.title.clone(), w.text.clone()))
            .collect()
    }

    /// The drag in progress in the root window, for tests.
    #[must_use]
    pub fn drag(&self) -> Option<DragView> {
        self.windows.get(&self.root).and_then(|w| w.dnd.view())
    }

    /// Whether the platform runs a drag the root window handed it, for tests.
    #[must_use]
    pub fn native_drag_running(&self) -> bool {
        self.windows
            .get(&self.root)
            .is_some_and(|w| w.dnd.native_drag_running())
    }

    /// The root viewport's window as the backend describes it. eframe's
    /// frame supplies it each frame; the test harness has no backend window
    /// and sets a stand-in here so the platform's drag source and drop
    /// target are consulted as they are in a real window.
    pub fn set_window_ref(&mut self, window: Option<WindowRef>) {
        self.window_ref = window;
        self.attached = false;
    }

    /// Keeps `window_ref` in step with the backend and tells the platform
    /// about the window once it exists.
    fn sync_window_ref(&mut self, frame: &eframe::Frame, ctx: &egui::Context) {
        if let (Ok(window), Ok(display)) = (frame.window_handle(), frame.display_handle()) {
            self.window_ref = Some(WindowRef {
                window: window.as_raw(),
                display: display.as_raw(),
                scale: ctx.native_pixels_per_point().unwrap_or(1.0),
            });
        }
        if !self.attached
            && let Some(window) = &self.window_ref
        {
            let wake = ctx.clone();
            self.app
                .platform()
                .attach(window, Arc::new(move || wake.request_repaint()));
            self.attached = true;
        }
    }

    /// Whether the move/copy/cancel prompt of a drop is up, for tests.
    #[must_use]
    pub fn drop_prompt_open(&self) -> bool {
        self.windows
            .get(&self.root)
            .is_some_and(|w| w.dnd.prompt_open())
    }

    /// A tab's header button in the last frame, for tests.
    #[must_use]
    pub fn tab_header_rect(&self, tab: TabId) -> Option<egui::Rect> {
        self.windows
            .values()
            .find_map(|w| w.tab_headers.get(&tab).copied())
    }

    /// A tab's list scroll offset in the last frame, for tests.
    #[must_use]
    pub fn scroll_offset(&self, tab: TabId) -> f32 {
        self.tabs.get(&tab).map_or(0.0, |t| t.scroll_offset)
    }

    /// The previews, for tests.
    #[must_use]
    pub fn previews(&self) -> &Previews {
        &self.previews
    }

    /// Whether the application is closing.
    #[must_use]
    pub fn quitting(&self) -> bool {
        self.quitting
    }

    /// The state of the window holding `pane`.
    fn window_ui_of(&self, pane: PaneId) -> Option<&WindowUi> {
        let window = self.app.model().window_of(pane)?;
        self.windows.get(&window)
    }

    fn window_ui_of_mut(&mut self, pane: PaneId) -> Option<&mut WindowUi> {
        let window = self.app.model().window_of(pane)?;
        self.windows.get_mut(&window)
    }

    fn toast(&mut self, level: Level, text: impl Into<String>) {
        self.toasts.push(Toast {
            level,
            text: text.into(),
            until: Instant::now() + TOAST_FOR,
        });
    }

    fn absorb(&mut self, events: Vec<Event>) {
        for event in events {
            match event {
                Event::Thumbnail { id, size, frame } => {
                    self.thumbnails.arrived(&self.ctx, id, size, frame);
                }
                Event::Notify { level, text } => self.toast(level, text),
                Event::Properties(info) => self.properties.open(*info),
                Event::HashesOpened { hashes, format, .. } => self.hash.open(hashes, format),
                Event::PreviewReady { pane, doc } => {
                    let entry = self.preview_entry(pane);
                    self.previews.ready(pane, doc, entry);
                }
                Event::PreviewFailed { pane, error } => {
                    let entry = self.preview_entry(pane);
                    self.previews.failed(pane, &error, entry);
                }
                Event::PreviewNone { pane, plugin } => {
                    let entry = self.preview_entry(pane);
                    self.previews.none(pane, plugin, entry);
                }
                Event::PreviewPrefix {
                    pane,
                    served,
                    total,
                } => self.previews.prefix(pane, served, total),
                Event::PreviewCapped {
                    pane,
                    size,
                    cap,
                    source,
                } => {
                    let entry = self.preview_entry(pane);
                    self.previews.capped(pane, size, cap, source, entry);
                }
                Event::ViewOpened { tab, view, .. } => {
                    if let ext::View::Console(console) = &view {
                        let ctx = self.ctx.clone();
                        console.set_waker(Arc::new(move || ctx.request_repaint()));
                        self.consoles.entry(tab).or_default().focus_next = true;
                    }
                    self.views.insert(tab, view);
                }
                _ => {}
            }
        }
    }

    /// Draws the text windows and drops the closed ones.
    fn show_texts(&mut self, ctx: &egui::Context) {
        self.texts.retain(|window| {
            let mut open = true;
            egui::Window::new(&window.title)
                .id(egui::Id::new(("text", window.serial)))
                .open(&mut open)
                .default_size([640.0, 420.0])
                .show(ctx, |ui| {
                    egui::ScrollArea::both()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(egui::RichText::new(&window.text).monospace())
                                    .wrap_mode(egui::TextWrapMode::Extend)
                                    .selectable(true),
                            );
                        });
                });
            open
        });
    }

    /// The entry the app is previewing in `pane`, if any: the Preview tab's.
    fn preview_entry(&self, pane: PaneId) -> Option<EntryId> {
        self.app.preview_entry(pane)
    }

    fn handle(&mut self, command: Command) {
        let mut events = Vec::new();
        self.app.handle(command, &mut events);
        self.absorb(events);
    }

    fn apply_settings(&mut self, ctx: &egui::Context) {
        let settings = &self.app.model().settings;
        let key = (settings.theme, settings.font_size, settings.density);
        if self.applied != Some(key) {
            ctx.set_theme(match settings.theme {
                Theme::System => egui::ThemePreference::System,
                Theme::Light => egui::ThemePreference::Light,
                Theme::Dark => egui::ThemePreference::Dark,
            });
            ctx.set_zoom_factor(f32::from(settings.font_size) / crate::theme::TEXT_BODY);
            // The scheme and density tokens on both of egui's themes, so
            // following the system keeps working.
            crate::theme::apply(ctx, settings.density);
            self.applied = Some(key);
        }
        if settings.shortcuts != self.keys_from {
            self.keys = KeyMap::new(self.app.registry(), settings);
            self.keys_from = settings.shortcuts.clone();
        }
    }

    /// The pane keyboard input goes to in `window`.
    fn focused_in(&self, window: WindowId) -> PaneId {
        self.app
            .model()
            .window(window)
            .map_or(self.app.model().focused, |w| w.focused)
    }

    /// The pane an intent about files goes to: `pane` itself, or, when its
    /// active tab is the Preview tab, the pane of the explorer the preview
    /// follows. The dock never focuses a preview pane on its own, but a
    /// Preview tab dragged into an explorer pane can be the active one.
    fn explorer_pane(&self, pane: PaneId) -> PaneId {
        if crate::dock::shows_preview(self.app.model(), pane) {
            self.app.preview_pane().unwrap_or(pane)
        } else {
            pane
        }
    }

    /// Where an intent lands: the tab-strip intents stay with the pane
    /// they were pressed in, everything else follows
    /// [`explorer_pane`](Self::explorer_pane).
    fn route(&self, intent: Intent, pane: PaneId) -> PaneId {
        match intent {
            Intent::CloseTab
            | Intent::CloseOtherTabs
            | Intent::NextTab
            | Intent::PrevTab
            | Intent::Tab(_)
            | Intent::MoveTabLeft
            | Intent::MoveTabRight
            | Intent::FocusPane(_) => pane,
            _ => self.explorer_pane(pane),
        }
    }

    /// The share of the dock the existing panes keep when the Preview tab
    /// opens, so that its pane is [`inspector_width`](Self::inspector_width)
    /// wide: the dock puts the divider at the fraction and gives each side
    /// half of its width.
    fn preview_fraction(&self, window: WindowId) -> f32 {
        let Some(w) = self.windows.get(&window) else {
            return 0.72;
        };
        let rects = w.dock.pane_rects(self.app.model());
        let left = rects
            .iter()
            .map(|(_, r)| r.left())
            .fold(f32::INFINITY, f32::min);
        let right = rects
            .iter()
            .map(|(_, r)| r.right())
            .fold(f32::NEG_INFINITY, f32::max);
        let width = right - left;
        if !width.is_finite() || width <= 0.0 {
            return 0.72;
        }
        (1.0 - (self.inspector_width + 0.5) / width).clamp(0.2, 0.9)
    }

    /// Turns this frame's key presses into intents, unless a text field, a
    /// dialogue, the palette or the shortcuts screen wants them.
    fn handle_keys(&mut self, ctx: &egui::Context, window: WindowId) {
        let pane = self.explorer_pane(self.focused_in(window));
        if self.dialog.is_some()
            || self.name_prompt.is_some()
            || self.shortcuts.rebinding.is_some()
            || !self.app.model().conflicts.is_empty()
            || !self.app.model().questions.is_empty()
        {
            return;
        }
        // An open menu owns the keyboard: Escape closes it, nothing else —
        // not even the side panel, whose row the menu may belong to.
        if egui::Popup::is_any_open(ctx) {
            return;
        }
        if self.side.focused() && self.side.handle_keys(ctx, &self.app, &mut self.requests) {
            return;
        }
        let search_active = self.windows.get(&window).is_some_and(|w| w.search.active());
        if search_active || ctx.egui_wants_keyboard_input() {
            return;
        }
        let filter_text = self
            .app
            .model()
            .active_tab(pane)
            .map(|(_, tab)| tab.filter.clone())
            .unwrap_or_default();
        // The Preview tab has the focus: its own table (Space plays) comes
        // first.
        let in_preview = crate::dock::shows_preview(self.app.model(), self.focused_in(window));
        let mut intents = Vec::new();
        let mut typed = String::new();
        let mut backspace = false;
        ctx.input_mut(|input| {
            input.events.retain(|event| match event {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if *key == egui::Key::Backspace
                        && modifiers.is_none()
                        && !filter_text.is_empty()
                    {
                        backspace = true;
                        return false;
                    }
                    match self
                        .keys
                        .match_key(*modifiers, *key, *physical_key, in_preview)
                    {
                        Some(intent) => {
                            intents.push(intent);
                            false
                        }
                        None => true,
                    }
                }
                // Type-to-filter.
                egui::Event::Text(text)
                    if text.chars().all(|c| !c.is_control() && !c.is_whitespace()) =>
                {
                    typed.push_str(text);
                    false
                }
                _ => true,
            });
        });
        if backspace {
            let mut text = filter_text.clone();
            text.pop();
            self.requests
                .push(Request::Command(Command::Filter { pane, text }));
        }
        if !typed.is_empty() {
            let text = format!("{filter_text}{typed}");
            self.requests
                .push(Request::Command(Command::Filter { pane, text }));
        }
        let strip = self.focused_in(window);
        for intent in intents {
            // The tab-strip intents act on the pane they were pressed in.
            let pane = if self.route(intent, strip) == strip {
                strip
            } else {
                pane
            };
            self.requests.push(Request::Intent { intent, pane });
        }
    }

    fn apply_requests(&mut self, ctx: &egui::Context) {
        let mut guard = 0;
        while !self.requests.is_empty() && guard < 16 {
            guard += 1;
            for request in std::mem::take(&mut self.requests) {
                match request {
                    Request::Command(command) => self.handle(command),
                    Request::Intent { intent, pane } => self.dispatch(intent, pane, ctx),
                    Request::OpenEntry { pane, id } => self.open_entry(pane, id),
                    Request::RunAction { action, ctx } => self.run_action_with(action, ctx),
                    Request::NewTabAt(location) => self.handle(Command::NewTab(location)),
                    Request::FocusPane(pane) => self.handle(Command::FocusPane(pane)),
                    Request::Toast(level, text) => self.toast(level, text),
                    Request::About => self.about_open = true,
                    Request::StartDrag {
                        pane,
                        tab,
                        items,
                        right_button,
                    } => self.start_drag(pane, tab, &items, right_button),
                    Request::SelectNamed { pane, name } => {
                        if let Some((tab_id, _)) = self.app.model().active_tab(pane) {
                            self.tab_ui(tab_id).pending_select = Some((name, Instant::now()));
                        }
                    }
                    Request::StartEdit { pane, kind } => self.start_edit(pane, kind),
                    Request::CommitEdit { pane, kind, name } => {
                        self.commit_edit(pane, kind, &name);
                    }
                    Request::OpenContaining {
                        pane,
                        entry,
                        new_tab,
                    } => self.open_containing(pane, &entry, new_tab),
                    Request::RenameLayout(name) => {
                        self.name_prompt = Some(NamePrompt {
                            input: name.clone(),
                            what: NameFor::RenameLayout(name),
                            focus: true,
                        });
                    }
                    Request::CloseWindow(window) => self.close_window(window, ctx),
                    Request::BrowseLibrary {
                        pane,
                        setting,
                        filters,
                    } => self.browse_library(pane, setting, filters),
                    Request::EditPlace { index, title, icon } => {
                        side_panel::begin_edit(&mut self.side, index, title, icon);
                    }
                    Request::AddConnection(provider) => {
                        side_panel::begin_connection(&mut self.side, &self.app, provider);
                    }
                    Request::EditConnection(provider, name) => {
                        side_panel::begin_edit_connection(
                            &mut self.side,
                            &self.app,
                            provider,
                            &name,
                        );
                    }
                }
            }
        }
    }

    /// Gives every tab of `window` its view state before anything else in
    /// the frame makes a default one, so a restored scroll offset is kept
    /// for the listing.
    fn adopt_tabs(&mut self, window: WindowId) {
        let model = self.app.model();
        let Some(w) = model.window(window) else {
            return;
        };
        for pane in w.dock.panes() {
            let Some(pane) = model.pane(pane) else {
                continue;
            };
            for tab in pane.tabs() {
                if let Some(state) = model.tab(*tab) {
                    self.tabs
                        .entry(*tab)
                        .or_insert_with(|| TabUi::for_tab(state));
                }
            }
        }
    }

    /// The view state of a tab, made from the model's tab the first time.
    fn tab_ui(&mut self, tab: TabId) -> &mut TabUi {
        let model_tab = self.app.model().tab(tab);
        self.tabs
            .entry(tab)
            .or_insert_with(|| model_tab.map_or_else(TabUi::default, TabUi::for_tab))
    }

    /// Closes a window: the last one closes the application, with the
    /// session saved by the exit hook.
    fn close_window(&mut self, window: WindowId, ctx: &egui::Context) {
        if self.app.model().windows().count() < 2 {
            self.quit(ctx);
            return;
        }
        self.handle(Command::CloseWindow(window));
    }

    /// Closes the application.
    fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
    }

    /// Starts a drag of `items` out of `pane`'s tab.
    fn start_drag(&mut self, pane: PaneId, tab: TabId, items: &[EntryId], right_button: bool) {
        let Some(state) = self.app.model().tab(tab) else {
            return;
        };
        let entries: Vec<types::Entry> = state
            .rows()
            .filter(|e| items.contains(&e.id))
            .cloned()
            .collect();
        if let Some(window) = self.window_ui_of_mut(pane) {
            window.dnd.start(
                Source::Internal {
                    from: pane,
                    tab,
                    items: entries,
                },
                right_button,
            );
        }
    }

    /// Selects the entries a `SelectNamed` asked for once they are listed: a
    /// pinned file's folder was opened and the file is in the list, a new
    /// folder's job finished and the refresh brought it, Open containing
    /// folder reached the parent. A name that never shows is dropped after
    /// [`SELECT_FOR`].
    fn resolve_pending_selects(&mut self) {
        let now = Instant::now();
        let mut found = Vec::new();
        for (tab_id, tab_ui) in &self.tabs {
            let Some((name, since)) = &tab_ui.pending_select else {
                continue;
            };
            let Some(tab) = self.app.model().tab(*tab_id) else {
                continue;
            };
            let hit = tab
                .rows()
                .position(|e| e.path.file_name() == Some(name.as_slice()))
                .and_then(|index| tab.row(index).map(|e| (index, e.id)));
            if hit.is_none() && now.duration_since(*since) < SELECT_FOR {
                continue;
            }
            found.push((*tab_id, self.app.model().pane_of(*tab_id), hit));
        }
        for (tab_id, pane, hit) in found {
            let Some(tab_ui) = self.tabs.get_mut(&tab_id) else {
                continue;
            };
            tab_ui.pending_select = None;
            if let (Some(pane), Some((index, id))) = (pane, hit) {
                tab_ui.scroll_to = Some(index);
                tab_ui.cursor = Some(id);
                tab_ui.anchor = Some(id);
                self.requests.push(Request::Command(Command::Select {
                    pane,
                    change: SelectionChange::Set(vec![id]),
                }));
            }
        }
    }

    /// Reports the scroll offsets of `window`'s tabs to the app, once their
    /// listings are complete, so the session keeps them.
    fn report_scroll(&mut self, window: WindowId) {
        let model = self.app.model();
        let Some(w) = model.window(window) else {
            return;
        };
        let mut changed = Vec::new();
        for pane in w.dock.panes() {
            let Some((tab_id, tab)) = model.active_tab(pane) else {
                continue;
            };
            let Some(tab_ui) = self.tabs.get(&tab_id) else {
                continue;
            };
            if !tab.listing.complete
                || tab_ui.restore_scroll.is_some()
                || tab_ui.pending_offset.is_some()
                || tab_ui.last_location.as_ref() != Some(&tab.location)
            {
                continue;
            }
            if (tab_ui.scroll_offset - tab.scroll).abs() > 0.5 {
                changed.push((tab_id, tab_ui.scroll_offset));
            }
        }
        for (tab, offset) in changed {
            self.handle(Command::Scroll { tab, offset });
        }
    }

    /// Reports the window's place on the screen, focus and close requests
    /// to the app, and hands a restored geometry to the backend once.
    fn report_viewport(&mut self, ctx: &egui::Context, window: WindowId) {
        let info = ctx.input(|i| i.viewport().clone());
        let is_root = window == self.root;
        let model_geometry = self.app.model().window(window).and_then(|w| w.geometry);
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        w.screen = info.inner_rect;
        if !w.placed {
            w.placed = true;
            if is_root && let Some(g) = model_geometry {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(g.x, g.y)));
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(
                    g.width.max(MIN_SIZE[0]),
                    g.height.max(MIN_SIZE[1]),
                )));
                if g.maximised {
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(true));
                }
            }
            return;
        }
        if info.focused == Some(true) && self.app.model().focused_window != window {
            self.handle(Command::FocusWindow(window));
        }
        if info.minimized != Some(true)
            && let (Some(outer), Some(inner)) = (info.outer_rect, info.inner_rect)
        {
            let maximised = info.maximized.unwrap_or(false) || info.fullscreen.unwrap_or(false);
            let geometry = if maximised {
                // Keep the restored size for when it is restored.
                Geometry {
                    maximised: true,
                    ..model_geometry.unwrap_or(Geometry {
                        x: outer.min.x,
                        y: outer.min.y,
                        width: inner.width(),
                        height: inner.height(),
                        maximised: true,
                    })
                }
            } else {
                Geometry {
                    x: outer.min.x,
                    y: outer.min.y,
                    width: inner.width(),
                    height: inner.height(),
                    maximised: false,
                }
            };
            let same = model_geometry.is_some_and(|g| {
                (g.x - geometry.x).abs() < 0.5
                    && (g.y - geometry.y).abs() < 0.5
                    && (g.width - geometry.width).abs() < 0.5
                    && (g.height - geometry.height).abs() < 0.5
                    && g.maximised == geometry.maximised
            });
            if !same && geometry.width > 0.0 && geometry.height > 0.0 {
                self.handle(Command::WindowGeometry { window, geometry });
            }
        }
        if info.close_requested() {
            let windows = self.app.model().windows().count();
            if is_root {
                if !self.quitting && windows > 1 {
                    // The root closes alone: another window takes over
                    // its viewport.
                    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                    self.handle(Command::CloseWindow(window));
                } else {
                    self.quitting = true;
                }
            } else {
                self.handle(Command::CloseWindow(window));
            }
        }
    }

    fn run_action(&mut self, action: ActionId, pane: PaneId, invoked_from: Invocation) {
        if let Some(ctx) = self.app.action_ctx(pane, invoked_from) {
            // In the Trash view Enter restores through the trash plugin's
            // action; the Delete key deletes for good through the view's
            // own `remove`, as the core action does everywhere.
            let action = if vfs_rick::is_trash(&ctx.location) && action.0 == "core.open_default" {
                ActionId("trash.restore_items")
            } else {
                action
            };
            self.run_action_with(action, ctx);
        }
    }

    /// Begins editing a name in the pane's list.
    fn start_edit(&mut self, pane: PaneId, kind: EditKind) {
        let Some((tab_id, tab)) = self.app.model().active_tab(pane) else {
            return;
        };
        let text = match kind {
            EditKind::Rename(id) => match tab.entry(id) {
                Some(entry) => entry.name.clone(),
                None => return,
            },
            EditKind::NewFile => "New file.txt".to_owned(),
            EditKind::NewFolder => "New folder".to_owned(),
        };
        let tab_ui = self.tabs.entry(tab_id).or_default();
        tab_ui.edit = Some(RowEdit::new(kind, text));
        if !matches!(kind, EditKind::Rename(_)) {
            tab_ui.scroll_to = Some(0);
        }
    }

    /// Runs the action behind an inline edit with the name pre-answered,
    /// then selects the result once it is listed.
    fn commit_edit(&mut self, pane: PaneId, kind: EditKind, name: &str) {
        let Some((_, tab)) = self.app.model().active_tab(pane) else {
            return;
        };
        let (action, selection) = match kind {
            EditKind::Rename(id) => match tab.entry(id) {
                Some(entry) => (ActionId("core.rename"), vec![entry.clone()]),
                None => return,
            },
            EditKind::NewFile => (ActionId("core.new_file"), Vec::new()),
            EditKind::NewFolder => (ActionId("core.new_folder"), Vec::new()),
        };
        let ctx = ActionCtx {
            pane,
            location: tab.location.clone(),
            selection,
            invoked_from: Invocation::Shortcut,
            other: None,
        };
        self.host.answer_prompt(name.to_owned());
        self.inline_commit = true;
        self.run_action_with(action, ctx);
        self.inline_commit = false;
        self.host.clear_answers();
        self.requests.push(Request::SelectNamed {
            pane,
            name: name.as_bytes().to_vec(),
        });
    }

    /// Open containing folder: the pane, or a new tab, goes to the entry's
    /// folder and selects the entry there.
    fn open_containing(&mut self, pane: PaneId, entry: &types::Entry, new_tab: bool) {
        let Some(parent) = entry.path.parent() else {
            return;
        };
        let Some(name) = entry.path.file_name() else {
            return;
        };
        let to = Location::new(entry.id.provider, parent);
        if new_tab {
            self.handle(Command::NewTab(to));
        } else {
            self.handle(Command::Navigate { pane, to });
        }
        let pane = self.app.model().focused;
        self.requests.push(Request::SelectNamed {
            pane,
            name: name.to_vec(),
        });
    }

    /// Whether the provider at `location` takes new entries: the inline
    /// edits start only where New file and New folder can finish.
    fn writable(&self, location: &Location) -> bool {
        self.app
            .mounts()
            .resolve(location)
            .is_ok_and(|p| p.capabilities(location).contains(ext::Capabilities::WRITE))
    }

    /// Whether the shell handles an action itself rather than running it:
    /// the three inline edits (QUESTIONS.md 26) and Open containing folder
    /// (QUESTIONS.md 19).
    fn intercept(&mut self, action: ActionId, ctx: &ActionCtx) -> bool {
        match action.0 {
            // Rename with several selected is the bulk rename, when the
            // rename plugin is loaded.
            "core.rename" if ctx.selection.len() > 1 && self.has_action("rename.bulk") => {
                self.run_action_with(ActionId("rename.bulk"), ctx.clone());
                true
            }
            "core.rename" if !self.inline_commit && ctx.selection.len() == 1 => {
                self.requests.push(Request::StartEdit {
                    pane: ctx.pane,
                    kind: EditKind::Rename(ctx.selection[0].id),
                });
                true
            }
            "core.new_file" if !self.inline_commit && self.writable(&ctx.location) => {
                self.requests.push(Request::StartEdit {
                    pane: ctx.pane,
                    kind: EditKind::NewFile,
                });
                true
            }
            "core.new_folder" if !self.inline_commit && self.writable(&ctx.location) => {
                self.requests.push(Request::StartEdit {
                    pane: ctx.pane,
                    kind: EditKind::NewFolder,
                });
                true
            }
            "core.open_containing" | "core.open_containing_tab" => {
                let Some(entry) = ctx.selection.first().cloned() else {
                    return false;
                };
                if entry.path.parent() == Some(ctx.location.path.clone()) {
                    return false;
                }
                self.requests.push(Request::OpenContaining {
                    pane: ctx.pane,
                    entry,
                    new_tab: action.0 == "core.open_containing_tab",
                });
                true
            }
            _ => false,
        }
    }

    /// Whether the registry holds an action with this id.
    fn has_action(&self, id: &str) -> bool {
        self.app.registry().actions().iter().any(|a| a.id().0 == id)
    }

    fn run_action_with(&mut self, action: ActionId, ctx: ActionCtx) {
        if self.intercept(action, &ctx) {
            return;
        }
        self.handle(Command::Run {
            action,
            ctx: ctx.clone(),
        });
        if let Some(ask) = self.host.take_asked() {
            let input = match &ask {
                Ask::Prompt { default, .. } => default.clone(),
                Ask::Confirm { .. } | Ask::Form(_) => String::new(),
            };
            let values = match &ask {
                Ask::Form(form) => form.defaults(),
                _ => ext::Values::default(),
            };
            let rows = match &ask {
                Ask::Form(form) => form.preview_rows(&values),
                _ => Vec::new(),
            };
            self.dialog = Some(Dialog {
                ask,
                origin: DialogOrigin::Rerun { action, ctx },
                input,
                focus: true,
                values,
                rows,
                browse: None,
            });
        }
    }

    /// Opens an entry: a trashed item is restored; otherwise the app
    /// decides: a folder or an archive navigates, anything else goes to the
    /// system default application.
    fn open_entry(&mut self, pane: PaneId, id: EntryId) {
        let Some((_, tab)) = self.app.model().active_tab(pane) else {
            return;
        };
        // A trashed item opens nowhere; Enter and a double-click restore it
        // through the trash plugin's action.
        if vfs_rick::is_trash(&tab.location) {
            let Some(entry) = tab.entry(id).cloned() else {
                return;
            };
            let ctx = ActionCtx {
                pane,
                location: tab.location.clone(),
                selection: vec![entry],
                invoked_from: Invocation::Shortcut,
                other: None,
            };
            self.run_action_with(ActionId("trash.restore_items"), ctx);
            return;
        }
        self.handle(Command::Open { pane, id });
    }

    #[allow(clippy::too_many_lines)]
    fn dispatch(&mut self, intent: Intent, pane: PaneId, ctx: &egui::Context) {
        let pane = self.route(intent, pane);
        self.dispatched.push_back(Dispatched { intent, pane });
        while self.dispatched.len() > RECENTS {
            self.dispatched.pop_front();
        }
        let window = self
            .app
            .model()
            .window_of(pane)
            .unwrap_or(self.app.model().focused_window);
        match intent {
            Intent::Action(action) => self.run_action(action, pane, Invocation::Shortcut),
            Intent::Open => self.open_selection(pane),
            Intent::Back => self.handle(Command::Back(pane)),
            Intent::Forward => self.handle(Command::Forward(pane)),
            Intent::Refresh => self.handle(Command::Refresh(pane)),
            Intent::Parent => {
                if let Some((tab_id, tab)) = self.app.model().active_tab(pane) {
                    let location = tab.location.clone();
                    let to = paths::parent_of(self.tab_ui(tab_id).crumbs_of(&location));
                    if let Some(to) = to {
                        self.handle(Command::Navigate { pane, to });
                    }
                }
            }
            Intent::Root => {
                if let Some((tab_id, tab)) = self.app.model().active_tab(pane) {
                    let location = tab.location.clone();
                    let to = paths::root_of(self.tab_ui(tab_id).crumbs_of(&location));
                    if let Some(to) = to {
                        self.handle(Command::Navigate { pane, to });
                    }
                }
            }
            Intent::FocusPath => {
                if let Some((tab_id, tab)) = self.app.model().active_tab(pane) {
                    let text = paths::text_of(&tab.location);
                    let tab_ui = self.tab_ui(tab_id);
                    tab_ui.path_edit = Some(text);
                    tab_ui.focus_path = true;
                }
            }
            Intent::Palette => {
                if let Some(w) = self.windows.get_mut(&window) {
                    w.search.focus_with(">");
                }
            }
            Intent::SelectAll => self.handle(Command::Select {
                pane,
                change: SelectionChange::All,
            }),
            Intent::InvertSelection => self.handle(Command::Select {
                pane,
                change: SelectionChange::Invert,
            }),
            Intent::PreviewPlay => {
                inspector::play_pause(&mut self.previews, self.app.preview_pane());
            }
            Intent::ToggleFocused => {
                let cursor = self
                    .app
                    .model()
                    .active_tab(pane)
                    .and_then(|(id, _)| self.tabs.get(&id))
                    .and_then(|t| t.cursor);
                if let Some(id) = cursor {
                    self.handle(Command::Select {
                        pane,
                        change: SelectionChange::Toggle(id),
                    });
                }
            }
            Intent::Escape => {
                if let Some((tab_id, tab)) = self.app.model().active_tab(pane) {
                    if self.tabs.get(&tab_id).is_some_and(|t| t.edit.is_some()) {
                        if let Some(t) = self.tabs.get_mut(&tab_id) {
                            t.edit = None;
                        }
                    } else if self
                        .tabs
                        .get(&tab_id)
                        .is_some_and(|t| t.path_edit.is_some())
                    {
                        if let Some(t) = self.tabs.get_mut(&tab_id) {
                            t.path_edit = None;
                        }
                    } else if !tab.filter.is_empty() {
                        self.handle(Command::Filter {
                            pane,
                            text: String::new(),
                        });
                    } else {
                        self.handle(Command::Select {
                            pane,
                            change: SelectionChange::Clear,
                        });
                    }
                }
            }
            Intent::Inspect => self.inspect(pane),
            Intent::Undo => self.handle(Command::Undo),
            Intent::Redo => self.handle(Command::Redo),
            Intent::NewTab | Intent::DuplicateTab => {
                if let Some((_, tab)) = self.app.model().active_tab(pane) {
                    let location = tab.location.clone();
                    self.handle(Command::NewTab(location));
                }
            }
            Intent::CloseTab => {
                if let Some((tab_id, tab)) = self.app.model().active_tab(pane) {
                    self.closed_tabs.push(tab.location.clone());
                    self.handle(Command::CloseTab(tab_id));
                }
            }
            Intent::ReopenTab => {
                if let Some(location) = self.closed_tabs.pop() {
                    self.handle(Command::NewTab(location));
                }
            }
            Intent::NextTab => self.step_tab(pane, 1),
            Intent::PrevTab => self.step_tab(pane, -1),
            Intent::Tab(n) => {
                let order = self.order_of(pane);
                let index = if n == 9 {
                    order.len().saturating_sub(1)
                } else {
                    usize::from(n).saturating_sub(1)
                };
                if let Some(tab) = order.get(index).copied() {
                    self.handle(Command::ActivateTab(tab));
                }
            }
            Intent::MoveTabLeft | Intent::MoveTabRight => {
                if let Some((tab_id, _)) = self.app.model().active_tab(pane)
                    && let Some(w) = self.window_ui_of_mut(pane)
                {
                    w.dock
                        .shift_tab(tab_id, if intent == Intent::MoveTabLeft { -1 } else { 1 });
                }
            }
            Intent::SplitRight => self.handle(Command::Split {
                pane,
                dir: model::SplitDir::Right,
            }),
            Intent::SplitDown => self.handle(Command::Split {
                pane,
                dir: model::SplitDir::Down,
            }),
            Intent::FocusPane(direction) => self.focus_towards(pane, direction),
            Intent::CloseOtherTabs => {
                if let Some((keep, _)) = self.app.model().active_tab(pane) {
                    let others: Vec<TabId> = self
                        .app
                        .model()
                        .pane(pane)
                        .map(|p| p.tabs().iter().copied().filter(|t| *t != keep).collect())
                        .unwrap_or_default();
                    for tab in others {
                        self.handle(Command::CloseTab(tab));
                    }
                }
            }
            Intent::Filter => {
                if let Some((tab_id, _)) = self.app.model().active_tab(pane) {
                    self.tab_ui(tab_id).focus_filter = true;
                }
            }
            Intent::ToggleHidden => self.handle(Command::ToggleHidden(pane)),
            Intent::ToggleSystem => {
                let on = !self.app.model().settings.show_system;
                self.handle(Command::Settings(SettingsChange::ShowSystem(on)));
            }
            Intent::ToggleExtensions => {
                let on = !self.app.model().settings.show_extensions;
                self.handle(Command::Settings(SettingsChange::ShowExtensions(on)));
            }
            Intent::ToggleFlatten => {
                if let Some((_, tab)) = self.app.model().active_tab(pane) {
                    let on = !tab.flatten;
                    self.handle(Command::Flatten { pane, on });
                }
            }
            Intent::ToggleSubtree => {
                // Without a subdirectory search index there is nothing to
                // walk, and the filter bar shows no toggle either.
                if self.app.model().subtree_search
                    && let Some((tab_id, tab)) = self.app.model().active_tab(pane)
                {
                    let on = !tab.subtree;
                    self.handle(Command::Subtree { pane, on });
                    if on {
                        self.tabs.entry(tab_id).or_default().focus_filter = true;
                    }
                }
            }
            Intent::ViewTree => self.handle(Command::SetView {
                pane,
                mode: ViewMode::Tree,
            }),
            Intent::ToggleInspector => {
                let fraction = self.preview_fraction(window);
                self.handle(Command::TogglePreview { window, fraction });
            }
            Intent::Settings => self.settings_open = true,
            Intent::Shortcuts => self.shortcuts.open = true,
            Intent::Extensions => self.extensions_open = true,
            Intent::ViewList => self.handle(Command::SetView {
                pane,
                mode: ViewMode::List,
            }),
            Intent::ViewDetails => self.handle(Command::SetView {
                pane,
                mode: ViewMode::Details,
            }),
            Intent::SizeColumns => {
                if let Some((tab_id, _)) = self.app.model().active_tab(pane) {
                    self.tab_ui(tab_id).reset_columns = true;
                }
            }
            Intent::Sort(column) => {
                if let Some((_, tab)) = self.app.model().active_tab(pane) {
                    let dir = if tab.sort.by == column {
                        match tab.sort.dir {
                            SortDir::Ascending => SortDir::Descending,
                            SortDir::Descending => SortDir::Ascending,
                        }
                    } else {
                        SortDir::Ascending
                    };
                    self.handle(Command::Sort {
                        pane,
                        by: column,
                        dir,
                    });
                }
            }
            Intent::ClearGrouping => self.handle(Command::GroupBy { pane, by: None }),
            Intent::Fullscreen => {
                let on = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!on));
            }
            Intent::Quit => self.quit(ctx),
            Intent::NewWindow => self.handle(Command::NewWindow),
            Intent::SaveLayout => {
                self.name_prompt = Some(NamePrompt {
                    what: NameFor::SaveLayout,
                    input: String::new(),
                    focus: true,
                });
            }
            Intent::LastSession => self.handle(Command::SwitchLayout(None)),
            Intent::Layout(index) => {
                if let Some(name) = self.app.layouts().get(usize::from(index)).cloned() {
                    self.handle(Command::SwitchLayout(Some(name)));
                }
            }
            Intent::CursorUp => self.move_cursor(pane, -1, false),
            Intent::CursorDown => self.move_cursor(pane, 1, false),
            Intent::CursorPageUp => self.move_cursor(pane, -self.page(pane), false),
            Intent::CursorPageDown => self.move_cursor(pane, self.page(pane), false),
            Intent::CursorHome => self.move_cursor(pane, isize::MIN, false),
            Intent::CursorEnd => self.move_cursor(pane, isize::MAX, false),
            Intent::ExtendUp => self.move_cursor(pane, -1, true),
            Intent::ExtendDown => self.move_cursor(pane, 1, true),
            Intent::ExtendPageUp => self.move_cursor(pane, -self.page(pane), true),
            Intent::ExtendPageDown => self.move_cursor(pane, self.page(pane), true),
            Intent::ExtendHome => self.move_cursor(pane, isize::MIN, true),
            Intent::ExtendEnd => self.move_cursor(pane, isize::MAX, true),
            Intent::QuickOpen => {
                if let Some(w) = self.windows.get_mut(&window) {
                    w.search.focus_with("");
                }
            }
            Intent::ViewIcons => {
                let size = match self.app.model().active_tab(pane).map(|(_, t)| t.view) {
                    Some(ViewMode::Icons(size)) => size,
                    _ => IconSize::Medium,
                };
                self.handle(Command::SetView {
                    pane,
                    mode: ViewMode::Icons(size),
                });
            }
            Intent::IconSizeUp | Intent::IconSizeDown => {
                let current = match self.app.model().active_tab(pane).map(|(_, t)| t.view) {
                    Some(ViewMode::Icons(size)) => size,
                    _ => IconSize::Medium,
                };
                let order = [
                    IconSize::Small,
                    IconSize::Medium,
                    IconSize::Large,
                    IconSize::ExtraLarge,
                ];
                let at = order.iter().position(|s| *s == current).unwrap_or(1);
                let next = if intent == Intent::IconSizeUp {
                    (at + 1).min(order.len() - 1)
                } else {
                    at.saturating_sub(1)
                };
                self.handle(Command::SetView {
                    pane,
                    mode: ViewMode::Icons(order[next]),
                });
            }
            Intent::ToggleSidePanel => {
                let visible = self
                    .app
                    .model()
                    .window(window)
                    .is_none_or(|w| w.side_panel.visible);
                self.handle(Command::SidePanel {
                    window,
                    change: SidePanelChange::Visible(!visible),
                });
            }
        }
    }

    /// The tab order of a pane as its window's dock shows it.
    fn order_of(&self, pane: PaneId) -> Vec<TabId> {
        self.window_ui_of(pane)
            .map(|w| w.dock.tab_order(self.app.model(), pane))
            .unwrap_or_default()
    }

    fn open_selection(&mut self, pane: PaneId) {
        let Some((_, tab)) = self.app.model().active_tab(pane) else {
            return;
        };
        let selected: Vec<&types::Entry> = tab.selected().collect();
        match selected.as_slice() {
            [] => {
                if !tab.filter.is_empty()
                    && let Some(first) = tab.row(0)
                {
                    let id = first.id;
                    self.open_entry(pane, id);
                }
            }
            // One entry: a folder or an archive navigates, a file goes to the
            // system application, a trashed item is restored; several entries
            // all go to the system (or back out of the trash).
            [one] => {
                let id = one.id;
                self.open_entry(pane, id);
            }
            _ => self.run_action(ActionId("core.open_default"), pane, Invocation::Shortcut),
        }
    }

    /// Opens the platform's file dialogue for a missing library's file; the
    /// answer is polled each frame ([`poll_library_browse`](Self::poll_library_browse)).
    fn browse_library(
        &mut self,
        pane: PaneId,
        setting: (types::ExtId, &'static str),
        filters: Vec<platform::DialogFilter>,
    ) {
        if self.library_browse.is_some() {
            return;
        }
        let platform = Arc::clone(self.app.platform());
        let title = t!("preview-library-choose", name = setting.0.0);
        let (tx, rx) = runtime::oneshot::channel();
        runtime::handle().spawn(async move {
            let _ = tx.send(platform.pick_files(&title, &filters).await);
        });
        self.library_browse = Some(LibraryBrowse {
            pane,
            setting,
            answer: rx,
        });
    }

    /// A file the *Browse…* dialogue picked goes into the previewer's
    /// setting, and the preview opens again.
    fn poll_library_browse(&mut self, ctx: &egui::Context) {
        let Some(browse) = self.library_browse.as_mut() else {
            return;
        };
        match browse.answer.try_recv() {
            Ok(files) => {
                let (pane, (ext, key)) = (browse.pane, browse.setting);
                self.library_browse = None;
                if let Some(path) = files.into_iter().next() {
                    self.handle(Command::ExtSetting {
                        ext: ext.0.to_owned(),
                        key: key.to_owned(),
                        value: ext::SettingValue::Text(path.display().to_string()),
                    });
                    self.handle(Command::PreviewRetry(pane));
                }
            }
            Err(runtime::oneshot::error::TryRecvError::Closed) => self.library_browse = None,
            Err(runtime::oneshot::error::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
    }

    /// Properties on the selection: the application reads the entry and
    /// answers with `Event::Properties`, which opens the dialogue.
    fn inspect(&mut self, pane: PaneId) {
        self.requests
            .push(Request::Command(Command::ShowProperties(pane)));
    }

    fn step_tab(&mut self, pane: PaneId, by: isize) {
        let order = self.order_of(pane);
        let Some((current, _)) = self.app.model().active_tab(pane) else {
            return;
        };
        let Some(index) = order.iter().position(|t| *t == current) else {
            return;
        };
        if order.is_empty() {
            return;
        }
        let next = (index as isize + by).rem_euclid(order.len() as isize) as usize;
        self.handle(Command::ActivateTab(order[next]));
    }

    fn focus_towards(&mut self, pane: PaneId, direction: Direction) {
        let rects = self
            .window_ui_of(pane)
            .map(|w| w.dock.pane_rects(self.app.model()))
            .unwrap_or_default();
        let Some((_, from)) = rects.iter().find(|(p, _)| *p == pane) else {
            return;
        };
        let from = *from;
        let best = rects
            .iter()
            .filter(|(p, _)| *p != pane && !crate::dock::shows_preview(self.app.model(), *p))
            .filter(|(_, r)| match direction {
                Direction::Left => r.max.x <= from.min.x + 1.0,
                Direction::Right => r.min.x >= from.max.x - 1.0,
                Direction::Up => r.max.y <= from.min.y + 1.0,
                Direction::Down => r.min.y >= from.max.y - 1.0,
            })
            .min_by(|(_, a), (_, b)| {
                let da = a.center().distance(from.center());
                let db = b.center().distance(from.center());
                da.total_cmp(&db)
            })
            .map(|(p, _)| *p);
        if let Some(target) = best {
            self.handle(Command::FocusPane(target));
        }
    }

    fn page(&self, pane: PaneId) -> isize {
        self.app
            .model()
            .active_tab(pane)
            .and_then(|(id, _)| self.tabs.get(&id))
            .map_or(20, |t| t.visible_rows.max(1) as isize)
    }

    fn move_cursor(&mut self, pane: PaneId, by: isize, extend: bool) {
        let model = self.app.model();
        let Some((tab_id, tab)) = model.active_tab(pane) else {
            return;
        };
        let count = tab.row_count();
        if count == 0 {
            return;
        }
        let tab_ui = self.tabs.entry(tab_id).or_default();
        let current = tab_ui
            .cursor
            .and_then(|c| tab.rows().position(|e| e.id == c));
        let target = match (by, current) {
            (isize::MIN, _) => 0,
            (isize::MAX, _) => count - 1,
            (by, Some(i)) => (i as isize + by).clamp(0, count as isize - 1) as usize,
            (by, None) if by < 0 => count - 1,
            (_, None) => 0,
        };
        let Some(id) = tab.row(target).map(|e| e.id) else {
            return;
        };
        let change = if extend {
            let anchor = tab_ui.anchor.or(tab_ui.cursor).unwrap_or(id);
            if tab_ui.anchor.is_none() {
                tab_ui.anchor = Some(anchor);
            }
            let anchor_index = tab.rows().position(|e| e.id == anchor).unwrap_or(target);
            let (a, b) = (anchor_index.min(target), anchor_index.max(target));
            SelectionChange::Set((a..=b).filter_map(|i| tab.row(i).map(|e| e.id)).collect())
        } else {
            tab_ui.anchor = Some(id);
            SelectionChange::Set(vec![id])
        };
        tab_ui.cursor = Some(id);
        tab_ui.scroll_to = Some(target);
        self.handle(Command::Select { pane, change });
    }

    /// The title row: burger menu, the search box, the layout toggles and
    /// the window controls. An empty part of it drags the window and a
    /// double click maximises or restores it.
    fn title_row(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        window: WindowId,
        focused: PaneId,
    ) {
        let chrome = self.app.platform().window_chrome();
        let height = TITLE_ROW_HEIGHT;
        egui::Panel::top(egui::Id::new(("title", window.0)))
            .exact_size(height)
            .show_separator_line(true)
            .show(ui, |ui| {
                let rect = ui.max_rect();
                // The background first, so every widget drawn after it wins
                // the pointer where they overlap.
                let background = ui.interact(
                    rect,
                    egui::Id::new(("title-drag", window.0)),
                    egui::Sense::click_and_drag(),
                );
                if background.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if background.double_clicked() {
                    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }

                // Left: room for the native controls, then the burger.
                let mut left = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                if chrome.native_controls && chrome.controls == ControlsSide::Left {
                    left.add_space(chrome.native_controls_width);
                }
                menus::burger(&mut left, &mut self.requests, focused, &self.app);
                let left_edge = left.min_rect().right();

                // Right: window controls, then the toggles.
                let mut right = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                );
                self.title_right(&mut right, ctx, chrome, window, focused);
                let right_edge = right.min_rect().left();

                self.title_search(ui, rect, left_edge, right_edge, window, focused);
            });
    }

    /// The side panel on its side with the operations area at its foot, or
    /// the operations area along the window's foot when the panel is hidden.
    fn side_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, window: WindowId) {
        let side = self
            .app
            .model()
            .window(window)
            .map(|w| w.side_panel.clone())
            .unwrap_or_default();
        let jobs = !self.app.model().jobs.is_empty();
        let operations_height = Operations::height(self.app.model());
        if !side.visible {
            if jobs {
                egui::Panel::bottom(egui::Id::new(("operations", window.0)))
                    .exact_size(operations_height)
                    .show(ui, |ui| {
                        self.operations
                            .show(ui, self.app.model(), &mut self.requests);
                    });
            }
            return;
        }
        let panel = match side.side {
            PanelSide::Left => egui::Panel::left(egui::Id::new(("side", window.0))),
            PanelSide::Right => egui::Panel::right(egui::Id::new(("side", window.0))),
        };
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let shown = panel
            .frame(egui::Frame::side_top_panel(ui.style()).fill(crate::theme::scheme(ui).bg_panel))
            .resizable(true)
            .default_size(f32::from(side.width))
            .size_range(120.0..=600.0)
            .show(ui, |ui| {
                if jobs {
                    egui::Panel::bottom(egui::Id::new(("operations", window.0)))
                        .exact_size(operations_height)
                        .show(ui, |ui| {
                            self.operations
                                .show(ui, self.app.model(), &mut self.requests);
                        });
                }
                side_panel::show(
                    ui,
                    &self.app,
                    window,
                    &side,
                    &mut self.side,
                    &mut self.requests,
                    &mut w.dnd,
                );
            });
        // The outer width is what egui restores from `default_size`, so it
        // is what persists; only once the pointer is up, so a drag writes
        // the file once.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let width = shown
            .response
            .rect
            .width()
            .round()
            .clamp(0.0, f32::from(u16::MAX)) as u16;
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        if !pointer_down && width > 0 && width != side.width {
            self.requests.push(Request::Command(Command::SidePanel {
                window,
                change: SidePanelChange::Width(width),
            }));
        }
    }

    /// Window controls and the layout toggles, right to left.
    fn title_right(
        &mut self,
        right: &mut egui::Ui,
        ctx: &egui::Context,
        chrome: platform::WindowChrome,
        window: WindowId,
        focused: PaneId,
    ) {
        if !chrome.native_controls {
            if right
                .add(egui::Button::new(icons::CLOSE).frame(false))
                .on_hover_text(t!("close"))
                .clicked()
            {
                self.requests.push(Request::CloseWindow(window));
            }
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            let glyph = if maximized {
                icons::RESTORE
            } else {
                icons::MAXIMISE
            };
            if right
                .add(egui::Button::new(glyph).frame(false))
                .on_hover_text(if maximized {
                    t!("shell-restore")
                } else {
                    t!("shell-maximise")
                })
                .clicked()
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            if right
                .add(egui::Button::new(icons::MINIMISE).frame(false))
                .on_hover_text(t!("shell-minimise"))
                .clicked()
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            right.separator();
        }
        let preview_open = self.app.model().preview_tab(window).is_some();
        if right
            .add(toggle_icon(icons::PREVIEW, preview_open))
            .on_hover_text(t!("shell-toggle-preview-pane"))
            .clicked()
        {
            self.requests.push(Request::Intent {
                intent: Intent::ToggleInspector,
                pane: focused,
            });
        }
        let side_visible = self
            .app
            .model()
            .window(window)
            .is_none_or(|w| w.side_panel.visible);
        if right
            .add(toggle_icon(icons::SIDE_PANEL, side_visible))
            .on_hover_text(t!("shell-toggle-side-panel"))
            .clicked()
        {
            self.requests.push(Request::Intent {
                intent: Intent::ToggleSidePanel,
                pane: focused,
            });
        }
        let running = self
            .app
            .model()
            .jobs
            .values()
            .filter(|j| matches!(j.state, JobState::Running(_)))
            .count();
        if running > 0 {
            right.spinner();
            right.weak(format!("{running} running"));
        }
    }

    /// The search box between the burger and the right-hand group, as wide
    /// as the room allows.
    fn title_search(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        left_edge: f32,
        right_edge: f32,
        window: WindowId,
        focused: PaneId,
    ) {
        let room = (right_edge - left_edge - 24.0).max(0.0);
        let width = (rect.width() * 0.4).clamp(160.0, 560.0).min(room);
        if width < 120.0 {
            return;
        }
        let centre = f32::midpoint(left_edge, right_edge).clamp(
            left_edge + width / 2.0 + 12.0,
            right_edge - width / 2.0 - 12.0,
        );
        let search_rect = egui::Rect::from_center_size(
            egui::pos2(centre, rect.center().y),
            egui::vec2(width, (rect.height() - 6.0).max(16.0)),
        );
        let recents: Vec<Intent> = self.dispatched.iter().rev().map(|d| d.intent).collect();
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let pick = w.search.show(
            ui,
            search_rect,
            &self.app,
            &self.keys,
            focused,
            &recents,
            window.0,
        );
        // The indexes are asked for the text once it changed, and told
        // when the box is cleared; without an available index there is
        // nothing to ask and 0.1's search of the current location stands.
        if let Some(text) = w.search.take_query() {
            let model = self.app.model();
            if text != model.index_search.text
                && (!model.search_indexes.is_empty() || text.is_empty())
            {
                self.requests
                    .push(Request::Command(Command::IndexSearch { text }));
            }
        }
        match pick {
            Some(Pick::Intent(intent)) => self.requests.push(Request::Intent {
                intent,
                pane: focused,
            }),
            Some(Pick::Hit(entry)) => self.requests.push(Request::OpenContaining {
                pane: focused,
                entry,
                new_tab: false,
            }),
            Some(Pick::Entry { index, id }) => {
                if let Some((tab_id, _)) = self.app.model().active_tab(focused) {
                    let tab_ui = self.tab_ui(tab_id);
                    tab_ui.scroll_to = Some(index);
                    tab_ui.cursor = Some(id);
                    tab_ui.anchor = Some(id);
                }
                self.requests.push(Request::Command(Command::Select {
                    pane: focused,
                    change: SelectionChange::Set(vec![id]),
                }));
            }
            None => {}
        }
    }

    // One dialogue, three shapes.
    #[allow(clippy::too_many_lines)]
    fn show_dialog(&mut self, ctx: &egui::Context) {
        let platform = Arc::clone(self.app.platform());
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        // A folder the Browse… button's dialogue picked lands in its field.
        if let Some((key, rx)) = dialog.browse.as_mut() {
            match rx.try_recv() {
                Ok(Some(path)) => {
                    dialog
                        .values
                        .set(key, ext::Value::Text(path.display().to_string()));
                    if let Ask::Form(form) = &dialog.ask {
                        dialog.rows = form.preview_rows(&dialog.values);
                    }
                    dialog.browse = None;
                }
                Ok(None) | Err(runtime::oneshot::error::TryRecvError::Closed) => {
                    dialog.browse = None;
                }
                Err(runtime::oneshot::error::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
            }
        }
        let mut outcome: Option<bool> = None;
        let wide = matches!(dialog.ask, Ask::Form(_));
        let modal = egui::Modal::new(egui::Id::new(t!("shell-ask"))).show(ctx, |ui| {
            ui.set_width(if wide { 560.0 } else { 360.0 });
            match &dialog.ask {
                Ask::Form(form) => {
                    ui.heading(&form.title);
                    let mut changed = false;
                    let mut browse: Option<&'static str> = None;
                    egui::Grid::new("ask-form")
                        .num_columns(2)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            for (index, field) in form.fields.iter().enumerate() {
                                ui.label(&field.label);
                                match &field.kind {
                                    ext::FieldKind::Text { hint, .. } => {
                                        let mut text = dialog.values.text(field.key).to_owned();
                                        let output = egui::TextEdit::singleline(&mut text)
                                            .hint_text(hint)
                                            .desired_width(f32::INFINITY)
                                            .show(ui);
                                        if dialog.focus && index == 0 {
                                            output.response.request_focus();
                                            let end = output.galley.text().len();
                                            let mut state = output.state;
                                            state.cursor.set_char_range(Some(
                                                egui::text::CCursorRange::two(
                                                    egui::text::CCursor::new(0),
                                                    egui::text::CCursor::new(end),
                                                ),
                                            ));
                                            state.store(ctx, output.response.id);
                                        }
                                        if output.response.changed() {
                                            dialog.values.set(field.key, ext::Value::Text(text));
                                            changed = true;
                                        }
                                        if output.response.lost_focus()
                                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                        {
                                            outcome = Some(true);
                                        }
                                    }
                                    ext::FieldKind::Folder { .. } => {
                                        ui.horizontal(|ui| {
                                            let mut text = dialog.values.text(field.key).to_owned();
                                            let response = ui.add(
                                                egui::TextEdit::singleline(&mut text)
                                                    .desired_width(ui.available_width() - 40.0),
                                            );
                                            if response.changed() {
                                                dialog
                                                    .values
                                                    .set(field.key, ext::Value::Text(text));
                                                changed = true;
                                            }
                                            if response.lost_focus()
                                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                            {
                                                outcome = Some(true);
                                            }
                                            if ui
                                                .button(icons::BROWSE)
                                                .on_hover_text(t!("shell-browse"))
                                                .clicked()
                                            {
                                                browse = Some(field.key);
                                            }
                                        });
                                    }
                                    ext::FieldKind::Check { .. } => {
                                        let mut on = dialog.values.check(field.key);
                                        if ui.checkbox(&mut on, "").changed() {
                                            dialog.values.set(field.key, ext::Value::Check(on));
                                            changed = true;
                                        }
                                    }
                                    ext::FieldKind::Choice { options, .. } => {
                                        let mut selected = dialog.values.choice(field.key);
                                        let current = options
                                            .get(selected)
                                            .map_or("", String::as_str)
                                            .to_owned();
                                        egui::ComboBox::from_id_salt(("ask-choice", field.key))
                                            .selected_text(current)
                                            .width(ui.available_width())
                                            .show_ui(ui, |ui| {
                                                for (i, option) in options.iter().enumerate() {
                                                    ui.selectable_value(&mut selected, i, option);
                                                }
                                            });
                                        if selected != dialog.values.choice(field.key) {
                                            dialog
                                                .values
                                                .set(field.key, ext::Value::Choice(selected));
                                            changed = true;
                                        }
                                    }
                                }
                                ui.end_row();
                            }
                        });
                    dialog.focus = false;
                    if changed {
                        dialog.rows = form.preview_rows(&dialog.values);
                    }
                    if let Some(key) = browse
                        && dialog.browse.is_none()
                    {
                        let (tx, rx) = runtime::oneshot::channel();
                        let title = format!("{} — choose a folder", form.title);
                        let platform = Arc::clone(&platform);
                        runtime::handle().spawn(async move {
                            let _ = tx.send(platform.pick_folder(&title).await);
                        });
                        dialog.browse = Some((key, rx));
                    }
                    if form.preview.is_some() {
                        ui.add_space(6.0);
                        let problems = dialog.rows.iter().filter(|r| r.problem.is_some()).count();
                        ui.weak(match (dialog.rows.len(), problems) {
                            (0, _) => "Nothing to preview".to_owned(),
                            (n, 0) => format!("{n} entries"),
                            (n, p) => format!("{n} entries, {p} with a problem"),
                        });
                        let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
                        egui::ScrollArea::vertical()
                            .max_height(240.0)
                            .auto_shrink([false, true])
                            .show_rows(ui, row_height, dialog.rows.len(), |ui, range| {
                                egui::Grid::new(("ask-preview", range.start))
                                    .num_columns(3)
                                    .striped(true)
                                    .min_col_width(80.0)
                                    .show(ui, |ui| {
                                        for row in &dialog.rows[range] {
                                            ui.label(&row.from);
                                            ui.label(icons::FORWARD);
                                            match &row.problem {
                                                Some(problem) => {
                                                    ui.colored_label(
                                                        ui.visuals().error_fg_color,
                                                        format!("{} — {problem}", row.to),
                                                    );
                                                }
                                                None => {
                                                    ui.label(&row.to);
                                                }
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                    }
                }
                Ask::Prompt { title, .. } => {
                    ui.heading(title);
                    let output = egui::TextEdit::singleline(&mut dialog.input)
                        .desired_width(f32::INFINITY)
                        .show(ui);
                    if dialog.focus {
                        output.response.request_focus();
                        let end = output.galley.text().len();
                        let mut state = output.state;
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(end),
                            )));
                        state.store(ctx, output.response.id);
                        dialog.focus = false;
                    }
                    if output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        outcome = Some(true);
                    }
                }
                Ask::Confirm { title, text } => {
                    ui.heading(title);
                    ui.label(text);
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        outcome = Some(true);
                    }
                }
            }
            ui.add_space(8.0);
            // A form with a problem in its preview cannot be confirmed.
            let blocked = matches!(&dialog.ask, Ask::Form(form) if form.preview.is_some())
                && dialog.rows.iter().any(|r| r.problem.is_some());
            let ok_label = match &dialog.ask {
                Ask::Form(form) => form.ok.as_str(),
                _ => "OK",
            };
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!blocked, egui::Button::new(ok_label))
                    .clicked()
                {
                    outcome = Some(true);
                }
                if ui.button(t!("cancel")).clicked() {
                    outcome = Some(false);
                }
            });
            if blocked && outcome == Some(true) {
                outcome = None;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                outcome = Some(false);
            }
        });
        if modal.should_close() && outcome.is_none() {
            outcome = Some(false);
        }
        let Some(ok) = outcome else {
            return;
        };
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        match dialog.origin {
            DialogOrigin::Rerun { action, ctx } => {
                if !ok {
                    self.host.clear_answers();
                    return;
                }
                match dialog.ask {
                    Ask::Prompt { .. } => self.host.answer_prompt(dialog.input),
                    Ask::Confirm { .. } => self.host.answer_confirm(true),
                    Ask::Form(_) => self.host.answer_form(dialog.values),
                }
                self.run_action_with(action, ctx);
            }
            DialogOrigin::Continue { pane, then } => {
                self.handle(Command::Continue {
                    pane,
                    then,
                    values: ok.then_some(dialog.values),
                });
            }
        }
    }

    /// The prompt for a layout name: Save layout as…, Rename….
    fn show_name_prompt(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.name_prompt.as_mut() else {
            return;
        };
        let mut outcome: Option<bool> = None;
        let title = match &prompt.what {
            NameFor::SaveLayout => "Save layout as".to_owned(),
            NameFor::RenameLayout(name) => format!("Rename layout {name}"),
        };
        let modal = egui::Modal::new(egui::Id::new(t!("shell-layout-name"))).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading(title);
            let output = egui::TextEdit::singleline(&mut prompt.input)
                .hint_text(t!("shell-layout-name-2"))
                .desired_width(f32::INFINITY)
                .show(ui);
            if prompt.focus {
                output.response.request_focus();
                let end = output.galley.text().len();
                let mut state = output.state;
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(end),
                    )));
                state.store(ctx, output.response.id);
                prompt.focus = false;
            }
            if output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                outcome = Some(true);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(t!("ok")).clicked() {
                    outcome = Some(true);
                }
                if ui.button(t!("cancel")).clicked() {
                    outcome = Some(false);
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                outcome = Some(false);
            }
        });
        if modal.should_close() && outcome.is_none() {
            outcome = Some(false);
        }
        let Some(ok) = outcome else {
            return;
        };
        let Some(prompt) = self.name_prompt.take() else {
            return;
        };
        let name = prompt.input.trim().to_owned();
        if !ok || name.is_empty() {
            return;
        }
        match prompt.what {
            NameFor::SaveLayout => self.handle(Command::SaveLayout(name)),
            NameFor::RenameLayout(from) => self.handle(Command::RenameLayout { from, to: name }),
        }
    }

    fn show_toasts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.toasts.retain(|t| t.until > now);
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 40.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                for toast in &self.toasts {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        let color = match toast.level {
                            Level::Info => ui.visuals().text_color(),
                            Level::Warning => ui.visuals().warn_fg_color,
                            Level::Error => ui.visuals().error_fg_color,
                        };
                        ui.colored_label(color, &toast.text);
                    });
                }
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

/// Renders tabs for the dock.
struct Viewer<'a> {
    app: &'a App,
    tabs: &'a mut HashMap<TabId, TabUi>,
    requests: &'a mut Vec<Request>,
    keys: &'a KeyMap,
    now: SystemTime,
    /// The tab the pointer was over last frame; it shows a close button.
    hovered_last: Option<TabId>,
    /// The tab the pointer is over this frame.
    hovered_now: &'a mut Option<TabId>,
    thumbnails: &'a mut Thumbnails,
    dnd: &'a mut Dnd,
    tab_headers: &'a mut HashMap<TabId, egui::Rect>,
    /// Shift was down when the frame began: the plus opens a window.
    shift: bool,
    /// The pane keyboard input goes to in this window.
    focused: PaneId,
    /// The previews, for the Preview tab.
    previews: &'a mut Previews,
    /// The Preview tab's own state.
    inspector_state: &'a mut inspector::PaneState,
    /// The views actions opened, by tab.
    views: &'a HashMap<TabId, ext::View>,
    /// Each console tab's state.
    consoles: &'a mut HashMap<TabId, ConsoleUi>,
}

impl TabViewer for Viewer<'_> {
    type Tab = TabId;

    fn id(&mut self, tab: &mut TabId) -> egui::Id {
        egui::Id::new(("tab", tab.0))
    }

    fn title(&mut self, tab: &mut TabId) -> egui::WidgetText {
        self.app.model().tab(*tab).map_or_else(
            || "?".into(),
            |t| match t.kind {
                TabKind::Preview => format!("{} Preview", icons::PREVIEW).into(),
                TabKind::Console => format!(
                    "{} {}",
                    icons::TERMINAL,
                    t.title.as_deref().unwrap_or("Console")
                )
                .into(),
                TabKind::Compare => format!(
                    "{} {}",
                    icons::COMPARE,
                    t.title.as_deref().unwrap_or("Compare")
                )
                .into(),
                TabKind::Explorer => {
                    // A folder icon for local folders, a view's own for a
                    // virtual location, the archive's for an archive, the
                    // provider's otherwise.
                    let icon = vfs_rick::Virtual::of(&t.location)
                        .map_or_else(|| paths::location_icon(&t.location), virtual_icon);
                    format!("{icon} {}", paths::title_of(&t.location)).into()
                }
            },
        )
    }

    fn on_tab_button(&mut self, tab: &mut TabId, response: &egui::Response) {
        self.tab_headers.insert(*tab, response.rect);
        if response.hovered() {
            *self.hovered_now = Some(*tab);
        }
        if response.middle_clicked() {
            self.requests
                .push(Request::Command(Command::CloseTab(*tab)));
        }
        // A drop target: into the folder the tab shows; hovering activates
        // the tab. The Preview tab shows no folder.
        if self.dnd.active()
            && let Some(state) = self.app.model().tab(*tab)
            && state.kind.lists()
        {
            let highlight = self.dnd.zone(Zone {
                key: egui::Id::new(("dnd-tab", tab.0)),
                rect: response.rect,
                target: crate::dnd::into_pane(
                    state,
                    Target::Folder {
                        to: state.location.clone(),
                        device: crate::dnd::listing_device(state),
                        spring: Some(Spring::Tab(*tab)),
                    },
                ),
            });
            if highlight {
                let painter = response.ctx.layer_painter(response.layer_id);
                let style = response.ctx.style_of(response.ctx.theme());
                crate::dnd::highlight(&painter, &style.visuals, response.rect);
            }
        }
    }

    fn is_closeable(&self, tab: &TabId) -> bool {
        // The close button shows on the active tab and the hovered one.
        let model = self.app.model();
        let active = model
            .pane_of(*tab)
            .and_then(|p| model.pane(p))
            .and_then(model::Pane::active)
            == Some(*tab);
        active || self.hovered_last == Some(*tab)
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut TabId) {
        let model = self.app.model();
        let Some(pane) = model.pane_of(*tab) else {
            return;
        };
        let Some(state) = model.tab(*tab) else {
            return;
        };
        if state.kind == TabKind::Preview {
            let close = inspector::show(
                ui,
                self.app,
                self.previews,
                self.inspector_state,
                self.requests,
            );
            if close {
                self.requests
                    .push(Request::Command(Command::CloseTab(*tab)));
            }
            return;
        }
        if state.kind.is_view() {
            match self.views.get(tab) {
                Some(ext::View::Console(console)) => {
                    let state = self.consoles.entry(*tab).or_default();
                    console::show(ui, state, console, *tab);
                }
                Some(ext::View::Compare(compare)) => compare::show(ui, compare, self.requests),
                None => {
                    ui.centered_and_justified(|ui| {
                        ui.weak(t!("shell-this-tab-s-content-is"));
                    });
                }
            }
            return;
        }
        let tab_ui = self
            .tabs
            .entry(*tab)
            .or_insert_with(|| TabUi::for_tab(state));
        let mut cx = PaneCx {
            app: self.app,
            pane,
            tab_id: *tab,
            tab: state,
            tab_ui,
            requests: self.requests,
            keys: self.keys,
            now: self.now,
            focused: self.focused == pane && model.focused == pane,
            thumbnails: self.thumbnails,
            dnd: self.dnd,
        };
        pane::show(ui, &mut cx);
    }

    fn on_close(&mut self, _tab: &mut TabId) -> egui_dock::widgets::tab_viewer::OnCloseResponse {
        egui_dock::widgets::tab_viewer::OnCloseResponse::Close
    }

    fn on_add(&mut self, _path: NodePath) {
        if self.shift {
            self.requests.push(Request::Command(Command::NewWindow));
            return;
        }
        let model = self.app.model();
        if let Some((_, tab)) = model.active_tab(self.focused) {
            self.requests.push(Request::NewTabAt(tab.location.clone()));
        }
    }

    fn allowed_in_windows(&self, _tab: &mut TabId) -> bool {
        false
    }

    fn clear_background(&self, _tab: &TabId) -> bool {
        true
    }

    fn scroll_bars(&self, _tab: &TabId) -> [bool; 2] {
        [false, false]
    }
}

impl RickUi {
    /// The window's panels: title row, side panel and the dock, the Preview
    /// tab among its tabs.
    fn panels(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, window: WindowId) {
        let focused = self.focused_in(window);
        self.title_row(ui, ctx, window, focused);
        self.apply_requests(ctx);

        self.side_panel(ui, ctx, window);

        let shift = ctx.input(|i| i.modifiers.shift);
        let dock_id = egui::Id::new(("dock", window.0));
        let strip_height = TITLE_ROW_HEIGHT;
        let drag_before = self.windows.get(&window).and_then(|w| w.tab_drag);
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let Some(w) = self.windows.get_mut(&window) else {
                return;
            };
            w.dock.rebuild(self.app.model());
            let hovered_last = w.hovered_tab.take();
            let mut viewer = Viewer {
                app: &self.app,
                tabs: &mut self.tabs,
                requests: &mut self.requests,
                keys: &self.keys,
                now: self.now,
                hovered_last,
                hovered_now: &mut w.hovered_tab,
                thumbnails: &mut self.thumbnails,
                dnd: &mut w.dnd,
                tab_headers: &mut w.tab_headers,
                shift,
                focused,
                previews: &mut self.previews,
                inspector_state: &mut self.inspector_state,
                views: &self.views,
                consoles: &mut self.consoles,
            };
            DockArea::new(&mut w.dock.state)
                .id(dock_id)
                .style(tab_strip_style(ui.style(), crate::theme::scheme(ui)))
                .show_add_buttons(true)
                .show_close_buttons(true)
                .show_leaf_close_all_buttons(false)
                .show_leaf_collapse_buttons(false)
                .show_inside(ui, &mut viewer);
            w.strips = w.dock.strips(self.app.model(), strip_height);
            w.tab_drag = w.dock.dragged_tab(ctx, dock_id);
            // The Preview tab's leaf below its strip, for tests.
            let model = self.app.model();
            w.preview_rect = model
                .preview_tab(window)
                .and_then(|tab| model.pane_of(tab))
                .filter(|pane| {
                    model.pane(*pane).and_then(model::Pane::active) == model.preview_tab(window)
                })
                .and_then(|pane| {
                    w.dock
                        .pane_rects(model)
                        .into_iter()
                        .find(|(p, _)| *p == pane)
                        .map(|(_, rect)| {
                            egui::Rect::from_min_max(
                                egui::pos2(rect.left(), rect.top() + strip_height),
                                rect.max,
                            )
                        })
                });
        });
        self.cross_window_drop(ctx, window, drag_before);
        let may_focus = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let mut events = Vec::new();
        if let Some(w) = self.windows.get_mut(&window) {
            w.dock.reconcile(
                &mut self.app,
                &mut events,
                ctx.pointer_latest_pos(),
                may_focus,
            );
        }
        self.absorb(events);
        self.apply_requests(ctx);
    }

    /// A tab dragged out of a strip and released outside this window: onto
    /// another window's strip, which takes it, or onto nothing, which opens
    /// a window for it.
    fn cross_window_drop(
        &mut self,
        ctx: &egui::Context,
        window: WindowId,
        dragging: Option<TabId>,
    ) {
        let Some(tab) = dragging else {
            return;
        };
        let released = ctx.input(|i| i.pointer.any_released());
        if !released {
            return;
        }
        let Some(pos) = ctx.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()))
        else {
            return;
        };
        let Some(w) = self.windows.get(&window) else {
            return;
        };
        if w.content.is_some_and(|c| c.contains(pos)) {
            return;
        }
        let Some(from_screen) = w.screen else {
            // No screen position (a test harness): nothing to tear out to.
            return;
        };
        let screen_pos = from_screen.min + pos.to_vec2();
        let size = from_screen.size();
        // Another window's strip under the pointer takes the tab.
        let target = self.windows.iter().find_map(|(other, state)| {
            if *other == window {
                return None;
            }
            let origin = state.screen?.min;
            state
                .strips
                .iter()
                .find(|(_, strip)| strip.translate(origin.to_vec2()).contains(screen_pos))
                .map(|(pane, _)| *pane)
        });
        if let Some(pane) = target {
            self.handle(Command::MoveTab {
                tab,
                to: DockTarget::Into(pane),
            });
            return;
        }
        self.handle(Command::MoveTab {
            tab,
            to: DockTarget::NewWindow,
        });
        let new = self.app.model().focused_window;
        if new != window {
            self.handle(Command::WindowGeometry {
                window: new,
                geometry: Geometry {
                    x: screen_pos.x - 80.0,
                    y: screen_pos.y - TITLE_ROW_HEIGHT / 2.0,
                    width: size.x,
                    height: size.y,
                    maximised: false,
                },
            });
        }
    }

    /// Whether this viewport should show the dialogues, the screens and the
    /// toasts: the window with the system's focus, or the root when none has
    /// it.
    fn is_front(&self, ctx: &egui::Context, window: WindowId) -> bool {
        match ctx.input(|i| i.viewport().focused) {
            Some(true) => true,
            Some(false) => false,
            None => window == self.root,
        }
    }

    /// Draws one window: everything from the frame to the toasts.
    // One window, top to bottom, in the order the parts depend on each
    // other.
    #[allow(clippy::too_many_lines)]
    fn draw_window(&mut self, ui: &mut egui::Ui, window: WindowId) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        if !self.windows.contains_key(&window) {
            let state = WindowUi::new(self.app.model(), window);
            self.windows.insert(window, state);
        }
        self.now = SystemTime::now();
        let mut events = Vec::new();
        self.app.poll(&mut events);
        self.absorb(events);
        self.adopt_tabs(window);
        for (level, text) in self.host.take_notices() {
            self.toast(level, text);
        }
        // A form a job finished with opens as the one dialogue, once the
        // one before it is answered.
        if self.dialog.is_none()
            && let Some((pane, form, then)) = self.host.take_later()
        {
            let values = form.defaults();
            let rows = form.preview_rows(&values);
            self.dialog = Some(Dialog {
                ask: Ask::Form(form),
                origin: DialogOrigin::Continue { pane, then },
                input: String::new(),
                focus: true,
                values,
                rows,
                browse: None,
            });
        }
        for (title, text) in self.host.take_shown() {
            self.next_text += 1;
            self.texts.push(TextWindow {
                serial: self.next_text,
                title,
                text,
            });
        }
        self.apply_settings(ctx);
        // Positions and the platform's own drops are known for the backend's
        // window, the root viewport; the other windows fall back.
        let window_ref = (window == self.root).then_some(self.window_ref).flatten();
        if let Some(w) = self.windows.get_mut(&window) {
            w.dnd.begin_frame(
                ctx,
                &self.app,
                window_ref.as_ref(),
                &mut self.tabs,
                &mut self.requests,
            );
        }
        self.resolve_pending_selects();
        self.poll_library_browse(ctx);
        self.handle_keys(ctx, window);
        self.apply_requests(ctx);
        // A closed console or comparison takes its view with it; dropping a
        // console ends its shell.
        let model = self.app.model();
        self.views.retain(|tab, _| model.tab(*tab).is_some());
        self.consoles.retain(|tab, _| model.tab(*tab).is_some());

        // Rick's own window frame where the system draws none; the panels
        // go inside it.
        let chrome = self.app.platform().window_chrome();
        let window_frame =
            frame::wanted(ctx, chrome).then(|| frame::Frame::paint(ui, ui.max_rect()));
        let content = window_frame.map_or(ui.max_rect(), |f| f.content());
        if let Some(w) = self.windows.get_mut(&window) {
            w.window_frame = window_frame;
            w.content = Some(content);
        }
        let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(content));
        self.panels(&mut inside, ctx, window);
        if let Some(window_frame) = window_frame {
            window_frame.grips(ui, window.0);
        }
        if let Some(w) = self.windows.get_mut(&window) {
            w.dnd.end_frame(
                ctx,
                &self.app,
                window_ref.as_ref(),
                &mut self.tabs,
                &mut self.requests,
            );
            w.dnd.show_prompt(ctx, &mut self.requests);
        }
        self.apply_requests(ctx);

        if self.is_front(ctx, window) {
            self.show_dialog(ctx);
            conflict::show(ctx, &self.app, &mut self.conflict, &mut self.requests);
            question::show(ctx, &self.app, &mut self.question, &mut self.requests);
            self.show_name_prompt(ctx);
            settings_ui::settings_window(
                ctx,
                &mut self.settings_open,
                &self.app,
                window,
                &mut self.requests,
            );
            self.shortcuts
                .show(ctx, &self.keys, self.app.registry(), &mut self.requests);
            settings_ui::extensions_window(
                ctx,
                &mut self.extensions_open,
                &self.app,
                &mut self.requests,
            );
            settings_ui::about_window(ctx, &mut self.about_open, &self.version);
            self.properties.show(ctx, &mut self.requests);
            self.hash.show(ctx, &self.app, &mut self.requests);
            self.show_texts(ctx);
            self.show_toasts(ctx);
            self.apply_requests(ctx);
        }

        self.report_scroll(window);
        self.report_viewport(ctx, window);
        self.apply_requests(ctx);

        if self.app.is_busy() || self.operations.has_finished_rows() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        // Keep the dock in step before the next frame draws it.
        if let Some(w) = self.windows.get_mut(&window) {
            w.dock.rebuild(self.app.model());
            w.dock.take_order(self.app.model());
        }
    }

    /// Keeps the windows in step with the model: the root viewport follows
    /// its model window, or takes over another when that one closed; every
    /// other model window is drawn in a viewport of its own, in this same
    /// frame; states of windows the model no longer has are dropped.
    fn sync_windows(&mut self, ctx: &egui::Context) {
        if self.app.exit_requested() && !self.quitting {
            self.quit(ctx);
        }
        let model_windows: Vec<WindowId> = self.app.model().windows().map(|(id, _)| id).collect();
        self.windows.retain(|id, _| model_windows.contains(id));
        if !model_windows.contains(&self.root) {
            let Some(next) = model_windows.first().copied() else {
                return;
            };
            self.root = next;
            // The root viewport takes the place of the window it now shows.
            let geometry = self.app.model().window(next).and_then(|w| w.geometry);
            if let Some(g) = geometry {
                ctx.send_viewport_cmd_to(
                    ViewportId::ROOT,
                    ViewportCommand::OuterPosition(egui::pos2(g.x, g.y)),
                );
                ctx.send_viewport_cmd_to(
                    ViewportId::ROOT,
                    ViewportCommand::InnerSize(egui::vec2(g.width, g.height)),
                );
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Maximized(g.maximised));
            }
            if let Some(w) = self.windows.get_mut(&next) {
                w.placed = true;
            }
        }
        let chrome = self.app.platform().window_chrome();
        let mut ids = vec![ViewportId::ROOT];
        for id in model_windows {
            if id == self.root {
                continue;
            }
            let geometry = self.app.model().window(id).and_then(|w| w.geometry);
            let viewport = viewport_of(id);
            ids.push(viewport);
            ctx.show_viewport_immediate(
                viewport,
                viewport_builder("Rick", chrome, geometry),
                |ui, _class| self.draw_window(ui, id),
            );
        }
        *self
            .viewports
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = ids;
    }
}

impl eframe::App for RickUi {
    /// Transparent around Rick's own frame, so its shadow shows the desktop
    /// through the margin; the panel colour otherwise.
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if self.windows.values().any(|w| w.window_frame.is_some()) {
            [0.0; 4]
        } else {
            visuals.panel_fill.to_normalized_gamma_f32()
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let root = self.root;
        let ctx = ui.ctx().clone();
        self.sync_window_ref(frame, &ctx);
        self.draw_window(ui, root);
        self.sync_windows(&ctx);
    }

    fn on_exit(&mut self) {
        if let Err(error) = self.app.save_session() {
            tracing::warn!(%error, "session not saved on exit");
        }
    }
}

impl std::fmt::Debug for RickUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RickUi")
            .field("windows", &self.windows.keys().collect::<Vec<_>>())
            .field("dispatched", &self.dispatched.len())
            .finish_non_exhaustive()
    }
}

/// The icon of a virtual location, for its tab and its Places row.
#[must_use]
pub fn virtual_icon(which: vfs_rick::Virtual) -> &'static str {
    match which {
        vfs_rick::Virtual::Favorites => icons::STAR,
        vfs_rick::Virtual::Recent => icons::RECENT,
        vfs_rick::Virtual::Trash => icons::TRASH,
        vfs_rick::Virtual::Machine => icons::MACHINE,
        vfs_rick::Virtual::Network => icons::NETWORK,
        vfs_rick::Virtual::Cloud => icons::CLOUD,
    }
}

/// Sort directions and columns are used by the palette's labels.
#[must_use]
pub fn column_label(column: Column) -> String {
    match column {
        Column::Name => t!("column-name-lower"),
        Column::Type => t!("column-type-lower"),
        Column::Modified => t!("column-modified-lower"),
        Column::Created => t!("column-created-lower"),
        Column::Accessed => t!("column-accessed-lower"),
        Column::Size => t!("column-size-lower"),
        Column::Items => t!("column-items-lower"),
    }
}
