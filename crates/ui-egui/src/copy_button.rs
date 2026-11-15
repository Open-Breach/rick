//! The copy button (SPEC.md *Copy buttons* under Look and feel): every
//! button that copies a value is this one widget, so each answers a click
//! in the same way. The glyph changes from `COPY` to `CHECK` in
//! `status/success`, the tooltip and the accessible name read *Copied*
//! while the tick shows, and after [`SHOWN`] seconds the button is as it
//! was. The box keeps its size and nothing moves; the glyphs fade into
//! each other, and swap without a fade under the system's reduce-motion
//! setting.
//!
//! Three forms: [`CopyButton::show_at`] in a rectangle a row has placed,
//! [`CopyButton::show`] in a layout, and [`labelled`], a dialogue button
//! that keeps its label and swaps the glyph. The caller copies on
//! `clicked()`; the widget only answers.

use std::fmt::Debug;
use std::hash::Hash;

use egui::{Align2, Color32, FontId, Rect, Sense};
use ext::t;

use crate::theme::{self, ICON};
use crate::{icons, widgets};

/// How long the tick shows after a click, in seconds.
pub const SHOWN: f64 = 1.5;
/// How long the two glyphs take to fade into each other, at either end.
const FADE: f64 = 0.12;
/// How strong a faint button's glyph is while the pointer is elsewhere.
const FAINT: f32 = 0.45;

/// When a button copied last, by the context's clock.
#[derive(Clone, Copy)]
struct CopiedAt(f64);

/// Whether motion is reduced, for every copy button of the context.
#[derive(Clone, Copy, Default)]
struct Still(bool);

fn still_id() -> egui::Id {
    egui::Id::new("copy-button-still")
}

/// Tells every copy button of the context whether the system asks for
/// reduced motion: the glyphs then swap without a fade. The shell calls
/// it with the platform's answer.
pub fn set_still(ctx: &egui::Context, still: bool) {
    ctx.data_mut(|d| d.insert_temp(still_id(), Still(still)));
}

/// How much of the tick shows `since` seconds after the click, from 0,
/// the `COPY` glyph alone, to 1, the `CHECK` alone; `None` before the
/// click and once the button is as it was. `still` is reduced motion: the
/// tick is whole for as long as it shows.
#[must_use]
pub fn tick_at(since: f64, still: bool) -> Option<f32> {
    if !(0.0..SHOWN).contains(&since) {
        return None;
    }
    if still {
        return Some(1.0);
    }
    let mix = (since / FADE).min((SHOWN - since) / FADE).clamp(0.0, 1.0);
    // Between 0 and 1.
    #[allow(clippy::cast_possible_truncation)]
    Some(mix as f32)
}

/// Records a click on the button `id` and says how much of the tick it
/// shows now ([`tick_at`]). Asks for the frames the fade and the way back
/// need, so an idle window still returns to `COPY`.
fn answer(ctx: &egui::Context, id: egui::Id, clicked: bool) -> Option<f32> {
    let now = ctx.input(|i| i.time);
    if clicked {
        ctx.data_mut(|d| d.insert_temp(id, CopiedAt(now)));
    }
    let CopiedAt(at) = ctx.data(|d| d.get_temp::<CopiedAt>(id))?;
    let since = now - at;
    let still = ctx
        .data(|d| d.get_temp::<Still>(still_id()))
        .unwrap_or_default()
        .0;
    let Some(mix) = tick_at(since, still) else {
        ctx.data_mut(|d| d.remove::<CopiedAt>(id));
        return None;
    };
    if mix < 1.0 {
        ctx.request_repaint();
    } else {
        let whole_until = if still { SHOWN } else { SHOWN - FADE };
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(
            (whole_until - since).max(0.0),
        ));
    }
    Some(mix)
}

/// Paints the button's glyph centred on `centre`: `COPY` in `colour`,
/// `CHECK` in `status/success`, each as strong as `mix` says.
fn glyphs(ui: &egui::Ui, centre: egui::Pos2, size: f32, colour: Color32, mix: f32) {
    let font = FontId::proportional(size);
    if mix < 1.0 {
        ui.painter().text(
            centre,
            Align2::CENTER_CENTER,
            icons::COPY,
            font.clone(),
            colour.gamma_multiply(1.0 - mix),
        );
    }
    if mix > 0.0 {
        ui.painter().text(
            centre,
            Align2::CENTER_CENTER,
            icons::DONE,
            font,
            theme::scheme(ui).status_success.gamma_multiply(mix),
        );
    }
}

/// The tooltip of a button that shows its tick: *Copied*, for as long as
/// the pointer is on it. egui hides a tooltip at a click, and this one is
/// the answer to the click.
fn copied_tooltip(response: &egui::Response) {
    if response.contains_pointer() {
        response.show_tooltip_text(t!("copy-button-copied"));
    }
}

/// What is behind the glyph while the pointer is on the button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hover {
    /// Nothing: the glyph alone.
    Bare,
    /// A tile with `radius/control` corners.
    Tile,
    /// A disc.
    Round,
}

/// A copy button of one glyph. `name` is its tooltip and its accessible
/// name, *Copy location*; both read *Copied* while the tick shows.
#[derive(Debug, Clone, Copy)]
#[must_use = "a copy button is drawn by `show` or `show_at`"]
pub struct CopyButton<'a> {
    name: &'a str,
    glyph: f32,
    hover: Hover,
    colour: Option<Color32>,
    faint: bool,
    lit: bool,
}

impl<'a> CopyButton<'a> {
    /// A button with a 16 px glyph and a `bg/subtle` tile under the
    /// pointer.
    pub fn new(name: &'a str) -> Self {
        Self {
            name,
            glyph: ICON,
            hover: Hover::Tile,
            colour: None,
            faint: false,
            lit: false,
        }
    }

    /// The glyph's size in points.
    pub fn glyph(mut self, size: f32) -> Self {
        self.glyph = size;
        self
    }

    /// What is behind the glyph under the pointer.
    pub fn hover(mut self, hover: Hover) -> Self {
        self.hover = hover;
        self
    }

    /// The colour of the `COPY` glyph, where the button stands among
    /// glyphs that are not in `icon/default`.
    pub fn colour(mut self, colour: Color32) -> Self {
        self.colour = Some(colour);
        self
    }

    /// A button at the end of a row: faint at rest, and full while the
    /// pointer is on it or, `lit`, on its row.
    pub fn faint(mut self, lit: bool) -> Self {
        self.faint = true;
        self.lit = lit;
        self
    }

    /// The button in `rect`, which a row has placed. `salt` tells it from
    /// the other buttons of the same `ui`.
    pub fn show_at(self, ui: &mut egui::Ui, rect: Rect, salt: impl Hash + Debug) -> egui::Response {
        let id = ui.id().with(("copy-button", salt));
        let response = ui.interact(rect, id, Sense::click());
        self.finish(ui, rect, id, response)
    }

    /// The button in the layout, as large as egui's frameless button of
    /// its glyph — the glyph itself, as tall as a control at least — and
    /// at least `min`.
    pub fn show(self, ui: &mut egui::Ui, min: egui::Vec2) -> egui::Response {
        let id = ui.id().with(("copy-button", self.name));
        let glyph = ui.painter().layout_no_wrap(
            icons::COPY.to_owned(),
            FontId::proportional(self.glyph),
            Color32::PLACEHOLDER,
        );
        let mut size = glyph.size();
        size.y = size.y.max(ui.spacing().interact_size.y);
        let (rect, response) = ui.allocate_exact_size(size.max(min), Sense::click());
        self.finish(ui, rect, id, response)
    }

    fn finish(
        self,
        ui: &egui::Ui,
        rect: Rect,
        id: egui::Id,
        response: egui::Response,
    ) -> egui::Response {
        let scheme = theme::scheme(ui);
        let mix = answer(ui.ctx(), id, response.clicked());
        if response.hovered() {
            let fill = scheme.bg_subtle;
            match self.hover {
                Hover::Bare => {}
                Hover::Tile => {
                    ui.painter()
                        .rect_filled(rect, theme::metrics(ui.ctx()).corner_radius(), fill);
                }
                Hover::Round => {
                    ui.painter().circle_filled(
                        rect.center(),
                        rect.width().min(rect.height()) / 2.0,
                        fill,
                    );
                }
            }
        }
        let colour = self.colour.unwrap_or(scheme.icon_default);
        let colour = if self.faint && !self.lit && !response.hovered() {
            colour.gamma_multiply(FAINT)
        } else {
            colour
        };
        glyphs(ui, rect.center(), self.glyph, colour, mix.unwrap_or(0.0));
        let spoken = if mix.is_some() {
            t!("copy-button-copied")
        } else {
            self.name.to_owned()
        };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &spoken));
        if mix.is_some() {
            copied_tooltip(&response);
            response
        } else {
            response.on_hover_text(self.name)
        }
    }
}

/// A dialogue's copy button with its label, *Copy details*: a secondary
/// button with the glyph before the label. It keeps the label, and with
/// it its accessible name, and swaps the glyph.
pub fn labelled(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    let id = ui.id().with(("copy-button", label));
    // The click is known once the button is laid out, a frame after the
    // glyph was chosen; `answer` asks for the frame that shows it.
    let mix = answer(ui.ctx(), id, false);
    let response = widgets::button_with(
        ui,
        label,
        widgets::ButtonKind::Secondary,
        enabled,
        |ui, centre, colour| glyphs(ui, centre, ICON, colour, mix.unwrap_or(0.0)),
    );
    if response.clicked() && enabled {
        answer(ui.ctx(), id, true);
    }
    if mix.is_some() {
        copied_tooltip(&response);
    }
    response
}

#[cfg(test)]
mod tests {
    use egui::vec2;

    use super::*;

    #[test]
    fn the_tick_fades_in_shows_and_fades_out() {
        assert_eq!(tick_at(-0.1, false), None, "before the click");
        assert_eq!(tick_at(0.0, false), Some(0.0), "the click itself");
        let rising = tick_at(FADE / 2.0, false).unwrap_or_default();
        assert!((rising - 0.5).abs() < 0.01, "half-way in: {rising}");
        assert_eq!(tick_at(FADE, false), Some(1.0));
        assert_eq!(tick_at(SHOWN / 2.0, false), Some(1.0));
        let falling = tick_at(SHOWN - FADE / 2.0, false).unwrap_or_default();
        assert!((falling - 0.5).abs() < 0.01, "half-way out: {falling}");
        assert_eq!(tick_at(SHOWN, false), None, "as it was");
    }

    #[test]
    fn with_reduced_motion_the_glyph_swaps_without_a_fade() {
        assert_eq!(tick_at(0.0, true), Some(1.0));
        assert_eq!(tick_at(FADE / 2.0, true), Some(1.0));
        assert_eq!(tick_at(SHOWN - FADE / 2.0, true), Some(1.0));
        assert_eq!(tick_at(SHOWN, true), None);
    }

    /// One frame of a context at `time` that draws one copy button and,
    /// when `click`, presses and releases the pointer on it.
    fn frame(
        ctx: &egui::Context,
        time: f64,
        click: bool,
    ) -> (egui::Response, Vec<egui::epaint::ClippedShape>) {
        let at = egui::pos2(20.0, 20.0);
        let mut events = vec![egui::Event::PointerMoved(at)];
        if click {
            for pressed in [true, false] {
                events.push(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                });
            }
        }
        let input = egui::RawInput {
            time: Some(time),
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(200.0, 100.0))),
            events,
            ..Default::default()
        };
        let mut response = None;
        let mut output = ctx.run_ui(input, |ui| {
            let rect = Rect::from_min_size(egui::pos2(8.0, 8.0), vec2(24.0, 24.0));
            response = Some(CopyButton::new("Copy location").show_at(ui, rect, "probe"));
        });
        // Nothing renders the frame, so nothing takes its textures.
        output.textures_delta.clear();
        let response = response.unwrap_or_else(|| panic!("no button drawn"));
        (response, output.shapes)
    }

    /// The glyphs the frame painted in the button, with their colours.
    fn painted(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Color32)> {
        shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.job.text.clone(),
                    text.override_text_color
                        .or_else(|| text.galley.job.sections.first().map(|s| s.format.color))
                        .unwrap_or(text.fallback_color),
                )),
                _ => None,
            })
            .filter(|(text, _)| text == icons::COPY || text == icons::DONE)
            .collect()
    }

    #[test]
    fn a_click_shows_the_tick_in_the_success_colour_and_the_button_comes_back() {
        let ctx = egui::Context::default();
        theme::apply(
            &ctx,
            model::Density::Comfortable,
            (Color32::WHITE, Color32::BLACK),
        );
        set_still(&ctx, true);
        // At rest: the copy glyph alone.
        frame(&ctx, 0.0, false);
        let (_, at_rest) = frame(&ctx, 0.1, false);
        let glyphs = painted(&at_rest);
        assert_eq!(glyphs.len(), 1, "{glyphs:?}");
        assert_eq!(glyphs[0].0, icons::COPY);

        // The click, and the frame after it: the tick alone, in
        // `status/success`.
        let (clicked, _) = frame(&ctx, 0.2, true);
        assert!(clicked.clicked(), "the caller copies on this");
        let (response, shown) = frame(&ctx, 0.3, false);
        let glyphs = painted(&shown);
        assert_eq!(glyphs.len(), 1, "{glyphs:?}");
        assert_eq!(glyphs[0].0, icons::DONE);
        assert_eq!(glyphs[0].1, theme::scheme_of(&ctx).status_success);
        assert_eq!(
            response.rect, clicked.rect,
            "the box keeps its size and place"
        );

        // Still the tick shortly before the time is up; the copy glyph
        // after it.
        let (_, late) = frame(&ctx, 0.2 + SHOWN - 0.05, false);
        assert_eq!(painted(&late)[0].0, icons::DONE);
        let (_, back) = frame(&ctx, 0.2 + SHOWN + 0.05, false);
        let glyphs = painted(&back);
        assert_eq!(glyphs.len(), 1, "{glyphs:?}");
        assert_eq!(glyphs[0].0, icons::COPY);
    }

    #[test]
    fn the_glyphs_fade_into_each_other_unless_motion_is_reduced() {
        let ctx = egui::Context::default();
        theme::apply(
            &ctx,
            model::Density::Comfortable,
            (Color32::WHITE, Color32::BLACK),
        );
        set_still(&ctx, false);
        frame(&ctx, 0.0, false);
        frame(&ctx, 0.1, true);
        // Half-way through the fade both glyphs are there.
        let (_, fading) = frame(&ctx, 0.1 + FADE / 2.0, false);
        let glyphs = painted(&fading);
        let names: Vec<&str> = glyphs.iter().map(|(text, _)| text.as_str()).collect();
        assert_eq!(names, [icons::COPY, icons::DONE]);
        // And then the tick alone.
        let (_, whole) = frame(&ctx, 0.1 + FADE * 2.0, false);
        assert_eq!(painted(&whole).len(), 1);
    }
}
