//! A tab's listing: the entries a provider streamed, and the view over them
//! (filtered, sorted, grouped) that the list shows.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use types::{Attrs, Entry, EntryId, EntryKind};

/// Views above this many rows sort on the compute pool.
const PARALLEL_SORT_FROM: usize = 4096;

/// How a tab shows its entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    /// Name and icon only, in wrapping columns.
    List,
    /// Sortable columns; the default.
    Details,
    /// Expandable folders inline.
    Tree,
    /// A grid of icons or thumbnails.
    Icons(IconSize),
}

/// Cell size of the icon view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IconSize {
    /// Small.
    Small,
    /// Medium.
    Medium,
    /// Large.
    Large,
    /// Extra large.
    ExtraLarge,
}

/// A sortable, groupable column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Column {
    /// The entry name.
    Name,
    /// The file type as text.
    Type,
    /// Last modification time.
    Modified,
    /// Creation time.
    Created,
    /// Last access time.
    Accessed,
    /// Size in bytes.
    Size,
    /// Entry count of folders.
    Items,
}

impl Column {
    /// Every column, in default display order.
    pub const ALL: [Column; 7] = [
        Column::Name,
        Column::Type,
        Column::Modified,
        Column::Created,
        Column::Accessed,
        Column::Size,
        Column::Items,
    ];
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDir {
    /// A to Z, small to large, old to new.
    Ascending,
    /// The reverse.
    Descending,
}

/// A column and a direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    /// The column sorted on.
    pub by: Column,
    /// The direction.
    pub dir: SortDir,
}

/// What the view shows and in which order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewSpec<'a> {
    /// Sort column and direction.
    pub sort: Sort,
    /// Folders above files whatever the sort.
    pub folders_first: bool,
    /// Name must contain this, case-insensitively; empty shows all.
    pub filter: &'a str,
    /// Show entries flagged hidden.
    pub show_hidden: bool,
    /// Show entries flagged system.
    pub show_system: bool,
}

/// A group header and the view rows under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The header text.
    pub label: String,
    /// Indices into the entries, in view order.
    pub rows: Vec<usize>,
}

/// The entries of one location and the view over them.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    entries: Vec<Entry>,
    index: HashMap<EntryId, usize>,
    /// Indices into `entries`, filtered and sorted.
    view: Vec<usize>,
    /// Whether the provider has delivered the last batch.
    pub complete: bool,
}

impl Listing {
    /// An empty, incomplete listing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets everything; the listing is incomplete again.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
        self.view.clear();
        self.complete = false;
    }

    /// Every entry known, in arrival order.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Number of entries known, visible or not.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when nothing is known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry with this id, if known.
    #[must_use]
    pub fn get(&self, id: EntryId) -> Option<&Entry> {
        self.index.get(&id).map(|&i| &self.entries[i])
    }

    /// The visible entries in view order.
    pub fn rows(&self) -> impl Iterator<Item = &Entry> + '_ {
        self.view.iter().map(|&i| &self.entries[i])
    }

    /// The visible row at `index` in view order.
    #[must_use]
    pub fn row(&self, index: usize) -> Option<&Entry> {
        self.view.get(index).map(|&i| &self.entries[i])
    }

    /// Ids of the visible entries in view order.
    #[must_use]
    pub fn row_ids(&self) -> Vec<EntryId> {
        self.rows().map(|e| e.id).collect()
    }

    /// Number of visible rows.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.view.len()
    }

    /// Adds a batch from the provider. Entries already known by id are
    /// replaced. The visible ones are merged into the view in order, which
    /// costs one pass over the view rather than a sort.
    pub fn extend(&mut self, batch: Vec<Entry>, spec: &ViewSpec<'_>) {
        let mut fresh = Vec::with_capacity(batch.len());
        for entry in batch {
            if let Some(&existing) = self.index.get(&entry.id) {
                self.entries[existing] = entry;
                if passes(&self.entries[existing], spec) && !self.view.contains(&existing) {
                    fresh.push(existing);
                }
                continue;
            }
            let position = self.entries.len();
            self.index.insert(entry.id, position);
            let visible = passes(&entry, spec);
            self.entries.push(entry);
            if visible {
                fresh.push(position);
            }
        }
        if fresh.is_empty() {
            return;
        }
        let entries = &self.entries;
        fresh.sort_by(|&a, &b| compare(&entries[a], &entries[b], spec));
        let mut merged = Vec::with_capacity(self.view.len() + fresh.len());
        let (mut i, mut j) = (0, 0);
        while i < self.view.len() && j < fresh.len() {
            if compare(&entries[self.view[i]], &entries[fresh[j]], spec) == Ordering::Greater {
                merged.push(fresh[j]);
                j += 1;
            } else {
                merged.push(self.view[i]);
                i += 1;
            }
        }
        merged.extend_from_slice(&self.view[i..]);
        merged.extend_from_slice(&fresh[j..]);
        self.view = merged;
    }

    /// Recomputes the view from scratch, after the sort, filter or hidden
    /// settings changed. Large views sort on the compute pool.
    pub fn refresh_view(&mut self, spec: &ViewSpec<'_>) {
        let entries = &self.entries;
        let mut view: Vec<usize> = (0..entries.len())
            .filter(|&i| passes(&entries[i], spec))
            .collect();
        if view.len() >= PARALLEL_SORT_FROM {
            runtime::compute::pool()
                .install(|| view.par_sort_by(|&a, &b| compare(&entries[a], &entries[b], spec)));
        } else {
            view.sort_by(|&a, &b| compare(&entries[a], &entries[b], spec));
        }
        self.view = view;
    }

    /// Inserts or replaces one entry, keeping the view in order.
    pub fn upsert(&mut self, entry: Entry, spec: &ViewSpec<'_>) {
        if self.index.contains_key(&entry.id) {
            self.remove(entry.id);
        }
        let position = self.entries.len();
        self.index.insert(entry.id, position);
        let visible = passes(&entry, spec);
        self.entries.push(entry);
        if visible {
            let entries = &self.entries;
            let at = self
                .view
                .binary_search_by(|&i| compare(&entries[i], &entries[position], spec))
                .unwrap_or_else(|at| at);
            self.view.insert(at, position);
        }
    }

    /// Removes one entry. Returns whether it was known.
    pub fn remove(&mut self, id: EntryId) -> bool {
        let Some(position) = self.index.remove(&id) else {
            return false;
        };
        let last = self.entries.len() - 1;
        self.entries.swap_remove(position);
        self.view.retain(|&i| i != position);
        if position != last {
            // The former last entry now lives at `position`.
            let moved = self.entries[position].id;
            self.index.insert(moved, position);
            for i in &mut self.view {
                if *i == last {
                    *i = position;
                }
            }
        }
        true
    }

    /// The visible rows under headers for `by`. `now` decides the date
    /// buckets. Rows keep their view order within a group.
    #[must_use]
    pub fn groups(&self, by: Column, now: SystemTime) -> Vec<Group> {
        let mut groups: Vec<Group> = Vec::new();
        for &i in &self.view {
            let label = group_label(&self.entries[i], by, now);
            match groups.iter_mut().find(|g| g.label == label) {
                Some(group) => group.rows.push(i),
                None => groups.push(Group {
                    label,
                    rows: vec![i],
                }),
            }
        }
        groups
    }
}

fn passes(entry: &Entry, spec: &ViewSpec<'_>) -> bool {
    if !spec.show_hidden && entry.attrs.contains(Attrs::HIDDEN) {
        return false;
    }
    if !spec.show_system && entry.attrs.contains(Attrs::SYSTEM) {
        return false;
    }
    spec.filter.is_empty() || contains_ignore_case(&entry.name, spec.filter)
}

/// `haystack` contains `needle` ignoring case.
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_ascii() && haystack.is_ascii() {
        let needle = needle.as_bytes();
        return haystack
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle));
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn is_dir(entry: &Entry) -> bool {
    entry.kind == EntryKind::Dir
}

/// The type text of an entry, as the Type column shows it.
#[must_use]
pub fn type_of(entry: &Entry) -> String {
    match &entry.kind {
        EntryKind::Dir => "Folder".to_owned(),
        EntryKind::Symlink { .. } => "Link".to_owned(),
        EntryKind::Other => "Special".to_owned(),
        EntryKind::File => entry
            .name
            .rsplit_once('.')
            .filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty())
            .map_or_else(|| "File".to_owned(), |(_, ext)| ext.to_uppercase()),
    }
}

/// Case-insensitive name order, with case as the tie-break so equal names
/// keep a stable order.
fn name_cmp(a: &str, b: &str) -> Ordering {
    let lower = a
        .chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase));
    lower.then_with(|| a.cmp(b))
}

fn compare(a: &Entry, b: &Entry, spec: &ViewSpec<'_>) -> Ordering {
    if spec.folders_first {
        match (is_dir(a), is_dir(b)) {
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            _ => {}
        }
    }
    let by_column = match spec.sort.by {
        Column::Name => Ordering::Equal,
        Column::Type => type_of(a).cmp(&type_of(b)),
        Column::Modified => a.modified.cmp(&b.modified),
        Column::Created => a.created.cmp(&b.created),
        Column::Accessed => a.accessed.cmp(&b.accessed),
        // Folders have no size and no provider counts items yet; both sort
        // as unknown, which keeps them together.
        Column::Size | Column::Items => a.size.cmp(&b.size),
    };
    let by_column = match spec.sort.dir {
        SortDir::Ascending => by_column,
        SortDir::Descending => by_column.reverse(),
    };
    let by_name = name_cmp(&a.name, &b.name);
    let by_name = match (spec.sort.by, spec.sort.dir) {
        (Column::Name, SortDir::Descending) => by_name.reverse(),
        _ => by_name,
    };
    by_column.then(by_name).then_with(|| a.id.cmp(&b.id))
}

fn group_label(entry: &Entry, by: Column, now: SystemTime) -> String {
    match by {
        Column::Name => entry
            .name
            .chars()
            .next()
            .filter(char::is_ascii_alphabetic)
            .map_or_else(|| "#".to_owned(), |c| c.to_ascii_uppercase().to_string()),
        Column::Type => type_of(entry),
        Column::Modified => date_bucket(entry.modified, now),
        Column::Created => date_bucket(entry.created, now),
        Column::Accessed => date_bucket(entry.accessed, now),
        Column::Size | Column::Items => {
            if is_dir(entry) {
                return "Folders".to_owned();
            }
            match entry.size {
                None => "Unknown",
                Some(0) => "Empty",
                Some(n) if n < 16 << 10 => "Tiny",
                Some(n) if n < 1 << 20 => "Small",
                Some(n) if n < 128 << 20 => "Medium",
                Some(n) if n < 1 << 30 => "Large",
                Some(_) => "Huge",
            }
            .to_owned()
        }
    }
}

fn date_bucket(time: Option<SystemTime>, now: SystemTime) -> String {
    const DAY: Duration = Duration::from_hours(24);
    let Some(time) = time else {
        return "Unknown".to_owned();
    };
    let day = |t: SystemTime| {
        t.duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() / DAY.as_secs())
    };
    let (today, that) = (day(now), day(time));
    if that >= today {
        "Today"
    } else if that + 1 == today {
        "Yesterday"
    } else if that + 7 > today {
        "This week"
    } else if that + 31 > today {
        "This month"
    } else {
        "Earlier"
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{ProviderId, VPath};

    fn entry(id: u128, name: &str, kind: EntryKind, size: Option<u64>, modified: u64) -> Entry {
        Entry {
            id: EntryId::new(ProviderId("t"), id),
            name: name.to_owned(),
            path: VPath::from(format!("/{name}")),
            kind,
            size,
            created: None,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(modified)),
            accessed: None,
            attrs: if name.starts_with('.') {
                Attrs::HIDDEN
            } else {
                Attrs::empty()
            },
            mime: None,
        }
    }

    fn spec(sort: Sort, filter: &str) -> ViewSpec<'_> {
        ViewSpec {
            sort,
            folders_first: true,
            filter,
            show_hidden: false,
            show_system: false,
        }
    }

    fn by(by: Column, dir: SortDir) -> Sort {
        Sort { by, dir }
    }

    fn sample() -> Vec<Entry> {
        vec![
            entry(1, "zeta.txt", EntryKind::File, Some(10), 300),
            entry(2, "Alpha.rs", EntryKind::File, Some(3000), 100),
            entry(3, "docs", EntryKind::Dir, None, 200),
            entry(4, ".hidden", EntryKind::File, Some(1), 400),
            entry(5, "beta.txt", EntryKind::File, Some(20), 500),
        ]
    }

    fn names(listing: &Listing) -> Vec<&str> {
        listing.rows().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn batches_merge_into_a_sorted_filtered_view() {
        let mut listing = Listing::new();
        let spec = spec(by(Column::Name, SortDir::Ascending), "");
        let mut all = sample();
        let second = all.split_off(3);
        listing.extend(all, &spec);
        assert_eq!(names(&listing), ["docs", "Alpha.rs", "zeta.txt"]);
        listing.extend(second, &spec);
        assert_eq!(
            names(&listing),
            ["docs", "Alpha.rs", "beta.txt", "zeta.txt"]
        );
        assert_eq!(listing.len(), 5, "hidden entries are known but not shown");
        assert_eq!(listing.row_count(), 4);
    }

    #[test]
    fn every_column_sorts_both_ways_with_folders_first() {
        let mut listing = Listing::new();
        listing.extend(sample(), &spec(by(Column::Name, SortDir::Ascending), ""));
        let mut s = spec(by(Column::Name, SortDir::Descending), "");
        listing.refresh_view(&s);
        assert_eq!(
            names(&listing),
            ["docs", "zeta.txt", "beta.txt", "Alpha.rs"]
        );
        s.sort = by(Column::Size, SortDir::Ascending);
        listing.refresh_view(&s);
        assert_eq!(
            names(&listing),
            ["docs", "zeta.txt", "beta.txt", "Alpha.rs"]
        );
        s.sort = by(Column::Size, SortDir::Descending);
        listing.refresh_view(&s);
        assert_eq!(
            names(&listing),
            ["docs", "Alpha.rs", "beta.txt", "zeta.txt"]
        );
        s.sort = by(Column::Modified, SortDir::Ascending);
        listing.refresh_view(&s);
        assert_eq!(
            names(&listing),
            ["docs", "Alpha.rs", "zeta.txt", "beta.txt"]
        );
        s.sort = by(Column::Type, SortDir::Ascending);
        listing.refresh_view(&s);
        assert_eq!(
            names(&listing),
            ["docs", "Alpha.rs", "beta.txt", "zeta.txt"]
        );
        s.folders_first = false;
        s.sort = by(Column::Name, SortDir::Ascending);
        listing.refresh_view(&s);
        assert_eq!(
            names(&listing),
            ["Alpha.rs", "beta.txt", "docs", "zeta.txt"]
        );
        s.show_hidden = true;
        listing.refresh_view(&s);
        assert_eq!(names(&listing)[0], ".hidden");
    }

    #[test]
    fn filter_is_case_insensitive_and_upsert_remove_keep_order() {
        let mut listing = Listing::new();
        let s = spec(by(Column::Name, SortDir::Ascending), "ETA");
        listing.extend(sample(), &s);
        assert_eq!(names(&listing), ["beta.txt", "zeta.txt"]);

        listing.upsert(entry(6, "Theta.md", EntryKind::File, Some(1), 1), &s);
        assert_eq!(names(&listing), ["beta.txt", "Theta.md", "zeta.txt"]);
        listing.upsert(entry(5, "beta.txt", EntryKind::File, Some(99), 1), &s);
        assert_eq!(
            listing
                .get(EntryId::new(ProviderId("t"), 5))
                .and_then(|e| e.size),
            Some(99)
        );
        assert!(listing.remove(EntryId::new(ProviderId("t"), 1)));
        assert!(!listing.remove(EntryId::new(ProviderId("t"), 1)));
        assert_eq!(names(&listing), ["beta.txt", "Theta.md"]);
        assert_eq!(listing.len(), 5);
        // Every surviving id still resolves to the right entry.
        for e in listing.entries() {
            assert_eq!(listing.get(e.id).map(|x| &x.name), Some(&e.name));
        }
    }

    #[test]
    fn groups_follow_the_spec_buckets() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_hours(40 * 24);
        let mut listing = Listing::new();
        let s = spec(by(Column::Name, SortDir::Ascending), "");
        listing.extend(
            vec![
                entry(1, "a", EntryKind::File, Some(0), 40 * 86_400),
                entry(2, "b", EntryKind::File, Some(5 << 20), 39 * 86_400),
                entry(3, "c", EntryKind::File, Some(2 << 30), 5 * 86_400),
                entry(4, "d", EntryKind::Dir, None, 36 * 86_400),
            ],
            &s,
        );
        let labels = |by| {
            listing
                .groups(by, now)
                .into_iter()
                .map(|g| (g.label, g.rows.len()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            labels(Column::Size),
            [
                ("Folders".into(), 1),
                ("Empty".into(), 1),
                ("Medium".into(), 1),
                ("Huge".into(), 1)
            ]
        );
        assert_eq!(
            labels(Column::Modified),
            [
                ("This week".into(), 1),
                ("Today".into(), 1),
                ("Yesterday".into(), 1),
                ("Earlier".into(), 1)
            ]
        );
        assert_eq!(labels(Column::Name).len(), 4);
        assert_eq!(
            type_of(&entry(9, "x.tar.gz", EntryKind::File, None, 0)),
            "GZ"
        );
        assert_eq!(
            type_of(&entry(9, "Makefile", EntryKind::File, None, 0)),
            "File"
        );
    }

    #[test]
    fn large_views_sort_in_parallel_and_match_the_serial_order() {
        let mut listing = Listing::new();
        let s = spec(by(Column::Size, SortDir::Descending), "");
        let entries: Vec<Entry> = (0..10_000u64)
            .map(|i| {
                entry(
                    u128::from(i),
                    &format!("f{}", (i * 7919) % 10_000),
                    EntryKind::File,
                    Some((i * 31) % 977),
                    i,
                )
            })
            .collect();
        listing.extend(entries, &s);
        let merged = listing.row_ids();
        listing.refresh_view(&s);
        assert_eq!(listing.row_ids(), merged);
        let sizes: Vec<u64> = listing.rows().filter_map(|e| e.size).collect();
        assert!(sizes.windows(2).all(|w| w[0] >= w[1]));
    }
}
