//! The settings snapshot: what `rick.toml` holds, as data.
//!
//! Only settings the specification names appear here. The TOML file itself
//! is read and written by `app`; the model only carries the values.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use types::{Location, ProviderId};

use crate::listing::{Column, SortDir};

/// Light or dark, or whatever the system says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// Follow the system.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// Row density of lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    /// Tight rows.
    Compact,
    /// The default.
    #[default]
    Normal,
    /// Roomy rows.
    Comfortable,
}

/// Everything in `rick.toml`. Missing keys take their defaults, so a file
/// with one line is valid.
// Settings are flags by nature; each maps to one TOML key.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Keep all folders above all files whatever the sort column.
    pub folders_first: bool,
    /// Show dot-files and entries with the hidden attribute.
    pub show_hidden: bool,
    /// Show entries with the system attribute.
    pub show_system: bool,
    /// Show file extensions in the name column.
    pub show_extensions: bool,
    /// Alternate the row background colour.
    pub zebra_stripes: bool,
    /// Default sort column for new tabs.
    pub sort_by: Column,
    /// Default sort direction for new tabs.
    pub sort_dir: SortDir,
    /// Light, dark or system.
    pub theme: Theme,
    /// UI font size in points.
    pub font_size: u8,
    /// Row density.
    pub density: Density,
    /// Accent colour as `#rrggbb`, or the theme's own.
    pub accent: Option<String>,
    /// The language of the user interface as a tag (`de`), or `None` for
    /// the system's; applied live, English the fallback for a missing key.
    pub language: Option<String>,
    /// Rebound shortcuts: action id to shortcut text, e.g. `"core.copy" =
    /// "Mod+C"`. Actions not listed keep their default.
    pub shortcuts: BTreeMap<String, String>,
    /// What Rick opens with: the last session or a named layout.
    pub session: SessionSettings,
    /// Folders and files pinned to the side panel, in display order.
    /// Written as `provider:path` strings; a bare path is a local one.
    #[serde(with = "pinned_toml")]
    pub pinned: Vec<Location>,
    /// Drag and drop: the spring-loading delay and where a pane is left
    /// after a drag.
    pub drag: Drag,
    /// The preview pane: text defaults and the pinned info section per
    /// content kind.
    pub preview: Preview,
    /// Search in subdirectories: the names the walk skips.
    pub search: Search,
    /// The viewer: the types that open with the system application instead.
    pub viewer: Viewer,
    /// Plugins: which of them not to load.
    pub plugins: Plugins,
    /// The network and cloud connections, one `[[connections]]` table
    /// each: the provider, the name and the non-secret fields. The
    /// secrets are in the platform keychain, never here.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionEntry>,
}

/// One `[[connections]]` table of `rick.toml`: a connection of a network
/// or cloud provider plugin with its non-secret fields inline, so a file
/// reads `provider = "s3"`, `name = "work"`, `endpoint = "…"`. A password
/// or key lives in the platform keychain under the plugin's key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ConnectionEntry {
    /// The provider id: `s3`, `azure`, `gcs`, `ssh`.
    pub provider: String,
    /// The name the user gave the connection; unique within the provider.
    pub name: String,
    /// The non-secret fields by key, inline in the table.
    #[serde(flatten)]
    pub settings: BTreeMap<String, String>,
}

impl ConnectionEntry {
    /// The entry as the extension API hands it to the provider's
    /// connector: the name and the fields.
    #[must_use]
    pub fn to_connection(&self) -> ext::Connection {
        ext::Connection {
            name: self.name.clone(),
            settings: self.settings.clone(),
        }
    }
}

/// The `[viewer]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Viewer {
    /// File name extensions, lower case without the dot, that Open hands to
    /// the system's default application instead of the viewer, previewable
    /// or not: the setting that flips the viewer's default per type.
    pub external: Vec<String>,
}

impl Viewer {
    /// Whether files named `name` open outside the viewer.
    #[must_use]
    pub fn opens_externally(&self, name: &str) -> bool {
        let Some((stem, ext)) = name.rsplit_once('.') else {
            return false;
        };
        !stem.is_empty()
            && !ext.is_empty()
            && self
                .external
                .iter()
                .any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(ext))
    }
}

/// The `[plugins]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Plugins {
    /// Ids of the plugins not to load on the next start; they stay on disk
    /// and the Extensions screen lists them as disabled.
    pub disabled: Vec<String>,
}

/// The `[search]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Search {
    /// Directory and file names the subtree search never descends into or
    /// reports.
    pub ignore: Vec<String>,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            ignore: vec![".git".into(), "node_modules".into(), "target".into()],
        }
    }
}

/// The `[drag]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Drag {
    /// How long a folder row, tab header or breadcrumb segment is hovered
    /// before the pane spring-loads into it, in milliseconds.
    pub spring_delay_ms: u64,
    /// Where a pane that spring-loaded during a drag is left when the drag
    /// ends.
    pub after: AfterDrag,
}

impl Default for Drag {
    fn default() -> Self {
        Self {
            spring_delay_ms: 1000,
            after: AfterDrag::Stay,
        }
    }
}

/// *After a drag: stay in the folder / return to where the drag started.*
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AfterDrag {
    /// The pane stays where it drilled to.
    #[default]
    Stay,
    /// The pane goes back to where the drag started.
    Return,
}

/// Pinned locations in the settings file: `"local:/home/me"`. A string
/// without a provider prefix is read as a local path, which is how older
/// files wrote them.
mod pinned_toml {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use types::Location;

    pub(super) fn serialize<S: Serializer>(
        pinned: &[Location],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let texts: Vec<String> = pinned.iter().map(ToString::to_string).collect();
        texts.serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Location>, D::Error> {
        let texts: Vec<String> = Vec::deserialize(deserializer)?;
        Ok(texts
            .iter()
            .map(|text| super::parse_location(text))
            .collect())
    }
}

/// `provider:path`, or a bare local path. A Windows drive letter is a path,
/// not a provider: the provider part never has a separator and is never one
/// letter long.
#[must_use]
pub fn parse_location(text: &str) -> Location {
    match text.split_once(':') {
        Some((provider, path))
            if provider.len() > 1
                && provider
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') =>
        {
            Location::new(ProviderId::intern(provider), path)
        }
        _ => Location::new(ProviderId("local"), text),
    }
}

/// The preview pane's persisted state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Preview {
    /// Text and code defaults.
    pub text: TextPreview,
    /// The pinned info section's id by content kind (`"text"`, `"image"`,
    /// `"pages"`); a kind without an entry has nothing pinned.
    pub pinned: BTreeMap<String, String>,
}

/// How text and code open in the preview pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextPreview {
    /// Line numbers in the gutter.
    pub line_numbers: bool,
    /// Long lines break at the pane width.
    pub wrap: bool,
}

impl Default for TextPreview {
    fn default() -> Self {
        Self {
            line_numbers: true,
            wrap: false,
        }
    }
}

/// Which edge of the window the side panel docks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PanelSide {
    /// The left edge, the default.
    #[default]
    Left,
    /// The right edge.
    Right,
}

/// The side panel's titled sections, top to bottom, below the Places list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    /// Every drive of the machine, fixed and removable.
    Machine,
    /// Mounted network file systems and connections.
    Network,
    /// The cloud providers' accounts and containers.
    Cloud,
}

impl Section {
    /// Every section in display order.
    pub const ALL: [Self; 3] = [Self::Machine, Self::Network, Self::Cloud];

    /// The section's title.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Machine => "This Machine",
            Self::Network => "Network",
            Self::Cloud => "Cloud",
        }
    }

    /// The section's name in the session file.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Machine => "machine",
            Self::Network => "network",
            Self::Cloud => "cloud",
        }
    }
}

/// The folded sections of a session file: names that are no section any
/// more (0.1's `recent`, `trash`, `pinned`, `tree`) are dropped instead of
/// failing the whole file.
fn lenient_sections<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeSet<Section>, D::Error> {
    let names: Vec<String> = Vec::deserialize(deserializer)?;
    Ok(names
        .iter()
        .filter_map(|name| Section::ALL.into_iter().find(|s| s.name() == name))
        .collect())
}

/// The side panel of one window: shown or hidden, which side, its width
/// and which of its sections are folded. Part of the session, not of the
/// settings file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SidePanel {
    /// Shown at all.
    pub visible: bool,
    /// Left or right.
    pub side: PanelSide,
    /// Width in logical points.
    pub width: u16,
    /// Sections folded shut.
    #[serde(deserialize_with = "lenient_sections")]
    pub folded: BTreeSet<Section>,
}

impl Default for SidePanel {
    fn default() -> Self {
        Self {
            visible: true,
            side: PanelSide::Left,
            width: 220,
            folded: BTreeSet::new(),
        }
    }
}

/// One change to a window's side panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidePanelChange {
    /// Show or hide the panel.
    Visible(bool),
    /// Dock it left or right.
    Side(PanelSide),
    /// Its width in logical points.
    Width(u16),
    /// Fold or unfold one section.
    Fold {
        /// Which section.
        section: Section,
        /// Folded shut, or open.
        folded: bool,
    },
}

impl SidePanel {
    /// Applies one change.
    pub fn apply(&mut self, change: SidePanelChange) {
        match change {
            SidePanelChange::Visible(v) => self.visible = v,
            SidePanelChange::Side(v) => self.side = v,
            SidePanelChange::Width(v) => self.width = v,
            SidePanelChange::Fold { section, folded } => {
                if folded {
                    self.folded.insert(section);
                } else {
                    self.folded.remove(&section);
                }
            }
        }
    }
}

/// The value of `[session] start` that means the last session.
pub const LAST_SESSION: &str = "last";

/// The `[session]` table: what Rick opens with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionSettings {
    /// `"last"` for the last session, otherwise the name of a saved layout.
    pub start: String,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            start: LAST_SESSION.to_owned(),
        }
    }
}

impl SessionSettings {
    /// The start layout's name, or `None` for the last session.
    #[must_use]
    pub fn start_layout(&self) -> Option<&str> {
        (self.start != LAST_SESSION).then_some(self.start.as_str())
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            folders_first: true,
            show_hidden: false,
            show_system: false,
            show_extensions: true,
            zebra_stripes: false,
            sort_by: Column::Name,
            sort_dir: SortDir::Ascending,
            theme: Theme::System,
            font_size: 13,
            density: Density::Normal,
            accent: None,
            shortcuts: BTreeMap::new(),
            session: SessionSettings::default(),
            pinned: Vec::new(),
            drag: Drag::default(),
            preview: Preview::default(),
            search: Search::default(),
            viewer: Viewer::default(),
            plugins: Plugins::default(),
            connections: Vec::new(),
            language: None,
        }
    }
}

/// One change to the settings, as the Settings screen makes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsChange {
    /// Folders above files.
    FoldersFirst(bool),
    /// Hidden entries shown.
    ShowHidden(bool),
    /// System entries shown.
    ShowSystem(bool),
    /// Extensions shown in the name column.
    ShowExtensions(bool),
    /// Zebra striping.
    ZebraStripes(bool),
    /// Default sort for new tabs.
    Sort(Column, SortDir),
    /// Theme.
    Theme(Theme),
    /// Font size in points.
    FontSize(u8),
    /// Row density.
    Density(Density),
    /// Accent colour, `None` for the theme's own.
    Accent(Option<String>),
    /// The language, `None` for the system's.
    Language(Option<String>),
    /// Rebind an action; `None` restores its default.
    Shortcut {
        /// The action id.
        action: String,
        /// The new binding as text, or `None`.
        binding: Option<String>,
    },
    /// What Rick opens with: `None` for the last session, or a layout name.
    StartLayout(Option<String>),
    /// Pin locations to the side panel at `at`, or at the end; a location
    /// already pinned stays where it is.
    Pin {
        /// What to pin, in order.
        items: Vec<Location>,
        /// The index to insert at; `None` appends.
        at: Option<usize>,
    },
    /// Unpin a location.
    Unpin(Location),
    /// The spring-loading delay in milliseconds.
    SpringDelay(u64),
    /// Where a pane is left after a drag.
    AfterDrag(AfterDrag),
    /// Line numbers in text previews, the new default.
    PreviewLineNumbers(bool),
    /// Word wrap in text previews, the new default.
    PreviewWrap(bool),
    /// Pin an info section for a content kind, or unpin with `None`.
    PreviewPin {
        /// The content kind: `"text"`, `"image"`, `"pages"`.
        kind: String,
        /// The section id to pin, or nothing.
        section: Option<String>,
    },
    /// The names the subtree search skips.
    SearchIgnore(Vec<String>),
    /// The file name extensions Open hands to the system application
    /// instead of the viewer.
    ViewerExternal(Vec<String>),
    /// Enable or disable a plugin; takes effect on the next start.
    Plugin {
        /// The plugin id.
        id: String,
        /// `false` adds it to `[plugins] disabled`, `true` removes it.
        enabled: bool,
    },
}

impl Settings {
    /// Applies one change.
    pub fn apply(&mut self, change: SettingsChange) {
        match change {
            SettingsChange::FoldersFirst(v) => self.folders_first = v,
            SettingsChange::ShowHidden(v) => self.show_hidden = v,
            SettingsChange::ShowSystem(v) => self.show_system = v,
            SettingsChange::ShowExtensions(v) => self.show_extensions = v,
            SettingsChange::ZebraStripes(v) => self.zebra_stripes = v,
            SettingsChange::Sort(by, dir) => {
                self.sort_by = by;
                self.sort_dir = dir;
            }
            SettingsChange::Theme(v) => self.theme = v,
            SettingsChange::FontSize(v) => self.font_size = v,
            SettingsChange::Density(v) => self.density = v,
            SettingsChange::Language(v) => self.language = v,
            SettingsChange::Accent(v) => self.accent = v,
            SettingsChange::Shortcut { action, binding } => match binding {
                Some(binding) => {
                    self.shortcuts.insert(action, binding);
                }
                None => {
                    self.shortcuts.remove(&action);
                }
            },
            SettingsChange::StartLayout(layout) => {
                self.session.start = layout.unwrap_or_else(|| LAST_SESSION.to_owned());
            }
            SettingsChange::Pin { items, at } => {
                let mut index = at.map_or(self.pinned.len(), |at| at.min(self.pinned.len()));
                for item in items {
                    if !self.pinned.contains(&item) {
                        self.pinned.insert(index, item);
                        index += 1;
                    }
                }
            }
            SettingsChange::Unpin(item) => self.pinned.retain(|p| *p != item),
            SettingsChange::SpringDelay(ms) => self.drag.spring_delay_ms = ms,
            SettingsChange::AfterDrag(after) => self.drag.after = after,
            SettingsChange::PreviewLineNumbers(v) => self.preview.text.line_numbers = v,
            SettingsChange::PreviewWrap(v) => self.preview.text.wrap = v,
            SettingsChange::PreviewPin { kind, section } => match section {
                Some(section) => {
                    self.preview.pinned.insert(kind, section);
                }
                None => {
                    self.preview.pinned.remove(&kind);
                }
            },
            SettingsChange::SearchIgnore(names) => self.search.ignore = names,
            SettingsChange::ViewerExternal(types) => {
                self.viewer.external = types
                    .into_iter()
                    .map(|t| t.trim().trim_start_matches('.').to_ascii_lowercase())
                    .filter(|t| !t.is_empty())
                    .collect();
            }
            SettingsChange::Plugin { id, enabled } => {
                self.plugins.disabled.retain(|d| *d != id);
                if !enabled {
                    self.plugins.disabled.push(id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_spec() {
        let s = Settings::default();
        assert!(s.folders_first);
        assert!(!s.show_hidden);
        assert_eq!(s.sort_by, Column::Name);
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.drag.spring_delay_ms, 1000);
        assert_eq!(s.drag.after, AfterDrag::Stay);
        assert!(s.preview.text.line_numbers, "line numbers on");
        assert!(!s.preview.text.wrap, "wrap off");
        assert!(s.preview.pinned.is_empty());
        assert_eq!(s.search.ignore, [".git", "node_modules", "target"]);
        assert!(
            s.viewer.external.is_empty(),
            "every type opens in the viewer"
        );
        let mut s = s;
        s.apply(SettingsChange::SearchIgnore(vec!["build".into()]));
        assert_eq!(s.search.ignore, ["build"]);
        s.apply(SettingsChange::ViewerExternal(vec![
            " .PDF ".into(),
            "docx".into(),
            String::new(),
        ]));
        assert_eq!(s.viewer.external, ["pdf", "docx"]);
        assert!(s.viewer.opens_externally("paper.pdf"));
        assert!(s.viewer.opens_externally("Paper.PDF"));
        assert!(!s.viewer.opens_externally("paper.png"));
        assert!(
            !s.viewer.opens_externally(".pdf"),
            "a dot-file has no extension"
        );
        assert!(!s.viewer.opens_externally("pdf"));
    }

    #[test]
    fn connections_round_trip_as_tables_with_their_fields_inline() {
        let mut s = Settings::default();
        s.connections.push(ConnectionEntry {
            provider: "s3".into(),
            name: "work".into(),
            settings: BTreeMap::from([
                ("endpoint".to_owned(), "http://127.0.0.1:9000".to_owned()),
                ("access_key".to_owned(), "minio".to_owned()),
            ]),
        });
        let text = toml::to_string(&s).unwrap_or_else(|e| panic!("{e}"));
        assert!(text.contains("[[connections]]"), "{text}");
        assert!(text.contains("provider = \"s3\""), "{text}");
        assert!(
            text.contains("endpoint = \"http://127.0.0.1:9000\""),
            "{text}"
        );
        let back: Settings = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, s);
        assert_eq!(
            back.connections[0].to_connection().get("access_key"),
            Some("minio")
        );
        assert_eq!(back.connections[0].to_connection().get("missing"), None);
        // No connection, no table.
        let text = toml::to_string(&Settings::default()).unwrap_or_else(|e| panic!("{e}"));
        assert!(!text.contains("connections"), "{text}");
    }

    #[test]
    fn pinned_locations_parse_with_and_without_a_provider() {
        assert_eq!(
            parse_location("local:/home/me"),
            Location::new(ProviderId("local"), "/home/me")
        );
        assert_eq!(
            parse_location("/home/me"),
            Location::new(ProviderId("local"), "/home/me")
        );
        assert_eq!(
            parse_location("C:\\Users\\me"),
            Location::new(ProviderId("local"), "C:\\Users\\me")
        );
        assert_eq!(
            parse_location("ssh:/srv"),
            Location::new(ProviderId::intern("ssh"), "/srv")
        );
        let text = Location::new(ProviderId("local"), "C:\\x").to_string();
        assert_eq!(parse_location(&text).path.to_string(), "C:\\x");
    }

    #[test]
    fn side_panel_changes_apply() {
        let mut p = SidePanel::default();
        p.apply(SidePanelChange::Fold {
            section: Section::Network,
            folded: true,
        });
        p.apply(SidePanelChange::Side(PanelSide::Right));
        p.apply(SidePanelChange::Width(300));
        p.apply(SidePanelChange::Visible(false));
        assert!(p.folded.contains(&Section::Network));
        assert_eq!(p.side, PanelSide::Right);
        assert_eq!(p.width, 300);
        assert!(!p.visible);
        p.apply(SidePanelChange::Fold {
            section: Section::Network,
            folded: false,
        });
        assert!(p.folded.is_empty());
    }

    #[test]
    fn a_session_file_with_old_section_names_still_reads() {
        let text = "visible = true\nside = \"left\"\nwidth = 220\nfolded = [\"pinned\", \"machine\", \"tree\"]\n";
        let panel: SidePanel = toml::from_str(text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            panel.folded.into_iter().collect::<Vec<_>>(),
            [Section::Machine]
        );
        let written = toml::to_string(&SidePanel {
            folded: [Section::Cloud].into_iter().collect(),
            ..SidePanel::default()
        })
        .unwrap_or_default();
        assert!(written.contains("folded = [\"cloud\"]"), "{written}");
    }

    #[test]
    fn preview_changes_apply_and_pins_are_per_kind() {
        let mut s = Settings::default();
        s.apply(SettingsChange::PreviewWrap(true));
        s.apply(SettingsChange::PreviewLineNumbers(false));
        assert!(s.preview.text.wrap);
        assert!(!s.preview.text.line_numbers);
        s.apply(SettingsChange::PreviewPin {
            kind: "image".into(),
            section: Some("exif".into()),
        });
        s.apply(SettingsChange::PreviewPin {
            kind: "text".into(),
            section: Some("file".into()),
        });
        assert_eq!(
            s.preview.pinned.get("image").map(String::as_str),
            Some("exif")
        );
        s.apply(SettingsChange::PreviewPin {
            kind: "image".into(),
            section: Some("file".into()),
        });
        assert_eq!(
            s.preview.pinned.get("image").map(String::as_str),
            Some("file")
        );
        s.apply(SettingsChange::PreviewPin {
            kind: "image".into(),
            section: None,
        });
        assert_eq!(s.preview.pinned.get("image"), None);
        assert_eq!(
            s.preview.pinned.get("text").map(String::as_str),
            Some("file")
        );
    }

    #[test]
    fn changes_apply_and_shortcuts_rebind() {
        let mut s = Settings::default();
        s.apply(SettingsChange::FoldersFirst(false));
        s.apply(SettingsChange::Sort(Column::Size, SortDir::Descending));
        s.apply(SettingsChange::Shortcut {
            action: "core.copy".into(),
            binding: Some("Mod+Shift+C".into()),
        });
        assert!(!s.folders_first);
        assert_eq!((s.sort_by, s.sort_dir), (Column::Size, SortDir::Descending));
        let local = |path: &str| Location::new(ProviderId("local"), path);
        let home = local("/home/me");
        s.apply(SettingsChange::Pin {
            items: vec![home.clone()],
            at: None,
        });
        s.apply(SettingsChange::Pin {
            items: vec![home.clone(), local("/srv")],
            at: None,
        });
        assert_eq!(s.pinned, [home.clone(), local("/srv")]);
        s.apply(SettingsChange::Pin {
            items: vec![local("/opt"), local("/var")],
            at: Some(1),
        });
        assert_eq!(
            s.pinned,
            [home.clone(), local("/opt"), local("/var"), local("/srv")]
        );
        s.apply(SettingsChange::Pin {
            items: vec![local("/x")],
            at: Some(99),
        });
        assert_eq!(s.pinned.last(), Some(&local("/x")), "a far index appends");
        s.apply(SettingsChange::Unpin(home));
        assert_eq!(s.pinned[0], local("/opt"));
        s.apply(SettingsChange::SpringDelay(250));
        s.apply(SettingsChange::AfterDrag(AfterDrag::Return));
        assert_eq!(s.drag.spring_delay_ms, 250);
        assert_eq!(s.drag.after, AfterDrag::Return);
        assert_eq!(s.session.start_layout(), None);
        s.apply(SettingsChange::StartLayout(Some("Review".into())));
        assert_eq!(s.session.start_layout(), Some("Review"));
        s.apply(SettingsChange::StartLayout(None));
        assert_eq!(s.session.start, LAST_SESSION);
        assert_eq!(
            s.shortcuts.get("core.copy").map(String::as_str),
            Some("Mod+Shift+C")
        );
        s.apply(SettingsChange::Shortcut {
            action: "core.copy".into(),
            binding: None,
        });
        assert!(s.shortcuts.is_empty());
    }
}
