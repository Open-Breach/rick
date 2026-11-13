//! The tar family: a stream, plain or through a codec, scanned once for
//! the index and again to reach an entry; a rewrite streams every kept
//! entry into a new tar through the same codec.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::SystemTime;

use types::{EntryKind, Error, VPath};

use crate::format::Codec;
use crate::index::{Item, normalize, unix_time};
use crate::io_error;
use crate::rewrite::{Edit, Plan};

/// The tar's byte stream, decompressed when `codec` says so.
pub(crate) fn stream(path: &Path, codec: Option<Codec>) -> Result<Box<dyn Read + Send>, Error> {
    let file = File::open(path).map_err(Error::from_io)?;
    let reader: Box<dyn Read + Send> = Box::new(io::BufReader::with_capacity(1 << 20, file));
    match codec {
        Some(codec) => codec.decoder(reader).map_err(io_error),
        None => Ok(reader),
    }
}

/// The tar's entries.
pub fn index(path: &Path, codec: Option<Codec>) -> Result<Vec<Item>, Error> {
    let mut archive = tar::Archive::new(stream(path, codec)?);
    let mut items = Vec::new();
    for entry in archive.entries().map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let header = entry.header();
        let name = normalize(&entry.path_bytes());
        if name.is_empty() {
            continue;
        }
        let kind = header.entry_type();
        let kind = if kind.is_dir() {
            EntryKind::Dir
        } else if kind.is_symlink() || kind.is_hard_link() {
            EntryKind::Symlink {
                target: entry
                    .link_name_bytes()
                    .map_or_default(|b| VPath::from_bytes(b.into_owned())),
                broken: false,
            }
        } else if kind.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        items.push(Item {
            path: name,
            size: (kind == EntryKind::File).then(|| entry.size()),
            modified: header.mtime().ok().and_then(unix_time),
            mode: header.mode().ok(),
            kind,
        });
    }
    Ok(items)
}

/// Decompresses the entry at `inner` into `sink`.
pub fn copy_out(
    path: &Path,
    codec: Option<Codec>,
    inner: &[u8],
    sink: &mut dyn Write,
) -> Result<(), Error> {
    let mut archive = tar::Archive::new(stream(path, codec)?);
    for entry in archive.entries().map_err(io_error)? {
        let mut entry = entry.map_err(io_error)?;
        if normalize(&entry.path_bytes()) == inner {
            io::copy(&mut entry, sink).map_err(io_error)?;
            return Ok(());
        }
    }
    Err(Error::NotFound)
}

/// A header for a new entry.
pub(crate) fn header(
    kind: tar::EntryType,
    size: u64,
    modified: SystemTime,
    mode: u32,
) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(kind);
    header.set_size(size);
    header.set_mode(mode);
    header.set_mtime(
        modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    );
    header
}

/// Writes the tar anew into `out` as `plan` says, through the codec.
pub fn rewrite(
    path: &Path,
    codec: Option<Codec>,
    out: &mut File,
    plan: &Plan,
) -> Result<(), Error> {
    let sink: Box<dyn Write + Send> = Box::new(io::BufWriter::with_capacity(
        1 << 20,
        out.try_clone().map_err(Error::from_io)?,
    ));
    let mut encoder = match codec {
        Some(codec) => Some(codec.encoder(sink).map_err(io_error)?),
        None => None,
    };
    {
        let writer: &mut dyn Write = match encoder.as_mut() {
            Some(encoder) => &mut **encoder,
            None => {
                // Without a codec the tar goes straight to the file.
                return rewrite_into(path, codec, plan, out);
            }
        };
        rewrite_into(path, codec, plan, writer)?;
    }
    if let Some(encoder) = encoder {
        encoder.finish().map_err(io_error)?;
    }
    out.flush().map_err(io_error)?;
    Ok(())
}

fn rewrite_into(
    path: &Path,
    codec: Option<Codec>,
    plan: &Plan,
    writer: &mut dyn Write,
) -> Result<(), Error> {
    let mut builder = tar::Builder::new(writer);
    builder.follow_symlinks(false);
    let mut archive = tar::Archive::new(stream(path, codec)?);
    for entry in archive.entries().map_err(io_error)? {
        let mut entry = entry.map_err(io_error)?;
        let name = normalize(&entry.path_bytes());
        let Some(new_name) = plan.new_path(&name) else {
            continue;
        };
        let mut header = entry.header().clone();
        let target = Path::new(&String::from_utf8_lossy(&new_name).into_owned()).to_path_buf();
        if header.entry_type().is_dir() {
            let dir = format!("{}/", target.display());
            builder
                .append_data(&mut header, dir, io::empty())
                .map_err(io_error)?;
        } else if header.entry_type().is_symlink() || header.entry_type().is_hard_link() {
            let link = entry
                .link_name_bytes()
                .map_or_default(|b| String::from_utf8_lossy(&b).into_owned());
            header.set_size(0);
            builder
                .append_link(&mut header, &target, link)
                .map_err(io_error)?;
        } else {
            builder
                .append_data(&mut header, &target, &mut entry)
                .map_err(io_error)?;
        }
    }
    for edit in &plan.edits {
        match edit {
            Edit::AddFile {
                path,
                source,
                modified,
            } => {
                let mut file = File::open(source).map_err(Error::from_io)?;
                let size = file.metadata().map_err(Error::from_io)?.len();
                let mut header = header(tar::EntryType::Regular, size, *modified, 0o644);
                let name = String::from_utf8_lossy(path).into_owned();
                builder
                    .append_data(&mut header, name, &mut file)
                    .map_err(io_error)?;
            }
            Edit::AddDir { path } => {
                let mut header = header(tar::EntryType::Directory, 0, SystemTime::now(), 0o755);
                let name = format!("{}/", String::from_utf8_lossy(path));
                builder
                    .append_data(&mut header, name, io::empty())
                    .map_err(io_error)?;
            }
            Edit::Remove(_) | Edit::Rename { .. } => {}
        }
    }
    builder.finish().map_err(io_error)?;
    Ok(())
}
