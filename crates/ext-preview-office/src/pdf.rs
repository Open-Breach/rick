//! The PDF path for a converted document: pdfium bound the way the PDF
//! previewer binds it (`RICK_PDFIUM_DIR`, then beside the executable, then
//! the system), pages rendered to the request, text per page as it is
//! shown. A plugin sees `ext` and `types` only, so this mirrors
//! `ext-preview-pdf` rather than calling it (QUESTIONS.md 87).

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use ext::{Frame, RenderRequest, TextLayer};
use pdfium_render::prelude::*;
use types::Error;

/// The environment variable naming the directory that holds the pdfium
/// library, shared with the PDF previewer.
pub const LIBRARY_DIR_VAR: &str = "RICK_PDFIUM_DIR";

static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();

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

/// A converted document's pages.
pub struct Pages {
    bytes: Arc<[u8]>,
    count: usize,
    layer: TextLayer,
}

impl std::fmt::Debug for Pages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pages")
            .field("count", &self.count)
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl Pages {
    /// Parses the PDF once for its page count.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` without pdfium, `Error::Io` for bytes pdfium
    /// cannot read.
    pub fn new(bytes: Vec<u8>) -> Result<Self, Error> {
        let pdfium = pdfium()?;
        let document = pdfium
            .load_pdf_from_byte_slice(&bytes, None)
            .map_err(pdf_error)?;
        let count = usize::try_from(document.pages().len()).unwrap_or(0);
        drop(document);
        Ok(Self {
            bytes: bytes.into(),
            count,
            layer: TextLayer {
                pages: vec![String::new(); count],
            },
        })
    }

    /// Number of pages.
    #[must_use]
    pub fn count(&self) -> usize {
        self.count
    }

    /// The text of the pages rendered so far.
    #[must_use]
    pub fn layer(&self) -> &TextLayer {
        &self.layer
    }

    /// The PDF itself.
    #[must_use]
    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }

    /// Renders `page` to fit `req`, keeping the aspect ratio, and keeps
    /// the page's text for the layer.
    ///
    /// # Errors
    ///
    /// `Error::NotFound` past the last page; pdfium's error otherwise.
    pub fn render(&mut self, page: usize, req: RenderRequest) -> Result<Frame, Error> {
        if page >= self.count {
            return Err(Error::NotFound);
        }
        let pdfium = pdfium()?;
        let document = pdfium
            .load_pdf_from_byte_slice(&self.bytes, None)
            .map_err(pdf_error)?;
        let index = PdfPageIndex::try_from(page).map_err(|_| Error::NotFound)?;
        let pdf_page = document.pages().get(index).map_err(pdf_error)?;
        let (pw, ph) = (
            f64::from(pdf_page.width().value),
            f64::from(pdf_page.height().value),
        );
        let scale = (f64::from(req.width.max(1)) / pw)
            .min(f64::from(req.height.max(1)) / ph)
            .min(4.0);
        let config = PdfRenderConfig::new()
            .set_target_size(pixels(pw, scale), pixels(ph, scale))
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
}
