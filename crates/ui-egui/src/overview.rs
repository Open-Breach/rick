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
        let text = match which {
            Virtual::Network => {
                "No network file system is mounted and no connection is configured. Add connection… in the side panel's Network section creates one."
            }
            Virtual::Cloud => {
                "No cloud account is configured. Add connection… in the side panel's Cloud section creates one."
            }
            _ => "No drive is mounted.",
        };
        ui.weak(text);
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

/// One header cell.
fn head(ui: &mut egui::Ui, text: &str) {
    ui.strong(text);
}

/// The drive table of This Machine and Network.
fn drive_table(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, entries: &[Entry]) {
    let grid = egui::Grid::new(("overview", cx.tab_id))
        .striped(true)
        .num_columns(9)
        .spacing(egui::vec2(16.0, 6.0));
    grid.show(ui, |ui| {
        for title in [
            "Name",
            "Mount point",
            "File system",
            "Total",
            "Used",
            "Free",
            "Utilisation",
            "Removable",
            "",
        ] {
            head(ui, title);
        }
        ui.end_row();
        for entry in entries {
            let volume = volume_of(cx, entry).cloned();
            let selected = cx.tab.selection.contains(&entry.id);
            let icon = volume.as_ref().map_or(icons::DISK, |v| volume_icon(v.kind));
            let response = ui.add(
                egui::Button::selectable(selected, format!("{icon} {}", entry.name)).truncate(),
            );
            row_interaction(cx, entry, &response);
            if let Some(v) = &volume {
                ui.label(v.mount_point.display().to_string());
                ui.label(if v.fs_type.is_empty() {
                    "—".to_owned()
                } else {
                    v.fs_type.clone()
                });
                ui.label(format::size(v.total));
                ui.label(format::size(v.used()));
                ui.label(format::size(v.free));
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(80.0, 8.0), egui::Sense::hover());
                    volume_bar(ui, rect, v);
                    ui.label(percent(v));
                });
                ui.label(if v.is_removable() {
                    t!("overview-yes")
                } else {
                    t!("overview-no")
                });
                if v.is_removable() {
                    if ui.small_button(format!("{} Eject", icons::EJECT)).clicked() {
                        cx.requests
                            .push(Request::Command(Command::Eject(v.mount_point.clone())));
                    }
                } else {
                    ui.label("");
                }
            } else {
                ui.label(entry.path.to_string());
                for _ in 0..7 {
                    ui.label("—");
                }
            }
            ui.end_row();
        }
    });
}

/// The Cloud overview, and the connections of the Network overview: one
/// row per account or connection.
fn cloud_table(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, entries: &[Entry], glyph: &str) {
    let grid = egui::Grid::new(("overview-accounts", cx.tab_id))
        .striped(true)
        .num_columns(3)
        .spacing(egui::vec2(16.0, 6.0));
    grid.show(ui, |ui| {
        for title in ["Name", "Provider", "Path"] {
            head(ui, title);
        }
        ui.end_row();
        for entry in entries {
            let selected = cx.tab.selection.contains(&entry.id);
            let response = ui.add(
                egui::Button::selectable(selected, format!("{glyph} {}", entry.name)).truncate(),
            );
            row_interaction(cx, entry, &response);
            ui.label(entry.id.provider.to_string());
            ui.label(entry.path.to_string());
            ui.end_row();
        }
    });
}

/// Selection, opening and the new-tab shortcuts of an overview row.
fn row_interaction(cx: &mut PaneCx<'_>, entry: &Entry, response: &egui::Response) {
    let location: Location = cx.tab.location_of(entry);
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
    response.clone().context_menu(|ui| {
        if ui.button(t!("open")).clicked() {
            cx.requests.push(Request::Command(Command::Navigate {
                pane: cx.pane,
                to: location.clone(),
            }));
            ui.close();
        }
        if ui.button(t!("open-in-new-tab")).clicked() {
            cx.requests.push(Request::NewTabAt(location.clone()));
            ui.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Scheme;

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
