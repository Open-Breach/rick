//! The file list: the details table and the list grid. Both lay out only
//! the rows in view, so a 200k-entry listing costs the same as a small one.

use egui::{Align, Sense};
use egui_extras::{Column as TableColumn, TableBuilder};
use ext::{Badge, CellValue, ColumnDef, Decoration};
use model::{Column, IconSize, SelectionChange, SortDir, ViewMode};
use types::{Entry, EntryKind, Location};

use crate::dnd::{self, Spring, Target, Zone};
use crate::format;
use crate::menus;
use crate::shell::{PaneCx, Request};

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

/// Row height for the density setting.
#[must_use]
pub fn row_height(ui: &egui::Ui, density: model::Density) -> f32 {
    let text = ui.text_style_height(&egui::TextStyle::Body);
    text + match density {
        model::Density::Compact => 2.0,
        model::Density::Normal => 6.0,
        model::Density::Comfortable => 12.0,
    }
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
            target: Target::Folder {
                to: cx.tab.location.clone(),
                device: dnd::listing_device(cx.tab),
                spring: None,
            },
        })
    {
        dnd::highlight(ui.painter(), ui.visuals(), area);
    }
    match cx.tab.view {
        ViewMode::List => list(ui, cx),
        ViewMode::Details | ViewMode::Tree => details(ui, cx),
        ViewMode::Icons(size) => icons(ui, cx, size),
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
        };
        background.context_menu(|ui| menus::context_menu(ui, cx, &action_ctx, None));
    }
}

/// The scroll offset to put back: a drag spring-loaded out of this location
/// and the tab is back at it with its listing complete. Any other change of
/// location forgets it.
fn restore_scroll(cx: &mut PaneCx<'_>) -> Option<f32> {
    let changed = cx.tab_ui.last_location.as_ref() != Some(&cx.tab.location);
    if changed {
        cx.tab_ui.last_location = Some(cx.tab.location.clone());
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

/// Registers a row as a drop zone, a folder row to move into and a file row
/// as a silent no-op, and paints its highlight.
fn row_zone(ui: &egui::Ui, cx: &mut PaneCx<'_>, entry: &Entry, rect: egui::Rect) {
    if !cx.dnd.active() {
        return;
    }
    let target = if entry.kind == EntryKind::Dir {
        Target::Folder {
            to: Location::new(cx.tab.location.provider, entry.path.clone()),
            device: dnd::device_of(entry.id),
            spring: Some(Spring::Pane(cx.pane)),
        }
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
    let Some(entry) = cx.tab.listing.row(index) else {
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
    let count = cx.tab.listing.row_count();
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
                    let Some(entry) = cx.tab.listing.row(index) else {
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
        if let Some(entry) = cx.tab.listing.row(*index).cloned() {
            row_zone(ui, cx, &entry, *rect);
        }
    }
}

/// One cell of the icon view.
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
    let visuals = ui.visuals();
    if selected {
        ui.painter()
            .rect_filled(rect, 6.0, visuals.selection.bg_fill);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, visuals.widgets.hovered.weak_bg_fill);
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
        let color = if selected {
            visuals.selection.stroke.color
        } else {
            visuals.text_color()
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

/// The details table: icon and name, type, date modified, size.
pub fn details(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let settings = &cx.app.model().settings;
    let height = row_height(ui, settings.density);
    let listing = &cx.tab.listing;
    let count = listing.row_count();
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
    ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
    let mut builder = TableBuilder::new(ui)
        .id_salt(("details", cx.tab_id))
        .striped(settings.zebra_stripes)
        .resizable(true)
        .sense(Sense::click_and_drag())
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(Align::Center))
        .column(TableColumn::remainder().at_least(140.0).clip(true))
        .column(TableColumn::initial(120.0).at_least(60.0).clip(true))
        .column(TableColumn::initial(130.0).at_least(60.0).clip(true))
        .column(TableColumn::initial(80.0).at_least(50.0));
    for _ in &extra {
        builder = builder.column(TableColumn::initial(90.0).at_least(40.0).clip(true));
    }
    if cx.tab_ui.reset_columns {
        builder.reset();
        cx.tab_ui.reset_columns = false;
    }
    if let Some(row) = cx.tab_ui.scroll_to.take() {
        builder = builder.scroll_to_row(row, Some(Align::Center));
    }
    if let Some(offset) = cx.tab_ui.pending_offset.take() {
        builder = builder.vertical_scroll_offset(offset);
    }

    let sort = cx.tab.sort;
    let mut sort_request = None;
    let table = builder.header(height, |mut row| {
        for (label, column) in [
            ("Name", Column::Name),
            ("Type", Column::Type),
            ("Date modified", Column::Modified),
            ("Size", Column::Size),
        ] {
            row.col(|ui| {
                if sort_header(ui, label, column, sort) {
                    sort_request = Some(column);
                }
                header_separator(ui);
            });
        }
        for def in &extra {
            row.col(|ui| {
                ui.strong(def.label);
                header_separator(ui);
            });
        }
    });
    let show_extensions = settings.show_extensions;
    let now = cx.now;
    let mut clicked: Vec<(usize, egui::Response)> = Vec::new();
    let mut drawn: Vec<(usize, egui::Rect)> = Vec::new();
    let out = table.body(|body| {
        body.rows(height, count, |mut row| {
            let index = row.index();
            let Some(entry) = listing.row(index) else {
                return;
            };
            details_row(&mut row, cx.tab, entry, &extra, show_extensions, now);
            let response = row.response();
            drawn.push((index, response.rect));
            if interacted(&response) {
                clicked.push((index, response));
            }
        });
    });
    cx.tab_ui.scroll_offset = out.state.offset.y;
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

/// One row of the details table: icon, badge, name, type, date, size and
/// the decorators' columns.
fn details_row(
    row: &mut egui_extras::TableRow<'_, '_>,
    tab: &model::Tab,
    entry: &Entry,
    extra: &[ColumnDef],
    show_extensions: bool,
    now: std::time::SystemTime,
) {
    row.set_selected(tab.selection.contains(&entry.id));
    let decoration = tab.decorations.get(&entry.id);
    row.col(|ui| {
        cell(ui, icon(entry));
        if let Some(badge) = decoration.and_then(|d| d.badge) {
            cell(ui, badge_text(badge));
        }
        let mut name = egui::RichText::new(display_name(entry, show_extensions));
        if let Some(tint) = decoration.and_then(|d| d.tint) {
            name = name.color(color(tint));
        }
        cell(ui, name);
    });
    row.col(|ui| {
        cell(ui, model::type_of(entry));
    });
    row.col(|ui| {
        let (brief, full) = format::date(entry.modified, now);
        cell(ui, brief).on_hover_text(full);
    });
    row.col(|ui| {
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if entry.kind != EntryKind::Dir {
                cell(ui, format::size(entry.size));
            }
        });
    });
    for def in extra {
        row.col(|ui| {
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

/// The vertical line between column titles, at the right edge of a header
/// cell, where the resize handle is.
fn header_separator(ui: &mut egui::Ui) {
    let rect = ui.max_rect();
    let x = rect.right() + ui.spacing().item_spacing.x / 2.0;
    let stroke = ui.visuals().widgets.noninteractive.fg_stroke;
    let stroke = egui::Stroke::new(1.0, stroke.color.gamma_multiply(0.4));
    ui.painter().vline(x, rect.y_range().shrink(3.0), stroke);
}

/// A column header that shows the sort marker; `true` when clicked.
fn sort_header(ui: &mut egui::Ui, label: &str, column: Column, sort: model::Sort) -> bool {
    let marker = if sort.by == column {
        match sort.dir {
            SortDir::Ascending => crate::icons::SORT_ASC,
            SortDir::Descending => crate::icons::SORT_DESC,
        }
    } else {
        ""
    };
    ui.add(egui::Button::new(format!("{label} {marker}").trim_end().to_owned()).frame(false))
        .clicked()
}

/// The list grid: icon and name only, in as many columns as fit.
pub fn list(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let settings = &cx.app.model().settings;
    let height = row_height(ui, settings.density);
    let cell_width = 220.0;
    let columns = ((ui.available_width() / cell_width).floor() as usize).max(1);
    let count = cx.tab.listing.row_count();
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
    let out = area.show_rows(ui, height, rows, |ui, range| {
        for row in range {
            ui.horizontal(|ui| {
                for column in 0..columns {
                    let index = row * columns + column;
                    let Some(entry) = cx.tab.listing.row(index) else {
                        break;
                    };
                    let selected = cx.tab.selection.contains(&entry.id);
                    let decoration = cx.tab.decorations.get(&entry.id);
                    let text = cell_text(
                        ui,
                        icon(entry),
                        display_name(entry, show_extensions),
                        decoration,
                    );
                    let response = ui.add_sized(
                        [cell_width - ui.spacing().item_spacing.x, height],
                        egui::Button::selectable(selected, text).sense(Sense::click_and_drag()),
                    );
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
    let Some(entry) = cx.tab.listing.row(index) else {
        return;
    };
    let id = entry.id;
    let modifiers = ui.input(|i| i.modifiers);
    let selected = cx.tab.selection.contains(&id);
    if response.double_clicked() {
        cx.requests.push(Request::OpenEntry { pane: cx.pane, id });
    } else if response.clicked() {
        if modifiers.command {
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
    if response.middle_clicked() && entry.kind == EntryKind::Dir {
        let location = types::Location::new(
            cx.tab.location.provider,
            cx.tab.location.path.join(entry.name.as_bytes()),
        );
        cx.requests.push(Request::NewTabAt(location));
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
    response.context_menu(|ui| menus::context_menu(ui, cx, &action_ctx, Some(entry)));
}
