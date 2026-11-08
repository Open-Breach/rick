//! The core text previewer: any text file, encoding detected, a bounded
//! window into large files, and a text layer for find.
//!
//! The previewer scores a file from its first bytes and its name: a sniff
//! without NUL bytes that decodes as text scores as plain text; a known text
//! extension scores higher. `open` reads at most [`WINDOW`] bytes, detects
//! the encoding (BOMs, then UTF-8, then Windows-1252) and keeps the decoded
//! text; `render` returns it as a [`Frame::Text`] the UI lays out.

#![forbid(unsafe_code)]

use std::sync::Arc;

use ext::Text;
use ext::{
    API_VERSION, Capabilities, ControlId, ControlValue, Extension, Frame, Icon, InfoSection,
    Manifest, Platforms, Points, PreviewControl, PreviewDoc, Previewer, Provider, Reads, Registry,
    RenderRequest, TextBlock, TextLayer, Transport, Version, async_trait,
};
use types::{CancelToken, Entry, Error, ExtId, Location};

/// The extension id.
pub const ID: ExtId = ExtId("preview-text");

/// How much of a file is read: large files show their top.
pub const WINDOW: usize = 1024 * 1024;

/// File name extensions that are text whatever the content looks like.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt",
    "text",
    "md",
    "markdown",
    "log",
    "csv",
    "tsv",
    "ini",
    "cfg",
    "conf",
    "toml",
    "rst",
    "nfo",
    "diff",
    "patch",
    "srt",
    "vtt",
    "license",
    "readme",
    "gitignore",
    "env",
];

/// The extension: registers [`TextPreviewer`].
#[derive(Debug)]
pub struct TextPreview {
    manifest: Manifest,
}

impl Default for TextPreview {
    fn default() -> Self {
        Self::new()
    }
}

impl TextPreview {
    /// The extension with its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Text preview",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PREVIEWER,
                platforms: Platforms::ALL,
            },
        }
    }
}

impl Extension for TextPreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        reg.previewer(Arc::new(TextPreviewer));
        Ok(())
    }
}

/// Previews text files.
#[derive(Debug, Default)]
pub struct TextPreviewer;

/// The lower-case extension of a name, if any.
#[must_use]
pub fn extension(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// Whether the first bytes of a file look like text: no NUL, and what is
/// not valid UTF-8 is at least mostly printable single bytes.
#[must_use]
pub fn looks_textual(sniff: &[u8]) -> bool {
    if sniff.is_empty() {
        return true;
    }
    if has_utf16_bom(sniff) {
        return true;
    }
    if sniff.contains(&0) {
        return false;
    }
    match std::str::from_utf8(sniff) {
        Ok(text) => text.chars().all(|c| !c.is_control() || c.is_whitespace()),
        // A cut multi-byte sequence at the end is fine; anything invalid
        // earlier counts as binary unless it is a plausible 8-bit encoding.
        Err(error) => {
            let valid_up_to = error.valid_up_to();
            if sniff.len() - valid_up_to <= 3 && error.error_len().is_none() {
                return true;
            }
            let printable = sniff
                .iter()
                .filter(|b| b.is_ascii_graphic() || b.is_ascii_whitespace() || **b >= 0xA0)
                .count();
            printable * 100 / sniff.len() >= 95
        }
    }
}

fn has_utf16_bom(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF])
}

/// Decodes bytes to text: BOM-marked UTF-8, UTF-16 LE and BE, then UTF-8,
/// then Windows-1252 as the fallback that never fails. A multi-byte sequence
/// cut at the end counts as UTF-8 only when the window was `truncated`.
/// Returns the text and the encoding's name.
#[must_use]
pub fn decode(bytes: &[u8], truncated: bool) -> (String, &'static str) {
    let (text, encoding, _) = decode_with_bom(bytes, truncated);
    (text, encoding)
}

/// [`decode`], also saying whether a byte order mark led the bytes.
#[must_use]
pub fn decode_with_bom(bytes: &[u8], truncated: bool) -> (String, &'static str, bool) {
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
            // A multi-byte sequence cut by the window.
            let (text, _) = bytes.split_at(error.valid_up_to());
            (String::from_utf8_lossy(text).into_owned(), "UTF-8", false)
        }
        Err(_) => {
            let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
            (text.into_owned(), "windows-1252", false)
        }
    }
}

/// The line ending convention of `text`: `LF`, `CRLF`, `CR`, `Mixed`, or
/// `None` for a single line.
#[must_use]
pub fn line_ending(text: &str) -> &'static str {
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

/// What the text previewers know about a file, for the pane's File info
/// section and status bar: shared by the text and code previewers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFacts {
    /// The encoding's name.
    pub encoding: &'static str,
    /// A byte order mark led the file.
    pub bom: bool,
    /// The line ending convention.
    pub line_ending: &'static str,
    /// Lines in the shown text.
    pub lines: usize,
    /// Bytes read into the window.
    pub bytes_read: usize,
    /// The file's size, when the provider said.
    pub size: Option<u64>,
    /// The window cut the file short.
    pub truncated: bool,
}

impl FileFacts {
    /// The facts of decoded `text` read from `bytes_read` bytes.
    #[must_use]
    pub fn new(
        text: &str,
        encoding: &'static str,
        bom: bool,
        bytes_read: usize,
        size: Option<u64>,
        truncated: bool,
    ) -> Self {
        Self {
            encoding,
            bom,
            line_ending: line_ending(text),
            lines: text.lines().count(),
            bytes_read,
            size,
            truncated,
        }
    }

    /// The File info rows: size, encoding, line ending, BOM, and the window
    /// when the file was cut.
    #[must_use]
    pub fn rows(&self) -> Vec<(String, String)> {
        let size = match self.size {
            Some(size) => format_size(size),
            None => format_size(self.bytes_read as u64),
        };
        let mut rows = vec![
            ("Size".to_owned(), size),
            ("Encoding".to_owned(), self.encoding.to_owned()),
            ("Line ending".to_owned(), self.line_ending.to_owned()),
            (
                "BOM".to_owned(),
                if self.bom { "Yes" } else { "No" }.to_owned(),
            ),
            ("Lines".to_owned(), self.lines.to_string()),
        ];
        if self.truncated {
            rows.push((
                "Shown".to_owned(),
                format!("first {} KiB", self.bytes_read / 1024),
            ));
        }
        rows
    }

    /// The status bar facts after the language: encoding and line count.
    #[must_use]
    pub fn status(&self) -> Vec<String> {
        vec![
            self.encoding.to_owned(),
            format!(
                "{}{} lines",
                self.lines,
                if self.truncated { "+" } else { "" }
            ),
        ]
    }
}

/// A byte count as `n B`, `n.n KiB` or `n.n MiB`.
#[must_use]
pub fn format_size(bytes: u64) -> String {
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

/// An opened text file.
#[derive(Debug)]
pub struct TextDoc {
    text: String,
    layer: TextLayer,
    facts: FileFacts,
}

impl TextDoc {
    /// Builds the document from the bytes read, noting whether the window
    /// cut the file short; `size` is the file's size when known.
    #[must_use]
    pub fn new(bytes: &[u8], truncated: bool, size: Option<u64>) -> Self {
        let (text, encoding, bom) = decode_with_bom(bytes, truncated);
        let facts = FileFacts::new(&text, encoding, bom, bytes.len(), size, truncated);
        Self {
            layer: TextLayer {
                pages: vec![text.clone()],
            },
            text,
            facts,
        }
    }

    /// The decoded text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// What is known about the file.
    #[must_use]
    pub fn facts(&self) -> &FileFacts {
        &self.facts
    }
}

impl PreviewDoc for TextDoc {
    fn pages(&self) -> Option<usize> {
        None
    }

    fn render(&mut self, page: usize, _req: RenderRequest) -> Result<Frame, Error> {
        if page != 0 {
            return Err(Error::NotFound);
        }
        Ok(Frame::Text(TextBlock {
            text: self.text.clone(),
            language: None,
            spans: Vec::new(),
        }))
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        Some(&self.layer)
    }

    fn sections(&self) -> Vec<InfoSection> {
        vec![InfoSection {
            id: "file",
            title: Text("preview-text-file-info"),
            icon: Icon::Info,
            rows: self.facts.rows(),
        }]
    }

    fn status(&self) -> Vec<String> {
        let mut status = vec!["Plain text".to_owned()];
        status.extend(self.facts.status());
        status
    }

    fn controls(&self) -> Vec<PreviewControl> {
        Vec::new()
    }

    fn set_control(&mut self, _id: ControlId, _value: ControlValue) -> Result<(), Error> {
        Err(Error::Unsupported("plain text has no controls of its own"))
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

#[async_trait]
impl Previewer for TextPreviewer {
    fn id(&self) -> ExtId {
        ID
    }

    /// The start of a file is enough: a remote file is served as a prefix
    /// with *Load more*.
    fn reads(&self) -> Reads {
        Reads::Prefix
    }

    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8 {
        if entry.kind != types::EntryKind::File {
            return 0;
        }
        let named = extension(&entry.name)
            .is_some_and(|ext| TEXT_EXTENSIONS.contains(&ext.as_str()))
            || TEXT_EXTENSIONS.contains(&entry.name.to_ascii_lowercase().as_str());
        let textual = looks_textual(sniff);
        match (named, textual) {
            (true, true) => 80,
            (true, false) => 30,
            (false, true) => 40,
            (false, false) => 0,
        }
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
        Ok(Box::new(TextDoc::new(shown, truncated, size)))
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

    #[test]
    fn scores_text_by_sniff_and_name() {
        let p = TextPreviewer;
        assert_eq!(p.score(&entry("notes.txt"), b"hello\nworld\n"), 80);
        assert_eq!(p.score(&entry("notes"), b"hello\nworld\n"), 40);
        assert_eq!(p.score(&entry("README"), b"hello"), 80);
        assert_eq!(
            p.score(&entry("photo.jpg"), b"\xFF\xD8\xFF\xE0\x00\x10JFIF"),
            0
        );
        assert_eq!(p.score(&entry("notes.txt"), b"\x00\x01\x02"), 30);
        assert_eq!(p.score(&entry("empty.bin"), b""), 40);
        assert_eq!(p.score(&entry("utf16.txt"), b"\xFF\xFEh\x00i\x00"), 80);
        let mut dir = entry("dir");
        dir.kind = EntryKind::Dir;
        assert_eq!(p.score(&dir, b""), 0);
    }

    #[test]
    fn decodes_boms_utf8_and_a_single_byte_fallback() {
        assert_eq!(
            decode("héllo".as_bytes(), false),
            ("héllo".to_owned(), "UTF-8")
        );
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("hi".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(decode(&utf16, false), ("hi".to_owned(), "UTF-16LE"));
        let utf8_bom = [0xEF, 0xBB, 0xBF, b'o', b'k'];
        assert_eq!(decode(&utf8_bom, false), ("ok".to_owned(), "UTF-8"));
        assert_eq!(
            decode(b"caf\xE9", false),
            ("café".to_owned(), "windows-1252")
        );
        // A multi-byte character cut by the window is dropped, not mangled.
        let cut = &"héllo".as_bytes()[..2];
        assert_eq!(decode(cut, true), ("h".to_owned(), "UTF-8"));
        assert_eq!(decode(cut, false).1, "windows-1252");
    }

    #[test]
    fn the_doc_renders_text_and_reports_truncation() {
        let mut doc = TextDoc::new(b"one\ntwo\n", true, None);
        assert_eq!(doc.pages(), None);
        let frame = doc
            .render(
                0,
                RenderRequest {
                    width: 10,
                    height: 10,
                    scale: 1.0,
                    dark: false,
                },
            )
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            matches!(frame, Frame::Text(TextBlock { text, language: None, spans }) if text == "one\ntwo\n" && spans.is_empty())
        );
        assert_eq!(doc.text_layer().map(|l| l.pages.len()), Some(1));
        let sections = doc.sections();
        assert_eq!(sections.len(), 1);
        assert_eq!(
            (sections[0].id, sections[0].title.key()),
            ("file", "preview-text-file-info")
        );
        let info = &sections[0].rows;
        assert!(info.iter().any(|(k, v)| k == "Lines" && v == "2"));
        assert!(info.iter().any(|(k, _)| k == "Shown"));
        assert!(
            doc.render(
                1,
                RenderRequest {
                    width: 1,
                    height: 1,
                    scale: 1.0,
                    dark: true
                }
            )
            .is_err()
        );
    }

    #[test]
    fn open_reads_a_bounded_window_through_the_provider() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let big = dir.path().join("big.log");
        let line = "0123456789abcdef\n";
        let body: String = std::iter::repeat_n(line, (WINDOW / line.len()) + 100).collect();
        std::fs::write(&big, &body).unwrap_or_else(|e| panic!("{e}"));
        let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
        let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&big));
        let doc = runtime::block_on(TextPreviewer.open(&loc, provider, CancelToken::new()))
            .unwrap_or_else(|e| panic!("{e}"));
        let rows = doc.sections().remove(0).rows;
        assert!(rows.iter().any(|(k, _)| k == "Shown"), "{rows:?}");
        let whole_file = format_size(body.len() as u64);
        assert!(
            rows.iter().any(|(k, v)| k == "Size" && *v == whole_file),
            "the size is the file's ({whole_file}), not the window's: {rows:?}"
        );
        let text_len = doc.text_layer().map_or(0, |l| l.pages[0].len());
        assert!(text_len <= WINDOW && text_len > WINDOW - 64, "{text_len}");
        assert!(doc.status()[2].ends_with("+ lines"), "{:?}", doc.status());
    }

    #[test]
    fn sections_status_and_controls_describe_the_file() {
        let mut doc = TextDoc::new(b"\xEF\xBB\xBFone\r\ntwo\r\n", false, Some(13));
        let section = doc.sections().remove(0);
        assert_eq!(section.icon, Icon::Info);
        let row = |key: &str| {
            section.rows.iter().find(|(k, _)| k == key).map_or_else(
                || panic!("no row {key}: {:?}", section.rows),
                |(_, v)| v.clone(),
            )
        };
        assert_eq!(row("Size"), "13 B");
        assert_eq!(row("Encoding"), "UTF-8");
        assert_eq!(row("Line ending"), "CRLF");
        assert_eq!(row("BOM"), "Yes");
        assert_eq!(row("Lines"), "2");
        assert!(!section.rows.iter().any(|(k, _)| k == "Shown"));
        assert_eq!(doc.status(), ["Plain text", "UTF-8", "2 lines"]);
        assert_eq!(doc.controls().len(), 0);
        assert!(
            doc.set_control(ControlId("x"), ControlValue::Pressed)
                .is_err()
        );
        assert_eq!(line_ending("a\nb\r\nc"), "Mixed");
        assert_eq!(line_ending("a\rb"), "CR");
        assert_eq!(line_ending("one line"), "None");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MiB");
    }
}
