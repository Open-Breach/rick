//! The time slider (`add-zfs-time-slider`): one row under a pane's toolbar
//! that moves the pane through the earlier states of the folder it shows.
//! It draws from the timeline the model holds, never asks a source itself,
//! and emits [`Command::TimeTravel`] — one history step per run.
//!
//! Layout as the mock-up proposes it: as tall as the toolbar with its
//! padding; the clock, the current point (*Now* with the count of earlier
//! states, or the point's date, label and source with a *Read-only* pill),
//! the axis with one dot per point, evenly spaced, and the handle, and *Now* or *Back to
//! now* at the right end. At a past point the row takes `bg/selected-soft`.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use app::Command;
use chrono::{DateTime, Local};
use ext::{Timeline, t};
use model::{PaneId, TimelineState};
use types::Location;

use crate::icons;
use crate::shell::{PaneCx, Request};
use crate::theme::{self, ICON, TEXT_BODY, TEXT_SECONDARY};

/// A point's dot radius.
const DOT: f32 = 3.5;
/// Dots closer than this, in logical pixels, are drawn as one: a dot's
/// width and a pixel of air.
pub const MIN_TICK_GAP: f32 = 2.0 * DOT + 1.0;
/// How long a dragged handle rests on a point before the pane goes there.
pub const REST: Duration = Duration::from_millis(200);
/// The handle's radius.
const HANDLE: f32 = 7.0;
/// The track's thickness.
const TRACK: f32 = 4.0;
/// The gap between the row's parts.
const GAP: f32 = 12.0;
/// The widest the label column gets, as a share of the row.
const LABEL_SHARE: f32 = 0.45;
/// The narrowest label column.
const LABEL_MIN: f32 = 180.0;
/// How near the pointer must be to a tick for its tooltip.
const HOVER: f32 = 5.0;
/// The keys the focused bar takes from the list.
pub const KEYS: [egui::Key; 4] = [
    egui::Key::ArrowLeft,
    egui::Key::ArrowRight,
    egui::Key::Home,
    egui::Key::End,
];

/// The bar's state for one tab, kept between frames.
#[derive(Debug, Clone, Default)]
pub struct BarState {
    /// While dragging: the point the handle is on, `None` for *Now*.
    pub drag: Option<Option<usize>>,
    /// When the dragged handle last moved to another point, in egui time.
    pub moved_at: f64,
    /// The location the bar last sent the pane to: a pane elsewhere starts
    /// a new run, recorded as one history step.
    pub last: Option<Location>,
    /// The bar's row in the last frame, for tests.
    pub rect: Option<egui::Rect>,
    /// The axis in the last frame, for tests.
    pub axis: Option<egui::Rect>,
}

/// One drawn dot: the points `first..=last`, merged into a pill where
/// they crowd.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick {
    /// The leftmost x of the merged points.
    pub from: f32,
    /// The rightmost x.
    pub to: f32,
    /// The first point's index.
    pub first: usize,
    /// The last point's index.
    pub last: usize,
}

/// The x of every point of `timeline` on an axis from `left` to `right`
/// whose right end is *Now*: evenly spaced by index, oldest first, the
/// newest one step left of *Now*, so every point is seen whatever its
/// time. The date is in the tooltip and the label.
#[must_use]
pub fn positions(timeline: &Timeline, left: f32, right: f32) -> Vec<f32> {
    let n = timeline.points.len();
    #[allow(clippy::cast_precision_loss)]
    let step = (right - left) / n.max(1) as f32;
    #[allow(clippy::cast_precision_loss)]
    (0..n).map(|i| left + i as f32 * step).collect()
}

/// The ticks to draw for points at `xs` (sorted): neighbours closer than
/// `gap` are one tick.
#[must_use]
pub fn ticks(xs: &[f32], gap: f32) -> Vec<Tick> {
    let mut out: Vec<Tick> = Vec::new();
    for (i, &x) in xs.iter().enumerate() {
        match out.last_mut() {
            Some(tick) if x - tick.to < gap => {
                tick.to = x;
                tick.last = i;
            }
            _ => out.push(Tick {
                from: x,
                to: x,
                first: i,
                last: i,
            }),
        }
    }
    out
}

/// The point nearest to `x` among `xs`, or `None` when the axis's right
/// end, *Now* at `now_x`, is nearer.
#[must_use]
pub fn nearest(xs: &[f32], x: f32, now_x: f32) -> Option<usize> {
    let best = xs
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (*a - x).abs().total_cmp(&(*b - x).abs()))?;
    ((best.1 - x).abs() <= (now_x - x).abs()).then_some(best.0)
}

/// A key's step from point `at` (`None` is *Now*) among `len` points: Left
/// and Right step by one, Home goes to the oldest, End to *Now*. `None`
/// for a key that does not step.
#[must_use]
pub fn step(at: Option<usize>, len: usize, key: egui::Key) -> Option<Option<usize>> {
    if len == 0 {
        return None;
    }
    let next = match key {
        egui::Key::ArrowLeft => Some(at.map_or(len - 1, |i| i.saturating_sub(1))),
        egui::Key::ArrowRight => at.and_then(|i| (i + 1 < len).then_some(i + 1)),
        egui::Key::Home => Some(0),
        egui::Key::End => None,
        _ => return None,
    };
    Some(next)
}

/// The bar's widget id in a pane, for the keyboard focus.
#[must_use]
pub fn id(pane: PaneId) -> egui::Id {
    egui::Id::new(("timeline", pane))
}

/// Whether the pane's bar has the keyboard focus.
#[must_use]
pub fn focused(ctx: &egui::Context, pane: PaneId) -> bool {
    ctx.memory(egui::Memory::focused) == Some(id(pane))
}

/// A point's time as the bar reads it: *Tue 4 Nov, 02:00*.
fn short(time: SystemTime) -> String {
    let time: DateTime<Local> = time.into();
    time.format("%a %-d %b, %H:%M").to_string()
}

/// A point's time in its tooltip: *Tue 4 Nov 2026, 02:00*.
fn long(time: SystemTime) -> String {
    let time: DateTime<Local> = time.into();
    time.format("%a %-d %b %Y, %H:%M").to_string()
}

/// What the bar shows this frame.
enum Shown {
    Reading,
    Points(Arc<Timeline>, Option<usize>),
}

/// Draws the bar when the pane shows it, as a top panel under the toolbar.
pub fn show(ui: &mut egui::Ui, cx: &mut PaneCx<'_>) {
    let model = cx.app.model();
    if !model.timeline_visible(cx.pane) {
        cx.tab_ui.timeline.rect = None;
        cx.tab_ui.timeline.drag = None;
        return;
    }
    let shown = match model.timeline(cx.pane) {
        TimelineState::Points(timeline) => {
            Shown::Points(Arc::clone(timeline), timeline.point_at(&cx.tab.location))
        }
        _ => Shown::Reading,
    };
    egui::Panel::top(egui::Id::new(("timeline", cx.tab_id)))
        .show_separator_line(false)
        .frame(egui::Frame::NONE)
        .show(ui, |ui| row(ui, cx, &shown));
}

/// The row: background, hairline, label, right end, axis.
fn row(ui: &mut egui::Ui, cx: &mut PaneCx<'_>, shown: &Shown) {
    let metrics = theme::metrics(ui.ctx());
    let scheme = theme::scheme(ui);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), metrics.toolbar_height()),
        egui::Sense::hover(),
    );
    cx.tab_ui.timeline.rect = Some(rect);
    let (timeline, at) = match shown {
        Shown::Points(timeline, at) => (Some(timeline), *at),
        Shown::Reading => (None, None),
    };
    // While dragging, the label follows the handle at once.
    let showing = cx.tab_ui.timeline.drag.unwrap_or(at);
    let past = showing.is_some();
    if past {
        ui.painter().rect_filled(rect, 0.0, scheme.bg_selected_soft);
    }
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, scheme.hairline());
    let inner = rect.shrink2(egui::vec2(metrics.space_toolbar_x, 0.0));
    // Fixed columns for the label and the right end, so the axis stays
    // put whatever they say: a drag into the past must not move the
    // ticks under the pointer.
    let label_width = label_width(ui, timeline.map(Arc::as_ref))
        .clamp(LABEL_MIN, (inner.width() * LABEL_SHARE).max(LABEL_MIN));
    let label_rect = inner.with_max_x(inner.left() + label_width);
    label(ui, label_rect, timeline.map(Arc::as_ref), showing);
    let right = inner.right() - back_button_size(ui).x;
    right_end(ui, cx, inner, timeline.map(Arc::as_ref), past);
    let axis = egui::Rect::from_min_max(
        egui::pos2(label_rect.right() + GAP + HANDLE, rect.top()),
        egui::pos2(right - GAP - HANDLE, rect.bottom()),
    );
    if axis.width() < 2.0 * HANDLE {
        cx.tab_ui.timeline.axis = None;
        return;
    }
    cx.tab_ui.timeline.axis = Some(axis);
    let track = egui::Rect::from_center_size(
        axis.center(),
        egui::vec2(axis.width() + 2.0 * HANDLE, TRACK),
    );
    let track_colour = if past {
        scheme.text_on_selected.gamma_multiply(0.22)
    } else if timeline.is_none() {
        scheme.border_strong.gamma_multiply(0.5)
    } else {
        scheme.bg_track
    };
    ui.painter().rect_filled(track, TRACK / 2.0, track_colour);
    if let Some(timeline) = timeline {
        let xs = positions(timeline, axis.left(), axis.right());
        draw_axis(ui, &xs, showing, axis);
        axis_input(ui, cx, timeline, &xs, at, axis);
    }
}

/// The clock and what the pane shows: *Now* and the count of earlier
/// states, a point's date, label, source and *Read-only*, or the reading
/// text, clipped to `inner`.
fn label(ui: &egui::Ui, inner: egui::Rect, timeline: Option<&Timeline>, showing: Option<usize>) {
    let scheme = theme::scheme(ui);
    let painter = ui.painter_at(inner);
    let centre = inner.center().y;
    let past = showing.is_some();
    let (strong, weak, icon) = if past {
        (
            scheme.text_on_selected,
            scheme.text_on_selected,
            scheme.text_on_selected,
        )
    } else {
        (
            scheme.text_primary,
            scheme.text_secondary,
            scheme.icon_default,
        )
    };
    let clock = painter.text(
        egui::pos2(inner.left(), centre),
        egui::Align2::LEFT_CENTER,
        icons::TIMELINE,
        egui::FontId::proportional(ICON),
        icon,
    );
    let mut x = clock.right() + 8.0;
    let mut text = |s: &str, colour: egui::Color32| {
        let r = painter.text(
            egui::pos2(x, centre),
            egui::Align2::LEFT_CENTER,
            s,
            egui::FontId::proportional(TEXT_BODY),
            colour,
        );
        x = r.right() + 6.0;
    };
    match (timeline, showing) {
        (None, _) => text(&t!("timeline-reading"), weak),
        (Some(timeline), None) => {
            text(&t!("timeline-now"), strong);
            let count = timeline.points.len();
            text(&format!("· {}", t!("timeline-states", count = count)), weak);
        }
        (Some(timeline), Some(i)) => {
            if let Some(point) = timeline.points.get(i) {
                text(&short(point.time), strong);
                text(&format!("· {} · {}", point.label, point.source), weak);
                pill(&painter, scheme, x, centre, &t!("timeline-read-only"));
            }
        }
    }
}

/// The width the label column needs for the longest thing it may say: *Now*
/// with the count, or the point with the longest name and source, its
/// date and the *Read-only* pill.
fn label_width(ui: &egui::Ui, timeline: Option<&Timeline>) -> f32 {
    let width = |text: String, size: f32| {
        ui.painter()
            .layout_no_wrap(
                text,
                egui::FontId::proportional(size),
                egui::Color32::PLACEHOLDER,
            )
            .size()
            .x
    };
    // The clock, its gap and the gaps between the parts.
    let fixed = ICON + 8.0 + 3.0 * 6.0;
    let Some(timeline) = timeline else {
        return fixed + width(t!("timeline-reading"), TEXT_BODY);
    };
    let now = width(t!("timeline-now"), TEXT_BODY)
        + width(
            format!("· {}", t!("timeline-states", count = timeline.points.len())),
            TEXT_BODY,
        );
    let past = timeline
        .points
        .iter()
        .max_by_key(|p| p.label.chars().count() + p.source.chars().count())
        .map_or(0.0, |p| {
            width(short(p.time), TEXT_BODY)
                + width(format!("· {} · {}", p.label, p.source), TEXT_BODY)
                + width(t!("timeline-read-only"), 11.0)
                + 20.0
        });
    fixed + now.max(past)
}

/// The *Back to now* button's size.
fn back_button_size(ui: &egui::Ui) -> egui::Vec2 {
    let galley = ui.painter().layout_no_wrap(
        t!("timeline-back-to-now"),
        egui::FontId::proportional(TEXT_BODY),
        egui::Color32::PLACEHOLDER,
    );
    egui::vec2(galley.size().x + 20.0, theme::FOOTER_CONTROL)
}

/// The right end: *Now*, or the *Back to now* button at a past point.
fn right_end(
    ui: &egui::Ui,
    cx: &mut PaneCx<'_>,
    inner: egui::Rect,
    timeline: Option<&Timeline>,
    past: bool,
) {
    let scheme = theme::scheme(ui);
    let painter = ui.painter();
    let centre = inner.center().y;
    let body = egui::FontId::proportional(TEXT_BODY);
    let Some(timeline) = timeline.filter(|_| past) else {
        let colour = if timeline.is_some() {
            scheme.accent_focus
        } else {
            scheme.text_secondary
        };
        // At the axis's end, in the room the button takes at a past point.
        painter.text(
            egui::pos2(inner.right() - back_button_size(ui).x, centre),
            egui::Align2::LEFT_CENTER,
            t!("timeline-now"),
            body,
            colour,
        );
        return;
    };
    let label = t!("timeline-back-to-now");
    let galley = painter.layout_no_wrap(label.clone(), body, scheme.text_primary);
    let size = back_button_size(ui);
    let button = egui::Rect::from_min_size(
        egui::pos2(inner.right() - size.x, centre - size.y / 2.0),
        size,
    );
    let response = ui.interact(button, id(cx.pane).with("now"), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let fill = if response.hovered() {
        scheme.bg_subtle
    } else {
        scheme.bg_control
    };
    painter.rect_filled(button, theme::metrics(ui.ctx()).corner_radius(), fill);
    painter.galley(
        button.center() - galley.size() / 2.0,
        galley,
        scheme.text_primary,
    );
    if response.clicked() {
        travel(cx, timeline.live.clone());
    }
}

/// The dots, merged where they crowd, and the handle on the point shown.
fn draw_axis(ui: &egui::Ui, xs: &[f32], showing: Option<usize>, axis: egui::Rect) {
    let scheme = theme::scheme(ui);
    let painter = ui.painter();
    let centre = axis.center().y;
    let past = showing.is_some();
    let tick_colour = if past {
        scheme.text_on_selected
    } else {
        scheme.text_secondary
    };
    for tick in ticks(xs, MIN_TICK_GAP) {
        let r = egui::Rect::from_center_size(
            egui::pos2(f32::midpoint(tick.from, tick.to), centre),
            egui::vec2(tick.to - tick.from + 2.0 * DOT, 2.0 * DOT),
        );
        painter.rect_filled(r, DOT, tick_colour);
    }
    let handle_x = showing
        .and_then(|i| xs.get(i).copied())
        .unwrap_or(axis.right());
    let ring = if past {
        scheme.bg_selected_soft
    } else {
        scheme.bg_pane
    };
    painter.circle_filled(
        egui::pos2(handle_x, centre + 1.0),
        HANDLE + 1.0,
        scheme.shadow(),
    );
    painter.circle(
        egui::pos2(handle_x, centre),
        HANDLE,
        scheme.accent_focus,
        egui::Stroke::new(2.0, ring),
    );
}

/// Click, drag with the rest rule, the keys while focused, and the
/// tooltip of the tick under the pointer.
fn axis_input(
    ui: &egui::Ui,
    cx: &mut PaneCx<'_>,
    timeline: &Timeline,
    xs: &[f32],
    at: Option<usize>,
    axis: egui::Rect,
) {
    let now_x = axis.right();
    let area = axis.expand2(egui::vec2(HANDLE, 0.0));
    let response = ui.interact(area, id(cx.pane), egui::Sense::click_and_drag());
    let name = t!("pane-timeline");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, &name));
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    if response.has_focus() {
        focus(ui, cx);
    }
    let time = ui.input(|i| i.time);
    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let mut target = None;
    if response.dragged()
        && let Some(pos) = pointer
    {
        let snapped = nearest(xs, pos.x, now_x);
        let state = &mut cx.tab_ui.timeline;
        if state.drag != Some(snapped) {
            state.drag = Some(snapped);
            state.moved_at = time;
        }
        if time - state.moved_at >= REST.as_secs_f64() {
            target = Some(snapped);
        } else {
            ui.ctx().request_repaint_after(REST);
        }
    } else if response.drag_stopped() {
        target = cx.tab_ui.timeline.drag.take();
    } else if response.clicked()
        && let Some(pos) = pointer
    {
        target = Some(nearest(xs, pos.x, now_x));
    } else {
        cx.tab_ui.timeline.drag = None;
    }
    if response.has_focus() {
        let pressed: Vec<egui::Key> =
            ui.input(|i| KEYS.iter().copied().filter(|k| i.key_pressed(*k)).collect());
        for key in pressed {
            target = step(at, xs.len(), key).or(target);
        }
    }
    if let Some(target) = target
        && target != at
    {
        go(cx, timeline, target);
    }
    if cx.tab_ui.timeline.drag.is_none() {
        tooltip(&response, timeline, xs);
    }
}

/// Keeps the arrows with the focused bar and draws the focus ring.
fn focus(ui: &egui::Ui, cx: &PaneCx<'_>) {
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            id(cx.pane),
            egui::EventFilter {
                horizontal_arrows: true,
                ..Default::default()
            },
        );
    });
    if let Some(rect) = cx.tab_ui.timeline.rect {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            0.0,
            theme::scheme(ui).focus_ring(),
            egui::StrokeKind::Inside,
        );
    }
}

/// The tooltip of the tick under the pointer: date and time, label and
/// source.
fn tooltip(response: &egui::Response, timeline: &Timeline, xs: &[f32]) {
    let Some(pos) = response.hover_pos() else {
        return;
    };
    let hit = xs
        .iter()
        .enumerate()
        .filter(|(_, x)| (**x - pos.x).abs() <= HOVER)
        .min_by(|(_, a), (_, b)| (**a - pos.x).abs().total_cmp(&(**b - pos.x).abs()))
        .and_then(|(i, _)| timeline.points.get(i));
    let Some(point) = hit else {
        return;
    };
    response.clone().on_hover_ui_at_pointer(|ui| {
        ui.label(egui::RichText::new(long(point.time)).strong());
        ui.horizontal(|ui| {
            ui.label(&point.label);
            ui.label(
                egui::RichText::new(format!("· {}", point.source))
                    .size(TEXT_SECONDARY)
                    .color(theme::scheme(ui).text_secondary),
            );
        });
    });
}

/// A small pill after the label: *Read-only*.
fn pill(painter: &egui::Painter, scheme: &theme::Scheme, x: f32, centre: f32, text: &str) {
    let font = egui::FontId::proportional(11.0);
    let galley = painter.layout_no_wrap(text.to_owned(), font, scheme.text_on_selected);
    let size = galley.size() + egui::vec2(16.0, 4.0);
    let rect = egui::Rect::from_min_size(egui::pos2(x + 2.0, centre - size.y / 2.0), size);
    painter.rect(
        rect,
        size.y / 2.0,
        scheme.bg_card,
        egui::Stroke::new(1.0, scheme.accent_focus),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        scheme.text_on_selected,
    );
}

/// Takes the pane to point `target` (`None`: *Now*).
fn go(cx: &mut PaneCx<'_>, timeline: &Timeline, target: Option<usize>) {
    let to = match target {
        Some(i) => match timeline.points.get(i) {
            Some(point) => point.location.clone(),
            None => return,
        },
        None => timeline.live.clone(),
    };
    travel(cx, to);
}

/// Emits the travel to `to`: the first of a run when the pane is not where
/// the bar last sent it.
fn travel(cx: &mut PaneCx<'_>, to: Location) {
    if cx.tab.location == to {
        return;
    }
    let first = cx.tab_ui.timeline.last.as_ref() != Some(&cx.tab.location);
    cx.tab_ui.timeline.last = Some(to.clone());
    cx.requests.push(Request::Command(Command::TimeTravel {
        pane: cx.pane,
        to,
        first,
    }));
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn timeline(times: &[u64]) -> Timeline {
        let here = || Location::new(types::ProviderId("t"), "/docs");
        Timeline {
            live: here(),
            points: times
                .iter()
                .map(|&t| ext::TimePoint {
                    time: at(t),
                    label: t.to_string(),
                    source: "test".to_owned(),
                    location: here(),
                })
                .collect(),
        }
    }

    /// Two snapshots in the same minute and one three days later, as on a
    /// real dataset: each has a place of its own, the newest clear of
    /// *Now*'s handle.
    #[test]
    fn points_are_evenly_spaced_by_index() {
        let xs = positions(&timeline(&[0, 30, 259_200]), 10.0, 130.0);
        assert_eq!(xs, [10.0, 50.0, 90.0]);
        assert!(130.0 - xs[2] > 2.0 * HANDLE, "clear of Now's handle");
        assert_eq!(ticks(&xs, MIN_TICK_GAP).len(), 3, "none merged");
        assert_eq!(positions(&timeline(&[]), 10.0, 130.0), Vec::<f32>::new());
        assert_eq!(positions(&timeline(&[5]), 10.0, 130.0), [10.0]);
    }

    #[test]
    fn crowded_ticks_merge() {
        let merged = ticks(&[0.0, 3.0, 7.5, 20.0, 40.0, 42.0], MIN_TICK_GAP);
        let spans: Vec<(usize, usize)> = merged.iter().map(|t| (t.first, t.last)).collect();
        assert_eq!(spans, [(0, 2), (3, 3), (4, 5)]);
        assert_eq!(ticks(&[], MIN_TICK_GAP), []);
    }

    #[test]
    fn thousands_of_points_stay_reachable() {
        let xs: Vec<f32> = (0..5000u16).map(|i| f32::from(i) * 0.1).collect();
        assert!(ticks(&xs, MIN_TICK_GAP).len() < 200, "drawn merged");
        // The keyboard reaches every one of them.
        let mut here = None;
        let mut seen = 0;
        while let Some(next) = step(here, xs.len(), egui::Key::ArrowLeft) {
            if next == here {
                break;
            }
            here = next;
            seen += 1;
        }
        assert_eq!(seen, 5000);
        assert_eq!(here, Some(0));
    }

    #[test]
    fn nearest_snaps_to_a_point_or_now() {
        let xs = [10.0, 50.0, 90.0];
        assert_eq!(nearest(&xs, 12.0, 100.0), Some(0));
        assert_eq!(nearest(&xs, 31.0, 100.0), Some(1));
        assert_eq!(nearest(&xs, 97.0, 100.0), None, "Now is nearer");
        assert_eq!(nearest(&[], 50.0, 100.0), None);
        assert_eq!(nearest(&[40.0], 0.0, 100.0), Some(0), "a single point");
    }

    #[test]
    fn keys_step_by_index() {
        use egui::Key::{A, ArrowLeft, ArrowRight, End, Home};
        assert_eq!(
            step(None, 3, ArrowLeft),
            Some(Some(2)),
            "from Now to the newest"
        );
        assert_eq!(step(Some(1), 3, ArrowLeft), Some(Some(0)));
        assert_eq!(
            step(Some(0), 3, ArrowLeft),
            Some(Some(0)),
            "stays at the oldest"
        );
        assert_eq!(step(Some(1), 3, ArrowRight), Some(Some(2)));
        assert_eq!(
            step(Some(2), 3, ArrowRight),
            Some(None),
            "past the newest is Now"
        );
        assert_eq!(step(None, 3, ArrowRight), Some(None));
        assert_eq!(step(Some(2), 3, Home), Some(Some(0)));
        assert_eq!(step(Some(0), 3, End), Some(None));
        assert_eq!(step(Some(0), 3, A), None);
        assert_eq!(step(None, 0, Home), None);
        assert_eq!(step(Some(0), 1, ArrowLeft), Some(Some(0)), "a single point");
    }
}
