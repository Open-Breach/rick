//! One tab's content: toolbar with path bar, the file view, and the foot
//! with the status line and the filter.

use ext::t;
use std::collections::{BTreeMap, BTreeSet};

use app::Command;
use egui::Align;
use model::{IconSize, SettingsChange, ViewMode};
use types::{EntryKind, Location};

use crate::dnd::{self, Spring, Target, Zone};
use crate::keys::Intent;
use crate::shell::{PaneCx, PaneDrag, Request};
use crate::theme::{self, ICON, TEXT_BODY, TEXT_CAPS, TEXT_SECONDARY};
use crate::widgets::switch;
use crate::{format, icons, paths, views};

/// The grab handle's width.
pub const HANDLE_WIDTH: f32 = 28.0;
/// The view options button's width: the sliders and the caret.
const VIEW_OPTIONS_WIDTH: f32 = 48.0;
/// The view options menu card's width.
const MENU_WIDTH: f32 = 260.0;
/// A row of the view options menu.
const MENU_ROW: f32 = 32.0;
/// The parent chip's size.
const CHIP: egui::Vec2 = egui::vec2(28.0, 26.0);
/// The footer's filter field: 200 px wide, 26 px tall (SPEC.md *Footer*).
const FILTER_FIELD: egui::Vec2 = egui::vec2(200.0, 26.0);
/// The padding inside the filter field.
const FIELD_PAD: f32 = 8.0;
/// The include-subfolders toggle inside the field's right end.
const FIELD_TOGGLE: f32 = 22.0;

/// Draws the whole tab.
pub fn show(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    egui::Panel::top(egui::Id::new(("toolbar", cx.tab_id)))
        .show_separator_line(false)
        .show(ui, |ui| toolbar(ui, cx));
    if cx.tab.failure.is_some() {
        egui::Panel::top(egui::Id::new(("failure", cx.tab_id)))
            .show_separator_line(true)
            .show(ui, |ui| failure_banner(ui, cx));
    }
    egui::Panel::bottom(egui::Id::new(("foot", cx.tab_id)))
        .show_separator_line(true)
        .show(ui, |ui| foot(ui, cx));
    egui::CentralPanel::no_frame().show(ui, |ui| views::show(ui, cx));
}

/// The banner over a list that could not be read: why, and *Retry*, which
/// lists the location again. A connection error is never a modal dialogue.
fn failure_banner(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let Some(failure) = cx.tab.failure.as_deref() else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{} {failure}", icons::FAILED))
                .color(ui.visuals().warn_fg_color),
        );
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if ui.button(t!("retry")).clicked() {
                cx.requests
                    .push(Request::Command(Command::Refresh(cx.pane)));
            }
        });
    });
}

fn intent(cx: &mut PaneCx<'_>, intent: Intent) {
    cx.requests.push(Request::Intent {
        intent,
        pane: cx.pane,
    });
}

/// The side of an icon-only tool button, `button/size` of the density, so
/// the two icon groups keep their width whatever the window size.
fn button_size(ui: &egui::Ui) -> f32 {
    crate::theme::metrics(ui.ctx()).button_size
}

/// The toolbar row's height for a density: a tool button plus the toolbar
/// padding above and below it, the density being the single knob.
#[must_use]
pub fn toolbar_height(density: model::Density) -> f32 {
    crate::theme::Metrics::of(density).toolbar_height()
}

/// The toolbar: a fixed group of navigation icons on the left, a fixed group
/// of view controls on the right, and the breadcrumb flexing in between,
/// never overlapping either.
fn toolbar(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    cx.tab_ui.toolbar_rects.clear();
    let height = toolbar_height(cx.app.model().settings.density);
    let (row, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    cx.tab_ui.toolbar_row = Some(row);
    // Left group, fixed.
    let mut left = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    left_group(&mut left, cx);
    let left_edge = left.min_rect().right();
    // Right group, fixed.
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    right_group(&mut right, cx);
    let right_edge = right.min_rect().left();
    // The breadcrumb gets what is left, clipped to it.
    let gap = ui.spacing().item_spacing.x;
    let middle = egui::Rect::from_min_max(
        egui::pos2(left_edge + gap, row.top()),
        egui::pos2((right_edge - gap).max(left_edge + gap), row.bottom()),
    );
    let mut centre = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(middle)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    centre.set_clip_rect(middle.intersect(centre.clip_rect()));
    path_bar(&mut centre, cx, middle);
}

fn icon_button(ui: &mut egui::Ui, glyph: &str, enabled: bool) -> egui::Response {
    let size = button_size(ui);
    ui.add_enabled(
        enabled,
        egui::Button::new(glyph).min_size(egui::vec2(size, size)),
    )
}

/// A 1 px hairline, `height` tall, centred in the row.
fn divider(ui: &mut egui::Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, height), egui::Sense::hover());
    let x = rect.center().x.round() + 0.5;
    ui.painter().line_segment(
        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
        theme::scheme(ui).hairline(),
    );
}

fn remember(cx: &mut PaneCx<'_>, group: &'static str, rect: egui::Rect) {
    cx.tab_ui.toolbar_rects.push((group, rect));
}

fn left_group(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let back = icon_button(ui, icons::BACK, !cx.tab.history.back.is_empty())
        .on_hover_text(t!("pane-back"));
    remember(cx, "left", back.rect);
    if back.clicked() {
        intent(cx, Intent::Back);
    }
    let forward = icon_button(ui, icons::FORWARD, !cx.tab.history.forward.is_empty())
        .on_hover_text(t!("pane-forward"));
    remember(cx, "left", forward.rect);
    if forward.clicked() {
        intent(cx, Intent::Forward);
    }
}

/// The grab handle, a hairline and the view options button, right to left
/// (SPEC.md *Toolbar*).
fn right_group(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    handle(ui, cx);
    divider(ui, 16.0);
    view_options(ui, cx);
}

/// The pane's grab handle: a `HANDLE_WIDTH` wide ghost area with
/// `DOTS_SIX_VERTICAL` at 70 % opacity; the grab cursor over it, the
/// grabbing cursor while it drags the pane (SPEC.md *Panes*).
fn handle(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let size = button_size(ui);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(HANDLE_WIDTH, size),
        egui::Sense::click_and_drag(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t!("pane-drag-pane"))
    });
    remember(cx, "right", rect);
    let scheme = theme::scheme(ui);
    if response.hovered() || response.dragged() {
        ui.painter().rect_filled(
            rect,
            theme::metrics(ui.ctx()).corner_radius(),
            scheme.bg_subtle,
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::HANDLE,
        egui::FontId::proportional(ICON),
        scheme.icon_default.gamma_multiply(0.7),
    );
    if response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    if response.drag_started() {
        *cx.pane_drag = Some(PaneDrag {
            tab: cx.tab_id,
            pane: cx.pane,
        });
    }
    response.on_hover_text(t!("pane-drag-pane"));
}

/// The glyph of a view mode.
#[must_use]
pub fn view_mode_icon(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::List => icons::VIEW_LIST,
        ViewMode::Details => icons::VIEW_DETAILS,
        ViewMode::Tree => icons::VIEW_TREE,
        ViewMode::Icons(_) => icons::VIEW_ICONS,
    }
}

/// The view options button — the sliders with a caret, a 7 px
/// `accent/focus` dot at the top right while a type filter is on — and its
/// menu (SPEC.md *View options menu*).
fn view_options(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let size = button_size(ui);
    let scheme = theme::scheme(ui);
    let label = egui::RichText::new(format!("{} {}", icons::VIEW_OPTIONS, icons::DROPDOWN))
        .color(scheme.icon_default);
    // A ghost button: no frame at rest, the hover frame when hovered.
    let button = egui::Button::new(label)
        .frame_when_inactive(false)
        .min_size(egui::vec2(VIEW_OPTIONS_WIDTH, size));
    let response = egui::containers::menu::MenuButton::from_button(button)
        // The switches and the type checkboxes flip without closing the
        // menu; a view pick or Refresh closes it themselves.
        .config(
            egui::containers::menu::MenuConfig::new()
                .style(menu_style)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
        )
        .ui(ui, |ui| view_options_menu(ui, cx))
        .0;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t!("pane-view-options"))
    });
    remember(cx, "right", response.rect);
    if cx.tab.types.is_some() {
        // The filter-active dot, over the icon's top right corner.
        let at = egui::pos2(response.rect.left() + 8.0 + ICON, response.rect.top() + 8.0);
        ui.painter().circle_filled(at, 3.5, scheme.accent_focus);
    }
    response.on_hover_text(t!("pane-view-options"));
}

/// The menu as a `bg/card` card with no padding of its own: the rows pad
/// themselves.
fn menu_style(style: &mut egui::Style) {
    style.spacing.menu_margin = egui::Margin::same(7);
    style.spacing.item_spacing = egui::Vec2::ZERO;
}

/// The three titled groups of the view options menu: *View*, *Refresh*
/// and *Filter*.
fn view_options_menu(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    ui.set_width(MENU_WIDTH - 14.0);
    let keys = cx.keys;
    let text_for = |intent: Intent| keys.text_for(intent).unwrap_or_default();

    group_title(ui, &t!("pane-group-view"));
    let modes = [
        (
            icons::VIEW_DETAILS,
            t!("pane-view-details"),
            ViewMode::Details,
            text_for(Intent::ViewDetails),
        ),
        (
            icons::VIEW_LIST,
            t!("pane-view-list"),
            ViewMode::List,
            text_for(Intent::ViewList),
        ),
    ];
    for (icon, label, mode, shortcut) in modes {
        let row = menu_row(ui, icon, &label, cx.tab.view == mode, |ui| {
            ui.label(
                egui::RichText::new(shortcut)
                    .size(TEXT_SECONDARY)
                    .color(theme::scheme(ui).text_secondary),
            );
        });
        if row.clicked() {
            cx.requests.push(Request::Command(Command::SetView {
                pane: cx.pane,
                mode,
            }));
            ui.close();
        }
    }
    let icons_on = matches!(cx.tab.view, ViewMode::Icons(_));
    let current_size = match cx.tab.view {
        ViewMode::Icons(size) => Some(size),
        _ => None,
    };
    let mut picked: Option<IconSize> = None;
    let row = menu_row(
        ui,
        icons::VIEW_ICONS,
        &t!("pane-view-icons"),
        icons_on,
        |ui| {
            picked = icon_sizes(ui, current_size);
        },
    );
    if let Some(size) = picked {
        cx.requests.push(Request::Command(Command::SetView {
            pane: cx.pane,
            mode: ViewMode::Icons(size),
        }));
        ui.close();
    } else if row.clicked() {
        cx.requests.push(Request::Command(Command::SetView {
            pane: cx.pane,
            mode: ViewMode::Icons(current_size.unwrap_or(IconSize::Medium)),
        }));
        ui.close();
    }
    let hidden = cx.tab.show_hidden;
    let row = switch_row(ui, icons::HIDDEN_SHOWN, &t!("pane-show-hidden"), hidden);
    if row.clicked() {
        intent(cx, Intent::ToggleHidden);
    }
    let folders_first = cx.app.model().settings.folders_first;
    let row = switch_row(ui, icons::FOLDER, &t!("pane-folders-first"), folders_first);
    if row.clicked() {
        cx.requests.push(Request::Command(Command::Settings(
            SettingsChange::FoldersFirst(!folders_first),
        )));
    }

    menu_separator(ui);
    group_title(ui, &t!("pane-group-refresh"));
    let shortcut = text_for(Intent::Refresh);
    let row = menu_row(ui, icons::REFRESH, &t!("pane-refresh-now"), false, |ui| {
        ui.label(
            egui::RichText::new(shortcut)
                .size(TEXT_SECONDARY)
                .color(theme::scheme(ui).text_secondary),
        );
    });
    if row.clicked() {
        intent(cx, Intent::Refresh);
        ui.close();
    }

    menu_separator(ui);
    group_title(ui, &t!("pane-group-filter"));
    type_filter_rows(ui, cx);
}

/// *Show all*, then one checkbox row per file type in the folder with its
/// icon and count, folders as *Folders*; ticking types narrows the list to
/// them.
fn type_filter_rows(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    // Every entry of the folder, the view's filter notwithstanding.
    let mut counts: BTreeMap<String, (usize, &'static str)> = BTreeMap::new();
    for entry in cx.tab.listing.entries() {
        let slot = counts
            .entry(model::type_of(entry))
            .or_insert((0, icons::for_entry(entry)));
        slot.0 += 1;
    }
    let all = cx.tab.types.is_none();
    let row = check_row(ui, all, None, &t!("pane-show-all"), None);
    if row.clicked() && !all {
        cx.requests.push(Request::Command(Command::TypeFilter {
            pane: cx.pane,
            types: None,
        }));
    }
    let folder = model::FOLDER_TYPE;
    let mut ordered: Vec<(&String, &(usize, &'static str))> = counts.iter().collect();
    // Folders first, then the types by count, the biggest first.
    ordered.sort_by(|(a, (na, _)), (b, (nb, _))| {
        (*b == folder)
            .cmp(&(*a == folder))
            .then(nb.cmp(na))
            .then(a.cmp(b))
    });
    for (type_name, (count, icon)) in ordered {
        let on = cx.tab.types.as_ref().is_some_and(|t| t.contains(type_name));
        let label = if type_name == folder {
            t!("pane-type-folders")
        } else {
            type_name.clone()
        };
        let row = check_row(ui, on, Some(icon), &label, Some(*count));
        if row.clicked() {
            let mut types: BTreeSet<String> = cx.tab.types.clone().unwrap_or_default();
            if on {
                types.remove(type_name);
            } else {
                types.insert(type_name.clone());
            }
            cx.requests.push(Request::Command(Command::TypeFilter {
                pane: cx.pane,
                types: Some(types),
            }));
        }
    }
}

/// A group title: 11 px capitals in `text/secondary`.
fn group_title(ui: &mut egui::Ui, text: &str) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 27.0), egui::Sense::hover());
    ui.painter().text(
        egui::pos2(rect.left() + 10.0, rect.center().y + 2.0),
        egui::Align2::LEFT_CENTER,
        text.to_uppercase(),
        egui::FontId::proportional(TEXT_CAPS),
        theme::scheme(ui).text_secondary,
    );
}

/// A hairline between two groups, with 4 px of air on each side.
fn menu_separator(ui: &mut egui::Ui) {
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

/// A row with a switch at its right: the whole row flips it, and the row
/// is the checkbox a screen reader hears.
fn switch_row(ui: &mut egui::Ui, icon: &str, label: &str, on: bool) -> egui::Response {
    menu_row_with(ui, icon, label, false, Some(on), |ui| switch(ui, on))
}

/// One row of the menu: a 16 px icon slot at 10 px, the label at 36 px,
/// and `right` laid out from the right edge. The active entry and the
/// hovered one are `bg/selected-soft`.
fn menu_row(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    active: bool,
    right: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    menu_row_with(ui, icon, label, active, None, right)
}

/// [`menu_row`] whose accessible node is a checkbox in state `toggled`
/// when that is given, a button otherwise.
fn menu_row_with(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    active: bool,
    toggled: Option<bool>,
    right: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let scheme = theme::scheme(ui);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), MENU_ROW),
        egui::Sense::click(),
    );
    response.widget_info(|| match toggled {
        Some(on) => egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, label),
        None => egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label),
    });
    if active || response.hovered() {
        ui.painter().rect_filled(
            rect,
            theme::metrics(ui.ctx()).corner_radius(),
            scheme.bg_selected_soft,
        );
    }
    let colour = if active {
        scheme.text_on_selected
    } else {
        scheme.text_primary
    };
    ui.painter().text(
        egui::pos2(rect.left() + 10.0 + ICON / 2.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(ICON),
        if active {
            scheme.text_on_selected
        } else {
            scheme.icon_default
        },
    );
    ui.painter().text(
        egui::pos2(rect.left() + 36.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(TEXT_BODY),
        colour,
    );
    let mut side = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_max(
                egui::pos2(rect.left() + 36.0, rect.top()),
                egui::pos2(rect.right() - 10.0, rect.bottom()),
            ))
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    right(&mut side);
    response
}

/// A checkbox row: a 16 px box with a `border/strong` border, filled
/// `accent/focus` with a white check when ticked; then the icon slot, the
/// label and the count at the right.
fn check_row(
    ui: &mut egui::Ui,
    on: bool,
    icon: Option<&str>,
    label: &str,
    count: Option<usize>,
) -> egui::Response {
    let scheme = theme::scheme(ui);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), MENU_ROW),
        egui::Sense::click(),
    );
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, label));
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            theme::metrics(ui.ctx()).corner_radius(),
            scheme.bg_selected_soft,
        );
    }
    let box_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 10.0 + ICON / 2.0, rect.center().y),
        egui::Vec2::splat(ICON),
    );
    if on {
        ui.painter().rect_filled(box_rect, 3.0, scheme.accent_focus);
        ui.painter().text(
            box_rect.center(),
            egui::Align2::CENTER_CENTER,
            icons::DONE,
            egui::FontId::proportional(12.0),
            theme::Scheme::ON_BUTTON,
        );
    } else {
        ui.painter().rect_stroke(
            box_rect,
            3.0,
            egui::Stroke::new(1.0, scheme.border_strong),
            egui::StrokeKind::Inside,
        );
    }
    let mut x = rect.left() + 36.0;
    if let Some(icon) = icon {
        ui.painter().text(
            egui::pos2(x + ICON / 2.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(ICON),
            scheme.icon_default,
        );
        x += ICON + 10.0;
    }
    ui.painter().text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(TEXT_BODY),
        scheme.text_primary,
    );
    if let Some(count) = count {
        ui.painter().text(
            egui::pos2(rect.right() - 10.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::proportional(TEXT_SECONDARY),
            scheme.text_secondary,
        );
    }
    response
}

/// The segmented control of the three icon sizes: a `bg/control` track
/// with the chosen size raised as a `bg/card` tile. Returns a size that
/// was clicked.
fn icon_sizes(ui: &mut egui::Ui, current: Option<IconSize>) -> Option<IconSize> {
    let scheme = theme::scheme(ui);
    let (track, _) = ui.allocate_exact_size(egui::vec2(92.0, 28.0), egui::Sense::hover());
    ui.painter().rect_filled(track, 6.0, scheme.bg_control);
    let mut picked = None;
    let sizes = [
        (IconSize::Small, icons::ICONS_SMALL, t!("pane-icons-small")),
        (
            IconSize::Medium,
            icons::ICONS_MEDIUM,
            t!("pane-icons-medium"),
        ),
        (IconSize::Large, icons::ICONS_LARGE, t!("pane-icons-large")),
    ];
    for (i, (size, glyph, name)) in sizes.into_iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let x = track.left() + 2.0 + i as f32 * 30.0;
        let tile =
            egui::Rect::from_min_size(egui::pos2(x, track.top() + 2.0), egui::vec2(28.0, 24.0));
        let response = ui.interact(tile, ui.id().with(("icon-size", i)), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::RadioButton,
                true,
                current == Some(size),
                &name,
            )
        });
        if current == Some(size) {
            ui.painter().rect_filled(tile, 4.0, scheme.bg_card);
        } else if response.hovered() {
            ui.painter().rect_filled(tile, 4.0, scheme.bg_subtle);
        }
        ui.painter().text(
            tile.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(ICON),
            scheme.icon_default,
        );
        if response.on_hover_text(name).clicked() {
            picked = Some(size);
        }
    }
    picked
}

/// The path bar in `rect`: the breadcrumb, or the text field while editing.
fn path_bar(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, rect: egui::Rect) {
    if cx.tab_ui.path_edit.is_some() {
        path_edit(ui, cx, rect);
        return;
    }
    breadcrumb(ui, cx, rect);
}

/// The breadcrumb as an editable text field.
fn path_edit(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, rect: egui::Rect) {
    let Some(text) = cx.tab_ui.path_edit.as_mut() else {
        return;
    };
    let output = egui::TextEdit::singleline(text)
        .desired_width(rect.width())
        .show(ui);
    if cx.tab_ui.focus_path {
        output.response.request_focus();
        cx.tab_ui.focus_path = false;
    }
    let enter = output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if enter {
        let typed = text.trim().to_owned();
        cx.tab_ui.path_edit = None;
        if !typed.is_empty() {
            let location = paths::typed(&cx.tab.location, &typed);
            cx.requests.push(Request::Command(Command::Navigate {
                pane: cx.pane,
                to: location,
            }));
        }
    } else if escape || (output.response.lost_focus() && !enter) {
        cx.tab_ui.path_edit = None;
    }
}

/// Which segments fit: the index of the first shown segment and the width
/// they use, from the right, with the chevron reserved as soon as one has
/// to go. Even the last segment stays, truncated, when nothing fits.
fn fit_segments(widths: &[f32], separator_w: f32, chevron_w: f32, available: f32) -> (usize, f32) {
    let last = widths.len().saturating_sub(1);
    let mut first_shown = widths.len();
    let mut used = 0.0;
    for i in (0..widths.len()).rev() {
        let need = widths[i] + if i < last { separator_w } else { 0.0 };
        let reserve = if i > 0 { chevron_w } else { 0.0 };
        if used + need + reserve > available {
            if i == last {
                first_shown = last;
                used += need;
            }
            break;
        }
        used += need;
        first_shown = i;
    }
    (first_shown, used)
}

/// The breadcrumb: segments from the right as far as they fit, the rest
/// folded into a chevron with a dropdown, the last one truncated when even
/// it does not fit, and the copy button at the right end.
// The breadcrumb's collapsing, editing and copy button in one pass.
#[allow(clippy::too_many_lines)]
fn breadcrumb(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, rect: egui::Rect) {
    let segments: Vec<paths::Crumb> = cx.tab_ui.crumbs_of(&cx.tab.location).to_vec();
    let last = segments.len().saturating_sub(1);
    let spacing = ui.spacing().item_spacing.x;
    let pad = 2.0 * ui.spacing().button_padding.x;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text_width = |ui: &egui::Ui, text: &str| {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE)
                .size()
                .x
        })
    };
    let separator_w = text_width(ui, icons::SEGMENT) + spacing;
    let copy_w = button_size(ui) + spacing;
    let chevron_w = button_size(ui) + separator_w;
    let widths: Vec<f32> = segments
        .iter()
        .map(|crumb| text_width(ui, &crumb.label) + pad)
        .collect();

    // The parent chip and its divider come first and take their room off
    // the segments'.
    let chip_w = CHIP.x + 4.0 + 1.0 + 6.0 + spacing;
    let available = (rect.width() - copy_w - chip_w).max(0.0);
    let (first_shown, used) = fit_segments(&widths, separator_w, chevron_w, available);
    remember(cx, "middle", rect);
    parent_chip(ui, cx);

    if first_shown > 0 {
        let hidden: Vec<paths::Crumb> = segments[..first_shown].to_vec();
        let response = ui.menu_button(icons::COLLAPSED, |ui| {
            for crumb in hidden {
                if ui.button(crumb.label).clicked() {
                    cx.requests.push(Request::Command(Command::Navigate {
                        pane: cx.pane,
                        to: crumb.location,
                    }));
                    ui.close();
                }
            }
        });
        response.response.on_hover_text(t!("pane-hidden-ancestors"));
        ui.weak(icons::SEGMENT);
    }
    let room_for_last = (available - (used - widths[last])).max(24.0);
    for (i, crumb) in segments.iter().enumerate().skip(first_shown) {
        let label = &crumb.label;
        let text = if i == last {
            egui::RichText::new(label).strong()
        } else {
            egui::RichText::new(label)
        };
        let mut button = egui::Button::new(text).frame(false);
        if i == last {
            button = button
                .wrap_mode(egui::TextWrapMode::Truncate)
                .min_size(egui::vec2(0.0, 0.0));
        }
        let response = if i == last {
            ui.add_sized(
                egui::vec2(widths[i].min(room_for_last), ui.spacing().interact_size.y),
                button,
            )
        } else {
            ui.add(button)
        };
        response
            .clone()
            .on_hover_text(paths::text_of(&crumb.location));
        crumb_zone(ui, cx, i, i != last, &crumb.location, response.rect);
        if response.clicked() && i != last {
            cx.requests.push(Request::Command(Command::Navigate {
                pane: cx.pane,
                to: crumb.location.clone(),
            }));
        }
        if i != last {
            ui.weak(icons::SEGMENT);
        }
    }

    // The copy button, inside the breadcrumb at its right end; empty space
    // before it switches to editing.
    let copy = ui
        .add(egui::Button::new(icons::COPY).frame(false))
        .on_hover_text(t!("pane-copy-path"));
    if copy.clicked() {
        cx.requests.push(Request::RunAction {
            action: ext::ActionId("core.copy_path"),
            ctx: ext::ActionCtx {
                pane: cx.pane,
                location: cx.tab.location.clone(),
                selection: Vec::new(),
                invoked_from: ext::Invocation::Menu,
                other: None,
            },
        });
    }
    let remaining = ui.available_size_before_wrap();
    let (_, response) = ui.allocate_exact_size(
        egui::vec2(
            remaining.x.max(8.0),
            remaining.y.max(ui.spacing().interact_size.y),
        ),
        egui::Sense::click(),
    );
    if response.clicked() {
        cx.tab_ui.path_edit = Some(paths::text_of(&cx.tab.location));
        cx.tab_ui.focus_path = true;
    }
}

/// The parent chip: `ARROW_UP` on a `bg/subtle` chip with `radius/control`
/// corners, click for the parent folder, then a 14 px hairline (SPEC.md
/// *Path bar*). A location without a parent shows the chip faded.
fn parent_chip(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let has_parent = cx.tab_ui.crumbs_of(&cx.tab.location).len() >= 2;
    ui.add_space(4.0);
    let (rect, response) = ui.allocate_exact_size(
        CHIP,
        if has_parent {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            has_parent,
            t!("pane-parent-directory"),
        )
    });
    let scheme = theme::scheme(ui);
    let fill = if response.hovered() && has_parent {
        scheme.bg_control
    } else {
        scheme.bg_subtle
    };
    ui.painter()
        .rect_filled(rect, theme::metrics(ui.ctx()).corner_radius(), fill);
    let colour = if has_parent {
        scheme.icon_default
    } else {
        scheme.icon_default.gamma_multiply(0.5)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::UP,
        egui::FontId::proportional(ICON),
        colour,
    );
    if response
        .on_hover_text(t!("pane-parent-directory"))
        .clicked()
    {
        intent(cx, Intent::Parent);
    }
    ui.add_space(6.0);
    divider(ui, 14.0);
    ui.add_space(6.0);
    // A drop on the chip goes to the parent folder, as on a segment.
    if has_parent && let Some(parent) = paths::parent_of(cx.tab_ui.crumbs_of(&cx.tab.location)) {
        crumb_zone(ui, cx, usize::MAX, true, &parent, rect);
    }
}

/// A breadcrumb segment as a drop target: into that ancestor; hovering one
/// that is not the current folder drills into it.
fn crumb_zone(
    ui: &egui::Ui,
    cx: &mut PaneCx<'_>,
    index: usize,
    ancestor: bool,
    to: &Location,
    rect: egui::Rect,
) {
    if cx.dnd.active()
        && cx.dnd.zone(Zone {
            key: egui::Id::new(("dnd-crumb", cx.tab_id, index)),
            rect,
            target: dnd::into_pane(
                cx.tab,
                Target::Folder {
                    to: to.clone(),
                    device: dnd::listing_device(cx.tab),
                    spring: ancestor.then_some(Spring::Pane(cx.pane)),
                },
            ),
        })
    {
        dnd::highlight(ui.painter(), ui.visuals(), rect);
    }
}

/// The footer (SPEC.md *Footer*): as tall as the toolbar, under a
/// hairline, with `space/toolbar-x` padding. Left to right: the
/// hidden-files toggle, the statistics, a divider and the selection pill
/// while something is selected, the pane's note; the filter field at the
/// right.
fn foot(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let metrics = theme::metrics(ui.ctx());
    let height = metrics.toolbar_height();
    let (row, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    cx.tab_ui.footer_row = Some(row);
    let scheme = theme::scheme(ui);
    ui.painter()
        .hline(row.x_range(), row.top() + 0.5, scheme.hairline());
    let inner = row.shrink2(egui::vec2(metrics.space_toolbar_x, 0.0));
    // The filter field at the right, fixed.
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    filter_field(&mut right, cx);
    let right_edge = right.min_rect().left();
    // The rest from the left, clipped before the field.
    let gap = ui.spacing().item_spacing.x;
    let left_rect = egui::Rect::from_min_max(
        inner.min,
        egui::pos2((right_edge - gap).max(inner.left()), inner.bottom()),
    );
    let mut left = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    left.set_clip_rect(left_rect.intersect(left.clip_rect()));
    left.spacing_mut().item_spacing.x = 12.0;
    hidden_toggle(&mut left, cx);
    statistics(&mut left, cx);
    if !cx.tab.selection.is_empty() {
        divider(&mut left, 16.0);
        selection_pill(&mut left, cx);
    }
    if let Some(note) = &cx.tab.note {
        divider(&mut left, 16.0);
        left.add(
            egui::Label::new(
                egui::RichText::new(note.as_str())
                    .size(TEXT_SECONDARY)
                    .color(scheme.text_secondary),
            )
            .selectable(false)
            .truncate(),
        );
    }
}

/// The hidden-files toggle, icon only: `EYE` on `bg/selected-soft` while
/// hidden files are shown, `EYE_SLASH` as a ghost button while they are
/// hidden.
fn hidden_toggle(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let shown = cx.tab.show_hidden;
    let scheme = theme::scheme(ui);
    let size = button_size(ui);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    let radius = theme::metrics(ui.ctx()).corner_radius();
    let (glyph, colour) = if shown {
        ui.painter()
            .rect_filled(rect, radius, scheme.bg_selected_soft);
        (icons::HIDDEN_SHOWN, scheme.text_on_selected)
    } else {
        if response.hovered() {
            ui.painter().rect_filled(rect, radius, scheme.bg_subtle);
        }
        (icons::HIDDEN_HIDDEN, scheme.icon_default)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(ICON),
        colour,
    );
    let text = if shown {
        t!("footer-hide-hidden")
    } else {
        t!("pane-show-hidden")
    };
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, shown, text.clone())
    });
    if response.on_hover_text(text).clicked() {
        intent(cx, Intent::ToggleHidden);
    }
}

/// The statistics in 12 px `text/secondary`: `FOLDER` and the folder
/// count, `FILE` and the file count, the files' total size; while a filter
/// narrows the list the counts read as *12 of 340*, and *listing…* follows
/// while entries still arrive.
fn statistics(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let listing = &cx.tab.listing;
    let count = |entries: &mut dyn Iterator<Item = &types::Entry>| {
        entries.fold((0usize, 0usize, 0u64), |(d, f, b), e| {
            if e.kind == EntryKind::Dir {
                (d + 1, f, b)
            } else {
                (d, f + 1, b + e.size.unwrap_or(0))
            }
        })
    };
    let (folders, files, bytes) = count(&mut listing.rows());
    let narrowed = !cx.tab.filter.is_empty() && !cx.tab.is_searching();
    let (folders_text, folders_label, files_text, files_label) = if narrowed {
        let (all_folders, all_files, _) = count(&mut listing.entries().iter());
        (
            t!("footer-of", shown = folders, total = all_folders),
            t!("footer-folders-of", shown = folders, total = all_folders),
            t!("footer-of", shown = files, total = all_files),
            t!("footer-files-of", shown = files, total = all_files),
        )
    } else {
        (
            folders.to_string(),
            t!("footer-folders", count = folders),
            files.to_string(),
            t!("footer-files", count = files),
        )
    };
    stat(ui, icons::FOLDER, &folders_text, &folders_label);
    stat(ui, icons::FILE, &files_text, &files_label);
    let size = format::size(Some(bytes));
    secondary_text(ui, &size, &t!("footer-total-size", size = size.as_str()));
    if !listing.complete {
        let listing_text = t!("footer-listing");
        secondary_text(ui, &listing_text, &listing_text);
    }
}

/// One statistic: a 16 px glyph in `icon/default` and its number, read
/// aloud as `label`.
fn stat(ui: &mut egui::Ui, glyph: &str, text: &str, label: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let scheme = theme::scheme(ui);
        ui.add(
            egui::Label::new(
                egui::RichText::new(glyph)
                    .size(ICON)
                    .color(scheme.icon_default),
            )
            .selectable(false),
        );
        secondary_text(ui, text, label);
    });
}

/// A 12 px `text/secondary` label whose accessible name is `label`.
fn secondary_text(ui: &mut egui::Ui, text: &str, label: &str) {
    let scheme = theme::scheme(ui);
    let response = ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .size(TEXT_SECONDARY)
                .color(scheme.text_secondary),
        )
        .selectable(false),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.to_owned()));
}

/// The selection pill: `bg/selected-soft` with `text/on-selected`,
/// *1 selected · 568 B*.
fn selection_pill(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let (count, bytes) = cx
        .tab
        .selected()
        .fold((0usize, 0u64), |(n, b), e| (n + 1, b + e.size.unwrap_or(0)));
    let text = t!(
        "footer-selected",
        count = count,
        size = format::size(Some(bytes)).as_str()
    );
    let scheme = theme::scheme(ui);
    let galley = ui.painter().layout_no_wrap(
        text.clone(),
        egui::FontId::proportional(TEXT_SECONDARY),
        scheme.text_on_selected,
    );
    let size = galley.size() + egui::vec2(16.0, 6.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, rect.height() / 2.0, scheme.bg_selected_soft);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        scheme.text_on_selected,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text.clone()));
}

/// The filter field (SPEC.md *Footer*): 200 × 26, a `MAGNIFYING_GLASS` at
/// 60 % opacity, the placeholder *Filter*, and inside its right end the
/// include-subfolders toggle when a subtree search is available. Typing
/// narrows the list live, Escape clears the text and returns the focus to
/// the list, Enter opens the first match.
fn filter_field(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    let (rect, _) = ui.allocate_exact_size(FILTER_FIELD, egui::Sense::hover());
    ui.painter().rect(
        rect,
        radius,
        scheme.bg_pane,
        egui::Stroke::new(1.0, scheme.border_default),
        egui::StrokeKind::Inside,
    );
    let glass = egui::Rect::from_min_size(
        rect.min + egui::vec2(FIELD_PAD, 0.0),
        egui::vec2(ICON, rect.height()),
    );
    ui.painter().text(
        glass.center(),
        egui::Align2::CENTER_CENTER,
        icons::SEARCH,
        egui::FontId::proportional(ICON),
        scheme.icon_default.gamma_multiply(0.6),
    );
    let mut right = rect.right() - FIELD_PAD;
    if cx.app.model().subtree_search {
        let toggle = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 2.0 - FIELD_TOGGLE / 2.0, rect.center().y),
            egui::vec2(FIELD_TOGGLE, FIELD_TOGGLE),
        );
        subtree_toggle(ui, cx, toggle);
        right = toggle.left() - 4.0;
    }
    let text_rect = egui::Rect::from_min_max(
        egui::pos2(glass.right() + 6.0, rect.top()),
        egui::pos2(right.max(glass.right() + 6.0), rect.bottom()),
    );
    let mut field = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text_rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    field.set_clip_rect(text_rect.intersect(field.clip_rect()));
    let mut text = cx.tab.filter.clone();
    let output = egui::TextEdit::singleline(&mut text)
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .font(egui::FontId::proportional(TEXT_BODY))
        .text_color(scheme.text_primary)
        .hint_text(
            egui::RichText::new(t!("footer-filter"))
                .size(TEXT_BODY)
                .color(scheme.text_placeholder),
        )
        .desired_width(text_rect.width())
        .vertical_align(Align::Center)
        .show(&mut field);
    if cx.tab_ui.focus_filter {
        output.response.request_focus();
        cx.tab_ui.focus_filter = false;
    }
    if output.response.changed() {
        cx.requests.push(Request::Command(Command::Filter {
            pane: cx.pane,
            text,
        }));
    }
    if output.response.lost_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            intent(cx, Intent::Open);
        } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !cx.tab.filter.is_empty() {
            cx.requests.push(Request::Command(Command::Filter {
                pane: cx.pane,
                text: String::new(),
            }));
        }
    }
}

/// The include-subfolders toggle inside the filter field: `TREE_STRUCTURE`,
/// on `bg/selected-soft` while on.
fn subtree_toggle(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, rect: egui::Rect) {
    let on = cx.tab.subtree;
    let scheme = theme::scheme(ui);
    let response = ui.interact(
        rect,
        ui.id().with(("subtree", cx.tab_id)),
        egui::Sense::click(),
    );
    let colour = if on {
        ui.painter().rect_filled(rect, 4.0, scheme.bg_selected_soft);
        scheme.text_on_selected
    } else {
        if response.hovered() {
            ui.painter().rect_filled(rect, 4.0, scheme.bg_subtle);
        }
        scheme.icon_default
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::SUBTREE,
        egui::FontId::proportional(ICON),
        colour,
    );
    let label = t!("footer-subtree");
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, label.clone())
    });
    let hint = if on {
        t!("footer-subtree-on")
    } else {
        t!("footer-subtree-off")
    };
    if response.on_hover_text(hint).clicked() {
        intent(cx, Intent::ToggleSubtree);
    }
}
