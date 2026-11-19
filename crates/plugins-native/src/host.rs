//! Discovery of the plugins directories, the manifest checks and the loading
//! of every plugin at start.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::platform::Platform;
use ext::{API_VERSION, Extension, Platforms, Points};
use tracing::{debug, info, info_span, warn};

use crate::load::{Loaded, load_library};
use crate::manifest::{self, Head, MANIFEST_FILE, ManifestError, PluginToml};
use crate::{HOST_RICK, HOST_TOOLCHAIN};

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
    /// What the manifest says the plugin does; empty when it says nothing.
    pub description: String,
    /// The version the manifest gives.
    pub version: String,
    /// The points the manifest lists.
    pub points: Points,
    /// The file name extensions its previewers claim, as the manifest
    /// lists them.
    pub previews: Vec<String>,
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
    /// The question queue lent to every plugin while it registers
    /// (`ext::Registry::prompts`); none when `None`, so a plugin that must
    /// ask refuses instead.
    pub prompts: Option<Arc<ext::Prompts>>,
    /// The settings values lent to every plugin while it registers
    /// (`ext::Registry::settings_store`); none when `None`, so a plugin
    /// works with its defaults and remembers nothing.
    pub settings: Option<Arc<dyn ext::SettingsStore>>,
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("dirs", &self.dirs)
            .field("disabled", &self.disabled)
            .field("compiled_in", &self.compiled_in)
            .field("platform", &self.platform.is_some())
            .field("prompts", &self.prompts.is_some())
            .field("settings", &self.settings.is_some())
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
    /// What its manifest file holds, or why there is none to read.
    text: Result<String, ManifestError>,
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

/// The subdirectories of `dir`, each with its manifest's text or the reason
/// it has none, in name order. A missing `dir` is nothing found.
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
            text: manifest::text(&dir.join(MANIFEST_FILE)),
            dir,
        })
        .collect()
}

/// The reason a plugin of another Rick release is refused with, as the
/// plugins table and the Extensions dialogue show it; `rick` is what its
/// manifest names, empty when it names none.
fn built_for(rick: &str) -> String {
    if rick.is_empty() {
        "Built for an unknown Rick".to_owned()
    } else {
        format!("Built for Rick {rick}")
    }
}

/// The row of a plugin whose manifest this host could not read as a whole:
/// what `head` says of it, the directory's name for its id when the file
/// names none, and the reason.
fn unread(candidate: Candidate, head: Head, reason: String) -> Plugin {
    warn!(dir = %candidate.dir.display(), %reason, "plugin refused");
    let dir_name = candidate
        .dir
        .file_name()
        .map_or_default(|n| n.to_string_lossy().into_owned());
    Plugin {
        id: head.id.unwrap_or(dir_name),
        name: head.name,
        description: head.description,
        version: head.version,
        points: Points::empty(),
        previews: Vec::new(),
        dir: candidate.dir,
        source: candidate.source,
        status: PluginStatus::Refused(reason),
    }
}

/// Checks one candidate and loads it when everything fits.
fn process(
    candidate: Candidate,
    options: &Options,
    taken: &mut BTreeMap<String, PathBuf>,
) -> (Plugin, Option<Arc<Loaded>>) {
    let text = match &candidate.text {
        Ok(text) => text,
        Err(error) => {
            let reason = error.to_string();
            return (unread(candidate, Head::default(), reason), None);
        }
    };
    let toml = match PluginToml::parse(text) {
        Ok(toml) => toml,
        Err(error) => {
            // The file of another release may hold keys this host does not
            // know. It is refused for its release, which the user can act
            // on, not for its spelling.
            let head = Head::parse(text);
            let reason = match &head {
                Some(head) if head.rick != HOST_RICK => built_for(&head.rick),
                _ => error.to_string(),
            };
            return (unread(candidate, head.unwrap_or_default(), reason), None);
        }
    };
    let span = info_span!("plugin", id = %toml.id, dir = %candidate.dir.display());
    let _enter = span.enter();
    let outcome = check_and_load(&toml, &candidate.dir, options, taken);
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
            points: toml.points().unwrap_or_default(),
            id: toml.id,
            name: toml.name,
            description: toml.description,
            version: toml.version,
            previews: toml.previews,
            dir: candidate.dir,
            source: candidate.source,
            status,
        },
        outcome.ok().flatten(),
    )
}

/// Whether the plugin folder `dir` holds a library besides the plugin's
/// own, `library`: one the plugin's library may link, as the media
/// previewer links the GStreamer runtime that travels in its folder.
fn brings_libraries(dir: &Path, library: &str) -> bool {
    let suffix = std::env::consts::DLL_SUFFIX;
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            !name.eq_ignore_ascii_case(library)
                && name.to_ascii_lowercase().ends_with(suffix)
                && entry.path().is_file()
        })
}

/// `path`, the value of a search path variable, with `dir` put first and
/// not named a second time.
fn with_first(dir: &Path, path: Option<&OsStr>) -> Result<OsString, std::env::JoinPathsError> {
    let rest = path
        .map(|path| std::env::split_paths(path).collect::<Vec<_>>())
        .unwrap_or_default();
    std::env::join_paths(
        std::iter::once(dir.to_path_buf()).chain(rest.into_iter().filter(|known| known != dir)),
    )
}

/// Puts the plugin folder `dir` first on the `PATH`, for the rest of the
/// process: Windows only.
///
/// Windows looks for the libraries a library links beside the binary, in
/// the system's folders and on the `PATH`, never in the folder of the
/// library that asks. A plugin that brings libraries of its own in its
/// folder would not load, wherever the folder lies. The `PATH` is the one
/// of those places a process can add to that every later load honours as
/// well, and later loads matter: GStreamer opens its elements by full path
/// when they are first needed, and what an element links lies in the
/// plugin's folder too.
fn lend_to_loader(dir: &Path) {
    match with_first(dir, std::env::var_os("PATH").as_deref()) {
        Ok(path) => {
            // SAFETY: changing the environment is unsound where another
            // thread may read it at the same moment without std's lock.
            // That cannot happen on Windows, where the system guards the
            // environment itself, and this function runs nowhere else
            // (the caller checks the operating system).
            unsafe { std::env::set_var("PATH", path) };
            debug!(dir = %dir.display(), "plugin folder put on the PATH");
        }
        Err(error) => warn!(dir = %dir.display(), %error, "plugin folder not put on the PATH"),
    }
}

/// The checks, in order, then the load; `Ok(None)` for a disabled plugin.
fn check_and_load(
    toml: &PluginToml,
    dir: &Path,
    options: &Options,
    taken: &mut BTreeMap<String, PathBuf>,
) -> Result<Option<Arc<Loaded>>, String> {
    // The release first, and from the manifest alone: a plugin shares the
    // kernel libraries with the Rick it was built with, so the library of
    // another release is never mapped (specs/plugin-distribution.md §3).
    // Its id stays free for a plugin of this release.
    if toml.rick != HOST_RICK {
        return Err(built_for(&toml.rick));
    }
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
    if os == "windows" && brings_libraries(dir, file) {
        lend_to_loader(dir);
    }
    let loaded = load_library(
        &path,
        options.platform(),
        options.prompts.clone(),
        options.settings.clone(),
    )
    .map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A plugin's folder counts as bringing libraries when a library other
    /// than the plugin's own lies in it: the rule by which the host lends
    /// the folder to the Windows loader (issue 250, item 5).
    #[test]
    fn a_folder_brings_libraries_when_one_lies_beside_the_plugin_s_own() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let suffix = std::env::consts::DLL_SUFFIX;
        let own = format!("ext_preview_media{suffix}");
        std::fs::write(dir.path().join(&own), "").unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join(MANIFEST_FILE), "").unwrap_or_else(|e| panic!("{e}"));
        // Elements in a folder below are GStreamer's to find, not the
        // loader's.
        let below = dir.path().join("lib").join("gstreamer-1.0");
        std::fs::create_dir_all(&below).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(below.join(format!("gstcoreelements{suffix}")), "")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!brings_libraries(dir.path(), &own));
        assert!(!brings_libraries(dir.path(), &own.to_uppercase()));

        std::fs::write(dir.path().join(format!("gstreamer-1.0-0{suffix}")), "")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(brings_libraries(dir.path(), &own));
        assert!(!brings_libraries(&dir.path().join("missing"), &own));
    }

    #[test]
    fn the_folder_goes_first_on_the_path_and_is_named_once() {
        let dir = Path::new("plugins").join("preview-media");
        let other = Path::new("somewhere").join("else");
        let split = |path: &OsString| std::env::split_paths(path).collect::<Vec<_>>();

        let alone = with_first(&dir, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(split(&alone), std::slice::from_ref(&dir));

        let before = std::env::join_paths([other.clone(), dir.clone(), other.join("bin")])
            .unwrap_or_else(|e| panic!("{e}"));
        let after = with_first(&dir, Some(&before)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(split(&after), [dir, other.clone(), other.join("bin")]);
    }
}
