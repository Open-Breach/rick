//! From the paths a tool prints to the hits a query asked for.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use ext::platform::fs::{RawEntry, RawKind};
use ext::{Filter, Query, SearchHit};
use types::{Attrs, Entry, EntryId, EntryKind, ProviderId};

/// The provider every hit belongs to: the local filesystem.
pub const PROVIDER_ID: ProviderId = ProviderId("local");

/// The hits for a batch of output lines: each line `path_of_line` reads
/// as a path, that is still on disk, lies under the query's root and
/// passes its filters.
#[must_use]
pub fn for_lines(
    lines: &[String],
    query: &Query,
    path_of_line: fn(&str) -> Option<PathBuf>,
) -> Vec<SearchHit> {
    lines
        .iter()
        .filter_map(|line| path_of_line(line))
        .filter_map(|path| hit_for(&path, query))
        .collect()
}

/// The hit for `path`, when it is on disk and the query admits it. The
/// filters that look at the path's components look below the query's
/// root when it has one, as a walk from the root would.
#[must_use]
pub fn hit_for(path: &Path, query: &Query) -> Option<SearchHit> {
    let root = match &query.root {
        Some(root) => Some(ext::platform::paths::to_path(&root.path).ok()?),
        None => None,
    };
    let below = match &root {
        Some(root) => path.strip_prefix(root).ok()?,
        None => path,
    };
    let entry = match entry_at(path) {
        Ok(entry) => entry,
        Err(error) => {
            tracing::debug!(path = %path.display(), %error, "hit not on disk; skipped");
            return None;
        }
    };
    admits(below, &entry, &query.filters).then_some(SearchHit {
        entry,
        snippet: None,
    })
}

/// The entry at `path`: what the local provider would list for it, read
/// through the platform's `stat`.
///
/// # Errors
///
/// The I/O error from examining the path.
pub fn entry_at(path: &Path) -> std::io::Result<Entry> {
    let RawEntry {
        name,
        id,
        kind,
        size,
        created,
        modified,
        accessed,
        attrs,
    } = ext::platform::fs::stat(path)?;
    let kind = match kind {
        RawKind::File => EntryKind::File,
        RawKind::Dir => EntryKind::Dir,
        RawKind::Other => EntryKind::Other,
        RawKind::Symlink => EntryKind::Symlink {
            target: std::fs::read_link(path)
                .map(|target| ext::platform::paths::from_path(&target))
                .unwrap_or_default(),
            broken: std::fs::metadata(path).is_err(),
        },
    };
    Ok(Entry {
        id: EntryId::new(PROVIDER_ID, id),
        name: name.to_string_lossy().into_owned(),
        path: ext::platform::paths::from_path(path),
        kind,
        size,
        created,
        modified,
        accessed,
        attrs,
        mime: None,
    })
}

/// Whether the entry passes every filter, `path` being its path below
/// the query's root (or the whole path without one). The index itself
/// searched by name or content; these narrow what it found.
#[must_use]
pub fn admits(path: &Path, entry: &Entry, filters: &[Filter]) -> bool {
    filters.iter().all(|filter| match filter {
        Filter::Extension(ext) => Path::new(&entry.name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(OsStr::new(ext))),
        Filter::MinSize(min) => entry.size.is_some_and(|s| s >= *min),
        Filter::MaxSize(max) => entry.size.is_some_and(|s| s <= *max),
        Filter::ModifiedAfter(after) => entry.modified.is_some_and(|m| m > *after),
        Filter::ModifiedBefore(before) => entry.modified.is_some_and(|m| m < *before),
        Filter::NoHidden => {
            !entry.attrs.contains(Attrs::HIDDEN)
                && !path
                    .components()
                    .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        }
        Filter::SkipNames(names) => !path
            .components()
            .any(|c| names.iter().any(|n| c.as_os_str() == OsStr::new(n))),
    })
}
