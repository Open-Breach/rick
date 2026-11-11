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
use egui_dock::{DockArea, TabViewer};
use ext::{ActionCtx, ActionId, Invocation, Level};
use model::{
    Column, Density, DockTarget, Geometry, IconSize, JobState, PaneId, PanelSide, SelectionChange,
    SettingsChange, SidePanelChange, SortDir, Tab, TabId, TabKind, Theme, ViewMode, WindowId,
    WorkspaceId,
};
use platform::raw_window_handle::{HasDisplayHandle as _, HasWindowHandle as _};
use platform::{ControlsSide, WindowRef};
use types::{EntryId, Location};

use crate::about::{self, AboutDialog};
use crate::console::ConsoleUi;
use crate::dnd::{Dnd, DragView, Source, Spring, Target, Zone};
use crate::dock::DockSync;
use crate::extensions::ExtensionsDialog;
use crate::host::{Ask, EguiHost};
use crate::inline::{EditKind, RowEdit};
use crate::keys::{self, Direction, Intent, KeyMap};
use crate::operations::Operations;
use crate::preview::Previews;
use crate::search::{Pick, SearchBox};
use crate::shortcuts::ShortcutsScreen;
use crate::side_panel::SidePanelState;
use crate::thumbs::Thumbnails;
use crate::{compare, console};
use crate::{
    conflict, frame, icons, inspector, menus, pane, paths, question, settings_ui, side_panel,
};
use crate::{hash, integration, properties};

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
    /// The footer row in the last frame, for tests.
    pub footer_row: Option<egui::Rect>,
    /// The details list's column header band in the last frame, for tests.
    pub header_band: Option<egui::Rect>,
    /// The x of every boundary between two columns in the last frame, for
    /// tests.
    pub column_boundaries: Vec<f32>,
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
    /// Open the Settings window on a plugin's page, by the plugin's id.
    PluginSettings(String),
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
    /// The pane drag by a handle, started by the toolbar's handle.
    pub pane_drag: &'a mut Option<PaneDrag>,
}

/// One dispatched intent, for the palette's recents and for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dispatched {
    /// What.
    pub intent: Intent,
    /// On which pane.
    pub pane: PaneId,
}

/// A notice at the top right of a window.
struct Toast {
    level: Level,
    text: String,
    /// When it leaves by itself; an error stays until it is dismissed.
    until: Option<Instant>,
    /// The window it is shown in — the one the user acted in — or the
    /// window in front when it is tied to none.
    window: Option<WindowId>,
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

/// The folder dialogue *Download…* opened: what to download once a folder
/// is picked, and the answer.
struct DownloadPick {
    ctx: ActionCtx,
    answer: runtime::oneshot::Receiver<Option<std::path::PathBuf>>,
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

/// The state of one window: its dock, search box and drags.
pub struct WindowUi {
    /// The dock tree of the shown workspace.
    dock: DockSync,
    search: SearchBox,
    /// Where the Preview tab's content was in the last frame, when the
    /// window showed it.
    preview_rect: Option<egui::Rect>,
    /// The pane bar's tab the pointer was over in the frame being drawn.
    hovered_tab: Option<TabId>,
    /// Each workspace tab of the title row in the last frame, by the tab
    /// its title comes from, for tests that drop on one.
    tab_headers: HashMap<TabId, egui::Rect>,
    /// Rick's own window frame as the last frame drew it, if it did; around
    /// a shadow frame the window is then cleared to transparent.
    window_frame: Option<frame::Frame>,
    /// Drag and drop of entries.
    dnd: Dnd,
    /// The pane being dragged by its handle, with its tab.
    pane_drag: Option<PaneDrag>,
    /// What the dragged pane looked like when it was grabbed: its ghost.
    pane_ghost: Option<crate::drag::Snapshot>,
    /// The drag ghost as this frame painted it, a pane's or a tab's.
    ghost: Option<crate::drag::Ghost>,
    /// The content rectangle of the last frame, window coordinates.
    content: Option<egui::Rect>,
    /// The window's inner rectangle on the screen, from the last frame.
    screen: Option<egui::Rect>,
    /// The title row's tab row in the last frame, window coordinates: where
    /// a dropped pane, or a tab dragged from another window, lands.
    tab_row: Option<egui::Rect>,
    /// A workspace tab being dragged along the title row.
    workspace_drag: Option<TabDrag>,
    /// Where each tab of the row is drawn while the tabs slide apart for
    /// a dragged one; `None` is the new-tab button after them.
    tab_slides: crate::drag::Slides<Option<WorkspaceId>>,
    /// Until when the row's tabs slide to their places: while a tab or a
    /// pane is dragged over the row and for one slide after.
    slide_until: f64,
    /// Where a pane dragged over the tab row would land as a new tab, in
    /// this frame.
    pane_drop_slot: Option<usize>,
    /// The workspace tab a dragged pane or tab rests on, and since when:
    /// after the spring-loading delay the window shows that workspace
    /// (SPEC.md *Workspace tabs*, Spring-loaded tabs).
    tab_rest: Option<TabRest>,
    /// The workspace the window showed when the drag of a pane or a tab
    /// began: what it shows again when the drag ends and nothing moved.
    drag_origin: Option<WorkspaceId>,
    /// Where this window would take what another window's drag carries
    /// over it, as the frame being drawn shows it.
    offer: Option<Offer>,
    /// The restored geometry has been handed to the backend.
    placed: bool,
    /// The operations drawer shows its summary alone (SPEC.md *Operations
    /// drawer*); remembered for the window.
    drawer_collapsed: bool,
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
            pane_drag: None,
            pane_ghost: None,
            ghost: None,
            content: None,
            screen: None,
            tab_row: None,
            workspace_drag: None,
            tab_slides: crate::drag::Slides::default(),
            slide_until: 0.0,
            pane_drop_slot: None,
            tab_rest: None,
            drag_origin: None,
            offer: None,
            placed: false,
            drawer_collapsed: false,
        }
    }
}

/// A pane being dragged by its handle (SPEC.md *Panes*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneDrag {
    /// The pane's tab, what the model moves.
    pub tab: TabId,
    /// The pane.
    pub pane: PaneId,
}

/// Where a dragged pane can land on the pane under it: the five tiles of
/// the dock cross.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockZone {
    Left,
    Right,
    Top,
    Bottom,
    Centre,
}

impl DockZone {
    /// The side of the pane under it a pane dropped here docks on; the
    /// centre is no side, it makes a new tab.
    fn side(self) -> Option<model::Side> {
        match self {
            Self::Left => Some(model::Side::Left),
            Self::Right => Some(model::Side::Right),
            Self::Top => Some(model::Side::Top),
            Self::Bottom => Some(model::Side::Bottom),
            Self::Centre => None,
        }
    }
}

/// Which dock targets a drag is offered on the pane under the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cross {
    /// A pane dragged by its handle: the four sides and the centre, which
    /// makes it a new tab.
    Pane,
    /// A tab that holds exactly one pane: the four sides, and no centre,
    /// the tab being a tab already (SPEC.md *Workspace tabs*).
    Tab,
}

impl Cross {
    /// The tiles of the cross over a pane's rectangle.
    fn tiles(self, rect: egui::Rect) -> impl Iterator<Item = (DockZone, egui::Rect)> {
        dock_tiles(rect)
            .into_iter()
            .filter(move |(zone, _)| self == Self::Pane || *zone != DockZone::Centre)
    }

    /// The tile of the cross over `rect` that `pointer` is on.
    fn zone_at(self, rect: egui::Rect, pointer: egui::Pos2) -> Option<DockZone> {
        self.tiles(rect)
            .find(|(_, tile)| tile.contains(pointer))
            .map(|(zone, _)| zone)
    }
}

/// The side of a dock tile, in points.
pub const DOCK_TILE: f32 = 40.0;
/// The distance between the centres of two neighbouring dock tiles.
pub const DOCK_TILE_PITCH: f32 = 48.0;

/// The five dock tiles over a pane's rectangle, in a cross around its
/// centre.
fn dock_tiles(rect: egui::Rect) -> [(DockZone, egui::Rect); 5] {
    let tile = |dx: f32, dy: f32| {
        egui::Rect::from_center_size(
            rect.center() + egui::vec2(dx, dy) * DOCK_TILE_PITCH,
            egui::Vec2::splat(DOCK_TILE),
        )
    };
    [
        (DockZone::Left, tile(-1.0, 0.0)),
        (DockZone::Right, tile(1.0, 0.0)),
        (DockZone::Top, tile(0.0, -1.0)),
        (DockZone::Bottom, tile(0.0, 1.0)),
        (DockZone::Centre, tile(0.0, 0.0)),
    ]
}

/// The half of `rect` a pane dropped on `zone` would take.
fn dock_half(rect: egui::Rect, zone: DockZone) -> Option<egui::Rect> {
    let c = rect.center();
    Some(match zone {
        DockZone::Left => egui::Rect::from_min_max(rect.min, egui::pos2(c.x, rect.max.y)),
        DockZone::Right => egui::Rect::from_min_max(egui::pos2(c.x, rect.min.y), rect.max),
        DockZone::Top => egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, c.y)),
        DockZone::Bottom => egui::Rect::from_min_max(egui::pos2(rect.min.x, c.y), rect.max),
        DockZone::Centre => return None,
    })
}

/// A dock tile's icon: a window outline with the half the pane would take
/// filled, or a top bar for the centre's new tab.
fn paint_split_icon(
    painter: &egui::Painter,
    tile: egui::Rect,
    zone: DockZone,
    colour: egui::Color32,
) {
    let frame = egui::Rect::from_center_size(tile.center(), egui::Vec2::splat(16.0));
    painter.rect_stroke(
        frame,
        3.0,
        egui::Stroke::new(1.5, colour),
        egui::StrokeKind::Inside,
    );
    let inner = frame.shrink(3.0);
    let c = inner.center();
    let filled = match zone {
        DockZone::Left => egui::Rect::from_min_max(inner.min, egui::pos2(c.x - 1.0, inner.max.y)),
        DockZone::Right => egui::Rect::from_min_max(egui::pos2(c.x + 1.0, inner.min.y), inner.max),
        DockZone::Top => egui::Rect::from_min_max(inner.min, egui::pos2(inner.max.x, c.y - 1.0)),
        DockZone::Bottom => egui::Rect::from_min_max(egui::pos2(inner.min.x, c.y + 1.0), inner.max),
        DockZone::Centre => {
            egui::Rect::from_min_max(inner.min, egui::pos2(inner.max.x, inner.min.y + 3.0))
        }
    };
    painter.rect_filled(filled, 1.0, colour);
}

/// The dock targets over the pane at `rect`: the tiles of `cross`, the
/// one of `hot` filled `accent/button`, and a translucent preview of the
/// half a pane dropped there would take (SPEC.md *Panes*).
fn paint_dock_cross(
    overlay: &egui::Painter,
    rect: egui::Rect,
    hot: Option<DockZone>,
    cross: Cross,
    scheme: &crate::theme::Scheme,
) {
    if let Some(half) = hot.and_then(|zone| dock_half(rect, zone)) {
        overlay.rect_filled(half, 0.0, scheme.accent_button.gamma_multiply(0.35));
    }
    for (zone, tile) in cross.tiles(rect) {
        let lit = hot == Some(zone);
        let fill = if lit {
            scheme.accent_button
        } else {
            scheme.bg_card
        };
        overlay.rect_filled(tile, 8.0, fill);
        overlay.rect_stroke(tile, 8.0, scheme.hairline(), egui::StrokeKind::Inside);
        let colour = if lit {
            crate::theme::Scheme::ON_BUTTON
        } else {
            scheme.icon_default
        };
        paint_split_icon(overlay, tile, zone, colour);
    }
}

/// The workspace tab a dragged pane or tab rests on.
#[derive(Debug, Clone, Copy)]
struct TabRest {
    /// The workspace of the tab.
    on: WorkspaceId,
    /// Since when, in the frame clock's seconds.
    since: f64,
    /// Where the cursor was when the rest began.
    at: egui::Pos2,
    /// The window has switched to it already.
    sprung: bool,
}

/// A workspace tab being dragged: along the title row, or off it as its
/// own ghost.
#[derive(Debug, Clone, Copy)]
struct TabDrag {
    /// The workspace.
    id: WorkspaceId,
    /// Where the pointer took the tab, from its top left corner.
    grab: egui::Vec2,
}

/// What a drag of the layout carries: a pane by its handle, or a workspace
/// tab.
#[derive(Debug, Clone, Copy)]
enum Carried {
    /// A pane, by its handle.
    Pane(PaneDrag),
    /// A workspace tab.
    Tab(TabDrag),
}

/// A drag of a pane or of a workspace tab as every window sees it (SPEC.md
/// *Windows*). The pointer belongs to the window the drag began in, so
/// that window publishes the drag each frame with the cursor's place in
/// the coordinates the windows share; the window the cursor is over then
/// offers its tab row and its dock targets and paints the ghost, and the
/// first window lands the drop where that one offered it.
#[derive(Debug, Clone, Copy)]
struct Roam {
    /// The window the drag began in.
    from: WindowId,
    /// What it carries.
    what: Carried,
    /// The cursor, in the coordinates the windows share: the screen's, or
    /// the root's where the windows are drawn inside it.
    at: egui::Pos2,
    /// The window the cursor is over.
    over: Option<WindowId>,
}

impl Roam {
    /// The cursor in `window`'s coordinates while it is over that window,
    /// `origin` being where the window's coordinates begin among the
    /// shared ones; `pointer` is the window's own pointer, which is the
    /// cursor of a drag that began in it.
    fn cursor_in(
        self,
        window: WindowId,
        origin: Option<egui::Vec2>,
        pointer: Option<egui::Pos2>,
    ) -> Option<egui::Pos2> {
        if self.over != Some(window) {
            None
        } else if self.from == window {
            pointer
        } else {
            origin.map(|origin| self.at - origin)
        }
    }

    /// The tab another window's drag carries, as `window` sees it.
    fn visiting_tab(self, window: WindowId) -> Option<TabDrag> {
        match self.what {
            Carried::Tab(tab) if self.from != window => Some(tab),
            _ => None,
        }
    }

    /// The pane another window's drag carries, as `window` sees it.
    fn visiting_pane(self, window: WindowId) -> Option<PaneDrag> {
        match self.what {
            Carried::Pane(pane) if self.from != window => Some(pane),
            _ => None,
        }
    }
}

/// Where a window would take what another window's drag carries over it,
/// as its last frame showed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Offer {
    /// A place in its tab row.
    Row(usize),
    /// A dock target of one of its panes.
    Dock {
        /// The pane under the cursor.
        pane: PaneId,
        /// The tile of its cross.
        zone: DockZone,
    },
}

/// What a workspace tab shows: the folder in the workspace's focused pane.
struct TabLabel {
    /// The place's, the view's or the provider's icon.
    icon: &'static str,
    /// The folder's name or the view's title.
    title: String,
    /// The tab the label comes from.
    tab: Option<TabId>,
    /// Where that tab is, when it lists a folder; *Reopen closed tab*
    /// brings it back.
    location: Option<Location>,
    /// What a drop of entries on the tab does, when it lists a folder.
    drop: Option<Target>,
}

/// The label of a workspace: the folder of its focused pane, or of its
/// first explorer when the focused pane shows the preview.
fn workspace_label(model: &model::Model, ws: &model::Workspace) -> TabLabel {
    let pane = if crate::dock::shows_preview(model, ws.focused) {
        ws.dock
            .panes()
            .into_iter()
            .find(|p| !crate::dock::shows_preview(model, *p))
            .unwrap_or(ws.focused)
    } else {
        ws.focused
    };
    let Some((id, tab)) = model.active_tab(pane) else {
        return TabLabel {
            icon: icons::FOLDER,
            title: "?".to_owned(),
            tab: None,
            location: None,
            drop: None,
        };
    };
    let (icon, title) = tab_label(model, tab);
    let lists = tab.kind.lists();
    TabLabel {
        icon,
        title,
        tab: Some(id),
        location: lists.then(|| tab.location.clone()),
        drop: lists.then(|| {
            crate::dnd::into_pane(
                tab,
                Target::Folder {
                    to: tab.location.clone(),
                    device: crate::dnd::listing_device(tab),
                    spring: Some(Spring::Tab(id)),
                },
            )
        }),
    }
}

/// A tab's icon and title: a view's own; for a folder, the icon the side
/// panel gives it when it is a place, a virtual location's own, the
/// provider's for an archive or a remote, `FOLDER` otherwise, and the
/// folder's name (SPEC.md *Workspace tabs*).
fn tab_label(model: &model::Model, tab: &Tab) -> (&'static str, String) {
    match tab.kind {
        TabKind::Preview => (icons::PREVIEW, t!("shell-preview-tab")),
        // A console is titled Console: not the folder it was opened in,
        // and not what its shell sets as its title.
        TabKind::Console => (icons::TERMINAL, t!("shell-console-tab")),
        TabKind::Compare => (
            icons::COMPARE,
            tab.title.clone().unwrap_or_else(|| t!("shell-compare-tab")),
        ),
        TabKind::Explorer => {
            let place = model
                .places
                .rows()
                .iter()
                .find(|p| p.location == tab.location)
                .map(|p| icons::named(&p.icon));
            let icon = place.unwrap_or_else(|| {
                vfs_rick::Virtual::of(&tab.location)
                    .map_or_else(|| paths::location_icon(&tab.location), virtual_icon)
            });
            (icon, paths::title_of(&tab.location))
        }
    }
}

/// Draws one workspace tab at `rect`: a 16 px icon, the title truncated
/// with an ellipsis and a close button. The active tab is filled `bg/pane`
/// and drawn in the scheme's own colours, lifting out of the title row's
/// band; an inactive one shows the band and takes its colours from it —
/// the band's hover under the pointer, its title and icon colours, the
/// close button at half opacity (SPEC.md *Workspace tabs*, *On the
/// band*). The grab cursor is over the tab's body. Returns the tab's
/// response and whether the close button was pressed.
fn workspace_tab(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: WorkspaceId,
    label: &TabLabel,
    active: bool,
) -> (egui::Response, bool) {
    use crate::theme::{ICON, TAB_CORNER, TEXT_BODY};
    let scheme = crate::theme::scheme(ui);
    let band = crate::theme::band(ui);
    let response = ui.interact(
        rect,
        egui::Id::new(("workspace-tab", id.0)),
        egui::Sense::click_and_drag(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.title.as_str())
    });
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 8.0 - ICON / 2.0, rect.center().y),
        egui::vec2(20.0, 20.0),
    );
    let close = ui.interact(
        close_rect,
        egui::Id::new(("workspace-tab-close", id.0)),
        egui::Sense::click(),
    );
    close.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t!("shell-close-tab"))
    });
    let corners = egui::CornerRadius {
        nw: TAB_CORNER,
        ne: TAB_CORNER,
        sw: 0,
        se: 0,
    };
    // The open hand over the tab's body: it can be dragged. Its close
    // button keeps the cursor it has (QUESTIONS.md 277); the tab counts
    // as hovered under it too, as it would take a drag from there.
    if response.hovered() && !close.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, corners, scheme.bg_pane);
    } else if response.hovered() || close.hovered() {
        painter.rect_filled(rect, corners, band.tab_hover);
    }
    // The active tab is the scheme's; the others sit on the band.
    let (icon, colour, close_hover) = if active {
        (scheme.icon_default, scheme.text_primary, scheme.bg_control)
    } else {
        (band.icon, band.title, band.hover)
    };
    let icon_font = egui::FontId::proportional(ICON);
    painter.text(
        egui::pos2(rect.left() + 14.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label.icon,
        icon_font.clone(),
        icon,
    );
    let title_left = rect.left() + 38.0;
    let title_width = (close_rect.left() - 6.0 - title_left).max(0.0);
    let mut job = egui::text::LayoutJob::simple_singleline(
        label.title.clone(),
        egui::FontId::proportional(TEXT_BODY),
        colour,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(title_width);
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let text_pos = egui::pos2(title_left, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_pos, galley, colour);
    let x_colour = if active {
        icon
    } else {
        icon.gamma_multiply(0.5)
    };
    if close.hovered() {
        ui.painter()
            .circle_filled(close_rect.center(), 10.0, close_hover);
    }
    ui.painter().text(
        close_rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::CLOSE,
        icon_font,
        x_colour,
    );
    let closed = close.on_hover_text(t!("shell-close-tab")).clicked();
    (response, closed)
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
    /// The window of the pane a job that just failed was started from:
    /// where the notice that follows is shown.
    failed_in: Option<WindowId>,
    /// What each window has seen of the copy, cut and paste chords, which
    /// do not arrive as key presses.
    chords: HashMap<WindowId, crate::chords::ClipboardChords>,
    dialog: Option<Dialog>,
    name_prompt: Option<NamePrompt>,
    settings: settings_ui::SettingsWindow,
    shortcuts: ShortcutsScreen,
    extensions: ExtensionsDialog,
    /// The About dialog (specs/about-dialog.md), with the binary's build
    /// details once it says them.
    about: AboutDialog,
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
    /// The folder dialogue of a *Download…* that waits for its answer.
    download_pick: Option<DownloadPick>,
    /// The Hash dialogue, open or not.
    hash: hash::HashDialog,
    /// The integration dialog, open or not, and the check after the
    /// first frame that may open it.
    integration: integration::IntegrationDialog,
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
    applied: Option<(Theme, u8, Density, (model::Rgb, model::Rgb))>,
    now: SystemTime,
    /// Each model window's state.
    windows: BTreeMap<WindowId, WindowUi>,
    /// The model window the root viewport shows.
    root: WindowId,
    /// The drag of a pane or a workspace tab in progress, as the window it
    /// began in published it for the others.
    roam: Option<Roam>,
    /// The windows are drawn inside the root window, not as windows of
    /// the system's (a test harness): they share the root's coordinates.
    embedded: bool,
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
    /// The frame drawn around every window Rick decorates itself: the
    /// shadow frame unless [`with_frame`](Self::with_frame) said otherwise.
    frame_mode: frame::Mode,
    /// Work from outside the windows (a second launch's command line,
    /// handed over through the single-instance endpoint), run against the
    /// application before the root window's frame.
    externals: Option<app::ExternalReceiver>,
    /// Continues from the copy Rick was installed to: the binary's hook,
    /// called with the installed binary once the session is saved.
    relaunch: Option<Relaunch>,
}

/// What continues Rick from the copy it was installed to
/// (specs/system-integration.md §6.1): given the installed binary, it
/// starts that copy and has the running process exit once the new one
/// answers on the single-instance endpoint. The binary's, since it holds
/// the endpoint; without one (the tests) nothing is relaunched.
pub type Relaunch = Arc<dyn Fn(&std::path::Path) + Send + Sync>;

/// How many dispatched intents are remembered.
const RECENTS: usize = 64;
/// The room between two workspace tabs.
const TAB_GAP: f32 = 2.0;
/// The new-tab button's width.
const NEW_TAB_WIDTH: f32 = 28.0;
/// The slot the tab row shows where a dropped pane would land as a new tab.
const DROP_SLOT_WIDTH: f32 = 160.0;
/// A new window's inner size before the user resizes it.
const DEFAULT_SIZE: [f32; 2] = [1100.0, 720.0];
/// The smallest inner size.
const MIN_SIZE: [f32; 2] = [480.0, 320.0];
/// The one identifier of the application (specs/app-icons.md): the Wayland
/// app id and the X11 `WM_CLASS` of every window, and so the name of the
/// desktop entry (`rick.desktop`), of its `Icon=` in the hicolor theme and
/// of its `StartupWMClass=`. A compositor takes the running window's icon
/// from the desktop entry this id names; were any of them to differ, GNOME
/// and KDE would show a generic icon and a second launcher entry.
pub const APP_ID: &str = "rick";
/// The window icon, the 256 px tile of the Portal burst icon: the title
/// bar, taskbar and Alt-Tab on Windows and X11 (Wayland takes the desktop
/// entry's, macOS the bundle's).
const WINDOW_ICON_PNG: &[u8] = include_bytes!("../../../assets/icons/png/rick-256.png");

/// The window icon decoded once; `None` when the bundled PNG does not
/// decode, which is said once and leaves the window with the system's
/// default icon.
fn window_icon() -> Option<Arc<egui::IconData>> {
    static ICON: std::sync::OnceLock<Option<Arc<egui::IconData>>> = std::sync::OnceLock::new();
    ICON.get_or_init(
        || match eframe::icon_data::from_png_bytes(WINDOW_ICON_PNG) {
            Ok(icon) => Some(Arc::new(icon)),
            Err(error) => {
                tracing::warn!(%error, "the window icon did not decode");
                None
            }
        },
    )
    .clone()
}

/// The dock's look (SPEC.md *Panes*): no tab bars, no border around the
/// focused pane, every pane a `bg/pane` surface, the panes separated by a
/// hairline that drags to resize.
fn dock_style(egui_style: &egui::Style, scheme: &crate::theme::Scheme) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(egui_style);
    style.dock_area_padding = None;
    style.main_surface_border_stroke = egui::Stroke::NONE;
    style.main_surface_border_rounding = egui::CornerRadius::ZERO;
    style.separator.width = 1.0;
    style.separator.extra_interact_width = 4.0;
    style.separator.color_idle = scheme.border_default;
    style.separator.color_hovered = scheme.accent_focus;
    style.separator.color_dragged = scheme.accent_focus;
    style.tab_bar.bg_fill = scheme.bg_pane;
    style.tab.tab_body.bg_fill = scheme.bg_pane;
    style.tab.tab_body.stroke = egui::Stroke::NONE;
    style.tab.tab_body.corner_radius = egui::CornerRadius::ZERO;
    style.tab.tab_body.inner_margin = egui::Margin::ZERO;
    // The bars are hidden: no strip to drag by and no button to show one.
    style.tab.tab_body.hidden_tab_bar_drag_height = Some(0.0);
    style.buttons.show_tab_bar_size = 0.0;
    style
}

/// The gap between two icons of the title row's window group.
const WINDOW_ICON_GAP: f32 = 12.0;
/// The gap between the layout toggles' icons and the window controls',
/// with the divider in its middle.
const WINDOW_GROUP_GAP: f32 = 25.0;

/// One control of the title row's window group (SPEC.md *Title row*): a
/// 16 px icon in a step of the icon and its 12 px gap, as tall as a tool
/// button, which answers over the whole of it — so the controls are
/// neither crowded nor small to hit. It is drawn on the band, in the
/// band's colours; `on` makes it a toggle, whose filled glyph and — on a
/// band that agrees with the scheme — `accent/focus` tint mark it active.
/// `name` is its accessible name and its tooltip.
fn window_control(ui: &mut egui::Ui, glyph: &str, name: &str, on: Option<bool>) -> egui::Response {
    use crate::theme::ICON;
    let metrics = crate::theme::metrics(ui.ctx());
    let scheme = crate::theme::scheme(ui);
    let band = crate::theme::band(ui);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ICON + WINDOW_ICON_GAP, metrics.button_size),
        egui::Sense::click(),
    );
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, metrics.corner_radius(), band.hover);
    }
    let active = on == Some(true);
    let colour = if active && !band.contrary {
        scheme.accent_focus
    } else {
        band.icon
    };
    let family = if active {
        egui::FontFamily::Name(icons::FILL_FAMILY.into())
    } else {
        egui::FontFamily::Proportional
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::new(ICON, family),
        colour,
    );
    response.widget_info(|| match on {
        Some(on) => {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, name.to_owned())
        }
        None => egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.to_owned()),
    });
    response.on_hover_text(name)
}
/// How long a notice that is not an error stays.
const TOAST_FOR: Duration = Duration::from_secs(5);
/// The width a notice's text wraps at.
const TOAST_WIDTH: f32 = 340.0;
/// The gap between the notices and the title row above them, and the
/// window's edge at their right.
const TOAST_GAP: f32 = 8.0;
/// How long a `SelectNamed` waits for its entry to be listed.
const SELECT_FOR: Duration = Duration::from_secs(5);

/// The viewport of a model window other than the root's.
fn viewport_of(window: WindowId) -> ViewportId {
    ViewportId::from_hash_of(("rick-window", window.0))
}

/// The viewport builder for a window: Rick's own chrome, the app id and the
/// window icon, at the window's restored geometry when it has one.
/// `transparent` is whether a window Rick frames itself asks the system for
/// transparency: the first window by [`frame::asks_transparency`], every
/// later one by the frame that was chosen ([`frame::Mode::transparent`]). A
/// window whose system keeps its controls never asks.
#[must_use]
pub fn viewport_builder(
    title: &str,
    chrome: platform::WindowChrome,
    geometry: Option<Geometry>,
    transparent: bool,
) -> ViewportBuilder {
    let mut viewport = ViewportBuilder::default()
        .with_title(title)
        .with_app_id(APP_ID)
        .with_min_inner_size(MIN_SIZE);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    viewport = match geometry {
        Some(g) => viewport
            .with_position([g.x, g.y])
            .with_inner_size([g.width.max(MIN_SIZE[0]), g.height.max(MIN_SIZE[1])])
            .with_maximized(g.maximised),
        None => viewport.with_inner_size(DEFAULT_SIZE),
    };
    // Without decorations Rick draws the frame itself (`frame`): for the
    // shadow frame the window is transparent, so the margin around the
    // content shows the shadow; for the plain frame it is not.
    if chrome.native_controls {
        viewport
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_fullsize_content_view(true)
    } else {
        viewport
            .with_decorations(false)
            .with_transparent(transparent)
    }
}

impl RickUi {
    /// The build details the About dialog shows, as the binary knows them
    /// (`rick --version` prints the same).
    #[must_use]
    pub fn with_build(mut self, build: about::Build) -> Self {
        self.about.set_build(build);
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
        let keys = KeyMap::of(&keys::rows_of(&app), &app.model().settings);
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
            failed_in: None,
            chords: HashMap::new(),
            dialog: None,
            name_prompt: None,
            settings: settings_ui::SettingsWindow::default(),
            shortcuts: ShortcutsScreen::default(),
            extensions: ExtensionsDialog::default(),
            about: AboutDialog::default(),
            inspector_state: inspector::PaneState::default(),
            inspector_width,
            properties: properties::PropertiesDialog::default(),
            library_browse: None,
            download_pick: None,
            hash: hash::HashDialog::default(),
            integration: integration::IntegrationDialog::default(),
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
            roam: None,
            embedded: false,
            quitting: false,
            window_ref: None,
            attached: false,
            frame_mode: frame::Mode::Shadow,
            viewports,
            externals: None,
            relaunch: None,
        }
    }

    /// Sets the frame Rick draws around its windows (specs/gui-title-row.md,
    /// *Window frame*). Without this call it is the shadow frame, which is
    /// right where the window is rendered off screen, as the test harness
    /// does; [`run`](crate::run) always calls it, with the plain frame
    /// unless the window is known to be transparent.
    #[must_use]
    pub fn with_frame(mut self, mode: frame::Mode) -> Self {
        self.frame_mode = mode;
        self
    }

    /// Takes the hook that continues Rick from the copy it was installed
    /// to: called when an integration apply ends with an install, after
    /// the session was saved.
    #[must_use]
    pub fn with_relaunch(mut self, relaunch: Relaunch) -> Self {
        self.relaunch = Some(relaunch);
        self
    }

    /// Takes the work from outside the windows: the single-instance
    /// endpoint's deliveries, run against the application before the root
    /// window's frame; a delivery wakes the window.
    #[must_use]
    pub fn with_externals(mut self, externals: app::ExternalReceiver) -> Self {
        let wake = self.ctx.clone();
        externals.set_waker(Arc::new(move || wake.request_repaint()));
        self.externals = Some(externals);
        self
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

    /// Whether the root window shows its Preview tab: one in a workspace
    /// that is not shown (a restored session's, when the start folder
    /// opened a workspace of its own) is out of sight, and the toggle
    /// opens it in the shown workspace (QUESTIONS.md 317).
    #[must_use]
    pub fn inspector_open(&self) -> bool {
        let model = self.app.model();
        model
            .preview_tab(self.root)
            .and_then(|tab| model.pane_of(tab))
            .zip(model.window(self.root))
            .is_some_and(|(pane, window)| window.dock().panes().contains(&pane))
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

    /// The integration dialog, for the tests.
    #[must_use]
    pub fn integration_dialog(&self) -> &integration::IntegrationDialog {
        &self.integration
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

    /// The info button of the Preview tab as last drawn: inert, with a
    /// card to open, or pinned.
    #[must_use]
    pub fn preview_info(&self) -> inspector::Info {
        self.inspector_state.info
    }

    /// The File info card's rows of the last frame, as label and value.
    #[must_use]
    pub fn preview_fields(&self) -> &[(String, String)] {
        &self.inspector_state.fields
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

    /// Whether the Settings window is open.
    #[must_use]
    pub fn settings_open(&self) -> bool {
        self.settings.is_open()
    }

    /// Opens the Settings window on the root window, as the menu does; for
    /// the tests.
    pub fn open_settings(&mut self) {
        self.settings.open(self.root);
    }

    /// Whether the Keyboard shortcuts screen is open.
    #[must_use]
    pub fn shortcuts_open(&self) -> bool {
        self.shortcuts.open
    }

    /// Whether the Extensions screen is open.
    #[must_use]
    pub fn extensions_open(&self) -> bool {
        self.extensions.is_open()
    }

    /// The About dialog, for the tests.
    #[must_use]
    pub fn about(&self) -> &AboutDialog {
        &self.about
    }

    /// Opens the About dialog, as the menu's *About Rick* does; for the
    /// tests.
    pub fn open_about(&mut self) {
        self.about.open();
    }

    /// Replaces the build details the About dialog shows; for the tests.
    pub fn set_build(&mut self, build: about::Build) {
        self.about.set_build(build);
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

    /// The workspace tabs of the window holding `pane`, in the order of
    /// the title row, for tests.
    #[must_use]
    pub fn tab_order(&self, pane: PaneId) -> Vec<WorkspaceId> {
        self.workspace_order(pane)
    }

    /// The title row's tab row of the root window in the last frame, for
    /// tests that drop on it.
    #[must_use]
    pub fn tab_row(&self) -> Option<egui::Rect> {
        self.windows.get(&self.root).and_then(|w| w.tab_row)
    }

    /// The tab row of `window` in the last frame, in that window's
    /// coordinates; for tests that drag between windows.
    #[must_use]
    pub fn tab_row_of(&self, window: WindowId) -> Option<egui::Rect> {
        self.windows.get(&window).and_then(|w| w.tab_row)
    }

    /// Each pane's rectangle in the last frame of `window`; for tests that
    /// drag between windows.
    #[must_use]
    pub fn pane_rects_of(&self, window: WindowId) -> Vec<(PaneId, egui::Rect)> {
        self.windows
            .get(&window)
            .map(|w| w.dock.pane_rects(self.app.model()))
            .unwrap_or_default()
    }

    /// The content of `window` in the last frame, in its own coordinates;
    /// for tests that drag between windows.
    #[must_use]
    pub fn content_of(&self, window: WindowId) -> Option<egui::Rect> {
        self.windows.get(&window).and_then(|w| w.content)
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

    /// A tab's column header band in the last frame, for tests.
    #[must_use]
    pub fn header_band(&self, tab: TabId) -> Option<egui::Rect> {
        self.tabs.get(&tab).and_then(|t| t.header_band)
    }

    /// The x of every boundary between two columns of a tab's details
    /// list in the last frame, left to right, for tests.
    #[must_use]
    pub fn column_boundaries(&self, tab: TabId) -> Vec<f32> {
        self.tabs
            .get(&tab)
            .map(|t| t.column_boundaries.clone())
            .unwrap_or_default()
    }

    /// The toolbar row of a tab in the last frame, for tests.
    #[must_use]
    pub fn toolbar_row(&self, tab: TabId) -> Option<egui::Rect> {
        self.tabs.get(&tab).and_then(|t| t.toolbar_row)
    }

    /// The footer row of a tab in the last frame, for tests.
    #[must_use]
    pub fn footer_row(&self, tab: TabId) -> Option<egui::Rect> {
        self.tabs.get(&tab).and_then(|t| t.footer_row)
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

    /// The drag ghost of a pane or a workspace tab as the last frame
    /// painted it, in whichever window has one; for tests.
    #[must_use]
    pub fn drag_ghost(&self) -> Option<crate::drag::Ghost> {
        self.windows.values().find_map(|w| w.ghost)
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

    /// Raises and focuses a window, as a second launch's command line
    /// asks: through the backend for every window, and through the
    /// platform's activation for the root window, whose handles the
    /// backend gives out (`xdg_activation` with the launcher's token,
    /// `_NET_ACTIVE_WINDOW`, `NSApp.activate`, `SetForegroundWindow`).
    fn activate(&mut self, window: WindowId, token: Option<&str>) {
        let viewport = if window == self.root {
            ViewportId::ROOT
        } else {
            viewport_of(window)
        };
        self.ctx
            .send_viewport_cmd_to(viewport, ViewportCommand::Minimized(false));
        self.ctx
            .send_viewport_cmd_to(viewport, ViewportCommand::Focus);
        if window == self.root
            && let Some(window_ref) = &self.window_ref
            && let Err(error) = self.app.platform().activate_window(window_ref, token)
        {
            tracing::debug!(%error, "the window was not raised through the platform");
        }
    }

    fn toast(&mut self, level: Level, text: impl Into<String>) {
        self.toast_in(level, text, None);
    }

    /// A notice in `window`, or in the window in front. An error stays
    /// until it is dismissed, so it is never missed; any other notice
    /// leaves by itself after [`TOAST_FOR`] (SPEC.md *Operations*).
    fn toast_in(&mut self, level: Level, text: impl Into<String>, window: Option<WindowId>) {
        let until = (level != Level::Error).then(|| Instant::now() + TOAST_FOR);
        self.toasts.push(Toast {
            level,
            text: text.into(),
            until,
            window,
        });
    }

    fn absorb(&mut self, events: Vec<Event>) {
        // A failed job and its notice come in one batch.
        self.failed_in = None;
        for event in events {
            match event {
                Event::Thumbnail { id, size, frame } => {
                    self.thumbnails.arrived(&self.ctx, id, size, frame);
                }
                Event::JobFailed { pane, .. } => {
                    // The notice that follows is shown where the user
                    // acted: in the window of the pane the job was
                    // started from.
                    self.failed_in = pane.and_then(|pane| self.app.model().window_of(pane));
                }
                Event::Notify { level, text } => {
                    let window = self.failed_in.take().filter(|_| level == Level::Error);
                    self.toast_in(level, text, window);
                }
                Event::Activate { window, token } => self.activate(window, token.as_deref()),
                Event::Properties(info) => self.properties.open(*info),
                Event::PluginInstalled { path } => self.extensions.installed(path),
                Event::PropertiesCounted {
                    pane,
                    path,
                    contents,
                } => self.properties.counted(pane, &path, contents),
                Event::HashesOpened { hashes, format, .. } => self.hash.open(hashes, format),
                Event::PreviewReady {
                    pane,
                    doc,
                    previewer,
                } => {
                    let entry = self.preview_entry(pane);
                    self.previews.ready(pane, doc, entry, previewer);
                }
                Event::PreviewFailed { pane, error } => {
                    let entry = self.preview_entry(pane);
                    self.previews.failed(pane, &error, entry);
                }
                Event::PreviewNone {
                    pane,
                    plugin,
                    permissions,
                } => {
                    let entry = self.preview_entry(pane);
                    self.previews.none(pane, plugin, permissions, entry);
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
                    provider,
                } => {
                    let entry = self.preview_entry(pane);
                    self.previews
                        .capped(pane, size, cap, (source, provider), entry);
                }
                Event::PreviewProgress { pane, read, total } => {
                    let entry = self.preview_entry(pane);
                    self.previews.progress(pane, entry, read, total);
                }
                Event::ViewOpened { tab, view, .. } => {
                    if let ext::View::Console(console) = &view {
                        let ctx = self.ctx.clone();
                        console.set_waker(Arc::new(move || ctx.request_repaint()));
                        self.consoles.entry(tab).or_default().focus_next = true;
                    }
                    self.views.insert(tab, view);
                }
                Event::IntegrationChecked { ask: true } => {
                    self.integration.open(&self.app.model().integration);
                }
                Event::IntegrationApplied { relaunch, .. } => {
                    self.integration.applied();
                    if let Some(binary) = relaunch {
                        self.relaunch_from(&binary);
                    }
                }
                _ => {}
            }
        }
    }

    /// Rick was installed to the standard location: the session is saved
    /// for the installed copy to restore, and the binary's hook starts it.
    fn relaunch_from(&mut self, binary: &std::path::Path) {
        let Some(relaunch) = self.relaunch.clone() else {
            return;
        };
        if let Err(error) = self.app.save_session() {
            tracing::warn!(%error, "session not saved before the relaunch");
        }
        tracing::info!(binary = %binary.display(), "continuing from the installed copy");
        relaunch(binary);
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
        let key = (
            settings.theme,
            settings.font_size,
            settings.density,
            settings.titlebar(),
        );
        if self.applied != Some(key) {
            ctx.set_theme(match settings.theme {
                Theme::System => egui::ThemePreference::System,
                Theme::Light => egui::ThemePreference::Light,
                Theme::Dark => egui::ThemePreference::Dark,
            });
            ctx.set_zoom_factor(f32::from(settings.font_size) / crate::theme::TEXT_BODY);
            // The scheme and density tokens on both of egui's themes, so
            // following the system keeps working.
            let (light, dark) = settings.titlebar();
            let band = (crate::theme::rgb(light), crate::theme::rgb(dark));
            crate::theme::apply(ctx, settings.density, band);
            self.applied = Some(key);
        }
        if settings.shortcuts != self.keys_from {
            self.keys = KeyMap::of(&keys::rows_of(&self.app), settings);
            self.keys_from = settings.shortcuts.clone();
        }
    }

    /// The pane keyboard input goes to in `window`.
    fn focused_in(&self, window: WindowId) -> PaneId {
        self.app
            .model()
            .window(window)
            .map_or(self.app.model().focused, model::Window::focused)
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
    /// Whether a dialogue is open: it has the keyboard, not the list
    /// behind it. Every dialogue the window shows is named here — one left
    /// out lets the list take its keys, Escape before all (issue 177).
    fn dialogue_open(&self) -> bool {
        self.dialog.is_some()
            || self.name_prompt.is_some()
            || self.shortcuts.open
            || self.extensions.is_open()
            || self.about.is_open()
            || self.properties.is_open()
            || self.hash.is_open()
            || self.integration.is_open()
            || !self.app.model().conflicts.is_empty()
            || !self.app.model().questions.is_empty()
    }

    /// The intents of the copy, cut and paste chords of this frame, for a
    /// list that has the keyboard: the window delivers them as events of
    /// their own, and they go through the keymap like any key press.
    fn clipboard_intents(
        &mut self,
        ctx: &egui::Context,
        window: WindowId,
        in_preview: bool,
    ) -> Vec<Intent> {
        let chords = self.chords.entry(window).or_default();
        ctx.input_mut(|input| chords.take(input))
            .into_iter()
            .filter_map(|chord| {
                self.keys
                    .match_key(chord.modifiers, chord.key, None, in_preview)
            })
            .collect()
    }

    /// What Escape calls off before the key clears a filter or the
    /// selection: Properties that is on its way, its entry still being
    /// read (issue 178), and a download for the preview (SPEC.md *Remote
    /// sources*). Returns whether the key was taken for one of them.
    fn escape_calls_off(&mut self, ctx: &egui::Context) -> bool {
        let escape = |ctx: &egui::Context| {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        };
        if self.app.properties_pending() && escape(ctx) {
            self.requests
                .push(Request::Command(Command::PropertiesClosed));
            return true;
        }
        if let Some(preview) = self.app.preview_pane()
            && self
                .previews
                .get(preview)
                .is_some_and(crate::preview::Slot::downloading)
            && escape(ctx)
        {
            self.requests
                .push(Request::Command(Command::PreviewCancel(preview)));
            return true;
        }
        false
    }

    fn handle_keys(&mut self, ctx: &egui::Context, window: WindowId) {
        // The copy, cut and paste chords are followed in every frame,
        // whoever has the keyboard: a paste is told by its key's release.
        let now = Instant::now();
        let chords = self.chords.entry(window).or_default();
        ctx.input(|input| chords.track(input, now));
        let pane = self.explorer_pane(self.focused_in(window));
        if self.dialogue_open() {
            return;
        }
        // An open menu owns the keyboard: Escape closes it, nothing else —
        // not even the side panel, whose row the menu may belong to.
        if egui::Popup::is_any_open(ctx) {
            return;
        }
        if self.escape_calls_off(ctx) {
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
        intents.extend(self.clipboard_intents(ctx, window, in_preview));
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
                    Request::About => self.about.open(),
                    Request::PluginSettings(id) => self.settings.open_plugin(self.root, &id),
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
                        let now = ctx.input(|i| i.time);
                        self.commit_edit(pane, kind, &name, now);
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
        for pane in w.panes() {
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
        for pane in w.dock().panes() {
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
    /// then selects the result once it is listed. A new folder's name the
    /// provider's own rules refuse — a container's — is refused in place,
    /// before anything is asked of the provider: the row stays in edit,
    /// shaking as for a name that is taken, and a notice names the rule.
    fn commit_edit(&mut self, pane: PaneId, kind: EditKind, name: &str, now: f64) {
        let Some((tab_id, tab)) = self.app.model().active_tab(pane) else {
            return;
        };
        if kind == EditKind::NewFolder {
            let at = Location::new(
                tab.location.provider,
                tab.location.path.join(name.as_bytes()),
            );
            let problem = self
                .app
                .mounts()
                .resolve(&at)
                .ok()
                .and_then(|provider| provider.name_problem(&at));
            if let Some(problem) = problem {
                self.tabs.entry(tab_id).or_default().edit = Some(RowEdit::refused(kind, name, now));
                self.toast(Level::Warning, problem);
                return;
            }
        }
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
        // A new tab opens in the focused pane; otherwise the pane asked
        // for is the one that goes there, focused or not.
        let pane = if new_tab {
            self.handle(Command::NewTab(to));
            self.app.model().focused
        } else {
            self.handle(Command::Navigate { pane, to });
            pane
        };
        self.requests.push(Request::SelectNamed {
            pane,
            name: name.to_vec(),
        });
    }

    /// Whether the provider at `location` offers `needed`: the inline
    /// edits start only where New file (`CREATE_FILE`) and New folder
    /// (`CREATE_DIR`) can finish.
    fn offers(&self, location: &Location, needed: ext::Capabilities) -> bool {
        self.app
            .mounts()
            .resolve(location)
            .is_ok_and(|p| p.capabilities(location).contains(needed))
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
            "core.new_file"
                if !self.inline_commit
                    && self.offers(&ctx.location, ext::Capabilities::CREATE_FILE) =>
            {
                self.requests.push(Request::StartEdit {
                    pane: ctx.pane,
                    kind: EditKind::NewFile,
                });
                true
            }
            "core.new_folder"
                if !self.inline_commit
                    && self.offers(&ctx.location, ext::Capabilities::CREATE_DIR) =>
            {
                self.requests.push(Request::StartEdit {
                    pane: ctx.pane,
                    kind: EditKind::NewFolder,
                });
                true
            }
            // Download… asks for the folder with the system's dialogue
            // before the action runs with it as the answer; where the
            // action is not available it runs, and says why.
            "core.download"
                if !self.inline_commit
                    && self
                        .app
                        .registry()
                        .actions()
                        .iter()
                        .find(|a| a.id() == action)
                        .is_some_and(|a| a.availability(ctx) == ext::Availability::Enabled) =>
            {
                self.pick_download_folder(ctx.clone());
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
                if let Some((_, id)) = self.app.model().workspace_of(pane) {
                    self.close_workspace(id);
                }
            }
            Intent::ReopenTab => {
                if let Some(location) = self.closed_tabs.pop() {
                    self.handle(Command::NewTab(location));
                }
            }
            Intent::NextTab => self.step_workspace(pane, 1),
            Intent::PrevTab => self.step_workspace(pane, -1),
            Intent::Tab(n) => {
                let order = self.workspace_order(pane);
                let index = if n == 9 {
                    order.len().saturating_sub(1)
                } else {
                    usize::from(n).saturating_sub(1)
                };
                if let Some(id) = order.get(index).copied() {
                    self.handle(Command::ActivateWorkspace(id));
                }
            }
            Intent::MoveTabLeft | Intent::MoveTabRight => {
                let model = self.app.model();
                if let Some((window, id)) = model.workspace_of(pane)
                    && let Some(at) = model.window(window).and_then(|w| w.position(id))
                {
                    let to = if intent == Intent::MoveTabLeft {
                        at.saturating_sub(1)
                    } else {
                        at + 1
                    };
                    self.handle(Command::MoveWorkspace { id, to });
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
                if let Some((window, keep)) = self.app.model().workspace_of(pane) {
                    let others: Vec<WorkspaceId> = self
                        .app
                        .model()
                        .workspaces(window)
                        .iter()
                        .map(|ws| ws.id)
                        .filter(|id| *id != keep)
                        .collect();
                    for id in others {
                        self.close_workspace(id);
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
                // Only where the location can be searched beneath: the
                // filter bar's toggle is disabled elsewhere, and the
                // shortcut does nothing.
                if let Some((tab_id, tab)) = self.app.model().active_tab(pane)
                    && self.app.subtree_search(&tab.location) == ext::Availability::Enabled
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
            Intent::Settings => self.settings.open(window),
            Intent::Shortcuts => self.shortcuts.open(),
            Intent::Extensions => self.extensions.open(),
            Intent::ViewList => self.handle(Command::SetView {
                pane,
                mode: ViewMode::List,
            }),
            Intent::ViewDetails => self.handle(Command::SetView {
                pane,
                mode: ViewMode::Details,
            }),
            Intent::SizeColumns => {
                if let Some((tab, _)) = self.app.model().active_tab(pane) {
                    self.handle(Command::ColumnWidths {
                        tab,
                        columns: BTreeMap::new(),
                    });
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

    /// The workspaces of the window holding `pane`, in the order of the
    /// title row.
    fn workspace_order(&self, pane: PaneId) -> Vec<WorkspaceId> {
        let model = self.app.model();
        model
            .window_of(pane)
            .map(|window| model.workspaces(window).iter().map(|ws| ws.id).collect())
            .unwrap_or_default()
    }

    /// Closes a workspace tab, remembering the folder of its focused pane
    /// so that *Reopen closed tab* brings it back.
    fn close_workspace(&mut self, id: WorkspaceId) {
        let model = self.app.model();
        if let Some(window) = model.window_of_workspace(id)
            && let Some(ws) = model.workspaces(window).iter().find(|ws| ws.id == id)
            && let Some((_, tab)) = model.active_tab(ws.focused)
            && tab.kind.lists()
        {
            self.closed_tabs.push(tab.location.clone());
        }
        self.handle(Command::CloseWorkspace(id));
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

    /// *Download…* asks for the target folder first (SPEC.md *Remote
    /// files*): the system's folder dialogue, starting in Downloads. The
    /// answer is polled each frame ([`poll_download_pick`](Self::poll_download_pick)).
    fn pick_download_folder(&mut self, ctx: ActionCtx) {
        if self.download_pick.is_some() {
            return;
        }
        let platform = Arc::clone(self.app.platform());
        let title = t!("download-to");
        let (tx, rx) = runtime::oneshot::channel();
        runtime::handle().spawn(async move {
            let start = platform
                .standard_folder(platform::StandardFolder::Downloads)
                .unwrap_or_else(|| platform.home_dir());
            let _ = tx.send(platform.pick_folder_in(&title, &start).await);
        });
        self.download_pick = Some(DownloadPick { ctx, answer: rx });
    }

    /// The folder the dialogue of *Download…* picked answers the action's
    /// question, and the download starts; a dismissed dialogue downloads
    /// nothing.
    fn poll_download_pick(&mut self, ctx: &egui::Context) {
        let Some(pick) = self.download_pick.as_mut() else {
            return;
        };
        match pick.answer.try_recv() {
            Ok(folder) => {
                let Some(pick) = self.download_pick.take() else {
                    return;
                };
                if let Some(folder) = folder {
                    self.host.answer_prompt(folder.display().to_string());
                    self.inline_commit = true;
                    self.run_action_with(ActionId("core.download"), pick.ctx);
                    self.inline_commit = false;
                    self.host.clear_answers();
                }
            }
            Err(runtime::oneshot::error::TryRecvError::Closed) => self.download_pick = None,
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

    fn step_workspace(&mut self, pane: PaneId, by: isize) {
        let order = self.workspace_order(pane);
        let Some((_, current)) = self.app.model().workspace_of(pane) else {
            return;
        };
        let Some(index) = order.iter().position(|id| *id == current) else {
            return;
        };
        let next = (index as isize + by).rem_euclid(order.len() as isize) as usize;
        self.handle(Command::ActivateWorkspace(order[next]));
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

    /// The title row (SPEC.md *Title row*): the menu slot above the side
    /// panel, the workspace tabs, the search box and the window group. An
    /// empty part of it drags the window and a double click maximises or
    /// restores it.
    fn title_row(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        window: WindowId,
        focused: PaneId,
    ) {
        let chrome = self.app.platform().window_chrome();
        let metrics = crate::theme::metrics(ctx);
        let height = metrics.titlebar_height;
        egui::Panel::top(egui::Id::new(("title", window.0)))
            .exact_size(height)
            .show_separator_line(false)
            .frame(egui::Frame::NONE.fill(crate::theme::band_of(ctx).fill))
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

                // Left: the menu slot. Room for the native controls, then
                // the burger, alone above the side panel when the panel is
                // on the left, so the first tab starts where the panes do.
                let mut left = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                left.add_space(metrics.space_toolbar_y);
                if chrome.native_controls && chrome.controls == ControlsSide::Left {
                    left.add_space(chrome.native_controls_width);
                }
                menus::burger(
                    &mut left,
                    &mut self.requests,
                    focused,
                    &self.app,
                    &self.keys,
                    &self.about.build().version,
                );
                let side = self
                    .app
                    .model()
                    .window(window)
                    .map(|w| w.side_panel.clone())
                    .unwrap_or_default();
                let burger_right = left.min_rect().right() + metrics.space_toolbar_y;
                let left_edge = if side.visible && side.side == PanelSide::Left {
                    burger_right.max(rect.left() + f32::from(side.width))
                } else {
                    burger_right
                };

                // Right: window controls, then the toggles.
                let mut right = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                );
                // The last icon ends `space/toolbar-x` from the edge: its
                // step reaches half an icon gap further.
                right.add_space((metrics.space_toolbar_x - WINDOW_ICON_GAP / 2.0).max(0.0));
                // The running count is text on the band, in its colours.
                let band = crate::theme::band(&right);
                let visuals = right.visuals_mut();
                visuals.weak_text_color = Some(band.title);
                visuals.widgets.noninteractive.fg_stroke.color = band.icon;
                self.title_right(&mut right, ctx, chrome, window, focused);
                let right_edge = right.min_rect().left();

                // The search box before the window group, then the tabs in
                // the room that is left.
                let tabs = self.app.model().workspaces(window).len();
                #[allow(clippy::cast_precision_loss)]
                let tabs_width = tabs as f32 * (crate::theme::TAB_WIDTH + TAB_GAP) + NEW_TAB_WIDTH;
                let search_left =
                    self.title_search(ui, rect, left_edge, right_edge, tabs_width, window, focused);
                self.title_tabs(
                    ui,
                    ctx,
                    rect,
                    left_edge,
                    search_left - metrics.space_toolbar_x,
                    window,
                    focused,
                );
            });
    }

    /// The workspace tabs from `left` to `right`, each `TAB_WIDTH` wide and
    /// `tab/height` tall on the bottom edge of the row, the new-tab button
    /// directly after the last; the row clips what does not fit. A tab
    /// dragged along the row reorders the tabs, its neighbours sliding
    /// apart to open the gap where it will land; one dragged off the row
    /// follows the cursor as its ghost. A tab holding exactly one pane —
    /// the model says so — is then offered the four side dock targets of
    /// the pane under the cursor, and released on one its pane docks
    /// there, the tab leaving the row; a tab of several panes is offered
    /// none. Released outside every window a tab opens a new one,
    /// released anywhere else it returns to its place. A pane's tab
    /// dragged over the row shows the slot where it would land as a new
    /// tab. A dragged pane, or a dragged tab that holds one pane, resting
    /// on another tab for the spring-loading delay switches the window to
    /// that tab's workspace (SPEC.md *Workspace tabs*).
    ///
    /// All of it holds for a tab or a pane dragged here from another
    /// window (SPEC.md *Windows*): that window publishes its drag as the
    /// [`Roam`], this one shows the gap, the slot, the targets and the
    /// lifted tab while the cursor is over it, and records what it offers
    /// for the other window to land the drop.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    fn title_tabs(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        rect: egui::Rect,
        left: f32,
        right: f32,
        window: WindowId,
        focused: PaneId,
    ) {
        use crate::theme::{ICON, TAB_WIDTH, TEXT_BODY};
        let metrics = crate::theme::metrics(ctx);
        let scheme = crate::theme::scheme(ui);
        let row = egui::Rect::from_min_max(
            egui::pos2(left, rect.bottom() - metrics.tab_height),
            egui::pos2(right.max(left), rect.bottom()),
        );
        let model = self.app.model();
        let active = model.window(window).map(|w| w.active);
        let tabs: Vec<(WorkspaceId, TabLabel)> = model
            .workspaces(window)
            .iter()
            .map(|ws| (ws.id, workspace_label(model, ws)))
            .collect();
        let still = self.app.platform().reduce_motion();
        let now = ctx.input(|i| i.time);
        // Another window's drag, which this window takes on its row and
        // its panes while the cursor is over it (SPEC.md *Windows*).
        let roam = self.roam;
        let origin = self.origin(window);
        let visiting = roam.and_then(|roam| roam.visiting_tab(window));
        let visiting_label = roam.zip(visiting).and_then(|(roam, tab)| {
            model
                .workspaces(roam.from)
                .iter()
                .find(|ws| ws.id == tab.id)
                .map(|ws| workspace_label(model, ws))
        });
        let visiting_pane = roam.is_some_and(|roam| roam.visiting_pane(window).is_some());
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        w.tab_row = Some(row);
        w.tab_headers.clear();
        let pitch = TAB_WIDTH + TAB_GAP;
        let pointer = ctx.pointer_latest_pos();
        let primary_down = ctx.input(|i| i.pointer.primary_down());
        let released = ctx.input(|i| i.pointer.any_released());
        // The cursor of the drag while it is over this window, in the
        // window's coordinates: its own pointer for a drag that began
        // here, the published place for another window's. Over another
        // window, that one shows the gap, the targets and the ghost.
        let cursor = roam.map_or(pointer, |roam| roam.cursor_in(window, origin, pointer));

        // A drag of a tab ends when the button comes up.
        let drag = w.workspace_drag.filter(|_| primary_down || released);
        if drag.is_none() {
            w.workspace_drag = None;
        }
        let dragged = drag.and_then(|d| tabs.iter().position(|(id, _)| *id == d.id));
        let others = tabs.len() - usize::from(dragged.is_some());
        // The tab over this window: its own, or another window's.
        let carried = drag.filter(|_| dragged.is_some()).or(visiting);
        let pane_carried = w.pane_drag.is_some() || visiting_pane;
        // The workspace to come back to when the drag moves nothing: the
        // one shown when it began, in whichever window it began.
        if drag.is_some() || w.pane_drag.is_some() || roam.is_some() {
            w.drag_origin = w.drag_origin.or(active);
        } else {
            w.drag_origin = None;
        }
        // The dragged tab rides the row while the cursor is in the title
        // row; off it, the tab follows the cursor as its own ghost and the
        // row keeps no room for it (QUESTIONS.md 273).
        let riding = carried.is_some() && cursor.is_some_and(|p| rect.contains(p));
        // The slot a tab's left edge at `x` lands in, among `count` tabs.
        let slot_of = |x: f32, count: usize| -> usize {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let slot = ((x - row.left()) / pitch).round().max(0.0) as usize;
            slot.min(count)
        };
        // The gap is where the dragged tab would land: any place of the
        // row, the last one included.
        let drag_slot = carried
            .filter(|_| riding)
            .and_then(|d| cursor.map(|p| slot_of(p.x - d.grab.x, others)));
        let pane_over_row =
            pane_carried && cursor.is_some_and(|p| row.expand2(egui::vec2(0.0, 6.0)).contains(p));
        let drop_slot = pane_over_row.then(|| cursor.map(|p| slot_of(p.x, tabs.len())));
        w.pane_drop_slot = drop_slot.flatten();
        // Off the row, a tab that holds exactly one pane can be docked: the
        // pane under the cursor shows the four side targets, and the tile
        // under the cursor is where the tab's pane would go. The model says
        // whether the tab holds one pane; a tab of several is offered
        // nothing. The tab's own pane, when its workspace is the one shown,
        // is no target for itself.
        let dock = carried
            .filter(|_| !riding)
            .and_then(|d| model.sole_pane(d.id))
            .zip(cursor)
            .and_then(|(sole, p)| {
                let (pane, rect) = w
                    .dock
                    .pane_rects(model)
                    .into_iter()
                    .find(|(pane, rect)| *pane != sole && rect.contains(p))?;
                Some((pane, rect, Cross::Tab.zone_at(rect, p)))
            });
        // The tabs slide to their places while something is dragged over
        // the row and for one slide after it, so the gap opens and closes
        // animated and a dropped tab settles into it; at any other time,
        // and under the system's reduce-motion setting, they are simply
        // where they belong (QUESTIONS.md 269).
        if drag.is_some() || visiting.is_some() || pane_over_row {
            w.slide_until = now + crate::drag::TAB_SLIDE;
        }
        let sliding = !still && now <= w.slide_until;
        w.tab_slides
            .retain(|key| key.is_none_or(|id| tabs.iter().any(|(tab, _)| *tab == id)));

        // What a dragged tab leans on: riding the row it is under the
        // cursor, so it rests on a neighbour while it overlaps it by a
        // quarter of a tab or more, short of the half where the two would
        // swap (QUESTIONS.md 272). Only a tab of one pane spring-loads; it
        // is the one that can be docked where the spring leads.
        let lean = carried
            .filter(|d| riding && model.sole_pane(d.id).is_some())
            .zip(drag_slot)
            .zip(cursor)
            .map(|((d, slot), p)| {
                let x =
                    (p.x - d.grab.x).clamp(row.left(), (row.right() - TAB_WIDTH).max(row.left()));
                (slot, x - (row.left() + slot as f32 * pitch))
            });
        let quarter = TAB_WIDTH / 4.0 + TAB_GAP;
        let mut rested_on: Option<WorkspaceId> = None;

        let clip = row.intersect(ui.clip_rect());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(row));
        child.set_clip_rect(clip);
        let mut x = row.left();
        let mut shown = 0usize;
        let mut clicked: Option<WorkspaceId> = None;
        let mut closed: Option<WorkspaceId> = None;
        let mut started: Option<TabDrag> = None;
        let slot_rect = |x: f32| {
            egui::Rect::from_min_size(
                egui::pos2(x, row.top() + 4.0),
                egui::vec2(DROP_SLOT_WIDTH, (row.height() - 4.0).max(1.0)),
            )
        };
        let paint_slot = |ui: &egui::Ui, x: f32| {
            let slot = slot_rect(x);
            ui.painter()
                .rect_filled(slot, metrics.corner_radius(), scheme.accent_button);
            ui.painter().text(
                slot.center(),
                egui::Align2::CENTER_CENTER,
                t!("shell-drop-for-new-tab"),
                egui::FontId::proportional(TEXT_BODY),
                crate::theme::Scheme::ON_BUTTON,
            );
        };
        for (i, (id, label)) in tabs.iter().enumerate() {
            if dragged == Some(i) {
                continue;
            }
            if drop_slot.flatten() == Some(shown) {
                paint_slot(&child, x);
                x += DROP_SLOT_WIDTH + TAB_GAP;
            }
            if drag_slot == Some(shown) {
                // The dragged tab's room: a gap, never an insertion line.
                x += pitch;
            }
            let drawn = if sliding {
                w.tab_slides.at(Some(*id), x, now)
            } else {
                w.tab_slides.put(Some(*id), x, now)
            };
            let tab_rect = egui::Rect::from_min_size(
                egui::pos2(drawn, row.top()),
                egui::vec2(TAB_WIDTH, row.height()),
            );
            let (response, close) =
                workspace_tab(&mut child, tab_rect, *id, label, Some(*id) == active);
            if let Some(tab) = label.tab {
                w.tab_headers.insert(tab, tab_rect);
            }
            // Spring-loaded tabs: a dragged pane rests on the tab the
            // cursor is on, a dragged tab on the neighbour it leans on —
            // the one after its gap or the one before it. The shown tab
            // has nothing to spring to.
            if Some(*id) != active {
                let rests = if pane_carried {
                    cursor.is_some_and(|p| tab_rect.contains(p))
                } else {
                    lean.is_some_and(|(slot, lean)| {
                        (lean >= quarter && shown == slot)
                            || (lean <= -quarter && shown + 1 == slot)
                    })
                };
                if rests {
                    rested_on = Some(*id);
                }
            }
            if response.clicked() {
                clicked = Some(*id);
            }
            if response.middle_clicked() || close {
                closed = Some(*id);
            }
            if response.drag_started()
                && let Some(p) = pointer
            {
                // Taken where the button went down, not where the drag was
                // recognised a few points on.
                let press = ctx.input(|i| i.pointer.press_origin()).unwrap_or(p);
                started = Some(TabDrag {
                    id: *id,
                    grab: press - tab_rect.min,
                });
            }
            if let Some(location) = &label.location {
                response.on_hover_text(paths::text_of(location));
            }
            // Entries dragged over the tab go into its folder; resting on it
            // shows the workspace.
            if w.dnd.active()
                && let Some(target) = &label.drop
            {
                let highlight = w.dnd.zone(Zone {
                    key: egui::Id::new(("dnd-workspace", id.0)),
                    rect: tab_rect,
                    target: target.clone(),
                });
                if highlight {
                    crate::dnd::highlight(child.painter(), &child.style().visuals, tab_rect);
                }
            }
            x += pitch;
            shown += 1;
        }
        if drop_slot.flatten() == Some(shown) {
            paint_slot(&child, x);
            x += DROP_SLOT_WIDTH + TAB_GAP;
        }
        if drag_slot == Some(shown) {
            x += pitch;
        }

        // The new-tab button, a ghost button directly after the last tab,
        // sliding with the tabs.
        let drawn = if sliding {
            w.tab_slides.at(None, x, now)
        } else {
            w.tab_slides.put(None, x, now)
        };
        let plus_rect = egui::Rect::from_min_size(
            egui::pos2(drawn, row.top()),
            egui::vec2(NEW_TAB_WIDTH, row.height()),
        );
        let plus = child.interact(
            plus_rect,
            egui::Id::new(("new-tab", window.0)),
            egui::Sense::click(),
        );
        plus.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t!("shell-new-tab"))
        });
        let band = crate::theme::band(&child);
        if plus.hovered() {
            child
                .painter()
                .rect_filled(plus_rect, metrics.corner_radius(), band.hover);
        }
        child.painter().text(
            plus_rect.center(),
            egui::Align2::CENTER_CENTER,
            icons::PLUS,
            egui::FontId::proportional(ICON),
            band.icon,
        );
        let plus_clicked = plus.on_hover_text(t!("shell-new-tab")).clicked();

        // The dragged tab is its own ghost (SPEC.md *Drag ghost*): the tab
        // itself, lifted with a shadow. In the title row it rides the row
        // under the pointer, held where it was grabbed; off the row it
        // follows the pointer, the grab point `GHOST_OFFSET` away from it.
        // A tab from another window is painted by the window the cursor is
        // over, this one, as the tab's own window would paint it.
        w.ghost = w.ghost.filter(|g| g.of != crate::drag::Dragged::Tab);
        let lifted = carried.zip(cursor).and_then(|(d, p)| {
            let label = dragged.map(|i| &tabs[i].1).or(visiting_label.as_ref())?;
            Some((d, p, label))
        });
        if let Some((d, p, label)) = lifted {
            let size = egui::vec2(TAB_WIDTH, row.height());
            let float = if riding {
                let x =
                    (p.x - d.grab.x).clamp(row.left(), (row.right() - TAB_WIDTH).max(row.left()));
                // Released, it settles into the gap from where it rides.
                w.tab_slides.put(Some(d.id), x, now);
                egui::Rect::from_min_size(egui::pos2(x, row.top()), size)
            } else {
                // Off the row its place in the row is the one it came from.
                if let Some(i) = dragged {
                    w.tab_slides
                        .put(Some(d.id), row.left() + i as f32 * pitch, now);
                }
                egui::Rect::from_min_size(p + crate::drag::GHOST_OFFSET - d.grab, size)
            };
            let layer = egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new(("workspace-drag", window.0)),
            );
            let mut top = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(float));
            // Over the whole window, not the title row alone: off the row
            // the tab is over the panes, and its shadow falls on them.
            top.set_clip_rect(ctx.content_rect());
            crate::drag::lift(
                top.painter(),
                float,
                egui::CornerRadius {
                    nw: crate::theme::TAB_CORNER,
                    ne: crate::theme::TAB_CORNER,
                    sw: 0,
                    se: 0,
                },
            );
            let (_, _) = workspace_tab(&mut top, float, d.id, label, true);
            // The dock targets over the lifted tab, not under it: the
            // cursor is inside the tab, which is wider than the cross and
            // would hide the targets it is being taken to.
            if let Some((_, rect, zone)) = dock {
                paint_dock_cross(top.painter(), rect, zone, Cross::Tab, scheme);
            }
            w.ghost = Some(crate::drag::Ghost {
                of: crate::drag::Dragged::Tab,
                rect: float,
                shapes: 0,
            });
        }
        // What a screen reader hears of the drag, from the window it
        // began in, wherever the cursor is.
        if let Some(i) = dragged {
            crate::drag::announce(
                ui,
                egui::Id::new(("drag-status", window.0)),
                &t!("shell-dragging-tab", title = tabs[i].1.title.as_str()),
            );
        }
        // Where another window's drag would land here, for that window to
        // land it; a pane's dock target is added when the panes are drawn.
        w.offer = drag_slot
            .or(drop_slot.flatten())
            .map(Offer::Row)
            .or_else(|| {
                dock.and_then(|(pane, _, zone)| zone.map(|zone| Offer::Dock { pane, zone }))
            });
        if sliding && w.tab_slides.moving(now) {
            ctx.request_repaint();
        }
        // The spring: the same tab rested on for the delay — the one
        // setting every kind of drag shares — shows its workspace, once.
        // A dragged tab leans on a neighbour for most of the way past it,
        // so it rests only where it is held still: a tab on the move
        // starts the clock again, and a slow reorder switches nothing.
        let at = cursor.unwrap_or_default();
        let on_the_move = !pane_carried
            && w.tab_rest
                .is_some_and(|resting| (at - resting.at).length() > crate::drag::REST_SLOP);
        w.tab_rest = match (w.tab_rest, rested_on) {
            (Some(resting), Some(on)) if resting.on == on && !on_the_move => Some(resting),
            (_, Some(on)) => Some(TabRest {
                on,
                since: now,
                at,
                sprung: false,
            }),
            (_, None) => None,
        };
        let mut spring: Option<WorkspaceId> = None;
        if let Some(resting) = &mut w.tab_rest
            && !resting.sprung
        {
            let delay = model.settings.drag.spring_delay_ms as f64 / 1000.0;
            let left = delay - (now - resting.since);
            if left <= 0.0 {
                resting.sprung = true;
                spring = Some(resting.on);
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64(left));
            }
        }
        if let Some(d) = started {
            w.workspace_drag = Some(d);
        }
        let dropped = drag.filter(|_| released).map(|d| (d, pointer));
        if dropped.is_some() {
            w.workspace_drag = None;
        }
        let in_window = w
            .content
            .is_none_or(|c| pointer.is_some_and(|p| c.contains(p)));

        if plus_clicked {
            if ctx.input(|i| i.modifiers.shift) {
                self.requests.push(Request::Command(Command::NewWindow));
            } else if let Some((_, tab)) = self.app.model().active_tab(focused) {
                self.requests.push(Request::NewTabAt(tab.location.clone()));
            }
        }
        if let Some(id) = clicked
            && drag.is_none()
        {
            self.requests
                .push(Request::Command(Command::ActivateWorkspace(id)));
        }
        if let Some(id) = spring {
            self.requests
                .push(Request::Command(Command::ActivateWorkspace(id)));
        }
        if let Some(id) = closed {
            self.close_workspace(id);
        }
        // Over another window, that window says where the tab lands.
        let elsewhere = roam
            .and_then(|roam| roam.over)
            .filter(|over| *over != window);
        if dropped.is_some() {
            self.roam = None;
        }
        match (dropped, elsewhere) {
            (Some((d, _)), Some(other)) => self.land_tab(window, other, d.id),
            // Released on the row: into the room the row kept for it. On
            // a dock target: its one pane docks there, and the model takes
            // the tab out of the row. Anywhere else in the window it
            // returns to its place (QUESTIONS.md 274).
            (Some((d, _)), None) if in_window => {
                if let Some(to) = drag_slot {
                    self.handle(Command::MoveWorkspace { id: d.id, to });
                } else if let Some((pane, _, Some(zone))) = dock
                    && let Some(side) = zone.side()
                {
                    self.handle(Command::DockWorkspace {
                        id: d.id,
                        pane,
                        side,
                    });
                } else {
                    // Nothing moved: every window shows what it showed
                    // when the drag began (QUESTIONS.md 276).
                    let _ = self.return_all_to_origin(window);
                }
            }
            (Some((d, Some(pos))), None) => self.drop_workspace_outside(window, d.id, pos),
            _ => {}
        }
    }

    /// Shows again the workspace `window` showed when the drag of a pane or
    /// a tab began, if spring-loading has switched it since: how such a
    /// drag ends when it moved nothing (QUESTIONS.md 276). Returns whether
    /// the window switched back, which whoever holds the dock's tree has
    /// to know: the tree then shows a workspace the model no longer does.
    fn return_to_origin(&mut self, window: WindowId) -> bool {
        let Some(origin) = self.windows.get(&window).and_then(|w| w.drag_origin) else {
            return false;
        };
        let back = self
            .app
            .model()
            .window(window)
            .is_some_and(|w| w.active != origin && w.position(origin).is_some());
        if back {
            self.handle(Command::ActivateWorkspace(origin));
        }
        back
    }

    /// Every window shows again the workspace it showed when the drag of
    /// a pane or a tab began: the drag went over other windows, whose
    /// tabs spring-load alike, and it moved nothing. Returns whether
    /// `window` switched back.
    fn return_all_to_origin(&mut self, window: WindowId) -> bool {
        let windows: Vec<WindowId> = self.windows.keys().copied().collect();
        let mut back = false;
        for id in windows {
            let switched = self.return_to_origin(id);
            back |= switched && id == window;
        }
        back
    }

    /// Escape cancels the drag of a pane or of a tab in progress (SPEC.md
    /// *No-ops and cancel*): the ghost goes, nothing moves, the tabs are
    /// back in their order and every window shows the workspace it showed
    /// when the drag began, whatever spring-loading has switched since.
    /// The key is consumed, so it reaches nothing else, and the button
    /// still held starts nothing new. Called before `window`'s frame
    /// draws, so the dock is drawn from the model as it then is; the key
    /// reaches the window the drag began in, or, where the windows share
    /// one input, whichever is drawn first.
    fn cancel_layout_drag(&mut self, ctx: &egui::Context, window: WindowId) {
        let from = self.roam.map_or(window, |roam| roam.from);
        if self
            .windows
            .get(&from)
            .is_none_or(|w| w.pane_drag.is_none() && w.workspace_drag.is_none())
        {
            return;
        }
        if !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            return;
        }
        if let Some(w) = self.windows.get_mut(&from) {
            w.pane_drag = None;
            w.pane_ghost = None;
            w.workspace_drag = None;
        }
        self.roam = None;
        // egui would report the handle or the tab as dragged until the
        // button comes up.
        ctx.stop_dragging();
        let _ = self.return_all_to_origin(window);
        for w in self.windows.values_mut() {
            w.ghost = None;
            w.tab_rest = None;
            w.pane_drop_slot = None;
            w.offer = None;
            w.drag_origin = None;
        }
    }

    /// Where `window`'s coordinates begin among those the windows share:
    /// its place on the screen, or the root's own origin where the windows
    /// are drawn inside the root. `None` while the system does not say
    /// where the window is.
    fn origin(&self, window: WindowId) -> Option<egui::Vec2> {
        if self.embedded {
            return Some(egui::Vec2::ZERO);
        }
        let screen = self.windows.get(&window)?.screen?;
        Some(screen.min.to_vec2())
    }

    /// The window under a place in the shared coordinates, for a drag that
    /// began in `from`. Windows may overlap, and nothing says which is in
    /// front: the one the drag began in was clicked last, so it is; drawn
    /// inside the root, every other window lies over the root.
    fn window_under(&self, at: egui::Pos2, from: WindowId) -> Option<WindowId> {
        let holds = |id: &WindowId| {
            self.origin(*id)
                .zip(self.windows.get(id).and_then(|w| w.content))
                .is_some_and(|(origin, content)| content.translate(origin).contains(at))
        };
        let root_last = self.embedded || from != self.root;
        let first = (!(self.embedded && from == self.root)).then_some(from);
        let others = self
            .windows
            .keys()
            .copied()
            .filter(|id| *id != from && *id != self.root);
        let last = (root_last && Some(self.root) != first).then_some(self.root);
        first.into_iter().chain(others).chain(last).find(holds)
    }

    /// Publishes the drag of a pane or a tab in progress in `window` for
    /// the other windows, with the cursor's place among them, and
    /// withdraws it when the drag is over. Called as the window's frame
    /// begins.
    fn publish_roam(&mut self, ctx: &egui::Context, window: WindowId) {
        let Some(w) = self.windows.get(&window) else {
            return;
        };
        let what = w
            .pane_drag
            .map(Carried::Pane)
            .or(w.workspace_drag.map(Carried::Tab));
        let Some((what, pointer)) = what.zip(ctx.pointer_latest_pos()) else {
            if self.roam.is_some_and(|roam| roam.from == window) {
                self.roam = None;
            }
            return;
        };
        let content = w.content;
        self.roam = Some(match self.origin(window) {
            Some(origin) => {
                let at = pointer + origin;
                Roam {
                    from: window,
                    what,
                    at,
                    over: self.window_under(at, window),
                }
            }
            // The system does not say where the windows are: the drag
            // knows its own window alone.
            None => Roam {
                from: window,
                what,
                at: pointer,
                over: content
                    .is_none_or(|c| c.contains(pointer))
                    .then_some(window),
            },
        });
    }

    /// A workspace tab released over another window: into the gap its tab
    /// row opened for it, or, holding one pane, onto the dock target of
    /// the pane under the cursor; anywhere else there nothing moves
    /// (SPEC.md *Windows*).
    fn land_tab(&mut self, window: WindowId, other: WindowId, id: WorkspaceId) {
        match self.windows.get(&other).and_then(|w| w.offer) {
            Some(Offer::Row(to)) => {
                self.handle(Command::WorkspaceToWindow {
                    id,
                    window: Some(other),
                });
                self.handle(Command::MoveWorkspace { id, to });
            }
            Some(Offer::Dock { pane, zone }) => {
                if let Some(side) = zone.side() {
                    self.handle(Command::DockWorkspace { id, pane, side });
                }
            }
            None => {
                let _ = self.return_all_to_origin(window);
            }
        }
    }

    /// A pane released over another window: a new workspace tab at the
    /// slot its tab row showed, or docked on the target of the pane under
    /// the cursor, whose centre makes it a new tab there. Returns whether
    /// it landed.
    fn land_pane(&mut self, other: WindowId, tab: TabId) -> bool {
        let Some(offer) = self.windows.get(&other).and_then(|w| w.offer) else {
            return false;
        };
        let side = match offer {
            Offer::Dock { pane, zone } => zone.side().map(|side| (pane, side)),
            Offer::Row(_) => None,
        };
        if let Some((pane, side)) = side {
            self.handle(Command::MoveTab {
                tab,
                to: DockTarget::Beside { pane, side },
            });
            return true;
        }
        self.handle(Command::MoveTab {
            tab,
            to: DockTarget::NewWorkspace,
        });
        if let Some(id) = self.workspace_of_tab(tab) {
            self.handle(Command::WorkspaceToWindow {
                id,
                window: Some(other),
            });
            if let Offer::Row(to) = offer {
                self.handle(Command::MoveWorkspace { id, to });
            }
        }
        true
    }

    /// A workspace tab released outside every window: a window opens for
    /// it.
    fn drop_workspace_outside(&mut self, window: WindowId, id: WorkspaceId, pos: egui::Pos2) {
        let Some((screen_pos, size)) = self.on_screen(window, pos) else {
            return;
        };
        self.handle(Command::WorkspaceToWindow { id, window: None });
        self.place_new_window(window, screen_pos, size);
    }

    /// A position in `window`'s coordinates on the screen, with the
    /// window's size; `None` while the window's place on the screen is
    /// unknown (a test harness).
    fn on_screen(&self, window: WindowId, pos: egui::Pos2) -> Option<(egui::Pos2, egui::Vec2)> {
        let screen = self.windows.get(&window)?.screen?;
        Some((screen.min + pos.to_vec2(), screen.size()))
    }

    /// Puts the window the model just opened under the pointer, as large
    /// as the one it came from.
    fn place_new_window(&mut self, from: WindowId, screen_pos: egui::Pos2, size: egui::Vec2) {
        let new = self.app.model().focused_window;
        if new == from {
            return;
        }
        let height = crate::theme::metrics(&self.ctx).titlebar_height;
        self.handle(Command::WindowGeometry {
            window: new,
            geometry: Geometry {
                x: screen_pos.x - 80.0,
                y: screen_pos.y - height / 2.0,
                width: size.x,
                height: size.y,
                maximised: false,
            },
        });
    }

    /// The side panel on its side with the operations drawer floating over
    /// its foot, or the drawer along the window's foot when the panel is
    /// hidden, so a running job is never invisible.
    fn side_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, window: WindowId) {
        let side = self
            .app
            .model()
            .window(window)
            .map(|w| w.side_panel.clone())
            .unwrap_or_default();
        let jobs = !self.app.model().jobs.is_empty();
        let still = self.app.platform().reduce_motion();
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        if !side.visible {
            if jobs {
                let mut collapsed = w.drawer_collapsed;
                let height = self.operations.height(self.app.model(), collapsed);
                egui::Panel::bottom(egui::Id::new(("operations", window.0)))
                    .exact_size(height)
                    .frame(egui::Frame::NONE)
                    .show_separator_line(false)
                    .show(ui, |ui| {
                        self.operations.show(
                            ui,
                            self.app.model(),
                            &mut self.requests,
                            &mut collapsed,
                            still,
                        );
                    });
                w.drawer_collapsed = collapsed;
            }
            return;
        }
        let panel = match side.side {
            PanelSide::Left => egui::Panel::left(egui::Id::new(("side", window.0))),
            PanelSide::Right => egui::Panel::right(egui::Id::new(("side", window.0))),
        };
        let shown = panel
            .frame(egui::Frame::new().fill(crate::theme::scheme(ui).bg_panel))
            .resizable(true)
            .default_size(f32::from(side.width))
            .size_range(120.0..=600.0)
            .show(ui, |ui| {
                let panel_rect = ui.max_rect();
                let height = if jobs {
                    self.operations.height(self.app.model(), w.drawer_collapsed)
                } else {
                    0.0
                };
                side_panel::show(
                    ui,
                    &self.app,
                    window,
                    &side,
                    &mut self.side,
                    &mut self.requests,
                    &mut w.dnd,
                    height,
                );
                if jobs {
                    // Drawn after the lists, so it floats over their end.
                    let drawer = egui::Rect::from_min_max(
                        egui::pos2(panel_rect.left(), panel_rect.bottom() - height),
                        panel_rect.max,
                    );
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(drawer)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    let mut collapsed = w.drawer_collapsed;
                    self.operations.show(
                        &mut child,
                        self.app.model(),
                        &mut self.requests,
                        &mut collapsed,
                        still,
                    );
                    w.drawer_collapsed = collapsed;
                }
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
        // The group's icons are 16 px and 12 px apart: each control is a
        // 28 px step that answers over its whole width (SPEC.md *Title
        // row*).
        right.spacing_mut().item_spacing.x = 0.0;
        if !chrome.native_controls {
            // Named apart from a dialogue's Close, which may be open.
            if window_control(right, icons::CLOSE, &t!("shell-close-window"), None).clicked() {
                self.requests.push(Request::CloseWindow(window));
            }
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            let (glyph, name) = if maximized {
                (icons::RESTORE, t!("shell-restore"))
            } else {
                (icons::MAXIMISE, t!("shell-maximise"))
            };
            if window_control(right, glyph, &name, None).clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            if window_control(right, icons::MINIMISE, &t!("shell-minimise"), None).clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            // The divider in the middle of the 25 px between the toggles'
            // icons and the window controls'.
            let band = crate::theme::band(right);
            let (gap, _) = right.allocate_exact_size(
                egui::vec2(WINDOW_GROUP_GAP - WINDOW_ICON_GAP, crate::theme::ICON),
                egui::Sense::hover(),
            );
            right.painter().vline(
                gap.center().x,
                gap.y_range(),
                egui::Stroke::new(1.0, band.divider),
            );
        }
        let preview_open = self.app.model().preview_tab(window).is_some();
        let name = t!("shell-toggle-preview-pane");
        if window_control(right, icons::PREVIEW, &name, Some(preview_open)).clicked() {
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
        let name = t!("shell-toggle-side-panel");
        if window_control(right, icons::SIDE_PANEL, &name, Some(side_visible)).clicked() {
            self.requests.push(Request::Intent {
                intent: Intent::ToggleSidePanel,
                pane: focused,
            });
        }
        right.add_space(crate::theme::metrics(ctx).space_toolbar_x);
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

    /// The search box, `SEARCH_WIDTH` wide and `search/height` tall before
    /// the window group, centred in the row; narrower only when the row
    /// cannot hold the tabs, the box and the controls, and absent when not
    /// even a narrow one fits. Returns its left edge, `right_edge` without
    /// a box.
    #[allow(clippy::too_many_arguments)]
    fn title_search(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        left_edge: f32,
        right_edge: f32,
        tabs_width: f32,
        window: WindowId,
        focused: PaneId,
    ) -> f32 {
        let metrics = crate::theme::metrics(ui.ctx());
        let gap = metrics.space_toolbar_x;
        let room = right_edge - left_edge - 2.0 * gap;
        let width = (room - tabs_width)
            .clamp(120.0, crate::theme::SEARCH_WIDTH)
            .min(room);
        if width < 120.0 {
            return right_edge;
        }
        let search_rect = egui::Rect::from_min_size(
            egui::pos2(
                right_edge - gap - width,
                rect.center().y - metrics.search_height / 2.0,
            ),
            egui::vec2(width, metrics.search_height),
        );
        let recents: Vec<Intent> = self.dispatched.iter().rev().map(|d| d.intent).collect();
        let Some(w) = self.windows.get_mut(&window) else {
            return right_edge;
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
        search_rect.left()
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
        let decided = crate::dialog::show(
            ctx,
            egui::Id::new(t!("shell-ask")),
            &dialog.ask,
            crate::dialog::State {
                values: &mut dialog.values,
                rows: &mut dialog.rows,
                input: &mut dialog.input,
                focus: &mut dialog.focus,
            },
        );
        if let Some((key, browse)) = decided.browse
            && dialog.browse.is_none()
        {
            let (tx, rx) = runtime::oneshot::channel();
            let title = match &dialog.ask {
                Ask::Form(form) => form.title.clone(),
                _ => String::new(),
            };
            let platform = Arc::clone(&platform);
            runtime::handle().spawn(async move {
                let picked = match browse {
                    crate::dialog::Browse::Folder => platform.pick_folder(&title).await,
                    crate::dialog::Browse::Files(filters) => {
                        let filters: Vec<platform::DialogFilter> = filters
                            .into_iter()
                            .map(|(name, extensions)| platform::DialogFilter { name, extensions })
                            .collect();
                        platform
                            .pick_files(&title, &filters)
                            .await
                            .into_iter()
                            .next()
                    }
                };
                let _ = tx.send(picked);
            });
            dialog.browse = Some((key, rx));
        }
        let mut outcome = decided.decided;
        if decided.dismissed && outcome.is_none() {
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

    /// The notices of `window` at its top right, under the title row: the
    /// ones tied to it and, when it is the window in `front`, the ones
    /// tied to none or to a window that is gone. A notice carries its
    /// whole text, wrapped and never cut; an error has a close button and
    /// stays until that is pressed.
    fn show_toasts(&mut self, ctx: &egui::Context, window: WindowId, front: bool) {
        let now = Instant::now();
        self.toasts
            .retain(|t| t.until.is_none_or(|until| until > now));
        let shown: Vec<usize> = self
            .toasts
            .iter()
            .enumerate()
            .filter(|(_, toast)| match toast.window {
                Some(of) if self.windows.contains_key(&of) => of == window,
                _ => front,
            })
            .map(|(index, _)| index)
            .collect();
        if shown.is_empty() {
            return;
        }
        let scheme = crate::theme::scheme_of(ctx);
        // Under the title row, at the right of what the window draws.
        let content = self
            .windows
            .get(&window)
            .and_then(|w| w.content)
            .unwrap_or_else(|| ctx.content_rect());
        let corner = egui::pos2(
            content.right() - TOAST_GAP,
            content.top() + crate::theme::metrics(ctx).titlebar_height + TOAST_GAP,
        );
        let mut dismissed = None;
        egui::Area::new(egui::Id::new(("toasts", window.0)))
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(corner)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for index in shown {
                    let toast = &self.toasts[index];
                    let sticky = toast.until.is_none();
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        let colour = match toast.level {
                            Level::Info => scheme.text_primary,
                            Level::Warning => scheme.status_warning,
                            Level::Error => scheme.status_danger,
                        };
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.allocate_ui(egui::vec2(TOAST_WIDTH, 0.0), |ui| {
                                ui.set_max_width(TOAST_WIDTH);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&toast.text).color(colour),
                                    )
                                    .wrap(),
                                );
                            });
                            if sticky {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(20.0, 20.0),
                                    egui::Sense::hover(),
                                );
                                let close = crate::properties::icon_button(
                                    ui,
                                    rect,
                                    icons::CLOSE,
                                    &t!("jobs-dismiss"),
                                );
                                if close.clicked() {
                                    dismissed = Some(index);
                                }
                            }
                        });
                    });
                }
            });
        if let Some(index) = dismissed {
            self.toasts.remove(index);
        }
        if self.toasts.iter().any(|toast| toast.until.is_some()) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
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
    /// The pane drag by a handle.
    pane_drag: &'a mut Option<PaneDrag>,
    /// Where the snapshot of a pane goes in the frame its drag starts.
    pane_ghost: &'a mut Option<crate::drag::Snapshot>,
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
        let model = self.app.model();
        model.tab(*tab).map_or_else(
            || "?".into(),
            |t| {
                let (icon, title) = tab_label(model, t);
                format!("{icon} {title}").into()
            },
        )
    }

    fn on_tab_button(&mut self, tab: &mut TabId, response: &egui::Response) {
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
        // A pane grabbed by its handle in this frame leaves its snapshot:
        // what it paints now is what its ghost shows (SPEC.md *Drag
        // ghost*).
        let dragging = self.pane_drag.is_some();
        let mark = crate::drag::mark(ui);
        let rect = ui.max_rect();
        self.pane(ui, *tab);
        if !dragging && self.pane_drag.is_some_and(|drag| drag.tab == *tab) {
            let ctx = ui.ctx();
            let grab = ctx
                .input(|i| i.pointer.press_origin().or_else(|| i.pointer.latest_pos()))
                .unwrap_or_else(|| rect.center());
            *self.pane_ghost = Some(crate::drag::Snapshot::take(ctx, mark, rect, grab));
        }
    }

    fn on_close(&mut self, _tab: &mut TabId) -> egui_dock::widgets::tab_viewer::OnCloseResponse {
        egui_dock::widgets::tab_viewer::OnCloseResponse::Close
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

impl Viewer<'_> {
    /// One pane: the tab's content, whatever its kind.
    fn pane(&mut self, ui: &mut egui::Ui, tab: TabId) {
        let model = self.app.model();
        let Some(pane) = model.pane_of(tab) else {
            return;
        };
        let Some(state) = model.tab(tab) else {
            return;
        };
        if state.kind == TabKind::Preview {
            let close = inspector::show(
                ui,
                self.app,
                self.previews,
                self.inspector_state,
                self.requests,
                &mut inspector::Handle {
                    drag: PaneDrag { tab, pane },
                    slot: self.pane_drag,
                },
            );
            if close {
                self.requests.push(Request::Command(Command::CloseTab(tab)));
            }
            return;
        }
        if state.kind.is_view() {
            match self.views.get(&tab) {
                Some(ext::View::Console(console)) => {
                    let state = self.consoles.entry(tab).or_default();
                    console::show(ui, state, console, tab);
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
            .entry(tab)
            .or_insert_with(|| TabUi::for_tab(state));
        let mut cx = PaneCx {
            app: self.app,
            pane,
            tab_id: tab,
            tab: state,
            tab_ui,
            requests: self.requests,
            keys: self.keys,
            now: self.now,
            focused: self.focused == pane && model.focused == pane,
            thumbnails: self.thumbnails,
            dnd: self.dnd,
            pane_drag: self.pane_drag,
        };
        pane::show(ui, &mut cx);
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

        let dock_id = egui::Id::new(("dock", window.0));
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
                pane_drag: &mut w.pane_drag,
                pane_ghost: &mut w.pane_ghost,
                focused,
                previews: &mut self.previews,
                inspector_state: &mut self.inspector_state,
                views: &self.views,
                consoles: &mut self.consoles,
            };
            DockArea::new(&mut w.dock.state)
                .id(dock_id)
                .style(dock_style(ui.style(), crate::theme::scheme(ui)))
                .hidable_tab_bars(true)
                .draggable_tabs(false)
                .show_add_buttons(false)
                .show_close_buttons(false)
                .show_leaf_close_all_buttons(false)
                .show_leaf_collapse_buttons(false)
                .show_inside(ui, &mut viewer);
            // The Preview tab's leaf, for tests.
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
                        .map(|(_, rect)| rect)
                });
        });
        let may_focus = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let mut events = Vec::new();
        if self.pane_drag(ui, ctx, window) == Some(true) {
            // The model moved on: the tree follows it, there is nothing of
            // the user's left in the tree to read.
            if let Some(w) = self.windows.get_mut(&window) {
                w.dock.rebuild(self.app.model());
            }
        } else if let Some(w) = self.windows.get_mut(&window) {
            w.dock.reconcile(
                &mut self.app,
                &mut events,
                ctx.pointer_latest_pos(),
                may_focus,
            );
        }
        self.absorb(events);
        self.visiting_pane(ctx, window);
        self.apply_requests(ctx);
    }

    /// A pane dragged from another window over this one (SPEC.md
    /// *Windows*): the pane under the cursor shows the dock targets, as
    /// it would for a pane of this window, and the ghost is painted here,
    /// the pane's own window having no say over these pixels. The target
    /// under the cursor is what this window offers the drop.
    fn visiting_pane(&mut self, ctx: &egui::Context, window: WindowId) {
        let roam = self.roam;
        let origin = self.origin(window);
        let Some(w) = self.windows.get(&window) else {
            return;
        };
        let own = w.pane_drag.is_some();
        let visiting = roam
            .and_then(|roam| Some((roam, roam.visiting_pane(window)?)))
            .and_then(|(roam, drag)| Some((roam, drag, roam.cursor_in(window, origin, None)?)));
        let Some((roam, drag, cursor)) = visiting else {
            if !own && let Some(w) = self.windows.get_mut(&window) {
                w.ghost = w.ghost.filter(|g| g.of != crate::drag::Dragged::Pane);
            }
            return;
        };
        let model = self.app.model();
        let under = w
            .dock
            .pane_rects(model)
            .into_iter()
            .find(|(pane, rect)| *pane != drag.pane && rect.contains(cursor));
        let zone = under.and_then(|(pane, rect)| Some((pane, Cross::Pane.zone_at(rect, cursor)?)));
        let overlay = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new(("pane-drag", window.0)),
        ));
        if let Some((_, rect)) = under {
            paint_dock_cross(
                &overlay,
                rect,
                zone.map(|(_, zone)| zone),
                Cross::Pane,
                crate::theme::scheme_of(ctx),
            );
        }
        let ghost = self
            .windows
            .get(&roam.from)
            .and_then(|from| from.pane_ghost.as_ref())
            .map(|snapshot| {
                let rect = snapshot.placed(ctx, cursor);
                snapshot.paint(ctx, overlay.layer_id(), rect);
                crate::drag::Ghost {
                    of: crate::drag::Dragged::Pane,
                    rect,
                    shapes: snapshot.len(),
                }
            });
        ctx.request_repaint();
        if let Some(w) = self.windows.get_mut(&window) {
            w.ghost = ghost;
            if let Some((pane, zone)) = zone {
                w.offer = Some(Offer::Dock { pane, zone });
            }
        }
    }

    /// The drag of a pane by its handle (SPEC.md *Panes*): while it lasts,
    /// the pane's ghost follows the pointer, the pane under it shows the five
    /// dock tiles with the hovered one lit and the half the pane would take
    /// previewed; on release the pane docks there, or lands on the tab row
    /// or outside the window as [`cross_window_drop`](Self::cross_window_drop)
    /// says. Returns `Some(handled)` when the drag ended this frame,
    /// `handled` telling whether the model changed.
    fn pane_drag(&mut self, ui: &egui::Ui, ctx: &egui::Context, window: WindowId) -> Option<bool> {
        let roam = self.roam;
        let origin = self.origin(window);
        let w = self.windows.get(&window)?;
        let drag = w.pane_drag?;
        let (pointer, down, released) = ctx.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.primary_down(),
                i.pointer.any_released(),
            )
        });
        // The cursor while it is over this window: over another, that one
        // shows the targets and the ghost (SPEC.md *Windows*).
        let cursor = roam.map_or(pointer, |roam| roam.cursor_in(window, origin, pointer));
        let model = self.app.model();
        let under = cursor.and_then(|p| {
            w.dock
                .pane_rects(model)
                .into_iter()
                .find(|(pane, rect)| *pane != drag.pane && rect.contains(p))
        });
        let zone = under.and_then(|(pane, rect)| {
            Cross::Pane
                .zone_at(rect, cursor?)
                .map(|zone| (pane, rect, zone))
        });
        if down && !released {
            let scheme = crate::theme::scheme_of(ctx);
            let overlay = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new(("pane-drag", window.0)),
            ));
            if let Some((_, rect)) = under {
                paint_dock_cross(
                    &overlay,
                    rect,
                    zone.map(|(_, _, zone)| zone),
                    Cross::Pane,
                    scheme,
                );
            }
            // The ghost: the pane as it was grabbed, anchored to the grab
            // point (SPEC.md *Drag ghost*).
            let ghost = cursor.zip(w.pane_ghost.as_ref()).map(|(p, snapshot)| {
                let rect = snapshot.placed(ctx, p);
                snapshot.paint(ctx, overlay.layer_id(), rect);
                crate::drag::Ghost {
                    of: crate::drag::Dragged::Pane,
                    rect,
                    shapes: snapshot.len(),
                }
            });
            // What a screen reader hears of the drag.
            let title = model
                .tab(drag.tab)
                .map(|t| tab_label(model, t).1)
                .unwrap_or_default();
            crate::drag::announce(
                ui,
                egui::Id::new(("drag-status", window.0)),
                &t!("shell-dragging-pane", title = title.as_str()),
            );
            ctx.request_repaint();
            if let Some(w) = self.windows.get_mut(&window) {
                w.ghost = ghost;
            }
            return None;
        }
        // The drag ended: with a release it may land somewhere; without one
        // (the button came up unseen) it just stops.
        if let Some(w) = self.windows.get_mut(&window) {
            w.pane_drag = None;
            w.pane_ghost = None;
            w.ghost = w.ghost.filter(|g| g.of != crate::drag::Dragged::Pane);
        }
        self.roam = None;
        if !released {
            return Some(self.return_all_to_origin(window));
        }
        if let Some((pane, _, zone)) = zone {
            let to = match zone.side() {
                Some(side) => DockTarget::Beside { pane, side },
                None => DockTarget::NewWorkspace,
            };
            self.handle(Command::MoveTab { tab: drag.tab, to });
            return Some(true);
        }
        // Released where it cannot land, nothing moved and every window
        // shows what it showed when the drag began; that is a change of
        // the model too, which the dock's tree has to follow, not report.
        let handled = match roam.and_then(|roam| roam.over) {
            // Over another window: where that window offered.
            Some(other) if other != window => self.land_pane(other, drag.tab),
            _ => self.cross_window_drop(ctx, window, Some(drag.tab)),
        };
        Some(handled || self.return_all_to_origin(window))
    }

    /// A pane released outside every dock tile of its window: on the
    /// window's tab row it becomes a new workspace tab at the slot shown,
    /// outside every window a window of its own. Returns whether a drop
    /// was handled.
    fn cross_window_drop(
        &mut self,
        ctx: &egui::Context,
        window: WindowId,
        dragging: Option<TabId>,
    ) -> bool {
        let Some(tab) = dragging else {
            return false;
        };
        let released = ctx.input(|i| i.pointer.any_released());
        if !released {
            return false;
        }
        let Some(pos) = ctx.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()))
        else {
            return false;
        };
        let Some(w) = self.windows.get(&window) else {
            return false;
        };
        if let Some(slot) = w.pane_drop_slot {
            self.handle(Command::MoveTab {
                tab,
                to: DockTarget::NewWorkspace,
            });
            if let Some(id) = self.workspace_of_tab(tab) {
                self.handle(Command::MoveWorkspace { id, to: slot });
            }
            return true;
        }
        if w.content.is_some_and(|c| c.contains(pos)) {
            return false;
        }
        let Some((screen_pos, size)) = self.on_screen(window, pos) else {
            // No screen position (a test harness): nothing to tear out to.
            return false;
        };
        self.handle(Command::MoveTab {
            tab,
            to: DockTarget::NewWindow,
        });
        self.place_new_window(window, screen_pos, size);
        true
    }

    /// The workspace holding a tab's pane.
    fn workspace_of_tab(&self, tab: TabId) -> Option<WorkspaceId> {
        let model = self.app.model();
        model.workspace_of(model.pane_of(tab)?).map(|(_, id)| id)
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
        if window == self.root
            && let Some(externals) = &mut self.externals
        {
            externals.drain(&mut self.app, &mut events);
        }
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
        self.publish_roam(ctx, window);
        self.cancel_layout_drag(ctx, window);
        self.resolve_pending_selects();
        self.poll_library_browse(ctx);
        self.poll_download_pick(ctx);
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
        let window_frame = frame::wanted(ctx, chrome)
            .then(|| frame::Frame::paint(ui, ui.max_rect(), self.frame_mode));
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
            self.shortcuts
                .show(ctx, &self.keys, &self.app, &mut self.requests);
            self.extensions.show(ctx, &self.app, &mut self.requests);
            self.about.show(ctx, &self.app, &mut self.requests);
            self.properties.show(ctx, &self.app, &mut self.requests);
            self.hash.show(ctx, &self.app, &mut self.requests);
            self.integration.show(ctx, &self.app, &mut self.requests);
            self.show_texts(ctx);
            self.apply_requests(ctx);
        }
        // A notice is shown in the window it belongs to, in front or not.
        let front = self.is_front(ctx, window);
        self.show_toasts(ctx, window, front);

        // The closed hand for as long as a pane or a tab is dragged,
        // whatever the cursor is over and in whichever window: set last,
        // so nothing drawn in this frame shows another cursor (SPEC.md
        // *Panes*: a drag never shows the arrow).
        if self.roam.is_some()
            || self
                .windows
                .get(&window)
                .is_some_and(|w| w.pane_drag.is_some() || w.workspace_drag.is_some())
        {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
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
                viewport_builder("Rick", chrome, geometry, self.frame_mode.transparent()),
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
    /// Transparent around Rick's shadow frame, so its shadow shows the
    /// desktop through the margin; the panel colour otherwise, and always
    /// behind the plain frame of a window that cannot be transparent.
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if self.frame_mode == frame::Mode::Shadow
            && self.windows.values().any(|w| w.window_frame.is_some())
        {
            [0.0; 4]
        } else {
            visuals.panel_fill.to_normalized_gamma_f32()
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let root = self.root;
        let ctx = ui.ctx().clone();
        self.embedded = ctx.embed_viewports();
        self.sync_window_ref(frame, &ctx);
        // The integration check: asked for once the first frame is drawn.
        self.integration.frame(&mut self.requests);
        self.draw_window(ui, root);
        // The Settings window: its own viewport, or an area over the root
        // window where the backend embeds viewports.
        self.settings.show(&ctx, &self.app, &mut self.requests);
        self.apply_requests(&ctx);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A drag published by one window is seen by the others where the
    /// cursor is in their own coordinates, and only by the one it is over
    /// (issue 80, rule 6): two windows side by side on a screen, the
    /// second 900 points right of the first.
    #[test]
    fn a_roaming_drag_is_seen_in_the_coordinates_of_the_window_it_is_over() {
        let (first, second) = (WindowId(1), WindowId(2));
        let tab = TabDrag {
            id: WorkspaceId(7),
            grab: egui::vec2(40.0, 10.0),
        };
        let origins = [egui::vec2(100.0, 50.0), egui::vec2(1000.0, 80.0)];
        // The pointer of the first window, 950 points right of its corner:
        // over the second window, 50 points into it.
        let pointer = egui::pos2(950.0, 60.0);
        let roam = Roam {
            from: first,
            what: Carried::Tab(tab),
            at: pointer + origins[0],
            over: Some(second),
        };
        assert_eq!(
            roam.cursor_in(first, Some(origins[0]), Some(pointer)),
            None,
            "not over the window it began in"
        );
        assert_eq!(
            roam.cursor_in(second, Some(origins[1]), None),
            Some(egui::pos2(50.0, 30.0)),
            "in the other window's coordinates"
        );
        assert_eq!(
            roam.cursor_in(second, None, None),
            None,
            "a window that does not know where it is takes nothing"
        );
        assert!(roam.visiting_tab(second).is_some_and(|t| t.id == tab.id));
        assert!(
            roam.visiting_tab(first).is_none(),
            "its own tab visits no one"
        );
        assert!(roam.visiting_pane(second).is_none());

        // Back over its own window, its own pointer is the cursor.
        let home = Roam {
            over: Some(first),
            ..roam
        };
        assert_eq!(
            home.cursor_in(first, Some(origins[0]), Some(pointer)),
            Some(pointer)
        );
        assert_eq!(home.cursor_in(second, Some(origins[1]), None), None);
    }

    /// A 900 by 600 window on the fake platform, where Rick draws the
    /// frame, after its first frame.
    fn framed(mode: frame::Mode) -> egui_kittest::Harness<'static, RickUi> {
        let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let host = Arc::new(EguiHost::new());
        let app = App::new(app::Config {
            platform: fake.clone(),
            ui: host.clone(),
            extensions: Vec::new(),
            config_dir: Some(platform::ConfigDir::config_dir(fake.as_ref())),
            start: None,
        });
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 600.0))
            .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx).with_frame(mode));
        harness.step();
        harness
    }

    /// Issue 158: behind a plain frame the window is cleared to the panel
    /// colour, opaque, so nothing of an opaque surface shows as black; the
    /// shadow frame's margin is cleared to transparent as before.
    #[test]
    fn the_window_is_cleared_to_the_panel_colour_behind_a_plain_frame() {
        let visuals = egui::Visuals::dark();
        let plain = framed(frame::Mode::Plain);
        assert!(plain.state().framed_content().is_some(), "a frame is drawn");
        let cleared = eframe::App::clear_color(plain.state(), &visuals);
        let panel = visuals.panel_fill.to_normalized_gamma_f32();
        assert!(
            cleared
                .iter()
                .zip(panel)
                .all(|(c, p)| (c - p).abs() < f32::EPSILON),
            "{cleared:?} is the panel colour {panel:?}"
        );
        assert!(
            (cleared[3] - 1.0).abs() < f32::EPSILON,
            "opaque: {cleared:?}"
        );

        let shadow = framed(frame::Mode::Shadow);
        assert!(
            shadow.state().framed_content().is_some(),
            "a frame is drawn"
        );
        let cleared = eframe::App::clear_color(shadow.state(), &visuals);
        assert!(
            cleared.iter().all(|c| c.abs() < f32::EPSILON),
            "transparent: {cleared:?}"
        );
    }

    /// Issue 158, every window: one Rick frames itself is undecorated and
    /// asks for transparency only when told to, which is for the shadow
    /// frame; one whose system keeps its controls never asks.
    #[test]
    fn a_window_asks_for_transparency_only_for_the_shadow_frame() {
        let chrome = |native_controls| platform::WindowChrome {
            controls: ControlsSide::Right,
            native_controls,
            native_controls_width: 0.0,
        };
        for (asked, mode) in [(true, frame::Mode::Shadow), (false, frame::Mode::Plain)] {
            let own = viewport_builder("Rick", chrome(false), None, mode.transparent());
            assert_eq!(own.decorations, Some(false));
            assert_eq!(own.transparent, Some(asked), "{mode:?}");
            let native = viewport_builder("Rick", chrome(true), None, mode.transparent());
            assert_eq!(native.transparent, None, "the system's frame");
            assert_eq!(native.decorations, None);
        }
    }

    /// The bundled icon is the 256 px tile, decoded once.
    #[test]
    fn the_window_icon_is_the_256_px_tile() {
        let icon = window_icon().unwrap_or_else(|| panic!("the window icon did not decode"));
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        assert!(Arc::ptr_eq(
            &icon,
            &window_icon().unwrap_or_else(|| panic!("the window icon did not decode"))
        ));
    }

    /// Every window carries the app id and the icon, whatever its chrome.
    #[test]
    fn every_viewport_carries_the_app_id_and_the_icon() {
        for native_controls in [false, true] {
            let chrome = platform::WindowChrome {
                controls: ControlsSide::Right,
                native_controls,
                native_controls_width: 0.0,
            };
            let viewport = viewport_builder("Rick", chrome, None, true);
            assert_eq!(viewport.app_id.as_deref(), Some(APP_ID));
            assert_eq!(APP_ID, "rick");
            let icon = viewport
                .icon
                .unwrap_or_else(|| panic!("the viewport has no icon"));
            assert_eq!((icon.width, icon.height), (256, 256));
        }
    }
}
