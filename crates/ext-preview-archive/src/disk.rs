//! Disk-image listings through the archive provider's reader: the 7-Zip
//! library opens the container, the partition table and the file systems
//! inside it (`ext_vfs_archive::diskimage`), and the entries come back as
//! this previewer's [`RawEntry`]s within its [`Limit`]. The library is
//! found through the provider's [`Resolver`], the one setting for both
//! plugins; when it is missing, the error names it and how to install it,
//! and the pane shows that in place of the listing.

use std::path::Path;

use ext::platform::ImageFormat;
use ext_vfs_archive::Resolver;
use ext_vfs_archive::diskimage;
use types::{EntryKind, Error};

use crate::{Facts, Limit, Listed, RawEntry};

/// Lists the disk image at `path` of `format`.
pub(crate) fn list(
    resolver: &Resolver,
    path: &Path,
    format: ImageFormat,
    limit: &Limit,
) -> Result<Listed, Error> {
    let mut entries = Vec::new();
    let mut truncated = false;
    let facts = diskimage::visit(resolver, path, format, &mut |found| {
        if limit.reached(entries.len()) {
            truncated = true;
            return Ok(true);
        }
        let kind = match found.item.kind {
            EntryKind::Dir => EntryKind::Dir,
            EntryKind::Symlink { target, broken } => EntryKind::Symlink { target, broken },
            _ => EntryKind::File,
        };
        entries.push(RawEntry {
            path: String::from_utf8_lossy(&found.path).into_owned(),
            size: found.item.size,
            compressed: None,
            modified: found.item.modified,
            encrypted: false,
            kind,
        });
        Ok(false)
    })?;
    let mut rows = vec![("Layout".to_owned(), facts.layout.join(" → "))];
    if !facts.labels.is_empty() {
        rows.push(("Volume label".to_owned(), facts.labels.join(", ")));
    }
    Ok(Listed {
        entries,
        truncated,
        facts: Facts {
            encrypted_headers: false,
            solid: false,
            rows,
        },
    })
}
