//! The hex view: a file's bytes as a hex dump with an ASCII column.
//!
//! The previewer claims no file on its own: it scores every file 0, so it
//! is never an automatic fallback (a file nobody claims shows the pane's
//! placeholder) and is chosen per type in settings like any other
//! previewer (SPEC.md, Preview, *No previewer*). `open` reads at
//! most [`WINDOW`] bytes; `render` returns the dump as a [`Frame::Text`] the
//! pane lays out line by line, so a long dump costs only its visible rows;
//! the dump doubles as the text layer, so find works over offsets, hex and
//! characters alike. Nothing here writes to the file.
//!
//! This crate is a plugin: it builds as a `cdylib` that the plugin host
//! loads from `plugins/preview-hex/` through the entry point
//! [`ext::plugin!`] declares, and as an `rlib` for the binary's opt-in
//! `preview-hex` feature and the tests.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

use std::fmt::Write as _;
use std::sync::Arc;

use ext::Text;
use ext::{
    API_VERSION, Capabilities, ControlId, ControlValue, Extension, Frame, Icon, InfoSection,
    Manifest, Platforms, Points, PreviewControl, PreviewDoc, Previewer, Provider, Reads, Registry,
    RenderRequest, TextBlock, TextLayer, Transport, Version, async_trait,
};
use types::{CancelToken, Entry, Error, ExtId, Location};

/// The extension id.
pub const ID: ExtId = ExtId("preview-hex");

/// The score for every file: none. The hex view claims nothing by itself
/// and is chosen per type in settings.
pub const SCORE: u8 = 0;

/// How much of a file is read: large files show their start.
pub const WINDOW: usize = 1024 * 1024;

/// Bytes per row of the dump.
pub const ROW: usize = 16;

/// The extension: registers [`HexPreviewer`].
#[derive(Debug)]
pub struct HexPreview {
    manifest: Manifest,
}

impl Default for HexPreview {
    fn default() -> Self {
        Self::new()
    }
}

impl HexPreview {
    /// The extension with its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Hex preview",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PREVIEWER,
                platforms: Platforms::ALL,
                previews: &[],
                commands: &[],
            },
        }
    }
}

impl Extension for HexPreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        reg.previewer(Arc::new(HexPreviewer));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(HexPreview::new()) as Arc<dyn Extension>);

/// Previews any file as a hex dump.
#[derive(Debug, Default)]
pub struct HexPreviewer;

#[async_trait]
impl Previewer for HexPreviewer {
    fn id(&self) -> ExtId {
        ID
    }

    /// The start of a file is enough: a remote file is served as a prefix
    /// with *Load more*.
    fn reads(&self) -> Reads {
        Reads::Prefix
    }

    fn score(&self, _entry: &Entry, _sniff: &[u8]) -> u8 {
        SCORE
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
        let total = size.unwrap_or(shown.len() as u64);
        tracing::debug!(path = %loc.path.to_string_lossy(), bytes = shown.len(), total, "hex: opened");
        Ok(Box::new(HexDoc::new(shown, total, truncated)))
    }
}

/// The dump of one file.
#[derive(Debug)]
pub struct HexDoc {
    shown: usize,
    total: u64,
    truncated: bool,
    layer: TextLayer,
}

impl HexDoc {
    /// The dump of `bytes`, the first `bytes.len()` of `total`.
    #[must_use]
    pub fn new(bytes: &[u8], total: u64, truncated: bool) -> Self {
        Self {
            shown: bytes.len(),
            total,
            truncated,
            layer: TextLayer {
                pages: vec![dump(bytes)],
            },
        }
    }

    /// The dump's text.
    #[must_use]
    pub fn text(&self) -> &str {
        self.layer.pages.first().map_or("", String::as_str)
    }
}

/// The hex dump of `bytes`: per row the offset in hex, the bytes as hex in
/// two groups of eight, and the printable ASCII with a dot for the rest.
#[must_use]
pub fn dump(bytes: &[u8]) -> String {
    let width = format!("{:x}", bytes.len().saturating_sub(1).max(1))
        .len()
        .max(8);
    let mut out = String::with_capacity(bytes.len() / ROW * (width + 3 * ROW + 6) + 1);
    for (row, chunk) in bytes.chunks(ROW).enumerate() {
        // Writing into a String cannot fail.
        let _ = write!(out, "{:0width$x}  ", row * ROW);
        for (i, byte) in chunk.iter().enumerate() {
            if i == ROW / 2 {
                out.push(' ');
            }
            let _ = write!(out, "{byte:02x} ");
        }
        for i in chunk.len()..ROW {
            if i == ROW / 2 {
                out.push(' ');
            }
            out.push_str("   ");
        }
        out.push_str(" |");
        for byte in chunk {
            out.push(if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                '.'
            });
        }
        out.push_str("|\n");
    }
    if bytes.is_empty() {
        out.push_str("(empty file)\n");
    }
    out
}

/// `1.5 KiB`-style sizes.
// A size shown to one decimal: the precision lost past 2^53 bytes is not
// shown anyway.
#[allow(clippy::cast_precision_loss)]
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

impl PreviewDoc for HexDoc {
    fn pages(&self) -> Option<usize> {
        None
    }

    fn render(&mut self, _page: usize, _req: RenderRequest) -> Result<Frame, Error> {
        Ok(Frame::Text(TextBlock {
            text: self.text().to_owned(),
            language: None,
            spans: Vec::new(),
        }))
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        Some(&self.layer)
    }

    fn sections(&self) -> Vec<InfoSection> {
        let mut rows = vec![("Size".to_owned(), format_size(self.total))];
        if self.truncated {
            rows.push(("Shown".to_owned(), format_size(self.shown as u64)));
        }
        rows.push(("Rows".to_owned(), self.shown.div_ceil(ROW).to_string()));
        vec![InfoSection {
            id: "file",
            title: Text("preview-hex-file-info"),
            icon: Icon::Info,
            rows,
        }]
    }

    fn status(&self) -> Vec<String> {
        let mut facts = vec!["Hex".to_owned(), format_size(self.total)];
        if self.truncated {
            facts.push(format!("first {} shown", format_size(self.shown as u64)));
        }
        facts
    }

    fn controls(&self) -> Vec<PreviewControl> {
        Vec::new()
    }

    fn set_control(&mut self, _id: ControlId, _value: ControlValue) -> Result<(), Error> {
        Err(Error::Unsupported("the hex view has no controls"))
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dump_has_offsets_hex_and_an_ascii_column() {
        let text = dump(b"Hello, world! \x00\x01\x02ABC");
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some("00000000  48 65 6c 6c 6f 2c 20 77  6f 72 6c 64 21 20 00 01  |Hello, world! ..|")
        );
        assert_eq!(
            lines.next(),
            Some("00000010  02 41 42 43                                       |.ABC|")
        );
        assert_eq!(lines.next(), None);
        assert_eq!(dump(b"").trim(), "(empty file)");
        let doc = HexDoc::new(b"abc", 3, false);
        assert_eq!(doc.status(), ["Hex", "3 B"]);
        assert_eq!(
            doc.sections()[0].rows[0],
            ("Size".to_owned(), "3 B".to_owned())
        );
        assert!(
            doc.text_layer()
                .is_some_and(|l| l.pages[0].contains("|abc|"))
        );
        let big = HexDoc::new(&[0; 32], 5 * 1024 * 1024, true);
        assert_eq!(big.status(), ["Hex", "5.0 MiB", "first 32 B shown"]);
    }

    #[test]
    fn no_file_is_claimed_by_itself() {
        let entry = Entry {
            id: types::EntryId::new(types::ProviderId("local"), 1),
            name: "x.bin".into(),
            path: "/x.bin".into(),
            kind: types::EntryKind::File,
            size: Some(0),
            created: None,
            modified: None,
            accessed: None,
            attrs: types::Attrs::empty(),
            mime: None,
        };
        assert_eq!(HexPreviewer.score(&entry, b"\x00\xff"), SCORE);
        assert_eq!(HexPreviewer.score(&entry, b"text"), SCORE);
    }

    #[test]
    fn open_reads_through_the_provider_and_renders_text() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("blob.bin");
        std::fs::write(&path, [0u8, 1, 2, 3, 0x41, 0x42]).unwrap_or_else(|e| panic!("{e}"));
        let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
        let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&path));
        let mut doc = runtime::block_on(HexPreviewer.open(&loc, provider, CancelToken::new()))
            .unwrap_or_else(|e| panic!("{e}"));
        let req = RenderRequest {
            width: 100,
            height: 100,
            scale: 1.0,
            dark: false,
        };
        match doc.render(0, req) {
            Ok(Frame::Text(block)) => assert!(block.text.contains("|....AB|"), "{}", block.text),
            other => panic!("{other:?}"),
        }
        assert_eq!(doc.status(), ["Hex", "6 B"]);
    }
}
