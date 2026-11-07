//! The operations area: one row per job of the job table with name,
//! progress bar, throughput and remaining time, plus pause, resume and
//! cancel. Finished jobs stay for a few seconds with their result; a failed
//! job stays until dismissed.

use std::collections::{HashMap, VecDeque};
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

impl Operations {
    /// Whether any finished row is still shown, so the frame keeps painting
    /// until it goes.
    #[must_use]
    pub fn has_finished_rows(&self) -> bool {
        !self.done_at.is_empty()
    }

    /// Draws one row per job in the table.
    pub fn show(&mut self, ui: &mut egui::Ui, model: &Model, requests: &mut Vec<Request>) {
        let now = Instant::now();
        let jobs: Vec<(JobId, JobInfo)> = model
            .jobs
            .iter()
            .map(|(id, info)| (*id, info.clone()))
            .collect();
        for (id, info) in &jobs {
            self.row(ui, now, *id, info, requests);
        }
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
            .inner_margin(egui::Margin::symmetric(6, 3))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(&info.name).truncate());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.controls(ui, now, id, info, requests);
                        });
                    });
                    match &info.state {
                        JobState::Running(progress) => self.progress(ui, now, id, info, progress),
                        JobState::Done => {
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

    /// Pause, resume and cancel for a running job; the close button for a
    /// finished one.
    fn controls(
        &mut self,
        ui: &mut egui::Ui,
        now: Instant,
        id: JobId,
        info: &JobInfo,
        requests: &mut Vec<Request>,
    ) {
        match &info.state {
            JobState::Running(_) => {
                if ui
                    .small_button(icons::CLOSE)
                    .on_hover_text("Cancel")
                    .clicked()
                {
                    requests.push(Request::Command(Command::CancelJob(id)));
                }
                if info.pausable {
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

    /// The bar with its fraction, and the throughput and remaining time
    /// under it.
    fn progress(
        &mut self,
        ui: &mut egui::Ui,
        now: Instant,
        id: JobId,
        info: &JobInfo,
        progress: &Progress,
    ) {
        let (done, total) = match progress {
            Progress::Items { done, total } | Progress::Bytes { done, total } => (*done, *total),
            Progress::Message(_) => (0, None),
        };
        let fraction = total.map_or(0.0, |t| done as f32 / t.max(1) as f32);
        let text = match progress {
            Progress::Items { done, total } => format!(
                "{done} of {}",
                total.map_or("?".to_owned(), |t| t.to_string())
            ),
            Progress::Bytes { done, total } => {
                format!("{} of {}", format::size(Some(*done)), format::size(*total))
            }
            Progress::Message(text) => text.clone(),
        };
        let bar = egui::ProgressBar::new(fraction)
            .desired_width(ui.available_width())
            .text(text);
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
}
