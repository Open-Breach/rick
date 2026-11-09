//! The About dialog (specs/about-dialog.md): a modal in the settings
//! window's chrome, opened from the app menu's *About Rick*. The About page
//! shows the hero — the app symbol, the name, the tagline, the version pill
//! and the one monospace build line — with *Copy details* and *What's new*
//! under it, then the link card, the licence card, the credits card and the
//! footer. A read-only text page shows the running version's section of
//! the changelog and the licence; the Third-party licences page lists
//! every component with its licence.
//!
//! [`Build`] is what the binary knows about its own build and hands in:
//! the build-time values of `crates/rick/build.rs`, the toolchain, the
//! contributors, and the component list cargo-about generated at release
//! time (`third-party.json` beside the binary, `None` in a build without
//! it). [`details`] is the plain text *Copy details* copies and
//! `rick --version` prints; everything the dialog shows is computed from
//! the build, the app's plugin rows and the embedded texts — nothing is
//! read from git or the network at run time.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use app::{App, Plugin, PluginStatus};
use egui::{Align, Align2, FontId, Rect, Sense, pos2, vec2};
use ext::{Level, t};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Deserializer};

use crate::keys::Intent;
use crate::shell::Request;
use crate::{icons, theme, widgets};

/// The dialog's width.
pub const WIDTH: f32 = 520.0;
/// The height of the pages with a scrolling body; the About page takes its
/// height from its content.
pub const PAGE_HEIGHT: f32 = 640.0;
/// The dialog's corner radius.
const RADIUS: u8 = 14;
/// A card's corner radius.
const CARD_RADIUS: u8 = 10;
/// The round close and back buttons.
const ROUND_BUTTON: f32 = 28.0;
/// The title bar: 14 above the round button.
const TITLE_BAR: f32 = 42.0;
/// The symbol in the hero.
const SYMBOL: f32 = 96.0;
/// The body's side padding.
const PAD: f32 = 24.0;
/// A card row and a credit row.
const ROW: f32 = 38.0;
const CREDIT_ROW: f32 = 52.0;
/// The avatar circle of a credit.
const AVATAR: f32 = 32.0;
/// The hero's text sizes: the name, the tagline, the build line.
const TEXT_NAME: f32 = 26.0;
const TEXT_TAGLINE: f32 = 14.0;
const TEXT_BUILD: f32 = 12.0;
/// A page title.
const TEXT_TITLE: f32 = 16.0;

/// Rick's repository, which the Source code row opens and the Website row
/// too until a site exists; also what marks the workspace's own crates in
/// the generated component list.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// GitHub's new-issue page; *Report an issue* appends the details.
const NEW_ISSUE: &str = "https://github.com/Open-Breach/rick/issues/new";
/// The generated component list's name, beside the binary.
pub const THIRD_PARTY_FILE: &str = "third-party.json";
/// The changelog and the licence, embedded at build time.
const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");
const LICENSE: &str = include_str!("../../../LICENSE");
/// The components shipped outside Cargo, listed by hand.
const BUNDLED: &str = include_str!("../../../about/bundled.toml");
/// The symbol the hero shows: the rings of the app icon alone, on nothing
/// (specs/app-icons.md).
const SYMBOL_SVG: &[u8] = include_bytes!("../../../assets/icons/svg/rick-symbol.svg");

/// What the binary knows about its own build.
#[derive(Debug, Clone, Default)]
pub struct Build {
    /// The package version, `0.3.0-dev`.
    pub version: String,
    /// The short commit (`RICK_GIT_HASH`).
    pub commit: String,
    /// The target triple (`RICK_TARGET`).
    pub target: String,
    /// The UTC build date, `YYYY-MM-DD` (`RICK_BUILD_DATE`).
    pub date: String,
    /// The toolchain, the `rustc -V` line (`RICK_TOOLCHAIN`).
    pub toolchain: String,
    /// The contributors' names, one per line (`RICK_CONTRIBUTORS`),
    /// embedded at build time.
    pub contributors: &'static str,
    /// The repository's `CONTRIBUTORS.md`, empty when there is none
    /// (`RICK_CONTRIBUTOR_ROLES`), embedded at build time.
    pub roles: &'static str,
    /// The release's `plugins/` folder beside the binary; a plugin loaded
    /// from anywhere else is third-party.
    pub plugins_dir: Option<PathBuf>,
    /// The crates of `third-party.json`, `None` in a build without the
    /// file.
    pub third_party: Option<Vec<Component>>,
}

/// One third-party component: a crate of the generated list, a bundled
/// runtime of `about/bundled.toml`, or an entry of a plugin's
/// `licenses.json`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Component {
    /// The crate's or the runtime's name.
    pub name: String,
    /// Its version.
    pub version: String,
    /// The SPDX licence id.
    pub license: String,
    /// The copyright holders: the crate's authors from Cargo's metadata
    /// (one string each), or the holder named by hand.
    #[serde(default, deserialize_with = "one_or_many")]
    pub copyright: Vec<String>,
    /// The repository or project URL (`url` in `bundled.toml`), empty when
    /// the crate names none.
    #[serde(default, alias = "url", deserialize_with = "null_as_empty")]
    pub repository: String,
    /// The full licence text.
    pub text: String,
}

impl Component {
    /// The copyright line to show: the first line of the licence text that
    /// says "Copyright", else the holders, emails left out.
    #[must_use]
    pub fn copyright_line(&self) -> String {
        if let Some(line) = self
            .text
            .lines()
            .map(|line| line.trim().trim_start_matches(['/', '#', '*']).trim())
            .find(|line| line.to_lowercase().starts_with("copyright"))
        {
            return line.to_owned();
        }
        self.copyright
            .iter()
            .map(|holder| match holder.split_once('<') {
                Some((name, _)) => name.trim().to_owned(),
                None => holder.trim().to_owned(),
            })
            .filter(|holder| !holder.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A string or an array of strings, as the generated file's `copyright`
/// holds the authors and a hand-written file holds one holder.
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

/// A string, or `null` for none, as cargo-about writes a crate without a
/// repository.
fn null_as_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

fn one_or_many<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    Ok(match Option::<OneOrMany>::deserialize(deserializer)? {
        None => Vec::new(),
        Some(OneOrMany::One(one)) => vec![one],
        Some(OneOrMany::Many(many)) => many,
    })
}

/// The crates of a `third-party.json` text, the workspace's own crates left
/// out (cargo-about lists them, their repository being Rick's; QUESTIONS.md
/// 261).
///
/// # Errors
///
/// The text is not the array of components `about-json.hbs` writes.
pub fn parse_third_party(json: &str) -> Result<Vec<Component>, serde_json::Error> {
    let mut list: Vec<Component> = serde_json::from_str(json)?;
    list.retain(|component| component.repository != REPOSITORY);
    Ok(list)
}

/// The generated component list from beside the binary — `exe`'s
/// directory, else `../Resources/` for an app bundle — `None` when neither
/// has it (a `cargo run`, a developer's bundle) or the file does not parse,
/// which is logged.
#[must_use]
pub fn load_third_party(exe: &Path) -> Option<Vec<Component>> {
    let dir = exe.parent()?;
    for path in [
        dir.join(THIRD_PARTY_FILE),
        dir.join("..").join("Resources").join(THIRD_PARTY_FILE),
    ] {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                return match parse_third_party(&text) {
                    Ok(list) => Some(list),
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error, "third-party.json did not parse");
                        None
                    }
                };
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "third-party.json could not be read");
                return None;
            }
        }
    }
    None
}

/// The bundled runtimes of `about/bundled.toml`: pdfium, GStreamer, 7-Zip
/// and the fonts. The file is the repository's own and a test parses it;
/// an unreadable one is logged and leaves the section empty.
#[must_use]
pub fn bundled() -> Vec<Component> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        component: Vec<Component>,
    }
    match toml::from_str::<File>(BUNDLED) {
        Ok(file) => file.component,
        Err(error) => {
            tracing::error!(%error, "about/bundled.toml did not parse");
            Vec::new()
        }
    }
}

/// The operating system and architecture of a target triple, as the build
/// line names them: `Linux x86_64`, `macOS aarch64`, `Windows x86_64`.
#[must_use]
pub fn os_arch(target: &str) -> (String, String) {
    let arch = target.split('-').next().unwrap_or(target).to_owned();
    let os = if target.contains("linux") {
        "Linux"
    } else if target.contains("darwin") || target.contains("apple") {
        "macOS"
    } else if target.contains("windows") {
        "Windows"
    } else {
        target.split('-').nth(2).unwrap_or("unknown")
    };
    (os.to_owned(), arch)
}

/// The build line: `<short commit> · <OS> <arch> · plugin ABI <n> · built
/// <YYYY-MM-DD>`.
#[must_use]
pub fn build_line(build: &Build) -> String {
    let (os, arch) = os_arch(&build.target);
    format!(
        "{} · {os} {arch} · plugin ABI {} · built {}",
        build.commit,
        ext::API_VERSION,
        build.date
    )
}

/// The status word of a plugin row, as `--list-plugins` prints it.
fn status_word(status: &PluginStatus) -> String {
    match status {
        PluginStatus::Loaded => "loaded".to_owned(),
        PluginStatus::Disabled => "disabled".to_owned(),
        PluginStatus::Refused(reason) => format!("refused: {reason}"),
    }
}

/// The plain text *Copy details* copies and `rick --version` prints: the
/// name and version, the build line, the toolchain, the OS and desktop,
/// then one `id version status` line per plugin.
#[must_use]
pub fn details(build: &Build, plugins: &[Plugin]) -> String {
    let mut text = format!(
        "Rick {}\n{}\n{}\n{}",
        build.version,
        build_line(build),
        build.toolchain,
        platform::os_description()
    );
    for plugin in plugins {
        // Writing into a String cannot fail.
        let _ = write!(
            text,
            "\n{} {} {}",
            plugin.id,
            plugin.version,
            status_word(&plugin.status)
        );
    }
    text
}

/// GitHub's new-issue URL with `details` as the issue body.
#[must_use]
pub fn issue_url(details: &str) -> String {
    format!(
        "{NEW_ISSUE}?body={}",
        utf8_percent_encode(details, NON_ALPHANUMERIC)
    )
}

/// The running version's section of the changelog: `## [<version>]` with
/// a pre-release suffix dropped, `## [Unreleased]` for a `-dev` build and
/// when the version has no section yet; up to the next `## ` heading.
#[must_use]
pub fn changelog_section(changelog: &str, version: &str) -> String {
    let heading = if version.ends_with("-dev") {
        "Unreleased"
    } else {
        version.split('-').next().unwrap_or(version)
    };
    section(changelog, heading)
        .or_else(|| section(changelog, "Unreleased"))
        .unwrap_or_default()
}

/// A Markdown text with its hard line breaks undone: the lines of a
/// paragraph or of a list item are joined, so the text view wraps them to
/// its own width; headings, list items, blank lines and the lines of a
/// fenced code block stay as they are.
#[must_use]
pub fn unwrap_markdown(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_code = false;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim_start().starts_with("```") {
            in_code = !in_code;
            out.push(trimmed.to_owned());
            continue;
        }
        let starts_block = trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with('|')
            || trimmed.starts_with('>')
            || trimmed
                .split_once(". ")
                .is_some_and(|(n, _)| n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty());
        let joins = !in_code
            && !starts_block
            && out.last().is_some_and(|last| {
                !last.is_empty() && !last.starts_with('#') && !last.starts_with("```")
            });
        if joins {
            if let Some(last) = out.last_mut() {
                last.push(' ');
                last.push_str(trimmed.trim_start());
            }
        } else {
            out.push(trimmed.to_owned());
        }
    }
    out.join("\n")
}

fn section(text: &str, heading: &str) -> Option<String> {
    let start = format!("## [{heading}]");
    let mut lines = text.lines();
    let first = lines.by_ref().find(|line| line.starts_with(&start))?;
    let mut out = first.to_owned();
    for line in lines {
        if line.starts_with("## ") {
            break;
        }
        out.push('\n');
        out.push_str(line);
    }
    Some(out.trim_end().to_owned())
}

/// A row of the credits card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credit {
    /// The name.
    pub name: String,
    /// The role line, when there is one.
    pub role: Option<String>,
}

/// The credits: the two fixed rows, then the contributors of `names` (one
/// per line) with the role `roles` (the text of `CONTRIBUTORS.md`) gives
/// them. A contributor whose first word is a fixed row's first word is the
/// same person and is not repeated (QUESTIONS.md 262).
#[must_use]
pub fn credits(names: &str, roles: &str) -> Vec<Credit> {
    let mut list = vec![
        Credit {
            name: "Serge".to_owned(),
            role: Some(t!("about-role-serge")),
        },
        Credit {
            name: "Claude (Anthropic)".to_owned(),
            role: Some(t!("about-role-claude")),
        },
    ];
    for name in names.lines().map(str::trim).filter(|name| !name.is_empty()) {
        let first = first_word(name);
        if list
            .iter()
            .any(|credit| first_word(&credit.name).eq_ignore_ascii_case(first))
        {
            continue;
        }
        list.push(Credit {
            name: name.to_owned(),
            role: role_of(name, roles),
        });
    }
    list
}

fn first_word(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or(name)
}

/// The initials for an avatar: the first letter of the first two words,
/// a parenthesised word left out — `S` for Serge, `C` for Claude
/// (Anthropic), `SB` for Serge Berwert.
#[must_use]
pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter(|word| !word.starts_with('('))
        .filter_map(|word| word.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// The role a `CONTRIBUTORS.md` line gives `name`: the line starts with
/// the name (after a list marker), and the rest after a dash, a colon or
/// a comma is the role.
fn role_of(name: &str, roles: &str) -> Option<String> {
    roles.lines().find_map(|line| {
        let line = line
            .trim()
            .trim_start_matches(|c: char| matches!(c, '-' | '*' | '|' | '.') || c.is_ascii_digit())
            .trim();
        let rest = line.strip_prefix(name)?;
        let role = rest
            .trim_start()
            .trim_start_matches(['—', '–', '-', ':', ',', '|'])
            .trim()
            .trim_end_matches('|')
            .trim();
        (!role.is_empty()).then(|| role.to_owned())
    })
}

/// Which page the dialog shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum Page {
    #[default]
    About,
    /// A read-only text: the changelog section or the licence.
    Text { title: String, text: String },
}

/// Where a click takes the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Go {
    /// Close the dialog.
    Close,
    /// Back to the About page.
    Back,
    /// Close and open the Extensions screen.
    Extensions,
}

/// What a frame's clicks asked for; applied once the modal is drawn.
#[derive(Debug, Default)]
struct Asked {
    go: Option<Go>,
    copy: bool,
    text: Option<Page>,
    open: Option<String>,
}

/// The About dialog's state between frames.
#[derive(Debug)]
pub struct AboutDialog {
    open: bool,
    page: Page,
    build: Build,
    bundled: Vec<Component>,
}

impl Default for AboutDialog {
    fn default() -> Self {
        Self {
            open: false,
            page: Page::About,
            build: Build {
                version: env!("CARGO_PKG_VERSION").to_owned(),
                commit: "unknown".to_owned(),
                target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
                date: "unknown".to_owned(),
                toolchain: "unknown".to_owned(),
                ..Build::default()
            },
            bundled: bundled(),
        }
    }
}

impl AboutDialog {
    /// Takes the binary's build details.
    pub fn set_build(&mut self, build: Build) {
        self.build = build;
    }

    /// The build details shown.
    #[must_use]
    pub fn build(&self) -> &Build {
        &self.build
    }

    /// Opens the dialog on the About page.
    pub fn open(&mut self) {
        self.open = true;
        self.page = Page::About;
    }

    /// Whether the dialog is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Whether a read-only text page is open: the title it shows.
    #[must_use]
    pub fn text_page(&self) -> Option<&str> {
        match &self.page {
            Page::Text { title, .. } => Some(title),
            Page::About => None,
        }
    }

    /// The number of third-party components the licence card counts: the
    /// bundled runtimes and the crates of the generated list, each crate
    /// once whatever the number of its licence texts; `None` in a build
    /// without the generated list.
    #[must_use]
    pub fn component_count(&self) -> Option<usize> {
        let crates = self.build.third_party.as_ref()?;
        let mut seen: std::collections::BTreeSet<(&str, &str)> = crates
            .iter()
            .map(|c| (c.name.as_str(), c.version.as_str()))
            .collect();
        seen.extend(
            self.bundled
                .iter()
                .map(|c| (c.name.as_str(), c.version.as_str())),
        );
        Some(seen.len())
    }

    /// The plugins the app found and how many of them came from outside
    /// the release's `plugins/` folder.
    fn plugin_counts(&self, plugins: &[Plugin]) -> (usize, usize) {
        let third = plugins
            .iter()
            .filter(|plugin| {
                self.build
                    .plugins_dir
                    .as_ref()
                    .is_some_and(|dir| !plugin.path.starts_with(dir))
            })
            .count();
        (plugins.len(), third)
    }

    /// Draws the dialog when open and acts on what was clicked.
    pub fn show(&mut self, ctx: &egui::Context, app: &App, requests: &mut Vec<Request>) {
        if !self.open {
            return;
        }
        let scheme = theme::scheme_of(ctx);
        let frame = egui::Frame::new()
            .fill(scheme.dialog_bg)
            .stroke(egui::Stroke::new(1.0, scheme.border_default))
            .corner_radius(RADIUS)
            .shadow(egui::epaint::Shadow {
                offset: [0, 8],
                blur: 32,
                spread: 0,
                color: scheme.shadow(),
            })
            .inner_margin(0);
        let mut asked = Asked::default();
        let plugins = app.plugins();
        let id = egui::Id::new("about-dialog");
        let modal = egui::Modal::new(id).frame(frame).show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            match &self.page {
                Page::About => self.about_page(ui, plugins, &mut asked),
                Page::Text { title, text } => text_page(ui, title, text, &mut asked),
            }
            // Escape, read as the other dialogues read it: a text page
            // goes back, the About page closes.
            if asked.go.is_none() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                asked.go = Some(if self.page == Page::About {
                    Go::Close
                } else {
                    Go::Back
                });
            }
        });
        if modal.backdrop_response.clicked() && asked.go.is_none() {
            asked.go = Some(Go::Close);
        }
        if asked.copy {
            let text = details(&self.build, plugins);
            match app.platform().set_text(&text) {
                Ok(()) => requests.push(Request::Toast(Level::Info, t!("about-details-copied"))),
                Err(error) => requests.push(Request::Toast(Level::Error, error.to_string())),
            }
        }
        if let Some(url) = asked.open {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
        if let Some(page) = asked.text {
            self.page = page;
        }
        match asked.go {
            Some(Go::Back) => self.page = Page::About,
            Some(Go::Extensions) => {
                requests.push(Request::Intent {
                    intent: Intent::Extensions,
                    pane: app.model().focused,
                });
                self.open = false;
                self.page = Page::About;
            }
            Some(Go::Close) => {
                self.open = false;
                self.page = Page::About;
            }
            None => {}
        }
    }

    /// The About page: title bar, hero, the three cards, the footer.
    fn about_page(&self, ui: &mut egui::Ui, plugins: &[Plugin], asked: &mut Asked) {
        let scheme = theme::scheme(ui);
        title_bar(ui, None, asked);
        self.hero(ui, asked);
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: PAD as i8,
                right: PAD as i8,
                top: 0,
                bottom: 22,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = vec2(0.0, 14.0);
                self.link_card(ui, plugins, asked);
                self.licence_card(ui, plugins, asked);
                self.credits_group(ui);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(t!("about-built-with"))
                            .size(theme::TEXT_SECONDARY)
                            .color(scheme.text_secondary),
                    )
                    .wrap()
                    .selectable(false),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(t!("about-copyright"))
                            .size(theme::TEXT_SECONDARY)
                            .color(scheme.text_placeholder),
                    )
                    .wrap()
                    .selectable(false),
                );
            });
    }

    /// The link card: Website, Source code, Report an issue, each opening
    /// the browser; the issue with the details as its body.
    fn link_card(&self, ui: &mut egui::Ui, plugins: &[Plugin], asked: &mut Asked) {
        card(ui, |ui| {
            let rows = [
                (
                    icons::LINK,
                    t!("about-website"),
                    Some(short_url(REPOSITORY)),
                    REPOSITORY,
                ),
                (
                    icons::CODE,
                    t!("about-source-code"),
                    Some(short_url(REPOSITORY)),
                    REPOSITORY,
                ),
                (icons::INFO, t!("about-report-an-issue"), None, NEW_ISSUE),
            ];
            let count = rows.len();
            for (index, (icon, label, trailing, url)) in rows.into_iter().enumerate() {
                if card_row(ui, index, count, icon, &label, trailing.as_deref()).clicked() {
                    asked.open = Some(if url == NEW_ISSUE {
                        issue_url(&details(&self.build, plugins))
                    } else {
                        url.to_owned()
                    });
                }
            }
        });
    }

    /// The licence card: the licence, the third-party component count and
    /// the plugins.
    fn licence_card(&self, ui: &mut egui::Ui, plugins: &[Plugin], asked: &mut Asked) {
        let third_party = match self.component_count() {
            Some(count) => t!("about-components", count = count as u64),
            None => t!("about-not-available"),
        };
        let (count, third) = self.plugin_counts(plugins);
        let plugins_line = t!(
            "about-plugins-summary",
            count = count as u64,
            third = third as u64
        );
        card(ui, |ui| {
            if card_row(
                ui,
                0,
                3,
                icons::DOCUMENT,
                &t!("about-licence"),
                Some("Apache-2.0"),
            )
            .clicked()
            {
                asked.text = Some(Page::Text {
                    title: t!("about-licence"),
                    text: LICENSE.to_owned(),
                });
            }
            if card_row(
                ui,
                1,
                3,
                icons::STAR,
                &t!("about-third-party-licences"),
                Some(&third_party),
            )
            .clicked()
            {
                // The Third-party licences page follows (issue 101, item 9).
            }
            if card_row(
                ui,
                2,
                3,
                icons::FOLDER,
                &t!("about-loaded-plugins"),
                Some(&plugins_line),
            )
            .clicked()
            {
                asked.go = Some(Go::Extensions);
            }
        });
    }

    /// The credits group: its title over the card of fixed rows and
    /// contributors.
    fn credits_group(&self, ui: &mut egui::Ui) {
        let scheme = theme::scheme(ui);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 8.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(t!("about-credits"))
                        .size(theme::TEXT_BODY)
                        .strong()
                        .color(scheme.text_secondary),
                )
                .selectable(false),
            );
            let credits = credits(self.build.contributors, self.build.roles);
            card(ui, |ui| {
                let count = credits.len();
                for (index, credit) in credits.iter().enumerate() {
                    credit_row(ui, index, count, credit);
                }
            });
        });
    }

    /// The hero: the symbol, the name, the tagline, the version pill, the
    /// build line and the two buttons, centred.
    fn hero(&self, ui: &mut egui::Ui, asked: &mut Asked) {
        let scheme = theme::scheme(ui);
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 32,
                right: 32,
                top: 10,
                bottom: 22,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = vec2(0.0, 10.0);
                ui.vertical_centered(|ui| {
                    match symbol(ui.ctx()) {
                        Some(texture) => {
                            ui.add(egui::Image::new((texture.id(), vec2(SYMBOL, SYMBOL))));
                        }
                        None => {
                            ui.allocate_space(vec2(SYMBOL, SYMBOL));
                        }
                    }
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(t!("rick"))
                                .size(TEXT_NAME)
                                .strong()
                                .color(scheme.text_primary),
                        )
                        .selectable(false),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(t!("about-tagline"))
                                .size(TEXT_TAGLINE)
                                .color(scheme.text_secondary),
                        )
                        .selectable(false),
                    );
                    widgets::pill(
                        ui,
                        &self.build.version,
                        scheme.bg_selected_soft,
                        scheme.text_on_selected,
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(build_line(&self.build))
                                .font(FontId::monospace(TEXT_BUILD))
                                .color(scheme.text_secondary),
                        )
                        .selectable(false),
                    );
                    ui.add_space(6.0);
                    let copy = t!("about-copy-details");
                    let whats_new = t!("about-whats-new");
                    let font = FontId::proportional(theme::TEXT_BODY);
                    let width = |text: &str, glyph: bool| {
                        ui.painter()
                            .layout_no_wrap(text.to_owned(), font.clone(), scheme.text_primary)
                            .size()
                            .x
                            + if glyph { theme::ICON + 8.0 } else { 0.0 }
                            + 32.0
                    };
                    let total = width(&copy, true) + 10.0 + width(&whats_new, false);
                    let (row, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
                    let mut buttons = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_size(
                                pos2(row.center().x - total / 2.0, row.top()),
                                vec2(total, 36.0),
                            ))
                            .layout(egui::Layout::left_to_right(Align::Center)),
                    );
                    buttons.spacing_mut().item_spacing.x = 10.0;
                    if widgets::icon_button(&mut buttons, Some(icons::COPY), &copy, false, true)
                        .clicked()
                    {
                        asked.copy = true;
                    }
                    if widgets::button(&mut buttons, &whats_new, false, true).clicked() {
                        asked.text = Some(Page::Text {
                            title: t!(
                                "about-whats-new-title",
                                version = self.build.version.clone()
                            ),
                            text: unwrap_markdown(&changelog_section(
                                CHANGELOG,
                                &self.build.version,
                            )),
                        });
                    }
                });
            });
    }
}

/// A read-only text page: the title bar with the back button and the
/// title, then the text in a scrolling mono box.
fn text_page(ui: &mut egui::Ui, title: &str, text: &str, asked: &mut Asked) {
    let scheme = theme::scheme(ui);
    title_bar(ui, Some(title), asked);
    let height = PAGE_HEIGHT - TITLE_BAR - PAD;
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: PAD as i8,
            right: PAD as i8,
            top: 0,
            bottom: PAD as i8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Frame::new()
                .fill(scheme.bg_card)
                .stroke(egui::Stroke::new(1.0, scheme.border_default))
                .corner_radius(CARD_RADIUS)
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_height(height - 20.0);
                    egui::ScrollArea::vertical()
                        .id_salt(("about-text", title))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(text)
                                        .font(FontId::monospace(TEXT_BUILD))
                                        .color(scheme.text_primary),
                                )
                                .wrap()
                                .selectable(true),
                            );
                        });
                });
        });
}

/// The title bar: a back button and the title when `title` is given, the
/// close button at the right.
fn title_bar(ui: &mut egui::Ui, title: Option<&str>, asked: &mut Asked) {
    let scheme = theme::scheme(ui);
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), TITLE_BAR), Sense::hover());
    let y = bar.top() + 14.0 + ROUND_BUTTON / 2.0;
    if let Some(title) = title {
        if round_button(
            ui,
            pos2(bar.left() + 16.0 + ROUND_BUTTON / 2.0, y),
            icons::BACK,
            &t!("about-back"),
        ) {
            asked.go = Some(Go::Back);
        }
        let x = bar.left() + 16.0 + ROUND_BUTTON + 10.0;
        ui.painter().text(
            pos2(x, y),
            Align2::LEFT_CENTER,
            title,
            FontId::proportional(TEXT_TITLE),
            scheme.text_primary,
        );
        let heading = Rect::from_min_max(
            pos2(x, y - 10.0),
            pos2(bar.right() - 16.0 - ROUND_BUTTON - 10.0, y + 10.0),
        );
        ui.interact(
            heading,
            ui.id().with(("about-title", title)),
            Sense::hover(),
        )
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title.to_owned()));
    }
    if round_button(
        ui,
        pos2(bar.right() - 16.0 - ROUND_BUTTON / 2.0, y),
        icons::CLOSE,
        &t!("close"),
    ) {
        asked.go = Some(Go::Close);
    }
}

/// A round button at `center`: `bg/subtle`, `bg/control` under the pointer,
/// a 16 px glyph; named `label`. Returns whether it was clicked.
fn round_button(ui: &mut egui::Ui, center: egui::Pos2, glyph: &str, label: &str) -> bool {
    let scheme = theme::scheme(ui);
    let rect = Rect::from_center_size(center, vec2(ROUND_BUTTON, ROUND_BUTTON));
    let response = ui.interact(rect, ui.id().with(("about-round", label)), Sense::click());
    ui.painter().circle_filled(
        center,
        ROUND_BUTTON / 2.0,
        if response.hovered() {
            scheme.bg_control
        } else {
            scheme.bg_subtle
        },
    );
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(theme::ICON),
        scheme.icon_default,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.to_owned())
    });
    response.clicked()
}

/// A card: `bg/card` with a `border/default` stroke and 10 px corners,
/// its rows drawn by `draw` with no spacing between them.
fn card(ui: &mut egui::Ui, draw: impl FnOnce(&mut egui::Ui)) {
    let scheme = theme::scheme(ui);
    egui::Frame::new()
        .fill(scheme.bg_card)
        .stroke(egui::Stroke::new(1.0, scheme.border_default))
        .corner_radius(CARD_RADIUS)
        .inner_margin(0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            draw(ui);
        });
}

/// The corners a row's hover fill rounds: the card's at the first and the
/// last row, so the fill stays inside the card.
fn row_corners(index: usize, count: usize) -> egui::CornerRadius {
    let r = CARD_RADIUS.saturating_sub(1);
    egui::CornerRadius {
        nw: if index == 0 { r } else { 0 },
        ne: if index == 0 { r } else { 0 },
        sw: if index + 1 == count { r } else { 0 },
        se: if index + 1 == count { r } else { 0 },
    }
}

/// A row of a card: a 16 px glyph, the label, a trailing value in
/// `text/secondary` and a chevron; a hairline above every row but the
/// first; `bg/subtle` under the pointer. Named `label` for the screen
/// reader.
fn card_row(
    ui: &mut egui::Ui,
    index: usize,
    count: usize,
    icon: &str,
    label: &str,
    trailing: Option<&str>,
) -> egui::Response {
    let scheme = theme::scheme(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let inner = Rect::from_min_max(
        pos2(rect.left() + 1.0, rect.top() + f32::from(index > 0)),
        pos2(
            rect.right() - 1.0,
            rect.bottom() - f32::from(index + 1 == count),
        ),
    );
    if response.hovered() {
        ui.painter()
            .rect_filled(inner, row_corners(index, count), scheme.bg_subtle);
    }
    if index > 0 {
        ui.painter().hline(
            (rect.left() + 1.0)..=(rect.right() - 1.0),
            rect.top() + 0.5,
            scheme.hairline(),
        );
    }
    ui.painter().text(
        pos2(rect.left() + 16.0 + theme::ICON / 2.0, rect.center().y),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(theme::ICON),
        scheme.icon_default,
    );
    ui.painter().text(
        pos2(rect.left() + 46.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(theme::TEXT_BODY),
        scheme.text_primary,
    );
    ui.painter().text(
        pos2(rect.right() - 16.0 - theme::ICON / 2.0, rect.center().y),
        Align2::CENTER_CENTER,
        icons::SEGMENT,
        FontId::proportional(theme::ICON),
        scheme.icon_default,
    );
    if let Some(trailing) = trailing {
        let galley = ui.painter().layout_no_wrap(
            trailing.to_owned(),
            FontId::proportional(theme::TEXT_SECONDARY),
            scheme.text_secondary,
        );
        let size = galley.size();
        let text_rect = Rect::from_min_size(
            pos2(rect.right() - 46.0 - size.x, rect.center().y - size.y / 2.0),
            size,
        );
        ui.painter()
            .galley(text_rect.min, galley, scheme.text_secondary);
        // Its own node, so the value is read aloud after the label.
        ui.interact(
            text_rect,
            ui.id().with(("about-trailing", label)),
            Sense::hover(),
        )
        .widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, trailing.to_owned())
        });
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.to_owned())
    });
    response
}

/// A row of the credits card: the initials in a `bg/control` circle, the
/// name and the role line.
fn credit_row(ui: &mut egui::Ui, index: usize, count: usize, credit: &Credit) {
    let scheme = theme::scheme(ui);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), CREDIT_ROW), Sense::hover());
    let _ = count;
    if index > 0 {
        ui.painter().hline(
            (rect.left() + 1.0)..=(rect.right() - 1.0),
            rect.top() + 0.5,
            scheme.hairline(),
        );
    }
    let centre = pos2(rect.left() + 16.0 + AVATAR / 2.0, rect.center().y);
    ui.painter()
        .circle_filled(centre, AVATAR / 2.0, scheme.bg_control);
    ui.painter().text(
        centre,
        Align2::CENTER_CENTER,
        initials(&credit.name),
        FontId::proportional(TEXT_TAGLINE),
        scheme.text_primary,
    );
    let x = rect.left() + 16.0 + AVATAR + 14.0;
    match &credit.role {
        Some(role) => {
            ui.painter().text(
                pos2(x, rect.center().y - 9.0),
                Align2::LEFT_CENTER,
                &credit.name,
                FontId::proportional(theme::TEXT_BODY),
                scheme.text_primary,
            );
            ui.painter().text(
                pos2(x, rect.center().y + 9.0),
                Align2::LEFT_CENTER,
                role,
                FontId::proportional(theme::TEXT_SECONDARY),
                scheme.text_secondary,
            );
        }
        None => {
            ui.painter().text(
                pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                &credit.name,
                FontId::proportional(theme::TEXT_BODY),
                scheme.text_primary,
            );
        }
    }
    let spoken = match &credit.role {
        Some(role) => format!("{}, {role}", credit.name),
        None => credit.name.clone(),
    };
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, spoken.clone()));
}

/// A URL without its scheme, as the link rows show it.
fn short_url(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned()
}

/// The symbol as a texture of [`SYMBOL`] points at the context's scale,
/// rasterised once per scale and kept in the context; `None` when the SVG
/// does not parse or the pixmap cannot be made, which is said once and
/// leaves the hero without the picture.
fn symbol(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let scale = ctx.pixels_per_point();
    let key = egui::Id::new(("about-symbol", scale.to_bits()));
    if let Some(texture) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return Some(texture);
    }
    let tree = match resvg::usvg::Tree::from_data(SYMBOL_SVG, &resvg::usvg::Options::default()) {
        Ok(tree) => tree,
        Err(error) => {
            tracing::warn!(%error, "the About symbol did not parse");
            return None;
        }
    };
    let edge = (SYMBOL * scale).round().max(1.0);
    let side = edge as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side)?;
    let intrinsic = tree.size();
    let transform = resvg::tiny_skia::Transform::from_scale(
        edge / intrinsic.width(),
        edge / intrinsic.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let image =
        egui::ColorImage::from_rgba_premultiplied([side as usize, side as usize], pixmap.data());
    let texture = ctx.load_texture(
        format!("about-symbol-{scale}"),
        image,
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(key, texture.clone()));
    Some(texture)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build() -> Build {
        Build {
            version: "0.3.0-dev".to_owned(),
            commit: "a1b2c3d".to_owned(),
            target: "x86_64-unknown-linux-gnu".to_owned(),
            date: "2026-11-09".to_owned(),
            toolchain: "rustc 1.97.0 (test)".to_owned(),
            contributors: "Claude\nSerge\nSerge Berwert\nAda Lovelace\n",
            roles: "# Contributors\n\n- Ada Lovelace — Analytical engines\n- Serge Berwert: everything\n",
            plugins_dir: Some(PathBuf::from("/opt/rick/plugins")),
            third_party: None,
        }
    }

    #[test]
    fn the_build_line_names_the_os_arch_abi_and_date() {
        assert_eq!(
            os_arch("x86_64-unknown-linux-gnu"),
            ("Linux".to_owned(), "x86_64".to_owned())
        );
        assert_eq!(
            os_arch("aarch64-apple-darwin"),
            ("macOS".to_owned(), "aarch64".to_owned())
        );
        assert_eq!(
            os_arch("x86_64-pc-windows-msvc"),
            ("Windows".to_owned(), "x86_64".to_owned())
        );
        assert_eq!(
            build_line(&build()),
            format!(
                "a1b2c3d · Linux x86_64 · plugin ABI {} · built 2026-11-09",
                ext::API_VERSION
            )
        );
    }

    #[test]
    fn the_details_are_the_name_the_lines_and_the_plugins() {
        let plugins = vec![
            Plugin {
                id: "git".to_owned(),
                name: "Git".to_owned(),
                version: "0.3.0-dev".to_owned(),
                points: ext::Points::empty(),
                previews: Vec::new(),
                status: PluginStatus::Loaded,
                path: PathBuf::from("/opt/rick/plugins/git"),
            },
            Plugin {
                id: "probe".to_owned(),
                name: "Probe".to_owned(),
                version: "1.0.0".to_owned(),
                points: ext::Points::empty(),
                previews: Vec::new(),
                status: PluginStatus::Refused("toolchain".to_owned()),
                path: PathBuf::from("/home/me/.config/rick/plugins/probe"),
            },
        ];
        let text = details(&build(), &plugins);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Rick 0.3.0-dev");
        assert_eq!(lines[1], build_line(&build()));
        assert_eq!(lines[2], "rustc 1.97.0 (test)");
        assert_ne!(lines[3], "", "the OS line");
        assert_eq!(lines[4], "git 0.3.0-dev loaded");
        assert_eq!(lines[5], "probe 1.0.0 refused: toolchain");
        assert_eq!(lines.len(), 6);
        let url = issue_url("a b\nc");
        assert_eq!(url, format!("{NEW_ISSUE}?body=a%20b%0Ac"));
        let dialog = AboutDialog {
            build: build(),
            ..AboutDialog::default()
        };
        assert_eq!(dialog.plugin_counts(&plugins), (2, 1));
    }

    #[test]
    fn the_changelog_section_is_the_versions_or_unreleased() {
        let changelog = "# Changelog\n\n## [Unreleased]\n\n- soon\n\n## [0.2.0] - 2026-11-08\n\n### Added\n\n- a thing\n\n## [0.1.0] - 2026-11-06\n\n- first\n";
        assert_eq!(
            changelog_section(changelog, "0.2.0"),
            "## [0.2.0] - 2026-11-08\n\n### Added\n\n- a thing"
        );
        assert_eq!(
            changelog_section(changelog, "0.2.0-rc1"),
            "## [0.2.0] - 2026-11-08\n\n### Added\n\n- a thing"
        );
        assert_eq!(
            changelog_section(changelog, "0.3.0-dev"),
            "## [Unreleased]\n\n- soon"
        );
        assert_eq!(
            changelog_section(changelog, "9.9.9"),
            "## [Unreleased]\n\n- soon"
        );
        assert_eq!(changelog_section("nothing", "1.0.0"), "");
        let wrapped = "## [Unreleased]\n\n### Added\n\n- A long item that\n  goes on, and on\n  (#91).\n- Short one.\n\nA paragraph\nin two lines.\n\n```text\nkeep\n  these\n```\n1. first\n   step\n";
        assert_eq!(
            unwrap_markdown(wrapped),
            "## [Unreleased]\n\n### Added\n\n- A long item that goes on, and on (#91).\n- Short one.\n\nA paragraph in two lines.\n\n```text\nkeep\n  these\n```\n1. first step"
        );
        // The repository's own changelog has the Unreleased section.
        assert!(changelog_section(CHANGELOG, "0.3.0-dev").starts_with("## [Unreleased]"));
        assert!(LICENSE.contains("Apache License"));
    }

    #[test]
    fn the_credits_are_the_fixed_rows_then_the_contributors_with_their_roles() {
        let list = credits(build().contributors, build().roles);
        let names: Vec<&str> = list.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Serge", "Claude (Anthropic)", "Ada Lovelace"]);
        assert_eq!(
            list[0].role.as_deref(),
            Some(t!("about-role-serge").as_str())
        );
        assert_eq!(list[2].role.as_deref(), Some("Analytical engines"));
        assert_eq!(
            credits("Grace Hopper\n", "")
                .last()
                .and_then(|c| c.role.clone()),
            None
        );
        assert_eq!(
            role_of("Serge Berwert", build().roles).as_deref(),
            Some("everything")
        );
        assert_eq!(initials("Serge"), "S");
        assert_eq!(initials("Claude (Anthropic)"), "C");
        assert_eq!(initials("Serge Berwert"), "SB");
        assert_eq!(initials("ada lovelace byron"), "AL");
    }

    #[test]
    fn the_component_lists_parse_and_leave_ricks_own_crates_out() {
        let json = format!(
            r#"[
              {{"name": "egui", "version": "0.36.2", "license": "Apache-2.0", "copyright": ["Emil Ernerfeldt <emil@example.com>"], "repository": "https://github.com/emilk/egui", "text": "Apache License\nVersion 2.0"}},
              {{"name": "app", "version": "0.3.0-dev", "license": "Apache-2.0", "copyright": [], "repository": "{REPOSITORY}", "text": "Apache License"}},
              {{"name": "mit-thing", "version": "1.0.0", "license": "MIT", "copyright": "Someone", "repository": null, "text": "MIT License\n\nCopyright (c) 2020 Someone\n\nPermission is hereby granted"}}
            ]"#
        );
        let list = parse_third_party(&json).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(list.len(), 2, "the workspace crate is left out");
        assert_eq!(list[0].copyright_line(), "Emil Ernerfeldt");
        assert_eq!(list[1].copyright, vec!["Someone".to_owned()]);
        assert_eq!(list[1].copyright_line(), "Copyright (c) 2020 Someone");
        assert_eq!(list[1].repository, "");
        assert!(parse_third_party("{}").is_err());

        let bundled = bundled();
        let names: Vec<&str> = bundled.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "pdfium",
                "GStreamer",
                "7-Zip",
                "Ubuntu Light",
                "Hack",
                "Noto Emoji",
                "emoji-icon-font",
                "Phosphor Icons"
            ]
        );
        for component in &bundled {
            assert_ne!(component.version, "", "{}", component.name);
            assert_ne!(component.license, "", "{}", component.name);
            assert!(
                component.repository.starts_with("https://"),
                "{}",
                component.name
            );
            assert!(component.text.len() > 500, "{}", component.name);
        }

        let mut dialog = AboutDialog::default();
        assert_eq!(dialog.component_count(), None);
        dialog.set_build(Build {
            third_party: Some(list),
            ..build()
        });
        assert_eq!(dialog.component_count(), Some(2 + 8));
    }

    #[test]
    fn a_missing_file_beside_the_binary_is_none_and_a_present_one_parses() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let exe = dir.path().join("rick");
        assert_eq!(load_third_party(&exe), None);
        std::fs::write(
            dir.path().join(THIRD_PARTY_FILE),
            r#"[{"name": "x", "version": "1", "license": "MIT", "copyright": [], "repository": "r", "text": "t"}]"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load_third_party(&exe).map(|l| l.len()), Some(1));
        // An app bundle: Contents/MacOS/rick with Contents/Resources/.
        let bundle = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let macos = bundle.path().join("Contents").join("MacOS");
        let resources = bundle.path().join("Contents").join("Resources");
        std::fs::create_dir_all(&macos).unwrap_or_else(|e| panic!("{e}"));
        std::fs::create_dir_all(&resources).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(resources.join(THIRD_PARTY_FILE), "[]").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            load_third_party(&macos.join("rick")).map(|l| l.len()),
            Some(0)
        );
        std::fs::write(resources.join(THIRD_PARTY_FILE), "not json")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load_third_party(&macos.join("rick")), None);
    }

    /// The hero draws the rings-only symbol at 96 px, rasterised at that
    /// size times the window's scale, once per scale.
    #[test]
    fn the_symbol_is_rasterised_at_96_px_once_per_scale() {
        let mut harness = egui_kittest::Harness::builder()
            .with_pixels_per_point(2.0)
            .build_ui(|ui| {
                symbol(ui.ctx());
            });
        harness.run();
        let texture =
            symbol(&harness.ctx).unwrap_or_else(|| panic!("the symbol did not rasterise"));
        assert_eq!(texture.size(), [192, 192]);
        assert_eq!(
            symbol(&harness.ctx).map(|t| t.id()),
            Some(texture.id()),
            "once per scale"
        );
    }
}
