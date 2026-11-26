//! The parts of `README.md` that are regenerated with every release
//! (specs/readme.md): the version in the status line and the call to
//! action, the Features list with its live items, the previewed file types
//! by kind, and the Install section.
//!
//! Nothing here is typed from memory. [`extensions`] is every extension a
//! release carries, the ones compiled into the binary and the plugins;
//! [`Facts::gather`] loads each into a registry and keeps what it
//! registered and what its manifest declares: the file types its
//! previewers claim, the providers, the search connectors and the
//! platforms they are for. [`features`], [`file_types`] and [`install`]
//! write the Markdown from that, and [`regenerate`] puts it between the
//! README's markers. The installer's feature item is not written here a
//! second time: its wording is fixed in specs/readme.md, and [`features`]
//! takes the sentence from there ([`fixed_wording`]).
//!
//! One fixed part is held against the spec here too, because it comes
//! into force with a release: the privacy paragraph (specs/readme.md,
//! section 9), which a README has from [`PRIVACY_SINCE`] on and not
//! before ([`check_privacy`]).
//!
//! The markers:
//!
//! ```text
//! <!-- generated:features -->
//! …
//! <!-- /generated:features -->
//! ```
//!
//! The test `tests/readme.rs` compares the README with what
//! [`regenerate`] gives and, with `RICK_README_WRITE` set, writes it
//! instead; `cargo xtask readme` runs it that way.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::{Extension, Platforms, Registry};

/// The block with the status line and the call to action.
pub const STATUS: &str = "status";
/// The block with the six feature items.
pub const FEATURES: &str = "features";
/// The block with the previewed file types, one folded list per kind.
pub const FILE_TYPES: &str = "file-types";
/// The block with the Install section's body.
pub const INSTALL: &str = "install";

/// Why the README could not be read or regenerated.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReadmeError {
    /// The README has no `<!-- generated:… -->` pair of that name.
    #[error("README.md has no generated block `{0}`")]
    NoBlock(&'static str),
    /// The status block has no line that starts with `v<version>`.
    #[error("the status line of README.md names no version")]
    NoVersion,
    /// The spec's Features list quotes no wording for an item.
    #[error("specs/readme.md quotes no wording for item {0} of its Features list")]
    NoWording(u8),
    /// The spec does not quote the privacy paragraph as one paragraph
    /// under its heading.
    #[error("specs/readme.md does not quote one paragraph under `{PRIVACY_HEADING}`")]
    NoPrivacyWording,
    /// The README describes a release that has the privacy paragraph,
    /// and the paragraph is not there, not in its place, or not the
    /// spec's word for word.
    #[error(
        "README.md describes {version} and has no privacy paragraph where specs/readme.md \
         (section 9) puts it: the spec's words, as the first line under the rule above the \
         closing links. The release pull request of {PRIVACY_SINCE} adds it"
    )]
    NoPrivacy {
        /// The version the status line names.
        version: String,
    },
    /// The README describes a release before the privacy paragraph and
    /// has it all the same.
    #[error(
        "README.md describes {version}, which fetches no plugin list, and has the privacy \
         paragraph of {PRIVACY_SINCE} already: it comes with the release pull request"
    )]
    PrivacyEarly {
        /// The version the status line names.
        version: String,
    },
    /// A previewer declares a file type the README has no kind for.
    #[error("{previewer} previews {name}, which has no kind in the README's file types")]
    NoKind {
        /// The type's short name.
        name: String,
        /// The crate of the previewer that declares it.
        previewer: &'static str,
    },
    /// `assets/readme/` does not hold exactly one model.
    #[error("assets/readme holds {0} `.stl` files; the release's animal is the one model there")]
    Model(usize),
    /// An extension of the bundle did not register.
    #[error("{name} did not register: {reason}")]
    Extension {
        /// The crate.
        name: &'static str,
        /// What the registry said.
        reason: String,
    },
}

/// The sample model of the README's picture: the one `.stl` of
/// `assets/readme/` under the workspace's `root`, the animal of the
/// release (specs/readme.md, *Screenshot*). A release replaces the model
/// and the script that writes it, and nothing else names the animal: the
/// picture's fixture and the README's test read it from here.
///
/// # Errors
///
/// [`ReadmeError::Model`] when the folder holds no model or more than one.
pub fn model(root: &Path) -> Result<PathBuf, ReadmeError> {
    let mut models: Vec<PathBuf> = std::fs::read_dir(root.join("assets/readme"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "stl"))
        .collect();
    match models.len() {
        1 => Ok(models.remove(0)),
        count => Err(ReadmeError::Model(count)),
    }
}

/// Every extension a release carries, by the name of its crate: the
/// previewers first, in the order the Features list names their types.
/// `tests/readme.rs` holds this list against the `ext-*` crates of the
/// workspace, so a new plugin cannot be left out of the README.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn extensions() -> Vec<(&'static str, Arc<dyn Extension>)> {
    vec![
        (
            "ext-preview-pdf",
            Arc::new(ext_preview_pdf::PdfPreview::new()),
        ),
        (
            "ext-preview-image",
            Arc::new(ext_preview_image::ImagePreview::new()),
        ),
        ("ext-preview-3d", Arc::new(ext_preview_3d::Preview3d::new())),
        (
            "ext-preview-markup",
            Arc::new(ext_preview_markup::MarkupPreview::new()),
        ),
        (
            "ext-preview-text",
            Arc::new(ext_preview_text::TextPreview::new()),
        ),
        (
            "ext-preview-code",
            Arc::new(ext_preview_code::CodePreview::new()),
        ),
        (
            "ext-preview-media",
            Arc::new(ext_preview_media::MediaPreview::new()),
        ),
        (
            "ext-preview-office",
            Arc::new(ext_preview_office::OfficePreview::new()),
        ),
        (
            "ext-preview-archive",
            Arc::new(ext_preview_archive::ArchivePreview::new()),
        ),
        (
            "ext-preview-icons",
            Arc::new(ext_preview_icons::IconsPreview::new()),
        ),
        (
            "ext-preview-hex",
            Arc::new(ext_preview_hex::HexPreview::new()),
        ),
        (
            "ext-vfs-archive",
            Arc::new(ext_vfs_archive::ArchiveExtension::new()),
        ),
        (
            "ext-vfs-trash",
            Arc::new(ext_vfs_trash::TrashExtension::new()),
        ),
        ("ext-ssh", Arc::new(ext_ssh::SshExtension::new())),
        ("ext-vfs-s3", Arc::new(ext_vfs_s3::S3Extension::new())),
        (
            "ext-vfs-azure",
            Arc::new(ext_vfs_azure::AzureExtension::new()),
        ),
        ("ext-vfs-gcs", Arc::new(ext_vfs_gcs::GcsExtension::new())),
        (
            "ext-search-walk",
            Arc::new(ext_search_walk::WalkExtension::new()),
        ),
        (
            "ext-search-tracker",
            Arc::new(ext_search_tracker::TrackerExtension::new()),
        ),
        (
            "ext-search-baloo",
            Arc::new(ext_search_baloo::BalooExtension::new()),
        ),
        (
            "ext-search-spotlight",
            Arc::new(ext_search_spotlight::SpotlightExtension::new()),
        ),
        (
            "ext-search-windows",
            Arc::new(ext_search_windows::WindowsSearchExtension::new()),
        ),
        ("ext-git", Arc::new(ext_git::GitExtension::new())),
        ("ext-svn", Arc::new(ext_svn::SvnExtension::new())),
        ("ext-zfs", Arc::new(ext_zfs::ZfsExtension::new())),
        (
            "ext-action-compress",
            Arc::new(ext_action_compress::CompressExtension::new()),
        ),
        (
            "ext-action-compare",
            Arc::new(ext_action_compare::CompareExtension::new()),
        ),
        (
            "ext-action-terminal",
            Arc::new(ext_action_terminal::TerminalExtension::new()),
        ),
        (
            "ext-action-rename",
            Arc::new(ext_action_rename::RenameExtension::new()),
        ),
        (
            "ext-installer",
            Arc::new(ext_installer::InstallerExtension::new()),
        ),
    ]
}

/// The kinds the README lists the previewed file types under
/// (specs/readme.md, *File types*), in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Images, icons, audio and video.
    Media,
    /// PDF, Office documents, Markdown, HTML, CSV and plain text.
    Documents,
    /// 3D and 3D-printing models.
    ThreeD,
    /// Source code, and data and config formats.
    Programming,
    /// Archives and disk images.
    Archives,
}

impl Kind {
    /// Every kind, in the order the README lists them.
    pub const ALL: [Self; 5] = [
        Self::Media,
        Self::Documents,
        Self::ThreeD,
        Self::Programming,
        Self::Archives,
    ];

    /// The kind as its list is headed.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Media => "Media",
            Self::Documents => "Documents",
            Self::ThreeD => "3D",
            Self::Programming => "Programming",
            Self::Archives => "Archives and disk images",
        }
    }
}

/// A previewer's crate, the kind of the types it declares, and the types
/// of its that are of another kind, by their short names.
type PreviewerKinds = (&'static str, Kind, &'static [(&'static str, Kind)]);

/// The kind each previewer gives the types it declares. A previewer that
/// is not here, or a new kind, is a decision (specs/readme.md): its types
/// fail [`Facts::gather`] instead of landing in a catch-all.
const KINDS: &[PreviewerKinds] = &[
    ("ext-preview-pdf", Kind::Documents, &[]),
    ("ext-preview-image", Kind::Media, &[]),
    ("ext-preview-3d", Kind::ThreeD, &[]),
    ("ext-preview-markup", Kind::Documents, &[]),
    (
        "ext-preview-text",
        Kind::Documents,
        // The config and data formats the text previewer shows as text.
        &[
            ("INI", Kind::Programming),
            ("CFG", Kind::Programming),
            ("CONF", Kind::Programming),
            ("TOML", Kind::Programming),
            ("Diff", Kind::Programming),
            (".gitignore", Kind::Programming),
            (".env", Kind::Programming),
        ],
    ),
    ("ext-preview-code", Kind::Programming, &[]),
    ("ext-preview-media", Kind::Media, &[]),
    ("ext-preview-office", Kind::Documents, &[]),
    // An eBook is a document (specs/readme.md, *File types*), though its
    // preview is the list of what the book holds.
    (
        "ext-preview-archive",
        Kind::Archives,
        &[("EPUB", Kind::Documents)],
    ),
    ("ext-preview-icons", Kind::Media, &[]),
];

/// The kind of the type `name` that the previewer of the crate
/// `previewer` declares.
fn kind_of(previewer: &str, name: &str) -> Option<Kind> {
    let (_, kind, others) = KINDS.iter().find(|(krate, _, _)| *krate == previewer)?;
    Some(
        others
            .iter()
            .find(|(other, _)| *other == name)
            .map_or(*kind, |(_, kind)| *kind),
    )
}

/// What the extensions of a release register, as the Features list says
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Facts {
    /// The file types the previewers claim, by their short names with
    /// their kinds, each once, in the order of [`extensions`] and of each
    /// manifest.
    pub previews: Vec<(Kind, String)>,
    /// The extensions that register a provider outside the cloud, by
    /// their manifests' names.
    pub storage: Vec<String>,
    /// The same for the cloud providers.
    pub cloud: Vec<String>,
    /// The search connectors a manifest gives to some platforms only, per
    /// platform in the order Linux, macOS, Windows.
    pub search: Vec<(&'static str, Vec<String>)>,
    /// The tools Rick works with: the extensions that register a decorator
    /// and actions and no location of their own.
    pub tools: Vec<String>,
}

impl Facts {
    /// Loads every extension into a registry of its own, on `platform`,
    /// and collects what it registered.
    ///
    /// # Errors
    ///
    /// [`ReadmeError::Extension`] when one does not register, and
    /// [`ReadmeError::NoKind`] when a previewer declares a type the README
    /// has no kind for.
    pub fn gather(
        extensions: &[(&'static str, Arc<dyn Extension>)],
        platform: &Arc<dyn platform::Platform>,
    ) -> Result<Self, ReadmeError> {
        let mut facts = Self {
            search: [Platforms::LINUX, Platforms::MACOS, Platforms::WINDOWS]
                .into_iter()
                .map(|p| (platform_title(p), Vec::new()))
                .collect(),
            ..Self::default()
        };
        for (name, extension) in extensions {
            let mut registry = Registry::with_platform(Arc::clone(platform));
            registry
                .load(&**extension)
                .map_err(|e| ReadmeError::Extension {
                    name,
                    reason: e.to_string(),
                })?;
            let manifest = extension.manifest();
            if !registry.previewers().is_empty() {
                for extension in manifest.previews {
                    let type_name = type_name(extension);
                    if facts.previews.iter().any(|(_, known)| *known == type_name) {
                        continue;
                    }
                    let kind = kind_of(name, &type_name).ok_or_else(|| ReadmeError::NoKind {
                        name: type_name.clone(),
                        previewer: name,
                    })?;
                    facts.previews.push((kind, type_name));
                }
            }
            if let Some(provider) = registry.providers().first() {
                if vfs_rick::is_cloud_provider(provider.id()) {
                    facts.cloud.push(manifest.name.to_owned());
                } else {
                    facts.storage.push(manifest.name.to_owned());
                }
            }
            if !registry.search_indexes().is_empty() && manifest.platforms != Platforms::ALL {
                for (title, names) in &mut facts.search {
                    if manifest
                        .platforms
                        .iter()
                        .any(|p| platform_title(p) == *title)
                    {
                        names.push(manifest.name.to_owned());
                    }
                }
            }
            if !registry.decorators().is_empty()
                && !registry.actions().is_empty()
                && registry.providers().is_empty()
                && registry.views().is_empty()
            {
                facts.tools.push(manifest.name.to_owned());
            }
        }
        Ok(facts)
    }
}

/// A platform as the README writes it.
fn platform_title(platform: Platforms) -> &'static str {
    if platform == Platforms::LINUX {
        "Linux"
    } else if platform == Platforms::MACOS {
        "macOS"
    } else {
        "Windows"
    }
}

/// The short names of the types whose extension is not the name in
/// capitals, and the spellings that are one type: `jpg` and `jpeg` are
/// JPG, `md` and `markdown` are Markdown.
const TYPE_NAMES: &[(&[&str], &str)] = &[
    (&["jpg", "jpeg"], "JPG"),
    (&["webp"], "WebP"),
    (&["tif", "tiff"], "TIFF"),
    (&["svg", "svgz"], "SVG"),
    (&["md", "markdown"], "Markdown"),
    (&["html", "htm"], "HTML"),
    (&["tsv", "tab"], "TSV"),
    (&["txt", "text", "readme", "license"], "TXT"),
    (&["diff", "patch"], "Diff"),
    (&["gitignore"], ".gitignore"),
    (&["env"], ".env"),
    (&["js", "mjs", "cjs", "jsx"], "JavaScript"),
    (&["ts", "tsx"], "TypeScript"),
    (&["java"], "Java"),
    (&["rs"], "Rust"),
    (&["cs"], "C#"),
    (&["fs", "fsx"], "F#"),
    (&["vb"], "Visual Basic"),
    (&["c", "h"], "C"),
    (&["cpp", "cc", "cxx", "hpp", "hh"], "C++"),
    (&["pl", "pm"], "Perl"),
    (&["yaml", "yml"], "YAML"),
    (&["sh", "bash"], "Shell"),
    (&["py"], "Python"),
    (&["rb"], "Ruby"),
    (&["go"], "Go"),
    (&["wav", "wave"], "WAV"),
    (&["mp4", "m4v"], "MP4"),
    (&["7z"], "7z"),
    (&["tar.gz", "tgz"], "TAR.GZ"),
    (&["tar.zst", "tzst"], "TAR.ZST"),
    (&["tar.xz", "txz"], "TAR.XZ"),
    (&["tar.bz2", "tbz", "tbz2"], "TAR.BZ2"),
    (&["tar.lz4", "tlz4"], "TAR.LZ4"),
    (&["img", "ima", "raw"], "IMG"),
    (&["qcow2", "qcow"], "QCOW2"),
];

/// The short name of the file type a previewer claims as `extension`: the
/// name the table of known spellings gives it, or the extension in capitals.
#[must_use]
pub fn type_name(extension: &str) -> String {
    TYPE_NAMES
        .iter()
        .find(|(spellings, _)| spellings.contains(&extension))
        .map_or_else(|| extension.to_uppercase(), |(_, name)| (*name).to_owned())
}

/// `a`, `a and b`, `a, b and c`.
fn and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// The item of the Features list that describes the installer
/// (specs/readme.md, *Features*, item 5).
const INSTALLER_ITEM: u8 = 5;

/// The wording `spec`, the text of specs/readme.md, fixes for item
/// `number` of its Features list: what the item's line quotes between its
/// first pair of backticks. The line is the one that starts with the
/// number, a full stop and the item's name in bold.
///
/// # Errors
///
/// [`ReadmeError::NoWording`] when the spec has no such line or the line
/// quotes nothing.
pub fn fixed_wording(spec: &str, number: u8) -> Result<&str, ReadmeError> {
    let start = format!("{number}. **");
    spec.lines()
        .find(|line| line.starts_with(&start))
        .and_then(|line| line.split('`').nth(1))
        .map(str::trim)
        .filter(|wording| !wording.is_empty())
        .ok_or(ReadmeError::NoWording(number))
}

/// The six items of the Features list (specs/readme.md, *Features*): the
/// first and the third from `facts`, the fourth with the tools' names
/// from them, the installer's in the wording `spec`, the text of
/// specs/readme.md, fixes for it, the others in their fixed wording.
///
/// # Errors
///
/// [`ReadmeError::NoWording`] when the spec quotes no wording for the
/// installer's item.
pub fn features(facts: &Facts, spec: &str) -> Result<String, ReadmeError> {
    // The spec's sentence, its opening in bold up to the first comma. The
    // item is marked as Linux only for as long as `ext-installer` is.
    let installer = fixed_wording(spec, INSTALLER_ITEM)?;
    let installer = match installer.split_once(',') {
        Some((opening, rest)) => format!("- **{opening}**,{rest} Linux only."),
        None => format!("- **{installer}** Linux only."),
    };
    let mut storage = vec!["local disks".to_owned()];
    storage.extend(facts.storage.iter().cloned());
    let cloud = if facts.cloud.is_empty() {
        String::new()
    } else {
        format!(" and the cloud ({})", facts.cloud.join(", "))
    };
    let search: Vec<String> = facts
        .search
        .iter()
        .filter(|(_, names)| !names.is_empty())
        .map(|(platform, names)| format!("{} on {platform}", and(names)))
        .collect();
    let tools = if facts.tools.is_empty() {
        String::new()
    } else {
        format!(", like {}", and(&facts.tools))
    };
    Ok([
        format!(
            "- **Previews {} file types**, listed by kind below.",
            facts.previews.len()
        ),
        "- **Multi-pane, multi-tab view:** split panes side by side, drag them around, keep as \
         many tabs as you like."
            .to_owned(),
        format!(
            "- **Browses** {}{cloud}, and **searches** through {}.",
            storage.join(", "),
            and(&search)
        ),
        format!(
            "- **Works with the tools you already use**{tools}: status badges in the file list \
             and actions in the context menu."
        ),
        installer,
        "- **Small things done right**, such as computing a file's hash from the context menu."
            .to_owned(),
    ]
    .join("\n"))
}

/// The previewed file types of the first feature item, complete, one
/// collapsible list per kind (specs/readme.md, *File types*): the kind and
/// its count in the summary, the types inside, the first kind open and the
/// others folded. A kind without a type has no list.
#[must_use]
pub fn file_types(facts: &Facts) -> String {
    let mut lists = Vec::new();
    for kind in Kind::ALL {
        let types: Vec<&str> = facts
            .previews
            .iter()
            .filter(|(of, _)| *of == kind)
            .map(|(_, name)| name.as_str())
            .collect();
        if types.is_empty() {
            continue;
        }
        let open = if lists.is_empty() { " open" } else { "" };
        lists.push(format!(
            "<details{open}>\n<summary>{} ({})</summary>\n\n{}\n\n</details>",
            kind.title(),
            types.len(),
            types.join(", ")
        ));
    }
    lists.join("\n\n")
}

/// The Linux archives of a release that calls Trebuchet's
/// `linux-packages.yml`: that workflow makes one per architecture, whatever
/// distribution releases it packages for (Open-Breach/trebuchet,
/// docs/linux-packages.md, *The tarball*).
const TREBUCHET_ARCHIVES: [&str; 2] = ["linux-x64", "linux-arm64"];

/// The platforms the release workflow builds an archive for, each once:
/// the ones its build matrix names (`macos-arm64`, `windows-x64`), and the
/// two Linux ones where a job calls Trebuchet's `linux-packages.yml`,
/// which builds them outside the matrix.
#[must_use]
pub fn release_platforms(workflow: &str) -> Vec<String> {
    let mut platforms: Vec<String> = Vec::new();
    for line in workflow.lines() {
        let key = line.trim_start();
        if key.starts_with("uses: ") && key.contains("/linux-packages.yml@") {
            for name in TREBUCHET_ARCHIVES {
                if !platforms.iter().any(|p| p == name) {
                    platforms.push(name.to_owned());
                }
            }
            continue;
        }
        let Some((_, rest)) = line.split_once("platform: ") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if !name.is_empty() && !platforms.contains(&name) {
            platforms.push(name);
        }
    }
    platforms
}

/// How a downloaded file is checked against its build attestation: the
/// two lines of specs/system-integration.md §9, the first for what
/// Trebuchet's workflow builds and the second for what Rick's own does,
/// and the sentence the same section asks the README to state plainly.
const VERIFY: &str = "Every file of a release has a GitHub build attestation. It proves where a \
     file comes from, not that it is safe. To check one with the GitHub CLI:\n\n```bash\n\
     gh attestation verify <file> --owner Open-Breach --signer-repo Open-Breach/trebuchet   \
     # a Linux file\n\
     gh attestation verify <file> --repo Open-Breach/rick                                   \
     # a macOS or Windows file\n```";

/// One platform's block of the Install section: its title, the links to
/// its archives and the steps.
fn install_block(title: &str, links: &[String], steps: &str) -> Option<String> {
    (!links.is_empty()).then(|| format!("**{title}** · {}\n\n{steps}", links.join(" · ")))
}

/// The line that installs Rick from the Open-Breach apt and dnf
/// repositories: Trebuchet's install script with the package name.
const PACKAGE_INSTALL: &str = "curl -fsSL https://get.openbreach.dev/install.sh | sh -s rick";

/// Where the distro releases with a repository and the manual steps are
/// written down, so that the README names no release: Trebuchet's
/// `docs/install.md`.
const PACKAGE_DOCS: &str = "https://github.com/Open-Breach/trebuchet/blob/main/docs/install.md";

/// The Linux block of the Install section: the package install first,
/// then the archive, with its `links` and the folder `linux` it unpacks
/// to.
fn linux_block(links: &[String], linux: &str) -> Option<String> {
    (!links.is_empty()).then(|| {
        format!(
            "**Linux** · Debian, Ubuntu, Fedora and the distros based on them\n\n\
             ```bash\n{PACKAGE_INSTALL}\n```\n\n\
             This adds the Open-Breach repository and installs Rick with apt or dnf; updates \
             then arrive with your system's. [Which releases, and the same steps by \
             hand]({PACKAGE_DOCS}).\n\n\
             Or run it from the archive, with nothing installed: {}\n\n\
             ```bash\ntar xzf {linux}.tar.gz && ./{linux}/rick\n```\n\n\
             Needs glibc 2.36 or later (Debian 12, Ubuntu 24.04). The first start offers to \
             set Rick up in your desktop: the launcher entry, the default file manager, the \
             `rick` command.",
            links.join(" · ")
        )
    })
}

/// The body of the Install section (specs/readme.md, *Install*): per
/// operating system the direct link to each archive of `platforms` for
/// the release `version`, and the steps to get Rick running from it, on
/// Linux after the package install; then how a file is verified, where
/// the Office and media previews come from and what they need installed,
/// and the lines on the plugins: the ones in the archive, the separate
/// downloads, the *Suggest plugins* setting and the plugin packages.
#[must_use]
pub fn install(version: &str, platforms: &[String]) -> String {
    let base = format!(
        "{}/releases/download/v{version}",
        env!("CARGO_PKG_REPOSITORY")
    );
    let links = |os: &str, extension: &str| -> Vec<String> {
        platforms
            .iter()
            .filter_map(|platform| {
                let arch = platform.strip_prefix(os)?.strip_prefix('-')?;
                Some(format!(
                    "[{arch}]({base}/rick-{version}-{platform}.{extension})"
                ))
            })
            .collect()
    };
    let first = |os: &str| -> String {
        platforms
            .iter()
            .find(|p| p.starts_with(os))
            .map_or_else(String::new, |p| format!("rick-{version}-{p}"))
    };
    let linux = first("linux");
    let blocks = [
        linux_block(&links("linux", "tar.gz"), &linux),
        install_block(
            "macOS",
            &links("macos", "zip"),
            "Unzip, drag `Rick.app` to Applications and open it. The app is not notarised yet: \
             allow it under System Settings → Privacy & Security.",
        ),
        install_block(
            "Windows",
            &links("windows", "zip"),
            "Unpack and run `rick.exe`. SmartScreen may warn the first time.",
        ),
    ];
    let mut out: Vec<String> = blocks.into_iter().flatten().collect();
    out.push(VERIFY.to_owned());
    out.push(
        "The Office preview is a separate download and needs LibreOffice installed. Audio and \
         video are in the Linux archive, where they need the GStreamer plug-ins, and a separate \
         download on macOS and Windows."
            .to_owned(),
    );
    out.push(
        "Plugins ship in the archive's `plugins/` folder; delete a plugin's folder and that \
         plugin is gone. The rest are separate downloads under *Extensions → Available*, and \
         Rick says so when one would serve what you are looking at. *Suggest plugins* in \
         Settings → Behaviour turns that off. A Rick installed from a package gets the rest as \
         packages: `rick-plugin-git`, or `rick-plugins` for all of them."
            .to_owned(),
    );
    out.join("\n\n")
}

/// The opening and closing marker of the block `name`.
fn markers(name: &str) -> (String, String) {
    (
        format!("<!-- generated:{name} -->\n"),
        format!("\n<!-- /generated:{name} -->"),
    )
}

/// Where the README holds the body of the block `name`: between its
/// markers.
fn span(readme: &str, name: &'static str) -> Result<Range<usize>, ReadmeError> {
    let (open, close) = markers(name);
    let start = readme.find(&open).ok_or(ReadmeError::NoBlock(name))? + open.len();
    let len = readme[start..]
        .find(&close)
        .ok_or(ReadmeError::NoBlock(name))?;
    Ok(start..start + len)
}

/// What the README holds between the markers of the block `name`.
///
/// # Errors
///
/// [`ReadmeError::NoBlock`] when the markers are not there.
pub fn block<'a>(readme: &'a str, name: &'static str) -> Result<&'a str, ReadmeError> {
    span(readme, name).map(|span| &readme[span])
}

/// The README with `body` between the markers of the block `name`.
///
/// # Errors
///
/// [`ReadmeError::NoBlock`] when the markers are not there.
pub fn with_block(readme: &str, name: &'static str, body: &str) -> Result<String, ReadmeError> {
    let span = span(readme, name)?;
    Ok(format!(
        "{}{body}{}",
        &readme[..span.start],
        &readme[span.end..]
    ))
}

/// The version the status line names: what follows the `v` its line
/// starts with.
///
/// # Errors
///
/// [`ReadmeError::NoBlock`] without the status block,
/// [`ReadmeError::NoVersion`] when no line of it starts with `v` and a
/// digit.
pub fn version(readme: &str) -> Result<String, ReadmeError> {
    block(readme, STATUS)?
        .lines()
        .filter_map(|line| line.strip_prefix('v'))
        .find(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        .map(|rest| {
            rest.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
                .collect()
        })
        .ok_or(ReadmeError::NoVersion)
}

/// The first release whose README has the privacy paragraph: the first
/// that fetches the plugin list by itself (specs/readme.md, section 9;
/// specs/plugin-distribution.md §6).
pub const PRIVACY_SINCE: &str = "0.6.0";

/// The heading of specs/readme.md under which the privacy paragraph is
/// quoted.
pub const PRIVACY_HEADING: &str = "### 9. Privacy (fixed)";

/// The privacy paragraph as `spec`, the text of specs/readme.md, fixes
/// it: the one paragraph of the block quote under [`PRIVACY_HEADING`].
///
/// # Errors
///
/// [`ReadmeError::NoPrivacyWording`] when the spec has no such heading,
/// no quote under it before the next heading, or a quote of more than
/// one paragraph.
pub fn privacy_wording(spec: &str) -> Result<&str, ReadmeError> {
    let (_, section) = spec
        .split_once(PRIVACY_HEADING)
        .ok_or(ReadmeError::NoPrivacyWording)?;
    let mut quoted = section
        .lines()
        .skip(1)
        .take_while(|line| !line.starts_with('#'))
        .skip_while(|line| !line.starts_with('>'))
        .take_while(|line| line.starts_with('>'))
        .map(|line| line.trim_start_matches('>').trim());
    match (quoted.next(), quoted.next()) {
        (Some(paragraph), None) if !paragraph.is_empty() => Ok(paragraph),
        _ => Err(ReadmeError::NoPrivacyWording),
    }
}

/// The three numbers of a release's version, without what follows them:
/// `0.6.0-rc1` is `[0, 6, 0]`.
fn numbers(version: &str) -> Option<[u64; 3]> {
    let mut numbers = version
        .split(['-', '+'])
        .next()?
        .split('.')
        .map(|number| number.parse::<u64>().ok());
    Some([numbers.next()??, numbers.next()??, numbers.next()??])
}

/// Holds `readme` against the privacy paragraph `spec`, the text of
/// specs/readme.md, fixes (section 9). This is how the paragraph comes
/// into force with a release and not before, while the README stays
/// untouched between releases:
///
/// - A README whose status line names [`PRIVACY_SINCE`] or a later
///   release has the paragraph, word for word, as a line of its own: the
///   first under the rule (`---`) that sets the closing lines off, above
///   the line of links.
/// - A README that names an earlier release does not have it: it
///   describes a release that makes no request by itself.
///
/// The release pull request writes the new version into the status line
/// (`cargo xtask readme`), and from that commit this fails until the
/// paragraph is there.
///
/// # Errors
///
/// [`ReadmeError::NoPrivacy`] and [`ReadmeError::PrivacyEarly`] as
/// above; [`ReadmeError::NoPrivacyWording`] when the spec quotes none;
/// [`ReadmeError::NoBlock`] and [`ReadmeError::NoVersion`] when the
/// status line names no release.
pub fn check_privacy(readme: &str, spec: &str) -> Result<(), ReadmeError> {
    let wording = privacy_wording(spec)?;
    let version = version(readme)?;
    let since = numbers(PRIVACY_SINCE).ok_or(ReadmeError::NoVersion)?;
    let described = numbers(&version).ok_or(ReadmeError::NoVersion)?;
    if described >= since {
        // Under the rule, a paragraph of its own, then the links.
        let in_place = format!("\n---\n\n{wording}\n\n[");
        if readme.matches(wording).count() == 1 && readme.contains(&in_place) {
            Ok(())
        } else {
            Err(ReadmeError::NoPrivacy { version })
        }
    } else if readme.contains(wording) {
        Err(ReadmeError::PrivacyEarly { version })
    } else {
        Ok(())
    }
}

/// The README brought up to the release `version`: the version in the
/// status line and the call to action, whose wording stays as it is, the
/// Features list and the file types from `facts`, with the wording `spec`
/// (the text of specs/readme.md) fixes for the installer's item, and the
/// Install section for `platforms`. Everything outside the four blocks is
/// left alone.
///
/// # Errors
///
/// [`ReadmeError`] when a block or the status line's version is missing,
/// or the spec quotes no wording for the installer's item.
pub fn regenerate(
    readme: &str,
    spec: &str,
    version: &str,
    facts: &Facts,
    platforms: &[String],
) -> Result<String, ReadmeError> {
    let old = self::version(readme)?;
    let status = block(readme, STATUS)?.replace(&format!("v{old}"), &format!("v{version}"));
    let readme = with_block(readme, STATUS, &status)?;
    let readme = with_block(&readme, FEATURES, &features(facts, spec)?)?;
    let readme = with_block(&readme, FILE_TYPES, &file_types(facts))?;
    with_block(&readme, INSTALL, &install(version, platforms))
}

#[cfg(test)]
mod tests {
    use super::*;

    const README: &str = "# Rick\n\n<!-- generated:status -->\nv0.4.0 · early · this page.\n\n\
        [Download v0.4.0](#install) now.\n<!-- /generated:status -->\n\n## Features\n\n\
        <!-- generated:features -->\nold\n<!-- /generated:features -->\n\n\
        <!-- generated:file-types -->\nold\n<!-- /generated:file-types -->\n\nComing:\n\n- more\n\n\
        ## Install\n\n<!-- generated:install -->\nold\n<!-- /generated:install -->\n\nEnd.\n";

    /// The Features list of a spec, as specs/readme.md writes it.
    const SPEC: &str = "### 3. Features (regenerated)\n\n\
        4. **Tool integration** (fixed wording, live names). `Works with the tools.`\n\
        5. **Package manager** (fixed). `Installs what you download, so a download becomes \
        an install, whatever it is.` Marked as Linux only for as long as `ext-installer` is.\n\
        6. **Convenience functions** (fixed). `Small things done right.`\n";

    fn facts() -> Facts {
        Facts {
            previews: vec![
                (Kind::Documents, "PDF".to_owned()),
                (Kind::Media, "JPG".to_owned()),
                (Kind::Media, "PNG".to_owned()),
            ],
            storage: vec!["Archives".to_owned()],
            cloud: vec!["S3".to_owned(), "Azure Blob".to_owned()],
            search: vec![
                ("Linux", vec!["Tracker".to_owned(), "Baloo".to_owned()]),
                ("macOS", vec!["Spotlight".to_owned()]),
                ("Windows", Vec::new()),
            ],
            tools: vec!["Git".to_owned(), "SVN".to_owned()],
        }
    }

    #[test]
    fn a_type_is_named_once_whatever_its_spellings() {
        assert_eq!(type_name("jpeg"), "JPG");
        assert_eq!(type_name("jpg"), "JPG");
        assert_eq!(type_name("markdown"), "Markdown");
        assert_eq!(type_name("tgz"), "TAR.GZ");
        // A type the table does not know is its extension in capitals.
        assert_eq!(type_name("stl"), "STL");
        assert_eq!(type_name("3mf"), "3MF");
    }

    #[test]
    fn the_features_count_the_types_and_name_what_is_there() {
        let text = features(&facts(), SPEC).unwrap_or_else(|e| panic!("{e}"));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(
            lines[0],
            "- **Previews 3 file types**, listed by kind below."
        );
        assert_eq!(
            lines[2],
            "- **Browses** local disks, Archives and the cloud (S3, Azure Blob), and \
             **searches** through Tracker and Baloo on Linux and Spotlight on macOS."
        );
        assert!(lines[3].contains("you already use**, like Git and SVN: status badges"));
        // No tool, no name.
        let none = features(
            &Facts {
                tools: Vec::new(),
                ..facts()
            },
            SPEC,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(none.contains("you already use**: status badges"));
    }

    /// The installer's item is the sentence the spec fixes, taken from the
    /// spec's text and not kept here a second time: its opening in bold up
    /// to the first comma, and marked as Linux only. A spec that quotes no
    /// wording for the item is refused.
    #[test]
    fn the_installers_item_is_the_wording_the_spec_fixes() {
        assert_eq!(
            fixed_wording(SPEC, 5),
            Ok("Installs what you download, so a download becomes an install, whatever it is.")
        );
        assert_eq!(fixed_wording(SPEC, 6), Ok("Small things done right."));
        let text = features(&facts(), SPEC).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            text.lines().nth(4),
            Some(
                "- **Installs what you download**, so a download becomes an install, whatever \
                 it is. Linux only."
            )
        );
        // A sentence without a comma is bold as a whole.
        let plain = features(&facts(), "5. **Installer** (fixed). `Installs things.`")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            plain.lines().nth(4),
            Some("- **Installs things.** Linux only.")
        );
        // No such item, an item that quotes nothing, an empty quotation.
        for spec in [
            "",
            "4. **Tool integration** (fixed). `Works.`\n15. **Other** (fixed). `No.`",
            "5. **Installer** (fixed). Installs things.",
            "5. **Installer** (fixed). ``",
        ] {
            assert_eq!(
                fixed_wording(spec, 5),
                Err(ReadmeError::NoWording(5)),
                "{spec:?}"
            );
            assert_eq!(
                features(&facts(), spec),
                Err(ReadmeError::NoWording(5)),
                "{spec:?}"
            );
            assert_eq!(
                regenerate(README, spec, "0.5.0", &facts(), &[]),
                Err(ReadmeError::NoWording(5)),
                "{spec:?}"
            );
        }
    }

    #[test]
    fn the_file_types_are_listed_by_kind_the_first_open() {
        assert_eq!(
            file_types(&facts()),
            "<details open>\n<summary>Media (2)</summary>\n\nJPG, PNG\n\n</details>\n\n\
             <details>\n<summary>Documents (1)</summary>\n\nPDF\n\n</details>"
        );
    }

    #[test]
    fn a_type_takes_its_previewers_kind_unless_it_is_named_otherwise() {
        assert_eq!(kind_of("ext-preview-text", "TXT"), Some(Kind::Documents));
        assert_eq!(kind_of("ext-preview-text", "TOML"), Some(Kind::Programming));
        assert_eq!(kind_of("ext-preview-code", "Rust"), Some(Kind::Programming));
        // A previewer nobody gave a kind has none for its types.
        assert_eq!(kind_of("ext-preview-new", "NEW"), None);
    }

    #[test]
    fn the_platforms_are_read_from_the_build_matrix() {
        let workflow = "matrix:\n  include:\n    - { os: ubuntu-24.04, platform: linux-x64, \
            pdfium: a }\n    - { os: macos-latest, platform: macos-arm64 }\n  smoke:\n    \
            - { os: ubuntu-24.04, platform: linux-x64 }\n";
        assert_eq!(release_platforms(workflow), ["linux-x64", "macos-arm64"]);
    }

    #[test]
    fn the_linux_archives_come_with_the_call_to_trebuchet() {
        let workflow = "# Linux: linux-packages.yml@main builds it\njobs:\n  linux:\n    uses: \
            Open-Breach/trebuchet/.github/workflows/linux-packages.yml@main\n  build:\n    \
            strategy:\n      matrix:\n        include:\n          - { os: macos-latest, \
            platform: macos-arm64 }\n";
        assert_eq!(
            release_platforms(workflow),
            ["linux-x64", "linux-arm64", "macos-arm64"]
        );
        // A comment that names the workflow is no call.
        let workflow =
            "# Linux: linux-packages.yml@main builds it\n    - { platform: macos-arm64 }\n";
        assert_eq!(release_platforms(workflow), ["macos-arm64"]);
    }

    #[test]
    fn the_install_section_links_every_archive_of_the_release() {
        let platforms = ["linux-x64", "linux-arm64", "windows-x64"].map(str::to_owned);
        let text = install("1.2.3", &platforms);
        // Linux leads with the package install and names no distro release
        // there: those are in Trebuchet's docs/install.md.
        assert!(text.starts_with(
            "**Linux** · Debian, Ubuntu, Fedora and the distros based on them\n\n```bash\n\
             curl -fsSL https://get.openbreach.dev/install.sh | sh -s rick\n```\n\n\
             This adds the Open-Breach repository and installs Rick with apt or dnf; updates \
             then arrive with your system's. [Which releases, and the same steps by hand]\
             (https://github.com/Open-Breach/trebuchet/blob/main/docs/install.md).\n\n\
             Or run it from the archive, with nothing installed: \
             [x64](https://github.com/Open-Breach/rick/releases/download/v1.2.3/\
             rick-1.2.3-linux-x64.tar.gz) · [arm64](https://github.com/Open-Breach/rick/\
             releases/download/v1.2.3/rick-1.2.3-linux-arm64.tar.gz)\n\n```bash\n\
             tar xzf rick-1.2.3-linux-x64.tar.gz && ./rick-1.2.3-linux-x64/rick\n```\n\n\
             Needs glibc 2.36 or later"
        ));
        // No archive for Linux, no Linux block, the package line included.
        assert!(!install("1.2.3", &["windows-x64".to_owned()]).contains("install.sh"));
        assert!(text.contains("rick-1.2.3-windows-x64.zip"));
        // No archive for a platform, no block for it.
        assert!(!text.contains("**macOS**"));
        // The two lines that verify a file, one for each builder.
        assert!(text.contains(
            "gh attestation verify <file> --owner Open-Breach --signer-repo Open-Breach/trebuchet"
        ));
        assert!(text.contains("gh attestation verify <file> --repo Open-Breach/rick"));
        assert!(text.contains("\n\nThe Office preview is a separate download and needs"));
        assert!(text.contains("\n\nPlugins ship in the archive's `plugins/` folder;"));
        assert!(text.ends_with(
            "*Suggest plugins* in Settings → Behaviour turns that off. A Rick installed from a \
             package gets the rest as packages: `rick-plugin-git`, or `rick-plugins` for all of \
             them."
        ));
    }

    #[test]
    fn regenerating_changes_the_blocks_and_nothing_else() {
        assert_eq!(version(README).as_deref(), Ok("0.4.0"));
        let platforms = ["linux-x64".to_owned()];
        let new = regenerate(README, SPEC, "0.5.0", &facts(), &platforms)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(version(&new).as_deref(), Ok("0.5.0"));
        assert_eq!(
            block(&new, STATUS),
            Ok("v0.5.0 · early · this page.\n\n[Download v0.5.0](#install) now.")
        );
        let list = features(&facts(), SPEC).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(block(&new, FEATURES), Ok(list.as_str()));
        assert_eq!(block(&new, FILE_TYPES), Ok(file_types(&facts()).as_str()));
        assert_eq!(
            block(&new, INSTALL),
            Ok(install("0.5.0", &platforms).as_str())
        );
        assert!(new.starts_with("# Rick\n\n<!-- generated:status -->\n"));
        assert!(new.contains("<!-- /generated:file-types -->\n\nComing:\n\n- more\n\n## Install"));
        assert!(new.ends_with("<!-- /generated:install -->\n\nEnd.\n"));
        // The same again changes nothing.
        assert_eq!(
            regenerate(&new, SPEC, "0.5.0", &facts(), &platforms),
            Ok(new)
        );
    }

    /// A spec that quotes a privacy paragraph, and the paragraph.
    const PRIVACY_SPEC: &str = "### 8. Portal animation (fixed)\n\nAn animation.\n\n\
        ### 9. Privacy (fixed)\n\nOne paragraph. Verbatim:\n\n\
        > Rick asks *one* thing by itself.\n\n- From 0.6.0.\n\n### Closing lines\n\n- Links.\n";
    const PRIVACY: &str = "Rick asks *one* thing by itself.";

    /// A README that describes `version`, with `foot` as its end.
    fn readme_of(version: &str, foot: &str) -> String {
        format!(
            "# Rick\n\n<!-- generated:status -->\nv{version} · early.\n\n\
             [Download v{version}](#install) now.\n<!-- /generated:status -->\n\n\
             ## Join in\n\nCome.\n\n{foot}"
        )
    }

    #[test]
    fn the_privacy_paragraph_is_the_one_the_spec_quotes() {
        assert_eq!(privacy_wording(PRIVACY_SPEC), Ok(PRIVACY));
        for spec in [
            // No such section, no quote in it, and a quote that belongs
            // to the next section.
            "### 8. Portal animation (fixed)\n\n> Not this.\n",
            "### 9. Privacy (fixed)\n\nOne paragraph.\n",
            "### 9. Privacy (fixed)\n\nOne paragraph.\n\n### Closing lines\n\n> Not this.\n",
            // Two paragraphs are not one.
            "### 9. Privacy (fixed)\n\n> One.\n>\n> Two.\n",
            "### 9. Privacy (fixed)\n\n>\n",
        ] {
            assert_eq!(
                privacy_wording(spec),
                Err(ReadmeError::NoPrivacyWording),
                "{spec:?}"
            );
        }
    }

    #[test]
    fn the_privacy_paragraph_comes_into_force_with_its_release() {
        let without = "---\n\n[Building](docs/building.md)\n\nLicensed.\n";
        let with = format!("---\n\n{PRIVACY}\n\n[Building](docs/building.md)\n\nLicensed.\n");
        let missing = |version: &str| {
            Err(ReadmeError::NoPrivacy {
                version: version.to_owned(),
            })
        };

        // Before the release: the README is as the last release left it,
        // and the paragraph would describe a request that release does
        // not make.
        for version in ["0.4.0", "0.5.0", "0.5.9"] {
            assert_eq!(
                check_privacy(&readme_of(version, without), PRIVACY_SPEC),
                Ok(()),
                "{version}"
            );
            assert_eq!(
                check_privacy(&readme_of(version, &with), PRIVACY_SPEC),
                Err(ReadmeError::PrivacyEarly {
                    version: version.to_owned()
                }),
                "{version}"
            );
        }

        // From the release on: the status line names it, and the
        // paragraph has to be there.
        for version in ["0.6.0", "0.6.1", "0.10.0", "1.0.0"] {
            assert_eq!(
                check_privacy(&readme_of(version, without), PRIVACY_SPEC),
                missing(version),
                "{version}"
            );
            assert_eq!(
                check_privacy(&readme_of(version, &with), PRIVACY_SPEC),
                Ok(()),
                "{version}"
            );
        }

        // Word for word, and in its place.
        for (case, foot) in [
            (
                "other words",
                "---\n\nRick asks one thing by itself.\n\n[Building](docs/building.md)\n"
                    .to_owned(),
            ),
            (
                "above the rule",
                format!("{PRIVACY}\n\n---\n\n[Building](docs/building.md)\n"),
            ),
            (
                "under the links",
                format!("---\n\n[Building](docs/building.md)\n\n{PRIVACY}\n"),
            ),
            (
                "inside another paragraph",
                format!("---\n\nNote: {PRIVACY}\n\n[Building](docs/building.md)\n"),
            ),
            (
                "twice",
                format!("{PRIVACY}\n\n---\n\n{PRIVACY}\n\n[Building](docs/building.md)\n"),
            ),
        ] {
            assert_eq!(
                check_privacy(&readme_of("0.6.0", &foot), PRIVACY_SPEC),
                missing("0.6.0"),
                "{case}"
            );
        }

        // A spec that quotes no paragraph, and a README that names no
        // release, are said as what they are.
        assert_eq!(
            check_privacy(&readme_of("0.6.0", &with), "### 9. Privacy (fixed)\n"),
            Err(ReadmeError::NoPrivacyWording)
        );
        assert_eq!(
            check_privacy("# Rick\n", PRIVACY_SPEC),
            Err(ReadmeError::NoBlock(STATUS))
        );
        assert_eq!(numbers("0.6.0-rc1+build.5"), Some([0, 6, 0]));
        assert_eq!(numbers("0.6"), None);
        assert_eq!(numbers("six"), None);
    }

    #[test]
    fn a_readme_without_its_markers_is_refused() {
        assert_eq!(
            block("# Rick\n", FEATURES),
            Err(ReadmeError::NoBlock(FEATURES))
        );
        assert_eq!(
            version("<!-- generated:status -->\nearly\n<!-- /generated:status -->"),
            Err(ReadmeError::NoVersion)
        );
    }
}
