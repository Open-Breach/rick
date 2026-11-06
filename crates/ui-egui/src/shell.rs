//! The window: top bar, dock of tabs and panes, inspector, operations area,
//! dialogues, palette, toasts, and the keyboard.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use app::{App, Command, Event};
use egui_dock::{DockArea, NodePath, TabViewer};
use ext::{ActionCtx, ActionId, Invocation, Level};
use model::{
    Column, Density, IconSize, JobState, PaneId, PanelSide, SelectionChange, SettingsChange,
    SortDir, Tab, TabId, Theme, ViewMode,
};
use platform::{ControlsSide, PropertiesOutcome};
use types::{EntryId, EntryKind, Location, Progress};

use crate::dnd::{Dnd, DragView, Source, Spring, Target, Zone};
use crate::dock::DockSync;
use crate::host::{Ask, EguiHost};
use crate::keys::{Direction, Intent, KeyMap};
use crate::preview::Previews;
use crate::search::{Pick, SearchBox};
use crate::settings_ui::ShortcutsScreen;
use crate::thumbs::Thumbnails;
use crate::{frame, icons, inspector, menus, pane, paths, settings_ui, side_panel};

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
    /// The location drawn last frame, to notice a change.
    pub last_location: Option<Location>,
    /// Select the entry with this name once the listing is complete.
    pub pending_select: Option<Vec<u8>>,
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
    action: ActionId,
    ctx: ActionCtx,
    input: String,
    focus: bool,
}

/// The eframe application.
pub struct RickUi {
    app: App,
    host: Arc<EguiHost>,
    dock: DockSync,
    keys: KeyMap,
    keys_from: std::collections::BTreeMap<String, String>,
    tabs: HashMap<TabId, TabUi>,
    requests: Vec<Request>,
    dispatched: VecDeque<Dispatched>,
    toasts: Vec<Toast>,
    search: SearchBox,
    dialog: Option<Dialog>,
    settings_open: bool,
    shortcuts: ShortcutsScreen,
    about_open: bool,
    inspector_open: bool,
    properties: Option<(String, Vec<(String, String)>)>,
    /// Read-only texts actions asked to show, each in its own window.
    texts: Vec<TextWindow>,
    next_text: u64,
    /// The tab the pointer was over in the frame being drawn.
    hovered_tab: Option<TabId>,
    /// The icon view's thumbnails.
    thumbnails: Thumbnails,
    /// The egui context, for uploading textures outside a frame callback.
    ctx: egui::Context,
    previews: Previews,
    closed_tabs: Vec<Location>,
    job_done_at: HashMap<types::JobId, Instant>,
    applied: Option<(Theme, u8, Density, Option<String>)>,
    now: SystemTime,
    /// Rick's own window frame as the last frame drew it, if it did; the
    /// window is then cleared to transparent around it.
    window_frame: Option<frame::Frame>,
    /// Drag and drop of entries.
    dnd: Dnd,
    /// Each tab's header button in the last frame, for tests that drop on
    /// one.
    tab_headers: HashMap<TabId, egui::Rect>,
}

/// How many dispatched intents are remembered.
const RECENTS: usize = 64;
/// The title row's height in points: that of a tab.
const TITLE_ROW_HEIGHT: f32 = 30.0;

/// The tab strip's look: VS Code-like padding, rounded top corners, the
/// new-tab button right after the last tab, a scroll bar on overflow.
fn tab_strip_style(egui_style: &egui::Style) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(egui_style);
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
        egui::RichText::new(glyph).family(egui::FontFamily::Name("phosphor-fill".into()))
    } else {
        egui::RichText::new(glyph)
    };
    egui::Button::new(text).frame(false).selected(on)
}
/// How long a notification stays.
const TOAST_FOR: Duration = Duration::from_secs(5);
/// How long a finished job stays listed before it is dismissed.
const DONE_FOR: Duration = Duration::from_secs(4);

impl RickUi {
    /// Wraps `app`; `host` is the `UiHost` the app was built with.
    #[must_use]
    pub fn new(app: App, host: Arc<EguiHost>, ctx: &egui::Context) -> Self {
        icons::install(ctx);
        let wake = ctx.clone();
        app.set_waker(Arc::new(move || wake.request_repaint()));
        let keys = KeyMap::new(app.registry(), &app.model().settings);
        let keys_from = app.model().settings.shortcuts.clone();
        let dock = DockSync::new(app.model());
        Self {
            app,
            host,
            dock,
            keys,
            keys_from,
            tabs: HashMap::new(),
            requests: Vec::new(),
            dispatched: VecDeque::new(),
            toasts: Vec::new(),
            search: SearchBox::default(),
            dialog: None,
            settings_open: false,
            shortcuts: ShortcutsScreen::default(),
            about_open: false,
            inspector_open: false,
            properties: None,
            texts: Vec::new(),
            next_text: 0,
            hovered_tab: None,
            thumbnails: Thumbnails::default(),
            ctx: ctx.clone(),
            previews: Previews::default(),
            closed_tabs: Vec::new(),
            job_done_at: HashMap::new(),
            applied: None,
            now: SystemTime::now(),
            window_frame: None,
            dnd: Dnd::default(),
            tab_headers: HashMap::new(),
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

    /// Whether the inspector pane is shown.
    #[must_use]
    pub fn inspector_open(&self) -> bool {
        self.inspector_open
    }

    /// Whether the title row's search box has the keyboard.
    #[must_use]
    pub fn search_active(&self) -> bool {
        self.search.active()
    }

    /// The search box's text.
    #[must_use]
    pub fn search_text(&self) -> &str {
        &self.search.text
    }

    /// Whether the Settings screen is open.
    #[must_use]
    pub fn settings_open(&self) -> bool {
        self.settings_open
    }

    /// Whether the Keyboard shortcuts screen is open.
    #[must_use]
    pub fn shortcuts_open(&self) -> bool {
        self.shortcuts.open
    }

    /// The question a dialogue is showing, if any.
    #[must_use]
    pub fn asking(&self) -> Option<&Ask> {
        self.dialog.as_ref().map(|d| &d.ask)
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
        self.dock.tab_order(self.app.model(), pane)
    }

    /// Queues an intent as a key press would.
    pub fn queue(&mut self, intent: Intent, pane: PaneId) {
        self.requests.push(Request::Intent { intent, pane });
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
                Event::PreviewReady { pane, doc } => {
                    let entry = self.preview_entry(pane);
                    self.previews.ready(pane, doc, entry);
                }
                Event::PreviewFailed { pane, error } => {
                    let entry = self.preview_entry(pane);
                    self.previews.failed(pane, &error, entry);
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

    /// The content area inside Rick's own window frame in the last frame,
    /// `None` when the system draws the frame or the window is maximised.
    #[must_use]
    pub fn framed_content(&self) -> Option<egui::Rect> {
        self.window_frame.map(|f| f.content())
    }

    /// Each pane's rectangle in the last frame, for tests that drag tabs.
    #[must_use]
    pub fn pane_rects(&self) -> Vec<(PaneId, egui::Rect)> {
        self.dock.pane_rects(self.app.model())
    }

    /// The texts shown right now, as (title, text), for tests.
    #[must_use]
    pub fn shown_texts(&self) -> Vec<(String, String)> {
        self.texts
            .iter()
            .map(|w| (w.title.clone(), w.text.clone()))
            .collect()
    }

    /// The drag in progress, for tests.
    #[must_use]
    pub fn drag(&self) -> Option<DragView> {
        self.dnd.view()
    }

    /// Whether the move/copy/cancel prompt of a drop is up, for tests.
    #[must_use]
    pub fn drop_prompt_open(&self) -> bool {
        self.dnd.prompt_open()
    }

    /// A tab's header button in the last frame, for tests.
    #[must_use]
    pub fn tab_header_rect(&self, tab: TabId) -> Option<egui::Rect> {
        self.tab_headers.get(&tab).copied()
    }

    /// A tab's list scroll offset in the last frame, for tests.
    #[must_use]
    pub fn scroll_offset(&self, tab: TabId) -> f32 {
        self.tabs.get(&tab).map_or(0.0, |t| t.scroll_offset)
    }

    /// The entry the app is previewing in `pane`, if any.
    fn preview_entry(&self, pane: PaneId) -> Option<EntryId> {
        self.app
            .preview_target()
            .filter(|(p, _)| *p == pane)
            .map(|(_, id)| id)
    }

    /// The previews, for tests.
    #[must_use]
    pub fn previews(&self) -> &Previews {
        &self.previews
    }

    fn handle(&mut self, command: Command) {
        let mut events = Vec::new();
        self.app.handle(command, &mut events);
        self.absorb(events);
    }

    fn apply_settings(&mut self, ctx: &egui::Context) {
        let settings = &self.app.model().settings;
        let key = (
            settings.theme,
            settings.font_size,
            settings.density,
            settings.accent.clone(),
        );
        if self.applied.as_ref() != Some(&key) {
            ctx.set_theme(match settings.theme {
                Theme::System => egui::ThemePreference::System,
                Theme::Light => egui::ThemePreference::Light,
                Theme::Dark => egui::ThemePreference::Dark,
            });
            ctx.set_zoom_factor(f32::from(settings.font_size) / 13.0);
            let accent = settings.accent.as_deref().and_then(settings_ui::parse_hex);
            let spacing = match settings.density {
                Density::Compact => 2.0,
                Density::Normal => 4.0,
                Density::Comfortable => 8.0,
            };
            for theme in [egui::Theme::Light, egui::Theme::Dark] {
                ctx.style_mut_of(theme, |style| {
                    let defaults = match theme {
                        egui::Theme::Light => egui::Visuals::light(),
                        egui::Theme::Dark => egui::Visuals::dark(),
                    };
                    style.visuals.selection = defaults.selection;
                    if let Some(color) = accent {
                        style.visuals.selection.bg_fill = color;
                    }
                    style.spacing.item_spacing.y = spacing;
                    style.spacing.button_padding.y = spacing / 2.0 + 1.0;
                });
            }
            self.applied = Some(key);
        }
        if settings.shortcuts != self.keys_from {
            self.keys = KeyMap::new(self.app.registry(), settings);
            self.keys_from = settings.shortcuts.clone();
        }
    }

    /// Turns this frame's key presses into intents, unless a text field, a
    /// dialogue, the palette or the shortcuts screen wants them.
    fn handle_keys(&mut self, ctx: &egui::Context) {
        let pane = self.app.model().focused;
        if self.dialog.is_some() || self.shortcuts.rebinding.is_some() {
            return;
        }
        // An open menu owns the keyboard: Escape closes it, nothing else.
        if self.search.active() || ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx)
        {
            return;
        }
        let filter_text = self
            .app
            .model()
            .active_tab(pane)
            .map(|(_, tab)| tab.filter.clone())
            .unwrap_or_default();
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
                    match self.keys.match_key(*modifiers, *key, *physical_key) {
                        Some(intent) => {
                            intents.push(intent);
                            false
                        }
                        None => true,
                    }
                }
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
        for intent in intents {
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
                            self.tabs.entry(tab_id).or_default().pending_select = Some(name);
                        }
                    }
                }
            }
        }
    }

    /// Starts a drag of `items` out of `pane`'s tab.
    fn start_drag(&mut self, pane: PaneId, tab: TabId, items: &[EntryId], right_button: bool) {
        let Some(state) = self.app.model().tab(tab) else {
            return;
        };
        let entries: Vec<types::Entry> = state
            .listing
            .rows()
            .filter(|e| items.contains(&e.id))
            .cloned()
            .collect();
        self.dnd.start(
            Source::Internal {
                from: pane,
                tab,
                items: entries,
            },
            right_button,
        );
    }

    /// Selects the entries a `SelectNamed` asked for once their listing is
    /// complete: a pinned file's folder was opened, and the file is now in
    /// the list.
    fn resolve_pending_selects(&mut self) {
        let mut found = Vec::new();
        for (tab_id, tab_ui) in &self.tabs {
            let Some(name) = &tab_ui.pending_select else {
                continue;
            };
            let Some(tab) = self.app.model().tab(*tab_id) else {
                continue;
            };
            if !tab.listing.complete {
                continue;
            }
            let hit = tab
                .listing
                .rows()
                .position(|e| e.path.file_name() == Some(name.as_slice()))
                .and_then(|index| tab.listing.row(index).map(|e| (index, e.id)));
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

    fn run_action(&mut self, action: ActionId, pane: PaneId, invoked_from: Invocation) {
        if let Some(ctx) = self.app.action_ctx(pane, invoked_from) {
            self.run_action_with(action, ctx);
        }
    }

    fn run_action_with(&mut self, action: ActionId, ctx: ActionCtx) {
        self.handle(Command::Run {
            action,
            ctx: ctx.clone(),
        });
        if let Some(ask) = self.host.take_asked() {
            let input = match &ask {
                Ask::Prompt { default, .. } => default.clone(),
                Ask::Confirm { .. } => String::new(),
            };
            self.dialog = Some(Dialog {
                ask,
                action,
                ctx,
                input,
                focus: true,
            });
        }
    }

    fn open_entry(&mut self, pane: PaneId, id: EntryId) {
        let Some((_, tab)) = self.app.model().active_tab(pane) else {
            return;
        };
        let Some(entry) = tab.listing.get(id).cloned() else {
            return;
        };
        if entry.kind == EntryKind::Dir {
            let to = Location::new(
                tab.location.provider,
                tab.location.path.join(entry.name.as_bytes()),
            );
            self.handle(Command::Navigate { pane, to });
        } else {
            let ctx = ActionCtx {
                pane,
                location: tab.location.clone(),
                selection: vec![entry],
                invoked_from: Invocation::Shortcut,
            };
            self.run_action_with(ActionId("core.open_default"), ctx);
        }
    }

    fn unavailable(&mut self, what: &str) {
        self.toast(Level::Info, format!("{what} is not part of version 0.1"));
    }

    #[allow(clippy::too_many_lines)]
    fn dispatch(&mut self, intent: Intent, pane: PaneId, ctx: &egui::Context) {
        self.dispatched.push_back(Dispatched { intent, pane });
        while self.dispatched.len() > RECENTS {
            self.dispatched.pop_front();
        }
        match intent {
            Intent::Action(action) => self.run_action(action, pane, Invocation::Shortcut),
            Intent::Open => self.open_selection(pane),
            Intent::Back => self.handle(Command::Back(pane)),
            Intent::Forward => self.handle(Command::Forward(pane)),
            Intent::Refresh => self.handle(Command::Refresh(pane)),
            Intent::Parent => {
                if let Some((_, tab)) = self.app.model().active_tab(pane)
                    && let Some(parent) = paths::parent(&tab.location.path)
                {
                    let to = Location::new(tab.location.provider, parent);
                    self.handle(Command::Navigate { pane, to });
                }
            }
            Intent::Root => {
                if let Some((_, tab)) = self.app.model().active_tab(pane)
                    && let Some(root) = paths::root(&tab.location.path)
                {
                    let to = Location::new(tab.location.provider, root);
                    self.handle(Command::Navigate { pane, to });
                }
            }
            Intent::FocusPath => {
                if let Some((tab_id, tab)) = self.app.model().active_tab(pane) {
                    let text = tab.location.path.to_string_lossy().into_owned();
                    let tab_ui = self.tabs.entry(tab_id).or_default();
                    tab_ui.path_edit = Some(text);
                    tab_ui.focus_path = true;
                }
            }
            Intent::Palette => self.search.focus_with(">"),
            Intent::SelectAll => self.handle(Command::Select {
                pane,
                change: SelectionChange::All,
            }),
            Intent::InvertSelection => self.handle(Command::Select {
                pane,
                change: SelectionChange::Invert,
            }),
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
                    if self
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
                let order = self.dock.tab_order(self.app.model(), pane);
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
                if let Some((tab_id, _)) = self.app.model().active_tab(pane) {
                    self.dock
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
                    self.tabs.entry(tab_id).or_default().focus_filter = true;
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
            Intent::ToggleInspector => self.inspector_open = !self.inspector_open,
            Intent::Settings => self.settings_open = true,
            Intent::Shortcuts => self.shortcuts.open = true,
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
                    self.tabs.entry(tab_id).or_default().reset_columns = true;
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
            Intent::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
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
            Intent::QuickOpen => self.search.focus_with(""),
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
            Intent::SelectByPattern
            | Intent::PasteAsLink
            | Intent::BulkRename
            | Intent::OpenWith
            | Intent::Console
            | Intent::ViewTree => {
                let label = intent.spec().map_or("This", |s| s.label);
                self.unavailable(label);
            }
            Intent::ToggleSidePanel => {
                let visible = self.app.model().settings.side_panel.visible;
                self.handle(Command::Settings(SettingsChange::SidePanelVisible(
                    !visible,
                )));
            }
        }
    }

    fn open_selection(&mut self, pane: PaneId) {
        let Some((_, tab)) = self.app.model().active_tab(pane) else {
            return;
        };
        let selected: Vec<&types::Entry> = tab.selected().collect();
        match selected.as_slice() {
            [] => {
                if !tab.filter.is_empty()
                    && let Some(first) = tab.listing.row(0)
                {
                    let id = first.id;
                    self.open_entry(pane, id);
                }
            }
            [one] if one.kind == EntryKind::Dir => {
                let id = one.id;
                self.open_entry(pane, id);
            }
            _ => self.run_action(ActionId("core.open_default"), pane, Invocation::Shortcut),
        }
    }

    fn inspect(&mut self, pane: PaneId) {
        let Some(ctx) = self.app.action_ctx(pane, Invocation::Shortcut) else {
            return;
        };
        let Some(entry) = ctx.selection.first() else {
            self.toast(Level::Info, "Select an entry to inspect");
            return;
        };
        let Ok(path) = platform::paths::to_path(&entry.path) else {
            return;
        };
        match self.app.platform().show(&path) {
            Ok(PropertiesOutcome::Native) => {}
            Ok(PropertiesOutcome::Rows(rows)) => self.properties = Some((entry.name.clone(), rows)),
            Err(error) => self.toast(Level::Error, format!("Properties: {error}")),
        }
    }

    fn step_tab(&mut self, pane: PaneId, by: isize) {
        let order = self.dock.tab_order(self.app.model(), pane);
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
        let rects = self.dock.pane_rects(self.app.model());
        let Some((_, from)) = rects.iter().find(|(p, _)| *p == pane) else {
            return;
        };
        let from = *from;
        let best = rects
            .iter()
            .filter(|(p, _)| *p != pane)
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
        let count = tab.listing.row_count();
        if count == 0 {
            return;
        }
        let tab_ui = self.tabs.entry(tab_id).or_default();
        let current = tab_ui
            .cursor
            .and_then(|c| tab.listing.rows().position(|e| e.id == c));
        let target = match (by, current) {
            (isize::MIN, _) => 0,
            (isize::MAX, _) => count - 1,
            (by, Some(i)) => (i as isize + by).clamp(0, count as isize - 1) as usize,
            (by, None) if by < 0 => count - 1,
            (_, None) => 0,
        };
        let Some(id) = tab.listing.row(target).map(|e| e.id) else {
            return;
        };
        let change = if extend {
            let anchor = tab_ui.anchor.or(tab_ui.cursor).unwrap_or(id);
            if tab_ui.anchor.is_none() {
                tab_ui.anchor = Some(anchor);
            }
            let anchor_index = tab
                .listing
                .rows()
                .position(|e| e.id == anchor)
                .unwrap_or(target);
            let (a, b) = (anchor_index.min(target), anchor_index.max(target));
            SelectionChange::Set(
                (a..=b)
                    .filter_map(|i| tab.listing.row(i).map(|e| e.id))
                    .collect(),
            )
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
    fn title_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, focused: PaneId) {
        let chrome = self.app.platform().window_chrome();
        let height = TITLE_ROW_HEIGHT;
        egui::Panel::top(egui::Id::new("title"))
            .exact_size(height)
            .show_separator_line(true)
            .show(ui, |ui| {
                let rect = ui.max_rect();
                // The background first, so every widget drawn after it wins
                // the pointer where they overlap.
                let background = ui.interact(
                    rect,
                    egui::Id::new("title-drag"),
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
                menus::burger(&mut left, &mut self.requests, focused);
                let left_edge = left.min_rect().right();

                // Right: window controls, then the toggles.
                let mut right = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                );
                self.title_right(&mut right, ctx, chrome, focused);
                let right_edge = right.min_rect().left();

                self.title_search(ui, rect, left_edge, right_edge, focused);
            });
    }

    /// The side panel on its side with the operations area at its foot, or
    /// the operations area along the window's foot when the panel is hidden.
    fn side_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, focused: PaneId) {
        let side = self.app.model().settings.side_panel.clone();
        if !side.visible {
            if !self.app.model().jobs.is_empty() {
                egui::Panel::bottom(egui::Id::new("operations"))
                    .show(ui, |ui| self.operations_area(ui));
            }
            return;
        }
        let panel = match side.side {
            PanelSide::Left => egui::Panel::left(egui::Id::new("side")),
            PanelSide::Right => egui::Panel::right(egui::Id::new("side")),
        };
        let shown = panel
            .resizable(true)
            .default_size(f32::from(side.width))
            .size_range(120.0..=600.0)
            .show(ui, |ui| {
                if !self.app.model().jobs.is_empty() {
                    egui::Panel::bottom(egui::Id::new("operations"))
                        .show(ui, |ui| self.operations_area(ui));
                }
                side_panel::show(ui, &self.app, focused, &mut self.requests, &mut self.dnd);
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
            self.requests.push(Request::Command(Command::Settings(
                SettingsChange::SidePanelWidth(width),
            )));
        }
    }

    /// Window controls and the layout toggles, right to left.
    fn title_right(
        &mut self,
        right: &mut egui::Ui,
        ctx: &egui::Context,
        chrome: platform::WindowChrome,
        focused: PaneId,
    ) {
        if !chrome.native_controls {
            if right
                .add(egui::Button::new(icons::CLOSE).frame(false))
                .on_hover_text("Close")
                .clicked()
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            let glyph = if maximized {
                icons::RESTORE
            } else {
                icons::MAXIMISE
            };
            if right
                .add(egui::Button::new(glyph).frame(false))
                .on_hover_text(if maximized { "Restore" } else { "Maximise" })
                .clicked()
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            if right
                .add(egui::Button::new(icons::MINIMISE).frame(false))
                .on_hover_text("Minimise")
                .clicked()
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            right.separator();
        }
        if right
            .add(toggle_icon(icons::INSPECTOR, self.inspector_open))
            .on_hover_text("Toggle inspector")
            .clicked()
        {
            self.inspector_open = !self.inspector_open;
        }
        let side_visible = self.app.model().settings.side_panel.visible;
        if right
            .add(toggle_icon(icons::SIDE_PANEL, side_visible))
            .on_hover_text("Toggle side panel")
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
        let pick = self
            .search
            .show(ui, search_rect, &self.app, &self.keys, focused, &recents);
        match pick {
            Some(Pick::Intent(intent)) => self.requests.push(Request::Intent {
                intent,
                pane: focused,
            }),
            Some(Pick::Entry { index, id }) => {
                if let Some((tab_id, _)) = self.app.model().active_tab(focused) {
                    let tab_ui = self.tabs.entry(tab_id).or_default();
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

    fn operations_area(&mut self, ui: &mut egui::Ui) {
        let now = Instant::now();
        let jobs: Vec<(types::JobId, model::JobInfo)> = self
            .app
            .model()
            .jobs
            .iter()
            .map(|(id, info)| (*id, info.clone()))
            .collect();
        for (id, info) in jobs {
            ui.horizontal(|ui| {
                ui.label(&info.name);
                match &info.state {
                    JobState::Running(progress) => {
                        let (fraction, text) = match progress {
                            Progress::Items { done, total } => (
                                total.map_or(0.0, |t| *done as f32 / t.max(1) as f32),
                                format!(
                                    "{done} of {}",
                                    total.map_or("?".to_owned(), |t| t.to_string())
                                ),
                            ),
                            Progress::Bytes { done, total } => (
                                total.map_or(0.0, |t| *done as f32 / t.max(1) as f32),
                                format!(
                                    "{} of {}",
                                    crate::format::size(Some(*done)),
                                    crate::format::size(*total)
                                ),
                            ),
                            Progress::Message(text) => (0.0, text.clone()),
                        };
                        ui.add(
                            egui::ProgressBar::new(fraction)
                                .desired_width(160.0)
                                .text(text),
                        );
                        if ui.small_button("Cancel").clicked() {
                            self.requests.push(Request::Command(Command::CancelJob(id)));
                        }
                    }
                    JobState::Done => {
                        ui.weak("done");
                        let at = *self.job_done_at.entry(id).or_insert(now);
                        if now.duration_since(at) > DONE_FOR
                            || ui.small_button(icons::CLOSE).clicked()
                        {
                            self.requests
                                .push(Request::Command(Command::DismissJob(id)));
                        }
                    }
                    JobState::Cancelled => {
                        ui.weak("cancelled");
                        let at = *self.job_done_at.entry(id).or_insert(now);
                        if now.duration_since(at) > DONE_FOR
                            || ui.small_button(icons::CLOSE).clicked()
                        {
                            self.requests
                                .push(Request::Command(Command::DismissJob(id)));
                        }
                    }
                    JobState::Failed(error) => {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                        if ui.small_button(icons::CLOSE).clicked() {
                            self.requests
                                .push(Request::Command(Command::DismissJob(id)));
                        }
                    }
                }
            });
        }
        self.job_done_at
            .retain(|id, _| self.app.model().jobs.contains_key(id));
    }

    fn show_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        let mut outcome: Option<bool> = None;
        let modal = egui::Modal::new(egui::Id::new("ask")).show(ctx, |ui| {
            ui.set_width(360.0);
            match &dialog.ask {
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
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    outcome = Some(true);
                }
                if ui.button("Cancel").clicked() {
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
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        if !ok {
            self.host.clear_answers();
            return;
        }
        match dialog.ask {
            Ask::Prompt { .. } => self.host.answer_prompt(dialog.input),
            Ask::Confirm { .. } => self.host.answer_confirm(true),
        }
        self.run_action_with(dialog.action, dialog.ctx);
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
}

impl TabViewer for Viewer<'_> {
    type Tab = TabId;

    fn id(&mut self, tab: &mut TabId) -> egui::Id {
        egui::Id::new(("tab", tab.0))
    }

    fn title(&mut self, tab: &mut TabId) -> egui::WidgetText {
        self.app.model().tab(*tab).map_or_else(
            || "?".into(),
            |t| {
                // A folder icon for local folders, the provider's otherwise.
                let icon = if t.location.provider.0 == "local" {
                    icons::FOLDER
                } else {
                    icons::PROVIDER
                };
                format!("{icon} {}", paths::title(&t.location.path)).into()
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
        // the tab.
        if self.dnd.active()
            && let Some(state) = self.app.model().tab(*tab)
        {
            let highlight = self.dnd.zone(Zone {
                key: egui::Id::new(("dnd-tab", tab.0)),
                rect: response.rect,
                target: Target::Folder {
                    to: state.location.clone(),
                    device: crate::dnd::listing_device(state),
                    spring: Some(Spring::Tab(*tab)),
                },
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
        let tab_ui = self.tabs.entry(*tab).or_default();
        let mut cx = PaneCx {
            app: self.app,
            pane,
            tab_id: *tab,
            tab: state,
            tab_ui,
            requests: self.requests,
            keys: self.keys,
            now: self.now,
            focused: model.focused == pane,
            thumbnails: self.thumbnails,
            dnd: self.dnd,
        };
        pane::show(ui, &mut cx);
    }

    fn on_close(&mut self, _tab: &mut TabId) -> egui_dock::widgets::tab_viewer::OnCloseResponse {
        egui_dock::widgets::tab_viewer::OnCloseResponse::Close
    }

    fn on_add(&mut self, _path: NodePath) {
        let model = self.app.model();
        if let Some((_, tab)) = model.active_tab(model.focused) {
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
    /// The window's panels: title row, side panel, inspector and the dock.
    fn panels(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let focused = self.app.model().focused;
        self.title_row(ui, ctx, focused);
        self.apply_requests(ctx);

        self.side_panel(ui, ctx, focused);

        if self.inspector_open {
            egui::Panel::right(egui::Id::new("inspector"))
                .default_size(260.0)
                .show(ui, |ui| {
                    inspector::show(ui, &self.app, focused, self.now, &mut self.previews);
                });
        }

        egui::CentralPanel::no_frame().show(ui, |ui| {
            self.dock.rebuild(self.app.model());
            let hovered_last = self.hovered_tab.take();
            let mut viewer = Viewer {
                app: &self.app,
                tabs: &mut self.tabs,
                requests: &mut self.requests,
                keys: &self.keys,
                now: self.now,
                hovered_last,
                hovered_now: &mut self.hovered_tab,
                thumbnails: &mut self.thumbnails,
                dnd: &mut self.dnd,
                tab_headers: &mut self.tab_headers,
            };
            DockArea::new(&mut self.dock.state)
                .id(egui::Id::new("dock"))
                .style(tab_strip_style(ui.style()))
                .show_add_buttons(true)
                .show_close_buttons(true)
                .show_leaf_close_all_buttons(false)
                .show_leaf_collapse_buttons(false)
                .show_inside(ui, &mut viewer);
        });
        let mut events = Vec::new();
        self.dock
            .reconcile(&mut self.app, &mut events, ctx.pointer_latest_pos());
        self.absorb(events);
        self.apply_requests(ctx);
    }
}

impl eframe::App for RickUi {
    /// Transparent around Rick's own frame, so its shadow shows the desktop
    /// through the margin; the panel colour otherwise.
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if self.window_frame.is_some() {
            [0.0; 4]
        } else {
            visuals.panel_fill.to_normalized_gamma_f32()
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        self.now = SystemTime::now();
        let mut events = Vec::new();
        self.app.poll(&mut events);
        self.absorb(events);
        for (level, text) in self.host.take_notices() {
            self.toast(level, text);
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
        let after = self.app.model().settings.drag.after;
        self.dnd
            .begin_frame(ctx, after, &mut self.tabs, &mut self.requests);
        self.resolve_pending_selects();
        self.handle_keys(ctx);
        self.apply_requests(ctx);

        // Rick's own window frame where the system draws none; the panels
        // go inside it.
        let chrome = self.app.platform().window_chrome();
        self.window_frame =
            frame::wanted(ctx, chrome).then(|| frame::Frame::paint(ui, ui.max_rect()));
        let window_frame = self.window_frame;
        let content = window_frame.map_or(ui.max_rect(), |f| f.content());
        let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(content));
        self.panels(&mut inside, ctx);
        if let Some(window_frame) = window_frame {
            window_frame.grips(ui);
        }
        self.dnd
            .end_frame(ctx, &self.app, &mut self.tabs, &mut self.requests);
        self.dnd.show_prompt(ctx, &mut self.requests);
        self.apply_requests(ctx);

        self.show_dialog(ctx);
        settings_ui::settings_window(
            ctx,
            &mut self.settings_open,
            &self.app.model().settings,
            &mut self.requests,
        );
        self.shortcuts
            .show(ctx, &self.keys, self.app.registry(), &mut self.requests);
        settings_ui::about_window(ctx, &mut self.about_open);
        settings_ui::properties_window(ctx, &mut self.properties);
        self.show_texts(ctx);
        self.show_toasts(ctx);
        self.apply_requests(ctx);

        if self.app.is_busy() || !self.job_done_at.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        // Keep the dock in step before the next frame draws it.
        self.dock.rebuild(self.app.model());
    }
}

impl std::fmt::Debug for RickUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RickUi")
            .field("dock", &self.dock)
            .field("dispatched", &self.dispatched.len())
            .finish_non_exhaustive()
    }
}

/// Sort directions and columns are used by the palette's labels.
#[must_use]
pub fn column_label(column: Column) -> &'static str {
    match column {
        Column::Name => "name",
        Column::Type => "type",
        Column::Modified => "modified",
        Column::Created => "created",
        Column::Accessed => "accessed",
        Column::Size => "size",
        Column::Items => "items",
    }
}
