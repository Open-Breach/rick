//! The File info card of the preview pane (SPEC.md *File info card*): one
//! card for a file's facts, whatever previewer shows the file.
//!
//! The **head**, 60 px: a 36 px tile with the file-type icon, the name on
//! one line and, under it, the type and the size; where the pane asks for
//! it, a pin button at its end. The **rows**, 32 px each under a hairline:
//! a label in an 88 px column, the value — cut with an ellipsis, shown
//! whole as its tooltip — and a copy button, faint until the pointer is on
//! the row. Beyond the first rows a line in `accent/focus`, *Show all n
//! fields*, unfolds the rest in place, each further section under its
//! title. The **action row**, for the card centred in the pane: 32 px
//! buttons, the first one primary, wrapping when they do not fit; and
//! under it, where there is something to unfold, *Where Rick looked*.
//!
//! The pane decides where the card stands — hanging under the info
//! button, pinned above the content, or centred — and what its rows are.

use egui::{Align2, FontId, Rect, Sense, pos2, vec2};
use ext::t;

use crate::icons;
use crate::theme::{self, CARD_RADIUS, ICON, TEXT_BODY, TEXT_CAPS, TEXT_SECONDARY};
use crate::widgets::{self, ButtonKind};

/// The head's height.
const HEAD: f32 = 60.0;
/// The icon tile's side.
const TILE: f32 = 36.0;
/// A row's height.
const ROW: f32 = 32.0;
/// The label column.
const LABEL: f32 = 88.0;
/// The card's inset at its sides.
const INSET: f32 = 14.0;
/// The side of a row's button.
const ROW_BUTTON: f32 = 22.0;
/// The side of the pin button.
const PIN_BUTTON: f32 = 24.0;

/// One row of the card.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    /// The title of the section this row begins, for every section after
    /// the first: shown in capitals above the row once the card is
    /// unfolded.
    pub section: Option<String>,
    /// The label.
    pub label: String,
    /// The value, which the copy button copies.
    pub value: String,
    /// A path or a permission string, in monospace.
    pub mono: bool,
    /// A button before the copy button: its glyph and its name.
    pub button: Option<(&'static str, String)>,
}

impl Row {
    /// A row of a label and its value.
    #[must_use]
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            ..Self::default()
        }
    }

    /// The row with its value in monospace.
    #[must_use]
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
}

/// A button of the action row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// The glyph before the label, if any.
    pub glyph: Option<&'static str>,
    /// The label.
    pub label: String,
}

/// What the card shows.
#[derive(Debug, Clone, Default)]
pub struct Card<'a> {
    /// The file-type icon.
    pub glyph: &'a str,
    /// The file's name.
    pub name: &'a str,
    /// The line under the name: the type and the size.
    pub line: &'a str,
    /// In place of rows: a title and, under it, a sentence that wraps —
    /// what the card has to say when it is no list of facts.
    pub note: Option<(&'a str, &'a str)>,
    /// The rows, every section's in order.
    pub rows: &'a [Row],
    /// How many rows show before *Show all n fields*; `None` shows all.
    pub first: Option<usize>,
    /// The head's pin button and whether the card is pinned; `None` for a
    /// card without one.
    pub pin: Option<bool>,
    /// The action row's buttons, the first one primary.
    pub actions: &'a [Action],
    /// What *Where Rick looked* unfolds, when there is something.
    pub looked: Option<&'a str>,
}

/// What stays of a card between frames.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State {
    /// *Show all n fields* was opened.
    pub unfolded: bool,
    /// *Where Rick looked* was opened.
    pub looked: bool,
}

/// What happened on the card in a frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    /// Where the card was drawn.
    pub rect: Option<Rect>,
    /// The pin button was clicked.
    pub pin: bool,
    /// The action row's button of this index was clicked.
    pub action: Option<usize>,
    /// The extra button of the row of this index was clicked.
    pub row_button: Option<usize>,
}

/// Draws the card at the width the `ui` has left. Its rows scroll inside
/// it once they are taller than `max_rows` points.
pub fn show(ui: &mut egui::Ui, card: &Card<'_>, state: &mut State, max_rows: f32) -> Outcome {
    let scheme = theme::scheme(ui);
    let mut outcome = Outcome::default();
    let frame = egui::Frame::new()
        .fill(scheme.bg_card)
        .stroke(egui::Stroke::new(1.0, scheme.border_default))
        .corner_radius(CARD_RADIUS)
        .inner_margin(0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            if head(ui, card) {
                outcome.pin = true;
            }
            if let Some((title, sentence)) = card.note {
                hairline(ui);
                note(ui, title, sentence);
            }
            if !card.rows.is_empty() {
                hairline(ui);
                rows(ui, card, state, max_rows, &mut outcome);
            }
            if !card.actions.is_empty() {
                actions(ui, card, state, &mut outcome);
            }
        });
    outcome.rect = Some(frame.response.rect);
    outcome
}

/// A hairline across the card.
fn hairline(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        theme::scheme(ui).hairline(),
    );
}

/// The head: the tile, the name and the line under it, and the pin button.
/// Returns whether the pin was clicked.
fn head(ui: &mut egui::Ui, card: &Card<'_>) -> bool {
    let scheme = theme::scheme(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEAD), Sense::hover());
    let tile = Rect::from_min_size(
        pos2(rect.left() + INSET, rect.center().y - TILE / 2.0),
        vec2(TILE, TILE),
    );
    ui.painter()
        .rect_filled(tile, CARD_RADIUS, scheme.bg_subtle);
    ui.painter().text(
        tile.center(),
        Align2::CENTER_CENTER,
        card.glyph,
        FontId::proportional(20.0),
        scheme.icon_default,
    );
    let left = tile.right() + 12.0;
    let mut right = rect.right() - INSET;
    let mut clicked = false;
    if let Some(pinned) = card.pin {
        let button = Rect::from_center_size(
            pos2(rect.right() - 10.0 - PIN_BUTTON / 2.0, rect.center().y),
            vec2(PIN_BUTTON, PIN_BUTTON),
        );
        let name = if pinned {
            t!("preview-unpin-info")
        } else {
            t!("preview-pin-info")
        };
        clicked = toggle_button(ui, button, icons::PIN, &name, pinned).clicked();
        right = button.left() - 8.0;
    }
    widgets::line(
        ui,
        pos2(left, rect.center().y - 8.5),
        card.name,
        FontId::proportional(TEXT_BODY),
        scheme.text_primary,
        right - left,
    );
    widgets::line(
        ui,
        pos2(left, rect.center().y + 9.0),
        card.line,
        FontId::proportional(TEXT_SECONDARY),
        scheme.text_secondary,
        right - left,
    );
    clicked
}

/// A button of one glyph that is a toggle: on, `bg/selected-soft` behind
/// an `accent/focus` glyph; off, `bg/subtle` under the pointer. Named
/// `name` for a screen reader and as its tooltip.
pub(crate) fn toggle_button(
    ui: &mut egui::Ui,
    rect: Rect,
    glyph: &str,
    name: &str,
    on: bool,
) -> egui::Response {
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    let response = ui.interact(rect, ui.id().with(("toggle", name)), Sense::click());
    if on {
        ui.painter()
            .rect_filled(rect, radius, scheme.bg_selected_soft);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, radius, scheme.bg_subtle);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(ICON),
        if on {
            scheme.accent_focus
        } else {
            scheme.icon_default
        },
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, name.to_owned())
    });
    response.on_hover_text(name)
}

/// A note in place of rows: the title in 13 px `text/primary` and the
/// sentence in 12 px `text/secondary`, wrapped to the card.
fn note(ui: &mut egui::Ui, title: &str, sentence: &str) {
    let scheme = theme::scheme(ui);
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 14,
            right: 14,
            top: 12,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(0.0, 6.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(title)
                        .size(TEXT_BODY)
                        .color(scheme.text_primary),
                )
                .selectable(false),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(sentence)
                        .size(TEXT_SECONDARY)
                        .color(scheme.text_secondary),
                )
                .selectable(false)
                .wrap(),
            );
        });
}

/// The rows: the first ones, or all of them once unfolded, and the line
/// that unfolds and folds them.
fn rows(
    ui: &mut egui::Ui,
    card: &Card<'_>,
    state: &mut State,
    max_rows: f32,
    outcome: &mut Outcome,
) {
    let all = card.rows.len();
    let first = card.first.unwrap_or(all).min(all);
    let unfolded = state.unfolded || first == all;
    let shown = if unfolded { all } else { first };
    let draw = |ui: &mut egui::Ui, outcome: &mut Outcome| {
        for (index, item) in card.rows.iter().enumerate().take(shown) {
            if let Some(section) = item.section.as_deref().filter(|_| unfolded) {
                section_title(ui, section);
            }
            if row(ui, index, item) {
                outcome.row_button = Some(index);
            }
        }
    };
    if unfolded && first < all {
        egui::ScrollArea::vertical()
            .id_salt("file-info-rows")
            .max_height(max_rows.max(ROW * 2.0))
            .auto_shrink([false, true])
            .show(ui, |ui| draw(ui, outcome));
    } else {
        draw(ui, outcome);
    }
    if first < all {
        let text = t!("preview-show-all-fields", count = all);
        if fold_line(ui, &text, state.unfolded, 26.0) {
            state.unfolded = !state.unfolded;
        }
    }
}

/// A section's title above its first row: 11 px capitals.
fn section_title(ui: &mut egui::Ui, title: &str) {
    let scheme = theme::scheme(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    widgets::line(
        ui,
        pos2(rect.left() + INSET, rect.center().y + 2.0),
        &title.to_uppercase(),
        FontId::proportional(TEXT_CAPS),
        scheme.text_secondary,
        rect.width() - INSET * 2.0,
    );
}

/// A line in `accent/focus` with a caret that unfolds something in place:
/// *Show all n fields*, *Where Rick looked*. Returns whether it was
/// clicked.
fn fold_line(ui: &mut egui::Ui, text: &str, open: bool, height: f32) -> bool {
    let scheme = theme::scheme(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let font = FontId::proportional(TEXT_SECONDARY);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, scheme.accent_focus);
    let line = Rect::from_min_size(
        pos2(rect.left() + INSET, rect.center().y - galley.size().y / 2.0),
        galley.size() + vec2(4.0 + 12.0, 0.0),
    );
    let response = ui.interact(
        line.expand2(vec2(4.0, 4.0)),
        ui.id().with(("fold", text)),
        Sense::click(),
    );
    ui.painter()
        .galley(line.min, galley.clone(), scheme.accent_focus);
    ui.painter().text(
        pos2(line.left() + galley.size().x + 4.0 + 6.0, line.center().y),
        Align2::CENTER_CENTER,
        if open {
            icons::CARET_UP
        } else {
            icons::DROPDOWN
        },
        FontId::proportional(12.0),
        scheme.accent_focus,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, text)
    });
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// One row: the label, the value and, at the right end, the row's own
/// button and the copy button. Returns whether the row's own button was
/// clicked.
fn row(ui: &mut egui::Ui, index: usize, item: &Row) -> bool {
    let scheme = theme::scheme(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    let hovered = ui.rect_contains_pointer(rect);
    widgets::line(
        ui,
        pos2(rect.left() + INSET, rect.center().y),
        &item.label,
        FontId::proportional(TEXT_SECONDARY),
        scheme.text_secondary,
        LABEL,
    );
    let mut right = rect.right() - 6.0;
    let mut place = || {
        let button = Rect::from_center_size(
            pos2(right - ROW_BUTTON / 2.0, rect.center().y),
            vec2(ROW_BUTTON, ROW_BUTTON),
        );
        right = button.left() - 2.0;
        button
    };
    let copy = place();
    let name = t!("preview-copy-field", field = item.label.as_str());
    if row_button(ui, copy, index, icons::COPY, &name, hovered).clicked() {
        ui.ctx().copy_text(item.value.clone());
    }
    let mut clicked = false;
    if let Some((glyph, name)) = &item.button {
        clicked = row_button(ui, place(), index, glyph, name, hovered).clicked();
    }
    let left = rect.left() + INSET + LABEL + 10.0;
    let font = if item.mono {
        FontId::monospace(TEXT_SECONDARY)
    } else {
        FontId::proportional(TEXT_BODY)
    };
    widgets::line(
        ui,
        pos2(left, rect.center().y),
        &item.value,
        font,
        scheme.text_primary,
        right - 6.0 - left,
    );
    clicked
}

/// A 22 px button at the end of a row: its glyph at 14 px, faint at rest
/// and full while the pointer is on the row.
fn row_button(
    ui: &mut egui::Ui,
    rect: Rect,
    index: usize,
    glyph: &str,
    name: &str,
    row_hovered: bool,
) -> egui::Response {
    let scheme = theme::scheme(ui);
    let response = ui.interact(
        rect,
        ui.id().with(("row-button", index, name)),
        Sense::click(),
    );
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            theme::metrics(ui.ctx()).corner_radius(),
            scheme.bg_subtle,
        );
    }
    let colour = if row_hovered || response.hovered() {
        scheme.icon_default
    } else {
        scheme.icon_default.gamma_multiply(0.45)
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(14.0),
        colour,
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.to_owned()));
    response.on_hover_text(name)
}

/// The action row and, under it, *Where Rick looked* with what it unfolds.
fn actions(ui: &mut egui::Ui, card: &Card<'_>, state: &mut State, outcome: &mut Outcome) {
    let scheme = theme::scheme(ui);
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 14,
            right: 14,
            top: 12,
            // *Where Rick looked* sits close under the buttons.
            bottom: if card.looked.is_some() { 6 } else { 12 },
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            ui.horizontal_wrapped(|ui| {
                for (index, action) in card.actions.iter().enumerate() {
                    let kind = if index == 0 {
                        ButtonKind::Primary
                    } else {
                        ButtonKind::Secondary
                    };
                    if widgets::card_button(ui, action.glyph, &action.label, kind).clicked() {
                        outcome.action = Some(index);
                    }
                }
            });
        });
    let Some(looked) = card.looked else {
        return;
    };
    if fold_line(ui, &t!("preview-where-rick-looked"), state.looked, 24.0) {
        state.looked = !state.looked;
    }
    if state.looked {
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 14,
                right: 14,
                top: 2,
                bottom: 12,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("file-info-looked")
                    .max_height(160.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(looked)
                                    .font(FontId::monospace(TEXT_CAPS))
                                    .color(scheme.text_secondary),
                            )
                            .wrap(),
                        );
                    });
            });
    } else {
        ui.add_space(8.0);
    }
}

/// The width of a card centred in the pane.
pub const CENTRED: f32 = 400.0;
/// The width of the size-limit card, whose action row holds a source's
/// name (QUESTIONS.md 362).
pub const CENTRED_WIDE: f32 = 440.0;

/// Draws the card centred in what the `ui` has left: `width` wide, or the
/// width less its margins. The card's height is known once it is laid
/// out, so the last frame's centres it.
pub fn show_centred(ui: &mut egui::Ui, card: &Card<'_>, state: &mut State, width: f32) -> Outcome {
    let whole = ui.available_rect_before_wrap();
    let width = (whole.width() - 24.0).clamp(0.0, width);
    let id = ui.id().with("file-info-centred");
    let last: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(220.0);
    let top = (whole.center().y - last / 2.0).max(whole.top() + 12.0);
    let rect = Rect::from_min_max(
        pos2(whole.center().x - width / 2.0, top),
        pos2(whole.center().x + width / 2.0, whole.bottom().max(top)),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let outcome = show(&mut child, card, state, whole.height() * 0.4);
    if let Some(drawn) = outcome.rect {
        if (drawn.height() - last).abs() > 0.5 {
            ui.ctx().request_repaint();
        }
        ui.data_mut(|d| d.insert_temp(id, drawn.height()));
    }
    outcome
}
