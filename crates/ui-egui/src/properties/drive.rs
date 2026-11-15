//! The Properties dialogue of a drive (SPEC.md *Properties dialogue*, *A
//! drive*): the frame, the header and the cards of the dialogue of an
//! entry, on one page without the segmented control. The header has the
//! drive's icon in the tile, its name, and the kind and the file system as
//! the secondary line. **Volume** has the label, the file system, the
//! mount point — the drive letter on Windows — and the device where the
//! platform gives one; **Capacity** the total, the used and the free
//! space and the usage bar of the drive card; **Flags** read-only and
//! removable, as switches that show and are not changed. The footer has
//! *Close* and, at the left, the actions there are for the drive: *Eject*
//! for a removable one, and *Format…*.
//!
//! The values are what the model's volumes hold — what `platform::volumes`
//! gives the This Machine overview — read every frame, so the dialogue
//! follows the drive: a value the platform does not give is left out, and
//! a drive that is unmounted takes its dialogue with it.

use std::path::{Path, PathBuf};

use app::{App, Command};
use egui::{Align2, FontId, pos2};
use ext::{Level, t};
use model::{Volume, VolumeKind};

use super::{GROUP_GAP, INSET, ROW, WIDTH, group, size_of};
use crate::shell::Request;
use crate::theme::{self, TEXT_BODY};
use crate::{dialog, overview, widgets};

/// The usage bar's height, the drive card's.
const BAR: f32 = 4.0;
/// The room the percentage has at the bar's right end.
const PERCENT: f32 = 44.0;

/// The dialogue's state between frames: the drive it shows, by its mount
/// point.
#[derive(Debug, Default)]
pub struct DriveDialog {
    mount_point: Option<PathBuf>,
}

/// What the dialogue was asked to do in a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ask {
    Close,
    Copy(String),
    Eject,
    Format,
}

impl DriveDialog {
    /// Shows the drive mounted at `mount_point`.
    pub fn open(&mut self, mount_point: PathBuf) {
        self.mount_point = Some(mount_point);
    }

    /// Whether the dialogue shows.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.mount_point.is_some()
    }

    /// The mount point of the drive shown; `None` while closed.
    #[must_use]
    pub fn mount_point(&self) -> Option<&Path> {
        self.mount_point.as_deref()
    }

    /// Draws the dialogue, when open.
    pub fn show(&mut self, ctx: &egui::Context, app: &App, requests: &mut Vec<Request>) {
        let Some(mount_point) = self.mount_point.clone() else {
            return;
        };
        // The drive as the overview has it now. Unmounted, it takes the
        // dialogue with it.
        let volume = app
            .model()
            .volumes
            .iter()
            .find(|volume| volume.mount_point == mount_point);
        let Some(volume) = volume else {
            self.mount_point = None;
            return;
        };
        let format_tool = app.platform().has_format_tool();
        let mut asked: Option<Ask> = None;
        let modal = dialog::modal(ctx, egui::Id::new("drive-properties"), WIDTH, |ui| {
            let icon = overview::volume_icon(volume.kind);
            if dialog::band(ui, Some(icon), &volume.name, &subtitle(volume)) {
                asked = Some(Ask::Close);
            }
            dialog::body(ui, |ui| {
                ui.spacing_mut().item_spacing.y = GROUP_GAP;
                page(ui, volume, &mut asked);
            });
            footer(ui, volume, format_tool, &mut asked);
        });
        if modal.should_close() {
            asked = Some(Ask::Close);
        }
        match asked {
            None => {}
            Some(Ask::Close) => self.mount_point = None,
            Some(Ask::Copy(text)) => {
                if let Err(error) = app.platform().set_text(&text) {
                    requests.push(Request::Toast(Level::Error, error.to_string()));
                }
            }
            // Eject closes the dialogue as it ejects; the drive goes with
            // the rules it has in This Machine.
            Some(Ask::Eject) => {
                requests.push(Request::Command(Command::Eject(mount_point)));
                self.mount_point = None;
            }
            Some(Ask::Format) => requests.push(Request::Command(Command::Format(mount_point))),
        }
    }
}

/// The kind of drive, in words.
fn kind_of(kind: VolumeKind) -> String {
    match kind {
        VolumeKind::Removable => t!("properties-drive-removable"),
        VolumeKind::Fixed => t!("properties-drive-local"),
        VolumeKind::Network => t!("properties-drive-network"),
        VolumeKind::Optical => t!("properties-drive-optical"),
    }
}

/// The header's secondary line: *Removable drive · ext4*, or the kind
/// alone where the system names no file system. The two are set apart as
/// the type and the size are in an entry's dialogue.
fn subtitle(volume: &Volume) -> String {
    let kind = kind_of(volume.kind);
    if volume.fs_type.is_empty() {
        kind
    } else {
        format!("{kind}  ·  {}", volume.fs_type)
    }
}

/// Whether a mount point is a drive letter, as Windows mounts a volume:
/// `C:\`. The row is named after what it holds.
fn is_drive_letter(mount_point: &Path) -> bool {
    let text = mount_point.to_string_lossy();
    let mut chars = text.chars();
    matches!(
        (chars.next(), chars.next(), chars.as_str()),
        (Some(letter), Some(':'), "" | "\\" | "/") if letter.is_ascii_alphabetic()
    )
}

/// The three groups: **Volume**, **Capacity** and **Flags**.
fn page(ui: &mut egui::Ui, volume: &Volume, asked: &mut Option<Ask>) {
    group(ui, &t!("properties-volume"), |card| {
        card.value(&t!("properties-label"), &volume.name);
        if !volume.fs_type.is_empty() {
            card.value(&t!("properties-file-system"), &volume.fs_type);
        }
        let mount_point = volume.mount_point.display().to_string();
        let (label, copy) = if is_drive_letter(&volume.mount_point) {
            (
                t!("properties-drive-letter"),
                t!("properties-copy-drive-letter"),
            )
        } else {
            (
                t!("properties-mount-point"),
                t!("properties-copy-mount-point"),
            )
        };
        if card.path(&label, &mount_point, &copy) {
            *asked = Some(Ask::Copy(mount_point));
        }
        if let Some(device) = &volume.device
            && card.path(
                &t!("properties-device"),
                device,
                &t!("properties-copy-device"),
            )
        {
            *asked = Some(Ask::Copy(device.clone()));
        }
    });

    // What the system reports of the space, and the bar; a volume whose
    // size is unknown and that is not read-only has no group at all.
    let sizes = [
        (t!("properties-total"), volume.total),
        (t!("properties-used"), volume.used()),
        (t!("properties-free"), volume.free),
    ];
    let bar = volume.read_only || volume.utilisation().is_some();
    if bar || sizes.iter().any(|(_, bytes)| bytes.is_some()) {
        group(ui, &t!("properties-capacity"), |card| {
            for (label, bytes) in &sizes {
                if let Some(bytes) = bytes {
                    card.value(label, &size_of(*bytes));
                }
            }
            if bar {
                usage(card, volume);
            }
        });
    }

    group(ui, &t!("properties-flags"), |card| {
        card.state(&t!("panel-read-only"), volume.read_only);
        card.state(&t!("properties-removable"), volume.is_removable());
    });
}

/// The usage bar of the drive card, with its rules: the fill in the
/// threshold colours and the percentage at its right end, or, for a
/// read-only medium, the full muted bar and no percentage.
fn usage(card: &mut super::Card<'_>, volume: &Volume) {
    let rect = card.take(ROW);
    let ui = &mut *card.ui;
    let scheme = theme::scheme(ui);
    let right = if volume.read_only {
        rect.right() - INSET
    } else {
        rect.right() - INSET - PERCENT
    };
    let bar = egui::Rect::from_min_max(
        pos2(rect.left() + INSET, rect.center().y - BAR / 2.0),
        pos2(right, rect.center().y + BAR / 2.0),
    );
    overview::volume_bar(ui, bar, volume);
    let spoken = if volume.read_only {
        t!("panel-read-only")
    } else {
        let percent = overview::percent(volume);
        ui.painter().text(
            pos2(rect.right() - INSET, rect.center().y),
            Align2::RIGHT_CENTER,
            &percent,
            FontId::proportional(TEXT_BODY),
            scheme.text_secondary,
        );
        percent
    };
    let name = t!("properties-usage");
    ui.interact(rect, ui.id().with("drive-usage"), egui::Sense::hover())
        .widget_info(|| {
            let mut info =
                egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, true, &name);
            info.current_text_value = Some(spoken.clone());
            info
        });
}

/// The footer: *Close*, and at the left the actions there are for the
/// drive, with the rules they have in This Machine — *Eject* for a
/// removable drive only, and *Format…* for a removable or a local one,
/// disabled with its tooltip where no disk utility is installed.
fn footer(ui: &mut egui::Ui, volume: &Volume, format_tool: bool, asked: &mut Option<Ask>) {
    let formattable = matches!(volume.kind, VolumeKind::Removable | VolumeKind::Fixed);
    let chosen = std::cell::Cell::new(None);
    dialog::footer(
        ui,
        |ui| {
            if volume.is_removable()
                && widgets::button(ui, &t!("properties-eject"), false, true).clicked()
            {
                chosen.set(Some(Ask::Eject));
            }
            if formattable {
                let format = widgets::button(ui, &t!("panel-format"), false, format_tool);
                if format_tool {
                    if format.clicked() {
                        chosen.set(Some(Ask::Format));
                    }
                } else {
                    format.on_disabled_hover_text(t!("panel-no-disk-utility"));
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mount_point_that_is_a_drive_letter_is_named_one() {
        for letter in ["C:\\", "d:\\", "E:", "Z:/"] {
            assert!(is_drive_letter(Path::new(letter)), "{letter}");
        }
        for path in [
            "/",
            "/mnt/usb",
            "/media/me/STICK",
            "C:\\Users",
            "\\\\nas\\share",
            "",
        ] {
            assert!(!is_drive_letter(Path::new(path)), "{path}");
        }
    }

    fn volume(kind: VolumeKind, fs_type: &str) -> Volume {
        Volume {
            name: "STICK".into(),
            mount_point: "/media/me/STICK".into(),
            kind,
            fs_type: fs_type.into(),
            device: None,
            total: None,
            free: None,
            read_only: false,
        }
    }

    #[test]
    fn the_secondary_line_is_the_kind_and_the_file_system() {
        crate::english();
        assert_eq!(
            subtitle(&volume(VolumeKind::Removable, "ext4")),
            "Removable drive  ·  ext4"
        );
        assert_eq!(
            subtitle(&volume(VolumeKind::Fixed, "NTFS")),
            "Local drive  ·  NTFS"
        );
        assert_eq!(subtitle(&volume(VolumeKind::Optical, "")), "Optical disc");
        assert_eq!(
            subtitle(&volume(VolumeKind::Network, "cifs")),
            "Network drive  ·  cifs"
        );
    }
}
