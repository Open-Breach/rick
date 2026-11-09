//! The session: everything that comes back on the next start, as plain data
//! that serialises to `session.toml` and to a saved layout.
//!
//! A [`Session`] holds every window with its geometry, side panel and
//! workspaces, every workspace with its dock tree (splits with their
//! fractions over its panes) and its panes with their tab, and every tab's
//! kind, location, view mode, sort, group, filter, flatten, hidden flag,
//! selection ids and scroll offset. Panes are indexed within their
//! workspace, so a file written by hand needs no ids. The
//! [`Model`](crate::Model) turns itself into one with
//! [`Model::session`](crate::Model::session) and is rebuilt from one with
//! [`Model::from_session`](crate::Model::from_session).
//!
//! A file of 0.2 holds the window's one layout directly in the window, its
//! panes with several tabs each; [`WindowSession::workspaces`] reads it as
//! one workspace, and the model opens the further tabs as workspaces of
//! their own.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use types::{EntryId, Location, ProviderId};

use crate::listing::{Column, Sort, ViewMode};
use crate::settings::{SidePanel, parse_location};
use crate::{Dock, PaneId, SplitDir, TabKind};

/// Where a window is on the screen, in logical points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Geometry {
    /// Left edge of the window's outer rectangle.
    pub x: f32,
    /// Top edge of the window's outer rectangle.
    pub y: f32,
    /// Inner width.
    pub width: f32,
    /// Inner height.
    pub height: f32,
    /// Maximised; the position and size are the restored ones.
    pub maximised: bool,
}

/// A whole session: every window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Session {
    /// Index of the window that had the focus.
    pub focused_window: usize,
    /// The windows, in creation order.
    pub windows: Vec<WindowSession>,
}

/// One window of a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WindowSession {
    /// Position, size and maximised state; `None` before the window
    /// reported them.
    pub geometry: Option<Geometry>,
    /// The side panel: docked side, width, collapsed state, folded sections.
    pub side_panel: SidePanel,
    /// Index into `workspaces` of the one shown.
    pub active_workspace: usize,
    /// The workspaces, in the order of the title row's tabs.
    pub workspaces: Vec<WorkspaceSession>,
    /// Index of the pane that had the focus, in a file of 0.2 holding the
    /// window's one layout directly; never written.
    #[serde(skip_serializing_if = "is_zero")]
    pub focused_pane: usize,
    /// The tree of splits over pane indices, in a file of 0.2; never
    /// written.
    #[serde(skip_serializing_if = "DockSession::is_default")]
    pub dock: DockSession,
    /// The panes the dock refers to, in a file of 0.2; never written.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub panes: Vec<PaneSession>,
}

// Serde hands the field by reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl WindowSession {
    /// The workspaces, a file of 0.2 holding one layout directly in the
    /// window read as one workspace.
    #[must_use]
    pub fn workspaces(&self) -> Cow<'_, [WorkspaceSession]> {
        if self.workspaces.is_empty() && !self.panes.is_empty() {
            Cow::Owned(vec![WorkspaceSession {
                focused_pane: self.focused_pane,
                dock: self.dock.clone(),
                panes: self.panes.clone(),
            }])
        } else {
            Cow::Borrowed(&self.workspaces)
        }
    }
}

/// One workspace of a window: a layout of panes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WorkspaceSession {
    /// Index of the pane that had the focus.
    pub focused_pane: usize,
    /// The tree of splits over pane indices.
    pub dock: DockSession,
    /// The panes the dock refers to.
    pub panes: Vec<PaneSession>,
}

/// The dock tree of a window, over pane indices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockSession {
    /// A leaf: the pane at this index.
    Pane(usize),
    /// Two subtrees side by side or stacked.
    Split {
        /// Which way the split goes.
        dir: SplitDir,
        /// How much of the room the first part takes, 0 to 1.
        fraction: f32,
        /// Left or top.
        first: Box<DockSession>,
        /// Right or bottom.
        second: Box<DockSession>,
    },
}

impl Default for DockSession {
    fn default() -> Self {
        Self::Pane(0)
    }
}

impl DockSession {
    fn is_default(&self) -> bool {
        *self == Self::Pane(0)
    }

    /// Every pane index in the tree, left-to-right, top-to-bottom.
    #[must_use]
    pub fn panes(&self) -> Vec<usize> {
        match self {
            Self::Pane(index) => vec![*index],
            Self::Split { first, second, .. } => {
                let mut out = first.panes();
                out.extend(second.panes());
                out
            }
        }
    }

    pub(crate) fn from_dock(dock: &Dock, index_of: &dyn Fn(PaneId) -> usize) -> Self {
        match dock {
            Dock::Pane(pane) => Self::Pane(index_of(*pane)),
            Dock::Split {
                dir,
                fraction,
                first,
                second,
            } => Self::Split {
                dir: *dir,
                fraction: *fraction,
                first: Box::new(Self::from_dock(first, index_of)),
                second: Box::new(Self::from_dock(second, index_of)),
            },
        }
    }

    /// The dock over pane ids, `None` when an index has no pane.
    pub(crate) fn to_dock(&self, ids: &[PaneId]) -> Option<Dock> {
        match self {
            Self::Pane(index) => ids.get(*index).map(|id| Dock::Pane(*id)),
            Self::Split {
                dir,
                fraction,
                first,
                second,
            } => match (first.to_dock(ids), second.to_dock(ids)) {
                (Some(first), Some(second)) => Some(Dock::Split {
                    dir: *dir,
                    fraction: fraction.clamp(0.05, 0.95),
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(one), None) | (None, Some(one)) => Some(one),
                (None, None) => None,
            },
        }
    }
}

/// One pane of a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PaneSession {
    /// Index into `tabs` of the active tab.
    pub active: Option<usize>,
    /// The pane's tab; several in a file of 0.2, whose panes had strips.
    pub tabs: Vec<TabSession>,
}

/// One tab of a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabSession {
    /// What the tab shows: an explorer unless the file says otherwise.
    pub kind: TabKind,
    /// Where the tab is, as `provider:path`.
    #[serde(with = "location_toml")]
    pub location: Location,
    /// How the entries are shown.
    pub view: ViewMode,
    /// Sort column and direction.
    pub sort: Sort,
    /// Group headers, if any.
    pub group: Option<Column>,
    /// The type-to-filter text.
    pub filter: String,
    /// Hidden entries shown.
    pub show_hidden: bool,
    /// Subfolders flattened into the list.
    pub flatten: bool,
    /// Selected entry ids, as `provider:key`.
    #[serde(with = "entry_ids_toml")]
    pub selection: Vec<EntryId>,
    /// The list's vertical scroll offset in logical points.
    pub scroll: f32,
    /// A note for the status line, set while restoring when the tab could
    /// not open where it was; never written to the file.
    #[serde(skip)]
    pub note: Option<String>,
}

impl Default for TabSession {
    fn default() -> Self {
        Self {
            kind: TabKind::Explorer,
            location: Location::new(ProviderId("local"), "/"),
            view: ViewMode::Details,
            sort: Sort {
                by: Column::Name,
                dir: crate::listing::SortDir::Ascending,
            },
            group: None,
            filter: String::new(),
            show_hidden: false,
            flatten: false,
            selection: Vec::new(),
            scroll: 0.0,
            note: None,
        }
    }
}

/// A location as a `provider:path` string.
pub(crate) mod location_toml {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use types::Location;

    pub(crate) fn serialize<S: Serializer>(
        location: &Location,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        location.to_string().serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Location, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(super::parse_location(&text))
    }
}

/// Entry ids as `provider:key` strings, the key in hexadecimal.
mod entry_ids_toml {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use types::EntryId;

    pub(super) fn serialize<S: Serializer>(
        ids: &[EntryId],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let texts: Vec<String> = ids.iter().map(ToString::to_string).collect();
        texts.serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<EntryId>, D::Error> {
        let texts: Vec<String> = Vec::deserialize(deserializer)?;
        Ok(texts
            .iter()
            .filter_map(|text| super::parse_entry_id(text))
            .collect())
    }
}

/// `provider:key` with the key in hexadecimal, as [`EntryId`] displays
/// itself; `None` for anything else.
#[must_use]
pub fn parse_entry_id(text: &str) -> Option<EntryId> {
    let (provider, key) = text.rsplit_once(':')?;
    if provider.is_empty() {
        return None;
    }
    let key = u128::from_str_radix(key, 16).ok()?;
    Some(EntryId::new(ProviderId::intern(provider), key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_ids_round_trip_through_text() {
        let id = EntryId::from_device_inode(ProviderId("local"), 7, 42);
        assert_eq!(parse_entry_id(&id.to_string()), Some(id));
        assert_eq!(parse_entry_id("nonsense"), None);
        assert_eq!(parse_entry_id(":1"), None);
        assert_eq!(parse_entry_id("local:zz"), None);
    }

    #[test]
    fn dock_sessions_map_to_and_from_docks() {
        let dock = Dock::Split {
            dir: SplitDir::Right,
            fraction: 0.3,
            first: Box::new(Dock::Pane(PaneId(5))),
            second: Box::new(Dock::Split {
                dir: SplitDir::Down,
                fraction: 0.5,
                first: Box::new(Dock::Pane(PaneId(7))),
                second: Box::new(Dock::Pane(PaneId(9))),
            }),
        };
        let ids = [PaneId(5), PaneId(7), PaneId(9)];
        let index_of = |pane: PaneId| ids.iter().position(|p| *p == pane).unwrap_or(0);
        let session = DockSession::from_dock(&dock, &index_of);
        assert_eq!(session.panes(), [0, 1, 2]);
        assert_eq!(session.to_dock(&ids), Some(dock));
        // An index without a pane collapses its split.
        let short = session.to_dock(&ids[..2]);
        assert_eq!(
            short,
            Some(Dock::Split {
                dir: SplitDir::Right,
                fraction: 0.3,
                first: Box::new(Dock::Pane(PaneId(5))),
                second: Box::new(Dock::Pane(PaneId(7))),
            })
        );
        assert_eq!(DockSession::Pane(3).to_dock(&ids), None);
    }
}
