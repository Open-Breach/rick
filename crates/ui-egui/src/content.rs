//! The content kinds the pane lays out itself, beyond pixels and text
//! ([`ext::Content`]): the read-only file list of a folder, an archive or
//! a disk image in the explorer's columns with its stats line, the grid of
//! a CSV file with a header row, and the rendered document of a Markdown
//! or HTML file with its blocks and inlines. A previewer's textual
//! fallback is for a pane that cannot show these; this pane can.
//!
//! Nothing here asks for width: the listing and the grid are tables that
//! clip their cells, a document wraps its text at the pane's width and
//! scrolls its code sideways, and a picture is scaled down to fit, never
//! up.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use egui::{Align, Rect, RichText, TextureHandle, TextureOptions, pos2, vec2};
use egui_extras::{Column as TableColumn, TableBuilder};
use ext::{Block, Content, Document, Grid, Image, Inline, Listing, ListingEntry, Style};
use types::EntryKind;

use crate::{format, icons};

/// How far a nested row or block is indented, in points.
const INDENT: f32 = 18.0;
/// The chevron's width in the listing.
const CHEVRON: f32 = 16.0;
/// How many grid columns are laid out; a wider table says so.
const MAX_COLUMNS: usize = 200;
/// The heading sizes, level 1 to 6, as factors of the body size.
const HEADING_SCALE: [f32; 6] = [1.8, 1.5, 1.3, 1.15, 1.05, 1.0];

/// The pane's state for the content it shows: what the listing has
/// folded, the textures of a document's pictures. Reset with the document.
#[derive(Default)]
pub struct State {
    /// The paths of the listing's collapsed directories.
    collapsed: HashSet<String>,
    /// The rows shown for the current fold state, as indexes into the
    /// listing; rebuilt when a fold changes.
    visible: Option<Vec<usize>>,
    /// A document's pictures by block position, decoded once; `None` when
    /// the bytes could not be decoded.
    images: HashMap<usize, Option<TextureHandle>>,
    /// What the last frame laid out, for tests.
    pub drawn: Drawn,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("collapsed", &self.collapsed.len())
            .field("visible", &self.visible.as_ref().map(Vec::len))
            .field("images", &self.images.len())
            .field("drawn", &self.drawn)
            .finish()
    }
}

/// What the content layout drew in the last frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Drawn {
    /// Nothing of these kinds.
    #[default]
    None,
    /// A listing: the rows shown (after folding) and the stats line.
    Listing {
        /// How many rows were laid out or scrolled past.
        rows: usize,
        /// The stats line's text.
        stats: String,
    },
    /// A grid: the header cells and the number of body rows.
    Grid {
        /// The header row.
        header: Vec<String>,
        /// The body rows.
        rows: usize,
    },
    /// A document: the number of blocks laid out at the top level.
    Document {
        /// Top-level blocks.
        blocks: usize,
    },
}

impl State {
    /// Folds or unfolds the listing's directory at `path`.
    pub fn toggle(&mut self, path: &str) {
        if !self.collapsed.remove(path) {
            self.collapsed.insert(path.to_owned());
        }
        self.visible = None;
    }

    /// Whether the listing's directory at `path` is folded.
    #[must_use]
    pub fn is_collapsed(&self, path: &str) -> bool {
        self.collapsed.contains(path)
    }

    /// The rows the fold state leaves visible, as indexes into `listing`.
    fn visible(&mut self, listing: &Listing) -> &[usize] {
        if self.visible.is_none() {
            let mut rows = Vec::with_capacity(listing.entries.len());
            // A collapsed directory hides everything deeper than it until
            // the depth comes back to its own.
            let mut hidden_below: Option<usize> = None;
            for (index, entry) in listing.entries.iter().enumerate() {
                if let Some(depth) = hidden_below {
                    if entry.depth > depth {
                        continue;
                    }
                    hidden_below = None;
                }
                rows.push(index);
                if entry.kind == EntryKind::Dir && self.collapsed.contains(&entry.path) {
                    hidden_below = Some(entry.depth);
                }
            }
            self.visible = Some(rows);
        }
        self.visible.as_deref().unwrap_or(&[])
    }
}

/// Draws `content` into the Ui's available space. A missing library is
/// not laid out here: the pane shows the File info card for it.
pub fn draw(ui: &mut egui::Ui, content: &Content, state: &mut State, now: SystemTime) {
    ui.style_mut().interaction.selectable_labels = false;
    match content {
        Content::Listing(listing) => draw_listing(ui, listing, state, now),
        Content::Grid(grid) => draw_grid(ui, &**grid, state),
        Content::Document(document) => draw_document(ui, document, state),
        Content::MissingLibrary(_) => {}
    }
}

/// The glyph of a listing row: the folder, the link, or the type by name.
fn listing_icon(entry: &ListingEntry) -> &'static str {
    match entry.kind {
        EntryKind::Dir => icons::FOLDER,
        EntryKind::Symlink { .. } => icons::LINK,
        _ => icons::by_extension(&entry.name),
    }
}

/// A read-only file list in the explorer's columns (name, type, date
/// modified, size; the compressed size when the format records it), the
/// directories folding in place, and the stats line under it.
fn draw_listing(ui: &mut egui::Ui, listing: &Listing, state: &mut State, now: SystemTime) {
    let compressed = listing.entries.iter().any(|e| e.compressed.is_some());
    let height = crate::views::row_height(ui);
    let stats_h = ui.text_style_height(&egui::TextStyle::Small) + 8.0;
    let whole = ui.available_rect_before_wrap();
    let table_rect = Rect::from_min_max(
        whole.min,
        pos2(whole.right(), (whole.bottom() - stats_h).max(whole.top())),
    );
    let visible: Vec<usize> = state.visible(listing).to_vec();
    let mut toggled: Option<String> = None;
    let mut table_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(table_rect)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    table_ui.set_clip_rect(table_rect.intersect(ui.clip_rect()));
    table_ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
    let mut builder = TableBuilder::new(&mut table_ui)
        .id_salt("preview-listing")
        .resizable(true)
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(Align::Center))
        .column(TableColumn::remainder().at_least(80.0).clip(true))
        .column(TableColumn::initial(60.0).at_least(30.0).clip(true))
        .column(TableColumn::initial(110.0).at_least(40.0).clip(true))
        .column(TableColumn::initial(70.0).at_least(40.0).clip(true));
    if compressed {
        builder = builder.column(TableColumn::initial(70.0).at_least(40.0).clip(true));
    }
    let mut headers = vec![
        ext::t!("preview-column-name"),
        ext::t!("preview-column-type"),
        ext::t!("preview-column-modified"),
        ext::t!("preview-column-size"),
    ];
    if compressed {
        headers.push(ext::t!("preview-column-compressed"));
    }
    builder
        .header(height, |mut row| {
            for header in &headers {
                row.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(header).strong()).truncate());
                });
            }
        })
        .body(|body| {
            body.rows(height, visible.len(), |mut row| {
                let Some(entry) = visible
                    .get(row.index())
                    .and_then(|i| listing.entries.get(*i))
                else {
                    return;
                };
                let folded = state.is_collapsed(&entry.path);
                if listing_row(&mut row, entry, folded, compressed, now) {
                    toggled = Some(entry.path.clone());
                }
            });
        });
    if let Some(path) = toggled {
        state.toggle(&path);
        ui.ctx().request_repaint();
    }
    let stats_rect = Rect::from_min_max(pos2(whole.left(), table_rect.bottom()), whole.max);
    draw_stats(ui, stats_rect, &listing.stats);
    state.drawn = Drawn::Listing {
        rows: visible.len(),
        stats: listing.stats.clone(),
    };
}

/// One row of the listing in the explorer's columns; returns whether its
/// chevron was clicked.
fn listing_row(
    row: &mut egui_extras::TableRow<'_, '_>,
    entry: &ListingEntry,
    folded: bool,
    compressed: bool,
    now: SystemTime,
) -> bool {
    let mut toggled = false;
    row.col(|ui| {
        ui.add_space(entry.depth as f32 * INDENT);
        if entry.kind == EntryKind::Dir {
            let glyph = if folded {
                icons::TREE_CLOSED
            } else {
                icons::TREE_OPEN
            };
            let chevron = ui.add_sized(
                [CHEVRON, ui.available_height()],
                egui::Button::new(RichText::new(glyph).weak()).frame(false),
            );
            toggled = chevron.clicked();
        } else {
            ui.add_space(CHEVRON + ui.spacing().item_spacing.x);
        }
        ui.label(listing_icon(entry));
        if entry.encrypted {
            ui.label(RichText::new(icons::LOCK).weak())
                .on_hover_text(ext::t!("preview-encrypted"));
        }
        ui.add(egui::Label::new(&entry.name).truncate());
    });
    row.col(|ui| {
        ui.add(egui::Label::new(listing_type(entry)).truncate());
    });
    row.col(|ui| {
        let (brief, full) = format::date(entry.modified, now);
        ui.add(egui::Label::new(brief).truncate())
            .on_hover_text(full);
    });
    row.col(|ui| {
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if entry.kind != EntryKind::Dir {
                ui.label(format::size(entry.size));
            }
        });
    });
    if compressed {
        row.col(|ui| {
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if let Some(size) = entry.compressed {
                    ui.label(format::size(Some(size)));
                }
            });
        });
    }
    toggled
}

/// The stats line under a listing, in small text.
fn draw_stats(ui: &mut egui::Ui, rect: Rect, stats: &str) {
    let mut foot = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    foot.set_clip_rect(rect.intersect(ui.clip_rect()));
    foot.add_space(6.0);
    foot.add(
        egui::Label::new(RichText::new(stats).small().weak())
            .selectable(false)
            .truncate(),
    );
}

/// The type text of a listing row, as the explorer's Type column would
/// show it.
fn listing_type(entry: &ListingEntry) -> String {
    match entry.kind {
        EntryKind::Dir => ext::t!("preview-type-folder"),
        EntryKind::Symlink { .. } => ext::t!("preview-type-link"),
        EntryKind::Other => ext::t!("preview-type-special"),
        EntryKind::File => entry
            .name
            .rsplit_once('.')
            .filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty())
            .map_or_else(
                || ext::t!("preview-type-file"),
                |(_, ext)| ext.to_uppercase(),
            ),
    }
}

/// A table as a virtualised grid: the header row in bold, then the rows,
/// each parsed as it comes into view.
fn draw_grid(ui: &mut egui::Ui, grid: &dyn Grid, state: &mut State) {
    let header = grid.header();
    let rows = grid.rows();
    let columns = header.len().clamp(1, MAX_COLUMNS);
    let height = crate::views::row_height(ui);
    ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
    let mut builder = TableBuilder::new(ui)
        .id_salt("preview-grid")
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(Align::Center));
    for _ in 0..columns {
        builder = builder.column(TableColumn::initial(110.0).at_least(32.0).clip(true));
    }
    builder
        .header(height, |mut row| {
            for (i, title) in header.iter().take(columns).enumerate() {
                row.col(|ui| {
                    let shown = if title.is_empty() {
                        (i + 1).to_string()
                    } else {
                        title.clone()
                    };
                    ui.add(egui::Label::new(RichText::new(shown).strong()).truncate());
                });
            }
            if header.len() > columns {
                row.col(|ui| {
                    ui.weak(ext::t!(
                        "preview-more-columns",
                        count = header.len() - columns
                    ));
                });
            }
        })
        .body(|body| {
            body.rows(height, rows, |mut row| {
                let index = row.index();
                let cells = grid.cells(index..index + 1);
                let cells = cells.first().map_or(&[][..], Vec::as_slice);
                for i in 0..columns {
                    row.col(|ui| {
                        if let Some(cell) = cells.get(i) {
                            ui.add(egui::Label::new(cell).truncate());
                        }
                    });
                }
            });
        });
    state.drawn = Drawn::Grid {
        header: header.to_vec(),
        rows,
    };
}

/// A document, top to bottom in one scroll: headings, paragraphs, lists,
/// code, quotes, tables, pictures and rules.
fn draw_document(ui: &mut egui::Ui, document: &Document, state: &mut State) {
    let width = ui.available_width();
    egui::ScrollArea::vertical()
        .id_salt("preview-document")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_max_width(width);
            ui.add_space(6.0);
            let mut position = 0usize;
            for block in &document.blocks {
                draw_block(ui, block, state, &mut position);
                ui.add_space(ui.spacing().item_spacing.y * 2.0);
            }
        });
    state.drawn = Drawn::Document {
        blocks: document.blocks.len(),
    };
}

/// One block; `position` counts the blocks and nested blocks laid out so
/// far, the key a picture's texture is cached under.
fn draw_block(ui: &mut egui::Ui, block: &Block, state: &mut State, position: &mut usize) {
    *position += 1;
    let here = *position;
    let left = ui.cursor().left();
    let right = ui.max_rect().right();
    let width = (right - left).max(0.0);
    match block {
        Block::Heading { level, text } => {
            let scale = HEADING_SCALE[usize::from(level.clamp(&1, &6) - 1)];
            let size = ui.text_style_height(&egui::TextStyle::Body) * scale;
            draw_inlines(ui, text, Some(size), width);
        }
        Block::Paragraph(inlines) => draw_inlines(ui, inlines, None, width),
        Block::List {
            ordered,
            start,
            items,
        } => {
            for (i, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}.", start + i as u64)
                } else {
                    "•".to_owned()
                };
                draw_indented(ui, &marker, item, state, position, width);
            }
        }
        Block::Code { language, text } => draw_code(ui, language.as_deref(), text, here, width),
        Block::Quote(blocks) => {
            let bar = ui.visuals().weak_text_color();
            let rect = draw_indented(ui, "", blocks, state, position, width);
            ui.painter().vline(
                rect.left() + 4.0,
                rect.y_range(),
                egui::Stroke::new(2.0, bar),
            );
        }
        Block::Table { header, rows } => draw_table(ui, header, rows, here, width),
        Block::Image(image) => draw_image(ui, image, state, here, width),
        Block::Rule => {
            ui.separator();
        }
    }
}

/// Blocks indented by one step behind a marker (a bullet, a number, or
/// nothing for a quotation); returns the rectangle they took.
fn draw_indented(
    ui: &mut egui::Ui,
    marker: &str,
    blocks: &[Block],
    state: &mut State,
    position: &mut usize,
    width: f32,
) -> Rect {
    ui.horizontal_top(|ui| {
        ui.add_sized(
            [INDENT, ui.text_style_height(&egui::TextStyle::Body)],
            egui::Label::new(RichText::new(marker).weak()),
        );
        ui.vertical(|ui| {
            ui.set_max_width((width - INDENT - ui.spacing().item_spacing.x).max(0.0));
            for block in blocks {
                draw_block(ui, block, state, position);
            }
        });
    })
    .response
    .rect
}

/// Preformatted code on its own background, scrolling sideways instead of
/// wrapping, with the language named under it.
fn draw_code(ui: &mut egui::Ui, language: Option<&str>, text: &str, key: usize, width: f32) {
    let fill = ui.visuals().extreme_bg_color;
    egui::Frame::new()
        .fill(fill)
        .corner_radius(4)
        .inner_margin(6)
        .show(ui, |ui| {
            ui.set_width((width - 12.0).max(0.0));
            egui::ScrollArea::horizontal()
                .id_salt(("preview-code", key))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(text).monospace())
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
            if let Some(language) = language {
                ui.label(RichText::new(language).small().weak());
            }
        });
}

/// A table: the header cells in bold, then the rows, striped.
fn draw_table(
    ui: &mut egui::Ui,
    header: &[Vec<Inline>],
    rows: &[Vec<Vec<Inline>>],
    key: usize,
    width: f32,
) {
    egui::Grid::new(("preview-table", key))
        .striped(true)
        .spacing(vec2(12.0, 4.0))
        .show(ui, |ui| {
            if !header.is_empty() {
                for cell in header {
                    ui.vertical(|ui| draw_inlines_strong(ui, cell, width));
                }
                ui.end_row();
            }
            for row in rows {
                for cell in row {
                    ui.vertical(|ui| draw_inlines(ui, cell, None, width));
                }
                ui.end_row();
            }
        });
}

/// A header cell: its inlines in bold.
fn draw_inlines_strong(ui: &mut egui::Ui, inlines: &[Inline], width: f32) {
    let bold: Vec<Inline> = inlines
        .iter()
        .map(|inline| match inline {
            Inline::Text { text, style } => Inline::Text {
                text: text.clone(),
                style: *style | Style::BOLD,
            },
            other => other.clone(),
        })
        .collect();
    draw_inlines(ui, &bold, None, width);
}

/// A run of inlines, wrapping at `width`: text in its style, a link that
/// opens with the system, a line break, a picture in the line.
fn draw_inlines(ui: &mut egui::Ui, inlines: &[Inline], size: Option<f32>, width: f32) {
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(width);
        ui.spacing_mut().item_spacing.x = 0.0;
        for inline in inlines {
            match inline {
                Inline::Text { text, style } => {
                    let mut rich = RichText::new(text.as_str());
                    if let Some(size) = size {
                        rich = rich.size(size).strong();
                    }
                    if style.contains(Style::BOLD) {
                        rich = rich.strong();
                    }
                    if style.contains(Style::ITALIC) {
                        rich = rich.italics();
                    }
                    if style.contains(Style::CODE) {
                        rich = rich.code();
                    }
                    if style.contains(Style::STRIKETHROUGH) {
                        rich = rich.strikethrough();
                    }
                    ui.add(egui::Label::new(rich).wrap());
                }
                Inline::Link { text, href } => {
                    let mut rich = RichText::new(text.as_str());
                    if let Some(size) = size {
                        rich = rich.size(size).strong();
                    }
                    let link = ui.add(egui::Link::new(rich)).on_hover_text(href);
                    if link.clicked() && is_url(href) {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(href));
                    }
                }
                Inline::Image(image) => {
                    ui.add(egui::Label::new(format!("{} {}", icons::PREVIEW, image.alt)).wrap())
                        .on_hover_text(&image.source);
                }
                Inline::Break => {
                    ui.end_row();
                }
            }
        }
    });
}

/// Whether a link's target is a URL the system can open; a relative path
/// or an anchor is not.
fn is_url(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
}

/// A picture on its own: decoded once into a texture and drawn scaled to
/// the width, never above its own size; its alt text when the bytes are
/// missing or no picture.
fn draw_image(ui: &mut egui::Ui, image: &Image, state: &mut State, key: usize, width: f32) {
    let texture = state
        .images
        .entry(key)
        .or_insert_with(|| decode(ui.ctx(), image, key))
        .clone();
    if let Some(texture) = texture {
        let native = texture.size_vec2();
        let scale = (width / native.x).min(1.0);
        let response =
            ui.add(egui::Image::from_texture(&texture).fit_to_exact_size(native * scale));
        if !image.alt.is_empty() {
            response.on_hover_text(&image.alt);
        }
    } else {
        let text = if image.alt.is_empty() {
            &image.source
        } else {
            &image.alt
        };
        ui.add(
            egui::Label::new(
                RichText::new(format!("{} {text}", icons::PREVIEW))
                    .weak()
                    .italics(),
            )
            .wrap(),
        )
        .on_hover_text(&image.source);
    }
}

/// Decodes a picture's bytes into a texture; `None` without bytes or for
/// a format the decoder does not know.
fn decode(ctx: &egui::Context, image: &Image, key: usize) -> Option<TextureHandle> {
    let data = image.data.as_ref()?;
    let decoded = image::load_from_memory(data).ok()?.into_rgba8();
    let (w, h) = decoded.dimensions();
    let color =
        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], decoded.as_raw());
    Some(ctx.load_texture(
        format!("preview-document-{key}"),
        color,
        TextureOptions::LINEAR,
    ))
}
