//! The operations area: the jobs of the job table, one row each with name,
//! a progress bar with bytes or items done of total, throughput and
//! remaining time, plus pause, resume and cancel; a header with the number
//! of running jobs and an overall bar when more than one runs. A job queued
//! behind another on its volume shows as *Waiting* with that job's name.
//! Finished jobs stay for a few seconds with their result; a failed job
//! stays until dismissed.
//!
//! The area has room to be read: a generous default height that grows with
//! the number of jobs up to [`ROWS_CAP`], after which the rows scroll.
//! Progress repaints are throttled upstream: the runtime coalesces a job's
//! reports to one every 50 ms and the shell repaints at most every 50 ms
//! while a job runs, so a job over hundreds of thousands of files never
//! redraws the panel per entry.

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use app::Command;
use model::{JobInfo, JobState, Model};
use types::{JobId, Progress};

use crate::format;
use crate::icons;
use crate::shell::Request;

/// How long a finished or cancelled job stays listed.
pub const DONE_FOR: Duration = Duration::from_secs(5);
/// How far back the throughput looks.
const RATE_WINDOW: Duration = Duration::from_secs(3);
/// The area shows room for at least this many rows.
pub const ROWS_MIN: usize = 2;
/// The area grows with the jobs up to this many rows; beyond, they scroll.
pub const ROWS_CAP: usize = 5;
/// The height of one job row: three lines and their padding.
const ROW_HEIGHT: f32 = 64.0;
/// The header's height.
const HEADER_HEIGHT: f32 = 24.0;

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

/// The area's own state.
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

/// `2 min left`, `12 s left`.
fn remaining(left: f64) -> String {
    if left < 1.0 {
        "less than a second left".to_owned()
    } else if left < 90.0 {
        format!("{} s left", left.round() as u64)
    } else if left < 90.0 * 60.0 {
        format!("{} min left", (left / 60.0).round() as u64)
    } else {
        format!("{} h left", (left / 3600.0).round() as u64)
    }
}

/// `done` and `total` of a progress report, when it counts.
fn counts(progress: &Progress) -> Option<(u64, Option<u64>)> {
    match progress {
        Progress::Items { done, total } | Progress::Bytes { done, total } => Some((*done, *total)),
        Progress::Message(_) => None,
    }
}

/// `3.2 MiB of 10.0 MiB`, `12 of 300`, or the message.
fn progress_text(progress: &Progress) -> String {
    match progress {
        Progress::Items { done, total } => format!(
            "{done} of {}",
            total.map_or("?".to_owned(), |t| t.to_string())
        ),
        Progress::Bytes { done, total } => {
            format!("{} of {}", format::size(Some(*done)), format::size(*total))
        }
        Progress::Message(text) => text.clone(),
    }
}

/// The overall fraction of the running jobs with a known total, if any.
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

    /// The height the area takes for the jobs in the table: room for
    /// [`ROWS_MIN`] rows at least, one per job up to [`ROWS_CAP`].
    #[must_use]
    pub fn height(model: &Model) -> f32 {
        let rows = model.jobs.len().clamp(ROWS_MIN, ROWS_CAP);
        HEADER_HEIGHT + rows as f32 * ROW_HEIGHT + 8.0
    }

    /// Draws the header and one row per job in the table.
    pub fn show(&mut self, ui: &mut egui::Ui, model: &Model, requests: &mut Vec<Request>) {
        self.paints += 1;
        let now = Instant::now();
        let jobs: Vec<(JobId, JobInfo)> = model
            .jobs
            .iter()
            .map(|(id, info)| (*id, info.clone()))
            .collect();
        header(ui, model);
        egui::ScrollArea::vertical()
            .id_salt("operations-rows")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (id, info) in &jobs {
                    self.row(ui, now, *id, info, requests);
                }
            });
        self.rates.retain(|id, _| model.jobs.contains_key(id));
        self.done_at.retain(|id, _| model.jobs.contains_key(id));
    }

    fn row(
        &mut self,
        ui: &mut egui::Ui,
        now: Instant,
        id: JobId,
        info: &JobInfo,
        requests: &mut Vec<Request>,
    ) {
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                ui.set_min_height(ROW_HEIGHT - 8.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(&info.name).truncate());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.controls(ui, now, id, info, requests);
                        });
                    });
                    match &info.state {
                        JobState::Waiting(for_job) => {
                            ui.add(
                                egui::ProgressBar::new(0.0)
                                    .desired_width(ui.available_width())
                                    .text("Waiting"),
                            );
                            ui.weak(format!("{} Waiting for {for_job}", icons::WAITING));
                        }
                        JobState::Running(progress) => self.progress(ui, now, id, info, progress),
                        JobState::Done => {
                            ui.add(
                                egui::ProgressBar::new(1.0)
                                    .desired_width(ui.available_width())
                                    .text("done"),
                            );
                            ui.weak(format!("{} done", icons::DONE));
                        }
                        JobState::Cancelled => {
                            ui.weak("cancelled");
                        }
                        JobState::Failed(error) => {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                    }
                });
            });
    }

    /// Pause, resume and cancel for a running or waiting job; the close
    /// button for a finished one.
    fn controls(
        &mut self,
        ui: &mut egui::Ui,
        now: Instant,
        id: JobId,
        info: &JobInfo,
        requests: &mut Vec<Request>,
    ) {
        match &info.state {
            JobState::Running(_) | JobState::Waiting(_) => {
                if ui
                    .small_button(icons::CLOSE)
                    .on_hover_text("Cancel")
                    .clicked()
                {
                    requests.push(Request::Command(Command::CancelJob(id)));
                }
                if info.pausable && matches!(info.state, JobState::Running(_)) {
                    if info.paused {
                        if ui
                            .small_button(icons::PLAY)
                            .on_hover_text("Resume")
                            .clicked()
                        {
                            requests.push(Request::Command(Command::ResumeJob(id)));
                        }
                    } else if ui
                        .small_button(icons::PAUSE)
                        .on_hover_text("Pause")
                        .clicked()
                    {
                        requests.push(Request::Command(Command::PauseJob(id)));
                    }
                }
            }
            JobState::Done | JobState::Cancelled => {
                let at = *self.done_at.entry(id).or_insert(now);
                if now.duration_since(at) >= DONE_FOR
                    || ui
                        .small_button(icons::CLOSE)
                        .on_hover_text("Dismiss")
                        .clicked()
                {
                    requests.push(Request::Command(Command::DismissJob(id)));
                }
            }
            JobState::Failed(_) => {
                if ui
                    .small_button(icons::CLOSE)
                    .on_hover_text("Dismiss")
                    .clicked()
                {
                    requests.push(Request::Command(Command::DismissJob(id)));
                }
            }
        }
    }

    /// The bar with its fraction and what is done of the total, and the
    /// throughput and remaining time under it.
    fn progress(
        &mut self,
        ui: &mut egui::Ui,
        now: Instant,
        id: JobId,
        info: &JobInfo,
        progress: &Progress,
    ) {
        let (done, total) = counts(progress).unwrap_or((0, None));
        let fraction = total.map_or(0.0, |t| done as f32 / t.max(1) as f32);
        let bar = egui::ProgressBar::new(fraction)
            .desired_width(ui.available_width())
            .text(progress_text(progress));
        ui.add(if info.paused { bar } else { bar.animate(true) });
        if info.paused {
            ui.weak("paused");
            return;
        }
        if matches!(progress, Progress::Message(_)) {
            return;
        }
        let rate = self.rates.entry(id).or_default();
        rate.push(now, done);
        if let Some(per_second) = rate.per_second() {
            let mut line = throughput(progress, per_second);
            if let Some(total) = total
                && total > done
            {
                let left = (total - done) as f64 / per_second;
                line.push_str(" · ");
                line.push_str(&remaining(left));
            }
            ui.weak(line);
        }
    }
}

/// The header: how many jobs run, how many wait, and the overall bar when
/// more than one job runs.
fn header(ui: &mut egui::Ui, model: &Model) {
    let running = model
        .jobs
        .values()
        .filter(|j| matches!(j.state, JobState::Running(_)))
        .count();
    let waiting = model
        .jobs
        .values()
        .filter(|j| matches!(j.state, JobState::Waiting(_)))
        .count();
    ui.horizontal(|ui| {
        ui.set_min_height(HEADER_HEIGHT - ui.spacing().item_spacing.y);
        let mut text = match running {
            0 => "No job running".to_owned(),
            1 => "1 job running".to_owned(),
            n => format!("{n} jobs running"),
        };
        if waiting > 0 {
            let _ = write!(text, " · {waiting} waiting");
        }
        ui.strong(text);
        if running > 1
            && let Some(fraction) = overall(model)
        {
            ui.add(
                egui::ProgressBar::new(fraction)
                    .desired_width(ui.available_width().max(40.0))
                    .text(format!("{}%", (fraction * 100.0).round() as u32)),
            );
        }
    });
    ui.separator();
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
        let two = Operations::height(&model);
        for i in 0..3 {
            model.job_started(JobId(i), format!("job {i}"), false);
        }
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(
            close(Operations::height(&model), two + ROW_HEIGHT),
            "three rows"
        );
        for i in 3..20 {
            model.job_started(JobId(i), format!("job {i}"), false);
        }
        assert!(
            close(
                Operations::height(&model),
                two + (ROWS_CAP - ROWS_MIN) as f32 * ROW_HEIGHT
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
    }
}
