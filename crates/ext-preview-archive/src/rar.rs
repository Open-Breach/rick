//! RAR listing through `unrar`'s list mode: headers only, no extraction.
//! Encrypted headers (`rar a -hp`) list nothing; encrypted content
//! (`rar a -p`) lists the names and marks the entries.

use std::path::Path;

use types::{EntryKind, Error};

use crate::{Facts, Limit, Listed, RawEntry, dos_to_system_time, invalid};

/// Lists the RAR at `path`.
pub(crate) fn list(path: &Path, limit: &Limit) -> Result<Listed, Error> {
    let archive = unrar::Archive::new(path);
    let open = match archive.open_for_listing() {
        Ok(open) => open,
        Err(error) if error.code == unrar::error::Code::MissingPassword => {
            return Ok(Listed {
                entries: Vec::new(),
                truncated: false,
                facts: Facts {
                    encrypted_headers: true,
                    ..Facts::default()
                },
            });
        }
        Err(error) => return Err(invalid(format!("RAR: {error}"))),
    };
    if open.has_encrypted_headers() {
        return Ok(Listed {
            entries: Vec::new(),
            truncated: false,
            facts: Facts {
                encrypted_headers: true,
                ..Facts::default()
            },
        });
    }
    let solid = open.is_solid();
    let mut entries = Vec::new();
    let mut truncated = false;
    for header in open {
        if limit.reached(entries.len()) {
            truncated = true;
            break;
        }
        let header = match header {
            Ok(header) => header,
            Err(error) if error.code == unrar::error::Code::MissingPassword => {
                // Encrypted headers that announced themselves only here.
                return Ok(Listed {
                    entries: Vec::new(),
                    truncated: false,
                    facts: Facts {
                        encrypted_headers: true,
                        ..Facts::default()
                    },
                });
            }
            Err(error) if !entries.is_empty() => {
                tracing::debug!(%error, "RAR walk stopped");
                truncated = true;
                break;
            }
            Err(error) => return Err(invalid(format!("RAR: {error}"))),
        };
        let time = header.file_time;
        // `file_time` is an MS-DOS date in the high half and time in the low.
        let modified = dos_to_system_time(
            u16::try_from(time >> 16).unwrap_or(0),
            u16::try_from(time & 0xFFFF).unwrap_or(0),
        );
        entries.push(RawEntry {
            path: header.filename.to_string_lossy().replace('\\', "/"),
            kind: if header.is_directory() {
                EntryKind::Dir
            } else {
                EntryKind::File
            },
            size: Some(header.unpacked_size),
            compressed: None,
            modified,
            encrypted: header.is_encrypted(),
        });
    }
    Ok(Listed {
        entries,
        truncated,
        facts: Facts {
            encrypted_headers: false,
            solid,
            rows: Vec::new(),
        },
    })
}
