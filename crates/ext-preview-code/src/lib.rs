//! Source code previews: the spec's language list, highlighted with syntect
//! into coloured spans the UI lays out, in a theme that follows the request.
//!
//! Scores above the text previewer for a known extension or a shebang, so
//! code gets colour and prose stays plain. The text itself comes through the
//! same bounded window and encoding detection as plain text.

#![forbid(unsafe_code)]

use std::sync::{Arc, LazyLock};

use ext::{
    API_VERSION, Capabilities, Extension, Frame, Manifest, PreviewDoc, Previewer, Provider,
    Registry, RenderRequest, TextBlock, TextLayer, TextSpan, Transport, Version, async_trait,
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

/// An opened source file.
pub struct CodeDoc {
    text: String,
    layer: TextLayer,
    syntax: Option<&'static SyntaxReference>,
    language: Option<String>,
    encoding: &'static str,
    truncated: bool,
    bytes_read: usize,
    cached: Option<(bool, Vec<TextSpan>)>,
}

impl std::fmt::Debug for CodeDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeDoc")
            .field("language", &self.language)
            .field("bytes", &self.text.len())
            .finish_non_exhaustive()
    }
}

impl CodeDoc {
    /// Builds the document from the bytes read for `name`.
    #[must_use]
    pub fn new(name: &str, bytes: &[u8], truncated: bool) -> Self {
        let (text, encoding) = decode(bytes, truncated);
        let token = language_for(name);
        let first_line = text.lines().next().unwrap_or("");
        let syntax = syntax_for(token, first_line);
        Self {
            layer: TextLayer {
                pages: vec![text.clone()],
            },
            language: syntax.map(|s| s.name.clone()),
            text,
            syntax,
            encoding,
            truncated,
            bytes_read: bytes.len(),
            cached: None,
        }
    }
}

/// Decodes bytes the way the text previewer does.
fn decode(bytes: &[u8], truncated: bool) -> (String, &'static str) {
    if let Some((encoding, bom_len)) = encoding_rs::Encoding::for_bom(bytes) {
        let (text, _, _) = encoding.decode(&bytes[bom_len..]);
        return (text.into_owned(), encoding.name());
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_owned(), "UTF-8"),
        Err(error)
            if truncated
                && bytes.len() - error.valid_up_to() <= 3
                && error.error_len().is_none() =>
        {
            let (text, _) = bytes.split_at(error.valid_up_to());
            (String::from_utf8_lossy(text).into_owned(), "UTF-8")
        }
        Err(_) => {
            let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
            (text.into_owned(), "windows-1252")
        }
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
        let spans = match (&self.cached, self.syntax) {
            (Some((dark, spans)), _) if *dark == req.dark => spans.clone(),
            (_, Some(syntax)) => {
                let spans = highlight(&self.text, syntax, req.dark);
                self.cached = Some((req.dark, spans.clone()));
                spans
            }
            (_, None) => Vec::new(),
        };
        Ok(Frame::Text(TextBlock {
            text: self.text.clone(),
            language: self.language.clone(),
            spans,
        }))
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        Some(&self.layer)
    }

    fn info(&self) -> Vec<(String, String)> {
        let mut rows = vec![
            (
                "Language".to_owned(),
                self.language
                    .clone()
                    .unwrap_or_else(|| "Plain text".to_owned()),
            ),
            ("Encoding".to_owned(), self.encoding.to_owned()),
            ("Lines".to_owned(), self.text.lines().count().to_string()),
        ];
        if self.truncated {
            rows.push((
                "Shown".to_owned(),
                format!("first {} KiB", self.bytes_read / 1024),
            ));
        }
        rows
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
        let bytes = ext::read_bytes(&*provider, loc, Some(WINDOW + 1), &cancel).await?;
        let truncated = bytes.len() > WINDOW;
        let shown = &bytes[..bytes.len().min(WINDOW)];
        let name = loc
            .path
            .file_name()
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        Ok(Box::new(CodeDoc::new(&name, shown, truncated)))
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
        let mut doc = CodeDoc::new("lib.rs", b"fn main() {\n    let x = 1;\n}\n", false);
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
        let mut doc = CodeDoc::new("data.unknownext", b"just words\n", false);
        let Frame::Text(block) = doc.render(0, req(false)).unwrap_or_else(|e| panic!("{e}")) else {
            panic!("not text")
        };
        assert_eq!(block.spans, Vec::new());
        assert_eq!(block.language, None);
        assert!(
            doc.info()
                .iter()
                .any(|(k, v)| k == "Language" && v == "Plain text")
        );
    }
}
