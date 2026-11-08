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
//! Markdown and HTML are also rendered: a [`Document`] of headings,
//! paragraphs, lists, code, tables, links and images ([`markdown`],
//! [`html`]) that a pane which knows documents lays out, with images from
//! the file's own folder read through the provider. A Rendered toggle in
//! the toolbar, on for a newly opened file, switches between it and the
//! highlighted source; `render` always gives the source, for a pane that
//! shows no documents.

#![forbid(unsafe_code)]

pub mod document;
pub mod html;
pub mod markdown;

use std::sync::{Arc, LazyLock};

use ext::{
    API_VERSION, Capabilities, Content, ControlId, ControlValue, Document, Extension, Frame, Icon,
    InfoSection, Manifest, Platforms, Points, PreviewControl, PreviewDoc, Previewer, Provider,
    Reads, Registry, RenderRequest, TextBlock, TextLayer, TextSpan, Transport, Version,
    async_trait,
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

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// The Pretty print toggle's id.
pub const PRETTY: ControlId = ControlId("code.pretty");

/// The Rendered toggle's id: the rendered document rather than the source.
pub const RENDERED: ControlId = ControlId("code.rendered");

/// An image named by a document is read up to this size.
pub const IMAGE_LIMIT: usize = 16 * 1024 * 1024;

/// All images of a document together are read up to this size.
pub const IMAGES_LIMIT: usize = 64 * 1024 * 1024;

/// At most this many images of a document are read.
pub const MAX_IMAGES: usize = 50;

/// A markup the previewer renders as a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Markup {
    /// Markdown, through `pulldown-cmark`.
    Markdown,
    /// HTML, through the sanitising renderer.
    Html,
}

impl Markup {
    /// The markup for a syntect syntax name.
    #[must_use]
    pub fn for_language(language: &str) -> Option<Self> {
        match language {
            "Markdown" => Some(Self::Markdown),
            "HTML" | "HTML (Rails)" => Some(Self::Html),
            _ => None,
        }
    }

    /// Renders `text` as a document.
    #[must_use]
    pub fn render(self, text: &str) -> Document {
        match self {
            Self::Markdown => markdown::render(text),
            Self::Html => html::render(text),
        }
    }
}

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
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PREVIEWER,
                platforms: Platforms::ALL,
            },
        }
    }
}

impl Extension for CodePreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
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
// Four independent facts and toggles about one file: a byte order mark,
// a cut window, the pretty-print toggle and the rendered toggle.
#[allow(clippy::struct_excessive_bools)]
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
    markup: Option<Markup>,
    /// The Rendered toggle: the document rather than the source.
    rendered: bool,
    document: Option<Arc<Document>>,
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
        let markup = language.as_deref().and_then(Markup::for_language);
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
            markup,
            rendered: true,
            document: None,
        }
    }

    /// The markup this file is rendered as, if any.
    #[must_use]
    pub fn markup(&self) -> Option<Markup> {
        self.markup
    }

    /// The decoded text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Renders the document from the text, images unread; `None` for a
    /// file that is no markup.
    #[must_use]
    pub fn render_document(&self) -> Option<Document> {
        self.markup.map(|markup| markup.render(&self.text))
    }

    /// Sets the rendered document, images read.
    pub fn set_document(&mut self, document: Document) {
        self.document = Some(Arc::new(document));
    }

    /// The rendered document, when the file is markup.
    #[must_use]
    pub fn document(&self) -> Option<&Arc<Document>> {
        self.document.as_ref()
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

/// Decodes bytes the way the text previewer does; the flag says whether a
/// byte order mark led them.
fn decode(bytes: &[u8], truncated: bool) -> (String, &'static str, bool) {
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

    fn content(&self) -> Option<Content> {
        if !self.rendered {
            return None;
        }
        self.document
            .as_ref()
            .map(|document| Content::Document(Arc::clone(document)))
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
            title: "File info",
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
        if self.markup.is_some() {
            controls.push(PreviewControl::Toggle {
                id: RENDERED,
                icon: Icon::Rendered,
                label: "Rendered",
                on: self.rendered,
            });
        }
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
            (RENDERED, ControlValue::Bool(on), _) if self.markup.is_some() => {
                self.rendered = on;
                Ok(())
            }
            (RENDERED, _, _) => Err(Error::Unsupported("this format is not rendered")),
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
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        let mut doc = CodeDoc::new(&name, shown, truncated, size);
        if let Some(mut document) = doc.render_document() {
            load_images(&mut document, loc, &*provider, &cancel).await?;
            doc.set_document(document);
        }
        Ok(Box::new(doc))
    }
}

/// The path of an image source relative to the document, when it names a
/// file in the document's folder or below: no scheme, no leading slash, no
/// parent steps; the query and fragment dropped and percent-escapes
/// decoded.
#[must_use]
pub fn relative_source(source: &str) -> Option<String> {
    let source = source.trim();
    let source = source.split(['?', '#']).next().unwrap_or("");
    if source.is_empty()
        || source.starts_with('/')
        || source.starts_with('\\')
        || source.contains("://")
        || source.starts_with("data:")
        || source.starts_with("mailto:")
    {
        return None;
    }
    let decoded = percent_decode(source);
    if decoded.split(['/', '\\']).any(|part| part == "..") {
        return None;
    }
    Some(decoded)
}

/// Decodes `%XX` escapes; invalid ones stay as they are.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(value) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Reads the images a document names from the document's folder through
/// the provider, within the limits; a source that cannot be read keeps
/// `None` for its data.
async fn load_images(
    document: &mut Document,
    loc: &Location,
    provider: &dyn Provider,
    cancel: &CancelToken,
) -> Result<(), Error> {
    let Some(folder) = loc.path.parent() else {
        return Ok(());
    };
    let mut total = 0usize;
    let mut count = 0usize;
    for image in document::images_mut(document) {
        if count >= MAX_IMAGES || total >= IMAGES_LIMIT {
            break;
        }
        let Some(relative) = relative_source(&image.source) else {
            continue;
        };
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        count += 1;
        let path = folder.join(relative.as_bytes());
        let target = Location::new(loc.provider, path);
        match ext::read_bytes(provider, &target, Some(IMAGE_LIMIT + 1), cancel).await {
            Ok(bytes) if bytes.len() <= IMAGE_LIMIT => {
                total += bytes.len();
                image.data = Some(bytes.into());
            }
            Ok(_) => tracing::debug!(source = %image.source, "image larger than the limit"),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(error) => tracing::debug!(source = %image.source, %error, "image not read"),
        }
    }
    Ok(())
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
        assert_eq!((sections[0].id, sections[0].title), ("file", "File info"));
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

    const ALL_MARKDOWN: &str = "# Title\n\nA paragraph with *emphasis*, **strong**, ~~struck~~, `code` and a [link](https://example.org/page).  \nHard break above. Soft\nbreak here.\n\n## Lists\n\n- one\n- two\n  - nested\n\n3. third\n4. fourth\n\n- [x] done\n- [ ] open\n\n> A quote\n> with two lines\n\n```rust\nfn main() {}\n```\n\n    indented code\n\n| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\n![Alt text](picture.png)\n\nInline ![icon](icon%20one.png) here and <b>raw html</b>.\n\n---\n\n### The end\n";

    #[test]
    fn markdown_renders_every_element() {
        let doc = markdown::render(ALL_MARKDOWN);
        let blocks = &doc.blocks;
        assert!(
            matches!(&blocks[0], ext::Block::Heading { level: 1, text } if text == &[ext::Inline::Text { text: "Title".into(), style: ext::Style::empty() }]),
            "{:?}",
            blocks[0]
        );
        let ext::Block::Paragraph(inlines) = &blocks[1] else {
            panic!("{:?}", blocks[1]);
        };
        assert!(inlines.iter().any(|i| matches!(i, ext::Inline::Text { text, style } if text == "emphasis" && *style == ext::Style::ITALIC)));
        assert!(inlines.iter().any(|i| matches!(i, ext::Inline::Text { text, style } if text == "strong" && *style == ext::Style::BOLD)));
        assert!(inlines.iter().any(|i| matches!(i, ext::Inline::Text { text, style } if text == "struck" && *style == ext::Style::STRIKETHROUGH)));
        assert!(inlines.iter().any(|i| matches!(i, ext::Inline::Text { text, style } if text == "code" && *style == ext::Style::CODE)));
        assert!(inlines.iter().any(|i| matches!(i, ext::Inline::Link { text, href } if text == "link" && href == "https://example.org/page")));
        assert!(inlines.iter().any(|i| matches!(i, ext::Inline::Break)));
        assert!(
            inlines.iter().any(
                |i| matches!(i, ext::Inline::Text { text, .. } if text.contains("Soft break here"))
            ),
            "{inlines:?}"
        );
        assert!(matches!(&blocks[2], ext::Block::Heading { level: 2, .. }));
        let ext::Block::List {
            ordered: false,
            items,
            ..
        } = &blocks[3]
        else {
            panic!("{:?}", blocks[3]);
        };
        assert_eq!(items.len(), 2);
        assert!(
            items[1].iter().any(
                |b| matches!(b, ext::Block::List { ordered: false, items, .. } if items.len() == 1)
            ),
            "nested: {:?}",
            items[1]
        );
        assert!(
            matches!(&blocks[4], ext::Block::List { ordered: true, start: 3, items } if items.len() == 2),
            "{:?}",
            blocks[4]
        );
        let ext::Block::List { items, .. } = &blocks[5] else {
            panic!("{:?}", blocks[5]);
        };
        assert!(
            matches!(&items[0][0], ext::Block::Paragraph(p) if matches!(&p[0], ext::Inline::Text { text, .. } if text == "[x] done")),
            "{:?}",
            items[0]
        );
        assert!(
            matches!(&blocks[6], ext::Block::Quote(inner) if inner.len() == 1),
            "{:?}",
            blocks[6]
        );
        assert!(
            matches!(&blocks[7], ext::Block::Code { language: Some(l), text } if l == "rust" && text == "fn main() {}"),
            "{:?}",
            blocks[7]
        );
        assert!(
            matches!(&blocks[8], ext::Block::Code { language: None, text } if text == "indented code"),
            "{:?}",
            blocks[8]
        );
        let ext::Block::Table { header, rows } = &blocks[9] else {
            panic!("{:?}", blocks[9]);
        };
        assert_eq!(header.len(), 2);
        assert_eq!(rows.len(), 2);
        assert!(matches!(&rows[1][1][0], ext::Inline::Text { text, .. } if text == "4"));
        assert!(
            matches!(&blocks[10], ext::Block::Image(image) if image.alt == "Alt text" && image.source == "picture.png" && image.data.is_none()),
            "{:?}",
            blocks[10]
        );
        let ext::Block::Paragraph(inlines) = &blocks[11] else {
            panic!("{:?}", blocks[11]);
        };
        assert!(
            inlines
                .iter()
                .any(|i| matches!(i, ext::Inline::Image(image) if image.alt == "icon")),
            "{inlines:?}"
        );
        assert!(
            inlines
                .iter()
                .any(|i| matches!(i, ext::Inline::Text { text, .. } if text.contains("<b>"))),
            "raw html stays text: {inlines:?}"
        );
        assert!(matches!(&blocks[12], ext::Block::Rule));
        assert!(matches!(&blocks[13], ext::Block::Heading { level: 3, .. }));
        assert_eq!(blocks.len(), 14, "{blocks:?}");
    }

    #[test]
    fn odd_markup_renders_without_panics() {
        for text in [
            "",
            "#",
            "- ",
            "1.",
            "> > >",
            "| a |\n|",
            "```\nunterminated",
            "![]()",
            "[]()",
            "***",
            "<div><p>unclosed",
            "</p></li></ul></table>",
            "<table><td>cell without row</td></table>",
            "<ul>text outside items<li>a<ul><li>b</ul></ul>",
            "<pre><code class=\"language-rs\">let x = 1;</code>",
            "<a href=\"javascript:alert(1)\">x</a><img src=\"\"><img alt=\"only alt\">",
            "<!-- open comment",
            "<b><i>crossed</b></i>",
            "a < b && c > d",
            "<script>never</script>after<style>.x{}</style>",
        ] {
            let md = markdown::render(text);
            let html = html::render(text);
            assert!(md.blocks.len() < 100 && html.blocks.len() < 100, "{text}");
        }
    }

    const SAMPLE_HTML: &str = "<!DOCTYPE html><html><head><title>Nope</title><style>p{color:red}</style><script>alert(1)</script></head><body><h1>Heading &amp; more</h1><p>First <b>bold</b> and <em>italic</em> with a <a href=\"docs/a.html\">link</a> and <a href=\"javascript:x\">unsafe</a>.<br>Second line</p><ul><li>one</li><li>two<ol start=\"5\"><li>five</li></ol></li></ul><blockquote><p>quoted</p></blockquote><pre><code class=\"language-rust\">fn x() {}\n</code></pre><table><tr><th>H1</th><th>H2</th></tr><tr><td>a</td><td>b &lt; c</td></tr></table><div><img src=\"pic%20one.png\" alt=\"A picture\"></div><p>Text <img src=\"http://x/y.png\" alt=\"remote\"> inline</p><hr><section>Loose text</section></body></html>";

    #[test]
    fn html_renders_sanitised() {
        let doc = html::render(SAMPLE_HTML);
        let blocks = &doc.blocks;
        assert!(
            matches!(&blocks[0], ext::Block::Heading { level: 1, text } if matches!(&text[0], ext::Inline::Text { text, .. } if text == "Heading & more")),
            "{:?}",
            blocks[0]
        );
        let ext::Block::Paragraph(inlines) = &blocks[1] else {
            panic!("{:?}", blocks[1]);
        };
        assert!(inlines.iter().any(|i| matches!(i, ext::Inline::Text { text, style } if text == "bold" && *style == ext::Style::BOLD)));
        assert!(inlines.iter().any(|i| matches!(i, ext::Inline::Text { text, style } if text == "italic" && *style == ext::Style::ITALIC)));
        assert!(inlines.iter().any(|i| matches!(i, ext::Inline::Link { text, href } if text == "link" && href == "docs/a.html")));
        assert!(
            inlines
                .iter()
                .any(|i| matches!(i, ext::Inline::Text { text, .. } if text.contains("unsafe"))),
            "unsafe link kept as text: {inlines:?}"
        );
        assert!(!inlines.iter().any(
            |i| matches!(i, ext::Inline::Link { href, .. } if href.starts_with("javascript"))
        ));
        assert!(inlines.iter().any(|i| matches!(i, ext::Inline::Break)));
        let ext::Block::List {
            ordered: false,
            items,
            ..
        } = &blocks[2]
        else {
            panic!("{:?}", blocks[2]);
        };
        assert_eq!(items.len(), 2);
        assert!(
            items[1].iter().any(|b| matches!(
                b,
                ext::Block::List {
                    ordered: true,
                    start: 5,
                    ..
                }
            )),
            "{:?}",
            items[1]
        );
        assert!(matches!(&blocks[3], ext::Block::Quote(_)));
        assert!(
            matches!(&blocks[4], ext::Block::Code { language: Some(l), text } if l == "rust" && text == "fn x() {}"),
            "{:?}",
            blocks[4]
        );
        let ext::Block::Table { header, rows } = &blocks[5] else {
            panic!("{:?}", blocks[5]);
        };
        assert_eq!(header.len(), 2);
        assert_eq!(rows.len(), 1);
        assert!(
            matches!(&rows[0][1][0], ext::Inline::Text { text, .. } if text == "b < c"),
            "{:?}",
            rows[0]
        );
        assert!(
            matches!(&blocks[6], ext::Block::Image(image) if image.alt == "A picture" && image.source == "pic%20one.png"),
            "{:?}",
            blocks[6]
        );
        let ext::Block::Paragraph(inlines) = &blocks[7] else {
            panic!("{:?}", blocks[7]);
        };
        assert!(
            inlines.iter().any(
                |i| matches!(i, ext::Inline::Image(image) if image.source == "http://x/y.png")
            )
        );
        assert!(matches!(&blocks[8], ext::Block::Rule));
        assert!(
            matches!(&blocks[9], ext::Block::Paragraph(p) if matches!(&p[0], ext::Inline::Text { text, .. } if text == "Loose text")),
            "{:?}",
            blocks[9]
        );
        assert_eq!(blocks.len(), 10, "{blocks:?}");
        let flat = format!("{doc:?}");
        assert!(
            !flat.contains("alert") && !flat.contains("color:red") && !flat.contains("Nope"),
            "{flat}"
        );
    }

    #[test]
    fn relative_sources_stay_inside_the_folder() {
        assert_eq!(
            relative_source("pic%20one.png?x=1#top").as_deref(),
            Some("pic one.png")
        );
        assert_eq!(
            relative_source("sub/pic.png").as_deref(),
            Some("sub/pic.png")
        );
        assert_eq!(relative_source("./pic.png").as_deref(), Some("./pic.png"));
        assert_eq!(relative_source("../pic.png"), None);
        assert_eq!(relative_source("/etc/passwd"), None);
        assert_eq!(relative_source("https://x/y.png"), None);
        assert_eq!(relative_source("data:image/png;base64,AAAA"), None);
        assert_eq!(relative_source(""), None);
        assert_eq!(percent_decode("a%2Fb%zz"), "a/b%zz");
    }

    #[test]
    fn markdown_files_get_a_rendered_toggle_that_starts_on() {
        let mut doc = CodeDoc::new("README.md", b"# Hi\n\nText\n", false, None);
        assert_eq!(doc.markup(), Some(Markup::Markdown));
        assert_eq!(doc.text(), "# Hi\n\nText\n");
        assert!(doc.content().is_none(), "no document until it is set");
        let document = doc.render_document().unwrap_or_else(|| panic!("document"));
        doc.set_document(document);
        assert!(
            matches!(
                doc.controls()[0],
                PreviewControl::Toggle {
                    id: RENDERED,
                    icon: Icon::Rendered,
                    on: true,
                    ..
                }
            ),
            "{:?}",
            doc.controls()
        );
        let Some(Content::Document(rendered)) = doc.content() else {
            panic!("no content");
        };
        assert!(matches!(
            &rendered.blocks[0],
            ext::Block::Heading { level: 1, .. }
        ));
        assert!(doc.document().is_some());
        // The source is always there for the pane that shows no documents.
        let Frame::Text(block) = doc.render(0, req(false)).unwrap_or_else(|e| panic!("{e}")) else {
            panic!("not text");
        };
        assert_eq!(block.text, "# Hi\n\nText\n");
        doc.set_control(RENDERED, ControlValue::Bool(false))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(doc.content().is_none());
        assert!(matches!(
            doc.controls()[0],
            PreviewControl::Toggle {
                id: RENDERED,
                on: false,
                ..
            }
        ));
        let html = CodeDoc::new("page.html", b"<p>x</p>", false, None);
        assert_eq!(html.markup(), Some(Markup::Html));
        assert_eq!(
            html.controls().len(),
            2,
            "rendered and pretty print: {:?}",
            html.controls()
        );
        let mut rust = CodeDoc::new("main.rs", b"fn main() {}", false, None);
        assert_eq!(rust.markup(), None);
        assert!(rust.render_document().is_none());
        assert!(
            rust.set_control(RENDERED, ControlValue::Bool(false))
                .is_err()
        );
        assert_eq!(rust.controls(), []);
    }

    #[test]
    fn open_reads_the_images_of_the_folder_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        std::fs::create_dir(dir.path().join("img")).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join("img").join("a b.png"), b"\x89PNG fake")
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join("secret.txt"), b"outside").unwrap_or_else(|e| panic!("{e}"));
        let sub = dir.path().join("docs");
        std::fs::create_dir(&sub).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(sub.join("local.png"), b"\x89PNG local").unwrap_or_else(|e| panic!("{e}"));
        let md = sub.join("page.md");
        std::fs::write(
            &md,
            "![local](local.png)\n\n![up](../img/a%20b.png)\n\n![missing](nope.png)\n\n![web](https://example.org/x.png)\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
        let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&md));
        let doc = runtime::block_on(CodePreviewer.open(&loc, provider, CancelToken::new()))
            .unwrap_or_else(|e| panic!("{e}"));
        let Some(Content::Document(document)) = doc.content() else {
            panic!("no document");
        };
        let images: Vec<&ext::Image> = document
            .blocks
            .iter()
            .filter_map(|b| match b {
                ext::Block::Image(image) => Some(image),
                _ => None,
            })
            .collect();
        assert_eq!(images.len(), 4, "{:?}", document.blocks);
        assert_eq!(
            images[0].data.as_deref(),
            Some(&b"\x89PNG local"[..]),
            "a file next to the document"
        );
        assert!(images[1].data.is_none(), "no parent steps");
        assert!(images[2].data.is_none(), "missing files stay empty");
        assert!(images[3].data.is_none(), "no network");
        assert_eq!(doc.status()[0], "Markdown");
    }
}
