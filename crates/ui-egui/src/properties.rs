//! The Properties dialogue (SPEC.md *Dialogues*): the shared dialogue
//! shell, 560 px wide, with the entry's icon, name, type and size in the
//! header and two tabs under it.
//!
//! *General* is groups of rows — where the entry is, what a link points
//! at, its size, its dates, whether it is hidden — with a copy button on
//! every path. A folder's contents are counted by the application in the
//! background: the header and the *Contents* row read *Calculating…* until
//! `Event::PropertiesCounted` arrives, and closing the dialogue cancels the
//! count. For a link, *Open target folder* shows what it points at in the
//! pane the dialogue was opened from.
//!
//! *Permissions* edits the owner, the group, the mode bits and the access
//! control list of a local entry in place and applies them as one job
//! (`Command::SetPermissions`), on everything inside a folder too when
//! *Apply to enclosed items* is on. The dialogue follows the entry: after
//! a change the application sends the new state and the controls reset to
//! it. Where the platform has a dialogue of its own (Windows, macOS),
//! *System properties…* opens it.

mod acl;
mod permissions;

use app::{App, Command, Contents, PermissionRequest, PropertiesInfo, Recursion};
use egui::{Align2, FontId, Rect, Sense, pos2, vec2};
use ext::{Level, t};
use platform::{Acl, PermissionChange, Principal};
use types::{Attrs, EntryKind};

use crate::shell::Request;
use crate::theme::{self, CARD_RADIUS, ICON, TEXT_BODY, TEXT_SECONDARY};
use crate::{dialog, format, icons, widgets};

/// The dialogue's width.
const WIDTH: f32 = dialog::FORM_WIDTH;
/// The space between two groups.
const GROUP_GAP: f32 = 16.0;
/// A row's inset from the card's left edge, and a text's from its right.
const INSET: f32 = 14.0;
/// A control's inset from the card's right edge.
const INSET_CONTROL: f32 = 10.0;
/// The label column of a row.
const LABEL: f32 = 96.0;
/// A row of text.
const ROW: f32 = 34.0;
/// A row that ends in a button.
const ROW_BUTTON: f32 = 42.0;
/// A row with a second line under its title.
const ROW_DESCRIBED: f32 = 50.0;
/// The side of a button inside a row.
const ROW_ICON_BUTTON: f32 = 24.0;

/// The dialogue's state between frames.
#[derive(Debug, Default)]
pub struct PropertiesDialog {
    /// What the application sent; `None` while closed.
    info: Option<PropertiesInfo>,
    /// The controls' values.
    edit: Edit,
    /// A change was sent and its answer is awaited.
    pub applying: bool,
    /// The tab shown.
    tab: Tab,
    /// Whether the access control list is open.
    acl_open: bool,
}

/// The dialogue's two tabs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Tab {
    /// Where the entry is, its size and its dates.
    #[default]
    General,
    /// Its owner, group, mode bits and access control list.
    Permissions,
}

/// The editable state, as the controls hold it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Edit {
    owner: Option<Principal>,
    group: Option<Principal>,
    /// The name typed after *Other…* in the owner's dropdown; `None` while
    /// the dropdown lists the machine's users.
    owner_other: Option<String>,
    /// The same for the group.
    group_other: Option<String>,
    mode: u32,
    /// The mode as typed in the octal field.
    octal: String,
    acl: Option<Acl>,
    /// *Apply to enclosed items*.
    recursive: bool,
    /// The name of a new ACL entry being added.
    new_name: String,
    new_group: bool,
    new_allow: bool,
}

/// What the dialogue was asked to do in a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ask {
    /// Close the dialogue.
    Close,
    /// Put this text on the clipboard.
    Copy(String),
    /// Show the link's target in the pane, and close.
    OpenTarget,
    /// Open the platform's own properties window.
    System,
    /// Apply the pending change.
    Apply,
    /// Put the controls back.
    Revert,
}

impl PropertiesDialog {
    /// Shows `info`, or refreshes the dialogue with the entry's new state:
    /// the same entry keeps its tab and its open list.
    pub fn open(&mut self, info: PropertiesInfo) {
        let same = self.info.as_ref().is_some_and(|shown| {
            shown.pane == info.pane && shown.path == info.path && shown.entry.id == info.entry.id
        });
        if !same {
            self.tab = Tab::General;
            self.acl_open = false;
        }
        self.edit = Edit::from(&info);
        self.info = Some(info);
        self.applying = false;
    }

    /// The count of the folder at `path`, shown from `pane`, is in.
    pub fn counted(
        &mut self,
        pane: model::PaneId,
        path: &std::path::Path,
        contents: app::FolderContents,
    ) {
        if let Some(info) = &mut self.info
            && info.pane == pane
            && info.path.as_deref() == Some(path)
        {
            info.contents = Contents::Counted(contents);
        }
    }

    /// Whether the dialogue is on screen.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.info.is_some()
    }

    /// The entry shown, if any.
    #[must_use]
    pub fn info(&self) -> Option<&PropertiesInfo> {
        self.info.as_ref()
    }

    /// The change the controls stand for right now, against the entry's
    /// state; `None` when nothing changed.
    #[must_use]
    pub fn pending_change(&self) -> Option<PermissionChange> {
        self.info
            .as_ref()
            .and_then(|info| pending(info, &self.edit))
    }

    /// Draws the dialogue, when open.
    pub fn show(&mut self, ctx: &egui::Context, app: &App, requests: &mut Vec<Request>) {
        let Self {
            info: shown,
            edit,
            applying,
            tab,
            acl_open,
        } = self;
        let Some(info) = shown.as_ref() else {
            return;
        };
        if info.permissions.is_none() {
            *tab = Tab::General;
        }
        let mut asked: Option<Ask> = None;
        let modal = dialog::modal(ctx, egui::Id::new("properties"), WIDTH, |ui| {
            let glyph = icons::for_entry(&info.entry);
            if dialog::band(ui, Some(glyph), &info.entry.name, &subtitle(info)) {
                asked = Some(Ask::Close);
            }
            tabs(ui, tab, info.permissions.is_some());
            dialog::body(ui, |ui| {
                ui.spacing_mut().item_spacing.y = GROUP_GAP;
                match *tab {
                    Tab::General => general(ui, info, &mut asked),
                    Tab::Permissions => permissions::page(ui, info, edit, acl_open),
                }
            });
            match *tab {
                Tab::General => general_footer(ui, info, &mut asked),
                Tab::Permissions => {
                    let dirty = pending(info, edit).is_some();
                    permissions::footer(ui, dirty, *applying, &mut asked);
                }
            }
        });
        if modal.should_close() {
            asked = Some(Ask::Close);
        }
        let mut close = false;
        match asked {
            None => {}
            Some(Ask::Close) => close = true,
            Some(Ask::Copy(text)) => {
                if let Err(error) = app.platform().set_text(&text) {
                    requests.push(Request::Toast(Level::Error, error.to_string()));
                }
            }
            Some(Ask::System) => {
                requests.push(Request::Command(Command::SystemProperties(info.pane)));
            }
            Some(Ask::Revert) => *edit = Edit::from(info),
            Some(Ask::Apply) => {
                if let Some(change) = pending(info, edit)
                    && let Some(path) = info.path.clone()
                {
                    let folder = info.entry.kind == EntryKind::Dir;
                    let recursive = (edit.recursive && folder).then_some(Recursion {
                        files: true,
                        folders: true,
                    });
                    requests.push(Request::Command(Command::SetPermissions {
                        pane: info.pane,
                        request: PermissionRequest {
                            targets: vec![path],
                            change,
                            recursive,
                        },
                    }));
                    *applying = true;
                }
            }
            Some(Ask::OpenTarget) => {
                if let Some(target) = &info.target {
                    requests.push(Request::OpenContaining {
                        pane: info.pane,
                        entry: target.clone(),
                        new_tab: false,
                    });
                    close = true;
                }
            }
        }
        if close {
            *shown = None;
            requests.push(Request::Command(Command::PropertiesClosed));
        }
    }
}

/// The difference between the controls and the entry's state.
fn pending(info: &PropertiesInfo, edit: &Edit) -> Option<PermissionChange> {
    let current = info.permissions.as_ref()?;
    let change = PermissionChange {
        owner: edit
            .owner
            .clone()
            .filter(|o| o.id != current.owner.id && o.id != current.owner.display()),
        group: edit
            .group
            .clone()
            .filter(|g| g.id != current.group.id && g.id != current.group.display()),
        mode: current.mode.filter(|m| *m != edit.mode).map(|_| edit.mode),
        acl: edit.acl.clone().filter(|a| Some(a) != current.acl.as_ref()),
    };
    (!change.is_empty()).then_some(change)
}

impl From<&PropertiesInfo> for Edit {
    fn from(info: &PropertiesInfo) -> Self {
        let Some(current) = &info.permissions else {
            return Self::default();
        };
        Self {
            owner: Some(current.owner.clone()),
            group: Some(current.group.clone()),
            owner_other: None,
            group_other: None,
            mode: current.mode.unwrap_or(0),
            octal: format!("{:04o}", current.mode.unwrap_or(0)),
            acl: current.acl.clone(),
            recursive: false,
            new_name: String::new(),
            new_group: false,
            new_allow: true,
        }
    }
}

/// The header's secondary line: the type and the size, *Folder · 12 items,
/// 3.4 GiB*; a folder still being counted reads *Calculating…*.
fn subtitle(info: &PropertiesInfo) -> String {
    let entry = &info.entry;
    let executable = entry.attrs.contains(Attrs::EXECUTABLE);
    let kind = format::type_name(&entry.kind, &entry.name, executable);
    let size = match (&entry.kind, info.contents) {
        (EntryKind::Dir, Contents::Counted(contents)) => Some(t!(
            "properties-folder-summary",
            count = contents.items,
            size = format::size(Some(contents.bytes))
        )),
        (EntryKind::Dir, Contents::Counting) => Some(t!("properties-calculating")),
        (EntryKind::Dir, Contents::None) => None,
        _ => entry.size.map(|bytes| format::size(Some(bytes))),
    };
    match size {
        Some(size) => format!("{kind}  ·  {size}"),
        None => kind,
    }
}

/// The segmented control under the header. Without permissions to show —
/// an entry on a provider that has none — *Permissions* cannot be chosen
/// and says why.
fn tabs(ui: &mut egui::Ui, tab: &mut Tab, permissions: bool) {
    let options = [t!("properties-general"), t!("permissions")];
    let font = FontId::proportional(TEXT_BODY);
    let widest = options
        .iter()
        .map(|option| {
            ui.painter()
                .layout_no_wrap(option.clone(), font.clone(), egui::Color32::PLACEHOLDER)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let (strip, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), 12.0 + 30.0), Sense::hover());
    let rect = Rect::from_min_size(
        pos2(strip.left() + dialog::PAD, strip.top() + 12.0),
        vec2((widest + 28.0) * 2.0 + 4.0, 30.0),
    );
    let why = t!("properties-no-permissions");
    let unavailable = (!permissions).then_some((1, why.as_str()));
    let selected = usize::from(*tab == Tab::Permissions);
    let clicked = widgets::segmented_with(
        ui,
        rect,
        ui.id().with("properties-tabs"),
        &options,
        selected,
        unavailable,
    );
    match clicked {
        Some(0) => *tab = Tab::General,
        Some(1) => *tab = Tab::Permissions,
        _ => {}
    }
}

/// One row of a group on the General tab.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// A label and its value.
    Value { label: String, value: String },
    /// A label and a path, in monospace, with a copy button named `copy`.
    Path {
        label: String,
        value: String,
        copy: String,
    },
    /// A label and a switch that shows a state and takes no click.
    State { label: String, on: bool },
}

impl Row {
    fn value(label: String, value: String) -> Self {
        Self::Value { label, value }
    }
}

/// A size in words.
fn size_of(bytes: u64) -> String {
    format::size(Some(bytes))
}

/// The rows of **Location**: where the entry is; for a link what it holds
/// and what that resolves to; for a file its media type, when it has one.
fn location_rows(info: &PropertiesInfo) -> Vec<Row> {
    let entry = &info.entry;
    let mut rows = vec![Row::Path {
        label: t!("properties-location"),
        value: info.location.clone(),
        copy: t!("properties-copy-location"),
    }];
    if let EntryKind::Symlink { target, broken } = &entry.kind {
        rows.push(Row::Path {
            label: t!("properties-target"),
            value: target.to_string(),
            copy: t!("properties-copy-target"),
        });
        let resolved = if *broken {
            Some(t!("properties-missing"))
        } else {
            info.target.as_ref().map(|target| {
                let kind = format::type_name(
                    &target.kind,
                    &target.name,
                    target.attrs.contains(Attrs::EXECUTABLE),
                );
                match target.size.filter(|_| target.kind != EntryKind::Dir) {
                    Some(bytes) => format!("{kind}  ·  {}", size_of(bytes)),
                    None => kind,
                }
            })
        };
        if let Some(resolved) = resolved {
            rows.push(Row::value(t!("properties-resolves-to"), resolved));
        }
    }
    if entry.kind == EntryKind::File
        && let Some(mime) = &entry.mime
    {
        rows.push(Row::value(
            t!("properties-media-type"),
            mime.as_str().to_owned(),
        ));
    }
    rows
}

/// The rows of **Size**, or of **Link** for a symbolic link: a folder's
/// contents as far as they are counted, a file's size, and what either
/// occupies on disk; a link's own size with its place on disk beside it.
fn size_rows(info: &PropertiesInfo) -> Vec<Row> {
    let entry = &info.entry;
    let mut rows = Vec::new();
    match (&entry.kind, info.contents) {
        (EntryKind::Symlink { .. }, _) => {
            if let Some(bytes) = entry.size {
                let text = match info.on_disk {
                    Some(on_disk) => t!(
                        "properties-link-size",
                        size = size_of(bytes),
                        on_disk = size_of(on_disk)
                    ),
                    None => size_of(bytes),
                };
                rows.push(Row::value(t!("size"), text));
            }
        }
        (EntryKind::Dir, Contents::Counting) => {
            for label in [t!("properties-contents"), t!("properties-on-disk")] {
                rows.push(Row::value(label, t!("properties-calculating")));
            }
        }
        (EntryKind::Dir, Contents::Counted(contents)) => {
            let counted = t!(
                "properties-contents-counted",
                count = contents.items,
                size = size_of(contents.bytes)
            );
            rows.push(Row::value(t!("properties-contents"), counted));
            // What is inside, and the folder's own blocks.
            let on_disk = contents.on_disk + info.on_disk.unwrap_or(0);
            rows.push(Row::value(t!("properties-on-disk"), size_of(on_disk)));
        }
        (EntryKind::Dir, Contents::None) => {}
        _ => {
            if let Some(bytes) = entry.size {
                rows.push(Row::value(t!("size"), size_of(bytes)));
            }
            if let Some(on_disk) = info.on_disk {
                rows.push(Row::value(t!("properties-on-disk"), size_of(on_disk)));
            }
        }
    }
    rows
}

/// The groups of the General tab for `info`, each a heading and its rows;
/// a group without rows is left out.
fn general_groups(info: &PropertiesInfo) -> Vec<(String, Vec<Row>)> {
    let entry = &info.entry;
    let link = matches!(entry.kind, EntryKind::Symlink { .. });
    let dates = [
        (t!("properties-modified"), entry.modified),
        (t!("properties-accessed"), entry.accessed),
        (t!("properties-created"), entry.created),
    ]
    .into_iter()
    .filter_map(|(label, time)| time.map(|time| Row::value(label, format::stamp(time))))
    .collect();
    let size = if link {
        t!("properties-link")
    } else {
        t!("size")
    };
    let mut groups = vec![
        (t!("properties-location"), location_rows(info)),
        (size, size_rows(info)),
        (t!("properties-dates"), dates),
    ];
    if !link {
        groups.push((
            t!("properties-options"),
            vec![Row::State {
                label: t!("properties-hidden"),
                on: entry.attrs.contains(Attrs::HIDDEN),
            }],
        ));
    }
    groups.retain(|(_, rows)| !rows.is_empty());
    groups
}

/// The General tab: its groups, each a caps heading over a card.
fn general(ui: &mut egui::Ui, info: &PropertiesInfo, asked: &mut Option<Ask>) {
    for (heading, rows) in general_groups(info) {
        group(ui, &heading, |card| {
            for row in &rows {
                match row {
                    Row::Value { label, value } => card.value(label, value),
                    Row::Path { label, value, copy } => {
                        if card.path(label, value, copy) {
                            *asked = Some(Ask::Copy(value.clone()));
                        }
                    }
                    Row::State { label, on } => card.state(label, *on),
                }
            }
        });
    }
}

/// The General tab's footer: *Close*, and at the left *Open target
/// folder* for a link — off for a broken one — and *System properties…*
/// where the platform has a window of its own.
fn general_footer(ui: &mut egui::Ui, info: &PropertiesInfo, asked: &mut Option<Ask>) {
    let link = matches!(info.entry.kind, EntryKind::Symlink { .. });
    let reachable = info.target.is_some();
    let system = info.native && info.path.is_some();
    let chosen = std::cell::Cell::new(None);
    dialog::footer(
        ui,
        |ui| {
            if link
                && widgets::button(ui, &t!("properties-open-target-folder"), false, reachable)
                    .clicked()
                && reachable
            {
                chosen.set(Some(Ask::OpenTarget));
            }
            if system
                && widgets::button(ui, &t!("properties-system-properties"), false, true).clicked()
            {
                chosen.set(Some(Ask::System));
            }
        },
        |ui| {
            if widgets::button(ui, &t!("close"), true, true).clicked() {
                chosen.set(Some(Ask::Close));
            }
        },
    );
    if let Some(ask) = chosen.into_inner() {
        *asked = Some(ask);
    }
}

/// A group: `heading` in caps over a `bg/card` card whose rows `rows`
/// adds, separated by hairlines.
fn group(ui: &mut egui::Ui, heading: &str, rows: impl FnOnce(&mut Card<'_>)) {
    let scheme = theme::scheme(ui);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        widgets::caps_heading(ui, heading);
        ui.add_space(6.0);
        egui::Frame::new()
            .fill(scheme.bg_card)
            .stroke(egui::Stroke::new(1.0, scheme.border_default))
            .corner_radius(CARD_RADIUS)
            .inner_margin(0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                rows(&mut Card { ui, drawn: 0 });
            });
    });
}

/// A card being filled, row by row.
struct Card<'a> {
    ui: &'a mut egui::Ui,
    /// The rows drawn so far.
    drawn: usize,
}

impl Card<'_> {
    /// Takes the next row, `height` tall, under a hairline unless it is
    /// the first.
    fn take(&mut self, height: f32) -> Rect {
        let (rect, _) = self
            .ui
            .allocate_exact_size(vec2(self.ui.available_width(), height), Sense::hover());
        if self.drawn > 0 {
            let hairline = theme::scheme(self.ui).hairline();
            self.ui
                .painter()
                .hline(rect.x_range(), rect.top() + 0.5, hairline);
        }
        self.drawn += 1;
        rect
    }

    /// The row's label, in `text/secondary` in its 96 px column; returns
    /// where the value starts.
    fn label(&mut self, rect: Rect, label: &str) -> f32 {
        let scheme = theme::scheme(self.ui);
        widgets::line(
            self.ui,
            pos2(rect.left() + INSET, rect.center().y),
            label,
            FontId::proportional(TEXT_BODY),
            scheme.text_secondary,
            LABEL,
        );
        rect.left() + INSET + LABEL + 12.0
    }

    /// A row of a label and its value, the value cut with an ellipsis.
    fn value(&mut self, label: &str, value: &str) {
        let scheme = theme::scheme(self.ui);
        let rect = self.take(ROW);
        let left = self.label(rect, label);
        widgets::line(
            self.ui,
            pos2(left, rect.center().y),
            value,
            FontId::proportional(TEXT_BODY),
            scheme.text_primary,
            rect.right() - INSET - left,
        );
    }

    /// A row of a label and a path in monospace, cut with an ellipsis,
    /// with a copy button named `copy` at its right end. Returns whether
    /// the button was clicked.
    fn path(&mut self, label: &str, value: &str, copy: &str) -> bool {
        let scheme = theme::scheme(self.ui);
        let rect = self.take(ROW_BUTTON);
        let left = self.label(rect, label);
        let button = Rect::from_center_size(
            pos2(
                rect.right() - INSET_CONTROL - ROW_ICON_BUTTON / 2.0,
                rect.center().y,
            ),
            vec2(ROW_ICON_BUTTON, ROW_ICON_BUTTON),
        );
        widgets::line(
            self.ui,
            pos2(left, rect.center().y),
            value,
            FontId::monospace(TEXT_SECONDARY),
            scheme.text_primary,
            button.left() - 12.0 - left,
        );
        icon_button(self.ui, button, icons::COPY, copy).clicked()
    }

    /// A row of a label and a switch that shows `on` and takes no click:
    /// read aloud as a checkbox that cannot be changed.
    fn state(&mut self, label: &str, on: bool) {
        let scheme = theme::scheme(self.ui);
        let rect = self.take(ROW + 2.0);
        self.ui.painter().text(
            pos2(rect.left() + INSET, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(TEXT_BODY),
            scheme.text_primary,
        );
        let switch = Rect::from_center_size(
            pos2(rect.right() - INSET - 16.0, rect.center().y),
            vec2(32.0, 18.0),
        );
        let mut child = self.ui.new_child(egui::UiBuilder::new().max_rect(switch));
        widgets::switch(&mut child, on);
        let spoken = label.to_owned();
        self.ui
            .interact(rect, self.ui.id().with(("state", label)), Sense::hover())
            .widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::Checkbox, false, on, spoken.clone())
            });
    }
}

/// A 24 px button of one glyph in `rect`, `bg/subtle` under the pointer,
/// named `name` for a screen reader and as its tooltip.
pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    rect: Rect,
    glyph: &str,
    name: &str,
) -> egui::Response {
    let scheme = theme::scheme(ui);
    let response = ui.interact(rect, ui.id().with(("icon-button", name)), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            theme::metrics(ui.ctx()).corner_radius(),
            scheme.bg_subtle,
        );
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(ICON),
        scheme.icon_default,
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.to_owned()));
    response.on_hover_text(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::FolderContents;
    use model::PaneId;
    use types::{Entry, EntryId, ProviderId, VPath};

    fn entry(name: &str, kind: EntryKind, size: Option<u64>) -> Entry {
        Entry {
            id: EntryId::new(ProviderId("local"), 1),
            name: name.to_owned(),
            path: VPath::from(format!("/tmp/{name}").as_str()),
            kind,
            size,
            created: None,
            modified: None,
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        }
    }

    fn info_of(entry: Entry) -> PropertiesInfo {
        let path = std::path::PathBuf::from(format!("/tmp/{}", entry.name));
        PropertiesInfo {
            pane: PaneId(1),
            entry,
            path: Some(path),
            location: "/tmp".to_owned(),
            on_disk: Some(4096),
            target: None,
            contents: Contents::None,
            permissions: None,
            principals: platform::Principals::default(),
            native: false,
        }
    }

    fn info(mode: u32) -> PropertiesInfo {
        PropertiesInfo {
            permissions: Some(platform::FilePermissions {
                owner: Principal::named("1000", "me"),
                group: Principal::named("1000", "me"),
                mode: Some(mode),
                acl: None,
                is_dir: false,
            }),
            ..info_of(entry("f", EntryKind::File, Some(1)))
        }
    }

    /// The rows of the group titled `heading`, as label and value.
    fn rows(info: &PropertiesInfo, heading: &str) -> Vec<(String, String)> {
        general_groups(info)
            .into_iter()
            .find(|(title, _)| title == heading)
            .map_or_default(|(_, rows)| {
                rows.into_iter()
                    .map(|row| match row {
                        Row::Value { label, value } | Row::Path { label, value, .. } => {
                            (label, value)
                        }
                        Row::State { label, on } => (label, on.to_string()),
                    })
                    .collect()
            })
    }

    fn pair(label: &str, value: &str) -> (String, String) {
        (label.to_owned(), value.to_owned())
    }

    #[test]
    fn the_pending_change_is_the_difference_to_the_entry() {
        let mut dialog = PropertiesDialog::default();
        assert!(dialog.pending_change().is_none());
        dialog.open(info(0o644));
        assert!(dialog.pending_change().is_none(), "nothing edited yet");
        dialog.edit.mode = 0o600;
        let change = dialog
            .pending_change()
            .unwrap_or_else(|| panic!("a change"));
        assert_eq!(change.mode, Some(0o600));
        assert!(change.owner.is_none() && change.group.is_none() && change.acl.is_none());
        dialog.edit.owner = Some(Principal::named("0", "root"));
        let change = dialog
            .pending_change()
            .unwrap_or_else(|| panic!("a change"));
        assert_eq!(change.owner.as_ref().map(|p| p.id.as_str()), Some("0"));
        // Opening again resets the controls.
        dialog.open(info(0o600));
        assert!(dialog.pending_change().is_none());
    }

    #[test]
    fn a_folder_reads_calculating_until_its_count_is_in() {
        crate::english();
        let mut dialog = PropertiesDialog::default();
        dialog.open(PropertiesInfo {
            contents: Contents::Counting,
            ..info_of(entry("serge", EntryKind::Dir, None))
        });
        let shown = |dialog: &PropertiesDialog| {
            let info = dialog.info().unwrap_or_else(|| panic!("open"));
            (subtitle(info), rows(info, "Size"))
        };
        let (line, size) = shown(&dialog);
        assert_eq!(line, "Folder  ·  Calculating…");
        assert_eq!(
            size,
            [
                pair("Contents", "Calculating…"),
                pair("On disk", "Calculating…")
            ]
        );
        let counted = FolderContents {
            items: 12,
            bytes: 3 * 1024 * 1024,
            on_disk: 3 * 1024 * 1024 + 8192,
        };
        // A count of another folder is not this dialogue's.
        dialog.counted(PaneId(1), std::path::Path::new("/tmp/other"), counted);
        assert_eq!(shown(&dialog).0, "Folder  ·  Calculating…");
        dialog.counted(PaneId(1), std::path::Path::new("/tmp/serge"), counted);
        let (line, size) = shown(&dialog);
        assert_eq!(line, "Folder  ·  12 items, 3.0 MiB");
        assert_eq!(
            size,
            [
                pair("Contents", "12 items  ·  3.0 MiB"),
                // The folder's own 4 KiB count too.
                pair("On disk", "3.0 MiB"),
            ]
        );
        // A folder elsewhere than on this machine is not counted.
        let remote = info_of(entry("bucket", EntryKind::Dir, None));
        assert_eq!(subtitle(&remote), "Folder");
        assert_eq!(rows(&remote, "Size"), Vec::new(), "no Size group");
    }

    #[test]
    fn a_link_shows_its_target_what_it_resolves_to_and_its_own_size() {
        crate::english();
        let link = EntryKind::Symlink {
            target: VPath::from("/opt/rider/bin/rider"),
            broken: false,
        };
        let mut target = entry("rider", EntryKind::File, Some(2 * 1024 * 1024));
        target.attrs = Attrs::EXECUTABLE;
        let info = PropertiesInfo {
            target: Some(target),
            ..info_of(entry("jetbrains-rider", link, Some(56)))
        };
        assert_eq!(subtitle(&info), "Symbolic link  ·  56 B");
        assert_eq!(
            rows(&info, "Location"),
            [
                pair("Location", "/tmp"),
                pair("Target", "/opt/rider/bin/rider"),
                pair("Resolves to", "Executable file  ·  2.0 MiB"),
            ]
        );
        assert_eq!(
            rows(&info, "Link"),
            [pair("Size", "56 B (on disk 4.0 KiB)")]
        );
        let titles: Vec<String> = general_groups(&info).into_iter().map(|g| g.0).collect();
        assert_eq!(titles, ["Location", "Link"], "no Size, no Options");

        let broken = info_of(entry(
            "gone",
            EntryKind::Symlink {
                target: VPath::from("nowhere"),
                broken: true,
            },
            Some(7),
        ));
        assert_eq!(
            rows(&broken, "Location").last(),
            Some(&pair("Resolves to", "Missing"))
        );
    }

    #[test]
    fn a_file_shows_its_size_its_media_type_and_whether_it_is_hidden() {
        crate::english();
        let mut file = entry(".notes.txt", EntryKind::File, Some(2048));
        file.attrs = Attrs::HIDDEN;
        file.mime = Some(types::Mime::from("text/plain"));
        let info = info_of(file);
        assert_eq!(subtitle(&info), "TXT file  ·  2.0 KiB");
        assert_eq!(
            rows(&info, "Location"),
            [pair("Location", "/tmp"), pair("Media type", "text/plain")]
        );
        assert_eq!(
            rows(&info, "Size"),
            [pair("Size", "2.0 KiB"), pair("On disk", "4.0 KiB")]
        );
        assert_eq!(rows(&info, "Options"), [pair("Hidden", "true")]);
    }
}
