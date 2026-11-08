//! PDF previews through pdfium: pages rendered to the requested width, page
//! navigation, and each page's text as it is shown.
//!
//! pdfium is a dynamic library the previewer binds lazily, on the first
//! PDF it is asked to show, the way SPEC.md's *Third-party libraries*
//! says: from the `library` setting of `preview-pdf.toml` when it is set,
//! then beside the binary and in the plugins directory, then the
//! platform's usual library directories and the system loader; the first
//! hit is written back into the setting through the host's
//! [`SettingsStore`], so the user sees and can change where Rick looks. No
//! environment variable is read. Without the library the previewer still
//! claims PDF files and `open` returns a document whose content is
//! [`Content::MissingLibrary`]: the pane shows the file's facts, the
//! download for this platform, *Browse…* and *Retry*.
//!
//! pdfium is not thread-safe and its documents cannot be sent between
//! threads, so a document is parsed from its bytes on every render; pdfium
//! parses lazily, so that costs a few milliseconds, and `pdfium-render`'s
//! `thread_safe` feature serialises the calls.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use ext::Text;
use ext::platform::DialogFilter;

use ext::{
    API_VERSION, Capabilities, Content, ControlId, ControlValue, Extension, Frame, Icon,
    InfoSection, Manifest, MissingLibrary, Platforms, Points, PreviewControl, PreviewDoc,
    Previewer, Provider, Registry, RenderRequest, SettingField, SettingKind, SettingValue,
    SettingsSchema, SettingsStore, TextBlock, TextLayer, Transport, Version, async_trait,
};
use pdfium_render::prelude::*;
use types::{CancelToken, Entry, Error, ExtId, Location};

/// The extension id, and the name of its settings file.
pub const ID: ExtId = ExtId("preview-pdf");

/// The setting that holds the path of the pdfium library (a file, or the
/// directory it is in), in `preview-pdf.toml`.
pub const LIBRARY_SETTING: &str = "library";

/// Where the pdfium builds are downloaded from.
pub const DOWNLOADS: &str = "https://github.com/bblanchon/pdfium-binaries/releases";

/// Files above this are not opened.
pub const MAX_BYTES: usize = 1024 * 1024 * 1024;

/// The extension: registers [`PdfPreviewer`] and the `library` setting.
#[derive(Debug)]
pub struct PdfPreview {
    manifest: Manifest,
    search: Search,
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
        Self::with_search(Search::Everywhere)
    }

    /// The extension looking for pdfium in `search` only: the tests' way
    /// of standing on a machine without the library.
    #[must_use]
    pub fn with_search(search: Search) -> Self {
        Self {
            search,
            manifest: Manifest {
                id: ID,
                name: "PDF preview",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PREVIEWER,
                platforms: Platforms::ALL,
                previews: &["pdf"],
            },
        }
    }
}

impl Extension for PdfPreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        reg.settings(settings_schema());
        let library = Library::new(
            reg.settings_store().cloned(),
            reg.platform().map(|platform| platform.config_dir()),
        )
        .with_search(self.search);
        reg.previewer(Arc::new(PdfPreviewer::new(Arc::new(library))));
        Ok(())
    }
}

/// The settings the Settings window shows for `preview-pdf.toml`: the
/// library's path.
#[must_use]
pub fn settings_schema() -> SettingsSchema {
    SettingsSchema {
        ext: ID,
        fields: vec![SettingField {
            key: LIBRARY_SETTING,
            label: Text("preview-pdf-library"),
            kind: SettingKind::Text {
                default: String::new(),
            },
        }],
    }
}

/// Where the previewer may look for pdfium.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Search {
    /// The setting, beside the binary, the plugins directory, the
    /// platform's library directories and the system loader.
    #[default]
    Everywhere,
    /// The setting alone: how the tests stand on a machine without the
    /// library whatever the machine has.
    SettingOnly,
}

/// The pdfium library: where it is looked for and, once found, the
/// binding, shared by every document.
pub struct Library {
    settings: Option<Arc<dyn SettingsStore>>,
    config_dir: Option<PathBuf>,
    search: Search,
    bound: Mutex<Option<Arc<Pdfium>>>,
}

impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library")
            .field("search", &self.search)
            .field("bound", &self.lock().is_some())
            .finish_non_exhaustive()
    }
}

impl Default for Library {
    fn default() -> Self {
        Self::new(None, None)
    }
}

/// The usual library directories of the three platforms, checked as plain
/// paths whatever the current one is (an absent directory is simply
/// absent), the way the Office previewer checks its program directories.
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

/// The pdfium download for this machine: the asset name of the official
/// builds.
fn download_asset() -> String {
    let os = match std::env::consts::OS {
        "macos" => "mac",
        "windows" => "win",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("pdfium-{os}-{arch}.tgz")
}

/// The library behind a binding attempt. pdfium-render keeps one set of
/// bindings per process: once anything has bound pdfium (this previewer, the Office previewer, an earlier
/// search), a new binding is refused with
/// `PdfiumLibraryBindingsAlreadyInitialized`, and `Pdfium::default()` then
/// hands out the bound library without loading anything.
fn pdfium_from(
    bound: Result<Box<dyn PdfiumLibraryBindings>, PdfiumError>,
) -> Result<Pdfium, PdfiumError> {
    match bound {
        Ok(bindings) => Ok(Pdfium::new(bindings)),
        Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Ok(Pdfium::default()),
        Err(error) => Err(error),
    }
}

impl Library {
    /// A library looked for everywhere, writing the hit back into
    /// `settings` when the host lent them; `config_dir` holds the per-user
    /// plugins directory.
    #[must_use]
    pub fn new(settings: Option<Arc<dyn SettingsStore>>, config_dir: Option<PathBuf>) -> Self {
        Self {
            settings,
            config_dir,
            search: Search::Everywhere,
            bound: Mutex::new(None),
        }
    }

    /// The same library, looked for in `search`.
    #[must_use]
    pub fn with_search(mut self, search: Search) -> Self {
        self.search = search;
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Arc<Pdfium>>> {
        self.bound.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The `library` setting as a path, when it is set.
    #[must_use]
    pub fn setting(&self) -> Option<PathBuf> {
        match self.settings.as_ref()?.get(ID, LIBRARY_SETTING)? {
            SettingValue::Text(text) if !text.trim().is_empty() => Some(PathBuf::from(text.trim())),
            _ => None,
        }
    }

    /// The library file a path names: itself, or the platform's library
    /// name inside it when it is a directory.
    fn file_at(path: &Path) -> PathBuf {
        if path.is_dir() {
            Pdfium::pdfium_platform_library_name_at_path(path)
        } else {
            path.to_path_buf()
        }
    }

    /// The files tried, in order: the setting, beside the binary, the
    /// plugins directories, the platform's library directories.
    #[must_use]
    pub fn candidates(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(setting) = self.setting() {
            out.push(Self::file_at(&setting));
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
            dirs.push(dir.join("plugins").join(ID.0));
        }
        if let Some(config) = &self.config_dir {
            dirs.push(config.join("plugins"));
            dirs.push(config.join("plugins").join(ID.0));
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

    /// The bound library, binding it on the first call: the candidates in
    /// order, then the system loader; the first hit goes into the setting.
    ///
    /// # Errors
    ///
    /// What the pane shows when nothing binds; the next call looks again.
    pub fn bind(&self) -> Result<Arc<Pdfium>, Box<MissingLibrary>> {
        if let Some(bound) = self.lock().as_ref() {
            return Ok(Arc::clone(bound));
        }
        let mut tried = Vec::new();
        let setting = self.setting();
        for path in self.candidates() {
            if !path.is_file() {
                tried.push(format!("{}: not there", path.display()));
                continue;
            }
            match pdfium_from(Pdfium::bind_to_library(&path)) {
                Ok(pdfium) => {
                    let pdfium = Arc::new(pdfium);
                    *self.lock() = Some(Arc::clone(&pdfium));
                    if setting.as_deref() != Some(path.as_path())
                        && let Some(settings) = &self.settings
                        && let Err(error) = settings.set(
                            ID,
                            LIBRARY_SETTING,
                            SettingValue::Text(path.display().to_string()),
                        )
                    {
                        tracing::warn!(%error, "the pdfium path was not written to the setting");
                    }
                    tracing::info!(path = %path.display(), "pdfium bound");
                    return Ok(pdfium);
                }
                Err(error) => tried.push(format!("{}: {error}", path.display())),
            }
        }
        if self.search == Search::Everywhere {
            match pdfium_from(Pdfium::bind_to_system_library()) {
                Ok(pdfium) => {
                    let pdfium = Arc::new(pdfium);
                    *self.lock() = Some(Arc::clone(&pdfium));
                    tracing::info!("pdfium bound from the system loader");
                    return Ok(pdfium);
                }
                Err(error) => tried.push(format!("the system loader: {error}")),
            }
        }
        Err(Box::new(self.missing(&tried.join("\n"))))
    }

    /// Whether the library binds on this machine.
    #[must_use]
    pub fn available(&self) -> bool {
        self.bind().is_ok()
    }

    /// The message for a library that is not there: the download for this
    /// platform, the setting *Browse…* fills, and `detail` on where it was
    /// looked for.
    #[must_use]
    pub fn missing(&self, detail: &str) -> MissingLibrary {
        MissingLibrary {
            name: "pdfium".to_owned(),
            install: DOWNLOADS.to_owned(),
            setting: (ID, LIBRARY_SETTING),
            filters: vec![DialogFilter {
                name: "pdfium library",
                extensions: &["so", "dylib", "dll"],
            }],
            detail: format!(
                "Download {} from the address above, unpack it and put {} beside the rick \
                 binary, or browse to it.\nLooked in:\n{detail}",
                download_asset(),
                Pdfium::pdfium_platform_library_name().to_string_lossy()
            ),
        }
    }
}

/// Previews PDF files.
pub struct PdfPreviewer {
    library: Arc<Library>,
}

impl std::fmt::Debug for PdfPreviewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PdfPreviewer")
            .field("library", &self.library)
            .finish()
    }
}

impl Default for PdfPreviewer {
    fn default() -> Self {
        Self::new(Arc::new(Library::default()))
    }
}

impl PdfPreviewer {
    /// A previewer binding pdfium through `library`.
    #[must_use]
    pub fn new(library: Arc<Library>) -> Self {
        Self { library }
    }

    /// Where the previewer looks for pdfium.
    #[must_use]
    pub fn library(&self) -> &Arc<Library> {
        &self.library
    }
}

/// Whether the pdfium library can be bound on this machine, looked for
/// everywhere but the setting.
#[must_use]
pub fn available() -> bool {
    Library::default().available()
}

/// The document shown for a PDF while pdfium is missing: the file's facts
/// and the message the pane lays out, with the text as the fallback.
pub struct MissingDoc {
    missing: Arc<MissingLibrary>,
    name: String,
    size: Option<u64>,
}

impl std::fmt::Debug for MissingDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MissingDoc")
            .field("name", &self.name)
            .field("library", &self.missing.name)
            .finish_non_exhaustive()
    }
}

impl MissingDoc {
    /// The document for the file `name` of `size` bytes.
    #[must_use]
    pub fn new(missing: MissingLibrary, name: String, size: Option<u64>) -> Self {
        Self {
            missing: Arc::new(missing),
            name,
            size,
        }
    }
}

impl PreviewDoc for MissingDoc {
    fn pages(&self) -> Option<usize> {
        None
    }

    fn render(&mut self, page: usize, _req: RenderRequest) -> Result<Frame, Error> {
        if page != 0 {
            return Err(Error::NotFound);
        }
        Ok(Frame::Text(TextBlock {
            text: format!(
                "{} is needed to show this file and was not found.
{}

{}",
                self.missing.name, self.missing.install, self.missing.detail
            ),
            language: None,
            spans: Vec::new(),
        }))
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        None
    }

    fn content(&self) -> Option<Content> {
        Some(Content::MissingLibrary(Arc::clone(&self.missing)))
    }

    fn sections(&self) -> Vec<InfoSection> {
        let mut rows = vec![("Name".to_owned(), self.name.clone())];
        if let Some(size) = self.size {
            rows.push(("Size".to_owned(), format_size(size)));
        }
        rows.push((
            "Preview".to_owned(),
            format!("{} not found", self.missing.name),
        ));
        vec![InfoSection {
            id: "file",
            title: Text("preview-pdf-file-info"),
            icon: Icon::Info,
            rows,
        }]
    }

    fn status(&self) -> Vec<String> {
        let mut status = vec!["PDF".to_owned()];
        if let Some(size) = self.size {
            status.push(format_size(size));
        }
        status.push(format!("{} not found", self.missing.name));
        status
    }

    fn controls(&self) -> Vec<PreviewControl> {
        Vec::new()
    }

    fn set_control(&mut self, _id: ControlId, _value: ControlValue) -> Result<(), Error> {
        Err(Error::Unsupported("nothing to control without the library"))
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

/// An opened PDF.
pub struct PdfDoc {
    pdfium: Arc<Pdfium>,
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
    /// Parses the document once for its page count and metadata, through
    /// the bound `pdfium`.
    ///
    /// # Errors
    ///
    /// `Error::Io` for a file pdfium cannot read.
    pub fn new(pdfium: Arc<Pdfium>, bytes: Vec<u8>) -> Result<Self, Error> {
        let document = pdfium
            .load_pdf_from_byte_slice(&bytes, None)
            .map_err(pdf_error)?;
        let pages = usize::try_from(document.pages().len()).unwrap_or(0);
        let mut info = Vec::new();
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
        info.push(("Pages".to_owned(), pages.to_string()));
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
        info.push(("Size".to_owned(), format_size(bytes.len() as u64)));
        drop(document);
        Ok(Self {
            pdfium,
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
        let document = self
            .pdfium
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

    fn sections(&self) -> Vec<InfoSection> {
        vec![InfoSection {
            id: "file",
            title: Text("preview-pdf-file-info"),
            icon: Icon::Info,
            rows: self.info.clone(),
        }]
    }

    fn status(&self) -> Vec<String> {
        vec![
            format!(
                "{} page{}",
                self.pages,
                if self.pages == 1 { "" } else { "s" }
            ),
            format_size(self.bytes.len() as u64),
        ]
    }

    fn controls(&self) -> Vec<PreviewControl> {
        Vec::new()
    }

    fn set_control(&mut self, _id: ControlId, _value: ControlValue) -> Result<(), Error> {
        Err(Error::Unsupported(
            "PDF documents have no controls of their own",
        ))
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
        // The library is resolved here, on the first file shown, never at
        // start; a miss is looked up again on every file (and *Retry*).
        let library = Arc::clone(&self.library);
        let size = bytes.len() as u64;
        let name = loc
            .path
            .file_name()
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        ext::blocking(move || match library.bind() {
            Ok(pdfium) => Ok(Box::new(PdfDoc::new(pdfium, bytes)?) as Box<dyn PreviewDoc>),
            Err(missing) => Ok(Box::new(MissingDoc::new(*missing, name, Some(size)))),
        })
        .await
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
        let p = PdfPreviewer::default();
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

    /// A store of one extension's values in memory, for the tests.
    #[derive(Default)]
    struct Memory(Mutex<Vec<(String, SettingValue)>>);

    impl SettingsStore for Memory {
        fn get(&self, ext: ExtId, key: &str) -> Option<SettingValue> {
            assert_eq!(ext, ID);
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        }
        fn set(&self, ext: ExtId, key: &str, value: SettingValue) -> Result<(), Error> {
            assert_eq!(ext, ID);
            let mut values = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            values.retain(|(k, _)| k != key);
            values.push((key.to_owned(), value));
            Ok(())
        }
    }

    #[test]
    fn the_setting_comes_first_then_the_binary_and_plugin_directories() {
        let store = Arc::new(Memory::default());
        store
            .set(
                ID,
                LIBRARY_SETTING,
                SettingValue::Text("/opt/pdf/libx.so".into()),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let library = Library::new(
            Some(store as Arc<dyn SettingsStore>),
            Some(config.path().to_path_buf()),
        );
        let candidates = library.candidates();
        assert_eq!(candidates[0], PathBuf::from("/opt/pdf/libx.so"));
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| panic!("exe"));
        let name = Pdfium::pdfium_platform_library_name();
        assert_eq!(candidates[1], exe_dir.join(&name), "beside the binary");
        assert_eq!(candidates[2], exe_dir.join("plugins").join(&name));
        assert!(candidates.contains(&config.path().join("plugins").join(&name)));
        assert!(candidates.contains(&PathBuf::from("/usr/lib").join(&name)));
        // The setting alone, for a machine without the library.
        let only = Library::new(None, None).with_search(Search::SettingOnly);
        assert_eq!(only.candidates(), Vec::<PathBuf>::new());
        let Err(missing) = only.bind() else {
            panic!("nothing to bind to");
        };
        assert_eq!(missing.name, "pdfium");
        assert_eq!(missing.setting, (ID, LIBRARY_SETTING));
        assert!(missing.install.starts_with("https://"));
        assert!(missing.detail.contains("pdfium-"), "{}", missing.detail);
        assert_eq!(missing.filters[0].extensions, ["so", "dylib", "dll"]);
    }

    #[test]
    fn a_setting_naming_a_directory_means_the_library_inside_it() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let store = Arc::new(Memory::default());
        store
            .set(
                ID,
                LIBRARY_SETTING,
                SettingValue::Text(dir.path().display().to_string()),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        let library = Library::new(Some(store as Arc<dyn SettingsStore>), None)
            .with_search(Search::SettingOnly);
        assert_eq!(
            library.candidates(),
            [Pdfium::pdfium_platform_library_name_at_path(dir.path())]
        );
    }

    /// Needs pdfium beside the test binary, in the platform's library
    /// directories or in the system loader's path. The acceptance run in
    /// CI has it beside the binary and writes the setting back.
    #[test]
    fn pages_render_and_carry_their_text_and_the_hit_is_written_to_the_setting() {
        let store = Arc::new(Memory::default());
        let library = Library::new(Some(Arc::clone(&store) as Arc<dyn SettingsStore>), None);
        let Ok(pdfium) = library.bind() else {
            eprintln!("pdfium not available; the render is not checked here");
            assert!(
                std::env::var_os("RICK_REQUIRE_PDFIUM").is_none(),
                "pdfium required but not found"
            );
            return;
        };
        // Found by its path (not the system loader): the setting now says
        // where, and the next look starts there.
        if let Some(SettingValue::Text(path)) = store.get(ID, LIBRARY_SETTING) {
            assert!(Path::new(&path).is_file(), "{path}");
            assert_eq!(library.candidates()[0], PathBuf::from(path));
        }
        assert!(library.available(), "bound once, bound for good");
        let mut doc = PdfDoc::new(pdfium, sample_pdf(5)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(doc.pages(), Some(5));
        let sections = doc.sections();
        assert_eq!(sections.len(), 1);
        assert_eq!(
            (sections[0].id, sections[0].title.key()),
            ("file", "preview-pdf-file-info")
        );
        let rows = &sections[0].rows;
        let row = |key: &str| {
            rows.iter()
                .find(|(k, _)| k == key)
                .map_or_else(|| panic!("no row {key}: {rows:?}"), |(_, v)| v.clone())
        };
        assert_eq!(row("Title"), "Rick sample");
        assert_eq!(row("Producer"), "Rick fixture");
        assert_eq!(row("Pages"), "5");
        assert_eq!(row("Page size"), "612 × 792 pt");
        assert!(row("Size").ends_with("KiB") || row("Size").ends_with('B'));
        assert_eq!(doc.status()[0], "5 pages");
        assert_eq!(doc.status().len(), 2);
        assert_eq!(doc.controls().len(), 0);
        assert!(
            doc.set_control(ControlId("x"), ControlValue::Pressed)
                .is_err()
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
