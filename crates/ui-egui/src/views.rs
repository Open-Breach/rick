//! The file list: the details table and the list grid. Both lay out only
//! the rows in view, so a 200k-entry listing costs the same as a small one.

use egui::{Align, Sense};
use egui_extras::{Column as TableColumn, TableBuilder};
use model::{Column, SelectionChange, SortDir, ViewMode};
use types::{Entry, EntryKind};

use crate::format;
use crate::menus;
use crate::shell::{PaneCx, Request};

/// The glyph in front of a name.
#[must_use]
pub fn icon(entry: &Entry) -> &'static str {
    match entry.kind {
        EntryKind::Dir => "📁",
        EntryKind::Symlink { .. } => "🔗",
        _ => "📄",
    }
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

/// Draws the active view of `cx`.
pub fn show(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    match cx.tab.view {
        ViewMode::List => list(ui, cx),
        ViewMode::Details | ViewMode::Tree | ViewMode::Icons(_) => details(ui, cx),
    }
}

/// The details table: icon and name, type, date modified, size.
pub fn details(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let settings = &cx.app.model().settings;
    let height = row_height(ui, settings.density);
    let listing = &cx.tab.listing;
    let count = listing.row_count();
    cx.tab_ui.visible_rows =
        ((ui.available_height() / (height + ui.spacing().item_spacing.y)) as usize).max(1);

    let mut builder = TableBuilder::new(ui)
        .id_salt(("details", cx.tab_id))
        .striped(settings.zebra_stripes)
        .resizable(true)
        .sense(Sense::click())
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(Align::Center))
        .column(TableColumn::remainder().at_least(140.0).clip(true))
        .column(TableColumn::initial(120.0).at_least(60.0).clip(true))
        .column(TableColumn::initial(130.0).at_least(60.0).clip(true))
        .column(TableColumn::initial(80.0).at_least(50.0));
    if cx.tab_ui.reset_columns {
        builder.reset();
        cx.tab_ui.reset_columns = false;
    }
    if let Some(row) = cx.tab_ui.scroll_to.take() {
        builder = builder.scroll_to_row(row, Some(Align::Center));
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
            });
        }
    });
    let show_extensions = settings.show_extensions;
    let now = cx.now;
    let mut clicked: Vec<(usize, egui::Response)> = Vec::new();
    table.body(|body| {
        body.rows(height, count, |mut row| {
            let index = row.index();
            let Some(entry) = listing.row(index) else {
                return;
            };
            row.set_selected(cx.tab.selection.contains(&entry.id));
            row.col(|ui| {
                ui.label(icon(entry));
                ui.label(display_name(entry, show_extensions));
            });
            row.col(|ui| {
                ui.label(model::type_of(entry));
            });
            row.col(|ui| {
                let (brief, full) = format::date(entry.modified, now);
                ui.label(brief).on_hover_text(full);
            });
            row.col(|ui| {
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if entry.kind != EntryKind::Dir {
                        ui.label(format::size(entry.size));
                    }
                });
            });
            let response = row.response();
            if response.clicked()
                || response.double_clicked()
                || response.secondary_clicked()
                || response.middle_clicked()
                || response.context_menu_opened()
            {
                clicked.push((index, response));
            }
        });
    });
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
        interact(ui, cx, index, &response);
    }
}

/// A column header that shows the sort marker; `true` when clicked.
fn sort_header(ui: &mut egui::Ui, label: &str, column: Column, sort: model::Sort) -> bool {
    let marker = if sort.by == column {
        match sort.dir {
            SortDir::Ascending => " ▲",
            SortDir::Descending => " ▼",
        }
    } else {
        ""
    };
    ui.add(egui::Button::new(format!("{label}{marker}")).frame(false))
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
    let mut clicked: Vec<(usize, egui::Response)> = Vec::new();
    area.show_rows(ui, height, rows, |ui, range| {
        for row in range {
            ui.horizontal(|ui| {
                for column in 0..columns {
                    let index = row * columns + column;
                    let Some(entry) = cx.tab.listing.row(index) else {
                        break;
                    };
                    let selected = cx.tab.selection.contains(&entry.id);
                    let text = format!("{} {}", icon(entry), display_name(entry, show_extensions));
                    let response = ui.add_sized(
                        [cell_width - ui.spacing().item_spacing.x, height],
                        egui::Button::selectable(selected, text),
                    );
                    if response.clicked()
                        || response.double_clicked()
                        || response.secondary_clicked()
                        || response.middle_clicked()
                        || response.context_menu_opened()
                    {
                        clicked.push((index, response));
                    }
                }
            });
        }
    });
    for (index, response) in clicked {
        interact(ui, cx, index, &response);
    }
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
