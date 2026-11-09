//! Small controls shared by the menus, the dialogues and the settings
//! (SPEC.md *Dialogues*, *Settings window*): the switch, the segmented
//! control, a pill, the two button kinds.

use egui::{Align2, FontId, Rect, Sense, pos2, vec2};

use crate::theme::{self, Scheme, TEXT_BODY, TEXT_SECONDARY};

/// A switch, 32 × 18: `accent/focus` with the knob at the right while on,
/// `bg/switch-off` with it at the left while off. Paint only; the caller
/// owns the click.
pub fn switch(ui: &mut egui::Ui, on: bool) {
    let scheme = theme::scheme(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(32.0, 18.0), Sense::hover());
    let fill = if on {
        scheme.accent_focus
    } else {
        scheme.bg_switch_off
    };
    ui.painter().rect_filled(rect, 9.0, fill);
    let knob_x = if on {
        rect.right() - 9.0
    } else {
        rect.left() + 9.0
    };
    ui.painter()
        .circle_filled(pos2(knob_x, rect.center().y), 7.0, Scheme::ON_BUTTON);
}

/// A segmented control in `rect`: a `bg/control` track with `radius`
/// corners, the options side by side, the chosen one raised as a
/// `bg/pane` tile. Returns an option that was clicked.
pub fn segmented(
    ui: &mut egui::Ui,
    rect: Rect,
    id: egui::Id,
    options: &[String],
    selected: usize,
) -> Option<usize> {
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    ui.painter().rect_filled(rect, radius, scheme.bg_control);
    let count = options.len().max(1);
    #[allow(clippy::cast_precision_loss)]
    let width = rect.width() / count as f32;
    let mut clicked = None;
    for (index, option) in options.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let tile = Rect::from_min_size(
            pos2(rect.left() + width * index as f32, rect.top()),
            vec2(width, rect.height()),
        )
        .shrink(2.0);
        let response = ui.interact(tile, id.with(index), Sense::click());
        if index == selected {
            ui.painter().rect_filled(tile, radius, scheme.bg_pane);
        } else if response.hovered() {
            ui.painter().rect_filled(tile, radius, scheme.bg_subtle);
        }
        ui.painter().text(
            tile.center(),
            Align2::CENTER_CENTER,
            option,
            FontId::proportional(TEXT_BODY),
            if index == selected {
                scheme.text_primary
            } else {
                scheme.text_secondary
            },
        );
        let spoken = option.clone();
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::RadioButton,
                true,
                index == selected,
                spoken.clone(),
            )
        });
        if response.clicked() {
            clicked = Some(index);
        }
    }
    clicked
}

/// A small pill: `text` in 11 px on a rounded `bg` tint. Returns the
/// rectangle it took.
pub fn pill(ui: &mut egui::Ui, text: &str, bg: egui::Color32, fg: egui::Color32) -> Rect {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(11.0), fg);
    let size = galley.size() + vec2(14.0, 4.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, rect.height() / 2.0, bg);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, fg);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text.to_owned()));
    rect
}

/// A dialogue button, 36 px tall with `radius/control` corners: the
/// primary filled `accent/button` with white text, a secondary one on
/// `bg/control`. Disabled, the primary is washed out.
pub fn button(ui: &mut egui::Ui, label: &str, primary: bool, enabled: bool) -> egui::Response {
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::proportional(TEXT_BODY),
        scheme.text_primary,
    );
    let size = vec2(galley.size().x + 32.0, 36.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let (fill, fg) = match (primary, enabled) {
        (true, true) => (scheme.accent_button, Scheme::ON_BUTTON),
        (true, false) => (scheme.accent_button.gamma_multiply(0.45), Scheme::ON_BUTTON),
        (false, _) => (scheme.bg_control, scheme.text_primary),
    };
    let fill = if response.hovered() && enabled {
        fill.gamma_multiply(0.9)
    } else {
        fill
    };
    ui.painter().rect_filled(rect, radius, fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, fg);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label.to_owned())
    });
    response
}

/// A caps heading in 11 px letter-spaced `text/secondary`, as the
/// dialogues and the settings title their groups.
pub fn caps_heading(ui: &mut egui::Ui, text: &str) {
    let scheme = theme::scheme(ui);
    ui.add(
        egui::Label::new(
            egui::RichText::new(text.to_uppercase())
                .size(theme::TEXT_CAPS)
                .color(scheme.text_secondary)
                .extra_letter_spacing(0.66),
        )
        .selectable(false),
    );
    let _ = TEXT_SECONDARY;
}
