//! The Previewer extension point: rendering a file type.
//!
//! A previewer scores how well it handles an entry from its metadata and the
//! first 4 KiB of content; the highest score wins. `open` produces a
//! [`PreviewDoc`] with pages, a `render` that yields frames the UI uploads as
//! textures (or a text block it lays out itself), a text layer for find,
//! metadata in [`InfoSection`]s for the pane's info badges, one-line facts
//! for its status bar, the previewer's own toolbar [`PreviewControl`]s, and a
//! transport for media. The pane builds its toolbar from the kind of document
//! (text layer, pages, raster) plus those controls; a previewer never draws.
//!
//! # Worked example
//!
//! A previewer for files whose name ends in `.txt`, rendering a text block
//! with one control of its own: an "Upper case" toggle. Reading the content
//! would go through the provider's `open_read`; this one shows the entry's
//! name to keep the example short.
//!
//! ```
//! use std::sync::Arc;
//!
//! use ext::{
//!     ControlId, ControlValue, Frame, Icon, InfoSection, PreviewControl, PreviewDoc, Previewer,
//!     Provider, RenderRequest, TextBlock, TextLayer, Transport, async_trait,
//! };
//! use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, ExtId, Location, ProviderId};
//!
//! struct PlainText;
//!
//! const UPPER: ControlId = ControlId("text.upper");
//!
//! struct TextDoc {
//!     body: String,
//!     layer: TextLayer,
//!     upper: bool,
//! }
//!
//! impl PreviewDoc for TextDoc {
//!     fn pages(&self) -> Option<usize> {
//!         None
//!     }
//!     fn render(&mut self, _page: usize, _req: RenderRequest) -> Result<Frame, Error> {
//!         let text = if self.upper { self.body.to_uppercase() } else { self.body.clone() };
//!         Ok(Frame::Text(TextBlock { text, language: None, spans: Vec::new() }))
//!     }
//!     fn text_layer(&self) -> Option<&TextLayer> {
//!         Some(&self.layer)
//!     }
//!     fn sections(&self) -> Vec<InfoSection> {
//!         vec![InfoSection {
//!             id: "file",
//!             title: "File info",
//!             icon: Icon::Info,
//!             rows: vec![("Characters".into(), self.body.chars().count().to_string())],
//!         }]
//!     }
//!     fn status(&self) -> Vec<String> {
//!         vec!["Plain text".into(), format!("{} lines", self.body.lines().count())]
//!     }
//!     fn controls(&self) -> Vec<PreviewControl> {
//!         vec![PreviewControl::Toggle { id: UPPER, icon: Icon::TextCase, label: "Upper case", on: self.upper }]
//!     }
//!     fn set_control(&mut self, id: ControlId, value: ControlValue) -> Result<(), Error> {
//!         match (id, value) {
//!             (UPPER, ControlValue::Bool(on)) => {
//!                 self.upper = on;
//!                 Ok(())
//!             }
//!             _ => Err(Error::Unsupported("no such control")),
//!         }
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
//!         Ok(Box::new(TextDoc { layer: TextLayer { pages: vec![body.clone()] }, body, upper: false }))
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
//!
//! let mut doc = TextDoc { body: "hi".into(), layer: TextLayer::default(), upper: false };
//! assert_eq!(doc.sections()[0].title, "File info");
//! assert!(matches!(doc.controls()[0], PreviewControl::Toggle { on: false, .. }));
//! doc.set_control(UPPER, ControlValue::Bool(true)).unwrap();
//! let req = RenderRequest { width: 10, height: 10, scale: 1.0, dark: false };
//! assert!(matches!(doc.render(0, req), Ok(Frame::Text(block)) if block.text == "HI"));
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

/// A glyph the pane draws for a section badge or a control. The pane maps
/// each to its icon set; extensions name the meaning, never the font.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Icon {
    /// General information about the file.
    Info,
    /// Camera metadata: EXIF, IPTC, XMP.
    Camera,
    /// Version control.
    Branch,
    /// Audio or video stream details.
    Film,
    /// A tag or label list.
    Tag,
    /// Pretty print: reformat structured text.
    PrettyPrint,
    /// Letter case.
    TextCase,
    /// Line numbers.
    LineNumbers,
    /// Word wrap.
    Wrap,
    /// Fit to the pane.
    Fit,
    /// Actual pixels.
    ActualSize,
    /// A step backwards or to the previous item.
    Previous,
    /// A step forwards or to the next item.
    Next,
    /// Rotate.
    Rotate,
    /// Zoom in.
    ZoomIn,
    /// Zoom out.
    ZoomOut,
}

/// A group of metadata rows behind one info badge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoSection {
    /// Stable id, unique within the document: `"file"`, `"exif"`. The pane
    /// remembers which id is pinned per content kind.
    pub id: &'static str,
    /// The badge's tooltip and the panel's heading.
    pub title: &'static str,
    /// The badge's glyph.
    pub icon: Icon,
    /// Key and value rows, in display order.
    pub rows: Vec<(String, String)>,
}

/// Identity of a previewer's toolbar control: `"code.pretty"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlId(pub &'static str);

/// What a control was set to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlValue {
    /// A toggle's new state.
    Bool(bool),
    /// A button was pressed.
    Pressed,
}

/// A toolbar control a previewer adds to the pane's standard ones. Icons
/// only; the label is the tooltip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewControl {
    /// On or off, drawn highlighted when on.
    Toggle {
        /// The control's id, passed back to [`PreviewDoc::set_control`].
        id: ControlId,
        /// The glyph.
        icon: Icon,
        /// The tooltip.
        label: &'static str,
        /// Current state.
        on: bool,
    },
    /// A one-shot action.
    Button {
        /// The control's id, passed back to [`PreviewDoc::set_control`].
        id: ControlId,
        /// The glyph.
        icon: Icon,
        /// The tooltip.
        label: &'static str,
        /// Greyed out when `false`.
        enabled: bool,
    },
}

/// An opened document the preview pane or viewer shows.
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

    /// Metadata, grouped for the pane's info badges. The first section is
    /// "File info"; others (EXIF, stream details) appear only when they
    /// apply.
    fn sections(&self) -> Vec<InfoSection>;

    /// One-line facts for the pane's status bar, in display order.
    fn status(&self) -> Vec<String>;

    /// The previewer's own toolbar controls (pretty print, say); the
    /// standard ones (line numbers, wrap, pager, fit and 1:1) come from the
    /// kind of document.
    fn controls(&self) -> Vec<PreviewControl>;

    /// A toolbar control was used; the next `render` reflects it.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` for an id this document has no control for, or
    /// a value of the wrong kind.
    fn set_control(&mut self, id: ControlId, value: ControlValue) -> Result<(), Error>;

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

/// Runs `work` on the runtime's blocking pool and awaits it, for extensions
/// that call into synchronous libraries or programs from a job.
///
/// # Errors
///
/// `work`'s error, or `Error::Io` when the blocking task panicked.
pub async fn blocking<T, F>(work: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, Error> + Send + 'static,
{
    runtime::handle()
        .spawn_blocking(work)
        .await
        .map_err(|join| Error::Io(std::io::Error::other(join)))?
}
