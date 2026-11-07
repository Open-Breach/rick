//! Menus: the burger menu and the context menu built from the action
//! registry.

use ext::{ActionCtx, ActionGroup, ActionId, Availability, Invocation};
use types::Entry;

use crate::keys::Intent;
use crate::shell::{PaneCx, Request};

/// The burger menu at the top of the window.
pub fn burger(ui: &mut egui::Ui, requests: &mut Vec<Request>, pane: model::PaneId) {
    ui.menu_button(crate::icons::MENU, |ui| {
        if ui.button("Settings…").clicked() {
            requests.push(Request::Intent {
                intent: Intent::Settings,
                pane,
            });
            ui.close();
        }
        if ui.button("Keyboard shortcuts…").clicked() {
            requests.push(Request::Intent {
                intent: Intent::Shortcuts,
                pane,
            });
            ui.close();
        }
        ui.separator();
        if ui.button("About Rick").clicked() {
            requests.push(Request::About);
            ui.close();
        }
    });
}

/// The action context for the pane: the selection, or `only` when the menu
/// was opened on an entry outside the selection.
#[must_use]
pub fn context_for(cx: &PaneCx<'_>, only: Option<Entry>) -> ActionCtx {
    let selection = match only {
        Some(entry) => vec![entry],
        None => cx.tab.selected().cloned().collect(),
    };
    ActionCtx {
        pane: cx.pane,
        location: cx.tab.location.clone(),
        selection,
        invoked_from: Invocation::ContextMenu,
    }
}

/// The spec's groups, as action ids in order.
const GROUPS: &[(&str, &[&str])] = &[
    (
        "Open",
        &[
            "core.open_default",
            "core.open_containing",
            "core.open_containing_tab",
            "core.pin",
            "core.unpin",
        ],
    ),
    (
        "Edit",
        &[
            "core.cut",
            "core.copy",
            "core.paste",
            "core.duplicate",
            "core.rename",
        ],
    ),
    ("Delete", &["core.trash", "core.delete"]),
    ("Create", &["core.new_folder", "core.new_file"]),
    ("Share", &["core.copy_path", "core.reveal"]),
];

/// The context menu for `entry` (or the empty area when `None`): the
/// spec's groups from the registered actions, then extension groups, then
/// Info. Entries that do not apply are hidden, as the spec asks.
// One menu, top to bottom, in the spec's order.
#[allow(clippy::too_many_lines)]
pub fn context_menu(
    ui: &mut egui::Ui,
    cx: &mut PaneCx<'_>,
    action_ctx: &ActionCtx,
    entry: Option<&Entry>,
) {
    ui.set_min_width(220.0);
    let actions = cx.app.registry().actions();
    let mut any = false;

    // Open: navigation first, then the registered open action.
    if let Some(entry) = entry
        && entry.kind == types::EntryKind::Dir
    {
        {
            if ui.button("Open").clicked() {
                cx.requests.push(Request::OpenEntry {
                    pane: cx.pane,
                    id: entry.id,
                });
                ui.close();
            }
            if ui.button("Open in new tab").clicked() {
                let location = types::Location::new(
                    cx.tab.location.provider,
                    cx.tab.location.path.join(entry.name.as_bytes()),
                );
                cx.requests.push(Request::NewTabAt(location));
                ui.close();
            }
            any = true;
        }
    }

    for (group, ids) in GROUPS {
        let mut shown = false;
        for id in *ids {
            let Some(action) = actions.iter().find(|a| a.id().0 == *id) else {
                continue;
            };
            if action.availability(action_ctx) != Availability::Enabled {
                continue;
            }
            if !shown && any {
                ui.separator();
            }
            shown = true;
            any = true;
            let intent = Intent::Action(action.id());
            let mut button = egui::Button::new(action.label());
            if let Some(text) = cx.keys.text_for(intent) {
                button = button.shortcut_text(text);
            }
            if ui.add(button).clicked() {
                cx.requests.push(Request::RunAction {
                    action: action.id(),
                    ctx: action_ctx.clone(),
                });
                ui.close();
            }
        }
        let _ = group;
    }

    // Extensions, one submenu per group.
    let mut extension_groups: Vec<&'static str> = actions
        .iter()
        .filter_map(|a| match a.group() {
            ActionGroup::Extension(name) => Some(name),
            ActionGroup::Core => None,
        })
        .collect();
    extension_groups.sort_unstable();
    extension_groups.dedup();
    for name in extension_groups {
        let members: Vec<ActionId> = actions
            .iter()
            .filter(|a| {
                a.group() == ActionGroup::Extension(name)
                    && a.availability(action_ctx) == Availability::Enabled
            })
            .map(|a| a.id())
            .collect();
        if members.is_empty() {
            continue;
        }
        if any {
            ui.separator();
        }
        any = true;
        ui.menu_button(name, |ui| {
            for id in members {
                let Some(action) = actions.iter().find(|a| a.id() == id) else {
                    continue;
                };
                if ui.button(action.label()).clicked() {
                    cx.requests.push(Request::RunAction {
                        action: id,
                        ctx: action_ctx.clone(),
                    });
                    ui.close();
                }
            }
        });
    }

    // Info.
    if !action_ctx.selection.is_empty() {
        if any {
            ui.separator();
        }
        let mut button = egui::Button::new("Properties");
        if let Some(text) = cx.keys.text_for(Intent::Inspect) {
            button = button.shortcut_text(text);
        }
        if ui.add(button).clicked() {
            cx.requests.push(Request::Intent {
                intent: Intent::Inspect,
                pane: cx.pane,
            });
            ui.close();
        }
    }
}
