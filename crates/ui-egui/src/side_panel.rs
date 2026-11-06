//! The side panel: Home, Recent, Network, Trash and Pinned as sections that
//! fold independently, with the operations area at its foot. Which sections
//! are folded, the panel's width, its side and whether it shows at all are
//! settings, so they come back on the next start.
//!
//! Recent, Network and Trash are sections that say what they will hold;
//! their contents come with the change feed, the network providers and the
//! trash location in later issues.

use std::path::PathBuf;

use app::{App, Command};
use model::{PaneId, Section, SettingsChange};
use types::Location;

use crate::shell::Request;

/// Draws the sections for the focused pane; navigation and pin changes go
/// out as requests.
pub fn show(ui: &mut egui::Ui, app: &App, focused: PaneId, requests: &mut Vec<Request>) {
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
                        body(ui, section, app, focused, current.as_ref(), requests);
                    });
                if response.header_response.clicked() {
                    requests.push(Request::Command(Command::Settings(
                        SettingsChange::SidePanelFold {
                            section,
                            folded: !folded,
                        },
                    )));
                }
            }
        });
}

/// "Pin current folder" for the focused pane's location, enabled when it is
/// a local folder that is not pinned yet.
fn pin_button(
    ui: &mut egui::Ui,
    app: &App,
    current: Option<&Location>,
    requests: &mut Vec<Request>,
) {
    let Some(path) = current.and_then(local_path) else {
        return;
    };
    let pinned = app.model().settings.pinned.contains(&path);
    if ui
        .add_enabled(!pinned, egui::Button::new("Pin current folder").small())
        .on_hover_text(if pinned {
            "This folder is pinned".to_owned()
        } else {
            path.display().to_string()
        })
        .clicked()
    {
        requests.push(Request::Command(Command::Settings(SettingsChange::Pin(
            path,
        ))));
    }
}

fn body(
    ui: &mut egui::Ui,
    section: Section,
    app: &App,
    focused: PaneId,
    current: Option<&Location>,
    requests: &mut Vec<Request>,
) {
    match section {
        Section::Home => {
            let home = app.platform().home_dir();
            location_row(
                ui,
                crate::icons::HOME,
                "Home",
                &home,
                focused,
                current,
                requests,
            );
        }
        Section::Recent => coming(ui, "Files and folders recently changed anywhere."),
        Section::Network => coming(ui, "Shares, hosts and mounted network file systems."),
        Section::Trash => coming(ui, "The platform trash, with restore and empty."),
        Section::Pinned => {
            pin_button(ui, app, current, requests);
            let pinned = app.model().settings.pinned.clone();
            if pinned.is_empty() {
                ui.weak("Nothing pinned yet. Pin the current folder with the button above.");
            }
            for path in pinned {
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                ui.horizontal(|ui| {
                    location_row(
                        ui,
                        crate::icons::PIN,
                        &name,
                        &path,
                        focused,
                        current,
                        requests,
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button(crate::icons::CLOSE)
                            .on_hover_text("Unpin")
                            .clicked()
                        {
                            requests.push(Request::Command(Command::Settings(
                                SettingsChange::Unpin(path.clone()),
                            )));
                        }
                    });
                });
            }
        }
    }
}

/// A clickable location, highlighted when the focused pane is there.
fn location_row(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    path: &std::path::Path,
    focused: PaneId,
    current: Option<&Location>,
    requests: &mut Vec<Request>,
) {
    let here = current.and_then(local_path).is_some_and(|c| c == path);
    if ui
        .selectable_label(here, format!("{icon} {label}"))
        .on_hover_text(path.display().to_string())
        .clicked()
        && let Some(current) = current
    {
        let to = Location::new(current.provider, platform::paths::from_path(path));
        requests.push(Request::Command(Command::Navigate { pane: focused, to }));
    }
}

fn coming(ui: &mut egui::Ui, what: &str) {
    ui.weak(what);
    ui.weak("Coming in a later issue.");
}

/// The OS path of a location on the local provider.
fn local_path(loc: &Location) -> Option<PathBuf> {
    (loc.provider.0 == "local")
        .then(|| platform::paths::to_path(&loc.path).ok())
        .flatten()
}
