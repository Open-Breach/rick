//! The Hash dialogue (SPEC.md *Hash dialogue*): 620 px wide in the
//! dialogue frame. The header has the `HASH` glyph, *File hashes*, the
//! file's name and size and the close button. The body starts with the
//! *VERIFY* field — prefilled from the clipboard when it holds something
//! that looks like a hash, with a *from clipboard* hint until edited, and
//! compared live against every row of its length: a match gives it a
//! `status/success` border and a *Matches SHA-256* pill and tints the row,
//! anything else that is not a match a `status/danger` border and *No
//! match* — then a card per file with one row per algorithm (the label,
//! the value in monospace, a copy icon), then the *Format* segmented
//! control (*hex*, *HEX*, *Base64*), remembered through
//! [`Command::HashFormat`]. The footer has *Copy all* and *Done*. While
//! the job runs the card gives way to the portal burst with *Hashing…*,
//! the bytes done of the total with the throughput and time left and a
//! 4 px bar; *Copy all* is disabled and *Cancel* stops the job and closes
//! the dialogue at once.

use std::sync::Arc;
use std::time::Instant;

use app::hash::{self, Format};
use app::{App, Command};
use egui::{Align, Align2, FontId, Rect, Sense, pos2, vec2};
use ext::t;
use ext::{Algorithm, Digests, Hashes, Header, HeaderIcon, Level};
use model::JobState;
use types::{JobId, Progress};

use crate::operations::{self, Rate};
use crate::shell::Request;
use crate::theme::{self, CARD_RADIUS, ICON, TEXT_BODY, TEXT_SECONDARY};
use crate::{dialog, format, icons, portal, widgets};

/// The dialogue's width.
const WIDTH: f32 = 620.0;
/// A hash row's height.
const ROW: f32 = 44.0;
/// The label column of a hash row.
const LABEL_COLUMN: f32 = 72.0;
/// The inset of a row's label from the card's edge.
const ROW_INSET: f32 = 14.0;
/// The hash values' text size: 64 monospace characters fit the row.
const VALUE_SIZE: f32 = 11.0;
/// The copy icon's hit area.
const COPY: f32 = 28.0;
/// The room the portal burst and its progress line take while hashing.
const BUSY_HEIGHT: f32 = 196.0;
/// The size of the *Format* segmented control.
const SEGMENTED: egui::Vec2 = vec2(150.0, 28.0);
/// A file's naming row over its card, with several files.
const FILE_ROW: f32 = 28.0;

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
    /// The Verify text came from the clipboard and was not edited yet.
    from_clipboard: bool,
    /// The clipboard is still to be read, on the first frame.
    fresh: bool,
    /// The job's progress samples, for the throughput and time left.
    rate: Rate,
}

impl std::fmt::Debug for HashDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HashDialog")
            .field("open", &self.open.as_ref().map(|o| &o.hashes))
            .finish()
    }
}

/// The segmented control's text for a format.
fn format_label(format: Format) -> String {
    match format {
        Format::Hex => t!("hash-format-hex"),
        Format::HexUpper => t!("hash-format-hex-upper"),
        Format::Base64 => t!("hash-format-base64"),
    }
}

/// The bytes a pasted value stands for: hex in either case first, then
/// Base64, as a download page writes either.
fn decode(text: &str) -> Option<Vec<u8>> {
    hash::parse_verify(text, Format::Hex).or_else(|| hash::parse_verify(text, Format::Base64))
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
    fn parse(text: &str) -> Self {
        if text.trim().is_empty() {
            return Self::Nothing;
        }
        match decode(text) {
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

/// Whether `text` looks like a hash: hex or Base64 of a digest's length.
fn looks_like_hash(text: &str) -> bool {
    matches!(Expected::parse(text), Expected::Bytes(..))
}

/// What the Verify field says over every file with digests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// Nothing typed, or no file has its digests yet.
    Pending,
    /// A row of this algorithm matches.
    Matches(Algorithm),
    /// No row matches.
    NoMatch,
}

fn verdict(hashes: &Hashes, expected: &Expected) -> Verdict {
    let results: Vec<Digests> = (0..hashes.names().len())
        .filter_map(|index| hashes.result(index).and_then(Result::ok))
        .collect();
    if matches!(expected, Expected::Nothing) || results.is_empty() {
        return Verdict::Pending;
    }
    let Expected::Bytes(bytes, algorithms) = expected else {
        return Verdict::NoMatch;
    };
    for digests in &results {
        for &algorithm in *algorithms {
            if digests.get(algorithm) == bytes.as_slice() {
                return Verdict::Matches(algorithm);
            }
        }
    }
    Verdict::NoMatch
}

/// What the frame's widgets asked for, applied after the modal closed
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
            from_clipboard: false,
            fresh: true,
            rate: Rate::default(),
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

    /// The format the control stands at, while open.
    #[must_use]
    pub fn format(&self) -> Option<Format> {
        self.open.as_ref().map(|o| o.format)
    }

    /// Draws the dialogue, when open.
    pub fn show(&mut self, ctx: &egui::Context, app: &App, requests: &mut Vec<Request>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if std::mem::take(&mut open.fresh)
            && let Ok(Some(text)) = app.platform().text()
            && looks_like_hash(&text)
        {
            text.trim().clone_into(&mut open.verify);
            open.from_clipboard = true;
        }
        let hashes = Arc::clone(&open.hashes);
        let before = open.format;
        let job = hashes.job().and_then(|id| {
            app.model()
                .jobs
                .get(&id)
                .map(|info| (id, info.state.clone()))
        });
        let running = matches!(job, Some((_, JobState::Running(_)))) && !hashes.is_complete();
        if let Some((
            _,
            JobState::Running(Progress::Bytes { done, .. } | Progress::Items { done, .. }),
        )) = &job
        {
            open.rate.push(Instant::now(), *done);
        }
        let still = app.platform().reduce_motion();
        let expected = Expected::parse(&open.verify);
        let verdict = verdict(&hashes, &expected);
        let mut asked = Asked::default();
        let modal = dialog::modal(ctx, egui::Id::new("hash"), WIDTH, |ui| {
            let header = Header::new(HeaderIcon::Glyph(icons::HASH)).subtitle(subtitle(&hashes));
            if dialog::header(ui, &t!("hash-title"), Some(&header), true) {
                asked.close = true;
            }
            dialog::body(ui, |ui| {
                verify_field(ui, open, verdict);
                ui.add_space(8.0);
                blocks(
                    ui,
                    &hashes,
                    open,
                    &expected,
                    job.as_ref(),
                    still,
                    &mut asked,
                );
                ui.add_space(4.0);
                format_row(ui, open);
            });
            dialog::footer(
                ui,
                |left| {
                    let copy = widgets::icon_button(
                        left,
                        Some(icons::COPY),
                        &t!("hash-copy-all"),
                        false,
                        !running,
                    );
                    if copy.clicked() && !running {
                        asked.copy = Some(copy_everything(&hashes, open.format));
                    }
                },
                |right| {
                    if widgets::button(right, &t!("hash-done"), true, true).clicked() {
                        asked.close = true;
                    }
                    if running
                        && let Some((id, _)) = &job
                        && widgets::button(right, &t!("cancel"), false, true).clicked()
                    {
                        asked.cancel = Some(*id);
                        asked.close = true;
                    }
                },
            );
        });
        if modal.should_close() {
            asked.close = true;
        }
        if running {
            ctx.request_repaint();
        }
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
        if asked.close {
            self.open = None;
        }
    }
}

/// The header's secondary line: the file's name and size, or the count of
/// files and their total.
fn subtitle(hashes: &Hashes) -> String {
    let names = hashes.names();
    if names.len() == 1 {
        let name = names.first().cloned().unwrap_or_default();
        match hashes.size(0) {
            Some(size) => format!("{name} · {}", format::size(Some(size))),
            None => name,
        }
    } else {
        let files = t!("hash-files", count = names.len());
        match hashes.total_size() {
            Some(total) => format!("{files} · {}", format::size(Some(total))),
            None => files,
        }
    }
}

/// The *VERIFY* heading and field, with the verdict's border and pill and
/// the *from clipboard* hint inside its right end.
fn verify_field(ui: &mut egui::Ui, open: &mut Open, verdict: Verdict) {
    let scheme = theme::scheme(ui);
    widgets::caps_heading(ui, &t!("hash-verify"));
    let border = match verdict {
        Verdict::Matches(_) => Some(scheme.status_success),
        Verdict::NoMatch => Some(scheme.status_danger),
        Verdict::Pending => None,
    };
    let from_clipboard = open.from_clipboard;
    let response = dialog::text_field_with(
        ui,
        &mut open.verify,
        &t!("hash-verify-hint"),
        true,
        border,
        false,
        |tail| {
            match verdict {
                Verdict::Matches(algorithm) => verdict_pill(
                    tail,
                    Some(icons::DONE),
                    &t!("hash-matches", algorithm = algorithm.label()),
                    scheme.status_success_bg,
                    scheme.status_success,
                ),
                Verdict::NoMatch => verdict_pill(
                    tail,
                    None,
                    &t!("hash-no-match"),
                    scheme.status_danger.gamma_multiply(0.15),
                    scheme.status_danger,
                ),
                Verdict::Pending => {}
            }
            if from_clipboard {
                tail.add(
                    egui::Label::new(
                        egui::RichText::new(t!("hash-from-clipboard"))
                            .size(theme::TEXT_CAPS)
                            .color(scheme.text_secondary),
                    )
                    .selectable(false),
                );
            }
        },
    );
    if response.changed() {
        open.from_clipboard = false;
    }
}

/// The verdict pill: an optional glyph and the text in 11 px on a rounded
/// tint, 22 px tall; read aloud as its text.
fn verdict_pill(
    ui: &mut egui::Ui,
    glyph: Option<&str>,
    text: &str,
    bg: egui::Color32,
    fg: egui::Color32,
) {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), FontId::proportional(theme::TEXT_CAPS), fg);
    let glyph_room = if glyph.is_some() { 12.0 + 4.0 } else { 0.0 };
    let size = vec2(galley.size().x + glyph_room + 16.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 11.0, bg);
    if let Some(glyph) = glyph {
        ui.painter().text(
            pos2(rect.left() + 8.0 + 6.0, rect.center().y),
            Align2::CENTER_CENTER,
            glyph,
            FontId::proportional(12.0),
            fg,
        );
    }
    ui.painter().galley(
        pos2(
            rect.left() + 8.0 + glyph_room,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        fg,
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text.to_owned()));
}

/// One block per file: its card once its digests are there, the portal
/// burst with the progress while the job reads it, or why there are none.
fn blocks(
    ui: &mut egui::Ui,
    hashes: &Hashes,
    open: &Open,
    expected: &Expected,
    job: Option<&(JobId, JobState)>,
    still: bool,
    asked: &mut Asked,
) {
    let scheme = theme::scheme(ui);
    let several = hashes.names().len() > 1;
    let mut burst_shown = false;
    for (index, name) in hashes.names().iter().enumerate() {
        if index > 0 {
            ui.add_space(8.0);
        }
        if several {
            file_row(ui, name, hashes.size(index));
        }
        match hashes.result(index) {
            Some(Ok(digests)) => block(ui, index, &digests, open.format, expected, asked),
            Some(Err(error)) => note_card(
                ui,
                &t!("hash-failed", error = error.as_str()),
                scheme.status_danger,
            ),
            None => match job {
                Some((_, JobState::Cancelled)) => {
                    note_card(ui, &t!("hash-cancelled"), scheme.text_secondary);
                }
                Some((_, JobState::Failed(error))) => note_card(
                    ui,
                    &t!("hash-failed", error = error.as_str()),
                    scheme.status_danger,
                ),
                _ if burst_shown => {
                    note_card(ui, &t!("hash-hashing"), scheme.text_secondary);
                }
                Some((_, JobState::Running(progress))) => {
                    busy(ui, Some(progress), open.rate.per_second(), still);
                    burst_shown = true;
                }
                _ => {
                    busy(ui, None, None, still);
                    burst_shown = true;
                }
            },
        }
    }
}

/// A row naming one of several files: the `FILE` glyph, the name and its
/// size.
fn file_row(ui: &mut egui::Ui, name: &str, size: Option<u64>) {
    let scheme = theme::scheme(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), FILE_ROW), Sense::hover());
    ui.painter().text(
        pos2(rect.left() + ICON / 2.0, rect.center().y),
        Align2::CENTER_CENTER,
        icons::FILE,
        FontId::proportional(ICON),
        scheme.icon_default,
    );
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(rect.left() + ICON + 10.0, rect.top()),
                rect.max,
            ))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 8.0;
    row.add(
        egui::Label::new(
            egui::RichText::new(name)
                .size(TEXT_BODY)
                .color(scheme.text_primary),
        )
        .truncate()
        .selectable(false),
    );
    if let Some(size) = size {
        row.add(
            egui::Label::new(
                egui::RichText::new(format::size(Some(size)))
                    .size(TEXT_SECONDARY)
                    .color(scheme.text_secondary),
            )
            .selectable(false),
        );
    }
}

/// A card with one line of text in `colour`, where a file's rows would be.
fn note_card(ui: &mut egui::Ui, text: &str, colour: egui::Color32) {
    dialog::card_frame(ui, |ui| {
        ui.add(
            egui::Label::new(egui::RichText::new(text).size(TEXT_BODY).color(colour))
                .wrap()
                .selectable(false),
        );
    });
}

/// One file's card: a row per algorithm with the label, the value in
/// monospace and the copy icon; the row that matches the Verify text
/// tinted `status/success-row` with its label in `status/success`.
#[allow(clippy::too_many_lines)]
fn block(
    ui: &mut egui::Ui,
    index: usize,
    digests: &Digests,
    format: Format,
    expected: &Expected,
    asked: &mut Asked,
) {
    let scheme = theme::scheme(ui);
    let rows = Algorithm::ALL.len();
    #[allow(clippy::cast_precision_loss)]
    let (card, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), ROW * rows as f32),
        Sense::hover(),
    );
    ui.painter().rect(
        card,
        CARD_RADIUS,
        scheme.bg_card,
        egui::Stroke::new(1.0, scheme.border_default),
        egui::StrokeKind::Inside,
    );
    for (i, algorithm) in Algorithm::ALL.into_iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let row = Rect::from_min_size(
            pos2(card.left(), card.top() + ROW * i as f32),
            vec2(card.width(), ROW),
        );
        let value = hash::render(digests.get(algorithm), format);
        let matches = expected.verdict(algorithm, digests.get(algorithm)) == Some(true);
        if matches {
            let radius = if i == 0 {
                egui::CornerRadius {
                    nw: CARD_RADIUS,
                    ne: CARD_RADIUS,
                    sw: 0,
                    se: 0,
                }
            } else if i + 1 == rows {
                egui::CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: CARD_RADIUS,
                    se: CARD_RADIUS,
                }
            } else {
                egui::CornerRadius::ZERO
            };
            ui.painter()
                .rect_filled(row.shrink(1.0), radius, scheme.status_success_row);
        }
        if i > 0 {
            ui.painter().hline(
                (row.left() + 1.0)..=(row.right() - 1.0),
                row.top() + 0.5,
                scheme.hairline(),
            );
        }
        let label_colour = if matches {
            scheme.status_success
        } else {
            scheme.text_primary
        };
        let mut label = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(row.left() + ROW_INSET, row.top()),
                    pos2(row.left() + ROW_INSET + LABEL_COLUMN, row.bottom()),
                ))
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        label.add(
            egui::Label::new(
                egui::RichText::new(algorithm.label())
                    .size(TEXT_BODY)
                    .color(label_colour),
            )
            .selectable(false),
        );
        let copy_rect = Rect::from_center_size(
            pos2(row.right() - 8.0 - COPY / 2.0, row.center().y),
            vec2(COPY, COPY),
        );
        let mut value_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(row.left() + ROW_INSET + LABEL_COLUMN, row.top()),
                    pos2(copy_rect.left() - 6.0, row.bottom()),
                ))
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        value_ui.add(
            egui::Label::new(
                egui::RichText::new(value.as_str())
                    .font(FontId::monospace(VALUE_SIZE))
                    .color(scheme.text_primary),
            )
            .truncate(),
        );
        let copy = ui.interact(
            copy_rect,
            ui.id().with(("hash-copy", index, i)),
            Sense::click(),
        );
        if copy.hovered() {
            ui.painter()
                .circle_filled(copy_rect.center(), COPY / 2.0, scheme.bg_subtle);
        }
        ui.painter().text(
            copy_rect.center(),
            Align2::CENTER_CENTER,
            icons::COPY,
            FontId::proportional(ICON),
            scheme.icon_default,
        );
        let spoken = t!("hash-copy", algorithm = algorithm.label());
        copy.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spoken.clone())
        });
        if copy.clicked() {
            asked.copy = Some(value);
        }
    }
}

/// While the job reads the file: the portal burst centred where the card
/// will be, the progress line under it — *Hashing…* then the bytes done of
/// the total, the throughput and the time left — and a 4 px bar.
fn busy(ui: &mut egui::Ui, progress: Option<&Progress>, rate: Option<f64>, still: bool) {
    let scheme = theme::scheme(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BUSY_HEIGHT), Sense::hover());
    let time = ui.input(|i| i.time);
    portal::burst(
        ui.painter(),
        pos2(rect.center().x, rect.top() + 72.0),
        time,
        scheme.accent_focus,
        scheme.icon_default,
        still,
    );
    let line_y = rect.bottom() - 28.0;
    let head = ui.painter().layout_no_wrap(
        t!("hash-hashing"),
        FontId::proportional(TEXT_BODY),
        scheme.text_primary,
    );
    let head_width = head.size().x;
    ui.painter().galley(
        pos2(rect.left(), line_y - head.size().y / 2.0),
        head,
        scheme.text_primary,
    );
    let mut spoken = t!("hash-hashing");
    if let Some(progress) = progress {
        let detail = detail_text(progress, rate);
        if !detail.is_empty() {
            ui.painter().text(
                pos2(rect.left() + head_width + 10.0, line_y),
                Align2::LEFT_CENTER,
                &detail,
                FontId::proportional(TEXT_SECONDARY),
                scheme.text_secondary,
            );
            spoken = format!("{spoken} {detail}");
        }
    }
    let line = Rect::from_min_max(
        pos2(rect.left(), line_y - 10.0),
        pos2(rect.right(), line_y + 10.0),
    );
    ui.interact(line, ui.id().with("hash-progress"), Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, spoken.clone()));
    let bar = Rect::from_min_max(
        pos2(rect.left(), rect.bottom() - 10.0),
        pos2(rect.right(), rect.bottom() - 6.0),
    );
    ui.painter().rect_filled(bar, 2.0, scheme.bg_track);
    if let Some(fraction) = progress.and_then(fraction) {
        ui.painter().rect_filled(
            Rect::from_min_max(
                bar.min,
                pos2(bar.left() + bar.width() * fraction, bar.bottom()),
            ),
            2.0,
            scheme.accent_focus,
        );
    }
}

/// `64.0 MiB of 343 MiB · 210 MiB/s · 1 s left`, as much of it as is
/// known.
fn detail_text(progress: &Progress, rate: Option<f64>) -> String {
    use std::fmt::Write as _;
    let mut text = operations::progress_text(progress);
    if let Some(rate) = rate
        && rate > 0.0
    {
        let _ = write!(text, " · {}", operations::throughput(progress, rate));
        if let Progress::Bytes {
            done,
            total: Some(total),
        }
        | Progress::Items {
            done,
            total: Some(total),
        } = progress
            && total > done
        {
            #[allow(clippy::cast_precision_loss)]
            let left = (total - done) as f64 / rate;
            let _ = write!(text, " · {}", operations::remaining(left));
        }
    }
    text
}

/// The job's fraction done, with a known total.
fn fraction(progress: &Progress) -> Option<f32> {
    match progress {
        Progress::Bytes {
            done,
            total: Some(total),
        }
        | Progress::Items {
            done,
            total: Some(total),
        } => {
            #[allow(clippy::cast_precision_loss)]
            let fraction = *done as f32 / (*total).max(1) as f32;
            Some(fraction.clamp(0.0, 1.0))
        }
        _ => None,
    }
}

/// The *Format* row: the label and the segmented control.
fn format_row(ui: &mut egui::Ui, open: &mut Open) {
    let scheme = theme::scheme(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), SEGMENTED.y), Sense::hover());
    let mut label = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                rect.min,
                pos2(rect.left() + LABEL_COLUMN, rect.bottom()),
            ))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    label.add(
        egui::Label::new(
            egui::RichText::new(t!("hash-format"))
                .size(TEXT_BODY)
                .color(scheme.text_primary),
        )
        .selectable(false),
    );
    let control = Rect::from_min_size(pos2(rect.left() + LABEL_COLUMN, rect.top()), SEGMENTED);
    let options: Vec<String> = Format::ALL.into_iter().map(format_label).collect();
    let selected = Format::ALL
        .iter()
        .position(|f| *f == open.format)
        .unwrap_or(0);
    if let Some(index) = widgets::segmented(
        ui,
        control,
        egui::Id::new("hash-format"),
        &options,
        selected,
    ) && let Some(format) = Format::ALL.get(index)
    {
        open.format = *format;
    }
}

/// The text *Copy all* puts on the clipboard: every file's rows, a blank
/// line between files.
fn copy_everything(hashes: &Hashes, format: Format) -> String {
    hashes
        .names()
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            hashes
                .result(index)
                .and_then(Result::ok)
                .map(|digests| hash::copy_all(name, &digests, format))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verify_text_is_read_once_per_frame() {
        let digests = hash::digests_of(b"abc");
        assert!(matches!(Expected::parse("  "), Expected::Nothing));
        assert!(matches!(Expected::parse("zz"), Expected::Unrecognised));
        assert!(matches!(
            Expected::parse(&"0".repeat(10)),
            Expected::Unrecognised
        ));
        let expected = Expected::parse(&hash::render(&digests.sha256, Format::Hex));
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

    #[test]
    fn a_clipboard_value_is_a_hash_in_hex_or_base64_of_a_digest_length() {
        let digests = hash::digests_of(b"abc");
        assert!(looks_like_hash(&hash::render(&digests.md5, Format::Hex)));
        assert!(looks_like_hash(&hash::render(
            &digests.sha1,
            Format::HexUpper
        )));
        assert!(looks_like_hash(&hash::render(
            &digests.blake3,
            Format::Base64
        )));
        assert!(looks_like_hash(&format!(
            "  {}\n",
            hash::render(&digests.sha256, Format::Hex)
        )));
        assert!(!looks_like_hash("hello"));
        assert!(!looks_like_hash(""));
        assert!(!looks_like_hash(&"a".repeat(48)));
        // Base64 of 48 bytes: a digest's length it is not.
        assert!(!looks_like_hash(&hash::render(&[7u8; 48], Format::Base64)));
    }

    #[test]
    fn the_verdict_is_the_first_matching_row_over_every_file() {
        let a = hash::digests_of(b"a");
        let b = hash::digests_of(b"b");
        let hashes = Hashes::pending(vec!["a".to_owned(), "b".to_owned()]);
        let expected = Expected::parse(&hash::render(&b.sha256, Format::Hex));
        assert_eq!(verdict(&hashes, &expected), Verdict::Pending);
        hashes.finish(0, Ok(a));
        assert_eq!(verdict(&hashes, &expected), Verdict::NoMatch);
        hashes.finish(1, Ok(b));
        assert_eq!(
            verdict(&hashes, &expected),
            Verdict::Matches(Algorithm::Sha256)
        );
        assert_eq!(
            verdict(&hashes, &Expected::parse("not a hash")),
            Verdict::NoMatch
        );
        assert_eq!(verdict(&hashes, &Expected::parse("")), Verdict::Pending);
    }

    #[test]
    fn the_detail_line_grows_with_what_is_known() {
        let progress = Progress::Bytes {
            done: 64 << 20,
            total: Some(343 << 20),
        };
        assert_eq!(detail_text(&progress, None), "64.0 MiB of 343 MiB");
        let line = detail_text(&progress, Some(210.0 * 1024.0 * 1024.0));
        assert_eq!(line, "64.0 MiB of 343 MiB · 210 MiB/s · 1 s left");
        assert!((fraction(&progress).unwrap_or(0.0) - 0.1866).abs() < 0.001);
    }
}
