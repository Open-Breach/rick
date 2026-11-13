//! Source code previews: the spec's language list, highlighted with syntect
//! into coloured spans the UI lays out, in a theme that follows the request.
//!
//! Scores above the text previewer for a known extension or a shebang, so
//! code gets colour and prose stays plain. The text itself comes through the
//! same bounded window and encoding detection as plain text. JSON, XML and
//! HTML carry a Pretty print control: a view transformation that reformats
//! the shown text with one element per line, off for every newly opened
//! file; invalid input shows the raw text with the error's line in red.
//!
//! Markdown and HTML are highlighted as source here, like any other
//! language of the list: that is the view `ext-preview-markup` toggles to
//! from its rendered document (it builds on [`CodeDoc`] for it) and what
//! shows when that plugin is disabled (SPEC.md "Preview support").

#![forbid(unsafe_code)]

use std::sync::{Arc, LazyLock};

use ext::Text;
use ext::{
    API_VERSION, Capabilities, ControlId, ControlValue, Extension, Frame, Icon, InfoSection,
    Manifest, Platforms, Points, PreviewControl, PreviewDoc, Previewer, Provider, Reads, Registry,
    RenderRequest, TextBlock, TextLayer, TextSpan, Transport, Version, async_trait,
};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use types::{CancelToken, Entry, Error, ExtId, Location};

/// The extension id.
pub const ID: ExtId = ExtId("preview-code");

/// How much of a file is read: large files show their top.
pub const WINDOW: usize = 1024 * 1024;

/// Extensions of the spec's code list, lower case, mapped to the syntect
/// token that names their syntax.
const LANGUAGES: &[(&str, &str)] = &[
    ("js", "js"),
    ("mjs", "js"),
    ("cjs", "js"),
    ("jsx", "js"),
    ("ts", "ts"),
    ("tsx", "ts"),
    ("java", "java"),
    ("rs", "rs"),
    ("cs", "cs"),
    ("fs", "fs"),
    ("fsx", "fs"),
    ("vb", "vb"),
    ("c", "c"),
    ("h", "c"),
    ("cpp", "cpp"),
    ("cc", "cpp"),
    ("cxx", "cpp"),
    ("hpp", "cpp"),
    ("hh", "cpp"),
    ("pl", "pl"),
    ("pm", "pl"),
    ("php", "php"),
    ("json", "json"),
    ("xml", "xml"),
    ("yaml", "yaml"),
    ("yml", "yaml"),
    ("html", "html"),
    ("htm", "html"),
    ("md", "md"),
    ("markdown", "md"),
    ("toml", "toml"),
    ("sh", "sh"),
    ("bash", "sh"),
    ("py", "py"),
    ("rb", "rb"),
    ("go", "go"),
    ("css", "css"),
    ("sql", "sql"),
];

/// The file name extensions the manifest claims: the keys of [`LANGUAGES`].
const PREVIEWS: &[&str] = &[
    "js", "mjs", "cjs", "jsx", "ts", "tsx", "java", "rs", "cs", "fs", "fsx", "vb", "c", "h", "cpp",
    "cc", "cxx", "hpp", "hh", "pl", "pm", "php", "json", "xml", "yaml", "yml", "html", "htm", "md",
    "markdown", "toml", "sh", "bash", "py", "rb", "go", "css", "sql",
];

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// The Pretty print toggle's id.
pub const PRETTY: ControlId = ControlId("code.pretty");

/// The colour of the line a parse error points at.
const ERROR_RED: [u8; 4] = [0xD3, 0x2F, 0x2F, 0xFF];

/// The extension: registers [`CodePreviewer`].
#[derive(Debug)]
pub struct CodePreview {
    manifest: Manifest,
}

impl Default for CodePreview {
    fn default() -> Self {
        Self::new()
    }
}

impl CodePreview {
    /// The extension with its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Code preview",
                description: "Previews source code with syntax highlighting, and shows the source of Markdown and HTML files.",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PREVIEWER,
                platforms: Platforms::ALL,
                previews: PREVIEWS,
                commands: &[],
            },
        }
    }
}

impl Extension for CodePreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        reg.previewer(Arc::new(CodePreviewer));
        Ok(())
    }
}

/// Previews source files with syntax colours.
#[derive(Debug, Default)]
pub struct CodePreviewer;

/// The syntect token for a file name's extension, from the spec's list.
#[must_use]
pub fn language_for(name: &str) -> Option<&'static str> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    let ext = ext.to_ascii_lowercase();
    LANGUAGES
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, token)| *token)
}

/// Tokens syntect's bundled syntaxes lack, with the nearest syntax that
/// still colours them sensibly (QUESTIONS.md).
const FALLBACKS: &[(&str, &str)] = &[("ts", "js"), ("tsx", "js")];

/// The syntax for a token or a first line, if syntect knows it.
fn syntax_for(token: Option<&str>, first_line: &str) -> Option<&'static SyntaxReference> {
    let set = &*SYNTAXES;
    let by_token = |t: &str| {
        set.find_syntax_by_token(t)
            .or_else(|| set.find_syntax_by_extension(t))
    };
    token
        .and_then(|t| {
            by_token(t).or_else(|| {
                FALLBACKS
                    .iter()
                    .find(|(from, _)| *from == t)
                    .and_then(|(_, to)| by_token(to))
            })
        })
        .or_else(|| set.find_syntax_by_first_line(first_line))
}

/// The theme for a light or dark UI.
fn theme(dark: bool) -> &'static Theme {
    let name = if dark {
        "base16-ocean.dark"
    } else {
        "InspiredGitHub"
    };
    THEMES
        .themes
        .get(name)
        .or_else(|| THEMES.themes.values().next())
        .unwrap_or_else(|| {
            // ThemeSet::load_defaults always carries themes; this is unreachable
            // but keeps the function total.
            static EMPTY: LazyLock<Theme> = LazyLock::new(Theme::default);
            &EMPTY
        })
}

/// Highlights `text` with `syntax` into spans, in the theme for `dark`.
#[must_use]
pub fn highlight(text: &str, syntax: &SyntaxReference, dark: bool) -> Vec<TextSpan> {
    let mut highlighter = HighlightLines::new(syntax, theme(dark));
    let mut spans = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let Ok(ranges) = highlighter.highlight_line(line, &SYNTAXES) else {
            break;
        };
        for (style, piece) in ranges {
            let start = offset;
            offset += piece.len();
            if piece.trim().is_empty() {
                continue;
            }
            spans.push(TextSpan {
                start,
                end: offset,
                color: [
                    style.foreground.r,
                    style.foreground.g,
                    style.foreground.b,
                    style.foreground.a,
                ],
                bold: style.font_style.contains(FontStyle::BOLD),
                italic: style.font_style.contains(FontStyle::ITALIC),
            });
        }
    }
    spans
}

/// Which pretty printer a syntax gets, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Formatter {
    /// `serde_json`, two-space indent, key order kept.
    Json,
    /// A tag-per-line indenter for XML and HTML.
    Markup,
}

/// The formatter for a syntect syntax name.
#[must_use]
pub fn formatter_for(language: &str) -> Option<Formatter> {
    match language {
        "JSON" => Some(Formatter::Json),
        "XML" | "HTML" | "HTML (Rails)" => Some(Formatter::Markup),
        _ => None,
    }
}

/// The result of pretty printing: the text to show and, when the input did
/// not parse, the message and the byte offset of the line it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pretty {
    /// The reformatted text, or the raw text when it did not parse.
    pub text: String,
    /// The parse error and the byte offset of its line in `text`.
    pub error: Option<(String, usize)>,
}

/// Reformats `text` with `formatter`.
#[must_use]
pub fn pretty_print(text: &str, formatter: Formatter) -> Pretty {
    match formatter {
        Formatter::Json => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(value) => Pretty {
                text: serde_json::to_string_pretty(&value).unwrap_or_else(|_| text.to_owned()),
                error: None,
            },
            Err(error) => {
                let line = error.line().max(1);
                let offset = text
                    .split_inclusive('\n')
                    .take(line - 1)
                    .map(str::len)
                    .sum();
                Pretty {
                    text: text.to_owned(),
                    error: Some((error.to_string(), offset)),
                }
            }
        },
        Formatter::Markup => Pretty {
            text: indent_markup(text),
            error: None,
        },
    }
}

/// Tags one per line, children indented two spaces: enough for minified XML
/// and HTML. Text between tags keeps its own line; void and self-closing
/// tags, comments, declarations and `<script>`/`<style>` bodies do not
/// change the depth.
#[must_use]
pub fn indent_markup(text: &str) -> String {
    const VOID: &[&str] = &[
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr", "!doctype",
    ];
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    let mut depth: usize = 0;
    let mut rest = text;
    let mut raw_until: Option<&str> = None;
    let push = |depth: usize, piece: &str, out: &mut String| {
        if piece.is_empty() {
            return;
        }
        out.extend(std::iter::repeat_n("  ", depth));
        out.push_str(piece);
        out.push('\n');
    };
    while !rest.is_empty() {
        if let Some(closer) = raw_until {
            // Inside <script> or <style>: everything up to the closer is text.
            let end = rest.to_ascii_lowercase().find(closer).unwrap_or(rest.len());
            let (body, tail) = rest.split_at(end);
            for line in body.lines() {
                push(depth, line.trim(), &mut out);
            }
            rest = tail;
            raw_until = None;
            continue;
        }
        let Some(open) = rest.find('<') else {
            push(depth, rest.trim(), &mut out);
            break;
        };
        let (before, from_tag) = rest.split_at(open);
        push(depth, before.trim(), &mut out);
        let Some(close) = from_tag.find('>') else {
            push(depth, from_tag.trim(), &mut out);
            break;
        };
        let (tag, tail) = from_tag.split_at(close + 1);
        rest = tail;
        let inner = tag[1..tag.len() - 1].trim();
        let lower = inner.to_ascii_lowercase();
        let name = lower
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or("");
        if inner.starts_with('/') {
            depth = depth.saturating_sub(1);
            push(depth, tag, &mut out);
        } else if inner.starts_with('!')
            || inner.starts_with('?')
            || inner.ends_with('/')
            || VOID.contains(&name)
        {
            push(depth, tag, &mut out);
        } else {
            push(depth, tag, &mut out);
            depth += 1;
            if name == "script" || name == "style" {
                raw_until = Some(if name == "script" {
                    "</script"
                } else {
                    "</style"
                });
            }
        }
    }
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

/// An opened source file.
pub struct CodeDoc {
    text: String,
    layer: TextLayer,
    syntax: Option<&'static SyntaxReference>,
    language: Option<String>,
    encoding: &'static str,
    bom: bool,
    truncated: bool,
    bytes_read: usize,
    size: Option<u64>,
    formatter: Option<Formatter>,
    pretty: bool,
    /// The pretty-printed text, cached while the toggle is on.
    formatted: Option<Pretty>,
    cached: Option<(bool, bool, Vec<TextSpan>)>,
}

impl std::fmt::Debug for CodeDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeDoc")
            .field("language", &self.language)
            .field("bytes", &self.text.len())
            .field("pretty", &self.pretty)
            .finish_non_exhaustive()
    }
}

impl CodeDoc {
    /// Builds the document from the bytes read for `name`; `size` is the
    /// file's size when known.
    #[must_use]
    pub fn new(name: &str, bytes: &[u8], truncated: bool, size: Option<u64>) -> Self {
        let (text, encoding, bom) = decode(bytes, truncated);
        let token = language_for(name);
        let first_line = text.lines().next().unwrap_or("");
        let syntax = syntax_for(token, first_line);
        let language = syntax.map(|s| s.name.clone());
        Self {
            layer: TextLayer {
                pages: vec![text.clone()],
            },
            formatter: language.as_deref().and_then(formatter_for),
            language,
            text,
            syntax,
            encoding,
            bom,
            truncated,
            bytes_read: bytes.len(),
            size,
            pretty: false,
            formatted: None,
            cached: None,
        }
    }

    /// The decoded text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The syntax's name, when syntect knows the file's language.
    #[must_use]
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// The text as shown: pretty printed when the toggle is on and the
    /// formatter is happy, raw otherwise.
    fn shown(&mut self) -> (String, Option<(String, usize)>) {
        match (self.pretty, self.formatter) {
            (true, Some(formatter)) => {
                let pretty = self
                    .formatted
                    .get_or_insert_with(|| pretty_print(&self.text, formatter));
                (pretty.text.clone(), pretty.error.clone())
            }
            _ => (self.text.clone(), None),
        }
    }
}

/// Decodes bytes the way the text previewer does: BOM-marked UTF-8 and
/// UTF-16, then UTF-8, then Windows-1252, a multi-byte sequence cut at the
/// end accepted when the window was `truncated`; the flag says whether a
/// byte order mark led them.
#[must_use]
pub fn decode(bytes: &[u8], truncated: bool) -> (String, &'static str, bool) {
    if let Some((encoding, bom_len)) = encoding_rs::Encoding::for_bom(bytes) {
        let (text, _, _) = encoding.decode(&bytes[bom_len..]);
        return (text.into_owned(), encoding.name(), true);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_owned(), "UTF-8", false),
        Err(error)
            if truncated
                && bytes.len() - error.valid_up_to() <= 3
                && error.error_len().is_none() =>
        {
            let (text, _) = bytes.split_at(error.valid_up_to());
            (String::from_utf8_lossy(text).into_owned(), "UTF-8", false)
        }
        Err(_) => {
            let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
            (text.into_owned(), "windows-1252", false)
        }
    }
}

/// The line ending convention of `text`.
fn line_ending(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    let cr = text.matches('\r').count() - crlf;
    match (crlf > 0, lf > 0, cr > 0) {
        (false, false, false) => "None",
        (true, false, false) => "CRLF",
        (false, true, false) => "LF",
        (false, false, true) => "CR",
        _ => "Mixed",
    }
}

/// A byte count as `n B`, `n.n KiB` or `n.n MiB`.
fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    // Sizes shown to one decimal; the precision loss is immaterial.
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if value < KIB * KIB {
        format!("{:.1} KiB", value / KIB)
    } else if value < KIB * KIB * KIB {
        format!("{:.1} MiB", value / (KIB * KIB))
    } else {
        format!("{:.2} GiB", value / (KIB * KIB * KIB))
    }
}

impl PreviewDoc for CodeDoc {
    fn pages(&self) -> Option<usize> {
        None
    }

    fn render(&mut self, page: usize, req: RenderRequest) -> Result<Frame, Error> {
        if page != 0 {
            return Err(Error::NotFound);
        }
        let (text, error) = self.shown();
        let pretty = self.pretty && error.is_none();
        let mut spans = match (&self.cached, self.syntax) {
            (Some((dark, was_pretty, spans)), _) if *dark == req.dark && *was_pretty == pretty => {
                spans.clone()
            }
            (_, Some(syntax)) => {
                let spans = highlight(&text, syntax, req.dark);
                self.cached = Some((req.dark, pretty, spans.clone()));
                spans
            }
            (_, None) => Vec::new(),
        };
        if let Some((_, offset)) = error {
            // The raw text with the error's line in red, over any colours.
            let end = text[offset..].find('\n').map_or(text.len(), |n| offset + n);
            spans.retain(|s| s.end <= offset || s.start >= end);
            if end > offset {
                spans.push(TextSpan {
                    start: offset,
                    end,
                    color: ERROR_RED,
                    bold: true,
                    italic: false,
                });
                spans.sort_by_key(|s| s.start);
            }
        }
        Ok(Frame::Text(TextBlock {
            text,
            language: self.language.clone(),
            spans,
        }))
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        Some(&self.layer)
    }

    fn sections(&self) -> Vec<InfoSection> {
        let size = self.size.unwrap_or(self.bytes_read as u64);
        let mut rows = vec![
            (
                "Language".to_owned(),
                self.language
                    .clone()
                    .unwrap_or_else(|| "Plain text".to_owned()),
            ),
            ("Size".to_owned(), format_size(size)),
            ("Encoding".to_owned(), self.encoding.to_owned()),
            ("Line ending".to_owned(), line_ending(&self.text).to_owned()),
            (
                "BOM".to_owned(),
                if self.bom { "Yes" } else { "No" }.to_owned(),
            ),
            ("Lines".to_owned(), self.text.lines().count().to_string()),
        ];
        if self.truncated {
            rows.push((
                "Shown".to_owned(),
                format!("first {} KiB", self.bytes_read / 1024),
            ));
        }
        if let Some(Pretty {
            error: Some((message, _)),
            ..
        }) = &self.formatted
            && self.pretty
        {
            rows.push(("Pretty print".to_owned(), message.clone()));
        }
        vec![InfoSection {
            id: "file",
            title: Text("preview-code-file-info"),
            icon: Icon::Info,
            rows,
        }]
    }

    fn status(&self) -> Vec<String> {
        vec![
            self.language
                .clone()
                .unwrap_or_else(|| "Plain text".to_owned()),
            self.encoding.to_owned(),
            format!(
                "{}{} lines",
                self.text.lines().count(),
                if self.truncated { "+" } else { "" }
            ),
        ]
    }

    fn controls(&self) -> Vec<PreviewControl> {
        let mut controls = Vec::new();
        if self.formatter.is_some() {
            controls.push(PreviewControl::Toggle {
                id: PRETTY,
                icon: Icon::PrettyPrint,
                label: "Pretty print",
                on: self.pretty,
            });
        }
        controls
    }

    fn set_control(&mut self, id: ControlId, value: ControlValue) -> Result<(), Error> {
        match (id, value, self.formatter) {
            (PRETTY, ControlValue::Bool(on), Some(_)) => {
                self.pretty = on;
                Ok(())
            }
            (PRETTY, _, None) => Err(Error::Unsupported("this format has no pretty printer")),
            _ => Err(Error::Unsupported("no such control")),
        }
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

#[async_trait]
impl Previewer for CodePreviewer {
    fn id(&self) -> ExtId {
        ID
    }

    /// The start of a file is enough: a remote file is served as a prefix
    /// with *Load more*.
    fn reads(&self) -> Reads {
        Reads::Prefix
    }

    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8 {
        if entry.kind != types::EntryKind::File || sniff.contains(&0) {
            return 0;
        }
        if language_for(&entry.name).is_some() {
            return 120;
        }
        let first_line = std::str::from_utf8(sniff)
            .ok()
            .and_then(|s| s.lines().next())
            .unwrap_or("");
        if first_line.starts_with("#!") && SYNTAXES.find_syntax_by_first_line(first_line).is_some()
        {
            return 100;
        }
        0
    }

    async fn open(
        &self,
        loc: &Location,
        provider: Arc<dyn Provider>,
        cancel: CancelToken,
    ) -> Result<Box<dyn PreviewDoc>, Error> {
        let size = provider.stat(loc).await.ok().and_then(|e| e.size);
        let bytes = ext::read_bytes(&*provider, loc, Some(WINDOW + 1), &cancel).await?;
        let truncated = bytes.len() > WINDOW;
        let shown = &bytes[..bytes.len().min(WINDOW)];
        let name = loc
            .path
            .file_name()
            .map_or_default(|n| String::from_utf8_lossy(n).into_owned());
        Ok(Box::new(CodeDoc::new(&name, shown, truncated, size)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Attrs, EntryId, EntryKind, ProviderId};

    fn entry(name: &str) -> Entry {
        Entry {
            id: EntryId::new(ProviderId("t"), 1),
            name: name.to_owned(),
            path: format!("/{name}").into(),
            kind: EntryKind::File,
            size: Some(1),
            created: None,
            modified: None,
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        }
    }

    fn req(dark: bool) -> RenderRequest {
        RenderRequest {
            width: 100,
            height: 100,
            scale: 1.0,
            dark,
        }
    }

    /// syntect's bundled set has no TypeScript, F#, Visual Basic or TOML;
    /// TypeScript falls back to JavaScript, the others render plain.
    #[test]
    fn every_language_in_the_list_has_a_syntax_or_a_known_gap() {
        let missing: Vec<&str> = LANGUAGES
            .iter()
            .filter(|(_, token)| syntax_for(Some(token), "").is_none())
            .map(|(ext, _)| *ext)
            .collect();
        assert_eq!(
            missing,
            ["fs", "fsx", "vb", "toml"],
            "unexpected gaps: {missing:?}"
        );
        assert_eq!(
            syntax_for(Some("ts"), "").map(|s| s.name.as_str()),
            Some("JavaScript")
        );
    }

    #[test]
    fn scores_code_by_extension_or_shebang_above_plain_text() {
        let p = CodePreviewer;
        assert_eq!(p.score(&entry("main.rs"), b"fn main() {}"), 120);
        assert_eq!(p.score(&entry("Main.JAVA"), b"class Main {}"), 120);
        assert_eq!(p.score(&entry("run"), b"#!/bin/bash\necho hi\n"), 100);
        assert_eq!(p.score(&entry("notes.txt"), b"hello"), 0);
        assert_eq!(p.score(&entry("blob.rs"), b"\x00\x01"), 0);
    }

    #[test]
    fn rust_source_gets_coloured_spans_in_both_themes() {
        let mut doc = CodeDoc::new("lib.rs", b"fn main() {\n    let x = 1;\n}\n", false, None);
        assert_eq!(doc.language.as_deref(), Some("Rust"));
        let Frame::Text(block) = doc.render(0, req(true)).unwrap_or_else(|e| panic!("{e}")) else {
            panic!("not text")
        };
        assert_ne!(block.spans.len(), 0);
        assert!(
            block
                .spans
                .iter()
                .all(|s| s.start < s.end && s.end <= block.text.len())
        );
        assert!(
            block.spans.windows(2).all(|w| w[0].end <= w[1].start),
            "spans overlap"
        );
        let fn_span = block
            .spans
            .iter()
            .find(|s| &block.text[s.start..s.end] == "fn")
            .unwrap_or_else(|| panic!("fn keyword"));
        let Frame::Text(light) = doc.render(0, req(false)).unwrap_or_else(|e| panic!("{e}")) else {
            panic!("not text")
        };
        let fn_light = light
            .spans
            .iter()
            .find(|s| &light.text[s.start..s.end] == "fn")
            .unwrap_or_else(|| panic!("fn keyword"));
        assert_ne!(fn_span.color, fn_light.color, "themes differ");
        assert_eq!(block.language.as_deref(), Some("Rust"));
    }

    #[test]
    fn unknown_extensions_render_plain() {
        let mut doc = CodeDoc::new("data.unknownext", b"just words\n", false, None);
        let Frame::Text(block) = doc.render(0, req(false)).unwrap_or_else(|e| panic!("{e}")) else {
            panic!("not text")
        };
        assert_eq!(block.spans, Vec::new());
        assert_eq!(block.language, None);
        assert!(
            doc.sections()[0]
                .rows
                .iter()
                .any(|(k, v)| k == "Language" && v == "Plain text")
        );
        assert_eq!(doc.status()[0], "Plain text");
        assert_eq!(doc.controls().len(), 0, "no pretty printer for prose");
    }

    #[test]
    fn sections_and_status_describe_the_file() {
        let doc = CodeDoc::new("a.rs", b"fn a() {}\r\nfn b() {}\r\n", false, Some(2048));
        let sections = doc.sections();
        assert_eq!(sections.len(), 1);
        assert_eq!(
            (sections[0].id, sections[0].title.key()),
            ("file", "preview-code-file-info")
        );
        let row = |key: &str| {
            sections[0]
                .rows
                .iter()
                .find(|(k, _)| k == key)
                .map_or_else(|| panic!("no row {key}"), |(_, v)| v.as_str())
        };
        assert_eq!(row("Language"), "Rust");
        assert_eq!(row("Size"), "2.0 KiB");
        assert_eq!(row("Encoding"), "UTF-8");
        assert_eq!(row("Line ending"), "CRLF");
        assert_eq!(row("BOM"), "No");
        assert_eq!(row("Lines"), "2");
        assert_eq!(doc.status(), ["Rust", "UTF-8", "2 lines"]);
    }

    #[test]
    fn json_gets_a_pretty_print_toggle_that_starts_off_and_reformats() {
        let mut doc = CodeDoc::new("d.json", br#"{"a":[1,2],"b":{"c":true}}"#, false, None);
        assert_eq!(
            doc.controls(),
            [PreviewControl::Toggle {
                id: PRETTY,
                icon: Icon::PrettyPrint,
                label: "Pretty print",
                on: false,
            }]
        );
        let Frame::Text(raw) = doc.render(0, req(false)).unwrap_or_else(|e| panic!("{e}")) else {
            panic!("not text")
        };
        assert_eq!(raw.text, r#"{"a":[1,2],"b":{"c":true}}"#);
        doc.set_control(PRETTY, ControlValue::Bool(true))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            doc.controls()[0],
            PreviewControl::Toggle { on: true, .. }
        ));
        let Frame::Text(pretty) = doc.render(0, req(false)).unwrap_or_else(|e| panic!("{e}"))
        else {
            panic!("not text")
        };
        assert_eq!(
            pretty.text,
            "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": {\n    \"c\": true\n  }\n}"
        );
        assert!(!pretty.spans.is_empty(), "the pretty text is highlighted");
        assert!(
            pretty.spans.iter().all(|s| s.end <= pretty.text.len()),
            "spans fit the pretty text"
        );
        // The file itself and the text layer stay as they were.
        assert_eq!(
            doc.text_layer().map(|l| l.pages[0].as_str()),
            Some(r#"{"a":[1,2],"b":{"c":true}}"#)
        );
        doc.set_control(PRETTY, ControlValue::Bool(false))
            .unwrap_or_else(|e| panic!("{e}"));
        let Frame::Text(back) = doc.render(0, req(false)).unwrap_or_else(|e| panic!("{e}")) else {
            panic!("not text")
        };
        assert_eq!(back.text, raw.text);
        assert!(
            doc.set_control(ControlId("nope"), ControlValue::Pressed)
                .is_err()
        );
    }

    #[test]
    fn invalid_json_stays_raw_with_the_error_line_in_red() {
        let mut doc = CodeDoc::new("bad.json", b"{\n  \"a\": 1,\n  oops\n}\n", false, None);
        doc.set_control(PRETTY, ControlValue::Bool(true))
            .unwrap_or_else(|e| panic!("{e}"));
        let Frame::Text(block) = doc.render(0, req(false)).unwrap_or_else(|e| panic!("{e}")) else {
            panic!("not text")
        };
        assert_eq!(block.text, "{\n  \"a\": 1,\n  oops\n}\n", "raw");
        let red = block
            .spans
            .iter()
            .find(|s| s.color == ERROR_RED)
            .unwrap_or_else(|| panic!("no red span: {:?}", block.spans));
        assert_eq!(&block.text[red.start..red.end], "  oops");
        assert!(
            doc.sections()[0]
                .rows
                .iter()
                .any(|(k, v)| k == "Pretty print" && v.contains("line 3")),
            "{:?}",
            doc.sections()[0].rows
        );
    }

    #[test]
    fn markup_is_indented_one_tag_per_line() {
        let html = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Hi</title></head><body><p>a<br>b</p><script>if (a<b) {x()}</script></body></html>";
        assert_eq!(
            indent_markup(html),
            "<!DOCTYPE html>\n<html>\n  <head>\n    <meta charset=\"utf-8\">\n    <title>\n      Hi\n    </title>\n  </head>\n  <body>\n    <p>\n      a\n      <br>\n      b\n    </p>\n    <script>\n      if (a<b) {x()}\n    </script>\n  </body>\n</html>"
        );
        assert_eq!(
            indent_markup("<a><b/><c>x</c></a>"),
            "<a>\n  <b/>\n  <c>\n    x\n  </c>\n</a>"
        );
        assert_eq!(formatter_for("XML"), Some(Formatter::Markup));
        assert_eq!(formatter_for("HTML"), Some(Formatter::Markup));
        assert_eq!(formatter_for("JSON"), Some(Formatter::Json));
        assert_eq!(formatter_for("Rust"), None);
        let doc = CodeDoc::new("x.xml", b"<a><b/></a>", false, None);
        assert_eq!(doc.controls().len(), 1);
    }
}
