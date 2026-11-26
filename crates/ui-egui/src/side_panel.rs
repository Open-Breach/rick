//! The side panel: the Places list, the This Machine, Network and Cloud
//! sections, and nothing else; the operations drawer floats over its foot
//! from the shell. The panel is navigation only: it takes the user places
//! and never performs a file operation, highlights no current location and
//! draws no indent guide. Rows are `sidebar/item-height` tall with
//! `radius/control` corners, inset 6 px from the panel's edges; the Trash
//! row carries the trash's size in a pill tinted by the share of its disk;
//! This Machine lists drive cards with a usage bar; section headers are
//! small capitals with a caret.
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

use ext::t;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use std::sync::Arc;

use app::{App, Command, ConnectionOp, Remedy, SuggestedConnection};
use ext::{
    ActionCtx, ActionGroup, Availability, ConnectionField, ConnectionFieldKind, ConnectionForm,
    ConnectionSection, Connector, Invocation, Level,
};
use model::{PaneId, Place, Section, SidePanel, SidePanelChange, Volume, VolumeKind, WindowId};
use types::{Entry, Location, ProviderId};
use vfs_rick::Virtual;

use crate::dnd::{Dnd, Target, Zone};
use crate::overview::{percent, read_only_bar, utilisation_bar, volume_icon};
use crate::shell::Request;
use crate::suggestion::Answer;
use crate::theme::{self, BAR, ICON, Scheme, TEXT_BODY, TEXT_CAPS, TEXT_SECONDARY};
use crate::{format, icons, menus};

/// How far a row sits in from the panel's edges (SPEC.md *Rows*).
const INSET: f32 = 6.0;
/// The gap between a row's icon and its label.
const ICON_GAP: f32 = 12.0;
/// A drive card's corners and the gap between cards (SPEC.md *This
/// Machine*).
const CARD_RADIUS: f32 = 8.0;
const CARD_GAP: f32 = 6.0;
/// A drive card's height in Comfortable and in Compact.
const DRIVE_CARD: f32 = 44.0;
const DRIVE_CARD_COMPACT: f32 = 26.0;
/// The hit area of a drive card's eject button, around its 16 px glyph.
const EJECT_HIT: f32 = 24.0;
/// The hover zone of a read-only drive's 4 px bar.
const READ_ONLY_HIT: f32 = 12.0;
/// The air above and below a section header (SPEC.md *Section headers*).
const SECTION_AIR_ABOVE: f32 = 18.0;
const SECTION_AIR_BELOW: f32 = 6.0;
/// A section header's height.
const SECTION_HEADER: f32 = 20.0;
/// The letter spacing of a section header's capitals.
const CAPS_SPACING: f32 = 0.66;
/// The trash pill's height and side padding.
const PILL_HEIGHT: f32 = 20.0;
const PILL_PAD: f32 = 8.0;

/// What sits at the right end of a row.
enum Right {
    Nothing,
    /// 12 px `text/secondary`, a drive's percentage.
    Text(String),
    /// A tinted pill, the Trash row's size.
    Pill {
        text: String,
        bg: egui::Color32,
        fg: egui::Color32,
    },
}

/// A row's content: a 16 px glyph, its label and what sits at the right.
struct RowContent<'a> {
    glyph: &'a str,
    label: &'a str,
    right: Right,
}

/// The rectangle a row paints: the allocated one inset from the panel's
/// edges.
fn inset(outer: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(outer.left() + INSET, outer.top()),
        egui::pos2(
            (outer.right() - INSET).max(outer.left() + INSET),
            outer.bottom(),
        ),
    )
}

/// One line of `text` at `size`, cut with an ellipsis to `max_width`.
fn truncated(
    ui: &egui::Ui,
    text: &str,
    size: f32,
    colour: egui::Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(
        text.to_owned(),
        egui::FontId::proportional(size),
        colour,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(0.0));
    ui.fonts_mut(|f| f.layout_job(job))
}

/// A row (SPEC.md *Rows*): `sidebar/item-height` tall with `radius/control`
/// corners and `space/row-x` padding, inset from the panel's edges; the
/// glyph, 12 px of gap, the label in 13 px, and the right end's content.
/// Hover is `bg/subtle`. Its accessible name is the label with the right
/// end's text after a dot.
fn row_widget(ui: &mut egui::Ui, content: &RowContent<'_>, sense: egui::Sense) -> egui::Response {
    let metrics = theme::metrics(ui.ctx());
    let scheme = theme::scheme(ui);
    let (outer, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), metrics.sidebar_item_height),
        sense,
    );
    let rect = inset(outer);
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, metrics.corner_radius(), scheme.bg_subtle);
    }
    let glyph_x = rect.left() + metrics.space_row_x;
    painter.text(
        egui::pos2(glyph_x + ICON / 2.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        content.glyph,
        egui::FontId::proportional(ICON),
        scheme.icon_default,
    );
    let mut right_edge = rect.right() - metrics.space_row_x;
    let mut spoken = content.label.to_owned();
    match &content.right {
        Right::Nothing => {}
        Right::Text(text) => {
            let galley = painter.layout_no_wrap(
                text.clone(),
                egui::FontId::proportional(TEXT_SECONDARY),
                scheme.text_secondary,
            );
            let size = galley.size();
            painter.galley(
                egui::pos2(right_edge - size.x, rect.center().y - size.y / 2.0),
                galley,
                scheme.text_secondary,
            );
            right_edge -= size.x + ICON_GAP;
            spoken = format!("{spoken} · {text}");
        }
        Right::Pill { text, bg, fg } => {
            let galley = painter.layout_no_wrap(
                text.clone(),
                egui::FontId::proportional(TEXT_SECONDARY),
                *fg,
            );
            let size = galley.size();
            let pill = egui::Rect::from_min_max(
                egui::pos2(
                    right_edge - size.x - 2.0 * PILL_PAD,
                    rect.center().y - PILL_HEIGHT / 2.0,
                ),
                egui::pos2(right_edge, rect.center().y + PILL_HEIGHT / 2.0),
            );
            painter.rect_filled(pill, PILL_HEIGHT / 2.0, *bg);
            painter.galley(
                egui::pos2(pill.left() + PILL_PAD, rect.center().y - size.y / 2.0),
                galley,
                *fg,
            );
            right_edge = pill.left() - ICON_GAP;
            spoken = format!("{spoken} · {text}");
        }
    }
    let label_x = glyph_x + ICON + ICON_GAP;
    let galley = truncated(
        ui,
        content.label,
        TEXT_BODY,
        scheme.text_primary,
        right_edge - label_x,
    );
    let size = galley.size();
    ui.painter().galley(
        egui::pos2(label_x, rect.center().y - size.y / 2.0),
        galley,
        scheme.text_primary,
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spoken.clone()));
    response
}

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

/// The *Add connection…* and *Edit…* dialogue of a network or cloud
/// provider.
#[derive(Debug, Clone)]
pub struct ConnectionDialog {
    /// The provider the connection is for.
    pub provider: ProviderId,
    /// The provider's name in the title.
    pub title: String,
    /// The connection's name as typed.
    pub name: String,
    /// The fields with their values as typed; a `Bool` holds `true` or
    /// `false`, a `Choice` the chosen option's text.
    pub values: Vec<(ConnectionField, String)>,
    /// What stopped the last OK.
    pub error: Option<String>,
    /// Editing an existing connection: the name is fixed and a secret left
    /// empty keeps the one in the keychain.
    pub editing: bool,
    /// The keys of the fields the user edited by hand, here or, in
    /// *Edit…*, when the connection was saved: what the connector's
    /// reaction does not fill.
    pub edited: BTreeSet<String>,
    /// The connector's notes by field key, from its last reaction.
    pub notes: Vec<(&'static str, String)>,
    focus: bool,
}

impl ConnectionDialog {
    /// The dialogue for `connector`, every field at its default.
    #[must_use]
    pub fn new(connector: &dyn Connector) -> Self {
        let schema = connector.schema();
        Self {
            provider: connector.provider(),
            title: schema.title.get(),
            name: String::new(),
            values: schema
                .fields
                .iter()
                .map(|field| {
                    let value = match &field.kind {
                        ConnectionFieldKind::Text { default } => default.clone(),
                        ConnectionFieldKind::Integer { default, .. } => default.to_string(),
                        ConnectionFieldKind::Bool { default } => default.to_string(),
                        ConnectionFieldKind::Choice { options, default } => {
                            options.get(*default).cloned().unwrap_or_default()
                        }
                        ConnectionFieldKind::Path | ConnectionFieldKind::Secret => String::new(),
                    };
                    (field.clone(), value)
                })
                .collect(),
            error: None,
            editing: false,
            edited: BTreeSet::new(),
            notes: Vec::new(),
            focus: true,
        }
    }

    /// The *Add connection…* dialogue for `connector` as it opens: every
    /// field at its default, and the connector's first reaction applied.
    #[must_use]
    pub fn open(connector: &dyn Connector) -> Self {
        let mut dialog = Self::new(connector);
        dialog.react(connector, None, true);
        dialog
    }

    /// The note the connector put by the field `key`.
    #[must_use]
    pub fn note(&self, key: &str) -> Option<&str> {
        self.notes
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, note)| note.as_str())
    }

    /// The user edited the field `key` by hand: it counts as typed from
    /// here on, and the connector reacts to the form as it stands.
    /// `settled` says the input is whole (pasted, picked, or the field was
    /// left) rather than being typed.
    pub fn edited(&mut self, connector: &dyn Connector, key: &'static str, settled: bool) {
        self.edited.insert(key.to_owned());
        self.react(connector, Some(key), settled);
    }

    /// The dialogue is about to save: the connector sees the form once
    /// more, whole, so input typed and never left is taken as it is.
    pub fn settle(&mut self, connector: &dyn Connector) {
        self.react(connector, None, true);
    }

    /// Asks the connector what the form does now and applies its answer:
    /// the rewritten fields, the fills of the fields the user has not
    /// edited by hand, and the notes. A secret is not in the form the
    /// connector sees, and a reaction does not set one.
    fn react(&mut self, connector: &dyn Connector, changed: Option<&str>, settled: bool) {
        let secret = |field: &ConnectionField| field.kind == ConnectionFieldKind::Secret;
        let mut values: BTreeMap<String, String> = self
            .values
            .iter()
            .filter(|(field, _)| !secret(field))
            .map(|(field, value)| (field.key.to_owned(), value.clone()))
            .collect();
        let reaction = connector.react(&ConnectionForm {
            values: &values,
            edited: &self.edited,
            changed,
            settled,
        });
        reaction.apply(&mut values, &self.edited);
        for (field, value) in &mut self.values {
            if !secret(field)
                && let Some(new) = values.get(field.key)
            {
                value.clone_from(new);
            }
        }
        self.edited
            .extend(reaction.rewrite.iter().map(|(key, _)| (*key).to_owned()));
        self.notes = reaction.notes;
    }

    /// The *Edit…* dialogue for the connection `entry` of `connector`: the
    /// name fixed, the fields as the settings hold them, the secrets empty
    /// (an empty secret keeps the keychain's).
    #[must_use]
    pub fn edit(connector: &dyn Connector, entry: &model::ConnectionEntry) -> Self {
        let mut dialog = Self::new(connector);
        dialog.name.clone_from(&entry.name);
        dialog.editing = true;
        for (field, value) in &mut dialog.values {
            if field.kind == ConnectionFieldKind::Secret {
                continue;
            }
            *value = entry.settings.get(field.key).cloned().unwrap_or_default();
            // What was saved counts as typed: a reaction does not replace
            // it. A field saved empty is open to a fill.
            if !value.is_empty() {
                dialog.edited.insert(field.key.to_owned());
            }
        }
        dialog.react(connector, None, true);
        dialog
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
        if !self.editing && taken.iter().any(|t| t == name) {
            return Err(format!("A connection called {name} exists already"));
        }
        let mut values = Vec::with_capacity(self.values.len());
        for (field, value) in &self.values {
            let value = value.trim();
            if value.is_empty() {
                // An edited connection keeps the secret it has.
                if field.required && !(self.editing && field.kind == ConnectionFieldKind::Secret) {
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
        if self.editing {
            Ok(Command::EditConnection {
                provider: self.provider,
                name: name.to_owned(),
                values,
            })
        } else {
            Ok(Command::AddConnection {
                provider: self.provider,
                name: name.to_owned(),
                values,
            })
        }
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
        .map_or(app.model().focused, model::Window::focused)
}

/// Draws the panel for `window`: navigation goes out as requests, drop
/// zones into `dnd`.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    app: &App,
    window: WindowId,
    panel: &SidePanel,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
    dnd: &mut Dnd,
    drawer_room: f32,
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
    egui::ScrollArea::vertical()
        .id_salt(("side-panel", window.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(4.0);
            places(ui, app, pane, state, requests, dnd);
            for section in sections_shown(app) {
                titled_section(ui, app, window, pane, panel, section, state, requests);
            }
            // Room for the operations drawer floating over the lists' end.
            ui.add_space(drawer_room + 8.0);
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

/// The kinds of connection the suggested plugins would add to `section`
/// (specs/plugin-distribution.md §8, *Adding a connection*): a plugin of
/// the registry that is not installed, or one that is on the disk and
/// disabled. None under This Machine.
fn suggested_kinds(app: &App, section: Section) -> Vec<SuggestedConnection> {
    match section {
        Section::Machine => Vec::new(),
        Section::Network => app.suggested_connections(ConnectionSection::Network),
        Section::Cloud => app.suggested_connections(ConnectionSection::Cloud),
    }
}

/// Opens the *Add connection…* dialogue of `provider`; the shell calls
/// this for `Request::AddConnection`.
pub fn begin_connection(state: &mut SidePanelState, app: &App, provider: ProviderId) {
    if let Some(connector) = app.registry().connector_of(provider) {
        state.connection = Some(ConnectionDialog::open(&**connector));
    }
}

/// Opens the *Edit…* dialogue of the connection `name` of `provider`; the
/// shell calls this for `Request::EditConnection`.
pub fn begin_edit_connection(
    state: &mut SidePanelState,
    app: &App,
    provider: ProviderId,
    name: &str,
) {
    let Some(connector) = app.registry().connector_of(provider) else {
        return;
    };
    let Some(entry) = app
        .model()
        .settings
        .connections
        .iter()
        .find(|c| c.provider == provider.0 && c.name == name)
    else {
        return;
    };
    state.connection = Some(ConnectionDialog::edit(&**connector, entry));
}

/// Whether the application keeps a connection `name` of `provider` in its
/// settings: those rows can be edited and removed; a row the provider lists
/// from elsewhere (a host of `~/.ssh/config`) cannot.
fn is_rick_connection(app: &App, provider: ProviderId, name: &str) -> bool {
    app.model()
        .settings
        .connections
        .iter()
        .any(|c| c.provider == provider.0 && c.name == name)
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
    let connector = app.registry().connector_of(dialog.provider).cloned();
    let mut touched: Option<(&'static str, bool)> = None;
    let mut open = true;
    let mut done: Option<bool> = None;
    let title = if dialog.editing {
        format!("Edit {} connection", dialog.title)
    } else {
        format!("Add {} connection", dialog.title)
    };
    egui::Window::new(title)
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
                    ui.label(t!("compare-name"));
                    let name = ui.add_enabled(
                        !dialog.editing,
                        egui::TextEdit::singleline(&mut dialog.name).desired_width(260.0),
                    );
                    if dialog.focus {
                        name.request_focus();
                        dialog.focus = false;
                    }
                    ui.end_row();
                    let editing = dialog.editing;
                    for (field, value) in &mut dialog.values {
                        let note = dialog
                            .notes
                            .iter()
                            .find(|(key, _)| *key == field.key)
                            .map(|(_, note)| note.as_str());
                        if let Some(settled) = connection_field(ui, field, value, editing, note) {
                            touched = Some((field.key, settled));
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
                if ui.button(t!("ok")).clicked() || enter {
                    done = Some(true);
                }
                if ui.button(t!("cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    done = Some(false);
                }
            });
        });
    if !open {
        done = Some(false);
    }
    // The connector reacts to the edit before anything else reads the
    // form, an OK in the same frame included.
    if let (Some((key, settled)), Some(connector)) = (touched, &connector) {
        dialog.edited(&**connector, key, settled);
    }
    if let (Some(true), Some(connector)) = (done, &connector) {
        dialog.settle(&**connector);
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

/// One row of the connection dialogue: the label, then the field as its
/// kind asks: a masked text for a secret (left empty while editing, it
/// keeps the keychain's), a checkbox, a combo box for a choice, a text
/// field for the rest. The connector's `note` for the field, when it has
/// one, has the row under the control. Returns `None` while the user left the
/// value alone, and otherwise whether the edit is whole: text that was
/// pasted, a field typed in and now left, a box ticked, an option picked.
/// Text being typed is `Some(false)`.
fn connection_field(
    ui: &mut egui::Ui,
    field: &ConnectionField,
    value: &mut String,
    editing: bool,
    note: Option<&str>,
) -> Option<bool> {
    let label = if field.required {
        format!("{} *", field.label)
    } else {
        field.label.get()
    };
    ui.label(label);
    // A text field: typing is not whole, a paste and leaving the field are.
    let typed = |ui: &egui::Ui, response: &egui::Response| {
        let pasted = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
        if response.changed() {
            Some(pasted)
        } else if response.lost_focus() {
            Some(true)
        } else {
            None
        }
    };
    let mut edit = None;
    match &field.kind {
        ConnectionFieldKind::Secret => {
            let hint = if editing { "(unchanged)" } else { "" };
            let response = ui.add(
                egui::TextEdit::singleline(value)
                    .password(true)
                    .hint_text(hint)
                    .desired_width(260.0),
            );
            edit = typed(ui, &response);
        }
        ConnectionFieldKind::Bool { .. } => {
            let mut on = value == "true";
            if ui.checkbox(&mut on, "").changed() {
                *value = on.to_string();
                edit = Some(true);
            }
        }
        ConnectionFieldKind::Choice { options, .. } => {
            let shown = if value.is_empty() {
                "(none)"
            } else {
                value.as_str()
            };
            egui::ComboBox::from_id_salt(("add-connection-choice", field.key))
                .selected_text(shown.to_owned())
                .width(260.0)
                .show_ui(ui, |ui| {
                    for option in options {
                        let label = if option.is_empty() {
                            "(none)"
                        } else {
                            option.as_str()
                        };
                        if ui.selectable_label(value == option, label).clicked() {
                            value.clone_from(option);
                            edit = Some(true);
                        }
                    }
                });
        }
        ConnectionFieldKind::Text { .. }
        | ConnectionFieldKind::Integer { .. }
        | ConnectionFieldKind::Path => {
            let response = ui.add(egui::TextEdit::singleline(value).desired_width(260.0));
            edit = typed(ui, &response);
        }
    }
    // The connector's note has the row under its field to itself, so the
    // controls keep their column and their width.
    if let Some(note) = note {
        ui.end_row();
        ui.label("");
        ui.weak(note);
    }
    edit
}

/// The menu of a section's header, on the menu card of the file list
/// (SPEC.md *Context menu*): *Open in new tab* for its overview and,
/// under a hairline, *Add … connection…* for each provider plugin of the
/// section, followed by the kinds the suggested plugins would add.
fn header_menu(
    title: &egui::Response,
    requests: &mut Vec<Request>,
    pane: PaneId,
    overview: &Location,
    connectors: &[Arc<dyn Connector>],
    suggested: &[SuggestedConnection],
) {
    menus::card(title, |ui| {
        if menus::item(ui, None, &t!("open-in-new-tab")).clicked() {
            open(requests, pane, overview.clone(), true);
            ui.close();
        }
        if !connectors.is_empty() || !suggested.is_empty() {
            menus::divider(ui);
        }
        add_items(ui, requests, connectors, suggested);
    });
}

/// What a section's *Add…* lists: the *Add connection…* entries of its
/// provider plugins and, after them, the kinds of the suggested plugins.
fn add_items(
    ui: &mut egui::Ui,
    requests: &mut Vec<Request>,
    connectors: &[Arc<dyn Connector>],
    suggested: &[SuggestedConnection],
) {
    add_connection_items(ui, requests, connectors);
    if !connectors.is_empty() && !suggested.is_empty() {
        menus::divider(ui);
    }
    suggested_items(ui, requests, suggested);
}

/// The kinds of connection of suggested plugins, as they follow the ones
/// that are there (specs/plugin-distribution.md §8): the kind's title
/// with `DOWNLOAD_SIMPLE` and the size of the plugin's package, and
/// choosing one installs the plugin. A plugin that is on the disk and
/// disabled has `PUZZLE_PIECE` and *Enable* in the size's place, and
/// choosing it switches the plugin on.
fn suggested_items(
    ui: &mut egui::Ui,
    requests: &mut Vec<Request>,
    suggested: &[SuggestedConnection],
) {
    for kind in suggested {
        let (glyph, right, answer) = match kind.plugin.remedy {
            Remedy::Install {
                installing: true, ..
            } => {
                let waiting = t!("extensions-installing");
                menus::greyed_item_with(ui, Some(icons::INSTALL), &kind.title, &waiting, &waiting);
                continue;
            }
            Remedy::Install { size, .. } => (
                icons::INSTALL,
                crate::format::size(Some(size)),
                Answer::Install,
            ),
            Remedy::Enable => (icons::PLUGIN, t!("suggest-enable"), Answer::Enable),
        };
        let entry = menus::item_with(ui, Some(glyph), &kind.title, &right);
        // What choosing it does, in the plugin's own words.
        let entry = entry.on_hover_text(crate::suggestion::text(&kind.plugin));
        if entry.clicked() {
            requests.push(answer.request(&kind.plugin.id, crate::suggestion::Place::List));
            ui.close();
        }
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
        let kind = connector.schema().title.to_string();
        let label = t!("panel-add-a-connection", kind = kind.as_str());
        if menus::item(ui, Some(icons::ADD), &label).clicked() {
            requests.push(Request::AddConnection(connector.provider()));
            ui.close();
        }
    }
}

/// The context-menu entries of a connection row: *Connect* or
/// *Disconnect*, and for a connection the application keeps *Edit…* and
/// *Remove connection*.
fn connection_items(
    ui: &mut egui::Ui,
    requests: &mut Vec<Request>,
    connector: &dyn Connector,
    name: &str,
    editable: bool,
) {
    let provider = connector.provider();
    if connector.is_connected(name) {
        if menus::item(ui, Some(icons::UNLINKED), &t!("panel-disconnect")).clicked() {
            requests.push(Request::Command(Command::Connection {
                provider,
                name: name.to_owned(),
                op: ConnectionOp::Disconnect,
            }));
            ui.close();
        }
    } else if menus::item(ui, Some(icons::LINKED), &t!("panel-connect")).clicked() {
        requests.push(Request::Command(Command::Connection {
            provider,
            name: name.to_owned(),
            op: ConnectionOp::Connect,
        }));
        ui.close();
    }
    if !editable {
        return;
    }
    if menus::item(ui, Some(icons::EDIT), &t!("panel-edit")).clicked() {
        requests.push(Request::EditConnection(provider, name.to_owned()));
        ui.close();
    }
    if menus::item(ui, Some(icons::TRASH), &t!("panel-remove-connection")).clicked() {
        requests.push(Request::Command(Command::Connection {
            provider,
            name: name.to_owned(),
            op: ConnectionOp::Remove,
        }));
        ui.close();
    }
}

/// *Always preview* in a connection row's menu, for every source that has
/// connections — an SSH host and a cloud account alike: the size limit of
/// previews on or off for this source, as its connection entry has it.
/// It is where the switch of the size-limit card is taken back.
fn preview_always_item(
    ui: &mut egui::Ui,
    app: &App,
    requests: &mut Vec<Request>,
    connector: &dyn Connector,
    name: &str,
) {
    let provider = connector.provider();
    let on = app
        .model()
        .settings
        .connections
        .iter()
        .find(|c| c.provider == provider.0 && c.name == name)
        .and_then(|c| c.settings.get(app::PREVIEW_ALWAYS))
        .is_some_and(|v| v == "true");
    if menus::check_item(ui, &t!("preview-always-preview"), on).clicked() {
        requests.push(Request::Command(Command::SetPreviewAlways {
            provider,
            name: name.to_owned(),
            on: !on,
        }));
        ui.close();
    }
}

/// The entries of the extension groups whose actions apply to a row's
/// location, as the context menu of a folder would list them: *Open console
/// here* and *Open in external terminal* on an SSH host.
fn row_actions(ui: &mut egui::Ui, app: &App, requests: &mut Vec<Request>, ctx: &ActionCtx) {
    let actions = app.registry().actions();
    let mut groups: Vec<&'static str> = actions
        .iter()
        .filter_map(|a| match a.group() {
            ActionGroup::Extension(name) => Some(name),
            ActionGroup::Core => None,
        })
        .collect();
    groups.sort_unstable();
    groups.dedup();
    for group in groups {
        let members: Vec<_> = actions
            .iter()
            .filter(|a| {
                a.group() == ActionGroup::Extension(group)
                    && a.availability(ctx) == Availability::Enabled
            })
            .collect();
        if members.is_empty() {
            continue;
        }
        menus::divider(ui);
        for action in members {
            if menus::item_of(ui, &**action).clicked() {
                requests.push(Request::RunAction {
                    action: action.id(),
                    ctx: ctx.clone(),
                });
                ui.close();
            }
        }
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
    let content = RowContent {
        glyph,
        label: &entry.name,
        right: Right::Nothing,
    };
    let connection = connection_of(app, entry);
    let action_ctx = ActionCtx {
        pane,
        location: location.clone(),
        selection: Vec::new(),
        invoked_from: Invocation::ContextMenu,
        other: None,
    };
    let mut extra = |ui: &mut egui::Ui, requests: &mut Vec<Request>| {
        if let Some((connector, name)) = &connection {
            let editable = is_rick_connection(app, connector.provider(), name);
            menus::divider(ui);
            connection_items(ui, requests, &***connector, name, editable);
            preview_always_item(ui, app, requests, &***connector, name);
        }
        row_actions(ui, app, requests, &action_ctx);
    };
    let response = location_row(
        ui,
        state,
        requests,
        pane,
        egui::Id::new(("side-provider-row", section.name(), index)),
        &content,
        egui::Sense::click(),
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

/// Paints the keyboard focus around a row when the panel has it: the 2 px
/// `accent/focus` ring inside the row's rectangle.
fn focus_ring(ui: &egui::Ui, state: &SidePanelState, id: egui::Id, rect: egui::Rect) {
    if state.focused && state.focus == Some(id) {
        ui.painter().rect_stroke(
            rect,
            theme::metrics(ui.ctx()).corner_radius(),
            theme::scheme(ui).focus_ring(),
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
    content: &RowContent<'_>,
    sense: egui::Sense,
    location: &Location,
    extra: &mut dyn FnMut(&mut egui::Ui, &mut Vec<Request>),
) -> egui::Response {
    let response = row_widget(ui, content, sense);
    state.row(id, Some(location.clone()));
    focus_ring(ui, state, id, inset(response.rect));
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
    // The menu card of the file list (SPEC.md *Context menu*): *Open in
    // new tab*, then what the row adds, each group under a hairline.
    menus::card(&response, |ui| {
        if menus::item(ui, None, &t!("open-in-new-tab")).clicked() {
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
    ui.painter()
        .hline(x_range, y, crate::theme::scheme(ui).focus_ring());
}

/// The Places list.
#[allow(clippy::too_many_lines)]
fn places(
    ui: &mut egui::Ui,
    app: &App,
    pane: PaneId,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
    dnd: &mut Dnd,
) {
    let rows: Vec<Place> = app.model().places.rows().to_vec();
    let trash = &app.model().trash;
    let scheme = theme::scheme(ui);
    let mut rects = Vec::with_capacity(rows.len());
    let mut lines: Vec<f32> = Vec::new();
    let list_top = ui.cursor().top();
    for (index, place) in rows.iter().enumerate() {
        let id = egui::Id::new(("side-place", index, &place.location));
        let glyph = icons::named(&place.icon);
        // The Trash pill: the trash's size, tinted by the share of its disk
        // it takes; nothing while the trash is empty.
        let right =
            if !trash.items.is_empty() && Virtual::of(&place.location) == Some(Virtual::Trash) {
                let (bg, fg) = trash_tint(scheme, trash.share);
                Right::Pill {
                    text: format::size(Some(trash.size)),
                    bg,
                    fg,
                }
            } else {
                Right::Nothing
            };
        let content = RowContent {
            glyph,
            label: &place.title,
            right,
        };
        let location = place.location.clone();
        let mut extra = |ui: &mut egui::Ui, requests: &mut Vec<Request>| {
            menus::divider(ui);
            if menus::item(ui, Some(icons::EDIT), &t!("panel-edit")).clicked() {
                state_edit_start(requests, index, place);
                ui.close();
            }
            if menus::item(ui, None, &t!("remove")).clicked() {
                requests.push(Request::Command(Command::PlaceRemove(index)));
                ui.close();
            }
        };
        let response = location_row(
            ui,
            state,
            requests,
            pane,
            id,
            &content,
            egui::Sense::click_and_drag(),
            &location,
            &mut extra,
        );
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
        ui.weak(t!("panel-drop-a-folder-here-to"));
    }
}

/// The Trash pill's colours for the share of its disk the trash takes
/// (SPEC.md *Trash pill*): `trash/low-*` under 1 %, `trash/mid-*` to 5 %,
/// `trash/high-*` over; low while the share is unknown.
fn trash_tint(scheme: &Scheme, share: Option<f32>) -> (egui::Color32, egui::Color32) {
    match share {
        Some(share) if share >= 0.05 => (scheme.trash_high_bg, scheme.trash_high_text),
        Some(share) if share >= 0.01 => (scheme.trash_mid_bg, scheme.trash_mid_text),
        _ => (scheme.trash_low_bg, scheme.trash_low_text),
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
    egui::Window::new(t!("panel-edit-place"))
        .id(egui::Id::new("edit-place"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(t!("panel-title"));
            let title = ui.text_edit_singleline(&mut edit.title);
            if edit.focus {
                title.request_focus();
                edit.focus = false;
            }
            ui.add_space(6.0);
            ui.label(t!("panel-icon"));
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
                if ui.button(t!("ok")).clicked() || enter {
                    done = Some(true);
                }
                if ui.button(t!("cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
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

/// A titled section (SPEC.md *Section headers*): the header with its caret
/// and small capitals, itself the row of the section's overview, then the
/// rows — drive cards under This Machine, network volumes and connections
/// under Network, accounts under Cloud — or the *Add…* row of an empty
/// section. Nothing is indented.
#[allow(clippy::too_many_arguments)]
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
    section_header(
        ui, app, window, pane, section, folded, &overview, state, requests,
    );
    if folded {
        return;
    }
    let mut any = false;
    let volumes: Vec<Volume> = app
        .model()
        .volumes
        .iter()
        .filter(|v| (v.kind == VolumeKind::Network) == (section == Section::Network))
        .cloned()
        .collect();
    let density = app.model().settings.density;
    let format_tool = app.model().format_tool;
    match section {
        Section::Machine => {
            for volume in &volumes {
                any = true;
                drive_card(ui, state, requests, pane, volume, density, format_tool);
            }
        }
        Section::Network => {
            for volume in &volumes {
                any = true;
                network_volume_row(ui, state, requests, pane, volume);
            }
        }
        Section::Cloud => {}
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
}

/// The header of a titled section: 18 px of air, a `CARET_DOWN` at 60 %
/// (`CARET_RIGHT` when folded) that folds the section, and the title in
/// 11 px letter-spaced capitals, which opens the section's overview like a
/// row; 6 px of air below.
#[allow(clippy::too_many_arguments)]
fn section_header(
    ui: &mut egui::Ui,
    app: &App,
    window: WindowId,
    pane: PaneId,
    section: Section,
    folded: bool,
    overview: &Location,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
) {
    let metrics = theme::metrics(ui.ctx());
    let scheme = theme::scheme(ui);
    ui.add_space(SECTION_AIR_ABOVE);
    let (outer, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), SECTION_HEADER),
        egui::Sense::hover(),
    );
    let rect = inset(outer);
    // Salted with the window: every window draws its own header.
    let id = egui::Id::new(("side-section", window.0, section.name()));
    // The caret.
    let caret_rect = egui::Rect::from_center_size(
        egui::pos2(
            rect.left() + metrics.space_row_x + ICON / 2.0,
            rect.center().y,
        ),
        egui::vec2(ICON + 4.0, SECTION_HEADER),
    );
    let caret = ui.interact(caret_rect, id.with("caret"), egui::Sense::click());
    ui.painter().text(
        caret_rect.center(),
        egui::Align2::CENTER_CENTER,
        if folded {
            icons::TREE_CLOSED
        } else {
            icons::TREE_OPEN
        },
        egui::FontId::proportional(ICON),
        scheme.icon_default.gamma_multiply(0.6),
    );
    let fold_text = if folded {
        t!("panel-unfold-section", section = section.title())
    } else {
        t!("panel-fold-section", section = section.title())
    };
    caret.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, fold_text.clone())
    });
    if caret.on_hover_text(fold_text).clicked() {
        requests.push(Request::Command(Command::SidePanel {
            window,
            change: SidePanelChange::Fold {
                section,
                folded: !folded,
            },
        }));
    }
    // The title, the overview's row.
    let title_rect =
        egui::Rect::from_min_max(egui::pos2(caret_rect.right() + 6.0, rect.top()), rect.max);
    let title = ui.interact(title_rect, id, egui::Sense::click());
    if title.hovered() {
        ui.painter()
            .rect_filled(title_rect, metrics.corner_radius(), scheme.bg_subtle);
    }
    let caps = egui::WidgetText::from(
        egui::RichText::new(section.title().to_uppercase())
            .size(TEXT_CAPS)
            .color(scheme.text_secondary)
            .extra_letter_spacing(CAPS_SPACING),
    )
    .into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        title_rect.width() - 2.0 * metrics.space_row_x,
        egui::TextStyle::Body,
    );
    let size = caps.size();
    ui.painter().galley(
        egui::pos2(
            title_rect.left() + metrics.space_row_x,
            title_rect.center().y - size.y / 2.0,
        ),
        caps,
        scheme.text_secondary,
    );
    state.row(id, Some(overview.clone()));
    focus_ring(ui, state, id, title_rect);
    title
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, section.title()));
    if title.clicked() {
        state.take_focus(id);
        let new_tab = ui.input(|i| i.modifiers.command);
        open(requests, pane, overview.clone(), new_tab);
    }
    if title.middle_clicked() {
        state.take_focus(id);
        open(requests, pane, overview.clone(), true);
    }
    if title.secondary_clicked() {
        state.take_focus(id);
    }
    let title = title.on_hover_text(t!("panel-open-overview", section = section.title()));
    header_menu(
        &title,
        requests,
        pane,
        overview,
        &connectors_of(app, section),
        &suggested_kinds(app, section),
    );
    ui.add_space(SECTION_AIR_BELOW);
}

/// A drive card (SPEC.md *This Machine*): a `bg/subtle` card with 8 px
/// corners, 44 px tall in Comfortable — the drive's icon at the left, the
/// name on one line with the utilisation percentage at the right end of
/// the line, a 4 px usage bar under them coloured by how full the drive is
/// — and 26 px in Compact, one line: icon, name, bar, percentage. A
/// read-only drive shows a full bar in `text/placeholder` and no
/// percentage, the bar's tooltip and accessible description *Read-only*.
/// A removable drive carries the eject button at the card's right edge,
/// vertically centred and always visible, and the rest of the row ends
/// before it. A click opens the drive's root; Eject is in the context menu
/// of removable media too.
#[allow(clippy::too_many_lines)]
fn drive_card(
    ui: &mut egui::Ui,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
    pane: PaneId,
    volume: &Volume,
    density: model::Density,
    format_tool: bool,
) {
    let metrics = theme::metrics(ui.ctx());
    let scheme = theme::scheme(ui);
    let compact = density == model::Density::Compact;
    let height = if compact {
        DRIVE_CARD_COMPACT
    } else {
        DRIVE_CARD
    };
    let (outer, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    ui.add_space(CARD_GAP);
    let rect = inset(outer);
    ui.painter()
        .rect_filled(rect, CARD_RADIUS, scheme.bg_subtle);
    // The eject button first, over the card and before the painter the
    // rest of the row draws with; the row's content ends before it.
    let mut right = rect.right() - metrics.space_row_x;
    let removable = volume.is_removable();
    let eject = removable.then(|| eject_button(ui, rect, right, &volume.name));
    if removable {
        right -= ICON + ICON_GAP;
    }
    let painter = ui.painter();
    let glyph_x = rect.left() + metrics.space_row_x;
    painter.text(
        egui::pos2(glyph_x + ICON / 2.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        volume_icon(volume.kind),
        egui::FontId::proportional(ICON),
        scheme.icon_default,
    );
    let label_x = glyph_x + ICON + ICON_GAP;
    // A read-only drive shows no percentage: the bar says it all.
    let read_only = volume.read_only;
    let percent_text = if read_only {
        t!("panel-read-only")
    } else {
        percent(volume)
    };
    let percent_galley = painter.layout_no_wrap(
        percent_text.clone(),
        egui::FontId::proportional(TEXT_SECONDARY),
        scheme.text_secondary,
    );
    let percent_size = if read_only {
        egui::Vec2::ZERO
    } else {
        percent_galley.size()
    };
    // The room the percentage takes before the bar, in Compact.
    let percent_gap = if read_only { 0.0 } else { ICON_GAP };
    let fraction = volume.utilisation().unwrap_or(0.0);
    let bar;
    if compact {
        // One line: name, the bar, the percentage.
        let bar_width = 48.0;
        if !read_only {
            painter.galley(
                egui::pos2(
                    right - percent_size.x,
                    rect.center().y - percent_size.y / 2.0,
                ),
                percent_galley,
                scheme.text_secondary,
            );
        }
        bar = egui::Rect::from_min_size(
            egui::pos2(
                right - percent_size.x - percent_gap - bar_width,
                rect.center().y - BAR / 2.0,
            ),
            egui::vec2(bar_width, BAR),
        );
        if read_only {
            read_only_bar(ui, bar);
        } else {
            utilisation_bar(ui, bar, fraction);
        }
        let name = truncated(
            ui,
            &volume.name,
            TEXT_BODY,
            scheme.text_primary,
            bar.left() - ICON_GAP - label_x,
        );
        let size = name.size();
        ui.painter().galley(
            egui::pos2(label_x, rect.center().y - size.y / 2.0),
            name,
            scheme.text_primary,
        );
    } else {
        let line_y = rect.top() + 15.0;
        if !read_only {
            painter.galley(
                egui::pos2(right - percent_size.x, line_y - percent_size.y / 2.0),
                percent_galley,
                scheme.text_secondary,
            );
        }
        let name = truncated(
            ui,
            &volume.name,
            TEXT_BODY,
            scheme.text_primary,
            right - percent_size.x - percent_gap - label_x,
        );
        let size = name.size();
        ui.painter().galley(
            egui::pos2(label_x, line_y - size.y / 2.0),
            name,
            scheme.text_primary,
        );
        bar = egui::Rect::from_min_size(
            egui::pos2(label_x, rect.top() + 30.0),
            egui::vec2((right - label_x).max(0.0), BAR),
        );
        if read_only {
            read_only_bar(ui, bar);
        } else {
            utilisation_bar(ui, bar, fraction);
        }
    }
    if read_only {
        read_only_hint(ui, bar, &volume.mount_point);
    }
    let location = Location::new(
        vfs_local::PROVIDER_ID,
        platform::paths::from_path(&volume.mount_point),
    );
    let id = egui::Id::new(("side-volume", &volume.mount_point));
    state.row(id, Some(location.clone()));
    // The ring follows the card's corners.
    if state.focused && state.focus == Some(id) {
        ui.painter().rect_stroke(
            rect,
            CARD_RADIUS,
            scheme.focus_ring(),
            egui::StrokeKind::Inside,
        );
    }
    let spoken = format!("{} · {percent_text}", volume.name);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spoken.clone()));
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
    let mount_point = volume.mount_point.clone();
    if eject.is_some_and(|eject| eject.clicked()) {
        state.take_focus(id);
        requests.push(Request::Command(Command::Eject(mount_point.clone())));
    }
    let response = response.on_hover_text(volume_hint(volume));
    menus::card(&response, |ui| {
        if menus::item(ui, None, &t!("open-in-new-tab")).clicked() {
            open(requests, pane, location.clone(), true);
            ui.close();
        }
        menus::drive_items(ui, volume, format_tool, requests);
    });
}

/// The hover zone of a read-only drive's bar, a little taller than the 4 px
/// bar: the tooltip *Read-only*, which is also the zone's accessible name
/// and description (SPEC.md *Accessibility*).
fn read_only_hint(ui: &mut egui::Ui, bar: egui::Rect, mount_point: &Path) {
    let zone = bar.expand2(egui::vec2(0.0, (READ_ONLY_HIT - BAR) / 2.0));
    let response = ui.interact(
        zone,
        egui::Id::new(("side-read-only", mount_point)),
        egui::Sense::hover(),
    );
    let text = t!("panel-read-only");
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, true, text.clone())
    });
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_description(text.clone());
    });
    response.on_hover_text(text);
}

/// The eject button of a removable drive's card: `EJECT` at the card's
/// right edge (`right` is where the row's content ends), vertically
/// centred, always visible; `bg/control` while hovered. Named *Eject
/// <drive>* for the tooltip and the screen reader.
fn eject_button(ui: &mut egui::Ui, card: egui::Rect, right: f32, name: &str) -> egui::Response {
    let scheme = theme::scheme(ui);
    let rect = egui::Rect::from_center_size(
        egui::pos2(right - ICON / 2.0, card.center().y),
        egui::vec2(EJECT_HIT, EJECT_HIT),
    );
    let response = ui.interact(
        rect,
        egui::Id::new(("side-eject", name)),
        egui::Sense::click(),
    );
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            theme::metrics(ui.ctx()).corner_radius(),
            scheme.bg_control,
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::EJECT,
        egui::FontId::proportional(ICON),
        scheme.icon_default,
    );
    let label = t!("panel-eject", name = name);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
    response.on_hover_text(label)
}

/// The hover text of a drive: mount point, file system, free of total.
fn volume_hint(volume: &Volume) -> String {
    format!(
        "{}\n{} · {} free of {}",
        volume.mount_point.display(),
        volume.fs_type,
        format::size(volume.free),
        format::size(volume.total)
    )
}

/// A mounted network file system under Network: a row with the network
/// icon, the share's name and its utilisation percentage at the right.
fn network_volume_row(
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
    let right = if volume.total.is_some() {
        Right::Text(percent(volume))
    } else {
        Right::Nothing
    };
    let content = RowContent {
        glyph: volume_icon(volume.kind),
        label: &volume.name,
        right,
    };
    let mount_point = volume.mount_point.clone();
    let removable = volume.is_removable();
    let mut extra = |ui: &mut egui::Ui, requests: &mut Vec<Request>| {
        if removable {
            menus::divider(ui);
            if menus::item(ui, Some(icons::EJECT), &t!("panel-eject-drive")).clicked() {
                requests.push(Request::Command(Command::Eject(mount_point.clone())));
                ui.close();
            }
        }
    };
    location_row(
        ui,
        state,
        requests,
        pane,
        id,
        &content,
        egui::Sense::click(),
        &location,
        &mut extra,
    )
    .on_hover_text(volume_hint(volume));
}

/// The *Add…* row of an empty section (`PLUS_CIRCLE`): how connections and
/// accounts are created. With one provider plugin of the section it opens
/// that plugin's *Add connection…* dialogue, with several it offers one
/// entry per plugin, and without any it says what would create them.
/// Where a plugin that is not there would add a kind of connection, the
/// row lists the kinds: those of the loaded plugins, then the suggested
/// ones, each with the size of its plugin.
fn add_row(
    ui: &mut egui::Ui,
    app: &App,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
    section: Section,
) {
    let id = egui::Id::new(("side-add", section.name()));
    let connectors = connectors_of(app, section);
    let suggested = suggested_kinds(app, section);
    let label = if connectors.len() == 1 && suggested.is_empty() {
        t!("panel-add-connection")
    } else {
        t!("panel-add")
    };
    let content = RowContent {
        glyph: icons::ADD,
        label: &label,
        right: Right::Nothing,
    };
    let response = row_widget(ui, &content, egui::Sense::click());
    state.row(id, None);
    focus_ring(ui, state, id, inset(response.rect));
    // One loaded kind and nothing suggested opens its dialogue at once;
    // anything else is a list to choose from. A suggested kind is never
    // chosen by the click on *Add…* itself: choosing it installs.
    if connectors.len() > 1 || !suggested.is_empty() {
        menus::card_under(&response, |ui| {
            add_items(ui, requests, &connectors, &suggested);
        });
    }
    if response.clicked() {
        state.take_focus(id);
        match connectors.as_slice() {
            _ if !suggested.is_empty() => {}
            [one] => requests.push(Request::AddConnection(one.provider())),
            [] => {
                let text = match section {
                    Section::Machine => "Drives appear here as the system mounts them.",
                    Section::Network => {
                        "Connections are created by a network provider plugin, such as ext-ssh; none is installed."
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

/// The sections drawn, in display order. Cloud is absent while no cloud
/// provider plugin is loaded and none is suggested: it shows, with
/// *Add…*, while the registry offers one (SPEC.md *Cloud*,
/// specs/plugin-distribution.md §8).
#[must_use]
pub fn sections_shown(app: &App) -> Vec<Section> {
    Section::ALL
        .into_iter()
        .filter(|section| {
            *section != Section::Cloud
                || !app.model().cloud_providers.is_empty()
                || !suggested_kinds(app, Section::Cloud).is_empty()
        })
        .collect()
}

/// The folded sections of a window, for tests.
#[must_use]
pub fn folded(app: &App, window: WindowId) -> BTreeSet<Section> {
    app.model()
        .window(window)
        .map_or_default(|w| w.side_panel.folded.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A connector that reacts as the worked example of `ext::connection`
    /// does: it settles the port of a host it recognises and says so, and
    /// takes a port typed onto the host into the port field.
    struct Reacting;

    #[ext::async_trait]
    impl Connector for Reacting {
        fn provider(&self) -> ProviderId {
            ProviderId("reacting")
        }
        fn schema(&self) -> ext::ConnectionSchema {
            let text = |key, default: &str| ConnectionField {
                key,
                label: ext::Text(key),
                kind: ConnectionFieldKind::Text {
                    default: default.to_owned(),
                },
                required: false,
            };
            ext::ConnectionSchema {
                title: ext::Text("reacting"),
                fields: vec![
                    text("host", ""),
                    text("port", "22"),
                    ConnectionField {
                        key: "token",
                        label: ext::Text("token"),
                        kind: ConnectionFieldKind::Secret,
                        required: false,
                    },
                ],
            }
        }
        fn update(&self, _connections: Vec<ext::Connection>) {}
        async fn connect(&self, _name: &str) -> Result<(), types::Error> {
            Ok(())
        }
        fn disconnect(&self, _name: &str) {}
        fn is_connected(&self, _name: &str) -> bool {
            false
        }
        fn react(&self, form: &ConnectionForm<'_>) -> ext::ConnectionReaction {
            assert!(
                !form.values.contains_key("token"),
                "a secret reached a reaction"
            );
            let mut reaction = ext::ConnectionReaction::default();
            let host = form.get("host");
            if let Some((name, port)) = host.split_once(':').filter(|_| form.settled) {
                reaction.rewrite.push(("host", name.to_owned()));
                reaction.rewrite.push(("port", port.to_owned()));
            } else if host.ends_with(".example.org") {
                reaction.fill.push(("port", "8443".to_owned()));
                reaction.fill.push(("token", "never".to_owned()));
                reaction.notes.push(("host", "Example Cloud".to_owned()));
            }
            reaction
        }
    }

    fn value<'a>(dialog: &'a ConnectionDialog, key: &str) -> &'a str {
        dialog
            .values
            .iter()
            .find(|(field, _)| field.key == key)
            .map_or("", |(_, value)| value.as_str())
    }

    fn type_into(dialog: &mut ConnectionDialog, key: &'static str, text: &str) {
        for (field, value) in &mut dialog.values {
            if field.key == key {
                text.clone_into(value);
            }
        }
        dialog.edited(&Reacting, key, true);
    }

    #[test]
    fn the_connection_dialogue_fills_what_the_user_has_not_typed_and_shows_the_note() {
        let mut dialog = ConnectionDialog::open(&Reacting);
        assert_eq!(value(&dialog, "port"), "22");
        assert_eq!(dialog.note("host"), None);
        type_into(&mut dialog, "host", "eu.example.org");
        assert_eq!(value(&dialog, "port"), "8443");
        assert_eq!(dialog.note("host"), Some("Example Cloud"));
        assert_eq!(value(&dialog, "token"), "", "a reaction set a secret");
        // A filled field stays open to the next reaction, and the note goes
        // with what it described.
        type_into(&mut dialog, "host", "elsewhere.test");
        assert_eq!(value(&dialog, "port"), "8443");
        assert_eq!(dialog.note("host"), None);
    }

    #[test]
    fn the_connection_dialogue_never_overwrites_a_value_typed_by_hand() {
        let mut dialog = ConnectionDialog::open(&Reacting);
        type_into(&mut dialog, "port", "2222");
        type_into(&mut dialog, "host", "eu.example.org");
        assert_eq!(value(&dialog, "port"), "2222");
        assert_eq!(dialog.note("host"), Some("Example Cloud"));
    }

    #[test]
    fn the_connection_dialogue_takes_rewritten_input_into_its_fields() {
        let mut dialog = ConnectionDialog::open(&Reacting);
        type_into(&mut dialog, "port", "2222");
        type_into(&mut dialog, "host", "eu.example.org:9000");
        assert_eq!(value(&dialog, "host"), "eu.example.org");
        assert_eq!(value(&dialog, "port"), "9000");
        // Half-typed input is left where the cursor is, and taken when the
        // dialogue saves.
        for (field, value) in &mut dialog.values {
            if field.key == "host" {
                "eu.example.org:70".clone_into(value);
            }
        }
        dialog.edited(&Reacting, "host", false);
        assert_eq!(value(&dialog, "host"), "eu.example.org:70");
        assert_eq!(value(&dialog, "port"), "9000");
        dialog.settle(&Reacting);
        assert_eq!(value(&dialog, "host"), "eu.example.org");
        assert_eq!(value(&dialog, "port"), "70");
        // What was rewritten is the user's own input: a later fill leaves it.
        type_into(&mut dialog, "host", "us.example.org");
        assert_eq!(value(&dialog, "port"), "70");
    }

    #[test]
    fn a_saved_value_counts_as_typed_in_the_edit_dialogue() {
        let entry = model::ConnectionEntry {
            provider: "reacting".to_owned(),
            name: "work".to_owned(),
            settings: BTreeMap::from([
                ("host".to_owned(), "eu.example.org".to_owned()),
                ("port".to_owned(), "2222".to_owned()),
            ]),
        };
        let dialog = ConnectionDialog::edit(&Reacting, &entry);
        assert_eq!(value(&dialog, "port"), "2222");
        assert_eq!(dialog.note("host"), Some("Example Cloud"));
    }

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
