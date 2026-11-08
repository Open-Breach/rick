//! One tab's content: toolbar with path bar, the file view, and the foot
//! with the status line and the filter.

use std::fmt::Write as _;

use app::Command;
use egui::Align;
use model::{IconSize, ViewMode};
use types::{EntryKind, Location};

use crate::dnd::{self, Spring, Target, Zone};
use crate::keys::Intent;
use crate::shell::{PaneCx, Request};
use crate::{format, icons, paths, views};

/// Draws the whole tab.
pub fn show(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    egui::Panel::top(egui::Id::new(("toolbar", cx.tab_id)))
        .show_separator_line(false)
        .show(ui, |ui| toolbar(ui, cx));
    egui::Panel::bottom(egui::Id::new(("foot", cx.tab_id)))
        .show_separator_line(true)
        .show(ui, |ui| foot(ui, cx));
    egui::CentralPanel::no_frame().show(ui, |ui| views::show(ui, cx));
}

fn intent(cx: &mut PaneCx<'_>, intent: Intent) {
    cx.requests.push(Request::Intent {
        intent,
        pane: cx.pane,
    });
}

/// Width of an icon-only toolbar button, so the two icon groups keep their
/// width whatever the window size.
const ICON_BUTTON: f32 = 28.0;

/// The toolbar: a fixed group of navigation icons on the left, a fixed group
/// of view controls on the right, and the breadcrumb flexing in between,
/// never overlapping either.
/// The toolbar row's height for a density: room for the icon buttons plus
/// padding above and below them, the density being the single knob.
#[must_use]
pub fn toolbar_height(density: model::Density) -> f32 {
    ICON_BUTTON
        + match density {
            model::Density::Compact => 6.0,
            model::Density::Normal => 10.0,
            model::Density::Comfortable => 16.0,
        }
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
    ui.add_enabled(
        enabled,
        egui::Button::new(glyph).min_size(egui::vec2(ICON_BUTTON, ICON_BUTTON)),
    )
}

fn remember(cx: &mut PaneCx<'_>, group: &'static str, rect: egui::Rect) {
    cx.tab_ui.toolbar_rects.push((group, rect));
}

fn left_group(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let back = icon_button(ui, icons::BACK, !cx.tab.history.back.is_empty()).on_hover_text("Back");
    remember(cx, "left", back.rect);
    if back.clicked() {
        intent(cx, Intent::Back);
    }
    let forward = icon_button(ui, icons::FORWARD, !cx.tab.history.forward.is_empty())
        .on_hover_text("Forward");
    remember(cx, "left", forward.rect);
    if forward.clicked() {
        intent(cx, Intent::Forward);
    }
    let has_parent = cx.tab_ui.crumbs_of(&cx.tab.location).len() >= 2;
    let up = icon_button(ui, icons::UP, has_parent).on_hover_text("Parent directory");
    remember(cx, "left", up.rect);
    if up.clicked() {
        intent(cx, Intent::Parent);
    }
}

/// View mode dropdown, hidden-files toggle and refresh, right to left.
fn right_group(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let refresh = icon_button(ui, icons::REFRESH, true).on_hover_text("Refresh");
    remember(cx, "right", refresh.rect);
    if refresh.clicked() {
        intent(cx, Intent::Refresh);
    }
    let hidden_glyph = if cx.tab.show_hidden {
        icons::HIDDEN_SHOWN
    } else {
        icons::HIDDEN_HIDDEN
    };
    let hidden = ui
        .add(
            egui::Button::new(hidden_glyph)
                .selected(cx.tab.show_hidden)
                .min_size(egui::vec2(ICON_BUTTON, ICON_BUTTON)),
        )
        .on_hover_text(if cx.tab.show_hidden {
            "Hide hidden files"
        } else {
            "Show hidden files"
        });
    remember(cx, "right", hidden.rect);
    if hidden.clicked() {
        intent(cx, Intent::ToggleHidden);
    }
    view_mode_dropdown(ui, cx);
}

/// The view modes the dropdown offers, in order: label, mode, available.
fn view_modes() -> [(&'static str, ViewMode, bool); 7] {
    [
        ("List", ViewMode::List, true),
        ("Details", ViewMode::Details, true),
        ("Tree", ViewMode::Tree, true),
        ("Icons, small", ViewMode::Icons(IconSize::Small), true),
        ("Icons, medium", ViewMode::Icons(IconSize::Medium), true),
        ("Icons, large", ViewMode::Icons(IconSize::Large), true),
        (
            "Icons, extra large",
            ViewMode::Icons(IconSize::ExtraLarge),
            true,
        ),
    ]
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

fn view_mode_dropdown(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let current = cx.tab.view;
    let label = format!("{} {}", view_mode_icon(current), icons::DROPDOWN);
    ui.style_mut().spacing.interact_size.y = ICON_BUTTON;
    let response = ui.menu_button(label, |ui| {
        for (label, mode, available) in view_modes() {
            let marked = if mode == current {
                format!("{} {label}", icons::DONE)
            } else {
                format!("   {label}")
            };
            let clicked = ui
                .add_enabled(available, egui::Button::new(marked))
                .on_disabled_hover_text("Not implemented yet")
                .clicked();
            if clicked {
                cx.requests.push(Request::Command(Command::SetView {
                    pane: cx.pane,
                    mode,
                }));
                ui.close();
            }
        }
    });
    remember(cx, "right", response.response.rect);
    response.response.on_hover_text("View mode");
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
    let copy_w = ICON_BUTTON + spacing;
    let chevron_w = ICON_BUTTON + separator_w;
    let widths: Vec<f32> = segments
        .iter()
        .map(|crumb| text_width(ui, &crumb.label) + pad)
        .collect();

    let available = (rect.width() - copy_w).max(0.0);
    let (first_shown, used) = fit_segments(&widths, separator_w, chevron_w, available);
    remember(cx, "middle", rect);

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
        response.response.on_hover_text("Hidden ancestors");
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
        .on_hover_text("Copy path");
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

fn foot(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    ui.horizontal(|ui| {
        ui.label(status_line(cx));
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if !cx.tab.filter.is_empty()
                && ui
                    .small_button(icons::CLOSE)
                    .on_hover_text("Clear filter")
                    .clicked()
            {
                cx.requests.push(Request::Command(Command::Filter {
                    pane: cx.pane,
                    text: String::new(),
                }));
            }
            // Search in subdirectories: the filter text matches the
            // subtree, when a search index that walks one is loaded.
            if cx.app.model().subtree_search {
                let subtree = ui
                    .add(
                        egui::Button::new(icons::SUBTREE)
                            .small()
                            .selected(cx.tab.subtree),
                    )
                    .on_hover_text(if cx.tab.subtree {
                        "Searching in subdirectories; click to filter this folder only"
                    } else {
                        "Search in subdirectories"
                    });
                if subtree.clicked() {
                    intent(cx, Intent::ToggleSubtree);
                }
            }
            let mut text = cx.tab.filter.clone();
            let output = egui::TextEdit::singleline(&mut text)
                .hint_text(if cx.tab.subtree {
                    "Search in subdirectories"
                } else {
                    "Filter"
                })
                .desired_width(180.0)
                .show(ui);
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
            if output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                intent(cx, Intent::Open);
            }
        });
    });
}

/// "3 folders, 12 files, 4.2 MiB · 2 selected, 1.1 MiB", or "12 of 340
/// entries" while a filter narrows the list.
fn status_line(cx: &PaneCx<'_>) -> String {
    let listing = &cx.tab.listing;
    let (mut folders, mut files, mut bytes) = (0usize, 0usize, 0u64);
    for entry in listing.rows() {
        if entry.kind == EntryKind::Dir {
            folders += 1;
        } else {
            files += 1;
            bytes += entry.size.unwrap_or(0);
        }
    }
    let mut text = if cx.tab.filter.is_empty() {
        format!(
            "{folders} folders, {files} files, {}",
            format::size(Some(bytes))
        )
    } else if cx.tab.is_searching() {
        format!("{} matches in subdirectories", listing.row_count())
    } else {
        format!("{} of {} entries", listing.row_count(), listing.len())
    };
    if !cx.tab.selection.is_empty() {
        let (count, selected_bytes) = cx
            .tab
            .selected()
            .fold((0usize, 0u64), |(n, b), e| (n + 1, b + e.size.unwrap_or(0)));
        let _ = write!(
            text,
            " · {count} selected, {}",
            format::size(Some(selected_bytes))
        );
    }
    if !listing.complete {
        text.push_str(" · listing…");
    }
    if let Some(note) = &cx.tab.note {
        let _ = write!(text, " · {note}");
    }
    text
}
