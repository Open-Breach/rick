//! Drags of a pane by its handle and of a workspace tab (SPEC.md
//! *Workspace tabs*, *Panes*): the numbers every such drag shares, kept in
//! this one place, and the **drag ghost**, what follows the cursor while
//! the drag lasts.
//!
//! The ghost of a pane is a snapshot of the pane itself, never an abstract
//! card: the shapes the pane painted in the frame it was grabbed are kept
//! ([`mark`], [`Snapshot::take`]) and painted again under the cursor,
//! scaled to [`GHOST_SCALE`] at [`GHOST_OPACITY`] ([`Snapshot::placed`],
//! [`Snapshot::paint`]).
//! Keeping the shapes rather than a screenshot of the frame works the same
//! in every window: the backend delivers a screenshot for its root
//! viewport only, and a Rick window beyond the first is a viewport of its
//! own. The ghost of a tab is the tab, drawn where the drag has it and
//! lifted with the same shadow ([`lift`]).
//!
//! Either ghost is anchored to the grab point: the point that was grabbed
//! sits [`GHOST_OFFSET`] down and right of the cursor, so the cursor stays
//! visible, and the ghost moves rigidly with it.
//!
//! ```
//! use ui_egui::drag::{GHOST_OFFSET, GHOST_SCALE, Snapshot};
//!
//! // A pane 400 × 800 at (100, 50), grabbed by its grip at (480, 70).
//! let pane = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(400.0, 800.0));
//! let snapshot = Snapshot::of(Vec::new(), pane, egui::pos2(480.0, 70.0));
//! // The ghost is a quarter of the pane, and its grip — the scaled grab
//! // point — sits 8 px down and right of the cursor, wherever that is.
//! let cursor = egui::pos2(300.0, 300.0);
//! let ghost = snapshot.rect(cursor);
//! assert_eq!(ghost.size(), pane.size() * GHOST_SCALE);
//! let grip = ghost.min + (egui::pos2(480.0, 70.0) - pane.min) * GHOST_SCALE;
//! assert_eq!(grip, cursor + GHOST_OFFSET);
//! ```

use egui::emath::{GuiRounding as _, TSTransform};
use egui::epaint::ClippedShape;
use egui::{LayerId, Pos2, Rect, Shape, Vec2};

use crate::theme;

/// The scale of a dragged pane's ghost: about a quarter of the pane.
pub const GHOST_SCALE: f32 = 0.25;
/// The opacity of a dragged pane's ghost.
pub const GHOST_OPACITY: f32 = 0.7;
/// How far down and right of the cursor the grab point of a ghost sits, so
/// that the cursor stays visible.
pub const GHOST_OFFSET: Vec2 = egui::vec2(8.0, 8.0);

/// Where a pane's shapes begin in the paint list of its layer.
#[derive(Debug, Clone, Copy)]
pub struct Mark {
    layer: LayerId,
    start: usize,
}

/// Marks the place in `ui`'s layer where what is painted next begins; taken
/// before a pane draws itself, so that [`Snapshot::take`] finds the pane's
/// shapes after it.
#[must_use]
pub fn mark(ui: &egui::Ui) -> Mark {
    let layer = ui.layer_id();
    let start = ui
        .ctx()
        .graphics(|layers| layers.get(layer).map_or(0, |list| list.next_idx().0));
    Mark { layer, start }
}

/// What a dragged pane looked like when it was grabbed: its shapes scaled
/// to the ghost's size, in the ghost's own coordinates.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The pane's shapes, scaled, the pane's corner at the origin.
    shapes: Vec<ClippedShape>,
    /// The ghost's size: the pane's, scaled.
    size: Vec2,
    /// Where the grab point lies in the ghost.
    anchor: Vec2,
}

impl Snapshot {
    /// The snapshot of the pane at `pane`, grabbed at `grab`: every shape
    /// painted into the marked layer since `mark`.
    #[must_use]
    pub fn take(ctx: &egui::Context, mark: Mark, pane: Rect, grab: Pos2) -> Self {
        let shapes = ctx.graphics(|layers| {
            layers.get(mark.layer).map_or_else(Vec::new, |list| {
                list.all_entries().skip(mark.start).cloned().collect()
            })
        });
        Self::of(shapes, pane, grab)
    }

    /// The snapshot made of `shapes`, painted at window coordinates by the
    /// pane at `pane`, grabbed at `grab`.
    #[must_use]
    pub fn of(shapes: Vec<ClippedShape>, pane: Rect, grab: Pos2) -> Self {
        // To the ghost's coordinates: the pane's corner to the origin, then
        // everything scaled about it.
        let to_ghost = TSTransform::new(-pane.min.to_vec2() * GHOST_SCALE, GHOST_SCALE);
        let shapes = shapes
            .into_iter()
            // A placeholder paints nothing, and a paint callback draws into
            // the frame itself: neither can be shown again at a quarter.
            .filter(|clipped| !matches!(clipped.shape, Shape::Noop | Shape::Callback(_)))
            .map(|mut clipped| {
                clipped.clip_rect = to_ghost * clipped.clip_rect.intersect(pane);
                clipped.shape.transform(to_ghost);
                clipped
            })
            .filter(|clipped| clipped.clip_rect.is_positive())
            .collect();
        Self {
            shapes,
            size: pane.size() * GHOST_SCALE,
            anchor: (grab - pane.min) * GHOST_SCALE,
        }
    }

    /// Where the ghost is while the cursor is at `cursor`: the scaled grab
    /// point [`GHOST_OFFSET`] down and right of it.
    #[must_use]
    pub fn rect(&self, cursor: Pos2) -> Rect {
        Rect::from_min_size(cursor + GHOST_OFFSET - self.anchor, self.size)
    }

    /// How many shapes the snapshot holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    /// Whether the pane painted nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// Where the ghost is painted for a cursor at `cursor`: [`rect`](Self::rect)
    /// on the pixel grid of `ctx`, so that the border stays one crisp line.
    #[must_use]
    pub fn placed(&self, ctx: &egui::Context, cursor: Pos2) -> Rect {
        let rect = self.rect(cursor);
        Rect::from_min_size(
            rect.min.round_to_pixels(ctx.pixels_per_point()),
            rect.size(),
        )
    }

    /// Paints the ghost at `rect` into `layer`: a soft shadow, the pane's
    /// surface with the snapshot on it, 8 px corners and a 1 px
    /// `border/strong` border, the whole at [`GHOST_OPACITY`].
    pub fn paint(&self, ctx: &egui::Context, layer: LayerId, rect: Rect) {
        let scheme = theme::scheme_of(ctx);
        let mut painter = ctx.layer_painter(layer);
        painter.set_opacity(GHOST_OPACITY);
        let shadow = ctx.global_style().visuals.popup_shadow;
        painter.add(shadow.as_shape(rect, theme::CARD_RADIUS));
        painter.rect_filled(rect, theme::CARD_RADIUS, scheme.bg_pane);
        let offset = rect.min.to_vec2();
        for clipped in &self.shapes {
            let mut shape = clipped.shape.clone();
            shift(&mut shape, offset);
            painter
                .with_clip_rect(clipped.clip_rect.translate(offset).intersect(rect))
                .add(shape);
        }
        painter.rect_stroke(
            rect,
            theme::CARD_RADIUS,
            egui::Stroke::new(theme::HAIRLINE, scheme.border_strong),
            egui::StrokeKind::Inside,
        );
    }
}

/// Moves a shape by `by`. Text moves by its position alone, which spares a
/// copy of its laid-out glyphs in every frame of a drag.
fn shift(shape: &mut Shape, by: Vec2) {
    match shape {
        Shape::Text(text) => text.pos += by,
        Shape::Vec(shapes) => {
            for shape in shapes {
                shift(shape, by);
            }
        }
        other => other.translate(by),
    }
}

/// The shadow under something lifted off the interface by a drag — a
/// dragged workspace tab — the soft shadow the menus have. Painted before
/// the thing itself.
pub fn lift(painter: &egui::Painter, rect: Rect, corners: impl Into<egui::CornerRadius>) {
    let shadow = painter.ctx().global_style().visuals.popup_shadow;
    painter.add(shadow.as_shape(rect, corners));
}

/// What is being dragged, for the ghost's record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dragged {
    /// A pane, by its handle.
    Pane,
    /// A workspace tab.
    Tab,
}

/// The drag ghost as a frame painted it: what tests and screenshots read
/// to tell where it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ghost {
    /// What it is the ghost of.
    pub of: Dragged,
    /// Where it was painted, window coordinates.
    pub rect: Rect,
    /// How many shapes of the pane it holds; none for a tab, which is
    /// drawn afresh.
    pub shapes: usize,
}

/// Tells a screen reader what a drag is doing (SPEC.md *Accessibility*): a
/// status node of `id` in `ui`, a polite live region labelled `text`, so a
/// change of the text is announced without moving the focus. It takes no
/// room and no pointer.
pub fn announce(ui: &egui::Ui, id: egui::Id, text: &str) {
    let response = ui.interact(Rect::NOTHING, id, egui::Sense::hover());
    // `Other`, so that the text is the node's name, what a live region
    // announces; a label's text would go to its value.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, text));
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_role(egui::accesskit::Role::Status);
        node.set_live(egui::accesskit::Live::Polite);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane() -> Rect {
        Rect::from_min_size(egui::pos2(270.0, 40.0), egui::vec2(524.0, 960.0))
    }

    #[test]
    fn the_ghost_is_a_quarter_of_the_pane_anchored_to_the_grab_point() {
        let pane = pane();
        // The grip, at the right end of the toolbar.
        let grab = egui::pos2(pane.right() - 26.0, pane.top() + 23.0);
        let snapshot = Snapshot::of(Vec::new(), pane, grab);
        for cursor in [
            grab,
            egui::pos2(100.0, 700.0),
            egui::pos2(1500.5, 12.25),
            egui::pos2(-40.0, -40.0),
        ] {
            let ghost = snapshot.rect(cursor);
            assert_eq!(ghost.size(), egui::vec2(131.0, 240.0));
            // The scaled grip sits 8 px down and right of the cursor, so
            // the ghost moves rigidly with it.
            let grip = ghost.min + (grab - pane.min) * GHOST_SCALE;
            assert!((grip - (cursor + GHOST_OFFSET)).length() < 1e-3, "{grip:?}");
        }
    }

    #[test]
    fn the_shapes_are_scaled_into_the_ghost_and_clipped_to_the_pane() {
        let pane = pane();
        let row = Rect::from_min_size(pane.min + egui::vec2(6.0, 86.0), egui::vec2(512.0, 32.0));
        let outside = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 30.0));
        let shapes = vec![
            ClippedShape {
                clip_rect: pane,
                shape: Shape::Noop,
            },
            ClippedShape {
                clip_rect: pane,
                shape: Shape::rect_filled(row, 6.0, egui::Color32::RED),
            },
            // Painted by the pane but clipped to somewhere else: not part
            // of what the pane looked like.
            ClippedShape {
                clip_rect: outside,
                shape: Shape::rect_filled(outside, 0.0, egui::Color32::BLUE),
            },
        ];
        let snapshot = Snapshot::of(shapes, pane, pane.center());
        assert_eq!(snapshot.len(), 1, "the placeholder and the stray are gone");
        let kept = &snapshot.shapes[0];
        assert_eq!(
            kept.clip_rect,
            Rect::from_min_size(Pos2::ZERO, pane.size() * GHOST_SCALE)
        );
        let Shape::Rect(scaled) = &kept.shape else {
            panic!("a rectangle stays a rectangle");
        };
        assert_eq!(
            scaled.rect,
            Rect::from_min_size(egui::pos2(1.5, 21.5), egui::vec2(128.0, 8.0))
        );
    }
}
