//! One tab's content: toolbar with path bar, the file view, and the foot
//! with the status line and the filter.

use std::fmt::Write as _;

use app::Command;
use egui::Align;
use types::{EntryKind, Location};

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

fn toolbar(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    ui.horizontal(|ui| {
        let back = ui
            .add_enabled(
                !cx.tab.history.back.is_empty(),
                egui::Button::new(icons::BACK),
            )
            .on_hover_text("Back");
        if back.clicked() {
            intent(cx, Intent::Back);
        }
        let forward = ui
            .add_enabled(
                !cx.tab.history.forward.is_empty(),
                egui::Button::new(icons::FORWARD),
            )
            .on_hover_text("Forward");
        if forward.clicked() {
            intent(cx, Intent::Forward);
        }
        let up = ui
            .add_enabled(
                paths::parent(&cx.tab.location.path).is_some(),
                egui::Button::new(icons::UP),
            )
            .on_hover_text("Parent directory");
        if up.clicked() {
            intent(cx, Intent::Parent);
        }
        ui.separator();

        // Right-aligned controls first, so the path bar gets the rest.
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if ui.button(icons::REFRESH).on_hover_text("Refresh").clicked() {
                intent(cx, Intent::Refresh);
            }
            let details = cx.tab.view != model::ViewMode::List;
            if ui
                .selectable_label(details, icons::VIEW_DETAILS)
                .on_hover_text("Details view")
                .clicked()
            {
                intent(cx, Intent::ViewDetails);
            }
            if ui
                .selectable_label(!details, icons::VIEW_LIST)
                .on_hover_text("List view")
                .clicked()
            {
                intent(cx, Intent::ViewList);
            }
            if ui
                .selectable_label(cx.tab.show_hidden, "Hidden")
                .on_hover_text("Show hidden files")
                .clicked()
            {
                intent(cx, Intent::ToggleHidden);
            }
            if ui.button(icons::COPY).on_hover_text("Copy path").clicked() {
                let text = cx.tab.location.path.to_string_lossy().into_owned();
                if let Err(error) = cx.app.platform().set_text(&text) {
                    cx.requests.push(Request::Toast(
                        ext::Level::Error,
                        format!("Copy path: {error}"),
                    ));
                }
            }
            ui.separator();
            ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                path_bar(ui, cx);
            });
        });
    });
}

fn path_bar(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    if let Some(text) = cx.tab_ui.path_edit.as_mut() {
        let output = egui::TextEdit::singleline(text)
            .desired_width(f32::INFINITY)
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
                let location = Location::new(cx.tab.location.provider, typed.as_str());
                cx.requests.push(Request::Command(Command::Navigate {
                    pane: cx.pane,
                    to: location,
                }));
            }
        } else if escape || (output.response.lost_focus() && !enter) {
            cx.tab_ui.path_edit = None;
        }
        return;
    }
    let segments = paths::segments(&cx.tab.location.path);
    let last = segments.len().saturating_sub(1);
    for (i, (label, prefix)) in segments.into_iter().enumerate() {
        let button = egui::Button::new(if i == last {
            egui::RichText::new(&label).strong()
        } else {
            egui::RichText::new(&label)
        })
        .frame(false);
        if ui.add(button).clicked() && i != last {
            let location = Location::new(cx.tab.location.provider, prefix);
            cx.requests.push(Request::Command(Command::Navigate {
                pane: cx.pane,
                to: location,
            }));
        }
        if i != last {
            ui.weak("›");
        }
    }
    let rest = ui.available_size_before_wrap();
    let (_, response) = ui.allocate_exact_size(
        egui::vec2(rest.x.max(8.0), rest.y.max(ui.spacing().interact_size.y)),
        egui::Sense::click(),
    );
    if response.clicked() {
        cx.tab_ui.path_edit = Some(cx.tab.location.path.to_string_lossy().into_owned());
        cx.tab_ui.focus_path = true;
    }
    response.on_hover_text("Click to edit the path");
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
            let mut text = cx.tab.filter.clone();
            let output = egui::TextEdit::singleline(&mut text)
                .hint_text("Filter")
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
    text
}
