//! The PDF path for a converted document: pdfium bound the way the PDF
//! previewer binds it, pages rendered to the request, text per page as it
//! is shown. A plugin sees `ext` and `types` only, so this mirrors
//! `ext-preview-pdf` rather than calling it (QUESTIONS.md 104); the
//! library is looked for in the same places (the PDF previewer's `library`
//! setting, read through the host's [`SettingsStore`] but never written
//! here, then beside the binary, the plugins directory, the platform's
//! library directories and the system loader), lazily, on the first
//! document shown. No environment variable is read.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ext::platform::DialogFilter;
use ext::{Frame, MissingLibrary, RenderRequest, SettingValue, SettingsStore, TextLayer};
use pdfium_render::prelude::*;
use types::{Error, ExtId};

/// The PDF previewer's id, whose `library` setting names the pdfium file.
pub const PDF_PREVIEWER: ExtId = ExtId("preview-pdf");

/// The PDF previewer's setting that holds the library's path.
pub const LIBRARY_SETTING: &str = "library";

/// Where the pdfium builds are downloaded from.
pub const DOWNLOADS: &str = "https://github.com/bblanchon/pdfium-binaries/releases";

/// Where the plugin may look for pdfium.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Search {
    /// The setting, beside the binary, the plugins directory, the
    /// platform's library directories and the system loader.
    #[default]
    Everywhere,
    /// The setting alone: how the tests stand on a machine without the
    /// library.
    SettingOnly,
}

/// The pdfium library as this plugin binds it: where it is looked for
/// and, once found, the binding every document shares.
pub struct Binding {
    settings: Option<Arc<dyn SettingsStore>>,
    config_dir: Option<PathBuf>,
    search: Search,
    bound: Mutex<Option<Arc<Pdfium>>>,
}

impl std::fmt::Debug for Binding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Binding")
            .field("search", &self.search)
            .field("bound", &self.lock().is_some())
            .finish_non_exhaustive()
    }
}

impl Default for Binding {
    fn default() -> Self {
        Self::new(None, None)
    }
}

/// The usual library directories of the three platforms, as plain paths.
fn system_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/local/lib"),
        PathBuf::from("/usr/lib"),
        PathBuf::from("/usr/lib64"),
        PathBuf::from("/usr/lib/x86_64-linux-gnu"),
        PathBuf::from("/usr/lib/aarch64-linux-gnu"),
        PathBuf::from("/opt/homebrew/lib"),
        PathBuf::from("/Library/Frameworks/pdfium"),
    ];
    for var in ["ProgramFiles", "LOCALAPPDATA", "SystemRoot"] {
        if let Some(base) = std::env::var_os(var) {
            let base = PathBuf::from(base);
            dirs.push(base.join("pdfium"));
            if var == "SystemRoot" {
                dirs.push(base.join("System32"));
            }
        }
    }
    dirs
}

impl Binding {
    /// A binding looked for everywhere, reading the PDF previewer's setting
    /// through `settings` when the host lent them; `config_dir` holds the
    /// per-user plugins directory.
    #[must_use]
    pub fn new(settings: Option<Arc<dyn SettingsStore>>, config_dir: Option<PathBuf>) -> Self {
        Self {
            settings,
            config_dir,
            search: Search::Everywhere,
            bound: Mutex::new(None),
        }
    }

    /// The same binding, looked for in `search`.
    #[must_use]
    pub fn with_search(mut self, search: Search) -> Self {
        self.search = search;
        self
    }

    fn lock(&self) -> MutexGuard<'_, Option<Arc<Pdfium>>> {
        self.bound.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The PDF previewer's `library` setting as a path, when it is set.
    #[must_use]
    pub fn setting(&self) -> Option<PathBuf> {
        match self
            .settings
            .as_ref()?
            .get(PDF_PREVIEWER, LIBRARY_SETTING)?
        {
            SettingValue::Text(text) if !text.trim().is_empty() => Some(PathBuf::from(text.trim())),
            _ => None,
        }
    }

    /// The files tried, in order.
    #[must_use]
    pub fn candidates(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(setting) = self.setting() {
            out.push(if setting.is_dir() {
                Pdfium::pdfium_platform_library_name_at_path(&setting)
            } else {
                setting
            });
        }
        if self.search == Search::SettingOnly {
            return out;
        }
        let mut dirs = Vec::new();
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            dirs.push(dir.to_path_buf());
            dirs.push(dir.join("plugins"));
            dirs.push(dir.join("plugins").join(PDF_PREVIEWER.0));
        }
        if let Some(config) = &self.config_dir {
            dirs.push(config.join("plugins"));
            dirs.push(config.join("plugins").join(PDF_PREVIEWER.0));
        }
        dirs.extend(system_dirs());
        for dir in dirs {
            let file = Pdfium::pdfium_platform_library_name_at_path(&dir);
            if !out.contains(&file) {
                out.push(file);
            }
        }
        out
    }

    /// The bound library, binding it on the first call.
    ///
    /// # Errors
    ///
    /// What the pane shows when nothing binds; the next call looks again.
    pub fn bind(&self) -> Result<Arc<Pdfium>, MissingLibrary> {
        if let Some(bound) = self.lock().as_ref() {
            return Ok(Arc::clone(bound));
        }
        let mut tried = Vec::new();
        for path in self.candidates() {
            if !path.is_file() {
                tried.push(format!("{}: not there", path.display()));
                continue;
            }
            match Pdfium::bind_to_library(&path) {
                Ok(bindings) => {
                    let pdfium = Arc::new(Pdfium::new(bindings));
                    *self.lock() = Some(Arc::clone(&pdfium));
                    return Ok(pdfium);
                }
                Err(error) => tried.push(format!("{}: {error}", path.display())),
            }
        }
        if self.search == Search::Everywhere {
            match Pdfium::bind_to_system_library() {
                Ok(bindings) => {
                    let pdfium = Arc::new(Pdfium::new(bindings));
                    *self.lock() = Some(Arc::clone(&pdfium));
                    return Ok(pdfium);
                }
                Err(error) => tried.push(format!("the system loader: {error}")),
            }
        }
        Err(missing(tried.join("\n")))
    }

    /// Whether the library binds on this machine.
    #[must_use]
    pub fn available(&self) -> bool {
        self.bind().is_ok()
    }
}

/// The message for a pdfium that is not there; *Browse…* fills the PDF
/// previewer's setting, which both previewers read.
#[must_use]
pub fn missing(detail: String) -> MissingLibrary {
    MissingLibrary {
        name: "pdfium".to_owned(),
        install: DOWNLOADS.to_owned(),
        setting: (PDF_PREVIEWER, LIBRARY_SETTING),
        filters: vec![DialogFilter {
            name: "pdfium library",
            extensions: &["so", "dylib", "dll"],
        }],
        detail: format!(
            "Download the build for this machine from the address above, unpack it and put {} \
             beside the rick binary, or browse to it.\nLooked in:\n{detail}",
            Pdfium::pdfium_platform_library_name().to_string_lossy()
        ),
    }
}

/// Whether the pdfium library can be bound on this machine, looked for
/// everywhere but the setting.
#[must_use]
pub fn available() -> bool {
    Binding::default().available()
}

/// The directory a library file is in, for messages.
#[must_use]
pub fn dir_of(path: &Path) -> PathBuf {
    path.parent().map_or_else(PathBuf::new, Path::to_path_buf)
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
    pdfium: Arc<Pdfium>,
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
    /// Parses the PDF once for its page count, through the bound `pdfium`.
    ///
    /// # Errors
    ///
    /// `Error::Io` for bytes pdfium cannot read.
    pub fn new(pdfium: Arc<Pdfium>, bytes: Vec<u8>) -> Result<Self, Error> {
        let document = pdfium
            .load_pdf_from_byte_slice(&bytes, None)
            .map_err(pdf_error)?;
        let count = usize::try_from(document.pages().len()).unwrap_or(0);
        drop(document);
        Ok(Self {
            pdfium,
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
        let document = self
            .pdfium
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
