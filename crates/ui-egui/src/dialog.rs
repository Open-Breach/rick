//! The dialogue frame (SPEC.md *Dialogues*): a `dialog/bg` window with
//! 12 px corners and a soft shadow in three bands — a fixed header with an
//! icon, the title and a secondary line, a body that scrolls when the
//! window is too short, and a fixed footer under a hairline with the
//! buttons right-aligned, the primary last and filled `accent/button`,
//! *Cancel* before it, a note at the left. A form's fields sit in a
//! two-column grid, labels left of their controls; grouped fields sit in
//! cards with hairlines between rows under a caps heading; a disclosure
//! card folds its fields away; the preview draws as a card of effects.
//! The prompt and the confirmation take the same frame.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use egui::{Align, Align2, FontId, Rect, Sense, pos2, vec2};
use ext::t;
use ext::{Field, FieldKind, Form, Header, HeaderIcon, PreviewRow, Value, Values};

use crate::host::Ask;
use crate::theme::{self, CARD_RADIUS, DIALOG_RADIUS, ICON, Scheme, TEXT_BODY, TEXT_SECONDARY};
use crate::{icons, widgets};

/// A form dialogue's width (SPEC.md *Install dialogue*).
pub const FORM_WIDTH: f32 = 560.0;
/// A prompt's or a confirmation's width.
const NARROW_WIDTH: f32 = 400.0;
/// The padding inside the bands.
const PAD: f32 = 24.0;
/// The label column of the field grid.
const LABEL_COLUMN: f32 = 128.0;
/// A control's height.
const FIELD_HEIGHT: f32 = 36.0;
/// The gap between rows.
const ROW_GAP: f32 = 12.0;
/// The header's app icon.
const HEADER_ICON: f32 = 48.0;
/// The title's size.
const TEXT_TITLE: f32 = 15.0;
/// The body scrolls beyond this share of the screen's height.
const BODY_SHARE: f32 = 0.75;

/// What a form's text field, folder or file field wants from the
/// platform: a folder dialogue, or a file dialogue with its filters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Browse {
    /// The folder dialogue.
    Folder,
    /// The file dialogue, limited to these filters.
    Files(Vec<(&'static str, &'static [&'static str])>),
}

/// The dialogue's own state the shell keeps between frames.
pub struct State<'a> {
    /// A form's values as typed so far.
    pub values: &'a mut Values,
    /// A form's preview rows for the values.
    pub rows: &'a mut Vec<PreviewRow>,
    /// A prompt's text.
    pub input: &'a mut String,
    /// Give the first field the keyboard this frame.
    pub focus: &'a mut bool,
}

/// What a frame of the dialogue decided.
#[derive(Debug, Default)]
pub struct Outcome {
    /// `Some(true)` when confirmed, `Some(false)` when cancelled.
    pub decided: Option<bool>,
    /// A field asked for the platform's dialogue.
    pub browse: Option<(&'static str, Browse)>,
    /// The modal was dismissed by a click outside it.
    pub dismissed: bool,
}

/// Draws the dialogue for `ask` and returns what it decided.
#[must_use]
pub fn show(ctx: &egui::Context, id: egui::Id, ask: &Ask, mut state: State<'_>) -> Outcome {
    let scheme = theme::scheme_of(ctx);
    let mut outcome = Outcome::default();
    let width = match ask {
        Ask::Form(_) => FORM_WIDTH,
        _ => NARROW_WIDTH,
    };
    let frame = egui::Frame::new()
        .fill(scheme.dialog_bg)
        .corner_radius(DIALOG_RADIUS)
        .shadow(egui::epaint::Shadow {
            offset: [0, 8],
            blur: 32,
            spread: 0,
            color: scheme.shadow(),
        })
        .inner_margin(0);
    let modal = egui::Modal::new(id).frame(frame).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        match ask {
            Ask::Form(form) => form_ui(ui, form, &mut state, &mut outcome),
            Ask::Prompt { title, .. } => {
                header_ui(ui, title, None);
                body_frame(ui, |ui| {
                    let output = text_field(ui, state.input, "", false, None, *state.focus);
                    *state.focus = false;
                    if output.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        outcome.decided = Some(true);
                    }
                });
                footer_ui(ui, None, &t!("shell-ok"), true, &mut outcome);
            }
            Ask::Confirm { title, text } => {
                header_ui(ui, title, None);
                body_frame(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(text)
                                .size(TEXT_BODY)
                                .color(scheme.text_primary),
                        )
                        .wrap(),
                    );
                });
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    outcome.decided = Some(true);
                }
                footer_ui(ui, None, &t!("shell-ok"), true, &mut outcome);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            outcome.decided = Some(false);
        }
    });
    if modal.should_close() && outcome.decided.is_none() {
        outcome.dismissed = true;
    }
    outcome
}

/// The form: header, the fields in their groups with the preview, footer.
fn form_ui(ui: &mut egui::Ui, form: &Form, state: &mut State<'_>, outcome: &mut Outcome) {
    let title = form
        .header
        .as_ref()
        .and_then(|h| h.title.as_deref())
        .unwrap_or(&form.title);
    header_ui(ui, title, form.header.as_ref());
    let mut changed = false;
    body_frame(ui, |ui| {
        fields_ui(ui, form, state, outcome, &mut changed);
    });
    if changed {
        *state.rows = form.preview_rows(state.values);
    }
    let blocked = (form.preview.is_some() && state.rows.iter().any(|r| r.problem.is_some()))
        || form
            .fields
            .iter()
            .any(|f| f.required.is_some() && state.values.text(f.key).trim().is_empty());
    footer_ui(ui, form.note.as_deref(), &form.ok, !blocked, outcome);
}

/// The header band: the icon, the title in medium weight and, under it,
/// the pill and the secondary line; a hairline below.
fn header_ui(ui: &mut egui::Ui, title: &str, header: Option<&Header>) {
    let scheme = theme::scheme(ui);
    let icon_size = match header.map(|h| &h.icon) {
        Some(HeaderIcon::Glyph(_)) | None => ICON,
        Some(_) => HEADER_ICON,
    };
    let has_line = header.is_some_and(|h| h.pill.is_some() || h.subtitle.is_some());
    let height = if has_line { 88.0 } else { 64.0 };
    let (band, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let icon_rect = Rect::from_center_size(
        pos2(band.left() + PAD + icon_size / 2.0, band.center().y),
        vec2(icon_size, icon_size),
    );
    match header.map(|h| &h.icon) {
        Some(HeaderIcon::Glyph(glyph)) => {
            ui.painter().text(
                icon_rect.center(),
                Align2::CENTER_CENTER,
                glyph,
                FontId::proportional(ICON),
                scheme.icon_default,
            );
        }
        Some(HeaderIcon::Monogram { letter, seed }) => monogram(ui, icon_rect, *letter, seed),
        Some(HeaderIcon::Image { bytes, svg }) if !*svg && png_icon(ui, icon_rect, bytes) => {}
        Some(HeaderIcon::Image { .. }) => {
            monogram(ui, icon_rect, title.chars().next().unwrap_or('?'), title);
        }
        None => {}
    }
    let text_x = icon_rect.right() + 16.0;
    let title_y = if has_line {
        band.top() + 34.0
    } else {
        band.center().y
    };
    ui.painter().text(
        pos2(text_x, title_y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(TEXT_TITLE),
        scheme.text_primary,
    );
    // The title is read aloud as the dialogue's heading.
    let heading = Rect::from_min_max(
        pos2(text_x, title_y - 10.0),
        pos2(band.right() - PAD, title_y + 10.0),
    );
    let response = ui.interact(heading, ui.id().with("dialog-title"), Sense::hover());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title.to_owned()));
    if let Some(header) = header
        && has_line
    {
        let line_y = band.top() + 58.0;
        let mut x = text_x;
        if let Some(pill) = &header.pill {
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(
                        pos2(x, line_y - 10.0),
                        pos2(band.right(), line_y + 10.0),
                    ))
                    .layout(egui::Layout::left_to_right(Align::Center)),
            );
            let rect = widgets::pill(
                &mut child,
                pill,
                scheme.bg_selected_soft,
                scheme.text_on_selected,
            );
            x = rect.right() + 10.0;
        }
        if let Some(subtitle) = &header.subtitle {
            let galley = ui.painter().layout_no_wrap(
                subtitle.clone(),
                FontId::proportional(TEXT_SECONDARY),
                scheme.text_secondary,
            );
            let size = galley.size();
            ui.painter().galley(
                pos2(x, line_y - size.y / 2.0),
                galley,
                scheme.text_secondary,
            );
            let rect = Rect::from_min_size(pos2(x, line_y - size.y / 2.0), size);
            ui.interact(rect, ui.id().with("dialog-subtitle"), Sense::hover())
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, true, subtitle.clone())
                });
        }
    }
    ui.painter()
        .hline(band.x_range(), band.bottom() - 0.5, scheme.hairline());
}

/// The monogram: the letter, white on a colour derived from `seed`.
fn monogram(ui: &egui::Ui, rect: Rect, letter: char, seed: &str) {
    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    #[allow(clippy::cast_precision_loss)]
    let hue = (hasher.finish() % 360) as f32 / 360.0;
    let colour: egui::Color32 = egui::ecolor::Hsva::new(hue, 0.55, 0.72, 1.0).into();
    ui.painter().rect_filled(rect, DIALOG_RADIUS, colour);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        letter.to_uppercase().to_string(),
        FontId::proportional(rect.height() * 0.46),
        Scheme::ON_BUTTON,
    );
}

/// Paints a PNG icon in `rect` with 12 px corners, decoded once and kept
/// as a texture; `false` when the bytes are not a PNG egui can show.
fn png_icon(ui: &egui::Ui, rect: Rect, bytes: &[u8]) -> bool {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    let key = egui::Id::new(("dialog-icon", hasher.finish()));
    let cached: Option<egui::TextureHandle> = ui.ctx().data(|d| d.get_temp(key));
    let texture = if let Some(texture) = cached {
        texture
    } else {
        let Ok(image) = image::load_from_memory(bytes) else {
            return false;
        };
        let image = image.into_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let colour = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let texture = ui.ctx().load_texture(
            format!("dialog-icon-{}", hasher.finish()),
            colour,
            egui::TextureOptions::LINEAR,
        );
        ui.ctx().data_mut(|d| d.insert_temp(key, texture.clone()));
        texture
    };
    // A rounded rectangle filled with the texture: the corners are cut.
    ui.painter().add(
        egui::epaint::RectShape::filled(rect, DIALOG_RADIUS, egui::Color32::WHITE).with_texture(
            texture.id(),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        ),
    );
    true
}

/// The body: a scroll area with the band's padding, capped at a share of
/// the screen's height.
fn body_frame(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    let max_height = ui.ctx().content_rect().height() * BODY_SHARE;
    egui::ScrollArea::vertical()
        .id_salt("dialog-body")
        .max_height(max_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(PAD as i8, 20))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, ROW_GAP);
                    ui.set_width(ui.available_width());
                    content(ui);
                });
        });
}

/// The footer: a hairline, the note at the left, *Cancel* and the primary
/// at the right.
fn footer_ui(
    ui: &mut egui::Ui,
    note: Option<&str>,
    ok: &str,
    enabled: bool,
    outcome: &mut Outcome,
) {
    let scheme = theme::scheme(ui);
    let (band, _) = ui.allocate_exact_size(vec2(ui.available_width(), 68.0), Sense::hover());
    ui.painter()
        .hline(band.x_range(), band.top() + 0.5, scheme.hairline());
    let inner = band.shrink2(vec2(PAD, 16.0));
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    right.spacing_mut().item_spacing.x = 10.0;
    if widgets::button(&mut right, ok, true, enabled).clicked() && enabled {
        outcome.decided = Some(true);
    }
    if widgets::button(&mut right, &t!("cancel"), false, true).clicked() {
        outcome.decided = Some(false);
    }
    if let Some(note) = note {
        let limit = right.min_rect().left() - 16.0;
        let mut left = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    inner.min,
                    pos2(limit.max(inner.left()), inner.bottom()),
                ))
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        left.add(
            egui::Label::new(
                egui::RichText::new(note)
                    .size(TEXT_SECONDARY)
                    .color(scheme.text_secondary),
            )
            .truncate()
            .selectable(false),
        );
    }
}

/// A group of fields: the implicit first one, a titled section, or a
/// disclosure card.
struct Group<'a> {
    head: Option<&'a Field>,
    fields: Vec<&'a Field>,
}

/// The fields in their groups, with the preview card at its marker or
/// after the last group.
#[allow(clippy::too_many_lines)]
fn fields_ui(
    ui: &mut egui::Ui,
    form: &Form,
    state: &mut State<'_>,
    outcome: &mut Outcome,
    changed: &mut bool,
) {
    let mut groups: Vec<Group<'_>> = vec![Group {
        head: None,
        fields: Vec::new(),
    }];
    let mut preview_at: Option<usize> = None;
    for field in &form.fields {
        match &field.kind {
            FieldKind::Section { .. } | FieldKind::Disclosure { .. } => groups.push(Group {
                head: Some(field),
                fields: Vec::new(),
            }),
            FieldKind::Preview => {
                preview_at = Some(groups.len());
                groups.push(Group {
                    head: None,
                    fields: Vec::new(),
                });
            }
            _ => {
                if let Some(group) = groups.last_mut() {
                    group.fields.push(field);
                }
            }
        }
    }
    let scheme = theme::scheme(ui);
    let mut first = true;
    for (index, group) in groups.iter().enumerate() {
        if preview_at == Some(index) && form.preview.is_some() {
            preview_ui(ui, form, state.rows);
        }
        match group.head.map(|f| &f.kind) {
            None => {
                for field in &group.fields {
                    field_row(ui, field, state, outcome, changed, &mut first);
                }
            }
            Some(FieldKind::Section { title, card }) => {
                if let Some(title) = title {
                    widgets::caps_heading(ui, title);
                }
                if *card {
                    card_frame(ui, |ui| {
                        for field in &group.fields {
                            field_row(ui, field, state, outcome, changed, &mut first);
                        }
                    });
                } else {
                    for field in &group.fields {
                        field_row(ui, field, state, outcome, changed, &mut first);
                    }
                }
            }
            Some(FieldKind::Disclosure {
                summary, counted, ..
            }) => {
                let Some(head) = group.head else { continue };
                let open = state.values.check(head.key);
                card_frame(ui, |ui| {
                    let (row, response) = ui.allocate_exact_size(
                        vec2(
                            ui.available_width(),
                            if open || summary.is_empty() {
                                40.0
                            } else {
                                52.0
                            },
                        ),
                        Sense::click(),
                    );
                    ui.painter().text(
                        pos2(row.left() + 14.0 + ICON / 2.0, row.top() + 20.0),
                        Align2::CENTER_CENTER,
                        if open {
                            icons::TREE_OPEN
                        } else {
                            icons::TREE_CLOSED
                        },
                        FontId::proportional(ICON),
                        scheme.icon_default,
                    );
                    let text_x = row.left() + 14.0 + ICON + 12.0;
                    ui.painter().text(
                        pos2(text_x, row.top() + 20.0),
                        Align2::LEFT_CENTER,
                        &head.label,
                        FontId::proportional(TEXT_BODY),
                        scheme.text_primary,
                    );
                    if !open && !summary.is_empty() {
                        // A truncating label: the summary never leaves the
                        // card.
                        let mut line = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(Rect::from_min_max(
                                    pos2(text_x, row.top() + 30.0),
                                    pos2(row.right() - 14.0, row.top() + 46.0),
                                ))
                                .layout(egui::Layout::left_to_right(Align::Center)),
                        );
                        line.add(
                            egui::Label::new(
                                egui::RichText::new(summary)
                                    .size(TEXT_SECONDARY)
                                    .color(scheme.text_secondary),
                            )
                            .truncate()
                            .selectable(false),
                        );
                    }
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Checkbox,
                            true,
                            open,
                            head.label.clone(),
                        )
                    });
                    if *counted {
                        let checks: Vec<&Field> = group
                            .fields
                            .iter()
                            .copied()
                            .filter(|f| matches!(f.kind, FieldKind::Check { .. }))
                            .collect();
                        let on = checks.iter().filter(|f| state.values.check(f.key)).count();
                        let mut side = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(Rect::from_min_max(
                                    pos2(text_x, row.top()),
                                    pos2(row.right() - 14.0, row.top() + 40.0),
                                ))
                                .layout(egui::Layout::right_to_left(Align::Center)),
                        );
                        side.spacing_mut().item_spacing.x = 12.0;
                        let all = side.add(
                            egui::Label::new(
                                egui::RichText::new(t!("menu-select-all"))
                                    .size(TEXT_SECONDARY)
                                    .color(scheme.accent_focus),
                            )
                            .sense(Sense::click())
                            .selectable(false),
                        );
                        if all.clicked() {
                            for field in &checks {
                                state.values.set(field.key, Value::Check(true));
                            }
                            *changed = true;
                        }
                        widgets::pill(
                            &mut side,
                            &t!("footer-of", shown = on, total = checks.len()),
                            scheme.bg_selected_soft,
                            scheme.text_on_selected,
                        );
                    }
                    if response.clicked() {
                        state.values.set(head.key, Value::Check(!open));
                    }
                    if open {
                        for field in &group.fields {
                            field_row(ui, field, state, outcome, changed, &mut first);
                        }
                    }
                });
            }
            Some(_) => {}
        }
    }
    if preview_at.is_none() && form.preview.is_some() {
        preview_ui(ui, form, state.rows);
    }
}

/// A `bg/card` card with `border/default` around rows separated by
/// hairlines.
fn card_frame(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    let scheme = theme::scheme(ui);
    egui::Frame::new()
        .fill(scheme.bg_card)
        .stroke(egui::Stroke::new(1.0, scheme.border_default))
        .corner_radius(CARD_RADIUS)
        .inner_margin(egui::Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(0.0, 8.0);
            content(ui);
        });
}

/// One row of the grid: the label at the left, the control at the right;
/// a required field's warning under it while it is empty.
#[allow(clippy::too_many_lines)]
fn field_row(
    ui: &mut egui::Ui,
    field: &Field,
    state: &mut State<'_>,
    outcome: &mut Outcome,
    changed: &mut bool,
    first: &mut bool,
) {
    let scheme = theme::scheme(ui);
    let focus_this = *first && *state.focus && matches!(field.kind, FieldKind::Text { .. });
    if matches!(field.kind, FieldKind::Text { .. }) {
        *first = false;
    }
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), FIELD_HEIGHT), Sense::hover());
    // The label, wrapped to its column.
    let label_rect = Rect::from_min_max(
        row.min,
        pos2(row.left() + LABEL_COLUMN - 12.0, row.bottom()),
    );
    let mut label_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(label_rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    label_ui.add(
        egui::Label::new(
            egui::RichText::new(&field.label)
                .size(TEXT_BODY)
                .color(scheme.text_secondary),
        )
        .selectable(false),
    );
    let control = Rect::from_min_max(pos2(row.left() + LABEL_COLUMN, row.top()), row.max);
    let mut control_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(control)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    match &field.kind {
        FieldKind::Text { hint, .. } => {
            let mut text = state.values.text(field.key).to_owned();
            let warning = field.required.is_some() && text.trim().is_empty();
            let output = text_field(
                &mut control_ui,
                &mut text,
                hint,
                field.mono,
                warning.then_some(scheme.status_warning_border),
                focus_this,
            );
            if focus_this {
                *state.focus = false;
            }
            if output.changed() {
                state.values.set(field.key, Value::Text(text));
                *changed = true;
            }
            if output.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                outcome.decided = Some(true);
            }
        }
        FieldKind::Folder { .. } | FieldKind::File { .. } => {
            let mut text = state.values.text(field.key).to_owned();
            let button_width = 90.0;
            let mut field_ui = control_ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(
                        control.min,
                        pos2(control.right() - button_width - 8.0, control.bottom()),
                    ))
                    .layout(egui::Layout::left_to_right(Align::Center)),
            );
            let output = text_field(&mut field_ui, &mut text, "", field.mono, None, false);
            if output.changed() {
                state.values.set(field.key, Value::Text(text));
                *changed = true;
            }
            let mut button_ui = control_ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(
                        pos2(control.right() - button_width, control.top()),
                        control.max,
                    ))
                    .layout(egui::Layout::right_to_left(Align::Center)),
            );
            let (label, browse) = match &field.kind {
                FieldKind::File { filters, .. } => {
                    (t!("dialog-choose"), Browse::Files(filters.clone()))
                }
                _ => (t!("shell-browse"), Browse::Folder),
            };
            if widgets::button(&mut button_ui, &label, false, true).clicked() {
                outcome.browse = Some((field.key, browse));
            }
        }
        FieldKind::Check { .. } => {
            let on = state.values.check(field.key);
            let response =
                control_ui.interact(control, control_ui.id().with(field.key), Sense::click());
            let mut switch_ui = control_ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(control)
                    .layout(egui::Layout::right_to_left(Align::Center)),
            );
            widgets::switch(&mut switch_ui, on);
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Checkbox,
                    true,
                    on,
                    field.label.clone(),
                )
            });
            if response.clicked() {
                state.values.set(field.key, Value::Check(!on));
                *changed = true;
            }
        }
        FieldKind::Choice { options, .. } => {
            let mut selected = state.values.choice(field.key);
            let current = options.get(selected).map_or("", String::as_str).to_owned();
            let font = if field.mono {
                FontId::monospace(TEXT_BODY)
            } else {
                FontId::proportional(TEXT_BODY)
            };
            let radius = theme::metrics(control_ui.ctx()).corner_radius();
            {
                let visuals = control_ui.visuals_mut();
                visuals.widgets.inactive.weak_bg_fill = scheme.bg_pane;
                visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, scheme.border_default);
                visuals.widgets.hovered.weak_bg_fill = scheme.bg_pane;
                visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, scheme.border_strong);
                visuals.widgets.open.weak_bg_fill = scheme.bg_pane;
                visuals.widgets.inactive.corner_radius = radius;
                visuals.widgets.hovered.corner_radius = radius;
                visuals.widgets.open.corner_radius = radius;
            }
            control_ui.spacing_mut().interact_size.y = FIELD_HEIGHT;
            control_ui.spacing_mut().button_padding = vec2(10.0, 8.0);
            egui::ComboBox::from_id_salt(("dialog-choice", field.key))
                .selected_text(
                    egui::RichText::new(current)
                        .font(font)
                        .color(scheme.text_primary),
                )
                .width(control.width())
                .show_ui(&mut control_ui, |ui| {
                    for (i, option) in options.iter().enumerate() {
                        ui.selectable_value(&mut selected, i, option);
                    }
                });
            if selected != state.values.choice(field.key) {
                state.values.set(field.key, Value::Choice(selected));
                *changed = true;
            }
        }
        FieldKind::Segmented { options, .. } => {
            let selected = state.values.choice(field.key);
            let id = control_ui.id().with(("dialog-segmented", field.key));
            if let Some(index) = widgets::segmented(&mut control_ui, control, id, options, selected)
            {
                state.values.set(field.key, Value::Choice(index));
                *changed = true;
            }
        }
        FieldKind::Section { .. } | FieldKind::Disclosure { .. } | FieldKind::Preview => {}
    }
    // A required field's warning.
    if let Some(warning) = &field.required
        && state.values.text(field.key).trim().is_empty()
    {
        let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
        let x = line.left() + LABEL_COLUMN;
        ui.painter().text(
            pos2(x + ICON / 2.0, line.center().y),
            Align2::CENTER_CENTER,
            icons::FAILED,
            FontId::proportional(14.0),
            scheme.status_warning,
        );
        let text_rect = Rect::from_min_max(pos2(x + ICON + 6.0, line.top()), line.max);
        let mut warn_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(text_rect)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        warn_ui.add(
            egui::Label::new(
                egui::RichText::new(warning)
                    .size(TEXT_SECONDARY)
                    .color(scheme.status_warning),
            )
            .truncate()
            .selectable(false),
        );
    }
}

/// A text field: a `border/default` frame with `radius/control` corners,
/// `border` in another colour when given, the text in the body or the
/// monospace font, the hint in `text/placeholder`.
fn text_field(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    mono: bool,
    border: Option<egui::Color32>,
    focus: bool,
) -> egui::Response {
    let scheme = theme::scheme(ui);
    let radius = theme::metrics(ui.ctx()).corner_radius();
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), FIELD_HEIGHT), Sense::hover());
    ui.painter().rect(
        rect,
        radius,
        scheme.bg_pane,
        egui::Stroke::new(1.0, border.unwrap_or(scheme.border_default)),
        egui::StrokeKind::Inside,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(10.0, 0.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let font = if mono {
        FontId::monospace(TEXT_BODY)
    } else {
        FontId::proportional(TEXT_BODY)
    };
    let output = egui::TextEdit::singleline(text)
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .font(font.clone())
        .text_color(scheme.text_primary)
        .hint_text(
            egui::RichText::new(hint)
                .font(font)
                .color(scheme.text_placeholder),
        )
        .desired_width(f32::INFINITY)
        .vertical_align(Align::Center)
        .show(&mut inner);
    if focus {
        output.response.request_focus();
        let end = output.galley.text().len();
        let mut state = output.state;
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(end),
            )));
        state.store(ui.ctx(), output.response.id);
    }
    output.response.response
}

/// The icon of a preview row, by what it describes.
fn row_icon(label: &str) -> Option<&'static str> {
    Some(match label {
        "App files" => icons::FOLDER,
        "Launcher" | "Desktop entry" => icons::MACHINE,
        "Command" | "PATH" | "Variable" => icons::TERMINAL,
        "Previous version" | "Mode" => icons::RECENT,
        "Archive" => icons::ARCHIVE,
        "Checksum" => icons::HASH,
        "Icon" => icons::PREVIEW,
        "Main executable" => icons::FILE,
        "Scope" => icons::HOME,
        "Hint" | "Not run" => icons::INFO,
        _ => return None,
    })
}

/// The preview: under its caps title a card with a row per effect — an
/// icon, the label and the detail in the monospace font, a problem in
/// `status/danger` — or, without a title, the from-to table.
#[allow(clippy::too_many_lines)]
fn preview_ui(ui: &mut egui::Ui, form: &Form, rows: &[PreviewRow]) {
    let scheme = theme::scheme(ui);
    if let Some(title) = &form.preview_title {
        widgets::caps_heading(ui, title);
        card_frame(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, row) in rows.iter().enumerate() {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
                if index > 0 {
                    ui.painter()
                        .hline(rect.x_range(), rect.top() + 0.5, scheme.hairline());
                }
                let mut x = rect.left();
                if let Some(icon) = row_icon(&row.from) {
                    ui.painter().text(
                        pos2(x + ICON / 2.0, rect.center().y),
                        Align2::CENTER_CENTER,
                        icon,
                        FontId::proportional(ICON),
                        scheme.icon_default,
                    );
                }
                x += ICON + 12.0;
                ui.painter().text(
                    pos2(x, rect.center().y),
                    Align2::LEFT_CENTER,
                    &row.from,
                    FontId::proportional(TEXT_BODY),
                    scheme.text_primary,
                );
                let detail_x = rect.left() + 150.0;
                let (detail, colour) = match &row.problem {
                    Some(problem) => (format!("{} — {problem}", row.to), scheme.status_danger),
                    None => (row.to.clone(), scheme.text_secondary),
                };
                let detail_rect = Rect::from_min_max(pos2(detail_x, rect.top()), rect.max);
                let mut detail_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(detail_rect)
                        .layout(egui::Layout::left_to_right(Align::Center)),
                );
                detail_ui.add(
                    egui::Label::new(
                        egui::RichText::new(detail)
                            .font(FontId::monospace(TEXT_SECONDARY))
                            .color(colour),
                    )
                    .truncate()
                    .selectable(false),
                );
                // The label is read with its detail.
                ui.interact(rect, ui.id().with(("preview-row", index)), Sense::hover())
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Label,
                            true,
                            format!("{} · {}", row.from, row.to),
                        )
                    });
            }
            if rows.is_empty() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(t!("dialog-nothing-to-preview"))
                            .size(TEXT_SECONDARY)
                            .color(scheme.text_secondary),
                    )
                    .selectable(false),
                );
            }
        });
        return;
    }
    // The from-to table of the rename and compress previews.
    let problems = rows.iter().filter(|r| r.problem.is_some()).count();
    ui.add(
        egui::Label::new(
            egui::RichText::new(match (rows.len(), problems) {
                (0, _) => t!("dialog-nothing-to-preview"),
                (n, 0) => t!("dialog-entries", count = n),
                (n, p) => t!("dialog-entries-with-problems", count = n, problems = p),
            })
            .size(TEXT_SECONDARY)
            .color(scheme.text_secondary),
        )
        .selectable(false),
    );
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    egui::ScrollArea::vertical()
        .id_salt("dialog-preview")
        .max_height(240.0)
        .auto_shrink([false, true])
        .show_rows(ui, row_height, rows.len(), |ui, range| {
            egui::Grid::new(("dialog-preview", range.start))
                .num_columns(3)
                .striped(true)
                .min_col_width(80.0)
                .show(ui, |ui| {
                    for row in &rows[range] {
                        ui.label(&row.from);
                        ui.label(icons::FORWARD);
                        match &row.problem {
                            Some(problem) => {
                                ui.colored_label(
                                    scheme.status_danger,
                                    format!("{} — {problem}", row.to),
                                );
                            }
                            None => {
                                ui.label(&row.to);
                            }
                        }
                        ui.end_row();
                    }
                });
        });
}
