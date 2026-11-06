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

use std::collections::BTreeMap;

use ext::{ActionId, Key, Modifiers, Registry, Shortcut};
use model::{Column, Settings};

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
    /// Select by glob (not in 0.1).
    SelectByPattern,
    /// Toggle the focused entry in the selection.
    ToggleFocused,
    /// Clear the selection, or the filter when one is set.
    Escape,
    /// Paste as link (not in 0.1).
    PasteAsLink,
    /// Bulk rename (not in 0.1).
    BulkRename,
    /// System properties of the selection.
    Inspect,
    /// Revert the last reversible operation.
    Undo,
    /// Apply the last undone operation again.
    Redo,
    /// Open with a chosen application (not in 0.1).
    OpenWith,
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
    /// System files on or off.
    ToggleSystem,
    /// File extensions in the name column on or off.
    ToggleExtensions,
    /// Flatten subfolders on or off.
    ToggleFlatten,
    /// The inspector pane on or off.
    ToggleInspector,
    /// A console at the location (not in 0.1).
    Console,
    /// The Settings screen.
    Settings,
    /// The Keyboard shortcuts screen.
    Shortcuts,
    /// List view.
    ViewList,
    /// Details view.
    ViewDetails,
    /// Tree view (not in 0.1).
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
        "Command palette",
        NAV,
        &["Mod+Shift+P"],
    ),
    spec(
        Intent::QuickOpen,
        "nav.quick_open",
        "Quick open",
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
        Intent::SelectByPattern,
        "sel.pattern",
        "Select by pattern",
        SEL,
        &["Mod+Shift+S"],
    ),
    spec(
        Intent::ToggleFocused,
        "sel.toggle",
        "Toggle selection of the focused entry",
        SEL,
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
        Intent::PasteAsLink,
        "ops.paste_link",
        "Paste as link",
        OPS,
        &["Mod+Shift+V"],
    ),
    spec(
        Intent::BulkRename,
        "ops.bulk_rename",
        "Bulk rename",
        OPS,
        &["Mod+F2"],
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
    spec(
        Intent::OpenWith,
        "ops.open_with",
        "Open with…",
        OPS,
        &["Mod+O"],
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
    spec(
        Intent::ToggleFlatten,
        "view.flatten",
        "Toggle flatten subfolders",
        VIEW,
        &["Mod+Shift+F"],
    ),
    spec(
        Intent::ToggleInspector,
        "view.inspector",
        "Toggle inspector",
        VIEW,
        &["Mod+Shift+I"],
    ),
    spec(
        Intent::Console,
        "view.console",
        "Open console",
        VIEW,
        &["Mod+`"],
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
        Intent::ToggleSidePanel,
        "window.side_panel",
        "Toggle side panel",
        WINDOW,
        &["Mod+B"],
    ),
    spec(Intent::CursorUp, "list.up", "Cursor up", LIST, &["Up"]),
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
    /// The row of [`UI_INTENTS`] for a UI intent; `None` for actions.
    #[must_use]
    pub fn spec(self) -> Option<&'static IntentSpec> {
        UI_INTENTS.iter().find(|s| s.intent == self)
    }

    /// The id used in `Settings::shortcuts`: the action id or the row id.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Action(id) => id.0,
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

/// One binding and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// The keys.
    pub shortcut: Shortcut,
    /// What they do.
    pub intent: Intent,
}

/// The live shortcut table: defaults with the user's rebindings applied.
#[derive(Debug, Clone, Default)]
pub struct KeyMap {
    bindings: Vec<Binding>,
}

impl KeyMap {
    /// Defaults from the registry's actions and [`UI_INTENTS`], then the
    /// settings' rebindings: a text replaces the defaults of its id, an
    /// empty text unbinds it, and an unparsable text is ignored.
    #[must_use]
    pub fn new(registry: &Registry, settings: &Settings) -> Self {
        let mut defaults: BTreeMap<Intent, Vec<Shortcut>> = BTreeMap::new();
        for spec in UI_INTENTS {
            let shortcuts = spec
                .defaults
                .iter()
                .filter_map(|t| t.parse().ok())
                .collect();
            defaults.insert(spec.intent, shortcuts);
        }
        for action in registry.actions() {
            let intent = Intent::Action(action.id());
            defaults.insert(intent, action.default_shortcut().into_iter().collect());
        }
        for (id, text) in &settings.shortcuts {
            let Some(intent) = Intent::from_id(id, registry) else {
                continue;
            };
            let shortcuts: Vec<Shortcut> = text
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .filter_map(|t| t.parse().ok())
                .collect();
            defaults.insert(intent, shortcuts);
        }
        let bindings = defaults
            .into_iter()
            .flat_map(|(intent, shortcuts)| {
                shortcuts
                    .into_iter()
                    .map(move |shortcut| Binding { shortcut, intent })
            })
            .collect();
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

    /// The first binding of an intent as text, for menus.
    #[must_use]
    pub fn text_for(&self, intent: Intent) -> Option<String> {
        self.shortcuts_for(intent).next().map(|s| s.text())
    }

    /// What this shortcut does, if anything.
    #[must_use]
    pub fn lookup(&self, shortcut: Shortcut) -> Option<Intent> {
        self.bindings
            .iter()
            .find(|b| b.shortcut == shortcut)
            .map(|b| b.intent)
    }

    /// What a key press does: the logical key is tried first, then the
    /// physical one, so `Mod+Shift+1` works where Shift+1 types `!`.
    #[must_use]
    pub fn match_key(
        &self,
        modifiers: egui::Modifiers,
        key: egui::Key,
        physical: Option<egui::Key>,
    ) -> Option<Intent> {
        let mods = from_egui_modifiers(modifiers);
        from_egui_key(key)
            .and_then(|key| {
                self.lookup(Shortcut {
                    modifiers: mods,
                    key,
                })
            })
            .or_else(|| {
                physical.and_then(from_egui_key).and_then(|key| {
                    self.lookup(Shortcut {
                        modifiers: mods,
                        key,
                    })
                })
            })
    }
}

// `Intent` needs an order for the map above; derive would expose the
// variant order as API, so it is a plain id comparison.
impl PartialOrd for Intent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Intent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id().cmp(other.id()).then_with(|| {
            // Two ids never collide between tables, but keep a total order.
            format!("{self:?}").cmp(&format!("{other:?}"))
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
        assert_eq!(map.text_for(Intent::Back).as_deref(), Some("Mod+Left"));
    }

    #[test]
    fn physical_keys_match_when_shift_changes_the_logical_one() {
        let map = KeyMap::new(&Registry::new(), &Settings::default());
        let mods = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
        assert_eq!(
            map.match_key(mods, egui::Key::Exclamationmark, Some(egui::Key::Num1)),
            Some(Intent::ViewList)
        );
        assert_eq!(
            map.match_key(mods, egui::Key::Pipe, Some(egui::Key::Backslash)),
            Some(Intent::SplitDown)
        );
        assert_eq!(
            map.match_key(egui::Modifiers::NONE, egui::Key::F5, None),
            Some(Intent::Refresh)
        );
        assert_eq!(
            map.match_key(egui::Modifiers::ALT, egui::Key::F5, None),
            None
        );
    }
}
