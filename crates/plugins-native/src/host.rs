//! Discovery of the plugins directories, the manifest checks and the loading
//! of every plugin at start.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::platform::Platform;
use ext::{API_VERSION, Extension, Platforms, Points};
use tracing::{debug, info, info_span, warn};

use crate::HOST_TOOLCHAIN;
use crate::load::{Loaded, load_library};
use crate::manifest::{MANIFEST_FILE, ManifestError, PluginToml};

/// The directory, beside the binary and in the config directory, whose
/// subdirectories are plugins.
pub const PLUGINS_DIR: &str = "plugins";

/// Which of the two plugins directories a plugin was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// `<binary dir>/plugins/`.
    BesideBinary,
    /// `<config dir>/plugins/`; wins over the other on an id clash.
    PerUser,
}

impl Source {
    /// A short description for the table.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::BesideBinary => "beside the binary",
            Self::PerUser => "per-user",
        }
    }
}

/// What became of a plugin that was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginStatus {
    /// Loaded; its contributions are registered.
    Loaded,
    /// Listed in `rick.toml` as disabled: on disk, not loaded.
    Disabled,
    /// Not loaded, for the reason given.
    Refused(String),
}

/// One row of the plugins table: a plugin directory that was found, what
/// its manifest says and what became of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    /// The extension id; the directory's name when the manifest could not
    /// be read.
    pub id: String,
    /// The name the manifest gives.
    pub name: String,
    /// The version the manifest gives.
    pub version: String,
    /// The points the manifest lists.
    pub points: Points,
    /// The plugin's directory.
    pub dir: PathBuf,
    /// Which plugins directory it was found in.
    pub source: Source,
    /// Loaded, disabled or refused.
    pub status: PluginStatus,
}

/// What the host is told at start.
#[derive(Clone, Default)]
pub struct Options {
    /// The plugins directories to scan, in order; see [`Options::dirs`].
    pub dirs: Vec<(Source, PathBuf)>,
    /// Ids the user disabled (`[plugins] disabled` in `rick.toml`): listed,
    /// not loaded.
    pub disabled: Vec<String>,
    /// Ids of the extensions compiled into the binary, which a plugin of the
    /// same id cannot replace.
    pub compiled_in: Vec<String>,
    /// The platform services lent to every plugin while it registers
    /// (`ext::Registry::platform`); the platform the binary runs on when
    /// `None`.
    pub platform: Option<Arc<dyn Platform>>,
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("dirs", &self.dirs)
            .field("disabled", &self.disabled)
            .field("compiled_in", &self.compiled_in)
            .field("platform", &self.platform.is_some())
            .finish()
    }
}

impl Options {
    /// The platform the plugins register with: the one given, else the
    /// binary's own.
    #[must_use]
    pub fn platform(&self) -> Arc<dyn Platform> {
        self.platform
            .clone()
            .unwrap_or_else(|| Arc::new(ext::platform::Native::new()))
    }

    /// The two plugins directories: beside the binary `exe` and in the
    /// per-user `config_dir`.
    #[must_use]
    pub fn dirs(exe: &Path, config_dir: &Path) -> Vec<(Source, PathBuf)> {
        let mut dirs = Vec::new();
        if let Some(bin_dir) = exe.parent() {
            dirs.push((Source::BesideBinary, bin_dir.join(PLUGINS_DIR)));
        }
        dirs.push((Source::PerUser, config_dir.join(PLUGINS_DIR)));
        dirs
    }
}

/// The plugins the host found and loaded. Keeps every loaded library
/// mapped; the binary keeps the host for the process lifetime.
#[derive(Debug)]
pub struct Host {
    plugins: Vec<Plugin>,
    loaded: Vec<Arc<Loaded>>,
}

/// A plugin directory before its checks.
struct Candidate {
    source: Source,
    dir: PathBuf,
    toml: Result<PluginToml, ManifestError>,
}

impl Host {
    /// Scans the directories, checks every manifest, loads what passes and
    /// is not disabled, and records the outcome per plugin, each under one
    /// tracing span. Never fails: a plugin that cannot be loaded is a row
    /// with a reason.
    #[must_use]
    pub fn load(options: &Options) -> Self {
        let mut candidates: Vec<Candidate> = options
            .dirs
            .iter()
            .flat_map(|(source, dir)| scan(*source, dir))
            .collect();
        // Per-user plugins claim their ids first, so one beside the binary
        // with the same id is shadowed.
        candidates.sort_by_key(|c| std::cmp::Reverse(c.source));
        let mut taken: BTreeMap<String, PathBuf> = BTreeMap::new();
        let mut plugins = Vec::new();
        let mut loaded = Vec::new();
        for candidate in candidates {
            let (plugin, library) = process(candidate, options, &mut taken);
            plugins.push(plugin);
            loaded.extend(library);
        }
        plugins.sort_by(|a, b| a.source.cmp(&b.source).then_with(|| a.dir.cmp(&b.dir)));
        loaded.sort_by(|a, b| a.manifest().id.0.cmp(b.manifest().id.0));
        Self { plugins, loaded }
    }

    /// The table: every plugin found, in directory order, beside the binary
    /// first.
    #[must_use]
    pub fn plugins(&self) -> &[Plugin] {
        &self.plugins
    }

    /// The loaded plugins, by id.
    #[must_use]
    pub fn loaded(&self) -> &[Arc<Loaded>] {
        &self.loaded
    }

    /// The loaded plugins as extensions the application registers like its
    /// built-ins, by id.
    #[must_use]
    pub fn extensions(&self) -> Vec<Arc<dyn Extension>> {
        self.loaded
            .iter()
            .map(|plugin| Arc::clone(plugin) as Arc<dyn Extension>)
            .collect()
    }
}

/// The subdirectories of `dir`, each with its manifest or the reason it
/// has none, in name order. A missing `dir` is nothing found.
fn scan(source: Source, dir: &Path) -> Vec<Candidate> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            debug!(dir = %dir.display(), %error, "no plugins directory");
            return Vec::new();
        }
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs.into_iter()
        .map(|dir| Candidate {
            source,
            toml: PluginToml::read(&dir.join(MANIFEST_FILE)),
            dir,
        })
        .collect()
}

/// Checks one candidate and loads it when everything fits.
fn process(
    candidate: Candidate,
    options: &Options,
    taken: &mut BTreeMap<String, PathBuf>,
) -> (Plugin, Option<Arc<Loaded>>) {
    let dir_name = candidate
        .dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let toml = match &candidate.toml {
        Ok(toml) => toml,
        Err(error) => {
            warn!(dir = %candidate.dir.display(), %error, "plugin refused");
            return (
                Plugin {
                    id: dir_name,
                    name: String::new(),
                    version: String::new(),
                    points: Points::empty(),
                    dir: candidate.dir,
                    source: candidate.source,
                    status: PluginStatus::Refused(error.to_string()),
                },
                None,
            );
        }
    };
    let span = info_span!("plugin", id = %toml.id, dir = %candidate.dir.display());
    let _enter = span.enter();
    let outcome = check_and_load(toml, &candidate.dir, options, taken);
    let status = match &outcome {
        Ok(Some(_)) => {
            info!("loaded");
            PluginStatus::Loaded
        }
        Ok(None) => {
            info!("disabled");
            PluginStatus::Disabled
        }
        Err(reason) => {
            warn!(%reason, "refused");
            PluginStatus::Refused(reason.clone())
        }
    };
    (
        Plugin {
            id: toml.id.clone(),
            name: toml.name.clone(),
            version: toml.version.clone(),
            points: toml.points().unwrap_or_default(),
            dir: candidate.dir,
            source: candidate.source,
            status,
        },
        outcome.ok().flatten(),
    )
}

/// The checks, in order, then the load; `Ok(None)` for a disabled plugin.
fn check_and_load(
    toml: &PluginToml,
    dir: &Path,
    options: &Options,
    taken: &mut BTreeMap<String, PathBuf>,
) -> Result<Option<Arc<Loaded>>, String> {
    let points = toml.points().map_err(|e| e.to_string())?;
    let platforms = toml.platforms().map_err(|e| e.to_string())?;
    if let Some(winner) = taken.get(&toml.id) {
        return Err(format!(
            "shadowed by the plugin with the same id at {}",
            winner.display()
        ));
    }
    if options.compiled_in.contains(&toml.id) {
        return Err("compiled into this binary".to_owned());
    }
    let current = Platforms::current();
    if !platforms.contains(current) {
        return Err(format!("built for {platforms}, not {current}"));
    }
    if toml.api_version != API_VERSION {
        return Err(format!(
            "extension API version {}; this host speaks {API_VERSION}",
            toml.api_version
        ));
    }
    if toml.toolchain != HOST_TOOLCHAIN {
        return Err(format!(
            "built with `{}`; this host with `{HOST_TOOLCHAIN}`",
            toml.toolchain
        ));
    }
    let os = std::env::consts::OS;
    let file = toml
        .library
        .for_os(os)
        .ok_or_else(|| format!("no library named for {os}"))?;
    let path = dir.join(file);
    if !path.is_file() {
        return Err(format!("library missing: {file}"));
    }
    // The id is claimed from here on, disabled or not: a disabled per-user
    // plugin still stands for its id.
    taken.insert(toml.id.clone(), dir.to_owned());
    if options.disabled.contains(&toml.id) {
        return Ok(None);
    }
    let loaded = load_library(&path, options.platform()).map_err(|e| e.to_string())?;
    let manifest = loaded.manifest();
    if manifest.id.0 != toml.id {
        return Err(format!(
            "the library is plugin `{}`, not `{}`",
            manifest.id.0, toml.id
        ));
    }
    if manifest.version.to_string() != toml.version {
        return Err(format!(
            "{MANIFEST_FILE} says version {}, the library {}",
            toml.version, manifest.version
        ));
    }
    if manifest.points != points {
        return Err(format!(
            "{MANIFEST_FILE} lists points {points}; the library declares {}",
            manifest.points
        ));
    }
    Ok(Some(Arc::new(loaded)))
}
