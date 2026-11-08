//! The Previewer extension point: rendering a file type.
//!
//! A previewer scores how well it handles an entry from its metadata and the
//! first 4 KiB of content; the highest score wins. `open` produces a
//! [`PreviewDoc`] with pages, a `render` that yields frames the UI uploads as
//! textures (or a text block it lays out itself), a text layer for find,
//! metadata in [`InfoSection`]s for the pane's info badges, one-line facts
//! for its status bar, the previewer's own toolbar [`PreviewControl`]s, and a
//! transport for media. A document whose content is neither pixels nor
//! plain text (an archive's file list, a CSV table, rendered Markdown)
//! offers it as [`Content`] through [`PreviewDoc::content`], which a pane
//! that knows the kind lays out itself, while `render` keeps giving a
//! textual fallback. The pane builds its toolbar from the kind of document
//! (text layer, pages, raster) plus those controls; a previewer never draws.
//! A document that answers to the pointer itself (a 3D view orbited by
//! dragging, zoomed by the wheel, reset by a double-click) declares the
//! gestures as [`PreviewControl::Pointer`] controls, and the pane hands it
//! the movement through `set_control` instead of panning or scrolling.
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

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use types::{CancelToken, Entry, EntryKind, Error, ExtId, Location};

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
///
/// The six required methods are the transport every player has; the rest
/// (volume, speed, frame step, loop) have defaults that do nothing, so a
/// transport that lacks one of them says so by leaving the default in
/// place: `volume` stays 1.0, `speed` 1.0 and `looping` false whatever is
/// set. While `playing` is `true` the UI renders again every frame; the
/// document's `render` returns the latest video frame (or the waveform
/// with the position marked) each time.
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
    /// Volume from 0.0 (silent) to 1.0 (full); 1.0 without a volume control.
    fn volume(&self) -> f32 {
        1.0
    }
    /// Sets the volume, clamped to 0.0 ..= 1.0; ignored without a volume
    /// control.
    fn set_volume(&mut self, _volume: f32) {}
    /// Playback speed as a factor of real time: 1.0 is normal, 0.5 half,
    /// 2.0 double.
    fn speed(&self) -> f32 {
        1.0
    }
    /// Sets the playback speed; ignored by transports that cannot change
    /// it.
    fn set_speed(&mut self, _speed: f32) {}
    /// Pauses and advances by one video frame; nothing for audio.
    fn step(&mut self) {}
    /// Whether playback starts over at the end instead of stopping.
    fn looping(&self) -> bool {
        false
    }
    /// Turns looping on or off.
    fn set_looping(&mut self, _on: bool) {}
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
    /// The rendered view of markup (Markdown, HTML), as opposed to its
    /// source.
    Rendered,
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlValue {
    /// A toggle's new state.
    Bool(bool),
    /// A button was pressed, or a double-click landed on the content.
    Pressed,
    /// A pointer movement over the content, in points: what a drag moved
    /// since the last report, or what the wheel scrolled.
    Delta {
        /// Rightwards.
        x: f32,
        /// Downwards.
        y: f32,
    },
}

/// A pointer gesture over the content that a previewer takes for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Gesture {
    /// A drag with the primary button; reported as [`ControlValue::Delta`]
    /// every frame it moves, in points since the last report.
    Drag,
    /// The wheel or a two-finger scroll without a modifier (the modifier
    /// zooms the pane); reported as [`ControlValue::Delta`] in points.
    Wheel,
    /// A double-click; reported as [`ControlValue::Pressed`].
    DoubleClick,
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
    /// A gesture over the content the document handles itself: a 3D
    /// view's orbit, zoom and reset. Nothing is drawn for it; the pane
    /// forwards the gesture to [`PreviewDoc::set_control`] instead of
    /// panning, scrolling or switching to actual size, and shows the last
    /// frame until the next one is rendered.
    Pointer {
        /// The control's id, passed back to [`PreviewDoc::set_control`].
        id: ControlId,
        /// The gesture claimed.
        gesture: Gesture,
        /// What the gesture does, for a hint: `"Orbit"`.
        label: &'static str,
    },
}

/// Content the UI lays out itself when it knows the kind, beyond pixels
/// and plain text: an archive's file list, a table, a rendered document.
/// A pane that does not know a kind shows the textual fallback `render`
/// returns for the same document.
#[derive(Clone)]
pub enum Content {
    /// A read-only file list in the explorer's columns.
    Listing(Arc<Listing>),
    /// A table shown as a virtualised grid with a header row.
    Grid(Arc<dyn Grid>),
    /// A document of headings, paragraphs, lists, code, tables and images.
    Document(Arc<Document>),
}

impl std::fmt::Debug for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Listing(listing) => f.debug_tuple("Listing").field(listing).finish(),
            Self::Grid(grid) => f
                .debug_struct("Grid")
                .field("header", &grid.header())
                .field("rows", &grid.rows())
                .finish(),
            Self::Document(document) => f.debug_tuple("Document").field(document).finish(),
        }
    }
}

/// A read-only file list, as an archive previewer produces it: the
/// explorer's columns (name, type, date modified, size) plus the
/// compressed size, with subfolders the UI expands in place.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listing {
    /// The entries in display order: a directory is followed by its
    /// contents, each one level deeper, so a pane that hides a collapsed
    /// directory skips the entries deeper than it until the depth comes
    /// back.
    pub entries: Vec<ListingEntry>,
    /// The stats line under the list: files, folders and sizes.
    pub stats: String,
    /// The list was cut short (a huge archive); `stats` says so.
    pub truncated: bool,
}

/// One row of a [`Listing`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingEntry {
    /// The display name, the last component of `path`.
    pub name: String,
    /// The path inside the archive, `/`-separated, without a leading `/`.
    pub path: String,
    /// Nesting depth: 0 at the top of the archive.
    pub depth: usize,
    /// File, directory, symbolic link or something else.
    pub kind: EntryKind,
    /// Uncompressed size, when known.
    pub size: Option<u64>,
    /// Size inside the archive, when the format records it per entry.
    pub compressed: Option<u64>,
    /// Modification time, when recorded.
    pub modified: Option<SystemTime>,
    /// The entry's content is encrypted; its name is still listed.
    pub encrypted: bool,
}

/// A table the UI shows as a virtualised grid: the header row and the
/// cells of any window of rows, parsed on demand so a million-row file
/// costs the rows on screen.
pub trait Grid: Send + Sync {
    /// The column titles.
    fn header(&self) -> &[String];
    /// Number of data rows, the header excluded.
    fn rows(&self) -> usize;
    /// The cells of the rows in `range`, in order; a row may have fewer or
    /// more cells than the header has columns. Must be cheap: the UI calls
    /// it for the rows on screen while scrolling.
    fn cells(&self, range: Range<usize>) -> Vec<Vec<String>>;
}

/// A document the UI lays out with its own fonts and widgets: what a
/// Markdown or HTML previewer produces.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    /// The blocks, top to bottom.
    pub blocks: Vec<Block>,
}

/// One block of a [`Document`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// A heading, levels 1 (largest) to 6.
    Heading {
        /// 1 to 6.
        level: u8,
        /// The heading's text.
        text: Vec<Inline>,
    },
    /// A paragraph of inline text.
    Paragraph(Vec<Inline>),
    /// A bulleted or numbered list; every item holds blocks, so lists nest.
    List {
        /// Numbered rather than bulleted.
        ordered: bool,
        /// The first number of an ordered list.
        start: u64,
        /// The items.
        items: Vec<Vec<Block>>,
    },
    /// Preformatted code.
    Code {
        /// The language named on the fence or the `class`, when any.
        language: Option<String>,
        /// The code, lines separated by `\n`.
        text: String,
    },
    /// A quotation of blocks.
    Quote(Vec<Block>),
    /// A table.
    Table {
        /// The header cells; empty when the table has no header row.
        header: Vec<Vec<Inline>>,
        /// The body rows.
        rows: Vec<Vec<Vec<Inline>>>,
    },
    /// A picture on its own.
    Image(Image),
    /// A horizontal rule.
    Rule,
}

/// A run of inline content inside a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Text in one style.
    Text {
        /// The text.
        text: String,
        /// Bold, italic, code or struck through.
        style: Style,
    },
    /// A link; the UI opens `href` with the system when it is a URL.
    Link {
        /// The link's text.
        text: String,
        /// The target as written.
        href: String,
    },
    /// A picture inside a line.
    Image(Image),
    /// A line break inside a block.
    Break,
}

bitflags::bitflags! {
    /// The style of an [`Inline::Text`]: any combination of the four.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Style: u8 {
        /// Bold weight.
        const BOLD = 1;
        /// Italic.
        const ITALIC = 2;
        /// Monospace, inline code.
        const CODE = 4;
        /// Struck through.
        const STRIKETHROUGH = 8;
    }
}

/// A picture in a [`Document`]: its bytes when the previewer could read
/// them (a file next to the document), else only its source and alt text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// The alternative text.
    pub alt: String,
    /// The source as written: a relative path or a URL.
    pub source: String,
    /// The encoded image (PNG, JPEG, …) for the UI to decode, when read.
    pub data: Option<Arc<[u8]>>,
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

    /// Content the UI lays out itself when it knows the kind: a file list,
    /// a grid or a document. `None`, the default, for pixels and text.
    /// `render` still answers for such a document with a textual fallback
    /// of the same content, for a pane that knows none of these kinds.
    fn content(&self) -> Option<Content> {
        None
    }

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

/// How much of a file a previewer reads, which decides how a file on a
/// remote provider (`Capabilities::REMOTE`) is served to it: a prefix
/// previewer gets the first chunk and the pane offers *Load more*; a
/// whole-file previewer is held behind a size cap with *Preview anyway*
/// (`specs/ext-ssh.md`, "Previews over SSH").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Reads {
    /// The previewer needs the whole file to show anything: an image, a
    /// PDF, a media file, an archive.
    #[default]
    Whole,
    /// The previewer shows the start of a file from its first bytes and
    /// can do with a prefix: text, code, CSV, a hex dump.
    Prefix,
}

/// Turns an entry into something the inspector or viewer can show.
#[async_trait]
pub trait Previewer: Send + Sync + 'static {
    /// The extension this previewer belongs to.
    fn id(&self) -> ExtId;

    /// 0 means cannot handle; the highest score wins. `sniff` is the first
    /// 4 KiB of the file.
    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8;

    /// Whether the previewer can work on a prefix of the file or needs the
    /// whole of it; [`Reads::Whole`] unless the previewer says otherwise.
    fn reads(&self) -> Reads {
        Reads::Whole
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// A transport with the six required methods only.
    struct Bare {
        playing: bool,
        at: Duration,
    }

    impl Transport for Bare {
        fn play(&mut self) {
            self.playing = true;
        }
        fn pause(&mut self) {
            self.playing = false;
        }
        fn seek(&mut self, to: Duration) {
            self.at = to;
        }
        fn position(&self) -> Duration {
            self.at
        }
        fn duration(&self) -> Option<Duration> {
            None
        }
        fn playing(&self) -> bool {
            self.playing
        }
    }

    #[test]
    fn the_optional_transport_methods_default_to_doing_nothing() {
        let mut t = Bare {
            playing: false,
            at: Duration::ZERO,
        };
        assert!((t.volume() - 1.0).abs() < f32::EPSILON);
        t.set_volume(0.2);
        assert!((t.volume() - 1.0).abs() < f32::EPSILON, "no volume control");
        assert!((t.speed() - 1.0).abs() < f32::EPSILON);
        t.set_speed(2.0);
        assert!((t.speed() - 1.0).abs() < f32::EPSILON);
        assert!(!t.looping());
        t.set_looping(true);
        assert!(!t.looping());
        t.play();
        t.step();
        assert!(t.playing(), "step is a no-op by default");
        t.seek(Duration::from_secs(3));
        assert_eq!(t.position(), Duration::from_secs(3));
    }

    struct Two;

    impl Grid for Two {
        fn header(&self) -> &[String] {
            static HEADER: std::sync::LazyLock<Vec<String>> =
                std::sync::LazyLock::new(|| vec!["a".to_owned(), "b".to_owned()]);
            &HEADER
        }
        fn rows(&self) -> usize {
            2
        }
        fn cells(&self, range: Range<usize>) -> Vec<Vec<String>> {
            range
                .map(|row| vec![row.to_string(), (row * 10).to_string()])
                .collect()
        }
    }

    #[test]
    fn content_debug_describes_a_grid_without_its_rows() {
        let grid = Content::Grid(Arc::new(Two));
        let text = format!("{grid:?}");
        assert!(text.contains("header"), "{text}");
        assert!(text.contains("rows: 2"), "{text}");
        let Content::Grid(grid) = grid.clone() else {
            panic!("not a grid");
        };
        assert_eq!(
            grid.cells(1..2),
            vec![vec!["1".to_owned(), "10".to_owned()]]
        );
        let listing = Content::Listing(Arc::new(Listing::default()));
        assert!(format!("{listing:?}").starts_with("Listing"));
        let document = Content::Document(Arc::new(Document {
            blocks: vec![Block::Rule],
        }));
        assert!(format!("{document:?}").contains("Rule"));
    }

    struct Pixels;

    impl PreviewDoc for Pixels {
        fn pages(&self) -> Option<usize> {
            None
        }
        fn render(&mut self, _page: usize, _req: RenderRequest) -> Result<Frame, Error> {
            Ok(Frame::External(TextureId(1)))
        }
        fn text_layer(&self) -> Option<&TextLayer> {
            None
        }
        fn sections(&self) -> Vec<InfoSection> {
            Vec::new()
        }
        fn status(&self) -> Vec<String> {
            Vec::new()
        }
        fn controls(&self) -> Vec<PreviewControl> {
            Vec::new()
        }
        fn set_control(&mut self, _id: ControlId, _value: ControlValue) -> Result<(), Error> {
            Err(Error::Unsupported("none"))
        }
        fn transport(&mut self) -> Option<&mut dyn Transport> {
            None
        }
    }

    #[test]
    fn a_document_has_no_content_unless_it_says_so() {
        assert!(Pixels.content().is_none());
    }
}
