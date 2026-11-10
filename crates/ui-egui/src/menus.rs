//! Menus: the burger menu and the context menu built from the action
//! registry.

use std::sync::Arc;

use app::{App, Command};
use ext::t;
use ext::{Action, ActionCtx, ActionGroup, Availability, Invocation};
use types::Entry;

use egui::Align;

use crate::icons;
use crate::keys::Intent;
use crate::shell::{PaneCx, Request};
use crate::theme::{self, ICON, TEXT_BODY, TEXT_SECONDARY};

/// The burger menu at the top of the window: New window, the Layouts
/// submenu, the actions extensions put in the application menu
/// ([`Action::app_menu`]) under one submenu per extension, Settings,
/// Keyboard shortcuts, Extensions, About.
pub fn burger(ui: &mut egui::Ui, requests: &mut Vec<Request>, pane: model::PaneId, app: &App) {
    ui.menu_button(crate::icons::MENU, |ui| {
        if ui.button(t!("menu-new-window")).clicked() {
            requests.push(Request::Intent {
                intent: Intent::NewWindow,
                pane,
            });
            ui.close();
        }
        ui.menu_button(t!("menu-layouts"), |ui| {
            layouts_menu(ui, requests, pane, app);
        });
        if let Some((ctx, groups)) = app_menu_actions(app, pane) {
            ui.separator();
            for (group, members) in groups {
                match group {
                    None => {
                        for action in members {
                            action_item(ui, requests, action.as_ref(), &ctx);
                        }
                    }
                    Some(name) => {
                        ui.menu_button(name, |ui| {
                            for action in &members {
                                action_item(ui, requests, action.as_ref(), &ctx);
                            }
                        });
                    }
                }
            }
        }
        ui.separator();
        if ui.button(t!("settings-ellipsis")).clicked() {
            requests.push(Request::Intent {
                intent: Intent::Settings,
                pane,
            });
            ui.close();
        }
        if ui.button(t!("keyboard-shortcuts-ellipsis")).clicked() {
            requests.push(Request::Intent {
                intent: Intent::Shortcuts,
                pane,
            });
            ui.close();
        }
        if ui.button(t!("extensions-ellipsis")).clicked() {
            requests.push(Request::Intent {
                intent: Intent::Extensions,
                pane,
            });
            ui.close();
        }
        ui.separator();
        if ui.button(t!("about-rick")).clicked() {
            requests.push(Request::About);
            ui.close();
        }
    });
}

/// The actions that asked for the application menu and apply to the
/// focused pane, with the context they run on: the core ones first, then
/// one group per extension, by name. `None` when the pane shows no
/// entries (a console, a comparison) or nothing applies.
#[allow(clippy::type_complexity)]
fn app_menu_actions(
    app: &App,
    pane: model::PaneId,
) -> Option<(ActionCtx, Vec<(Option<&'static str>, Vec<Arc<dyn Action>>)>)> {
    let ctx = app.action_ctx(pane, Invocation::Menu)?;
    let mut groups: Vec<(Option<&'static str>, Vec<Arc<dyn Action>>)> = Vec::new();
    for action in app.registry().actions() {
        if !action.app_menu() || action.availability(&ctx) != Availability::Enabled {
            continue;
        }
        let group = match action.group() {
            ActionGroup::Core => None,
            ActionGroup::Extension(name) => Some(name),
        };
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, members)) => members.push(Arc::clone(action)),
            None => groups.push((group, vec![Arc::clone(action)])),
        }
    }
    groups.sort_by_key(|(group, _)| *group);
    (!groups.is_empty()).then_some((ctx, groups))
}

/// One action as a menu item, run on `ctx` from the application menu.
fn action_item(
    ui: &mut egui::Ui,
    requests: &mut Vec<Request>,
    action: &dyn Action,
    ctx: &ActionCtx,
) {
    if ui.button(action.label()).clicked() {
        requests.push(Request::RunAction {
            action: action.id(),
            ctx: ctx.clone(),
        });
        ui.close();
    }
}

/// The Layouts submenu: "Continue where I left off" first, then one
/// submenu per saved layout with Switch, Set as start layout, Rename and
/// Delete, then Save layout as….
fn layouts_menu(ui: &mut egui::Ui, requests: &mut Vec<Request>, pane: model::PaneId, app: &App) {
    ui.set_min_width(200.0);
    if ui.button(t!("continue-where-i-left-off")).clicked() {
        requests.push(Request::Intent {
            intent: Intent::LastSession,
            pane,
        });
        ui.close();
    }
    let start = app.model().settings.session.start_layout();
    for (index, name) in app.layouts().iter().enumerate() {
        let Ok(index) = u16::try_from(index) else {
            break;
        };
        let title = if start == Some(name.as_str()) {
            format!("{name} (start layout)")
        } else {
            name.clone()
        };
        ui.menu_button(title, |ui| {
            if ui.button(t!("menu-switch")).clicked() {
                requests.push(Request::Intent {
                    intent: Intent::Layout(index),
                    pane,
                });
                ui.close();
            }
            let is_start = start == Some(name.as_str());
            if ui
                .add_enabled(!is_start, egui::Button::new(t!("menu-set-as-start-layout")))
                .clicked()
            {
                requests.push(Request::Command(Command::Settings(
                    model::SettingsChange::StartLayout(Some(name.clone())),
                )));
                ui.close();
            }
            if ui.button(t!("rename-ellipsis")).clicked() {
                requests.push(Request::RenameLayout(name.clone()));
                ui.close();
            }
            if ui.button(t!("delete")).clicked() {
                requests.push(Request::Command(Command::DeleteLayout(name.clone())));
                ui.close();
            }
        });
    }
    ui.separator();
    if ui.button(t!("menu-save-layout-as")).clicked() {
        requests.push(Request::Intent {
            intent: Intent::SaveLayout,
            pane,
        });
        ui.close();
    }
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
        other: cx
            .app
            .action_ctx(cx.pane, Invocation::ContextMenu)
            .and_then(|c| c.other),
    }
}

/// The context menu's width (SPEC.md *Context menu*).
pub const CONTEXT_MENU_WIDTH: f32 = 260.0;
/// An entry's height.
const ENTRY_HEIGHT: f32 = 30.0;
/// The padding of the menu card and of an entry.
const MENU_PAD: f32 = 7.0;
const ENTRY_PAD_X: f32 = 10.0;
/// The gap between an entry's icon slot and its label.
const ICON_GAP: f32 = 10.0;

/// The style of a menu card: `bg/card` with the rows padding themselves,
/// the hovered entry `bg/selected-soft`.
pub fn menu_style(style: &mut egui::Style) {
    style.spacing.menu_margin = egui::Margin::same(MENU_PAD as i8);
    style.spacing.menu_width = CONTEXT_MENU_WIDTH - 2.0 * MENU_PAD;
    style.spacing.item_spacing = egui::Vec2::ZERO;
    style.spacing.button_padding = egui::vec2(ENTRY_PAD_X, 0.0);
}

/// Fixes a menu's content width to the card's.
fn card_width(ui: &mut egui::Ui) {
    ui.set_min_width(CONTEXT_MENU_WIDTH - 2.0 * MENU_PAD);
    ui.set_max_width(CONTEXT_MENU_WIDTH - 2.0 * MENU_PAD);
}

/// The icon of an action's entry, by its id; `None` leaves the slot empty.
fn action_icon(id: &str) -> Option<&'static str> {
    Some(match id {
        "core.open_default" => icons::FOLDER_OPEN,
        "core.open_with" => icons::OPEN_WITH,
        "core.mount" => icons::MOUNT,
        "core.open_containing" | "core.open_containing_tab" => icons::FOLDER,
        "core.reveal" => icons::REVEAL,
        "core.pin" => icons::STAR,
        "core.unpin" => icons::UNPIN,
        "terminal.console" => icons::TERMINAL,
        "compare.compare" => icons::COMPARE,
        "core.cut" => icons::CUT,
        "core.copy" => icons::COPY,
        "core.paste" => icons::PASTE,
        "core.paste_link" | "core.create_link" => icons::LINK,
        "core.duplicate" => icons::DUPLICATE,
        "core.rename" | "rename.bulk" => icons::RENAME,
        "core.trash" => icons::TRASH,
        "core.delete" => icons::CLOSE,
        "core.new_folder" => icons::NEW_FOLDER,
        "core.new_file" => icons::NEW_FILE,
        "core.hash" => icons::HASH,
        "installer.install" => icons::INSTALL,
        _ => return None,
    })
}

/// The destructive entries, drawn in `status/danger`.
fn is_destructive(id: &str) -> bool {
    matches!(id, "core.trash" | "core.delete")
}

/// The icon of an extension group's submenu.
fn group_icon(name: &str) -> Option<&'static str> {
    match name {
        "Git" | "SVN" => Some(icons::BRANCH),
        "Compress" => Some(icons::ARCHIVE),
        "Installer" => Some(icons::INSTALL),
        "Trash" => Some(icons::TRASH),
        _ => None,
    }
}

/// The spec's groups of a file's menu, as action ids in order: Open, Edit
/// and Change. The *Copy as* submenu sits in Edit after *Paste as link*;
/// the extension groups and Info follow.
const FILE_GROUPS: &[&[&str]] = &[
    &[
        "core.open_default",
        "core.open_with",
        "core.open_containing",
        "core.open_containing_tab",
        "terminal.console",
        "core.mount",
        "compare.compare",
        "core.reveal",
        "core.pin",
        "core.unpin",
    ],
    &[
        "core.cut",
        "core.copy",
        "core.paste",
        "core.paste_link",
        "core.create_link",
        "menu.copy_as",
        "core.duplicate",
        "core.select_pattern",
    ],
    &["core.rename", "rename.bulk", "core.trash", "core.delete"],
];

/// The *Copy as* submenu: the three copy actions.
const COPY_AS: &[&str] = &[
    "core.copy_name",
    "core.copy_path",
    "core.copy_path_and_name",
];

/// The compress plugin's actions, a *Compress* submenu in the extension
/// group although the plugin registers them as core actions.
const COMPRESS: &[&str] = &[
    "compress.compress",
    "compress.compress_as",
    "compress.extract_here",
    "compress.extract_folder",
    "compress.extract_to",
];

/// The empty area's groups (SPEC.md *Context menu*, *Entries on empty
/// space*): New folder and New file; Paste, Paste as link and Select all;
/// Open terminal here, Open in system file manager and Refresh; Folder
/// properties. Select all, Refresh and Folder properties are intents, not
/// actions.
const EMPTY_GROUPS: &[&[&str]] = &[
    &["core.new_folder", "core.new_file"],
    &["core.paste", "core.paste_link", "menu.select_all"],
    &["terminal.console", "core.reveal", "menu.refresh"],
];

/// One entry of the menu: a 16 px icon slot, the label and the shortcut at
/// the right; `destructive` draws it in `status/danger`. Its accessible
/// name is the label alone.
fn entry(
    ui: &mut egui::Ui,
    icon: Option<&str>,
    label: &str,
    shortcut: Option<String>,
    destructive: bool,
) -> egui::Response {
    let response = ui.add(entry_button(ui, icon, label, shortcut, destructive, None));
    let spoken = label.to_owned();
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spoken.clone()));
    response
}

/// The button of an entry, for [`entry`] and for a submenu's row.
fn entry_button<'a>(
    ui: &egui::Ui,
    icon: Option<&str>,
    label: &str,
    shortcut: Option<String>,
    destructive: bool,
    right: Option<&str>,
) -> egui::Button<'a> {
    let scheme = theme::scheme(ui);
    let colour = if destructive {
        scheme.status_danger
    } else {
        scheme.text_primary
    };
    let mut job = egui::text::LayoutJob::default();
    let mut icon_format = egui::TextFormat {
        font_id: egui::FontId::proportional(ICON),
        color: if destructive {
            scheme.status_danger
        } else {
            scheme.icon_default
        },
        valign: Align::Center,
        ..Default::default()
    };
    if icon.is_none() {
        icon_format.color = egui::Color32::TRANSPARENT;
    }
    // The slot keeps its width without an icon: a transparent glyph.
    job.append(icon.unwrap_or(icons::FOLDER), 0.0, icon_format);
    job.append(
        label,
        ICON_GAP,
        egui::TextFormat {
            font_id: egui::FontId::proportional(TEXT_BODY),
            color: colour,
            valign: Align::Center,
            ..Default::default()
        },
    );
    // One line: a long label is cut with an ellipsis before the shortcut.
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let mut button = egui::Button::new(job)
        .wrap_mode(egui::TextWrapMode::Truncate)
        .frame_when_inactive(false)
        .min_size(egui::vec2(0.0, ENTRY_HEIGHT));
    if let Some(text) = shortcut {
        button = button.shortcut_text(
            egui::RichText::new(text)
                .size(TEXT_SECONDARY)
                .color(scheme.text_secondary),
        );
    }
    if let Some(right) = right {
        button = button.right_text(
            egui::RichText::new(right)
                .size(ICON)
                .color(scheme.icon_default),
        );
    }
    button
}

/// A submenu row: the entry with a caret at its right, opening `content`
/// to the right on hover. Its accessible name is the label with egui's
/// arrow, as every submenu's.
fn submenu(
    ui: &mut egui::Ui,
    icon: Option<&str>,
    label: &str,
    content: impl FnOnce(&mut egui::Ui),
) {
    let button = entry_button(ui, icon, label, None, false, Some(icons::TREE_CLOSED));
    let (response, _) = egui::containers::menu::SubMenuButton::from_button(button)
        .config(egui::containers::menu::MenuConfig::new().style(menu_style))
        .ui(ui, |ui| {
            card_width(ui);
            content(ui);
        });
    let spoken = format!(
        "{label} {}",
        egui::containers::menu::SubMenuButton::RIGHT_ARROW
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spoken.clone()));
}

/// A hairline between two groups, with 4 px of air on each side.
fn separator(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 9.0), egui::Sense::hover());
    let y = rect.center().y.round() + 0.5;
    ui.painter().line_segment(
        [
            egui::pos2(rect.left() + 6.0, y),
            egui::pos2(rect.right() - 6.0, y),
        ],
        theme::scheme(ui).hairline(),
    );
}

/// The menu's own wording of an action, where the spec's entry is shorter
/// than the action's label: *Open* for the system default, and *Name*,
/// *Path* and *Path and name* in the *Copy as* submenu.
fn menu_label(action: &dyn ext::Action) -> String {
    match action.id().0 {
        "core.open_default" => t!("open"),
        "core.copy_name" => t!("menu-copy-as-name"),
        "core.copy_path" => t!("menu-copy-as-path"),
        "core.copy_path_and_name" => t!("menu-copy-as-path-and-name"),
        _ => action.label(),
    }
}

/// Runs an action's entry: the row, and the request when clicked.
fn action_entry(
    ui: &mut egui::Ui,
    cx: &mut PaneCx<'_>,
    action_ctx: &ActionCtx,
    action: &dyn ext::Action,
) {
    let id = action.id();
    // Open takes Enter's spelling: that is how a file opens from the list.
    let shortcut = if id.0 == "core.open_default" {
        cx.keys.text_for(Intent::Open)
    } else {
        cx.keys.text_for(Intent::Action(id))
    };
    if entry(
        ui,
        action_icon(id.0),
        &menu_label(action),
        shortcut,
        is_destructive(id.0),
    )
    .clicked()
    {
        cx.requests.push(Request::RunAction {
            action: id,
            ctx: action_ctx.clone(),
        });
        ui.close();
    }
}

/// An intent's entry: the row with the intent's shortcut, queued when
/// clicked.
fn intent_entry(
    ui: &mut egui::Ui,
    cx: &mut PaneCx<'_>,
    icon: Option<&str>,
    label: &str,
    intent: Intent,
) {
    if entry(ui, icon, label, cx.keys.text_for(intent), false).clicked() {
        cx.requests.push(Request::Intent {
            intent,
            pane: cx.pane,
        });
        ui.close();
    }
}

/// The available actions among `ids`, in that order.
fn available<'a>(
    actions: &'a [Arc<dyn ext::Action>],
    ids: &[&str],
    action_ctx: &ActionCtx,
) -> Vec<&'a Arc<dyn ext::Action>> {
    ids.iter()
        .filter_map(|id| actions.iter().find(|a| a.id().0 == *id))
        .filter(|a| a.availability(action_ctx) == Availability::Enabled)
        .collect()
}

/// The context menu (SPEC.md *Context menu*): a 260 px card of groups
/// separated by hairlines, each entry with an icon slot, its label and its
/// shortcut, destructive entries in `status/danger`, submenus opening to
/// the right. On `entry` the file's menu — Open, Edit, Change, the
/// extension groups and Info — on `None` the empty area's. Entries that
/// do not apply are hidden.
#[allow(clippy::too_many_lines)]
pub fn context_menu(
    ui: &mut egui::Ui,
    cx: &mut PaneCx<'_>,
    action_ctx: &ActionCtx,
    entry_of: Option<&Entry>,
) {
    card_width(ui);
    let actions = cx.app.registry().actions();
    let mut any = false;
    let group_start = |ui: &mut egui::Ui, any: &mut bool, shown: &mut bool| {
        if !*shown && *any {
            separator(ui);
        }
        *shown = true;
        *any = true;
    };

    if let Some(target) = entry_of {
        // Open: a folder's Open and Open in new tab first.
        let mut shown = false;
        if target.kind == types::EntryKind::Dir {
            group_start(ui, &mut any, &mut shown);
            if entry(
                ui,
                Some(icons::FOLDER_OPEN),
                &t!("open"),
                cx.keys.text_for(Intent::Open),
                false,
            )
            .clicked()
            {
                cx.requests.push(Request::OpenEntry {
                    pane: cx.pane,
                    id: target.id,
                });
                ui.close();
            }
            if entry(ui, None, &t!("open-in-new-tab"), None, false).clicked() {
                cx.requests
                    .push(Request::NewTabAt(cx.tab.location_of(target)));
                ui.close();
            }
        }
        for (index, ids) in FILE_GROUPS.iter().enumerate() {
            if index > 0 {
                shown = false;
            }
            for id in *ids {
                if *id == "menu.copy_as" {
                    let copies = available(actions, COPY_AS, action_ctx);
                    if copies.is_empty() {
                        continue;
                    }
                    group_start(ui, &mut any, &mut shown);
                    submenu(ui, None, &t!("menu-copy-as"), |ui| {
                        for action in copies {
                            action_entry(ui, cx, action_ctx, &**action);
                        }
                    });
                    continue;
                }
                let Some(action) = actions.iter().find(|a| a.id().0 == *id) else {
                    continue;
                };
                if action.availability(action_ctx) != Availability::Enabled {
                    continue;
                }
                group_start(ui, &mut any, &mut shown);
                action_entry(ui, cx, action_ctx, &**action);
            }
        }
    } else {
        for ids in EMPTY_GROUPS {
            let mut shown = false;
            for id in *ids {
                match *id {
                    "menu.select_all" => {
                        group_start(ui, &mut any, &mut shown);
                        intent_entry(
                            ui,
                            cx,
                            Some(icons::SELECT_ALL),
                            &t!("menu-select-all"),
                            Intent::SelectAll,
                        );
                    }
                    "menu.refresh" => {
                        group_start(ui, &mut any, &mut shown);
                        intent_entry(
                            ui,
                            cx,
                            Some(icons::REFRESH),
                            &t!("menu-refresh"),
                            Intent::Refresh,
                        );
                    }
                    id => {
                        let Some(action) = actions.iter().find(|a| a.id().0 == id) else {
                            continue;
                        };
                        if action.availability(action_ctx) != Availability::Enabled {
                            continue;
                        }
                        group_start(ui, &mut any, &mut shown);
                        action_entry(ui, cx, action_ctx, &**action);
                    }
                }
            }
        }
    }

    // Extensions: Compress, the plugin groups (a submenu, or the one entry
    // of a group with one), Install as application, Compute hash….
    let mut shown = false;
    let compress = available(actions, COMPRESS, action_ctx);
    if !compress.is_empty() {
        group_start(ui, &mut any, &mut shown);
        submenu(ui, group_icon("Compress"), "Compress", |ui| {
            for action in compress {
                action_entry(ui, cx, action_ctx, &**action);
            }
        });
    }
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
        let members: Vec<&Arc<dyn ext::Action>> = actions
            .iter()
            .filter(|a| {
                a.group() == ActionGroup::Extension(name)
                    && a.availability(action_ctx) == Availability::Enabled
            })
            .collect();
        match members.as_slice() {
            [] => {}
            [one] => {
                group_start(ui, &mut any, &mut shown);
                let one: &dyn ext::Action = &***one;
                let icon = action_icon(one.id().0).or_else(|| group_icon(name));
                let shortcut = cx.keys.text_for(Intent::Action(one.id()));
                if entry(ui, icon, &one.label(), shortcut, false).clicked() {
                    cx.requests.push(Request::RunAction {
                        action: one.id(),
                        ctx: action_ctx.clone(),
                    });
                    ui.close();
                }
            }
            _ => {
                group_start(ui, &mut any, &mut shown);
                submenu(ui, group_icon(name), name, |ui| {
                    for action in members {
                        action_entry(ui, cx, action_ctx, &**action);
                    }
                });
            }
        }
    }
    if let Some(hash) = available(actions, &["core.hash"], action_ctx).first() {
        group_start(ui, &mut any, &mut shown);
        action_entry(ui, cx, action_ctx, &***hash);
    }

    // Info: Properties on a selection, Folder properties on the empty area.
    let mut shown = false;
    group_start(ui, &mut any, &mut shown);
    let label = if entry_of.is_some() {
        t!("properties")
    } else {
        t!("menu-folder-properties")
    };
    intent_entry(ui, cx, Some(icons::INFO), &label, Intent::Inspect);
}
