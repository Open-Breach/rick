//! The index: what the registry says exists, and which of it fits a Rick.
//!
//! The shape is `docs/spec.md` 7.2 in Open-Breach/trebuchet. A field this
//! reader does not know is ignored, and so is every application of the
//! index but Rick.

use serde::Deserialize;

use crate::Error;

/// The `schema` this client reads. The publisher raises it only for a
/// change an old reader cannot ignore, and an index above it is not used.
pub const SCHEMA: u32 = 1;

/// The registry's index, as far as Rick reads it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct Index {
    /// The schema the document was written to; at most [`SCHEMA`].
    pub schema: u32,
    /// When the publisher wrote the document, as it says it
    /// (`2026-11-19T02:00:00Z`).
    pub generated: String,
    apps: Apps,
}

/// The applications of the index: Rick's entry, the others unread.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
struct Apps {
    rick: App,
}

/// Rick's entry of the index.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
struct App {
    plugins: Vec<Plugin>,
}

/// One plugin of the registry. Name, description and `meta` are those of
/// its newest build.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct Plugin {
    /// The extension id, `git`.
    pub id: String,
    /// The plugin's name, *Git*.
    pub name: String,
    /// What it does, in one line.
    pub description: String,
    /// Who publishes it.
    pub publisher: String,
    /// What Rick keeps about its own plugins: the extension points and
    /// the triggers.
    pub meta: Meta,
    /// Every file the registry has of the plugin.
    pub builds: Vec<Build>,
}

/// A plugin's `meta`, which the publisher copies without reading it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct Meta {
    /// The extension points the plugin fills, as `plugin.toml` names
    /// them: `provider`, `previewer`, `action`.
    pub points: Vec<String>,
    /// What the plugin is for (specs/plugin-distribution.md §5).
    pub triggers: Triggers,
}

/// What Rick can know about a plugin without having it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct Triggers {
    /// The file name extensions its previewers claim, lower case without
    /// the dot.
    pub previews: Vec<String>,
    /// The names of an entry that make a folder the plugin's business.
    pub markers: Vec<String>,
    /// The kinds of connection it adds to *Add…* in the side panel.
    pub connections: Vec<Connection>,
}

/// One kind of connection a plugin adds.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct Connection {
    /// The side panel's section: `network` or `cloud`.
    pub section: String,
    /// The title of the kind of connection, *Amazon S3*.
    pub title: String,
}

/// One file of a plugin: a package for one platform, built with one
/// release of Rick.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct Build {
    /// The plugin's own version.
    pub version: String,
    /// The release of Rick the plugin was built with.
    pub app_version: String,
    /// Rick's plugin interface version at that release. Carried and
    /// shown; it decides nothing while [`Plugin::fitting`] is stricter.
    pub api_version: u32,
    /// The platform, named as the release archives are: `linux-x64`.
    pub platform: String,
    /// Where the package is downloaded from.
    pub url: String,
    /// The package's size in bytes.
    pub size: u64,
    /// The SHA-256 of the package, in hexadecimal.
    pub sha256: String,
}

/// A plugin of the registry with its build that fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offered<'a> {
    /// The plugin.
    pub plugin: &'a Plugin,
    /// Its build for the platform and the Rick that were asked for.
    pub build: &'a Build,
}

/// What the registry has for one Rick on one platform
/// (specs/plugin-distribution.md §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer<'a> {
    /// The Rick's version has a pre-release part (`0.6.0-dev`): nothing
    /// fits it.
    PreRelease,
    /// The Rick is older than the registry's oldest release: its plugins
    /// are no longer published.
    TooOld,
    /// The plugins with a build that fits, in the index's order. Empty
    /// when the registry has none for this Rick, which is not said to be
    /// anybody's fault.
    Plugins(Vec<Offered<'a>>),
}

/// Whether `version` has a pre-release part, as `0.6.0-dev` has.
#[must_use]
pub fn is_pre_release(version: &str) -> bool {
    semver::Version::parse(version).map_or_else(
        // Not a version as Cargo writes one: its release part ends at
        // the first hyphen all the same.
        |_| version.contains('-'),
        |version| !version.pre.is_empty(),
    )
}

/// The release's name for the platform a build of `os` and `arch` (as
/// `std::env::consts` spell them) is for: `linux-x64`, `linux-arm64`,
/// `macos-arm64`, `windows-x64`. These are the names of the release
/// archives, and of a build's `platform`.
#[must_use]
pub fn platform_name(os: &str, arch: &str) -> String {
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{os}-{arch}")
}

/// The name of the platform this Rick was built for.
#[must_use]
pub fn platform() -> String {
    platform_name(std::env::consts::OS, std::env::consts::ARCH)
}

/// `version` as a version to compare, when it is one.
fn parsed(version: &str) -> Option<semver::Version> {
    semver::Version::parse(version).ok()
}

impl Plugin {
    /// The build that fits a Rick of version `rick` on `platform`: the
    /// one whose `platform` and `app_version` are exactly these, compared
    /// as whole strings. Should a release publish a plugin twice, the
    /// build with the higher plugin version is the one.
    #[must_use]
    pub fn fitting(&self, platform: &str, rick: &str) -> Option<&Build> {
        self.builds
            .iter()
            .filter(|build| build.platform == platform && build.app_version == rick)
            .max_by(|a, b| match (parsed(&a.version), parsed(&b.version)) {
                (Some(a), Some(b)) => a.cmp(&b),
                _ => a.version.cmp(&b.version),
            })
    }
}

impl Index {
    /// Reads an index from the bytes of `index.json`. The caller has
    /// verified their signature; nothing here does.
    ///
    /// # Errors
    ///
    /// [`Error::NewerSchema`] for a `schema` above [`SCHEMA`], and
    /// [`Error::Unreadable`] for bytes that are no index.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let unreadable = |error: serde_json::Error| Error::Unreadable(error.to_string());
        let document: serde_json::Value = serde_json::from_slice(bytes).map_err(unreadable)?;
        if !document.is_object() {
            return Err(Error::Unreadable("not a JSON object".to_owned()));
        }
        // The one key every schema has, read before the rest: a newer
        // document may hold what this reader cannot take.
        let schema = match document.get("schema") {
            None => 0,
            Some(schema) => schema
                .as_u64()
                .and_then(|schema| u32::try_from(schema).ok())
                .ok_or_else(|| Error::Unreadable("schema is not a number".to_owned()))?,
        };
        if schema > SCHEMA {
            return Err(Error::NewerSchema(schema));
        }
        serde_json::from_value(document).map_err(unreadable)
    }

    /// Rick's plugins, in the index's order.
    #[must_use]
    pub fn plugins(&self) -> &[Plugin] {
        &self.apps.rick.plugins
    }

    /// The plugin `id`.
    #[must_use]
    pub fn plugin(&self, id: &str) -> Option<&Plugin> {
        self.plugins().iter().find(|plugin| plugin.id == id)
    }

    /// The release of Rick the registry's oldest build was made with.
    fn oldest_release(&self) -> Option<semver::Version> {
        self.plugins()
            .iter()
            .flat_map(|plugin| &plugin.builds)
            .filter_map(|build| parsed(&build.app_version))
            .min()
    }

    /// What the registry has for a Rick of version `rick` on `platform`.
    #[must_use]
    pub fn offer(&self, platform: &str, rick: &str) -> Offer<'_> {
        if is_pre_release(rick) {
            return Offer::PreRelease;
        }
        if let (Some(rick), Some(oldest)) = (parsed(rick), self.oldest_release())
            && rick < oldest
        {
            return Offer::TooOld;
        }
        Offer::Plugins(
            self.plugins()
                .iter()
                .filter_map(|plugin| {
                    let build = plugin.fitting(platform, rick)?;
                    Some(Offered { plugin, build })
                })
                .collect(),
        )
    }
}
