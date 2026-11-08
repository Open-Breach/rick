//! The installer's own errors, carried inside [`types::Error::Extension`].

use std::io;

use types::Error;

use crate::ID;

/// Why an install, update or detection stopped. Every message is what the
/// user reads, so each case has its own.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    /// The archive holds source code and no compiled executable.
    #[error("This archive contains source code and needs to be compiled; Rick can't install it.")]
    Source,
    /// The main executable is built for another machine.
    #[error("the main executable is built for {found}; this machine is {running}")]
    Architecture {
        /// The archive's machine.
        found: String,
        /// The running machine.
        running: String,
    },
    /// The archive's hash is not the expected one.
    #[error("the archive's SHA-256 is {actual}, not the expected {expected}")]
    Checksum {
        /// What the user typed.
        expected: String,
        /// What the file hashes to.
        actual: String,
    },
    /// The destination's volume has less room than the archive unpacks to.
    #[error("not enough free space at {at}: the archive unpacks to {needed}, {free} is free")]
    DiskFull {
        /// Bytes the archive unpacks to, formatted.
        needed: String,
        /// Bytes free, formatted.
        free: String,
        /// The destination.
        at: String,
    },
    /// The disk filled up while writing.
    #[error("the disk is full")]
    StorageFull,
    /// The archive cannot be read to the end.
    #[error("the archive is damaged or not an archive Rick reads: {0}")]
    Corrupt(String),
    /// A command link or desktop entry of another origin is in the way.
    #[error("{what} already exists and was not created by Rick; tick Overwrite to replace it")]
    Collision {
        /// The path in the way.
        what: String,
    },
    /// The app runs from the version about to be replaced.
    #[error("{name} is running; close it and run the update again (what was extracted is kept)")]
    Running {
        /// The app's name.
        name: String,
    },
    /// The chosen main executable is not in the archive or is not ELF.
    #[error("the main executable {0} is not in the archive as an ELF program")]
    NoMainExecutable(String),
    /// A field of the form does not fit.
    #[error("{0}")]
    Invalid(String),
    /// System-wide scope, before the helper exists.
    #[error(
        "system-wide installs need the privileged helper rick-install-helper, which is not part of this version"
    )]
    SystemUnavailable,
    /// The plugin runs on Linux only.
    #[error("unsupported on this platform")]
    Unsupported,
    /// No record for the app id.
    #[error("no install record for {0}")]
    NoRecord(String),
    /// Nothing to revert to.
    #[error("{0} has no previous version to revert to")]
    NoPrevious(String),
}

/// Wraps the installer's error in the kernel's.
#[must_use]
pub fn err(error: InstallError) -> Error {
    Error::Extension {
        ext: ID,
        source: Box::new(error),
    }
}

/// An error while reading the archive: the archive is the suspect.
// Taken by value so `map_err(io_read)` reads as the other mappers do.
#[allow(clippy::needless_pass_by_value)]
#[must_use]
pub fn io_read(error: io::Error) -> Error {
    match error.kind() {
        io::ErrorKind::NotFound => Error::NotFound,
        io::ErrorKind::PermissionDenied => Error::PermissionDenied,
        _ => err(InstallError::Corrupt(error.to_string())),
    }
}

/// An error while writing to the destination: the disk is the suspect.
#[must_use]
pub fn io_write(error: io::Error) -> Error {
    match error.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => err(InstallError::StorageFull),
        io::ErrorKind::PermissionDenied => Error::PermissionDenied,
        _ => Error::Io(error),
    }
}

/// `12.3 MB`, `640 kB`, `12 bytes`.
#[must_use]
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["kB", "MB", "GB", "TB", "PB"];
    if n < 1000 {
        return format!("{n} bytes");
    }
    // A size for a message: the precision loss is fine.
    #[allow(clippy::cast_precision_loss)]
    let mut value = n as f64;
    let mut unit = "";
    for name in UNITS {
        value /= 1000.0;
        unit = name;
        if value < 1000.0 {
            break;
        }
    }
    if value < 10.0 {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.0} {unit}")
    }
}
