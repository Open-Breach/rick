//! The plugin manifest file, `plugin.toml`, beside every plugin library.
//!
//! `cargo xtask bundle` writes the file from the extension's [`Manifest`],
//! the crate's version and the toolchain of the build; nobody maintains it
//! by hand (QUESTIONS.md 83). The host reads it before touching the
//! library, so a plugin whose `api_version`, `toolchain` or `platforms` do
//! not fit is refused without being loaded.
//!
//! ```toml
//! id = "git"
//! name = "Git"
//! version = "0.2.0-dev"
//! api_version = 8
//! points = ["decorator", "action"]
//! platforms = ["linux", "macos", "windows"]
//! toolchain = "rustc 1.97.0 (2d8144b78 2026-07-07)"
//!
//! [library]
//! linux = "libext_git.so"
//! macos = "libext_git.dylib"
//! windows = "ext_git.dll"
//! ```

use std::path::{Path, PathBuf};

use ext::{Manifest, Platforms, Points};
use serde::{Deserialize, Serialize};

/// The file's name inside a plugin's directory.
pub const MANIFEST_FILE: &str = "plugin.toml";

/// Why a manifest file could not be read, written or understood.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// The directory has no `plugin.toml`.
    #[error("no {MANIFEST_FILE}")]
    Missing,
    /// The file could not be read.
    #[error("{MANIFEST_FILE}: {0}")]
    Read(#[source] std::io::Error),
    /// The file is not the TOML this host understands.
    #[error("{MANIFEST_FILE}: {0}")]
    Parse(String),
    /// The file could not be written.
    #[error("{path}: {source}")]
    Write {
        /// The file.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// `points` names something that is not an extension point.
    #[error("{MANIFEST_FILE}: unknown extension point `{0}`")]
    UnknownPoint(String),
    /// `platforms` names something that is not a platform.
    #[error("{MANIFEST_FILE}: unknown platform `{0}`")]
    UnknownPlatform(String),
}

/// The library's file name on each platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    /// On Linux: `libext_git.so`.
    pub linux: String,
    /// On macOS: `libext_git.dylib`.
    pub macos: String,
    /// On Windows: `ext_git.dll`.
    pub windows: String,
}

impl Library {
    /// The names cargo gives the `cdylib` of the crate `name`
    /// (`ext-git` or `ext_git`) on the three platforms.
    #[must_use]
    pub fn for_crate(name: &str) -> Self {
        let stem = name.replace('-', "_");
        Self {
            linux: format!("lib{stem}.so"),
            macos: format!("lib{stem}.dylib"),
            windows: format!("{stem}.dll"),
        }
    }

    /// The name for the operating system `os`, as `std::env::consts::OS`
    /// spells it; `None` for any other.
    #[must_use]
    pub fn for_os(&self, os: &str) -> Option<&str> {
        match os {
            "linux" => Some(&self.linux),
            "macos" => Some(&self.macos),
            "windows" => Some(&self.windows),
            _ => None,
        }
    }
}

/// What `plugin.toml` holds: the extension's manifest as the host can read
/// it without loading the library, plus the facts of the build the library
/// came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginToml {
    /// The extension id, also the plugin's directory name.
    pub id: String,
    /// The human-readable name.
    pub name: String,
    /// The crate's version, from its `Cargo.toml`.
    pub version: String,
    /// The extension API version the library was built against.
    pub api_version: u32,
    /// The extension points it fills, by name.
    pub points: Vec<String>,
    /// The platforms it runs on, by name.
    pub platforms: Vec<String>,
    /// The exact `rustc -V` line of the compiler that built the library.
    pub toolchain: String,
    /// The library's file name per platform.
    pub library: Library,
}

impl PluginToml {
    /// The file for a library whose entry point registered `manifest`,
    /// built from the crate `crate_name` with `toolchain`.
    #[must_use]
    pub fn describe(manifest: &Manifest, crate_name: &str, toolchain: &str) -> Self {
        Self {
            id: manifest.id.0.to_owned(),
            name: manifest.name.to_owned(),
            version: manifest.version.to_string(),
            api_version: manifest.api_version,
            points: manifest
                .points
                .names()
                .iter()
                .map(|n| (*n).to_owned())
                .collect(),
            platforms: manifest
                .platforms
                .names()
                .iter()
                .map(|n| (*n).to_owned())
                .collect(),
            toolchain: toolchain.to_owned(),
            library: Library::for_crate(crate_name),
        }
    }

    /// Reads `path`.
    ///
    /// # Errors
    ///
    /// [`ManifestError::Missing`] when there is no file, otherwise the read
    /// or parse error.
    pub fn read(path: &Path) -> Result<Self, ManifestError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(ManifestError::Missing);
            }
            Err(error) => return Err(ManifestError::Read(error)),
        };
        toml::from_str(&text).map_err(|error| ManifestError::Parse(error.message().to_owned()))
    }

    /// Writes `path`, creating its directory.
    ///
    /// # Errors
    ///
    /// The I/O error.
    pub fn write(&self, path: &Path) -> Result<(), ManifestError> {
        let write = |source| ManifestError::Write {
            path: path.to_owned(),
            source,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(write)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|error| write(std::io::Error::other(error.to_string())))?;
        std::fs::write(path, text).map_err(write)
    }

    /// The points, parsed.
    ///
    /// # Errors
    ///
    /// [`ManifestError::UnknownPoint`] for a name that is not a point.
    pub fn points(&self) -> Result<Points, ManifestError> {
        self.points.iter().try_fold(Points::empty(), |acc, name| {
            Points::named(name)
                .map(|point| acc | point)
                .ok_or_else(|| ManifestError::UnknownPoint(name.clone()))
        })
    }

    /// The platforms, parsed.
    ///
    /// # Errors
    ///
    /// [`ManifestError::UnknownPlatform`] for a name that is not a platform.
    pub fn platforms(&self) -> Result<Platforms, ManifestError> {
        self.platforms
            .iter()
            .try_fold(Platforms::empty(), |acc, name| {
                Platforms::named(name)
                    .map(|platform| acc | platform)
                    .ok_or_else(|| ManifestError::UnknownPlatform(name.clone()))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ext::{API_VERSION, Capabilities, Version};
    use types::ExtId;

    #[test]
    fn describe_round_trips_through_the_file() {
        let manifest = Manifest {
            id: ExtId("git"),
            name: "Git",
            version: Version::parse("0.2.0-dev").unwrap_or_else(|e| panic!("{e}")),
            api_version: API_VERSION,
            requires: Capabilities::empty(),
            points: Points::DECORATOR | Points::ACTION,
            platforms: Platforms::ALL,
        };
        let toml = PluginToml::describe(&manifest, "ext-git", "rustc 1.97.0 (abc 2026-07-07)");
        assert_eq!(toml.library.linux, "libext_git.so");
        assert_eq!(toml.library.macos, "libext_git.dylib");
        assert_eq!(toml.library.windows, "ext_git.dll");
        assert_eq!(toml.points, ["decorator", "action"]);
        assert_eq!(toml.points().unwrap_or_default(), manifest.points);
        assert_eq!(toml.platforms().unwrap_or_default(), Platforms::ALL);

        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("git").join(MANIFEST_FILE);
        toml.write(&path).unwrap_or_else(|e| panic!("{e}"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(text.starts_with("id = \"git\"\n"), "{text}");
        assert!(text.contains("\n[library]\n"), "{text}");
        assert_eq!(
            PluginToml::read(&path).unwrap_or_else(|e| panic!("{e}")),
            toml
        );
    }

    #[test]
    fn a_missing_or_broken_file_is_reported() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join(MANIFEST_FILE);
        assert!(matches!(
            PluginToml::read(&path),
            Err(ManifestError::Missing)
        ));
        std::fs::write(&path, "id = \n").unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            PluginToml::read(&path),
            Err(ManifestError::Parse(_))
        ));
        std::fs::write(&path, "id = \"x\"\nname = \"x\"\nversion = \"1\"\napi_version = 1\npoints = [\"wizard\"]\nplatforms = [\"linux\"]\ntoolchain = \"t\"\n[library]\nlinux = \"a\"\nmacos = \"b\"\nwindows = \"c\"\n")
            .unwrap_or_else(|e| panic!("{e}"));
        let parsed = PluginToml::read(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(parsed.points(), Err(ManifestError::UnknownPoint(p)) if p == "wizard"));
        assert_eq!(parsed.library.for_os("macos"), Some("b"));
        assert_eq!(parsed.library.for_os("plan9"), None);
    }
}
