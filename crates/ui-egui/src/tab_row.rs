//! How the workspace tabs share the title row (SPEC.md *Tabs that outgrow
//! the row*), as in Chrome: a tab is 220 px wide while they all fit; when
//! the row is full they all shrink together, every tab to the same width,
//! down to a minimum; and when they do not fit at the minimum either, the
//! row shows as many as fit at that width — a run of neighbouring tabs in
//! their order, which always holds the active one — and a dropdown button
//! with the list of every tab stands after the last of them, before the
//! new-tab button.
//!
//! [`lay_out`] is the whole rule, a function of the row's width and the
//! number of tabs; the shell draws what it answers.
//!
//! ```
//! use ui_egui::tab_row::{TAB_MIN_WIDTH, lay_out};
//! use ui_egui::theme::TAB_WIDTH;
//!
//! // Three tabs in a wide row: all of them, at their full width.
//! let row = lay_out(1000.0, 3, Some(0), 0);
//! assert_eq!((row.width, row.count, row.overflow), (TAB_WIDTH, 3, false));
//!
//! // Forty in the same row: as many as fit at the minimum, and the list.
//! let row = lay_out(1000.0, 40, Some(0), 0);
//! assert_eq!((row.width, row.first, row.overflow), (TAB_MIN_WIDTH, 0, true));
//! assert!(row.count < 40);
//! ```

use crate::theme::TAB_WIDTH;

/// The narrowest a workspace tab gets: the icon, about four characters of
/// the title before its ellipsis, and the close button.
pub const TAB_MIN_WIDTH: f32 = 100.0;
/// The room between two workspace tabs.
pub const TAB_GAP: f32 = 2.0;
/// The width of the new-tab button, and of the dropdown button that stands
/// before it while tabs are left out of the row.
pub const NEW_TAB_WIDTH: f32 = 28.0;

/// How the workspace tabs of a window share their row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabRow {
    /// The width of every tab in the row.
    pub width: f32,
    /// The index of the first tab the row shows.
    pub first: usize,
    /// How many tabs the row shows, from `first` on.
    pub count: usize,
    /// Whether tabs are left out of the row: the dropdown button with the
    /// list of all of them is then shown.
    pub overflow: bool,
}

impl TabRow {
    /// Whether the row shows the tab at `index`.
    #[must_use]
    pub fn shows(&self, index: usize) -> bool {
        (self.first..self.first + self.count).contains(&index)
    }
}

/// Lays `tabs` tabs out in a row `room` wide, the new-tab button included.
///
/// `active` is the index of the active tab, which is always in the row.
/// `first` is where the run of shown tabs began the last time; it moves
/// just far enough to hold the active tab, and back only as far as it has
/// to when tabs are closed.
#[must_use]
pub fn lay_out(room: f32, tabs: usize, active: Option<usize>, first: usize) -> TabRow {
    let all = |width: f32| TabRow {
        width,
        first: 0,
        count: tabs,
        overflow: false,
    };
    if tabs == 0 {
        return all(TAB_WIDTH);
    }
    #[allow(clippy::cast_precision_loss)]
    let each = (room - NEW_TAB_WIDTH) / tabs as f32 - TAB_GAP;
    if each >= TAB_WIDTH {
        return all(TAB_WIDTH);
    }
    if each >= TAB_MIN_WIDTH {
        // Whole points, so that the titles do not shimmer as the row
        // changes by a fraction.
        return all(each.floor());
    }
    // As many as fit at the minimum beside the two buttons, one at least.
    let fit = ((room - 2.0 * NEW_TAB_WIDTH) / (TAB_MIN_WIDTH + TAB_GAP)).floor();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = (fit.max(1.0) as usize).min(tabs);
    let mut first = first.min(tabs - count);
    if let Some(active) = active.filter(|active| *active < tabs) {
        if active < first {
            first = active;
        } else if active >= first + count {
            first = active + 1 - count;
        }
    }
    TabRow {
        width: TAB_MIN_WIDTH,
        first,
        count,
        overflow: count < tabs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The room `tabs` tabs of `width` take, with the new-tab button.
    #[allow(clippy::cast_precision_loss)]
    fn room_for(tabs: usize, width: f32) -> f32 {
        tabs as f32 * (width + TAB_GAP) + NEW_TAB_WIDTH
    }

    #[test]
    fn tabs_are_220_px_while_they_fit() {
        for tabs in 1..=4 {
            let row = lay_out(room_for(4, TAB_WIDTH), tabs, Some(0), 0);
            assert_eq!(
                row,
                TabRow {
                    width: TAB_WIDTH,
                    first: 0,
                    count: tabs,
                    overflow: false
                }
            );
        }
        // No tab at all is no division by nothing.
        assert_eq!(lay_out(500.0, 0, None, 3).count, 0);
    }

    #[test]
    fn a_full_row_shrinks_every_tab_to_the_same_width_down_to_the_minimum() {
        let room = room_for(4, TAB_WIDTH);
        let mut last = TAB_WIDTH;
        for tabs in 5..=8 {
            let row = lay_out(room, tabs, Some(tabs - 1), 0);
            assert!(
                !row.overflow && row.first == 0 && row.count == tabs,
                "{row:?}"
            );
            assert!(row.width < last && row.width >= TAB_MIN_WIDTH, "{row:?}");
            assert!(room_for(tabs, row.width) <= room, "they fit: {row:?}");
            assert!(
                room_for(tabs, row.width + 1.0) > room,
                "and take the row: {row:?}"
            );
            last = row.width;
        }
        // Exactly at the minimum they are all still in the row.
        let row = lay_out(room_for(9, TAB_MIN_WIDTH), 9, Some(0), 0);
        assert_eq!(
            (row.width, row.count, row.overflow),
            (TAB_MIN_WIDTH, 9, false)
        );
    }

    #[test]
    fn past_the_minimum_the_row_shows_as_many_as_fit_and_the_list() {
        // One point short of nine tabs at the minimum.
        let room = room_for(9, TAB_MIN_WIDTH) - 1.0;
        let row = lay_out(room, 9, Some(0), 0);
        assert_eq!(row.width, TAB_MIN_WIDTH);
        assert!(row.overflow);
        // The dropdown button takes its room beside the new-tab button.
        assert_eq!(row.count, 8);
        assert!(room_for(row.count, TAB_MIN_WIDTH) + NEW_TAB_WIDTH <= room);

        // Forty tabs in a row of 700 px.
        let row = lay_out(700.0, 40, Some(0), 0);
        assert_eq!((row.first, row.count, row.overflow), (0, 6, true));
        // A row with room for less than one tab still shows the active one.
        let row = lay_out(60.0, 40, Some(17), 0);
        assert_eq!((row.first, row.count), (17, 1));
    }

    #[test]
    fn the_run_moves_just_far_enough_to_hold_the_active_tab() {
        let lay = |active: usize, first: usize| lay_out(700.0, 40, Some(active), first);
        // Inside the run: nothing moves.
        assert_eq!(lay(3, 0).first, 0);
        assert_eq!(lay(5, 0).first, 0);
        // Past its end: the active tab becomes the last one shown.
        assert_eq!(lay(6, 0).first, 1);
        assert_eq!(lay(39, 0).first, 34);
        // Before its start: the active tab becomes the first one shown.
        assert_eq!(lay(20, 30).first, 20);
        assert_eq!(lay(0, 34).first, 0);
        // Inside a run that has moved: it stays where it is.
        assert_eq!(lay(12, 10).first, 10);
        for (active, first) in [(0, 0), (6, 0), (39, 0), (20, 30), (12, 10)] {
            assert!(lay(active, first).shows(active), "{active} from {first}");
        }
        // Tabs were closed: the run comes back only as far as it has to.
        let row = lay_out(700.0, 8, Some(7), 34);
        assert_eq!((row.first, row.count), (2, 6));
        // An active tab that is not among them moves nothing.
        assert_eq!(lay_out(700.0, 40, None, 12).first, 12);
        assert_eq!(lay_out(700.0, 40, Some(99), 12).first, 12);
    }
}
