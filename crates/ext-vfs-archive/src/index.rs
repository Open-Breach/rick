//! The index of an archive: every entry with its kind, size and time, and
//! the tree over them, read once per archive and cached.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use types::EntryKind;

use crate::format::Format;

/// One entry of an archive, as its index knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The path inside the archive, `/`-separated, without a leading or
    /// trailing separator; empty for the root.
    pub path: Vec<u8>,
    /// File, folder or link.
    pub kind: EntryKind,
    /// The uncompressed size, when the format records it.
    pub size: Option<u64>,
    /// The modification time, when the format records it.
    pub modified: Option<SystemTime>,
    /// The Unix mode bits, when the format records them.
    pub mode: Option<u32>,
}

impl Item {
    /// A regular file.
    #[must_use]
    pub fn file(path: &[u8], size: u64, modified: Option<SystemTime>) -> Self {
        Self {
            path: normalize(path),
            kind: EntryKind::File,
            size: Some(size),
            modified,
            mode: None,
        }
    }

    /// A folder.
    #[must_use]
    pub fn dir(path: &[u8], modified: Option<SystemTime>) -> Self {
        Self {
            path: normalize(path),
            kind: EntryKind::Dir,
            size: None,
            modified,
            mode: None,
        }
    }

    /// The last component of the path.
    #[must_use]
    pub fn name(&self) -> &[u8] {
        self.path
            .rsplit(|b| *b == b'/')
            .next()
            .unwrap_or(&self.path)
    }
}

/// The path with `\` turned into `/`, a leading `./` or `/` and a trailing
/// `/` dropped, and empty components collapsed.
#[must_use]
pub fn normalize(path: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(path.len());
    for component in path.split(|b| matches!(b, b'/' | b'\\')) {
        if component.is_empty() || component == b"." {
            continue;
        }
        if !out.is_empty() {
            out.push(b'/');
        }
        out.extend_from_slice(component);
    }
    out
}

/// The folder a normalized path is in: empty for a top-level entry.
#[must_use]
pub fn parent(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|b| *b == b'/')
        .map_or(&[][..], |i| &path[..i])
}

/// Whether `path` is `dir` or lies under it; the empty `dir` is the root
/// and holds everything.
#[must_use]
pub fn is_inside(path: &[u8], dir: &[u8]) -> bool {
    if dir.is_empty() {
        return true;
    }
    path.starts_with(dir) && (path.len() == dir.len() || path[dir.len()] == b'/')
}

/// Seconds since the Unix epoch as a time; `None` for zero.
#[must_use]
pub fn unix_time(seconds: u64) -> Option<SystemTime> {
    (seconds > 0).then(|| SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
}

/// A civil date and time, taken as UTC, as a time; `None` when the fields
/// are out of range.
#[must_use]
pub fn civil_time(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<SystemTime> {
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    // Howard Hinnant's days-from-civil.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (i64::from(month) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds =
        days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second);
    let seconds = u64::try_from(seconds).ok()?;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
}

/// The contents of one archive.
#[derive(Debug)]
pub struct Index {
    format: Format,
    items: Vec<Item>,
    by_path: HashMap<Vec<u8>, usize>,
    children: HashMap<Vec<u8>, Vec<usize>>,
}

impl Index {
    /// Builds the index from the entries a format reader found, adding the
    /// folders they imply (a zip may hold `a/b.txt` and no `a/`) and
    /// keeping the first of two entries with one path.
    #[must_use]
    pub fn new(format: Format, items: Vec<Item>) -> Self {
        let mut by_path: HashMap<Vec<u8>, usize> = HashMap::with_capacity(items.len());
        let mut kept: Vec<Item> = Vec::with_capacity(items.len());
        for item in items {
            if item.path.is_empty() || by_path.contains_key(&item.path) {
                continue;
            }
            by_path.insert(item.path.clone(), kept.len());
            kept.push(item);
        }
        // Implicit folders, parents before children.
        let mut implied: Vec<Vec<u8>> = Vec::new();
        for item in &kept {
            let mut dir = parent(&item.path);
            while !dir.is_empty() && !by_path.contains_key(dir) {
                implied.push(dir.to_vec());
                by_path.insert(dir.to_vec(), usize::MAX);
                dir = parent(dir);
            }
        }
        for dir in implied {
            by_path.insert(dir.clone(), kept.len());
            kept.push(Item {
                path: dir,
                kind: EntryKind::Dir,
                size: None,
                modified: None,
                mode: None,
            });
        }
        let mut children: HashMap<Vec<u8>, Vec<usize>> = HashMap::new();
        for (i, item) in kept.iter().enumerate() {
            children
                .entry(parent(&item.path).to_vec())
                .or_default()
                .push(i);
        }
        Self {
            format,
            items: kept,
            by_path,
            children,
        }
    }

    /// The archive's format.
    #[must_use]
    pub fn format(&self) -> Format {
        self.format
    }

    /// Every entry, in archive order with the implied folders last.
    #[must_use]
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The entry at a normalized path.
    #[must_use]
    pub fn get(&self, path: &[u8]) -> Option<&Item> {
        self.by_path.get(path).map(|i| &self.items[*i])
    }

    /// Whether `path` is the root or an entry.
    #[must_use]
    pub fn contains(&self, path: &[u8]) -> bool {
        path.is_empty() || self.by_path.contains_key(path)
    }

    /// The direct children of a folder (the root when `dir` is empty).
    #[must_use]
    pub fn children(&self, dir: &[u8]) -> Vec<&Item> {
        self.children
            .get(dir)
            .map_or_default(|ids| ids.iter().map(|i| &self.items[*i]).collect())
    }

    /// Everything under a folder, at any depth.
    #[must_use]
    pub fn descendants(&self, dir: &[u8]) -> Vec<&Item> {
        self.items
            .iter()
            .filter(|item| item.path != dir && is_inside(&item.path, dir))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_normalize_and_nest() {
        assert_eq!(normalize(b"./a//b\\c/"), b"a/b/c");
        assert_eq!(normalize(b"/"), b"");
        assert_eq!(parent(b"a/b/c"), b"a/b");
        assert_eq!(parent(b"a"), b"");
        assert!(is_inside(b"a/b", b"a"));
        assert!(is_inside(b"a", b"a"));
        assert!(!is_inside(b"ab", b"a"));
        assert!(is_inside(b"x", b""));
        assert_eq!(
            civil_time(1970, 1, 1, 0, 0, 0),
            Some(SystemTime::UNIX_EPOCH)
        );
        assert_eq!(
            civil_time(2000, 3, 1, 12, 30, 7),
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(951_913_807))
        );
        assert_eq!(civil_time(2000, 13, 1, 0, 0, 0), None);
    }

    #[test]
    fn the_index_adds_implied_folders_and_answers_children() {
        let index = Index::new(
            Format::Zip,
            vec![
                Item::file(b"docs/readme.txt", 3, None),
                Item::file(b"docs/deep/x.bin", 1, None),
                Item::dir(b"empty/", None),
                Item::file(b"top.txt", 2, None),
                Item::file(b"top.txt", 9, None),
            ],
        );
        assert_eq!(index.items().len(), 6, "docs and docs/deep are implied");
        let mut top: Vec<&[u8]> = index
            .children(b"")
            .iter()
            .map(|i| i.path.as_slice())
            .collect();
        top.sort();
        assert_eq!(top, [&b"docs"[..], b"empty", b"top.txt"]);
        assert_eq!(
            index.get(b"top.txt").and_then(|i| i.size),
            Some(2),
            "the first wins"
        );
        assert_eq!(
            index.get(b"docs").map(|i| i.kind.clone()),
            Some(EntryKind::Dir)
        );
        assert_eq!(index.children(b"docs/deep").len(), 1);
        assert_eq!(index.descendants(b"docs").len(), 3);
        assert!(index.contains(b""));
        assert!(!index.contains(b"nope"));
        assert_eq!(
            index.get(b"docs/readme.txt").map(Item::name),
            Some(&b"readme.txt"[..])
        );
    }
}
