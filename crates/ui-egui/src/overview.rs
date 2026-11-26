//! The section overviews: what a tab shows at `rick://machine`,
//! `rick://network` and `rick://cloud`. This Machine and Network are the
//! full drive table — name, mount point or drive letter, file system,
//! total, used, free, a utilisation bar and whether the drive is removable,
//! with Eject for removable media; Cloud lists each provider's accounts and
//! containers. A double-click, or Enter on the row, opens the root in the
//! pane; the rows are the tab's listing, so the keyboard and the selection
//! work as in any list.

use app::Command;
use ext::t;
use model::{SelectionChange, Volume, VolumeKind};
use types::{Entry, Location};
use vfs_rick::Virtual;

use crate::shell::{PaneCx, Request};
use crate::{format, icons};

/// The icon of a volume's kind.
#[must_use]
pub fn volume_icon(kind: VolumeKind) -> &'static str {
    match kind {
        VolumeKind::Removable => icons::USB,
        VolumeKind::Network => icons::NETWORK,
        VolumeKind::Optical => icons::DISC,
        VolumeKind::Fixed => icons::DISK,
    }
}

/// The fill colour of a usage bar by how full the drive is, like the Trash
/// pill (SPEC.md *This Machine*): `accent/usage` below 80 %,
/// `status/warning` from 80 %, `status/danger` from 90 %.
#[must_use]
pub fn usage_colour(scheme: &crate::theme::Scheme, fraction: f32) -> egui::Color32 {
    if fraction >= 0.9 {
        scheme.status_danger
    } else if fraction >= 0.8 {
        scheme.status_warning
    } else {
        scheme.accent_usage
    }
}

/// Paints a small utilisation bar in `rect`, `fraction` of it filled in
/// the colour of [`usage_colour`] on a `bg/track` track.
pub fn utilisation_bar(ui: &egui::Ui, rect: egui::Rect, fraction: f32) {
    let scheme = crate::theme::scheme(ui);
    ui.painter().rect_filled(rect, 2.0, scheme.bg_track);
    let filled = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()),
    );
    ui.painter()
        .rect_filled(filled, 2.0, usage_colour(scheme, fraction));
}

/// Paints the bar of a read-only volume in `rect`: full, in
/// `text/placeholder`, no threshold colour — the medium is full by design
/// (SPEC.md *This Machine*).
pub fn read_only_bar(ui: &egui::Ui, rect: egui::Rect) {
    let scheme = crate::theme::scheme(ui);
    ui.painter().rect_filled(rect, 2.0, scheme.text_placeholder);
}

/// The bar of a volume: the read-only bar for read-only media, the
/// utilisation bar otherwise.
pub fn volume_bar(ui: &egui::Ui, rect: egui::Rect, volume: &Volume) {
    if volume.read_only {
        read_only_bar(ui, rect);
    } else {
        utilisation_bar(ui, rect, volume.utilisation().unwrap_or(0.0));
    }
}

/// `45%`, or an em dash when the volume's size is unknown.
#[must_use]
pub fn percent(volume: &Volume) -> String {
    volume.utilisation().map_or_else(
        || "—".to_owned(),
        |u| format!("{}%", (u * 100.0).round() as u32),
    )
}

/// The volume a listing entry of the Machine or Network overview stands
/// for, by mount point.
fn volume_of<'a>(cx: &'a PaneCx<'_>, entry: &Entry) -> Option<&'a Volume> {
    let path = platform::paths::to_path(&entry.path).ok()?;
    cx.app
        .model()
        .volumes
        .iter()
        .find(|v| v.mount_point == path)
}

/// Draws the overview of `which`.
pub fn show(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, which: Virtual) {
    let entries: Vec<Entry> = cx.tab.listing.rows().cloned().collect();
    ui.add_space(6.0);
    if entries.is_empty() && cx.tab.listing.complete {
        empty(ui, cx, which);
        return;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match which {
            Virtual::Cloud => cloud_table(ui, cx, &entries, icons::CLOUD),
            Virtual::Network => {
                // The mounted shares as drives, then the connections of the
                // network providers as the Cloud overview lists accounts.
                let (volumes, connections): (Vec<Entry>, Vec<Entry>) = entries
                    .into_iter()
                    .partition(|e| e.id.provider == vfs_local::PROVIDER_ID);
                if !volumes.is_empty() {
                    drive_table(ui, cx, &volumes);
                }
                if !connections.is_empty() {
                    if !volumes.is_empty() {
                        ui.add_space(12.0);
                    }
                    ui.strong(t!("overview-connections"));
                    ui.add_space(4.0);
                    cloud_table(ui, cx, &connections, icons::NETWORK);
                }
            }
            _ => drive_table(ui, cx, &entries),
        });
}

/// An overview with nothing in it: centred in the pane like the empty
/// folder's message, the section's glyph, what is missing and, for Cloud
/// and Network, a button per kind of connection a plugin offers, which
/// opens its *Add connection…* dialogue right here.
fn empty(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, which: Virtual) {
    let scheme = crate::theme::scheme(ui);
    let rect = ui.available_rect_before_wrap();
    let (glyph, title, hint, section) = match which {
        Virtual::Cloud => (
            icons::CLOUD,
            t!("overview-empty-cloud"),
            t!("overview-empty-cloud-hint"),
            Some(model::Section::Cloud),
        ),
        Virtual::Network => (
            icons::NETWORK,
            t!("overview-empty-network"),
            t!("overview-empty-network-hint"),
            Some(model::Section::Network),
        ),
        _ => (
            icons::DISK,
            t!("overview-empty-machine"),
            String::new(),
            None,
        ),
    };
    let connectors = section.map_or_else(Vec::new, |s| crate::side_panel::connectors_of(cx.app, s));
    let hint = if section.is_some() && connectors.is_empty() {
        t!("overview-empty-no-plugin")
    } else {
        hint
    };
    // The block's height, so that it sits in the middle as a whole.
    let buttons = if connectors.is_empty() {
        0.0
    } else {
        36.0 + 20.0
    };
    let height = 64.0 + 16.0 + 20.0 + 6.0 + 18.0 + buttons;
    let top = (rect.center().y - height / 2.0).max(rect.top() + 12.0);
    let mut block = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_max(
                egui::pos2(rect.left(), top),
                rect.max,
            ))
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    block.spacing_mut().item_spacing.y = 0.0;
    block.add(
        egui::Label::new(
            egui::RichText::new(glyph)
                .size(64.0)
                .color(scheme.icon_default.gamma_multiply(0.45)),
        )
        .selectable(false),
    );
    block.add_space(16.0);
    block.add(
        egui::Label::new(
            egui::RichText::new(title)
                .size(crate::theme::TEXT_BODY)
                .color(scheme.text_primary),
        )
        .selectable(false),
    );
    if !hint.is_empty() {
        block.add_space(6.0);
        block.add(
            egui::Label::new(
                egui::RichText::new(hint)
                    .size(crate::theme::TEXT_SECONDARY)
                    .color(scheme.text_secondary),
            )
            .selectable(false),
        );
    }
    if !connectors.is_empty() {
        block.add_space(20.0);
        add_buttons(&mut block, cx, &connectors, rect.width());
    }
    ui.allocate_rect(rect, egui::Sense::hover());
}

/// The buttons of an empty overview, side by side and centred as a row:
/// one per kind of connection, the first the filled one. A click opens
/// that kind's *Add connection…* dialogue.
fn add_buttons(
    ui: &mut egui::Ui,
    cx: &mut PaneCx<'_>,
    connectors: &[std::sync::Arc<dyn ext::Connector>],
    room: f32,
) {
    let labels: Vec<String> = connectors
        .iter()
        .map(|c| {
            let kind = c.schema().title.to_string();
            t!("panel-add-a-connection", kind = kind.as_str())
        })
        .collect();
    let widths: f32 = labels
        .iter()
        .map(|label| text_width(ui, label) + 32.0)
        .sum();
    #[allow(clippy::cast_precision_loss)]
    let row = widths + 10.0 * labels.len().saturating_sub(1) as f32;
    ui.allocate_ui_with_layout(
        egui::vec2(row.min(room), 36.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for (i, (connector, label)) in connectors.iter().zip(&labels).enumerate() {
                if crate::widgets::button(ui, label, i == 0, true).clicked() {
                    cx.requests
                        .push(Request::AddConnection(connector.provider()));
                }
            }
        },
    );
}

/// A table of an overview, drawn with the table the details view's
/// columns come from (`crate::table`, SPEC.md *File list*): the header
/// band and the rows of the guide, columns that fit their content on
/// their own, a drag on a divider that resizes, a double-click that fits
/// again (specs/s3-connection.md 2.2).
///
/// Until the user resizes a column the widths follow the content, so a
/// row added later is not cut off; a resize is kept for the tab. `cell`
/// lays out a cell, `content` says how wide its content is.
fn fitted_table(
    ui: &mut egui::Ui,
    id: egui::Id,
    columns: &[crate::table::Column],
    rows: usize,
    cell: &mut dyn FnMut(&mut egui::Ui, usize, usize),
    content: &dyn Fn(&egui::Ui, usize, usize) -> f32,
) {
    let kept: Option<Vec<f32>> = ui.data(|data| data.get_temp(id));
    let mut widths = kept.unwrap_or_else(|| {
        (0..columns.len().saturating_sub(1))
            .map(|col| {
                let title = text_width(ui, &columns[col].title);
                fitted_width(
                    title,
                    (0..rows)
                        .map(|row| content(ui, row, col))
                        .fold(0.0, f32::max),
                )
            })
            .collect()
    });
    let before = widths.clone();
    #[allow(clippy::cast_precision_loss)]
    let height = crate::table::HEADER
        + rows as f32 * crate::views::row_height(ui)
        + ui.spacing().scroll.bar_width;
    let size = egui::vec2(ui.available_width(), height);
    ui.allocate_ui(size, |ui| {
        ui.set_min_size(size);
        crate::table::show(ui, id, columns, &mut widths, rows, cell, content);
    });
    // A drag moves a divider a little in every frame, and a double-click
    // fits at once: what the user set is kept from the first change on.
    if widths != before {
        ui.data_mut(|data| data.insert_temp(id, widths));
    }
}

/// The width a column fits itself to: its title or its widest cell,
/// whichever is wider, with a cell's padding at both sides, and no
/// narrower than the table lets a column be. The table would widen a
/// narrower one itself, and that must not read as the user's resize,
/// which stops the fitting.
fn fitted_width(title: f32, content: f32) -> f32 {
    (title.max(content) + 24.0).max(crate::table::MIN)
}

/// How wide `text` is in the body font.
fn text_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            text.to_owned(),
            egui::FontId::proportional(crate::theme::TEXT_BODY),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        .x
}

/// The width of the utilisation bar in a drive's row.
const BAR_WIDTH: f32 = 80.0;

/// The drive table of This Machine and Network, in the overviews' table:
/// the name, where the drive is mounted, its file system, its sizes, the
/// utilisation bar with its percentage, whether it is removable and, for
/// one that is, *Eject*.
fn drive_table(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, entries: &[Entry]) {
    use crate::table::Column;
    let columns = [
        Column::new(t!("overview-column-name")),
        Column::new(t!("overview-column-mount-point")),
        Column::new(t!("overview-column-file-system")),
        Column::right(t!("overview-column-total")),
        Column::right(t!("overview-column-used")),
        Column::right(t!("overview-column-free")),
        Column::new(t!("overview-column-utilisation")),
        Column::new(t!("overview-column-removable")),
        Column::new(""),
    ];
    let volumes: Vec<Option<Volume>> = entries
        .iter()
        .map(|entry| volume_of(cx, entry).cloned())
        .collect();
    let eject = format!("{} {}", icons::EJECT, t!("overview-eject"));
    // The text of a cell, for the columns that are text.
    let text = |row: usize, col: usize| -> String {
        let Some(entry) = entries.get(row) else {
            return String::new();
        };
        let Some(v) = volumes.get(row).and_then(Option::as_ref) else {
            return match col {
                0 => format!("{} {}", icons::DISK, entry.name),
                1 => entry.path.to_string(),
                8 => String::new(),
                _ => "—".to_owned(),
            };
        };
        match col {
            0 => format!("{} {}", volume_icon(v.kind), entry.name),
            1 => v.mount_point.display().to_string(),
            2 if v.fs_type.is_empty() => "—".to_owned(),
            2 => v.fs_type.clone(),
            3 => format::size(v.total),
            4 => format::size(v.used()),
            5 => format::size(v.free),
            6 => percent(v),
            7 if v.is_removable() => t!("overview-yes"),
            7 => t!("overview-no"),
            _ => String::new(),
        }
    };
    let button = 2.0 * ui.spacing().button_padding.x;
    fitted_table(
        ui,
        egui::Id::new(("overview", cx.tab_id)),
        &columns,
        entries.len(),
        &mut |ui, row, col| {
            let Some(entry) = entries.get(row) else {
                return;
            };
            let volume = volumes.get(row).and_then(Option::as_ref);
            match (col, volume) {
                (0, _) => {
                    let selected = cx.tab.selection.contains(&entry.id);
                    let response =
                        ui.add(egui::Button::selectable(selected, text(row, col)).truncate());
                    row_interaction(cx, entry, &response);
                }
                (6, Some(v)) => {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(BAR_WIDTH, 8.0), egui::Sense::hover());
                    volume_bar(ui, rect, v);
                    ui.label(text(row, col));
                }
                (8, Some(v)) if v.is_removable() => {
                    if ui.small_button(&eject).clicked() {
                        cx.requests
                            .push(Request::Command(Command::Eject(v.mount_point.clone())));
                    }
                }
                _ => {
                    ui.add(egui::Label::new(text(row, col)).truncate());
                }
            }
        },
        &|ui, row, col| {
            let width = text_width(ui, &text(row, col));
            let removable = volumes
                .get(row)
                .and_then(Option::as_ref)
                .is_some_and(Volume::is_removable);
            match col {
                0 => width + button,
                6 => width + BAR_WIDTH + ui.spacing().item_spacing.x,
                8 if removable => text_width(ui, &eject) + button,
                _ => width,
            }
        },
    );
}

/// The Cloud overview, and the connections of the Network overview: one
/// row per account or connection, in the overviews' table.
fn cloud_table(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, entries: &[Entry], glyph: &str) {
    let columns = [
        crate::table::Column::new(t!("overview-column-name")),
        crate::table::Column::new(t!("overview-column-provider")),
        crate::table::Column::new(t!("overview-column-path")),
    ];
    let text = |entry: &Entry, col: usize| match col {
        0 => format!("{glyph} {}", entry.name),
        1 => entry.id.provider.to_string(),
        _ => entry.path.to_string(),
    };
    let button = 2.0 * ui.spacing().button_padding.x;
    fitted_table(
        ui,
        egui::Id::new(("overview-accounts", cx.tab_id)),
        &columns,
        entries.len(),
        &mut |ui, row, col| {
            let Some(entry) = entries.get(row) else {
                return;
            };
            if col == 0 {
                let selected = cx.tab.selection.contains(&entry.id);
                let response =
                    ui.add(egui::Button::selectable(selected, text(entry, col)).truncate());
                row_interaction(cx, entry, &response);
            } else {
                ui.add(egui::Label::new(text(entry, col)).truncate());
            }
        },
        &|ui, row, col| {
            entries.get(row).map_or(0.0, |entry| {
                text_width(ui, &text(entry, col)) + if col == 0 { button } else { 0.0 }
            })
        },
    );
}

/// Selection, opening and the new-tab shortcuts of an overview row.
fn row_interaction(cx: &mut PaneCx<'_>, entry: &Entry, response: &egui::Response) {
    let location: Location = cx.tab.location_of(entry);
    // The drive the row stands for, in the This Machine overview.
    let volume = volume_of(cx, entry).cloned();
    let format_tool = cx.app.platform().has_format_tool();
    if response.double_clicked() {
        cx.requests.push(Request::Command(Command::Navigate {
            pane: cx.pane,
            to: location.clone(),
        }));
    } else if response.clicked() {
        if response.ctx.input(|i| i.modifiers.command) {
            cx.requests.push(Request::NewTabAt(location.clone()));
        } else {
            cx.requests.push(Request::Command(Command::Select {
                pane: cx.pane,
                change: SelectionChange::Set(vec![entry.id]),
            }));
            cx.tab_ui.cursor = Some(entry.id);
        }
    }
    if response.middle_clicked() {
        cx.requests.push(Request::NewTabAt(location.clone()));
    }
    if !cx.focused && (response.clicked() || response.middle_clicked()) {
        cx.requests.push(Request::FocusPane(cx.pane));
    }
    // The menu card of the file list (SPEC.md *Context menu*). A drive's
    // row has what its card in the side panel has: Eject, Format… and
    // Properties.
    crate::menus::card(response, |ui| {
        if crate::menus::item(ui, Some(icons::FOLDER_OPEN), &t!("open")).clicked() {
            cx.requests.push(Request::Command(Command::Navigate {
                pane: cx.pane,
                to: location.clone(),
            }));
            ui.close();
        }
        if crate::menus::item(ui, None, &t!("open-in-new-tab")).clicked() {
            cx.requests.push(Request::NewTabAt(location.clone()));
            ui.close();
        }
        if let Some(volume) = &volume {
            crate::menus::drive_items(ui, volume, format_tool, cx.requests);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Scheme;

    /// A column of dashes under a short title, in a narrow font, would
    /// come out under the table's smallest column. The table widened it,
    /// the overview took that for the user's resize, and the columns
    /// stopped fitting in their first frame (macOS, tier 2).
    #[test]
    fn a_fitted_column_is_never_narrower_than_the_table_s_smallest() {
        assert!((fitted_width(18.0, 6.0) - crate::table::MIN).abs() < f32::EPSILON);
        assert!((fitted_width(0.0, 0.0) - crate::table::MIN).abs() < f32::EPSILON);
        assert!((fitted_width(40.0, 120.0) - 144.0).abs() < f32::EPSILON);
        assert!((fitted_width(120.0, 40.0) - 144.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_usage_colour_follows_the_fill_like_the_trash_pill() {
        for scheme in [&Scheme::LIGHT, &Scheme::DARK] {
            assert_eq!(usage_colour(scheme, 0.0), scheme.accent_usage);
            assert_eq!(usage_colour(scheme, 0.4), scheme.accent_usage);
            assert_eq!(usage_colour(scheme, 0.799), scheme.accent_usage);
            assert_eq!(usage_colour(scheme, 0.8), scheme.status_warning);
            assert_eq!(usage_colour(scheme, 0.85), scheme.status_warning);
            assert_eq!(usage_colour(scheme, 0.9), scheme.status_danger);
            assert_eq!(usage_colour(scheme, 1.0), scheme.status_danger);
        }
    }
}
