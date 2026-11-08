//! zip: the central directory is the index, entries decompress on their
//! own, and a rewrite copies the untouched entries' compressed bytes.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use types::{EntryKind, Error};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::index::{Item, civil_time, normalize};
use crate::rewrite::{Edit, Plan};
use crate::{ext_error, io_error};

fn open(path: &Path) -> Result<ZipArchive<File>, Error> {
    let file = File::open(path).map_err(Error::from_io)?;
    ZipArchive::new(file).map_err(ext_error)
}

fn time_of(dt: Option<zip::DateTime>) -> Option<std::time::SystemTime> {
    let dt = dt?;
    civil_time(
        i64::from(dt.year()),
        u32::from(dt.month()),
        u32::from(dt.day()),
        u32::from(dt.hour()),
        u32::from(dt.minute()),
        u32::from(dt.second()),
    )
}

/// The zip's entries.
pub fn index(path: &Path) -> Result<Vec<Item>, Error> {
    let archive = open(path)?;
    let mut items = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive.by_index_data(i).map_err(ext_error)?;
        let name = normalize(entry.name_raw());
        if name.is_empty() {
            continue;
        }
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
        items.push(Item {
            path: name,
            size: (kind != EntryKind::Dir).then(|| entry.size()),
            modified: time_of(entry.last_modified()),
            mode: entry.unix_mode(),
            kind,
        });
    }
    Ok(items)
}

/// The index of the entry at `inner` in `archive`.
fn index_of(archive: &ZipArchive<File>, inner: &[u8]) -> Result<usize, Error> {
    (0..archive.len())
        .find(|i| {
            archive
                .by_index_data(*i)
                .is_ok_and(|e| normalize(e.name_raw()) == inner)
        })
        .ok_or(Error::NotFound)
}

/// Decompresses the entry at `inner` into `sink`.
pub fn copy_out(path: &Path, inner: &[u8], sink: &mut dyn Write) -> Result<(), Error> {
    let mut archive = open(path)?;
    let index = index_of(&archive, inner)?;
    let mut entry = archive.by_index(index).map_err(ext_error)?;
    io::copy(&mut entry, sink).map_err(io_error)?;
    Ok(())
}

fn options(modified: std::time::SystemTime, mode: Option<u32>) -> SimpleFileOptions {
    let mut options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(true);
    if let Some(dt) = zip_time(modified) {
        options = options.last_modified_time(dt);
    }
    if let Some(mode) = mode {
        options = options.unix_permissions(mode);
    }
    options
}

/// A time as the zip format stores it, `None` outside its 1980–2107 range.
fn zip_time(time: std::time::SystemTime) -> Option<zip::DateTime> {
    let secs = time
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let days = i64::try_from(secs / 86_400).ok()?;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    let rem = secs % 86_400;
    zip::DateTime::from_date_and_time(
        u16::try_from(year).ok()?,
        u8::try_from(m).ok()?,
        u8::try_from(d).ok()?,
        u8::try_from(rem / 3600).ok()?,
        u8::try_from((rem % 3600) / 60).ok()?,
        u8::try_from(rem % 60).ok()?,
    )
    .ok()
}

/// Writes the zip anew into `out` as `plan` says: untouched entries copy
/// their compressed bytes, renamed ones under their new name, removed ones
/// not at all, then the additions.
pub fn rewrite(path: &Path, out: &mut File, plan: &Plan) -> Result<(), Error> {
    let mut archive = open(path)?;
    let mut writer = ZipWriter::new(out);
    for i in 0..archive.len() {
        let name = {
            let entry = archive.by_index_data(i).map_err(ext_error)?;
            normalize(entry.name_raw())
        };
        let Some(new_name) = plan.new_path(&name) else {
            continue;
        };
        let entry = archive.by_index_raw(i).map_err(ext_error)?;
        let is_dir = entry.is_dir();
        let target = String::from_utf8_lossy(&new_name).into_owned();
        if is_dir {
            writer
                .add_directory(&target, options_like(&entry))
                .map_err(ext_error)?;
        } else {
            writer
                .raw_copy_file_rename(entry, &target)
                .map_err(ext_error)?;
        }
    }
    for edit in &plan.edits {
        match edit {
            Edit::AddFile {
                path,
                source,
                modified,
            } => {
                let name = String::from_utf8_lossy(path).into_owned();
                writer
                    .start_file(&name, options(*modified, None))
                    .map_err(ext_error)?;
                let mut file = File::open(source).map_err(Error::from_io)?;
                io::copy(&mut file, &mut writer).map_err(io_error)?;
            }
            Edit::AddDir { path } => {
                let name = String::from_utf8_lossy(path).into_owned();
                writer
                    .add_directory(&name, options(std::time::SystemTime::now(), None))
                    .map_err(ext_error)?;
            }
            Edit::Remove(_) | Edit::Rename { .. } => {}
        }
    }
    writer
        .finish()
        .map_err(ext_error)?
        .flush()
        .map_err(io_error)?;
    Ok(())
}

/// Options that keep an existing entry's time and mode.
fn options_like<R: Read>(entry: &zip::read::ZipFile<'_, R>) -> SimpleFileOptions {
    let modified = time_of(entry.last_modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    options(modified, entry.unix_mode())
}
