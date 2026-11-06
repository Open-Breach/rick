//! The side panel: Home, Recent, Network, Trash and Pinned as sections that
//! fold independently, with the operations area at its foot. Which sections
//! are folded, the panel's width, its side and whether it shows at all are
//! settings, so they come back on the next start.
//!
//! Recent, Network and Trash are sections that say what they will hold;
//! their contents come with the change feed, the network providers and the
//! trash location in later issues. Home, Trash and the whole Pinned section
//! are drop targets while a drag is in progress.

use std::path::PathBuf;

use app::{App, Command};
use model::{PaneId, Section};
use types::Location;

use crate::dnd::{self, Dnd, Target, Zone};
use crate::shell::Request;

/// Draws the sections for the focused pane; navigation and pin changes go
/// out as requests, drop zones into `dnd`.
pub fn show(
    ui: &mut egui::Ui,
    app: &App,
    focused: PaneId,
    requests: &mut Vec<Request>,
    dnd: &mut Dnd,
) {
    let settings = &app.model().settings;
    let current = app
        .model()
        .active_tab(focused)
        .map(|(_, tab)| tab.location.clone());
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for section in Section::ALL {
                // The settings drive the open state; a click on the header
                // asks for the change, and the next frame shows it.
                let folded = settings.side_panel.folded.contains(&section);
                let response = egui::CollapsingHeader::new(section.title())
                    .id_salt(("side-section", section.title()))
                    .open(Some(!folded))
                    .show(ui, |ui| {
                        body(ui, section, app, focused, current.as_ref(), requests, dnd);
                    });
                if response.header_response.clicked() {
                    requests.push(Request::Command(Command::Settings(
                        model::SettingsChange::SidePanelFold {
                            section,
                            folded: !folded,
                        },
                    )));
                }
                let whole = response
                    .body_response
                    .as_ref()
                    .map_or(response.header_response.rect, |b| {
                        response.header_response.rect.union(b.rect)
                    });
                section_zone(ui, section, whole, dnd);
            }
        });
}

/// The section as a drop target: Trash trashes, Pinned pins at the end.
/// Registered after the entries inside it, which are smaller and win.
fn section_zone(ui: &egui::Ui, section: Section, rect: egui::Rect, dnd: &mut Dnd) {
    if !dnd.active() {
        return;
    }
    let target = match section {
        Section::Trash => Target::Trash,
        Section::Pinned => Target::Pin { at: None },
        Section::Home | Section::Recent | Section::Network => return,
    };
    if dnd.zone(Zone {
        key: egui::Id::new(("dnd-section", section.title())),
        rect,
        target,
    }) {
        dnd::highlight(ui.painter(), ui.visuals(), rect);
    }
}

/// "Pin current folder" for the focused pane's location, enabled when it is
/// not pinned yet.
fn pin_button(
    ui: &mut egui::Ui,
    app: &App,
    current: Option<&Location>,
    requests: &mut Vec<Request>,
) {
    let Some(location) = current else {
        return;
    };
    let pinned = app.model().settings.pinned.contains(location);
    if ui
        .add_enabled(!pinned, egui::Button::new("Pin current folder").small())
        .on_hover_text(if pinned {
            "This folder is pinned".to_owned()
        } else {
            location.path.to_string()
        })
        .clicked()
    {
        requests.push(Request::Command(Command::Pin {
            items: vec![location.clone()],
            at: None,
        }));
    }
}

fn body(
    ui: &mut egui::Ui,
    section: Section,
    app: &App,
    focused: PaneId,
    current: Option<&Location>,
    requests: &mut Vec<Request>,
    dnd: &mut Dnd,
) {
    match section {
        Section::Home => {
            let home = app.platform().home_dir();
            let home = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&home));
            let response = location_row(ui, crate::icons::HOME, "Home", &home, current);
            if response.clicked() {
                requests.push(Request::Command(Command::Navigate {
                    pane: focused,
                    to: home.clone(),
                }));
            }
            folder_zone(ui, dnd, egui::Id::new("dnd-home"), response.rect, home);
        }
        Section::Recent => coming(ui, "Files and folders recently changed anywhere."),
        Section::Network => coming(ui, "Shares, hosts and mounted network file systems."),
        Section::Trash => coming(ui, "The platform trash, with restore and empty."),
        Section::Pinned => {
            pin_button(ui, app, current, requests);
            let pinned = app.model().settings.pinned.clone();
            if pinned.is_empty() {
                ui.weak("Nothing pinned yet. Pin the current folder with the button above, or drop entries here.");
            }
            for (index, location) in pinned.into_iter().enumerate() {
                let name = location.path.file_name().map_or_else(
                    || location.path.to_string(),
                    |n| String::from_utf8_lossy(n).into_owned(),
                );
                let response = ui
                    .horizontal(|ui| {
                        let response =
                            location_row(ui, crate::icons::PIN, &name, &location, current);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button(crate::icons::UNPIN)
                                .on_hover_text("Unpin")
                                .clicked()
                            {
                                requests.push(Request::Command(Command::Unpin(location.clone())));
                            }
                        });
                        response
                    })
                    .inner;
                if response.clicked() {
                    open_pinned(&location, focused, requests);
                }
                response.context_menu(|ui| {
                    if ui.button("Unpin").clicked() {
                        requests.push(Request::Command(Command::Unpin(location.clone())));
                        ui.close();
                    }
                    if ui.button("Open in new tab").clicked() {
                        requests.push(Request::NewTabAt(folder_of(&location)));
                        ui.close();
                    }
                });
                // A drop on a pinned entry inserts before it.
                if dnd.active()
                    && dnd.zone(Zone {
                        key: egui::Id::new(("dnd-pinned", index)),
                        rect: response.rect,
                        target: Target::Pin { at: Some(index) },
                    })
                {
                    dnd::highlight(ui.painter(), ui.visuals(), response.rect);
                }
            }
        }
    }
}

/// A pinned folder opens in the focused pane; a pinned file opens its folder
/// with the file selected. Whether a local path is a folder is one metadata
/// call; a location on another provider is taken for a folder.
fn open_pinned(location: &Location, focused: PaneId, requests: &mut Vec<Request>) {
    let is_file = local_path(location)
        .is_some_and(|path| std::fs::symlink_metadata(&path).is_ok_and(|meta| !meta.is_dir()));
    if is_file && let Some(name) = location.path.file_name() {
        requests.push(Request::Command(Command::Navigate {
            pane: focused,
            to: folder_of(location),
        }));
        requests.push(Request::SelectNamed {
            pane: focused,
            name: name.to_vec(),
        });
    } else {
        requests.push(Request::Command(Command::Navigate {
            pane: focused,
            to: location.clone(),
        }));
    }
}

/// The folder a pinned location opens: itself for a folder, its parent for
/// a file.
fn folder_of(location: &Location) -> Location {
    let is_file = local_path(location)
        .is_some_and(|path| std::fs::symlink_metadata(&path).is_ok_and(|meta| !meta.is_dir()));
    match (is_file, location.path.parent()) {
        (true, Some(parent)) => Location::new(location.provider, parent),
        _ => location.clone(),
    }
}

/// A folder entry of the panel as a drop target.
fn folder_zone(ui: &egui::Ui, dnd: &mut Dnd, key: egui::Id, rect: egui::Rect, to: Location) {
    if dnd.active()
        && dnd.zone(Zone {
            key,
            rect,
            target: Target::Folder {
                to,
                device: None,
                spring: None,
            },
        })
    {
        dnd::highlight(ui.painter(), ui.visuals(), rect);
    }
}

/// A clickable location, highlighted when the focused pane is there.
fn location_row(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    location: &Location,
    current: Option<&Location>,
) -> egui::Response {
    let here = current == Some(location);
    ui.selectable_label(here, format!("{icon} {label}"))
        .on_hover_text(location.path.to_string())
}

fn coming(ui: &mut egui::Ui, what: &str) {
    ui.weak(what);
    ui.weak("Coming in a later issue.");
}

/// The OS path of a location on the local provider.
fn local_path(loc: &Location) -> Option<PathBuf> {
    (loc.provider == vfs_local::PROVIDER_ID)
        .then(|| platform::paths::to_path(&loc.path).ok())
        .flatten()
}
