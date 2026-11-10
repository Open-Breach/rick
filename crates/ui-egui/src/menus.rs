//! Menus: the burger menu and the context menu built from the action
//! registry.

use std::sync::Arc;

use app::{App, Command};
use ext::t;
use ext::{Action, ActionCtx, ActionGroup, Availability, Invocation};
use types::Entry;

use egui::Align;

use crate::icons;
use crate::keys::{Intent, KeyMap};
use crate::shell::{PaneCx, Request};
use crate::theme::{self, ICON, Scheme, TEXT_BODY, TEXT_SECONDARY};

/// The application menu's least width (SPEC.md *Menu*); the card grows
/// when an entry needs more.
pub const APP_MENU_WIDTH: f32 = 280.0;
/// The gap between the menu button and the card that opens under it.
const APP_MENU_GAP: f32 = 6.0;
/// The padding of the application menu's card inside its 1 px border, so
/// that an entry starts 7 px from the card's edge.
const APP_MENU_PAD: f32 = 6.0;
/// The least content width of a submenu of the application menu.
const SUBMENU_WIDTH: f32 = 200.0;

/// The burger menu at the top of the window, the application menu
/// (SPEC.md *Menu*): the menu button, a tool button, and under it — left
/// edges aligned, 6 px below — a card in the form of the other menus. Three
/// groups: New window, the Layouts submenu and one submenu per extension
/// that put actions in the application menu ([`Action::app_menu`]);
/// Settings, Keyboard shortcuts and Extensions; About Rick with `version`
/// at its right. Every entry has an icon, and its hint is the chord the
/// keymap binds to it, nothing when it is unbound.
pub fn burger(
    ui: &mut egui::Ui,
    requests: &mut Vec<Request>,
    pane: model::PaneId,
    app: &App,
    keys: &KeyMap,
    version: &str,
) {
    let scheme = theme::scheme(ui);
    // The button sits on the title row's band and is drawn in its colours.
    let band = theme::band(ui);
    let metrics = theme::metrics(ui.ctx());
    let size = metrics.button_size;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    let name = t!("menu-application-menu");
    let spoken = name.clone();
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spoken.clone()));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    // A tool button: no frame at rest, `bg/subtle` under the pointer,
    // `bg/active` while pressed and while its menu is open. On a band
    // that does not agree with the scheme both are the band's own hover,
    // on which its icon reads.
    let fill = if open || response.is_pointer_button_down_on() {
        Some(if band.contrary {
            band.hover
        } else {
            scheme.bg_active
        })
    } else if response.hovered() {
        Some(band.hover)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter()
            .rect_filled(rect, metrics.corner_radius(), fill);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            metrics.corner_radius(),
            scheme.focus_ring(),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::MENU,
        egui::FontId::proportional(ICON),
        band.icon,
    );
    let response = if open {
        response
    } else {
        response.on_hover_text(name)
    };
    egui::Popup::menu(&response)
        .gap(APP_MENU_GAP)
        .style(app_menu_style)
        .show(|ui| app_menu(ui, requests, pane, app, keys, version));
}

/// One group of the application menu's extension actions: the core ones
/// (`None`) or an extension's, by its name.
type AppMenuGroup = (Option<&'static str>, Vec<Arc<dyn Action>>);
/// The extension actions of the application menu with the context they
/// run on, as [`app_menu_actions`] gives them.
type AppMenuActions = (ActionCtx, Vec<AppMenuGroup>);

/// The card of the application menu: its three groups.
fn app_menu(
    ui: &mut egui::Ui,
    requests: &mut Vec<Request>,
    pane: model::PaneId,
    app: &App,
    keys: &KeyMap,
    version: &str,
) {
    let extensions = app_menu_actions(app, pane);
    fit_app_menu(ui, keys, version, extensions.as_ref());

    // Window and tools.
    intent_item(
        ui,
        requests,
        pane,
        keys,
        Some(icons::NEW_WINDOW),
        &t!("menu-new-window"),
        Intent::NewWindow,
    );
    submenu_in(ui, Some(icons::LAYOUTS), &t!("menu-layouts"), APP, |ui| {
        layouts_menu(ui, requests, pane, app, keys);
    });
    if let Some(extensions) = extensions {
        extension_items(ui, requests, extensions);
    }

    // Configuration.
    separator(ui);
    for (icon, label, intent) in [
        (icons::SETTINGS, t!("settings"), Intent::Settings),
        (
            icons::SHORTCUTS,
            t!("keyboard-shortcuts"),
            Intent::Shortcuts,
        ),
        (icons::PLUGIN, t!("extensions"), Intent::Extensions),
    ] {
        intent_item(ui, requests, pane, keys, Some(icon), &label, intent);
    }

    // About, with the version where a shortcut would be.
    separator(ui);
    let label = t!("about-rick");
    let about = entry_button(
        ui,
        Some(icons::INFO),
        &label,
        Some(version.to_owned()),
        false,
        None,
        APP,
    );
    if labelled(ui.add(about), &label).clicked() {
        requests.push(Request::About);
        ui.close();
    }
}

/// Sets the application menu's width from what it will list: its own six
/// entries with their hints, the version, and the extensions' entries.
fn fit_app_menu(
    ui: &mut egui::Ui,
    keys: &KeyMap,
    version: &str,
    extensions: Option<&AppMenuActions>,
) {
    let own = [
        (t!("menu-new-window"), keys.text_for(Intent::NewWindow)),
        (t!("settings"), keys.text_for(Intent::Settings)),
        (t!("keyboard-shortcuts"), keys.text_for(Intent::Shortcuts)),
        (t!("extensions"), keys.text_for(Intent::Extensions)),
    ];
    let layouts = t!("menu-layouts");
    let about = t!("about-rick");
    let groups = extensions.map(|(_, groups)| groups.as_slice());
    let actions: Vec<String> = groups
        .into_iter()
        .flatten()
        .filter(|(group, _)| group.is_none())
        .flat_map(|(_, members)| members.iter().map(|action| action.label()))
        .collect();
    let entries = own
        .iter()
        .map(|(label, hint)| (label.as_str(), Trailing::of(hint.as_deref())))
        .chain([
            (layouts.as_str(), Trailing::Caret),
            (about.as_str(), Trailing::Hint(version)),
        ])
        .chain(actions.iter().map(|l| (l.as_str(), Trailing::None)))
        .chain(
            groups
                .into_iter()
                .flatten()
                .filter_map(|(group, _)| group.map(|name| (name, Trailing::Caret))),
        );
    fit_width(ui, APP_MENU_WIDTH - 2.0 * (APP_MENU_PAD + 1.0), entries);
}

/// The extensions' part of the application menu: the core-grouped actions
/// as entries, then one submenu per extension with its group's icon.
fn extension_items(ui: &mut egui::Ui, requests: &mut Vec<Request>, extensions: AppMenuActions) {
    let (ctx, groups) = extensions;
    for (group, members) in groups {
        let Some(name) = group else {
            for action in members {
                action_item(ui, requests, action.as_ref(), &ctx);
            }
            continue;
        };
        let icon = group_icon(name).unwrap_or(icons::PLUGIN);
        submenu_in(ui, Some(icon), name, APP, |ui| {
            let labels: Vec<String> = members.iter().map(|a| a.label()).collect();
            fit_width(
                ui,
                SUBMENU_WIDTH,
                labels.iter().map(|l| (l.as_str(), Trailing::None)),
            );
            for action in &members {
                action_item(ui, requests, action.as_ref(), &ctx);
            }
        });
    }
}

/// One entry of the application menu that queues an intent: its icon, its
/// label and the chord the keymap binds to it.
fn intent_item(
    ui: &mut egui::Ui,
    requests: &mut Vec<Request>,
    pane: model::PaneId,
    keys: &KeyMap,
    icon: Option<&str>,
    label: &str,
    intent: Intent,
) {
    let row = entry_button(ui, icon, label, keys.text_for(intent), false, None, APP);
    if labelled(ui.add(row), label).clicked() {
        requests.push(Request::Intent { intent, pane });
        ui.close();
    }
}

/// The actions that asked for the application menu and apply to the
/// focused pane, with the context they run on: the core ones first, then
/// one group per extension, by name. `None` when the pane shows no
/// entries (a console, a comparison) or nothing applies.
fn app_menu_actions(app: &App, pane: model::PaneId) -> Option<AppMenuActions> {
    let ctx = app.action_ctx(pane, Invocation::Menu)?;
    let mut groups: Vec<AppMenuGroup> = Vec::new();
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
    let label = action.label();
    let row = entry_button(
        ui,
        action_icon(action.id().0),
        &label,
        None,
        false,
        None,
        APP,
    );
    if labelled(ui.add(row), &label).clicked() {
        requests.push(Request::RunAction {
            action: action.id(),
            ctx: ctx.clone(),
        });
        ui.close();
    }
}

/// The Layouts submenu: "Continue where I left off" first, then one
/// submenu per saved layout with Switch, Set as start layout, Rename and
/// Delete, then Save layout as…. Its rows have the form of the menu's.
fn layouts_menu(
    ui: &mut egui::Ui,
    requests: &mut Vec<Request>,
    pane: model::PaneId,
    app: &App,
    keys: &KeyMap,
) {
    let start = app.model().settings.session.start_layout();
    let titles: Vec<String> = app
        .layouts()
        .iter()
        .map(|name| {
            if start == Some(name.as_str()) {
                format!("{name} (start layout)")
            } else {
                name.clone()
            }
        })
        .collect();
    let last_session = t!("continue-where-i-left-off");
    let save_as = t!("menu-save-layout-as");
    let hints = [
        keys.text_for(Intent::LastSession),
        keys.text_for(Intent::SaveLayout),
    ];
    fit_width(
        ui,
        SUBMENU_WIDTH,
        [
            (last_session.as_str(), Trailing::of(hints[0].as_deref())),
            (save_as.as_str(), Trailing::of(hints[1].as_deref())),
        ]
        .into_iter()
        .chain(titles.iter().map(|t| (t.as_str(), Trailing::Caret))),
    );
    intent_item(
        ui,
        requests,
        pane,
        keys,
        None,
        &last_session,
        Intent::LastSession,
    );
    for (index, (name, title)) in app.layouts().iter().zip(&titles).enumerate() {
        let Ok(index) = u16::try_from(index) else {
            break;
        };
        submenu_in(ui, None, title, APP, |ui| {
            let labels = [
                t!("menu-switch"),
                t!("menu-set-as-start-layout"),
                t!("rename-ellipsis"),
                t!("delete"),
            ];
            fit_width(
                ui,
                SUBMENU_WIDTH,
                labels.iter().map(|l| (l.as_str(), Trailing::None)),
            );
            intent_item(
                ui,
                requests,
                pane,
                keys,
                None,
                &t!("menu-switch"),
                Intent::Layout(index),
            );
            let is_start = start == Some(name.as_str());
            let label = t!("menu-set-as-start-layout");
            let row = entry_button(ui, None, &label, None, false, None, APP);
            if labelled(ui.add_enabled(!is_start, row), &label).clicked() {
                requests.push(Request::Command(Command::Settings(
                    model::SettingsChange::StartLayout(Some(name.clone())),
                )));
                ui.close();
            }
            let label = t!("rename-ellipsis");
            let row = entry_button(ui, None, &label, None, false, None, APP);
            if labelled(ui.add(row), &label).clicked() {
                requests.push(Request::RenameLayout(name.clone()));
                ui.close();
            }
            let label = t!("delete");
            let row = entry_button(ui, None, &label, None, false, None, APP);
            if labelled(ui.add(row), &label).clicked() {
                requests.push(Request::Command(Command::DeleteLayout(name.clone())));
                ui.close();
            }
        });
    }
    separator(ui);
    intent_item(ui, requests, pane, keys, None, &save_as, Intent::SaveLayout);
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
/// The least room between a label and the hint or caret at its right.
const HINT_GAP: f32 = 24.0;

/// The form of a menu's rows: the context menus' or the application
/// menu's.
#[derive(Clone, Copy)]
struct Rows {
    /// An entry's height.
    height: f32,
    /// Whether the card is the context menus' 260 px one; the application
    /// menu's cards are fitted to their entries ([`fit_width`]).
    fixed: bool,
}

/// The context menus: 30 px entries in a 260 px card.
const CONTEXT: Rows = Rows {
    height: ENTRY_HEIGHT,
    fixed: true,
};
/// The application menu: 32 px entries, as the View options menu has, in
/// a card that is at least [`APP_MENU_WIDTH`] wide.
const APP: Rows = Rows {
    height: 32.0,
    fixed: false,
};

/// What an entry shows at its right edge, for [`fit_width`].
#[derive(Clone, Copy)]
enum Trailing<'a> {
    /// Nothing.
    None,
    /// A shortcut hint, or the version beside About Rick.
    Hint(&'a str),
    /// A submenu's caret.
    Caret,
}

impl<'a> Trailing<'a> {
    /// A hint when there is one, nothing otherwise.
    fn of(hint: Option<&'a str>) -> Self {
        hint.map_or(Trailing::None, Trailing::Hint)
    }
}

/// Sets a card's content width to its widest entry, and to `least` at
/// least: the padding, the icon slot, the label and what stands at the
/// right with its gap. The rows fill that width, so a card is as wide as
/// its entries need and no wider.
fn fit_width<'a>(
    ui: &mut egui::Ui,
    least: f32,
    entries: impl IntoIterator<Item = (&'a str, Trailing<'a>)>,
) {
    let measure = |ui: &egui::Ui, text: &str, size: f32| {
        ui.painter()
            .layout_no_wrap(
                text.to_owned(),
                egui::FontId::proportional(size),
                egui::Color32::PLACEHOLDER,
            )
            .size()
            .x
    };
    let mut width = least;
    for (label, trailing) in entries {
        let right = match trailing {
            Trailing::None => 0.0,
            Trailing::Hint(hint) => HINT_GAP + measure(ui, hint, TEXT_SECONDARY),
            Trailing::Caret => HINT_GAP + ICON,
        };
        let row = 2.0 * ENTRY_PAD_X + ICON + ICON_GAP + measure(ui, label, TEXT_BODY) + right;
        width = width.max(row.ceil());
    }
    ui.set_min_width(width);
    ui.set_max_width(width);
}

/// The style of a menu card: `bg/card` with the rows padding themselves,
/// the hovered entry `bg/selected-soft`.
pub fn menu_style(style: &mut egui::Style) {
    style.spacing.menu_margin = egui::Margin::same(MENU_PAD as i8);
    style.spacing.menu_width = CONTEXT_MENU_WIDTH - 2.0 * MENU_PAD;
    style.spacing.item_spacing = egui::Vec2::ZERO;
    style.spacing.button_padding = egui::vec2(ENTRY_PAD_X, 0.0);
}

/// The style of the application menu's card and of its submenus: a
/// `bg/card` card whose rows pad themselves, the entry under the pointer
/// and the row of an open submenu `bg/selected-soft` with no outline.
pub fn app_menu_style(style: &mut egui::Style) {
    let scheme = if style.visuals.dark_mode {
        &Scheme::DARK
    } else {
        &Scheme::LIGHT
    };
    style.spacing.menu_margin = egui::Margin::same(APP_MENU_PAD as i8);
    style.spacing.item_spacing = egui::Vec2::ZERO;
    style.spacing.button_padding = egui::vec2(ENTRY_PAD_X, 0.0);
    style.visuals.window_fill = scheme.bg_card;
    let widgets = &mut style.visuals.widgets;
    for state in [&mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
        state.weak_bg_fill = scheme.bg_selected_soft;
        state.bg_fill = scheme.bg_selected_soft;
        state.bg_stroke = egui::Stroke::NONE;
        state.expansion = 0.0;
    }
}

/// Gives a row its accessible name: the label alone, without the hint at
/// its right.
fn labelled(response: egui::Response, label: &str) -> egui::Response {
    let spoken = label.to_owned();
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spoken.clone()));
    response
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
    let row = entry_button(ui, icon, label, shortcut, destructive, None, CONTEXT);
    labelled(ui.add(row), label)
}

/// The button of an entry, for [`entry`] and for a submenu's row.
fn entry_button<'a>(
    ui: &egui::Ui,
    icon: Option<&str>,
    label: &str,
    shortcut: Option<String>,
    destructive: bool,
    right: Option<&str>,
    rows: Rows,
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
        .min_size(egui::vec2(0.0, rows.height));
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
    submenu_in(ui, icon, label, CONTEXT, content);
}

/// [`submenu`] in the given row form: the context menus' card of fixed
/// width, or the application menu's, whose content sets its own width
/// with [`fit_width`].
fn submenu_in(
    ui: &mut egui::Ui,
    icon: Option<&str>,
    label: &str,
    rows: Rows,
    content: impl FnOnce(&mut egui::Ui),
) {
    let caret = Some(icons::TREE_CLOSED);
    let button = entry_button(ui, icon, label, None, false, caret, rows);
    let style: fn(&mut egui::Style) = if rows.fixed {
        menu_style
    } else {
        app_menu_style
    };
    let (response, _) = egui::containers::menu::SubMenuButton::from_button(button)
        .config(egui::containers::menu::MenuConfig::new().style(style))
        .ui(ui, |ui| {
            if rows.fixed {
                card_width(ui);
            }
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

/// Where a paste from the menu on `target` goes: into the folder the menu
/// was opened on, when that one folder is what the menu is about; into the
/// pane's folder on a file, on several entries and on the empty area.
fn paste_ctx(cx: &PaneCx<'_>, action_ctx: &ActionCtx, target: Option<&Entry>) -> ActionCtx {
    match target {
        Some(folder)
            if folder.kind == types::EntryKind::Dir
                && matches!(action_ctx.selection.as_slice(), [one] if one.id == folder.id) =>
        {
            ActionCtx {
                location: cx.tab.location_of(folder),
                selection: Vec::new(),
                ..action_ctx.clone()
            }
        }
        _ => action_ctx.clone(),
    }
}

/// The *Paste* and *Paste as link* entries (issue 176). *Paste* is always
/// there where the folder can be written to: greyed, with the reason as its
/// tooltip, while there is nothing to paste. *Paste as link* is hidden
/// when it does not apply, as every other entry. Returns whether a row was
/// drawn.
fn paste_entry(
    ui: &mut egui::Ui,
    cx: &mut PaneCx<'_>,
    into: &ActionCtx,
    action: &dyn ext::Action,
    start: impl FnOnce(&mut egui::Ui),
) {
    match action.availability(into) {
        Availability::Enabled => {
            start(ui);
            action_entry(ui, cx, into, action);
        }
        Availability::Disabled(why) if action.id().0 == "core.paste" => {
            start(ui);
            let label = menu_label(action);
            let shortcut = cx.keys.text_for(Intent::Action(action.id()));
            let row = entry_button(
                ui,
                action_icon(action.id().0),
                &label,
                shortcut,
                false,
                None,
                CONTEXT,
            );
            let response = ui.add_enabled(false, row);
            let spoken = label.clone();
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, false, spoken.clone())
            });
            response.on_disabled_hover_text(why);
        }
        Availability::Disabled(_) | Availability::Hidden => {}
    }
}

/// The context menu (SPEC.md *Context menu*): a 260 px card of groups
/// separated by hairlines, each entry with an icon slot, its label and its
/// shortcut, destructive entries in `status/danger`, submenus opening to
/// the right. On `entry` the file's menu — Open, Edit, Change, the
/// extension groups and Info — on `None` the empty area's. Entries that
/// do not apply are hidden; *Paste* alone is greyed while the clipboard is
/// empty, and on a folder it pastes into that folder.
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
                if matches!(*id, "core.paste" | "core.paste_link") {
                    let into = paste_ctx(cx, action_ctx, entry_of);
                    paste_entry(ui, cx, &into, &**action, |ui| {
                        group_start(ui, &mut any, &mut shown);
                    });
                    continue;
                }
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
                        if id == "core.paste" {
                            paste_entry(ui, cx, action_ctx, &**action, |ui| {
                                group_start(ui, &mut any, &mut shown);
                            });
                            continue;
                        }
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
