//! The window frame Rick draws where the system draws none.
//!
//! Turning the decorations off, which every platform but macOS does for the
//! title row, takes the system's border, shadow and resize handles with it.
//! This module puts them back, in one of two ways ([`Mode`];
//! specs/gui-title-row.md, *Window frame*). Around a transparent window, the
//! shadow frame: a transparent margin around the content holds a light
//! shadow, a one-point border runs around the content, and grips along the
//! edges and corners resize the window through the backend. Around a window
//! that cannot be transparent, where that margin would be composited as a
//! black band, the plain frame: no margin and no shadow, the border at the
//! window's edge and the grips on it. A maximised or fullscreen window has
//! no frame, and neither has one whose system keeps its own controls.

use egui::viewport::{ResizeDirection, ViewportCommand};
use egui::{CursorIcon, Pos2, Rect, Sense, StrokeKind, Vec2, pos2};
use platform::WindowChrome;

/// The margin around the content of the shadow frame, holding the shadow.
pub const MARGIN: f32 = 10.0;
/// How far inside the border a grip still reaches.
const GRIP_INSIDE: f32 = 4.0;
/// How far from a corner a grip resizes diagonally.
const CORNER: f32 = 24.0;

/// Which of the two frames Rick draws around a window it decorates itself
/// (specs/gui-title-row.md, *Window frame*), the same for every window of
/// the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The frame of a transparent window: a transparent margin around the
    /// content holds the shadow, the border runs around the content, and
    /// the grips cover the margin.
    Shadow,
    /// The frame of an opaque window, where a transparent margin would be
    /// composited as a black band: no margin and no shadow, the border at
    /// the window's edge and the grips on it.
    Plain,
}

impl Mode {
    /// The margin between the window's edge and the border.
    #[must_use]
    pub const fn margin(self) -> f32 {
        match self {
            Self::Shadow => MARGIN,
            Self::Plain => 0.0,
        }
    }
}

/// Whether the frame is drawn now: the system draws no chrome and the
/// window is neither maximised nor fullscreen. The same in both modes.
#[must_use]
pub fn wanted(ctx: &egui::Context, chrome: WindowChrome) -> bool {
    if chrome.native_controls {
        return false;
    }
    ctx.input(|i| {
        let viewport = i.viewport();
        !viewport.maximized.unwrap_or(false) && !viewport.fullscreen.unwrap_or(false)
    })
}

/// A painted frame: the window rect and the bordered rect inside the
/// mode's margin.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    outer: Rect,
    inner: Rect,
}

impl Frame {
    /// The frame of `mode` in the window rect `outer`, not painted: the
    /// border runs inside the mode's margin.
    #[must_use]
    pub fn of(outer: Rect, mode: Mode) -> Self {
        Self {
            outer,
            inner: outer.shrink(mode.margin()),
        }
    }

    /// Paints the frame of `mode` into `outer`: the shadow in the margin
    /// of the shadow frame, then the fill and the border, square-cornered.
    #[must_use]
    pub fn paint(ui: &egui::Ui, outer: Rect, mode: Mode) -> Self {
        let frame = Self::of(outer, mode);
        let inner = frame.inner;
        let visuals = ui.visuals();
        let painter = ui.painter();
        if mode == Mode::Shadow {
            let shadow = egui::epaint::Shadow {
                offset: [0, 2],
                blur: 14,
                spread: 0,
                color: crate::theme::scheme(ui).shadow(),
            };
            painter.add(shadow.as_shape(inner, 0));
        }
        painter.rect(
            inner,
            0,
            visuals.panel_fill,
            visuals.window_stroke(),
            StrokeKind::Inside,
        );
        frame
    }

    /// The rect the content gets: inside the border.
    #[must_use]
    pub fn content(&self) -> Rect {
        self.inner.shrink(1.0)
    }

    /// Registers the resize grips along the edges, after the content so they
    /// win the pointer where they overlap it: each covers the margin and
    /// reaches `GRIP_INSIDE` inside the border, which for the plain frame is
    /// a band of that width on the window's edge. Hovering shows the resize
    /// cursor; a drag starts a resize through the backend. `salt` tells one
    /// window's grips from another's.
    pub fn grips(&self, ui: &mut egui::Ui, salt: u64) {
        let (outer, inner) = (self.outer, self.inner);
        let bands = [
            (
                "west",
                Rect::from_min_max(outer.min, pos2(inner.left() + GRIP_INSIDE, outer.bottom())),
            ),
            (
                "east",
                Rect::from_min_max(pos2(inner.right() - GRIP_INSIDE, outer.top()), outer.max),
            ),
            (
                "north",
                Rect::from_min_max(outer.min, pos2(outer.right(), inner.top() + GRIP_INSIDE)),
            ),
            (
                "south",
                Rect::from_min_max(pos2(outer.left(), inner.bottom() - GRIP_INSIDE), outer.max),
            ),
        ];
        for (name, band) in bands {
            let response = ui.interact(
                band,
                egui::Id::new(("window-grip", salt, name)),
                Sense::drag(),
            );
            let Some(pos) = response
                .interact_pointer_pos()
                .or_else(|| response.hover_pos())
            else {
                continue;
            };
            let Some(direction) = direction(pos, inner) else {
                continue;
            };
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(cursor(direction));
            }
            if response.drag_started() {
                ui.ctx()
                    .send_viewport_cmd(ViewportCommand::BeginResize(direction));
            }
        }
    }
}

/// Which way a grip at `pos` resizes, by its distance to the edges of the
/// bordered rect; `None` well inside it.
#[must_use]
pub fn direction(pos: Pos2, inner: Rect) -> Option<ResizeDirection> {
    let west = pos.x < inner.left() + GRIP_INSIDE;
    let east = pos.x > inner.right() - GRIP_INSIDE;
    let north = pos.y < inner.top() + GRIP_INSIDE;
    let south = pos.y > inner.bottom() - GRIP_INSIDE;
    let near_left = pos.x < inner.left() + CORNER;
    let near_right = pos.x > inner.right() - CORNER;
    let near_top = pos.y < inner.top() + CORNER;
    let near_bottom = pos.y > inner.bottom() - CORNER;
    if (north && near_left) || (west && near_top) {
        Some(ResizeDirection::NorthWest)
    } else if (north && near_right) || (east && near_top) {
        Some(ResizeDirection::NorthEast)
    } else if (south && near_left) || (west && near_bottom) {
        Some(ResizeDirection::SouthWest)
    } else if (south && near_right) || (east && near_bottom) {
        Some(ResizeDirection::SouthEast)
    } else if north {
        Some(ResizeDirection::North)
    } else if south {
        Some(ResizeDirection::South)
    } else if west {
        Some(ResizeDirection::West)
    } else if east {
        Some(ResizeDirection::East)
    } else {
        None
    }
}

fn cursor(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
    }
}

/// The size the content loses to the frame of `mode` on each axis: its
/// margin and the border, on both sides.
#[must_use]
pub fn inset(mode: Mode) -> Vec2 {
    Vec2::splat(2.0 * (mode.margin() + 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INNER: Rect = Rect::from_min_max(pos2(10.0, 10.0), pos2(410.0, 310.0));

    #[test]
    fn edges_resize_straight_and_corners_diagonally() {
        assert_eq!(
            direction(pos2(5.0, 150.0), INNER),
            Some(ResizeDirection::West)
        );
        assert_eq!(
            direction(pos2(415.0, 150.0), INNER),
            Some(ResizeDirection::East)
        );
        assert_eq!(
            direction(pos2(200.0, 5.0), INNER),
            Some(ResizeDirection::North)
        );
        assert_eq!(
            direction(pos2(200.0, 315.0), INNER),
            Some(ResizeDirection::South)
        );
        assert_eq!(
            direction(pos2(5.0, 12.0), INNER),
            Some(ResizeDirection::NorthWest)
        );
        assert_eq!(
            direction(pos2(400.0, 5.0), INNER),
            Some(ResizeDirection::NorthEast)
        );
        assert_eq!(
            direction(pos2(12.0, 315.0), INNER),
            Some(ResizeDirection::SouthWest)
        );
        assert_eq!(
            direction(pos2(415.0, 300.0), INNER),
            Some(ResizeDirection::SouthEast)
        );
    }

    #[test]
    fn the_grip_reaches_a_little_inside_and_no_further() {
        assert_eq!(
            direction(pos2(13.0, 150.0), INNER),
            Some(ResizeDirection::West)
        );
        assert_eq!(direction(pos2(15.0, 150.0), INNER), None);
        assert_eq!(direction(pos2(200.0, 150.0), INNER), None);
    }

    #[test]
    fn no_frame_with_native_controls() {
        let ctx = egui::Context::default();
        let native = WindowChrome {
            controls: platform::ControlsSide::Left,
            native_controls: true,
            native_controls_width: 78.0,
        };
        assert!(!wanted(&ctx, native));
        let own = WindowChrome {
            native_controls: false,
            ..native
        };
        assert!(wanted(&ctx, own));
    }

    /// A 900 by 600 window, as a frame is given it.
    const WINDOW: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(900.0, 600.0));

    /// The chrome of a platform where Rick draws the controls.
    const OWN: WindowChrome = WindowChrome {
        controls: platform::ControlsSide::Right,
        native_controls: false,
        native_controls_width: 0.0,
    };

    #[test]
    fn the_shadow_frame_keeps_its_margin() {
        assert!((Mode::Shadow.margin() - MARGIN).abs() < f32::EPSILON);
        assert_eq!(inset(Mode::Shadow), Vec2::splat(22.0));
        assert_eq!(
            Frame::of(WINDOW, Mode::Shadow).content(),
            Rect::from_min_max(pos2(11.0, 11.0), pos2(889.0, 589.0))
        );
    }

    /// Issue 158: an opaque window has no margin to show as a black band.
    #[test]
    fn the_plain_frame_has_no_margin() {
        assert!(Mode::Plain.margin().abs() < f32::EPSILON);
        assert_eq!(inset(Mode::Plain), Vec2::splat(2.0), "the border only");
    }

    #[test]
    fn the_plain_frames_content_is_the_window_inside_its_border() {
        assert_eq!(
            Frame::of(WINDOW, Mode::Plain).content(),
            Rect::from_min_max(pos2(1.0, 1.0), pos2(899.0, 599.0))
        );
    }

    #[test]
    fn the_plain_frames_grips_are_on_the_windows_edge() {
        let frame = Frame::of(WINDOW, Mode::Plain);
        let at = |x, y| direction(pos2(x, y), frame.inner);
        // Along the edges, reaching 4 points inwards and no further.
        assert_eq!(at(2.0, 300.0), Some(ResizeDirection::West));
        assert_eq!(at(898.0, 300.0), Some(ResizeDirection::East));
        assert_eq!(at(450.0, 2.0), Some(ResizeDirection::North));
        assert_eq!(at(450.0, 598.0), Some(ResizeDirection::South));
        assert_eq!(at(3.5, 300.0), Some(ResizeDirection::West));
        assert_eq!(at(5.0, 300.0), None);
        assert_eq!(at(450.0, 594.0), None);
        // The same corner rule: within 24 points of a corner, diagonally.
        assert_eq!(at(2.0, 2.0), Some(ResizeDirection::NorthWest));
        assert_eq!(at(2.0, 20.0), Some(ResizeDirection::NorthWest));
        assert_eq!(at(880.0, 2.0), Some(ResizeDirection::NorthEast));
        assert_eq!(at(2.0, 590.0), Some(ResizeDirection::SouthWest));
        assert_eq!(at(898.0, 598.0), Some(ResizeDirection::SouthEast));
        assert_eq!(at(2.0, 30.0), Some(ResizeDirection::West));
    }

    /// What `paint` hands the renderer for a 900 by 600 window: the rects
    /// of its shapes, the frame's own after the pass's background.
    fn painted(mode: Mode) -> Vec<egui::epaint::RectShape> {
        let ctx = egui::Context::default();
        let mut rects = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new("frame-test"));
            let ui = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(WINDOW));
            let _ = Frame::paint(&ui, WINDOW, mode);
            ui.ctx().graphics(|graphics| {
                if let Some(list) = graphics.get(layer) {
                    for clipped in list.all_entries() {
                        if let egui::Shape::Rect(rect) = &clipped.shape {
                            rects.push(rect.clone());
                        }
                    }
                }
            });
        });
        // No renderer takes the font atlas in a unit test.
        output.textures_delta.clear();
        rects
    }

    #[test]
    fn the_shadow_frame_paints_its_shadow_and_its_border_inside_the_margin() {
        let rects = painted(Mode::Shadow);
        assert_eq!(rects.len(), 2, "the shadow, then the bordered fill");
        assert!(rects[0].blur_width > 0.0, "the shadow is blurred");
        assert_eq!(rects[1].rect, WINDOW.shrink(MARGIN));
        assert!((rects[1].stroke.width - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_plain_frame_paints_its_border_at_the_windows_edge_and_no_shadow() {
        let rects = painted(Mode::Plain);
        assert_eq!(rects.len(), 1, "the bordered fill only: {rects:?}");
        let border = &rects[0];
        assert_eq!(border.rect, WINDOW, "at the window's edge");
        assert!((border.stroke.width - 1.0).abs() < f32::EPSILON);
        assert_eq!(border.stroke_kind, StrokeKind::Inside);
        assert_eq!(border.corner_radius, egui::CornerRadius::ZERO, "square");
        assert!(border.blur_width.abs() < f32::EPSILON);
        assert_eq!(border.fill.a(), 255, "nothing shows through");
    }

    /// As before, and in both modes: a maximised or fullscreen window has
    /// no frame.
    #[test]
    fn no_frame_when_maximised_or_fullscreen() {
        let seen_with = |maximized: bool, fullscreen: bool| {
            let ctx = egui::Context::default();
            let mut raw = egui::RawInput::default();
            let viewport = raw.viewports.entry(egui::ViewportId::ROOT).or_default();
            viewport.maximized = Some(maximized);
            viewport.fullscreen = Some(fullscreen);
            let mut seen = None;
            let mut output = ctx.run_ui(raw, |ui| seen = Some(wanted(ui.ctx(), OWN)));
            output.textures_delta.clear();
            seen
        };
        assert_eq!(seen_with(false, false), Some(true));
        assert_eq!(seen_with(true, false), Some(false));
        assert_eq!(seen_with(false, true), Some(false));
    }
}
