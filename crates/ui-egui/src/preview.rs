//! The preview area of the inspector: one slot per pane holding the opened
//! document, the texture it rendered to, and the render in flight.
//!
//! Rendering decodes pixels, so it never runs on the UI thread: the slot
//! hands its document to a blocking task on the runtime and takes it back
//! with the frame through a channel it polls each frame, showing the last
//! frame or a placeholder meanwhile. Textures are cached per pane and
//! replaced only when the page, the size or the theme changes.

use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, TextStyle, TextureHandle, TextureOptions};
use ext::{Frame, PreviewDoc, RenderRequest, TextBlock};
use model::PaneId;
use runtime::oneshot;
use types::{EntryId, Error};

type Rendered = (Box<dyn PreviewDoc>, Result<Frame, Error>);

/// Requests are rounded up to this many pixels so a slow resize does not
/// re-render on every frame.
const QUANTUM: f32 = 64.0;

/// A text frame prepared for drawing: the block plus its line ranges.
struct TextView {
    block: TextBlock,
    lines: Vec<Range<usize>>,
}

/// One pane's preview.
#[derive(Default)]
pub struct Slot {
    entry: Option<EntryId>,
    doc: Option<Box<dyn PreviewDoc>>,
    pending: Option<oneshot::Receiver<Rendered>>,
    rendered_for: Option<(usize, u32, u32, bool)>,
    texture: Option<TextureHandle>,
    texture_size: (u32, u32),
    text: Option<TextView>,
    /// The page shown, zero-based.
    pub page: usize,
    pages: Option<usize>,
    info: Vec<(String, String)>,
    error: Option<String>,
    /// Wrap long lines of text previews.
    pub wrap: bool,
    /// Show line numbers in text previews.
    pub numbers: bool,
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slot")
            .field("entry", &self.entry)
            .field("open", &self.doc.is_some())
            .field("pending", &self.pending.is_some())
            .field("page", &self.page)
            .field("pages", &self.pages)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl Slot {
    fn reset(&mut self, entry: Option<EntryId>) {
        let (wrap, numbers) = (self.wrap, self.numbers);
        *self = Self {
            entry,
            wrap,
            numbers,
            ..Self::default()
        };
    }

    fn set_doc(&mut self, doc: Box<dyn PreviewDoc>) {
        self.pages = doc.pages();
        self.info = doc.info();
        self.page = 0;
        self.rendered_for = None;
        self.error = None;
        self.texture = None;
        self.text = None;
        self.doc = Some(doc);
    }

    /// Whether a render is in flight.
    #[must_use]
    pub fn rendering(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether a frame has been shown for the current document.
    #[must_use]
    pub fn has_frame(&self) -> bool {
        self.texture.is_some() || self.text.is_some()
    }

    /// Takes a finished render back, if any.
    fn poll(&mut self, ctx: &egui::Context, pane: PaneId) {
        let Some(rx) = self.pending.as_mut() else {
            return;
        };
        match rx.try_recv() {
            Ok((doc, result)) => {
                self.pending = None;
                self.pages = doc.pages().or(self.pages);
                self.info = doc.info();
                self.doc = Some(doc);
                match result {
                    Ok(Frame::Rgba {
                        width,
                        height,
                        data,
                    }) => {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [width as usize, height as usize],
                            &data,
                        );
                        self.texture = Some(ctx.load_texture(
                            format!("preview-{}", pane.0),
                            image,
                            TextureOptions::LINEAR,
                        ));
                        self.texture_size = (width, height);
                        self.text = None;
                        self.error = None;
                    }
                    Ok(Frame::Text(block)) => {
                        let lines = line_ranges(&block.text);
                        self.text = Some(TextView { block, lines });
                        self.texture = None;
                        self.error = None;
                    }
                    Ok(Frame::External(_)) => {
                        self.error = Some("external textures are not supported".to_owned());
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            Err(oneshot::error::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
            Err(oneshot::error::TryRecvError::Closed) => {
                self.pending = None;
                self.error = Some("the render task ended without a frame".to_owned());
            }
        }
    }

    /// Starts a render for `req` on the runtime's blocking pool when the
    /// shown frame is not for this page, size or theme.
    fn request(&mut self, ctx: &egui::Context, req: RenderRequest) {
        let key = (self.page, req.width, req.height, req.dark);
        if self.pending.is_some() || self.rendered_for == Some(key) {
            return;
        }
        // Text lays itself out; one render per document and theme is enough.
        if self.text.is_some()
            && self
                .rendered_for
                .is_some_and(|(_, _, _, dark)| dark == req.dark)
        {
            self.rendered_for = Some(key);
            return;
        }
        let Some(mut doc) = self.doc.take() else {
            return;
        };
        let page = self.page;
        let (tx, rx) = oneshot::channel();
        runtime::handle().spawn_blocking(move || {
            let result = doc.render(page, req);
            let _ = tx.send((doc, result));
        });
        self.pending = Some(rx);
        self.rendered_for = Some(key);
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}

/// Byte ranges of the lines of `text`, without their line ends.
fn line_ranges(text: &str) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            let end = if i > start && text.as_bytes()[i - 1] == b'\r' {
                i - 1
            } else {
                i
            };
            lines.push(start..end);
            start = i + 1;
        }
    }
    if start < text.len() || lines.is_empty() {
        lines.push(start..text.len());
    }
    lines
}

/// The previews of every pane.
#[derive(Debug, Default)]
pub struct Previews {
    slots: HashMap<PaneId, Slot>,
}

impl Previews {
    fn slot(&mut self, pane: PaneId) -> &mut Slot {
        self.slots.entry(pane).or_default()
    }

    /// The pane's slot, if it has one.
    #[must_use]
    pub fn get(&self, pane: PaneId) -> Option<&Slot> {
        self.slots.get(&pane)
    }

    /// A document arrived for `pane`, about `entry`.
    pub fn ready(&mut self, pane: PaneId, doc: Box<dyn PreviewDoc>, entry: Option<EntryId>) {
        let slot = self.slot(pane);
        if slot.entry != entry {
            slot.reset(entry);
        }
        slot.set_doc(doc);
    }

    /// No document could be opened for `pane`'s selection.
    pub fn failed(&mut self, pane: PaneId, error: &Error, entry: Option<EntryId>) {
        let slot = self.slot(pane);
        if slot.entry != entry {
            slot.reset(entry);
        }
        slot.error = Some(error.to_string());
    }

    /// Draws `pane`'s preview for the entry the app is following; clears the
    /// slot when that changed.
    pub fn show(&mut self, ui: &mut egui::Ui, pane: PaneId, target: Option<EntryId>) {
        let slot = self.slot(pane);
        if slot.entry != target {
            slot.reset(target);
        }
        let ctx = ui.ctx().clone();
        slot.poll(&ctx, pane);

        if let Some(pages) = slot.pages
            && pages > 1
        {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(slot.page > 0, egui::Button::new("◀"))
                    .clicked()
                {
                    slot.page -= 1;
                }
                ui.label(format!("{} / {pages}", slot.page + 1));
                if ui
                    .add_enabled(slot.page + 1 < pages, egui::Button::new("▶"))
                    .clicked()
                {
                    slot.page += 1;
                }
            });
        }
        if slot.text.is_some() || slot.doc.as_ref().is_some_and(|d| d.text_layer().is_some()) {
            ui.horizontal(|ui| {
                ui.checkbox(&mut slot.wrap, "Wrap");
                ui.checkbox(&mut slot.numbers, "Line numbers");
            });
        }

        let avail = ui.available_size();
        let info_height =
            (slot.info.len() as f32 + 1.0) * ui.text_style_height(&TextStyle::Body) * 1.4;
        let frame_height = (avail.y - info_height).max(120.0);
        let ppp = ctx.pixels_per_point();
        let quantise =
            |points: f32| ((points * ppp / QUANTUM).ceil() * QUANTUM).max(QUANTUM) as u32;
        let req = RenderRequest {
            width: quantise(avail.x),
            height: quantise(frame_height),
            scale: ppp,
            dark: ui.visuals().dark_mode,
        };
        slot.request(&ctx, req);

        let body = egui::vec2(avail.x, frame_height);
        ui.allocate_ui(body, |ui| {
            ui.set_min_size(body);
            if let Some(texture) = &slot.texture {
                let (w, h) = slot.texture_size;
                let native = egui::vec2(w as f32 / ppp, h as f32 / ppp);
                let scale = (body.x / native.x).min(body.y / native.y).min(1.0);
                let size = native * scale;
                ui.vertical_centered(|ui| {
                    ui.add(egui::Image::from_texture(texture).fit_to_exact_size(size));
                });
            } else if let Some(view) = &slot.text {
                draw_text(ui, view, slot.wrap, slot.numbers);
            } else if let Some(error) = &slot.error {
                ui.colored_label(ui.visuals().warn_fg_color, format!("No preview: {error}"));
            } else if target.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak(if slot.doc.is_some() || slot.pending.is_some() {
                        "Rendering…"
                    } else {
                        "Opening…"
                    });
                });
            } else {
                ui.weak("Select a file to preview it");
            }
        });
        if slot.pending.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Rendering…");
            });
        }
        if !slot.info.is_empty() {
            ui.separator();
            egui::Grid::new(("preview-info", pane.0))
                .num_columns(2)
                .show(ui, |ui| {
                    for (key, value) in &slot.info {
                        ui.weak(key);
                        ui.label(value);
                        ui.end_row();
                    }
                });
        }
    }
}

/// Draws a text frame: one virtualised row per line, or one wrapped block.
fn draw_text(ui: &mut egui::Ui, view: &TextView, wrap: bool, numbers: bool) {
    let font = TextStyle::Monospace.resolve(ui.style());
    let default = ui.visuals().text_color();
    let gutter = ui.visuals().weak_text_color();
    let width = if numbers {
        view.lines.len().to_string().len()
    } else {
        0
    };
    if wrap {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut job = LayoutJob::default();
                job.wrap.max_width = ui.available_width();
                for (i, line) in view.lines.iter().enumerate() {
                    if numbers {
                        job.append(
                            &format!("{:>width$} ", i + 1),
                            0.0,
                            format(&font, gutter, false),
                        );
                    }
                    append_line(&mut job, view, line, &font, default);
                    job.append("\n", 0.0, format(&font, default, false));
                }
                ui.label(job);
            });
        return;
    }
    let row_height = ui.fonts_mut(|f| f.row_height(&font));
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show_rows(ui, row_height, view.lines.len(), |ui, range| {
            for i in range {
                let Some(line) = view.lines.get(i) else {
                    break;
                };
                let mut job = LayoutJob::default();
                job.wrap.max_width = f32::INFINITY;
                if numbers {
                    job.append(
                        &format!("{:>width$} ", i + 1),
                        0.0,
                        format(&font, gutter, false),
                    );
                }
                append_line(&mut job, view, line, &font, default);
                ui.label(job);
            }
        });
}

fn format(font: &FontId, color: Color32, italics: bool) -> TextFormat {
    TextFormat {
        font_id: font.clone(),
        color,
        italics,
        ..TextFormat::default()
    }
}

/// Appends one line of the block with its spans; gaps take the default
/// colour.
fn append_line(
    job: &mut LayoutJob,
    view: &TextView,
    line: &Range<usize>,
    font: &FontId,
    default: Color32,
) {
    let text = &view.block.text;
    let mut cursor = line.start;
    // Spans are sorted; find the first that may touch this line.
    let first = view.block.spans.partition_point(|s| s.end <= line.start);
    for span in &view.block.spans[first..] {
        if span.start >= line.end {
            break;
        }
        let start = span.start.max(line.start);
        let end = span.end.min(line.end);
        if start > cursor {
            job.append(&text[cursor..start], 0.0, format(font, default, false));
        }
        let color = Color32::from_rgba_unmultiplied(
            span.color[0],
            span.color[1],
            span.color[2],
            span.color[3],
        );
        job.append(&text[start..end], 0.0, format(font, color, span.italic));
        cursor = end;
    }
    if cursor < line.end {
        job.append(&text[cursor..line.end], 0.0, format(font, default, false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_on_newlines_and_keep_a_trailing_line() {
        assert_eq!(line_ranges("a\nbc\n"), [0..1, 2..4]);
        assert_eq!(line_ranges("a\r\nb"), [0..1, 3..4]);
        let empty = line_ranges("");
        assert_eq!((empty.len(), empty[0].clone()), (1, 0..0));
        let one = line_ranges("x");
        assert_eq!((one.len(), one[0].clone()), (1, 0..1));
    }
}
