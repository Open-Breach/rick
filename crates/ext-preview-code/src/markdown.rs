//! Markdown to a document through `pulldown-cmark`: headings, paragraphs,
//! emphasis, code (inline and fenced), lists with task markers, quotes,
//! tables, links, images and rules; raw HTML inside Markdown is shown as
//! text.

use ext::{Document, Style};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::document::Builder;

/// Renders Markdown `text`.
#[must_use]
pub fn render(text: &str) -> Document {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut builder = Builder::new();
    let mut code: Option<(Option<String>, String)> = None;
    for event in Parser::new_ext(text, options) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => builder.end_paragraph(),
                Tag::Heading { level, .. } => builder.start_heading(heading_level(level)),
                Tag::BlockQuote(_) => builder.start_quote(),
                Tag::CodeBlock(kind) => {
                    let language = match kind {
                        CodeBlockKind::Fenced(info) => info
                            .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
                            .next()
                            .filter(|l| !l.is_empty())
                            .map(str::to_owned),
                        CodeBlockKind::Indented => None,
                    };
                    code = Some((language, String::new()));
                }
                Tag::List(start) => builder.start_list(start.is_some(), start.unwrap_or(1)),
                Tag::Item => builder.start_item(),
                Tag::Table(_) => builder.start_table(),
                Tag::TableHead => {
                    builder.start_head();
                    builder.start_row();
                }
                Tag::TableRow => builder.start_row(),
                Tag::TableCell => builder.start_cell(false),
                Tag::Emphasis => builder.push_style(Style::ITALIC),
                Tag::Strong => builder.push_style(Style::BOLD),
                Tag::Strikethrough => builder.push_style(Style::STRIKETHROUGH),
                Tag::Link { dest_url, .. } => builder.start_link(&dest_url),
                Tag::Image { dest_url, .. } => builder.start_image(&dest_url),
                Tag::HtmlBlock
                | Tag::FootnoteDefinition(_)
                | Tag::DefinitionList
                | Tag::DefinitionListTitle
                | Tag::DefinitionListDefinition
                | Tag::MetadataBlock(_)
                | Tag::Superscript
                | Tag::Subscript => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => builder.end_paragraph(),
                TagEnd::Heading(_) => builder.end_heading(),
                TagEnd::BlockQuote(_) => builder.end_quote(),
                TagEnd::CodeBlock => {
                    if let Some((language, body)) = code.take() {
                        builder.code_block(language.as_deref(), &body);
                    }
                }
                TagEnd::List(_) => builder.end_list(),
                TagEnd::Item => builder.end_item(),
                TagEnd::Table => builder.end_table(),
                TagEnd::TableHead => builder.end_head(),
                TagEnd::TableRow => builder.end_row(),
                TagEnd::TableCell => builder.end_cell(),
                TagEnd::Emphasis => builder.pop_style(Style::ITALIC),
                TagEnd::Strong => builder.pop_style(Style::BOLD),
                TagEnd::Strikethrough => builder.pop_style(Style::STRIKETHROUGH),
                TagEnd::Link => builder.end_link(),
                TagEnd::Image => builder.end_image(),
                TagEnd::HtmlBlock
                | TagEnd::FootnoteDefinition
                | TagEnd::DefinitionList
                | TagEnd::DefinitionListTitle
                | TagEnd::DefinitionListDefinition
                | TagEnd::MetadataBlock(_)
                | TagEnd::Superscript
                | TagEnd::Subscript => {}
            },
            Event::Text(text) => match &mut code {
                Some((_, body)) => body.push_str(&text),
                None => builder.text(&text),
            },
            Event::Code(text) => {
                builder.push_style(Style::CODE);
                builder.text(&text);
                builder.pop_style(Style::CODE);
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                // Raw HTML is shown as what it is, not interpreted.
                builder.text(&html);
            }
            Event::SoftBreak => builder.text(" "),
            Event::HardBreak => builder.line_break(),
            Event::Rule => builder.rule(),
            Event::TaskListMarker(done) => builder.text(if done { "[x] " } else { "[ ] " }),
            Event::FootnoteReference(label) => builder.text(&format!("[{label}]")),
            Event::InlineMath(text) | Event::DisplayMath(text) => builder.text(&text),
        }
    }
    builder.finish()
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}
