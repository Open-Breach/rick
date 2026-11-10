//! Icon containers: ICO and CUR (Windows), ICNS (macOS) and XPM (X11),
//! previewed with every variant as a page, magnified pixel-sharp
//! (SPEC.md, *Icon containers*).
//!
//! The previewer claims the four formats by their magic. An ICO, CUR or
//! ICNS holds several variants of one picture; the decodable ones are the
//! document's pages, largest first, so the pane's standard pager steps
//! through them. A variant the plugin cannot decode (an ICNS JPEG 2000
//! entry, an unknown type, a corrupt entry) is no page: File info lists it
//! as not decodable and the footer counts it, and the file still previews
//! while any variant decodes; a file with none fails to open. XPM holds
//! one picture and has no pager.
//!
//! Icons are small, so the rule that a picture is never upscaled does not
//! hold here: for a fitted request `render` magnifies the variant by the
//! largest whole factor up to [`MAX_FACTOR`] that fits the request,
//! nearest-neighbour, so every pixel stays a crisp square once the pane
//! maps the frame one to one; for the pane's actual-size request
//! ([`ACTUAL_SIZE`] on both axes, its `MAX_ACTUAL`) it returns the variant
//! at its own pixels. Both are the previewer's work on the frame, so the
//! pane needs nothing new. Transparent pixels stay transparent. A CUR
//! draws its hotspot as a small crosshair, high-contrast against the
//! pixels under it, with the [`HOTSPOT`] toggle to hide it. XPM is text:
//! its source is the text layer, so *Copy* copies it and find searches
//! it; the binary containers have no text layer and no *Copy*.
//!
//! Pure Rust throughout: the `ico` and `icns` crates and an XPM parser of
//! the crate's own ([`xpm`]); no third-party library. This crate is a
//! plugin: it builds as a `cdylib` the host loads from
//! `plugins/preview-icons/` through the entry point [`ext::plugin!`]
//! declares, and as an `rlib` for the binary's opt-in `preview-icons`
//! feature and the tests.

// `deny`, not `forbid`: the entry point's export carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

pub mod container;
mod x11;
pub mod xpm;

use std::sync::Arc;

use ext::{
    API_VERSION, Capabilities, ControlId, ControlValue, Extension, Frame, Icon, InfoSection,
    Manifest, Platforms, Points, PreviewControl, PreviewDoc, Previewer, Provider, Registry,
    RenderRequest, Text, TextLayer, Transport, Version, async_trait,
};
use types::{CancelToken, Entry, EntryKind, Error, ExtId, Location};

pub use container::{Container, Format, Variant, is_ico_or_cur};
pub use xpm::Xpm;

/// The extension id.
pub const ID: ExtId = ExtId("preview-icons");

/// The hotspot toggle's id: the crosshair over a cursor's variant.
pub const HOTSPOT: ControlId = ControlId("icons.hotspot");

/// The largest whole magnification in Fit.
pub const MAX_FACTOR: u32 = 8;

/// A request at least this wide and this high asks for actual pixels: the
/// preview pane's `MAX_ACTUAL`, what it sends for 1:1.
pub const ACTUAL_SIZE: u32 = 8192;

/// Files above this are not opened; icon containers are small.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;

/// The file name extensions the previewer claims, for the manifest.
const PREVIEWS: &[&str] = &["cur", "icns", "ico", "xpm"];

/// The extension: registers [`IconsPreviewer`].
#[derive(Debug)]
pub struct IconsPreview {
    manifest: Manifest,
}

impl Default for IconsPreview {
    fn default() -> Self {
        Self::new()
    }
}

impl IconsPreview {
    /// The extension with its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Icon preview",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
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

impl Extension for IconsPreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        reg.previewer(Arc::new(IconsPreviewer));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(IconsPreview::new()) as Arc<dyn Extension>);

/// Previews icon containers.
#[derive(Debug, Default)]
pub struct IconsPreviewer;

#[async_trait]
impl Previewer for IconsPreviewer {
    fn id(&self) -> ExtId {
        ID
    }

    /// By magic alone: the ICO and CUR header, `icns`, `/* XPM */`.
    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8 {
        if entry.kind != EntryKind::File {
            return 0;
        }
        if is_ico_or_cur(sniff) || sniff.starts_with(b"icns") || xpm::is_xpm(sniff) {
            200
        } else {
            0
        }
    }

    async fn open(
        &self,
        loc: &Location,
        provider: Arc<dyn Provider>,
        cancel: CancelToken,
    ) -> Result<Box<dyn PreviewDoc>, Error> {
        let bytes = ext::read_bytes(&*provider, loc, Some(MAX_BYTES + 1), &cancel).await?;
        if bytes.len() > MAX_BYTES {
            return Err(Error::Unsupported("icon container larger than 64 MiB"));
        }
        let doc = IconDoc::new(&bytes)?;
        tracing::debug!(
            path = %loc.path.to_string_lossy(),
            format = doc.format().name(),
            variants = doc.variants().len(),
            skipped = doc.skipped(),
            "icons: opened"
        );
        Ok(Box::new(doc))
    }
}

/// An opened icon container.
pub struct IconDoc {
    container: Container,
    /// Indices into the variants of the decodable ones: the pages.
    pages: Vec<usize>,
    /// The XPM's colour count and characters per pixel.
    xpm: Option<(usize, usize)>,
    /// The XPM source.
    layer: Option<TextLayer>,
    file_size: usize,
    hotspot_shown: bool,
    /// The page last rendered, which the footer describes.
    shown: usize,
}

impl std::fmt::Debug for IconDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IconDoc")
            .field("format", &self.container.format)
            .field("variants", &self.container.variants.len())
            .field("pages", &self.pages.len())
            .finish_non_exhaustive()
    }
}

impl IconDoc {
    /// Decodes the bytes of a file.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` for bytes that are no icon container,
    /// `Error::Io` when the container does not parse (a truncated file, an
    /// XPM with a bad colour) or when no variant of it can be decoded.
    pub fn new(bytes: &[u8]) -> Result<Self, Error> {
        let invalid = |message: String| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                message,
            ))
        };
        let (container, xpm, layer) = if xpm::is_xpm(bytes) {
            let text = String::from_utf8_lossy(bytes).into_owned();
            let picture = xpm::parse(&text).map_err(|e| invalid(format!("XPM: {e}")))?;
            let container = Container {
                format: Format::Xpm,
                variants: vec![Variant {
                    width: picture.width,
                    height: picture.height,
                    depth: None,
                    ostype: None,
                    encoding: "XPM".to_owned(),
                    hotspot: None,
                    pixels: Ok(picture.pixels),
                }],
            };
            (
                container,
                Some((picture.colors, picture.chars_per_pixel)),
                Some(TextLayer { pages: vec![text] }),
            )
        } else {
            (Container::parse(bytes)?, None, None)
        };
        let pages: Vec<usize> = container
            .variants
            .iter()
            .enumerate()
            .filter(|(_, v)| v.decodable())
            .map(|(i, _)| i)
            .collect();
        if pages.is_empty() {
            let reasons: Vec<String> = container
                .variants
                .iter()
                .filter_map(|v| v.pixels.as_ref().err().cloned())
                .collect();
            return Err(invalid(format!(
                "no variant of the {} can be decoded: {}",
                container.format.name(),
                if reasons.is_empty() {
                    "it holds none".to_owned()
                } else {
                    reasons.join("; ")
                }
            )));
        }
        Ok(Self {
            container,
            pages,
            xpm,
            layer,
            file_size: bytes.len(),
            hotspot_shown: true,
            shown: 0,
        })
    }

    /// The format.
    #[must_use]
    pub fn format(&self) -> Format {
        self.container.format
    }

    /// Every variant, largest first, decodable or not.
    #[must_use]
    pub fn variants(&self) -> &[Variant] {
        &self.container.variants
    }

    /// The variants that are pages, in page order.
    #[must_use]
    pub fn page_variants(&self) -> Vec<&Variant> {
        self.pages
            .iter()
            .map(|&i| &self.container.variants[i])
            .collect()
    }

    /// How many variants are listed but not shown.
    #[must_use]
    pub fn skipped(&self) -> usize {
        self.container.skipped()
    }

    /// Whether the hotspot crosshair is drawn.
    #[must_use]
    pub fn hotspot_shown(&self) -> bool {
        self.hotspot_shown
    }
}

/// The largest whole factor from 1 to [`MAX_FACTOR`] by which a `width`
/// by `height` picture fits a request of `req_width` by `req_height`; 1
/// when even that does not fit, and the pane scales the frame down.
#[must_use]
pub fn factor_for(width: u32, height: u32, req_width: u32, req_height: u32) -> u32 {
    (1..=MAX_FACTOR)
        .rev()
        .find(|f| width.saturating_mul(*f) <= req_width && height.saturating_mul(*f) <= req_height)
        .unwrap_or(1)
}

/// `pixels` (RGBA, `width` by `height`) magnified by a whole `factor`,
/// nearest-neighbour: every source pixel becomes a `factor` by `factor`
/// square.
#[must_use]
pub fn magnify(pixels: &[u8], width: u32, height: u32, factor: u32) -> Vec<u8> {
    let factor = factor.max(1) as usize;
    let (w, h) = (width as usize, height as usize);
    if factor == 1 {
        return pixels.to_vec();
    }
    let mut out = Vec::with_capacity(w * h * factor * factor * 4);
    for y in 0..h {
        let row_start = out.len();
        for x in 0..w {
            let px = &pixels[(y * w + x) * 4..(y * w + x) * 4 + 4];
            for _ in 0..factor {
                out.extend_from_slice(px);
            }
        }
        let row_len = out.len() - row_start;
        for _ in 1..factor {
            out.extend_from_within(row_start..row_start + row_len);
        }
    }
    out
}

/// Draws a crosshair into an RGBA frame `width` wide: two one-pixel lines
/// of `arm` pixels each way through (`cx`, `cy`), each pixel white over a
/// dark pixel and black over a light or transparent one.
fn crosshair(frame: &mut [u8], width: u32, height: u32, cx: u32, cy: u32, arm: u32) {
    let mut mark = |x: i64, y: i64| {
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            return;
        };
        if x >= width || y >= height {
            return;
        }
        let at = ((y * width + x) * 4) as usize;
        let [red, green, blue, alpha] = [frame[at], frame[at + 1], frame[at + 2], frame[at + 3]];
        let luminance =
            (u32::from(red) * 299 + u32::from(green) * 587 + u32::from(blue) * 114) / 1000;
        let dark = alpha >= 128 && luminance < 128;
        let shade = if dark { 255 } else { 0 };
        frame[at..at + 4].copy_from_slice(&[shade, shade, shade, 255]);
    };
    let (cx, cy, arm) = (i64::from(cx), i64::from(cy), i64::from(arm));
    for d in -arm..=arm {
        mark(cx + d, cy);
        if d != 0 {
            mark(cx, cy + d);
        }
    }
}

impl PreviewDoc for IconDoc {
    /// The decodable variants, when there are two or more; a single one,
    /// and an XPM, have no pager.
    fn pages(&self) -> Option<usize> {
        (self.pages.len() > 1).then_some(self.pages.len())
    }

    /// The variant magnified by the largest whole factor that fits a
    /// fitted request, or at its own pixels for an actual-size request;
    /// the hotspot crosshair over a cursor while the toggle is on.
    fn render(&mut self, page: usize, req: RenderRequest) -> Result<Frame, Error> {
        let index = *self.pages.get(page).ok_or(Error::NotFound)?;
        let variant = &self.container.variants[index];
        let Ok(pixels) = &variant.pixels else {
            return Err(Error::NotFound);
        };
        let actual = req.width >= ACTUAL_SIZE && req.height >= ACTUAL_SIZE;
        let factor = if actual {
            1
        } else {
            factor_for(variant.width, variant.height, req.width, req.height)
        };
        let (width, height) = (variant.width * factor, variant.height * factor);
        let mut frame = magnify(pixels, variant.width, variant.height, factor);
        if self.hotspot_shown
            && let Some((x, y)) = variant.hotspot
        {
            crosshair(
                &mut frame,
                width,
                height,
                x * factor + factor / 2,
                y * factor + factor / 2,
                3 * factor,
            );
        }
        self.shown = page;
        Ok(Frame::Rgba {
            width,
            height,
            data: frame.into(),
        })
    }

    /// The XPM source; nothing for the binary containers.
    fn text_layer(&self) -> Option<&TextLayer> {
        self.layer.as_ref()
    }

    /// File info: the format and, for a container, one row per variant —
    /// `256 × 256 · 32-bit · PNG`, `128 × 128 · ic07 · PNG`,
    /// `not decodable (JPEG 2000)` — with the hotspot for a cursor; for
    /// an XPM its size, colours and characters per pixel.
    fn sections(&self) -> Vec<InfoSection> {
        let format = self.container.format;
        let mut rows = vec![("Format".to_owned(), format.name().to_owned())];
        if let (Some((colors, chars)), Some(picture)) = (self.xpm, self.container.variants.first())
        {
            rows.push((
                "Dimensions".to_owned(),
                format!("{} × {}", picture.width, picture.height),
            ));
            rows.push(("Colours".to_owned(), colors.to_string()));
            rows.push(("Chars per pixel".to_owned(), chars.to_string()));
        } else {
            let skipped = self.skipped();
            let total = self.container.variants.len();
            rows.push((
                "Variants".to_owned(),
                if skipped > 0 {
                    format!("{total} ({skipped} not decodable)")
                } else {
                    total.to_string()
                },
            ));
            for (i, variant) in self.container.variants.iter().enumerate() {
                rows.push((format!("Variant {}", i + 1), variant.describe()));
            }
        }
        rows.push(("Size".to_owned(), format_size(self.file_size as u64)));
        vec![InfoSection {
            id: "file",
            title: Text("preview-icons-file-info"),
            icon: Icon::Info,
            rows,
        }]
    }

    /// `ICO · 7 variants · 48 × 48 · 32-bit · PNG · 23.4 KiB`, with
    /// `2 not shown` when variants were skipped; `XPM · 32 × 32 ·
    /// 16 colours · 2.1 KiB`.
    fn status(&self) -> Vec<String> {
        let mut status = vec![self.container.format.name().to_owned()];
        if let Some((colors, _)) = self.xpm {
            if let Some(picture) = self.container.variants.first() {
                status.push(format!("{} × {}", picture.width, picture.height));
            }
            status.push(format!(
                "{colors} colour{}",
                if colors == 1 { "" } else { "s" }
            ));
        } else {
            let total = self.container.variants.len();
            status.push(format!(
                "{total} variant{}",
                if total == 1 { "" } else { "s" }
            ));
            if let Some(&index) = self.pages.get(self.shown) {
                status.extend(self.container.variants[index].facts());
            }
        }
        status.push(format_size(self.file_size as u64));
        let skipped = self.skipped();
        if skipped > 0 {
            status.push(format!("{skipped} not shown"));
        }
        status
    }

    /// The hotspot toggle for a cursor; nothing else.
    fn controls(&self) -> Vec<PreviewControl> {
        if self.container.format == Format::Cur {
            vec![PreviewControl::Toggle {
                id: HOTSPOT,
                icon: Icon::Tag,
                label: "Hotspot",
                on: self.hotspot_shown,
            }]
        } else {
            Vec::new()
        }
    }

    fn set_control(&mut self, id: ControlId, value: ControlValue) -> Result<(), Error> {
        match (id, value, self.container.format) {
            (HOTSPOT, ControlValue::Bool(on), Format::Cur) => {
                self.hotspot_shown = on;
                Ok(())
            }
            (HOTSPOT, ControlValue::Bool(_), _) => {
                Err(Error::Unsupported("only a cursor has a hotspot"))
            }
            (HOTSPOT, _, _) => Err(Error::Unsupported("the hotspot toggle takes on or off")),
            _ => Err(Error::Unsupported("no such control")),
        }
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_factor_is_the_largest_whole_one_that_fits_up_to_eight() {
        assert_eq!(factor_for(16, 16, 300, 300), 8);
        assert_eq!(
            factor_for(32, 32, 300, 300),
            8,
            "256 fits, nine would not be tried"
        );
        assert_eq!(factor_for(48, 48, 300, 300), 6);
        assert_eq!(factor_for(64, 32, 200, 300), 3, "the width limits");
        assert_eq!(factor_for(256, 256, 300, 300), 1);
        assert_eq!(factor_for(512, 512, 300, 300), 1, "larger than the request");
        assert_eq!(factor_for(16, 16, 0, 0), 1);
    }

    #[test]
    fn magnification_is_nearest_neighbour() {
        let src = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let out = magnify(&src, 2, 2, 2);
        assert_eq!(out.len(), 4 * 4 * 4);
        let px = |x: usize, y: usize| &out[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4];
        assert_eq!(px(0, 0), [1, 2, 3, 4]);
        assert_eq!(px(1, 1), [1, 2, 3, 4]);
        assert_eq!(px(2, 0), [5, 6, 7, 8]);
        assert_eq!(px(0, 3), [9, 10, 11, 12]);
        assert_eq!(px(3, 3), [13, 14, 15, 16]);
        assert_eq!(magnify(&src, 2, 2, 1), src);
    }

    #[test]
    fn the_crosshair_contrasts_with_what_is_under_it_and_stays_in_the_frame() {
        // A 5 × 5 frame: the left half black, the right half transparent.
        let mut frame: Vec<u8> = (0..25)
            .flat_map(|i| {
                if i % 5 < 2 {
                    [0, 0, 0, 255]
                } else {
                    [0, 0, 0, 0]
                }
            })
            .collect();
        crosshair(&mut frame, 5, 5, 2, 2, 2);
        let px = |x: usize, y: usize| &frame[(y * 5 + x) * 4..(y * 5 + x) * 4 + 4];
        assert_eq!(px(0, 2), [255, 255, 255, 255], "white over black");
        assert_eq!(px(1, 2), [255, 255, 255, 255]);
        assert_eq!(px(2, 2), [0, 0, 0, 255], "black over transparent");
        assert_eq!(px(4, 2), [0, 0, 0, 255]);
        assert_eq!(px(2, 0), [0, 0, 0, 255]);
        assert_eq!(px(2, 4), [0, 0, 0, 255]);
        assert_eq!(px(1, 1), [0, 0, 0, 255], "off the cross: untouched black");
        assert_eq!(
            px(3, 3),
            [0, 0, 0, 0],
            "off the cross: untouched transparent"
        );
        // Arms past the edge are clipped, not a panic.
        crosshair(&mut frame, 5, 5, 0, 0, 100);
        crosshair(&mut frame, 5, 5, 4, 4, 100);
    }

    #[test]
    fn sizes_read_in_binary_units() {
        assert_eq!(format_size(63), "63 B");
        assert_eq!(format_size(1546), "1.5 KiB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MiB");
    }
}
