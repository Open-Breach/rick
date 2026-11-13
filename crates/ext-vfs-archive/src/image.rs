//! ISO 9660 and UDF disc images through the hadris readers, read-only: an
//! `.iso` is tried as ISO 9660 first (Rock Ridge or Joliet names when the
//! image has them), then as UDF.

use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, SystemTime};

use hadris_fs::sync::{FileSystem, Volume};
use hadris_fs::{DateTime, FileType, MountOptions, OpenOptions};
use hadris_storage::host::FileDevice;
use types::{EntryKind, Error, VPath};

use crate::index::{Item, normalize};
use crate::{ext_error_text, io_error};

/// A mounted image of either kind.
enum Image {
    Iso(Volume<hadris_iso::sync::IsoFs<FileDevice>>),
    Udf(Volume<hadris_udf::sync::UdfFs<FileDevice>>),
}

fn device(path: &Path) -> Result<FileDevice, Error> {
    FileDevice::open(path).map_err(Error::from_io)
}

fn mount(path: &Path) -> Result<Image, Error> {
    let iso = hadris_iso::sync::IsoFs::mount(device(path)?, MountOptions::new());
    let iso_error = match iso {
        Ok(fs) => return Ok(Image::Iso(Volume::new(fs))),
        Err(error) => error.to_string(),
    };
    match hadris_udf::sync::UdfFs::mount(device(path)?, MountOptions::new()) {
        Ok(fs) => Ok(Image::Udf(Volume::new(fs))),
        Err(udf_error) => Err(ext_error_text(format!(
            "not an ISO 9660 image ({iso_error}) nor a UDF one ({udf_error})"
        ))),
    }
}

fn time_of(time: Option<DateTime>) -> Option<SystemTime> {
    let time = time?;
    let seconds = u64::try_from(time.unix_seconds()).ok()?;
    Some(SystemTime::UNIX_EPOCH + Duration::new(seconds, time.nanoseconds()))
}

fn walk<F: FileSystem>(volume: &Volume<F>, dir: &[u8], items: &mut Vec<Item>) -> Result<(), Error> {
    let path = if dir.is_empty() {
        b"/".to_vec()
    } else {
        [b"/", dir].concat()
    };
    let mut entries = volume
        .read_dir(&path)
        .map_err(|e| ext_error_text(e.to_string()))?;
    let mut folders = Vec::new();
    while let Some(entry) = entries.next_entry() {
        let entry = entry.map_err(|e| ext_error_text(e.to_string()))?;
        let name = entry.name().as_bytes();
        if name == b"." || name == b".." || name.is_empty() {
            continue;
        }
        let full = if dir.is_empty() {
            name.to_vec()
        } else {
            [dir, b"/", name].concat()
        };
        let meta = entry.metadata();
        let kind = match meta.file_type() {
            FileType::Dir => EntryKind::Dir,
            FileType::File => EntryKind::File,
            FileType::Symlink => EntryKind::Symlink {
                target: volume
                    .read_link([b"/", full.as_slice()].concat())
                    .map_or_default(VPath::from_bytes),
                broken: false,
            },
            _ => EntryKind::Other,
        };
        if kind == EntryKind::Dir {
            folders.push(full.clone());
        }
        items.push(Item {
            path: normalize(&full),
            size: (kind == EntryKind::File).then(|| meta.len()),
            modified: time_of(meta.modified()),
            mode: Some(meta.permissions().bits()),
            kind,
        });
    }
    for folder in folders {
        walk(volume, &folder, items)?;
    }
    Ok(())
}

/// The image's entries.
pub fn index(path: &Path) -> Result<Vec<Item>, Error> {
    let mut items = Vec::new();
    match mount(path)? {
        Image::Iso(volume) => walk(&volume, b"", &mut items)?,
        Image::Udf(volume) => walk(&volume, b"", &mut items)?,
    }
    Ok(items)
}

fn copy_from<F: FileSystem>(
    volume: &Volume<F>,
    inner: &[u8],
    sink: &mut dyn Write,
) -> Result<(), Error> {
    let path = [b"/", inner].concat();
    let mut file = volume
        .open(&path, OpenOptions::new().read())
        .map_err(|e| ext_error_text(e.to_string()))?;
    io::copy(&mut file, sink).map_err(io_error)?;
    Ok(())
}

/// Reads the file at `inner` into `sink`.
pub fn copy_out(path: &Path, inner: &[u8], sink: &mut dyn Write) -> Result<(), Error> {
    match mount(path)? {
        Image::Iso(volume) => copy_from(&volume, inner, sink),
        Image::Udf(volume) => copy_from(&volume, inner, sink),
    }
}
