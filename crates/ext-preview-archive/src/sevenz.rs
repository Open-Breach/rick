//! 7z listing: the archive's header, decoded (it is usually LZMA
//! compressed) but no file content; encrypted headers list nothing.

use std::fs::File;
use std::path::Path;

use sevenz_rust2::{Archive, Password};
use types::{EntryKind, Error};

use crate::{Facts, Limit, Listed, RawEntry, invalid};

/// Lists the 7z at `path`.
pub(crate) fn list(path: &Path, limit: &Limit) -> Result<Listed, Error> {
    let mut file = File::open(path).map_err(Error::from_io)?;
    let archive = match Archive::read(&mut file, &Password::empty()) {
        Ok(archive) => archive,
        Err(sevenz_rust2::Error::PasswordRequired | sevenz_rust2::Error::MaybeBadPassword(_)) => {
            return Ok(Listed {
                entries: Vec::new(),
                truncated: false,
                facts: Facts {
                    encrypted_headers: true,
                    ..Facts::default()
                },
            });
        }
        Err(error) => return Err(invalid(format!("7z: {error}"))),
    };
    let mut entries = Vec::with_capacity(archive.files.len().min(limit.max_entries));
    let mut truncated = false;
    for (index, file) in archive.files.iter().enumerate() {
        if limit.reached(entries.len()) {
            truncated = true;
            break;
        }
        if file.is_anti_item {
            continue;
        }
        entries.push(RawEntry {
            path: file.name.clone(),
            kind: if file.is_directory {
                EntryKind::Dir
            } else {
                EntryKind::File
            },
            size: Some(file.size),
            compressed: (file.compressed_size > 0).then_some(file.compressed_size),
            modified: file
                .has_last_modified_date
                .then(|| std::time::SystemTime::from(file.last_modified_date)),
            encrypted: archive.is_file_encrypted(index),
        });
    }
    Ok(Listed {
        entries,
        truncated,
        facts: Facts {
            encrypted_headers: false,
            solid: archive.is_solid,
            rows: Vec::new(),
        },
    })
}
