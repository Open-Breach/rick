//! The suggestion bar (specs/plugin-distribution.md §8): one line on
//! `bg/subtle` that says a plugin exists for what the pane shows.
//!
//! `PUZZLE_PIECE`, the text built from the plugin's name and description
//! — *Git plugin — status badges and the Git actions of the context menu*
//! — and then **Install** (or **Enable**, for a plugin that is on the disk
//! and disabled), **Not now** and **Don't suggest again**. The bar stands
//! at the top of the preview pane where the hex view shows a file, and
//! between the toolbar and the list of a pane whose folder holds a
//! plugin's marker. Whether there is anything to suggest is the
//! application's to say ([`app::Suggestion`]); the bar draws it and hands
//! back what was clicked.
//!
//! After **Install** the bar reads *Git is installed. Restart Rick to use
//! it.* with **Restart now** ([`note`]), and after **Enable** the same of
//! a plugin that was switched on. A plugin that is on the disk is no
//! longer suggested, so the place that showed the offer remembers which
//! plugin it was ([`Shown`]) and says so for as long as the pane stays on
//! that file or in that folder.
//!
//! The bar takes no focus and no key: its buttons answer the pointer only,
//! and nothing of it is in the Tab order. It is not drawn in Figma
//! (`specs/open.md`); it is built from the rule above and the rows it
//! stands with — the footer's height and the 26 px control of the time
//! slider's *Back to now*. In a pane too narrow for the plugin's name
//! beside the three buttons, the buttons go under the text, onto as many
//! lines as they need, rather than cut it to nothing.

use app::{App, Pending, Remedy, Suggestion};
use egui::{Align2, FontId, Rect, pos2, vec2};
use ext::t;

use crate::icons;
use crate::shell::Request;
use crate::theme::{self, FOOTER_CONTROL, ICON, Scheme, TEXT_BODY};
use crate::widgets;

/// The room between the bar's parts.
const GAP: f32 = 8.0;
/// What a button has at each side of its label.
const BUTTON_PAD: f32 = 10.0;
/// The least room the text is given on the buttons' line: under it the
/// buttons go onto a line of their own.
const TEXT_MIN: f32 = 160.0;
/// The room between the lines of a bar that wrapped.
const LINE_GAP: f32 = 4.0;

/// What a click on a suggestion asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answer {
    /// **Install** the plugin from the registry.
    Install,
    /// **Enable** the plugin that is on the disk.
    Enable,
    /// **Not now**: not again until Rick is started again.
    NotNow,
    /// **Don't suggest again**.
    Never,
    /// **Restart now**, of a plugin that was installed or switched on.
    Restart,
}

/// Where a suggestion was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// On the bar, or on the *No previewer* card: the place stays and
    /// says what came of it.
    Pane,
    /// In a list of *Add…*, which closes with the choice: what came of
    /// it is said in a notice.
    List,
}

impl Answer {
    /// The request that does what was asked about the plugin `id`, where
    /// it was asked.
    #[must_use]
    pub fn request(self, id: &str, place: Place) -> Request {
        Request::Suggestion {
            id: id.to_owned(),
            answer: self,
            place,
        }
    }
}

/// The plugin a place showed the offer of: what it remembers, to say
/// what came of the offer once the plugin is no longer suggested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// The plugin's id.
    pub id: String,
    /// Its name.
    pub name: String,
}

impl Shown {
    /// What a place remembers of `suggestion`.
    #[must_use]
    pub fn of(suggestion: &Suggestion) -> Self {
        Self {
            id: suggestion.id.clone(),
            name: suggestion.name.clone(),
        }
    }

    /// What the place says now that the plugin is no longer suggested:
    /// *Git is installed. Restart Rick to use it.* while the plugin waits
    /// for the next start, nothing when the offer was declined.
    #[must_use]
    pub fn after(&self, app: &App) -> Option<String> {
        Some(installed_text(&self.name, app.plugin_pending(&self.id)?))
    }
}

/// *Git is installed. Restart Rick to use it.*, or the same of a plugin
/// that was switched on.
#[must_use]
pub fn installed_text(name: &str, pending: Pending) -> String {
    match pending {
        Pending::Installed => t!("suggest-installed", name = name),
        Pending::Enabled => t!("suggest-enabled", name = name),
    }
}

/// What the bar says of `suggestion`: *Git plugin — status badges and the
/// Git actions of the context menu*, from the plugin's name and its
/// description; the name alone where nothing describes the plugin.
#[must_use]
pub fn text(suggestion: &Suggestion) -> String {
    let description = suggestion.description.trim();
    if description.is_empty() {
        t!("suggest-plugin", name = suggestion.name.as_str())
    } else {
        t!(
            "suggest-plugin-does",
            name = suggestion.name.as_str(),
            description = description
        )
    }
}

/// The label of the button that acts on `remedy`, what it asks for, and
/// whether it can be pressed now.
fn remedy_button(remedy: Remedy) -> (String, Answer, bool) {
    match remedy {
        Remedy::Install {
            installing: true, ..
        } => (t!("extensions-installing"), Answer::Install, false),
        Remedy::Install { .. } => (t!("suggest-install"), Answer::Install, true),
        Remedy::Enable => (t!("suggest-enable"), Answer::Enable, true),
    }
}

/// The label and the glyph of the button a card's action row has for
/// `suggestion`, **Install 3D Models** or **Enable 3D Models**, what it
/// asks for, and whether it can be pressed now.
#[must_use]
pub fn card_action(suggestion: &Suggestion) -> (&'static str, String, Answer, bool) {
    let name = suggestion.name.as_str();
    match suggestion.remedy {
        Remedy::Install {
            installing: true, ..
        } => (
            icons::INSTALL,
            t!("extensions-installing"),
            Answer::Install,
            false,
        ),
        Remedy::Install { .. } => (
            icons::INSTALL,
            t!("suggest-install-plugin", name = name),
            Answer::Install,
            true,
        ),
        Remedy::Enable => (
            icons::PLUGIN,
            t!("suggest-enable-plugin", name = name),
            Answer::Enable,
            true,
        ),
    }
}

/// One button of the bar, measured.
struct Button {
    label: String,
    answer: Answer,
    primary: bool,
    enabled: bool,
    width: f32,
}

/// A button's width for its label.
fn button_width(ui: &egui::Ui, label: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::proportional(TEXT_BODY),
        egui::Color32::PLACEHOLDER,
    );
    galley.size().x + 2.0 * BUTTON_PAD
}

/// Draws one button into `rect` and says whether it was clicked. It
/// answers the pointer and is not focusable: the bar takes no key.
fn button(ui: &egui::Ui, id: egui::Id, rect: Rect, button: &Button) -> bool {
    let scheme = theme::scheme(ui);
    let sense = if button.enabled {
        egui::Sense::CLICK
    } else {
        egui::Sense::HOVER
    };
    let response = ui.interact(rect, id.with(button.answer), sense);
    let label = button.label.clone();
    let enabled = button.enabled;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &label));
    let (fill, fg) = match (button.primary, button.enabled) {
        (true, true) => (scheme.accent_button, Scheme::ON_BUTTON),
        (true, false) => (scheme.accent_button.gamma_multiply(0.45), Scheme::ON_BUTTON),
        (false, true) => (scheme.bg_control, scheme.text_primary),
        (false, false) => (scheme.bg_control, scheme.text_secondary),
    };
    let fill = if response.hovered() && button.enabled {
        fill.gamma_multiply(0.9)
    } else {
        fill
    };
    let painter = ui.painter();
    painter.rect_filled(rect, theme::metrics(ui.ctx()).corner_radius(), fill);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        &button.label,
        FontId::proportional(TEXT_BODY),
        fg,
    );
    button.enabled && response.clicked()
}

/// Where the buttons of a bar that wrapped stand: each button's line,
/// counted from the first line under the text, and its offset from the
/// left edge the buttons share, for the room `width`. A button goes onto
/// the next line when it does not fit behind the one before it.
fn flow(widths: &[f32], width: f32) -> Vec<(usize, f32)> {
    let mut places = Vec::with_capacity(widths.len());
    let (mut line, mut x) = (0, 0.0);
    for &each in widths {
        if x > 0.0 && x + each > width {
            line += 1;
            x = 0.0;
        }
        places.push((line, x));
        x += each + GAP;
    }
    places
}

/// Draws the bar for `suggestion` across the width `ui` has left and
/// returns where it stands and what was clicked. `id` tells this bar from
/// another pane's.
pub fn bar(ui: &mut egui::Ui, id: egui::Id, suggestion: &Suggestion) -> (Rect, Option<Answer>) {
    let (label, answer, enabled) = remedy_button(suggestion.remedy);
    let buttons = [
        (label, answer, true, enabled),
        (t!("suggest-not-now"), Answer::NotNow, false, true),
        (t!("suggest-never"), Answer::Never, false, true),
    ];
    draw(ui, id, &text(suggestion), buttons)
}

/// Draws the bar after **Install** or **Enable**: `text`, *Git is
/// installed. Restart Rick to use it.*, with **Restart now**. Returns
/// where it stands and whether the button was clicked.
pub fn note(ui: &mut egui::Ui, id: egui::Id, text: &str) -> (Rect, Option<Answer>) {
    let buttons = [(t!("extensions-restart-now"), Answer::Restart, true, true)];
    draw(ui, id, text, buttons)
}

/// The bar itself: the glyph, `text` and `buttons` — each its label, what
/// it asks for, whether it is the primary one and whether it can be
/// pressed now.
fn draw<const N: usize>(
    ui: &mut egui::Ui,
    id: egui::Id,
    text: &str,
    buttons: [(String, Answer, bool, bool); N],
) -> (Rect, Option<Answer>) {
    let metrics = theme::metrics(ui.ctx());
    let scheme = theme::scheme(ui);
    let buttons: Vec<Button> = buttons
        .into_iter()
        .map(|(label, answer, primary, enabled)| Button {
            width: button_width(ui, &label),
            label,
            answer,
            primary,
            enabled,
        })
        .collect();
    let widths: Vec<f32> = buttons.iter().map(|b| b.width).collect();
    #[allow(clippy::cast_precision_loss)]
    let gaps = GAP * buttons.len().saturating_sub(1) as f32;
    let buttons_width = widths.iter().sum::<f32>() + gaps;

    let width = ui.available_width();
    let inner_width = (width - 2.0 * metrics.space_toolbar_x).max(0.0);
    let font = FontId::proportional(TEXT_BODY);
    let whole = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), scheme.text_primary)
        .size()
        .x;
    // The text's room beside the buttons; with less than the text needs
    // and less than a name's worth, the buttons go under the text.
    let beside = inner_width - ICON - GAP - GAP - buttons_width;
    let wrapped = beside < whole.min(TEXT_MIN);
    let under = (inner_width - ICON - GAP).max(0.0);
    // Each button's line under the first, and its offset on it.
    let places = if wrapped {
        flow(&widths, under)
            .into_iter()
            .map(|(line, x)| (line + 1, x))
            .collect()
    } else {
        let mut x = under - buttons_width;
        widths
            .iter()
            .map(|each| {
                let place = (0, x);
                x += each + GAP;
                place
            })
            .collect::<Vec<_>>()
    };
    let lines = places.iter().map(|(line, _)| line + 1).max().unwrap_or(1);
    let line = FOOTER_CONTROL;
    // One line is the footer's height; every further line adds its own.
    #[allow(clippy::cast_precision_loss)]
    let height = metrics.footer_height() + (lines - 1) as f32 * (line + LINE_GAP);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, scheme.bg_subtle);
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, scheme.hairline());
    let inner = rect.shrink2(vec2(metrics.space_toolbar_x, 0.0));
    let first = inner.top() + metrics.space_toolbar_y + line / 2.0;

    ui.painter().text(
        pos2(inner.left(), first),
        Align2::LEFT_CENTER,
        icons::PLUGIN,
        FontId::proportional(ICON),
        scheme.icon_default,
    );
    let text_left = inner.left() + ICON + GAP;
    let text_width = if wrapped { under } else { beside };
    if text_width > 0.0 {
        widgets::line(
            ui,
            pos2(text_left, first),
            text,
            font,
            scheme.text_primary,
            text_width,
        );
    }

    // The buttons: at the right end of the one line, or under the text
    // from its left edge.
    let mut clicked = None;
    for (each, (row, x)) in buttons.iter().zip(places) {
        #[allow(clippy::cast_precision_loss)]
        let centre = first + row as f32 * (line + LINE_GAP);
        let place = Rect::from_min_size(
            pos2(text_left + x, centre - line / 2.0),
            vec2(each.width, line),
        );
        if button(ui, id, place, each) {
            clicked = Some(each.answer);
        }
    }
    (rect, clicked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_flow_onto_the_next_line_when_they_do_not_fit() {
        let widths = [60.0, 70.0, 140.0];
        // Room for all three.
        assert_eq!(flow(&widths, 300.0), [(0, 0.0), (0, 68.0), (0, 146.0)]);
        // The third goes under the first two.
        assert_eq!(flow(&widths, 250.0), [(0, 0.0), (0, 68.0), (1, 0.0)]);
        // One to a line; a button wider than the room still gets a line.
        assert_eq!(flow(&widths, 100.0), [(0, 0.0), (1, 0.0), (2, 0.0)]);
        assert_eq!(flow(&widths, 10.0), [(0, 0.0), (1, 0.0), (2, 0.0)]);
    }
}
