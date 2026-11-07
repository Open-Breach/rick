//! The side panel: Home, Recent, Network, Trash, Pinned and Tree as sections
//! that fold independently, with the operations area at its foot. Which
//! sections are folded, the panel's width, its side and whether it shows at
//! all are part of the window's session state, so they come back on the
//! next start.
//!
//! Recent lists what changed lately under the home directory and the pinned
//! folders; Network the mounted volumes other than the system disk; Trash is
//! one row that opens the Trash view, `rick://trash`, in the focused pane,
//! where Restore and Empty trash live (SPEC.md "Places"); Tree a folder
//! tree following the focused pane. Home, Trash, the volumes and the whole
//! Pinned section are drop targets while a drag is in progress; a drop on
//! Trash trashes.

use std::path::PathBuf;

use app::{App, Command};
use ext::{ActionCtx, ActionId, Invocation};
use model::{PaneId, Section, SidePanel, SidePanelChange, VolumeKind, WindowId};
use types::{Entry, EntryKind, Location, ProviderId, VPath};

use crate::dnd::{self, Dnd, Target, Zone};
use crate::shell::Request;
use crate::{format, icons};

/// Draws the sections for the focused pane of `window`; navigation, pin and
/// fold changes go out as requests, drop zones into `dnd`.
pub fn show(
    ui: &mut egui::Ui,
    app: &App,
    window: WindowId,
    panel: &SidePanel,
    requests: &mut Vec<Request>,
    dnd: &mut Dnd,
) {
    let focused = app
        .model()
        .window(window)
        .map_or(app.model().focused, |w| w.focused);
    let current = app
        .model()
        .active_tab(focused)
        .map(|(_, tab)| tab.location.clone());
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for section in Section::ALL {
                // The model drives the open state; a click on the header
                // asks for the change, and the next frame shows it.
                let folded = panel.folded.contains(&section);
                let response = egui::CollapsingHeader::new(section.title())
                    .id_salt(("side-section", section.title()))
                    .open(Some(!folded))
                    .show(ui, |ui| {
                        body(ui, section, app, focused, current.as_ref(), requests, dnd);
                    });
                if response.header_response.clicked() {
                    requests.push(Request::Command(Command::SidePanel {
                        window,
                        change: SidePanelChange::Fold {
                            section,
                            folded: !folded,
                        },
                    }));
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
        Section::Home | Section::Recent | Section::Network | Section::Tree => return,
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
            let response = location_row(ui, icons::HOME, "Home", &home, current);
            if response.clicked() {
                requests.push(Request::Command(Command::Navigate {
                    pane: focused,
                    to: home.clone(),
                }));
            }
            folder_zone(ui, dnd, egui::Id::new("dnd-home"), response.rect, home);
        }
        Section::Recent => recent(ui, app, focused, requests),
        Section::Network => network(ui, app, focused, current, requests, dnd),
        Section::Trash => trash(ui, app, focused, current, requests),
        Section::Pinned => pinned(ui, app, focused, current, requests, dnd),
        Section::Tree => tree(ui, app, focused, current, requests),
    }
}

/// The Recent section: icon, name and parent folder per row, newest first;
/// click selects it in the focused pane, double-click opens it.
fn recent(ui: &mut egui::Ui, app: &App, focused: PaneId, requests: &mut Vec<Request>) {
    let entries = app.model().recent.entries();
    if entries.is_empty() {
        ui.weak("Nothing changed recently under Home or the pinned folders.");
        return;
    }
    for entry in entries {
        let parent = entry
            .path
            .parent()
            .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
        let response = ui
            .add(
                egui::Button::new(two_line_text(
                    ui,
                    icons::for_entry(entry),
                    &entry.name,
                    &parent,
                ))
                .frame(false)
                .min_size(egui::vec2(ui.available_width(), 0.0)),
            )
            .on_hover_text(entry.path.to_string_lossy());
        if response.double_clicked() {
            open_entry(entry, focused, requests);
        } else if response.clicked() {
            requests.push(Request::OpenContaining {
                pane: focused,
                entry: entry.clone(),
                new_tab: false,
            });
        }
        response.context_menu(|ui| {
            if ui.button("Open containing folder").clicked() {
                requests.push(Request::OpenContaining {
                    pane: focused,
                    entry: entry.clone(),
                    new_tab: false,
                });
                ui.close();
            }
            if ui.button("Open containing folder in new tab").clicked() {
                requests.push(Request::OpenContaining {
                    pane: focused,
                    entry: entry.clone(),
                    new_tab: true,
                });
                ui.close();
            }
            if ui.button("Open").clicked() {
                open_entry(entry, focused, requests);
                ui.close();
            }
        });
    }
}

/// Opens an entry from the side panel: a folder navigates the focused pane,
/// a file opens with the system default.
fn open_entry(entry: &Entry, focused: PaneId, requests: &mut Vec<Request>) {
    if entry.kind == EntryKind::Dir {
        requests.push(Request::Command(Command::Navigate {
            pane: focused,
            to: Location::new(entry.id.provider, entry.path.clone()),
        }));
    } else {
        let location = entry.path.parent().unwrap_or_else(|| entry.path.clone());
        requests.push(Request::RunAction {
            action: ActionId("core.open_default"),
            ctx: ActionCtx {
                pane: focused,
                location: Location::new(entry.id.provider, location),
                selection: vec![entry.clone()],
                invoked_from: Invocation::Menu,
            },
        });
    }
}

/// A row of two lines: the icon and name, then a weak second line.
fn two_line_text(ui: &egui::Ui, icon: &str, name: &str, below: &str) -> egui::WidgetText {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let small = egui::TextStyle::Small.resolve(ui.style());
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = ui.available_width() - 8.0;
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = true;
    job.append(
        &format!("{icon} {name}\n"),
        0.0,
        egui::TextFormat::simple(font, ui.visuals().text_color()),
    );
    job.append(
        below,
        0.0,
        egui::TextFormat::simple(small, ui.visuals().weak_text_color()),
    );
    job.into()
}

/// The Network section: the mounted volumes with name, mount point and free
/// space; click navigates the focused pane there.
fn network(
    ui: &mut egui::Ui,
    app: &App,
    focused: PaneId,
    current: Option<&Location>,
    requests: &mut Vec<Request>,
    dnd: &mut Dnd,
) {
    let volumes = &app.model().volumes;
    if volumes.is_empty() {
        ui.weak("No removable media or network mounts.");
        return;
    }
    for (index, volume) in volumes.iter().enumerate() {
        let location = Location::new(
            vfs_local::PROVIDER_ID,
            platform::paths::from_path(&volume.mount_point),
        );
        let icon = match volume.kind {
            VolumeKind::Removable => icons::USB,
            VolumeKind::Network => icons::NETWORK,
            VolumeKind::Optical => icons::DISC,
            VolumeKind::Fixed => icons::DISK,
        };
        let space = match (volume.free, volume.total) {
            (Some(free), Some(total)) => format!(
                "{} · {} free of {}",
                volume.mount_point.display(),
                format::size(Some(free)),
                format::size(Some(total))
            ),
            _ => volume.mount_point.display().to_string(),
        };
        let here = current == Some(&location);
        let response = ui
            .add(
                egui::Button::new(two_line_text(ui, icon, &volume.name, &space))
                    .frame(false)
                    .selected(here)
                    .min_size(egui::vec2(ui.available_width(), 0.0)),
            )
            .on_hover_text(volume.mount_point.display().to_string());
        if response.clicked() {
            requests.push(Request::Command(Command::Navigate {
                pane: focused,
                to: location.clone(),
            }));
        }
        folder_zone(
            ui,
            dnd,
            egui::Id::new(("dnd-volume", index)),
            response.rect,
            location,
        );
    }
}

/// The Trash section: one row that opens the Trash view, `rick://trash`,
/// in the focused pane; Restore and Empty trash live in the view. Without
/// the trash provider plugin the row says so instead.
fn trash(
    ui: &mut egui::Ui,
    app: &App,
    focused: PaneId,
    current: Option<&Location>,
    requests: &mut Vec<Request>,
) {
    let view = trash_view();
    if app.mounts().view(TRASH_VIEW).is_none() {
        ui.weak("The Trash view needs the vfs-trash plugin.");
        return;
    }
    let response = location_row(ui, icons::TRASH, "Trash", &view, current);
    if response.clicked() {
        requests.push(Request::Command(Command::Navigate {
            pane: focused,
            to: view.clone(),
        }));
    }
    if response.middle_clicked() {
        requests.push(Request::NewTabAt(view.clone()));
    }
    response.context_menu(|ui| {
        if ui.button("Open in new tab").clicked() {
            requests.push(Request::NewTabAt(view.clone()));
            ui.close();
        }
    });
}

/// The name of the Trash view under the `rick` provider.
const TRASH_VIEW: &str = "trash";

/// The location of the Trash view: `rick://trash`.
#[must_use]
pub fn trash_view() -> Location {
    Location::new(ProviderId::RICK, format!("/{TRASH_VIEW}"))
}

/// The Pinned section.
fn pinned(
    ui: &mut egui::Ui,
    app: &App,
    focused: PaneId,
    current: Option<&Location>,
    requests: &mut Vec<Request>,
    dnd: &mut Dnd,
) {
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
        // The unpin button takes its place first; the name gets the rest and
        // is truncated to it, so a long name cannot widen the panel.
        let response = ui
            .horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(icons::UNPIN)
                        .on_hover_text("Unpin")
                        .clicked()
                    {
                        requests.push(Request::Command(Command::Unpin(location.clone())));
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        location_row(ui, icons::PIN, &name, &location, current)
                    })
                    .inner
                })
                .inner
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

/// The Tree section: a folder tree from the root of the focused pane's
/// volume, following its location unless unlinked; a node navigates, its
/// chevron opens or closes it.
fn tree(
    ui: &mut egui::Ui,
    app: &App,
    focused: PaneId,
    current: Option<&Location>,
    requests: &mut Vec<Request>,
) {
    let tree = &app.model().folder_tree;
    ui.horizontal(|ui| {
        let glyph = if tree.linked {
            icons::LINKED
        } else {
            icons::UNLINKED
        };
        if ui
            .add(egui::Button::new(glyph).small().selected(tree.linked))
            .on_hover_text(if tree.linked {
                "Following the focused pane; click to unlink"
            } else {
                "Unlinked; click to follow the focused pane"
            })
            .clicked()
        {
            requests.push(Request::Command(Command::FolderTreeLink(!tree.linked)));
        }
    });
    let Some(root) = tree.root.clone() else {
        ui.weak("The tree follows the focused pane.");
        return;
    };
    let label = crate::paths::title(&root.path);
    tree_node(ui, app, &root, &label, 0, focused, current, requests);
}

/// One node of the folder tree and, when open, its children.
#[allow(clippy::too_many_arguments)]
fn tree_node(
    ui: &mut egui::Ui,
    app: &App,
    location: &Location,
    label: &str,
    depth: usize,
    focused: PaneId,
    current: Option<&Location>,
    requests: &mut Vec<Request>,
) {
    if depth > 48 {
        return;
    }
    let tree = &app.model().folder_tree;
    let open = tree.expanded.contains(&location.path);
    let here = current.is_some_and(|c| c.path == location.path);
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 14.0);
        let chevron = ui.add(
            egui::Button::new(egui::RichText::new(if open {
                icons::TREE_OPEN
            } else {
                icons::TREE_CLOSED
            }))
            .frame(false)
            .small(),
        );
        if chevron.clicked() {
            requests.push(Request::Command(Command::FolderTreeToggle(
                location.path.clone(),
            )));
        }
        let icon = if open {
            icons::FOLDER_OPEN
        } else {
            icons::FOLDER
        };
        // Truncated to the panel: a long name must not widen it.
        let node = ui
            .add(egui::Button::selectable(here, format!("{icon} {label}")).truncate())
            .on_hover_text(location.path.to_string_lossy());
        if node.clicked() {
            requests.push(Request::Command(Command::Navigate {
                pane: focused,
                to: location.clone(),
            }));
        }
        if node.double_clicked() {
            requests.push(Request::Command(Command::FolderTreeToggle(
                location.path.clone(),
            )));
        }
    });
    if !open {
        return;
    }
    match tree.children(&location.path) {
        Some(children) => {
            for child in children {
                let child_location = Location::new(location.provider, child.path.clone());
                tree_node(
                    ui,
                    app,
                    &child_location,
                    &child.name,
                    depth + 1,
                    focused,
                    current,
                    requests,
                );
            }
        }
        None => {
            ui.horizontal(|ui| {
                ui.add_space((depth + 1) as f32 * 14.0 + 20.0);
                if tree.loading.contains(&location.path) {
                    ui.spinner();
                } else {
                    ui.weak("…");
                }
            });
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
    ui.add(egui::Button::selectable(here, format!("{icon} {label}")).truncate())
        .on_hover_text(location.path.to_string())
}

/// The OS path of a location on the local provider.
fn local_path(loc: &Location) -> Option<PathBuf> {
    (loc.provider == vfs_local::PROVIDER_ID)
        .then(|| platform::paths::to_path(&loc.path).ok())
        .flatten()
}

/// The path a tree node stands for, for tests.
#[must_use]
pub fn node_path(location: &Location) -> &VPath {
    &location.path
}
