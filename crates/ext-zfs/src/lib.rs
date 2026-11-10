//! The ZFS extension: the snapshots of OpenZFS datasets as a read-only
//! location, file versions, restore, snapshot management and the dataset's
//! properties in the info panel, on Linux.
//!
//! Everything goes through the `zfs` command line behind the [`Zfs`]
//! runner trait, which tests replace with a scripted fake. Without `zfs`,
//! or for a location outside every mounted dataset, the extension
//! contributes nothing visible.
//!
//! The crate builds as a `cdylib` that the plugin host loads from
//! `plugins/zfs/` through the entry point [`ext::plugin!`] declares, and as
//! an `rlib` for the binary's opt-in `zfs` feature and the tests. Its
//! manifest names Linux as its only platform.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

use std::sync::Arc;

use ext::{
    API_VERSION, Capabilities, Extension, Manifest, Platforms, Points, Registry, Version,
};
use types::{Error, ExtId};

/// The extension id.
pub const ID: ExtId = ExtId("zfs");

/// The extension: the `zfs` provider, the decorator and the "ZFS" group.
#[derive(Debug)]
pub struct ZfsExtension {
    manifest: Manifest,
}

impl Default for ZfsExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl ZfsExtension {
    /// The extension over the system's `zfs` command line.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "ZFS",
                description: "Shows the snapshots of OpenZFS datasets as read-only folders, lists the versions of a file, restores from a snapshot and manages snapshots.",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PROVIDER | Points::DECORATOR | Points::ACTION,
                platforms: Platforms::LINUX,
                previews: &[],
                commands: &[],
            },
        }
    }
}

impl Extension for ZfsExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[
            ("en", include_str!("../locales/en.ftl")),
            ("de", include_str!("../locales/de.ftl")),
        ]);
        Ok(())
    }
}

ext::plugin!(|| Arc::new(ZfsExtension::new()) as Arc<dyn Extension>);
