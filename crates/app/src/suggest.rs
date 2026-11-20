//! Suggestions (specs/plugin-distribution.md §8): which plugin that is
//! not there would serve what the user is looking at.
//!
//! A suggestion is made from two things Rick holds already, and from
//! nothing else: the plugin list it has cached
//! ([`App::plugin_registry`](crate::App::plugin_registry)) and the
//! manifests of the plugins in the plugins folders
//! ([`App::plugins`](crate::App::plugins)). Nothing here reads a file or
//! asks anybody: a file that is selected, a folder that is entered and a
//! side panel that is drawn never make a request.
//!
//! - **Who is suggested.** A plugin of the list with a build that fits
//!   this Rick and that is in neither plugins folder, with **Install**; and
//!   a plugin that is in a plugins folder and disabled, from its own
//!   manifest and with no list at all, with **Enable**
//!   ([`Remedy`]). A plugin on the disk comes before one of the list.
//! - **For what.** Its triggers (§5): a file whose name ends in one of
//!   its `previews` ([`App::suggestion_for_file`]), a folder whose listing
//!   holds an entry named in its `markers`
//!   ([`App::suggestion_for_folder`]), and the kinds of connection it adds
//!   to a section of the side panel ([`App::suggested_connections`]).
//! - **Where.** In the preview pane, for the file a pane's preview
//!   follows while no loaded previewer claims it but the one of last
//!   resort, or none at all ([`App::preview_suggestion`]): the hex view
//!   then has the bar above it, and the *No previewer* card the button.
//!   In the file list, for the folder of the local file system a pane
//!   shows, when the listing the pane has loaded holds an entry that is
//!   a plugin's marker ([`App::folder_suggestion`]). The application
//!   notes such entries as the listing arrives and changes — a name held
//!   against a handful of markers, so a folder of 200,000 entries costs
//!   nothing that can be measured — and looks at no file for it.
//! - **When not.** With *Suggest plugins* off (`[plugins] suggest`), for a
//!   plugin *Don't suggest again* silenced (`[plugins] no_suggest`,
//!   [`Command::SuggestNever`](crate::Command::SuggestNever)), and for one
//!   *Not now* silenced until Rick is started again
//!   ([`Command::SuggestNotNow`](crate::Command::SuggestNotNow)).
//!   [`Command::SuggestAgain`](crate::Command::SuggestAgain) takes both
//!   back.

use std::collections::{BTreeMap, BTreeSet};

use ext::{ConnectionSection, Depth, FsChange};
use model::{PaneId, TabId};
use plugins_registry::Offer;
use types::{Entry, Location};

use crate::{App, ListKey, Plugin, PluginStatus};

/// A kind of connection a plugin adds to *Add…* in the side panel, as its
/// manifest lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionKind {
    /// The section whose *Add…* lists the kind.
    pub section: ConnectionSection,
    /// The kind's title there, *Amazon S3*.
    pub title: String,
}

/// What a suggestion offers to do about its plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remedy {
    /// The plugin list has a build that fits this Rick and the plugin is
    /// in neither plugins folder: **Install**, a package of `size` bytes.
    Install {
        /// The size of the package in bytes.
        size: u64,
        /// Its install is running: the button waits for it.
        installing: bool,
    },
    /// The plugin is in a plugins folder and disabled: **Enable**.
    Enable,
}

/// A plugin that is not there and would serve what the user is looking
/// at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The extension id.
    pub id: String,
    /// The plugin's name; its id when nothing names it.
    pub name: String,
    /// What it does, in one line; empty when nothing says.
    pub description: String,
    /// What there is to do about it.
    pub remedy: Remedy,
}

/// A kind of connection a suggested plugin would add to a section's
/// *Add…*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestedConnection {
    /// The kind's title, *Amazon S3*.
    pub title: String,
    /// The plugin that adds it.
    pub plugin: Suggestion,
}

/// What the application keeps for the suggestions in this run of Rick.
#[derive(Debug, Default)]
pub(crate) struct Suggestions {
    /// The plugins *Not now* silenced: not suggested until Rick is started
    /// again.
    not_now: BTreeSet<String>,
    /// What the listing a pane has loaded holds of the plugins' markers,
    /// for the panes that show a folder of the local file system.
    folders: BTreeMap<PaneId, Folder>,
}

/// The folder a pane shows and the markers in its loaded listing.
#[derive(Debug)]
struct Folder {
    /// The tab the listing is for.
    tab: TabId,
    /// The folder.
    location: Location,
    /// The names of the listing's entries that are a marker of some
    /// plugin, of the plugin list or of a plugins folder, whether that
    /// plugin may be suggested now or not.
    found: BTreeSet<String>,
}

/// The suffixes of `name` after each of its dots, lower case: `tar.gz` and
/// `gz` for `a.tar.gz`. A plugin claims a file by any of them.
pub(crate) fn suffixes(name: &str) -> Vec<String> {
    let lower = name.to_ascii_lowercase();
    lower
        .char_indices()
        .filter(|(_, c)| *c == '.')
        .map(|(i, _)| lower[i + 1..].to_owned())
        .filter(|suffix| !suffix.is_empty())
        .collect()
}

/// Whether `claimed`, the file name extensions a plugin's previewers
/// claim, hold one of `suffixes`.
pub(crate) fn claims(claimed: &[String], suffixes: &[String]) -> bool {
    claimed
        .iter()
        .any(|claimed| suffixes.iter().any(|s| claimed.eq_ignore_ascii_case(s)))
}

/// Where a candidate's triggers are read from.
#[derive(Clone, Copy)]
enum Triggers<'a> {
    /// The manifest of a plugin on the disk.
    Manifest(&'a Plugin),
    /// The plugin list.
    List(&'a plugins_registry::Triggers),
}

/// A plugin that may be suggested, with what it is for.
#[derive(Clone, Copy)]
struct Candidate<'a> {
    id: &'a str,
    name: &'a str,
    description: &'a str,
    remedy: Remedy,
    triggers: Triggers<'a>,
}

impl Candidate<'_> {
    fn previews(&self) -> &[String] {
        match self.triggers {
            Triggers::Manifest(plugin) => &plugin.previews,
            Triggers::List(triggers) => &triggers.previews,
        }
    }

    fn markers(&self) -> &[String] {
        match self.triggers {
            Triggers::Manifest(plugin) => &plugin.markers,
            Triggers::List(triggers) => &triggers.markers,
        }
    }

    /// The titles of the kinds of connection it adds to `section`.
    fn connections(&self, section: ConnectionSection) -> Vec<&str> {
        match self.triggers {
            Triggers::Manifest(plugin) => plugin
                .connections
                .iter()
                .filter(|kind| kind.section == section)
                .map(|kind| kind.title.as_str())
                .collect(),
            Triggers::List(triggers) => triggers
                .connections
                .iter()
                .filter(|kind| ConnectionSection::named(&kind.section) == Some(section))
                .map(|kind| kind.title.as_str())
                .collect(),
        }
    }

    fn suggestion(&self) -> Suggestion {
        Suggestion {
            id: self.id.to_owned(),
            name: if self.name.is_empty() {
                self.id.to_owned()
            } else {
                self.name.to_owned()
            },
            description: self.description.to_owned(),
            remedy: self.remedy,
        }
    }
}

impl App {
    /// Whether the user silenced the suggestions of the plugin `id`: with
    /// *Don't suggest again* (`[plugins] no_suggest`), or with *Not now*
    /// in this run of Rick. The Extensions dialogue says *Not suggested*
    /// of such a plugin and offers **Suggest again**.
    #[must_use]
    pub fn suggestion_silenced(&self, id: &str) -> bool {
        self.suggestions.not_now.contains(id)
            || self
                .model
                .settings
                .plugins
                .no_suggest
                .iter()
                .any(|silenced| silenced == id)
    }

    /// The plugins that may be suggested now, those on the disk first:
    /// the disabled plugins of the plugins folders, then the plugins of
    /// the list that fit this Rick and are in neither folder, in the
    /// list's order. None with *Suggest plugins* off, and none that the
    /// user silenced.
    fn candidates(&self) -> Vec<Candidate<'_>> {
        let settings = &self.model.settings.plugins;
        if !settings.suggest {
            return Vec::new();
        }
        // Disabled as the host found it and as the settings still have
        // it: one the user has switched on again waits for the next start
        // and needs no suggestion.
        let disabled = self
            .plugins
            .iter()
            .filter(|row| !row.removed && row.status == PluginStatus::Disabled)
            .filter(|row| settings.disabled.contains(&row.id))
            .map(|row| Candidate {
                id: &row.id,
                name: &row.name,
                description: &row.description,
                remedy: Remedy::Enable,
                triggers: Triggers::Manifest(row),
            });
        let run = &self.plugin_registry.view;
        let offered = match run
            .index
            .as_deref()
            .map(|index| index.offer(&run.platform, &run.rick))
        {
            Some(Offer::Plugins(offered)) => offered,
            // No list, or one that has nothing for this Rick.
            Some(Offer::PreRelease | Offer::TooOld) | None => Vec::new(),
        };
        let listed = offered
            .into_iter()
            .filter(|offered| {
                !self
                    .plugins
                    .iter()
                    .any(|row| row.id == offered.plugin.id && !row.removed)
            })
            .map(|offered| Candidate {
                id: &offered.plugin.id,
                name: &offered.plugin.name,
                description: &offered.plugin.description,
                remedy: Remedy::Install {
                    size: offered.build.size,
                    installing: self.plugin_installing(&offered.plugin.id),
                },
                triggers: Triggers::List(&offered.plugin.meta.triggers),
            });
        disabled
            .chain(listed)
            .filter(|candidate| !self.suggestion_silenced(candidate.id))
            .collect()
    }

    /// The plugin suggested for a file named `name`: one whose `previews`
    /// hold the file's extension, by any suffix after a dot and whatever
    /// the case, so `a.tar.gz` finds a plugin that claims `tar.gz` or
    /// `gz`. Whether a loaded previewer shows the file already is the
    /// caller's to know.
    #[must_use]
    pub fn suggestion_for_file(&self, name: &str) -> Option<Suggestion> {
        let suffixes = suffixes(name);
        if suffixes.is_empty() {
            return None;
        }
        self.candidates()
            .iter()
            .find(|candidate| claims(candidate.previews(), &suffixes))
            .map(Candidate::suggestion)
    }

    /// The plugin suggested for the file the preview of `pane` shows
    /// (specs/plugin-distribution.md §8, *The preview pane*): one whose
    /// `previews` hold the file's extension, while no loaded previewer
    /// claims the file but the one of last resort — the hex view shows
    /// it — or none claims it at all and the pane shows the *No
    /// previewer* card. `None` while a previewer of the file's own shows
    /// it, for an empty file, which is given to no previewer, and until
    /// the preview has opened.
    #[must_use]
    pub fn preview_suggestion(&self, pane: model::PaneId) -> Option<Suggestion> {
        let job = self.preview.get(&pane).filter(|job| job.unclaimed)?;
        self.suggestion_for_file(&job.full.name)
    }

    /// The plugin suggested for the folder `pane` shows
    /// (specs/plugin-distribution.md §8, *The file list*): the folder is
    /// one of the local file system, and the listing the pane has loaded
    /// — hidden entries included — holds an entry named in the plugin's
    /// `markers`. `None` for a pane that shows search results or a
    /// flattened listing, and as soon as the pane has left the folder.
    #[must_use]
    pub fn folder_suggestion(&self, pane: PaneId) -> Option<Suggestion> {
        let folder = self.suggestions.folders.get(&pane)?;
        if folder.found.is_empty() {
            return None;
        }
        // Still there: a pane that moved on has no bar, whatever the
        // listing it left behind held.
        let (tab, shown) = self.model.active_tab(pane)?;
        if tab != folder.tab || shown.location != folder.location {
            return None;
        }
        self.suggestion_for_folder(folder.found.iter().map(String::as_str))
    }

    /// The names that make a folder some plugin's business: the `markers`
    /// of the plugin list and of the manifests in the plugins folders,
    /// whether the plugin may be suggested now or not.
    fn marker_names(&self) -> Vec<&str> {
        let listed = self
            .plugin_registry
            .view
            .index
            .iter()
            .flat_map(|index| index.plugins())
            .flat_map(|plugin| &plugin.meta.triggers.markers);
        let on_disk = self.plugins.iter().flat_map(|row| &row.markers);
        listed.chain(on_disk).map(String::as_str).collect()
    }

    /// The names of `entries` that are a plugin's marker. A name is held
    /// against a handful of markers: nothing of a large listing is kept.
    fn markers_in<'a>(&self, entries: impl IntoIterator<Item = &'a Entry>) -> Vec<String> {
        let markers = self.marker_names();
        if markers.is_empty() {
            return Vec::new();
        }
        entries
            .into_iter()
            .filter(|entry| markers.contains(&entry.name.as_str()))
            .map(|entry| entry.name.clone())
            .collect()
    }

    /// A pane's listing starts for `key`. A folder of the local file
    /// system, listed as itself, is one whose markers are noted; search
    /// results, a flattened listing and every other provider's are not.
    /// With `reconcile` the listing that is shown stays until the new one
    /// is complete, and so do the markers noted for it, when it is the
    /// same folder in the same tab.
    pub(crate) fn suggest_listing(&mut self, pane: PaneId, key: &ListKey, reconcile: bool) {
        let noted = key.search.is_none()
            && key.opts.depth == Depth::One
            && key.location.provider == vfs_local::PROVIDER_ID;
        if !noted {
            self.suggestions.folders.remove(&pane);
            return;
        }
        let same = self
            .suggestions
            .folders
            .get(&pane)
            .is_some_and(|folder| folder.tab == key.tab && folder.location == key.location);
        if !(same && reconcile) {
            self.suggestions.folders.insert(
                pane,
                Folder {
                    tab: key.tab,
                    location: key.location.clone(),
                    found: BTreeSet::new(),
                },
            );
        }
    }

    /// A batch of a pane's listing arrived.
    pub(crate) fn suggest_listed(&mut self, pane: PaneId, batch: &[Entry]) {
        if !self.suggestions.folders.contains_key(&pane) {
            return;
        }
        let found = self.markers_in(batch);
        if let Some(folder) = self.suggestions.folders.get_mut(&pane) {
            folder.found.extend(found);
        }
    }

    /// A pane's listing was replaced by `entries` in one step, as a
    /// refresh ends.
    pub(crate) fn suggest_relisted(&mut self, pane: PaneId, entries: &[Entry]) {
        if !self.suggestions.folders.contains_key(&pane) {
            return;
        }
        let found = self.markers_in(entries);
        if let Some(folder) = self.suggestions.folders.get_mut(&pane) {
            folder.found = found.into_iter().collect();
        }
    }

    /// The folder a pane shows is about to change by `change`, which the
    /// listing has not taken yet: an entry that goes is still in it.
    pub(crate) fn suggest_change(&mut self, pane: PaneId, change: &FsChange) {
        if !self.suggestions.folders.contains_key(&pane) {
            return;
        }
        let (gone, came) = match change {
            FsChange::Created(entry) | FsChange::Modified(entry) => (None, Some(entry)),
            FsChange::Removed(id) => (Some(*id), None),
            FsChange::Renamed { from, to } => (Some(*from), Some(to)),
            // The pane lists again, and the new listing is looked at.
            FsChange::Rescan => (None, None),
        };
        let gone = gone.and_then(|id| {
            let (_, tab) = self.model.active_tab(pane)?;
            tab.listing.get(id).map(|entry| entry.name.clone())
        });
        let came = self.markers_in(came);
        if let Some(folder) = self.suggestions.folders.get_mut(&pane) {
            if let Some(gone) = gone {
                folder.found.remove(&gone);
            }
            folder.found.extend(came);
        }
    }

    /// The markers changed — a plugin list arrived, or the plugin host's
    /// rows did: the listings the panes have loaded are looked at again.
    pub(crate) fn suggest_markers_changed(&mut self) {
        let panes: Vec<(PaneId, TabId)> = self
            .suggestions
            .folders
            .iter()
            .map(|(pane, folder)| (*pane, folder.tab))
            .collect();
        for (pane, tab) in panes {
            let found = match self.model.active_tab(pane) {
                Some((shown, state)) if shown == tab => self.markers_in(state.listing.entries()),
                _ => Vec::new(),
            };
            if let Some(folder) = self.suggestions.folders.get_mut(&pane) {
                folder.found = found.into_iter().collect();
            }
        }
    }

    /// The plugin suggested for a folder whose listing holds entries of
    /// the names `names`: one with an entry's name among its `markers`,
    /// compared as whole names, case as written.
    #[must_use]
    pub fn suggestion_for_folder<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Option<Suggestion> {
        let candidates = self.candidates();
        if candidates.iter().all(|c| c.markers().is_empty()) {
            return None;
        }
        // The first candidate with a marker in the folder, not the plugin
        // of the first marker the listing happens to hold.
        let mut found: Option<usize> = None;
        for name in names {
            let hit = candidates
                .iter()
                .position(|c| c.markers().iter().any(|marker| marker == name));
            found = match (found, hit) {
                (Some(found), Some(hit)) => Some(found.min(hit)),
                (found, hit) => found.or(hit),
            };
            if found == Some(0) {
                break;
            }
        }
        found.map(|index| candidates[index].suggestion())
    }

    /// The kinds of connection the suggested plugins would add to
    /// `section` of the side panel, in the order the plugins are
    /// suggested in: what follows the kinds that are there in the
    /// section's *Add…*.
    #[must_use]
    pub fn suggested_connections(&self, section: ConnectionSection) -> Vec<SuggestedConnection> {
        self.candidates()
            .iter()
            .flat_map(|candidate| {
                let plugin = candidate.suggestion();
                candidate
                    .connections(section)
                    .into_iter()
                    .map(move |title| SuggestedConnection {
                        title: title.to_owned(),
                        plugin: plugin.clone(),
                    })
            })
            .collect()
    }

    /// **Not now** on a suggestion: the plugin `id` is not suggested again
    /// until Rick is started again.
    pub(crate) fn suggest_not_now(&mut self, id: String) {
        self.suggestions.not_now.insert(id);
    }

    /// **Don't suggest again**: the plugin `id` is written to
    /// `[plugins] no_suggest` and never suggested.
    pub(crate) fn suggest_never(&mut self, id: String) {
        self.change_settings(model::SettingsChange::NoSuggest { id, silenced: true });
    }

    /// **Suggest again** of the Extensions dialogue: the plugin `id` is
    /// taken off `[plugins] no_suggest` and off the plugins *Not now*
    /// silenced.
    pub(crate) fn suggest_again(&mut self, id: String) {
        self.suggestions.not_now.remove(&id);
        if self.model.settings.plugins.no_suggest.contains(&id) {
            self.change_settings(model::SettingsChange::NoSuggest {
                id,
                silenced: false,
            });
        }
    }
}
