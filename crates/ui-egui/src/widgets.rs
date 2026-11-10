//! Small controls shared by the menus, the dialogues and the settings
//! (SPEC.md *Dialogues*, *Settings window*): the switch, the checkbox,
//! the segmented control, a pill, the two button kinds; and the pane
//! handle and the
//! hairline divider every pane's toolbar ends in (SPEC.md *Panes*).

use egui::{Align2, FontId, Rect, Sense, pos2, vec2};
use ext::t;

use crate::icons;
use crate::theme::{self, ICON, Scheme, TEXT_BODY, TEXT_SECONDARY};

/// The height of a field or dropdown in a dialogue.
pub const FIELD_HEIGHT: f32 = 36.0;
/// The grab handle's width.
pub const HANDLE_WIDTH: f32 = 28.0;

/// A pane's grab handle (SPEC.md *Panes*): a [`HANDLE_WIDTH`] wide,
/// `button/size` tall ghost area with `DOTS_SIX_VERTICAL` at 70 %
/// opacity, `bg/subtle` while hovered or dragged, the grab cursor over it
/// and the grabbing cursor while it drags; *Drag pane* as its accessible
/// name and tooltip. The caller starts the pane drag on `drag_started`.
pub fn pane_handle(ui: &mut egui::Ui) -> egui::Response {
    let size = theme::metrics(ui.ctx()).button_size;
    let (rect, response) =
        ui.allocate_exact_size(vec2(HANDLE_WIDTH, size), Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t!("pane-drag-pane"))
    });
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
        Align2::CENTER_CENTER,
        icons::HANDLE,
        FontId::proportional(ICON),
        scheme.icon_default.gamma_multiply(0.7),
    );
    if response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    response.on_hover_text(t!("pane-drag-pane"))
}

/// A 1 px hairline, `height` tall, centred in the row.
pub fn divider(ui: &mut egui::Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
    let x = rect.center().x.round() + 0.5;
    ui.painter().line_segment(
        [pos2(x, rect.top()), pos2(x, rect.bottom())],
        theme::scheme(ui).hairline(),
    );
}

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

/// A switch that takes the click itself: [`switch`]'s look, `Sense::click`,
/// read aloud as a checkbox named `label`. Returns the response.
pub fn toggle(ui: &mut egui::Ui, on: bool, label: &str) -> egui::Response {
    let scheme = theme::scheme(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(32.0, 18.0), Sense::click());
    let fill = if on {
        scheme.accent_focus
    } else {
        scheme.bg_switch_off
    };
    let fill = if response.hovered() {
        fill.gamma_multiply(0.9)
    } else {
        fill
    };
    ui.painter().rect_filled(rect, 9.0, fill);
    let knob_x = if on {
        rect.right() - 9.0
    } else {
        rect.left() + 9.0
    };
    ui.painter()
        .circle_filled(pos2(knob_x, rect.center().y), 7.0, Scheme::ON_BUTTON);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, label.to_owned())
    });
    response
}

/// A checkbox with its label: a 16 px box with a `border/strong` border,
/// filled `accent/focus` with a white check when ticked, then `label` in
/// the body size. The box and the label take the click; returns the
/// response.
pub fn checkbox(ui: &mut egui::Ui, on: bool, label: &str) -> egui::Response {
    let scheme = theme::scheme(ui);
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::proportional(TEXT_BODY),
        scheme.text_primary,
    );
    let size = vec2(ICON + 8.0 + galley.size().x, ICON.max(galley.size().y));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let box_rect = Rect::from_center_size(
        pos2(rect.left() + ICON / 2.0, rect.center().y),
        vec2(ICON, ICON),
    );
    paint_check(ui, box_rect, on);
    ui.painter().galley(
        pos2(
            box_rect.right() + 8.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        scheme.text_primary,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, label.to_owned())
    });
    response
}

/// The box of a checkbox in `rect`: filled `accent/focus` with a white
/// check when ticked, a `border/strong` outline when not.
fn paint_check(ui: &egui::Ui, rect: Rect, on: bool) {
    let scheme = theme::scheme(ui);
    if on {
        ui.painter().rect_filled(rect, 3.0, scheme.accent_focus);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            icons::DONE,
            FontId::proportional(12.0),
            Scheme::ON_BUTTON,
        );
    } else {
        ui.painter().rect_stroke(
            rect,
            3.0,
            egui::Stroke::new(1.0, scheme.border_strong),
            egui::StrokeKind::Inside,
        );
    }
}

/// A checkbox without a label of its own, its 16 px box centred on
/// `center`: one cell of a grid whose row and column say what it is, and
/// `name` says it to a screen reader. A little room around the box takes
/// the click too; returns the response, the caller owns the value.
pub fn check_at(ui: &mut egui::Ui, center: egui::Pos2, on: bool, name: &str) -> egui::Response {
    let rect = Rect::from_center_size(center, vec2(ICON, ICON));
    let response = ui.interact(
        rect.expand(5.0),
        ui.id().with(("check", name)),
        Sense::click(),
    );
    paint_check(ui, rect, on);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, name.to_owned())
    });
    response
}

/// A dropdown `width` wide: a `bg/pane` button with a `border/default`
/// frame and `radius/control` corners showing the chosen option and a
/// caret, the options listed when open. Returns the option clicked.
pub fn dropdown(
    ui: &mut egui::Ui,
    id: egui::Id,
    width: f32,
    options: &[String],
    selected: usize,
    mono: bool,
) -> Option<usize> {
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    let current = options.get(selected).map_or("", String::as_str).to_owned();
    let font = if mono {
        FontId::monospace(TEXT_BODY)
    } else {
        FontId::proportional(TEXT_BODY)
    };
    {
        let visuals = ui.visuals_mut();
        visuals.widgets.inactive.weak_bg_fill = scheme.bg_pane;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, scheme.border_default);
        visuals.widgets.hovered.weak_bg_fill = scheme.bg_pane;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, scheme.border_strong);
        visuals.widgets.open.weak_bg_fill = scheme.bg_pane;
        visuals.widgets.inactive.corner_radius = radius;
        visuals.widgets.hovered.corner_radius = radius;
        visuals.widgets.open.corner_radius = radius;
    }
    ui.spacing_mut().interact_size.y = FIELD_HEIGHT;
    ui.spacing_mut().button_padding = vec2(10.0, 8.0);
    let mut picked = selected;
    egui::ComboBox::from_id_salt(id)
        .selected_text(
            egui::RichText::new(current)
                .font(font)
                .color(scheme.text_primary),
        )
        .width(width)
        .show_ui(ui, |ui| {
            for (i, option) in options.iter().enumerate() {
                ui.selectable_value(&mut picked, i, option);
            }
        });
    (picked != selected).then_some(picked)
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
    segmented_with(ui, rect, id, options, selected, None)
}

/// [`segmented`] with one option that cannot be chosen: `unavailable` is
/// its index and why, shown as its tooltip. The option is drawn washed
/// out, takes no click and is read aloud as disabled.
pub fn segmented_with(
    ui: &mut egui::Ui,
    rect: Rect,
    id: egui::Id,
    options: &[String],
    selected: usize,
    unavailable: Option<(usize, &str)>,
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
        let why = unavailable
            .filter(|(at, _)| *at == index)
            .map(|(_, why)| why);
        let sense = if why.is_some() {
            Sense::hover()
        } else {
            Sense::click()
        };
        let response = ui.interact(tile, id.with(index), sense);
        if index == selected {
            ui.painter().rect_filled(tile, radius, scheme.bg_pane);
        } else if response.hovered() && why.is_none() {
            ui.painter().rect_filled(tile, radius, scheme.bg_subtle);
        }
        let colour = if index == selected {
            scheme.text_primary
        } else if why.is_some() {
            scheme.text_secondary.gamma_multiply(0.55)
        } else {
            scheme.text_secondary
        };
        ui.painter().text(
            tile.center(),
            Align2::CENTER_CENTER,
            option,
            FontId::proportional(TEXT_BODY),
            colour,
        );
        let spoken = option.clone();
        let enabled = why.is_none();
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::RadioButton,
                enabled,
                index == selected,
                spoken.clone(),
            )
        });
        if response.clicked() {
            clicked = Some(index);
        }
        if let Some(why) = why {
            response.on_hover_text(why);
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
/// `bg/control`. Disabled, the primary is washed out and the secondary's
/// text goes `text/secondary`.
pub fn button(ui: &mut egui::Ui, label: &str, primary: bool, enabled: bool) -> egui::Response {
    icon_button(ui, None, label, primary, enabled)
}

/// [`button`] with a 16 px glyph before its label.
pub fn icon_button(
    ui: &mut egui::Ui,
    glyph: Option<&str>,
    label: &str,
    primary: bool,
    enabled: bool,
) -> egui::Response {
    let kind = if primary {
        ButtonKind::Primary
    } else {
        ButtonKind::Secondary
    };
    button_of(ui, glyph, label, kind, enabled)
}

/// What a dialogue button is for, which decides its colours (SPEC.md
/// *Controls*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// The dialogue's main action: `accent/button` with white text.
    Primary,
    /// Any other action: `bg/control` with `text/primary`.
    Secondary,
    /// A choice that overwrites something: `trash/mid-bg` with
    /// `trash/mid-text`, amber and not the red of a destructive entry.
    Caution,
}

/// A caution button: [`button`] in the amber of [`ButtonKind::Caution`].
pub fn caution_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    button_of(ui, None, label, ButtonKind::Caution, enabled)
}

/// [`icon_button`] of the given kind.
pub fn button_of(
    ui: &mut egui::Ui,
    glyph: Option<&str>,
    label: &str,
    kind: ButtonKind,
    enabled: bool,
) -> egui::Response {
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    let (fill, fg) = match (kind, enabled) {
        (ButtonKind::Primary, true) => (scheme.accent_button, Scheme::ON_BUTTON),
        (ButtonKind::Primary, false) => {
            (scheme.accent_button.gamma_multiply(0.45), Scheme::ON_BUTTON)
        }
        (ButtonKind::Secondary, true) => (scheme.bg_control, scheme.text_primary),
        (ButtonKind::Secondary, false) => (scheme.bg_control, scheme.text_secondary),
        (ButtonKind::Caution, true) => (scheme.trash_mid_bg, scheme.trash_mid_text),
        (ButtonKind::Caution, false) => (
            scheme.trash_mid_bg.gamma_multiply(0.5),
            scheme.trash_mid_text,
        ),
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(TEXT_BODY), fg);
    let glyph_room = if glyph.is_some() {
        theme::ICON + 8.0
    } else {
        0.0
    };
    let size = vec2(galley.size().x + glyph_room + 32.0, 36.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.hovered() && enabled {
        fill.gamma_multiply(0.9)
    } else {
        fill
    };
    ui.painter().rect_filled(rect, radius, fill);
    let text_left = rect.left() + 16.0 + glyph_room;
    if let Some(glyph) = glyph {
        ui.painter().text(
            pos2(rect.left() + 16.0 + theme::ICON / 2.0, rect.center().y),
            Align2::CENTER_CENTER,
            glyph,
            FontId::proportional(theme::ICON),
            fg,
        );
    }
    ui.painter().galley(
        pos2(text_left, rect.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label.to_owned())
    });
    response
}

/// One line of text laid out for `width`: cut with an ellipsis at its end
/// when it is longer, never wrapped. The galley's `elided` says whether it
/// was cut.
#[must_use]
pub fn elided(
    ui: &egui::Ui,
    text: &str,
    font: FontId,
    colour: egui::Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, colour);
    job.wrap = egui::text::TextWrapping {
        max_width: width.max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

/// Paints one line of text at `pos` (its left centre), cut with an
/// ellipsis at `width`. The line is a label for a screen reader, with the
/// whole text as its name, and shows the whole text as its tooltip when
/// it was cut. Returns the rect it took.
pub fn line(
    ui: &mut egui::Ui,
    pos: egui::Pos2,
    text: &str,
    font: FontId,
    colour: egui::Color32,
    width: f32,
) -> Rect {
    let galley = elided(ui, text, font, colour, width);
    let size = galley.size();
    let rect = Rect::from_min_size(pos2(pos.x, pos.y - size.y / 2.0), size);
    let cut = galley.elided;
    ui.painter().galley(rect.min, galley, colour);
    // The same text may stand twice in one dialogue: the place tells them
    // apart.
    let place = (rect.left().to_bits(), rect.top().to_bits());
    let response = ui.interact(rect, ui.id().with(("line", text, place)), Sense::hover());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text.to_owned()));
    if cut {
        response.on_hover_text(text);
    }
    rect
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_line_ends_in_an_ellipsis_at_its_width_and_a_short_one_is_whole() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let font = FontId::proportional(TEXT_BODY);
            let name =
                "Camera white appartment 5-14-2026 1.32.41pm GMT2 - 5-14-2026 1.33.08pm GMT2.mp4";
            let cut = elided(ui, name, font.clone(), egui::Color32::WHITE, 398.0);
            assert!(cut.elided, "the name is longer than its card");
            assert!(cut.size().x <= 398.0, "{} px", cut.size().x);
            assert_eq!(cut.rows.len(), 1, "one line, never wrapped");
            let last = cut.rows[0].glyphs.last().map(|g| g.chr);
            assert_eq!(last, Some('…'), "it ends in an ellipsis");

            let whole = elided(ui, "notes.md", font, egui::Color32::WHITE, 398.0);
            assert!(!whole.elided);
            assert_eq!(whole.text(), "notes.md");
        });
        // No renderer takes the font atlas in a unit test.
        output.textures_delta.clear();
    }
}
