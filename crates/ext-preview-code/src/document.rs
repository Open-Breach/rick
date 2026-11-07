//! Building an [`ext::Document`] from a stream of block and inline events:
//! the common ground of the Markdown and HTML renderers.

use std::sync::Arc;

use ext::{Block, Document, Image, Inline, Style};

/// A container being filled.
#[derive(Debug)]
enum Frame {
    /// The root, a list item or a quotation: a run of blocks.
    Blocks(Vec<Block>),
    /// A list whose items are collected one by one.
    List {
        ordered: bool,
        start: u64,
        items: Vec<Vec<Block>>,
    },
    /// A table: header cells, body rows, the row and cell in progress.
    Table {
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
        in_head: bool,
        row: Vec<Vec<Inline>>,
        cell: Option<Vec<Inline>>,
    },
}

/// Collects blocks and inlines into a document.
#[derive(Debug)]
pub struct Builder {
    stack: Vec<Frame>,
    inlines: Vec<Inline>,
    style: Style,
    /// The link whose text is being collected, and the text so far.
    link: Option<(String, String)>,
    /// The image whose alt text is being collected, and the text so far.
    image: Option<(String, String)>,
    heading: Option<u8>,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    /// An empty document.
    #[must_use]
    pub fn new() -> Self {
        Self {
            stack: vec![Frame::Blocks(Vec::new())],
            inlines: Vec::new(),
            style: Style::empty(),
            link: None,
            image: None,
            heading: None,
        }
    }

    /// The blocks of the innermost run, creating one when the innermost
    /// frame is a list or table outside an item or cell (ill-formed input).
    fn blocks(&mut self) -> &mut Vec<Block> {
        if !matches!(self.stack.last(), Some(Frame::Blocks(_))) {
            self.stack.push(Frame::Blocks(Vec::new()));
        }
        match self.stack.last_mut() {
            Some(Frame::Blocks(blocks)) => blocks,
            // Just pushed one.
            _ => unreachable!("a blocks frame was pushed"),
        }
    }

    /// Adds inline text in the current style, into the link text when a
    /// link is open, into the table cell when one is open.
    pub fn text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some((_, alt)) = &mut self.image {
            alt.push_str(text);
            return;
        }
        if let Some((_, link_text)) = &mut self.link {
            link_text.push_str(text);
            return;
        }
        let style = self.style;
        let target = self.inline_target();
        if let Some(Inline::Text {
            text: last,
            style: last_style,
        }) = target.last_mut()
            && *last_style == style
        {
            last.push_str(text);
            return;
        }
        target.push(Inline::Text {
            text: text.to_owned(),
            style,
        });
    }

    /// Where inlines go right now: the open table cell, else the run.
    fn inline_target(&mut self) -> &mut Vec<Inline> {
        if let Some(Frame::Table {
            cell: Some(cell), ..
        }) = self.stack.last_mut()
        {
            return cell;
        }
        &mut self.inlines
    }

    /// Turns a style on.
    pub fn push_style(&mut self, style: Style) {
        self.style |= style;
    }

    /// Turns a style off.
    pub fn pop_style(&mut self, style: Style) {
        self.style &= !style;
    }

    /// A hard line break.
    pub fn line_break(&mut self) {
        if self.link.is_some() {
            self.text(" ");
            return;
        }
        self.inline_target().push(Inline::Break);
    }

    /// Opens a link; its text is what follows until [`Self::end_link`].
    pub fn start_link(&mut self, href: &str) {
        self.end_link();
        self.link = Some((href.to_owned(), String::new()));
    }

    /// Closes the link, if one is open.
    pub fn end_link(&mut self) {
        if let Some((href, text)) = self.link.take() {
            let text = if text.trim().is_empty() {
                href.clone()
            } else {
                text
            };
            self.inline_target().push(Inline::Link { text, href });
        }
    }

    /// Opens a picture whose alt text is what follows until
    /// [`Self::end_image`].
    pub fn start_image(&mut self, source: &str) {
        self.end_image();
        self.image = Some((source.to_owned(), String::new()));
    }

    /// Closes the picture, if one is open.
    pub fn end_image(&mut self) {
        if let Some((source, alt)) = self.image.take() {
            self.image(&alt, &source);
        }
    }

    /// A picture: on its own when nothing else is in the paragraph, inline
    /// otherwise.
    pub fn image(&mut self, alt: &str, source: &str) {
        let image = Image {
            alt: alt.to_owned(),
            source: source.to_owned(),
            data: None,
        };
        if self.link.is_some() {
            self.text(alt);
            return;
        }
        self.inline_target().push(Inline::Image(image));
    }

    /// Ends the paragraph in progress, if it holds anything: a lone image
    /// becomes a block of its own.
    pub fn end_paragraph(&mut self) {
        self.end_image();
        self.end_link();
        let mut inlines = std::mem::take(&mut self.inlines);
        trim_inlines(&mut inlines);
        if inlines.is_empty() {
            return;
        }
        let block = match (inlines.len(), inlines.first()) {
            (1, Some(Inline::Image(image))) => Block::Image(image.clone()),
            _ => Block::Paragraph(inlines),
        };
        if let Some(level) = self.heading.take() {
            let text = match block {
                Block::Paragraph(inlines) => inlines,
                Block::Image(image) => vec![Inline::Image(image)],
                _ => Vec::new(),
            };
            self.blocks().push(Block::Heading { level, text });
        } else {
            self.blocks().push(block);
        }
    }

    /// Starts a heading of `level` (1 to 6); its text is the paragraph
    /// that follows.
    pub fn start_heading(&mut self, level: u8) {
        self.end_paragraph();
        self.heading = Some(level.clamp(1, 6));
    }

    /// Ends the heading.
    pub fn end_heading(&mut self) {
        if self.heading.is_some() {
            if self.inlines.is_empty() {
                self.heading = None;
            } else {
                self.end_paragraph();
            }
        }
    }

    /// Starts a list.
    pub fn start_list(&mut self, ordered: bool, start: u64) {
        self.end_paragraph();
        self.stack.push(Frame::List {
            ordered,
            start,
            items: Vec::new(),
        });
    }

    /// Starts a list item.
    pub fn start_item(&mut self) {
        self.end_paragraph();
        if !matches!(self.stack.last(), Some(Frame::List { .. })) {
            self.start_list(false, 1);
        }
        self.stack.push(Frame::Blocks(Vec::new()));
    }

    /// Ends the list item.
    pub fn end_item(&mut self) {
        self.end_paragraph();
        if matches!(self.stack.last(), Some(Frame::Blocks(_))) && self.stack.len() > 1 {
            let Some(Frame::Blocks(blocks)) = self.stack.pop() else {
                return;
            };
            if let Some(Frame::List { items, .. }) = self.stack.last_mut() {
                items.push(blocks);
            } else {
                // Not inside a list after all: keep the blocks.
                self.blocks().extend(blocks);
            }
        }
    }

    /// Ends the list.
    pub fn end_list(&mut self) {
        self.end_paragraph();
        // An item left open closes with the list.
        if matches!(self.stack.last(), Some(Frame::Blocks(_)))
            && matches!(self.stack.iter().rev().nth(1), Some(Frame::List { .. }))
        {
            self.end_item();
        }
        if let Some(Frame::List {
            ordered,
            start,
            items,
        }) = self.stack.pop()
        {
            if !items.is_empty() {
                self.blocks().push(Block::List {
                    ordered,
                    start,
                    items,
                });
            }
        } else if let Some(frame) = self.stack.pop() {
            // Not a list: put it back.
            self.stack.push(frame);
        }
    }

    /// Starts a quotation.
    pub fn start_quote(&mut self) {
        self.end_paragraph();
        self.stack.push(Frame::Blocks(Vec::new()));
    }

    /// Ends the quotation.
    pub fn end_quote(&mut self) {
        self.end_paragraph();
        if self.stack.len() > 1
            && let Some(Frame::Blocks(blocks)) = self.stack.pop()
        {
            self.blocks().push(Block::Quote(blocks));
        }
    }

    /// A code block.
    pub fn code_block(&mut self, language: Option<&str>, text: &str) {
        self.end_paragraph();
        let text = text.strip_suffix('\n').unwrap_or(text).to_owned();
        self.blocks().push(Block::Code {
            language: language
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned),
            text,
        });
    }

    /// A horizontal rule.
    pub fn rule(&mut self) {
        self.end_paragraph();
        self.blocks().push(Block::Rule);
    }

    /// Starts a table.
    pub fn start_table(&mut self) {
        self.end_paragraph();
        self.stack.push(Frame::Table {
            header: Vec::new(),
            rows: Vec::new(),
            in_head: false,
            row: Vec::new(),
            cell: None,
        });
    }

    /// The rows that follow are the header.
    pub fn start_head(&mut self) {
        if let Some(Frame::Table { in_head, .. }) = self.stack.last_mut() {
            *in_head = true;
        }
    }

    /// The header is complete.
    pub fn end_head(&mut self) {
        self.end_row();
        if let Some(Frame::Table { in_head, .. }) = self.stack.last_mut() {
            *in_head = false;
        }
    }

    /// Starts a row.
    pub fn start_row(&mut self) {
        self.end_row();
    }

    /// Ends the row, if one has cells.
    pub fn end_row(&mut self) {
        self.end_cell();
        if let Some(Frame::Table {
            header,
            rows,
            in_head,
            row,
            ..
        }) = self.stack.last_mut()
            && !row.is_empty()
        {
            let row = std::mem::take(row);
            if *in_head && header.is_empty() {
                *header = row;
            } else {
                rows.push(row);
            }
        }
    }

    /// Starts a cell; `header` marks a `th`, which makes a first row the
    /// header.
    pub fn start_cell(&mut self, header: bool) {
        self.end_cell();
        if let Some(Frame::Table {
            cell,
            in_head,
            rows,
            header: head,
            ..
        }) = self.stack.last_mut()
        {
            if header && rows.is_empty() && head.is_empty() {
                *in_head = true;
            }
            *cell = Some(Vec::new());
        }
    }

    /// Ends the cell.
    pub fn end_cell(&mut self) {
        self.end_link();
        if let Some(Frame::Table { cell, row, .. }) = self.stack.last_mut()
            && let Some(mut cell) = cell.take()
        {
            trim_inlines(&mut cell);
            row.push(cell);
        }
    }

    /// Ends the table.
    pub fn end_table(&mut self) {
        self.end_row();
        if let Some(Frame::Table { header, rows, .. }) = self.stack.pop() {
            if !header.is_empty() || !rows.is_empty() {
                self.blocks().push(Block::Table { header, rows });
            }
        } else if let Some(frame) = self.stack.pop() {
            self.stack.push(frame);
        }
    }

    /// Whether a table is being filled.
    #[must_use]
    pub fn in_table(&self) -> bool {
        matches!(self.stack.last(), Some(Frame::Table { .. }))
    }

    /// Closes whatever is open and returns the document.
    #[must_use]
    pub fn finish(mut self) -> Document {
        self.end_paragraph();
        while self.stack.len() > 1 {
            match self.stack.last() {
                Some(Frame::Table { .. }) => self.end_table(),
                Some(Frame::List { .. }) => self.end_list(),
                Some(Frame::Blocks(_)) => {
                    if matches!(self.stack.iter().rev().nth(1), Some(Frame::List { .. })) {
                        self.end_item();
                    } else {
                        self.end_quote();
                    }
                }
                None => break,
            }
        }
        let blocks = match self.stack.pop() {
            Some(Frame::Blocks(blocks)) => blocks,
            _ => Vec::new(),
        };
        Document { blocks }
    }
}

/// Drops leading and trailing whitespace and breaks of a run.
fn trim_inlines(inlines: &mut Vec<Inline>) {
    while let Some(first) = inlines.first_mut() {
        match first {
            Inline::Break => {
                inlines.remove(0);
            }
            Inline::Text { text, .. } => {
                let trimmed = text.trim_start();
                if trimmed.is_empty() {
                    inlines.remove(0);
                } else {
                    *text = trimmed.to_owned();
                    break;
                }
            }
            _ => break,
        }
    }
    while let Some(last) = inlines.last_mut() {
        match last {
            Inline::Break => {
                inlines.pop();
            }
            Inline::Text { text, .. } => {
                let trimmed = text.trim_end();
                if trimmed.is_empty() {
                    inlines.pop();
                } else {
                    *text = trimmed.to_owned();
                    break;
                }
            }
            _ => break,
        }
    }
}

/// Every image of a document, for loading their bytes.
pub fn images_mut(document: &mut Document) -> Vec<&mut Image> {
    fn walk<'a>(blocks: &'a mut [Block], out: &mut Vec<&'a mut Image>) {
        for block in blocks {
            match block {
                Block::Image(image) => out.push(image),
                Block::Paragraph(inlines) | Block::Heading { text: inlines, .. } => {
                    inline_images(inlines, out);
                }
                Block::List { items, .. } => {
                    for item in items {
                        walk(item, out);
                    }
                }
                Block::Quote(blocks) => walk(blocks, out),
                Block::Table { header, rows } => {
                    inline_images(header.iter_mut().flatten(), out);
                    inline_images(rows.iter_mut().flatten().flatten(), out);
                }
                Block::Code { .. } | Block::Rule => {}
            }
        }
    }
    fn inline_images<'a>(
        inlines: impl IntoIterator<Item = &'a mut Inline>,
        out: &mut Vec<&'a mut Image>,
    ) {
        for inline in inlines {
            if let Inline::Image(image) = inline {
                out.push(image);
            }
        }
    }
    let mut out = Vec::new();
    walk(&mut document.blocks, &mut out);
    out
}

/// The document shared, as [`ext::Content`] wants it.
#[must_use]
pub fn share(document: Document) -> Arc<Document> {
    Arc::new(document)
}
