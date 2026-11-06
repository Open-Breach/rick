//! Integration tests and the generated fixture tree for Rick.
//!
//! The crate exposes [`fixture`], which generates a deterministic directory
//! tree of N entries for tests and benchmarks, and [`media`], which writes a
//! photo with EXIF and a PDF of any page size. The integration tests
//! themselves live under `tests/`.

#![forbid(unsafe_code)]

pub mod fixture;
pub mod media;

/// A local location for an OS path.
#[must_use]
pub fn local(path: &std::path::Path) -> types::Location {
    types::Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path))
}
