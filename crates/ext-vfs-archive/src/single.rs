//! A single compressed file (gzip, zstd, xz, bzip2, LZ4, Brotli, Snappy) as
//! a one-entry archive: the entry is the file without its codec extension.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use types::Error;

use crate::format::Codec;
use crate::index::Item;
use crate::io_error;

/// The one entry's name: the file name without the codec's extension
/// (`notes.txt` for `notes.txt.gz`), or the file name with `.out` when it
/// has no extension to drop.
#[must_use]
pub fn entry_name(path: &Path) -> Vec<u8> {
    let name = path
        .file_name()
        .map_or_default(|n| n.to_string_lossy().into_owned());
    match name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty() && Codec::by_extension(&ext.to_ascii_lowercase()).is_some() =>
        {
            stem.as_bytes().to_vec()
        }
        _ => format!("{name}.out").into_bytes(),
    }
}

/// The gzip trailer's size, modulo 4 GiB; other codecs record none.
fn gzip_size(path: &Path) -> Option<u64> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len < 18 {
        return None;
    }
    io::Seek::seek(&mut file, io::SeekFrom::End(-4)).ok()?;
    let mut trailer = [0u8; 4];
    file.read_exact(&mut trailer).ok()?;
    Some(u64::from(u32::from_le_bytes(trailer)))
}

/// The one entry.
#[must_use]
pub fn index(path: &Path, codec: Codec) -> Vec<Item> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut item = Item::file(&entry_name(path), 0, modified);
    item.size = if codec == Codec::Gzip {
        gzip_size(path)
    } else {
        None
    };
    vec![item]
}

/// Decompresses the file into `sink`.
pub fn copy_out(
    path: &Path,
    codec: Codec,
    inner: &[u8],
    sink: &mut dyn Write,
) -> Result<(), Error> {
    if inner != entry_name(path) {
        return Err(Error::NotFound);
    }
    let file = File::open(path).map_err(Error::from_io)?;
    let mut decoder = codec
        .decoder(Box::new(io::BufReader::new(file)))
        .map_err(io_error)?;
    io::copy(&mut decoder, sink).map_err(io_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_drops_the_codec_extension() {
        assert_eq!(entry_name(Path::new("/x/notes.txt.gz")), b"notes.txt");
        assert_eq!(entry_name(Path::new("/x/data.zst")), b"data");
        assert_eq!(entry_name(Path::new("/x/blob")), b"blob.out");
        assert_eq!(entry_name(Path::new("/x/.gz")), b".gz.out");
    }
}
