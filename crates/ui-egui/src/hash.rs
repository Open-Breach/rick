//! The Hash dialogue: MD5, SHA-1, SHA-256 and BLAKE3 of the selected
//! files, as the job the Compute hash action started computes them. While
//! the job runs the dialogue shows its byte progress and a Cancel button;
//! each file's block fills in when its digests arrive — one row per
//! algorithm with the value in a read-only monospace field and a Copy
//! button, then Copy all — and under the blocks sit the format toggle
//! (lower-case hex, upper-case hex, Base64; remembered through
//! [`Command::HashFormat`]) and the Verify field, whose text is compared
//! live against every row of its length: green *Match*, red *Mismatch*,
//! or *Unrecognised length* under the field.

use std::sync::Arc;
use std::time::Duration;

use app::hash::{self, Format};
use app::{App, Command};
use egui::Color32;
use ext::t;
use ext::{Algorithm, Hashes, Level};
use model::JobState;
use types::{JobId, Progress};

use crate::format;
use crate::shell::Request;

/// The dialogue's state between frames.
#[derive(Default)]
pub struct HashDialog {
    open: Option<Open>,
}

/// What the open dialogue shows.
struct Open {
    hashes: Arc<Hashes>,
    format: Format,
    /// The Verify field's text.
    verify: String,
}

impl std::fmt::Debug for HashDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HashDialog")
            .field("open", &self.open.as_ref().map(|o| &o.hashes))
            .finish()
    }
}

/// The green of a matching row: egui's visuals name an error and a
/// warning colour but no success one, so the dialogue picks one that
/// reads on each theme.
fn success_color(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode {
        Color32::from_rgb(0x5c, 0xd0, 0x6a)
    } else {
        Color32::from_rgb(0x1e, 0x8a, 0x32)
    }
}

/// The toggle's text for a format.
fn format_label(format: Format) -> String {
    match format {
        Format::Hex => t!("hash-format-hex"),
        Format::HexUpper => t!("hash-format-hex-upper"),
        Format::Base64 => t!("hash-format-base64"),
    }
}

/// What the Verify field's text decodes to, parsed once per frame: nothing
/// typed, a value that does not decode or fits no algorithm, or the bytes
/// and the algorithms of their length.
enum Expected {
    Nothing,
    Unrecognised,
    Bytes(Vec<u8>, &'static [Algorithm]),
}

impl Expected {
    fn parse(text: &str, format: Format) -> Self {
        if text.trim().is_empty() {
            return Self::Nothing;
        }
        match hash::parse_verify(text, format) {
            Some(bytes) => match hash::detect(bytes.len()) {
                [] => Self::Unrecognised,
                algorithms => Self::Bytes(bytes, algorithms),
            },
            None => Self::Unrecognised,
        }
    }

    /// Whether `digest` is the pasted value, when its algorithm has the
    /// pasted length.
    fn verdict(&self, algorithm: Algorithm, digest: &[u8]) -> Option<bool> {
        match self {
            Self::Bytes(bytes, algorithms) if algorithms.contains(&algorithm) => {
                Some(bytes.as_slice() == digest)
            }
            _ => None,
        }
    }
}

/// What the frame's widgets asked for, applied after the window closed
/// its borrows.
#[derive(Default)]
struct Asked {
    copy: Option<String>,
    cancel: Option<JobId>,
    close: bool,
}

impl HashDialog {
    /// Shows the dialogue on `hashes`, with the remembered `format`.
    pub fn open(&mut self, hashes: Arc<Hashes>, format: Format) {
        self.open = Some(Open {
            hashes,
            format,
            verify: String::new(),
        });
    }

    /// Whether the dialogue is on screen.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The slot shown, if any.
    #[must_use]
    pub fn hashes(&self) -> Option<&Arc<Hashes>> {
        self.open.as_ref().map(|o| &o.hashes)
    }

    /// The format the toggle stands at, while open.
    #[must_use]
    pub fn format(&self) -> Option<Format> {
        self.open.as_ref().map(|o| o.format)
    }

    /// Draws the dialogue, when open.
    pub fn show(&mut self, ctx: &egui::Context, app: &App, requests: &mut Vec<Request>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let hashes = Arc::clone(&open.hashes);
        let first = hashes.names().first().cloned().unwrap_or_default();
        let before = open.format;
        let job = hashes.job().and_then(|id| {
            app.model()
                .jobs
                .get(&id)
                .map(|info| (id, info.state.clone()))
        });
        let mut asked = Asked::default();
        let mut keep = true;
        egui::Window::new(t!("hash-title", name = first.as_str()))
            .id(egui::Id::new("hash"))
            .open(&mut keep)
            .default_width(620.0)
            .resizable(true)
            .show(ctx, |ui| {
                let expected = Expected::parse(&open.verify, open.format);
                blocks(
                    ui,
                    &hashes,
                    open.format,
                    &expected,
                    job.as_ref(),
                    &mut asked,
                );
                controls(ui, open, &expected, &mut asked);
            });
        if open.format != before {
            requests.push(Request::Command(Command::HashFormat(open.format)));
        }
        if let Some(text) = asked.copy
            && let Err(error) = app.platform().set_text(&text)
        {
            requests.push(Request::Toast(Level::Error, error.to_string()));
        }
        if let Some(id) = asked.cancel {
            requests.push(Request::Command(Command::CancelJob(id)));
        }
        if !keep || asked.close {
            self.open = None;
        }
    }
}

/// One block per file — its rows once its digests are there, else what
/// the job is up to — and, while the job runs, its progress bar and
/// Cancel.
fn blocks(
    ui: &mut egui::Ui,
    hashes: &Hashes,
    format: Format,
    expected: &Expected,
    job: Option<&(JobId, JobState)>,
    asked: &mut Asked,
) {
    let several = hashes.names().len() > 1;
    for (index, name) in hashes.names().iter().enumerate() {
        if index > 0 {
            ui.add_space(8.0);
        }
        if several {
            ui.strong(name);
        }
        match hashes.result(index) {
            Some(Ok(digests)) => block(ui, index, name, &digests, format, expected, asked),
            Some(Err(error)) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    t!("hash-failed", error = error.as_str()),
                );
            }
            None => match job {
                Some((_, JobState::Cancelled)) => {
                    ui.weak(t!("hash-cancelled"));
                }
                Some((_, JobState::Failed(error))) => {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        t!("hash-failed", error = error.as_str()),
                    );
                }
                _ => {
                    ui.weak(t!("hash-hashing"));
                }
            },
        }
    }
    if let Some((id, JobState::Running(progress))) = job
        && !hashes.is_complete()
    {
        ui.add_space(8.0);
        progress_bar(ui, progress);
        if ui.button(t!("cancel")).clicked() {
            asked.cancel = Some(*id);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
}

/// Under the blocks: the format toggle, the Verify field with its verdict
/// on an unrecognised value, and Close (Escape closes too).
fn controls(ui: &mut egui::Ui, open: &mut Open, expected: &Expected, asked: &mut Asked) {
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(t!("hash-format"));
        egui::ComboBox::from_id_salt("hash-format")
            .selected_text(format_label(open.format))
            .show_ui(ui, |ui| {
                for format in Format::ALL {
                    ui.selectable_value(&mut open.format, format, format_label(format));
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label(t!("hash-verify"));
        ui.add(
            egui::TextEdit::singleline(&mut open.verify)
                .font(egui::TextStyle::Monospace)
                .desired_width(value_width(ui))
                .hint_text(t!("hash-verify-hint")),
        );
    });
    if matches!(expected, Expected::Unrecognised) {
        ui.colored_label(ui.visuals().warn_fg_color, t!("hash-unrecognised-length"));
    }
    ui.add_space(4.0);
    if ui.button(t!("close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        asked.close = true;
    }
}

/// The width of a value field: 64 monospace characters, the hex of a
/// 32-byte digest.
fn value_width(ui: &egui::Ui) -> f32 {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    ui.fonts_mut(|f| f.glyph_width(&font, '0')) * 64.0 + 12.0
}

/// One file's block: a row per algorithm, then Copy all.
fn block(
    ui: &mut egui::Ui,
    index: usize,
    name: &str,
    digests: &ext::Digests,
    format: Format,
    expected: &Expected,
    asked: &mut Asked,
) {
    let width = value_width(ui);
    let success = success_color(ui.visuals());
    let error = ui.visuals().error_fg_color;
    egui::Grid::new(("hash-rows", index))
        .num_columns(4)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for algorithm in Algorithm::ALL {
                let value = hash::render(digests.get(algorithm), format);
                let verdict = expected.verdict(algorithm, digests.get(algorithm));
                ui.weak(algorithm.label());
                // An immutable buffer: the field selects and copies, never
                // edits.
                let mut shown = value.as_str();
                let mut field = egui::TextEdit::singleline(&mut shown)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(width);
                match verdict {
                    Some(true) => field = field.text_color(success),
                    Some(false) => field = field.text_color(error),
                    None => {}
                }
                ui.add(field);
                if ui.button(t!("hash-copy")).clicked() {
                    asked.copy = Some(value.clone());
                }
                match verdict {
                    Some(true) => {
                        ui.colored_label(success, t!("hash-match"));
                    }
                    Some(false) => {
                        ui.colored_label(error, t!("hash-mismatch"));
                    }
                    None => {
                        ui.label("");
                    }
                }
                ui.end_row();
            }
        });
    if ui.button(t!("hash-copy-all")).clicked() {
        asked.copy = Some(hash::copy_all(name, digests, format));
    }
}

/// The job's bytes done of the selection's total.
fn progress_bar(ui: &mut egui::Ui, progress: &Progress) {
    let (done, total) = match progress {
        Progress::Bytes { done, total } | Progress::Items { done, total } => (*done, *total),
        Progress::Message(_) => (0, None),
    };
    let fraction = total.map_or(0.0, |t| done as f32 / t.max(1) as f32);
    ui.add(
        egui::ProgressBar::new(fraction)
            .desired_width(ui.available_width())
            .text(format!(
                "{} / {}",
                format::size(Some(done)),
                format::size(total)
            ))
            .animate(true),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verify_text_is_read_once_per_frame() {
        let digests = hash::digests_of(b"abc");
        assert!(matches!(
            Expected::parse("  ", Format::Hex),
            Expected::Nothing
        ));
        assert!(matches!(
            Expected::parse("zz", Format::Hex),
            Expected::Unrecognised
        ));
        assert!(matches!(
            Expected::parse(&"0".repeat(10), Format::Hex),
            Expected::Unrecognised
        ));
        let expected = Expected::parse(&hash::render(&digests.sha256, Format::Hex), Format::Hex);
        assert_eq!(
            expected.verdict(Algorithm::Sha256, &digests.sha256),
            Some(true)
        );
        assert_eq!(
            expected.verdict(Algorithm::Blake3, &digests.blake3),
            Some(false)
        );
        assert_eq!(expected.verdict(Algorithm::Md5, &digests.md5), None);
    }
}
