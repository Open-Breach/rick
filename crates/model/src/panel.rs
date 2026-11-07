//! The side panel's data: the Places list and the trash's contents.

use serde::{Deserialize, Serialize};
use types::Location;

use crate::session::location_toml;

/// The Phosphor icon name a place gets when none is chosen.
pub const DEFAULT_PLACE_ICON: &str = "FOLDER";

/// One row of the Places list: a title, a Phosphor icon name and the
/// location it opens, a folder or a virtual location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    /// What the row says.
    pub title: String,
    /// The Phosphor icon's name, `FOLDER` by default.
    pub icon: String,
    /// Where the row goes, as `provider:path`.
    #[serde(with = "location_toml")]
    pub location: Location,
}

impl Place {
    /// A place with the default icon.
    #[must_use]
    pub fn new(title: impl Into<String>, location: Location) -> Self {
        Self {
            title: title.into(),
            icon: DEFAULT_PLACE_ICON.to_owned(),
            location,
        }
    }

    /// The same place with another icon.
    #[must_use]
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = icon.into();
        self
    }
}

/// The Places list as the model keeps it: in display order, one row per
/// location.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Places {
    rows: Vec<Place>,
}

impl Places {
    /// The rows in display order.
    #[must_use]
    pub fn rows(&self) -> &[Place] {
        &self.rows
    }

    /// Number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the list has no row.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Replaces every row.
    pub fn set(&mut self, rows: Vec<Place>) {
        self.rows = rows;
    }

    /// The index of the row opening `location`, if one does.
    #[must_use]
    pub fn position(&self, location: &Location) -> Option<usize> {
        self.rows.iter().position(|p| p.location == *location)
    }

    /// Inserts a row at `at`, or at the end; a location already listed
    /// stays where it is. Returns whether the list changed.
    pub fn add(&mut self, place: Place, at: Option<usize>) -> bool {
        if self.position(&place.location).is_some() {
            return false;
        }
        let at = at.map_or(self.rows.len(), |at| at.min(self.rows.len()));
        self.rows.insert(at, place);
        true
    }

    /// Removes the row at `index`.
    pub fn remove(&mut self, index: usize) -> Option<Place> {
        (index < self.rows.len()).then(|| self.rows.remove(index))
    }

    /// Moves the row at `from` so that it sits at `to` afterwards, both in
    /// the current numbering. Returns whether anything moved.
    pub fn shift(&mut self, from: usize, to: usize) -> bool {
        if from >= self.rows.len() || from == to {
            return false;
        }
        let to = to.min(self.rows.len() - 1);
        let row = self.rows.remove(from);
        self.rows.insert(to, row);
        true
    }

    /// Changes the title and icon of the row at `index`; an empty title or
    /// icon keeps the old one. Returns whether anything changed.
    pub fn edit(&mut self, index: usize, title: &str, icon: &str) -> bool {
        let Some(row) = self.rows.get_mut(index) else {
            return false;
        };
        let mut changed = false;
        let title = title.trim();
        if !title.is_empty() && row.title != title {
            title.clone_into(&mut row.title);
            changed = true;
        }
        let icon = icon.trim();
        if !icon.is_empty() && row.icon != icon {
            icon.clone_into(&mut row.icon);
            changed = true;
        }
        changed
    }
}

/// The trash as the side panel knows it: enough for the Trash row's hint.
#[derive(Debug, Clone, Default)]
pub struct TrashView {
    /// The items, newest first.
    pub items: Vec<platform::TrashedItem>,
    /// `true` when the platform listed its trash; `false` when the items are
    /// Rick's own record of what it trashed, because the platform cannot
    /// list original paths (macOS).
    pub listed: bool,
    /// The platform's trash could not be read at all; the text says why.
    pub error: Option<String>,
}

impl TrashView {
    /// Replaces the items, newest first.
    pub fn set(&mut self, mut items: Vec<platform::TrashedItem>, listed: bool) {
        items.sort_by_key(|item| std::cmp::Reverse(item.deleted_at));
        self.items = items;
        self.listed = listed;
        self.error = None;
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use types::ProviderId;

    use super::*;

    fn place(name: &str) -> Place {
        Place::new(name, Location::new(ProviderId("local"), format!("/{name}")))
    }

    #[test]
    fn places_add_in_order_reorder_edit_and_remove() {
        let mut places = Places::default();
        assert!(places.is_empty());
        assert!(places.add(place("a"), None));
        assert!(places.add(place("c"), None));
        assert!(places.add(place("b"), Some(1)));
        assert!(!places.add(place("a"), Some(0)), "a listed location stays");
        let names =
            |p: &Places| -> Vec<String> { p.rows().iter().map(|r| r.title.clone()).collect() };
        assert_eq!(names(&places), ["a", "b", "c"]);
        assert!(places.shift(0, 2));
        assert_eq!(names(&places), ["b", "c", "a"]);
        assert!(places.shift(2, 0));
        assert_eq!(names(&places), ["a", "b", "c"]);
        assert!(!places.shift(1, 1));
        assert!(!places.shift(9, 0));
        assert!(places.shift(0, 99), "a far index moves to the end");
        assert_eq!(names(&places), ["b", "c", "a"]);
        assert!(places.edit(0, "Beta", "STAR"));
        assert_eq!(places.rows()[0].title, "Beta");
        assert_eq!(places.rows()[0].icon, "STAR");
        assert!(!places.edit(0, "  ", ""), "empty keeps the old values");
        assert!(!places.edit(7, "x", "y"));
        assert_eq!(
            places.position(&Location::new(ProviderId("local"), "/c")),
            Some(1)
        );
        assert_eq!(places.remove(1).map(|p| p.title), Some("c".to_owned()));
        assert_eq!(places.remove(5), None);
        assert_eq!(places.len(), 2);
        let text = toml::to_string(&places.rows()[0]).unwrap_or_default();
        assert!(text.contains("location = \"local:/b\""), "{text}");
        let back: Place = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, places.rows()[0]);
    }

    #[test]
    fn the_trash_view_sorts_newest_first() {
        let mut view = TrashView::default();
        let item = |n: u64| platform::TrashedItem {
            original: std::path::PathBuf::from(format!("/o/{n}")),
            trashed: std::path::PathBuf::from(format!("/t/{n}")),
            deleted_at: SystemTime::UNIX_EPOCH + Duration::from_secs(n),
        };
        view.set(vec![item(1), item(3), item(2)], true);
        let order: Vec<_> = view.items.iter().map(|i| i.deleted_at).collect();
        assert_eq!(
            order,
            [item(3).deleted_at, item(2).deleted_at, item(1).deleted_at]
        );
        assert!(view.listed);
    }
}
