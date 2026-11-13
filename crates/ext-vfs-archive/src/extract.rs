//! Unpacking a whole archive to disk in one pass: what the Extract actions
//! of `ext-action-compress` run.
//!
//! Reading an archive entry by entry through the provider's `open_read`
//! scans a tar or a solid 7z block from its start for every entry; this
//! pass reads each format the way it is laid out, once, and asks the caller
//! before every entry where it goes ([`Target`]), so the caller decides
//! about existing names, reports progress and stops when cancelled. Paths
//! are normalised and an entry that would climb out of the target folder
//! is refused.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use types::{EntryKind, Error};

use crate::format::{Codec, Format};
use crate::index::{Item, normalize};
use crate::{ext_error, io_error};

/// One entry about to be unpacked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracting {
    /// The path inside the archive, normalised.
    pub path: Vec<u8>,
    /// File, folder or link.
    pub kind: EntryKind,
    /// The size in bytes, for a file whose format records it.
    pub size: Option<u64>,
    /// The modification time the archive records.
    pub modified: Option<SystemTime>,
    /// The Unix permission bits the archive records.
    pub mode: Option<u32>,
}

/// Where an entry goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Write it here, replacing whatever is there.
    Write(PathBuf),
    /// Leave it out.
    Skip,
}

/// Decides where an entry goes; an error stops the extraction.
pub type Decide<'a> = dyn FnMut(&Extracting) -> Result<Target, Error> + 'a;

/// Unpacks the archive at `archive`, of `format`, asking `decide` for every
/// entry in the archive's own order.
///
/// # Errors
///
/// Whatever `decide` returns; `Unsupported` for an entry whose path climbs
/// out of its folder; the I/O or library error otherwise. Entries written
/// before an error stay.
pub fn extract(archive: &Path, format: Format, decide: &mut Decide<'_>) -> Result<(), Error> {
    match format {
        Format::Zip => extract_zip(archive, decide),
        Format::Tar(codec) => extract_tar(archive, codec, decide),
        Format::SevenZ => extract_7z(archive, decide),
        Format::Rar => extract_rar(archive, decide),
        Format::Image => extract_by_item(
            archive,
            &crate::image::index(archive)?,
            decide,
            |inner, sink| crate::image::copy_out(archive, inner, sink),
        ),
        Format::Disk(image) => {
            let resolver = crate::library::Resolver::shared();
            extract_by_item(
                archive,
                &crate::diskimage::index(&resolver, archive, image)?,
                decide,
                |inner, sink| crate::diskimage::copy_out(&resolver, archive, image, inner, sink),
            )
        }
        Format::Single(codec) => {
            let items = crate::single::index(archive, codec);
            extract_by_item(archive, &items, decide, |inner, sink| {
                crate::single::copy_out(archive, codec, inner, sink)
            })
        }
    }
}

/// Whether a normalised path stays inside its folder: no `..` component.
fn is_safe(path: &[u8]) -> bool {
    !path.is_empty() && !path.split(|b| *b == b'/').any(|c| c == b"..")
}

fn checked(path: Vec<u8>) -> Result<Vec<u8>, Error> {
    if is_safe(&path) {
        Ok(path)
    } else {
        Err(Error::Unsupported(
            "an entry's path leaves the folder it is extracted to",
        ))
    }
}

fn prepare(target: &Path) -> Result<(), Error> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(Error::from_io)?;
    }
    // Replace a folder or a link that is in the way of a file.
    match std::fs::symlink_metadata(target) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(target).map_err(Error::from_io)?,
        Ok(_) => std::fs::remove_file(target).map_err(Error::from_io)?,
        Err(_) => {}
    }
    Ok(())
}

/// Applies the recorded time and mode to a written entry.
fn finish(target: &Path, extracting: &Extracting) {
    if let Some(modified) = extracting.modified
        && let Ok(file) = File::options().write(true).open(target)
    {
        let _ = file.set_modified(modified);
    }
    if let Some(mode) = extracting.mode
        && extracting.kind == EntryKind::File
    {
        let _ = ext::platform::fs::set_unix_mode(target, mode);
    }
}

fn make_dir(target: &Path) -> Result<(), Error> {
    if std::fs::symlink_metadata(target).is_ok_and(|m| !m.is_dir()) {
        std::fs::remove_file(target).map_err(Error::from_io)?;
    }
    std::fs::create_dir_all(target).map_err(Error::from_io)
}

fn write_from(reader: &mut dyn Read, target: &Path, extracting: &Extracting) -> Result<(), Error> {
    prepare(target)?;
    let mut file = File::create(target).map_err(Error::from_io)?;
    io::copy(reader, &mut file).map_err(io_error)?;
    file.flush().map_err(io_error)?;
    drop(file);
    finish(target, extracting);
    Ok(())
}

fn extract_zip(archive: &Path, decide: &mut Decide<'_>) -> Result<(), Error> {
    let file = File::open(archive).map_err(Error::from_io)?;
    let mut zip = zip::ZipArchive::new(file).map_err(ext_error)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(ext_error)?;
        let path = normalize(entry.name_raw());
        if path.is_empty() {
            continue;
        }
        let path = checked(path)?;
        let kind = if entry.is_dir() {
            EntryKind::Dir
        } else if entry.is_symlink() {
            EntryKind::Symlink {
                target: types::VPath::new(),
                broken: false,
            }
        } else {
            EntryKind::File
        };
        let extracting = Extracting {
            path,
            size: (kind == EntryKind::File).then(|| entry.size()),
            modified: crate::zipfmt::time_of(entry.last_modified()),
            mode: entry.unix_mode(),
            kind: kind.clone(),
        };
        let Target::Write(target) = decide(&extracting)? else {
            continue;
        };
        match kind {
            EntryKind::Dir => make_dir(&target)?,
            EntryKind::Symlink { .. } => {
                let mut link = Vec::new();
                entry.read_to_end(&mut link).map_err(io_error)?;
                prepare(&target)?;
                let link = PathBuf::from(String::from_utf8_lossy(&link).into_owned());
                ext::platform::fs::symlink(&link, &target).map_err(Error::from_io)?;
            }
            _ => write_from(&mut entry, &target, &extracting)?,
        }
    }
    Ok(())
}

fn extract_tar(archive: &Path, codec: Option<Codec>, decide: &mut Decide<'_>) -> Result<(), Error> {
    let mut tar = tar::Archive::new(crate::tarfmt::stream(archive, codec)?);
    tar.set_preserve_mtime(true);
    tar.set_preserve_permissions(true);
    for entry in tar.entries().map_err(io_error)? {
        let mut entry = entry.map_err(io_error)?;
        let path = normalize(&entry.path_bytes());
        if path.is_empty() {
            continue;
        }
        let path = checked(path)?;
        let header = entry.header();
        let kind = header.entry_type();
        let kind = if kind.is_dir() {
            EntryKind::Dir
        } else if kind.is_symlink() || kind.is_hard_link() {
            EntryKind::Symlink {
                target: entry
                    .link_name_bytes()
                    .map_or_default(|b| types::VPath::from_bytes(b.into_owned())),
                broken: false,
            }
        } else if kind.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        let extracting = Extracting {
            path,
            size: (kind == EntryKind::File).then(|| entry.size()),
            modified: header.mtime().ok().and_then(crate::index::unix_time),
            mode: header.mode().ok(),
            kind: kind.clone(),
        };
        let Target::Write(target) = decide(&extracting)? else {
            continue;
        };
        match kind {
            EntryKind::Dir => {
                make_dir(&target)?;
            }
            EntryKind::Other => {}
            _ => {
                prepare(&target)?;
                // The tar crate writes files and links with their recorded
                // mode and time.
                entry.unpack(&target).map_err(io_error)?;
            }
        }
    }
    Ok(())
}

fn extract_7z(archive: &Path, decide: &mut Decide<'_>) -> Result<(), Error> {
    let mut reader = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty())
        .map_err(ext_error)?;
    let mut failure: Option<Error> = None;
    reader
        .for_each_entries(|entry, data| {
            let path = normalize(entry.name().as_bytes());
            if path.is_empty() || entry.is_anti_item {
                return Ok(true);
            }
            let outcome = (|| -> Result<(), Error> {
                let path = checked(path)?;
                let kind = if entry.is_directory {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                };
                let extracting = Extracting {
                    path,
                    size: (!entry.is_directory).then_some(entry.size),
                    modified: entry
                        .has_last_modified_date
                        .then(|| SystemTime::from(entry.last_modified_date)),
                    mode: None,
                    kind: kind.clone(),
                };
                let Target::Write(target) = decide(&extracting)? else {
                    return Ok(());
                };
                if kind == EntryKind::Dir {
                    make_dir(&target)
                } else {
                    write_from(data, &target, &extracting)
                }
            })();
            match outcome {
                Ok(()) => Ok(true),
                Err(error) => {
                    failure = Some(error);
                    Ok(false)
                }
            }
        })
        .map_err(ext_error)?;
    failure.map_or(Ok(()), Err)
}

fn extract_rar(archive: &Path, decide: &mut Decide<'_>) -> Result<(), Error> {
    let mut rar = unrar::Archive::new(archive)
        .open_for_processing()
        .map_err(ext_error)?;
    while let Some(before) = rar.read_header().map_err(ext_error)? {
        let header = before.entry();
        let path = normalize(header.filename.to_string_lossy().as_bytes());
        if path.is_empty() {
            rar = before.skip().map_err(ext_error)?;
            continue;
        }
        let path = checked(path)?;
        let is_dir = header.is_directory();
        let extracting = Extracting {
            path,
            kind: if is_dir {
                EntryKind::Dir
            } else {
                EntryKind::File
            },
            size: (!is_dir).then_some(header.unpacked_size),
            modified: crate::rar::dos_time(header.file_time),
            mode: None,
        };
        let Target::Write(target) = decide(&extracting)? else {
            rar = before.skip().map_err(ext_error)?;
            continue;
        };
        if is_dir {
            make_dir(&target)?;
            rar = before.skip().map_err(ext_error)?;
        } else {
            prepare(&target)?;
            let (data, rest) = before.read().map_err(ext_error)?;
            let mut file = File::create(&target).map_err(Error::from_io)?;
            file.write_all(&data).map_err(io_error)?;
            drop(file);
            finish(&target, &extracting);
            rar = rest;
        }
    }
    Ok(())
}

/// Formats with random access and an index: each item is copied out on
/// its own.
fn extract_by_item(
    _archive: &Path,
    items: &[Item],
    decide: &mut Decide<'_>,
    mut copy_out: impl FnMut(&[u8], &mut dyn Write) -> Result<(), Error>,
) -> Result<(), Error> {
    for item in items {
        let path = checked(item.path.clone())?;
        let extracting = Extracting {
            path,
            kind: item.kind.clone(),
            size: item.size,
            modified: item.modified,
            mode: item.mode,
        };
        let Target::Write(target) = decide(&extracting)? else {
            continue;
        };
        if item.kind == EntryKind::Dir {
            make_dir(&target)?;
            continue;
        }
        prepare(&target)?;
        let mut file = File::create(&target).map_err(Error::from_io)?;
        copy_out(&item.path, &mut file)?;
        file.flush().map_err(io_error)?;
        drop(file);
        finish(&target, &extracting);
    }
    Ok(())
}
