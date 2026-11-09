//! Vector images: SVG and SVGZ, rasterised with `resvg` at the size the pane
//! asks for, the source one toggle away (SPEC.md, *Vector images*).
//!
//! An SVG has no pixels of its own, so [`SvgDoc::render`] rasterises the
//! document for every request: fitted into the request's width and height
//! (the pane's size) keeping the aspect ratio, or at the intrinsic size
//! times the display scale when the pane asks for actual pixels (a request
//! of [`ACTUAL_SIZE`] on both axes, the pane's `MAX_ACTUAL`). A new request
//! size is a new rasterisation; nothing is ever scaled up from a cached
//! bitmap. The long edge is capped at [`MAX_RASTER`] pixels: above it the
//! picture is rendered at the cap, the pane scales it, and the footer says
//! so.
//!
//! The intrinsic size is the root element's `width` and `height`, the
//! `viewBox` when they are missing, and [`DEFAULT_SIZE`] when both are.
//! Text is set through usvg's font database over the system fonts, loaded
//! once per process on the first document that has text. Scripts never
//! run (resvg has none); `href`s to files or URLs, external style sheets
//! and remote images are not resolved — only data URIs are honoured — so
//! nothing is read beyond the file itself.
//!
//! The *Rendered* toggle ([`RENDERED`]), on for every newly shown file,
//! switches to the highlighted XML of `ext-preview-code`, the path the
//! markup plugin takes for its source view. The text layer is the source,
//! decompressed for SVGZ, so *Copy* copies the raw SVG as text and find
//! searches it. A document usvg rejects falls to the source with the
//! parser's message (line and column when it has them) in the footer, and
//! *Rendered* stays available.

use std::io::Read as _;
use std::sync::{Arc, OnceLock};

use ext::{
    ControlId, ControlValue, Frame, Icon, InfoSection, PreviewControl, PreviewDoc, RenderRequest,
    Text, TextLayer, Transport,
};
use ext_preview_code::CodeDoc;
use resvg::tiny_skia;
use resvg::usvg;
use types::Error;

/// The Rendered toggle's id: the picture rather than the source.
pub const RENDERED: ControlId = ControlId("image.rendered");

/// The longest edge a raster is rendered at. Above it the picture is
/// rendered at the cap and scaled by the pane, and the footer says so.
pub const MAX_RASTER: u32 = 8192;

/// A request at least this wide and this high asks for actual pixels: the
/// preview pane's `MAX_ACTUAL`, what it sends for 1:1.
pub const ACTUAL_SIZE: u32 = 8192;

/// The intrinsic size assumed when the document has neither `width` and
/// `height` nor a `viewBox`.
pub const DEFAULT_SIZE: (f32, f32) = (300.0, 150.0);

/// The gzip magic that opens an SVGZ.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Whether `sniff` starts an SVG document: after an optional byte order
/// mark and any XML declaration, processing instructions, comments and
/// DOCTYPE, the root element is `svg` (a namespace prefix allowed). A
/// sniff cut short before the root element is not one.
#[must_use]
pub fn is_svg(sniff: &[u8]) -> bool {
    let mut rest = sniff.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(sniff);
    loop {
        rest = match rest.iter().position(|b| !b.is_ascii_whitespace()) {
            Some(at) => &rest[at..],
            None => return false,
        };
        if let Some(after) = rest.strip_prefix(b"<?") {
            let Some(end) = find(after, b"?>") else {
                return false;
            };
            rest = &after[end + 2..];
        } else if let Some(after) = rest.strip_prefix(b"<!--") {
            let Some(end) = find(after, b"-->") else {
                return false;
            };
            rest = &after[end + 3..];
        } else if let Some(after) = rest.strip_prefix(b"<!") {
            // A DOCTYPE, possibly with an internal subset in brackets.
            let mut depth = 0usize;
            let Some(end) = after.iter().position(|&b| {
                match b {
                    b'[' => depth += 1,
                    b']' => depth = depth.saturating_sub(1),
                    b'>' if depth == 0 => return true,
                    _ => {}
                }
                false
            }) else {
                return false;
            };
            rest = &after[end + 1..];
        } else if let Some(after) = rest.strip_prefix(b"<") {
            return is_svg_tag(after);
        } else {
            return false;
        }
    }
}

/// Whether the element name at the start of `tag` (the bytes after `<`)
/// is `svg`, with or without a namespace prefix.
fn is_svg_tag(tag: &[u8]) -> bool {
    let name_end = tag
        .iter()
        .position(|b| !(b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':')))
        .unwrap_or(tag.len());
    let name = &tag[..name_end];
    let local = name
        .iter()
        .rposition(|&b| b == b':')
        .map_or(name, |colon| &name[colon + 1..]);
    local == b"svg"
}

/// The first position of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Whether the file named `name` with the first bytes `sniff` is an SVGZ:
/// the `.svgz` extension together with the gzip magic.
#[must_use]
pub fn is_svgz(name: &str, sniff: &[u8]) -> bool {
    sniff.starts_with(&GZIP_MAGIC)
        && name
            .rsplit_once('.')
            .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("svgz"))
}

/// The system fonts, loaded once per process on the first document with
/// text.
fn system_fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    Arc::clone(FONTS.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        tracing::debug!(faces = db.len(), "svg: system fonts loaded");
        Arc::new(db)
    }))
}

/// The parsing options: the spec's default size, data URIs honoured and
/// nothing else resolved, the system fonts when the document has text.
fn options(has_text: bool) -> usvg::Options<'static> {
    let mut options = usvg::Options {
        resources_dir: None,
        default_size: usvg::Size::from_wh(DEFAULT_SIZE.0, DEFAULT_SIZE.1)
            .unwrap_or_else(|| usvg::Options::default().default_size),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            // A path or a URL in an `href` is never followed.
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    if has_text {
        options.fontdb = system_fonts();
    }
    options
}

/// Where the document's size comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum Intrinsic {
    /// The root element's `width` and `height`, in pixels.
    Size(f32, f32),
    /// Only a `viewBox`: its text, normalised, and the size it gives.
    ViewBox(String, f32, f32),
    /// Neither: [`DEFAULT_SIZE`].
    Default,
}

impl Intrinsic {
    /// The size in pixels at scale 1.
    #[must_use]
    pub fn size(&self) -> (f32, f32) {
        match self {
            Self::Size(w, h) | Self::ViewBox(_, w, h) => (*w, *h),
            Self::Default => DEFAULT_SIZE,
        }
    }

    /// The footer's and File info's wording: `512 × 512 px` or
    /// `viewBox 0 0 24 24`.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::ViewBox(text, _, _) => format!("viewBox {text}"),
            Self::Size(w, h) => format!("{} × {} px", px(*w), px(*h)),
            Self::Default => format!("{} × {} px", px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)),
        }
    }
}

/// A pixel count for display: whole when it is one, else to one decimal.
fn px(value: f32) -> String {
    if (value - value.round()).abs() < 0.05 {
        format!("{}", value.round())
    } else {
        format!("{value:.1}")
    }
}

/// What the root element says, read with roxmltree before usvg parses.
struct Root {
    has_size: bool,
    view_box: Option<String>,
    elements: usize,
}

fn root(text: &str) -> Option<Root> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    let doc = roxmltree::Document::parse_with_options(text, options).ok()?;
    let root = doc.root_element();
    let view_box = root
        .attribute("viewBox")
        .map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "));
    Some(Root {
        has_size: root.attribute("width").is_some() && root.attribute("height").is_some(),
        view_box,
        elements: doc
            .descendants()
            .filter(roxmltree::Node::is_element)
            .count(),
    })
}

/// The parser's message for the footer, with line and column when the
/// error has a position.
fn describe(error: &usvg::Error) -> String {
    match error {
        usvg::Error::ParsingFailed(parse) => {
            let pos = parse.pos();
            let message = parse.to_string();
            let message = message
                .strip_suffix(&format!(" at {pos}"))
                .unwrap_or(&message);
            format!(
                "not rendered: {message} at line {}, column {}",
                pos.row, pos.col
            )
        }
        other => format!("not rendered: {other}"),
    }
}

/// An opened SVG or SVGZ.
pub struct SvgDoc {
    /// The highlighted XML and the text layer.
    source: CodeDoc,
    /// The parsed document, or why usvg rejected it.
    tree: Result<usvg::Tree, String>,
    intrinsic: Intrinsic,
    /// Elements in the document, when it parsed as XML.
    elements: Option<usize>,
    /// The file's size on disk; the compressed size for an SVGZ.
    file_size: usize,
    /// The decompressed size of an SVGZ.
    decompressed: Option<usize>,
    rendered: bool,
    /// The last frame was rendered at the cap.
    capped: bool,
}

impl std::fmt::Debug for SvgDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SvgDoc")
            .field("intrinsic", &self.intrinsic)
            .field("parsed", &self.tree.is_ok())
            .field("rendered", &self.rendered)
            .finish_non_exhaustive()
    }
}

impl SvgDoc {
    /// Opens the bytes of the file `name`: decompresses an SVGZ, keeps the
    /// source for the text layer and parses the document.
    ///
    /// # Errors
    ///
    /// `Error::Io` when an SVGZ is not valid gzip or inflates beyond the
    /// size limit. A document usvg rejects is not an error here: it opens
    /// on the source view with the message in the footer.
    pub fn new(name: &str, bytes: &[u8]) -> Result<Self, Error> {
        let invalid =
            |e: String| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
        let file_size = bytes.len();
        let mut decompressed = None;
        let xml: Vec<u8> = if bytes.starts_with(&GZIP_MAGIC) {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(bytes)
                .take(super::MAX_BYTES as u64 + 1)
                .read_to_end(&mut out)
                .map_err(|e| invalid(format!("SVGZ: {e}")))?;
            if out.len() > super::MAX_BYTES {
                return Err(Error::Unsupported("SVGZ inflates beyond 512 MiB"));
            }
            decompressed = Some(out.len());
            out
        } else {
            bytes.to_vec()
        };
        // The code previewer picks the syntax from the name's extension;
        // SVG is XML, so the source is highlighted as such.
        let source = CodeDoc::new(&format!("{name}.xml"), &xml, false, Some(file_size as u64));
        let text = source.text();
        let root = root(text);
        let tree = usvg::Tree::from_str(text, &options(text.contains("<text")));
        let intrinsic = match (&tree, &root) {
            (Ok(tree), Some(root)) if root.has_size => {
                Intrinsic::Size(tree.size().width(), tree.size().height())
            }
            (
                Ok(tree),
                Some(Root {
                    view_box: Some(view_box),
                    ..
                }),
            ) => Intrinsic::ViewBox(view_box.clone(), tree.size().width(), tree.size().height()),
            _ => Intrinsic::Default,
        };
        if let Err(error) = &tree {
            tracing::debug!(%error, name, "svg: not rendered");
        }
        Ok(Self {
            source,
            tree: tree.map_err(|e| describe(&e)),
            intrinsic,
            elements: root.map(|r| r.elements),
            file_size,
            decompressed,
            rendered: true,
            capped: false,
        })
    }

    /// The decoded source text.
    #[must_use]
    pub fn text(&self) -> &str {
        self.source.text()
    }

    /// Where the size comes from.
    #[must_use]
    pub fn intrinsic(&self) -> &Intrinsic {
        &self.intrinsic
    }

    /// Why usvg rejected the document, when it did.
    #[must_use]
    pub fn parse_error(&self) -> Option<&str> {
        self.tree.as_ref().err().map(String::as_str)
    }

    /// Whether the Rendered toggle is on.
    #[must_use]
    pub fn rendered(&self) -> bool {
        self.rendered
    }

    /// Whether the last frame was rendered at the cap.
    #[must_use]
    pub fn capped(&self) -> bool {
        self.capped
    }

    /// The format's name for the footer and File info.
    fn format_name(&self) -> &'static str {
        if self.decompressed.is_some() {
            "SVGZ"
        } else {
            "SVG"
        }
    }

    /// The raster size for a request and the scale that gives it: the
    /// intrinsic size times the display scale for an actual-size request,
    /// the fit into the request otherwise, the long edge capped.
    fn target(&self, req: RenderRequest) -> ((u32, u32), f32, bool) {
        // A request is a pane's or the cap's size, far below 2^24, so the
        // conversions are exact.
        #[allow(clippy::cast_precision_loss)]
        let px = |v: u32| v.max(1) as f32;
        let cap = px(MAX_RASTER);
        let (w, h) = self.intrinsic.size();
        let actual = req.width >= ACTUAL_SIZE && req.height >= ACTUAL_SIZE;
        let mut scale = if actual {
            req.scale.max(0.01)
        } else {
            (px(req.width) / w).min(px(req.height) / h)
        };
        let long = (w * scale).max(h * scale);
        let capped = long > cap;
        if capped {
            scale *= cap / long;
        }
        let dim = |v: f32| {
            let out = (v * scale).round().clamp(1.0, cap);
            // Clamped into range, so the cast is exact.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let out = out as u32;
            out
        };
        ((dim(w), dim(h)), scale, capped)
    }

    /// Rasterises the document for the request.
    fn rasterise(&mut self, req: RenderRequest) -> Result<Frame, Error> {
        let Ok(tree) = &self.tree else {
            return self.source.render(0, req);
        };
        let ((width, height), scale, capped) = self.target(req);
        let mut pixmap = tiny_skia::Pixmap::new(width, height)
            .ok_or(Error::Unsupported("the raster size is out of range"))?;
        resvg::render(
            tree,
            tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        self.capped = capped;
        Ok(Frame::Rgba {
            width,
            height,
            data: straight_alpha(pixmap.take()).into(),
        })
    }
}

/// tiny-skia's premultiplied RGBA as straight RGBA, in place.
fn straight_alpha(mut data: Vec<u8>) -> Vec<u8> {
    for px in data.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = u8::try_from((u32::from(*c) * 255 + a / 2) / a).unwrap_or(255);
            }
        }
    }
    data
}

impl PreviewDoc for SvgDoc {
    fn pages(&self) -> Option<usize> {
        None
    }

    /// The picture at the requested size while *Rendered* is on and the
    /// document parsed; the highlighted source otherwise.
    fn render(&mut self, page: usize, req: RenderRequest) -> Result<Frame, Error> {
        if page != 0 {
            return Err(Error::NotFound);
        }
        if self.rendered {
            self.rasterise(req)
        } else {
            self.source.render(0, req)
        }
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        self.source.text_layer()
    }

    fn sections(&self) -> Vec<InfoSection> {
        let mut file_rows = vec![("Format".to_owned(), self.format_name().to_owned())];
        if self.tree.is_ok() {
            file_rows.push(("Dimensions".to_owned(), self.intrinsic.describe()));
        }
        file_rows.push(("Size".to_owned(), super::format_size(self.file_size as u64)));
        let file = InfoSection {
            id: "file",
            title: Text("preview-image-file-info"),
            icon: Icon::Info,
            rows: file_rows,
        };
        let mut rows = Vec::new();
        match &self.intrinsic {
            Intrinsic::Size(w, h) => {
                rows.push(("Width".to_owned(), format!("{} px", px(*w))));
                rows.push(("Height".to_owned(), format!("{} px", px(*h))));
            }
            Intrinsic::ViewBox(text, w, h) => {
                rows.push(("viewBox".to_owned(), text.clone()));
                rows.push(("Size".to_owned(), format!("{} × {} px", px(*w), px(*h))));
            }
            Intrinsic::Default if self.tree.is_ok() => {
                rows.push((
                    "Size".to_owned(),
                    format!("{} (assumed)", self.intrinsic.describe()),
                ));
            }
            Intrinsic::Default => {}
        }
        if let Some(elements) = self.elements {
            rows.push(("Elements".to_owned(), elements.to_string()));
        }
        if let Some(decompressed) = self.decompressed {
            rows.push((
                "Compressed".to_owned(),
                super::format_size(self.file_size as u64),
            ));
            rows.push((
                "Uncompressed".to_owned(),
                super::format_size(decompressed as u64),
            ));
        }
        if let Err(error) = &self.tree {
            rows.push(("Rendering".to_owned(), error.clone()));
        }
        vec![
            file,
            InfoSection {
                id: "image",
                title: Text("preview-image-image"),
                icon: Icon::Info,
                rows,
            },
        ]
    }

    /// `SVG · 512 × 512 px · 14.2 KiB`, `viewBox 0 0 24 24` in place of the
    /// size when only a viewBox exists, `rendered at 8192 px cap` after a
    /// capped render; for a document that did not parse, no size and the
    /// parser's message.
    fn status(&self) -> Vec<String> {
        let mut status = vec![self.format_name().to_owned()];
        if self.tree.is_ok() {
            status.push(self.intrinsic.describe());
        }
        status.push(super::format_size(self.file_size as u64));
        if self.capped {
            status.push(format!("rendered at {MAX_RASTER} px cap"));
        }
        if let Err(error) = &self.tree {
            status.push(error.clone());
        }
        status
    }

    /// The Rendered toggle first; the source's own controls (pretty print)
    /// while the source is shown.
    fn controls(&self) -> Vec<PreviewControl> {
        let mut controls = vec![PreviewControl::Toggle {
            id: RENDERED,
            icon: Icon::Rendered,
            label: "Rendered",
            on: self.rendered,
        }];
        if !self.rendered || self.tree.is_err() {
            controls.extend(self.source.controls());
        }
        controls
    }

    fn set_control(&mut self, id: ControlId, value: ControlValue) -> Result<(), Error> {
        match (id, value) {
            (RENDERED, ControlValue::Bool(on)) => {
                self.rendered = on;
                Ok(())
            }
            (RENDERED, _) => Err(Error::Unsupported("the Rendered toggle takes on or off")),
            (id, value) => self.source.set_control(id, value),
        }
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(width: u32, height: u32) -> RenderRequest {
        RenderRequest {
            width,
            height,
            scale: 1.0,
            dark: false,
        }
    }

    #[test]
    fn sniffs_the_root_element_past_declaration_comments_and_doctype() {
        assert!(is_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
        assert!(is_svg(
            b"\xEF\xBB\xBF<?xml version=\"1.0\"?>\n<!-- a comment -->\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\" [ <!ENTITY a \"b\"> ]>\n<svg:svg xmlns:svg=\"http://www.w3.org/2000/svg\">"
        ));
        assert!(is_svg(b"  \n<svg>"));
        assert!(!is_svg(b"<svgfoo>"));
        assert!(!is_svg(b"<html><body><svg/></body></html>"));
        assert!(!is_svg(b"<?xml version=\"1.0\"?><rect/>"));
        assert!(!is_svg(b"<?xml version=\"1.0\"?><!-- cut short"));
        assert!(!is_svg(b"PNG"));
        assert!(!is_svg(b""));
        assert!(is_svgz("a.svgz", &[0x1f, 0x8b, 8, 0]));
        assert!(is_svgz("A.SVGZ", &[0x1f, 0x8b, 8, 0]));
        assert!(!is_svgz("a.svg", &[0x1f, 0x8b, 8, 0]));
        assert!(!is_svgz("a.svgz", b"<svg/>"));
        assert!(!is_svgz("a.gz", &[0x1f, 0x8b, 8, 0]));
    }

    #[test]
    fn the_target_follows_the_request_and_the_cap() {
        let doc = SvgDoc::new(
            "a.svg",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"100\"/>",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(doc.intrinsic(), &Intrinsic::Size(200.0, 100.0));
        // Fitted: the limiting axis fills the request.
        assert_eq!(doc.target(req(400, 400)).0, (400, 200));
        assert_eq!(doc.target(req(100, 400)).0, (100, 50));
        // Actual size: the intrinsic size times the display scale.
        let mut actual = req(ACTUAL_SIZE, ACTUAL_SIZE);
        actual.scale = 2.0;
        assert_eq!(doc.target(actual), ((400, 200), 2.0, false));
        // Capped on the long edge.
        let ((w, h), _, capped) = doc.target(RenderRequest {
            scale: 100.0,
            ..actual
        });
        assert!(capped);
        assert_eq!((w, h), (MAX_RASTER, MAX_RASTER / 2));
    }

    #[test]
    fn premultiplied_pixels_come_out_straight() {
        let data = straight_alpha(vec![0, 0, 0, 0, 64, 0, 0, 128, 255, 255, 255, 255]);
        assert_eq!(data, vec![0, 0, 0, 0, 128, 0, 0, 128, 255, 255, 255, 255]);
    }

    #[test]
    fn pixel_counts_read_whole_or_to_one_decimal() {
        assert_eq!(px(24.0), "24");
        assert_eq!(px(24.004), "24");
        assert_eq!(px(24.5), "24.5");
    }
}
