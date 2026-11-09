//! A small ELF reader: enough of the header, the program headers and the
//! dynamic section to tell an executable from a library, read its machine
//! type and list the shared libraries it needs. Headers and text only; a
//! candidate is never run (specs/ext-installer.md "Archive safety").

use std::io::{self, Read, Seek, SeekFrom};

/// The four bytes every ELF file starts with.
pub const MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const DT_NULL: i64 = 0;
const DT_NEEDED: i64 = 1;
const DT_STRTAB: i64 = 5;
const DT_STRSZ: i64 = 10;
const DT_SONAME: i64 = 14;
const DT_RPATH: i64 = 15;
const DT_RUNPATH: i64 = 29;

/// `e_machine` values of the machines Rick runs on, plus the common
/// others, for the architecture check and its message.
pub const EM_386: u16 = 3;
/// ARM, 32-bit.
pub const EM_ARM: u16 = 40;
/// x86-64.
pub const EM_X86_64: u16 = 62;
/// `AArch64`.
pub const EM_AARCH64: u16 = 183;
/// RISC-V.
pub const EM_RISCV: u16 = 243;

/// What the reader found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Elf {
    /// 64-bit (`ELFCLASS64`) rather than 32-bit.
    pub class64: bool,
    /// The file's `e_type`: 2 is an executable, 3 a shared object, which
    /// position-independent executables are too.
    pub kind: u16,
    /// The `e_machine` value.
    pub machine: u16,
    /// Whether the file names a dynamic loader (`PT_INTERP`): true for every
    /// dynamically linked program, false for libraries and static programs.
    pub interp: bool,
    /// The shared libraries it needs (`DT_NEEDED`), in order.
    pub needed: Vec<String>,
    /// Its own name as a library (`DT_SONAME`).
    pub soname: Option<String>,
    /// `DT_RPATH` entries, `:`-separated as recorded.
    pub rpath: Vec<String>,
    /// `DT_RUNPATH` entries.
    pub runpath: Vec<String>,
}

impl Elf {
    /// Whether this is a program rather than a library: an `ET_EXEC`, or an
    /// `ET_DYN` with a loader (a position-independent executable) or without
    /// a soname.
    #[must_use]
    pub fn is_executable(&self) -> bool {
        match self.kind {
            ET_EXEC => true,
            ET_DYN => self.interp || self.soname.is_none(),
            _ => false,
        }
    }

    /// The search paths the file records, `DT_RUNPATH` first when both are
    /// present (the loader ignores `DT_RPATH` then), each already split on
    /// `:`.
    #[must_use]
    pub fn search_paths(&self) -> Vec<String> {
        let list = if self.runpath.is_empty() {
            &self.rpath
        } else {
            &self.runpath
        };
        list.iter()
            .flat_map(|p| p.split(':'))
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

/// Whether `head` starts like an ELF file.
#[must_use]
pub fn is_elf(head: &[u8]) -> bool {
    head.len() >= 4 && head[..4] == MAGIC
}

/// The machine's name for a message: `x86_64`, `aarch64`, …
#[must_use]
pub fn machine_name(machine: u16) -> String {
    match machine {
        EM_386 => "x86 (32-bit)".to_owned(),
        EM_ARM => "arm (32-bit)".to_owned(),
        EM_X86_64 => "x86_64".to_owned(),
        EM_AARCH64 => "aarch64".to_owned(),
        EM_RISCV => "riscv".to_owned(),
        other => format!("machine type {other}"),
    }
}

/// The `e_machine` of the machine this process runs on, when it is one the
/// reader knows.
#[must_use]
pub fn current_machine() -> Option<u16> {
    match std::env::consts::ARCH {
        "x86_64" => Some(EM_X86_64),
        "aarch64" => Some(EM_AARCH64),
        "x86" => Some(EM_386),
        "arm" => Some(EM_ARM),
        "riscv64" | "riscv32" => Some(EM_RISCV),
        _ => None,
    }
}

/// The dynamic-library file name for the running machine's multiarch
/// directory under `/usr/lib`, where Debian and Ubuntu keep the system
/// libraries.
#[must_use]
pub fn multiarch_dir(machine: u16) -> Option<&'static str> {
    match machine {
        EM_X86_64 => Some("x86_64-linux-gnu"),
        EM_AARCH64 => Some("aarch64-linux-gnu"),
        EM_386 => Some("i386-linux-gnu"),
        EM_ARM => Some("arm-linux-gnueabihf"),
        EM_RISCV => Some("riscv64-linux-gnu"),
        _ => None,
    }
}

struct Reader<R> {
    inner: R,
    little: bool,
    class64: bool,
}

impl<R: Read + Seek> Reader<R> {
    fn at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.inner.seek(SeekFrom::Start(offset))?;
        self.inner.read_exact(buf)
    }

    fn u16(&self, bytes: &[u8]) -> u16 {
        let pair = [bytes[0], bytes[1]];
        if self.little {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    }

    fn u32(&self, bytes: &[u8]) -> u32 {
        let quad = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if self.little {
            u32::from_le_bytes(quad)
        } else {
            u32::from_be_bytes(quad)
        }
    }

    fn u64(&self, bytes: &[u8]) -> u64 {
        let mut eight = [0u8; 8];
        eight.copy_from_slice(&bytes[..8]);
        if self.little {
            u64::from_le_bytes(eight)
        } else {
            u64::from_be_bytes(eight)
        }
    }
}

/// A loadable segment, for mapping virtual addresses to file offsets.
struct Load {
    vaddr: u64,
    offset: u64,
    filesz: u64,
}

/// Reads what the installer needs from an ELF file. Returns `None` for a
/// file that is not ELF; a file that is ELF but truncated or odd still
/// yields its header fields, with the dynamic information as far as it
/// could be read.
///
/// # Errors
///
/// The I/O error from reading the header.
pub fn read<R: Read + Seek>(mut source: R) -> io::Result<Option<Elf>> {
    let mut ident = [0u8; 20];
    source.seek(SeekFrom::Start(0))?;
    let got = read_up_to(&mut source, &mut ident)?;
    if got < 20 || !is_elf(&ident) {
        return Ok(None);
    }
    let class64 = match ident[4] {
        1 => false,
        2 => true,
        _ => return Ok(None),
    };
    let little = match ident[5] {
        1 => true,
        2 => false,
        _ => return Ok(None),
    };
    let mut reader = Reader {
        inner: source,
        little,
        class64,
    };
    let mut elf = Elf {
        class64,
        kind: reader.u16(&ident[16..18]),
        machine: reader.u16(&ident[18..20]),
        ..Elf::default()
    };
    // Best effort from here: the header fields above are the ones the
    // architecture check needs; the rest is for the library check.
    let _ = read_dynamic(&mut reader, &mut elf);
    Ok(Some(elf))
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

// A parser walks the header, the program headers and the dynamic
// entries in one place.
#[allow(clippy::too_many_lines)]
fn read_dynamic<R: Read + Seek>(reader: &mut Reader<R>, elf: &mut Elf) -> io::Result<()> {
    let header_len = if reader.class64 { 64 } else { 52 };
    let mut header = vec![0u8; header_len];
    reader.at(0, &mut header)?;
    let (phoff, phentsize, phnum) = if reader.class64 {
        (
            reader.u64(&header[32..40]),
            reader.u16(&header[54..56]),
            reader.u16(&header[56..58]),
        )
    } else {
        (
            u64::from(reader.u32(&header[28..32])),
            reader.u16(&header[42..44]),
            reader.u16(&header[44..46]),
        )
    };
    if phnum == 0 || phnum > 512 || usize::from(phentsize) < if reader.class64 { 56 } else { 32 } {
        return Ok(());
    }
    let mut loads = Vec::new();
    let mut dynamic: Option<(u64, u64)> = None;
    let mut ph = vec![0u8; usize::from(phentsize)];
    for i in 0..u64::from(phnum) {
        reader.at(phoff + i * u64::from(phentsize), &mut ph)?;
        let p_type = reader.u32(&ph[0..4]);
        let (offset, vaddr, filesz) = if reader.class64 {
            (
                reader.u64(&ph[8..16]),
                reader.u64(&ph[16..24]),
                reader.u64(&ph[32..40]),
            )
        } else {
            (
                u64::from(reader.u32(&ph[4..8])),
                u64::from(reader.u32(&ph[8..12])),
                u64::from(reader.u32(&ph[16..20])),
            )
        };
        match p_type {
            PT_INTERP => elf.interp = true,
            PT_LOAD => loads.push(Load {
                vaddr,
                offset,
                filesz,
            }),
            PT_DYNAMIC => dynamic = Some((offset, filesz)),
            _ => {}
        }
    }
    let Some((dyn_offset, dyn_size)) = dynamic else {
        return Ok(());
    };
    let entry_size = if reader.class64 { 16 } else { 8 };
    let count = usize::try_from(dyn_size / entry_size as u64)
        .unwrap_or(0)
        .min(4096);
    let mut entries = Vec::with_capacity(count);
    let mut buf = vec![0u8; entry_size];
    for i in 0..count {
        reader.at(dyn_offset + (i * entry_size) as u64, &mut buf)?;
        let (tag, value) = if reader.class64 {
            (
                reader.u64(&buf[0..8]).cast_signed(),
                reader.u64(&buf[8..16]),
            )
        } else {
            (
                i64::from(reader.u32(&buf[0..4]).cast_signed()),
                u64::from(reader.u32(&buf[4..8])),
            )
        };
        if tag == DT_NULL {
            break;
        }
        entries.push((tag, value));
    }
    let strtab_vaddr = entries
        .iter()
        .find(|(t, _)| *t == DT_STRTAB)
        .map(|(_, v)| *v);
    let strsz = entries
        .iter()
        .find(|(t, _)| *t == DT_STRSZ)
        .map_or(0, |(_, v)| *v)
        .min(1 << 20);
    let Some(strtab_vaddr) = strtab_vaddr else {
        return Ok(());
    };
    let Some(load) = loads
        .iter()
        .find(|l| strtab_vaddr >= l.vaddr && strtab_vaddr < l.vaddr + l.filesz)
    else {
        return Ok(());
    };
    let strtab_offset = strtab_vaddr - load.vaddr + load.offset;
    let available = (load.filesz - (strtab_vaddr - load.vaddr)).min(strsz);
    let mut strtab = vec![0u8; usize::try_from(available).unwrap_or(0)];
    reader.at(strtab_offset, &mut strtab)?;
    let string_at = |offset: u64| -> Option<String> {
        let start = usize::try_from(offset).ok()?;
        let rest = strtab.get(start..)?;
        let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
        Some(String::from_utf8_lossy(&rest[..end]).into_owned())
    };
    for (tag, value) in entries {
        match tag {
            DT_NEEDED => elf.needed.extend(string_at(value)),
            DT_SONAME => elf.soname = string_at(value),
            DT_RPATH => elf.rpath.extend(string_at(value)),
            DT_RUNPATH => elf.runpath.extend(string_at(value)),
            _ => {}
        }
    }
    Ok(())
}

/// Reads an ELF file on disk; `None` when the file is not ELF.
///
/// # Errors
///
/// The I/O error from opening or reading it.
pub fn read_file(path: &std::path::Path) -> io::Result<Option<Elf>> {
    read(std::fs::File::open(path)?)
}

/// Reads an ELF from bytes already in memory (the head of an archive
/// entry); `None` when they are not ELF.
#[must_use]
pub fn read_bytes(bytes: &[u8]) -> Option<Elf> {
    read(io::Cursor::new(bytes)).ok().flatten()
}
