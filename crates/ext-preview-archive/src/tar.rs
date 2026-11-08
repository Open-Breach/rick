//! tar listing: the headers of a plain tar, skipping the data by seeking,
//! or of a compressed one, decoding through it until the budget is spent.

use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use types::{EntryKind, Error, VPath};

use crate::{Compression, Facts, Limit, Listed, RawEntry, invalid, unix_to_system_time};

/// Lists the tar at `path`, compressed as `compression` says.
pub(crate) fn list(path: &Path, compression: Compression, limit: &Limit) -> Result<Listed, Error> {
    let file = File::open(path).map_err(Error::from_io)?;
    let reader = BufReader::with_capacity(1 << 20, file);
    match compression {
        Compression::None => {
            let mut archive = tar::Archive::new(reader);
            let entries = archive
                .entries_with_seek()
                .map_err(|e| invalid(format!("tar: {e}")))?;
            walk(entries, limit)
        }
        Compression::Gzip => {
            let mut archive = tar::Archive::new(flate2::read::MultiGzDecoder::new(reader));
            let entries = archive
                .entries()
                .map_err(|e| invalid(format!("tar.gz: {e}")))?;
            walk(entries, limit)
        }
        Compression::Zstd => {
            let decoder = zstd::stream::read::Decoder::new(reader)
                .map_err(|e| invalid(format!("tar.zst: {e}")))?;
            let mut archive = tar::Archive::new(decoder);
            let entries = archive
                .entries()
                .map_err(|e| invalid(format!("tar.zst: {e}")))?;
            walk(entries, limit)
        }
        Compression::Xz => {
            let mut archive = tar::Archive::new(lzma_rust2::XzReader::new(reader, true));
            let entries = archive
                .entries()
                .map_err(|e| invalid(format!("tar.xz: {e}")))?;
            walk(entries, limit)
        }
        Compression::Bzip2 => {
            let mut archive = tar::Archive::new(bzip2::read::MultiBzDecoder::new(reader));
            let entries = archive
                .entries()
                .map_err(|e| invalid(format!("tar.bz2: {e}")))?;
            walk(entries, limit)
        }
        Compression::Lz4 => {
            let mut archive = tar::Archive::new(lz4_flex::frame::FrameDecoder::new(reader));
            let entries = archive
                .entries()
                .map_err(|e| invalid(format!("tar.lz4: {e}")))?;
            walk(entries, limit)
        }
    }
}

/// Reads every header `entries` yields, up to the limit.
fn walk<R: Read>(entries: tar::Entries<'_, R>, limit: &Limit) -> Result<Listed, Error> {
    let mut out = Vec::new();
    let mut truncated = false;
    let mut any = false;
    for entry in entries {
        if limit.reached(out.len()) {
            truncated = true;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            // A truncated or damaged tail: keep what was read.
            Err(error) if any => {
                tracing::debug!(%error, "tar walk stopped");
                truncated = true;
                break;
            }
            Err(error) => return Err(invalid(format!("tar: {error}"))),
        };
        any = true;
        let header = entry.header();
        let path = entry.path_bytes().iter().copied().collect::<Vec<u8>>();
        let path = String::from_utf8_lossy(&path).into_owned();
        let kind = match header.entry_type() {
            t if t.is_dir() => EntryKind::Dir,
            t if t.is_symlink() => EntryKind::Symlink {
                target: entry
                    .link_name_bytes()
                    .map_or_else(|| VPath::from(""), |b| VPath::from_bytes(b.into_owned())),
                broken: false,
            },
            t if t.is_file() || t.is_hard_link() || t.is_contiguous() => EntryKind::File,
            _ => EntryKind::Other,
        };
        out.push(RawEntry {
            path,
            kind,
            size: Some(entry.size()),
            compressed: None,
            modified: header.mtime().ok().and_then(unix_to_system_time),
            encrypted: false,
        });
    }
    Ok(Listed {
        entries: out,
        truncated,
        facts: Facts::default(),
    })
}

/// Whether `reader` starts with a tar header whose checksum holds: for a
/// `.tar` without the ustar magic (an old V7 tar).
#[allow(dead_code)]
pub(crate) fn looks_like_tar(mut reader: impl Read + Seek) -> bool {
    let mut block = [0u8; 512];
    if reader.read_exact(&mut block).is_err() {
        return false;
    }
    let stored = std::str::from_utf8(&block[148..156])
        .ok()
        .and_then(|s| u32::from_str_radix(s.trim_matches(|c: char| c == '\0' || c == ' '), 8).ok());
    let mut sum: u32 = 0;
    for (i, b) in block.iter().enumerate() {
        sum += if (148..156).contains(&i) {
            32
        } else {
            u32::from(*b)
        };
    }
    stored == Some(sum)
}
