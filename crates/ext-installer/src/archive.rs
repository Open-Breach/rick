//! Reading the archive: its format, its index together with the heads of
//! chosen entries in one streaming pass, and the one-pass extraction under
//! the archive-safety rules.
//!
//! The crate reads tar (plain, gzip, xz, bzip2, zstd) and zip with the
//! `tar` and `zip` crates itself, so it depends on no other plugin
//! (specs/ext-installer.md "Plugin architecture"). Every path is
//! normalised to `/`-separated components without `.`; an entry that is
//! absolute or carries `..` is marked and never written; links are kept
//! only when their target stays inside the folder; special entries are
//! skipped; set-uid and set-gid bits are stripped.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use types::{CancelToken, Error};

use crate::error::{InstallError, err, io_read, io_write};

/// The compression around a tar stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// A plain `.tar`.
    Plain,
    /// `.tar.gz`, `.tgz`.
    Gzip,
    /// `.tar.xz`, `.txz`.
    Xz,
    /// `.tar.bz2`, `.tbz2`.
    Bzip2,
    /// `.tar.zst`, `.tzst`.
    Zstd,
}

/// An archive format the installer reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A tar stream behind a codec.
    Tar(Codec),
    /// A zip file.
    Zip,
}

impl Format {
    /// The format an archive's name says it is, by extension; `None` for
    /// a name the installer does not take.
    #[must_use]
    pub fn by_name(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        let ends = |suffixes: &[&str]| suffixes.iter().any(|s| lower.ends_with(s));
        if ends(&[".tar.gz", ".tgz"]) {
            Some(Self::Tar(Codec::Gzip))
        } else if ends(&[".tar.xz", ".txz"]) {
            Some(Self::Tar(Codec::Xz))
        } else if ends(&[".tar.bz2", ".tbz2", ".tbz"]) {
            Some(Self::Tar(Codec::Bzip2))
        } else if ends(&[".tar.zst", ".tzst"]) {
            Some(Self::Tar(Codec::Zstd))
        } else if ends(&[".tar"]) {
            Some(Self::Tar(Codec::Plain))
        } else if ends(&[".zip"]) {
            Some(Self::Zip)
        } else {
            None
        }
    }

    /// The format of the file at `path`: what its name says, corrected by
    /// the magic bytes when they disagree, so a misnamed download still
    /// opens.
    ///
    /// # Errors
    ///
    /// `Error::NotFound` or the I/O error from reading the head; the
    /// installer's `Corrupt` error when neither the name nor the bytes say
    /// what the file is.
    pub fn detect(path: &Path) -> Result<Self, Error> {
        let mut head = [0u8; 512 + 8];
        let mut file = File::open(path).map_err(io_read)?;
        let got = read_up_to(&mut file, &mut head).map_err(io_read)?;
        let head = &head[..got];
        let by_magic = if head.starts_with(&[0x1f, 0x8b]) {
            Some(Self::Tar(Codec::Gzip))
        } else if head.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
            Some(Self::Tar(Codec::Xz))
        } else if head.starts_with(b"BZh") {
            Some(Self::Tar(Codec::Bzip2))
        } else if head.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
            Some(Self::Tar(Codec::Zstd))
        } else if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
            Some(Self::Zip)
        } else if head.len() >= 262 && &head[257..262] == b"ustar" {
            Some(Self::Tar(Codec::Plain))
        } else {
            None
        };
        let by_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(Self::by_name);
        match (by_name, by_magic) {
            (_, Some(magic)) => Ok(magic),
            (Some(Self::Tar(Codec::Plain)), None) => Ok(Self::Tar(Codec::Plain)),
            (Some(named), None) => Err(err(InstallError::Corrupt(format!(
                "the file is named like a {} archive but does not start like one",
                named.name()
            )))),
            (None, None) => Err(err(InstallError::Corrupt(
                "not a tar or zip archive".to_owned(),
            ))),
        }
    }

    /// `tar.gz`, `zip`, …
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Tar(Codec::Plain) => "tar",
            Self::Tar(Codec::Gzip) => "tar.gz",
            Self::Tar(Codec::Xz) => "tar.xz",
            Self::Tar(Codec::Bzip2) => "tar.bz2",
            Self::Tar(Codec::Zstd) => "tar.zst",
            Self::Zip => "zip",
        }
    }
}

/// What kind of entry an archive item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// A regular file.
    File,
    /// A folder.
    Dir,
    /// A symbolic link; [`Item::link`] is its target.
    Symlink,
    /// A hard link to an earlier entry; [`Item::link`] names it.
    HardLink,
    /// A device node, FIFO, socket or anything else: never written.
    Other,
}

/// One entry of the archive, as the index lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The normalised path: `/`-separated, no empty or `.` components.
    pub path: String,
    /// What it is.
    pub kind: ItemKind,
    /// Its size in bytes (0 for anything but a file).
    pub size: u64,
    /// Its Unix permission bits as recorded, `0o644` when the archive
    /// records none.
    pub mode: u32,
    /// A link's target as recorded.
    pub link: Option<String>,
    /// The recorded path was absolute, carried `..`, or held a NUL: the
    /// entry is listed and never written.
    pub hostile: bool,
}

impl Item {
    /// The file name, the last component.
    #[must_use]
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// The first component: the top-level folder or file.
    #[must_use]
    pub fn top(&self) -> &str {
        self.path.split('/').next().unwrap_or(&self.path)
    }
}

/// The archive's table of contents.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Index {
    /// Every entry, in the archive's order.
    pub items: Vec<Item>,
    /// The sum of the file sizes: what the archive unpacks to.
    pub total_size: u64,
    /// The one folder every entry lives under, when there is one; `None`
    /// for a tarbomb, whose entries land directly in the version folder.
    pub top_level: Option<String>,
}

impl Index {
    /// An entry's path relative to the version folder: without the
    /// top-level folder when there is one. `None` for the folder itself.
    #[must_use]
    pub fn relative<'a>(&self, path: &'a str) -> Option<&'a str> {
        match &self.top_level {
            None => Some(path),
            Some(top) => {
                let rest = path.strip_prefix(top.as_str())?;
                let rest = rest.strip_prefix('/')?;
                (!rest.is_empty()).then_some(rest)
            }
        }
    }

    /// The item at the relative path `rel`, if any.
    #[must_use]
    pub fn at(&self, rel: &str) -> Option<&Item> {
        self.items
            .iter()
            .find(|i| self.relative(&i.path) == Some(rel))
    }

    /// Whether the archive holds the file `rel`.
    #[must_use]
    pub fn has_file(&self, rel: &str) -> bool {
        self.at(rel).is_some_and(|i| i.kind == ItemKind::File)
    }
}

/// The most entries an archive may have before it counts as pathological.
pub const MAX_ENTRIES: usize = 200_000;

/// Normalises a recorded path; the flag says whether it was absolute,
/// climbed with `..` or held a NUL.
fn normalise(raw: &[u8]) -> (String, bool) {
    let mut hostile = raw.first().is_some_and(|b| *b == b'/' || *b == b'\\')
        || raw.contains(&0)
        || (raw.len() >= 2 && raw[1] == b':');
    let mut out = String::new();
    for component in String::from_utf8_lossy(raw).split(['/', '\\']) {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            hostile = true;
        }
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(component);
    }
    (out, hostile)
}

/// The folder of a normalised path, empty at the top.
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// Where a link at `at` with `target` points, as a normalised path inside
/// the folder; `None` when the target is absolute or climbs out.
#[must_use]
pub fn resolve_link(at: &str, target: &str) -> Option<String> {
    if target.starts_with('/') || target.starts_with('\\') || target.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = parent(at).split('/').filter(|p| !p.is_empty()).collect();
    for component in target.split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

fn read_up_to<R: Read>(source: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match source.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// A reader that counts the bytes read through it: the compressed stream
/// behind a decoder, for progress against the file's size.
struct Counting<R> {
    inner: R,
    read: Arc<AtomicU64>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let got = self.inner.read(buf)?;
        self.read.fetch_add(got as u64, Ordering::Relaxed);
        Ok(got)
    }
}

/// The decoded tar stream of `path` and the counter of the compressed
/// bytes read from the file so far.
fn tar_reader(path: &Path, codec: Codec) -> Result<(Box<dyn Read>, Arc<AtomicU64>), Error> {
    let read = Arc::new(AtomicU64::new(0));
    let file = BufReader::with_capacity(
        1 << 16,
        Counting {
            inner: File::open(path).map_err(io_read)?,
            read: Arc::clone(&read),
        },
    );
    let reader: Box<dyn Read> = match codec {
        Codec::Plain => Box::new(file),
        Codec::Gzip => Box::new(flate2::read::MultiGzDecoder::new(file)),
        Codec::Xz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(file)),
        Codec::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(file)),
        Codec::Zstd => Box::new(zstd::stream::read::Decoder::new(file).map_err(io_read)?),
    };
    Ok((reader, read))
}

/// Whether to go on after an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visit {
    /// Next entry.
    Next,
    /// Stop reading.
    Stop,
}

/// Walks the archive once, in its own order, handing every entry and a
/// reader over its contents to `f`. Reading from the reader is optional:
/// an entry not read is skipped at the format's own pace.
///
/// # Errors
///
/// What `f` returns; the installer's `Corrupt` error when the archive
/// cannot be read, with a cap on the entry count.
pub fn visit(
    path: &Path,
    format: Format,
    f: &mut dyn FnMut(&Item, &mut dyn Read) -> Result<Visit, Error>,
) -> Result<(), Error> {
    visit_counting(path, format, &mut |item, reader, _| f(item, reader))
}

/// What [`visit_counting`] calls per entry: the entry, a reader over its
/// contents, and the bytes of the file read so far.
type CountingVisitor<'a> = dyn FnMut(&Item, &mut dyn Read, u64) -> Result<Visit, Error> + 'a;

/// [`visit`] with the bytes of the file read so far as a third argument:
/// the compressed stream behind a tar's decoder, the compressed sizes of
/// the entries passed for a zip, which is read by seeking.
fn visit_counting(path: &Path, format: Format, f: &mut CountingVisitor<'_>) -> Result<(), Error> {
    match format {
        Format::Tar(codec) => visit_tar(path, codec, f),
        Format::Zip => visit_zip(path, f),
    }
}

fn visit_tar(path: &Path, codec: Codec, f: &mut CountingVisitor<'_>) -> Result<(), Error> {
    let (reader, read) = tar_reader(path, codec)?;
    let mut archive = tar::Archive::new(reader);
    let mut count = 0usize;
    for entry in archive.entries().map_err(io_read)? {
        let mut entry = entry.map_err(io_read)?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err(err(InstallError::Corrupt(format!(
                "more than {MAX_ENTRIES} entries"
            ))));
        }
        let (normalised, hostile) = normalise(&entry.path_bytes());
        if normalised.is_empty() {
            continue;
        }
        let header = entry.header();
        let kind = entry.header().entry_type();
        let kind = if kind.is_dir() {
            ItemKind::Dir
        } else if kind.is_symlink() {
            ItemKind::Symlink
        } else if kind.is_hard_link() {
            ItemKind::HardLink
        } else if kind.is_file() || kind.is_contiguous() {
            ItemKind::File
        } else {
            ItemKind::Other
        };
        let link = entry
            .link_name_bytes()
            .map(|l| String::from_utf8_lossy(&l).into_owned());
        let item = Item {
            path: normalised,
            kind,
            size: if kind == ItemKind::File {
                entry.size()
            } else {
                0
            },
            mode: header.mode().unwrap_or(0o644),
            link,
            hostile,
        };
        if f(&item, &mut entry, read.load(Ordering::Relaxed))? == Visit::Stop {
            return Ok(());
        }
    }
    Ok(())
}

fn visit_zip(path: &Path, f: &mut CountingVisitor<'_>) -> Result<(), Error> {
    let file = File::open(path).map_err(io_read)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
    if archive.len() > MAX_ENTRIES {
        return Err(err(InstallError::Corrupt(format!(
            "more than {MAX_ENTRIES} entries"
        ))));
    }
    let mut passed: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(zip_error)?;
        passed = passed.saturating_add(entry.compressed_size());
        let (normalised, hostile) = normalise(entry.name_raw());
        if normalised.is_empty() {
            continue;
        }
        let kind = if entry.is_dir() {
            ItemKind::Dir
        } else if entry.is_symlink() {
            ItemKind::Symlink
        } else {
            ItemKind::File
        };
        let mode = entry
            .unix_mode()
            .unwrap_or(if kind == ItemKind::Dir { 0o755 } else { 0o644 });
        let mut link = None;
        if kind == ItemKind::Symlink {
            let mut target = Vec::new();
            (&mut entry)
                .take(4096)
                .read_to_end(&mut target)
                .map_err(io_read)?;
            link = Some(String::from_utf8_lossy(&target).into_owned());
        }
        let item = Item {
            path: normalised,
            kind,
            size: if kind == ItemKind::File {
                entry.size()
            } else {
                0
            },
            mode,
            link,
            hostile,
        };
        if f(&item, &mut entry, passed)? == Visit::Stop {
            return Ok(());
        }
    }
    Ok(())
}

fn zip_error(error: zip::result::ZipError) -> Error {
    match error {
        zip::result::ZipError::Io(io) => io_read(io),
        other => err(InstallError::Corrupt(other.to_string())),
    }
}

/// Reads the archive's table of contents.
///
/// # Errors
///
/// The errors of [`scan`].
pub fn index(path: &Path, format: Format) -> Result<Index, Error> {
    scan(
        path,
        format,
        &CancelToken::new(),
        &mut |_| {},
        &mut |_, _| Ok(()),
    )
}

/// Reads the archive's table of contents in one streaming pass, handing
/// every file entry and a reader over its contents to `probe` on the way,
/// so the first bytes of the files that matter (ELF headers, desktop
/// entries, icon headers, shebangs) are read in the same sweep. `cancel`
/// is checked between entries; `progress` is told the bytes of the file
/// read so far, against which the caller sets the file's size.
///
/// # Errors
///
/// `Error::Cancelled` when `cancel` fires; what `probe` returns; the
/// errors of [`visit`].
pub fn scan(
    path: &Path,
    format: Format,
    cancel: &CancelToken,
    progress: &mut OnProgress<'_>,
    probe: &mut dyn FnMut(&Item, &mut dyn Read) -> Result<(), Error>,
) -> Result<Index, Error> {
    let mut items = Vec::new();
    visit_counting(path, format, &mut |item, reader, read| {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        items.push(item.clone());
        if item.kind == ItemKind::File && !item.hostile {
            probe(item, reader)?;
        }
        progress(read);
        Ok(Visit::Next)
    })?;
    let total_size = items.iter().map(|i| i.size).sum();
    let top_level = top_level(&items);
    Ok(Index {
        items,
        total_size,
        top_level,
    })
}

/// The single top-level folder every entry lives under, if there is one.
fn top_level(items: &[Item]) -> Option<String> {
    let sane: Vec<&Item> = items.iter().filter(|i| !i.hostile).collect();
    let first = sane.first()?.top().to_owned();
    if first.is_empty() {
        return None;
    }
    let all_under = sane.iter().all(|i| i.top() == first);
    let is_dir = sane
        .iter()
        .any(|i| i.path == first && i.kind == ItemKind::Dir)
        || sane.iter().any(|i| i.path.len() > first.len() + 1);
    let has_children = sane.iter().any(|i| i.path.len() > first.len());
    (all_under && is_dir && has_children).then_some(first)
}

/// What an extraction left out, for the install summary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Skipped {
    /// The entry's recorded path and why it was not written.
    pub entries: Vec<(String, String)>,
}

/// Progress of an extraction: bytes written so far.
pub type OnProgress<'a> = dyn FnMut(u64) + 'a;

/// The permission bits an entry keeps: no set-uid, set-gid or sticky bit.
const MODE_MASK: u32 = 0o777;

/// Unpacks the archive into `dest`, which must exist and be empty, under
/// the archive-safety rules: hostile paths, outside-pointing links, special
/// entries and duplicate names are left out and listed; set-uid and
/// set-gid bits are stripped; the bytes written are checked against what
/// the index declares.
///
/// # Errors
///
/// `Error::Cancelled` when `cancel` fires; the installer's `StorageFull`
/// or `Corrupt` errors; `Error::PermissionDenied` or the I/O error from
/// writing.
#[allow(clippy::too_many_lines)]
pub fn extract(
    path: &Path,
    format: Format,
    index: &Index,
    dest: &Path,
    cancel: &CancelToken,
    progress: &mut OnProgress<'_>,
) -> Result<Skipped, Error> {
    let mut skipped = Skipped::default();
    let mut written_paths: HashSet<String> = HashSet::new();
    let mut written_bytes: u64 = 0;
    let budget = index.total_size.saturating_mul(2).saturating_add(64 << 20);
    let mut buf = vec![0u8; 1 << 16];
    visit(path, format, &mut |item, reader| {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if item.hostile {
            skipped.entries.push((
                item.path.clone(),
                "its path is absolute or leaves the folder".to_owned(),
            ));
            return Ok(Visit::Next);
        }
        let Some(rel) = index.relative(&item.path) else {
            // The top-level folder itself.
            return Ok(Visit::Next);
        };
        let target = dest.join(rel);
        match item.kind {
            ItemKind::Dir => {
                std::fs::create_dir_all(&target).map_err(io_write)?;
                let _ = ext::platform::fs::set_unix_mode(&target, item.mode & MODE_MASK | 0o700);
            }
            ItemKind::File => {
                if !written_paths.insert(rel.to_owned()) {
                    skipped
                        .entries
                        .push((item.path.clone(), "written twice by the archive".to_owned()));
                    return Ok(Visit::Next);
                }
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(io_write)?;
                }
                let mut file = File::create(&target).map_err(io_write)?;
                loop {
                    let got = reader.read(&mut buf).map_err(io_read)?;
                    if got == 0 {
                        break;
                    }
                    written_bytes += got as u64;
                    if written_bytes > budget {
                        return Err(err(InstallError::Corrupt(
                            "the archive unpacks to far more than it declares".to_owned(),
                        )));
                    }
                    io::Write::write_all(&mut file, &buf[..got]).map_err(io_write)?;
                    if cancel.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                }
                drop(file);
                progress(written_bytes);
                let _ = ext::platform::fs::set_unix_mode(&target, item.mode & MODE_MASK | 0o600);
            }
            ItemKind::Symlink => {
                let Some(link) = item.link.as_deref() else {
                    skipped
                        .entries
                        .push((item.path.clone(), "a link without a target".to_owned()));
                    return Ok(Visit::Next);
                };
                let inside = resolve_link(rel, link).is_some();
                if !inside {
                    skipped.entries.push((
                        item.path.clone(),
                        format!("a link to {link}, outside the app's folder"),
                    ));
                    return Ok(Visit::Next);
                }
                if !written_paths.insert(rel.to_owned()) {
                    skipped
                        .entries
                        .push((item.path.clone(), "written twice by the archive".to_owned()));
                    return Ok(Visit::Next);
                }
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(io_write)?;
                }
                ext::platform::fs::symlink(Path::new(link), &target).map_err(io_write)?;
            }
            ItemKind::HardLink => {
                let source = item
                    .link
                    .as_deref()
                    .map(|l| normalise(l.as_bytes()))
                    .and_then(|(p, hostile)| (!hostile).then_some(p))
                    .and_then(|p| index.relative(&p).map(str::to_owned))
                    .filter(|p| written_paths.contains(p));
                let Some(source) = source else {
                    skipped.entries.push((
                        item.path.clone(),
                        "a hard link to something outside the app's folder".to_owned(),
                    ));
                    return Ok(Visit::Next);
                };
                if !written_paths.insert(rel.to_owned()) {
                    skipped
                        .entries
                        .push((item.path.clone(), "written twice by the archive".to_owned()));
                    return Ok(Visit::Next);
                }
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(io_write)?;
                }
                std::fs::hard_link(dest.join(&source), &target).map_err(io_write)?;
            }
            ItemKind::Other => {
                skipped.entries.push((
                    item.path.clone(),
                    "a device node, FIFO or other special entry".to_owned(),
                ));
            }
        }
        Ok(Visit::Next)
    })?;
    progress(written_bytes);
    Ok(skipped)
}

/// The SHA-256 of the file at `path`, as lower-case hex, with `progress`
/// told the bytes hashed so far.
///
/// # Errors
///
/// `Error::Cancelled`, or the I/O error from reading.
pub fn sha256(
    path: &Path,
    cancel: &CancelToken,
    progress: &mut OnProgress<'_>,
) -> Result<String, Error> {
    use sha2::Digest as _;
    let mut file = File::open(path).map_err(io_read)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let got = file.read(&mut buf).map_err(io_read)?;
        if got == 0 {
            break;
        }
        hasher.update(&buf[..got]);
        done += got as u64;
        progress(done);
    }
    Ok(hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        }))
}

/// The OS path of an archive entry inside an extracted folder.
#[must_use]
pub fn path_in(dest: &Path, rel: &str) -> PathBuf {
    dest.join(rel)
}
