//! 7z through `sevenz-rust2`: the header lists the entries, each is
//! decoded in its block, and a rewrite re-encodes every kept entry.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::SystemTime;

use sevenz_rust2::{ArchiveEntry, ArchiveReader, ArchiveWriter, NtTime, Password};
use types::{EntryKind, Error};

use crate::index::{Item, normalize};
use crate::rewrite::{Edit, Plan};
use crate::{ext_error, io_error};

fn open(path: &Path) -> Result<ArchiveReader<File>, Error> {
    ArchiveReader::open(path, Password::empty()).map_err(ext_error)
}

fn time_of(entry: &ArchiveEntry) -> Option<SystemTime> {
    entry
        .has_last_modified_date
        .then(|| SystemTime::from(entry.last_modified_date))
}

/// The archive's entries.
pub fn index(path: &Path) -> Result<Vec<Item>, Error> {
    let reader = open(path)?;
    Ok(reader
        .archive()
        .files
        .iter()
        .filter(|entry| !entry.is_anti_item)
        .filter_map(|entry| {
            let name = normalize(entry.name().as_bytes());
            (!name.is_empty()).then(|| Item {
                path: name,
                kind: if entry.is_directory {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                },
                size: (!entry.is_directory).then_some(entry.size),
                modified: time_of(entry),
                mode: None,
            })
        })
        .collect())
}

/// Decodes the entry at `inner` into `sink`.
pub fn copy_out(path: &Path, inner: &[u8], sink: &mut dyn Write) -> Result<(), Error> {
    let mut reader = open(path)?;
    let mut found = false;
    let mut failure: Option<io::Error> = None;
    reader
        .for_each_entries(|entry, data| {
            if normalize(entry.name().as_bytes()) != inner || entry.is_directory {
                return Ok(true);
            }
            found = true;
            if let Err(error) = io::copy(data, sink) {
                failure = Some(error);
            }
            // The rest of the block is not needed.
            Ok(false)
        })
        .map_err(ext_error)?;
    if let Some(error) = failure {
        return Err(io_error(error));
    }
    if found { Ok(()) } else { Err(Error::NotFound) }
}

fn nt_time(time: SystemTime) -> NtTime {
    NtTime::try_from(time).unwrap_or_default()
}

/// Writes the archive anew into `out` as `plan` says.
pub fn rewrite(path: &Path, out: &mut File, plan: &Plan) -> Result<(), Error> {
    let mut reader = open(path)?;
    let mut writer =
        ArchiveWriter::new(out.try_clone().map_err(Error::from_io)?).map_err(ext_error)?;
    let mut failure: Option<Error> = None;
    reader
        .for_each_entries(|entry, data| {
            let name = normalize(entry.name().as_bytes());
            let Some(new_name) = plan.new_path(&name) else {
                return Ok(true);
            };
            let mut copy = entry.clone();
            copy.name = String::from_utf8_lossy(&new_name).into_owned();
            let outcome = if entry.is_directory {
                writer.push_archive_entry(copy, None::<&mut dyn Read>)
            } else {
                writer.push_archive_entry(copy, Some(data))
            };
            if let Err(error) = outcome {
                failure = Some(ext_error(error));
                return Ok(false);
            }
            Ok(true)
        })
        .map_err(ext_error)?;
    if let Some(error) = failure {
        return Err(error);
    }
    for edit in &plan.edits {
        match edit {
            Edit::AddFile {
                path,
                source,
                modified,
            } => {
                let name = String::from_utf8_lossy(path).into_owned();
                let mut entry = ArchiveEntry::new_file(&name);
                entry.has_last_modified_date = true;
                entry.last_modified_date = nt_time(*modified);
                let file = File::open(source).map_err(Error::from_io)?;
                writer
                    .push_archive_entry(entry, Some(file))
                    .map_err(ext_error)?;
            }
            Edit::AddDir { path } => {
                let name = String::from_utf8_lossy(path).into_owned();
                let mut entry = ArchiveEntry::new_directory(&name);
                entry.has_last_modified_date = true;
                entry.last_modified_date = nt_time(SystemTime::now());
                writer
                    .push_archive_entry(entry, None::<File>)
                    .map_err(ext_error)?;
            }
            Edit::Remove(_) | Edit::Rename { .. } => {}
        }
    }
    writer.finish().map_err(io_error)?;
    out.flush().map_err(io_error)?;
    Ok(())
}
