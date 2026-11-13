//! Integration tests and the generated fixture tree for Rick.
//!
//! The crate exposes [`fixture`], which generates a deterministic directory
//! tree of N entries for tests and benchmarks, [`media`], which writes a
//! photo with EXIF and a PDF of any page size, [`mem`], an in-memory
//! provider that stands in for a cloud provider, [`timeline`], a time
//! slider source over a folder's copies, [`mod@bench`], the targets of
//! the acceptance benchmarks per operating system, and [`readme`], the
//! parts of `README.md` that are regenerated with every release from
//! what the extensions register. The integration tests themselves live
//! under `tests/`.

#![forbid(unsafe_code)]

pub mod bench;
pub mod fixture;
pub mod media;
pub mod mem;
pub mod readme;
pub mod timeline;

/// A local location for an OS path.
#[must_use]
pub fn local(path: &std::path::Path) -> types::Location {
    types::Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path))
}

/// The text of the file at `path` once `done` holds for it, or as it is
/// after ten seconds. The settings file is written by a task of the
/// runtime after the command that changed it, which a test's `settle` does
/// not wait for, so a test that looks at what a command wrote reads the
/// file this way rather than once: the Linux arm64 runner read `rick.toml`
/// before an edit had reached it (full.yml run 42522726087).
///
/// # Panics
///
/// When the file cannot be read for a reason other than not existing yet.
#[must_use]
pub fn read_until(path: &std::path::Path, done: impl Fn(&str) -> bool) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => panic!("{}: {e}", path.display()),
        };
        if done(&text) || std::time::Instant::now() >= deadline {
            return text;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
