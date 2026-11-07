//! zip listing: the central directory, read without decompressing.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use types::{EntryKind, Error, VPath};

use crate::{Facts, Limit, Listed, RawEntry, civil_to_system_time, invalid};

/// Lists the zip at `path`.
pub(crate) fn list(path: &Path, limit: &Limit) -> Result<Listed, Error> {
    let file = File::open(path).map_err(Error::from_io)?;
    let mut archive =
        zip::ZipArchive::new(BufReader::new(file)).map_err(|e| invalid(format!("zip: {e}")))?;
    let mut entries = Vec::with_capacity(archive.len().min(limit.max_entries));
    let mut truncated = false;
    for index in 0..archive.len() {
        if limit.reached(entries.len()) {
            truncated = true;
            break;
        }
        let entry = archive
            .by_index_raw(index)
            .map_err(|e| invalid(format!("zip entry {index}: {e}")))?;
        let name = String::from_utf8_lossy(entry.name_raw()).into_owned();
        let kind = if entry.is_dir() {
            EntryKind::Dir
        } else if entry.is_symlink() {
            // The target is the entry's content, which a listing does not
            // read; the link shows as one without a target.
            EntryKind::Symlink {
                target: VPath::from(""),
                broken: false,
            }
        } else {
            EntryKind::File
        };
        let modified = entry.last_modified().and_then(|t| {
            civil_to_system_time(
                i64::from(t.year()),
                u32::from(t.month()),
                u32::from(t.day()),
                u32::from(t.hour()),
                u32::from(t.minute()),
                u32::from(t.second()),
            )
        });
        entries.push(RawEntry {
            path: name,
            kind,
            size: Some(entry.size()),
            compressed: Some(entry.compressed_size()),
            modified,
            encrypted: entry.encrypted(),
        });
    }
    let mut facts = Facts::default();
    let comment = String::from_utf8_lossy(archive.comment()).trim().to_owned();
    if !comment.is_empty() {
        facts
            .rows
            .push(("Comment".to_owned(), comment.chars().take(200).collect()));
    }
    Ok(Listed {
        entries,
        truncated,
        facts,
    })
}
