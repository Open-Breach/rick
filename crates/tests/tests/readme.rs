//! Acceptance (specs/readme.md): `README.md` has the parts of the spec in
//! their order with the fixed text word for word, and its regenerated
//! parts are what the release's extensions register — the README is
//! checked, not trusted.
//!
//! The README describes the latest release, so its live parts are held
//! against the code only when the workspace's version is a release
//! version: between releases, on a `-dev` version, the page stays as the
//! last release left it. `RICK_README_WRITE=1` makes the live test write
//! the README instead of comparing it, which is what `cargo xtask readme`
//! runs in the release pull request.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use platform::Fake;
use tests::readme::{self, FEATURES, Facts, INSTALL, STATUS};

/// The workspace's version: every crate has it.
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    let path = root().join(path);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn facts() -> Facts {
    let platform: Arc<dyn platform::Platform> =
        Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    Facts::gather(&readme::extensions(), &platform).unwrap_or_else(|e| panic!("{e}"))
}

/// The paragraphs of the block quote that follows `heading` in the spec.
fn quoted(spec: &str, heading: &str) -> Vec<String> {
    let (_, after) = spec
        .split_once(heading)
        .unwrap_or_else(|| panic!("specs/readme.md has no `{heading}`"));
    after
        .lines()
        .skip_while(|line| !line.trim_start().starts_with('>'))
        .take_while(|line| line.trim_start().starts_with('>'))
        .map(|line| line.trim_start().trim_start_matches('>').trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

/// The live lists of the Features section are what the extensions of the
/// release register, and the Install section links the archives the
/// release workflow builds, for the version of the workspace.
#[test]
fn the_regenerated_parts_are_what_the_release_registers() {
    let text = read("README.md");
    let platforms = readme::release_platforms(&read(".github/workflows/release.yml"));
    assert!(!platforms.is_empty(), "no platform in release.yml");
    if std::env::var_os("RICK_README_WRITE").is_some() {
        let new = readme::regenerate(&text, VERSION, &facts(), &platforms)
            .unwrap_or_else(|e| panic!("{e}"));
        fs::write(root().join("README.md"), new).unwrap_or_else(|e| panic!("{e}"));
        return;
    }
    if !semver_is_release(VERSION) {
        eprintln!(
            "the workspace is at {VERSION}, between releases: README.md describes {} and is \
             not held against the code",
            readme::version(&text).unwrap_or_else(|e| panic!("{e}"))
        );
        return;
    }
    let expected =
        readme::regenerate(&text, VERSION, &facts(), &platforms).unwrap_or_else(|e| panic!("{e}"));
    for name in [STATUS, FEATURES, INSTALL] {
        assert_eq!(
            readme::block(&text, name),
            readme::block(&expected, name),
            "the `{name}` block of README.md is not what release {VERSION} registers; run \
             `cargo xtask readme`"
        );
    }
}

/// A release version has no pre-release part: `0.4.0`, not `0.5.0-dev`.
fn semver_is_release(version: &str) -> bool {
    !version.contains('-')
}

/// The list of extensions the README is written from is the `ext-*`
/// crates of the workspace, no more and no fewer: a new plugin cannot be
/// left out, and what the bundle builds is what is asked.
#[test]
fn every_extension_crate_is_asked() {
    let mut crates: Vec<String> = fs::read_dir(root().join("crates"))
        .unwrap_or_else(|e| panic!("{e}"))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("ext-"))
        .collect();
    crates.sort();
    let mut asked: Vec<String> = readme::extensions()
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();
    asked.sort();
    assert_eq!(asked, crates);
}

/// What the previewers, providers and connectors of this build register
/// reaches the facts: the model of the screenshot has a previewer, the
/// cloud is told from the rest, each platform has its search, and the
/// plugins with a view of their own are not tools.
#[test]
fn the_facts_are_read_from_the_registries() {
    let facts = facts();
    for name in ["PDF", "PNG", "JPG", "SVG", "STL", "Markdown"] {
        assert!(facts.previews.iter().any(|p| p == name), "{name}");
    }
    let mut once = facts.previews.clone();
    once.sort();
    once.dedup();
    assert_eq!(once.len(), facts.previews.len(), "a type is named twice");
    assert_eq!(facts.storage, ["Archives", "SSH"]);
    assert_eq!(facts.cloud, ["S3", "Azure Blob", "Google Cloud Storage"]);
    assert_eq!(
        facts.search,
        [
            ("Linux", vec!["Tracker".to_owned(), "Baloo".to_owned()]),
            ("macOS", vec!["Spotlight".to_owned()]),
            ("Windows", vec!["Windows Search".to_owned()]),
        ]
    );
    assert_eq!(facts.tools, ["Git", "SVN"]);
}

/// The parts of specs/readme.md, *Structure*, in their order, the fixed
/// text as the spec quotes it, and everything the page points at in the
/// repository.
#[test]
fn the_readme_has_the_structure_of_its_spec() {
    let text = read("README.md");
    let spec = read("specs/readme.md");

    // The parts, each after the one before.
    let parts = [
        "# Rick\n",
        "<!-- generated:status -->",
        "## Why Rick exists\n",
        "## Features\n",
        "<!-- generated:features -->",
        "\nComing:\n",
        "<picture>",
        "## Install\n",
        "<!-- generated:install -->",
        "> **Use with caution.**",
        "## Join in\n",
        "(docs/building.md)",
        "(docs/command-line.md)",
        "(docs/architecture.md)",
        "(CHANGELOG.md)",
        "[Apache-2.0](LICENSE)",
        "*Named after a certain genius scientist",
    ];
    let mut from = 0;
    for part in parts {
        let at = text[from..]
            .find(part)
            .unwrap_or_else(|| panic!("README.md: `{}` is missing or out of order", part.trim()));
        from += at + part.len();
    }
    assert!(text.starts_with("# Rick\n\n"));

    // Sections 1 and 2, word for word.
    let what = quoted(&spec, "### 1. What Rick is (fixed)");
    assert_eq!(what.len(), 1);
    assert!(
        text.starts_with(&format!("# Rick\n\n{}\n", what[0])),
        "the first paragraph is not the spec's"
    );
    let why = quoted(&spec, "### 2. Why Rick exists (fixed)");
    assert_eq!(why.len(), 3);
    assert!(
        text.contains(&format!("## Why Rick exists\n\n{}\n", why.join("\n\n"))),
        "*Why Rick exists* is not the spec's text"
    );

    // The status line and the call to action name one version.
    let version = readme::version(&text).unwrap_or_else(|e| panic!("{e}"));
    let status = readme::block(&text, STATUS).unwrap_or_else(|e| panic!("{e}"));
    let mut lines = status.lines().filter(|line| !line.is_empty());
    let line = lines.next().unwrap_or_default();
    assert!(
        line.starts_with(&format!("v{version} · "))
            && line.ends_with(" · this page describes the latest release; main may be ahead."),
        "{line}"
    );
    let call = quoted(&spec, "3. The **call to action**");
    let plain: String = lines
        .next()
        .unwrap_or_default()
        .replace("](#install)", "")
        .replace(&format!("]({}/issues)", env!("CARGO_PKG_REPOSITORY")), "")
        .replace('[', "");
    assert_eq!(plain, call[0].replace("v0.4.0", &format!("v{version}")));
    assert_eq!(lines.next(), None);

    // Six features, and the Coming list: Serge's, of five entries at most.
    let features = readme::block(&text, FEATURES).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(features.lines().count(), 6);
    assert!(features.lines().all(|line| line.starts_with("- ")));
    let (_, after) = text
        .split_once("\nComing:\n\n")
        .unwrap_or_else(|| panic!("no Coming list"));
    let coming = after.lines().take_while(|l| l.starts_with("- ")).count();
    assert!((1..=5).contains(&coming), "{coming} entries under Coming");

    // The picture in both schemes, and the model it shows.
    assert!(text.contains(
        "<source media=\"(prefers-color-scheme: dark)\" srcset=\"docs/screenshots/readme-dark.png\">"
    ));
    assert!(text.contains("src=\"docs/screenshots/readme-light.png\""));
    for file in [
        "docs/screenshots/readme-light.png",
        "docs/screenshots/readme-dark.png",
        "assets/readme/robot-dog.stl",
        "docs/building.md",
        "docs/command-line.md",
        "docs/architecture.md",
        "CHANGELOG.md",
        "LICENSE",
    ] {
        assert!(root().join(file).is_file(), "{file} is not there");
    }

    // Nothing that belongs to the changelog: no issue or pull request
    // number, no entry of the retired questions file.
    let numbered = text
        .match_indices('#')
        .any(|(at, _)| text[at + 1..].starts_with(|c: char| c.is_ascii_digit()));
    assert!(!numbered, "README.md names an issue or pull request number");
    assert!(!text.contains("QUESTIONS.md"));
}
