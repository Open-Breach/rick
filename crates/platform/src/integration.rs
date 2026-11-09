//! The integration items (specs/system-integration.md §6 and §7): what
//! makes Rick the file manager of the machine rather than a window on it,
//! one [`Item`] each, behind the [`Integration`] trait — list the items of
//! this platform and desktop, check one, perform one, undo one.
//!
//! The check reads and changes nothing. Performing an item answers with a
//! [`Receipt`] of what was written, which the caller keeps (the
//! application stores it in `rick.toml` beside the item's state) and hands
//! back for the next check, so a file Rick wrote is told from one that was
//! there before, and for the undo, which removes exactly what the receipt
//! lists and puts back what it replaced.
//!
//! Linux has the six items of §7.1; Windows and macOS list none yet
//! (milestones 4 and 5 of the spec). [`FakeIntegration`] is the scripted
//! one the tests of the layers above run against.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use types::Error;

/// One integration item (specs/system-integration.md §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Item {
    /// Rick with its plugins and `share/` under the standard location,
    /// `~/.local/opt/rick/<version>` with a `current` link. Performed by
    /// the installer plugin, not by the platform, which only checks it.
    InstallLocation,
    /// The desktop entry and the hicolor icons.
    LauncherEntry,
    /// `inode/directory` and `x-scheme-handler/rick` in `mimeapps.list`.
    DefaultFileManager,
    /// The `org.freedesktop.FileManager1` D-Bus service's activation file.
    ShowInFolder,
    /// The `rick` link in the bin directory and the shell function.
    RickCommand,
    /// Rick among the dock's or the launcher's favourites.
    DockFavourite,
}

impl Item {
    /// Every item, in the order the dialog lists them.
    pub const ALL: [Self; 6] = [
        Self::InstallLocation,
        Self::LauncherEntry,
        Self::DefaultFileManager,
        Self::ShowInFolder,
        Self::RickCommand,
        Self::DockFavourite,
    ];

    /// The item's name in `rick.toml`.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::InstallLocation => "install-location",
            Self::LauncherEntry => "launcher-entry",
            Self::DefaultFileManager => "default-file-manager",
            Self::ShowInFolder => "show-in-folder",
            Self::RickCommand => "rick-command",
            Self::DockFavourite => "dock-favourite",
        }
    }

    /// The item a `rick.toml` key names, if any.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.key() == key)
    }

    /// Whether the item is *Best effort*: its failure is reported, never
    /// retried, and never blocks the others.
    #[must_use]
    pub const fn best_effort(self) -> bool {
        matches!(self, Self::DockFavourite)
    }
}

/// For whom an item is set up: the dialog's *Install for*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Scope {
    /// *Just me*: under the user's home.
    #[default]
    User,
    /// *All users*: under `/opt` and `/usr`, with the administrator's
    /// password.
    System,
}

/// A desktop whose dock or favourites Rick knows how to pin itself to
/// (specs/system-integration.md §2, decision 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Desktop {
    /// GNOME Shell.
    Gnome,
    /// KDE Plasma.
    Plasma,
    /// COSMIC.
    Cosmic,
    /// Cinnamon.
    Cinnamon,
}

impl Desktop {
    /// The desktop among the names of `XDG_CURRENT_DESKTOP`, a
    /// colon-separated list (`ubuntu:GNOME`), if it is one of the four.
    #[must_use]
    pub fn from_current_desktop(value: &str) -> Option<Self> {
        value
            .split(':')
            .find_map(|name| match name.trim().to_ascii_lowercase().as_str() {
                "gnome" | "gnome-classic" => Some(Self::Gnome),
                "kde" | "plasma" => Some(Self::Plasma),
                "cosmic" => Some(Self::Cosmic),
                "x-cinnamon" | "cinnamon" => Some(Self::Cinnamon),
                _ => None,
            })
    }
}

/// What the platform offers: the items of this platform and desktop.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Items {
    /// The operating system and the desktop in one line, as
    /// [`os_description`](crate::os_description) words it.
    pub system: String,
    /// The desktop, when it is one of the four with first-class support.
    pub desktop: Option<Desktop>,
    /// The items, in the dialog's order; empty where the platform has
    /// none yet.
    pub items: Vec<Item>,
}

/// Why an item cannot be set up here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The desktop is not one whose dock Rick knows.
    Desktop,
    /// A program the item needs is not installed; its name.
    Program(String),
    /// The configuration the item edits was not found; where it was
    /// looked for.
    Config(PathBuf),
    /// Setting up for all users is not available yet.
    AllUsers,
    /// The plugin that performs the item is not loaded; its name. Never
    /// the platform's answer: the application's, for an item a plugin
    /// performs.
    Plugin(String),
}

/// What a check found of an item.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Actual {
    /// Everything the item needs is in place.
    SetUp,
    /// Something of it is missing, or points somewhere else.
    #[default]
    Missing,
    /// It cannot be set up here.
    Unsupported(Reason),
}

/// The result of checking one item: its state and the facts the item's
/// row words its line from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Check {
    /// Set up, missing or unsupported.
    pub actual: Actual,
    /// Where the item is or goes: the install folder, the data directory
    /// of the launcher entry, the command's link, the service file.
    pub place: Option<PathBuf>,
    /// What it points at or comes from: the folder Rick runs from for the
    /// install location, the link's target for the command.
    pub target: Option<PathBuf>,
    /// A name the line mentions: the application that is the default
    /// file manager now.
    pub name: Option<String>,
    /// Files in the item's way that Rick did not write: performing the
    /// item replaces them only when the request says so.
    pub foreign: Vec<PathBuf>,
}

impl Check {
    /// A check that found the item set up.
    #[must_use]
    pub fn set_up() -> Self {
        Self {
            actual: Actual::SetUp,
            ..Self::default()
        }
    }

    /// A check that found the item missing.
    #[must_use]
    pub fn missing() -> Self {
        Self::default()
    }

    /// A check of an item that cannot be set up here.
    #[must_use]
    pub fn unsupported(reason: Reason) -> Self {
        Self {
            actual: Actual::Unsupported(reason),
            ..Self::default()
        }
    }
}

/// What Rick wrote for an item, so it can be undone: the files and links
/// it created and the values it replaced. Kept in `rick.toml` beside the
/// item's state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Receipt {
    /// The files and links created, with `~` for the home directory.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// The values replaced, by key, as they were before; an empty value
    /// for a key that had none.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub previous: BTreeMap<String, String>,
}

impl Receipt {
    /// Whether nothing was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.previous.is_empty()
    }
}

/// What a check, a perform or an undo is asked with.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Request {
    /// For whom.
    pub scope: Scope,
    /// The Rick the item points at; `None` for the one that is running,
    /// through its `current` link when it runs from an install location.
    /// Set after an install, whose copy the other items then act on.
    pub binary: Option<PathBuf>,
    /// What Rick recorded writing for the item before.
    pub receipt: Receipt,
    /// Replace the files in the item's way that Rick did not write
    /// ([`Check::foreign`]): the user was asked and agreed.
    pub replace: bool,
}

/// The integration items of the platform: list, check, perform, undo
/// (specs/system-integration.md §6.1).
pub trait Integration: Send + Sync {
    /// The items of this platform and desktop, with the line that names
    /// both. Empty on a platform without items.
    fn integration_items(&self) -> Items;

    /// Compares what `item` wants with what is there. Reads files only;
    /// changes nothing.
    fn check_integration(&self, item: Item, request: &Request) -> Check;

    /// Sets `item` up and answers what was written, on top of the
    /// request's receipt.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` for an item the platform does not perform
    /// (the install location, which is the installer plugin's) or cannot
    /// perform here; an `AlreadyExists` I/O error when a file Rick did
    /// not write is in the way and the request does not say to replace
    /// it; otherwise the I/O error of the write.
    fn perform_integration(&self, item: Item, request: &Request) -> Result<Receipt, Error>;

    /// Removes what the request's receipt lists and puts back what it
    /// replaced; what is no longer there is skipped.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` for an item the platform does not perform;
    /// otherwise the I/O error of a removal.
    fn undo_integration(&self, item: Item, request: &Request) -> Result<(), Error>;
}

/// A call a [`FakeIntegration`] received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// [`Integration::check_integration`].
    Check(Item),
    /// [`Integration::perform_integration`], with its request.
    Perform(Item, Request),
    /// [`Integration::undo_integration`], with its request.
    Undo(Item, Request),
}

/// A scripted [`Integration`] that touches nothing: the items and what a
/// check of each finds are what the test set, performing an item makes it
/// set up (or fails, when the test said so), undoing it makes it missing,
/// and every call is recorded. Cloning shares the state, so a test keeps a
/// handle beside the one the platform holds.
#[derive(Debug, Clone, Default)]
pub struct FakeIntegration {
    state: Arc<Mutex<FakeState>>,
}

#[derive(Debug, Default)]
struct FakeState {
    system: String,
    desktop: Option<Desktop>,
    items: Vec<(Item, Check)>,
    failing: Vec<Item>,
    calls: Vec<Call>,
}

impl FakeIntegration {
    /// An integration without items on the system `system` names.
    #[must_use]
    pub fn new(system: &str, desktop: Option<Desktop>) -> Self {
        let fake = Self::default();
        {
            let mut state = fake.lock();
            system.clone_into(&mut state.system);
            state.desktop = desktop;
        }
        fake
    }

    fn lock(&self) -> MutexGuard<'_, FakeState> {
        // Plain data; a panic elsewhere leaves it usable.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lists `item`, or changes what a check of it finds.
    pub fn set(&self, item: Item, check: Check) {
        let mut state = self.lock();
        if let Some(slot) = state.items.iter_mut().find(|(i, _)| *i == item) {
            slot.1 = check;
        } else {
            state.items.push((item, check));
        }
    }

    /// Makes performing `item` fail from now on.
    pub fn fail(&self, item: Item) {
        self.lock().failing.push(item);
    }

    /// What a check of `item` finds now.
    #[must_use]
    pub fn actual(&self, item: Item) -> Option<Actual> {
        self.lock()
            .items
            .iter()
            .find(|(i, _)| *i == item)
            .map(|(_, check)| check.actual.clone())
    }

    /// Every call so far, oldest first.
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.lock().calls.clone()
    }
}

impl Integration for FakeIntegration {
    fn integration_items(&self) -> Items {
        let state = self.lock();
        Items {
            system: state.system.clone(),
            desktop: state.desktop,
            items: state.items.iter().map(|(item, _)| *item).collect(),
        }
    }

    fn check_integration(&self, item: Item, _request: &Request) -> Check {
        let mut state = self.lock();
        state.calls.push(Call::Check(item));
        state
            .items
            .iter()
            .find(|(i, _)| *i == item)
            .map_or_else(Check::missing, |(_, check)| check.clone())
    }

    fn perform_integration(&self, item: Item, request: &Request) -> Result<Receipt, Error> {
        let mut state = self.lock();
        state.calls.push(Call::Perform(item, request.clone()));
        if state.failing.contains(&item) {
            return Err(Error::Io(std::io::Error::other(format!(
                "the fake was told to fail {}",
                item.key()
            ))));
        }
        let Some(slot) = state.items.iter_mut().find(|(i, _)| *i == item) else {
            return Err(Error::Unsupported("the fake does not list this item"));
        };
        if !slot.1.foreign.is_empty() && !request.replace {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "a file Rick did not write is in the way",
            )));
        }
        slot.1.actual = Actual::SetUp;
        slot.1.foreign.clear();
        let mut receipt = request.receipt.clone();
        let file = format!("fake/{}", item.key());
        if !receipt.files.contains(&file) {
            receipt.files.push(file);
        }
        Ok(receipt)
    }

    fn undo_integration(&self, item: Item, request: &Request) -> Result<(), Error> {
        let mut state = self.lock();
        state.calls.push(Call::Undo(item, request.clone()));
        if let Some(slot) = state.items.iter_mut().find(|(i, _)| *i == item) {
            slot.1.actual = Actual::Missing;
        }
        Ok(())
    }
}

/// The integration of a platform without items yet: an empty list under
/// the system's own line, and `Unsupported` for everything else.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) mod none {
    use types::Error;

    use super::{Check, Item, Items, Reason, Receipt, Request};

    const NOT_HERE: &str = "this platform has no integration items yet";

    pub(crate) fn items() -> Items {
        Items {
            system: crate::os_description(),
            desktop: None,
            items: Vec::new(),
        }
    }

    pub(crate) fn check(_item: Item, _request: &Request) -> Check {
        Check::unsupported(Reason::Desktop)
    }

    pub(crate) fn perform(_item: Item, _request: &Request) -> Result<Receipt, Error> {
        Err(Error::Unsupported(NOT_HERE))
    }

    pub(crate) fn undo(_item: Item, _request: &Request) -> Result<(), Error> {
        Err(Error::Unsupported(NOT_HERE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_name_the_items_both_ways() {
        for item in Item::ALL {
            assert_eq!(Item::from_key(item.key()), Some(item));
        }
        assert_eq!(Item::from_key("context-menu"), None);
        assert!(Item::DockFavourite.best_effort());
        assert!(!Item::LauncherEntry.best_effort());
    }

    #[test]
    fn the_desktop_is_read_from_the_list_of_names() {
        assert_eq!(Desktop::from_current_desktop("GNOME"), Some(Desktop::Gnome));
        assert_eq!(
            Desktop::from_current_desktop("ubuntu:GNOME"),
            Some(Desktop::Gnome)
        );
        assert_eq!(Desktop::from_current_desktop("KDE"), Some(Desktop::Plasma));
        assert_eq!(
            Desktop::from_current_desktop("COSMIC"),
            Some(Desktop::Cosmic)
        );
        assert_eq!(
            Desktop::from_current_desktop("X-Cinnamon"),
            Some(Desktop::Cinnamon)
        );
        assert_eq!(Desktop::from_current_desktop("XFCE"), None);
        assert_eq!(Desktop::from_current_desktop(""), None);
    }

    #[test]
    fn a_receipt_is_a_small_toml_table() {
        let receipt = Receipt {
            files: vec!["~/.local/bin/rick".to_owned()],
            previous: BTreeMap::from([(
                "inode/directory".to_owned(),
                "org.gnome.Nautilus.desktop".to_owned(),
            )]),
        };
        let text = toml::to_string(&receipt).unwrap_or_else(|e| panic!("{e}"));
        assert!(text.contains("files = [\"~/.local/bin/rick\"]"), "{text}");
        let back: Receipt = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, receipt);
        assert!(Receipt::default().is_empty());
        assert_eq!(toml::to_string(&Receipt::default()).as_deref(), Ok(""));
    }

    #[test]
    fn the_fake_performs_undoes_and_records() {
        let fake = FakeIntegration::new("Fedora Linux 43 · GNOME", Some(Desktop::Gnome));
        fake.set(Item::LauncherEntry, Check::missing());
        fake.set(
            Item::RickCommand,
            Check {
                foreign: vec![PathBuf::from("/home/u/.local/bin/rick")],
                ..Check::missing()
            },
        );
        fake.set(Item::DockFavourite, Check::missing());
        fake.fail(Item::DockFavourite);
        let listed = fake.integration_items();
        assert_eq!(listed.system, "Fedora Linux 43 · GNOME");
        assert_eq!(
            listed.items,
            [Item::LauncherEntry, Item::RickCommand, Item::DockFavourite]
        );
        let request = Request::default();
        let receipt = fake
            .perform_integration(Item::LauncherEntry, &request)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(receipt.files, ["fake/launcher-entry"]);
        assert_eq!(fake.actual(Item::LauncherEntry), Some(Actual::SetUp));
        // A foreign file is replaced only when the request says so.
        assert!(
            fake.perform_integration(Item::RickCommand, &request)
                .is_err()
        );
        let replace = Request {
            replace: true,
            ..Request::default()
        };
        assert!(
            fake.perform_integration(Item::RickCommand, &replace)
                .is_ok()
        );
        assert!(
            fake.check_integration(Item::RickCommand, &request)
                .foreign
                .is_empty()
        );
        assert!(
            fake.perform_integration(Item::DockFavourite, &request)
                .is_err()
        );
        assert_eq!(fake.actual(Item::DockFavourite), Some(Actual::Missing));
        fake.undo_integration(
            Item::LauncherEntry,
            &Request {
                receipt,
                ..Request::default()
            },
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fake.actual(Item::LauncherEntry), Some(Actual::Missing));
        assert_eq!(fake.calls().len(), 6);
    }
}
