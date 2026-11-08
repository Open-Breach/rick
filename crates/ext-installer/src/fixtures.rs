//! The test fixtures of specs/ext-installer.md, generated on demand so no
//! binary is committed: ten small archives, each exercising one detection
//! or safety rule, with tiny ELF programs that exit 0 when run on this
//! machine. Built with the `fixtures` feature and in the crate's tests.

// Fixture bytes are small by construction: the casts between the ELF
// header's widths and the buffers' lengths cannot truncate.
#![allow(clippy::cast_possible_truncation)]

use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};

use crate::elf::{EM_AARCH64, EM_X86_64};

/// The virtual address the tiny programs load at.
const BASE: u64 = 0x40_0000;

/// A tiny static ELF64 program for `machine` that exits with status 0, with
/// a dynamic section naming `needed` libraries so a reader finds them (the
/// kernel ignores the section without a loader, so the program still runs).
/// The x86-64 and `AArch64` code is real; any other machine gets the x86-64
/// bytes, which only a header check reads.
#[must_use]
pub fn tiny_elf(machine: u16, needed: &[&str]) -> Vec<u8> {
    let code: &[u8] = if machine == EM_AARCH64 {
        // mov x0, #0; mov x8, #93 (exit); svc #0
        &[
            0x00, 0x00, 0x80, 0xd2, 0xa8, 0x0b, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4,
        ]
    } else {
        // xor edi, edi; mov eax, 60 (exit); syscall
        &[0x31, 0xff, 0xb8, 0x3c, 0x00, 0x00, 0x00, 0x0f, 0x05]
    };
    let phnum: u64 = if needed.is_empty() { 1 } else { 2 };
    let code_off: u64 = 0x100;
    let mut strtab = vec![0u8];
    let mut name_offsets = Vec::new();
    for name in needed {
        name_offsets.push(strtab.len() as u64);
        strtab.extend_from_slice(name.as_bytes());
        strtab.push(0);
    }
    let str_off = align(code_off + code.len() as u64, 8);
    let dyn_off = align(str_off + strtab.len() as u64, 8);
    let mut dynamic: Vec<u8> = Vec::new();
    if !needed.is_empty() {
        let mut push = |tag: u64, value: u64| {
            dynamic.extend_from_slice(&tag.to_le_bytes());
            dynamic.extend_from_slice(&value.to_le_bytes());
        };
        push(5, BASE + str_off);
        push(10, strtab.len() as u64);
        for offset in &name_offsets {
            push(1, *offset);
        }
        push(0, 0);
    }
    let total = dyn_off + dynamic.len() as u64;

    let mut out = Vec::with_capacity(total as usize);
    // The ELF header.
    out.extend_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0]);
    out.extend_from_slice(&[0u8; 8]);
    out.extend_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    out.extend_from_slice(&machine.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(BASE + code_off).to_le_bytes());
    out.extend_from_slice(&64u64.to_le_bytes()); // e_phoff
    out.extend_from_slice(&0u64.to_le_bytes()); // e_shoff
    out.extend_from_slice(&0u32.to_le_bytes()); // e_flags
    out.extend_from_slice(&64u16.to_le_bytes());
    out.extend_from_slice(&56u16.to_le_bytes());
    out.extend_from_slice(&(phnum as u16).to_le_bytes());
    out.extend_from_slice(&[0u8; 6]); // e_shentsize, e_shnum, e_shstrndx
    // PT_LOAD over the whole file, readable, writable and executable.
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&7u32.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&BASE.to_le_bytes());
    out.extend_from_slice(&BASE.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&0x1000u64.to_le_bytes());
    if !needed.is_empty() {
        out.extend_from_slice(&2u32.to_le_bytes()); // PT_DYNAMIC
        out.extend_from_slice(&6u32.to_le_bytes());
        out.extend_from_slice(&dyn_off.to_le_bytes());
        out.extend_from_slice(&(BASE + dyn_off).to_le_bytes());
        out.extend_from_slice(&(BASE + dyn_off).to_le_bytes());
        out.extend_from_slice(&(dynamic.len() as u64).to_le_bytes());
        out.extend_from_slice(&(dynamic.len() as u64).to_le_bytes());
        out.extend_from_slice(&8u64.to_le_bytes());
    }
    out.resize(code_off as usize, 0);
    out.extend_from_slice(code);
    out.resize(str_off as usize, 0);
    out.extend_from_slice(&strtab);
    out.resize(dyn_off as usize, 0);
    out.extend_from_slice(&dynamic);
    out
}

fn align(value: u64, to: u64) -> u64 {
    value.div_ceil(to) * to
}

/// The machine of this process, for a fixture that must run here; x86-64
/// where the reader does not know the machine.
#[must_use]
pub fn this_machine() -> u16 {
    crate::elf::current_machine().unwrap_or(EM_X86_64)
}

/// A machine this is not, for the wrong-architecture fixture.
#[must_use]
pub fn other_machine() -> u16 {
    if this_machine() == EM_X86_64 {
        EM_AARCH64
    } else {
        EM_X86_64
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// A valid PNG of `size` by `size` pixels, one flat colour, stored without
/// compression.
#[must_use]
pub fn tiny_png(size: u32) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let chunk = |out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    let mut raw = Vec::new();
    for _ in 0..size {
        raw.push(0u8);
        raw.extend(std::iter::repeat_n([0x33, 0x99, 0xcc], size as usize).flatten());
    }
    let mut zlib = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        zlib.push(u8::from(last));
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A small SVG icon.
pub const ICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="7" fill="#39c"/></svg>
"##;

/// The ten fixture archives of the spec's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fixture {
    /// Vendor `.desktop`, PNG icon, single top-level folder: `fixgui-1.5.0`.
    GuiWithDesktop,
    /// One ELF linking GTK and `resources/icon.svg`: `fixnodesk-0.3.0`.
    GuiNoDesktop,
    /// `bin/` with three ELF files, no icons: `fixtool-1.2.0`.
    Toolchain,
    /// No top-level folder: `fixbomb` at the root of a zip.
    Tarbomb,
    /// `Cargo.toml` and `src/`, no ELF: `fixsrc-0.1.0`.
    Source,
    /// An ELF for the other machine: `fixarch-1.0.0`.
    WrongArch,
    /// Two top-level ELF candidates: `fixamb-2.0.0`.
    Ambiguous,
    /// `..`, an absolute path, a link to `/etc`, a set-uid bit, a FIFO and
    /// `install.sh`: `fixhostile-1.0.0`.
    Hostile,
    /// The GUI app again as `fixgui-2.0.0`.
    UpdateV2,
    /// The GUI app again as `fixgui-1.0.0`.
    UpdateV1,
}

impl Fixture {
    /// Every fixture, in the spec's order.
    pub const ALL: [Self; 10] = [
        Self::GuiWithDesktop,
        Self::GuiNoDesktop,
        Self::Toolchain,
        Self::Tarbomb,
        Self::Source,
        Self::WrongArch,
        Self::Ambiguous,
        Self::Hostile,
        Self::UpdateV2,
        Self::UpdateV1,
    ];

    /// The archive's file name, as the spec's table names it.
    #[must_use]
    pub fn file_name(self) -> &'static str {
        match self {
            Self::GuiWithDesktop => "gui-with-desktop.tar.gz",
            Self::GuiNoDesktop => "gui-no-desktop.tar.xz",
            Self::Toolchain => "toolchain.tar.gz",
            Self::Tarbomb => "tarbomb.zip",
            Self::Source => "source.tar.gz",
            Self::WrongArch => "wrong-arch.tar.gz",
            Self::Ambiguous => "ambiguous.tar.gz",
            Self::Hostile => "hostile.tar",
            Self::UpdateV2 => "update-v2.tar.gz",
            Self::UpdateV1 => "update-v1.tar.gz",
        }
    }

    /// The archive's bytes.
    ///
    /// # Errors
    ///
    /// The I/O error from an encoder, which an in-memory build never
    /// raises in practice.
    #[allow(clippy::too_many_lines)]
    pub fn bytes(self) -> io::Result<Vec<u8>> {
        match self {
            Self::GuiWithDesktop => gui_tar("1.5.0"),
            Self::UpdateV2 => gui_tar("2.0.0"),
            Self::UpdateV1 => gui_tar("1.0.0"),
            Self::GuiNoDesktop => {
                let mut tar = TarBuild::new();
                tar.dir("fixnodesk-0.3.0");
                tar.file(
                    "fixnodesk-0.3.0/fixnodesk",
                    0o755,
                    &tiny_elf(this_machine(), &["libgtk-3.so.0", "libc.so.6"]),
                );
                tar.dir("fixnodesk-0.3.0/resources");
                tar.file(
                    "fixnodesk-0.3.0/resources/icon.svg",
                    0o644,
                    ICON_SVG.as_bytes(),
                );
                let plain = tar.finish()?;
                let mut encoder = liblzma::write::XzEncoder::new(Vec::new(), 6);
                encoder.write_all(&plain)?;
                encoder.finish()
            }
            Self::Toolchain => {
                let mut tar = TarBuild::new();
                tar.dir("fixtool-1.2.0");
                tar.dir("fixtool-1.2.0/bin");
                for name in ["fixa", "fixb", "fixc"] {
                    tar.file(
                        &format!("fixtool-1.2.0/bin/{name}"),
                        0o755,
                        &tiny_elf(this_machine(), &[]),
                    );
                }
                tar.file("fixtool-1.2.0/bin/fixscript", 0o644, b"#!/bin/sh\nexit 0\n");
                tar.dir("fixtool-1.2.0/lib");
                tar.file("fixtool-1.2.0/lib/libfix.so.1", 0o644, b"not a library");
                tar.file("fixtool-1.2.0/LICENSE", 0o644, b"Apache-2.0\n");
                gzip(&tar.finish()?)
            }
            Self::Tarbomb => {
                let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
                let exe = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .unix_permissions(0o755);
                let plain = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .unix_permissions(0o644);
                zip.start_file("fixbomb", exe).map_err(zip_io)?;
                zip.write_all(&tiny_elf(this_machine(), &[]))?;
                zip.start_file("README.md", plain).map_err(zip_io)?;
                zip.write_all(b"# fixbomb\n")?;
                zip.add_directory("data/", plain).map_err(zip_io)?;
                zip.start_file("data/notes.txt", plain).map_err(zip_io)?;
                zip.write_all(b"notes\n")?;
                zip.add_symlink("fixbomb-link", "fixbomb", plain)
                    .map_err(zip_io)?;
                Ok(zip.finish().map_err(zip_io)?.into_inner())
            }
            Self::Source => {
                let mut tar = TarBuild::new();
                tar.dir("fixsrc-0.1.0");
                tar.file(
                    "fixsrc-0.1.0/Cargo.toml",
                    0o644,
                    b"[package]\nname = \"fixsrc\"\nversion = \"0.1.0\"\n",
                );
                tar.dir("fixsrc-0.1.0/src");
                tar.file("fixsrc-0.1.0/src/main.rs", 0o644, b"fn main() {}\n");
                gzip(&tar.finish()?)
            }
            Self::WrongArch => {
                let mut tar = TarBuild::new();
                tar.dir("fixarch-1.0.0");
                tar.file(
                    "fixarch-1.0.0/fixarch",
                    0o755,
                    &tiny_elf(other_machine(), &[]),
                );
                gzip(&tar.finish()?)
            }
            Self::Ambiguous => {
                let mut tar = TarBuild::new();
                tar.dir("fixamb-2.0.0");
                tar.file("fixamb-2.0.0/alpha", 0o755, &tiny_elf(this_machine(), &[]));
                tar.file("fixamb-2.0.0/beta", 0o755, &tiny_elf(this_machine(), &[]));
                gzip(&tar.finish()?)
            }
            Self::Hostile => {
                let mut tar = TarBuild::new();
                tar.dir("fixhostile-1.0.0");
                tar.file(
                    "fixhostile-1.0.0/fixhostile",
                    0o755,
                    &tiny_elf(this_machine(), &[]),
                );
                tar.raw_file("../evil", 0o644, b"climbed out\n");
                tar.raw_file("/abs/evil", 0o644, b"absolute\n");
                tar.symlink("fixhostile-1.0.0/etc-link", "/etc/passwd");
                tar.symlink("fixhostile-1.0.0/self-link", "fixhostile");
                tar.file("fixhostile-1.0.0/suid", 0o4755, b"#!/bin/sh\nexit 0\n");
                tar.file(
                    "fixhostile-1.0.0/install.sh",
                    0o755,
                    b"#!/bin/sh\ntouch /tmp/rick-fixture-ran\n",
                );
                tar.special("fixhostile-1.0.0/pipe", tar::EntryType::Fifo);
                tar.finish()
            }
        }
    }

    /// Writes the archive into `dir` under its file name.
    ///
    /// # Errors
    ///
    /// The I/O error from writing.
    pub fn write(self, dir: &Path) -> io::Result<PathBuf> {
        let path = dir.join(self.file_name());
        std::fs::write(&path, self.bytes()?)?;
        Ok(path)
    }
}

/// Writes every fixture into `dir`.
///
/// # Errors
///
/// The I/O error from writing.
pub fn write_all(dir: &Path) -> io::Result<Vec<PathBuf>> {
    Fixture::ALL.iter().map(|f| f.write(dir)).collect()
}

fn zip_io(error: zip::result::ZipError) -> io::Error {
    io::Error::other(error)
}

fn gzip(plain: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(plain)?;
    encoder.finish()
}

/// The GUI app's tarball at `version`: `fixgui-<version>/` with the
/// program, the vendor's desktop entry, a 64×64 PNG and a README.
fn gui_tar(version: &str) -> io::Result<Vec<u8>> {
    let top = format!("fixgui-{version}");
    let mut tar = TarBuild::new();
    tar.dir(&top);
    tar.file(
        &format!("{top}/fixgui"),
        0o755,
        &tiny_elf(this_machine(), &[]),
    );
    tar.file(
        &format!("{top}/fixgui.desktop"),
        0o644,
        b"[Desktop Entry]\nType=Application\nName=Fixture GUI\nComment=A fixture application\nExec=fixgui %U\nIcon=fixgui\nCategories=Utility;\nStartupWMClass=FixGui\nMimeType=text/plain;\n",
    );
    tar.file(&format!("{top}/fixgui.png"), 0o644, &tiny_png(64));
    tar.file(
        &format!("{top}/README"),
        0o644,
        format!("Fixture GUI {version}\n").as_bytes(),
    );
    gzip(&tar.finish()?)
}

/// A tar in memory, with the raw header access the hostile fixture needs.
struct TarBuild {
    builder: tar::Builder<Vec<u8>>,
}

impl TarBuild {
    fn new() -> Self {
        Self {
            builder: tar::Builder::new(Vec::new()),
        }
    }

    fn header(path: &str, mode: u32, size: u64, kind: tar::EntryType) -> tar::Header {
        let mut header = tar::Header::new_ustar();
        header.set_mode(mode);
        header.set_size(size);
        header.set_entry_type(kind);
        header.set_mtime(1_700_000_000);
        header.set_uid(1000);
        header.set_gid(1000);
        // The name field as bytes: `set_path` would refuse `..` and strip a
        // leading slash, which the hostile fixture needs to keep.
        if let Some(ustar) = header.as_ustar_mut() {
            let bytes = path.as_bytes();
            let n = bytes.len().min(ustar.name.len());
            ustar.name[..n].copy_from_slice(&bytes[..n]);
        }
        header
    }

    fn dir(&mut self, path: &str) {
        let mut header = Self::header(&format!("{path}/"), 0o755, 0, tar::EntryType::Directory);
        header.set_cksum();
        let _ = self.builder.append(&header, io::empty());
    }

    fn file(&mut self, path: &str, mode: u32, data: &[u8]) {
        self.raw_file(path, mode, data);
    }

    fn raw_file(&mut self, path: &str, mode: u32, data: &[u8]) {
        let mut header = Self::header(path, mode, data.len() as u64, tar::EntryType::Regular);
        header.set_cksum();
        let _ = self.builder.append(&header, data);
    }

    fn symlink(&mut self, path: &str, target: &str) {
        let mut header = Self::header(path, 0o777, 0, tar::EntryType::Symlink);
        let _ = header.set_link_name_literal(target);
        header.set_cksum();
        let _ = self.builder.append(&header, io::empty());
    }

    fn special(&mut self, path: &str, kind: tar::EntryType) {
        let mut header = Self::header(path, 0o644, 0, kind);
        header.set_cksum();
        let _ = self.builder.append(&header, io::empty());
    }

    fn finish(mut self) -> io::Result<Vec<u8>> {
        self.builder.finish()?;
        self.builder.into_inner()
    }
}
