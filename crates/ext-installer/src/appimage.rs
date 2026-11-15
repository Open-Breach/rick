//! AppImages (specs/ext-installer.md "AppImages"): telling one from any
//! other file, *Run*, which starts one as it is, and what the install form
//! needs of one. An AppImage is an ELF program, its runtime, with a
//! file-system image behind it. Nothing of it is run to learn about it and
//! nothing is unpacked or mounted: the header is read, and for type 2 the
//! desktop entry, the icon and the AppStream metainfo are read out of the
//! SquashFS image that starts where the ELF ends, as data, with links
//! followed only while they stay inside the image. Whether an AppImage
//! needs FUSE 2 is read from its runtime the same way.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use backhand::compression::{CompressionAction, Compressor, DefaultCompressor};
use backhand::{
    BackhandError, FilesystemCompressor, FilesystemReader, InnerNode, Node, SquashfsFileReader,
    SuperBlock,
};
use ext::platform::{Platform, fs as pfs};
use types::Error;

use crate::archive::{self, Format, Index, Item, ItemKind};
use crate::desktop::DesktopEntry;
use crate::detect::{self, Candidate, Detected, IconFile, ShippedDesktop};
use crate::elf;
use crate::error::{InstallError, err, io_read, io_write};
use crate::install::Libraries;
use crate::record::Kind;

/// The extension an AppImage usually carries, compared in any case.
const EXTENSION: &str = ".appimage";

/// Where the AppImage magic sits: bytes 8 to 10 of the ELF identification,
/// which the ELF format leaves as padding.
const MAGIC_AT: usize = 8;

/// The owner's execute bit.
const OWNER_EXECUTE: u32 = 0o100;

/// The library an AppImage on the older runtime needs to mount its image,
/// and newer distributions no longer ship by default.
pub const FUSE2: &str = "libfuse.so.2";

/// Where the AppImage project explains FUSE.
pub const FUSE_DOCUMENTATION: &str =
    "https://docs.appimage.org/user-guide/troubleshooting/fuse.html";

/// The warning an install carries, in the dialogue's summary and in the
/// job's closing one, when the AppImage needs FUSE 2 and the machine does
/// not have it: what is missing, the install line of each distribution
/// and where to read more. *Run* shows the same in a dialogue of its own.
pub const FUSE2_WARNING: &str = "FUSE 2 is required to run this AppImage: it needs libfuse.so.2, \
     which is not installed. Debian and Ubuntu: sudo apt install libfuse2 (libfuse2t64 on \
     Ubuntu 24.04 and later); Fedora: sudo dnf install fuse-libs; Arch: sudo pacman -S fuse2. \
     See https://docs.appimage.org/user-guide/troubleshooting/fuse.html";

/// How much of a file's head is searched for the library's name where the
/// header does not say where the runtime ends, and at most.
const RUNTIME_MAX: u64 = 4 << 20;

/// The most of a desktop entry or a metainfo file that is read: a longer
/// one is not the text a launcher shows.
const TEXT_MAX: usize = 1 << 20;

/// The largest icon taken out of an image.
const ICON_MAX: usize = 8 << 20;

/// How many links are followed from one name before it counts as a loop.
const LINK_HOPS: usize = 16;

/// The machine names a file name on the usual `Name-<version>-<arch>`
/// pattern ends in.
const MACHINES: [&str; 10] = [
    "x86_64", "amd64", "x64", "aarch64", "arm64", "armhf", "armv7l", "i386", "i686", "riscv64",
];

/// What separates the parts of a file name.
const SEPARATORS: [char; 4] = ['-', '_', '.', ' '];

/// The AppImage type an ELF header declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    /// Type 1: an ISO 9660 image behind the runtime.
    One,
    /// Type 2: a SquashFS image behind the runtime.
    Two,
}

/// Whether `name` ends in `.AppImage`, in any case.
#[must_use]
pub fn has_extension(name: &str) -> bool {
    name.len()
        .checked_sub(EXTENSION.len())
        .and_then(|at| name.get(at..))
        .is_some_and(|end| end.eq_ignore_ascii_case(EXTENSION))
}

/// The AppImage type `head`, the first bytes of a file, declares: an ELF
/// header that carries `AI` and then `0x01` or `0x02` at offset 8.
#[must_use]
pub fn magic(head: &[u8]) -> Option<Type> {
    if !elf::is_elf(head) {
        return None;
    }
    match head.get(MAGIC_AT..MAGIC_AT + 3)? {
        [b'A', b'I', 1] => Some(Type::One),
        [b'A', b'I', 2] => Some(Type::Two),
        _ => None,
    }
}

/// The AppImage type the header of the file at `path` declares; `None`
/// for a file without the magic and for one that cannot be read.
#[must_use]
pub fn magic_of(path: &Path) -> Option<Type> {
    let mut head = [0u8; MAGIC_AT + 3];
    let mut file = File::open(path).ok()?;
    file.read_exact(&mut head).ok()?;
    magic(&head)
}

/// Whether the file at `path`, listed as `name`, is an AppImage: its name
/// ends in `.AppImage`, or its ELF header carries the AppImage magic. The
/// file is read only when the name does not say so.
#[must_use]
pub fn is_appimage(path: &Path, name: &str) -> bool {
    has_extension(name) || magic_of(path).is_some()
}

/// Whether the AppImage at `path` needs FUSE 2 to run, read from its
/// runtime — the ELF at the head of the file, up to where the image
/// starts — without running it. The older runtime loads `libfuse.so.2` by
/// name, so the name stands in it, among its needed libraries or as the
/// string it opens the library by; the static runtime carries no such
/// name. The image behind the runtime is not searched, so a file inside it
/// that happens to hold the name changes nothing.
///
/// # Errors
///
/// The I/O error from reading the file.
pub fn needs_fuse2(path: &Path) -> std::io::Result<bool> {
    let mut file = File::open(path)?;
    let Some(header) = elf::read(&mut file)? else {
        return Ok(false);
    };
    if header.needed.iter().any(|library| library == FUSE2) {
        return Ok(true);
    }
    let length = match header.end {
        0 => RUNTIME_MAX,
        end => end.min(RUNTIME_MAX),
    };
    file.seek(SeekFrom::Start(0))?;
    let mut runtime = Vec::new();
    file.take(length).read_to_end(&mut runtime)?;
    Ok(runtime
        .windows(FUSE2.len())
        .any(|window| window == FUSE2.as_bytes()))
}

/// How a *Run* ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ran {
    /// The file was started.
    Started,
    /// The file needs FUSE 2 and the machine does not have it: nothing was
    /// started, and the caller shows the warning.
    NeedsFuse2,
}

/// *Run*: starts the AppImage at `path` as it is. The file's ELF machine
/// type is compared with the running machine first, and a mismatch stops
/// here; the execute bit is set for the owner when it is missing, nothing
/// else about the file changes; where the file needs FUSE 2
/// ([`needs_fuse2`]) and `libraries` does not have it, nothing is started
/// and the answer says so; otherwise the file is started detached from
/// Rick, with its folder as the working directory.
///
/// # Errors
///
/// [`InstallError::NotAProgram`] for a file that is not ELF,
/// [`InstallError::AppImageArchitecture`] for one built for another
/// machine, and the I/O error from reading the file, setting the bit or
/// starting the file.
pub fn run(platform: &dyn Platform, libraries: &Libraries, path: &Path) -> Result<Ran, Error> {
    let header = elf::read_file(path)
        .map_err(Error::from_io)?
        .ok_or_else(|| err(InstallError::NotAProgram))?;
    if let Some(running) = elf::current_machine()
        && running != header.machine
    {
        return Err(err(InstallError::AppImageArchitecture {
            found: elf::machine_name(header.machine),
            running: elf::machine_name(running),
        }));
    }
    if let Some(mode) = pfs::unix_mode(path).map_err(Error::from_io)?
        && mode & OWNER_EXECUTE == 0
    {
        pfs::set_unix_mode(path, mode | OWNER_EXECUTE).map_err(io_write)?;
    }
    if needs_fuse2(path).map_err(Error::from_io)? && !libraries.has(FUSE2) {
        return Ok(Ran::NeedsFuse2);
    }
    let dir = path.parent().unwrap_or(path);
    platform.run_detached(path, &[], dir)?;
    Ok(Ran::Started)
}

/// What an AppImage holds for the launcher, read out of its image as data.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bundled {
    /// The desktop entry at the top of the image: its file's name and its
    /// text.
    pub desktop: Option<(String, String)>,
    /// The icon: `.DirIcon`, or the one the entry names.
    pub icon: Option<BundledIcon>,
    /// The newest release the AppStream metainfo lists, by its version.
    pub release: Option<String>,
}

/// An icon read out of an AppImage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledIcon {
    /// Its name at the top of the image.
    pub name: String,
    /// `png` or `svg`, by what the bytes are.
    pub ext: &'static str,
    /// The file's bytes.
    pub data: Vec<u8>,
}

/// How the blocks of an image are unpacked: by the SquashFS crate's own
/// compressors, and for zstd by the `zstd` crate the workspace has.
///
/// This adapter exists for one reason. `backhand`'s own zstd support asks
/// for `zstd` 0.13, and the workspace, `zip` and `sevenz-rust2` are on
/// 0.14, so its `zstd` feature would put a second zstd into the lock
/// file. The crate is built without that feature and handed this in its
/// place, through its hook for a compressor of one's own.
///
/// It goes away once a `backhand` release asks for `zstd` 0.14: switch
/// the `zstd` feature back on in the workspace's `Cargo.toml`, open the
/// image with `FilesystemReader::from_reader_with_offset` again, and
/// delete this type, [`CODECS`] and the writer's twin in the fixtures.
/// `cargo deny check` fails while two versions of zstd are in the lock
/// file, so the feature cannot come back early unnoticed.
struct Codecs;

impl CompressionAction for Codecs {
    type Error = BackhandError;
    type Compressor = Compressor;
    type FilesystemCompressor = FilesystemCompressor;
    type SuperBlock = SuperBlock;

    fn decompress(
        &self,
        bytes: &[u8],
        out: &mut Vec<u8>,
        compressor: Compressor,
    ) -> Result<(), BackhandError> {
        if compressor == Compressor::Zstd {
            // As the SquashFS crate's own zstd support does it.
            zstd::bulk::Decompressor::new()?.decompress_to_buffer(bytes, out)?;
            return Ok(());
        }
        DefaultCompressor.decompress(bytes, out, compressor)
    }

    /// Nothing is written through this: an image is only read.
    fn compress(
        &self,
        bytes: &[u8],
        compressor: FilesystemCompressor,
        block_size: u32,
    ) -> Result<Vec<u8>, BackhandError> {
        DefaultCompressor.compress(bytes, compressor, block_size)
    }
}

/// The one [`Codecs`], which the SquashFS crate keeps a reference to.
static CODECS: Codecs = Codecs;

/// The entries of an image by their path inside it, `/`-separated and
/// without a leading slash.
type Nodes<'a> = HashMap<String, &'a Node<SquashfsFileReader>>;

/// A path of the image as [`Nodes`] keys it.
fn inside(path: &Path) -> String {
    path.components()
        .filter_map(|part| match part {
            std::path::Component::Normal(name) => Some(name.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The regular file `path` names inside the image. A link is followed
/// only when its target is inside the image: an absolute target and one
/// that climbs out are not, and neither is a chain that does not end.
fn resolve<'a>(nodes: &Nodes<'a>, path: &str) -> Option<&'a SquashfsFileReader> {
    let mut at = path.to_owned();
    for _ in 0..LINK_HOPS {
        let node: &'a Node<SquashfsFileReader> = nodes.get(at.as_str())?;
        match &node.inner {
            InnerNode::File(file) => return Some(file),
            InnerNode::Symlink(link) => {
                at = archive::resolve_link(&at, &link.link.to_string_lossy())?;
            }
            _ => return None,
        }
    }
    None
}

/// The bytes of the file `path` names inside the image, when it is one of
/// at most `max` bytes.
fn read_inside(
    image: &FilesystemReader<'_>,
    nodes: &Nodes<'_>,
    path: &str,
    max: usize,
) -> Option<Vec<u8>> {
    let file = resolve(nodes, path)?;
    if file.file_len() > max {
        return None;
    }
    let mut bytes = Vec::new();
    image
        .file(file)
        .reader()
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= max).then_some(bytes)
}

/// `png` or `svg`, by what an icon's bytes are; `None` for anything else,
/// an XPM included.
fn icon_kind(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("png");
    }
    let head = String::from_utf8_lossy(&data[..data.len().min(4096)]);
    let text = head.trim_start_matches('\u{feff}').trim_start();
    (text.starts_with('<') && text.contains("<svg")).then_some("svg")
}

/// The value of the attribute `name` among a tag's `attributes`.
fn attribute(attributes: &str, name: &str) -> Option<String> {
    let mut rest = attributes;
    loop {
        rest = rest.trim_start();
        let key_end = rest
            .find(|c: char| c == '=' || c == '/' || c.is_whitespace())
            .unwrap_or(rest.len());
        let key = &rest[..key_end];
        if key.is_empty() {
            return None;
        }
        rest = rest[key_end..].trim_start();
        let Some(value) = rest.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let value = &value[1..];
        let end = value.find(quote)?;
        if key == name {
            return Some(value[..end].trim().to_owned());
        }
        rest = &value[end + 1..];
    }
}

/// The versions of the `<release>` elements of an AppStream metainfo
/// file, in the file's order. The text is scanned for the tags, comments
/// left out; it is not parsed as XML.
#[must_use]
pub fn release_versions(xml: &str) -> Vec<String> {
    let mut versions = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find('<') {
        rest = &rest[at..];
        if let Some(comment) = rest.strip_prefix("<!--") {
            rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
            continue;
        }
        let end = rest.find('>').unwrap_or(rest.len());
        let attributes = rest[1..end]
            .strip_prefix("release")
            .filter(|after| after.starts_with(char::is_whitespace));
        if let Some(version) = attributes.and_then(|a| attribute(a, "version")) {
            versions.push(version);
        }
        rest = &rest[end..];
    }
    versions
}

/// Reads what the AppImage at `path` holds for the launcher out of its
/// SquashFS image, which starts where the file's ELF ends: the `.desktop`
/// file at the top of the image, the icon (`.DirIcon`, or the PNG or SVG
/// the entry's `Icon` names) and the newest release of the metainfo under
/// `usr/share/metainfo`. `None` when there is no image the reader takes
/// there: an AppImage of type 1, a compression it does not have, a
/// damaged image. Blocking: a job's work.
#[must_use]
pub fn read_bundled(path: &Path) -> Option<Bundled> {
    let mut file = File::open(path).ok()?;
    let start = elf::read(&mut file).ok()??.end;
    if start == 0 {
        return None;
    }
    let image = FilesystemReader::from_reader_with_offset_and_kind(
        BufReader::new(file),
        start,
        backhand::kind::Kind::new_v4(&CODECS),
    )
    .ok()?;
    let nodes: Nodes<'_> = image
        .files()
        .map(|node| (inside(&node.fullpath), node))
        .collect();
    let mut top: Vec<&str> = nodes
        .keys()
        .map(String::as_str)
        .filter(|path| !path.is_empty() && !path.contains('/'))
        .collect();
    top.sort_unstable();

    let desktop = top
        .iter()
        .filter(|name| name.to_ascii_lowercase().ends_with(".desktop"))
        .find_map(|name| {
            let bytes = read_inside(&image, &nodes, name, TEXT_MAX)?;
            Some((
                (*name).to_owned(),
                String::from_utf8_lossy(&bytes).into_owned(),
            ))
        });

    // The icon the entry names, by its bare name: `Icon=fix`, and what a
    // careless entry writes, `Icon=/usr/share/pixmaps/fix.png`.
    let named = desktop
        .as_ref()
        .and_then(|(_, text)| DesktopEntry::parse(text).get("Icon").map(str::to_owned))
        .map(|icon| icon.rsplit('/').next().unwrap_or(&icon).to_owned())
        .map(|icon| {
            let lower = icon.to_ascii_lowercase();
            match [".png", ".svg", ".xpm"]
                .iter()
                .find(|e| lower.ends_with(*e))
            {
                Some(ext) => icon[..icon.len() - ext.len()].to_owned(),
                None => icon,
            }
        })
        .filter(|icon| !icon.is_empty());
    let mut icon_names = vec![".DirIcon".to_owned()];
    if let Some(named) = named {
        icon_names.push(format!("{named}.png"));
        icon_names.push(format!("{named}.svg"));
    }
    let icon = icon_names.into_iter().find_map(|name| {
        let data = read_inside(&image, &nodes, &name, ICON_MAX)?;
        let ext = icon_kind(&data)?;
        Some(BundledIcon { name, ext, data })
    });

    let mut metainfo: Vec<&str> = nodes
        .keys()
        .map(String::as_str)
        .filter(|path| {
            path.strip_prefix("usr/share/metainfo/")
                .is_some_and(|name| {
                    !name.contains('/') && name.to_ascii_lowercase().ends_with(".xml")
                })
        })
        .collect();
    metainfo.sort_unstable();
    let release = metainfo
        .iter()
        .filter_map(|path| read_inside(&image, &nodes, path, TEXT_MAX))
        .flat_map(|bytes| release_versions(&String::from_utf8_lossy(&bytes)))
        .filter(|version| detect::valid_version(version))
        .max_by(|a, b| detect::compare_versions(a, b));

    Some(Bundled {
        desktop,
        icon,
        release,
    })
}

/// A file name without its `.AppImage` ending.
fn stem(name: &str) -> &str {
    if has_extension(name) {
        &name[..name.len() - EXTENSION.len()]
    } else {
        name
    }
}

/// The app's name as its file's name gives it, on the usual
/// `Name-<version>-<arch>.AppImage` pattern: what stands before the
/// version, or the whole of it without a machine name at its end.
fn name_from(stem: &str) -> String {
    let before = match detect::split_version(stem) {
        Some((_, version)) => &stem[..version.start],
        None => stem,
    };
    let mut name = before.trim_matches(SEPARATORS);
    for machine in MACHINES {
        let Some(cut) = name.len().checked_sub(machine.len()) else {
            continue;
        };
        if name
            .get(cut..)
            .is_some_and(|end| end.eq_ignore_ascii_case(machine))
            && (cut == 0 || name[..cut].ends_with(SEPARATORS))
        {
            name = name[..cut].trim_end_matches(SEPARATORS);
            break;
        }
    }
    if name.is_empty() {
        detect::strip_version(stem)
    } else {
        name.to_owned()
    }
}

/// Detects what the AppImage at `path` is and proposes the install form's
/// defaults, without running it. Name and icon come from the desktop
/// entry and the icon of its image; the version is the first of
/// `X-AppImage-Version` in that entry, the newest release of its
/// metainfo, and the digits in the file's name, and stays empty when none
/// of them has one. Where the image cannot be read the name and the
/// version are the file name's and there is no icon. The AppImage itself
/// is the one program, and the architecture is its own header's; whether
/// it needs FUSE 2 is read from its runtime.
///
/// # Errors
///
/// [`InstallError::NotAProgram`] for a file that is no ELF program;
/// `Error::NotFound`, `Error::PermissionDenied` or the I/O error from
/// reading it.
pub fn detect(path: &Path) -> Result<Detected, Error> {
    let size = std::fs::metadata(path).map_err(io_read)?.len();
    let file_name = path
        .file_name()
        .map_or_default(|name| name.to_string_lossy().into_owned());
    let header = elf::read_file(path)
        .map_err(io_read)?
        .ok_or_else(|| err(InstallError::NotAProgram))?;
    // Type 1 carries an ISO 9660 image, which is not read.
    let bundled = if magic_of(path) == Some(Type::One) {
        Bundled::default()
    } else {
        read_bundled(path).unwrap_or_default()
    };

    let desktop = bundled.desktop.map(|(rel, text)| ShippedDesktop {
        rel,
        entry: DesktopEntry::parse(&text),
    });
    let stem = stem(&file_name);
    let (file_version, source_pattern) = detect::split_version(stem).map_or_else(
        || (None, file_name.clone()),
        |(version, range)| {
            let mut pattern = stem.to_owned();
            pattern.replace_range(range, "*");
            (Some(version), pattern + &file_name[stem.len()..])
        },
    );
    let version = desktop
        .as_ref()
        .and_then(|d| d.entry.get("X-AppImage-Version"))
        .map(str::to_owned)
        .filter(|version| !version.is_empty())
        .or(bundled.release)
        .or(file_version)
        .unwrap_or_default();
    let name = desktop
        .as_ref()
        .and_then(|d| d.entry.get("Name"))
        .map(str::to_owned)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| name_from(stem));

    let icons: Vec<IconFile> = bundled
        .icon
        .into_iter()
        .map(|icon| IconFile {
            rel: icon.name,
            ext: icon.ext.to_owned(),
            size: detect::png_size(&icon.data[..icon.data.len().min(24)]),
            bytes: icon.data.len() as u64,
            data: Some(Arc::new(icon.data)),
        })
        .collect();
    let arch_mismatch = elf::current_machine()
        .filter(|running| *running != header.machine)
        .map(|_| header.machine);

    Ok(Detected {
        archive: path.to_path_buf(),
        format: Format::AppImage,
        index: Arc::new(Index {
            items: vec![Item {
                path: file_name.clone(),
                kind: ItemKind::File,
                size,
                mode: 0o755,
                link: None,
                hostile: false,
            }],
            total_size: size,
            top_level: None,
        }),
        source: false,
        candidates: vec![Candidate {
            rel: file_name,
            elf: header,
        }],
        main: Some(0),
        kind: Kind::Gui,
        kind_reason: "an AppImage is an application for the launcher",
        desktop,
        icon_trusted: !icons.is_empty(),
        icons,
        name,
        version,
        source_pattern,
        bins: Vec::new(),
        install_script: None,
        arch_mismatch,
        hostile: Vec::new(),
        needs_fuse2: needs_fuse2(path).unwrap_or(false),
    })
}
