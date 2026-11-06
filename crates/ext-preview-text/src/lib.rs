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

use ext::{
    API_VERSION, Capabilities, Extension, Frame, Manifest, PreviewDoc, Previewer, Provider,
    Registry, RenderRequest, TextBlock, TextLayer, Transport, Version, async_trait,
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
            },
        }
    }
}

impl Extension for TextPreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
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
            // A multi-byte sequence cut by the window.
            let (text, _) = bytes.split_at(error.valid_up_to());
            (String::from_utf8_lossy(text).into_owned(), "UTF-8")
        }
        Err(_) => {
            let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
            (text.into_owned(), "windows-1252")
        }
    }
}

/// An opened text file.
#[derive(Debug)]
pub struct TextDoc {
    text: String,
    layer: TextLayer,
    encoding: &'static str,
    bytes_read: usize,
    truncated: bool,
    lines: usize,
}

impl TextDoc {
    /// Builds the document from the bytes read, noting whether the window
    /// cut the file short.
    #[must_use]
    pub fn new(bytes: &[u8], truncated: bool) -> Self {
        let (text, encoding) = decode(bytes, truncated);
        let lines = text.lines().count();
        Self {
            layer: TextLayer {
                pages: vec![text.clone()],
            },
            text,
            encoding,
            bytes_read: bytes.len(),
            truncated,
            lines,
        }
    }

    /// The decoded text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
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

    fn info(&self) -> Vec<(String, String)> {
        let mut rows = vec![
            ("Encoding".to_owned(), self.encoding.to_owned()),
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

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

#[async_trait]
impl Previewer for TextPreviewer {
    fn id(&self) -> ExtId {
        ID
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
        let bytes = ext::read_bytes(&*provider, loc, Some(WINDOW + 1), &cancel).await?;
        let truncated = bytes.len() > WINDOW;
        let shown = &bytes[..bytes.len().min(WINDOW)];
        Ok(Box::new(TextDoc::new(shown, truncated)))
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
        let mut doc = TextDoc::new(b"one\ntwo\n", true);
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
        let info = doc.info();
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
        assert!(
            doc.info().iter().any(|(k, _)| k == "Shown"),
            "{:?}",
            doc.info()
        );
        let text_len = doc.text_layer().map_or(0, |l| l.pages[0].len());
        assert!(text_len <= WINDOW && text_len > WINDOW - 64, "{text_len}");
    }
}
