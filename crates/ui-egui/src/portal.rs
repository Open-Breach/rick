//! The portal family of loading animations (SPEC.md *Loading animations*):
//! an abstract portal in `accent/focus`, drawn with the painter.
//!
//! The **spinner** is three arc rings around a softly pulsing glow,
//! turning at one, two (the other way) and three times the base speed,
//! one loop about a second. The **burst** is the spinner at 96 px with
//! folder and file glyphs spiralling out of it, tumbling and fading as
//! they go. With the system's reduce-motion setting on, both are a still
//! portal: the rings at rest, nothing flying. A caller that animates asks
//! for a repaint itself.

use std::f32::consts::TAU;

use egui::epaint::{PathShape, PathStroke, TextShape};
use egui::{Color32, FontId, Painter, Pos2, Shape, Stroke, pos2, vec2};

use crate::icons;

/// The base size of the spinner, in points.
pub const SPINNER: f32 = 24.0;
/// The size of the burst, in points.
pub const BURST: f32 = 96.0;
/// One loop of the slowest ring, in seconds.
const LOOP: f32 = 1.0;
/// How long a glyph of the burst takes from the portal to vanishing.
const FLIGHT: f32 = 1.6;

/// The spinner at `size`, centred at `center`, at the animation time `time`
/// (seconds); `still` draws the rings at rest.
pub fn spinner(
    painter: &Painter,
    center: Pos2,
    size: f32,
    time: f64,
    colour: Color32,
    still: bool,
) {
    #[allow(clippy::cast_possible_truncation)]
    let t = if still {
        0.0
    } else {
        (time as f32 / LOOP).fract()
    };
    // The glow: a soft disc that breathes.
    let pulse = if still {
        0.4
    } else {
        0.3 + 0.2 * (t * TAU).sin()
    };
    for (step, alpha) in [
        (0.32, 0.35 * pulse),
        (0.22, 0.55 * pulse),
        (0.12, 0.9 * pulse),
    ] {
        painter.circle_filled(center, size * step, colour.gamma_multiply(alpha));
    }
    // Three rings: radius, sweep, speed and direction, stroke.
    let rings = [
        (0.5, 0.68, 1.0, size / 10.0),
        (0.37, 0.56, -2.0, size / 12.0),
        (0.25, 0.44, 3.0, size / 14.0),
    ];
    for (i, (radius, sweep, speed, width)) in rings.into_iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let start = i as f32 * 1.9 + t * TAU * speed;
        ring(
            painter,
            center,
            size * radius,
            start,
            sweep * TAU,
            Stroke::new(width, colour),
        );
    }
}

/// An arc of `sweep` radians from `start`, as a polyline.
fn ring(painter: &Painter, center: Pos2, radius: f32, start: f32, sweep: f32, stroke: Stroke) {
    let steps = 24;
    let points: Vec<Pos2> = (0..=steps)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let a = start + sweep * i as f32 / steps as f32;
            center + vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    painter.add(Shape::Path(PathShape::line(
        points,
        PathStroke::from(stroke),
    )));
}

/// One glyph flying out of the portal: which glyph, its phase in the
/// flight, its direction and how it tumbles.
#[derive(Debug, Clone, Copy)]
struct Flyer {
    glyph: &'static str,
    phase: f32,
    angle: f32,
    tumble: f32,
}

const FLYERS: [Flyer; 5] = [
    Flyer {
        glyph: icons::FOLDER,
        phase: 0.0,
        angle: 0.8,
        tumble: 1.0,
    },
    Flyer {
        glyph: icons::FILE,
        phase: 0.35,
        angle: 3.4,
        tumble: -1.3,
    },
    Flyer {
        glyph: icons::FOLDER,
        phase: 0.55,
        angle: 5.2,
        tumble: 0.7,
    },
    Flyer {
        glyph: icons::FILE,
        phase: 0.75,
        angle: 1.9,
        tumble: -0.9,
    },
    Flyer {
        glyph: icons::FILE,
        phase: 0.15,
        angle: 4.3,
        tumble: 1.6,
    },
];

/// The burst centred at `center`: the spinner at half its size with
/// glyphs spiralling out of it; `still` draws the portal alone, at rest.
pub fn burst(
    painter: &Painter,
    center: Pos2,
    time: f64,
    colour: Color32,
    glyph_colour: Color32,
    still: bool,
) {
    spinner(painter, center, BURST / 2.0, time, colour, still);
    if still {
        return;
    }
    #[allow(clippy::cast_possible_truncation)]
    let t = time as f32;
    let font = FontId::proportional(crate::theme::ICON);
    for flyer in FLYERS {
        // Where in its flight this glyph is, 0 at the portal, 1 gone.
        let progress = (t / FLIGHT + flyer.phase).fract();
        let radius = BURST * (0.12 + 0.4 * progress);
        let angle = flyer.angle + progress * 1.4;
        let at = center + vec2(angle.cos(), angle.sin()) * radius;
        let alpha = (1.0 - progress).powi(2);
        let galley = painter.layout_no_wrap(
            flyer.glyph.to_owned(),
            font.clone(),
            glyph_colour.gamma_multiply(alpha),
        );
        let size = galley.size();
        let shape = TextShape::new(
            pos2(at.x - size.x / 2.0, at.y - size.y / 2.0),
            galley,
            glyph_colour,
        )
        .with_angle(progress * TAU * flyer.tumble);
        painter.add(shape);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painter() -> Painter {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        Painter::new(
            ctx,
            egui::LayerId::background(),
            egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0)),
        )
    }

    /// Everything the painter received this pass.
    fn shapes(painter: &Painter) -> Vec<Shape> {
        painter.ctx().graphics_mut(|g| {
            g.entry(painter.layer_id())
                .all_entries()
                .map(|c| c.shape.clone())
                .collect()
        })
    }

    #[test]
    fn the_spinner_is_three_rings_over_a_glow_and_the_burst_adds_flying_glyphs() {
        let p = painter();
        spinner(&p, pos2(100.0, 100.0), SPINNER, 0.3, Color32::BLUE, false);
        let drawn = shapes(&p);
        let rings = drawn.iter().filter(|s| matches!(s, Shape::Path(_))).count();
        let glow = drawn
            .iter()
            .filter(|s| matches!(s, Shape::Circle(_)))
            .count();
        assert_eq!(rings, 3);
        assert_eq!(glow, 3);

        let p = painter();
        burst(
            &p,
            pos2(200.0, 200.0),
            0.3,
            Color32::BLUE,
            Color32::GRAY,
            false,
        );
        let drawn = shapes(&p);
        let texts = drawn.iter().filter(|s| matches!(s, Shape::Text(_))).count();
        assert_eq!(texts, FLYERS.len(), "every glyph is in flight");
    }

    #[test]
    fn reduce_motion_stills_the_portal_and_grounds_the_glyphs() {
        let a = painter();
        spinner(&a, pos2(100.0, 100.0), SPINNER, 0.1, Color32::BLUE, true);
        let b = painter();
        spinner(&b, pos2(100.0, 100.0), SPINNER, 0.7, Color32::BLUE, true);
        let (a, b) = (shapes(&a), shapes(&b));
        assert_eq!(format!("{a:?}"), format!("{b:?}"), "nothing moves");

        let p = painter();
        burst(
            &p,
            pos2(200.0, 200.0),
            0.3,
            Color32::BLUE,
            Color32::GRAY,
            true,
        );
        assert!(
            shapes(&p).iter().all(|s| !matches!(s, Shape::Text(_))),
            "no glyph flies"
        );
    }

    #[test]
    fn the_rings_turn_at_one_two_and_three_times_the_base_speed() {
        // One loop of the slowest ring brings every ring back to its start,
        // so the shapes at t and t + LOOP are the same.
        let a = painter();
        spinner(&a, pos2(100.0, 100.0), SPINNER, 0.25, Color32::BLUE, false);
        let b = painter();
        spinner(
            &b,
            pos2(100.0, 100.0),
            SPINNER,
            0.25 + f64::from(LOOP),
            Color32::BLUE,
            false,
        );
        let (a, b) = (shapes(&a), shapes(&b));
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
        // And a quarter loop later they have moved.
        let c = painter();
        spinner(&c, pos2(100.0, 100.0), SPINNER, 0.5, Color32::BLUE, false);
        assert_ne!(format!("{a:?}"), format!("{:?}", shapes(&c)));
    }
}
