//! The section overviews: what a tab shows at `rick://machine`,
//! `rick://network` and `rick://cloud`. This Machine and Network are the
//! full drive table — name, mount point or drive letter, file system,
//! total, used, free, a utilisation bar and whether the drive is removable,
//! with Eject for removable media; Cloud lists each provider's accounts and
//! containers. A double-click, or Enter on the row, opens the root in the
//! pane; the rows are the tab's listing, so the keyboard and the selection
//! work as in any list.

use app::Command;
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

/// Paints a small utilisation bar in `rect`, `fraction` of it filled.
pub fn utilisation_bar(ui: &egui::Ui, rect: egui::Rect, fraction: f32) {
    let visuals = ui.visuals();
    ui.painter()
        .rect_filled(rect, 2.0, visuals.widgets.noninteractive.bg_fill);
    let filled = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()),
    );
    let color = if fraction > 0.9 {
        visuals.warn_fg_color
    } else {
        visuals.selection.bg_fill
    };
    ui.painter().rect_filled(filled, 2.0, color);
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
                "No network file system is mounted. Add… in the side panel creates a connection once a network provider is installed."
            }
            Virtual::Cloud => {
                "No cloud account is configured. Add… in the side panel creates one once a cloud provider is installed."
            }
            _ => "No drive is mounted.",
        };
        ui.weak(text);
        return;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match which {
            Virtual::Cloud => cloud_table(ui, cx, &entries),
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
                    utilisation_bar(ui, rect, v.utilisation().unwrap_or(0.0));
                    ui.label(percent(v));
                });
                ui.label(if v.is_removable() { "Yes" } else { "No" });
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

/// The Cloud overview: one row per account or container.
fn cloud_table(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, entries: &[Entry]) {
    let grid = egui::Grid::new(("overview", cx.tab_id))
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
                egui::Button::selectable(selected, format!("{} {}", icons::CLOUD, entry.name))
                    .truncate(),
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
        if ui.button("Open").clicked() {
            cx.requests.push(Request::Command(Command::Navigate {
                pane: cx.pane,
                to: location.clone(),
            }));
            ui.close();
        }
        if ui.button("Open in new tab").clicked() {
            cx.requests.push(Request::NewTabAt(location.clone()));
            ui.close();
        }
    });
}
