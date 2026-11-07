//! The window frame Rick draws where the system draws none.
//!
//! Turning the decorations off, which every platform but macOS does for the
//! title row, takes the system's border, shadow and resize handles with it.
//! This module puts them back: a transparent margin around the content holds
//! a light shadow, a one-point border runs around the content, and grips along
//! the edges and corners resize the window through the backend. A maximised
//! or fullscreen window has no frame, and neither has one whose system keeps
//! its own controls.

use egui::viewport::{ResizeDirection, ViewportCommand};
use egui::{CursorIcon, Pos2, Rect, Sense, StrokeKind, Vec2, pos2};
use platform::WindowChrome;

/// The margin around the content, holding the shadow.
pub const MARGIN: f32 = 10.0;
/// How far inside the border a grip still reaches.
const GRIP_INSIDE: f32 = 4.0;
/// How far from a corner a grip resizes diagonally.
const CORNER: f32 = 24.0;

/// Whether the frame is drawn now: the system draws no chrome and the
/// window is neither maximised nor fullscreen.
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

/// A painted frame: the window rect and the bordered rect inside the margin.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    outer: Rect,
    inner: Rect,
}

impl Frame {
    /// Paints the shadow, the fill and the border into `outer`.
    #[must_use]
    pub fn paint(ui: &egui::Ui, outer: Rect) -> Self {
        let inner = outer.shrink(MARGIN);
        let visuals = ui.visuals();
        let shadow = egui::epaint::Shadow {
            offset: [0, 2],
            blur: 14,
            spread: 0,
            color: egui::Color32::from_black_alpha(if visuals.dark_mode { 120 } else { 56 }),
        };
        let painter = ui.painter();
        painter.add(shadow.as_shape(inner, 0));
        painter.rect(
            inner,
            0,
            visuals.panel_fill,
            visuals.window_stroke(),
            StrokeKind::Inside,
        );
        Self { outer, inner }
    }

    /// The rect the content gets: inside the border.
    #[must_use]
    pub fn content(&self) -> Rect {
        self.inner.shrink(1.0)
    }

    /// Registers the resize grips along the edges, after the content so they
    /// win the pointer where they overlap it. Hovering shows the resize
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

/// The size the content loses to the frame on each axis.
#[must_use]
pub fn inset() -> Vec2 {
    Vec2::splat(2.0 * (MARGIN + 1.0))
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
}
