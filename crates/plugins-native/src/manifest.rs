//! The plugin manifest file, `plugin.toml`, beside every plugin library.
//!
//! `cargo xtask bundle` writes the file from the extension's [`Manifest`],
//! the crate's version, the version of Rick and the toolchain of the build;
//! nobody maintains it by hand (QUESTIONS.md 83). The host reads it before
//! touching the library, so a plugin whose `rick`, `api_version`,
//! `toolchain` or `platforms` do not fit is refused without being loaded.
//!
//! ```toml
//! id = "git"
//! name = "Git"
//! description = "Shows Git status badges and adds the Git actions."
//! version = "0.5.0"
//! api_version = 24
//! points = ["decorator", "action"]
//! platforms = ["linux", "macos", "windows"]
//! previews = []
//! markers = [".git"]
//! rick = "0.5.0"
//! toolchain = "rustc 1.97.0 (2d8144b78 2026-07-07)"
//!
//! [library]
//! linux = "libext_git.so"
//! macos = "libext_git.dylib"
//! windows = "ext_git.dll"
//! ```
//!
//! A plugin with subcommands of the `rick` command line
//! (specs/system-integration.md §4) carries one `[[commands]]` table per
//! command, name and help line, so `rick --help` can list them without
//! loading the plugin:
//!
//! ```toml
//! [[commands]]
//! name = "install"
//! help = "Install a tar or zip archive as an application"
//! ```
//!
//! `previews`, `markers` and `connections` are the plugin's triggers: what
//! Rick can know about a plugin without having it
//! (specs/plugin-distribution.md §5). A plugin that adds a kind of
//! connection to *Add…* in the side panel carries one `[[connections]]`
//! table per kind, the section and the title:
//!
//! ```toml
//! [[connections]]
//! section = "cloud"
//! title = "S3"
//! ```

use std::path::{Path, PathBuf};

use ext::{CommandSpec, ConnectionSpec, Manifest, Platforms, Points};
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

/// A subcommand of the `rick` command line the plugin provides: one
/// `[[commands]]` table, from the manifest's [`CommandSpec`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandToml {
    /// The subcommand, as typed after `rick`.
    pub name: String,
    /// The one line `rick --help` shows beside it.
    pub help: String,
}

impl From<&CommandSpec> for CommandToml {
    fn from(spec: &CommandSpec) -> Self {
        Self {
            name: spec.name.to_owned(),
            help: spec.help.to_owned(),
        }
    }
}

/// A kind of connection the plugin adds to *Add…* in the side panel: one
/// `[[connections]]` table, from the manifest's [`ConnectionSpec`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionToml {
    /// The side panel's section, by name: `network` or `cloud`.
    pub section: String,
    /// The title of the kind of connection, as its *Add…* entry reads.
    pub title: String,
}

impl From<&ConnectionSpec> for ConnectionToml {
    fn from(spec: &ConnectionSpec) -> Self {
        Self {
            section: spec.section.name().to_owned(),
            title: spec.title.to_owned(),
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
    /// What the plugin does, in a sentence or two: the text under its
    /// name in the Extensions dialogue. Absent in a manifest written
    /// before API 18.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The crate's version, from its `Cargo.toml`.
    pub version: String,
    /// The extension API version the library was built against.
    pub api_version: u32,
    /// The extension points it fills, by name.
    pub points: Vec<String>,
    /// The platforms it runs on, by name.
    pub platforms: Vec<String>,
    /// The file name extensions its previewers claim, lower case without
    /// the dot; what the host answers *Plugin available* from without
    /// loading the plugin. Absent in a manifest written before API 12.
    #[serde(default)]
    pub previews: Vec<String>,
    /// The names of an entry directly inside a folder that make the folder
    /// the plugin's business (`.git`), compared as whole names. Absent in
    /// a manifest written before API 23.
    #[serde(default)]
    pub markers: Vec<String>,
    /// The version of Rick the plugin was built with: a plugin loads in
    /// that release and in no other (specs/plugin-distribution.md §3).
    /// Absent in a manifest written before the field existed, which the
    /// host refuses as built for an unknown Rick.
    #[serde(default)]
    pub rick: String,
    /// The exact `rustc -V` line of the compiler that built the library.
    pub toolchain: String,
    /// The library's file name per platform.
    pub library: Library,
    /// The kinds of connection the plugin adds to *Add…* in the side
    /// panel, as `[[connections]]` tables after the library. Absent in a
    /// manifest written before API 23.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionToml>,
    /// The subcommands of the `rick` command line the plugin provides,
    /// as `[[commands]]` tables after the library; what `rick --help`
    /// lists without loading the plugin. Absent in a manifest written
    /// before API 15.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandToml>,
}

impl PluginToml {
    /// The file for a library whose entry point registered `manifest`,
    /// built from the crate `crate_name` for the Rick release `rick` with
    /// `toolchain`.
    #[must_use]
    pub fn describe(manifest: &Manifest, crate_name: &str, rick: &str, toolchain: &str) -> Self {
        Self {
            id: manifest.id.0.to_owned(),
            name: manifest.name.to_owned(),
            description: manifest.description.to_owned(),
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
            previews: manifest.previews.iter().map(|n| (*n).to_owned()).collect(),
            markers: manifest.markers.iter().map(|n| (*n).to_owned()).collect(),
            rick: rick.to_owned(),
            toolchain: toolchain.to_owned(),
            library: Library::for_crate(crate_name),
            connections: manifest
                .connections
                .iter()
                .map(ConnectionToml::from)
                .collect(),
            commands: manifest.commands.iter().map(CommandToml::from).collect(),
        }
    }

    /// Reads `path`.
    ///
    /// # Errors
    ///
    /// [`ManifestError::Missing`] when there is no file, otherwise the read
    /// or parse error.
    pub fn read(path: &Path) -> Result<Self, ManifestError> {
        Self::parse(&text(path)?)
    }

    /// Understands `text`, the content of a manifest file.
    ///
    /// # Errors
    ///
    /// [`ManifestError::Parse`] for anything that is not the file this
    /// host writes.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        toml::from_str(text).map_err(|error| ManifestError::Parse(error.message().to_owned()))
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

/// The text of the manifest file at `path`.
///
/// # Errors
///
/// [`ManifestError::Missing`] when there is no file, otherwise the read
/// error.
pub(crate) fn text(path: &Path) -> Result<String, ManifestError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(ManifestError::Missing),
        Err(error) => Err(ManifestError::Read(error)),
    }
}

/// What the host reads of a manifest it does not understand as a whole:
/// the Rick release that wrote it, and what the plugins table shows of the
/// plugin. Every key may be missing and any other key is passed over,
/// since the file of another release may hold keys this host does not
/// know.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Head {
    /// The version of Rick the plugin was built with; empty when the file
    /// does not say.
    #[serde(default)]
    pub rick: String,
    /// The extension id.
    pub id: Option<String>,
    /// The human-readable name.
    #[serde(default)]
    pub name: String,
    /// What the plugin does.
    #[serde(default)]
    pub description: String,
    /// The plugin's version.
    #[serde(default)]
    pub version: String,
}

impl Head {
    /// The head of `text`; `None` when it is not TOML at all, or one of
    /// the keys read here is not text.
    pub fn parse(text: &str) -> Option<Self> {
        toml::from_str(text).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ext::{API_VERSION, Capabilities, ConnectionSection, Version};
    use types::ExtId;

    #[test]
    fn describe_round_trips_through_the_file() {
        let manifest = Manifest {
            id: ExtId("git"),
            name: "Git",
            description: "",
            version: Version::parse("0.2.0-dev").unwrap_or_else(|e| panic!("{e}")),
            api_version: API_VERSION,
            requires: Capabilities::empty(),
            points: Points::DECORATOR | Points::ACTION,
            platforms: Platforms::ALL,
            previews: &["pdf", "tar.gz"],
            markers: &[".git"],
            connections: &[ConnectionSpec {
                section: ConnectionSection::Cloud,
                title: "Amazon S3",
            }],
            commands: &[CommandSpec {
                name: "install",
                help: "Install an archive",
            }],
        };
        let toml = PluginToml::describe(
            &manifest,
            "ext-git",
            "0.5.0",
            "rustc 1.97.0 (abc 2026-07-07)",
        );
        assert_eq!(toml.previews, ["pdf", "tar.gz"]);
        assert_eq!(toml.rick, "0.5.0");
        assert_eq!(toml.markers, [".git"]);
        assert_eq!(
            toml.connections,
            [ConnectionToml {
                section: "cloud".to_owned(),
                title: "Amazon S3".to_owned()
            }]
        );
        assert_eq!(
            toml.commands,
            [CommandToml {
                name: "install".to_owned(),
                help: "Install an archive".to_owned()
            }]
        );
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
        assert!(text.contains("\nrick = \"0.5.0\"\n"), "{text}");
        assert!(text.contains("\nmarkers = [\".git\"]\n"), "{text}");
        assert!(text.contains("\n[library]\n"), "{text}");
        assert!(
            text.contains("\n[[connections]]\nsection = \"cloud\"\ntitle = \"Amazon S3\"\n"),
            "{text}"
        );
        assert!(
            text.contains("\n[[commands]]\nname = \"install\"\nhelp = \"Install an archive\"\n"),
            "{text}"
        );
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
        assert!(
            parsed.previews.is_empty(),
            "an older manifest names no previews"
        );
        assert!(
            parsed.commands.is_empty(),
            "an older manifest names no commands"
        );
        assert!(parsed.rick.is_empty(), "an older manifest names no Rick");
        assert!(
            parsed.markers.is_empty() && parsed.connections.is_empty(),
            "an older manifest names no markers and no connections"
        );
        assert!(matches!(parsed.points(), Err(ManifestError::UnknownPoint(p)) if p == "wizard"));
        assert_eq!(parsed.library.for_os("macos"), Some("b"));
        assert_eq!(parsed.library.for_os("plan9"), None);
    }

    #[test]
    fn the_head_of_another_release_s_file_is_read_past_what_this_host_does_not_know() {
        let text = "id = \"git\"\nname = \"Git\"\nversion = \"9.0.0\"\nrick = \"9.0.0\"\n\
                    holograms = true\n[library]\nplan9 = \"git.a\"\n";
        assert!(matches!(
            PluginToml::parse(text),
            Err(ManifestError::Parse(_))
        ));
        let head = Head::parse(text).unwrap_or_else(|| panic!("no head"));
        assert_eq!(head.rick, "9.0.0");
        assert_eq!(head.id.as_deref(), Some("git"));
        assert_eq!(
            (head.name.as_str(), head.version.as_str()),
            ("Git", "9.0.0")
        );
        assert_eq!(head.description, "");
        // A file without the key names no release; one that is not TOML
        // has no head.
        assert!(Head::parse("id = \"x\"\n").is_some_and(|h| h.rick.is_empty()));
        assert!(Head::parse("id = \n").is_none());
    }
}
