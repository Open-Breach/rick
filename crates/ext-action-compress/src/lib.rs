//! The compress and extract actions, as a plugin.
//!
//! *Compress* (zip, next to the selection), *Compress to…* (the name, the
//! format and the folder in a form), *Extract here*, *Extract to folder*
//! (named after the archive) and *Extract to…* (a chosen folder). Every
//! run is one pausable job with bytes and items progress; an extraction
//! asks about an existing name through the job's conflict queue, the same
//! dialogue a copy uses, and a folder over a folder merges. The archives
//! are written by [`ext_vfs_archive::create`] and read by
//! [`ext_vfs_archive::extract`] (QUESTIONS.md 127), on the blocking pool.
//!
//! The crate builds as a `cdylib` the plugin host loads from
//! `plugins/action-compress/` and as an `rlib` for the binary's opt-in
//! `action-compress` feature and the tests.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

mod actions;

use std::sync::Arc;

use ext::{API_VERSION, Capabilities, Extension, Manifest, Platforms, Points, Registry, Version};
use types::{Error, ExtId};

pub use actions::{
    COMPRESS, COMPRESS_AS, EXTRACT_FOLDER, EXTRACT_HERE, EXTRACT_TO, archive_stem, is_archive,
};

/// The extension id.
pub const ID: ExtId = ExtId("action-compress");

/// The extension: five actions.
#[derive(Debug)]
pub struct CompressExtension {
    manifest: Manifest,
}

impl Default for CompressExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl CompressExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Compress and extract",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::WRITE,
                points: Points::ACTION,
                platforms: Platforms::ALL,
            },
        }
    }
}

impl Extension for CompressExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        for action in actions::all() {
            reg.action(action);
        }
        Ok(())
    }
}

ext::plugin!(|| Arc::new(CompressExtension::new()) as Arc<dyn Extension>);

#[cfg(test)]
mod tests;
