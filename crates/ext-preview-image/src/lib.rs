//! Image previews: JPEG, PNG, WebP, BMP, TIFF and GIF through the `image`
//! crate, EXIF orientation applied, downscaled to the request.
//!
//! `open` reads the file and only its header: dimensions, format and EXIF
//! come back at once, so the pane can show them while the pixels are still
//! being decoded. The File info section carries the basics and the camera,
//! date taken and exposure when present; an EXIF section lists every tag
//! `kamadak-exif` reads (IPTC and XMP wait for a reader). `render` decodes
//! off the UI thread (the caller runs
//! it in a blocking task), picks the smallest JPEG DCT scale that still
//! covers the request, applies the orientation and downscales to fit, never
//! upscaling. GIF frames are pages.
//!
//! SVG and SVGZ belong here too (SPEC.md, *Vector images*): sniffed by
//! their root element, rasterised with `resvg` at the exact size the pane
//! asks for and again at every change of it, the highlighted source one
//! *Rendered* toggle away and the text layer, so *Copy* copies the raw SVG;
//! see [`svg`].

#![forbid(unsafe_code)]

pub mod svg;

use std::io::Cursor;
use std::sync::Arc;

use ext::Text;
use ext::{
    API_VERSION, Capabilities, ControlId, ControlValue, Extension, Frame, Icon, InfoSection,
    Manifest, Platforms, Points, PreviewControl, PreviewDoc, Previewer, Provider, Registry,
    RenderRequest, TextLayer, Transport, Version, async_trait,
};
use image::{AnimationDecoder as _, DynamicImage, ImageFormat, RgbaImage};
use types::{CancelToken, Entry, Error, ExtId, Location};

/// The extension id.
pub const ID: ExtId = ExtId("preview-image");

/// Files above this are not decoded; the preview says so instead.
pub const MAX_BYTES: usize = 512 * 1024 * 1024;

/// Raster extensions that name an image even when the sniff is cut short.
const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "bmp", "webp", "tif", "tiff"];

/// Every extension the previewer claims, for the manifest: the raster
/// formats and the vector ones, which score by content alone.
const PREVIEWS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "tif", "tiff", "svg", "svgz",
];

/// The extension: registers [`ImagePreviewer`].
#[derive(Debug)]
pub struct ImagePreview {
    manifest: Manifest,
}

impl Default for ImagePreview {
    fn default() -> Self {
        Self::new()
    }
}

impl ImagePreview {
    /// The extension with its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Image preview",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PREVIEWER,
                platforms: Platforms::ALL,
                previews: PREVIEWS,
            },
        }
    }
}

impl Extension for ImagePreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        reg.previewer(Arc::new(ImagePreviewer));
        Ok(())
    }
}

/// Previews images: the raster formats as [`ImageDoc`], SVG and SVGZ as
/// [`svg::SvgDoc`].
#[derive(Debug, Default)]
pub struct ImagePreviewer;

/// The formats this previewer decodes.
fn supported(format: ImageFormat) -> bool {
    matches!(
        format,
        ImageFormat::Jpeg
            | ImageFormat::Png
            | ImageFormat::Gif
            | ImageFormat::Bmp
            | ImageFormat::WebP
            | ImageFormat::Tiff
    )
}

/// EXIF orientation 1 to 8, as the TIFF tag defines it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Orientation(pub u32);

impl Orientation {
    /// Whether width and height swap after applying it.
    #[must_use]
    pub fn swaps_axes(self) -> bool {
        matches!(self.0, 5..=8)
    }

    /// Applies the orientation to pixels.
    #[must_use]
    pub fn apply(self, image: RgbaImage) -> RgbaImage {
        use image::imageops;
        match self.0 {
            2 => imageops::flip_horizontal(&image),
            3 => imageops::rotate180(&image),
            4 => imageops::flip_vertical(&image),
            5 => imageops::flip_horizontal(&imageops::rotate90(&image)),
            6 => imageops::rotate90(&image),
            7 => imageops::flip_horizontal(&imageops::rotate270(&image)),
            8 => imageops::rotate270(&image),
            _ => image,
        }
    }
}

/// What the header says, before any pixel is decoded.
#[derive(Debug, Clone)]
pub struct Header {
    /// The format.
    pub format: ImageFormat,
    /// Width and height as stored, before orientation.
    pub stored: (u32, u32),
    /// The EXIF orientation, 1 when absent.
    pub orientation: Orientation,
    /// The photo basics for File info: camera, date taken, exposure.
    pub exif: Vec<(String, String)>,
    /// Every EXIF tag, for the EXIF section; empty without an EXIF block.
    pub exif_all: Vec<(String, String)>,
}

impl Header {
    /// Reads the header and EXIF of `bytes`.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when the bytes are not an image this previewer
    /// decodes.
    pub fn read(bytes: &[u8]) -> Result<Self, Error> {
        let format = image::guess_format(bytes)
            .ok()
            .filter(|f| supported(*f))
            .ok_or(Error::Unsupported("not an image this previewer decodes"))?;
        let stored = image::ImageReader::with_format(Cursor::new(bytes), format)
            .into_dimensions()
            .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
        let (orientation, exif, exif_all) = read_exif(bytes);
        Ok(Self {
            format,
            stored,
            orientation,
            exif,
            exif_all,
        })
    }

    /// Width and height as shown, after orientation.
    #[must_use]
    pub fn shown(&self) -> (u32, u32) {
        if self.orientation.swaps_axes() {
            (self.stored.1, self.stored.0)
        } else {
            self.stored
        }
    }
}

/// A value longer than this is cut with an ellipsis in the EXIF section.
const MAX_VALUE: usize = 200;

/// The orientation, the File info photo rows and every tag of the EXIF
/// block, if any.
#[allow(clippy::type_complexity)]
fn read_exif(bytes: &[u8]) -> (Orientation, Vec<(String, String)>, Vec<(String, String)>) {
    let Ok(exif) = exif::Reader::new().read_from_container(&mut Cursor::new(bytes)) else {
        return (Orientation(1), Vec::new(), Vec::new());
    };
    let orientation = exif
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .filter(|o| (1..=8).contains(o))
        .map_or(Orientation(1), Orientation);
    let rows = [
        ("Camera", exif::Tag::Make),
        ("Model", exif::Tag::Model),
        ("Taken", exif::Tag::DateTimeOriginal),
        ("Exposure", exif::Tag::ExposureTime),
        ("Aperture", exif::Tag::FNumber),
        ("ISO", exif::Tag::PhotographicSensitivity),
        ("Focal length", exif::Tag::FocalLength),
    ]
    .into_iter()
    .filter_map(|(label, tag)| {
        let field = exif.get_field(tag, exif::In::PRIMARY)?;
        let value = shown_value(field, &exif);
        (!value.is_empty()).then(|| (label.to_owned(), value))
    })
    .collect();
    let all = exif
        .fields()
        .map(|field| {
            let name = if field.ifd_num == exif::In::PRIMARY {
                field.tag.to_string()
            } else {
                format!("{} ({})", field.tag, field.ifd_num)
            };
            let mut value = shown_value(field, &exif);
            if value.chars().count() > MAX_VALUE {
                value = value.chars().take(MAX_VALUE).collect::<String>() + "…";
            }
            (name, value)
        })
        .collect();
    (orientation, rows, all)
}

/// A field's value as text: with its unit, trimmed, and without the quotes
/// `kamadak-exif` puts around ASCII strings.
fn shown_value(field: &exif::Field, exif: &exif::Exif) -> String {
    let text = field.display_value().with_unit(exif).to_string();
    let text = text.trim();
    let unquoted = text
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(text);
    unquoted.trim().to_owned()
}

/// An opened image.
pub struct ImageDoc {
    bytes: Arc<[u8]>,
    header: Header,
    /// Decoded, oriented frames: one, or one per GIF frame. `full` says the
    /// cache is at native size; a scaled JPEG decode is replaced when a
    /// larger request arrives.
    frames: Option<Vec<RgbaImage>>,
    full: bool,
}

impl std::fmt::Debug for ImageDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageDoc")
            .field("format", &self.header.format)
            .field("stored", &self.header.stored)
            .field("decoded", &self.frames.is_some())
            .finish_non_exhaustive()
    }
}

impl ImageDoc {
    /// Reads the header of `bytes`; pixels wait for the first `render`.
    ///
    /// # Errors
    ///
    /// See [`Header::read`].
    pub fn new(bytes: Vec<u8>) -> Result<Self, Error> {
        let header = Header::read(&bytes)?;
        Ok(Self {
            bytes: bytes.into(),
            header,
            frames: None,
            full: false,
        })
    }

    /// The header.
    #[must_use]
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Decodes for a request, reusing the cache when it is big enough.
    fn frames_for(&mut self, req: RenderRequest) -> Result<&[RgbaImage], Error> {
        let (want_w, want_h) = if self.header.orientation.swaps_axes() {
            (req.height, req.width)
        } else {
            (req.width, req.height)
        };
        let big_enough = self.frames.as_ref().is_some_and(|frames| {
            self.full
                || frames.first().is_some_and(|f| {
                    f.width() >= want_w.min(self.header.stored.0)
                        && f.height() >= want_h.min(self.header.stored.1)
                })
        });
        if !big_enough {
            let (frames, full) = decode(&self.bytes, self.header.format, want_w, want_h)?;
            let orientation = self.header.orientation;
            self.frames = Some(frames.into_iter().map(|f| orientation.apply(f)).collect());
            self.full = full;
        }
        Ok(self.frames.as_deref().unwrap_or_default())
    }
}

/// Decodes `bytes`: a JPEG at the smallest DCT scale covering `want`, any
/// other format at native size. Returns the frames and whether they are at
/// native size.
fn decode(
    bytes: &[u8],
    format: ImageFormat,
    want_w: u32,
    want_h: u32,
) -> Result<(Vec<RgbaImage>, bool), Error> {
    let invalid = |e: String| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
    match format {
        ImageFormat::Jpeg => {
            let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
            decoder.read_info().map_err(|e| invalid(e.to_string()))?;
            let info = decoder
                .info()
                .ok_or_else(|| invalid("no JPEG header".into()))?;
            let (w, h) = decoder
                .scale(
                    u16::try_from(want_w.max(1)).unwrap_or(u16::MAX),
                    u16::try_from(want_h.max(1)).unwrap_or(u16::MAX),
                )
                .map_err(|e| invalid(e.to_string()))?;
            let pixels = decoder.decode().map_err(|e| invalid(e.to_string()))?;
            let full = (w, h) == (info.width, info.height);
            let rgba = to_rgba(
                decoder.info().map_or(info.pixel_format, |i| i.pixel_format),
                &pixels,
            );
            let image = RgbaImage::from_raw(u32::from(w), u32::from(h), rgba)
                .ok_or_else(|| invalid("JPEG size mismatch".into()))?;
            Ok((vec![image], full))
        }
        ImageFormat::Bmp
            if bmp_header(bytes)
                .is_some_and(|h| h.width > 2 * want_w.max(1) || h.height > 2 * want_h.max(1)) =>
        {
            // Uncompressed pixels: sample every n-th one straight from the
            // file, so a 50 MB bitmap shows in a few milliseconds; a larger
            // request decodes it whole.
            let header = bmp_header(bytes).ok_or_else(|| invalid("BMP header".into()))?;
            let step = (header.width / want_w.max(1))
                .max(header.height / want_h.max(1))
                .max(2);
            if let Some(image) = bmp_subsampled(bytes, &header, step) {
                Ok((vec![image], false))
            } else {
                let image = image::load_from_memory_with_format(bytes, format)
                    .map_err(|e| invalid(e.to_string()))?;
                Ok((vec![image.into_rgba8()], true))
            }
        }
        ImageFormat::Gif => {
            let decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes))
                .map_err(|e| invalid(e.to_string()))?;
            let frames: Vec<RgbaImage> = decoder
                .into_frames()
                .collect_frames()
                .map_err(|e| invalid(e.to_string()))?
                .into_iter()
                .map(image::Frame::into_buffer)
                .collect();
            if frames.is_empty() {
                return Err(invalid("GIF without frames".into()));
            }
            Ok((frames, true))
        }
        _ => {
            let image: DynamicImage = image::load_from_memory_with_format(bytes, format)
                .map_err(|e| invalid(e.to_string()))?;
            Ok((vec![image.into_rgba8()], true))
        }
    }
}

/// The header of an uncompressed 24- or 32-bit BMP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BmpHeader {
    width: u32,
    height: u32,
    top_down: bool,
    bytes_per_pixel: usize,
    offset: usize,
    stride: usize,
}

/// Parses the header when the file is a BMP this fast path handles:
/// `BI_RGB`, 24 or 32 bits per pixel, any DIB header size from 40 up.
fn bmp_header(bytes: &[u8]) -> Option<BmpHeader> {
    if bytes.len() < 54 || &bytes[..2] != b"BM" {
        return None;
    }
    let le32 =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let le16 = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let offset = le32(10) as usize;
    if le32(14) < 40 {
        return None;
    }
    let width = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
    let height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
    let bpp = le16(28);
    if le32(30) != 0 || !(bpp == 24 || bpp == 32) || width <= 0 || height == 0 {
        return None;
    }
    let bytes_per_pixel = usize::from(bpp / 8);
    let width = width.unsigned_abs();
    let stride = (width as usize * bytes_per_pixel).div_ceil(4) * 4;
    Some(BmpHeader {
        width,
        height: height.unsigned_abs(),
        top_down: height < 0,
        bytes_per_pixel,
        offset,
        stride,
    })
}

/// Every `step`-th pixel of every `step`-th row of an uncompressed BMP, as
/// RGBA. `None` when the file is shorter than its header claims.
fn bmp_subsampled(bytes: &[u8], header: &BmpHeader, step: u32) -> Option<RgbaImage> {
    let step = step.max(1);
    let out_w = header.width.div_ceil(step);
    let out_h = header.height.div_ceil(step);
    let mut data = Vec::with_capacity(out_w as usize * out_h as usize * 4);
    for oy in 0..out_h {
        let sy = oy * step;
        let row = if header.top_down {
            sy
        } else {
            header.height - 1 - sy
        };
        let base = header.offset + row as usize * header.stride;
        let end = base + header.width as usize * header.bytes_per_pixel;
        if end > bytes.len() {
            return None;
        }
        for ox in 0..out_w {
            let p = base + (ox * step) as usize * header.bytes_per_pixel;
            data.extend_from_slice(&[bytes[p + 2], bytes[p + 1], bytes[p], 255]);
        }
    }
    RgbaImage::from_raw(out_w, out_h, data)
}

/// Expands jpeg-decoder's pixel formats to RGBA8.
fn to_rgba(format: jpeg_decoder::PixelFormat, pixels: &[u8]) -> Vec<u8> {
    match format {
        jpeg_decoder::PixelFormat::RGB24 => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|&l| [l, l, l, 255]).collect(),
        jpeg_decoder::PixelFormat::L16 => pixels
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], 255])
            .collect(),
        jpeg_decoder::PixelFormat::CMYK32 => pixels
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| {
                let k = u32::from(p[3]);
                let channel =
                    |c: u8| u8::try_from((255 - u32::from(c)) * (255 - k) / 255).unwrap_or(255);
                [channel(p[0]), channel(p[1]), channel(p[2]), 255]
            })
            .collect(),
    }
}

/// Fits `image` inside `width` by `height`, never upscaling.
#[must_use]
pub fn fit(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    let (w, h) = (image.width(), image.height());
    if w == 0 || h == 0 || (w <= width && h <= height) {
        return image.clone();
    }
    let scale = (f64::from(width) / f64::from(w)).min(f64::from(height) / f64::from(h));
    image::imageops::thumbnail(image, scaled(w, scale), scaled(h, scale))
}

/// `size * scale`, rounded, at least 1; `scale` is below 1 here.
fn scaled(size: u32, scale: f64) -> u32 {
    let value = (f64::from(size) * scale)
        .round()
        .clamp(1.0, f64::from(u32::MAX));
    // Clamped into range, so the cast is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let out = value as u32;
    out
}

impl PreviewDoc for ImageDoc {
    fn pages(&self) -> Option<usize> {
        match (self.header.format, &self.frames) {
            (ImageFormat::Gif, Some(frames)) if frames.len() > 1 => Some(frames.len()),
            _ => None,
        }
    }

    fn render(&mut self, page: usize, req: RenderRequest) -> Result<Frame, Error> {
        let frames = self.frames_for(req)?;
        let frame = frames.get(page).ok_or(Error::NotFound)?;
        let fitted = fit(frame, req.width.max(1), req.height.max(1));
        Ok(Frame::Rgba {
            width: fitted.width(),
            height: fitted.height(),
            data: fitted.into_raw().into(),
        })
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        None
    }

    fn sections(&self) -> Vec<InfoSection> {
        let (w, h) = self.header.shown();
        let mut rows = vec![
            ("Format".to_owned(), self.format_name()),
            ("Dimensions".to_owned(), format!("{w} × {h}")),
            ("Size".to_owned(), format_size(self.bytes.len() as u64)),
        ];
        if self.header.orientation.0 != 1 {
            rows.push((
                "Orientation".to_owned(),
                self.header.orientation.0.to_string(),
            ));
        }
        if let Some(frames) = &self.frames
            && frames.len() > 1
        {
            rows.push(("Frames".to_owned(), frames.len().to_string()));
        }
        rows.extend(self.header.exif.iter().cloned());
        let mut sections = vec![InfoSection {
            id: "file",
            title: Text("preview-image-file-info"),
            icon: Icon::Info,
            rows,
        }];
        if !self.header.exif_all.is_empty() {
            sections.push(InfoSection {
                id: "exif",
                title: Text("preview-image-exif"),
                icon: Icon::Camera,
                rows: self.header.exif_all.clone(),
            });
        }
        sections
    }

    fn status(&self) -> Vec<String> {
        let (w, h) = self.header.shown();
        vec![
            format!("{w} × {h}"),
            self.format_name(),
            format_size(self.bytes.len() as u64),
        ]
    }

    fn controls(&self) -> Vec<PreviewControl> {
        Vec::new()
    }

    fn set_control(&mut self, _id: ControlId, _value: ControlValue) -> Result<(), Error> {
        Err(Error::Unsupported("images have no controls of their own"))
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

impl ImageDoc {
    /// The format's name in capitals: `JPEG`, `PNG`.
    fn format_name(&self) -> String {
        format!("{:?}", self.header.format).to_uppercase()
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

#[async_trait]
impl Previewer for ImagePreviewer {
    fn id(&self) -> ExtId {
        ID
    }

    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8 {
        if entry.kind != types::EntryKind::File {
            return 0;
        }
        // Vector images by content: the root element, or the gzip magic
        // with the `.svgz` name; never by the extension alone.
        if svg::is_svg(sniff) || svg::is_svgz(&entry.name, sniff) {
            return 200;
        }
        if image::guess_format(sniff).is_ok_and(supported) {
            return 200;
        }
        let ext = entry
            .name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if EXTENSIONS.contains(&ext.as_str()) {
            60
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
            return Err(Error::Unsupported("image larger than 512 MiB"));
        }
        let name = loc
            .path
            .to_string_lossy()
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_owned();
        if svg::is_svgz(&name, &bytes) || svg::is_svg(&bytes) {
            return Ok(Box::new(svg::SvgDoc::new(&name, &bytes)?));
        }
        Ok(Box::new(ImageDoc::new(bytes)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageEncoder as _, Rgba};
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

    fn req(width: u32, height: u32) -> RenderRequest {
        RenderRequest {
            width,
            height,
            scale: 1.0,
            dark: false,
        }
    }

    /// A 40×20 image whose left half is red and right half blue.
    fn sample() -> RgbaImage {
        RgbaImage::from_fn(40, 20, |x, _| {
            if x < 20 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        })
    }

    fn encode(image: &RgbaImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        if format == ImageFormat::Jpeg {
            // JPEG has no alpha channel.
            DynamicImage::ImageRgba8(image.clone())
                .into_rgb8()
                .write_to(&mut out, format)
                .unwrap_or_else(|e| panic!("{format:?}: {e}"));
        } else {
            image
                .write_to(&mut out, format)
                .unwrap_or_else(|e| panic!("{format:?}: {e}"));
        }
        out.into_inner()
    }

    /// A JPEG with an EXIF block carrying `orientation`.
    pub fn jpeg_with_orientation(image: &RgbaImage, orientation: u16) -> Vec<u8> {
        let rgb = DynamicImage::ImageRgba8(image.clone()).into_rgb8();
        let mut out = Cursor::new(Vec::new());
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95)
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .unwrap_or_else(|e| panic!("{e}"));
        let jpeg = out.into_inner();
        // APP1: "Exif\0\0", TIFF header (little endian, IFD at 8), one entry.
        let mut tiff = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x0112u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&0u32.to_le_bytes());
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(&tiff);
        let length = u16::try_from(payload.len() + 2).unwrap_or_else(|e| panic!("{e}"));
        let mut with_exif = jpeg[..2].to_vec();
        with_exif.extend_from_slice(&[0xFF, 0xE1]);
        with_exif.extend_from_slice(&length.to_be_bytes());
        with_exif.extend_from_slice(&payload);
        with_exif.extend_from_slice(&jpeg[2..]);
        with_exif
    }

    #[test]
    fn scores_images_by_magic_bytes() {
        let p = ImagePreviewer;
        let png = encode(&sample(), ImageFormat::Png);
        assert_eq!(p.score(&entry("x.bin"), &png[..64]), 200);
        assert_eq!(p.score(&entry("x.png"), b"not an image at all"), 60);
        assert_eq!(p.score(&entry("x.txt"), b"not an image at all"), 0);
        let mut dir = entry("x.png");
        dir.kind = EntryKind::Dir;
        assert_eq!(p.score(&dir, &png), 0);
    }

    #[test]
    fn every_format_opens_with_its_header_and_renders_to_fit() {
        for format in [
            ImageFormat::Png,
            ImageFormat::Bmp,
            ImageFormat::WebP,
            ImageFormat::Tiff,
            ImageFormat::Gif,
        ] {
            let bytes = encode(&sample(), format);
            let mut doc = ImageDoc::new(bytes).unwrap_or_else(|e| panic!("{format:?}: {e}"));
            assert_eq!(doc.header().shown(), (40, 20), "{format:?}");
            assert!(doc.frames.is_none(), "open decodes nothing");
            let Frame::Rgba {
                width,
                height,
                data,
            } = doc
                .render(0, req(20, 20))
                .unwrap_or_else(|e| panic!("{format:?}: {e}"))
            else {
                panic!("not pixels")
            };
            assert_eq!((width, height), (20, 10), "{format:?} fits the request");
            assert_eq!(data.len(), 20 * 10 * 4);
            // Left red, right blue, whatever the codec.
            assert!(
                data[0] > 200 && data[2] < 60,
                "{format:?}: {:?}",
                &data[..4]
            );
            let last = &data[(20 * 10 - 1) * 4..];
            assert!(last[2] > 200 && last[0] < 60, "{format:?}: {last:?}");
            // Never upscaled.
            let Frame::Rgba { width, height, .. } = doc
                .render(0, req(400, 400))
                .unwrap_or_else(|e| panic!("{e}"))
            else {
                panic!("not pixels")
            };
            assert_eq!((width, height), (40, 20), "{format:?}");
        }
    }

    #[test]
    fn jpeg_applies_exif_orientation_and_scales_the_decode() {
        let bytes = jpeg_with_orientation(&sample(), 6);
        let mut doc = ImageDoc::new(bytes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(doc.header().orientation, Orientation(6));
        assert_eq!(doc.header().shown(), (20, 40), "rotated 90°");
        assert!(
            doc.sections()[0]
                .rows
                .iter()
                .any(|(k, v)| k == "Orientation" && v == "6")
        );
        assert_eq!(
            doc.sections()[1].id,
            "exif",
            "an EXIF block gives a section"
        );
        let Frame::Rgba {
            width,
            height,
            data,
        } = doc
            .render(0, req(100, 100))
            .unwrap_or_else(|e| panic!("{e}"))
        else {
            panic!("not pixels")
        };
        assert_eq!((width, height), (20, 40));
        // Rotating 90° clockwise puts the red left half at the top.
        assert!(data[0] > 180 && data[2] < 80, "{:?}", &data[..4]);
        let bottom = &data[(20 * 40 - 1) * 4..];
        assert!(bottom[2] > 180 && bottom[0] < 80, "{bottom:?}");

        // A big JPEG asked small decodes at a DCT scale, then grows on demand.
        let big = RgbaImage::from_fn(640, 480, |x, y| {
            Rgba([
                u8::try_from(x % 256).unwrap_or(0),
                u8::try_from(y % 256).unwrap_or(0),
                128,
                255,
            ])
        });
        let mut doc =
            ImageDoc::new(encode(&big, ImageFormat::Jpeg)).unwrap_or_else(|e| panic!("{e}"));
        let Frame::Rgba { width, .. } =
            doc.render(0, req(80, 60)).unwrap_or_else(|e| panic!("{e}"))
        else {
            panic!("not pixels")
        };
        assert_eq!(width, 80);
        assert!(!doc.full, "decoded at a reduced scale");
        let cached = doc.frames.as_ref().map_or(0, |f| f[0].width());
        assert!((80..640).contains(&cached), "{cached}");
        let Frame::Rgba { width, .. } = doc
            .render(0, req(640, 480))
            .unwrap_or_else(|e| panic!("{e}"))
        else {
            panic!("not pixels")
        };
        assert_eq!(width, 640);
        assert!(doc.full);
    }

    #[test]
    fn big_bitmaps_are_sampled_from_the_file_first() {
        let big = RgbaImage::from_fn(400, 300, |x, _| {
            if x < 200 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        });
        let rgb = DynamicImage::ImageRgba8(big).into_rgb8();
        let mut out = Cursor::new(Vec::new());
        rgb.write_to(&mut out, ImageFormat::Bmp)
            .unwrap_or_else(|e| panic!("{e}"));
        let bytes = out.into_inner();
        let header = bmp_header(&bytes).unwrap_or_else(|| panic!("header"));
        assert_eq!(
            (header.width, header.height, header.bytes_per_pixel),
            (400, 300, 3)
        );
        let mut doc = ImageDoc::new(bytes).unwrap_or_else(|e| panic!("{e}"));
        let Frame::Rgba {
            width,
            height,
            data,
        } = doc.render(0, req(50, 50)).unwrap_or_else(|e| panic!("{e}"))
        else {
            panic!("not pixels")
        };
        assert!(!doc.full, "sampled, not decoded");
        assert_eq!((width, height), (50, 38), "{width}x{height}");
        assert!(data[0] > 200 && data[2] < 60, "{:?}", &data[..4]);
        let last = &data[data.len() - 4..];
        assert!(last[2] > 200 && last[0] < 60, "{last:?}");
        let Frame::Rgba { width, .. } = doc
            .render(0, req(400, 300))
            .unwrap_or_else(|e| panic!("{e}"))
        else {
            panic!("not pixels")
        };
        assert_eq!(width, 400);
        assert!(doc.full);
    }

    #[test]
    fn gif_frames_are_pages() {
        let mut out = Cursor::new(Vec::new());
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
            let a = image::Frame::new(RgbaImage::from_pixel(8, 8, Rgba([255, 0, 0, 255])));
            let b = image::Frame::new(RgbaImage::from_pixel(8, 8, Rgba([0, 255, 0, 255])));
            encoder
                .encode_frames([a, b])
                .unwrap_or_else(|e| panic!("{e}"));
        }
        let mut doc = ImageDoc::new(out.into_inner()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(doc.pages(), None, "unknown until decoded");
        let Frame::Rgba { data, .. } = doc.render(1, req(8, 8)).unwrap_or_else(|e| panic!("{e}"))
        else {
            panic!("not pixels")
        };
        assert!(
            data[1] > 200 && data[0] < 60,
            "second frame is green: {:?}",
            &data[..4]
        );
        assert_eq!(doc.pages(), Some(2));
        assert!(doc.render(2, req(8, 8)).is_err());
    }

    #[test]
    fn open_goes_through_the_provider() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("pic.png");
        std::fs::write(&path, encode(&sample(), ImageFormat::Png))
            .unwrap_or_else(|e| panic!("{e}"));
        let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
        let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&path));
        let doc = runtime::block_on(ImagePreviewer.open(&loc, provider, CancelToken::new()))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            doc.sections()[0]
                .rows
                .iter()
                .any(|(k, v)| k == "Dimensions" && v == "40 × 20"),
            "{:?}",
            doc.sections()
        );
    }

    /// A JPEG whose EXIF block has a camera, a date and an exposure, as a
    /// photo would.
    fn photo_jpeg() -> Vec<u8> {
        let rgb = DynamicImage::ImageRgba8(sample()).into_rgb8();
        let mut out = Cursor::new(Vec::new());
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .unwrap_or_else(|e| panic!("{e}"));
        let jpeg = out.into_inner();
        let mut with_exif = jpeg[..2].to_vec();
        let payload = exif_payload();
        let length = u16::try_from(payload.len() + 2).unwrap_or_else(|e| panic!("{e}"));
        with_exif.extend_from_slice(&[0xFF, 0xE1]);
        with_exif.extend_from_slice(&length.to_be_bytes());
        with_exif.extend_from_slice(&payload);
        with_exif.extend_from_slice(&jpeg[2..]);
        with_exif
    }

    /// `Exif\0\0` + a little-endian TIFF with IFD0 (`Make`, `Model`, Exif
    /// pointer) and an Exif IFD (`DateTimeOriginal`, `ExposureTime`,
    /// `FNumber`).
    fn exif_payload() -> Vec<u8> {
        let len = |bytes: &[u8]| u32::try_from(bytes.len()).unwrap_or_else(|e| panic!("{e}"));
        let mut tiff = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
        // IFD0 at 8: 3 entries of 12 bytes + the next pointer = 42 → ends at 50.
        let make = b"Rick Cameras\0";
        let model = b"RC-1\0";
        let date = b"2026:11:06 12:34:56\0";
        let make_at: u32 = 50;
        let model_at = make_at + len(make);
        let exif_ifd_at = model_at + len(model);
        // Exif IFD: 3 entries + the next pointer = 42.
        let date_at = exif_ifd_at + 42;
        let exposure_at = date_at + len(date);
        let fnumber_at = exposure_at + 8;
        let entry = |tiff: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: [u8; 4]| {
            tiff.extend_from_slice(&tag.to_le_bytes());
            tiff.extend_from_slice(&kind.to_le_bytes());
            tiff.extend_from_slice(&count.to_le_bytes());
            tiff.extend_from_slice(&value);
        };
        tiff.extend_from_slice(&3u16.to_le_bytes());
        entry(&mut tiff, 0x010F, 2, len(make), make_at.to_le_bytes());
        entry(&mut tiff, 0x0110, 2, len(model), model_at.to_le_bytes());
        entry(&mut tiff, 0x8769, 4, 1, exif_ifd_at.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(tiff.len(), 50);
        tiff.extend_from_slice(make);
        tiff.extend_from_slice(model);
        assert_eq!(len(&tiff), exif_ifd_at);
        tiff.extend_from_slice(&3u16.to_le_bytes());
        entry(&mut tiff, 0x9003, 2, len(date), date_at.to_le_bytes());
        entry(&mut tiff, 0x829A, 5, 1, exposure_at.to_le_bytes());
        entry(&mut tiff, 0x829D, 5, 1, fnumber_at.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        tiff.extend_from_slice(date);
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&250u32.to_le_bytes());
        tiff.extend_from_slice(&28u32.to_le_bytes());
        tiff.extend_from_slice(&10u32.to_le_bytes());
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(&tiff);
        payload
    }

    #[test]
    fn a_photo_has_file_info_with_the_camera_and_an_exif_section_with_every_tag() {
        let doc = ImageDoc::new(photo_jpeg()).unwrap_or_else(|e| panic!("{e}"));
        let sections = doc.sections();
        assert_eq!(sections.len(), 2, "{sections:?}");
        let file = &sections[0];
        assert_eq!(
            (file.id, file.title.key(), file.icon),
            ("file", "preview-image-file-info", Icon::Info)
        );
        let row = |rows: &[(String, String)], key: &str| {
            rows.iter()
                .find(|(k, _)| k == key)
                .map_or_else(|| panic!("no row {key}: {rows:?}"), |(_, v)| v.clone())
        };
        assert_eq!(row(&file.rows, "Format"), "JPEG");
        assert_eq!(row(&file.rows, "Dimensions"), "40 × 20");
        assert_eq!(row(&file.rows, "Camera"), "Rick Cameras");
        assert_eq!(row(&file.rows, "Model"), "RC-1");
        assert_eq!(row(&file.rows, "Taken"), "2026-11-06 12:34:56");
        assert_eq!(row(&file.rows, "Exposure"), "1/250 s");
        assert_eq!(row(&file.rows, "Aperture"), "f/2.8");
        let exif = &sections[1];
        assert_eq!(
            (exif.id, exif.title.key(), exif.icon),
            ("exif", "preview-image-exif", Icon::Camera)
        );
        assert_eq!(exif.rows.len(), 5, "every tag: {:?}", exif.rows);
        assert_eq!(row(&exif.rows, "Make"), "Rick Cameras");
        assert_eq!(row(&exif.rows, "DateTimeOriginal"), "2026-11-06 12:34:56");
        assert_eq!(row(&exif.rows, "FNumber"), "f/2.8");
        assert_eq!(doc.status()[0], "40 × 20");
        assert_eq!(doc.status()[1], "JPEG");
        assert!(doc.status()[2].ends_with("KiB") || doc.status()[2].ends_with('B'));
        assert_eq!(doc.controls().len(), 0);

        // No EXIF block: one section only.
        let plain =
            ImageDoc::new(encode(&sample(), ImageFormat::Png)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(plain.sections().len(), 1);
        assert!(!plain.sections()[0].rows.iter().any(|(k, _)| k == "Camera"));
    }
}
