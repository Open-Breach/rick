//! PDF previews through pdfium: pages rendered to the requested width, page
//! navigation, and each page's text as it is shown.
//!
//! pdfium is a dynamic library the previewer binds at first use: from the
//! directory in `RICK_PDFIUM_DIR`, then next to the executable, then the
//! system. Without it the previewer still claims PDF files and `open` fails
//! with a message saying what to install, which the inspector shows.
//!
//! pdfium is not thread-safe and its documents cannot be sent between
//! threads, so a document is parsed from its bytes on every render; pdfium
//! parses lazily, so that costs a few milliseconds, and `pdfium-render`'s
//! `thread_safe` feature serialises the calls.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use ext::{
    API_VERSION, Capabilities, Extension, Frame, Manifest, PreviewDoc, Previewer, Provider,
    Registry, RenderRequest, TextLayer, Transport, Version, async_trait,
};
use pdfium_render::prelude::*;
use types::{CancelToken, Entry, Error, ExtId, Location};

/// The extension id.
pub const ID: ExtId = ExtId("preview-pdf");

/// The environment variable naming the directory that holds the pdfium
/// library.
pub const LIBRARY_DIR_VAR: &str = "RICK_PDFIUM_DIR";

/// Files above this are not opened.
pub const MAX_BYTES: usize = 1024 * 1024 * 1024;

/// The extension: registers [`PdfPreviewer`].
#[derive(Debug)]
pub struct PdfPreview {
    manifest: Manifest,
}

impl Default for PdfPreview {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfPreview {
    /// The extension with its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "PDF preview",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
            },
        }
    }
}

impl Extension for PdfPreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.previewer(Arc::new(PdfPreviewer));
        Ok(())
    }
}

/// Previews PDF files.
#[derive(Debug, Default)]
pub struct PdfPreviewer;

static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();

/// The directories searched for the library, in order.
fn library_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os(LIBRARY_DIR_VAR) {
        dirs.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.to_path_buf());
    }
    dirs
}

/// The bound library, or why it is not there.
fn pdfium() -> Result<&'static Pdfium, Error> {
    PDFIUM
        .get_or_init(|| {
            let mut tried = Vec::new();
            for dir in library_dirs() {
                let path = Pdfium::pdfium_platform_library_name_at_path(&dir);
                match Pdfium::bind_to_library(&path) {
                    Ok(bindings) => return Ok(Pdfium::new(bindings)),
                    Err(error) => tried.push(format!("{}: {error}", path.display())),
                }
            }
            match Pdfium::bind_to_system_library() {
                Ok(bindings) => Ok(Pdfium::new(bindings)),
                Err(error) => {
                    tried.push(format!("system library: {error}"));
                    Err(tried.join("; "))
                }
            }
        })
        .as_ref()
        .map_err(|_| Error::Unsupported("pdfium not found: set RICK_PDFIUM_DIR to its directory"))
}

/// Whether the pdfium library can be bound on this machine.
#[must_use]
pub fn available() -> bool {
    pdfium().is_ok()
}

/// An opened PDF.
pub struct PdfDoc {
    bytes: Arc<[u8]>,
    pages: usize,
    layer: TextLayer,
    info: Vec<(String, String)>,
}

impl std::fmt::Debug for PdfDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PdfDoc")
            .field("pages", &self.pages)
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

fn pdf_error(error: impl std::fmt::Display) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        error.to_string(),
    ))
}

/// `size * scale`, rounded, at least 1, as the pixel count pdfium takes.
fn pixels(size: f64, scale: f64) -> Pixels {
    let value = (size * scale).round().clamp(1.0, f64::from(i32::MAX));
    // Clamped into range, so the cast is exact.
    #[allow(clippy::cast_possible_truncation)]
    let out = value as Pixels;
    out
}

impl PdfDoc {
    /// Parses the document once for its page count and metadata.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` without pdfium, `Error::Io` for a file pdfium
    /// cannot read.
    pub fn new(bytes: Vec<u8>) -> Result<Self, Error> {
        let pdfium = pdfium()?;
        let document = pdfium
            .load_pdf_from_byte_slice(&bytes, None)
            .map_err(pdf_error)?;
        let pages = usize::try_from(document.pages().len()).unwrap_or(0);
        let mut info = vec![("Pages".to_owned(), pages.to_string())];
        for (label, tag) in [
            ("Title", PdfDocumentMetadataTagType::Title),
            ("Author", PdfDocumentMetadataTagType::Author),
            ("Subject", PdfDocumentMetadataTagType::Subject),
            ("Producer", PdfDocumentMetadataTagType::Producer),
        ] {
            if let Some(value) = document.metadata().get(tag) {
                let value = value.value().trim().to_owned();
                if !value.is_empty() {
                    info.push((label.to_owned(), value));
                }
            }
        }
        if let Ok(first) = document.pages().first() {
            info.push((
                "Page size".to_owned(),
                format!(
                    "{:.0} × {:.0} pt",
                    first.width().value,
                    first.height().value
                ),
            ));
        }
        drop(document);
        Ok(Self {
            bytes: bytes.into(),
            pages,
            layer: TextLayer {
                pages: vec![String::new(); pages],
            },
            info,
        })
    }
}

impl PreviewDoc for PdfDoc {
    fn pages(&self) -> Option<usize> {
        Some(self.pages)
    }

    fn render(&mut self, page: usize, req: RenderRequest) -> Result<Frame, Error> {
        if page >= self.pages {
            return Err(Error::NotFound);
        }
        let pdfium = pdfium()?;
        let document = pdfium
            .load_pdf_from_byte_slice(&self.bytes, None)
            .map_err(pdf_error)?;
        let index = PdfPageIndex::try_from(page).map_err(|_| Error::NotFound)?;
        let pdf_page = document.pages().get(index).map_err(pdf_error)?;
        // Fit the page into the request, never upscaling past 4× the point
        // size, which keeps a tiny pane from asking for a huge bitmap.
        let (pw, ph) = (
            f64::from(pdf_page.width().value),
            f64::from(pdf_page.height().value),
        );
        let scale = (f64::from(req.width.max(1)) / pw)
            .min(f64::from(req.height.max(1)) / ph)
            .min(4.0);
        let width = pixels(pw, scale);
        let height = pixels(ph, scale);
        let config = PdfRenderConfig::new()
            .set_target_size(width, height)
            .render_form_data(true);
        let bitmap = pdf_page.render_with_config(&config).map_err(pdf_error)?;
        let (w, h) = (bitmap.width(), bitmap.height());
        let data = bitmap.as_rgba_bytes();
        if self.layer.pages[page].is_empty()
            && let Ok(text) = pdf_page.text()
        {
            let all = text.all();
            self.layer.pages[page] = if all.is_empty() { " ".to_owned() } else { all };
        }
        Ok(Frame::Rgba {
            width: u32::try_from(w).unwrap_or(0),
            height: u32::try_from(h).unwrap_or(0),
            data: data.into(),
        })
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        Some(&self.layer)
    }

    fn info(&self) -> Vec<(String, String)> {
        self.info.clone()
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

#[async_trait]
impl Previewer for PdfPreviewer {
    fn id(&self) -> ExtId {
        ID
    }

    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8 {
        if entry.kind != types::EntryKind::File {
            return 0;
        }
        if sniff.starts_with(b"%PDF-") {
            return 200;
        }
        if entry.name.to_ascii_lowercase().ends_with(".pdf") {
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
            return Err(Error::Unsupported("PDF larger than 1 GiB"));
        }
        Ok(Box::new(PdfDoc::new(bytes)?))
    }
}

/// Writes a minimal PDF with `pages` pages, each showing its number in
/// Helvetica, for tests and fixtures.
#[must_use]
pub fn sample_pdf(pages: usize) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = Vec::new();
    // 1: catalog, 2: pages, 3: font, then per page: page + content.
    let first_page_object = 4;
    let kids: Vec<String> = (0..pages)
        .map(|i| format!("{} 0 R", first_page_object + 2 * i))
        .collect();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objects.push(
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            pages
        )
        .into_bytes(),
    );
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    for i in 0..pages {
        let content = format!("BT /F1 36 Tf 72 700 Td (Page {} of {}) Tj ET", i + 1, pages);
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
                first_page_object + 2 * i + 1
            )
            .into_bytes(),
        );
        objects.push(
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content.len(),
                content
            )
            .into_bytes(),
        );
    }
    objects.push(b"<< /Title (Rick sample) /Producer (Rick fixture) >>".to_vec());
    let info_object = objects.len();
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info {info_object} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
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
    fn scores_pdf_by_header_then_name() {
        let p = PdfPreviewer;
        assert_eq!(p.score(&entry("doc.bin"), b"%PDF-1.7\n"), 200);
        assert_eq!(p.score(&entry("doc.pdf"), b"garbage"), 60);
        assert_eq!(p.score(&entry("doc.txt"), b"garbage"), 0);
    }

    #[test]
    fn the_sample_pdf_is_well_formed() {
        let bytes = sample_pdf(3);
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(bytes.ends_with(b"%%EOF\n"));
        assert_eq!(
            bytes.windows(11).filter(|w| w == b"/Type /Page").count(),
            3 + 1
        );
    }

    /// Needs pdfium: see `LIBRARY_DIR_VAR`. The acceptance run in CI has it.
    #[test]
    fn pages_render_and_carry_their_text() {
        if !available() {
            eprintln!("pdfium not available; set {LIBRARY_DIR_VAR}");
            assert!(
                std::env::var_os("RICK_REQUIRE_PDFIUM").is_none(),
                "pdfium required but not found"
            );
            return;
        }
        let mut doc = PdfDoc::new(sample_pdf(5)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(doc.pages(), Some(5));
        assert!(
            doc.info()
                .iter()
                .any(|(k, v)| k == "Title" && v == "Rick sample"),
            "{:?}",
            doc.info()
        );
        let req = RenderRequest {
            width: 306,
            height: 1000,
            scale: 1.0,
            dark: false,
        };
        let Frame::Rgba {
            width,
            height,
            data,
        } = doc.render(2, req).unwrap_or_else(|e| panic!("{e}"))
        else {
            panic!("not pixels")
        };
        assert_eq!(
            (width, height),
            (306, 396),
            "fits the width, keeps the aspect"
        );
        assert_eq!(data.len(), 306 * 396 * 4);
        assert!(
            data.as_chunks::<4>().0.iter().any(|p| p[0] < 128),
            "some ink on the page"
        );
        let layer = doc.text_layer().unwrap_or_else(|| panic!("layer"));
        assert!(
            layer.pages[2].contains("Page 3 of 5"),
            "{:?}",
            layer.pages[2]
        );
        assert!(
            layer.pages[0].is_empty(),
            "other pages wait for their render"
        );
        assert!(doc.render(5, req).is_err());
    }
}
