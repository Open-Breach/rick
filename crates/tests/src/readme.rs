//! The parts of `README.md` that are regenerated with every release
//! (specs/readme.md): the version in the status line and the call to
//! action, the Features list with its live items, and the Install section.
//!
//! Nothing here is typed from memory. [`extensions`] is every extension a
//! release carries, the ones compiled into the binary and the plugins;
//! [`Facts::gather`] loads each into a registry and keeps what it
//! registered and what its manifest declares: the file types its
//! previewers claim, the providers, the search connectors and the
//! platforms they are for. [`features`] and [`install`] write the Markdown
//! from that, and [`regenerate`] puts it between the README's markers:
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
use std::sync::Arc;

use ext::{Extension, Platforms, Registry};

/// The block with the status line and the call to action.
pub const STATUS: &str = "status";
/// The block with the six feature items.
pub const FEATURES: &str = "features";
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
    /// An extension of the bundle did not register.
    #[error("{name} did not register: {reason}")]
    Extension {
        /// The crate.
        name: &'static str,
        /// What the registry said.
        reason: String,
    },
}

/// Every extension a release carries, by the name of its crate: the
/// previewers first, in the order the Features list names their types.
/// `tests/readme.rs` holds this list against the `ext-*` crates of the
/// workspace, so a new plugin cannot be left out of the README.
#[must_use]
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

/// What the extensions of a release register, as the Features list says
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Facts {
    /// The file types the previewers claim, by their short names, each
    /// once, in the order of [`extensions`] and of each manifest.
    pub previews: Vec<String>,
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
    /// [`ReadmeError::Extension`] when one does not register.
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
                    let name = type_name(extension);
                    if !facts.previews.contains(&name) {
                        facts.previews.push(name);
                    }
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

/// The short name of the file type a previewer claims as `extension`:
/// the entry of [`TYPE_NAMES`], or the extension in capitals.
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

/// The six items of the Features list (specs/readme.md, *Features*): the
/// first and the third from `facts`, the fourth with the tools' names
/// from them, the others in their fixed wording.
#[must_use]
pub fn features(facts: &Facts) -> String {
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
    [
        format!(
            "- **Previews {} file types:** {}.",
            facts.previews.len(),
            facts.previews.join(", ")
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
        "- **Installs and uninstalls software distributed as tarballs**, so a tar.gz download \
         becomes a managed install instead of a folder you forget about. Linux only."
            .to_owned(),
        "- **Small things done right**, such as computing a file's hash from the context menu."
            .to_owned(),
    ]
    .join("\n")
}

/// The platforms the release workflow builds an archive for, as its build
/// matrix names them (`linux-x64`, `macos-arm64`), each once.
#[must_use]
pub fn release_platforms(workflow: &str) -> Vec<String> {
    let mut platforms: Vec<String> = Vec::new();
    for line in workflow.lines() {
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

/// One platform's block of the Install section: its title, the links to
/// its archives and the steps.
fn install_block(title: &str, links: &[String], steps: &str) -> Option<String> {
    (!links.is_empty()).then(|| format!("**{title}** · {}\n\n{steps}", links.join(" · ")))
}

/// The body of the Install section (specs/readme.md, *Install*): per
/// operating system the direct link to each archive of `platforms` for
/// the release `version`, and the steps to get Rick running from it; then
/// the line on the plugins.
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
        install_block(
            "Linux",
            &links("linux", "tar.gz"),
            &format!(
                "```bash\ntar xzf {linux}.tar.gz && ./{linux}/rick\n```\n\nNeeds glibc 2.39 or \
                 later (Ubuntu 24.04). The first start offers to set Rick up in your desktop: \
                 the launcher entry, the default file manager, the `rick` command."
            ),
        ),
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
    out.push(
        "Plugins ship in the archive's `plugins/` folder; delete a plugin's folder and that \
         plugin is gone."
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

/// The README brought up to the release `version`: the version in the
/// status line and the call to action, whose wording stays as it is, the
/// Features list from `facts` and the Install section for `platforms`.
/// Everything outside the three blocks is left alone.
///
/// # Errors
///
/// [`ReadmeError`] when a block or the status line's version is missing.
pub fn regenerate(
    readme: &str,
    version: &str,
    facts: &Facts,
    platforms: &[String],
) -> Result<String, ReadmeError> {
    let old = self::version(readme)?;
    let status = block(readme, STATUS)?.replace(&format!("v{old}"), &format!("v{version}"));
    let readme = with_block(readme, STATUS, &status)?;
    let readme = with_block(&readme, FEATURES, &features(facts))?;
    with_block(&readme, INSTALL, &install(version, platforms))
}

#[cfg(test)]
mod tests {
    use super::*;

    const README: &str = "# Rick\n\n<!-- generated:status -->\nv0.4.0 · early · this page.\n\n\
        [Download v0.4.0](#install) now.\n<!-- /generated:status -->\n\n## Features\n\n\
        <!-- generated:features -->\nold\n<!-- /generated:features -->\n\nComing:\n\n- more\n\n\
        ## Install\n\n<!-- generated:install -->\nold\n<!-- /generated:install -->\n\nEnd.\n";

    fn facts() -> Facts {
        Facts {
            previews: vec!["PDF".to_owned(), "JPG".to_owned()],
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
        let text = features(&facts());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0], "- **Previews 2 file types:** PDF, JPG.");
        assert_eq!(
            lines[2],
            "- **Browses** local disks, Archives and the cloud (S3, Azure Blob), and \
             **searches** through Tracker and Baloo on Linux and Spotlight on macOS."
        );
        assert!(lines[3].contains("you already use**, like Git and SVN: status badges"));
        // No tool, no name.
        let none = features(&Facts {
            tools: Vec::new(),
            ..facts()
        });
        assert!(none.contains("you already use**: status badges"));
    }

    #[test]
    fn the_platforms_are_read_from_the_build_matrix() {
        let workflow = "matrix:\n  include:\n    - { os: ubuntu-24.04, platform: linux-x64, \
            pdfium: a }\n    - { os: macos-latest, platform: macos-arm64 }\n  smoke:\n    \
            - { os: ubuntu-24.04, platform: linux-x64 }\n";
        assert_eq!(release_platforms(workflow), ["linux-x64", "macos-arm64"]);
    }

    #[test]
    fn the_install_section_links_every_archive_of_the_release() {
        let platforms = ["linux-x64", "linux-arm64", "windows-x64"].map(str::to_owned);
        let text = install("1.2.3", &platforms);
        assert!(text.contains(
            "**Linux** · [x64](https://github.com/Open-Breach/rick/releases/download/v1.2.3/\
             rick-1.2.3-linux-x64.tar.gz) · [arm64](https://github.com/Open-Breach/rick/\
             releases/download/v1.2.3/rick-1.2.3-linux-arm64.tar.gz)"
        ));
        assert!(
            text.contains("tar xzf rick-1.2.3-linux-x64.tar.gz && ./rick-1.2.3-linux-x64/rick")
        );
        assert!(text.contains("rick-1.2.3-windows-x64.zip"));
        // No archive for a platform, no block for it.
        assert!(!text.contains("macOS"));
        assert!(text.ends_with("that plugin is gone."));
    }

    #[test]
    fn regenerating_changes_the_blocks_and_nothing_else() {
        assert_eq!(version(README).as_deref(), Ok("0.4.0"));
        let platforms = ["linux-x64".to_owned()];
        let new =
            regenerate(README, "0.5.0", &facts(), &platforms).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(version(&new).as_deref(), Ok("0.5.0"));
        assert_eq!(
            block(&new, STATUS),
            Ok("v0.5.0 · early · this page.\n\n[Download v0.5.0](#install) now.")
        );
        assert_eq!(block(&new, FEATURES), Ok(features(&facts()).as_str()));
        assert_eq!(
            block(&new, INSTALL),
            Ok(install("0.5.0", &platforms).as_str())
        );
        assert!(new.starts_with("# Rick\n\n<!-- generated:status -->\n"));
        assert!(new.contains("<!-- /generated:features -->\n\nComing:\n\n- more\n\n## Install"));
        assert!(new.ends_with("<!-- /generated:install -->\n\nEnd.\n"));
        // The same again changes nothing.
        assert_eq!(regenerate(&new, "0.5.0", &facts(), &platforms), Ok(new));
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
