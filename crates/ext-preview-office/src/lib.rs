//! Office previews: DOCX, XLSX, PPTX, ODT, ODS and ODP, converted to PDF by
//! LibreOffice running headless and shown through the PDF path; without
//! LibreOffice, the metadata the file carries and a hint to install it.
//!
//! `open` reads the file's metadata from its zip (title, author, dates,
//! page, sheet or slide count), looks for `soffice` the way SPEC.md's
//! *Third-party libraries* says, lazily on the first file shown: the
//! `soffice` setting of `preview-office.toml` when it is set, then beside
//! the binary and the plugins directory, the `PATH` and the usual install
//! directories of the three platforms, the first hit written back into
//! the setting through the host's [`SettingsStore`]; no environment
//! variable is read. When it is there the conversion runs on the blocking
//! pool inside the preview job: cancelling the preview kills the process,
//! and so does the [`CONVERT_TIMEOUT`]. The PDF lands in a cache under the
//! temporary directory, keyed by the file's path, size and modification
//! time, so a second look costs nothing; the cache is trimmed to
//! [`CACHE_LIMIT`]. Pages render through pdfium like the PDF previewer's
//! (`pdf`). Without LibreOffice (or pdfium) the document's content is
//! [`Content::MissingLibrary`]: the pane shows the file's facts, the
//! install command for this platform, *Browse…* and *Retry*. LibreOffice
//! is never bundled.

#![deny(unsafe_code)]

pub mod fixtures;
pub mod meta;
pub mod pdf;

use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use ext::Text;
use ext::platform::DialogFilter;
use ext::{
    API_VERSION, Capabilities, Content, ControlId, ControlValue, Extension, Frame, Icon,
    InfoSection, Manifest, MissingLibrary, Platforms, Points, PreviewControl, PreviewDoc,
    Previewer, Provider, Registry, RenderRequest, SettingField, SettingKind, SettingValue,
    SettingsSchema, SettingsStore, TextBlock, TextLayer, Transport, Version, async_trait,
};
use pdfium_render::prelude::Pdfium;
use types::{CancelToken, Entry, EntryKind, Error, ExtId, Location};

pub use meta::Meta;
pub use pdf::{Binding, Pages, Search};

/// The extension id.
pub const ID: ExtId = ExtId("preview-office");

/// Files above this are not converted; the preview shows metadata only.
pub const MAX_BYTES: usize = 256 * 1024 * 1024;

/// A conversion running longer than this is killed and reported.
pub const CONVERT_TIMEOUT: Duration = Duration::from_mins(1);

/// The cache keeps at most this many bytes of converted PDFs, oldest out.
pub const CACHE_LIMIT: u64 = 512 * 1024 * 1024;

/// The cache keeps at most this many converted PDFs.
pub const CACHE_FILES: usize = 200;

/// The setting that holds the path of the LibreOffice program (`soffice`,
/// or the directory it is in), in `preview-office.toml`.
pub const SOFFICE_SETTING: &str = "soffice";

/// The file name extensions the manifest claims: the six formats.
const PREVIEWS: &[&str] = &["docx", "xlsx", "pptx", "odt", "ods", "odp"];

/// Where to get LibreOffice, as the hint says.
pub const DOWNLOAD_URL: &str = "https://www.libreoffice.org/download/";

/// The extension: registers [`OfficePreviewer`].
#[derive(Debug)]
pub struct OfficePreview {
    manifest: Manifest,
}

impl Default for OfficePreview {
    fn default() -> Self {
        Self::new()
    }
}

impl OfficePreview {
    /// The extension with its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Office preview",
                description: "Previews office documents, Word, Excel, PowerPoint and OpenDocument, by converting them with LibreOffice.",
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

impl Extension for OfficePreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        reg.settings(settings_schema());
        let settings = reg.settings_store().cloned();
        let config_dir = reg.platform().map(|platform| platform.config_dir());
        reg.previewer(Arc::new(OfficePreviewer::new(
            Arc::new(Lookup::new(settings.clone(), config_dir.clone())),
            Arc::new(Binding::new(settings, config_dir)),
        )));
        Ok(())
    }
}

/// The settings the Settings window shows for `preview-office.toml`: the
/// program's path.
#[must_use]
pub fn settings_schema() -> SettingsSchema {
    SettingsSchema {
        ext: ID,
        fields: vec![SettingField {
            key: SOFFICE_SETTING,
            label: Text("preview-office-soffice"),
            kind: SettingKind::Text {
                default: String::new(),
            },
        }],
    }
}

ext::plugin!(|| Arc::new(OfficePreview::new()) as Arc<dyn Extension>);

/// Previews office documents.
#[derive(Debug)]
pub struct OfficePreviewer {
    soffice: Arc<Lookup>,
    pdfium: Arc<Binding>,
}

impl Default for OfficePreviewer {
    fn default() -> Self {
        Self::new(Arc::new(Lookup::default()), Arc::new(Binding::default()))
    }
}

impl OfficePreviewer {
    /// A previewer finding LibreOffice through `soffice` and pdfium through
    /// `pdfium`.
    #[must_use]
    pub fn new(soffice: Arc<Lookup>, pdfium: Arc<Binding>) -> Self {
        Self { soffice, pdfium }
    }

    /// Where the previewer looks for LibreOffice.
    #[must_use]
    pub fn soffice(&self) -> &Arc<Lookup> {
        &self.soffice
    }
}

/// The six formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Word, Office Open XML.
    Docx,
    /// Excel, Office Open XML.
    Xlsx,
    /// `PowerPoint`, Office Open XML.
    Pptx,
    /// OpenDocument text.
    Odt,
    /// OpenDocument spreadsheet.
    Ods,
    /// OpenDocument presentation.
    Odp,
}

impl Kind {
    /// All six, for the fixtures and the tests.
    pub const ALL: [Self; 6] = [
        Self::Docx,
        Self::Xlsx,
        Self::Pptx,
        Self::Odt,
        Self::Ods,
        Self::Odp,
    ];

    /// The file extension, lower case, without the dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
            Self::Odt => "odt",
            Self::Ods => "ods",
            Self::Odp => "odp",
        }
    }

    /// The kind for a file extension.
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|k| k.extension().eq_ignore_ascii_case(ext))
    }

    /// The name shown in the status bar.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Docx => "Word document",
            Self::Xlsx => "Excel workbook",
            Self::Pptx => "PowerPoint presentation",
            Self::Odt => "OpenDocument text",
            Self::Ods => "OpenDocument spreadsheet",
            Self::Odp => "OpenDocument presentation",
        }
    }

    /// What the kind counts: pages, sheets or slides.
    #[must_use]
    pub fn unit(self) -> &'static str {
        match self {
            Self::Docx | Self::Odt => "page",
            Self::Xlsx | Self::Ods => "sheet",
            Self::Pptx | Self::Odp => "slide",
        }
    }

    /// Whether the format is OpenDocument rather than Office Open XML.
    #[must_use]
    pub fn is_opendocument(self) -> bool {
        matches!(self, Self::Odt | Self::Ods | Self::Odp)
    }

    /// The OpenDocument media type, for the `mimetype` entry.
    #[must_use]
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            Self::Odt => "application/vnd.oasis.opendocument.text",
            Self::Ods => "application/vnd.oasis.opendocument.spreadsheet",
            Self::Odp => "application/vnd.oasis.opendocument.presentation",
        }
    }
}

/// The lower-case extension of a name, if any.
fn extension(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// What the file is, with the score to claim it: 190 for an OpenDocument
/// file whose `mimetype` entry says so (whatever its name), 180 for a
/// known extension on a zip, 60 for a known extension on something that
/// is no zip, `None` otherwise.
#[must_use]
pub fn detect(name: &str, sniff: &[u8]) -> Option<(Kind, u8)> {
    let is_zip = sniff.starts_with(b"PK\x03\x04");
    if is_zip && sniff.len() > 38 && &sniff[30..38] == b"mimetype" {
        let text = String::from_utf8_lossy(&sniff[38..sniff.len().min(140)]);
        let kind = if text.contains("application/vnd.oasis.opendocument.text") {
            Some(Kind::Odt)
        } else if text.contains("application/vnd.oasis.opendocument.spreadsheet") {
            Some(Kind::Ods)
        } else if text.contains("application/vnd.oasis.opendocument.presentation") {
            Some(Kind::Odp)
        } else {
            None
        };
        if let Some(kind) = kind {
            return Some((kind, 190));
        }
    }
    let kind = Kind::from_extension(&extension(name)?)?;
    Some((kind, if is_zip || sniff.is_empty() { 180 } else { 60 }))
}

/// The executable names the lookup tries on the `PATH` and in a directory.
const PATH_NAMES: [&str; 3] = ["soffice", "libreoffice", "soffice.exe"];

/// The install directories of the three platforms, checked whatever the
/// current one is (a path that does not exist is simply absent).
fn standard_locations() -> Vec<PathBuf> {
    let mut out = vec![
        PathBuf::from("/usr/bin/soffice"),
        PathBuf::from("/usr/local/bin/soffice"),
        PathBuf::from("/usr/lib/libreoffice/program/soffice"),
        PathBuf::from("/usr/lib64/libreoffice/program/soffice"),
        PathBuf::from("/snap/bin/libreoffice"),
        PathBuf::from("/var/lib/flatpak/exports/bin/org.libreoffice.LibreOffice"),
        PathBuf::from("/Applications/LibreOffice.app/Contents/MacOS/soffice"),
    ];
    if let Ok(entries) = std::fs::read_dir("/opt") {
        for entry in entries.filter_map(Result::ok) {
            if entry
                .file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with("libreoffice")
            {
                out.push(entry.path().join("program").join("soffice"));
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        out.push(
            PathBuf::from(&home)
                .join("Applications")
                .join("LibreOffice.app")
                .join("Contents")
                .join("MacOS")
                .join("soffice"),
        );
        out.push(
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("flatpak")
                .join("exports")
                .join("bin")
                .join("org.libreoffice.LibreOffice"),
        );
    }
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(var) {
            let base = PathBuf::from(base);
            out.push(base.join("LibreOffice").join("program").join("soffice.exe"));
            out.push(
                base.join("Programs")
                    .join("LibreOffice")
                    .join("program")
                    .join("soffice.exe"),
            );
        }
    }
    out
}

/// Where LibreOffice is looked for and, once found, remembered.
pub struct Lookup {
    settings: Option<Arc<dyn SettingsStore>>,
    config_dir: Option<PathBuf>,
    search: Search,
    known: Mutex<Option<PathBuf>>,
}

impl std::fmt::Debug for Lookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lookup")
            .field("search", &self.search)
            .field("known", &*self.lock())
            .finish_non_exhaustive()
    }
}

impl Default for Lookup {
    fn default() -> Self {
        Self::new(None, None)
    }
}

/// The program a path names: itself, or `soffice` inside it when it is a
/// directory.
fn program_at(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        PATH_NAMES
            .iter()
            .map(|name| path.join(name))
            .find(|candidate| candidate.is_file())
    } else {
        Some(path.to_path_buf())
    }
}

/// What installs LibreOffice on this platform.
fn install_hint() -> String {
    match std::env::consts::OS {
        "linux" => "sudo apt install libreoffice".to_owned(),
        "macos" => "brew install --cask libreoffice".to_owned(),
        "windows" => "winget install TheDocumentFoundation.LibreOffice".to_owned(),
        _ => DOWNLOAD_URL.to_owned(),
    }
}

impl Lookup {
    /// A lookup over every place, writing the hit back into `settings`
    /// when the host lent them; `config_dir` holds the per-user plugins
    /// directory.
    #[must_use]
    pub fn new(settings: Option<Arc<dyn SettingsStore>>, config_dir: Option<PathBuf>) -> Self {
        Self {
            settings,
            config_dir,
            search: Search::Everywhere,
            known: Mutex::new(None),
        }
    }

    /// The same lookup, over `search` only.
    #[must_use]
    pub fn with_search(mut self, search: Search) -> Self {
        self.search = search;
        self
    }

    fn lock(&self) -> MutexGuard<'_, Option<PathBuf>> {
        self.known.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The `soffice` setting as a path, when it is set.
    #[must_use]
    pub fn setting(&self) -> Option<PathBuf> {
        match self.settings.as_ref()?.get(ID, SOFFICE_SETTING)? {
            SettingValue::Text(text) if !text.trim().is_empty() => Some(PathBuf::from(text.trim())),
            _ => None,
        }
    }

    /// The places tried, in order: the setting, beside the binary and the
    /// plugins directories, the `PATH`, the platforms' install directories.
    #[must_use]
    pub fn candidates(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(setting) = self.setting() {
            out.push(program_at(&setting).unwrap_or(setting));
        }
        if self.search == Search::SettingOnly {
            return out;
        }
        let mut dirs = Vec::new();
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            dirs.push(dir.to_path_buf());
            dirs.push(dir.join("plugins").join(ID.0));
        }
        if let Some(config) = &self.config_dir {
            dirs.push(config.join("plugins").join(ID.0));
        }
        if let Some(paths) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&paths));
        }
        for dir in dirs {
            for name in PATH_NAMES {
                let candidate = dir.join(name);
                if !out.contains(&candidate) {
                    out.push(candidate);
                }
            }
        }
        for path in standard_locations() {
            if !out.contains(&path) {
                out.push(path);
            }
        }
        out
    }

    /// The LibreOffice program: the one remembered while it is still
    /// there, else the first candidate that is a file, written into the
    /// setting. A miss is looked up again next time, so installing
    /// LibreOffice while Rick runs is picked up.
    ///
    /// # Errors
    ///
    /// What the pane shows when nothing is found.
    pub fn find(&self) -> Result<PathBuf, Box<MissingLibrary>> {
        if let Some(path) = self.lock().as_ref().filter(|p| p.is_file()) {
            return Ok(path.clone());
        }
        let setting = self.setting();
        let candidates = self.candidates();
        let Some(found) = candidates.iter().find(|p| p.is_file()).cloned() else {
            return Err(Box::new(
                self.missing(
                    &candidates
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            ));
        };
        if setting.as_deref() != Some(found.as_path())
            && let Some(settings) = &self.settings
            && let Err(error) = settings.set(
                ID,
                SOFFICE_SETTING,
                SettingValue::Text(found.display().to_string()),
            )
        {
            tracing::warn!(%error, "the LibreOffice path was not written to the setting");
        }
        *self.lock() = Some(found.clone());
        Ok(found)
    }

    /// The message for a LibreOffice that is not there: the install
    /// command for this platform, the setting *Browse…* fills, and
    /// `detail` on where it was looked for.
    #[must_use]
    pub fn missing(&self, detail: &str) -> MissingLibrary {
        MissingLibrary {
            name: "LibreOffice".to_owned(),
            install: install_hint(),
            setting: (ID, SOFFICE_SETTING),
            filters: vec![DialogFilter {
                name: "LibreOffice program",
                extensions: &["exe", ""],
            }],
            detail: format!(
                "Rick shows office documents by converting them to PDF with LibreOffice \
                 ({DOWNLOAD_URL}). It was looked for at:\n{detail}"
            ),
        }
    }
}

/// Looks for the LibreOffice executable everywhere but a setting: the
/// `PATH`, then the standard install directories.
#[must_use]
pub fn find_soffice() -> Option<PathBuf> {
    Lookup::default().find().ok()
}

/// Whether LibreOffice can be found on this machine.
#[must_use]
pub fn available() -> bool {
    find_soffice().is_some()
}

/// The cache directory: `<temp dir>/rick/office` (QUESTIONS.md 104).
#[must_use]
pub fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("rick").join("office")
}

/// The cache file for `path` as it was at `modified` with `size` bytes.
#[must_use]
pub fn cache_key(path: &Path, modified: Option<SystemTime>, size: Option<u64>) -> String {
    let mut hasher = std::hash::DefaultHasher::new();
    path.hash(&mut hasher);
    modified
        .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| (d.as_secs(), d.subsec_nanos()))
        .hash(&mut hasher);
    size.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Drops the oldest cached PDFs until the cache fits its limits.
fn trim_cache(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "pdf"))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                meta.len(),
                e.path(),
            ))
        })
        .collect();
    files.sort();
    let mut total: u64 = files.iter().map(|(_, len, _)| *len).sum();
    let mut count = files.len();
    for (_, len, path) in files {
        if total <= CACHE_LIMIT && count <= CACHE_FILES {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
            count -= 1;
        }
    }
}

/// Why a conversion did not produce a PDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertError {
    /// LibreOffice was not found.
    NotInstalled,
    /// The process ran longer than [`CONVERT_TIMEOUT`].
    TimedOut,
    /// The process ended without a PDF; its last output.
    Failed(String),
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled => write!(f, "LibreOffice is not installed"),
            Self::TimedOut => write!(
                f,
                "LibreOffice took longer than {} s",
                CONVERT_TIMEOUT.as_secs()
            ),
            Self::Failed(output) if output.is_empty() => {
                write!(f, "LibreOffice did not produce a PDF")
            }
            Self::Failed(output) => write!(f, "LibreOffice did not produce a PDF: {output}"),
        }
    }
}

/// Converts `input` to a PDF with `soffice`, writing it to `pdf`; the
/// process is killed when `cancel` fires or [`CONVERT_TIMEOUT`] passes.
/// Blocking: call it on the blocking pool.
///
/// # Errors
///
/// [`Error::Cancelled`] when cancelled; otherwise a [`ConvertError`]
/// wrapped as `Error::Extension`.
pub fn convert(
    soffice: &Path,
    input: &Path,
    pdf: &Path,
    cancel: &CancelToken,
) -> Result<(), Error> {
    let cache = pdf.parent().map_or_else(cache_dir, Path::to_path_buf);
    std::fs::create_dir_all(&cache).map_err(Error::from_io)?;
    let work = tempfile::Builder::new()
        .prefix("convert-")
        .tempdir_in(&cache)
        .map_err(Error::from_io)?;
    // Its own user profile: a LibreOffice the user has open would
    // otherwise take the command and return at once with nothing.
    let profile = cache.join("profile");
    let profile_uri = format!(
        "file://{}",
        profile
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
    );
    let profile_uri = if profile_uri.starts_with("file:///") {
        profile_uri
    } else {
        profile_uri.replacen("file://", "file:///", 1)
    };
    let mut child = Command::new(soffice)
        .arg("--headless")
        .arg("--norestore")
        .arg("--nologo")
        .arg("--nodefault")
        .arg(format!("-env:UserInstallation={profile_uri}"))
        .arg("--convert-to")
        .arg("pdf")
        .arg("--outdir")
        .arg(work.path())
        .arg(input)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| extension_error(ConvertError::Failed(format!("could not start: {e}"))))?;
    let deadline = Instant::now() + CONVERT_TIMEOUT;
    let status = loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Cancelled);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(extension_error(ConvertError::TimedOut));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(Error::from_io(e)),
        }
    };
    let output = child.wait_with_output().map_or_default(|o| {
        let stderr = String::from_utf8_lossy(&o.stderr);
        let stdout = String::from_utf8_lossy(&o.stdout);
        let text = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        text.lines().last().unwrap_or("").to_owned()
    });
    let produced = std::fs::read_dir(work.path())
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "pdf"));
    match produced {
        Some(path) if status.success() => {
            std::fs::rename(&path, pdf)
                .or_else(|_| std::fs::copy(&path, pdf).map(|_| ()))
                .map_err(Error::from_io)?;
            trim_cache(&cache);
            Ok(())
        }
        _ => Err(extension_error(ConvertError::Failed(output))),
    }
}

fn extension_error(error: ConvertError) -> Error {
    Error::Extension {
        ext: ID,
        source: Box::new(ConvertErrorWrapper(error)),
    }
}

/// `ConvertError` as a `std::error::Error`.
#[derive(Debug)]
struct ConvertErrorWrapper(ConvertError);

impl std::fmt::Display for ConvertErrorWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ConvertErrorWrapper {}

/// The pages, or why there are none.
#[derive(Debug)]
pub enum Rendering {
    /// The converted PDF, through pdfium.
    Pages(Pages),
    /// No PDF: the conversion failed, or the file is too large.
    None(String),
    /// No PDF: LibreOffice or pdfium is not there; the pane shows the
    /// message with *Browse…* and *Retry*.
    Missing(Arc<MissingLibrary>),
}

/// An opened office document.
#[derive(Debug)]
pub struct OfficeDoc {
    kind: Kind,
    meta: Meta,
    rendering: Rendering,
    size: Option<u64>,
    /// How the PDF came to be: converted now, or from the cache.
    cached: bool,
}

impl OfficeDoc {
    /// A document of `kind` with its metadata and the result of the
    /// conversion.
    #[must_use]
    pub fn new(
        kind: Kind,
        meta: Meta,
        rendering: Rendering,
        size: Option<u64>,
        cached: bool,
    ) -> Self {
        Self {
            kind,
            meta,
            rendering,
            size,
            cached,
        }
    }

    /// The kind.
    #[must_use]
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The metadata read from the file.
    #[must_use]
    pub fn meta(&self) -> &Meta {
        &self.meta
    }

    /// Whether the pages come from a PDF.
    #[must_use]
    pub fn has_pages(&self) -> bool {
        matches!(self.rendering, Rendering::Pages(_))
    }

    /// The count shown: the PDF's pages when converted, else what the
    /// file states.
    fn count(&self) -> Option<(usize, &'static str)> {
        match &self.rendering {
            Rendering::Pages(pages) => Some((pages.count(), "page")),
            Rendering::None(_) | Rendering::Missing(_) => {
                self.meta.count.map(|n| (n, self.kind.unit()))
            }
        }
    }

    /// Why there are no pages, in one line.
    fn reason(&self) -> String {
        match &self.rendering {
            Rendering::Pages(_) => String::new(),
            Rendering::None(reason) => reason.clone(),
            Rendering::Missing(missing) => format!("{} is not installed", missing.name),
        }
    }

    /// The text shown when there are no pages: the reason, the install
    /// hint when a library is missing, and the metadata.
    fn fallback_text(&self, reason: &str) -> String {
        let mut text = String::new();
        text.push_str(self.kind.name());
        text.push_str("\n\n");
        text.push_str(reason);
        text.push('\n');
        if let Rendering::Missing(missing) = &self.rendering {
            text.push_str(&missing.install);
            text.push('\n');
            text.push_str(&missing.detail);
            text.push('\n');
        }
        if !self.meta.rows.is_empty() || self.meta.count.is_some() {
            text.push('\n');
            for (key, value) in &self.meta.rows {
                text.push_str(key);
                text.push_str(": ");
                text.push_str(value);
                text.push('\n');
            }
            if let Some((count, unit)) = self.count() {
                let _ = writeln!(text, "{count} {unit}{}", if count == 1 { "" } else { "s" });
            }
        }
        text
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

impl PreviewDoc for OfficeDoc {
    fn pages(&self) -> Option<usize> {
        match &self.rendering {
            Rendering::Pages(pages) => Some(pages.count()),
            Rendering::None(_) | Rendering::Missing(_) => None,
        }
    }

    fn render(&mut self, page: usize, req: RenderRequest) -> Result<Frame, Error> {
        match &mut self.rendering {
            Rendering::Pages(pages) => pages.render(page, req),
            Rendering::None(_) | Rendering::Missing(_) => {
                if page != 0 {
                    return Err(Error::NotFound);
                }
                let reason = self.reason();
                Ok(Frame::Text(TextBlock {
                    text: self.fallback_text(&reason),
                    language: None,
                    spans: Vec::new(),
                }))
            }
        }
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        match &self.rendering {
            Rendering::Pages(pages) => Some(pages.layer()),
            Rendering::None(_) | Rendering::Missing(_) => None,
        }
    }

    fn content(&self) -> Option<Content> {
        match &self.rendering {
            Rendering::Missing(missing) => Some(Content::MissingLibrary(Arc::clone(missing))),
            Rendering::Pages(_) | Rendering::None(_) => None,
        }
    }

    fn sections(&self) -> Vec<InfoSection> {
        let mut rows = vec![("Kind".to_owned(), self.kind.name().to_owned())];
        rows.extend(self.meta.rows.iter().cloned());
        if let Some((count, unit)) = self.count() {
            let key = format!("{}{}", unit[..1].to_ascii_uppercase(), &unit[1..]);
            rows.push((format!("{key}s"), count.to_string()));
        }
        if let Some(size) = self.size {
            rows.push(("Size".to_owned(), format_size(size)));
        }
        rows.push((
            "Preview".to_owned(),
            match &self.rendering {
                Rendering::Pages(_) if self.cached => {
                    "converted to PDF by LibreOffice (cached)".to_owned()
                }
                Rendering::Pages(_) => "converted to PDF by LibreOffice".to_owned(),
                Rendering::None(_) | Rendering::Missing(_) => self.reason(),
            },
        ));
        vec![InfoSection {
            id: "file",
            title: Text("preview-office-file-info"),
            icon: Icon::Info,
            rows,
        }]
    }

    fn status(&self) -> Vec<String> {
        let mut status = vec![self.kind.name().to_owned()];
        if let Some((count, unit)) = self.count() {
            status.push(format!(
                "{count} {unit}{}",
                if count == 1 { "" } else { "s" }
            ));
        }
        if let Some(size) = self.size {
            status.push(format_size(size));
        }
        if matches!(self.rendering, Rendering::None(_) | Rendering::Missing(_)) {
            status.push(self.reason());
        }
        status
    }

    fn controls(&self) -> Vec<PreviewControl> {
        Vec::new()
    }

    fn set_control(&mut self, _id: ControlId, _value: ControlValue) -> Result<(), Error> {
        Err(Error::Unsupported(
            "office documents have no controls of their own",
        ))
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

/// The OS path of a local location; `None` for other providers.
fn local_path(loc: &Location) -> Option<PathBuf> {
    (loc.provider.0 == "local").then(|| PathBuf::from(loc.path.to_string_lossy().into_owned()))
}

/// Converts `input` (the file at `path` as the key says) unless the cache
/// has it, and parses the PDF. Blocking.
fn pages_for(
    soffice: &Path,
    pdfium: &Arc<Pdfium>,
    input: &Path,
    key: &str,
    cancel: &CancelToken,
) -> Result<(Pages, bool), Error> {
    let dir = cache_dir();
    let pdf = dir.join(format!("{key}.pdf"));
    if let Ok(bytes) = std::fs::read(&pdf)
        && let Ok(pages) = Pages::new(Arc::clone(pdfium), bytes)
    {
        tracing::debug!(key, "office preview from the cache");
        return Ok((pages, true));
    }
    let started = Instant::now();
    convert(soffice, input, &pdf, cancel)?;
    tracing::info!(
        key,
        took_ms = started.elapsed().as_millis(),
        "converted to PDF"
    );
    let bytes = std::fs::read(&pdf).map_err(Error::from_io)?;
    Ok((Pages::new(Arc::clone(pdfium), bytes)?, false))
}

#[async_trait]
impl Previewer for OfficePreviewer {
    fn id(&self) -> ExtId {
        ID
    }

    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8 {
        if entry.kind != EntryKind::File {
            return 0;
        }
        detect(&entry.name, sniff).map_or(0, |(_, score)| score)
    }

    async fn open(
        &self,
        loc: &Location,
        provider: Arc<dyn Provider>,
        cancel: CancelToken,
    ) -> Result<Box<dyn PreviewDoc>, Error> {
        let entry = provider.stat(loc).await?;
        let bytes = ext::read_bytes(&*provider, loc, Some(MAX_BYTES + 1), &cancel).await?;
        let too_big = bytes.len() > MAX_BYTES;
        let (kind, _) = detect(&entry.name, &bytes[..bytes.len().min(4096)])
            .ok_or(Error::Unsupported("not an office document"))?;
        let size = entry.size;
        let modified = entry.modified;
        let name = entry.name.clone();
        let local = local_path(loc);
        let token = cancel.clone();
        let lookup = Arc::clone(&self.soffice);
        let binding = Arc::clone(&self.pdfium);
        let doc = ext::blocking(move || {
            let meta = meta::read(&bytes, kind);
            if too_big {
                return Ok(OfficeDoc::new(
                    kind,
                    meta,
                    Rendering::None("the document is larger than 256 MiB".to_owned()),
                    size,
                    false,
                ));
            }
            // Both libraries are resolved here, on the first file shown,
            // never at start; a miss is looked up again on every file.
            let soffice = match lookup.find() {
                Ok(soffice) => soffice,
                Err(missing) => {
                    return Ok(OfficeDoc::new(
                        kind,
                        meta,
                        Rendering::Missing(Arc::new(*missing)),
                        size,
                        false,
                    ));
                }
            };
            let pdfium = match binding.bind() {
                Ok(pdfium) => pdfium,
                Err(missing) => {
                    return Ok(OfficeDoc::new(
                        kind,
                        meta,
                        Rendering::Missing(Arc::new(*missing)),
                        size,
                        false,
                    ));
                }
            };
            // A file on another provider is copied next to the cache with
            // its extension kept, which LibreOffice reads the type from.
            let temp;
            let (input, key): (&Path, String) = if let Some(path) = &local {
                (path, cache_key(path, modified, size))
            } else {
                let dir = cache_dir();
                std::fs::create_dir_all(&dir).map_err(Error::from_io)?;
                temp = tempfile::Builder::new()
                    .prefix("remote-")
                    .suffix(&format!(".{}", kind.extension()))
                    .tempfile_in(&dir)
                    .map_err(Error::from_io)?;
                std::fs::write(temp.path(), &bytes).map_err(Error::from_io)?;
                (temp.path(), cache_key(Path::new(&name), modified, size))
            };
            let (rendering, cached) = match pages_for(&soffice, &pdfium, input, &key, &token) {
                Ok((pages, cached)) => (Rendering::Pages(pages), cached),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(error) => {
                    tracing::warn!(%error, "office conversion failed");
                    (Rendering::None(error.to_string()), false)
                }
            };
            Ok(OfficeDoc::new(kind, meta, rendering, size, cached))
        })
        .await?;
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        Ok(Box::new(doc))
    }
}

#[cfg(test)]
mod tests;
