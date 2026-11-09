//! The operations drawer (SPEC.md *Operations drawer*): a `bg/card` card
//! at the foot of the side panel — or along the window's foot while the
//! panel is hidden — with its top corners rounded, a hairline on top and a
//! soft shadow upwards, floating over the end of the panel's lists. It is
//! there only while the job table has jobs. Top to bottom: the summary
//! (*2 running · 2 waiting*, the overall percentage, a caret) over a 4 px
//! overall bar, which a click collapses to and expands from; one block per
//! running job — its icon, title, pause and cancel, a thin bar and a detail
//! line with what is done of the total, the throughput and the time left;
//! the waiting jobs under a *WAITING* header with why each waits; finished
//! jobs as compact rows that leave after a few seconds, a failed one with
//! its error until dismissed. The rows scroll beyond a cap. Progress
//! repaints are throttled upstream: the runtime coalesces a job's reports
//! to one every 50 ms and the shell repaints at most every 50 ms while a
//! job runs, so a job over hundreds of thousands of files never redraws
//! the drawer per entry.

use ext::t;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use app::Command;
use egui::{Align, Align2, CornerRadius, FontId, Rect, Sense, pos2, vec2};
use model::{JobInfo, JobState, Model};
use types::{JobId, Progress};

use crate::shell::Request;
use crate::theme::{self, BAR, ICON, Scheme, TEXT_BODY, TEXT_SECONDARY};
use crate::{format, icons, portal};

/// How long a finished or cancelled job stays listed.
pub const DONE_FOR: Duration = Duration::from_secs(5);
/// How far back the throughput looks.
const RATE_WINDOW: Duration = Duration::from_secs(3);
/// The drawer's top corners.
pub const DRAWER_RADIUS: u8 = 12;
/// The padding inside the drawer.
const PAD: f32 = 12.0;
/// The summary's height: its line and the overall bar.
const SUMMARY: f32 = 36.0;
/// A running job's block: the title line, the thin bar, the detail line.
const RUNNING: f32 = 64.0;
/// A waiting job's row: its title line and the reason under it.
const WAITING: f32 = 44.0;
/// The *WAITING* header.
const WAITING_HEADER: f32 = 22.0;
/// A finished or failed job's compact row.
const COMPACT: f32 = 28.0;
/// The rows scroll beyond this many running blocks' worth of height.
pub const ROWS_CAP: usize = 5;
/// The round cancel button's diameter.
const ROUND: f32 = 22.0;
/// The detail line's text size.
const TEXT_DETAIL: f32 = 11.0;
/// The *WAITING* header's text size.
const TEXT_HEADER: f32 = 10.0;

/// Progress samples of one job, for its throughput.
#[derive(Debug, Default)]
struct Rate {
    samples: VecDeque<(Instant, u64)>,
}

impl Rate {
    fn push(&mut self, now: Instant, done: u64) {
        if self.samples.back().is_some_and(|(_, d)| *d == done) {
            return;
        }
        self.samples.push_back((now, done));
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > RATE_WINDOW)
            && self.samples.len() > 2
        {
            self.samples.pop_front();
        }
    }

    /// Units per second over the window, if there are two samples.
    fn per_second(&self) -> Option<f64> {
        let (first, last) = (self.samples.front()?, self.samples.back()?);
        let seconds = last.0.duration_since(first.0).as_secs_f64();
        if seconds <= 0.0 || last.1 <= first.1 {
            return None;
        }
        Some((last.1 - first.1) as f64 / seconds)
    }
}

/// The drawer's own state.
#[derive(Debug, Default)]
pub struct Operations {
    rates: HashMap<JobId, Rate>,
    done_at: HashMap<JobId, Instant>,
    /// How many times the rows were drawn, for the repaint tests.
    paints: u64,
}

/// `12.3 MiB/s`, or `48 items/s`.
fn throughput(progress: &Progress, rate: f64) -> String {
    match progress {
        Progress::Bytes { .. } => format!("{}/s", format::size(Some(rate as u64))),
        Progress::Items { .. } => format!("{} items/s", rate.round() as u64),
        Progress::Message(_) => String::new(),
    }
}

/// `53 s left`, `10 min left`, `2 h left`.
fn remaining(left: f64) -> String {
    if left < 1.0 {
        "less than a second left".to_owned()
    } else if left < 90.0 {
        format!("{} s left", left.round() as u64)
    } else if left < 5400.0 {
        format!("{} min left", (left / 60.0).round() as u64)
    } else {
        format!("{} h left", (left / 3600.0).round() as u64)
    }
}

/// Done and total of a progress report, in its unit.
fn counts(progress: &Progress) -> Option<(u64, Option<u64>)> {
    match progress {
        Progress::Bytes { done, total } | Progress::Items { done, total } => Some((*done, *total)),
        Progress::Message(_) => None,
    }
}

/// `473 MiB of 2.2 GiB`, `12 of 40 items`, or the job's message.
fn progress_text(progress: &Progress) -> String {
    match progress {
        Progress::Bytes { done, total } => match total {
            Some(total) => format!(
                "{} of {}",
                format::size(Some(*done)),
                format::size(Some(*total))
            ),
            None => format::size(Some(*done)),
        },
        Progress::Items { done, total } => match total {
            Some(total) => format!("{done} of {total} items"),
            None => format!("{done} items"),
        },
        Progress::Message(message) => message.clone(),
    }
}

/// The mean fraction of the running jobs with a known total.
fn overall(model: &Model) -> Option<f32> {
    let fractions: Vec<f32> = model
        .jobs
        .values()
        .filter_map(|job| match &job.state {
            JobState::Running(progress) => counts(progress),
            _ => None,
        })
        .filter_map(|(done, total)| total.map(|t| done as f32 / t.max(1) as f32))
        .collect();
    if fractions.is_empty() {
        None
    } else {
        Some(fractions.iter().sum::<f32>() / fractions.len() as f32)
    }
}

/// *2 running · 2 waiting*: the waiting part only while something waits.
fn summary_text(running: usize, waiting: usize) -> String {
    let mut text = t!("jobs-running-count", count = running);
    if waiting > 0 {
        text.push_str(" · ");
        text.push_str(&t!("jobs-waiting-count", count = waiting));
    }
    text
}

/// The glyph of a job that is not a copy or a move, by its title: the
/// archive glyph for an archive job. A copy or a move gets the portal
/// spinner instead.
fn job_glyph(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    ["extract", "compress", "archive", "zip", "unzip"]
        .iter()
        .any(|word| lower.starts_with(word))
        .then_some(icons::ARCHIVE)
}

/// A 4 px bar in `rect`: `accent/focus` over `bg/subtle`.
fn thin_bar(ui: &egui::Ui, rect: Rect, fraction: f32, scheme: &Scheme) {
    ui.painter().rect_filled(rect, 2.0, scheme.bg_subtle);
    let filled = Rect::from_min_size(
        rect.min,
        vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()),
    );
    ui.painter().rect_filled(filled, 2.0, scheme.accent_focus);
}

/// A round 22 px button with a glyph: `bg/control` when `filled`, a ghost
/// button otherwise. Its accessible name and tooltip are `label`.
fn round_button(
    ui: &mut egui::Ui,
    id: egui::Id,
    center: egui::Pos2,
    glyph: &str,
    label: &str,
    filled: bool,
) -> egui::Response {
    let scheme = theme::scheme(ui);
    let rect = Rect::from_center_size(center, vec2(ROUND, ROUND));
    let response = ui.interact(rect, id, Sense::click());
    if filled || response.hovered() {
        ui.painter().circle_filled(
            center,
            ROUND / 2.0,
            if response.hovered() {
                scheme.bg_active
            } else {
                scheme.bg_control
            },
        );
    }
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(12.0),
        scheme.icon_default,
    );
    let spoken = label.to_owned();
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spoken.clone()));
    response.on_hover_text(label)
}

/// A label laid out from the left of `rect`, truncated, a widget so it is
/// read aloud.
fn placed_label(ui: &mut egui::Ui, rect: Rect, text: egui::RichText) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    child.add(egui::Label::new(text).truncate().selectable(false));
}

impl Operations {
    /// Whether any finished row is still shown, so the frame keeps painting
    /// until it goes.
    #[must_use]
    pub fn has_finished_rows(&self) -> bool {
        !self.done_at.is_empty()
    }

    /// How many times the rows were drawn so far.
    #[must_use]
    pub fn paints(&self) -> u64 {
        self.paints
    }

    /// The height the rows take: a block per running job, the header and
    /// a row per waiting job, a compact row per finished one.
    fn rows_height(model: &Model) -> f32 {
        let mut height = 0.0;
        let mut waiting = false;
        for job in model.jobs.values() {
            height += match job.state {
                JobState::Running(_) => RUNNING,
                JobState::Waiting(_) => {
                    waiting = true;
                    WAITING
                }
                JobState::Done | JobState::Cancelled | JobState::Failed(_) => COMPACT,
            };
        }
        if waiting {
            height += WAITING_HEADER;
        }
        height
    }

    /// The drawer's height for the jobs in the table: the summary alone
    /// while collapsed, else the summary and the rows up to the cap of
    /// [`ROWS_CAP`] running blocks, beyond which the rows scroll.
    #[must_use]
    pub fn height(model: &Model, collapsed: bool) -> f32 {
        let mut height = PAD + SUMMARY + PAD;
        if !collapsed {
            let rows = Self::rows_height(model);
            if rows > 0.0 {
                height += rows.min(ROWS_CAP as f32 * RUNNING) + PAD;
            }
        }
        height
    }

    /// Draws the drawer in the `ui`'s rectangle: the card, the summary and,
    /// unless `collapsed`, the rows. A click on the summary flips
    /// `collapsed`. `still` draws the spinners at rest (reduce motion).
    #[allow(clippy::too_many_lines)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        model: &Model,
        requests: &mut Vec<Request>,
        collapsed: &mut bool,
        still: bool,
    ) {
        self.paints += 1;
        let now = Instant::now();
        let scheme = theme::scheme(ui);
        let rect = ui.max_rect();
        let radius = CornerRadius {
            nw: DRAWER_RADIUS,
            ne: DRAWER_RADIUS,
            sw: 0,
            se: 0,
        };
        let shadow = egui::epaint::Shadow {
            offset: [0, -4],
            blur: 16,
            spread: 0,
            color: scheme.shadow(),
        };
        ui.painter().add(shadow.as_shape(rect, radius));
        ui.painter().rect_filled(rect, radius, scheme.bg_card);
        ui.painter()
            .hline(rect.x_range(), rect.top() + 0.5, scheme.hairline());
        let jobs: Vec<(JobId, JobInfo)> = model
            .jobs
            .iter()
            .map(|(id, info)| (*id, info.clone()))
            .collect();
        let running = jobs
            .iter()
            .filter(|(_, j)| matches!(j.state, JobState::Running(_)))
            .count();
        let waiting = jobs
            .iter()
            .filter(|(_, j)| matches!(j.state, JobState::Waiting(_)))
            .count();
        // The summary and the overall bar.
        let summary = Rect::from_min_size(
            pos2(rect.left() + PAD, rect.top() + PAD),
            vec2((rect.width() - 2.0 * PAD).max(0.0), SUMMARY),
        );
        let response = ui.interact(summary, ui.id().with("operations-summary"), Sense::click());
        let line_y = summary.top() + 9.0;
        let text = summary_text(running, waiting);
        let caret =
            Rect::from_center_size(pos2(summary.right() - ICON / 2.0, line_y), vec2(ICON, ICON));
        ui.painter().text(
            caret.center(),
            Align2::CENTER_CENTER,
            if *collapsed {
                icons::CARET_UP
            } else {
                icons::TREE_OPEN
            },
            FontId::proportional(ICON),
            scheme.icon_default,
        );
        let mut right = caret.left() - 8.0;
        if let Some(fraction) = overall(model) {
            let percent = ui.painter().layout_no_wrap(
                format!("{}%", (fraction * 100.0).round() as u32),
                FontId::proportional(TEXT_BODY),
                scheme.accent_focus,
            );
            let size = percent.size();
            ui.painter().galley(
                pos2(right - size.x, line_y - size.y / 2.0),
                percent,
                scheme.accent_focus,
            );
            right -= size.x + 8.0;
        }
        let title = egui::WidgetText::from(
            egui::RichText::new(&text)
                .size(TEXT_BODY)
                .strong()
                .color(scheme.text_primary),
        )
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (right - summary.left()).max(0.0),
            egui::TextStyle::Body,
        );
        let title_size = title.size();
        ui.painter().galley(
            pos2(summary.left(), line_y - title_size.y / 2.0),
            title,
            scheme.text_primary,
        );
        thin_bar(
            ui,
            Rect::from_min_size(
                pos2(summary.left(), summary.bottom() - BAR),
                vec2(summary.width(), BAR),
            ),
            overall(model).unwrap_or(0.0),
            scheme,
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text.clone())
        });
        let hint = if *collapsed {
            t!("jobs-expand")
        } else {
            t!("jobs-collapse")
        };
        if response.on_hover_text(hint).clicked() {
            *collapsed = !*collapsed;
        }
        if !*collapsed {
            let rows = Rect::from_min_max(
                pos2(rect.left(), summary.bottom() + PAD),
                pos2(rect.right(), rect.bottom() - PAD),
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rows)
                    .layout(egui::Layout::top_down(Align::Min)),
            );
            egui::ScrollArea::vertical()
                .id_salt("operations-rows")
                .auto_shrink([false, false])
                .max_height(rows.height())
                .show(&mut child, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (id, info) in &jobs {
                        if let JobState::Running(progress) = &info.state {
                            self.running_block(ui, now, *id, info, progress, requests, still);
                        }
                    }
                    if waiting > 0 {
                        let (header, _) = ui.allocate_exact_size(
                            vec2(ui.available_width(), WAITING_HEADER),
                            Sense::hover(),
                        );
                        placed_label(
                            ui,
                            Rect::from_min_max(
                                pos2(header.left() + PAD, header.top() + 6.0),
                                header.max,
                            ),
                            egui::RichText::new(t!("jobs-waiting").to_uppercase())
                                .size(TEXT_HEADER)
                                .color(scheme.text_secondary)
                                .extra_letter_spacing(0.8),
                        );
                        for (id, info) in &jobs {
                            if let JobState::Waiting(for_job) = &info.state {
                                Self::waiting_row(ui, *id, info, for_job, requests);
                            }
                        }
                    }
                    for (id, info) in &jobs {
                        match &info.state {
                            JobState::Done | JobState::Cancelled | JobState::Failed(_) => {
                                self.compact_row(ui, now, *id, info, requests);
                            }
                            JobState::Running(_) | JobState::Waiting(_) => {}
                        }
                    }
                });
        }
        self.rates.retain(|id, _| model.jobs.contains_key(id));
        self.done_at.retain(|id, _| model.jobs.contains_key(id));
    }

    /// A running job under a hairline: its icon, its title with pause and
    /// cancel at the right, the thin bar, and the detail line — what is
    /// done of the total, the throughput and the time left, or the job's
    /// message; *Paused* while paused.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn running_block(
        &mut self,
        ui: &mut egui::Ui,
        now: Instant,
        id: JobId,
        info: &JobInfo,
        progress: &Progress,
        requests: &mut Vec<Request>,
        still: bool,
    ) {
        let scheme = theme::scheme(ui);
        let (outer, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), RUNNING), Sense::hover());
        let rect = outer.shrink2(vec2(PAD, 0.0));
        ui.painter()
            .hline(outer.x_range(), outer.top() + 0.5, scheme.hairline());
        let line_y = rect.top() + 18.0;
        let icon_at = pos2(rect.left() + ICON / 2.0, line_y);
        if let Some(glyph) = job_glyph(&info.name) {
            ui.painter().text(
                icon_at,
                Align2::CENTER_CENTER,
                glyph,
                FontId::proportional(ICON),
                scheme.icon_default,
            );
        } else {
            let time = ui.input(|i| i.time);
            portal::spinner(
                ui.painter(),
                icon_at,
                ICON,
                time,
                scheme.accent_focus,
                still,
            );
            if !still {
                ui.ctx().request_repaint();
            }
        }
        // Cancel, and pause or resume before it for a pausable job.
        let cancel_at = pos2(rect.right() - ROUND / 2.0, line_y);
        if round_button(
            ui,
            ui.id().with(("cancel", id)),
            cancel_at,
            icons::CLOSE,
            &t!("jobs-cancel"),
            true,
        )
        .clicked()
        {
            requests.push(Request::Command(Command::CancelJob(id)));
        }
        let mut buttons_left = cancel_at.x - ROUND / 2.0;
        if info.pausable {
            let at = pos2(buttons_left - 6.0 - ROUND / 2.0, line_y);
            let (glyph, label, command) = if info.paused {
                (icons::PLAY, t!("jobs-resume"), Command::ResumeJob(id))
            } else {
                (icons::PAUSE, t!("jobs-pause"), Command::PauseJob(id))
            };
            if round_button(ui, ui.id().with(("pause", id)), at, glyph, &label, false).clicked() {
                requests.push(Request::Command(command));
            }
            buttons_left = at.x - ROUND / 2.0;
        }
        let title_x = rect.left() + ICON + 10.0;
        placed_label(
            ui,
            Rect::from_min_max(
                pos2(title_x, line_y - 9.0),
                pos2((buttons_left - 8.0).max(title_x), line_y + 9.0),
            ),
            egui::RichText::new(&info.name)
                .size(TEXT_SECONDARY)
                .strong()
                .color(scheme.text_primary),
        );
        let (done, total) = counts(progress).unwrap_or((0, None));
        let fraction = total.map_or(0.0, |t| done as f32 / t.max(1) as f32);
        thin_bar(
            ui,
            Rect::from_min_size(
                pos2(rect.left(), rect.top() + 34.0),
                vec2(rect.width(), BAR),
            ),
            fraction,
            scheme,
        );
        let detail = if info.paused {
            t!("jobs-paused-line")
        } else {
            let mut line = progress_text(progress);
            if !matches!(progress, Progress::Message(_)) {
                let rate = self.rates.entry(id).or_default();
                rate.push(now, done);
                if let Some(per_second) = rate.per_second() {
                    line.push_str(" · ");
                    line.push_str(&throughput(progress, per_second));
                    if let Some(total) = total
                        && total > done
                    {
                        line.push_str(" · ");
                        line.push_str(&remaining((total - done) as f64 / per_second));
                    }
                }
            }
            line
        };
        placed_label(
            ui,
            Rect::from_min_max(
                pos2(rect.left(), rect.top() + 42.0),
                pos2(rect.right(), rect.bottom()),
            ),
            egui::RichText::new(detail)
                .size(TEXT_DETAIL)
                .color(scheme.text_secondary),
        );
    }

    /// A waiting job: the hourglass at half opacity, the title in
    /// `text/secondary`, the round cancel button, and under the row,
    /// indented to the title, why it waits.
    fn waiting_row(
        ui: &mut egui::Ui,
        id: JobId,
        info: &JobInfo,
        for_job: &str,
        requests: &mut Vec<Request>,
    ) {
        let scheme = theme::scheme(ui);
        let (outer, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), WAITING), Sense::hover());
        let rect = outer.shrink2(vec2(PAD, 0.0));
        let line_y = rect.top() + 12.0;
        ui.painter().text(
            pos2(rect.left() + ICON / 2.0, line_y),
            Align2::CENTER_CENTER,
            icons::WAITING,
            FontId::proportional(ICON),
            scheme.icon_default.gamma_multiply(0.5),
        );
        let cancel_at = pos2(rect.right() - ROUND / 2.0, line_y);
        if round_button(
            ui,
            ui.id().with(("cancel", id)),
            cancel_at,
            icons::CLOSE,
            &t!("jobs-cancel"),
            true,
        )
        .clicked()
        {
            requests.push(Request::Command(Command::CancelJob(id)));
        }
        let title_x = rect.left() + ICON + 10.0;
        placed_label(
            ui,
            Rect::from_min_max(
                pos2(title_x, line_y - 9.0),
                pos2((cancel_at.x - ROUND / 2.0 - 8.0).max(title_x), line_y + 9.0),
            ),
            egui::RichText::new(&info.name)
                .size(TEXT_SECONDARY)
                .color(scheme.text_secondary),
        );
        placed_label(
            ui,
            Rect::from_min_max(
                pos2(title_x, rect.top() + 24.0),
                pos2(rect.right(), rect.bottom()),
            ),
            egui::RichText::new(t!("jobs-queued-behind", job = for_job))
                .size(TEXT_DETAIL)
                .color(scheme.text_secondary),
        );
    }

    /// A finished job as a compact row with its result, gone after
    /// [`DONE_FOR`] or a click on its button; a failed job with its error in
    /// `status/danger` until dismissed.
    fn compact_row(
        &mut self,
        ui: &mut egui::Ui,
        now: Instant,
        id: JobId,
        info: &JobInfo,
        requests: &mut Vec<Request>,
    ) {
        let scheme = theme::scheme(ui);
        let (outer, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), COMPACT), Sense::hover());
        let rect = outer.shrink2(vec2(PAD, 0.0));
        let line_y = rect.center().y;
        let (glyph, result, colour) = match &info.state {
            JobState::Failed(error) => (icons::FAILED, error.clone(), scheme.status_danger),
            JobState::Cancelled => (icons::CLOSE, t!("jobs-cancelled"), scheme.text_secondary),
            _ => (icons::DONE, t!("jobs-done"), scheme.text_secondary),
        };
        ui.painter().text(
            pos2(rect.left() + ICON / 2.0, line_y),
            Align2::CENTER_CENTER,
            glyph,
            FontId::proportional(ICON),
            colour,
        );
        let dismiss_at = pos2(rect.right() - ROUND / 2.0, line_y);
        let dismiss = round_button(
            ui,
            ui.id().with(("dismiss", id)),
            dismiss_at,
            icons::CLOSE,
            &t!("jobs-dismiss"),
            true,
        )
        .clicked();
        let title_x = rect.left() + ICON + 10.0;
        let mut text = egui::text::LayoutJob::default();
        text.append(
            &info.name,
            0.0,
            egui::TextFormat::simple(FontId::proportional(TEXT_SECONDARY), scheme.text_secondary),
        );
        text.append(
            &format!(" · {result}"),
            0.0,
            egui::TextFormat::simple(FontId::proportional(TEXT_DETAIL), colour),
        );
        placed_label(
            ui,
            Rect::from_min_max(
                pos2(title_x, line_y - 9.0),
                pos2(
                    (dismiss_at.x - ROUND / 2.0 - 8.0).max(title_x),
                    line_y + 9.0,
                ),
            ),
            egui::RichText::new(format!("{} · {result}", info.name))
                .size(TEXT_SECONDARY)
                .color(colour),
        );
        let gone = if matches!(info.state, JobState::Failed(_)) {
            dismiss
        } else {
            let at = *self.done_at.entry(id).or_insert(now);
            dismiss || now.duration_since(at) >= DONE_FOR
        };
        if gone {
            requests.push(Request::Command(Command::DismissJob(id)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throughput_and_remaining_read_well() {
        let bytes = Progress::Bytes {
            done: 10,
            total: Some(20),
        };
        assert_eq!(throughput(&bytes, 1_048_576.0), "1.0 MiB/s");
        assert_eq!(progress_text(&bytes), "10 B of 20 B");
        assert_eq!(
            throughput(
                &Progress::Items {
                    done: 1,
                    total: None
                },
                3.4
            ),
            "3 items/s"
        );
        assert_eq!(remaining(0.4), "less than a second left");
        assert_eq!(remaining(12.0), "12 s left");
        assert_eq!(remaining(600.0), "10 min left");
        assert_eq!(remaining(7200.0), "2 h left");
        let mut rate = Rate::default();
        let start = Instant::now();
        rate.push(start, 0);
        rate.push(start + Duration::from_secs(1), 100);
        assert!(rate.per_second().is_some_and(|r| (r - 100.0).abs() < 1e-6));
        rate.push(start + Duration::from_secs(2), 100);
        assert_eq!(rate.samples.len(), 2, "an unchanged count is no sample");
    }

    #[test]
    fn the_height_grows_with_the_jobs_up_to_the_cap() {
        let mut model = Model::new(
            model::Settings::default(),
            types::Location::new(types::ProviderId("t"), "/"),
        );
        let summary = Operations::height(&model, false);
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(close(summary, PAD + SUMMARY + PAD), "the summary alone");
        for i in 0..3 {
            model.job_started(JobId(i), format!("job {i}"), false);
        }
        assert!(
            close(
                Operations::height(&model, false),
                summary + 3.0 * RUNNING + PAD
            ),
            "three blocks"
        );
        assert!(
            close(Operations::height(&model, true), summary),
            "collapsed to the summary"
        );
        for i in 3..20 {
            model.job_started(JobId(i), format!("job {i}"), false);
        }
        assert!(
            close(
                Operations::height(&model, false),
                summary + ROWS_CAP as f32 * RUNNING + PAD
            ),
            "capped"
        );
        model.job_progress(
            JobId(0),
            Progress::Items {
                done: 1,
                total: Some(2),
            },
        );
        model.job_progress(
            JobId(1),
            Progress::Items {
                done: 1,
                total: Some(4),
            },
        );
        assert!(overall(&model).is_some_and(|f| (f - 0.375).abs() < 1e-6));
        // The summary's words come from the locale files, which the unit
        // tests do not load; the acceptance tests read them.
        assert!(summary_text(2, 0).len() < summary_text(2, 2).len());
        assert_eq!(job_glyph("Extract archive.tar.gz"), Some(icons::ARCHIVE));
        assert_eq!(job_glyph("Copy IMG_1200.png to raw"), None);
    }
}
