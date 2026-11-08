//! The side panel: the Places list, the This Machine, Network and Cloud
//! sections, and nothing else; the operations area sits under it in the
//! shell. The panel is navigation only: it takes the user places and never
//! performs a file operation, highlights no current location and draws no
//! indent guide.
//!
//! Places is a flat, headerless, user-owned list: a click opens the row in
//! the focused pane, a middle click, a Mod+click or the context menu's
//! *Open in new tab* in a new tab; rows reorder by drag, the context menu
//! offers *Remove* and *Edit…* (title and a Phosphor icon), and a folder
//! dragged from a file list becomes a place at the insertion line. Files
//! are refused anywhere in the panel. The three titled sections fold with a
//! chevron; their header opens the section's overview (`rick://machine`,
//! `rick://network`, `rick://cloud`) and their rows a root. The Network and
//! Cloud sections list one row per connection of the network and cloud
//! provider plugins; the header's context menu and the *Add…* row of an
//! empty section offer *Add connection…* per plugin, which opens the
//! dialogue the plugin's `Connector` describes, and a connection row's
//! context menu offers *Connect*, *Disconnect* and *Remove connection*.
//! With the panel focused, Up and Down move, Enter opens, Mod+Enter opens
//! in a new tab.

use std::collections::BTreeSet;

use std::sync::Arc;

use app::{App, Command, ConnectionOp};
use ext::{ConnectionField, ConnectionFieldKind, Connector, Level};
use model::{PaneId, Place, Section, SidePanel, SidePanelChange, Volume, VolumeKind, WindowId};
use types::{Entry, Location, ProviderId};
use vfs_rick::Virtual;

use crate::dnd::{Dnd, Target, Zone};
use crate::icons;
use crate::overview::{percent, utilisation_bar, volume_icon};
use crate::shell::Request;

/// The height of a utilisation bar under a drive row.
const BAR_HEIGHT: f32 = 4.0;

/// One row the keyboard can land on.
#[derive(Debug, Clone)]
struct Row {
    id: egui::Id,
    location: Option<Location>,
}

/// The Edit… dialogue of a place.
#[derive(Debug, Clone)]
pub struct PlaceEdit {
    /// The row being edited.
    pub index: usize,
    /// The title as typed.
    pub title: String,
    /// The Phosphor icon name picked.
    pub icon: String,
    focus: bool,
}

/// The *Add connection…* dialogue of a network or cloud provider.
#[derive(Debug, Clone)]
pub struct ConnectionDialog {
    /// The provider the connection is for.
    pub provider: ProviderId,
    /// The provider's name in the title.
    pub title: &'static str,
    /// The connection's name as typed.
    pub name: String,
    /// The fields with their values as typed; a `Bool` holds `true` or
    /// `false`.
    pub values: Vec<(ConnectionField, String)>,
    /// What stopped the last OK.
    pub error: Option<String>,
    focus: bool,
}

impl ConnectionDialog {
    /// The dialogue for `connector`, every field at its default.
    #[must_use]
    pub fn new(connector: &dyn Connector) -> Self {
        let schema = connector.schema();
        Self {
            provider: connector.provider(),
            title: schema.title,
            name: String::new(),
            values: schema
                .fields
                .iter()
                .map(|field| {
                    let value = match &field.kind {
                        ConnectionFieldKind::Text { default } => default.clone(),
                        ConnectionFieldKind::Integer { default, .. } => default.to_string(),
                        ConnectionFieldKind::Bool { default } => default.to_string(),
                        ConnectionFieldKind::Path | ConnectionFieldKind::Secret => String::new(),
                    };
                    (field.clone(), value)
                })
                .collect(),
            error: None,
            focus: true,
        }
    }

    /// The command OK sends.
    ///
    /// # Errors
    ///
    /// What stops it, to show in the dialogue: an empty name or one that is
    /// taken, a required field without a value, a number out of its bounds.
    pub fn command(&self, taken: &[String]) -> Result<Command, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("The connection needs a name".to_owned());
        }
        if name.contains(['/', '\\']) {
            return Err("The name cannot contain a slash".to_owned());
        }
        if taken.iter().any(|t| t == name) {
            return Err(format!("A connection called {name} exists already"));
        }
        let mut values = Vec::with_capacity(self.values.len());
        for (field, value) in &self.values {
            let value = value.trim();
            if value.is_empty() {
                if field.required {
                    return Err(format!("{} needs a value", field.label));
                }
                continue;
            }
            if let ConnectionFieldKind::Integer { min, max, .. } = field.kind {
                match value.parse::<i64>() {
                    Ok(n) if (min..=max).contains(&n) => {}
                    _ => {
                        return Err(format!(
                            "{} must be a number from {min} to {max}",
                            field.label
                        ));
                    }
                }
            }
            values.push((field.key.to_owned(), value.to_owned()));
        }
        Ok(Command::AddConnection {
            provider: self.provider,
            name: name.to_owned(),
            values,
        })
    }
}

/// The panel's own state: the keyboard focus, a reorder in progress, the
/// Edit… dialogue, the *Add connection…* dialogue.
#[derive(Debug, Default)]
pub struct SidePanelState {
    /// The row the keyboard is on.
    focus: Option<egui::Id>,
    /// The panel took the last click, so Up, Down and Enter act on it.
    focused: bool,
    /// The rows drawn last frame, top to bottom.
    rows: Vec<Row>,
    /// A Places row being dragged to reorder: its index.
    reorder: Option<usize>,
    /// The Edit… dialogue, when open.
    pub edit: Option<PlaceEdit>,
    /// The *Add connection…* dialogue, when open.
    pub connection: Option<ConnectionDialog>,
}

impl SidePanelState {
    /// Whether the panel has the keyboard.
    #[must_use]
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// The row the keyboard is on, for tests: its location.
    #[must_use]
    pub fn focused_location(&self) -> Option<Location> {
        let id = self.focus?;
        self.rows
            .iter()
            .find(|r| r.id == id)
            .and_then(|r| r.location.clone())
    }

    /// Up and Down move, Enter opens, Mod+Enter opens in a new tab, Escape
    /// gives the keyboard back to the list. Returns whether a key was taken.
    pub fn handle_keys(
        &mut self,
        ctx: &egui::Context,
        app: &App,
        requests: &mut Vec<Request>,
    ) -> bool {
        if !self.focused || self.rows.is_empty() {
            return false;
        }
        let (up, down, enter, mod_enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            )
        });
        if escape {
            self.focused = false;
            return true;
        }
        let at = self
            .focus
            .and_then(|id| self.rows.iter().position(|r| r.id == id));
        if up || down {
            let next = match (at, up) {
                (None, _) => 0,
                (Some(i), true) => i.saturating_sub(1),
                (Some(i), false) => (i + 1).min(self.rows.len() - 1),
            };
            self.focus = Some(self.rows[next].id);
            return true;
        }
        if enter || mod_enter {
            if let Some(location) = at.and_then(|i| self.rows[i].location.clone()) {
                open(requests, app.model().focused, location, mod_enter);
            }
            return true;
        }
        false
    }

    /// Records a row for the keyboard, in display order.
    fn row(&mut self, id: egui::Id, location: Option<Location>) {
        self.rows.push(Row { id, location });
    }

    /// Takes the keyboard for the row that was clicked.
    fn take_focus(&mut self, id: egui::Id) {
        self.focus = Some(id);
        self.focused = true;
    }
}

/// Opens a location in the focused pane, or in a new tab.
fn open(requests: &mut Vec<Request>, pane: PaneId, location: Location, new_tab: bool) {
    if new_tab {
        requests.push(Request::NewTabAt(location));
    } else {
        requests.push(Request::Command(Command::Navigate { pane, to: location }));
    }
}

/// The focused pane of `window`.
fn focused_pane(app: &App, window: WindowId) -> PaneId {
    app.model()
        .window(window)
        .map_or(app.model().focused, |w| w.focused)
}

/// Draws the panel for `window`: navigation goes out as requests, drop
/// zones into `dnd`.
pub fn show(
    ui: &mut egui::Ui,
    app: &App,
    window: WindowId,
    panel: &SidePanel,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
    dnd: &mut Dnd,
) {
    let pane = focused_pane(app, window);
    let rect = ui.max_rect();
    // A click anywhere else takes the keyboard away from the panel.
    if state.focused
        && ui.input(|i| i.pointer.any_pressed())
        && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        && !rect.contains(pos)
    {
        state.focused = false;
    }
    state.rows.clear();
    // The panel is never a file-operation target: outside the Places list
    // a drop is refused, and the Places list accepts folders only.
    if dnd.active() {
        dnd.zone(Zone {
            key: egui::Id::new(("dnd-side-panel", window.0)),
            rect,
            target: Target::Refused,
        });
    }
    ui.visuals_mut().indent_has_left_vline = false;
    egui::ScrollArea::vertical()
        .id_salt(("side-panel", window.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            places(ui, app, pane, state, requests, dnd);
            ui.add_space(6.0);
            for section in Section::ALL {
                if section == Section::Cloud && app.model().cloud_providers.is_empty() {
                    continue;
                }
                titled_section(ui, app, window, pane, panel, section, state, requests);
            }
        });
    edit_dialog(ui.ctx(), app, state, requests);
    connection_dialog(ui.ctx(), app, state, requests);
}

/// The connectors whose provider belongs to `section`: the cloud plugins
/// under Cloud, the network plugins under Network, none under This
/// Machine.
fn connectors_of(app: &App, section: Section) -> Vec<Arc<dyn Connector>> {
    app.registry()
        .connectors()
        .iter()
        .filter(|c| match section {
            Section::Machine => false,
            Section::Network => vfs_rick::is_network_provider(c.provider()),
            Section::Cloud => vfs_rick::is_cloud_provider(c.provider()),
        })
        .cloned()
        .collect()
}

/// Opens the *Add connection…* dialogue of `provider`; the shell calls
/// this for `Request::AddConnection`.
pub fn begin_connection(state: &mut SidePanelState, app: &App, provider: ProviderId) {
    if let Some(connector) = app.registry().connector_of(provider) {
        state.connection = Some(ConnectionDialog::new(&**connector));
    }
}

/// The names of `provider`'s connections, which a new one cannot take.
fn connection_names(app: &App, provider: ProviderId) -> Vec<String> {
    app.model()
        .settings
        .connections
        .iter()
        .filter(|c| c.provider == provider.0)
        .map(|c| c.name.clone())
        .collect()
}

/// The *Add connection…* dialogue: the name, the connector's fields, OK
/// and Cancel.
fn connection_dialog(
    ctx: &egui::Context,
    app: &App,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
) {
    let Some(dialog) = state.connection.as_mut() else {
        return;
    };
    let taken = connection_names(app, dialog.provider);
    let mut open = true;
    let mut done: Option<bool> = None;
    egui::Window::new(format!("Add {} connection", dialog.title))
        .id(egui::Id::new("add-connection"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            egui::Grid::new("add-connection-fields")
                .num_columns(2)
                .spacing(egui::vec2(12.0, 6.0))
                .show(ui, |ui| {
                    ui.label("Name");
                    let name =
                        ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(260.0));
                    if dialog.focus {
                        name.request_focus();
                        dialog.focus = false;
                    }
                    ui.end_row();
                    for (field, value) in &mut dialog.values {
                        let label = if field.required {
                            format!("{} *", field.label)
                        } else {
                            field.label.to_owned()
                        };
                        ui.label(label);
                        match &field.kind {
                            ConnectionFieldKind::Secret => {
                                ui.add(
                                    egui::TextEdit::singleline(value)
                                        .password(true)
                                        .desired_width(260.0),
                                );
                            }
                            ConnectionFieldKind::Bool { .. } => {
                                let mut on = value == "true";
                                if ui.checkbox(&mut on, "").changed() {
                                    *value = on.to_string();
                                }
                            }
                            ConnectionFieldKind::Text { .. }
                            | ConnectionFieldKind::Integer { .. }
                            | ConnectionFieldKind::Path => {
                                ui.add(egui::TextEdit::singleline(value).desired_width(260.0));
                            }
                        }
                        ui.end_row();
                    }
                });
            if let Some(error) = &dialog.error {
                ui.add_space(4.0);
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("OK").clicked() || enter {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    done = Some(false);
                }
            });
        });
    if !open {
        done = Some(false);
    }
    match done {
        Some(true) => match dialog.command(&taken) {
            Ok(command) => {
                requests.push(Request::Command(command));
                state.connection = None;
            }
            Err(error) => dialog.error = Some(error),
        },
        Some(false) => state.connection = None,
        None => {}
    }
}

/// The *Add connection…* entries of a section's menu, one per provider
/// plugin of the section.
fn add_connection_items(
    ui: &mut egui::Ui,
    requests: &mut Vec<Request>,
    connectors: &[Arc<dyn Connector>],
) {
    for connector in connectors {
        if ui
            .button(format!(
                "{} Add {} connection…",
                icons::ADD,
                connector.schema().title
            ))
            .clicked()
        {
            requests.push(Request::AddConnection(connector.provider()));
            ui.close();
        }
    }
}

/// The context-menu entries of a connection row: *Connect* or
/// *Disconnect*, and *Remove connection*.
fn connection_items(
    ui: &mut egui::Ui,
    requests: &mut Vec<Request>,
    connector: &dyn Connector,
    name: &str,
) {
    let provider = connector.provider();
    if connector.is_connected(name) {
        if ui
            .button(format!("{} Disconnect", icons::UNLINKED))
            .clicked()
        {
            requests.push(Request::Command(Command::Connection {
                provider,
                name: name.to_owned(),
                op: ConnectionOp::Disconnect,
            }));
            ui.close();
        }
    } else if ui.button(format!("{} Connect", icons::LINKED)).clicked() {
        requests.push(Request::Command(Command::Connection {
            provider,
            name: name.to_owned(),
            op: ConnectionOp::Connect,
        }));
        ui.close();
    }
    if ui
        .button(format!("{} Remove connection", icons::TRASH))
        .clicked()
    {
        requests.push(Request::Command(Command::Connection {
            provider,
            name: name.to_owned(),
            op: ConnectionOp::Remove,
        }));
        ui.close();
    }
}

/// The connection a row of a provider's root stands for: its name, when
/// the row is a direct child of the root and the provider has a connector.
fn connection_of<'a>(app: &'a App, entry: &Entry) -> Option<(&'a Arc<dyn Connector>, String)> {
    let connector = app.registry().connector_of(entry.id.provider)?;
    let bytes = entry.path.as_bytes();
    let trimmed = bytes
        .strip_prefix(b"/")
        .or_else(|| bytes.strip_prefix(b"\\"))
        .unwrap_or(bytes);
    if trimmed.is_empty() || trimmed.iter().any(|b| matches!(b, b'/' | b'\\')) {
        return None;
    }
    Some((connector, String::from_utf8_lossy(trimmed).into_owned()))
}

/// A row for a connection or an account a provider lists at its root.
#[allow(clippy::too_many_arguments)]
fn provider_row(
    ui: &mut egui::Ui,
    app: &App,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
    pane: PaneId,
    section: Section,
    index: usize,
    entry: &Entry,
) {
    let location = Location::new(entry.id.provider, entry.path.clone());
    let glyph = match section {
        Section::Cloud => icons::CLOUD,
        _ => icons::NETWORK,
    };
    let button = egui::Button::new(format!("{glyph} {}", entry.name))
        .frame(false)
        .truncate();
    let connection = connection_of(app, entry);
    let mut extra = |ui: &mut egui::Ui, requests: &mut Vec<Request>| {
        if let Some((connector, name)) = &connection {
            connection_items(ui, requests, &***connector, name);
        }
    };
    let response = location_row(
        ui,
        state,
        requests,
        pane,
        egui::Id::new(("side-provider-row", section.name(), index)),
        button,
        &location,
        &mut extra,
    );
    if let Some((connector, name)) = &connection {
        let state_text = if connector.is_connected(name) {
            "connected"
        } else {
            "not connected"
        };
        response.on_hover_text(format!("{} · {state_text}", entry.id.provider));
    }
}

/// Paints the keyboard focus around a row when the panel has it.
fn focus_ring(ui: &egui::Ui, state: &SidePanelState, id: egui::Id, rect: egui::Rect) {
    if state.focused && state.focus == Some(id) {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            3.0,
            egui::Stroke::new(1.0, ui.visuals().selection.stroke.color),
            egui::StrokeKind::Inside,
        );
    }
}

/// A row that opens a location: click, middle click, Mod+click and the
/// context menu with *Open in new tab* first and `extra` after it. Returns
/// the response for the caller's own handling (a drag, a drop zone).
#[allow(clippy::too_many_arguments)]
fn location_row(
    ui: &mut egui::Ui,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
    pane: PaneId,
    id: egui::Id,
    button: egui::Button<'_>,
    location: &Location,
    extra: &mut dyn FnMut(&mut egui::Ui, &mut Vec<Request>),
) -> egui::Response {
    let response = ui.add(button.min_size(egui::vec2(ui.available_width(), 0.0)));
    state.row(id, Some(location.clone()));
    focus_ring(ui, state, id, response.rect);
    if response.clicked() {
        state.take_focus(id);
        let new_tab = ui.input(|i| i.modifiers.command);
        open(requests, pane, location.clone(), new_tab);
    }
    if response.middle_clicked() {
        state.take_focus(id);
        open(requests, pane, location.clone(), true);
    }
    if response.secondary_clicked() {
        state.take_focus(id);
    }
    response.context_menu(|ui| {
        if ui.button("Open in new tab").clicked() {
            open(requests, pane, location.clone(), true);
            ui.close();
        }
        extra(ui, requests);
    });
    response
}

/// Which gap of the Places list a pointer at `y` points into, given the
/// rows' rectangles top to bottom.
fn gap_at(rects: &[egui::Rect], y: f32) -> usize {
    for (index, rect) in rects.iter().enumerate() {
        if y < rect.center().y {
            return index;
        }
    }
    rects.len()
}

/// The index a row at `from` ends up at when dropped into gap `gap`.
fn target_of(from: usize, gap: usize) -> usize {
    if gap > from { gap - 1 } else { gap }
}

/// Paints the insertion line at `y` across the list.
fn insertion_line(ui: &egui::Ui, x_range: egui::Rangef, y: f32) {
    ui.painter().hline(
        x_range,
        y,
        egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
    );
}

/// The Places list.
fn places(
    ui: &mut egui::Ui,
    app: &App,
    pane: PaneId,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
    dnd: &mut Dnd,
) {
    let rows: Vec<Place> = app.model().places.rows().to_vec();
    let trash_count = app.model().trash.items.len();
    let mut rects = Vec::with_capacity(rows.len());
    let mut lines: Vec<f32> = Vec::new();
    let list_top = ui.cursor().top();
    for (index, place) in rows.iter().enumerate() {
        let id = egui::Id::new(("side-place", index, &place.location));
        let glyph = icons::named(&place.icon);
        let mut button = egui::Button::new(format!("{glyph} {}", place.title))
            .frame(false)
            .truncate()
            .sense(egui::Sense::click_and_drag());
        if trash_count > 0 && Virtual::of(&place.location) == Some(Virtual::Trash) {
            button = button.right_text(egui::RichText::new(trash_count.to_string()).weak());
        }
        let location = place.location.clone();
        let mut extra = |ui: &mut egui::Ui, requests: &mut Vec<Request>| {
            if ui.button(format!("{} Edit…", icons::EDIT)).clicked() {
                state_edit_start(requests, index, place);
                ui.close();
            }
            if ui.button("Remove").clicked() {
                requests.push(Request::Command(Command::PlaceRemove(index)));
                ui.close();
            }
        };
        let response = location_row(ui, state, requests, pane, id, button, &location, &mut extra);
        rects.push(response.rect);
        // Dragging a row reorders the list; a drag of entries is the
        // shell's drag and never starts here.
        if response.drag_started_by(egui::PointerButton::Primary) && !dnd.active() {
            state.reorder = Some(index);
        }
        // A folder dropped on the upper half goes before the row, on the
        // lower half after it.
        if dnd.active() {
            let (upper, lower) = response.rect.split_top_bottom_at_fraction(0.5);
            if dnd.zone(Zone {
                key: egui::Id::new(("dnd-place-before", index)),
                rect: upper,
                target: Target::Place { at: Some(index) },
            }) {
                lines.push(response.rect.top());
            }
            if dnd.zone(Zone {
                key: egui::Id::new(("dnd-place-after", index)),
                rect: lower,
                target: Target::Place {
                    at: Some(index + 1),
                },
            }) {
                lines.push(response.rect.bottom());
            }
        }
    }
    // The empty strip under the list: a drop there appends.
    let (strip, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 10.0), egui::Sense::hover());
    if dnd.active()
        && dnd.zone(Zone {
            key: egui::Id::new("dnd-place-end"),
            rect: strip,
            target: Target::Place { at: None },
        })
    {
        lines.push(rects.last().map_or(list_top, egui::Rect::bottom));
    }
    let x_range = egui::Rangef::new(ui.max_rect().left(), ui.max_rect().right());
    for y in lines {
        insertion_line(ui, x_range, y);
    }
    // The reorder in progress: the line where the row would land, and the
    // move on release.
    if let Some(from) = state.reorder {
        let pointer = ui.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.any_released(),
                i.pointer.any_down(),
            )
        });
        let gap = pointer.0.map(|p| gap_at(&rects, p.y));
        if let Some(gap) = gap {
            let y = if gap == rects.len() {
                rects.last().map_or(list_top, egui::Rect::bottom)
            } else {
                rects[gap].top()
            };
            insertion_line(ui, x_range, y);
        }
        if pointer.1 || !pointer.2 {
            state.reorder = None;
            if let Some(gap) = gap {
                let to = target_of(from, gap);
                if to != from {
                    requests.push(Request::Command(Command::PlaceMove { from, to }));
                }
            }
        }
        ui.ctx().request_repaint();
    }
    if rows.is_empty() {
        ui.weak("Drop a folder here to add a place.");
    }
}

/// Opens the Edit… dialogue for a place through the request queue, so the
/// borrow of the list ends first.
fn state_edit_start(requests: &mut Vec<Request>, index: usize, place: &Place) {
    requests.push(Request::EditPlace {
        index,
        title: place.title.clone(),
        icon: place.icon.clone(),
    });
}

/// Begins editing a place; the shell calls this for `Request::EditPlace`.
pub fn begin_edit(state: &mut SidePanelState, index: usize, title: String, icon: String) {
    state.edit = Some(PlaceEdit {
        index,
        title,
        icon,
        focus: true,
    });
}

/// The Edit… dialogue: the title, the Phosphor icon picked from a grid,
/// OK and Cancel.
fn edit_dialog(
    ctx: &egui::Context,
    app: &App,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
) {
    let Some(edit) = state.edit.as_mut() else {
        return;
    };
    if app.model().places.rows().get(edit.index).is_none() {
        state.edit = None;
        return;
    }
    let mut open = true;
    let mut done: Option<bool> = None;
    egui::Window::new("Edit place")
        .id(egui::Id::new("edit-place"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label("Title");
            let title = ui.text_edit_singleline(&mut edit.title);
            if edit.focus {
                title.request_focus();
                edit.focus = false;
            }
            ui.add_space(6.0);
            ui.label("Icon");
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (name, glyph) in icons::PICKABLE {
                            let selected = edit.icon == *name;
                            if ui
                                .add(egui::Button::selectable(selected, *glyph))
                                .on_hover_text(*name)
                                .clicked()
                            {
                                (*name).clone_into(&mut edit.icon);
                            }
                        }
                    });
                });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let enter = title.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("OK").clicked() || enter {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    done = Some(false);
                }
            });
        });
    if !open {
        done = Some(false);
    }
    if let Some(ok) = done {
        if ok {
            requests.push(Request::Command(Command::PlaceEdit {
                index: edit.index,
                title: edit.title.clone(),
                icon: edit.icon.clone(),
            }));
        }
        state.edit = None;
    }
}

/// A titled section: the header with its chevron and its rows, indented
/// without a guide line.
// One section, header then rows, top to bottom.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn titled_section(
    ui: &mut egui::Ui,
    app: &App,
    window: WindowId,
    pane: PaneId,
    panel: &SidePanel,
    section: Section,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
) {
    let folded = panel.folded.contains(&section);
    let overview = match section {
        Section::Machine => Virtual::Machine,
        Section::Network => Virtual::Network,
        Section::Cloud => Virtual::Cloud,
    }
    .location();
    ui.horizontal(|ui| {
        let chevron = ui.add(
            egui::Button::new(if folded {
                icons::TREE_CLOSED
            } else {
                icons::TREE_OPEN
            })
            .frame(false)
            .small(),
        );
        if chevron.clicked() {
            requests.push(Request::Command(Command::SidePanel {
                window,
                change: SidePanelChange::Fold {
                    section,
                    folded: !folded,
                },
            }));
        }
        let id = egui::Id::new(("side-section", section.name()));
        let button = egui::Button::new(egui::RichText::new(section.title()).strong())
            .frame(false)
            .truncate();
        let connectors = connectors_of(app, section);
        location_row(
            ui,
            state,
            requests,
            pane,
            id,
            button,
            &overview,
            &mut |ui, requests| add_connection_items(ui, requests, &connectors),
        )
        .on_hover_text(format!("Open the {} overview", section.title()));
    });
    if folded {
        return;
    }
    ui.indent(("side-section-body", section.name()), |ui| {
        let mut any = false;
        let volumes: Vec<Volume> = app
            .model()
            .volumes
            .iter()
            .filter(|v| (v.kind == VolumeKind::Network) == (section == Section::Network))
            .cloned()
            .collect();
        if section != Section::Cloud {
            for volume in &volumes {
                any = true;
                volume_row(ui, state, requests, pane, volume);
            }
        }
        let rows: &[Entry] = match section {
            Section::Machine => &[],
            Section::Network => &app.model().network,
            Section::Cloud => &app.model().cloud,
        };
        for (index, entry) in rows.iter().enumerate() {
            any = true;
            provider_row(ui, app, state, requests, pane, section, index, entry);
        }
        if !any {
            add_row(ui, app, state, requests, section);
        }
    });
}

/// A drive row: icon, name and the utilisation percentage, with a small bar
/// under it; Eject in the context menu of removable media.
fn volume_row(
    ui: &mut egui::Ui,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
    pane: PaneId,
    volume: &Volume,
) {
    let location = Location::new(
        vfs_local::PROVIDER_ID,
        platform::paths::from_path(&volume.mount_point),
    );
    let id = egui::Id::new(("side-volume", &volume.mount_point));
    let mut button = egui::Button::new(format!("{} {}", volume_icon(volume.kind), volume.name))
        .frame(false)
        .truncate();
    if volume.total.is_some() {
        button = button.right_text(egui::RichText::new(percent(volume)).weak());
    }
    let mount_point = volume.mount_point.clone();
    let removable = volume.is_removable();
    let mut extra = |ui: &mut egui::Ui, requests: &mut Vec<Request>| {
        if removable && ui.button(format!("{} Eject", icons::EJECT)).clicked() {
            requests.push(Request::Command(Command::Eject(mount_point.clone())));
            ui.close();
        }
    };
    let response = location_row(ui, state, requests, pane, id, button, &location, &mut extra)
        .on_hover_text(format!(
            "{}\n{} · {} free of {}",
            volume.mount_point.display(),
            volume.fs_type,
            crate::format::size(volume.free),
            crate::format::size(volume.total)
        ));
    if let Some(fraction) = volume.utilisation() {
        let width = response.rect.width() - 2.0 * ui.spacing().button_padding.x;
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(width.max(20.0), BAR_HEIGHT),
            egui::Sense::hover(),
        );
        utilisation_bar(ui, rect, fraction);
    }
}

/// The *Add…* row of an empty section: how connections and accounts are
/// created. With one provider plugin of the section it opens that
/// plugin's *Add connection…* dialogue, with several it offers one entry
/// per plugin, and without any it says what would create them.
fn add_row(
    ui: &mut egui::Ui,
    app: &App,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
    section: Section,
) {
    let id = egui::Id::new(("side-add", section.name()));
    let connectors = connectors_of(app, section);
    let label = if connectors.len() == 1 {
        format!("{} Add connection…", icons::ADD)
    } else {
        format!("{} Add…", icons::ADD)
    };
    let response = ui.add(
        egui::Button::new(egui::RichText::new(label).weak())
            .frame(false)
            .min_size(egui::vec2(ui.available_width(), 0.0)),
    );
    state.row(id, None);
    focus_ring(ui, state, id, response.rect);
    if connectors.len() > 1 {
        egui::Popup::menu(&response).show(|ui| add_connection_items(ui, requests, &connectors));
    }
    if response.clicked() {
        state.take_focus(id);
        match connectors.as_slice() {
            [one] => requests.push(Request::AddConnection(one.provider())),
            [] => {
                let text = match section {
                    Section::Machine => "Drives appear here as the system mounts them.",
                    Section::Network => {
                        "Connections are created by a network provider plugin, such as ext-vfs-sftp; none is installed."
                    }
                    Section::Cloud => {
                        "Accounts are created by a cloud provider plugin; none is installed."
                    }
                };
                requests.push(Request::Toast(Level::Info, text.to_owned()));
            }
            _ => {}
        }
    }
}

/// The sections drawn, for tests: those shown in display order.
#[must_use]
pub fn sections_shown(app: &App) -> Vec<Section> {
    Section::ALL
        .into_iter()
        .filter(|s| *s != Section::Cloud || !app.model().cloud_providers.is_empty())
        .collect()
}

/// The folded sections of a window, for tests.
#[must_use]
pub fn folded(app: &App, window: WindowId) -> BTreeSet<Section> {
    app.model()
        .window(window)
        .map(|w| w.side_panel.folded.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_and_targets_follow_the_pointer() {
        let rects: Vec<egui::Rect> = (0..3)
            .map(|i| {
                egui::Rect::from_min_size(egui::pos2(0.0, i as f32 * 20.0), egui::vec2(100.0, 20.0))
            })
            .collect();
        assert_eq!(gap_at(&rects, 5.0), 0);
        assert_eq!(gap_at(&rects, 15.0), 1);
        assert_eq!(gap_at(&rects, 45.0), 2);
        assert_eq!(gap_at(&rects, 99.0), 3);
        assert_eq!(target_of(0, 3), 2, "the first row dropped at the end");
        assert_eq!(target_of(2, 0), 0, "the last row dropped at the top");
        assert_eq!(target_of(1, 1), 1, "dropped where it is");
        assert_eq!(target_of(1, 2), 1, "dropped right after itself");
    }
}
