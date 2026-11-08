//! Disk images through the 7-Zip library, read-only: the container (VHD,
//! VHDX, VMDK, VDI, QCOW2, DMG) opened with its handler, the disk inside
//! it taken as a stream and opened with the partition table handlers (GPT,
//! MBR), each partition opened with the file system handlers (NTFS, FAT,
//! ext, HFS+, APFS, UDF, ISO), the way 7-Zip's own browser descends. A raw
//! image is a disk or a partition with no container around it. The tree
//! lists one folder per partition, named as 7-Zip names them
//! (`0.Basic data partition.ntfs`), with the partition's files inside; a
//! partition no handler recognises is listed as a file of its size.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use ext::platform::ImageFormat;
use ext::platform::sevenzip::{Archive, SevenZip, SevenZipError, Stream, prop};
use types::{EntryKind, Error, VPath};

use crate::index::{Item, normalize};
use crate::library::Resolver;
use crate::{ID, ext_error_text};

/// The handler of each container; a raw image has none.
fn container_handler(format: ImageFormat) -> Option<&'static str> {
    match format {
        ImageFormat::Img | ImageFormat::Iso => None,
        ImageFormat::Dmg => Some("Dmg"),
        ImageFormat::Vhd => Some("VHD"),
        ImageFormat::Vhdx => Some("VHDX"),
        ImageFormat::Vmdk => Some("VMDK"),
        ImageFormat::Vdi => Some("VDI"),
        ImageFormat::Qcow2 => Some("QCOW"),
    }
}

/// The handlers tried, in order, on a disk: the partition tables first,
/// then the file systems a disk without a table carries.
const ON_A_DISK: &[&str] = &[
    "GPT", "MBR", "NTFS", "FAT", "Ext", "HFS", "APFS", "Udf", "Iso",
];

/// The handlers tried, in order, on a partition.
const ON_A_PARTITION: &[&str] = &["NTFS", "FAT", "Ext", "HFS", "APFS", "Udf", "Iso"];

/// The handlers whose items are partitions with streams of their own.
const TABLES: &[&str] = &["GPT", "MBR"];

/// How deep containers, tables and file systems may nest.
const MAX_DEPTH: usize = 6;

/// One entry the walk found: the item, where it is and how to read it.
pub struct Found<'a> {
    /// The archive (file system, table or container) holding the item.
    pub archive: &'a Archive,
    /// The item's index in that archive.
    pub index: u32,
    /// The full path inside the image, normalised.
    pub path: Vec<u8>,
    /// The entry as the index records it.
    pub item: Item,
}

/// What the walk learnt about the image beside its entries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Facts {
    /// The handlers on the way in: `VHDX`, `GPT`, `NTFS`.
    pub layout: Vec<String>,
    /// The labels of the file systems, when they have one.
    pub labels: Vec<String>,
}

/// A library error as the kernel's.
fn library_error(error: SevenZipError) -> Error {
    Error::Extension {
        ext: ID,
        source: Box::new(error),
    }
}

/// The item of `archive` at `index` as the index records it, under
/// `prefix`.
fn item_of(archive: &Archive, index: u32, prefix: &[u8]) -> Result<(Vec<u8>, Item), Error> {
    let raw = archive.item(index).map_err(library_error)?;
    let name = normalize(raw.path.as_bytes());
    let path = if prefix.is_empty() {
        name
    } else if name.is_empty() {
        prefix.to_vec()
    } else {
        [prefix, b"/", name.as_slice()].concat()
    };
    let kind = if raw.is_dir {
        EntryKind::Dir
    } else if let Some(target) = &raw.symlink {
        EntryKind::Symlink {
            target: VPath::from_bytes(target.as_bytes().to_vec()),
            broken: false,
        }
    } else {
        EntryKind::File
    };
    let item = Item {
        path: path.clone(),
        size: (kind != EntryKind::Dir).then_some(raw.size.unwrap_or(0)),
        modified: raw.modified,
        mode: raw.mode,
        kind,
    };
    Ok((path, item))
}

/// Lists every item of `archive` under `prefix`.
fn list_items(
    archive: &Archive,
    prefix: &[u8],
    visit: &mut dyn FnMut(Found<'_>) -> Result<bool, Error>,
) -> Result<bool, Error> {
    for index in 0..archive.len().map_err(library_error)? {
        let (path, item) = item_of(archive, index, prefix)?;
        if path.is_empty() {
            continue;
        }
        if visit(Found {
            archive,
            index,
            path,
            item,
        })? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Opens `stream` with the first of `handlers` that recognises it and
/// walks it; `Ok(false)` when none does.
fn open_inside(
    library: &Arc<SevenZip>,
    stream: &Stream,
    handlers: &[&str],
    prefix: &[u8],
    depth: usize,
    facts: &mut Facts,
    visit: &mut dyn FnMut(Found<'_>) -> Result<bool, Error>,
) -> Result<Option<bool>, Error> {
    for name in handlers {
        let Some(format) = library.format(name) else {
            continue;
        };
        match library.open_stream(stream.clone(), format) {
            Ok(inner) => {
                return walk(library, &inner, prefix, depth + 1, facts, visit).map(Some);
            }
            Err(SevenZipError::NotArchive(_) | SevenZipError::Call { .. }) => {}
            Err(error) => return Err(library_error(error)),
        }
    }
    Ok(None)
}

/// Walks `archive`: into its disk when it is a container, into each
/// partition when it is a table, over its files otherwise. `visit`
/// returns `true` to stop; the result says whether it did.
fn walk(
    library: &Arc<SevenZip>,
    archive: &Archive,
    prefix: &[u8],
    depth: usize,
    facts: &mut Facts,
    visit: &mut dyn FnMut(Found<'_>) -> Result<bool, Error>,
) -> Result<bool, Error> {
    let name = archive.format().to_owned();
    facts.layout.push(name.clone());
    if let Some(Some(label)) = archive
        .archive_property(prop::VOLUME_NAME)
        .ok()
        .map(|v| v.and_then(|v| v.text().map(str::to_owned)))
        && !label.trim().is_empty()
    {
        facts.labels.push(label);
    }
    if depth >= MAX_DEPTH {
        return list_items(archive, prefix, visit);
    }
    let is_container = ImageFormat::ALL
        .iter()
        .filter_map(|f| container_handler(*f))
        .any(|handler| handler.eq_ignore_ascii_case(&name));
    if is_container {
        let main = archive.main_subfile().map_err(library_error)?;
        if let Some(main) = main
            && let Ok(stream) = archive.sub_stream(main)
            && let Some(stopped) =
                open_inside(library, &stream, ON_A_DISK, prefix, depth, facts, visit)?
        {
            return Ok(stopped);
        }
        // No disk inside the library knows: the container's own items.
        return list_items(archive, prefix, visit);
    }
    if TABLES.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
        for index in 0..archive.len().map_err(library_error)? {
            let (path, item) = item_of(archive, index, prefix)?;
            if path.is_empty() {
                continue;
            }
            if let Ok(stream) = archive.sub_stream(index)
                && let Some(stopped) =
                    open_inside(library, &stream, ON_A_PARTITION, &path, depth, facts, visit)?
            {
                if stopped {
                    return Ok(true);
                }
                continue;
            }
            // A partition without a file system the library reads: a file.
            if visit(Found {
                archive,
                index,
                path,
                item,
            })? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    list_items(archive, prefix, visit)
}

/// Opens the image at `path` of `format` and walks it, calling `visit`
/// for every entry until it returns `true`.
///
/// # Errors
///
/// The missing-library error when the 7-Zip library is not there; an
/// `Extension` error when the file is not an image of that format or the
/// library fails; whatever `visit` returns.
pub fn visit(
    resolver: &Resolver,
    path: &Path,
    format: ImageFormat,
    visit: &mut dyn FnMut(Found<'_>) -> Result<bool, Error>,
) -> Result<Facts, Error> {
    let library = resolver.library()?;
    let mut facts = Facts::default();
    match container_handler(format) {
        Some(handler) => {
            let container = library.format(handler).ok_or_else(|| {
                ext_error_text(format!(
                    "the 7-Zip library at {} has no {handler} handler",
                    library.path().display()
                ))
            })?;
            let archive = library.open_file(path, container).map_err(library_error)?;
            walk(&library, &archive, b"", 0, &mut facts, visit)?;
        }
        None => {
            let stream = Stream::file(path).map_err(library_error)?;
            if open_inside(&library, &stream, ON_A_DISK, b"", 0, &mut facts, visit)?.is_none() {
                return Err(ext_error_text(
                    "no partition table or file system the 7-Zip library knows".to_owned(),
                ));
            }
        }
    }
    Ok(facts)
}

/// The image's entries, in the walk's order.
///
/// # Errors
///
/// As [`visit`].
pub fn index(resolver: &Resolver, path: &Path, format: ImageFormat) -> Result<Vec<Item>, Error> {
    let mut items = Vec::new();
    visit(resolver, path, format, &mut |found| {
        items.push(found.item);
        Ok(false)
    })?;
    Ok(items)
}

/// Writes the content of the entry at `inner` into `sink`.
///
/// # Errors
///
/// `Error::NotFound` when no entry has that path; as [`visit`] otherwise.
pub fn copy_out(
    resolver: &Resolver,
    path: &Path,
    format: ImageFormat,
    inner: &[u8],
    sink: &mut dyn Write,
) -> Result<(), Error> {
    let mut outcome: Option<Result<(), Error>> = None;
    visit(resolver, path, format, &mut |found| {
        if found.path != inner {
            return Ok(false);
        }
        outcome = Some(
            found
                .archive
                .extract(found.index, sink)
                .map_err(library_error),
        );
        Ok(true)
    })?;
    outcome.unwrap_or(Err(Error::NotFound))
}
