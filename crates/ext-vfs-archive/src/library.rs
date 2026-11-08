//! Finding the 7-Zip library the disk images are read with, as *Third-party
//! libraries* (SPEC.md) wants it: lazily, on the first image shown, in
//! the order setting → beside the binary and the plugins directory → the
//! platform's usual install locations, the first hit written back into the
//! setting (`sevenzip_library` in `vfs-archive.toml`) so the user can see
//! and change where Rick looks. Nothing is probed at start, and no
//! environment variable is read. The setting is read and written through
//! the [`SettingsStore`] the registry lends (QUESTIONS.md 193); without
//! one, as in the compress plugin's extraction, nothing is remembered.
//! When nothing is found, [`MissingLibrary`] names the library, how it is
//! installed on this platform and where Rick looked; the provider raises
//! it as an error, and the archive previewer, which shares this resolver so
//! the library is one setting for both plugins (QUESTIONS.md 204), shows
//! it in the pane as [`ext::MissingLibrary`] ([`MissingLibrary::for_pane`])
//! with *Browse…*, whose choice the pane stores into the setting, and
//! *Retry*.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use ext::platform::DialogFilter;
use ext::platform::sevenzip::{self, SevenZip, SevenZipError};
use ext::{SettingField, SettingKind, SettingValue, SettingsSchema, SettingsStore, Text};
use types::Error;

use crate::ID;

/// The setting's key in `vfs-archive.toml`: the path of the library.
pub const LIBRARY_SETTING: &str = "sevenzip_library";

/// The library's name, as the pane's message calls it.
pub const LIBRARY_NAME: &str = "7-Zip library";

/// The settings the plugin declares: the library path, a text field.
#[must_use]
pub fn settings_schema() -> SettingsSchema {
    SettingsSchema {
        ext: ID,
        fields: vec![SettingField {
            key: LIBRARY_SETTING,
            label: Text("vfs-archive-sevenzip-library"),
            kind: SettingKind::Text {
                default: String::new(),
            },
        }],
    }
}

/// The library was not found: what the pane shows in place of a disk
/// image's listing. `Display` is the whole message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "The 7-Zip library ({file}) is not installed or not where Rick looks. It reads disk images (VHD, VHDX, VMDK, VDI, QCOW2, DMG, raw) and the file systems inside them; to install it, {hint}; or set `{setting}` in {settings_file} to its path. Looked in: {looked}{errors}"
)]
pub struct MissingLibrary {
    /// The library's file name on this platform.
    pub file: &'static str,
    /// How it is installed here.
    pub hint: &'static str,
    /// The setting that names it.
    pub setting: &'static str,
    /// The settings file the setting lives in.
    pub settings_file: String,
    /// Where Rick looked, comma-separated.
    pub looked: String,
    /// What went wrong with a file that was there but did not load.
    pub errors: String,
}

impl MissingLibrary {
    /// The error as the kernel carries it, so the pane can read the
    /// message and downcast to this type.
    #[must_use]
    pub fn into_error(self) -> Error {
        Error::Extension {
            ext: ID,
            source: Box::new(self),
        }
    }

    /// The same facts as the content the preview pane lays out: the
    /// library's name, the install command for this platform, the setting
    /// *Browse…* fills and where Rick looked.
    #[must_use]
    pub fn for_pane(&self) -> ext::MissingLibrary {
        ext::MissingLibrary {
            name: LIBRARY_NAME.to_owned(),
            install: self.hint.to_owned(),
            setting: (ID, LIBRARY_SETTING),
            filters: vec![DialogFilter {
                name: LIBRARY_NAME,
                extensions: &["so", "dylib", "dll"],
            }],
            detail: format!(
                "It reads disk images (VHD, VHDX, VMDK, VDI, QCOW2, DMG, raw) and the file \
                 systems inside them. Looked for {}: {}{}",
                self.file, self.looked, self.errors
            ),
        }
    }

    /// The missing library in an error, when `error` carries one.
    #[must_use]
    pub fn in_error(error: &Error) -> Option<&Self> {
        match error {
            Error::Extension { source, .. } => source.downcast_ref::<Self>(),
            _ => None,
        }
    }
}

/// Resolves and caches the library for one plugin instance.
pub struct Resolver {
    settings: Option<Arc<dyn SettingsStore>>,
    /// The config directory, whose `plugins/` is one of the places the
    /// bundle puts the library.
    config_dir: Option<PathBuf>,
    /// The places searched after the setting, when a test narrows them;
    /// `None` for the bundle and the platform's usual locations.
    locations: Option<Vec<PathBuf>>,
    cache: Mutex<Option<Arc<SevenZip>>>,
}

impl std::fmt::Debug for Resolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolver")
            .field("settings", &self.settings.is_some())
            .field("config_dir", &self.config_dir)
            .field("locations", &self.locations)
            .field("loaded", &lock(&self.cache).is_some())
            .finish()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Plain data; a panic elsewhere leaves it usable.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The resolver a caller without a registry uses (the compress plugin's
/// extraction): no setting, the bundle and the usual locations only.
static SHARED: LazyLock<Arc<Resolver>> = LazyLock::new(|| Arc::new(Resolver::new(None, None)));

impl Resolver {
    /// A resolver reading and writing the setting through `settings`, the
    /// store the registry lends, and looking in `config_dir`'s plugins
    /// directory among the bundle's places; without a store, the setting
    /// is skipped and nothing is written back.
    #[must_use]
    pub fn new(settings: Option<Arc<dyn SettingsStore>>, config_dir: Option<PathBuf>) -> Self {
        Self {
            settings,
            config_dir,
            locations: None,
            cache: Mutex::new(None),
        }
    }

    /// A resolver that, after the setting, looks in `locations` only
    /// instead of the bundle and the platform's usual places: how a test
    /// stands in for a machine without the library.
    #[must_use]
    pub fn with_locations(
        settings: Option<Arc<dyn SettingsStore>>,
        locations: Vec<PathBuf>,
    ) -> Self {
        Self {
            settings,
            config_dir: None,
            locations: Some(locations),
            cache: Mutex::new(None),
        }
    }

    /// The process-wide resolver without a store.
    #[must_use]
    pub fn shared() -> Arc<Self> {
        Arc::clone(&SHARED)
    }

    /// The settings file, for the message: in the config directory when
    /// it is known.
    fn settings_file(&self) -> String {
        let name = format!("{}.toml", ID.0);
        self.config_dir
            .as_ref()
            .map_or(name.clone(), |dir| dir.join(name).display().to_string())
    }

    /// The path the setting holds, when set and not empty.
    #[must_use]
    pub fn setting(&self) -> Option<PathBuf> {
        match self.settings.as_ref()?.get(ID, LIBRARY_SETTING)? {
            SettingValue::Text(text) if !text.trim().is_empty() => Some(PathBuf::from(text.trim())),
            _ => None,
        }
    }

    /// Writes `path` into the setting, when there is a store and the
    /// setting does not name it already.
    fn write_setting(&self, path: &Path) -> Result<(), Error> {
        let Some(settings) = &self.settings else {
            return Ok(());
        };
        if self.setting().as_deref() == Some(path) {
            return Ok(());
        }
        settings.set(
            ID,
            LIBRARY_SETTING,
            SettingValue::Text(path.display().to_string()),
        )
    }

    /// The directories of the bundle the library travels in: beside the
    /// binary, and the plugins directories beside the binary and in the
    /// config directory, each with the plugin's own folder.
    fn bundle_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            dirs.push(dir.to_path_buf());
            dirs.push(dir.join("plugins").join(ID.0));
            dirs.push(dir.join("plugins"));
        }
        if let Some(config) = &self.config_dir {
            let plugins = config.join("plugins");
            dirs.push(plugins.join(ID.0));
            dirs.push(plugins);
        }
        dirs
    }

    /// Every path tried, in order: the setting, the bundle, the platform.
    fn candidates(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(setting) = self.setting() {
            out.push(setting);
        }
        if let Some(locations) = &self.locations {
            out.extend(locations.iter().cloned());
            return out;
        }
        for dir in self.bundle_dirs() {
            out.push(dir.join(sevenzip::LIBRARY_FILE));
        }
        out.extend(sevenzip::usual_locations());
        out
    }

    /// The library, loaded on the first call and kept; the path it was
    /// found at is written into the setting.
    ///
    /// # Errors
    ///
    /// [`MissingLibrary`] (as `Error::Extension`) when no candidate loads;
    /// the I/O error when the setting cannot be written.
    pub fn library(&self) -> Result<Arc<SevenZip>, Error> {
        if let Some(library) = lock(&self.cache).as_ref() {
            return Ok(Arc::clone(library));
        }
        let mut errors = Vec::new();
        let candidates = self.candidates();
        for candidate in &candidates {
            if !candidate.is_file() {
                continue;
            }
            match SevenZip::load(candidate) {
                Ok(library) => {
                    tracing::info!(path = %candidate.display(), "7-Zip library loaded");
                    self.write_setting(candidate)?;
                    *lock(&self.cache) = Some(Arc::clone(&library));
                    return Ok(library);
                }
                Err(error) => {
                    tracing::debug!(path = %candidate.display(), %error, "7-Zip library not loaded");
                    errors.push(error.to_string());
                }
            }
        }
        Err(self.missing(&candidates, &errors).into_error())
    }

    /// The error for a search that found nothing.
    fn missing(&self, looked: &[PathBuf], errors: &[String]) -> MissingLibrary {
        let mut seen = std::collections::BTreeSet::new();
        let looked: Vec<String> = looked
            .iter()
            .map(|p| p.display().to_string())
            .filter(|p| seen.insert(p.clone()))
            .collect();
        MissingLibrary {
            file: sevenzip::LIBRARY_FILE,
            hint: sevenzip::install_hint(),
            setting: LIBRARY_SETTING,
            settings_file: self.settings_file(),
            looked: looked.join(", "),
            errors: if errors.is_empty() {
                String::new()
            } else {
                format!(". Found but not loaded: {}", errors.join("; "))
            },
        }
    }

    /// Takes the library at `path` (what *Browse…* chose, when the caller
    /// has no pane to store the choice through): loads it, stores the path
    /// in the setting and keeps it.
    ///
    /// # Errors
    ///
    /// The load error, as `Error::Extension`, when the file is not the
    /// library; the I/O error when the setting cannot be written.
    pub fn set_library(&self, path: &Path) -> Result<Arc<SevenZip>, Error> {
        let library = SevenZip::load(path).map_err(|error: SevenZipError| Error::Extension {
            ext: ID,
            source: Box::new(error),
        })?;
        self.write_setting(path)?;
        *lock(&self.cache) = Some(Arc::clone(&library));
        Ok(library)
    }

    /// Forgets the loaded library, so the next call resolves again (what
    /// *Retry* does after the user installed it).
    pub fn forget(&self) {
        *lock(&self.cache) = None;
    }

    /// Whether a library is loaded.
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        lock(&self.cache).is_some()
    }
}
