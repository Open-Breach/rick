//! The file list: the details table (also the tree view, with folders
//! expandable in place), the list grid and the icon grid. All lay out only
//! the rows in view, so a 200k-entry listing costs the same as a small one.
//! Rename, New file and New folder edit a name in the row itself.

use std::collections::BTreeMap;

use egui::{Align, Sense};
use egui_extras::{Column as TableColumn, TableBuilder};
use ext::{ActionId, Badge, CellValue, ColumnDef, Decoration, t};
use model::{Column, IconSize, SelectionChange, SortDir, Tab, ViewMode};
use types::{Entry, EntryId, EntryKind};

use crate::dnd::{self, Spring, Target, Zone};
use crate::inline::{EditKind, EditOutcome, RowEdit, editor};
use crate::keys::Intent;
use crate::shell::{PaneCx, Request};
use crate::theme::{self, COLUMN_HEADER, Scheme, TEXT_BODY, TEXT_SECONDARY};
use crate::{format, menus, portal};

/// The inset of a row's background from the pane's edges (SPEC.md *Row
/// states*).
const ROW_INSET: f32 = 6.0;
/// Where the Name column's content starts: the icon sits 6 px inside the
/// row background, which is itself `ROW_INSET` from the pane's edge.
const NAME_INSET: f32 = ROW_INSET + 6.0;

/// The columns' default widths in points; Name's default is what the
/// others leave, and the last column takes what is left.
const SIZE_WIDTH: f32 = 80.0;
const TYPE_WIDTH: f32 = 110.0;
const MODIFIED_WIDTH: f32 = 130.0;
const EXTRA_WIDTH: f32 = 90.0;
/// No column gets narrower than this (SPEC.md *File list*).
pub const COLUMN_MIN: f32 = 48.0;
/// The hit zone of the boundary between two headers.
const BOUNDARY_HIT: f32 = 6.0;

/// Indent per tree level, in points.
const TREE_INDENT: f32 = 16.0;
/// Width of the expand chevron's slot, in points.
const CHEVRON: f32 = 16.0;
/// A second click on the sole selected row this long after the first (and
/// past the double-click window) starts a rename, in seconds.
const SLOW_CLICK: f64 = 0.5;

/// The glyph in front of a name.
#[must_use]
pub fn icon(entry: &Entry) -> &'static str {
    crate::icons::for_entry(entry)
}

/// The name as the list shows it: without the extension when the setting
/// says so, for files only.
#[must_use]
pub fn display_name(entry: &Entry, show_extensions: bool) -> &str {
    if show_extensions || entry.kind == EntryKind::Dir {
        return &entry.name;
    }
    match entry.name.rfind('.') {
        Some(dot) if dot > 0 => &entry.name[..dot],
        _ => &entry.name,
    }
}

/// The name as a flattened view or search results show it: the path relative
/// to the tab's location, so `sub/inner.txt`; the plain name elsewhere.
#[must_use]
pub fn shown_name(tab: &Tab, entry: &Entry, show_extensions: bool) -> String {
    if !(tab.flatten || tab.is_searching()) {
        return display_name(entry, show_extensions).to_owned();
    }
    let base = tab.location.path.as_bytes();
    let rest = entry.path.as_bytes().strip_prefix(base).map(|rest| {
        rest.strip_prefix(b"/")
            .or_else(|| rest.strip_prefix(b"\\"))
            .unwrap_or(rest)
    });
    match rest {
        Some(rest) if !rest.is_empty() => {
            let text = String::from_utf8_lossy(rest).into_owned();
            if show_extensions || entry.kind == EntryKind::Dir {
                text
            } else {
                match text.rfind('.') {
                    Some(dot) if dot > text.rfind(['/', '\\']).map_or(0, |s| s + 1) => {
                        text[..dot].to_owned()
                    }
                    _ => text,
                }
            }
        }
        _ => display_name(entry, show_extensions).to_owned(),
    }
}

/// Whether a name exists among the tab's own entries, hidden ones included.
fn name_taken(tab: &Tab, name: &str) -> bool {
    tab.listing.entries().iter().any(|e| e.name == name)
}

/// A row's height: the density's `row/height` token, less the spacing the
/// table puts between rows, so the pitch of the rows is the token.
#[must_use]
pub fn row_height(ui: &egui::Ui) -> f32 {
    (crate::theme::metrics(ui.ctx()).row_height - ui.spacing().item_spacing.y).max(1.0)
}

/// Draws the active view of `cx`. A right click on empty space, below the
/// last row or anywhere in an empty folder, opens the context menu for the
/// folder itself with nothing selected.
pub fn show(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    // Registered first, so every row drawn after it wins the pointer.
    let area = ui.available_rect_before_wrap();
    let background = ui.interact(
        area,
        ui.id().with(("empty-space", cx.tab_id)),
        egui::Sense::click(),
    );
    if let Some(offset) = restore_scroll(cx) {
        cx.tab_ui.pending_offset = Some(offset);
    }
    // The pane's space is a drop target: into the folder it shows.
    if cx.dnd.active()
        && cx.dnd.zone(Zone {
            key: egui::Id::new(("dnd-pane", cx.tab_id)),
            rect: area,
            target: dnd::into_pane(
                cx.tab,
                Target::Folder {
                    to: cx.tab.location.clone(),
                    device: dnd::listing_device(cx.tab),
                    spring: None,
                },
            ),
        })
    {
        dnd::highlight(ui.painter(), ui.visuals(), area);
    }
    let overview = vfs_rick::Virtual::of(&cx.tab.location).filter(|v| v.is_overview());
    // Nothing to list yet, or nothing at all: the loading portal or the
    // empty-folder message take the view's place, unless a New file or New
    // folder is being named.
    let bare = overview.is_none() && cx.tab.row_count() == 0 && cx.tab_ui.edit.is_none();
    if bare && !cx.tab.listing.complete {
        loading_state(ui, cx);
    } else if bare && cx.tab.failure.is_none() {
        empty_state(ui, cx);
    } else {
        match (overview, cx.tab.view) {
            (Some(which), _) => crate::overview::show(ui, cx, which),
            (None, ViewMode::List) => {
                new_row_strip(ui, cx);
                list(ui, cx);
            }
            (None, ViewMode::Details | ViewMode::Tree) => details(ui, cx),
            (None, ViewMode::Icons(size)) => {
                new_row_strip(ui, cx);
                icons(ui, cx, size);
            }
        }
    }
    if background.secondary_clicked() && !cx.tab.selection.is_empty() {
        cx.requests.push(Request::Command(app::Command::Select {
            pane: cx.pane,
            change: SelectionChange::Clear,
        }));
    }
    if background.secondary_clicked() || background.context_menu_opened() {
        if !cx.focused {
            cx.requests.push(Request::FocusPane(cx.pane));
        }
        let action_ctx = ext::ActionCtx {
            pane: cx.pane,
            location: cx.tab.location.clone(),
            selection: Vec::new(),
            invoked_from: ext::Invocation::ContextMenu,
            other: cx
                .app
                .action_ctx(cx.pane, ext::Invocation::ContextMenu)
                .and_then(|c| c.other),
        };
        egui::Popup::context_menu(&background)
            .style(menus::menu_style)
            .show(|ui| menus::context_menu(ui, cx, &action_ctx, None));
    }
}

/// The empty folder (SPEC.md *Empty folder*): `FOLDER_OPEN` at 64 px,
/// *This folder is empty* and the hint under it; or *No entries match the
/// filter* when the filter or the type filter emptied the list.
fn empty_state(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let rect = ui.available_rect_before_wrap();
    let scheme = theme::scheme(ui);
    let centre = rect.center();
    let filtered = !cx.tab.filter.is_empty() || cx.tab.types.is_some();
    ui.painter().text(
        centre - egui::vec2(0.0, 44.0),
        egui::Align2::CENTER_CENTER,
        crate::icons::FOLDER_OPEN,
        egui::FontId::proportional(64.0),
        scheme.icon_default.gamma_multiply(0.45),
    );
    let title = if filtered {
        t!("list-no-match")
    } else {
        t!("list-empty-title")
    };
    // Labels, not painted text, so a screen reader hears the message.
    centred_label(
        ui,
        centre + egui::vec2(0.0, 14.0),
        egui::RichText::new(title)
            .size(TEXT_BODY)
            .color(scheme.text_primary),
    );
    if !filtered {
        let shortcut = cx
            .keys
            .text_for(Intent::Action(ActionId("core.new_folder")))
            .unwrap_or_default();
        centred_label(
            ui,
            centre + egui::vec2(0.0, 38.0),
            egui::RichText::new(t!("list-empty-hint", shortcut = shortcut))
                .size(TEXT_SECONDARY)
                .color(scheme.text_secondary),
        );
    }
    ui.allocate_rect(rect, Sense::hover());
}

/// A one-line label centred on `at`, a widget so that it is read aloud.
fn centred_label(ui: &mut egui::Ui, at: egui::Pos2, text: egui::RichText) {
    let galley = egui::WidgetText::from(text.clone()).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    let rect = egui::Rect::from_center_size(at, galley.size());
    ui.put(rect, egui::Label::new(text).selectable(false));
}

/// The loading state (SPEC.md *Loading*): the portal burst centred in the
/// content with *Reading folder…* and the count so far under it, until
/// the first rows arrive.
fn loading_state(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let rect = ui.available_rect_before_wrap();
    let scheme = theme::scheme(ui);
    let still = cx.app.platform().reduce_motion();
    let time = ui.input(|i| i.time);
    let centre = rect.center();
    portal::burst(
        ui.painter(),
        centre - egui::vec2(0.0, 24.0),
        time,
        scheme.accent_focus,
        scheme.icon_default,
        still,
    );
    let count = cx.tab.listing.len();
    let text = if count == 0 {
        t!("list-reading")
    } else {
        t!("list-reading-count", count = count.to_string())
    };
    centred_label(
        ui,
        centre + egui::vec2(0.0, 48.0),
        egui::RichText::new(text)
            .size(TEXT_SECONDARY)
            .color(scheme.text_secondary),
    );
    if !still {
        ui.ctx().request_repaint();
    }
    ui.allocate_rect(rect, Sense::hover());
}

/// The scroll offset to put back: the session's, once the restored tab's
/// listing is complete; or a drag spring-loaded out of this location and
/// the tab is back at it with its listing complete. Any other change of
/// location forgets it.
fn restore_scroll(cx: &mut PaneCx<'_>) -> Option<f32> {
    let changed = cx.tab_ui.last_location.as_ref() != Some(&cx.tab.location);
    if changed {
        // A restored offset belongs to the first location drawn; a move
        // away from it forgets it.
        if cx.tab_ui.last_location.is_some() {
            cx.tab_ui.restore_scroll = None;
        }
        cx.tab_ui.last_location = Some(cx.tab.location.clone());
    }
    if cx.tab_ui.restore_scroll.is_some() {
        if !cx.tab.listing.complete {
            return None;
        }
        return cx.tab_ui.restore_scroll.take();
    }
    let (location, offset) = cx.tab_ui.scroll_restore.clone()?;
    if location != cx.tab.location {
        if changed {
            cx.tab_ui.scroll_restore = None;
        }
        return None;
    }
    if !cx.tab.listing.complete {
        return None;
    }
    cx.tab_ui.scroll_restore = None;
    Some(offset)
}

/// The inline editor of a New file or New folder above the list and icon
/// grids, which have no first row to put it in.
fn new_row_strip(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let Some(edit) = cx.tab_ui.edit.as_mut() else {
        return;
    };
    let glyph = match edit.kind {
        EditKind::NewFolder => crate::icons::FOLDER,
        EditKind::NewFile => crate::icons::by_extension(&edit.text),
        EditKind::Rename(_) => return,
    };
    let height = row_height(ui);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let mut strip = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    cell(&mut strip, glyph);
    let edit_rect =
        egui::Rect::from_min_max(strip.cursor().min, rect.max).shrink2(egui::vec2(0.0, 1.0));
    let tab = cx.tab;
    let outcome = editor(&mut strip, edit, edit_rect, false, &|name| {
        name_taken(tab, name)
    });
    finish_edit(cx, outcome);
}

/// Turns an editor's outcome into the shell's requests.
fn finish_edit(cx: &mut PaneCx<'_>, outcome: EditOutcome) {
    match outcome {
        EditOutcome::Editing => {}
        EditOutcome::Cancel => cx.tab_ui.edit = None,
        EditOutcome::Commit(name) => {
            if let Some(edit) = cx.tab_ui.edit.take() {
                cx.requests.push(Request::CommitEdit {
                    pane: cx.pane,
                    kind: edit.kind,
                    name,
                });
            }
        }
    }
}

/// Registers a row as a drop zone, a folder row to move into and a file row
/// as a silent no-op, and paints its highlight.
fn row_zone(ui: &egui::Ui, cx: &mut PaneCx<'_>, entry: &Entry, rect: egui::Rect) {
    if !cx.dnd.active() {
        return;
    }
    let target = if entry.kind == EntryKind::Dir && !vfs_rick::is_trash(&cx.tab.location) {
        dnd::into_pane(
            cx.tab,
            Target::Folder {
                to: cx.tab.location_of(entry),
                device: dnd::device_of(entry.id),
                spring: Some(Spring::Pane(cx.pane)),
            },
        )
    } else {
        Target::NoOp
    };
    let zone = Zone {
        key: egui::Id::new(("dnd-row", cx.tab_id, entry.id)),
        rect,
        target,
    };
    if cx.dnd.zone(zone) {
        dnd::highlight(ui.painter(), ui.visuals(), rect);
    }
}

/// Starts a drag when a row is pulled past egui's click distance: the whole
/// selection when the row is in it, the row alone otherwise.
fn drag_start(cx: &mut PaneCx<'_>, index: usize, response: &egui::Response) {
    let right_button = if response.drag_started_by(egui::PointerButton::Secondary) {
        true
    } else if response.drag_started_by(egui::PointerButton::Primary) {
        false
    } else {
        return;
    };
    let Some(entry) = cx.tab.row(index) else {
        return;
    };
    let id = entry.id;
    let items: Vec<types::EntryId> = if cx.tab.selection.contains(&id) {
        cx.tab.selected().map(|e| e.id).collect()
    } else {
        cx.requests.push(Request::Command(app::Command::Select {
            pane: cx.pane,
            change: SelectionChange::Set(vec![id]),
        }));
        cx.tab_ui.cursor = Some(id);
        cx.tab_ui.anchor = Some(id);
        vec![id]
    };
    cx.requests.push(Request::StartDrag {
        pane: cx.pane,
        tab: cx.tab_id,
        items,
        right_button,
    });
}

/// Whether a response is worth a second look after the layout: a click of
/// any kind, a menu, or the start of a drag.
fn interacted(response: &egui::Response) -> bool {
    response.clicked()
        || response.double_clicked()
        || response.secondary_clicked()
        || response.middle_clicked()
        || response.context_menu_opened()
        || response.drag_started()
}

/// Cell width and thumbnail edge, in points, of an icon size.
#[must_use]
pub fn icon_cell(size: IconSize) -> (f32, f32) {
    match size {
        IconSize::Small => (80.0, 48.0),
        IconSize::Medium => (120.0, 80.0),
        IconSize::Large => (176.0, 128.0),
        IconSize::ExtraLarge => (264.0, 208.0),
    }
}

/// The icon view: a grid of thumbnails, or file-type icons until a
/// thumbnail arrives or when no previewer draws the file, with the name
/// below each.
pub fn icons(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, size: IconSize) {
    let (cell_w, thumb) = icon_cell(size);
    let spacing = ui.spacing().item_spacing;
    let label_h = 2.0 * ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    let cell_h = thumb + label_h + 12.0;
    let columns = ((ui.available_width() + spacing.x) / (cell_w + spacing.x))
        .floor()
        .max(1.0) as usize;
    let count = cx.tab.row_count();
    let rows = count.div_ceil(columns);
    cx.tab_ui.visible_rows =
        ((ui.available_height() / (cell_h + spacing.y)) as usize).max(1) * columns;
    let show_extensions = cx.app.model().settings.show_extensions;
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("icons", cx.tab_id))
        .auto_shrink([false, false]);
    if let Some(index) = cx.tab_ui.scroll_to.take() {
        let row = index / columns;
        area = area.vertical_scroll_offset(row as f32 * (cell_h + spacing.y));
    }
    if let Some(offset) = cx.tab_ui.pending_offset.take() {
        area = area.vertical_scroll_offset(offset);
    }
    let mut clicked: Vec<(usize, egui::Response)> = Vec::new();
    let mut drawn: Vec<(usize, egui::Rect)> = Vec::new();
    let out = area.show_rows(ui, cell_h, rows, |ui, range| {
        for row in range {
            ui.horizontal(|ui| {
                for column in 0..columns {
                    let index = row * columns + column;
                    let Some(entry) = cx.tab.row(index) else {
                        break;
                    };
                    let response =
                        icon_cell_ui(ui, cx, entry, cell_w, cell_h, thumb, show_extensions);
                    drawn.push((index, response.rect));
                    if interacted(&response) {
                        clicked.push((index, response));
                    }
                }
            });
        }
    });
    cx.tab_ui.scroll_offset = out.state.offset.y;
    zones_for(ui, cx, &drawn);
    for (index, response) in clicked {
        drag_start(cx, index, &response);
        interact(ui, cx, index, &response);
    }
}

/// Registers the drawn rows as drop zones.
fn zones_for(ui: &egui::Ui, cx: &mut PaneCx<'_>, drawn: &[(usize, egui::Rect)]) {
    if !cx.dnd.active() {
        return;
    }
    for (index, rect) in drawn {
        if let Some(entry) = cx.tab.row(*index).cloned() {
            row_zone(ui, cx, &entry, *rect);
        }
    }
}

/// One cell of the icon view.
#[allow(clippy::too_many_lines)]
fn icon_cell_ui(
    ui: &mut egui::Ui,
    cx: &mut PaneCx<'_>,
    entry: &Entry,
    cell_w: f32,
    cell_h: f32,
    thumb: f32,
    show_extensions: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(cell_w, cell_h), egui::Sense::click_and_drag());
    let name = display_name(entry, show_extensions).to_owned();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let selected = cx.tab.selection.contains(&entry.id);
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    if selected {
        // The focused pane's selection is blue, the other pane's grey
        // (SPEC.md *Row states*).
        let fill = if cx.focused {
            scheme.bg_selected_soft
        } else {
            scheme.bg_selected_inactive
        };
        ui.painter().rect_filled(rect, radius, fill);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, radius, scheme.bg_subtle);
    }
    if cx.focused && cx.tab_ui.cursor == Some(entry.id) {
        ui.painter()
            .rect_stroke(rect, radius, scheme.focus_ring(), egui::StrokeKind::Inside);
    }
    let image_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + 6.0 + thumb / 2.0),
        egui::vec2(thumb, thumb),
    );
    // Pixels are points here: thumbnails are rendered at logical size.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let px = (thumb * ui.ctx().pixels_per_point()).round().max(1.0) as u32;
    let wants_thumbnail = entry.kind == EntryKind::File;
    let drawn = match cx.thumbnails.get(entry.id, px) {
        crate::thumbs::Thumb::Ready(texture) => {
            let size = texture.size_vec2();
            let scale = (thumb / size.x)
                .min(thumb / size.y)
                .min(1.0_f32.max(thumb / size.x.max(size.y)));
            let fitted = egui::Rect::from_center_size(image_rect.center(), size * scale);
            egui::Image::new((texture.id(), fitted.size())).paint_at(ui, fitted);
            true
        }
        crate::thumbs::Thumb::Missing if wants_thumbnail => {
            cx.thumbnails.requested(entry.id, px);
            cx.requests.push(Request::Command(app::Command::Thumbnail {
                pane: cx.pane,
                id: entry.id,
                size: px,
            }));
            false
        }
        _ => false,
    };
    if !drawn {
        let color = if selected && cx.focused {
            scheme.text_on_selected
        } else {
            scheme.text_primary
        };
        ui.painter().text(
            image_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon(entry),
            egui::FontId::proportional(thumb * 0.6),
            color,
        );
    }
    let decoration = cx.tab.decorations.get(&entry.id);
    if let Some(badge) = decoration.and_then(|d| d.badge) {
        ui.painter().text(
            image_rect.right_bottom(),
            egui::Align2::RIGHT_BOTTOM,
            badge.glyph,
            egui::FontId::proportional(14.0),
            color(badge.color),
        );
    }
    let text_color = decoration
        .and_then(|d| d.tint)
        .map_or_else(|| ui.visuals().text_color(), color);
    if cx
        .tab_ui
        .edit
        .as_ref()
        .is_some_and(|e| e.kind == EditKind::Rename(entry.id))
    {
        let edit_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 4.0, image_rect.bottom() + 4.0),
            egui::pos2(rect.right() - 4.0, rect.bottom() - 4.0),
        );
        let tab = cx.tab;
        let outcome = cx
            .tab_ui
            .edit
            .as_mut()
            .map(|edit| editor(ui, edit, edit_rect, true, &|name| name_taken(tab, name)));
        if let Some(outcome) = outcome {
            finish_edit(cx, outcome);
        }
        return response;
    }
    let mut job = egui::text::LayoutJob::simple(
        name,
        egui::TextStyle::Body.resolve(ui.style()),
        text_color,
        cell_w - 8.0,
    );
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    job.halign = egui::Align::Center;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().galley(
        egui::pos2(rect.center().x, image_rect.bottom() + 4.0),
        galley,
        text_color,
    );
    response
}

/// The details table: icon and name, type, date modified, size. In Tree
/// view the same table shows folders expandable in place, their children
/// indented under them.
// One table, header to body.
#[allow(clippy::too_many_lines)]
pub fn details(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let settings = &cx.app.model().settings;
    let height = row_height(ui);
    let count = cx.tab.row_count();
    // A New file or New folder in progress is the first row.
    let new_row = usize::from(
        cx.tab_ui
            .edit
            .as_ref()
            .is_some_and(|e| matches!(e.kind, EditKind::NewFile | EditKind::NewFolder)),
    );
    cx.tab_ui.visible_rows =
        ((ui.available_height() / (height + ui.spacing().item_spacing.y)) as usize).max(1);
    // egui's labels select text by default, and a text-selectable label takes
    // the click before the row under it sees it. A click on a name selects
    // the row, so the cells' labels only hover.
    ui.style_mut().interaction.selectable_labels = false;

    let extra = extra_columns(cx);
    // No grid lines in the body: the table paints its column boundaries
    // with the non-interactive stroke, which goes; the resize handles still
    // show when hovered or dragged, and the header draws its own separators.
    // The rows paint their own backgrounds (inset, rounded), so the table's
    // hover and selection fills are off.
    ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
    ui.visuals_mut().widgets.hovered.bg_fill = egui::Color32::TRANSPARENT;
    ui.visuals_mut().widgets.hovered.weak_bg_fill = egui::Color32::TRANSPARENT;
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    // The header band: `bg/subtle` between two hairlines, the width of the
    // list (SPEC.md *File list*).
    let outer = ui.available_rect_before_wrap();
    let band = egui::Rect::from_min_size(outer.min, egui::vec2(outer.width(), COLUMN_HEADER));
    cx.tab_ui.header_band = Some(band);
    ui.painter().rect_filled(band, 0.0, scheme.bg_subtle);
    for y in [band.top() + 0.5, band.bottom() - 0.5] {
        ui.painter().hline(band.x_range(), y, scheme.hairline());
    }
    let row_x = egui::Rangef::new(outer.left() + ROW_INSET, outer.right() - ROW_INSET);
    let clip = ui.clip_rect();
    // The columns' widths are the tab's (SPEC.md *File list*): the ones the
    // user gave, or the defaults, Name's being what the others leave; the
    // last column takes what is left. The table lays them out as given and
    // the header's boundaries below do the resizing.
    let spacing_x = ui.spacing().item_spacing.x;
    let ids = column_ids(&extra);
    let widths = column_widths(cx.tab, &ids, outer.width(), spacing_x);
    let mut builder = TableBuilder::new(ui)
        .id_salt(("details", cx.tab_id))
        .striped(false)
        .resizable(false)
        .sense(Sense::click_and_drag())
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(Align::Center));
    let last = widths.len() - 1;
    for (i, width) in widths.iter().enumerate() {
        builder = builder.column(if i == last {
            TableColumn::remainder().at_least(COLUMN_MIN).clip(true)
        } else {
            TableColumn::exact(*width).clip(true)
        });
    }
    if let Some(row) = cx.tab_ui.scroll_to.take() {
        builder = builder.scroll_to_row(row + new_row, Some(Align::Center));
    }
    if let Some(offset) = cx.tab_ui.pending_offset.take() {
        builder = builder.vertical_scroll_offset(offset);
    }

    let sort = cx.tab.sort;
    let mut sort_request = None;
    // The right edge of every header cell, the cell's own rectangle: the
    // boundaries between the columns, handled after the table.
    let mut edges: Vec<f32> = Vec::with_capacity(widths.len());
    let table = builder.header(COLUMN_HEADER, |mut row| {
        for (label, column) in [
            (t!("list-column-name"), Column::Name),
            (t!("list-column-size"), Column::Size),
            (t!("list-column-type"), Column::Type),
            (t!("list-column-modified"), Column::Modified),
        ] {
            row.col(|ui| {
                edges.push(ui.max_rect().right());
                if column == Column::Name {
                    ui.add_space(NAME_INSET);
                }
                if column == Column::Size {
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if sort_header(ui, &label, column, sort) {
                            sort_request = Some(column);
                        }
                    });
                } else if sort_header(ui, &label, column, sort) {
                    sort_request = Some(column);
                }
            });
        }
        for def in &extra {
            row.col(|ui| {
                edges.push(ui.max_rect().right());
                ui.label(
                    egui::RichText::new(def.label.get())
                        .size(TEXT_SECONDARY)
                        .color(theme::scheme(ui).text_secondary),
                );
            });
        }
    });
    let show_extensions = settings.show_extensions;
    let zebra = settings.zebra_stripes;
    let now = cx.now;
    let tab = cx.tab;
    let is_tree = tab.view == ViewMode::Tree;
    let focused = cx.focused;
    let cursor = cx.tab_ui.cursor;
    let edit = &mut cx.tab_ui.edit;
    let mut clicked: Vec<(usize, egui::Response)> = Vec::new();
    let mut drawn: Vec<(usize, egui::Rect)> = Vec::new();
    let mut toggled: Vec<EntryId> = Vec::new();
    let mut outcome: Option<EditOutcome> = None;
    let out = table.body(|body| {
        body.rows(height, count + new_row, |mut row| {
            let mut index = row.index();
            if new_row == 1 {
                if index == 0 {
                    if let Some(edit) = edit.as_mut() {
                        outcome = Some(new_row_ui(&mut row, tab, edit, &extra));
                    }
                    return;
                }
                index -= 1;
            }
            let Some(entry) = tab.row(index) else {
                return;
            };
            let depth = tab.depth(index);
            let output = details_row(
                &mut row,
                tab,
                edit,
                entry,
                &extra,
                RowLook {
                    depth,
                    is_tree,
                    show_extensions,
                    now,
                    focused,
                    cursor,
                    zebra: zebra && index % 2 == 1,
                    row_x,
                    clip,
                    scheme,
                    radius,
                },
            );
            if output.toggle {
                toggled.push(entry.id);
            }
            if output.edit.is_some() {
                outcome = output.edit;
            }
            let response = row.response();
            drawn.push((index, response.rect));
            if interacted(&response) {
                clicked.push((index, response));
            }
        });
    });
    cx.tab_ui.scroll_offset = out.state.offset.y;
    column_boundaries(ui, cx, band, &edges, &ids, &widths, &extra);
    if let Some(outcome) = outcome {
        finish_edit(cx, outcome);
    }
    for id in toggled {
        let expanded = cx.tab.tree.expanded.contains(&id);
        cx.requests.push(Request::Command(if expanded {
            app::Command::TreeCollapse { pane: cx.pane, id }
        } else {
            app::Command::TreeExpand { pane: cx.pane, id }
        }));
    }
    zones_for(ui, cx, &drawn);
    if let Some(column) = sort_request {
        let dir = if sort.by == column {
            match sort.dir {
                SortDir::Ascending => SortDir::Descending,
                SortDir::Descending => SortDir::Ascending,
            }
        } else {
            SortDir::Ascending
        };
        cx.requests.push(Request::Command(app::Command::Sort {
            pane: cx.pane,
            by: column,
            dir,
        }));
    }
    for (index, response) in clicked {
        drag_start(cx, index, &response);
        interact(ui, cx, index, &response);
    }
}

/// How a details row is drawn.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_excessive_bools)]
struct RowLook {
    /// Indent level in Tree view.
    depth: u16,
    /// Whether folders get a chevron.
    is_tree: bool,
    show_extensions: bool,
    now: std::time::SystemTime,
    /// Whether the pane has the focus: its selection is blue, another
    /// pane's grey.
    focused: bool,
    /// The entry with the keyboard cursor, ringed when the pane is focused.
    cursor: Option<EntryId>,
    /// Whether this row is a tinted zebra stripe.
    zebra: bool,
    /// The x range of a row's background: the list inset from the pane's
    /// edges.
    row_x: egui::Rangef,
    /// The list's clip rectangle, so a row's background stops where the
    /// list does.
    clip: egui::Rect,
    scheme: &'static Scheme,
    radius: egui::CornerRadius,
}

impl RowLook {
    /// Paints the row's background behind `cell`, the first cell of the
    /// row, across the whole list width (SPEC.md *Row states*): zebra
    /// tint, hover `bg/subtle`, the selection in `bg/selected-soft` or
    /// `bg/selected-inactive`, and the 2 px `accent/focus` ring of the
    /// keyboard cursor. Returns the text colour the row's cells take, when
    /// the selection changes it.
    fn paint_background(
        &self,
        ui: &egui::Ui,
        selected: bool,
        id: EntryId,
    ) -> Option<egui::Color32> {
        let rect = egui::Rect::from_x_y_ranges(self.row_x, ui.max_rect().y_range());
        // The cell's painter is clipped to the cell; the background spans
        // the row, so the clip is set outright.
        let mut painter = ui.painter().clone();
        painter.set_clip_rect(rect.expand(1.0).intersect(self.clip));
        let hovered = ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|p| rect.contains(p) && self.clip.contains(p));
        let mut text = None;
        if selected {
            if self.focused {
                painter.rect_filled(rect, self.radius, self.scheme.bg_selected_soft);
                text = Some(self.scheme.text_on_selected);
            } else {
                painter.rect_filled(rect, self.radius, self.scheme.bg_selected_inactive);
            }
        } else if hovered {
            painter.rect_filled(rect, self.radius, self.scheme.bg_subtle);
        } else if self.zebra {
            painter.rect_filled(rect, self.radius, self.scheme.bg_subtle.gamma_multiply(0.5));
        }
        if self.focused && self.cursor == Some(id) {
            painter.rect_stroke(
                rect,
                self.radius,
                self.scheme.focus_ring(),
                egui::StrokeKind::Inside,
            );
        }
        text
    }

    /// The colour of a secondary column's text: `text/secondary`, or the
    /// selection's text colour on a selected row of the focused pane.
    fn secondary(&self, selection_text: Option<egui::Color32>) -> egui::Color32 {
        selection_text.unwrap_or(self.scheme.text_secondary)
    }
}

/// What a details row produced.
#[derive(Debug, Default)]
struct RowOutput {
    /// The chevron was clicked: expand or collapse the folder.
    toggle: bool,
    /// The row's inline editor finished a frame.
    edit: Option<EditOutcome>,
}

/// The first row while a New file or New folder is being named.
fn new_row_ui(
    row: &mut egui_extras::TableRow<'_, '_>,
    tab: &Tab,
    edit: &mut RowEdit,
    extra: &[ColumnDef],
) -> EditOutcome {
    let mut outcome = EditOutcome::Editing;
    row.col(|ui| {
        ui.add_space(NAME_INSET);
        let glyph = match edit.kind {
            EditKind::NewFolder => crate::icons::FOLDER,
            _ => crate::icons::by_extension(&edit.text),
        };
        cell(ui, glyph);
        let rect = egui::Rect::from_min_max(ui.cursor().min, ui.max_rect().max)
            .shrink2(egui::vec2(0.0, 1.0));
        outcome = editor(ui, edit, rect, false, &|name| name_taken(tab, name));
    });
    row.col(|ui| {
        cell(
            ui,
            egui::RichText::new(match edit.kind {
                EditKind::NewFolder => "Folder",
                _ => "New file",
            })
            .weak(),
        );
    });
    row.col(|_ui| {});
    row.col(|_ui| {});
    for _ in extra {
        row.col(|_ui| {});
    }
    outcome
}

/// One row of the details table: chevron and indent in Tree view, icon,
/// badge, name (or its editor), type, date, size and the decorators'
/// columns.
fn details_row(
    row: &mut egui_extras::TableRow<'_, '_>,
    tab: &Tab,
    edit: &mut Option<RowEdit>,
    entry: &Entry,
    extra: &[ColumnDef],
    look: RowLook,
) -> RowOutput {
    let mut output = RowOutput::default();
    let selected = tab.selection.contains(&entry.id);
    let decoration = tab.decorations.get(&entry.id);
    let mut selection_text: Option<egui::Color32> = None;
    row.col(|ui| {
        selection_text = look.paint_background(ui, selected, entry.id);
        if let Some(colour) = selection_text {
            ui.visuals_mut().override_text_color = Some(colour);
        }
        ui.add_space(NAME_INSET);
        if look.is_tree {
            ui.add_space(f32::from(look.depth) * TREE_INDENT);
            if entry.kind == EntryKind::Dir {
                let expanded = tab.tree.expanded.contains(&entry.id);
                let glyph = if expanded {
                    crate::icons::TREE_OPEN
                } else {
                    crate::icons::TREE_CLOSED
                };
                let chevron = ui.add_sized(
                    [CHEVRON, ui.available_height()],
                    egui::Button::new(egui::RichText::new(glyph).weak()).frame(false),
                );
                if chevron.clicked() {
                    output.toggle = true;
                }
            } else {
                ui.add_space(CHEVRON + ui.spacing().item_spacing.x);
            }
        }
        cell(ui, icon(entry));
        if let Some(badge) = decoration.and_then(|d| d.badge) {
            cell(ui, badge_text(badge));
        }
        if let Some(active) = edit
            .as_mut()
            .filter(|e| e.kind == EditKind::Rename(entry.id))
        {
            let rect = egui::Rect::from_min_max(ui.cursor().min, ui.max_rect().max)
                .shrink2(egui::vec2(0.0, 1.0));
            output.edit = Some(editor(ui, active, rect, true, &|name| {
                name != entry.name && name_taken(tab, name)
            }));
            return;
        }
        let mut name = egui::RichText::new(shown_name(tab, entry, look.show_extensions));
        if let Some(tint) = decoration.and_then(|d| d.tint) {
            name = name.color(color(tint));
        }
        cell(ui, name);
        if look.is_tree && tab.tree.is_loading(entry.id) {
            cell(ui, egui::RichText::new("…").weak());
        }
    });
    // Size, Type and Modified in `text/secondary` (SPEC.md *File list*); a
    // folder's size is an em dash until one is asked for.
    let secondary = look.secondary(selection_text);
    row.col(|ui| {
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let text = if entry.kind == EntryKind::Dir {
                "\u{2014}".to_owned()
            } else {
                format::size(entry.size)
            };
            cell(ui, egui::RichText::new(text).color(secondary));
        });
    });
    row.col(|ui| {
        cell(
            ui,
            egui::RichText::new(model::type_of(entry)).color(secondary),
        );
    });
    row.col(|ui| {
        let (brief, full) = format::date(entry.modified, look.now);
        cell(ui, egui::RichText::new(brief).color(secondary)).on_hover_text(full);
    });
    for def in extra {
        row.col(|ui| {
            if let Some(colour) = selection_text {
                ui.visuals_mut().override_text_color = Some(colour);
            }
            let value = decoration
                .and_then(|d| d.cells.iter().find(|(id, _)| *id == def.id).map(|(_, v)| v));
            match value {
                Some(CellValue::Text(text)) => {
                    cell(ui, text);
                }
                Some(CellValue::Number(n)) => {
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        cell(ui, n.to_string());
                    });
                }
                Some(CellValue::Pending) => {
                    cell(ui, egui::RichText::new("…").weak());
                }
                None => {}
            }
        });
    }
    output
}

/// A label in a row that leaves clicks to the row, so a click on the text
/// selects and a right-click opens the menu.
fn cell(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add(egui::Label::new(text).selectable(false))
}

/// The columns the decorators that apply to the tab's location add.
fn extra_columns(cx: &PaneCx<'_>) -> Vec<ColumnDef> {
    cx.app
        .registry()
        .decorators()
        .iter()
        .filter(|d| d.applies_to(&cx.tab.location))
        .flat_map(|d| d.columns().iter().copied())
        .collect()
}

/// An `ext` colour as egui sees it.
#[must_use]
pub fn color(color: ext::Color) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(color.r, color.g, color.b, color.a)
}

/// A badge as a coloured, bold glyph.
#[must_use]
pub fn badge_text(badge: Badge) -> egui::RichText {
    egui::RichText::new(badge.glyph)
        .color(color(badge.color))
        .strong()
}

/// The id of a column in the tab's width map: the four of the list, then
/// the decorators' columns by their own ids.
fn column_key(column: Column) -> &'static str {
    match column {
        Column::Name => "name",
        Column::Size => "size",
        Column::Type => "type",
        Column::Modified => "modified",
        Column::Created => "created",
        Column::Accessed => "accessed",
        Column::Items => "items",
    }
}

/// The ids of the details list's columns in display order.
fn column_ids(extra: &[ColumnDef]) -> Vec<&'static str> {
    let mut ids = vec![
        column_key(Column::Name),
        column_key(Column::Size),
        column_key(Column::Type),
        column_key(Column::Modified),
    ];
    ids.extend(extra.iter().map(|def| def.id.0));
    ids
}

/// The default width of a column: Name's is what the others leave of
/// `available`, never under [`COLUMN_MIN`].
fn default_width(id: &str, ids: &[&str], available: f32, spacing: f32) -> f32 {
    match id {
        "name" => {
            let others: f32 = ids
                .iter()
                .filter(|other| **other != "name")
                .map(|other| default_width(other, ids, available, spacing))
                .sum();
            (available - others - ids.len() as f32 * spacing).max(COLUMN_MIN)
        }
        "size" => SIZE_WIDTH,
        "type" => TYPE_WIDTH,
        "modified" => MODIFIED_WIDTH,
        _ => EXTRA_WIDTH,
    }
}

/// The widths the table lays the columns out with: the tab's where the
/// user set one, the default otherwise, none under [`COLUMN_MIN`].
fn column_widths(tab: &Tab, ids: &[&str], available: f32, spacing: f32) -> Vec<f32> {
    ids.iter()
        .map(|id| {
            tab.columns
                .get(*id)
                .copied()
                .unwrap_or_else(|| default_width(id, ids, available, spacing))
                .max(COLUMN_MIN)
        })
        .collect()
}

/// The boundaries between the columns (SPEC.md *File list*): no permanent
/// line; a hit zone about 6 px wide over the header band that, hovered,
/// shows a 1 px `border/strong` divider across the band and the
/// column-resize cursor. Dragging it resizes the column before it, a
/// double-click fits that column to its content, and no column gets
/// narrower than [`COLUMN_MIN`]. A change goes to the model as the tab's
/// widths, every column's, so the last one takes what is left from then
/// on. `edges` are the header cells' right edges; the last, the table's
/// edge, has no boundary.
#[allow(clippy::too_many_arguments)]
fn column_boundaries(
    ui: &mut egui::Ui,
    cx: &mut PaneCx<'_>,
    band: egui::Rect,
    edges: &[f32],
    ids: &[&str],
    widths: &[f32],
    extra: &[ColumnDef],
) {
    let half_gap = ui.spacing().item_spacing.x / 2.0;
    let scheme = theme::scheme(ui);
    cx.tab_ui.column_boundaries.clear();
    let mut changed: Option<BTreeMap<String, f32>> = None;
    for (i, right) in edges.iter().enumerate().take(edges.len().saturating_sub(1)) {
        let x = right + half_gap;
        cx.tab_ui.column_boundaries.push(x);
        let hit = egui::Rect::from_center_size(
            egui::pos2(x, band.center().y),
            egui::vec2(BOUNDARY_HIT, band.height()),
        );
        let response = ui.interact(
            hit,
            egui::Id::new(("column-boundary", cx.tab_id, i)),
            Sense::click_and_drag(),
        );
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
            ui.painter().vline(
                x.round() + 0.5,
                band.y_range(),
                egui::Stroke::new(1.0, scheme.border_strong),
            );
        }
        let new_width = if response.double_clicked() {
            Some(fit_width(ui, cx, i, ids[i], extra))
        } else if response.dragged() {
            Some(widths[i] + response.drag_delta().x)
        } else {
            None
        };
        if let Some(width) = new_width {
            let columns = changed.get_or_insert_with(|| {
                ids.iter()
                    .zip(widths)
                    .take(edges.len().saturating_sub(1))
                    .map(|(id, w)| ((*id).to_owned(), *w))
                    .collect()
            });
            columns.insert(ids[i].to_owned(), width.max(COLUMN_MIN));
        }
    }
    if let Some(columns) = changed {
        cx.requests
            .push(Request::Command(app::Command::ColumnWidths {
                tab: cx.tab_id,
                columns,
            }));
    }
}

/// The width that fits column `index` to its content: the widest of its
/// title and the cells of the rows in view, with the cells' padding, the
/// Name column's icon and inset included.
fn fit_width(ui: &egui::Ui, cx: &PaneCx<'_>, index: usize, id: &str, extra: &[ColumnDef]) -> f32 {
    let body = egui::TextStyle::Body.resolve(ui.style());
    let small = egui::FontId::proportional(TEXT_SECONDARY);
    let spacing = ui.spacing().item_spacing.x;
    let pad = 2.0 * ui.spacing().button_padding.x;
    let measure = |text: &str, font: &egui::FontId| {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE)
                .size()
                .x
        })
    };
    let title = match index {
        0 => t!("list-column-name"),
        1 => t!("list-column-size"),
        2 => t!("list-column-type"),
        3 => t!("list-column-modified"),
        _ => extra
            .get(index - 4)
            .map(|d| d.label.get())
            .unwrap_or_default(),
    };
    let mut widest =
        measure(&title, &small) + pad + spacing + measure(crate::icons::SORT_ASC, &small);
    let show_extensions = cx.app.model().settings.show_extensions;
    let first = (cx.tab_ui.scroll_offset / row_height(ui).max(1.0)) as usize;
    for entry in cx
        .tab
        .rows()
        .skip(first)
        .take(cx.tab_ui.visible_rows.max(1))
    {
        let width = match id {
            "name" => {
                NAME_INSET
                    + measure(icon(entry), &body)
                    + spacing
                    + measure(&shown_name(cx.tab, entry, show_extensions), &body)
            }
            "size" => measure(&format::size(entry.size), &body),
            "type" => measure(&model::type_of(entry), &body),
            "modified" => measure(&format::date(entry.modified, cx.now).0, &body),
            _ => cx
                .tab
                .decorations
                .get(&entry.id)
                .and_then(|d| d.cells.iter().find(|(c, _)| c.0 == id))
                .map_or(0.0, |(_, value)| match value {
                    CellValue::Text(text) => measure(text, &body),
                    CellValue::Number(n) => measure(&n.to_string(), &body),
                    CellValue::Pending => 0.0,
                }),
        };
        widest = widest.max(width + pad);
    }
    widest.max(COLUMN_MIN)
}

/// A column title in 12 px `text/secondary` with the sort glyph after the
/// sorted column's; `true` when clicked.
fn sort_header(ui: &mut egui::Ui, label: &str, column: Column, sort: model::Sort) -> bool {
    let marker = if sort.by == column {
        match sort.dir {
            SortDir::Ascending => crate::icons::SORT_ASC,
            SortDir::Descending => crate::icons::SORT_DESC,
        }
    } else {
        ""
    };
    let text = egui::RichText::new(format!("{label} {marker}").trim_end().to_owned())
        .size(TEXT_SECONDARY)
        .color(theme::scheme(ui).text_secondary);
    ui.add(egui::Button::new(text).frame(false)).clicked()
}

/// The list grid: icon and name only, in as many columns as fit.
pub fn list(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let settings = &cx.app.model().settings;
    let height = row_height(ui);
    let cell_width = 220.0;
    let columns = ((ui.available_width() / cell_width).floor() as usize).max(1);
    let count = cx.tab.row_count();
    let rows = count.div_ceil(columns);
    cx.tab_ui.visible_rows =
        ((ui.available_height() / (height + ui.spacing().item_spacing.y)) as usize).max(1)
            * columns;
    let show_extensions = settings.show_extensions;
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("list", cx.tab_id))
        .auto_shrink([false, false]);
    if let Some(index) = cx.tab_ui.scroll_to.take() {
        let row = index / columns;
        area = area.vertical_scroll_offset(row as f32 * (height + ui.spacing().item_spacing.y));
    }
    if let Some(offset) = cx.tab_ui.pending_offset.take() {
        area = area.vertical_scroll_offset(offset);
    }
    let mut clicked: Vec<(usize, egui::Response)> = Vec::new();
    let mut drawn: Vec<(usize, egui::Rect)> = Vec::new();
    let mut edit_outcome: Option<EditOutcome> = None;
    // A pane without the focus shows its selection grey (SPEC.md *Row
    // states*).
    if !cx.focused {
        let scheme = theme::scheme(ui);
        ui.visuals_mut().selection.bg_fill = scheme.bg_selected_inactive;
        ui.visuals_mut().selection.stroke.color = scheme.text_primary;
    }
    let out = area.show_rows(ui, height, rows, |ui, range| {
        for row in range {
            ui.horizontal(|ui| {
                for column in 0..columns {
                    let index = row * columns + column;
                    let Some(entry) = cx.tab.row(index) else {
                        break;
                    };
                    let selected = cx.tab.selection.contains(&entry.id);
                    let decoration = cx.tab.decorations.get(&entry.id);
                    let text = cell_text(
                        ui,
                        icon(entry),
                        &shown_name(cx.tab, entry, show_extensions),
                        decoration,
                    );
                    let response = ui.add_sized(
                        [cell_width - ui.spacing().item_spacing.x, height],
                        egui::Button::selectable(selected, text).sense(Sense::click_and_drag()),
                    );
                    if cx
                        .tab_ui
                        .edit
                        .as_ref()
                        .is_some_and(|e| e.kind == EditKind::Rename(entry.id))
                    {
                        let tab = cx.tab;
                        let rect = response.rect.shrink2(egui::vec2(2.0, 1.0));
                        let outcome = cx.tab_ui.edit.as_mut().map(|edit| {
                            editor(ui, edit, rect, true, &|name| name_taken(tab, name))
                        });
                        edit_outcome = outcome;
                    }
                    drawn.push((index, response.rect));
                    if interacted(&response) {
                        clicked.push((index, response));
                    }
                }
            });
        }
    });
    cx.tab_ui.scroll_offset = out.state.offset.y;
    if let Some(outcome) = edit_outcome {
        finish_edit(cx, outcome);
    }
    zones_for(ui, cx, &drawn);
    for (index, response) in clicked {
        drag_start(cx, index, &response);
        interact(ui, cx, index, &response);
    }
}

/// Icon, badge and name of a list cell, the badge in its colour and the
/// name in its tint.
fn cell_text(
    ui: &egui::Ui,
    icon: &str,
    name: &str,
    decoration: Option<&Decoration>,
) -> egui::WidgetText {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let plain = ui.visuals().text_color();
    let mut job = egui::text::LayoutJob::default();
    job.append(icon, 0.0, egui::TextFormat::simple(font.clone(), plain));
    if let Some(badge) = decoration.and_then(|d| d.badge) {
        job.append(
            badge.glyph,
            4.0,
            egui::TextFormat::simple(font.clone(), color(badge.color)),
        );
    }
    let tint = decoration.and_then(|d| d.tint).map_or(plain, color);
    job.append(name, 4.0, egui::TextFormat::simple(font, tint));
    job.into()
}

/// Clicks on a row: select, extend, toggle, open, new tab, context menu.
fn interact(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, index: usize, response: &egui::Response) {
    let Some(entry) = cx.tab.row(index) else {
        return;
    };
    let id = entry.id;
    let modifiers = ui.input(|i| i.modifiers);
    let selected = cx.tab.selection.contains(&id);
    let now = ui.input(|i| i.time);
    if response.double_clicked() {
        cx.tab_ui.last_click = None;
        cx.requests.push(Request::OpenEntry { pane: cx.pane, id });
    } else if response.clicked() {
        // A slow second click on the one selected row renames it.
        let slow_second = modifiers.is_none()
            && selected
            && cx.tab.selection.len() == 1
            && cx
                .tab_ui
                .last_click
                .is_some_and(|(last, at)| last == id && now - at >= SLOW_CLICK);
        cx.tab_ui.last_click = Some((id, now));
        if slow_second && cx.tab_ui.edit.is_none() {
            cx.requests.push(Request::StartEdit {
                pane: cx.pane,
                kind: EditKind::Rename(id),
            });
        } else if modifiers.command {
            cx.requests.push(Request::Command(app::Command::Select {
                pane: cx.pane,
                change: SelectionChange::Toggle(id),
            }));
            cx.tab_ui.cursor = Some(id);
        } else if modifiers.shift {
            let anchor = cx.tab_ui.anchor.or(cx.tab_ui.cursor);
            let anchor_index = anchor.and_then(|a| cx.tab.listing.rows().position(|e| e.id == a));
            let range = anchor_index.map_or(index..=index, |a| a.min(index)..=a.max(index));
            let ids = range
                .filter_map(|i| cx.tab.listing.row(i).map(|e| e.id))
                .collect();
            cx.requests.push(Request::Command(app::Command::Select {
                pane: cx.pane,
                change: SelectionChange::Set(ids),
            }));
            cx.tab_ui.cursor = Some(id);
        } else {
            cx.requests.push(Request::Command(app::Command::Select {
                pane: cx.pane,
                change: SelectionChange::Set(vec![id]),
            }));
            cx.tab_ui.cursor = Some(id);
            cx.tab_ui.anchor = Some(id);
        }
    }
    if response.middle_clicked()
        && entry.kind == EntryKind::Dir
        && !vfs_rick::is_trash(&cx.tab.location)
    {
        cx.requests
            .push(Request::NewTabAt(cx.tab.location_of(entry)));
    }
    if response.secondary_clicked() && !selected {
        cx.requests.push(Request::Command(app::Command::Select {
            pane: cx.pane,
            change: SelectionChange::Set(vec![id]),
        }));
        cx.tab_ui.cursor = Some(id);
        cx.tab_ui.anchor = Some(id);
    }
    if !cx.focused {
        cx.requests.push(Request::FocusPane(cx.pane));
    }
    let action_ctx = menus::context_for(cx, if selected { None } else { Some(entry.clone()) });
    egui::Popup::context_menu(response)
        .style(menus::menu_style)
        .show(|ui| menus::context_menu(ui, cx, &action_ctx, Some(entry)));
}
