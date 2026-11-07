//! The side panel: Home, Recent, Network, Trash, Pinned and Tree as sections
//! that fold independently, with the operations area at its foot. Which
//! sections are folded, the panel's width, its side and whether it shows at
//! all are settings, so they come back on the next start.
//!
//! Recent lists what changed lately under the home directory and the pinned
//! folders; Network the mounted volumes other than the system disk; Trash
//! the trash's contents with restore, delete and empty; Tree a folder tree
//! following the focused pane. Home, Trash, the volumes and the whole Pinned
//! section are drop targets while a drag is in progress.

use std::collections::BTreeSet;
use std::path::PathBuf;

use app::{App, Command};
use ext::{ActionCtx, ActionId, Invocation};
use model::{PaneId, Section, TrashedItem, VolumeKind};
use types::{Entry, EntryKind, Location, VPath};

use crate::dnd::{self, Dnd, Target, Zone};
use crate::shell::Request;
use crate::{format, icons};

/// The panel's own state: what is selected in the Trash section.
#[derive(Debug, Default)]
pub struct SidePanelState {
    /// Selected trash rows, by trashed path.
    pub trash_selection: BTreeSet<PathBuf>,
    /// Where a range selection in the trash starts.
    trash_anchor: Option<PathBuf>,
    /// The trash list took the last click, so Delete and Enter act on it.
    pub trash_focused: bool,
    /// The Trash section's rectangle in the last frame.
    trash_rect: Option<egui::Rect>,
}

impl SidePanelState {
    /// The trash rows selected, as the items.
    fn selected_items(&self, app: &App) -> Vec<TrashedItem> {
        app.model()
            .trash
            .items
            .iter()
            .filter(|item| self.trash_selection.contains(&item.trashed))
            .cloned()
            .collect()
    }

    /// Delete and Enter while the trash list has the keyboard: Delete
    /// permanently and Restore trash items. Returns whether a key was taken.
    pub fn handle_keys(
        &mut self,
        ctx: &egui::Context,
        app: &App,
        requests: &mut Vec<Request>,
    ) -> bool {
        if self.trash_selection.is_empty() {
            return false;
        }
        let (delete, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            )
        });
        let action = if delete {
            "core.purge_trash"
        } else if enter {
            "core.restore_trash"
        } else {
            return false;
        };
        requests.push(trash_action(app, self, action));
        true
    }
}

/// The request that runs a trash action on the selected trash rows.
fn trash_action(app: &App, state: &SidePanelState, action: &'static str) -> Request {
    Request::RunAction {
        action: ActionId(action),
        ctx: ActionCtx {
            pane: app.model().focused,
            location: ops::trash_location(),
            selection: state
                .selected_items(app)
                .iter()
                .map(ops::trash_entry)
                .collect(),
            invoked_from: Invocation::ContextMenu,
        },
    }
}

/// Draws the sections for the focused pane; navigation and pin changes go
/// out as requests, drop zones into `dnd`.
pub fn show(
    ui: &mut egui::Ui,
    app: &App,
    focused: PaneId,
    state: &mut SidePanelState,
    requests: &mut Vec<Request>,
    dnd: &mut Dnd,
) {
    let settings = &app.model().settings;
    let current = app
        .model()
        .active_tab(focused)
        .map(|(_, tab)| tab.location.clone());
    // A click anywhere but the trash list takes the keys away from it.
    if state.trash_focused
        && ui.input(|i| i.pointer.any_pressed())
        && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        && !state.trash_rect.is_some_and(|r| r.contains(pos))
    {
        state.trash_focused = false;
    }
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
                        body(
                            ui,
                            section,
                            app,
                            focused,
                            current.as_ref(),
                            state,
                            requests,
                            dnd,
                        );
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
                if section == Section::Trash {
                    state.trash_rect = Some(whole);
                }
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

#[allow(clippy::too_many_arguments)]
fn body(
    ui: &mut egui::Ui,
    section: Section,
    app: &App,
    focused: PaneId,
    current: Option<&Location>,
    state: &mut SidePanelState,
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
        Section::Trash => trash(ui, app, state, requests),
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

/// The Trash section: an Empty trash button in the header row, then the
/// items with name, original folder and deletion date, newest first, with
/// multi-select, a context menu and the keyboard.
// One list, top to bottom.
#[allow(clippy::too_many_lines)]
fn trash(ui: &mut egui::Ui, app: &App, state: &mut SidePanelState, requests: &mut Vec<Request>) {
    let view = &app.model().trash;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !view.items.is_empty(),
                egui::Button::new(format!("{} Empty trash", icons::TRASH)).small(),
            )
            .clicked()
        {
            requests.push(Request::RunAction {
                action: ActionId("core.empty_trash"),
                ctx: ActionCtx {
                    pane: app.model().focused,
                    location: ops::trash_location(),
                    selection: Vec::new(),
                    invoked_from: Invocation::Menu,
                },
            });
        }
        if !state.trash_selection.is_empty()
            && ui
                .small_button(format!("{} Restore", icons::UNTRASH))
                .on_hover_text("Restore trash items")
                .clicked()
        {
            requests.push(trash_action(app, state, "core.restore_trash"));
        }
    });
    if let Some(error) = &view.error {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!("Trash not read: {error}"),
        );
    }
    if !view.listed {
        ui.weak("Items trashed elsewhere are not listed; these are the ones Rick trashed.");
    }
    if view.items.is_empty() {
        ui.weak("The trash is empty.");
        state.trash_selection.clear();
        return;
    }
    // Rows that left the trash leave the selection.
    state
        .trash_selection
        .retain(|p| view.items.iter().any(|i| i.trashed == *p));
    let now = std::time::SystemTime::now();
    let items = view.items.clone();
    for item in &items {
        let name = item.original.file_name().map_or_else(
            || item.original.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let folder = item
            .original
            .parent()
            .map_or_else(String::new, |p| p.display().to_string());
        let (date, full_date) = format::date(Some(item.deleted_at), now);
        let is_dir = item.trashed.is_dir();
        let icon = if is_dir {
            icons::FOLDER
        } else {
            icons::by_extension(&name)
        };
        let selected = state.trash_selection.contains(&item.trashed);
        let response = ui
            .add(
                egui::Button::new(two_line_text(
                    ui,
                    icon,
                    &name,
                    &format!("{folder} · {date}"),
                ))
                .frame(false)
                .selected(selected)
                .min_size(egui::vec2(ui.available_width(), 0.0)),
            )
            .on_hover_text(format!("{}\nDeleted {full_date}", item.original.display()));
        if response.clicked() || (response.secondary_clicked() && !selected) {
            state.trash_focused = true;
            let modifiers = ui.input(|i| i.modifiers);
            if modifiers.command && response.clicked() {
                if !state.trash_selection.remove(&item.trashed) {
                    state.trash_selection.insert(item.trashed.clone());
                }
            } else if modifiers.shift && response.clicked() {
                let anchor = state
                    .trash_anchor
                    .as_ref()
                    .and_then(|a| items.iter().position(|i| i.trashed == *a));
                let here = items.iter().position(|i| i.trashed == item.trashed);
                if let (Some(a), Some(b)) = (anchor, here) {
                    state.trash_selection = items[a.min(b)..=a.max(b)]
                        .iter()
                        .map(|i| i.trashed.clone())
                        .collect();
                }
            } else {
                state.trash_selection.clear();
                state.trash_selection.insert(item.trashed.clone());
                state.trash_anchor = Some(item.trashed.clone());
            }
        }
        response.context_menu(|ui| {
            let restore = if state.trash_selection.len() > 1 {
                "core.restore_trash"
            } else {
                "core.restore"
            };
            if ui
                .button(if state.trash_selection.len() > 1 {
                    "Restore trash items"
                } else {
                    "Restore"
                })
                .clicked()
            {
                requests.push(trash_action(app, state, restore));
                ui.close();
            }
            if ui.button("Delete permanently").clicked() {
                requests.push(trash_action(app, state, "core.purge_trash"));
                ui.close();
            }
        });
    }
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
        let response = ui
            .horizontal(|ui| {
                let response = location_row(ui, icons::PIN, &name, &location, current);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(icons::UNPIN)
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
        let node = ui
            .selectable_label(here, format!("{icon} {label}"))
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
    ui.selectable_label(here, format!("{icon} {label}"))
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
