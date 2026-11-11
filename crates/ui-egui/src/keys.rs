//! The keyboard shortcut table: every row of the spec's tables as an
//! [`Intent`], the default bindings, and the [`KeyMap`] that applies the
//! user's rebindings from the settings.
//!
//! File operations are registered actions and take their defaults from
//! [`Action::default_shortcut`](ext::Action::default_shortcut); everything
//! the UI does itself (navigation, selection, tabs, panes, view) is listed in
//! [`UI_INTENTS`]. Both kinds are rebound through
//! `Settings::shortcuts`, keyed by the intent's id (`core.copy`,
//! `nav.back`), with an empty text meaning unbound.
//!
//! [`rows`] is the table as the Keyboard shortcuts dialogue lists it, each
//! row under its [`Category`]; the [`KeyMap`] keeps that order, which
//! decides what a chord bound twice runs.

use ext::{ActionId, Key, Modifiers, Registry, Shortcut};
use model::{Column, Settings};
use types::ExtId;

/// A direction between panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Left.
    Left,
    /// Right.
    Right,
    /// Up.
    Up,
    /// Down.
    Down,
}

/// Something the user asked the shell to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intent {
    /// A registered action, run on the focused pane's context.
    Action(ActionId),
    /// Enter: a folder navigates, a file opens with the system default.
    Open,
    /// Back in the tab's history.
    Back,
    /// Forward in the tab's history.
    Forward,
    /// The parent directory.
    Parent,
    /// The root of the volume or provider.
    Root,
    /// Edit the path bar.
    FocusPath,
    /// List the location again.
    Refresh,
    /// The command palette.
    Palette,
    /// Quick open by name (not in 0.1).
    QuickOpen,
    /// Select every visible entry.
    SelectAll,
    /// Swap selected and unselected.
    InvertSelection,
    /// Toggle the focused entry in the selection.
    ToggleFocused,
    /// Preview pane: play or pause the media it shows.
    PreviewPlay,
    /// Clear the selection, or the filter when one is set.
    Escape,
    /// System properties of the selection.
    Inspect,
    /// Revert the last reversible operation.
    Undo,
    /// Apply the last undone operation again.
    Redo,
    /// A new tab at the current location.
    NewTab,
    /// Close the active tab.
    CloseTab,
    /// Reopen the most recently closed tab.
    ReopenTab,
    /// A new tab next to the current one at the same location.
    DuplicateTab,
    /// The next tab in the pane.
    NextTab,
    /// The previous tab in the pane.
    PrevTab,
    /// Tab by position, 1 to 9.
    Tab(u8),
    /// Move the active tab one position left.
    MoveTabLeft,
    /// Move the active tab one position right.
    MoveTabRight,
    /// Split the pane to the right.
    SplitRight,
    /// Split the pane downwards.
    SplitDown,
    /// Focus the pane in that direction.
    FocusPane(Direction),
    /// Close every other tab in the pane.
    CloseOtherTabs,
    /// Focus the filter.
    Filter,
    /// Hidden files on or off.
    ToggleHidden,
    /// The pane's time slider on or off.
    ToggleTimeline,
    /// System files on or off.
    ToggleSystem,
    /// File extensions in the name column on or off.
    ToggleExtensions,
    /// Flatten subfolders on or off.
    ToggleFlatten,
    /// Search in subdirectories on or off.
    ToggleSubtree,
    /// The preview pane on or off.
    ToggleInspector,
    /// The Settings screen.
    Settings,
    /// The Keyboard shortcuts screen.
    Shortcuts,
    /// The Extensions screen.
    Extensions,
    /// List view.
    ViewList,
    /// Details view.
    ViewDetails,
    /// Tree view.
    ViewTree,
    /// Icons view (not in 0.1).
    ViewIcons,
    /// Larger icons (not in 0.1).
    IconSizeUp,
    /// Smaller icons (not in 0.1).
    IconSizeDown,
    /// Size every column to its content.
    SizeColumns,
    /// Sort by a column; a second time reverses.
    Sort(Column),
    /// Back to the flat list.
    ClearGrouping,
    /// Full screen on or off.
    Fullscreen,
    /// Quit.
    Quit,
    /// Show or hide the side panel.
    ToggleSidePanel,
    /// A new window with the start layout.
    NewWindow,
    /// Store the current windows and tabs as a named layout.
    SaveLayout,
    /// Back to the last session: "Continue where I left off".
    LastSession,
    /// Switch to the saved layout at this index of `App::layouts`.
    Layout(u16),
    /// Move the list cursor one column to the left, in the List view.
    CursorLeft,
    /// Move the list cursor one column to the right, in the List view.
    CursorRight,
    /// Move the list cursor one row up.
    CursorUp,
    /// Move the list cursor one row down.
    CursorDown,
    /// Move the list cursor a page up.
    CursorPageUp,
    /// Move the list cursor a page down.
    CursorPageDown,
    /// Move the list cursor to the first row.
    CursorHome,
    /// Move the list cursor to the last row.
    CursorEnd,
    /// Extend the selection one row up.
    ExtendUp,
    /// Extend the selection one row down.
    ExtendDown,
    /// Extend the selection a page up.
    ExtendPageUp,
    /// Extend the selection a page down.
    ExtendPageDown,
    /// Extend the selection to the first row.
    ExtendHome,
    /// Extend the selection to the last row.
    ExtendEnd,
}

/// One row of the UI's part of the shortcut table.
#[derive(Debug, Clone, Copy)]
pub struct IntentSpec {
    /// The intent.
    pub intent: Intent,
    /// Stable id, the key in `Settings::shortcuts`.
    pub id: &'static str,
    /// Text for menus, the palette and the shortcuts screen.
    pub label: &'static str,
    /// The spec table the row comes from.
    pub group: &'static str,
    /// Default bindings in the spec's text form.
    pub defaults: &'static [&'static str],
}

const fn spec(
    intent: Intent,
    id: &'static str,
    label: &'static str,
    group: &'static str,
    defaults: &'static [&'static str],
) -> IntentSpec {
    IntentSpec {
        intent,
        id,
        label,
        group,
        defaults,
    }
}

const NAV: &str = "Navigation";
const SEL: &str = "Selection";
const OPS: &str = "File operations";
const TABS: &str = "Tabs and panes";
const VIEW: &str = "View";
const WINDOW: &str = "Window";
const LIST: &str = "List";
/// The preview pane's own table: bound while the Preview tab has the
/// focus, where its rows win over the list's.
pub const PREVIEW: &str = "Preview pane";

/// A shortcut as the user reads it in a menu or a hint: the canonical
/// text with `Mod` written as the platform's key, *Ctrl* or *Cmd*
/// (SPEC.md *Keyboard shortcuts*). The keymap file keeps `Mod`.
#[must_use]
pub fn spelled(shortcut: Shortcut) -> String {
    let text = shortcut.text();
    match text.strip_prefix("Mod+") {
        Some(rest) => format!("{}+{rest}", platform::MOD_KEY),
        None => text,
    }
}

/// Every intent the UI handles itself, with the spec's default bindings.
/// File operations are not here: they come from the action registry.
pub const UI_INTENTS: &[IntentSpec] = &[
    spec(Intent::Open, "nav.open", "Open", NAV, &["Enter"]),
    spec(
        Intent::Back,
        "nav.back",
        "Back",
        NAV,
        &["Backspace", "Alt+Left"],
    ),
    spec(
        Intent::Forward,
        "nav.forward",
        "Forward",
        NAV,
        &["Alt+Right"],
    ),
    spec(
        Intent::Parent,
        "nav.parent",
        "Go to parent",
        NAV,
        &["Alt+Up"],
    ),
    spec(
        Intent::Root,
        "nav.root",
        "Go to root",
        NAV,
        &["Mod+Shift+Home"],
    ),
    spec(
        Intent::FocusPath,
        "nav.path",
        "Focus the path bar",
        NAV,
        &["Mod+L"],
    ),
    spec(
        Intent::Refresh,
        "nav.refresh",
        "Refresh",
        NAV,
        &["F5", "Mod+R"],
    ),
    spec(
        Intent::Palette,
        "nav.palette",
        "Search commands",
        NAV,
        &["Mod+Shift+P"],
    ),
    spec(
        Intent::QuickOpen,
        "nav.quick_open",
        "Search by name",
        NAV,
        &["Mod+P"],
    ),
    spec(Intent::SelectAll, "sel.all", "Select all", SEL, &["Mod+A"]),
    spec(
        Intent::InvertSelection,
        "sel.invert",
        "Invert selection",
        SEL,
        &["Mod+Shift+A"],
    ),
    spec(
        Intent::ToggleFocused,
        "sel.toggle",
        "Toggle selection of the focused entry",
        SEL,
        &["Space"],
    ),
    spec(
        Intent::PreviewPlay,
        "preview.play",
        "Play / pause media",
        PREVIEW,
        &["Space"],
    ),
    spec(
        Intent::Escape,
        "sel.escape",
        "Clear selection / close filter",
        SEL,
        &["Escape"],
    ),
    spec(
        Intent::Inspect,
        "ops.inspect",
        "Inspect",
        OPS,
        &["Mod+I", "Alt+Enter"],
    ),
    spec(Intent::Undo, "ops.undo", "Undo", OPS, &["Mod+Z"]),
    spec(
        Intent::Redo,
        "ops.redo",
        "Redo",
        OPS,
        &["Mod+Shift+Z", "Mod+Y"],
    ),
    spec(Intent::NewTab, "tabs.new", "New tab", TABS, &["Mod+T"]),
    spec(
        Intent::CloseTab,
        "tabs.close",
        "Close tab",
        TABS,
        &["Mod+W"],
    ),
    spec(
        Intent::ReopenTab,
        "tabs.reopen",
        "Reopen closed tab",
        TABS,
        &["Mod+Shift+T"],
    ),
    spec(
        Intent::DuplicateTab,
        "tabs.duplicate",
        "Duplicate tab",
        TABS,
        &["Mod+Shift+D"],
    ),
    spec(
        Intent::NextTab,
        "tabs.next",
        "Next tab",
        TABS,
        &["Mod+Tab", "Mod+PageDown"],
    ),
    spec(
        Intent::PrevTab,
        "tabs.prev",
        "Previous tab",
        TABS,
        &["Mod+Shift+Tab", "Mod+PageUp"],
    ),
    spec(Intent::Tab(1), "tabs.1", "Tab 1", TABS, &["Mod+1"]),
    spec(Intent::Tab(2), "tabs.2", "Tab 2", TABS, &["Mod+2"]),
    spec(Intent::Tab(3), "tabs.3", "Tab 3", TABS, &["Mod+3"]),
    spec(Intent::Tab(4), "tabs.4", "Tab 4", TABS, &["Mod+4"]),
    spec(Intent::Tab(5), "tabs.5", "Tab 5", TABS, &["Mod+5"]),
    spec(Intent::Tab(6), "tabs.6", "Tab 6", TABS, &["Mod+6"]),
    spec(Intent::Tab(7), "tabs.7", "Tab 7", TABS, &["Mod+7"]),
    spec(Intent::Tab(8), "tabs.8", "Tab 8", TABS, &["Mod+8"]),
    spec(Intent::Tab(9), "tabs.9", "Tab 9", TABS, &["Mod+9"]),
    spec(
        Intent::MoveTabLeft,
        "tabs.move_left",
        "Move tab left",
        TABS,
        &["Mod+Shift+PageUp"],
    ),
    spec(
        Intent::MoveTabRight,
        "tabs.move_right",
        "Move tab right",
        TABS,
        &["Mod+Shift+PageDown"],
    ),
    spec(
        Intent::SplitRight,
        "tabs.split_right",
        "Split right",
        TABS,
        &["Mod+\\"],
    ),
    spec(
        Intent::SplitDown,
        "tabs.split_down",
        "Split down",
        TABS,
        &["Mod+Shift+\\"],
    ),
    spec(
        Intent::FocusPane(Direction::Left),
        "tabs.focus_left",
        "Focus pane to the left",
        TABS,
        &["Mod+Alt+Left"],
    ),
    spec(
        Intent::FocusPane(Direction::Right),
        "tabs.focus_right",
        "Focus pane to the right",
        TABS,
        &["Mod+Alt+Right"],
    ),
    spec(
        Intent::FocusPane(Direction::Up),
        "tabs.focus_up",
        "Focus pane above",
        TABS,
        &["Mod+Alt+Up"],
    ),
    spec(
        Intent::FocusPane(Direction::Down),
        "tabs.focus_down",
        "Focus pane below",
        TABS,
        &["Mod+Alt+Down"],
    ),
    spec(
        Intent::CloseOtherTabs,
        "tabs.close_others",
        "Close other tabs",
        TABS,
        &["Mod+Shift+W"],
    ),
    spec(Intent::Filter, "view.filter", "Filter", VIEW, &["Mod+F"]),
    spec(
        Intent::ToggleHidden,
        "view.hidden",
        "Toggle hidden files",
        VIEW,
        &["Mod+H"],
    ),
    spec(
        Intent::ToggleTimeline,
        "view.timeline",
        "Toggle timeline",
        VIEW,
        &[],
    ),
    spec(
        Intent::ToggleSystem,
        "view.system",
        "Toggle system files",
        VIEW,
        &["Mod+Shift+H"],
    ),
    spec(
        Intent::ToggleExtensions,
        "view.extensions",
        "Toggle file extensions",
        VIEW,
        &["Mod+E"],
    ),
    // The spec's View table gives Mod+Shift+F to Flatten, its Main area to
    // Search in subdirectories; issue A of the close-out settles it on the
    // search (QUESTIONS.md), so Flatten keeps no default binding.
    spec(
        Intent::ToggleFlatten,
        "view.flatten",
        "Toggle flatten subfolders",
        VIEW,
        &[],
    ),
    spec(
        Intent::ToggleSubtree,
        "view.subtree",
        "Search in subdirectories",
        VIEW,
        &["Mod+Shift+F"],
    ),
    spec(
        Intent::ToggleInspector,
        "view.inspector",
        "Toggle preview pane",
        VIEW,
        &["Mod+Shift+I"],
    ),
    spec(
        Intent::Settings,
        "view.settings",
        "Settings",
        VIEW,
        &["Mod+,"],
    ),
    spec(
        Intent::Shortcuts,
        "view.shortcuts",
        "Keyboard shortcuts",
        VIEW,
        &[],
    ),
    spec(Intent::Extensions, "view.plugins", "Extensions", VIEW, &[]),
    spec(
        Intent::ViewList,
        "view.list",
        "List view",
        VIEW,
        &["Mod+Shift+1"],
    ),
    spec(
        Intent::ViewDetails,
        "view.details",
        "Details view",
        VIEW,
        &["Mod+Shift+2"],
    ),
    spec(
        Intent::ViewTree,
        "view.tree",
        "Tree view",
        VIEW,
        &["Mod+Shift+3"],
    ),
    spec(
        Intent::ViewIcons,
        "view.icons",
        "Icons view",
        VIEW,
        &["Mod+Shift+4"],
    ),
    spec(
        Intent::IconSizeUp,
        "view.icons_larger",
        "Larger icons",
        VIEW,
        &["Mod+Plus"],
    ),
    spec(
        Intent::IconSizeDown,
        "view.icons_smaller",
        "Smaller icons",
        VIEW,
        &["Mod+Minus"],
    ),
    spec(
        Intent::SizeColumns,
        "view.size_columns",
        "Size columns to fit",
        VIEW,
        &["Mod+Shift+R"],
    ),
    spec(
        Intent::Sort(Column::Name),
        "view.sort_name",
        "Sort by name",
        VIEW,
        &["Mod+Alt+N"],
    ),
    spec(
        Intent::Sort(Column::Size),
        "view.sort_size",
        "Sort by size",
        VIEW,
        &["Mod+Alt+S"],
    ),
    spec(
        Intent::Sort(Column::Modified),
        "view.sort_modified",
        "Sort by modified",
        VIEW,
        &["Mod+Alt+M"],
    ),
    spec(
        Intent::Sort(Column::Type),
        "view.sort_type",
        "Sort by type",
        VIEW,
        &["Mod+Alt+T"],
    ),
    spec(
        Intent::ClearGrouping,
        "view.clear_grouping",
        "Clear grouping",
        VIEW,
        &["Mod+Alt+G"],
    ),
    spec(
        Intent::Fullscreen,
        "window.fullscreen",
        "Full screen",
        WINDOW,
        &["F11"],
    ),
    spec(Intent::Quit, "window.quit", "Quit", WINDOW, &["Mod+Q"]),
    spec(
        Intent::NewWindow,
        "window.new",
        "New window",
        WINDOW,
        &["Mod+N"],
    ),
    spec(
        Intent::SaveLayout,
        "window.save_layout",
        "Save layout as…",
        WINDOW,
        &[],
    ),
    spec(
        Intent::LastSession,
        "window.last_session",
        "Continue where I left off",
        WINDOW,
        &[],
    ),
    spec(
        Intent::ToggleSidePanel,
        "window.side_panel",
        "Toggle side panel",
        WINDOW,
        &["Mod+B"],
    ),
    spec(Intent::CursorUp, "list.up", "Cursor up", LIST, &["Up"]),
    spec(
        Intent::CursorLeft,
        "list.left",
        "Cursor one column left",
        LIST,
        &["Left"],
    ),
    spec(
        Intent::CursorRight,
        "list.right",
        "Cursor one column right",
        LIST,
        &["Right"],
    ),
    spec(
        Intent::CursorDown,
        "list.down",
        "Cursor down",
        LIST,
        &["Down"],
    ),
    spec(
        Intent::CursorPageUp,
        "list.page_up",
        "Cursor page up",
        LIST,
        &["PageUp"],
    ),
    spec(
        Intent::CursorPageDown,
        "list.page_down",
        "Cursor page down",
        LIST,
        &["PageDown"],
    ),
    spec(
        Intent::CursorHome,
        "list.home",
        "Cursor to first",
        LIST,
        &["Home"],
    ),
    spec(
        Intent::CursorEnd,
        "list.end",
        "Cursor to last",
        LIST,
        &["End"],
    ),
    spec(
        Intent::ExtendUp,
        "list.extend_up",
        "Extend selection up",
        LIST,
        &["Shift+Up"],
    ),
    spec(
        Intent::ExtendDown,
        "list.extend_down",
        "Extend selection down",
        LIST,
        &["Shift+Down"],
    ),
    spec(
        Intent::ExtendPageUp,
        "list.extend_page_up",
        "Extend selection a page up",
        LIST,
        &["Shift+PageUp"],
    ),
    spec(
        Intent::ExtendPageDown,
        "list.extend_page_down",
        "Extend selection a page down",
        LIST,
        &["Shift+PageDown"],
    ),
    spec(
        Intent::ExtendHome,
        "list.extend_home",
        "Extend selection to first",
        LIST,
        &["Shift+Home"],
    ),
    spec(
        Intent::ExtendEnd,
        "list.extend_end",
        "Extend selection to last",
        LIST,
        &["Shift+End"],
    ),
];

impl Intent {
    /// The row of [`UI_INTENTS`] for a UI intent; `None` for actions and
    /// for the saved layouts, which the palette lists by name.
    #[must_use]
    pub fn spec(self) -> Option<&'static IntentSpec> {
        UI_INTENTS.iter().find(|s| s.intent == self)
    }

    /// The id used in `Settings::shortcuts`: the action id or the row id.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Action(id) => id.0,
            Self::Layout(_) => "window.layout",
            other => other.spec().map_or("?", |s| s.id),
        }
    }

    /// The intent with this id, from the UI table or the registry.
    #[must_use]
    pub fn from_id(id: &str, registry: &Registry) -> Option<Self> {
        if let Some(spec) = UI_INTENTS.iter().find(|s| s.id == id) {
            return Some(spec.intent);
        }
        registry
            .actions()
            .iter()
            .find(|a| a.id().0 == id)
            .map(|a| Self::Action(a.id()))
    }
}

/// Where the Keyboard shortcuts dialogue files a row (SPEC.md *Keyboard
/// shortcuts dialogue*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// Navigation.
    Navigation,
    /// File operations: the kernel's actions, with inspect, undo and redo.
    FileOperations,
    /// View.
    View,
    /// Selection.
    Selection,
    /// Panes & tabs, the window commands among them.
    Panes,
    /// Search: the search box in both its modes, the filter and the
    /// search in subdirectories.
    Search,
    /// The preview pane's own table.
    Preview,
    /// The list's cursor keys: bound, and rebound in the file, but not
    /// listed in the dialogue.
    List,
    /// A plugin's actions, filed under the plugin and nowhere else.
    Plugin {
        /// The plugin's id.
        id: ExtId,
        /// Its name, as its manifest gives it.
        name: &'static str,
    },
}

/// The categories of Rick's own rows, in the order of the dialogue's
/// list; the plugins' follow, by name.
const BUILT_IN: [Category; 8] = [
    Category::Navigation,
    Category::FileOperations,
    Category::View,
    Category::Selection,
    Category::Panes,
    Category::Search,
    Category::Preview,
    Category::List,
];

impl IntentSpec {
    /// The category the row is filed under: its table's, with the window
    /// commands among *Panes & tabs* and the search commands — the search
    /// box in both modes, the filter, the search in subdirectories —
    /// under *Search*.
    #[must_use]
    pub fn category(&self) -> Category {
        match self.intent {
            Intent::Palette | Intent::QuickOpen | Intent::Filter | Intent::ToggleSubtree => {
                return Category::Search;
            }
            _ => {}
        }
        match self.group {
            NAV => Category::Navigation,
            SEL => Category::Selection,
            OPS => Category::FileOperations,
            TABS | WINDOW => Category::Panes,
            VIEW => Category::View,
            PREVIEW => Category::Preview,
            _ => Category::List,
        }
    }
}

/// One row of the shortcut table: what it does, what it is called, where
/// it is filed and what it is bound to before any rebinding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The intent.
    pub intent: Intent,
    /// Its name: the action's label, or the table row's.
    pub label: String,
    /// The category it is filed under.
    pub category: Category,
    /// The default bindings.
    pub defaults: Vec<Shortcut>,
}

/// Every intent that takes a shortcut, in the order of the Keyboard
/// shortcuts dialogue's list: Rick's own categories first — the kernel's
/// actions at the head of *File operations* — then the plugins' actions,
/// plugin by plugin in the order of their names. `plugin_of` gives the id
/// and the name of the plugin that registered an action, `None` for the
/// kernel's own, so an action is filed under its plugin and nowhere else.
#[must_use]
pub fn rows(
    registry: &Registry,
    plugin_of: &dyn Fn(ActionId) -> Option<(ExtId, &'static str)>,
) -> Vec<Row> {
    let mut core = Vec::new();
    let mut plugins = Vec::new();
    for action in registry.actions() {
        let plugin = plugin_of(action.id());
        let row = Row {
            intent: Intent::Action(action.id()),
            label: action.label(),
            category: plugin.map_or(Category::FileOperations, |(id, name)| Category::Plugin {
                id,
                name,
            }),
            defaults: action.default_shortcut().into_iter().collect(),
        };
        if plugin.is_some() {
            plugins.push(row);
        } else {
            core.push(row);
        }
    }
    let mut rows = Vec::new();
    for category in BUILT_IN {
        if category == Category::FileOperations {
            rows.append(&mut core);
        }
        rows.extend(
            UI_INTENTS
                .iter()
                .filter(|spec| spec.category() == category)
                .map(|spec| Row {
                    intent: spec.intent,
                    label: spec.label.to_owned(),
                    category,
                    defaults: spec
                        .defaults
                        .iter()
                        .filter_map(|text| text.parse().ok())
                        .collect(),
                }),
        );
    }
    // By name, and as registered within a plugin.
    plugins.sort_by_key(|row| match row.category {
        Category::Plugin { id, name } => (name.to_lowercase(), id),
        _ => (String::new(), ExtId("")),
    });
    rows.append(&mut plugins);
    rows
}

/// The rows of an application: [`rows`] with every plugin's actions
/// filed under the plugin, as the application recorded where each action
/// came from.
#[must_use]
pub fn rows_of(app: &app::App) -> Vec<Row> {
    rows(app.registry(), &|action| {
        app.action_extension(action)
            .map(|manifest| (manifest.id, manifest.name))
    })
}

/// One binding and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// The keys.
    pub shortcut: Shortcut,
    /// What they do.
    pub intent: Intent,
    /// A row of the Preview pane table: live only while the Preview tab
    /// has the focus, where it wins over any other row of the same keys.
    pub preview: bool,
}

/// Whether an intent is a row of the Preview pane table.
#[must_use]
pub fn is_preview(intent: Intent) -> bool {
    intent.spec().is_some_and(|s| s.group == PREVIEW)
}

/// The live shortcut table: defaults with the user's rebindings applied.
#[derive(Debug, Clone, Default)]
pub struct KeyMap {
    bindings: Vec<Binding>,
}

impl KeyMap {
    /// Defaults from the registry's actions and [`UI_INTENTS`], then the
    /// settings' rebindings: a text replaces the defaults of its id, an
    /// empty text unbinds it, and of a text only the chords that parse
    /// count. Every action counts as the kernel's own; [`of`](Self::of)
    /// is the table when plugins are told apart.
    #[must_use]
    pub fn new(registry: &Registry, settings: &Settings) -> Self {
        Self::of(&rows(registry, &|_| None), settings)
    }

    /// The live table of `rows`, in their order, with the settings'
    /// rebindings applied. The order is what a chord bound to two rows
    /// runs: the first of them, so Rick's own action before a plugin's
    /// and otherwise the one higher up in the dialogue's list.
    #[must_use]
    pub fn of(rows: &[Row], settings: &Settings) -> Self {
        let mut bindings = Vec::new();
        for row in rows {
            let preview = row.category == Category::Preview;
            let rebound = settings.shortcuts.get(row.intent.id()).map(|text| {
                text.split(',')
                    .map(str::trim)
                    .filter(|chord| !chord.is_empty())
                    .filter_map(|chord| chord.parse().ok())
                    .collect::<Vec<Shortcut>>()
            });
            let shortcuts = rebound.unwrap_or_else(|| row.defaults.clone());
            bindings.extend(shortcuts.into_iter().map(|shortcut| Binding {
                shortcut,
                intent: row.intent,
                preview,
            }));
        }
        Self { bindings }
    }

    /// Every binding, in table order.
    #[must_use]
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// The bindings of one intent.
    pub fn shortcuts_for(&self, intent: Intent) -> impl Iterator<Item = Shortcut> + '_ {
        self.bindings
            .iter()
            .filter(move |b| b.intent == intent)
            .map(|b| b.shortcut)
    }

    /// The other intents bound to a chord of `intent`, each once and in
    /// table order: what the Keyboard shortcuts dialogue names in the
    /// *also:* chip. The preview pane's table is bound only while the
    /// preview has the focus, so a chord there and the same chord in the
    /// list are not in conflict.
    #[must_use]
    pub fn conflicts(&self, intent: Intent) -> Vec<Intent> {
        let mut others = Vec::new();
        for own in self.bindings.iter().filter(|b| b.intent == intent) {
            for other in &self.bindings {
                if other.intent != intent
                    && other.shortcut == own.shortcut
                    && other.preview == own.preview
                    && !others.contains(&other.intent)
                {
                    others.push(other.intent);
                }
            }
        }
        others
    }

    /// The first binding of an intent as a hint for menus, spelled as the
    /// platform spells it ([`spelled`]); `None` when it is unbound.
    #[must_use]
    pub fn text_for(&self, intent: Intent) -> Option<String> {
        self.shortcuts_for(intent).next().map(spelled)
    }

    /// What this shortcut does in an explorer, if anything.
    #[must_use]
    pub fn lookup(&self, shortcut: Shortcut) -> Option<Intent> {
        self.lookup_in(shortcut, false)
    }

    /// What this shortcut does, in the preview pane or an explorer: in the
    /// preview pane its own table is tried first; in an explorer that
    /// table is not bound.
    #[must_use]
    pub fn lookup_in(&self, shortcut: Shortcut, in_preview: bool) -> Option<Intent> {
        if in_preview {
            self.bindings
                .iter()
                .filter(|b| b.preview)
                .chain(self.bindings.iter().filter(|b| !b.preview))
                .find(|b| b.shortcut == shortcut)
                .map(|b| b.intent)
        } else {
            self.bindings
                .iter()
                .find(|b| !b.preview && b.shortcut == shortcut)
                .map(|b| b.intent)
        }
    }

    /// What a key press does: the logical key is tried first, then the
    /// physical one, so `Mod+Shift+1` works where Shift+1 types `!`.
    /// `in_preview` says whether the Preview tab takes the keys.
    #[must_use]
    pub fn match_key(
        &self,
        modifiers: egui::Modifiers,
        key: egui::Key,
        physical: Option<egui::Key>,
        in_preview: bool,
    ) -> Option<Intent> {
        let mods = from_egui_modifiers(modifiers);
        from_egui_key(key)
            .and_then(|key| {
                self.lookup_in(
                    Shortcut {
                        modifiers: mods,
                        key,
                    },
                    in_preview,
                )
            })
            .or_else(|| {
                physical.and_then(from_egui_key).and_then(|key| {
                    self.lookup_in(
                        Shortcut {
                            modifiers: mods,
                            key,
                        },
                        in_preview,
                    )
                })
            })
    }
}

/// Our modifiers for egui's; Mod is Cmd on macOS and Ctrl elsewhere.
#[must_use]
pub fn from_egui_modifiers(m: egui::Modifiers) -> Modifiers {
    let mut out = Modifiers::empty();
    if m.command {
        out |= Modifiers::MOD;
    }
    if m.shift {
        out |= Modifiers::SHIFT;
    }
    if m.alt {
        out |= Modifiers::ALT;
    }
    out
}

/// egui's modifiers for ours.
#[must_use]
pub fn to_egui_modifiers(m: Modifiers) -> egui::Modifiers {
    let mut out = egui::Modifiers::NONE;
    if m.contains(Modifiers::MOD) {
        out = out.plus(egui::Modifiers::COMMAND);
    }
    if m.contains(Modifiers::SHIFT) {
        out = out.plus(egui::Modifiers::SHIFT);
    }
    if m.contains(Modifiers::ALT) {
        out = out.plus(egui::Modifiers::ALT);
    }
    out
}

/// Our key for egui's, for the keys a shortcut can name.
#[must_use]
pub fn from_egui_key(key: egui::Key) -> Option<Key> {
    use egui::Key as K;
    Some(match key {
        K::Enter => Key::Enter,
        K::Escape => Key::Escape,
        K::Tab => Key::Tab,
        K::Backspace => Key::Backspace,
        K::Delete => Key::Delete,
        K::Insert => Key::Insert,
        K::Home => Key::Home,
        K::End => Key::End,
        K::PageUp => Key::PageUp,
        K::PageDown => Key::PageDown,
        K::ArrowUp => Key::Up,
        K::ArrowDown => Key::Down,
        K::ArrowLeft => Key::Left,
        K::ArrowRight => Key::Right,
        K::Space => Key::Space,
        K::Comma => Key::Char(','),
        K::Period => Key::Char('.'),
        K::Plus => Key::Char('+'),
        K::Minus => Key::Char('-'),
        K::Backslash => Key::Char('\\'),
        K::Slash => Key::Char('/'),
        K::Backtick => Key::Char('`'),
        K::Semicolon => Key::Char(';'),
        K::Equals => Key::Char('='),
        K::OpenBracket => Key::Char('['),
        K::CloseBracket => Key::Char(']'),
        K::Quote => Key::Char('\''),
        other => {
            let name = other.name();
            if let Some(n) = name.strip_prefix('F')
                && let Ok(n) = n.parse::<u8>()
            {
                return (1..=24).contains(&n).then_some(Key::F(n));
            }
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => Key::Char(c.to_ascii_lowercase()),
                _ => return None,
            }
        }
    })
}

/// egui's key for ours.
#[must_use]
pub fn to_egui_key(key: Key) -> Option<egui::Key> {
    use egui::Key as K;
    Some(match key {
        Key::Enter => K::Enter,
        Key::Escape => K::Escape,
        Key::Tab => K::Tab,
        Key::Backspace => K::Backspace,
        Key::Delete => K::Delete,
        Key::Insert => K::Insert,
        Key::Home => K::Home,
        Key::End => K::End,
        Key::PageUp => K::PageUp,
        Key::PageDown => K::PageDown,
        Key::Up => K::ArrowUp,
        Key::Down => K::ArrowDown,
        Key::Left => K::ArrowLeft,
        Key::Right => K::ArrowRight,
        Key::Space => K::Space,
        Key::F(n) => K::from_name(&format!("F{n}"))?,
        Key::Char(',') => K::Comma,
        Key::Char('.') => K::Period,
        Key::Char('+') => K::Plus,
        Key::Char('-') => K::Minus,
        Key::Char('\\') => K::Backslash,
        Key::Char('/') => K::Slash,
        Key::Char('`') => K::Backtick,
        Key::Char(';') => K::Semicolon,
        Key::Char('=') => K::Equals,
        Key::Char('[') => K::OpenBracket,
        Key::Char(']') => K::CloseBracket,
        Key::Char('\'') => K::Quote,
        Key::Char(c) => K::from_name(&c.to_ascii_uppercase().to_string())?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_in_the_table_parses_and_round_trips_through_egui() {
        for spec in UI_INTENTS {
            for text in spec.defaults {
                let shortcut: Shortcut = text.parse().unwrap_or_else(|e| panic!("{text}: {e}"));
                let key =
                    to_egui_key(shortcut.key).unwrap_or_else(|| panic!("{text}: no egui key"));
                assert_eq!(from_egui_key(key), Some(shortcut.key), "{text}");
                let mods = to_egui_modifiers(shortcut.modifiers);
                assert_eq!(from_egui_modifiers(mods), shortcut.modifiers, "{text}");
            }
        }
    }

    #[test]
    fn ids_are_unique_and_match_their_intent() {
        let mut ids: Vec<&str> = UI_INTENTS.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), UI_INTENTS.len());
        for spec in UI_INTENTS {
            assert_eq!(spec.intent.id(), spec.id);
            assert_eq!(spec.intent.spec().map(|s| s.id), Some(spec.id));
        }
    }

    #[test]
    fn settings_rebind_and_unbind() {
        let registry = Registry::new();
        let mut settings = Settings::default();
        let map = KeyMap::new(&registry, &settings);
        let back: Vec<String> = map.shortcuts_for(Intent::Back).map(|s| s.text()).collect();
        assert_eq!(back, ["Backspace", "Alt+Left"]);
        assert_eq!(
            map.lookup("Mod+Shift+P".parse().unwrap_or_else(|e| panic!("{e}"))),
            Some(Intent::Palette)
        );

        settings
            .shortcuts
            .insert("nav.back".into(), "Mod+Left, Mod+[".into());
        settings
            .shortcuts
            .insert("nav.palette".into(), String::new());
        settings.shortcuts.insert("bogus".into(), "F1".into());
        let map = KeyMap::new(&registry, &settings);
        let back: Vec<String> = map.shortcuts_for(Intent::Back).map(|s| s.text()).collect();
        assert_eq!(back, ["Mod+Left", "Mod+["]);
        assert_eq!(map.shortcuts_for(Intent::Palette).count(), 0);
        // A menu's hint is the first chord, spelled as the platform does.
        assert_eq!(
            map.text_for(Intent::Back),
            Some(format!("{}+Left", platform::MOD_KEY))
        );
        assert_eq!(map.text_for(Intent::Palette), None);
    }

    #[test]
    fn a_hint_spells_mod_as_the_platform_s_key() {
        let key = |text: &str| -> Shortcut { text.parse().unwrap_or_else(|e| panic!("{e}")) };
        let name = platform::MOD_KEY;
        assert!(name == "Ctrl" || name == "Cmd", "{name}");
        assert_eq!(spelled(key("Mod+N")), format!("{name}+N"));
        assert_eq!(spelled(key("Mod+Shift+P")), format!("{name}+Shift+P"));
        assert_eq!(spelled(key("Mod+,")), format!("{name}+,"));
        // Without Mod nothing changes, and the keymap's own text keeps it.
        assert_eq!(spelled(key("Alt+Left")), "Alt+Left");
        assert_eq!(spelled(key("F2")), "F2");
        assert_eq!(key("Mod+N").text(), "Mod+N");
    }

    /// An action for the table: an id, a default chord, a label.
    struct Act(&'static str, Option<&'static str>);

    impl ext::Action for Act {
        fn id(&self) -> ActionId {
            ActionId(self.0)
        }
        fn label(&self) -> String {
            self.0.to_owned()
        }
        fn group(&self) -> ext::ActionGroup {
            ext::ActionGroup::Core
        }
        fn default_shortcut(&self) -> Option<Shortcut> {
            self.1.and_then(|text| text.parse().ok())
        }
        fn availability(&self, _: &ext::ActionCtx) -> ext::Availability {
            ext::Availability::Enabled
        }
        fn run(
            &self,
            _: ext::ActionCtx,
            _: &runtime::JobRunner,
            _: &dyn ext::UiHost,
        ) -> Result<ext::ActionOutcome, types::Error> {
            Ok(ext::ActionOutcome::Done)
        }
    }

    /// A registry with the kernel's `core.copy` and two plugins' actions,
    /// and who registered which.
    fn with_plugins() -> (Registry, impl Fn(ActionId) -> Option<(ExtId, &'static str)>) {
        let mut registry = Registry::new();
        // Registered before the kernel's own, to show that the order of
        // the list does not follow it.
        registry.action(std::sync::Arc::new(Act("zip.pack", Some("Mod+C"))));
        registry.action(std::sync::Arc::new(Act("core.copy", Some("Mod+C"))));
        registry.action(std::sync::Arc::new(Act("git.commit", None)));
        registry.action(std::sync::Arc::new(Act("git.push", Some("Mod+T"))));
        let plugin_of = |action: ActionId| match action.0.split_once('.') {
            Some(("git", _)) => Some((ExtId("git"), "Git")),
            Some(("zip", _)) => Some((ExtId("action-zip"), "archives")),
            _ => None,
        };
        (registry, plugin_of)
    }

    #[test]
    fn rows_file_a_plugin_s_actions_under_the_plugin_after_rick_s_own() {
        let (registry, plugin_of) = with_plugins();
        let rows = rows(&registry, &plugin_of);
        let place = |id: &str| {
            rows.iter()
                .position(|row| row.intent.id() == id)
                .unwrap_or_else(|| panic!("{id} is not a row"))
        };
        let category = |id: &str| rows[place(id)].category;
        // The kernel's action heads File operations, before undo and redo.
        assert_eq!(category("core.copy"), Category::FileOperations);
        assert!(place("nav.back") < place("core.copy"));
        assert!(place("core.copy") < place("ops.undo"));
        assert!(place("ops.undo") < place("view.hidden"));
        // The tables' rows under the dialogue's categories.
        assert_eq!(category("window.fullscreen"), Category::Panes);
        assert_eq!(category("tabs.new"), Category::Panes);
        assert_eq!(category("nav.palette"), Category::Search);
        assert_eq!(category("view.filter"), Category::Search);
        assert_eq!(category("preview.play"), Category::Preview);
        assert_eq!(category("list.down"), Category::List);
        // The plugins' follow everything of Rick's, by the plugins' names
        // whatever their case, and as registered within a plugin.
        let git = Category::Plugin {
            id: ExtId("git"),
            name: "Git",
        };
        assert_eq!(category("git.commit"), git);
        assert_eq!(category("git.push"), git);
        assert!(place("list.extend_end") < place("zip.pack"));
        assert!(place("zip.pack") < place("git.commit"));
        assert!(place("git.commit") < place("git.push"));
        assert_eq!(rows.len(), UI_INTENTS.len() + 4, "every row once");
    }

    #[test]
    fn a_chord_bound_twice_runs_rick_s_own_action_and_names_the_other() {
        let (registry, plugin_of) = with_plugins();
        let rows = rows(&registry, &plugin_of);
        let key = |text: &str| -> Shortcut { text.parse().unwrap_or_else(|e| panic!("{e}")) };
        let copy = Intent::Action(ActionId("core.copy"));
        let pack = Intent::Action(ActionId("zip.pack"));
        let push = Intent::Action(ActionId("git.push"));

        // Mod+C is the kernel's and a plugin's: the kernel's runs, though
        // the plugin registered first, and each names the other.
        let map = KeyMap::of(&rows, &Settings::default());
        assert_eq!(map.lookup(key("Mod+C")), Some(copy));
        assert_eq!(map.conflicts(copy), [pack]);
        assert_eq!(map.conflicts(pack), [copy]);
        // Mod+T is New tab's and a plugin's.
        assert_eq!(map.lookup(key("Mod+T")), Some(Intent::NewTab));
        assert_eq!(map.conflicts(push), [Intent::NewTab]);
        assert_eq!(map.conflicts(Intent::Back), Vec::new());

        // Between two plugins the first in the list runs.
        let mut settings = Settings::default();
        settings.shortcuts.insert("git.commit".into(), "F9".into());
        settings.shortcuts.insert("zip.pack".into(), "F9".into());
        let map = KeyMap::of(&rows, &settings);
        assert_eq!(map.lookup(key("F9")), Some(pack));
        assert_eq!(
            map.conflicts(copy),
            Vec::new(),
            "Mod+C is Copy's alone again"
        );

        // The preview pane's table is bound only in the preview: Space
        // there and Space in the list are not in conflict.
        assert_eq!(map.conflicts(Intent::PreviewPlay), Vec::new());
        assert_eq!(map.conflicts(Intent::ToggleFocused), Vec::new());
        // The same chord on another row of the list is.
        settings.shortcuts.insert("nav.back".into(), "Space".into());
        let map = KeyMap::of(&rows, &settings);
        assert_eq!(map.conflicts(Intent::ToggleFocused), [Intent::Back]);
        assert_eq!(map.conflicts(Intent::PreviewPlay), Vec::new());
    }

    #[test]
    fn the_preview_pane_table_is_live_in_the_preview_pane_only() {
        let map = KeyMap::new(&Registry::new(), &Settings::default());
        let key = |text: &str| -> Shortcut { text.parse().unwrap_or_else(|e| panic!("{e}")) };
        // In an explorer Space toggles the selection.
        assert_eq!(map.lookup(key("Space")), Some(Intent::ToggleFocused));
        assert_eq!(
            map.lookup_in(key("Space"), false),
            Some(Intent::ToggleFocused)
        );
        // In the preview pane it plays or pauses; everything else stays.
        assert_eq!(map.lookup_in(key("Space"), true), Some(Intent::PreviewPlay));
        assert_eq!(map.lookup_in(key("Mod+W"), true), Some(Intent::CloseTab));
        assert_eq!(
            map.match_key(egui::Modifiers::NONE, egui::Key::Space, None, true),
            Some(Intent::PreviewPlay)
        );
        assert_eq!(
            map.match_key(egui::Modifiers::NONE, egui::Key::Space, None, false),
            Some(Intent::ToggleFocused)
        );
    }

    #[test]
    fn physical_keys_match_when_shift_changes_the_logical_one() {
        let map = KeyMap::new(&Registry::new(), &Settings::default());
        let mods = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
        assert_eq!(
            map.match_key(
                mods,
                egui::Key::Exclamationmark,
                Some(egui::Key::Num1),
                false
            ),
            Some(Intent::ViewList)
        );
        assert_eq!(
            map.match_key(mods, egui::Key::Pipe, Some(egui::Key::Backslash), false),
            Some(Intent::SplitDown)
        );
        assert_eq!(
            map.match_key(egui::Modifiers::NONE, egui::Key::F5, None, false),
            Some(Intent::Refresh)
        );
        assert_eq!(
            map.match_key(egui::Modifiers::ALT, egui::Key::F5, None, false),
            None
        );
    }
}
