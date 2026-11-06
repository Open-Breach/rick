//! The Previewer extension point: rendering a file type.
//!
//! A previewer scores how well it handles an entry from its metadata and the
//! first 4 KiB of content; the highest score wins. `open` produces a
//! [`PreviewDoc`] with pages, a `render` that yields frames the UI uploads as
//! textures (or a text block it lays out itself), a text layer for find,
//! metadata rows, and a transport for media.
//!
//! # Worked example
//!
//! A previewer for files whose name ends in `.txt`, rendering a text block.
//! Reading the content would go through the provider's `open_read`; this one
//! shows the entry's name to keep the example short.
//!
//! ```
//! use std::sync::Arc;
//!
//! use ext::{Frame, PreviewDoc, Previewer, Provider, RenderRequest, TextBlock, TextLayer, Transport, async_trait};
//! use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, ExtId, Location, ProviderId};
//!
//! struct PlainText;
//!
//! struct TextDoc {
//!     body: String,
//!     layer: TextLayer,
//! }
//!
//! impl PreviewDoc for TextDoc {
//!     fn pages(&self) -> Option<usize> {
//!         None
//!     }
//!     fn render(&mut self, _page: usize, _req: RenderRequest) -> Result<Frame, Error> {
//!         Ok(Frame::Text(TextBlock { text: self.body.clone(), language: None, spans: Vec::new() }))
//!     }
//!     fn text_layer(&self) -> Option<&TextLayer> {
//!         Some(&self.layer)
//!     }
//!     fn info(&self) -> Vec<(String, String)> {
//!         vec![("Characters".into(), self.body.chars().count().to_string())]
//!     }
//!     fn transport(&mut self) -> Option<&mut dyn Transport> {
//!         None
//!     }
//! }
//!
//! #[async_trait]
//! impl Previewer for PlainText {
//!     fn id(&self) -> ExtId {
//!         ExtId("preview-text")
//!     }
//!
//!     fn score(&self, entry: &Entry, sniff: &[u8]) -> u8 {
//!         let named = entry.name.ends_with(".txt");
//!         let looks_textual = sniff.iter().all(|b| !b.is_ascii_control() || b.is_ascii_whitespace());
//!         match (named, looks_textual) {
//!             (true, _) => 200,
//!             (false, true) => 20,
//!             (false, false) => 0,
//!         }
//!     }
//!
//!     async fn open(
//!         &self,
//!         loc: &Location,
//!         _provider: Arc<dyn Provider>,
//!         _cancel: CancelToken,
//!     ) -> Result<Box<dyn PreviewDoc>, Error> {
//!         let body = loc.path.to_string_lossy().into_owned();
//!         Ok(Box::new(TextDoc { layer: TextLayer { pages: vec![body.clone()] }, body }))
//!     }
//! }
//!
//! let entry = Entry {
//!     id: EntryId::new(ProviderId("local"), 1),
//!     name: "notes.txt".into(),
//!     path: "/notes.txt".into(),
//!     kind: EntryKind::File,
//!     size: Some(5),
//!     created: None,
//!     modified: None,
//!     accessed: None,
//!     attrs: Attrs::empty(),
//!     mime: None,
//! };
//! assert_eq!(PlainText.score(&entry, b"hello"), 200);
//! assert_eq!(PlainText.score(&Entry { name: "a.bin".into(), ..entry.clone() }, b"\x00\x01"), 0);
//! ```

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use types::{CancelToken, Entry, Error, ExtId, Location};

use crate::provider::Provider;

/// What size a frame is wanted at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderRequest {
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Display scale factor, 1.0 for 96 dpi.
    pub scale: f32,
    /// Whether the UI shows a dark theme, for previewers that colour text.
    pub dark: bool,
}

/// A GPU texture the UI already holds, for previewers that upload themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureId(pub u64);

/// Text the UI lays out and highlights.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBlock {
    /// The text.
    pub text: String,
    /// Syntax to highlight with, e.g. `"rust"`, when known.
    pub language: Option<String>,
    /// Coloured runs over `text`, in order and non-overlapping; empty for
    /// plain text in the UI's default colour.
    pub spans: Vec<TextSpan>,
}

/// One coloured run of a [`TextBlock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSpan {
    /// Byte offset of the first character.
    pub start: usize,
    /// Byte offset past the last character.
    pub end: usize,
    /// Foreground colour, RGBA.
    pub color: [u8; 4],
    /// Bold weight.
    pub bold: bool,
    /// Italic style.
    pub italic: bool,
}

/// Searchable text of a document, one string per page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextLayer {
    /// Page texts; a single-surface document has one.
    pub pages: Vec<String>,
}

/// One rendered page.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// Pixels, tightly packed RGBA8, row-major.
    Rgba {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// `width * height * 4` bytes.
        data: Arc<[u8]>,
    },
    /// Text for the UI to lay out.
    Text(TextBlock),
    /// A texture the previewer uploaded itself.
    External(TextureId),
}

/// Playback control for audio and video previews.
pub trait Transport {
    /// Starts or resumes playback.
    fn play(&mut self);
    /// Pauses playback.
    fn pause(&mut self);
    /// Jumps to a position.
    fn seek(&mut self, to: Duration);
    /// Current position.
    fn position(&self) -> Duration;
    /// Total length, when known.
    fn duration(&self) -> Option<Duration>;
    /// `true` while playing.
    fn playing(&self) -> bool;
}

/// An opened document the inspector or viewer shows.
pub trait PreviewDoc: Send {
    /// Number of pages; `None` for a single surface.
    fn pages(&self) -> Option<usize>;

    /// Produces a frame for one page at the requested size; the UI uploads it
    /// to a texture. Text documents return [`Frame::Text`] and let the UI lay
    /// out.
    ///
    /// # Errors
    ///
    /// Fails when the page does not exist or cannot be decoded.
    fn render(&mut self, page: usize, req: RenderRequest) -> Result<Frame, Error>;

    /// Text for find, when the document has any.
    fn text_layer(&self) -> Option<&TextLayer>;

    /// Metadata rows for the info panel.
    fn info(&self) -> Vec<(String, String)>;

    /// Playback control; media only.
    fn transport(&mut self) -> Option<&mut dyn Transport>;
}

/// Turns an entry into something the inspector or viewer can show.
#[async_trait]
pub trait Previewer: Send + Sync + 'static {
    /// The extension this previewer belongs to.
    fn id(&self) -> ExtId;

    /// 0 means cannot handle; the highest score wins. `sniff` is the first
    /// 4 KiB of the file.
    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8;

    /// Opens the entry through its provider.
    async fn open(
        &self,
        loc: &Location,
        provider: Arc<dyn Provider>,
        cancel: CancelToken,
    ) -> Result<Box<dyn PreviewDoc>, Error>;
}

/// Reads an entry's content through its provider, at most `limit` bytes when
/// given, stopping with `Error::Cancelled` when the token fires. Previewers
/// use it for the file and the kernel for the 4 KiB sniff.
///
/// # Errors
///
/// The provider's error, or `Error::Cancelled`.
pub async fn read_bytes(
    provider: &dyn Provider,
    loc: &Location,
    limit: Option<usize>,
    cancel: &CancelToken,
) -> Result<Vec<u8>, Error> {
    use tokio::io::AsyncReadExt as _;

    let mut reader = provider.open_read(loc).await?;
    let mut out = Vec::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let wanted = match limit {
            Some(limit) if out.len() >= limit => break,
            Some(limit) => buffer.len().min(limit - out.len()),
            None => buffer.len(),
        };
        let read = reader
            .read(&mut buffer[..wanted])
            .await
            .map_err(Error::from_io)?;
        if read == 0 {
            break;
        }
        out.extend_from_slice(&buffer[..read]);
    }
    Ok(out)
}
