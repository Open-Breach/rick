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
    /// Rebound shortcuts: action id to shortcut text, e.g. `"core.copy" =
    /// "Mod+C"`. Actions not listed keep their default.
    pub shortcuts: BTreeMap<String, String>,
    /// The side panel: shown or hidden, which side, its width and which of
    /// its sections are folded.
    pub side_panel: SidePanel,
    /// Folders and files pinned to the side panel, in display order.
    /// Written as `provider:path` strings; a bare path is a local one.
    #[serde(with = "pinned_toml")]
    pub pinned: Vec<Location>,
    /// Drag and drop: the spring-loading delay and where a pane is left
    /// after a drag.
    pub drag: Drag,
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

/// The side panel's sections, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    /// The user's home directory.
    Home,
    /// Recently changed files and folders.
    Recent,
    /// Shares, hosts and mounted network file systems.
    Network,
    /// The platform trash.
    Trash,
    /// Locations the user pinned.
    Pinned,
}

impl Section {
    /// Every section in display order.
    pub const ALL: [Self; 5] = [
        Self::Home,
        Self::Recent,
        Self::Network,
        Self::Trash,
        Self::Pinned,
    ];

    /// The section's title.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Recent => "Recent",
            Self::Network => "Network",
            Self::Trash => "Trash",
            Self::Pinned => "Pinned",
        }
    }
}

/// The side panel's persisted state.
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
            side_panel: SidePanel::default(),
            pinned: Vec::new(),
            drag: Drag::default(),
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
    /// Rebind an action; `None` restores its default.
    Shortcut {
        /// The action id.
        action: String,
        /// The new binding as text, or `None`.
        binding: Option<String>,
    },
    /// Show or hide the side panel.
    SidePanelVisible(bool),
    /// Dock the side panel left or right.
    SidePanelSide(PanelSide),
    /// The side panel's width in logical points.
    SidePanelWidth(u16),
    /// Fold or unfold one section of the side panel.
    SidePanelFold {
        /// Which section.
        section: Section,
        /// Folded shut, or open.
        folded: bool,
    },
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
            SettingsChange::Accent(v) => self.accent = v,
            SettingsChange::Shortcut { action, binding } => match binding {
                Some(binding) => {
                    self.shortcuts.insert(action, binding);
                }
                None => {
                    self.shortcuts.remove(&action);
                }
            },
            SettingsChange::SidePanelVisible(v) => self.side_panel.visible = v,
            SettingsChange::SidePanelSide(v) => self.side_panel.side = v,
            SettingsChange::SidePanelWidth(v) => self.side_panel.width = v,
            SettingsChange::SidePanelFold { section, folded } => {
                if folded {
                    self.side_panel.folded.insert(section);
                } else {
                    self.side_panel.folded.remove(&section);
                }
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
            parse_location("sftp:/srv"),
            Location::new(ProviderId::intern("sftp"), "/srv")
        );
        let text = Location::new(ProviderId("local"), "C:\\x").to_string();
        assert_eq!(parse_location(&text).path.to_string(), "C:\\x");
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
        s.apply(SettingsChange::SidePanelFold {
            section: Section::Recent,
            folded: true,
        });
        s.apply(SettingsChange::SidePanelSide(PanelSide::Right));
        s.apply(SettingsChange::SidePanelWidth(300));
        s.apply(SettingsChange::SidePanelVisible(false));
        assert!(s.side_panel.folded.contains(&Section::Recent));
        assert_eq!(s.side_panel.side, PanelSide::Right);
        assert_eq!(s.side_panel.width, 300);
        assert!(!s.side_panel.visible);
        s.apply(SettingsChange::SidePanelFold {
            section: Section::Recent,
            folded: false,
        });
        assert!(s.side_panel.folded.is_empty());
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
