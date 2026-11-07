//! RAR through the bundled unrar library: listing and extraction only,
//! as the spec rules (creation is not licensable).

use std::io::Write;
use std::path::Path;

use types::{EntryKind, Error};

use crate::index::{Item, civil_time, normalize};
use crate::{ext_error, io_error};

/// An MS-DOS date and time, as RAR headers carry it.
fn dos_time(time: u32) -> Option<std::time::SystemTime> {
    civil_time(
        i64::from((time >> 25) & 0x7f) + 1980,
        (time >> 21) & 0x0f,
        (time >> 16) & 0x1f,
        (time >> 11) & 0x1f,
        (time >> 5) & 0x3f,
        (time & 0x1f) * 2,
    )
}

fn name_of(header: &unrar::FileHeader) -> Vec<u8> {
    normalize(header.filename.to_string_lossy().as_bytes())
}

/// The archive's entries.
pub fn index(path: &Path) -> Result<Vec<Item>, Error> {
    let archive = unrar::Archive::new(path)
        .open_for_listing()
        .map_err(ext_error)?;
    let mut items = Vec::new();
    for header in archive {
        let header = header.map_err(ext_error)?;
        let name = name_of(&header);
        if name.is_empty() {
            continue;
        }
        let is_dir = header.is_directory();
        items.push(Item {
            path: name,
            kind: if is_dir {
                EntryKind::Dir
            } else {
                EntryKind::File
            },
            size: (!is_dir).then_some(header.unpacked_size),
            modified: dos_time(header.file_time),
            mode: None,
        });
    }
    Ok(items)
}

/// Extracts the entry at `inner` into `sink`. The library hands a file's
/// bytes over whole, so the entry is held in memory while it is written.
pub fn copy_out(path: &Path, inner: &[u8], sink: &mut dyn Write) -> Result<(), Error> {
    let mut archive = unrar::Archive::new(path)
        .open_for_processing()
        .map_err(ext_error)?;
    while let Some(before_file) = archive.read_header().map_err(ext_error)? {
        if name_of(before_file.entry()) == inner {
            let (data, _rest) = before_file.read().map_err(ext_error)?;
            sink.write_all(&data).map_err(io_error)?;
            return Ok(());
        }
        archive = before_file.skip().map_err(ext_error)?;
    }
    Err(Error::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dos_times_decode() {
        // 2024-03-05 14:30:20: year 44 since 1980, month 3, day 5, 14:30, 10 two-second units.
        let packed = (44u32 << 25) | (3 << 21) | (5 << 16) | (14 << 11) | (30 << 5) | 0x0a;
        assert_eq!(dos_time(packed), civil_time(2024, 3, 5, 14, 30, 20));
        assert_eq!(dos_time(0), None, "month zero is no date");
    }
}
