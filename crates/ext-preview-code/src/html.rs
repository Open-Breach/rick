//! HTML to a document without a browser engine: a small tokenizer over the
//! tags that matter (headings, paragraphs, lists, quotes, code, tables,
//! links, images, emphasis), everything else passed through as text or
//! dropped. Sanitising: scripts, styles, frames, forms and their content
//! are dropped; of the attributes only `href`, `src`, `alt` and `start`
//! are read, and a link target is kept only when it is a web, mail or
//! relative address.

use ext::{Document, Style};

use crate::document::Builder;

/// Tags whose whole content is dropped.
const DROPPED: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "noscript", "template", "svg", "math", "head",
    "title", "form", "input", "button", "select", "textarea", "canvas", "video", "audio",
];

/// Tags that end the paragraph before and after them.
const BLOCK_BOUNDARY: &[&str] = &[
    "div",
    "section",
    "article",
    "main",
    "header",
    "footer",
    "nav",
    "aside",
    "figure",
    "figcaption",
    "details",
    "summary",
    "address",
    "dl",
    "dt",
    "dd",
    "fieldset",
    "legend",
    "body",
    "html",
    "center",
    "menu",
    "dialog",
];

/// One token of the markup.
#[derive(Debug, PartialEq, Eq)]
enum Token<'a> {
    Text(&'a str),
    /// `<name attrs>`; `closing` for `</name>`, `void` for `<name/>`.
    Tag {
        name: String,
        attrs: &'a str,
        closing: bool,
    },
}

/// Splits `text` into tokens; comments and declarations are dropped.
fn tokens(text: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            out.push(Token::Text(rest));
            break;
        };
        if open > 0 {
            out.push(Token::Text(&rest[..open]));
        }
        let from_tag = &rest[open..];
        if from_tag.starts_with("<!--") {
            let end = from_tag.find("-->").map_or(from_tag.len(), |e| e + 3);
            rest = &from_tag[end..];
            continue;
        }
        if from_tag.starts_with("<!") || from_tag.starts_with("<?") {
            let end = from_tag.find('>').map_or(from_tag.len(), |e| e + 1);
            rest = &from_tag[end..];
            continue;
        }
        let Some(close) = find_tag_end(from_tag) else {
            // A lone `<`: text.
            out.push(Token::Text(from_tag));
            break;
        };
        let inner = &from_tag[1..close];
        rest = &from_tag[close + 1..];
        let (closing, inner) = match inner.strip_prefix('/') {
            Some(inner) => (true, inner),
            None => (false, inner),
        };
        let inner = inner.trim().trim_end_matches('/').trim();
        let name_end = inner
            .find(|c: char| c.is_whitespace())
            .unwrap_or(inner.len());
        let name = inner[..name_end].to_ascii_lowercase();
        if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            // `<` followed by something that is no tag: text.
            out.push(Token::Text(&from_tag[..=close]));
            continue;
        }
        out.push(Token::Tag {
            name,
            attrs: &inner[name_end..],
            closing,
        });
    }
    out
}

/// The index of the `>` that ends the tag at the start of `text`, quotes
/// respected.
fn find_tag_end(text: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, b) in text.bytes().enumerate().skip(1) {
        match (quote, b) {
            (Some(q), b) if b == q => quote = None,
            (None, b'"' | b'\'') => quote = Some(b),
            (None, b'>') => return Some(i),
            _ => {}
        }
    }
    None
}

/// The value of attribute `name` in `attrs`, entities decoded.
fn attribute(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let mut at = 0;
    while let Some(found) = lower[at..].find(name) {
        let start = at + found;
        let before_ok = start == 0 || {
            let b = lower.as_bytes()[start - 1];
            !(b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
        };
        let after = &attrs[start + name.len()..];
        let after_trimmed = after.trim_start();
        if before_ok && after_trimmed.starts_with('=') {
            let value = after_trimmed[1..].trim_start();
            let raw = match value.chars().next() {
                Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or(""),
                _ => value
                    .split(|c: char| c.is_whitespace())
                    .next()
                    .unwrap_or(""),
            };
            return Some(html_escape::decode_html_entities(raw).into_owned());
        }
        at = start + name.len();
    }
    None
}

/// Whether a link target is one the pane may offer to open.
fn safe_href(href: &str) -> bool {
    let lower = href.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    match lower.split_once(':') {
        Some((scheme, _))
            if scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') =>
        {
            matches!(scheme, "http" | "https" | "mailto" | "ftp")
        }
        _ => true,
    }
}

/// Collapses runs of whitespace into one space.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
                space = true;
            }
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

/// Renders HTML `text`.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn render(text: &str) -> Document {
    let mut builder = Builder::new();
    let tokens = tokens(text);
    let mut skip_until: Option<String> = None;
    let mut pre: Option<(Option<String>, String)> = None;
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        index += 1;
        if let Some(until) = &skip_until {
            if matches!(token, Token::Tag { name, closing: true, .. } if name == until) {
                skip_until = None;
            }
            continue;
        }
        if let Some((language, body)) = &mut pre {
            match token {
                Token::Text(text) => body.push_str(&html_escape::decode_html_entities(text)),
                Token::Tag {
                    name,
                    closing: true,
                    ..
                } if name == "pre" => {
                    let (language, body) = pre.take().unwrap_or_default();
                    builder.code_block(language.as_deref(), &body);
                }
                Token::Tag {
                    name,
                    attrs,
                    closing: false,
                } if name == "code" => {
                    if let Some(class) = attribute(attrs, "class")
                        && let Some(lang) = class.split_whitespace().find_map(|c| {
                            c.strip_prefix("language-")
                                .or_else(|| c.strip_prefix("lang-"))
                        })
                    {
                        *language = Some(lang.to_owned());
                    }
                }
                Token::Tag { name, .. } if name == "br" => body.push('\n'),
                Token::Tag { .. } => {}
            }
            continue;
        }
        match token {
            Token::Text(text) => {
                let decoded = html_escape::decode_html_entities(text);
                builder.text(&collapse(&decoded));
            }
            Token::Tag {
                name,
                attrs,
                closing,
            } => {
                let name = name.as_str();
                if DROPPED.contains(&name) {
                    if !closing && !matches!(name, "input") {
                        skip_until = Some(name.to_owned());
                    }
                    continue;
                }
                match (name, *closing) {
                    ("p", _) => builder.end_paragraph(),
                    ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                        builder.start_heading(name.as_bytes()[1] - b'0');
                    }
                    ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", true) => builder.end_heading(),
                    ("br", _) => builder.line_break(),
                    ("hr", _) => builder.rule(),
                    ("ul", false) => builder.start_list(false, 1),
                    ("ol", false) => {
                        let start = attribute(attrs, "start")
                            .and_then(|s| s.trim().parse().ok())
                            .unwrap_or(1);
                        builder.start_list(true, start);
                    }
                    ("ul" | "ol", true) => builder.end_list(),
                    ("li", false) => builder.start_item(),
                    ("li", true) => builder.end_item(),
                    ("blockquote", false) => builder.start_quote(),
                    ("blockquote", true) => builder.end_quote(),
                    ("pre", false) => {
                        builder.end_paragraph();
                        pre = Some((None, String::new()));
                    }
                    ("table", false) => builder.start_table(),
                    ("table", true) => builder.end_table(),
                    ("thead", false) => builder.start_head(),
                    ("thead", true) => builder.end_head(),
                    ("tr", false) => builder.start_row(),
                    ("tr", true) => builder.end_row(),
                    ("td" | "th", false) => builder.start_cell(name == "th"),
                    ("td" | "th", true) => builder.end_cell(),
                    ("tbody" | "tfoot" | "caption" | "colgroup" | "col", _) => {}
                    ("b" | "strong", false) => builder.push_style(Style::BOLD),
                    ("b" | "strong", true) => builder.pop_style(Style::BOLD),
                    ("i" | "em" | "cite" | "dfn" | "var", false) => {
                        builder.push_style(Style::ITALIC);
                    }
                    ("i" | "em" | "cite" | "dfn" | "var", true) => builder.pop_style(Style::ITALIC),
                    ("code" | "kbd" | "samp" | "tt", false) => builder.push_style(Style::CODE),
                    ("code" | "kbd" | "samp" | "tt", true) => builder.pop_style(Style::CODE),
                    ("s" | "del" | "strike", false) => builder.push_style(Style::STRIKETHROUGH),
                    ("s" | "del" | "strike", true) => builder.pop_style(Style::STRIKETHROUGH),
                    ("a", false) => {
                        if let Some(href) = attribute(attrs, "href").filter(|h| safe_href(h)) {
                            builder.start_link(href.trim());
                        }
                    }
                    ("a", true) => builder.end_link(),
                    ("img", _) => {
                        let alt = attribute(attrs, "alt").unwrap_or_default();
                        let src = attribute(attrs, "src").unwrap_or_default();
                        if !src.is_empty() {
                            builder.image(&alt, src.trim());
                        } else if !alt.is_empty() {
                            builder.text(&alt);
                        }
                    }
                    (tag, _) if BLOCK_BOUNDARY.contains(&tag) => builder.end_paragraph(),
                    _ => {}
                }
            }
        }
    }
    if let Some((language, body)) = pre.take() {
        builder.code_block(language.as_deref(), &body);
    }
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_split_tags_text_and_skip_comments() {
        let toks = tokens("a <b class=\"x>y\">bold</b><!-- no --> <br/> 1 < 2");
        assert_eq!(toks[0], Token::Text("a "));
        assert!(
            matches!(&toks[1], Token::Tag { name, attrs, closing: false } if name == "b" && attrs.contains("x>y"))
        );
        assert_eq!(toks[2], Token::Text("preview-code-bold"));
        assert!(matches!(&toks[3], Token::Tag { name, closing: true, .. } if name == "b"));
        assert!(matches!(&toks[5], Token::Tag { name, closing: false, .. } if name == "br"));
        assert!(
            toks.iter()
                .any(|t| matches!(t, Token::Text(text) if text.contains("< 2")))
        );
    }

    #[test]
    fn attributes_and_hrefs() {
        assert_eq!(
            attribute(" href='a.html' class=x", "href").as_deref(),
            Some("a.html")
        );
        assert_eq!(
            attribute(" data-href=\"no\" href=yes", "href").as_deref(),
            Some("yes")
        );
        assert_eq!(
            attribute(" alt=\"a &amp; b\"", "alt").as_deref(),
            Some("a & b")
        );
        assert_eq!(attribute(" class=x", "href"), None);
        assert!(safe_href("https://example.org"));
        assert!(safe_href("mailto:a@b.c"));
        assert!(safe_href("docs/page.html#top"));
        assert!(!safe_href("javascript:alert(1)"));
        assert!(!safe_href("  JavaScript:alert(1)"));
        assert!(!safe_href(""));
        assert_eq!(collapse("a \n\t b  c"), "a b c");
    }
}
