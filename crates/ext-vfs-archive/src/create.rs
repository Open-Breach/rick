//! Writing a new archive from files on disk: zip, the tar family, 7z, or
//! a single file through a codec. What the Compress action of
//! `ext-action-compress` writes through.
//!
//! [`collect`] turns a selection into the entries of the archive, each
//! selected item under its own name with everything below it; [`create`]
//! writes them into a temporary file beside the target and renames it over
//! the target when the last entry is in, calling back before every entry so
//! the caller can report progress, wait while paused or stop.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use types::Error;

use crate::format::{Codec, Format};
use crate::index::normalize;
use crate::{ext_error, io_error};

/// One entry of the archive to be: a file or folder on disk and its path
/// inside the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The path inside the archive, normalised (`/` separators, no leading
    /// slash).
    pub path: Vec<u8>,
    /// Where the entry is on disk.
    pub source: PathBuf,
    /// What it is.
    pub kind: SourceKind,
    /// Its modification time.
    pub modified: SystemTime,
    /// Its Unix permission bits, where the platform has them.
    pub mode: Option<u32>,
}

/// What a source is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    /// A regular file of this many bytes.
    File {
        /// The size in bytes.
        size: u64,
    },
    /// A folder; its contents are sources of their own.
    Dir,
    /// A symbolic link to this target, stored as a link where the format
    /// has them (zip, tar) and as the target's contents otherwise.
    Symlink {
        /// The link's target, as the link stores it.
        target: PathBuf,
    },
}

impl Source {
    /// The bytes this entry adds to the total, for progress.
    #[must_use]
    pub fn size(&self) -> u64 {
        match &self.kind {
            SourceKind::File { size } => *size,
            SourceKind::Dir | SourceKind::Symlink { .. } => 0,
        }
    }
}

/// The entries for `items`: each item under its own file name at the top
/// of the archive, a folder with everything below it in a stable order
/// (folders before their contents, names sorted).
///
/// # Errors
///
/// The I/O error from reading an item or its folders; `Unsupported` for an
/// item without a file name.
pub fn collect(items: &[PathBuf]) -> Result<Vec<Source>, Error> {
    let mut out = Vec::new();
    for item in items {
        let name = item
            .file_name()
            .ok_or(Error::Unsupported("a root cannot be compressed"))?;
        let inner = normalize(name.as_encoded_bytes());
        walk(item, &inner, &mut out)?;
    }
    Ok(out)
}

fn source_of(path: &Path, inner: Vec<u8>) -> Result<Source, Error> {
    let meta = std::fs::symlink_metadata(path).map_err(Error::from_io)?;
    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let mode = ext::platform::fs::unix_mode(path).unwrap_or(None);
    let kind = if meta.file_type().is_symlink() {
        SourceKind::Symlink {
            target: std::fs::read_link(path).map_err(Error::from_io)?,
        }
    } else if meta.is_dir() {
        SourceKind::Dir
    } else {
        SourceKind::File { size: meta.len() }
    };
    Ok(Source {
        path: inner,
        source: path.to_path_buf(),
        kind,
        modified,
        mode,
    })
}

fn walk(path: &Path, inner: &[u8], out: &mut Vec<Source>) -> Result<(), Error> {
    let source = source_of(path, inner.to_vec())?;
    let is_dir = source.kind == SourceKind::Dir;
    out.push(source);
    if !is_dir {
        return Ok(());
    }
    let mut children: Vec<_> = std::fs::read_dir(path)
        .map_err(Error::from_io)?
        .collect::<Result<_, _>>()
        .map_err(Error::from_io)?;
    children.sort_by_key(std::fs::DirEntry::file_name);
    for child in children {
        let name = child.file_name();
        let mut child_inner = inner.to_vec();
        child_inner.push(b'/');
        child_inner.extend(normalize(name.as_encoded_bytes()));
        walk(&child.path(), &child_inner, out)?;
    }
    Ok(())
}

/// Called before every entry is written, with the entry and the bytes of
/// every entry before it; an error stops the write and leaves no archive.
pub type Before<'a> = dyn FnMut(&Source, u64) -> Result<(), Error> + 'a;

/// The archive formats that can be created, with the extension each one
/// is named by; `.gz` and the other single-file codecs take one file.
pub const CREATABLE: &[(&str, Format)] = &[
    ("zip", Format::Zip),
    ("tar.gz", Format::Tar(Some(Codec::Gzip))),
    ("tar.zst", Format::Tar(Some(Codec::Zstd))),
    ("7z", Format::SevenZ),
    ("tar", Format::Tar(None)),
    ("tar.xz", Format::Tar(Some(Codec::Xz))),
    ("tar.bz2", Format::Tar(Some(Codec::Bzip2))),
    ("gz", Format::Single(Codec::Gzip)),
    ("zst", Format::Single(Codec::Zstd)),
    ("xz", Format::Single(Codec::Xz)),
    ("bz2", Format::Single(Codec::Bzip2)),
];

/// The extension of a creatable format, as [`CREATABLE`] names it.
#[must_use]
pub fn extension_of(format: Format) -> Option<&'static str> {
    CREATABLE
        .iter()
        .find(|(_, f)| *f == format)
        .map(|(ext, _)| *ext)
}

/// Writes `sources` into a new archive of `format` at `out`, through a
/// temporary file beside it. `before` is called before every entry.
///
/// # Errors
///
/// `Unsupported` for a format that cannot be created (RAR, a disc image)
/// or a single-file codec with anything but one file; whatever `before`
/// returns; the I/O or library error otherwise. On an error nothing is
/// left at `out`.
pub fn create(
    format: Format,
    out: &Path,
    sources: &[Source],
    before: &mut Before<'_>,
) -> Result<(), Error> {
    if !format.writable() && !matches!(format, Format::Single(_)) {
        return Err(Error::Unsupported("this archive format cannot be created"));
    }
    let dir = out.parent().unwrap_or_else(|| Path::new("."));
    let temp = tempfile::Builder::new()
        .prefix(".rick-")
        .tempfile_in(dir)
        .map_err(Error::from_io)?;
    {
        let mut file = temp.as_file().try_clone().map_err(Error::from_io)?;
        match format {
            Format::Zip => write_zip(&mut file, sources, before)?,
            Format::Tar(codec) => write_tar(&mut file, codec, sources, before)?,
            Format::SevenZ => write_7z(&mut file, sources, before)?,
            Format::Single(codec) => write_single(&mut file, codec, sources, before)?,
            Format::Rar | Format::Image => {
                return Err(Error::Unsupported("this archive format cannot be created"));
            }
        }
        file.sync_all().map_err(Error::from_io)?;
    }
    temp.persist(out)
        .map_err(|error| Error::from_io(error.error))?;
    Ok(())
}

fn name_of(source: &Source) -> String {
    String::from_utf8_lossy(&source.path).into_owned()
}

fn write_zip(out: &mut File, sources: &[Source], before: &mut Before<'_>) -> Result<(), Error> {
    let mut writer = zip::ZipWriter::new(out);
    let mut done = 0;
    for source in sources {
        before(source, done)?;
        let name = name_of(source);
        let options = crate::zipfmt::options(source.modified, source.mode);
        match &source.kind {
            SourceKind::Dir => writer.add_directory(&name, options).map_err(ext_error)?,
            SourceKind::Symlink { target } => writer
                .add_symlink(&name, target.to_string_lossy(), options)
                .map_err(ext_error)?,
            SourceKind::File { size } => {
                writer.start_file(&name, options).map_err(ext_error)?;
                let mut file = File::open(&source.source).map_err(Error::from_io)?;
                io::copy(&mut file, &mut writer).map_err(io_error)?;
                done += size;
            }
        }
    }
    writer
        .finish()
        .map_err(ext_error)?
        .flush()
        .map_err(io_error)?;
    Ok(())
}

fn write_tar(
    out: &mut File,
    codec: Option<Codec>,
    sources: &[Source],
    before: &mut Before<'_>,
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
            None => out,
        };
        let mut builder = tar::Builder::new(writer);
        builder.follow_symlinks(false);
        let mut done = 0;
        for source in sources {
            before(source, done)?;
            let name = name_of(source);
            match &source.kind {
                SourceKind::Dir => {
                    let mut header = crate::tarfmt::header(
                        tar::EntryType::Directory,
                        0,
                        source.modified,
                        source.mode.unwrap_or(0o755),
                    );
                    builder
                        .append_data(&mut header, format!("{name}/"), io::empty())
                        .map_err(io_error)?;
                }
                SourceKind::Symlink { target } => {
                    let mut header = crate::tarfmt::header(
                        tar::EntryType::Symlink,
                        0,
                        source.modified,
                        source.mode.unwrap_or(0o777),
                    );
                    builder
                        .append_link(&mut header, &name, target)
                        .map_err(io_error)?;
                }
                SourceKind::File { size } => {
                    let mut header = crate::tarfmt::header(
                        tar::EntryType::Regular,
                        *size,
                        source.modified,
                        source.mode.unwrap_or(0o644),
                    );
                    let mut file = File::open(&source.source).map_err(Error::from_io)?;
                    builder
                        .append_data(&mut header, &name, &mut file)
                        .map_err(io_error)?;
                    done += size;
                }
            }
        }
        builder.finish().map_err(io_error)?;
    }
    if let Some(encoder) = encoder {
        encoder.finish().map_err(io_error)?;
    }
    out.flush().map_err(io_error)?;
    Ok(())
}

fn write_7z(out: &mut File, sources: &[Source], before: &mut Before<'_>) -> Result<(), Error> {
    let mut writer = sevenz_rust2::ArchiveWriter::new(out.try_clone().map_err(Error::from_io)?)
        .map_err(ext_error)?;
    let mut done = 0;
    for source in sources {
        before(source, done)?;
        let name = name_of(source);
        let time = sevenz_rust2::NtTime::try_from(source.modified).unwrap_or_default();
        match &source.kind {
            SourceKind::Dir => {
                let mut entry = sevenz_rust2::ArchiveEntry::new_directory(&name);
                entry.has_last_modified_date = true;
                entry.last_modified_date = time;
                writer
                    .push_archive_entry(entry, None::<File>)
                    .map_err(ext_error)?;
            }
            // 7z has no portable link entry: the target's contents go in.
            SourceKind::File { size: _ } | SourceKind::Symlink { .. } => {
                let mut entry = sevenz_rust2::ArchiveEntry::new_file(&name);
                entry.has_last_modified_date = true;
                entry.last_modified_date = time;
                let file = File::open(&source.source).map_err(Error::from_io)?;
                writer
                    .push_archive_entry(entry, Some(file))
                    .map_err(ext_error)?;
                done += source.size();
            }
        }
    }
    writer.finish().map_err(io_error)?;
    out.flush().map_err(io_error)?;
    Ok(())
}

fn write_single(
    out: &mut File,
    codec: Codec,
    sources: &[Source],
    before: &mut Before<'_>,
) -> Result<(), Error> {
    let [source] = sources else {
        return Err(Error::Unsupported(
            "a compressed file holds one file; use an archive for several",
        ));
    };
    if !matches!(source.kind, SourceKind::File { .. }) {
        return Err(Error::Unsupported(
            "a compressed file holds one file; use an archive for a folder",
        ));
    }
    before(source, 0)?;
    let sink: Box<dyn Write + Send> = Box::new(io::BufWriter::with_capacity(
        1 << 20,
        out.try_clone().map_err(Error::from_io)?,
    ));
    let mut encoder = codec.encoder(sink).map_err(io_error)?;
    let mut file = File::open(&source.source).map_err(Error::from_io)?;
    io::copy(&mut file, &mut *encoder).map_err(io_error)?;
    encoder.finish().map_err(io_error)?;
    out.flush().map_err(io_error)?;
    Ok(())
}
