//! Archive previews: the contents of a zip, tar (plain or compressed with
//! gzip, zstd, xz, bzip2 or lz4), 7z, RAR or ISO 9660 file, or of a disk
//! image (raw, DMG, VHD, VHDX, VMDK, VDI, QCOW2, through the 7-Zip library
//! the archive provider finds), as a read-only file list, without
//! extracting or mounting anything.
//!
//! `open` reads the archive's directory (or walks a tar's headers, skipping
//! the data) on the blocking pool, builds an [`ext::Listing`] in explorer
//! order (folders first, then names, each directory followed by its
//! contents) and keeps it; `render` gives the same list as text for a pane
//! that lays out no listings. Encrypted entries are listed with their
//! names and marked; an archive whose directory itself is encrypted (7z
//! and RAR with encrypted headers) lists nothing and says so (QUESTIONS.md
//! 88). The status bar shows the entry count, the compressed and
//! uncompressed sizes and the format. A disk image without the 7-Zip
//! library opens as a [`disk::MissingDoc`]: the file's facts and the
//! [`ext::MissingLibrary`] content that names the library, how to install
//! it on this platform and the archive provider's setting, which the pane
//! lays out with *Browse…* (the choice stored into that setting, shared by
//! both plugins) and *Retry*.

#![deny(unsafe_code)]

pub mod disk;
pub mod fixtures;
mod iso;
mod rar;
mod sevenz;
mod tar;
mod zip;

use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use ext::Text;
use ext::platform::ImageFormat;
use ext::{
    API_VERSION, Capabilities, Content, ControlId, ControlValue, Extension, Frame, Icon,
    InfoSection, Listing, ListingEntry, Manifest, Platforms, Points, PreviewControl, PreviewDoc,
    Previewer, Provider, Registry, RenderRequest, TextBlock, TextLayer, Transport, Version,
    async_trait,
};
pub use ext_vfs_archive::Resolver;
use types::{CancelToken, Entry, EntryKind, Error, ExtId, Location};

/// The extension id.
pub const ID: ExtId = ExtId("preview-archive");

/// The listing stops after this many entries and says so.
pub const MAX_ENTRIES: usize = 200_000;

/// Walking a compressed tar reads through it; the walk stops after this
/// long and the listing says it is cut.
pub const TIME_BUDGET: Duration = Duration::from_secs(10);

/// An archive on another provider than the local one is copied to a
/// temporary file first; above this size it is not previewed.
pub const REMOTE_LIMIT: usize = 256 * 1024 * 1024;

/// The file name extensions the manifest claims: what [`detect`] knows by
/// name, the disk images of `ImageFormat::for_name` among them.
const PREVIEWS: &[&str] = &[
    "zip", "jar", "war", "ear", "apk", "epub", "xpi", "7z", "rar", "iso", "tar", "tar.gz", "tgz",
    "tar.zst", "tzst", "tar.xz", "txz", "tar.bz2", "tbz", "tbz2", "tar.lz4", "tlz4", "img", "ima",
    "raw", "dmg", "vhd", "vhdx", "vmdk", "vdi", "qcow2", "qcow",
];

/// How many entries the textual fallback lists.
const FALLBACK_LINES: usize = 10_000;

/// The extension: registers [`ArchivePreviewer`].
#[derive(Debug)]
pub struct ArchivePreview {
    manifest: Manifest,
}

impl Default for ArchivePreview {
    fn default() -> Self {
        Self::new()
    }
}

impl ArchivePreview {
    /// The extension with its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Archive preview",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PREVIEWER,
                platforms: Platforms::ALL,
                previews: PREVIEWS,
                commands: &[],
            },
        }
    }
}

impl Extension for ArchivePreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The previewer, its disk images read with the 7-Zip library found
    /// through the archive provider's setting in the store the host lends
    /// and the bundle under the host's config directory.
    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        reg.previewer(Arc::new(ArchivePreviewer::new(Arc::new(Resolver::new(
            reg.settings_store().cloned(),
            reg.platform().map(|p| p.config_dir()),
        )))));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(ArchivePreview::new()) as Arc<dyn Extension>);

/// Previews archives and disk images.
#[derive(Debug)]
pub struct ArchivePreviewer {
    resolver: Arc<Resolver>,
}

impl Default for ArchivePreviewer {
    /// A previewer finding the 7-Zip library without a setting: the
    /// bundle and the platform's usual places.
    fn default() -> Self {
        Self::new(Resolver::shared())
    }
}

impl ArchivePreviewer {
    /// A previewer whose disk images are read with the library `resolver`
    /// finds.
    #[must_use]
    pub fn new(resolver: Arc<Resolver>) -> Self {
        Self { resolver }
    }

    /// The library resolver.
    #[must_use]
    pub fn resolver(&self) -> &Arc<Resolver> {
        &self.resolver
    }
}

/// How a tar is compressed, if at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// A plain tar.
    None,
    /// gzip (`.tar.gz`, `.tgz`).
    Gzip,
    /// Zstandard (`.tar.zst`).
    Zstd,
    /// xz (`.tar.xz`).
    Xz,
    /// bzip2 (`.tar.bz2`).
    Bzip2,
    /// lz4 frames (`.tar.lz4`).
    Lz4,
}

/// The archive formats this previewer lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// zip and its relatives (jar, apk, epub).
    Zip,
    /// tar, compressed or not.
    Tar(Compression),
    /// 7-Zip.
    SevenZ,
    /// RAR 4 and 5.
    Rar,
    /// An ISO 9660 disc image, through the native reader.
    Iso,
    /// A disk image of the Disk images table, through the 7-Zip library;
    /// never [`ImageFormat::Iso`].
    Disk(ImageFormat),
}

impl Format {
    /// The name shown in the status bar.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Tar(Compression::None) => "tar",
            Self::Tar(Compression::Gzip) => "tar.gz",
            Self::Tar(Compression::Zstd) => "tar.zst",
            Self::Tar(Compression::Xz) => "tar.xz",
            Self::Tar(Compression::Bzip2) => "tar.bz2",
            Self::Tar(Compression::Lz4) => "tar.lz4",
            Self::SevenZ => "7z",
            Self::Rar => "RAR",
            Self::Iso | Self::Disk(ImageFormat::Iso) => "ISO 9660",
            Self::Disk(ImageFormat::Img) => "raw disk image",
            Self::Disk(ImageFormat::Dmg) => "DMG",
            Self::Disk(ImageFormat::Vhd) => "VHD",
            Self::Disk(ImageFormat::Vhdx) => "VHDX",
            Self::Disk(ImageFormat::Vmdk) => "VMDK",
            Self::Disk(ImageFormat::Vdi) => "VDI",
            Self::Disk(ImageFormat::Qcow2) => "QCOW2",
        }
    }
}

/// The disk-image container whose signature `sniff` starts with: VHDX,
/// QCOW2, VMDK, a dynamic VHD, VDI. A fixed VHD's footer, a DMG's trailer
/// and a raw image carry nothing at the start.
fn disk_by_magic(sniff: &[u8]) -> Option<ImageFormat> {
    if sniff.starts_with(b"vhdxfile") {
        Some(ImageFormat::Vhdx)
    } else if sniff.starts_with(b"QFI\xfb") {
        Some(ImageFormat::Qcow2)
    } else if sniff.starts_with(b"KDMV") || sniff.starts_with(b"# Disk DescriptorFile") {
        Some(ImageFormat::Vmdk)
    } else if sniff.starts_with(b"conectix") {
        Some(ImageFormat::Vhd)
    } else if sniff.starts_with(b"<<< ")
        && sniff.len() >= 0x44
        && sniff[0x40..0x44] == [0x7f, 0x10, 0xda, 0xbe]
    {
        Some(ImageFormat::Vdi)
    } else {
        None
    }
}

/// The lower-case suffix of `name` after its first dot that matters:
/// `tar.gz` for `a.tar.gz`, `zip` for `A.ZIP`.
fn suffixes(name: &str) -> Vec<String> {
    let lower = name.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut rest = lower.as_str();
    while let Some((_, tail)) = rest.split_once('.') {
        if !tail.is_empty() {
            out.push(tail.to_owned());
        }
        rest = tail;
    }
    out
}

/// Whether `bytes` carry the ustar magic at the tar header's offset.
fn has_ustar_magic(bytes: &[u8]) -> bool {
    bytes.len() >= 263 && &bytes[257..262] == b"ustar"
}

/// Whether the first bytes of the stream `decoder` decodes are a tar
/// header. The sniff is the start of the compressed file, so a cut stream
/// is expected; what came out before the cut is what counts.
fn decodes_to_tar(mut decoder: impl Read) -> bool {
    let mut head = vec![0u8; 512];
    let mut filled = 0;
    while filled < head.len() {
        match decoder.read(&mut head[filled..]) {
            Ok(n) if n > 0 => filled += n,
            _ => break,
        }
    }
    has_ustar_magic(&head[..filled])
}

/// What the file is, from its name and first bytes, with the score to
/// claim it: 150 when the content says so, 120 for a name whose content
/// could not be checked (an ISO's signature sits at 32 KiB; `bzip2` and
/// `lz4` decode nothing from a 4 KiB sniff), 60 for a name the content
/// contradicts, `None` otherwise.
#[must_use]
pub fn detect(name: &str, sniff: &[u8]) -> Option<(Format, u8)> {
    let suffixes = suffixes(name);
    let named = |candidates: &[&str]| candidates.iter().any(|c| suffixes.iter().any(|s| s == c));
    let by_magic = if sniff.starts_with(b"PK\x03\x04")
        || sniff.starts_with(b"PK\x05\x06")
        || sniff.starts_with(b"PK\x07\x08")
    {
        Some(Format::Zip)
    } else if sniff.starts_with(b"7z\xBC\xAF\x27\x1C") {
        Some(Format::SevenZ)
    } else if sniff.starts_with(b"Rar!\x1A\x07") {
        Some(Format::Rar)
    } else if has_ustar_magic(sniff) {
        Some(Format::Tar(Compression::None))
    } else if sniff.starts_with(b"\x1F\x8B") {
        decodes_to_tar(flate2::read::GzDecoder::new(sniff))
            .then_some(Format::Tar(Compression::Gzip))
            .or_else(|| named(&["tar.gz", "tgz"]).then_some(Format::Tar(Compression::Gzip)))
    } else if sniff.starts_with(b"\x28\xB5\x2F\xFD") {
        zstd::stream::read::Decoder::new(sniff)
            .is_ok_and(decodes_to_tar)
            .then_some(Format::Tar(Compression::Zstd))
            .or_else(|| named(&["tar.zst", "tzst"]).then_some(Format::Tar(Compression::Zstd)))
    } else if sniff.starts_with(b"\xFD7zXZ\x00") {
        decodes_to_tar(lzma_rust2::XzReader::new(sniff, true))
            .then_some(Format::Tar(Compression::Xz))
            .or_else(|| named(&["tar.xz", "txz"]).then_some(Format::Tar(Compression::Xz)))
    } else if sniff.starts_with(b"BZh") {
        return named(&["tar.bz2", "tbz", "tbz2"])
            .then_some((Format::Tar(Compression::Bzip2), 120));
    } else if sniff.starts_with(b"\x04\x22\x4D\x18") {
        return named(&["tar.lz4", "tlz4"]).then_some((Format::Tar(Compression::Lz4), 120));
    } else {
        disk_by_magic(sniff).map(Format::Disk)
    };
    if let Some(format) = by_magic {
        return Some((format, 150));
    }
    // A disk image whose name is all there is: a fixed VHD, a DMG, a raw
    // image; the library checks the content on `open`.
    if let Some(image) = ImageFormat::for_name(name).filter(|f| *f != ImageFormat::Iso) {
        return Some((Format::Disk(image), 120));
    }
    let by_name = if named(&["zip", "jar", "war", "ear", "apk", "epub", "xpi"]) {
        Format::Zip
    } else if named(&["7z"]) {
        Format::SevenZ
    } else if named(&["rar"]) {
        Format::Rar
    } else if named(&["iso"]) {
        // The signature sits at sector 16, past the sniff: the name is
        // all there is until `open` checks it.
        return Some((Format::Iso, 120));
    } else if named(&["tar"]) {
        // A V7 tar carries no magic; `open` checks the checksum.
        return Some((Format::Tar(Compression::None), 120));
    } else if named(&["tar.gz", "tgz"]) {
        Format::Tar(Compression::Gzip)
    } else if named(&["tar.zst", "tzst"]) {
        Format::Tar(Compression::Zstd)
    } else if named(&["tar.xz", "txz"]) {
        Format::Tar(Compression::Xz)
    } else if named(&["tar.bz2", "tbz", "tbz2"]) {
        Format::Tar(Compression::Bzip2)
    } else if named(&["tar.lz4", "tlz4"]) {
        Format::Tar(Compression::Lz4)
    } else {
        return None;
    };
    // Named but the content says otherwise (or is empty): a weak claim.
    Some((by_name, if sniff.is_empty() { 120 } else { 60 }))
}

/// One entry as a format module reads it, before the tree is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    /// The path inside the archive as stored; separators of either kind,
    /// a trailing one for a directory.
    pub path: String,
    /// What the entry is.
    pub kind: EntryKind,
    /// Uncompressed size.
    pub size: Option<u64>,
    /// Compressed size, where the format keeps one per entry.
    pub compressed: Option<u64>,
    /// Modification time.
    pub modified: Option<SystemTime>,
    /// The content is encrypted.
    pub encrypted: bool,
}

/// Archive-level facts a format module reports beside the entries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Facts {
    /// The entry list itself is encrypted; nothing could be listed.
    pub encrypted_headers: bool,
    /// Compressed as one stream (7z, RAR).
    pub solid: bool,
    /// Extra key and value rows for File info: a comment, a volume label.
    pub rows: Vec<(String, String)>,
}

/// What a format module produces.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listed {
    /// The entries, in the archive's order.
    pub entries: Vec<RawEntry>,
    /// The walk stopped before the end.
    pub truncated: bool,
    /// Archive-level facts.
    pub facts: Facts,
}

/// When a listing walk must stop.
#[derive(Debug, Clone)]
pub struct Limit {
    /// Entries at most.
    pub max_entries: usize,
    /// Stop after this instant.
    pub deadline: Instant,
    /// Stop when cancelled.
    pub cancel: CancelToken,
}

impl Limit {
    /// The limits `open` uses.
    #[must_use]
    pub fn standard(cancel: CancelToken) -> Self {
        Self {
            max_entries: MAX_ENTRIES,
            deadline: Instant::now() + TIME_BUDGET,
            cancel,
        }
    }

    /// Whether a walk that has `listed` entries should stop; checks the
    /// clock and the token every 256 entries.
    #[must_use]
    pub fn reached(&self, listed: usize) -> bool {
        listed >= self.max_entries
            || (listed.is_multiple_of(256)
                && (self.cancel.is_cancelled() || Instant::now() > self.deadline))
    }
}

/// An I/O error with a message, for a file that is not what it claims.
fn invalid(message: impl Into<String>) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        message.into(),
    ))
}

/// A `SystemTime` for a civil date and time taken as UTC; `None` outside
/// the representable range.
#[must_use]
pub fn civil_to_system_time(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<SystemTime> {
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    // Days from civil (Howard Hinnant), 1970-01-01 is day 0.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds =
        days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second);
    if seconds >= 0 {
        SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(u64::try_from(seconds).ok()?))
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(Duration::from_secs(u64::try_from(-seconds).ok()?))
    }
}

/// A `SystemTime` for an MS-DOS date and time pair, as zip and RAR store
/// them.
#[must_use]
pub fn dos_to_system_time(date: u16, time: u16) -> Option<SystemTime> {
    civil_to_system_time(
        i64::from(date >> 9) + 1980,
        u32::from((date >> 5) & 0xF),
        u32::from(date & 0x1F),
        u32::from(time >> 11),
        u32::from((time >> 5) & 0x3F),
        u32::from(time & 0x1F) * 2,
    )
}

/// A `SystemTime` for seconds since the Unix epoch.
#[must_use]
pub fn unix_to_system_time(seconds: u64) -> Option<SystemTime> {
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
}

/// A byte count as `n B`, `n.n KiB` or `n.n MiB`.
#[must_use]
pub fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    // Sizes shown to one decimal; the precision loss is immaterial.
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if value < KIB * KIB {
        format!("{:.1} KiB", value / KIB)
    } else if value < KIB * KIB * KIB {
        format!("{:.1} MiB", value / (KIB * KIB))
    } else {
        format!("{:.2} GiB", value / (KIB * KIB * KIB))
    }
}

/// Counts over a listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Totals {
    /// Files (and links and other non-directories).
    pub files: usize,
    /// Directories, implicit ones included.
    pub dirs: usize,
    /// Sum of the known uncompressed sizes.
    pub size: u64,
    /// Sum of the known compressed sizes, when any entry has one.
    pub compressed: Option<u64>,
    /// Entries with encrypted content.
    pub encrypted: usize,
}

/// The components of a stored path: separators of either kind, a leading
/// `./` or `/` and empty components dropped.
fn components(path: &str) -> Vec<&str> {
    path.split(['/', '\\'])
        .filter(|c| !c.is_empty() && *c != ".")
        .collect()
}

/// Key of one entry in the ordered tree: one (folder-first flag,
/// lower-case name, exact name) per path component, so that folders at a
/// level sort before files and names compare without case.
type Key = Vec<(u8, String, String)>;

fn key_of(parts: &[String], is_dir: bool) -> Key {
    parts
        .iter()
        .enumerate()
        .map(|(i, part)| {
            let last = i + 1 == parts.len();
            (u8::from(last && !is_dir), part.to_lowercase(), part.clone())
        })
        .collect()
}

/// The entries as a tree in explorer order: implicit directories added,
/// duplicates resolved to the last one.
fn tree(raw: Vec<RawEntry>) -> std::collections::BTreeMap<Key, (Vec<String>, RawEntry)> {
    let mut nodes = std::collections::BTreeMap::new();
    for mut entry in raw {
        let is_dir = matches!(entry.kind, EntryKind::Dir)
            || entry.path.ends_with('/')
            || entry.path.ends_with('\\');
        let parts: Vec<String> = components(&entry.path)
            .into_iter()
            .map(str::to_owned)
            .collect();
        if parts.is_empty() {
            continue;
        }
        // Every ancestor is a directory, implicit until an entry names it.
        for depth in 1..parts.len() {
            let ancestor = &parts[..depth];
            nodes.entry(key_of(ancestor, true)).or_insert_with(|| {
                (
                    ancestor.to_vec(),
                    RawEntry {
                        path: ancestor.join("/"),
                        kind: EntryKind::Dir,
                        size: None,
                        compressed: None,
                        modified: None,
                        encrypted: false,
                    },
                )
            });
        }
        if is_dir {
            entry.kind = EntryKind::Dir;
        }
        nodes.insert(key_of(&parts, is_dir), (parts, entry));
    }
    nodes
}

/// The stats line under the list.
fn stats_line(totals: &Totals, facts: &Facts, truncated: bool, shown: usize) -> String {
    let mut stats = if facts.encrypted_headers {
        "Encrypted archive: the list of files needs a password".to_owned()
    } else {
        let mut s = format!(
            "{} file{}, {} folder{}, {}",
            totals.files,
            if totals.files == 1 { "" } else { "s" },
            totals.dirs,
            if totals.dirs == 1 { "" } else { "s" },
            format_size(totals.size)
        );
        if let Some(compressed) = totals.compressed {
            let _ = write!(s, " ({} compressed)", format_size(compressed));
        }
        if totals.encrypted > 0 {
            let _ = write!(s, ", {} encrypted", totals.encrypted);
        }
        s
    };
    if truncated {
        let _ = write!(stats, " · first {shown} entries shown");
    }
    stats
}

/// Builds the explorer-ordered tree from the entries a format read:
/// implicit directories added, duplicates resolved to the last one,
/// folders before files at each level, names compared without case. Also
/// returns the totals.
#[must_use]
pub fn build_listing(raw: Vec<RawEntry>, truncated: bool, facts: &Facts) -> (Listing, Totals) {
    let mut totals = Totals::default();
    let mut any_compressed = false;
    let mut compressed_sum = 0u64;
    let entries: Vec<ListingEntry> = tree(raw)
        .into_values()
        .map(|(parts, entry)| {
            let is_dir = entry.kind == EntryKind::Dir;
            if is_dir {
                totals.dirs += 1;
            } else {
                totals.files += 1;
                totals.size = totals.size.saturating_add(entry.size.unwrap_or(0));
                if let Some(c) = entry.compressed {
                    any_compressed = true;
                    compressed_sum = compressed_sum.saturating_add(c);
                }
            }
            if entry.encrypted {
                totals.encrypted += 1;
            }
            ListingEntry {
                name: parts.last().cloned().unwrap_or_default(),
                path: parts.join("/"),
                depth: parts.len() - 1,
                kind: entry.kind,
                size: if is_dir { None } else { entry.size },
                compressed: if is_dir { None } else { entry.compressed },
                modified: entry.modified,
                encrypted: entry.encrypted,
            }
        })
        .collect();
    totals.compressed = any_compressed.then_some(compressed_sum);
    let stats = stats_line(&totals, facts, truncated, entries.len());
    (
        Listing {
            entries,
            stats,
            truncated,
        },
        totals,
    )
}

/// The archive file the format modules read: the local file itself, or a
/// temporary copy of a file on another provider.
enum Source {
    Local(PathBuf),
    Temp(tempfile::NamedTempFile),
}

impl Source {
    fn path(&self) -> &Path {
        match self {
            Self::Local(path) => path,
            Self::Temp(file) => file.path(),
        }
    }
}

/// The OS path of a local location; `None` for other providers.
fn local_path(loc: &Location) -> Option<PathBuf> {
    (loc.provider.0 == "local").then(|| PathBuf::from(loc.path.to_string_lossy().into_owned()))
}

/// Lists the archive at `path` as `format`; a disk image is read with the
/// 7-Zip library `resolver` finds.
///
/// # Errors
///
/// `Error::Io` with `InvalidData` when the file is not an archive of that
/// format, the file's own I/O error, `Error::Cancelled`, or the
/// missing-library error for a disk image without the 7-Zip library.
pub fn list(
    resolver: &Resolver,
    path: &Path,
    format: Format,
    limit: &Limit,
) -> Result<Listed, Error> {
    let listed = match format {
        Format::Zip => zip::list(path, limit),
        Format::Tar(compression) => tar::list(path, compression, limit),
        Format::SevenZ => sevenz::list(path, limit),
        Format::Rar => rar::list(path, limit),
        Format::Iso => iso::list(path, limit),
        Format::Disk(image) => disk::list(resolver, path, image, limit),
    }?;
    if limit.cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    Ok(listed)
}

/// An opened archive.
pub struct ArchiveDoc {
    listing: Arc<Listing>,
    totals: Totals,
    facts: Facts,
    format: Format,
    file_size: Option<u64>,
    layer: TextLayer,
}

impl std::fmt::Debug for ArchiveDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveDoc")
            .field("format", &self.format)
            .field("entries", &self.listing.entries.len())
            .finish_non_exhaustive()
    }
}

impl ArchiveDoc {
    /// Builds the document from what a format module listed; `file_size`
    /// is the archive's own size when known.
    #[must_use]
    pub fn new(format: Format, listed: Listed, file_size: Option<u64>) -> Self {
        let (listing, totals) = build_listing(listed.entries, listed.truncated, &listed.facts);
        let text = fallback_text(&listing);
        Self {
            listing: Arc::new(listing),
            totals,
            facts: listed.facts,
            format,
            file_size,
            layer: TextLayer { pages: vec![text] },
        }
    }

    /// The listing.
    #[must_use]
    pub fn listing(&self) -> &Arc<Listing> {
        &self.listing
    }

    /// The totals over the listing.
    #[must_use]
    pub fn totals(&self) -> Totals {
        self.totals
    }

    /// The format.
    #[must_use]
    pub fn format(&self) -> Format {
        self.format
    }
}

/// The list as text: one line per entry, indented by depth, the size after
/// a file's name, the stats line first.
fn fallback_text(listing: &Listing) -> String {
    let mut out = String::new();
    out.push_str(&listing.stats);
    out.push('\n');
    for entry in listing.entries.iter().take(FALLBACK_LINES) {
        out.push('\n');
        out.extend(std::iter::repeat_n("  ", entry.depth));
        out.push_str(&entry.name);
        match entry.kind {
            EntryKind::Dir => out.push('/'),
            EntryKind::Symlink { ref target, .. } => {
                out.push_str(" -> ");
                out.push_str(&target.to_string_lossy());
            }
            _ => {}
        }
        if let Some(size) = entry.size {
            out.push_str("  ");
            out.push_str(&format_size(size));
        }
        if entry.encrypted {
            out.push_str("  (encrypted)");
        }
    }
    if listing.entries.len() > FALLBACK_LINES {
        let _ = write!(
            out,
            "\n\n… and {} more",
            listing.entries.len() - FALLBACK_LINES
        );
    }
    out
}

impl PreviewDoc for ArchiveDoc {
    fn pages(&self) -> Option<usize> {
        None
    }

    fn render(&mut self, page: usize, _req: RenderRequest) -> Result<Frame, Error> {
        if page != 0 {
            return Err(Error::NotFound);
        }
        Ok(Frame::Text(TextBlock {
            text: self.layer.pages[0].clone(),
            language: None,
            spans: Vec::new(),
        }))
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        Some(&self.layer)
    }

    fn content(&self) -> Option<Content> {
        Some(Content::Listing(Arc::clone(&self.listing)))
    }

    fn sections(&self) -> Vec<InfoSection> {
        let mut rows = vec![
            ("Format".to_owned(), self.format.name().to_owned()),
            ("Entries".to_owned(), self.listing.entries.len().to_string()),
            ("Files".to_owned(), self.totals.files.to_string()),
            ("Folders".to_owned(), self.totals.dirs.to_string()),
            ("Uncompressed".to_owned(), format_size(self.totals.size)),
        ];
        let compressed = self.totals.compressed.or(self.file_size);
        if let Some(compressed) = compressed {
            rows.push(("Compressed".to_owned(), format_size(compressed)));
            if self.totals.size > 0 {
                // A percentage; the rounding is the point.
                #[allow(
                    clippy::cast_precision_loss,
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss
                )]
                let percent = (compressed as f64 * 100.0 / self.totals.size as f64).round() as u64;
                rows.push(("Ratio".to_owned(), format!("{percent} %")));
            }
        }
        if let Some(size) = self.file_size {
            rows.push(("Size".to_owned(), format_size(size)));
        }
        if self.facts.encrypted_headers {
            rows.push(("Encrypted".to_owned(), "the file list itself".to_owned()));
        } else if self.totals.encrypted > 0 {
            rows.push((
                "Encrypted".to_owned(),
                format!(
                    "{} entr{}",
                    self.totals.encrypted,
                    if self.totals.encrypted == 1 {
                        "y"
                    } else {
                        "ies"
                    }
                ),
            ));
        }
        if self.facts.solid {
            rows.push(("Solid".to_owned(), "Yes".to_owned()));
        }
        rows.extend(self.facts.rows.iter().cloned());
        if self.listing.truncated {
            rows.push((
                "Shown".to_owned(),
                format!("first {} entries", self.listing.entries.len()),
            ));
        }
        vec![InfoSection {
            id: "file",
            title: Text("preview-archive-file-info"),
            icon: Icon::Info,
            rows,
        }]
    }

    fn status(&self) -> Vec<String> {
        let entries = self.listing.entries.len();
        let sizes = match (
            self.totals.compressed.or(self.file_size),
            self.facts.encrypted_headers,
        ) {
            (_, true) => "encrypted".to_owned(),
            (Some(compressed), false) => {
                format!(
                    "{} → {}",
                    format_size(compressed),
                    format_size(self.totals.size)
                )
            }
            (None, false) => format_size(self.totals.size),
        };
        vec![
            format!(
                "{entries}{} entr{}",
                if self.listing.truncated { "+" } else { "" },
                if entries == 1 { "y" } else { "ies" }
            ),
            sizes,
            self.format.name().to_owned(),
        ]
    }

    fn controls(&self) -> Vec<PreviewControl> {
        Vec::new()
    }

    fn set_control(&mut self, _id: ControlId, _value: ControlValue) -> Result<(), Error> {
        Err(Error::Unsupported("archives have no controls of their own"))
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

#[async_trait]
impl Previewer for ArchivePreviewer {
    fn id(&self) -> ExtId {
        ID
    }

    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8 {
        if entry.kind != EntryKind::File {
            return 0;
        }
        detect(&entry.name, sniff).map_or(0, |(_, score)| score)
    }

    async fn open(
        &self,
        loc: &Location,
        provider: Arc<dyn Provider>,
        cancel: CancelToken,
    ) -> Result<Box<dyn PreviewDoc>, Error> {
        let entry = provider.stat(loc).await?;
        let sniff = ext::read_bytes(&*provider, loc, Some(4096), &cancel).await?;
        let (format, _) = detect(&entry.name, &sniff)
            .ok_or(Error::Unsupported("not an archive this previewer lists"))?;
        let source = if let Some(path) = local_path(loc) {
            Source::Local(path)
        } else {
            let bytes = ext::read_bytes(&*provider, loc, Some(REMOTE_LIMIT + 1), &cancel).await?;
            if bytes.len() > REMOTE_LIMIT {
                return Err(Error::Unsupported(
                    "archive on a remote provider larger than 256 MiB",
                ));
            }
            let file = ext::blocking(move || {
                let mut file = tempfile::Builder::new()
                    .prefix("rick-archive-")
                    .tempfile()
                    .map_err(Error::from_io)?;
                std::io::Write::write_all(&mut file, &bytes).map_err(Error::from_io)?;
                Ok(file)
            })
            .await?;
            Source::Temp(file)
        };
        let limit = Limit::standard(cancel.clone());
        let size = entry.size;
        let name = entry.name.clone();
        let resolver = Arc::clone(&self.resolver);
        let doc = ext::blocking(move || {
            match list(&resolver, source.path(), format, &limit) {
                Ok(listed) => {
                    Ok(Box::new(ArchiveDoc::new(format, listed, size)) as Box<dyn PreviewDoc>)
                }
                // Without the library the pane gets the message with
                // Browse… and Retry, not an error.
                Err(error) => match ext_vfs_archive::MissingLibrary::in_error(&error) {
                    Some(missing) => {
                        Ok(Box::new(disk::MissingDoc::new(missing, format, name, size)))
                    }
                    None => Err(error),
                },
            }
        })
        .await?;
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        Ok(doc)
    }
}

#[cfg(test)]
mod tests;
