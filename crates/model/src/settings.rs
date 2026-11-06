//! The settings snapshot: what `rick.toml` holds, as data.
//!
//! Only settings the specification names appear here. The TOML file itself
//! is read and written by `app`; the model only carries the values.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
