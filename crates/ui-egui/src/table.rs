//! The table of a preview (SPEC.md *Tables in previews*): the archive
//! tree, the CSV grid and whatever table a preview shows later have one
//! form.
//!
//! A 30 px header band in `bg/panel` over a hairline, the titles in 12 px
//! `text/secondary`; rows `row/height` tall, striped from the second, so
//! the first data row is `bg/pane` and no tinted row sits under the
//! header. Between every pair of header cells a 1 px divider, in the
//! header only, that is the resize handle: `border/strong` and the
//! column-resize cursor under the pointer, a drag resizes the column
//! before it, a double-click fits that column to the cells in view, no
//! column gets narrower than 48 px and the last one takes the rest. Rows
//! are laid out only where they are in view; a table wider than the pane
//! scrolls sideways, its header with it.
//!
//! The caller owns the widths — every column's but the last — and is told
//! when a resize ends, so it can remember them.

use egui::{Align, FontId, Rect, Sense, pos2, vec2};

use crate::theme::{self, TEXT_SECONDARY};
use crate::widgets;

/// The header band's height.
pub const HEADER: f32 = 30.0;
/// No column is narrower than this.
pub const MIN: f32 = crate::views::COLUMN_MIN;
/// A cell's padding at each side.
const PAD: f32 = 12.0;
/// The width of a divider's hit zone.
const HIT: f32 = 7.0;

/// One column: its title and how its cells are aligned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// The title in the header.
    pub title: String,
    /// Cells are laid out from the right edge: numbers.
    pub right: bool,
}

impl Column {
    /// A column of left-aligned cells.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            right: false,
        }
    }

    /// A column of right-aligned cells.
    #[must_use]
    pub fn right(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            right: true,
        }
    }
}

/// What a frame of the table drew and what happened on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    /// The header band.
    pub header: Rect,
    /// The x of every divider, one between each pair of header cells.
    pub dividers: Vec<f32>,
    /// The widths of all columns, the last one's being what was left.
    pub widths: Vec<f32>,
    /// The first data row in view, when there is one.
    pub first_row: Option<Rect>,
    /// A resize ended in this frame: the widths to remember, every
    /// column's but the last.
    pub resized: Option<Vec<f32>>,
}

/// Draws the table into what the `ui` has left. `widths` are the widths
/// of every column but the last, which a drag on a divider changes; `cell`
/// lays out the cell of a row and a column in the ui it is handed;
/// `content` measures a cell's content for a double-click's fit.
pub fn show(
    ui: &mut egui::Ui,
    id: egui::Id,
    columns: &[Column],
    widths: &mut Vec<f32>,
    rows: usize,
    cell: &mut dyn FnMut(&mut egui::Ui, usize, usize),
    content: &dyn Fn(&egui::Ui, usize, usize) -> f32,
) -> Drawn {
    let whole = ui.available_rect_before_wrap();
    let band = Rect::from_min_size(whole.min, vec2(whole.width(), HEADER.min(whole.height())));
    let below = Rect::from_min_max(pos2(whole.left(), band.bottom()), whole.max);
    let count = columns.len();
    widths.resize(count.saturating_sub(1), MIN);
    for width in widths.iter_mut() {
        *width = width.max(MIN);
    }
    let fixed: f32 = widths.iter().sum();
    let last = (whole.width() - fixed).max(MIN);
    let all: Vec<f32> = widths.iter().copied().chain([last]).take(count).collect();

    // The rows first: where they are scrolled to places the header.
    let body = body(ui, id, below, columns, &all, rows, cell);
    let mut drawn = Drawn {
        header: band,
        dividers: Vec::new(),
        widths: all,
        first_row: body.first_row,
        resized: None,
    };
    let fit = |ui: &egui::Ui, col: usize| {
        body.in_view
            .clone()
            .map(|row| content(ui, row, col))
            .fold(0.0, f32::max)
    };
    if header(ui, id, columns, widths, body.offset, &mut drawn, &fit) {
        drawn.resized = Some(widths.clone());
    }
    drawn
}

/// What laying out the rows told.
struct Body {
    /// How far the rows are scrolled sideways.
    offset: f32,
    /// The first row in view.
    first_row: Option<Rect>,
    /// The rows in view.
    in_view: std::ops::Range<usize>,
}

/// The rows in `rect`, those in view only, striped from the second, each
/// cell laid out by `cell` in its own clipped ui.
fn body(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    columns: &[Column],
    widths: &[f32],
    rows: usize,
    cell: &mut dyn FnMut(&mut egui::Ui, usize, usize),
) -> Body {
    let scheme = theme::scheme(ui);
    let row_h = crate::views::row_height(ui);
    let total: f32 = widths.iter().sum();
    let mut body_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    body_ui.set_clip_rect(rect.intersect(ui.clip_rect()));
    let mut first_row = None;
    let mut in_view = 0..0;
    let scrolled = egui::ScrollArea::both()
        .id_salt(id)
        .auto_shrink([false, false])
        .show_viewport(&mut body_ui, |ui, viewport| {
            #[allow(clippy::cast_precision_loss)]
            let height = rows as f32 * row_h;
            let (space, _) = ui.allocate_exact_size(vec2(total, height), Sense::hover());
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let first = (viewport.top().max(0.0) / row_h).floor() as usize;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let shown = (viewport.height().max(0.0) / row_h).ceil() as usize + 1;
            in_view = first.min(rows)..(first + shown).min(rows);
            for row in in_view.clone() {
                #[allow(clippy::cast_precision_loss)]
                let top = space.top() + row as f32 * row_h;
                let line = Rect::from_min_size(
                    pos2(space.left(), top),
                    vec2(total.max(viewport.width()), row_h),
                );
                if row == in_view.start {
                    first_row = Some(line);
                }
                // Striped from the second row: the first is the pane's own.
                if row % 2 == 1 {
                    ui.painter().rect_filled(line, 0.0, scheme.bg_subtle);
                }
                let mut left = space.left();
                for (col, width) in widths.iter().enumerate() {
                    let cell_rect = Rect::from_min_size(
                        pos2(left + PAD, top),
                        vec2((width - 2.0 * PAD).max(0.0), row_h),
                    );
                    let layout = if columns[col].right {
                        egui::Layout::right_to_left(Align::Center)
                    } else {
                        egui::Layout::left_to_right(Align::Center)
                    };
                    let mut cell_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt((row, col))
                            .max_rect(cell_rect)
                            .layout(layout),
                    );
                    cell_ui.set_clip_rect(cell_rect.intersect(ui.clip_rect()));
                    cell(&mut cell_ui, row, col);
                    left += width;
                }
            }
        });
    Body {
        offset: scrolled.state.offset.x,
        first_row,
        in_view,
    }
}

/// The header band of `drawn`, over the rows' left edge as they are
/// scrolled by `offset`: the titles, and after every column but the last
/// its divider, the handle that resizes it. `fit` gives the widest content
/// of a column's cells in view. Returns whether a resize ended.
fn header(
    ui: &mut egui::Ui,
    id: egui::Id,
    columns: &[Column],
    widths: &mut [f32],
    offset: f32,
    drawn: &mut Drawn,
    fit: &dyn Fn(&egui::Ui, usize) -> f32,
) -> bool {
    let scheme = theme::scheme(ui);
    let band = drawn.header;
    let font = FontId::proportional(TEXT_SECONDARY);
    ui.painter().rect_filled(band, 0.0, scheme.bg_panel);
    ui.painter().hline(
        band.x_range(),
        band.bottom() - 0.5,
        egui::Stroke::new(1.0, scheme.border_default),
    );
    let mut head = ui.new_child(egui::UiBuilder::new().max_rect(band));
    head.set_clip_rect(band.intersect(ui.clip_rect()));
    let mut left = band.left() - offset;
    let mut changed = false;
    let all = drawn.widths.clone();
    for (col, width) in all.iter().enumerate() {
        widgets::line(
            &mut head,
            pos2(left + PAD, band.center().y),
            &columns[col].title,
            font.clone(),
            scheme.text_secondary,
            (width - 2.0 * PAD).max(0.0),
        );
        left += width;
        if col + 1 == all.len() {
            break;
        }
        let x = left.round() + 0.5;
        drawn.dividers.push(x);
        let hit = Rect::from_center_size(pos2(x, band.center().y), vec2(HIT, band.height()));
        let response = head.interact(hit, id.with(("divider", col)), Sense::click_and_drag());
        let active = response.hovered() || response.dragged();
        if active {
            head.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
        }
        let colour = if active {
            scheme.border_strong
        } else {
            scheme.border_default
        };
        head.painter()
            .vline(x, band.y_range(), egui::Stroke::new(1.0, colour));
        if response.double_clicked() {
            let title = head
                .painter()
                .layout_no_wrap(
                    columns[col].title.clone(),
                    font.clone(),
                    scheme.text_secondary,
                )
                .size()
                .x;
            widths[col] = (fit(&head, col).max(title) + 2.0 * PAD).ceil().max(MIN);
            changed = true;
        } else if response.dragged() {
            widths[col] = (widths[col] + response.drag_delta().x).max(MIN);
        }
        changed |= response.drag_stopped();
    }
    changed
}
