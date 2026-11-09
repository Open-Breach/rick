//! Drag and drop of entries: the drag state, the drop targets the widgets
//! register each frame, the move-or-copy rule, spring-loading, the ghost
//! with its indicator, and the move/copy/cancel prompt. The side panel is
//! never a file-operation target: a folder dropped on its Places list
//! becomes a place, a file is refused anywhere in it.
//!
//! The widgets never decide anything themselves. A row that is dragged asks
//! the shell to [`start`](Dnd::start) a drag; every target that is drawn
//! registers a [`Zone`] with [`Dnd::zone`] and learns whether to highlight;
//! at the end of the frame [`Dnd::end_frame`] resolves the target under the
//! cursor, the modifiers and the hover time into an indicator, a
//! spring-loading step or, on release, the `Command` the drop means. Drops
//! from outside the process arrive through egui's hovered and dropped files,
//! or through the platform where the backend receives none (Wayland), with
//! the cursor position the platform supplies where it can; without one the
//! drop lands in the focused pane's folder, as the spec's table says. A drag
//! that leaves the window is handed to the platform's drag source one frame
//! later, so the ghost is gone from the screen when the desktop takes over,
//! and egui's pointer is put right when that drag ends, since the backend
//! never sees its button come up.

use ext::t;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use app::{App, Command, TransferMode};
use egui::{Id, LayerId, Order, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use model::{AfterDrag, PaneId, Place, Tab, TabId};
use platform::WindowRef;
use types::{Entry, EntryId, EntryKind, Location};

use crate::shell::{Request, TabUi};
use crate::{icons, views};

/// What the indicator under the cursor says the drop will do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indicator {
    /// Move into the target folder.
    Move,
    /// Copy into the target folder.
    Copy,
    /// Become a place in the side panel's Places list.
    Place,
    /// Nothing: the side panel does not take this drop.
    Refused,
    /// Move to the trash: the drop lands in a pane showing the Trash view.
    Trash,
}

impl Indicator {
    /// The Phosphor glyph the spec names for the outcome.
    #[must_use]
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Move => icons::FORWARD,
            Self::Copy => icons::PLUS,
            Self::Place => icons::PIN,
            Self::Refused => icons::REFUSED,
            Self::Trash => icons::TRASH,
        }
    }
}

/// How a target spring-loads when the cursor rests on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spring {
    /// Navigate this pane into the target folder (a folder row or a
    /// breadcrumb segment).
    Pane(PaneId),
    /// Show this tab's workspace and focus its pane (a workspace tab of
    /// the title row, or a pane bar's tab).
    Tab(TabId),
}

/// What a drop on a zone does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Move or copy into a folder.
    Folder {
        /// The folder.
        to: Location,
        /// The folder's device, when a listing told it; decides move or
        /// copy inside one process.
        device: Option<u64>,
        /// What hovering for the spring delay does, if anything.
        spring: Option<Spring>,
    },
    /// Nothing, silently: a file row, or a folder the items are in already.
    NoOp,
    /// Every dragged folder becomes a place at this index of the Places
    /// list, or at the end; a drag with a file in it is refused.
    Place {
        /// The index, `None` for the end.
        at: Option<usize>,
    },
    /// Nothing, with the not-allowed cursor: the side panel outside its
    /// Places list, or files over the Places list.
    Refused,
    /// Move to trash through the action: anywhere in a pane showing the
    /// Trash view (issue 20).
    Trash,
}

/// A drop target drawn this frame.
#[derive(Debug, Clone)]
pub struct Zone {
    /// The same key every frame the same target is drawn, for hover timing.
    pub key: Id,
    /// Where it is.
    pub rect: Rect,
    /// What a drop does.
    pub target: Target,
}

/// Where the dragged items come from.
#[derive(Debug, Clone)]
pub enum Source {
    /// Rows of a pane of this window.
    Internal {
        /// The pane they were dragged out of.
        from: PaneId,
        /// Its tab at the time.
        tab: TabId,
        /// The entries, in view order.
        items: Vec<Entry>,
    },
    /// Files from another application or another Rick process.
    External {
        /// The paths, as far as the platform told them.
        paths: Vec<PathBuf>,
    },
}

impl Source {
    fn locations(&self) -> Vec<Location> {
        match self {
            Self::Internal { items, .. } => items
                .iter()
                .map(|e| Location::new(e.id.provider, e.path.clone()))
                .collect(),
            Self::External { paths } => paths
                .iter()
                .map(|p| Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(p)))
                .collect(),
        }
    }

    fn count(&self) -> usize {
        match self {
            Self::Internal { items, .. } => items.len(),
            Self::External { paths } => paths.len(),
        }
    }

    /// The first item's icon and name, for the ghost.
    fn first(&self) -> (&'static str, String) {
        match self {
            Self::Internal { items, .. } => {
                items.first().map_or((icons::FOLDER, String::new()), |e| {
                    (views::icon(e), e.name.clone())
                })
            }
            Self::External { paths } => paths.first().map_or((icons::FOLDER, String::new()), |p| {
                let name = p.file_name().map_or_else(
                    || p.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                (icons::by_extension(&name), name)
            }),
        }
    }

    /// The device of the dragged items, when known.
    fn device(&self) -> Option<u64> {
        match self {
            Self::Internal { items, .. } => items.first().and_then(|e| device_of(e.id)),
            Self::External { .. } => None,
        }
    }

    fn is_external(&self) -> bool {
        matches!(self, Self::External { .. })
    }

    /// Whether every dragged item is a folder; a drop on the Places list
    /// takes only those. An external path is examined on the disk.
    fn folders_only(&self) -> bool {
        match self {
            Self::Internal { items, .. } => items.iter().all(|e| e.kind == EntryKind::Dir),
            Self::External { paths } => paths.iter().all(|p| p.is_dir()),
        }
    }

    /// The places the dragged folders would become, titled by their names.
    fn places(&self) -> Vec<Place> {
        self.locations()
            .into_iter()
            .map(|location| {
                let title = location.path.file_name().map_or_else(
                    || location.path.to_string(),
                    |n| String::from_utf8_lossy(n).into_owned(),
                );
                Place::new(title, location)
            })
            .collect()
    }
}

/// The device half of a local entry id; `None` for other providers.
#[must_use]
pub fn device_of(id: EntryId) -> Option<u64> {
    (id.provider == vfs_local::PROVIDER_ID).then_some((id.key >> 64) as u64)
}

/// The device of a listing's folder, from its entries: a file first, since
/// a folder entry may be a mount point of another device.
#[must_use]
pub fn listing_device(tab: &Tab) -> Option<u64> {
    let entries = tab.listing.entries();
    entries
        .iter()
        .find(|e| e.kind == EntryKind::File)
        .or_else(|| entries.first())
        .and_then(|e| device_of(e.id))
}

/// The folder of the focused pane as a drop target, for an external drop
/// whose position the platform cannot tell: the spec's fallback.
fn focused_pane_target(app: &App) -> Option<(Id, Target)> {
    let model = app.model();
    let (_, tab) = model.active_tab(model.focused)?;
    let target = into_pane(
        tab,
        Target::Folder {
            to: tab.location.clone(),
            device: listing_device(tab),
            spring: None,
        },
    );
    Some((Id::new("dnd-focused-pane"), target))
}

/// Forgets egui's pointer: the platform took the button away with the
/// drag, so the release will never arrive and the row would stay dragged.
fn release_pointer(ctx: &egui::Context) {
    ctx.input_mut(|i| i.pointer = egui::PointerState::default());
    ctx.stop_dragging();
}

/// Where a pane was before a drag drilled into it, for *After a drag*.
#[derive(Debug, Clone)]
enum Origin {
    /// The pane spring-loaded out of `location`.
    Pane {
        pane: PaneId,
        tab: TabId,
        location: Location,
        scroll: f32,
    },
    /// The window showed `pane`'s workspace instead of `previous`'s.
    Tab { pane: PaneId, previous: TabId },
}

#[derive(Debug, Clone)]
struct Hover {
    key: Id,
    since: f64,
    sprung: bool,
}

/// A drag in progress.
#[derive(Debug, Clone)]
pub struct Drag {
    source: Source,
    /// Started with the right button: ask on drop.
    right_button: bool,
    hover: Option<Hover>,
    /// Panes that already got their first spring-loading step.
    sprung: BTreeSet<PaneId>,
    origins: Vec<Origin>,
    /// The cursor left the window: the platform was, or is about to be,
    /// asked to take the drag over, once.
    left_window: bool,
    /// The target under the cursor as of the last frame.
    target: Option<(Id, Target)>,
    indicator: Option<Indicator>,
}

/// What a test can see of a drag.
#[derive(Debug, Clone, PartialEq)]
pub struct DragView {
    /// How many items are dragged.
    pub count: usize,
    /// The indicator under the cursor, if any.
    pub indicator: Option<Indicator>,
    /// The target under the cursor, if any.
    pub target: Option<Target>,
    /// Whether the items come from outside the process.
    pub external: bool,
    /// Panes the drag spring-loaded.
    pub sprung: Vec<PaneId>,
}

/// A drop that waits for the move/copy/cancel answer.
#[derive(Debug, Clone)]
pub struct Prompt {
    source: Source,
    to: Location,
    pos: Pos2,
}

/// The drag-and-drop state of the window.
#[derive(Debug, Default)]
pub struct Dnd {
    drag: Option<Drag>,
    zones: Vec<Zone>,
    /// After Escape, egui still reports the row as dragged until the button
    /// comes up; ignore starts until then.
    suppressed: bool,
    prompt: Option<Prompt>,
    /// Files dropped from outside this frame.
    dropped: Option<Vec<PathBuf>>,
    /// Where the platform says the cursor of an external drag is this frame.
    external_pos: Option<Pos2>,
    /// The platform's own drop target sees a drag over the window this
    /// frame (Wayland), where egui's hovered files say nothing.
    platform_hovering: bool,
    /// The paths of a drag that left the window last frame, to hand to the
    /// platform this frame, once the ghost-free frame is on the screen.
    handover: Option<Vec<PathBuf>>,
    /// The platform runs a drag Rick started; polled until it ends.
    native_running: bool,
    /// The `after` setting and the pane origins to apply it to once the
    /// frame's requests are in.
    finished: Vec<(AfterDrag, Origin)>,
}

/// How far the ghost sits from the cursor.
const GHOST_OFFSET: Vec2 = vec2(18.0, -10.0);
/// Where the indicator sits, under the cursor.
const INDICATOR_OFFSET: Vec2 = vec2(10.0, 12.0);

impl Dnd {
    /// Whether a drag is in progress.
    #[must_use]
    pub fn active(&self) -> bool {
        self.drag.is_some()
    }

    /// The drag, for tests.
    #[must_use]
    pub fn view(&self) -> Option<DragView> {
        self.drag.as_ref().map(|d| DragView {
            count: d.source.count(),
            indicator: d.indicator,
            target: d.target.as_ref().map(|(_, t)| t.clone()),
            external: d.source.is_external(),
            sprung: d.sprung.iter().copied().collect(),
        })
    }

    /// Whether the move/copy/cancel prompt is up.
    #[must_use]
    pub fn prompt_open(&self) -> bool {
        self.prompt.is_some()
    }

    /// Whether every dragged item is a folder; `None` without a drag.
    #[must_use]
    pub fn folders_only(&self) -> Option<bool> {
        self.drag.as_ref().map(|d| d.source.folders_only())
    }

    /// Starts a drag of `source`; `right_button` asks on drop.
    pub fn start(&mut self, source: Source, right_button: bool) {
        if self.suppressed || source.count() == 0 {
            return;
        }
        if self.drag.is_none() {
            self.drag = Some(Drag {
                source,
                right_button,
                hover: None,
                sprung: BTreeSet::new(),
                origins: Vec::new(),
                left_window: false,
                target: None,
                indicator: None,
            });
        }
    }

    /// Registers a target drawn this frame. Returns whether it is the one
    /// under the cursor and should highlight.
    pub fn zone(&mut self, zone: Zone) -> bool {
        let Some(drag) = &self.drag else {
            return false;
        };
        let highlight = drag.target.as_ref().is_some_and(|(key, target)| {
            *key == zone.key && !matches!(target, Target::NoOp | Target::Refused)
        });
        self.zones.push(zone);
        highlight
    }

    /// Start of frame: files hovered or dropped from outside (egui's, and
    /// the platform's own where the backend has none), their cursor
    /// position from the platform, Escape, and the zone list of the frame.
    /// `window` is the backend's window when this is the root viewport;
    /// positions are only known for it.
    pub fn begin_frame(
        &mut self,
        ctx: &egui::Context,
        app: &App,
        window: Option<&WindowRef>,
        tabs: &mut HashMap<TabId, TabUi>,
        requests: &mut Vec<Request>,
    ) {
        self.zones.clear();
        let after = app.model().settings.drag.after;
        let (mut hovered, mut dropped) = ctx.input(|i| {
            let hovered: Vec<PathBuf> = i
                .raw
                .hovered_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect();
            let dropped: Vec<PathBuf> = i
                .raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect();
            (
                (!i.raw.hovered_files.is_empty()).then_some(hovered),
                (!i.raw.dropped_files.is_empty()).then_some(dropped),
            )
        });
        self.platform_hovering = false;
        if window.is_some() {
            let incoming = app.platform().incoming_files();
            self.platform_hovering = incoming.hovering;
            if incoming.hovering {
                // The files may still be on their way; ask again next frame.
                ctx.request_repaint();
            }
            if self.platform_hovering && hovered.is_none() {
                hovered = Some(incoming.hovered);
            }
            if !incoming.dropped.is_empty() {
                dropped
                    .get_or_insert_with(Vec::new)
                    .extend(incoming.dropped);
            }
        }
        if let Some(paths) = hovered {
            match &mut self.drag {
                None => self.start(Source::External { paths }, false),
                Some(Drag {
                    source: Source::External { paths: known },
                    ..
                }) if !paths.is_empty() => *known = paths,
                Some(_) => {}
            }
        }
        if let Some(paths) = dropped {
            if self.drag.is_none() {
                self.start(
                    Source::External {
                        paths: paths.clone(),
                    },
                    false,
                );
            }
            if let Some(Drag {
                source: Source::External { paths: known },
                ..
            }) = &mut self.drag
                && !paths.is_empty()
            {
                known.clone_from(&paths);
            }
            self.dropped = Some(paths);
        }
        self.external_pos = match (&self.drag, window) {
            (Some(drag), Some(window)) if drag.source.is_external() => {
                app.platform().drop_position(window).map(|p| pos2(p.x, p.y))
            }
            _ => None,
        };
        if self.suppressed && !ctx.input(|i| i.pointer.any_down()) {
            self.suppressed = false;
        }
        if self.drag.is_some()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.cancel(after, tabs, requests);
        }
    }

    /// Cancels the drag in progress: the ghost goes, the model is untouched,
    /// the panes that spring-loaded follow the `after` setting.
    pub fn cancel(
        &mut self,
        after: AfterDrag,
        tabs: &mut HashMap<TabId, TabUi>,
        requests: &mut Vec<Request>,
    ) {
        if self.drag.is_some() {
            self.suppressed = true;
        }
        self.end(after);
        self.finish(tabs, requests);
    }

    /// Applies *After a drag* to every pane a finished drag drilled into.
    fn finish(&mut self, tabs: &mut HashMap<TabId, TabUi>, requests: &mut Vec<Request>) {
        for (after, origin) in std::mem::take(&mut self.finished) {
            Self::apply_origin(after, origin, tabs, requests);
        }
    }

    fn apply_origin(
        after: AfterDrag,
        origin: Origin,
        tabs: &mut HashMap<TabId, TabUi>,
        requests: &mut Vec<Request>,
    ) {
        match origin {
            Origin::Pane {
                pane,
                tab,
                location,
                scroll,
            } => {
                let tab_ui = tabs.entry(tab).or_default();
                tab_ui.scroll_restore = Some((location, scroll));
                if after == AfterDrag::Return {
                    requests.push(Request::Command(Command::Back(pane)));
                }
            }
            Origin::Tab { previous, .. } => {
                if after == AfterDrag::Return {
                    requests.push(Request::Command(Command::ActivateTab(previous)));
                }
            }
        }
    }

    /// Ends the drag with the `after` setting queued for `finish`.
    fn end(&mut self, after: AfterDrag) {
        if let Some(drag) = self.drag.take() {
            for origin in drag.origins {
                self.finished.push((after, origin));
            }
        }
    }

    /// End of frame: resolves the target under the cursor, the modifiers
    /// and the hover time; acts on a release or an external drop; paints
    /// the ghost; hands a drag that left the window to the platform and
    /// follows that drag to its end.
    pub fn end_frame(
        &mut self,
        ctx: &egui::Context,
        app: &App,
        window: Option<&WindowRef>,
        tabs: &mut HashMap<TabId, TabUi>,
        requests: &mut Vec<Request>,
    ) {
        let dropped = self.dropped.take();
        let settings = &app.model().settings;
        let after = settings.drag.after;
        self.follow_native_drag(ctx, app);
        if let Some(paths) = self.handover.take()
            && let Some(window) = window
        {
            // The frame without the ghost is on the screen: the desktop
            // takes the drag from here.
            match app.platform().start_drag(window, &paths) {
                Ok(()) => {
                    tracing::debug!("drag handed to the platform");
                    self.end(after);
                    self.finish(tabs, requests);
                    self.native_running = true;
                    self.suppressed = true;
                    release_pointer(ctx);
                    if !app.platform().drag_in_progress() {
                        self.native_running = false;
                    }
                    return;
                }
                Err(error) => tracing::debug!(%error, "drag stays in the window"),
            }
        }
        let Some(drag) = &mut self.drag else {
            self.finish(tabs, requests);
            return;
        };
        let (pointer_pos, modifiers, released, now, any_down) = ctx.input(|i| {
            (
                i.pointer.latest_pos().or_else(|| i.pointer.hover_pos()),
                i.modifiers,
                i.pointer.any_released(),
                i.time,
                i.pointer.any_down(),
            )
        });
        // An external drag is where the platform says it is, not where the
        // backend last saw the pointer, which it stops reporting during one.
        let external = drag.source.is_external();
        let pos = if external {
            self.external_pos
        } else {
            pointer_pos
        };

        // Outside the window: hand the drag to the platform, from the next
        // frame on, so this frame is drawn without the ghost first.
        let inside = pos.is_some_and(|p| ctx.viewport_rect().contains(p));
        if !inside && !drag.left_window && !external && any_down && window.is_some() {
            drag.left_window = true;
            self.handover = Some(
                drag.source
                    .locations()
                    .iter()
                    .filter_map(|l| platform::paths::to_path(&l.path).ok())
                    .collect(),
            );
            ctx.request_repaint();
            return;
        }

        // The target: the smallest zone under the cursor, then the no-op
        // rule and the kind of source; an external drag whose position the
        // platform cannot tell lands in the focused pane's folder.
        let zone = pos.and_then(|p| smallest_zone(&self.zones, p));
        let resolved = zone
            .map(|z| (z.key, resolve(&drag.source, &z.target)))
            .or_else(|| {
                (external && pos.is_none())
                    .then(|| focused_pane_target(app))
                    .flatten()
                    .map(|(key, target)| (key, resolve(&drag.source, &target)))
            });
        drag.indicator = resolved
            .as_ref()
            .and_then(|(_, t)| indicator_for(&drag.source, t, modifiers, drag.right_button));
        if drag.indicator == Some(Indicator::Refused) {
            ctx.set_cursor_icon(egui::CursorIcon::NotAllowed);
        }
        drag.target.clone_from(&resolved);

        // Spring-loading: the same key for the delay.
        let delay = settings.drag.spring_delay_ms as f64 / 1000.0;
        if let Some(remaining) =
            spring_tick(drag, resolved.as_ref(), now, delay, app, tabs, requests)
        {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(remaining + 0.001));
        }

        // The drop.
        let dropped_outside = dropped.is_some();
        if (released && !external) || dropped_outside {
            let target = resolved.map(|(_, t)| t);
            let drag = drag.clone();
            self.drop(&drag, target, pos, modifiers, app, requests);
            self.end(after);
            self.finish(tabs, requests);
            return;
        }
        if external && ctx.input(|i| i.raw.hovered_files.is_empty()) && !self.platform_hovering {
            // The external drag left the window without a drop.
            self.end(after);
            self.finish(tabs, requests);
            return;
        }

        if let Some(pos) = pos {
            paint_ghost(ctx, drag, pos);
        }
        ctx.request_repaint();
    }

    /// Polls a drag the platform runs for Rick until it ends; the backend
    /// never sees its button come up, so egui's pointer is put right then.
    fn follow_native_drag(&mut self, ctx: &egui::Context, app: &App) {
        if !self.native_running {
            return;
        }
        if app.platform().drag_in_progress() {
            ctx.request_repaint();
        } else {
            tracing::debug!("platform drag ended");
            self.native_running = false;
            release_pointer(ctx);
        }
    }

    /// Whether the platform runs a drag Rick handed over, for tests.
    #[must_use]
    pub fn native_drag_running(&self) -> bool {
        self.native_running
    }

    /// Turns a release over `target` into the drop's command.
    fn drop(
        &mut self,
        drag: &Drag,
        target: Option<Target>,
        pos: Option<Pos2>,
        modifiers: egui::Modifiers,
        app: &App,
        requests: &mut Vec<Request>,
    ) {
        let Some(target) = target else {
            return;
        };
        match target {
            Target::NoOp | Target::Refused => {}
            Target::Folder { to, device, .. } => {
                let mode = mode_for(&drag.source, device, modifiers, drag.right_button);
                if mode == TransferMode::Ask {
                    self.prompt = Some(Prompt {
                        source: drag.source.clone(),
                        to,
                        pos: pos.unwrap_or_default(),
                    });
                    return;
                }
                requests.push(Request::Command(transfer_command(&drag.source, to, mode)));
            }
            Target::Place { at } => {
                // Each folder in turn, after the one before it.
                let mut at = at;
                for place in drag.source.places() {
                    requests.push(Request::Command(Command::PlaceAdd { place, at }));
                    at = at.map(|i| i + 1);
                }
            }
            Target::Trash => {
                if let Source::Internal { from, tab, items } = &drag.source
                    && let Some(tab) = app.model().tab(*tab)
                {
                    requests.push(Request::RunAction {
                        action: ext::ActionId("core.trash"),
                        ctx: ext::ActionCtx {
                            pane: *from,
                            location: tab.location.clone(),
                            selection: items.clone(),
                            invoked_from: ext::Invocation::Menu,
                            other: None,
                        },
                    });
                }
            }
        }
    }

    /// The move/copy/cancel prompt at the drop point, while one is pending.
    pub fn show_prompt(&mut self, ctx: &egui::Context, requests: &mut Vec<Request>) {
        let Some(prompt) = &self.prompt else {
            return;
        };
        let mut choice: Option<Option<TransferMode>> = None;
        egui::Area::new(Id::new("dnd-prompt"))
            .order(Order::Foreground)
            .fixed_pos(prompt.pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button(format!("{} Move", icons::FORWARD)).clicked() {
                            choice = Some(Some(TransferMode::Move));
                        }
                        if ui.button(format!("{} Copy", icons::PLUS)).clicked() {
                            choice = Some(Some(TransferMode::Copy));
                        }
                        if ui.button(t!("cancel")).clicked() {
                            choice = Some(None);
                        }
                    });
                });
            });
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            choice = Some(None);
        }
        let Some(choice) = choice else {
            return;
        };
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        if let Some(mode) = choice {
            requests.push(Request::Command(transfer_command(
                &prompt.source,
                prompt.to,
                mode,
            )));
        }
    }
}

/// What the indicator shows over `target`: for a folder the mode the rule
/// gives, with `Ask` showing what the rule would do without the modifier,
/// since the prompt decides.
fn indicator_for(
    source: &Source,
    target: &Target,
    modifiers: egui::Modifiers,
    right_button: bool,
) -> Option<Indicator> {
    match target {
        Target::Folder { device, .. } => {
            Some(match mode_for(source, *device, modifiers, right_button) {
                TransferMode::Copy => Indicator::Copy,
                TransferMode::Move => Indicator::Move,
                TransferMode::Ask => {
                    match mode_for(source, *device, egui::Modifiers::NONE, false) {
                        TransferMode::Copy => Indicator::Copy,
                        _ => Indicator::Move,
                    }
                }
            })
        }
        Target::Place { .. } => Some(Indicator::Place),
        Target::Refused => Some(Indicator::Refused),
        Target::Trash => Some(Indicator::Trash),
        Target::NoOp => None,
    }
}

/// Keeps the hover clock: the same target key across frames accumulates,
/// a new one starts over, and a spring-able folder held for `delay`
/// seconds gets its spring-loading step once. Returns how long the current
/// target still needs, so the frame can ask to be repainted then.
fn spring_tick(
    drag: &mut Drag,
    resolved: Option<&(Id, Target)>,
    now: f64,
    delay: f64,
    app: &App,
    tabs: &HashMap<TabId, TabUi>,
    requests: &mut Vec<Request>,
) -> Option<f64> {
    match (&mut drag.hover, resolved) {
        (Some(hover), Some((key, _))) if hover.key == *key => {}
        (hover, Some((key, _))) => {
            *hover = Some(Hover {
                key: *key,
                since: now,
                sprung: false,
            });
        }
        (hover, None) => *hover = None,
    }
    let Some((
        _,
        Target::Folder {
            to,
            spring: Some(spring),
            ..
        },
    )) = resolved
    else {
        return None;
    };
    let hover = drag.hover.as_mut()?;
    if hover.sprung {
        return None;
    }
    let elapsed = now - hover.since;
    if elapsed >= delay {
        hover.sprung = true;
        spring_load(drag, app, tabs, spring, to, requests);
        None
    } else {
        Some(delay - elapsed)
    }
}

/// The `Transfer` or `Receive` a drop of `source` into `to` means.
fn transfer_command(source: &Source, to: Location, mode: TransferMode) -> Command {
    match source {
        Source::Internal { from, items, .. } => Command::Transfer {
            items: items.iter().map(|e| e.id).collect(),
            from: *from,
            to,
            mode,
        },
        Source::External { .. } => Command::Receive {
            items: source.locations(),
            to,
            mode,
        },
    }
}

/// The spec's move-or-copy rule. Inside one process: Alt or the right
/// button asks, Mod copies, Shift moves, else the volume decides. From
/// outside: Shift moves, else copy.
fn mode_for(
    source: &Source,
    target_device: Option<u64>,
    modifiers: egui::Modifiers,
    right_button: bool,
) -> TransferMode {
    if source.is_external() {
        return if modifiers.shift {
            TransferMode::Move
        } else {
            TransferMode::Copy
        };
    }
    if modifiers.alt || right_button {
        return TransferMode::Ask;
    }
    if modifiers.command {
        return TransferMode::Copy;
    }
    if modifiers.shift {
        return TransferMode::Move;
    }
    match (source.device(), target_device) {
        (Some(a), Some(b)) if a != b => TransferMode::Copy,
        _ => TransferMode::Move,
    }
}

/// The smallest zone holding `pos`; the later one on a tie, since the
/// widgets drawn later are on top.
fn smallest_zone(zones: &[Zone], pos: Pos2) -> Option<&Zone> {
    zones
        .iter()
        .filter(|z| z.rect.contains(pos))
        .min_by(|a, b| {
            let (aa, ab) = (a.rect.area(), b.rect.area());
            aa.partial_cmp(&ab)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(std::cmp::Ordering::Greater)
        })
}

/// The target of a drop into a pane's space, tab header or row: the folder
/// given, except that a pane showing the Trash view trashes what lands
/// anywhere in it, as the side panel's Trash row does.
#[must_use]
pub fn into_pane(tab: &Tab, folder: Target) -> Target {
    if vfs_rick::view_name(&tab.location) == Some(vfs_rick::Virtual::Trash.name()) {
        Target::Trash
    } else {
        folder
    }
}

/// The zone's target for this source: a folder the items would not move
/// by going into, or a virtual location, is a no-op; the Places list takes
/// folders only and refuses a drag with a file in it.
fn resolve(source: &Source, target: &Target) -> Target {
    match target {
        Target::Folder { to, .. } => {
            let locations = source.locations();
            if to.provider == vfs_rick::PROVIDER_ID
                || locations.is_empty()
                || locations
                    .iter()
                    .all(|source| app::transfer_is_noop(source, to))
            {
                Target::NoOp
            } else {
                target.clone()
            }
        }
        Target::Place { .. } if !source.folders_only() => Target::Refused,
        // Only this process's own rows know their entries to trash.
        Target::Trash if source.is_external() => Target::NoOp,
        other => other.clone(),
    }
}

/// One spring-loading step: into the folder, or onto the tab.
fn spring_load(
    drag: &mut Drag,
    app: &App,
    tabs: &HashMap<TabId, TabUi>,
    spring: &Spring,
    to: &Location,
    requests: &mut Vec<Request>,
) {
    match spring {
        Spring::Pane(pane) => {
            let Some((tab_id, tab)) = app.model().active_tab(*pane) else {
                return;
            };
            if tab.location == *to {
                return;
            }
            let first = !drag.sprung.contains(pane);
            if first {
                drag.origins.push(Origin::Pane {
                    pane: *pane,
                    tab: tab_id,
                    location: tab.location.clone(),
                    scroll: tabs.get(&tab_id).map_or(0.0, |t| t.scroll_offset),
                });
                drag.sprung.insert(*pane);
            }
            requests.push(Request::Command(Command::SpringLoad {
                pane: *pane,
                to: to.clone(),
                first,
            }));
        }
        Spring::Tab(tab) => {
            // A workspace tab of the title row: resting on it shows the
            // workspace, with its pane focused; the tab that had the
            // window's focus is the one to come back to.
            let model = app.model();
            let Some(pane) = model.pane_of(*tab) else {
                return;
            };
            let Some(window) = model.window_of(pane) else {
                return;
            };
            let focused = model.window(window).map(model::Window::focused);
            if focused == Some(pane) {
                return;
            }
            if let Some(previous) = focused.and_then(|p| model.active_tab(p)).map(|(id, _)| id)
                && !drag
                    .origins
                    .iter()
                    .any(|o| matches!(o, Origin::Tab { pane: p, .. } if *p == pane))
            {
                drag.origins.push(Origin::Tab { pane, previous });
            }
            requests.push(Request::Command(Command::ActivateTab(*tab)));
        }
    }
}

/// The ghost: the first item's icon and name, a count badge for several
/// items, and the indicator under the cursor.
fn paint_ghost(ctx: &egui::Context, drag: &Drag, pos: Pos2) {
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("dnd-ghost")));
    let style = ctx.style_of(ctx.theme());
    let visuals = &style.visuals;
    let font = egui::TextStyle::Body.resolve(&style);
    let (icon, name) = drag.source.first();
    let text = format!("{icon} {name}");
    let galley = painter.layout_no_wrap(text, font.clone(), visuals.text_color());
    let pad = vec2(8.0, 4.0);
    let origin = pos + GHOST_OFFSET;
    let rect = Rect::from_min_size(origin, galley.size() + 2.0 * pad);
    painter.rect(
        rect,
        4.0,
        visuals.window_fill.gamma_multiply(0.95),
        Stroke::new(1.0, visuals.window_stroke.color),
        egui::StrokeKind::Outside,
    );
    painter.galley(origin + pad, galley, visuals.text_color());
    let count = drag.source.count();
    if count > 1 {
        let centre = pos2(rect.right(), rect.top());
        let label = count.to_string();
        let scheme = crate::theme::scheme_of(ctx);
        let badge = painter.layout_no_wrap(label, font, crate::theme::Scheme::ON_BUTTON);
        let radius = (badge.size().x / 2.0 + 4.0).max(9.0);
        painter.circle_filled(centre, radius, scheme.accent_button);
        painter.galley(
            centre - badge.size() / 2.0,
            badge,
            crate::theme::Scheme::ON_BUTTON,
        );
    }
    if let Some(indicator) = drag.indicator {
        let glyph = painter.layout_no_wrap(
            indicator.glyph().to_owned(),
            egui::FontId::proportional(13.0),
            visuals.strong_text_color(),
        );
        let at = pos + INDICATOR_OFFSET;
        let rect = Rect::from_min_size(at, glyph.size() + vec2(6.0, 4.0));
        painter.rect(
            rect,
            3.0,
            visuals.extreme_bg_color,
            Stroke::new(1.0, visuals.window_stroke.color),
            egui::StrokeKind::Outside,
        );
        painter.galley(at + vec2(3.0, 2.0), glyph, visuals.strong_text_color());
    }
}

/// Paints the drop highlight over a target's rectangle.
pub fn highlight(painter: &egui::Painter, visuals: &egui::Visuals, rect: Rect) {
    let color = visuals.selection.stroke.color;
    painter.rect_filled(rect, 3.0, color.gamma_multiply(0.18));
    painter.rect_stroke(
        rect.shrink(0.5),
        3.0,
        Stroke::new(1.5, color),
        egui::StrokeKind::Inside,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Attrs, ProviderId, VPath};

    fn entry(key: u128, path: &str, kind: EntryKind) -> Entry {
        Entry {
            id: EntryId::new(vfs_local::PROVIDER_ID, key),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            path: VPath::from(path),
            kind,
            size: None,
            created: None,
            modified: None,
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        }
    }

    fn internal(items: Vec<Entry>) -> Source {
        Source::Internal {
            from: PaneId(1),
            tab: TabId(1),
            items,
        }
    }

    #[test]
    fn the_mode_rule_follows_modifiers_then_the_volume() {
        let same = internal(vec![entry(1 << 64 | 7, "/a/x.txt", EntryKind::File)]);
        let m = |mods: egui::Modifiers, device: u64, right: bool| {
            mode_for(&same, Some(device), mods, right)
        };
        assert_eq!(m(egui::Modifiers::NONE, 1, false), TransferMode::Move);
        assert_eq!(m(egui::Modifiers::NONE, 2, false), TransferMode::Copy);
        assert_eq!(m(egui::Modifiers::COMMAND, 1, false), TransferMode::Copy);
        assert_eq!(m(egui::Modifiers::SHIFT, 2, false), TransferMode::Move);
        assert_eq!(m(egui::Modifiers::ALT, 1, false), TransferMode::Ask);
        assert_eq!(m(egui::Modifiers::NONE, 1, true), TransferMode::Ask);
        assert_eq!(
            mode_for(&same, None, egui::Modifiers::NONE, false),
            TransferMode::Move,
            "an unknown volume moves"
        );
        let outside = Source::External {
            paths: vec![PathBuf::from("/a/x.txt")],
        };
        assert_eq!(
            mode_for(&outside, Some(1), egui::Modifiers::NONE, false),
            TransferMode::Copy
        );
        assert_eq!(
            mode_for(
                &outside,
                Some(1),
                egui::Modifiers::COMMAND | egui::Modifiers::ALT,
                true
            ),
            TransferMode::Copy
        );
        assert_eq!(
            mode_for(&outside, Some(1), egui::Modifiers::SHIFT, false),
            TransferMode::Move
        );
    }

    #[test]
    fn no_op_targets_resolve_silently() {
        let dir = internal(vec![entry(1, "/a/sub", EntryKind::Dir)]);
        let folder = |to: &str| Target::Folder {
            to: Location::new(vfs_local::PROVIDER_ID, to),
            device: None,
            spring: None,
        };
        assert_eq!(resolve(&dir, &folder("/a")), Target::NoOp, "its own folder");
        assert_eq!(
            resolve(&dir, &folder("/a/sub")),
            Target::NoOp,
            "onto itself"
        );
        assert_eq!(
            resolve(&dir, &folder("/a/sub/x")),
            Target::NoOp,
            "own subtree"
        );
        assert_eq!(resolve(&dir, &folder("/b")), folder("/b"));
        assert_eq!(
            resolve(&dir, &Target::Place { at: Some(1) }),
            Target::Place { at: Some(1) },
            "a folder becomes a place"
        );
        let file = internal(vec![entry(2, "/a/x.txt", EntryKind::File)]);
        assert_eq!(
            resolve(&file, &Target::Place { at: None }),
            Target::Refused,
            "a file is refused"
        );
        let mixed = internal(vec![
            entry(1, "/a/sub", EntryKind::Dir),
            entry(2, "/a/x.txt", EntryKind::File),
        ]);
        assert_eq!(
            resolve(&mixed, &Target::Place { at: None }),
            Target::Refused
        );
        assert_eq!(resolve(&dir, &Target::Refused), Target::Refused);
        let virtual_view = Target::Folder {
            to: vfs_rick::Virtual::Favorites.location(),
            device: None,
            spring: None,
        };
        assert_eq!(
            resolve(&dir, &virtual_view),
            Target::NoOp,
            "no drop into a view"
        );
        let outside = Source::External {
            paths: vec![PathBuf::from("/a/x.txt")],
        };
        assert_eq!(
            resolve(&outside, &Target::Place { at: None }),
            Target::Refused,
            "a path that is no folder on the disk"
        );
        assert_eq!(resolve(&outside, &folder("/a")), Target::NoOp);
        assert_eq!(resolve(&outside, &folder("/b")), folder("/b"));
        assert_eq!(resolve(&dir, &Target::Trash), Target::Trash);
        assert_eq!(resolve(&outside, &Target::Trash), Target::NoOp);
        assert_eq!(
            dir.places()[0].title,
            "sub",
            "the place is titled by the folder's name"
        );
        let other = Location::new(ProviderId("other"), "/a");
        assert_eq!(
            resolve(
                &dir,
                &Target::Folder {
                    to: other.clone(),
                    device: None,
                    spring: None
                }
            ),
            Target::Folder {
                to: other,
                device: None,
                spring: None
            }
        );
    }

    #[test]
    fn the_smallest_zone_wins() {
        let zone = |key: &str, rect: Rect| Zone {
            key: Id::new(key),
            rect,
            target: Target::NoOp,
        };
        let zones = vec![
            zone(
                "pane",
                Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0)),
            ),
            zone(
                "row",
                Rect::from_min_size(pos2(0.0, 10.0), vec2(100.0, 20.0)),
            ),
            zone(
                "row2",
                Rect::from_min_size(pos2(0.0, 30.0), vec2(100.0, 20.0)),
            ),
        ];
        assert_eq!(
            smallest_zone(&zones, pos2(5.0, 15.0)).map(|z| z.key),
            Some(Id::new("row"))
        );
        assert_eq!(
            smallest_zone(&zones, pos2(5.0, 80.0)).map(|z| z.key),
            Some(Id::new("pane"))
        );
        assert!(smallest_zone(&zones, pos2(500.0, 80.0)).is_none());
    }
}
