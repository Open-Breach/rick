//! AppImages (specs/ext-installer.md "AppImages"): telling one from any
//! other file, and *Run*, which starts one as it is. An AppImage is an ELF
//! program, its runtime, with a file-system image behind it; only its
//! header is read here, and nothing is unpacked or mounted.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use ext::platform::{Platform, fs as pfs};
use types::Error;

use crate::elf;
use crate::error::{InstallError, err, io_write};

/// The extension an AppImage usually carries, compared in any case.
const EXTENSION: &str = ".appimage";

/// Where the AppImage magic sits: bytes 8 to 10 of the ELF identification,
/// which the ELF format leaves as padding.
const MAGIC_AT: usize = 8;

/// The owner's execute bit.
const OWNER_EXECUTE: u32 = 0o100;

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

/// *Run*: starts the AppImage at `path` as it is. The file's ELF machine
/// type is compared with the running machine first, and a mismatch stops
/// here; the execute bit is set for the owner when it is missing, nothing
/// else about the file changes; then the file is started detached from
/// Rick, with its folder as the working directory.
///
/// # Errors
///
/// [`InstallError::NotAProgram`] for a file that is not ELF,
/// [`InstallError::AppImageArchitecture`] for one built for another
/// machine, and the I/O error from reading the header, setting the bit
/// or starting the file.
pub fn run(platform: &dyn Platform, path: &Path) -> Result<(), Error> {
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
    let dir = path.parent().unwrap_or(path);
    platform.run_detached(path, &[], dir)
}
