//! Detection: the archive's index and the heads of its candidate files
//! answer the spec's five questions (source or prebuilt, main executable,
//! GUI or CLI, icon, name and version) and fill the install form with
//! defaults. Headers and text only; nothing from the archive runs.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use types::Error;

use crate::archive::{self, Format, Index, ItemKind, Visit};
use crate::desktop::{self, DesktopEntry};
use crate::elf::{self, Elf};
use crate::record::Kind;

/// How much of an ELF candidate is read for its dynamic section.
const ELF_HEAD: u64 = 1 << 20;
/// How much of a text or icon file is read.
const TEXT_MAX: u64 = 1 << 20;

/// Top-level files that mean a source tree.
const SOURCE_MARKERS: [&str; 11] = [
    "configure",
    "configure.ac",
    "autogen.sh",
    "Makefile",
    "CMakeLists.txt",
    "Cargo.toml",
    "meson.build",
    "setup.py",
    "pyproject.toml",
    "build.gradle",
    "go.mod",
];

/// Libraries a GUI program links against; a `DT_NEEDED` entry starting
/// with one of these means GUI.
const GUI_LIBRARIES: [&str; 13] = [
    "libX11",
    "libxcb",
    "libwayland-client",
    "libgtk",
    "libgdk",
    "libQt",
    "libSDL",
    "libGL.",
    "libGLX",
    "libEGL",
    "libglfw",
    "libgio",
    "libatk",
];

/// A top-level script an archive asks the user to run; never run by Rick.
const INSTALL_SCRIPTS: [&str; 6] = [
    "install.sh",
    "install",
    "INSTALL.sh",
    "setup.sh",
    "postinst",
    "uninstall.sh",
];

/// The categories the form offers.
pub const CATEGORIES: [&str; 11] = [
    "Utility",
    "Development",
    "Network",
    "Office",
    "Graphics",
    "AudioVideo",
    "Game",
    "Education",
    "Science",
    "Settings",
    "System",
];

/// An executable found in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Its path relative to the version folder.
    pub rel: String,
    /// What its header says.
    pub elf: Elf,
}

impl Candidate {
    /// The file name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.rel.rsplit('/').next().unwrap_or(&self.rel)
    }

    /// Whether it sits at the top level or in `bin/`.
    #[must_use]
    pub fn is_prominent(&self) -> bool {
        !self.rel.contains('/')
            || self
                .rel
                .strip_prefix("bin/")
                .is_some_and(|r| !r.contains('/'))
    }
}

/// An icon file found in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconFile {
    /// Its path relative to the version folder.
    pub rel: String,
    /// `svg` or `png`.
    pub ext: String,
    /// The raster's width and height from its header, if read.
    pub size: Option<(u32, u32)>,
    /// Its file size.
    pub bytes: u64,
}

impl IconFile {
    /// The file name without its extension.
    #[must_use]
    pub fn stem(&self) -> &str {
        let name = self.rel.rsplit('/').next().unwrap_or(&self.rel);
        name.rsplit_once('.').map_or(name, |(stem, _)| stem)
    }

    /// The hicolor size folder the icon goes to: `scalable` for SVG, the
    /// raster's own size (`512x512`) otherwise; `48x48` when the header
    /// could not be read.
    #[must_use]
    pub fn size_dir(&self) -> String {
        if self.ext == "svg" {
            return "scalable".to_owned();
        }
        match self.size {
            Some((w, h)) => format!("{w}x{h}"),
            None => "48x48".to_owned(),
        }
    }
}

/// A `.desktop` file shipped in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShippedDesktop {
    /// Its path relative to the version folder.
    pub rel: String,
    /// The parsed entry.
    pub entry: DesktopEntry,
}

/// What detection found, the defaults of the install form.
#[derive(Debug, Clone)]
pub struct Detected {
    /// The archive.
    pub archive: PathBuf,
    /// Its format.
    pub format: Format,
    /// Its index.
    pub index: Arc<Index>,
    /// The archive is a source tree: refused.
    pub source: bool,
    /// The executables, in archive order.
    pub candidates: Vec<Candidate>,
    /// The index into `candidates` detection prefers, `None` when several
    /// remain and the user picks.
    pub main: Option<usize>,
    /// GUI or CLI.
    pub kind: Kind,
    /// Why that kind, for the summary.
    pub kind_reason: &'static str,
    /// The vendor's desktop entry, if shipped.
    pub desktop: Option<ShippedDesktop>,
    /// The icon files, best first.
    pub icons: Vec<IconFile>,
    /// The display name.
    pub name: String,
    /// The version, possibly empty.
    pub version: String,
    /// The archive file name with the version masked.
    pub source_pattern: String,
    /// The commands under `bin/`: ELF programs and shebang scripts, by
    /// relative path.
    pub bins: Vec<String>,
    /// A top-level install script, which is never run.
    pub install_script: Option<String>,
    /// The main executable's machine when it is not this machine's.
    pub arch_mismatch: Option<u16>,
    /// Entries the archive-safety rules will leave out: hostile paths.
    pub hostile: Vec<String>,
}

impl Detected {
    /// The preferred main executable's relative path, if one is preferred.
    #[must_use]
    pub fn main_rel(&self) -> Option<&str> {
        self.main.map(|i| self.candidates[i].rel.as_str())
    }

    /// The archive's file name.
    #[must_use]
    pub fn archive_name(&self) -> String {
        self.archive
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The app id detection proposes.
    #[must_use]
    pub fn app_id(&self) -> String {
        slug(&self.name)
    }

    /// The `StartupWMClass` the vendor's entry carries.
    #[must_use]
    pub fn wm_class(&self) -> Option<String> {
        self.desktop
            .as_ref()
            .and_then(|d| d.entry.get("StartupWMClass"))
            .map(str::to_owned)
    }
}

/// What one pass over the archive read of its files.
#[derive(Default)]
struct Heads {
    elves: HashMap<String, Elf>,
    desktops: HashMap<String, String>,
    icons: HashMap<String, Option<(u32, u32)>>,
    shebangs: Vec<String>,
}

fn is_icon_name(name: &str) -> Option<&'static str> {
    let ext = Path::new(name).extension()?;
    if ext.eq_ignore_ascii_case("svg") {
        Some("svg")
    } else if ext.eq_ignore_ascii_case("png") {
        Some("png")
    } else {
        None
    }
}

fn is_library_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("so"))
        || name.contains(".so.")
}

/// The width and height of a PNG from its header.
#[must_use]
pub fn png_size(head: &[u8]) -> Option<(u32, u32)> {
    if head.len() < 24 || !head.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return None;
    }
    if &head[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
    let h = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
    Some((w, h))
}

/// Reads the heads of the files that matter in one pass.
fn read_heads(archive: &Path, format: Format, index: &Index) -> Result<Heads, Error> {
    let mut heads = Heads::default();
    archive::visit(archive, format, &mut |item, reader| {
        if item.kind != ItemKind::File || item.hostile {
            return Ok(Visit::Next);
        }
        let Some(rel) = index.relative(&item.path) else {
            return Ok(Visit::Next);
        };
        let name = item.name();
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".desktop") {
            let mut text = Vec::new();
            reader
                .take(TEXT_MAX)
                .read_to_end(&mut text)
                .map_err(crate::error::io_read)?;
            heads
                .desktops
                .insert(rel.to_owned(), String::from_utf8_lossy(&text).into_owned());
            return Ok(Visit::Next);
        }
        if is_icon_name(name).is_some() {
            let mut head = [0u8; 24];
            let got = read_up_to(reader, &mut head)?;
            heads.icons.insert(rel.to_owned(), png_size(&head[..got]));
            return Ok(Visit::Next);
        }
        if is_library_name(name) || item.size < 2 {
            return Ok(Visit::Next);
        }
        let mut head = [0u8; 4];
        let got = read_up_to(reader, &mut head)?;
        if got == 4 && elf::is_elf(&head) && item.size >= 52 {
            let mut bytes = head.to_vec();
            reader
                .take(ELF_HEAD - 4)
                .read_to_end(&mut bytes)
                .map_err(crate::error::io_read)?;
            if let Some(elf) = elf::read_bytes(&bytes)
                && elf.is_executable()
            {
                heads.elves.insert(rel.to_owned(), elf);
            }
        } else if got >= 2 && &head[..2] == b"#!" && rel.starts_with("bin/") {
            heads.shebangs.push(rel.to_owned());
        }
        Ok(Visit::Next)
    })?;
    Ok(heads)
}

fn read_up_to(reader: &mut dyn Read, buf: &mut [u8]) -> Result<usize, Error> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(crate::error::io_read(e)),
        }
    }
    Ok(filled)
}

/// Detects what the archive at `path` is and proposes the form's defaults.
///
/// # Errors
///
/// The errors of reading the archive.
#[allow(clippy::too_many_lines)]
pub fn detect(path: &Path) -> Result<Detected, Error> {
    let format = Format::detect(path)?;
    let index = Arc::new(archive::index(path, format)?);
    let heads = read_heads(path, format, &index)?;

    let hostile: Vec<String> = index
        .items
        .iter()
        .filter(|i| i.hostile)
        .map(|i| i.path.clone())
        .collect();

    let mut candidates: Vec<Candidate> = index
        .items
        .iter()
        .filter_map(|i| {
            let rel = index.relative(&i.path)?;
            heads.elves.get(rel).map(|elf| Candidate {
                rel: rel.to_owned(),
                elf: elf.clone(),
            })
        })
        .collect();
    candidates.sort_by(|a, b| a.rel.cmp(&b.rel));

    let top_files: Vec<&str> = index
        .items
        .iter()
        .filter_map(|i| index.relative(&i.path))
        .filter(|rel| !rel.contains('/'))
        .collect();
    let source = candidates.is_empty()
        && top_files
            .iter()
            .any(|f| SOURCE_MARKERS.contains(f) || *f == "src");

    let desktop = {
        let mut paths: Vec<&String> = heads.desktops.keys().collect();
        paths.sort_by_key(|p| (p.matches('/').count(), (*p).clone()));
        paths.first().map(|rel| ShippedDesktop {
            rel: (*rel).clone(),
            entry: DesktopEntry::parse(&heads.desktops[*rel]),
        })
    };

    let archive_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = archive_stem(&archive_name);
    let (file_version, pattern) = split_version(&stem).map_or_else(
        || (None, archive_name.clone()),
        |(version, range)| {
            let mut pattern = stem.clone();
            pattern.replace_range(range, "*");
            (Some(version), pattern + &archive_name[stem.len()..])
        },
    );
    let folder_version = index
        .top_level
        .as_deref()
        .and_then(|top| split_version(top).map(|(v, _)| v));
    let version = file_version.or(folder_version).unwrap_or_default();

    let base_name = index
        .top_level
        .as_deref()
        .map(strip_version)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| strip_version(&stem));
    let name = desktop
        .as_ref()
        .and_then(|d| d.entry.get("Name"))
        .map(str::to_owned)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| base_name.clone());
    let hint = slug(&base_name);

    // The main executable: the desktop file's Exec, then a prominent
    // binary named like the archive, then the only prominent or only one.
    let exec_program = desktop
        .as_ref()
        .and_then(|d| d.entry.get("Exec"))
        .and_then(desktop::exec_program);
    let main = exec_program
        .as_deref()
        .and_then(|program| {
            candidates
                .iter()
                .position(|c| c.name() == program || c.rel == program)
        })
        .or_else(|| {
            candidates
                .iter()
                .position(|c| c.is_prominent() && slug(c.name()) == hint)
        })
        .or_else(|| {
            let prominent: Vec<usize> = candidates
                .iter()
                .enumerate()
                .filter(|(_, c)| c.is_prominent())
                .map(|(i, _)| i)
                .collect();
            match prominent.as_slice() {
                [one] => Some(*one),
                _ => (candidates.len() == 1).then_some(0),
            }
        });

    let mut icons: Vec<IconFile> = index
        .items
        .iter()
        .filter_map(|i| {
            let rel = index.relative(&i.path)?;
            let ext = is_icon_name(i.name())?;
            Some(IconFile {
                rel: rel.to_owned(),
                ext: ext.to_owned(),
                size: heads.icons.get(rel).copied().flatten(),
                bytes: i.size,
            })
        })
        .collect();
    let icon_hint = desktop
        .as_ref()
        .and_then(|d| d.entry.get("Icon"))
        .map(|i| i.rsplit('/').next().unwrap_or(i).to_owned())
        .map(|i| i.rsplit_once('.').map_or(i.clone(), |(s, _)| s.to_owned()));
    icons.sort_by_key(|icon| {
        let named_in_desktop = icon_hint
            .as_deref()
            .is_some_and(|h| icon.stem().eq_ignore_ascii_case(h));
        let named_like_app = slug(icon.stem()) == hint || slug(icon.stem()) == slug(&name);
        let under_icon_dir = icon.rel.starts_with("icons/")
            || icon.rel.starts_with("resources/")
            || icon.rel.starts_with("share/")
            || icon.rel.contains("/icons/");
        let area = icon.size.map_or(0, |(w, h)| u64::from(w) * u64::from(h));
        (
            std::cmp::Reverse(u8::from(named_in_desktop)),
            std::cmp::Reverse(u8::from(named_like_app)),
            std::cmp::Reverse(u8::from(under_icon_dir)),
            std::cmp::Reverse(u8::from(icon.ext == "svg")),
            std::cmp::Reverse(area),
            std::cmp::Reverse(icon.bytes),
        )
    });
    if desktop.is_none() && icon_hint.is_none() {
        // Without a desktop file, only an icon named like the app or kept
        // in an icon folder counts; a stray PNG is a screenshot.
        icons.retain(|icon| {
            slug(icon.stem()) == hint
                || slug(icon.stem()) == slug(&name)
                || icon.rel.starts_with("icons/")
                || icon.rel.starts_with("resources/")
                || icon.rel.starts_with("share/")
                || icon.rel.contains("/icons/")
        });
    }

    let mut bins: Vec<String> = candidates
        .iter()
        .filter(|c| c.rel.strip_prefix("bin/").is_some_and(|r| !r.contains('/')))
        .map(|c| c.rel.clone())
        .chain(heads.shebangs.iter().cloned())
        .collect();
    bins.sort();
    bins.dedup();
    // A toolchain's commands are all equal: its first one stands in as the
    // main executable rather than making the user pick.
    let main = main.or_else(|| {
        (bins.len() >= 2)
            .then(|| {
                bins.iter()
                    .find_map(|b| candidates.iter().position(|c| c.rel == *b))
            })
            .flatten()
    });

    let (kind, kind_reason) = if desktop.is_some() {
        (Kind::Gui, "the archive ships a desktop entry")
    } else if !icons.is_empty() {
        (Kind::Gui, "the archive ships an icon")
    } else if main.is_some_and(|i| {
        candidates[i]
            .elf
            .needed
            .iter()
            .any(|lib| GUI_LIBRARIES.iter().any(|g| lib.starts_with(g)))
    }) {
        (Kind::Gui, "the main executable links against a GUI toolkit")
    } else if bins.len() >= 2 {
        (Kind::Cli, "a bin folder with several commands and no icons")
    } else {
        (Kind::Cli, "no desktop entry, icon or GUI toolkit")
    };

    let install_script = top_files
        .iter()
        .find(|f| INSTALL_SCRIPTS.contains(f))
        .map(|f| (*f).to_owned());

    let arch_mismatch = main.and_then(|i| {
        let found = candidates[i].elf.machine;
        elf::current_machine()
            .filter(|running| *running != found)
            .map(|_| found)
    });

    Ok(Detected {
        archive: path.to_path_buf(),
        format,
        index,
        source,
        candidates,
        main,
        kind,
        kind_reason,
        desktop,
        icons,
        name,
        version,
        source_pattern: pattern,
        bins,
        install_script,
        arch_mismatch,
        hostile,
    })
}

/// The archive's file name without its archive extension.
#[must_use]
pub fn archive_stem(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for ext in [
        ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tgz", ".txz", ".tbz2", ".tbz", ".tzst",
        ".tar", ".zip",
    ] {
        if lower.ends_with(ext) {
            return name[..name.len() - ext.len()].to_owned();
        }
    }
    name.to_owned()
}

/// The version digits in a name, conservatively: the last run of
/// `digits(.digits)+`, optionally led by `v`, bounded by separators or the
/// name's ends; the range covers the `v` too, so a pattern masks it.
#[must_use]
pub fn split_version(name: &str) -> Option<(String, std::ops::Range<usize>)> {
    let bytes = name.as_bytes();
    let mut best: Option<(String, std::ops::Range<usize>)> = None;
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        let mut dots = 0;
        while end < bytes.len() {
            if bytes[end].is_ascii_digit() {
                end += 1;
            } else if bytes[end] == b'.' && end + 1 < bytes.len() && bytes[end + 1].is_ascii_digit()
            {
                dots += 1;
                end += 1;
            } else {
                break;
            }
        }
        let bounded_after = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        let mut range_start = start;
        let led_by_v = start > 0
            && matches!(bytes[start - 1], b'v' | b'V')
            && (start == 1 || !bytes[start - 2].is_ascii_alphanumeric());
        if led_by_v {
            range_start -= 1;
        }
        let bounded_before = range_start == 0 || !bytes[range_start - 1].is_ascii_alphanumeric();
        if dots >= 1 && bounded_after && bounded_before {
            best = Some((name[start..end].to_owned(), range_start..end));
        }
        i = end.max(start + 1);
    }
    best
}

/// A name without its version digits and the separators around them:
/// `Postman-linux-x64-11.3.0` → `Postman-linux-x64`.
#[must_use]
pub fn strip_version(name: &str) -> String {
    let mut out = match split_version(name) {
        Some((_, range)) => {
            let mut s = name.to_owned();
            s.replace_range(range, "-");
            s
        }
        None => name.to_owned(),
    };
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches(['-', '_', '.', ' ']).to_owned()
}

/// The app id of a display name: lowercase, hyphenated, `[a-z0-9-]`.
#[must_use]
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// Whether `id` is a valid app id: `[a-z0-9][a-z0-9-]*`.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Whether `version` can name a folder: not empty, no separators, not
/// `current` or a staging name.
#[must_use]
pub fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version != "current"
        && !version.starts_with('.')
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+' | '~'))
}

/// Compares two versions component by component: numeric components by
/// value, others as text, a shorter version lower than its extension.
#[must_use]
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let parts = |v: &str| -> Vec<String> {
        v.split(['.', '-', '_', '+', '~'])
            .filter(|p| !p.is_empty())
            .map(|p| p.trim_start_matches(['v', 'V']).to_owned())
            .collect()
    };
    let (pa, pb) = (parts(a), parts(b));
    for (x, y) in pa.iter().zip(pb.iter()) {
        let order = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            _ => x.cmp(y),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    pa.len().cmp(&pb.len())
}
