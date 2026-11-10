//! The settings snapshot: what `rick.toml` holds, as data.
//!
//! Only settings the specification names appear here. The TOML file itself
//! is read and written by `app`; the model only carries the values.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use types::{Location, ProviderId};

use crate::integration::{IntegrationSettings, ItemRecord};
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

/// The density of the interface: the *Density* token set of SPEC.md's
/// *Look and feel*, two levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    /// Tight rows and small controls.
    Compact,
    /// The default; the `normal` of 0.1 and 0.2 reads as this.
    #[default]
    #[serde(alias = "normal")]
    Comfortable,
}

/// A colour as `rick.toml` writes it, `#rrggbb`: red, green, blue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    /// The colour a `#rrggbb` text names, the `#` optional and the case
    /// free; `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.trim();
        let hex = hex.strip_prefix('#').unwrap_or(hex);
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
        Some(Self([channel(0)?, channel(2)?, channel(4)?]))
    }

    /// The colour as `#rrggbb`.
    #[must_use]
    pub fn text(self) -> String {
        let [r, g, b] = self.0;
        format!("#{r:02x}{g:02x}{b:02x}")
    }
}

impl Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text())
    }
}

/// The colour of the title row's band (SPEC.md *Title bar colour*): one
/// of six presets, each a light and a dark value so that one choice works
/// in both schemes, or the user's own pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TitlebarColour {
    /// `#ece0db` and `#332c2c`: the values of the `bg/titlebar` token.
    #[default]
    DustyRose,
    /// `#e6e2dc` and `#2c2a28`.
    WarmGrey,
    /// `#d9e4e4` and `#263233`.
    SlateTeal,
    /// `#dfe6d8` and `#283027`.
    Sage,
    /// `#e4dcea` and `#3a2e4a`.
    Plum,
    /// `#2f3237` in both schemes: a dark band in Light too.
    DarkBand,
    /// `titlebar_custom_light` and `titlebar_custom_dark`.
    Custom,
}

impl TitlebarColour {
    /// The choices in the order the settings show them: the presets as
    /// SPEC.md's table lists them, then *Custom*.
    pub const ALL: [Self; 7] = [
        Self::DustyRose,
        Self::WarmGrey,
        Self::SlateTeal,
        Self::Sage,
        Self::Plum,
        Self::DarkBand,
        Self::Custom,
    ];

    /// A preset's light and dark value; `None` for *Custom*, whose pair
    /// the settings hold.
    #[must_use]
    pub const fn preset(self) -> Option<(Rgb, Rgb)> {
        let pair = match self {
            Self::DustyRose => ([0xec, 0xe0, 0xdb], [0x33, 0x2c, 0x2c]),
            Self::WarmGrey => ([0xe6, 0xe2, 0xdc], [0x2c, 0x2a, 0x28]),
            Self::SlateTeal => ([0xd9, 0xe4, 0xe4], [0x26, 0x32, 0x33]),
            Self::Sage => ([0xdf, 0xe6, 0xd8], [0x28, 0x30, 0x27]),
            Self::Plum => ([0xe4, 0xdc, 0xea], [0x3a, 0x2e, 0x4a]),
            Self::DarkBand => ([0x2f, 0x32, 0x37], [0x2f, 0x32, 0x37]),
            Self::Custom => return None,
        };
        Some((Rgb(pair.0), Rgb(pair.1)))
    }
}

/// The default of `titlebar_custom_light`: the default preset's light
/// value, so that *Custom* starts from the band the user knows.
const fn custom_light() -> Rgb {
    Rgb([0xec, 0xe0, 0xdb])
}

/// The default of `titlebar_custom_dark`.
const fn custom_dark() -> Rgb {
    Rgb([0x33, 0x2c, 0x2c])
}

/// A `#rrggbb` key of `rick.toml`: anything that is no colour reads as
/// `fallback` instead of failing the whole file.
fn lenient_rgb<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
    fallback: Rgb,
) -> Result<Rgb, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Text(String),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Value::deserialize(deserializer)? {
        Value::Text(text) => Rgb::parse(&text).unwrap_or(fallback),
        Value::Other(_) => fallback,
    })
}

fn lenient_light<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Rgb, D::Error> {
    lenient_rgb(deserializer, custom_light())
}

fn lenient_dark<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Rgb, D::Error> {
    lenient_rgb(deserializer, custom_dark())
}

/// The `titlebar_colour` key: a name that is no preset reads as the
/// default instead of failing the whole file.
fn lenient_titlebar<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<TitlebarColour, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Known(TitlebarColour),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Value::deserialize(deserializer)? {
        Value::Known(colour) => colour,
        Value::Other(_) => TitlebarColour::default(),
    })
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
    /// The density of the interface.
    pub density: Density,
    /// The colour of the title row's band: a preset, or `custom`.
    #[serde(deserialize_with = "lenient_titlebar")]
    pub titlebar_colour: TitlebarColour,
    /// The band in the light scheme while `titlebar_colour` is `custom`.
    #[serde(deserialize_with = "lenient_light")]
    pub titlebar_custom_light: Rgb,
    /// The band in the dark scheme while `titlebar_colour` is `custom`.
    #[serde(deserialize_with = "lenient_dark")]
    pub titlebar_custom_dark: Rgb,
    /// The language of the user interface as a tag (`de`), or `None` for
    /// the system's; applied live, English the fallback for a missing key.
    pub language: Option<String>,
    /// Rebound shortcuts: action id to shortcut text, e.g. `"core.copy" =
    /// "Mod+C"`. Actions not listed keep their default.
    pub shortcuts: BTreeMap<String, String>,
    /// What Rick opens with: the last session or a named layout.
    pub session: SessionSettings,
    /// The windows: which frame Rick draws around one it decorates itself.
    pub window: WindowSettings,
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
    /// Archives: what Open does with one.
    pub archives: Archives,
    /// Plugins: which of them not to load.
    pub plugins: Plugins,
    /// The network and cloud connections, one `[[connections]]` table
    /// each: the provider, the name and the non-secret fields. The
    /// secrets are in the platform keychain, never here.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionEntry>,
    /// System integration (specs/system-integration.md §6.1): one record
    /// per integration item, whether the dialog asks, and the switches of
    /// Settings → Integration.
    pub integration: IntegrationSettings,
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

/// The `[archives]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Archives {
    /// What Enter and a double-click do with an archive or a disk image.
    pub on_open: OnOpen,
}

/// What Open does with an archive or a disk image (SPEC.md, *Opening
/// files*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnOpen {
    /// Enter it as a read-only location in the current tab, through its
    /// provider.
    #[default]
    Enter,
    /// Hand it to the system's default application, like any other file.
    System,
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
    /// Whether the File info card is pinned, by the id of the previewer
    /// that shows the file (`preview-image = true`); a previewer without
    /// an entry has it unpinned. An entry of 0.3, which named a section
    /// per content kind, is dropped when the file is read.
    #[serde(deserialize_with = "lenient_pins")]
    pub pinned: BTreeMap<String, bool>,
    /// The widths of the columns of the table a previewer shows, by the
    /// previewer's id and then by position — every column but the last,
    /// which takes what is left (`preview-archive = [240, 72, 112, 88]`).
    /// A previewer without an entry has the default widths, and so has a
    /// column beyond those remembered.
    pub columns: BTreeMap<String, Vec<u16>>,
}

/// The pins of `[preview.pinned]`: the entries that are `true`. Anything
/// else under a key — 0.3's section name — is no pin and is dropped
/// instead of failing the whole file.
fn lenient_pins<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, bool>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Pin {
        Flag(bool),
        Other(serde::de::IgnoredAny),
    }
    let entries: BTreeMap<String, Pin> = BTreeMap::deserialize(deserializer)?;
    Ok(entries
        .into_iter()
        .filter(|(_, pin)| matches!(pin, Pin::Flag(true)))
        .map(|(previewer, _)| (previewer, true))
        .collect())
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

/// The `[window]` table. Its keys are in the config file only, with no
/// entry in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WindowSettings {
    /// Which frame Rick draws around a window it decorates itself. Read
    /// when Rick starts.
    #[serde(deserialize_with = "lenient_frame")]
    pub frame: WindowFrame,
}

/// `frame` in the `[window]` table (specs/gui-title-row.md, *Window
/// frame*): the override of the check whether the window is transparent,
/// for a system that reports a transparent surface and composites it
/// wrongly, or the other way round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowFrame {
    /// The shadow frame where the window is transparent, the plain frame
    /// where it is not.
    #[default]
    Auto,
    /// Always the shadow frame: a transparent margin with a shadow.
    Shadow,
    /// Always the plain frame, and no transparent window is asked for.
    Plain,
}

/// The `frame` key: a value that is none of the three reads as `auto`
/// instead of failing the whole file.
fn lenient_frame<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<WindowFrame, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Known(WindowFrame),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Value::deserialize(deserializer)? {
        Value::Known(frame) => frame,
        Value::Other(_) => WindowFrame::default(),
    })
}

impl Settings {
    /// The band of the title row in the light and in the dark scheme: the
    /// chosen preset's pair, or the custom one.
    #[must_use]
    pub fn titlebar(&self) -> (Rgb, Rgb) {
        self.titlebar_colour
            .preset()
            .unwrap_or((self.titlebar_custom_light, self.titlebar_custom_dark))
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
            density: Density::Comfortable,
            titlebar_colour: TitlebarColour::default(),
            titlebar_custom_light: custom_light(),
            titlebar_custom_dark: custom_dark(),
            shortcuts: BTreeMap::new(),
            session: SessionSettings::default(),
            window: WindowSettings::default(),
            pinned: Vec::new(),
            drag: Drag::default(),
            preview: Preview::default(),
            search: Search::default(),
            archives: Archives::default(),
            plugins: Plugins::default(),
            connections: Vec::new(),
            integration: IntegrationSettings::default(),
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
    /// The density of the interface.
    Density(Density),
    /// The title row's band: a preset, or the custom pair.
    TitlebarColour(TitlebarColour),
    /// One colour of the custom pair: the light scheme's, or the dark
    /// scheme's.
    TitlebarCustom {
        /// The dark scheme's value; the light scheme's otherwise.
        dark: bool,
        /// The colour.
        colour: Rgb,
    },
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
    /// Remember the widths of the columns of a previewer's table.
    PreviewColumns {
        /// The previewer's id.
        previewer: String,
        /// The widths by position, every column's but the last.
        widths: Vec<u16>,
    },
    /// Pin or unpin the File info card for the files a previewer shows.
    PreviewPin {
        /// The previewer's id.
        previewer: String,
        /// Pinned above the content, or not.
        pinned: bool,
    },
    /// The names the subtree search skips.
    SearchIgnore(Vec<String>),
    /// What Open does with an archive or a disk image.
    ArchivesOnOpen(OnOpen),
    /// Enable or disable a plugin; takes effect on the next start.
    Plugin {
        /// The plugin id.
        id: String,
        /// `false` adds it to `[plugins] disabled`, `true` removes it.
        enabled: bool,
    },
    /// Whether the integration dialog opens when the startup check finds
    /// something; `false` is its *Don't ask again*.
    IntegrationAsk(bool),
    /// *Check integration at startup*.
    IntegrationCheckAtStartup(bool),
    /// *Shell integration*: the console follows the tab and the tab the
    /// console.
    ShellIntegration(bool),
    /// The integration items' records, as a check or an apply left them.
    IntegrationRecords(BTreeMap<String, ItemRecord>),
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
            SettingsChange::TitlebarColour(v) => self.titlebar_colour = v,
            SettingsChange::TitlebarCustom { dark: true, colour } => {
                self.titlebar_custom_dark = colour;
            }
            SettingsChange::TitlebarCustom {
                dark: false,
                colour,
            } => self.titlebar_custom_light = colour,
            SettingsChange::Language(v) => self.language = v,
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
            SettingsChange::PreviewColumns { previewer, widths } => {
                self.preview.columns.insert(previewer, widths);
            }
            SettingsChange::PreviewPin { previewer, pinned } => {
                if pinned {
                    self.preview.pinned.insert(previewer, true);
                } else {
                    self.preview.pinned.remove(&previewer);
                }
            }
            SettingsChange::SearchIgnore(names) => self.search.ignore = names,
            SettingsChange::ArchivesOnOpen(on_open) => self.archives.on_open = on_open,
            SettingsChange::Plugin { id, enabled } => {
                self.plugins.disabled.retain(|d| *d != id);
                if !enabled {
                    self.plugins.disabled.push(id);
                }
            }
            SettingsChange::IntegrationAsk(v) => self.integration.ask = v,
            SettingsChange::IntegrationCheckAtStartup(v) => self.integration.check_at_startup = v,
            SettingsChange::ShellIntegration(v) => self.integration.shell_integration = v,
            SettingsChange::IntegrationRecords(records) => self.integration.items = records,
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
        assert_eq!(s.archives.on_open, OnOpen::Enter, "an archive is entered");
        let mut s = s;
        s.apply(SettingsChange::SearchIgnore(vec!["build".into()]));
        assert_eq!(s.search.ignore, ["build"]);
        s.apply(SettingsChange::ArchivesOnOpen(OnOpen::System));
        assert_eq!(s.archives.on_open, OnOpen::System);
        let text = toml::to_string(&s).unwrap_or_default();
        assert!(text.contains("[archives]\non_open = \"system\""), "{text}");
    }

    #[test]
    fn the_title_bar_colour_is_a_preset_or_a_custom_pair() {
        // The presets, as SPEC.md's table lists them.
        let table = [
            (
                TitlebarColour::DustyRose,
                "dusty-rose",
                "#ece0db",
                "#332c2c",
            ),
            (TitlebarColour::WarmGrey, "warm-grey", "#e6e2dc", "#2c2a28"),
            (
                TitlebarColour::SlateTeal,
                "slate-teal",
                "#d9e4e4",
                "#263233",
            ),
            (TitlebarColour::Sage, "sage", "#dfe6d8", "#283027"),
            (TitlebarColour::Plum, "plum", "#e4dcea", "#3a2e4a"),
            (TitlebarColour::DarkBand, "dark-band", "#2f3237", "#2f3237"),
        ];
        for (index, (preset, key, light, dark)) in table.into_iter().enumerate() {
            assert_eq!(TitlebarColour::ALL[index], preset);
            let pair = preset.preset().map(|(l, d)| (l.text(), d.text()));
            assert_eq!(pair, Some((light.to_owned(), dark.to_owned())), "{key}");
            // The choice is written under its name and read back.
            let mut s = Settings::default();
            s.apply(SettingsChange::TitlebarColour(preset));
            let text = toml::to_string(&s).unwrap_or_default();
            assert!(
                text.contains(&format!("titlebar_colour = \"{key}\"")),
                "{text}"
            );
            let back: Settings = toml::from_str(&text).unwrap_or_default();
            assert_eq!(back.titlebar_colour, preset);
            assert_eq!(back.titlebar().0.text(), light);
        }
        assert_eq!(TitlebarColour::ALL[6], TitlebarColour::Custom);
        assert_eq!(TitlebarColour::Custom.preset(), None);

        // The default is Dusty rose, and Custom starts from its pair.
        let mut s = Settings::default();
        assert_eq!(s.titlebar_colour, TitlebarColour::DustyRose);
        s.apply(SettingsChange::TitlebarColour(TitlebarColour::Custom));
        assert_eq!(
            s.titlebar(),
            (Rgb([0xec, 0xe0, 0xdb]), Rgb([0x33, 0x2c, 0x2c]))
        );
        s.apply(SettingsChange::TitlebarCustom {
            dark: false,
            colour: Rgb([0x10, 0x20, 0x30]),
        });
        s.apply(SettingsChange::TitlebarCustom {
            dark: true,
            colour: Rgb([0xa0, 0xb0, 0xc0]),
        });
        assert_eq!(
            s.titlebar(),
            (Rgb([0x10, 0x20, 0x30]), Rgb([0xa0, 0xb0, 0xc0]))
        );
        let text = toml::to_string(&s).unwrap_or_default();
        assert!(text.contains("titlebar_colour = \"custom\""), "{text}");
        assert!(
            text.contains("titlebar_custom_light = \"#102030\""),
            "{text}"
        );
        assert!(
            text.contains("titlebar_custom_dark = \"#a0b0c0\""),
            "{text}"
        );
        let back: Settings = toml::from_str(&text).unwrap_or_default();
        assert_eq!(back, s);
        // A preset keeps the custom pair for the next time.
        s.apply(SettingsChange::TitlebarColour(TitlebarColour::Sage));
        assert_eq!(s.titlebar().0.text(), "#dfe6d8");
        assert_eq!(s.titlebar_custom_light, Rgb([0x10, 0x20, 0x30]));

        // What is no colour and no preset reads as the default, and the
        // rest of the file stands.
        let odd = "show_hidden = true\ntitlebar_colour = \"neon\"\n\
                   titlebar_custom_light = \"blue\"\ntitlebar_custom_dark = 7\n";
        let read: Settings = toml::from_str(odd).unwrap_or_else(|e| panic!("{e}"));
        assert!(read.show_hidden);
        assert_eq!(read.titlebar_colour, TitlebarColour::DustyRose);
        assert_eq!(read.titlebar_custom_light, Rgb([0xec, 0xe0, 0xdb]));
        assert_eq!(read.titlebar_custom_dark, Rgb([0x33, 0x2c, 0x2c]));
        // A colour is six hex digits, the hash and the case free.
        assert_eq!(Rgb::parse("#ECE0DB"), Some(Rgb([0xec, 0xe0, 0xdb])));
        assert_eq!(Rgb::parse(" 2f3237 "), Some(Rgb([0x2f, 0x32, 0x37])));
        for bad in ["", "#fff", "#12345g", "#1234567", "rose"] {
            assert_eq!(Rgb::parse(bad), None, "{bad}");
        }
    }

    /// Issue 158: `frame` of the `[window]` table is `auto`, `shadow` or
    /// `plain`; absent it is `auto`, and so is a value Rick does not know.
    #[test]
    fn the_window_frame_is_auto_shadow_or_plain() {
        assert_eq!(Settings::default().window.frame, WindowFrame::Auto);
        let old: Settings = toml::from_str("show_hidden = true\n").unwrap_or_default();
        assert_eq!(old.window, WindowSettings::default(), "an older file");
        for (text, frame) in [
            ("auto", WindowFrame::Auto),
            ("shadow", WindowFrame::Shadow),
            ("plain", WindowFrame::Plain),
        ] {
            let file = format!("[window]\nframe = \"{text}\"\n");
            let read: Settings = toml::from_str(&file).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(read.window.frame, frame, "{text}");
            // Written under the same name and read back.
            let written = toml::to_string(&read).unwrap_or_default();
            assert!(written.contains(&file), "{written}");
            let back: Settings = toml::from_str(&written).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(back, read);
        }
        // What is none of the three reads as `auto`, and the rest of the
        // file stands.
        for odd in ["frame = \"glass\"", "frame = 3", "frame = true"] {
            let file = format!("show_hidden = true\n[window]\n{odd}\n");
            let read: Settings = toml::from_str(&file).unwrap_or_else(|e| panic!("{e}"));
            assert!(read.show_hidden, "{odd}");
            assert_eq!(read.window.frame, WindowFrame::Auto, "{odd}");
        }
    }

    #[test]
    fn the_integration_table_has_its_switches_and_records() {
        use crate::integration::ItemState;
        let mut s = Settings::default();
        assert!(s.integration.ask, "the dialog asks by default");
        assert!(s.integration.check_at_startup);
        assert!(s.integration.shell_integration);
        s.apply(SettingsChange::IntegrationAsk(false));
        s.apply(SettingsChange::IntegrationCheckAtStartup(false));
        s.apply(SettingsChange::ShellIntegration(false));
        s.apply(SettingsChange::IntegrationRecords(BTreeMap::from([(
            "dock-favourite".to_owned(),
            ItemRecord::new(ItemState::Declined),
        )])));
        let text = toml::to_string(&s).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            text.contains(
                "[integration]\nask = false\ncheck_at_startup = false\nshell_integration = false"
            ),
            "{text}"
        );
        assert!(
            text.contains("[integration.items.dock-favourite]\nstate = \"declined\""),
            "{text}"
        );
        let back: Settings = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, s);
        // A file from before the table reads as the defaults.
        let old: Settings = toml::from_str("theme = \"dark\"\n").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(old.integration, IntegrationSettings::default());
    }

    #[test]
    fn the_old_density_and_accent_keys_still_read() {
        // 0.1 and 0.2 had three densities and an accent colour (QUESTIONS.md
        // 225 and 226): `normal` is Comfortable now and `accent` is dropped
        // on the next write.
        let s: Settings = toml::from_str("density = \"normal\"\naccent = \"#ff0000\"\n")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.density, Density::Comfortable);
        let s: Settings =
            toml::from_str("density = \"compact\"\n").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.density, Density::Compact);
        assert_eq!(Settings::default().density, Density::Comfortable);
        let text = toml::to_string(&s).unwrap_or_else(|e| panic!("{e}"));
        assert!(!text.contains("accent"), "{text}");
        assert!(text.contains("density = \"compact\""), "{text}");
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
    fn preview_changes_apply_and_pins_are_per_previewer() {
        let mut s = Settings::default();
        s.apply(SettingsChange::PreviewWrap(true));
        s.apply(SettingsChange::PreviewLineNumbers(false));
        assert!(s.preview.text.wrap);
        assert!(!s.preview.text.line_numbers);
        let pin = |previewer: &str, pinned: bool| SettingsChange::PreviewPin {
            previewer: previewer.into(),
            pinned,
        };
        s.apply(pin("preview-image", true));
        s.apply(pin("preview-text", true));
        assert_eq!(s.preview.pinned.get("preview-image"), Some(&true));
        s.apply(pin("preview-image", false));
        assert_eq!(s.preview.pinned.get("preview-image"), None);
        assert_eq!(s.preview.pinned.get("preview-text"), Some(&true));
        // Written as a table of switches and read back.
        let text = toml::to_string(&s).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            text.contains("[preview.pinned]\npreview-text = true"),
            "{text}"
        );
        let back: Settings = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.preview.pinned, s.preview.pinned);
        // The widths of a previewer's table are kept by position.
        s.apply(SettingsChange::PreviewColumns {
            previewer: "preview-archive".into(),
            widths: vec![240, 72, 112, 88],
        });
        let text = toml::to_string(&s).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            text.contains("preview-archive = [240, 72, 112, 88]"),
            "{text}"
        );
        let back: Settings = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            back.preview.columns.get("preview-archive"),
            Some(&vec![240, 72, 112, 88])
        );
        // A pin of 0.3, a section per content kind, is not carried over
        // and does not stop the file from being read.
        let old: Settings = toml::from_str(
            "theme = \"dark\"\n[preview.pinned]\nimage = \"exif\"\npreview-pdf = true\noff = false\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(old.theme, Theme::Dark);
        assert_eq!(
            old.preview.pinned.keys().collect::<Vec<_>>(),
            ["preview-pdf"]
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
