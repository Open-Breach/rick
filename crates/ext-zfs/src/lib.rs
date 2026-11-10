//! The ZFS extension: the snapshots of `OpenZFS` datasets as a read-only
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

pub mod actions;
pub mod datasets;
pub mod decorator;
pub mod edit;
pub mod helper;
pub mod manage;
pub mod parse;
pub mod props;
pub mod provider;
pub mod restore;
mod versions;
pub mod zfs;

use std::sync::Arc;

use ext::{API_VERSION, Capabilities, Extension, Manifest, Platforms, Points, Registry, Version};
use types::{Error, ExtId};

pub use datasets::Datasets;
pub use provider::{
    PROVIDER_ID, Target, ZfsProvider, dataset_location, snapshot_location, target,
    versions_location,
};
pub use zfs::{Output, ScriptedZfs, SystemZfs, Zfs, ZfsError};

/// The extension id.
pub const ID: ExtId = ExtId("zfs");

/// The extension: the `zfs` provider, the decorator and the "ZFS" group.
#[derive(Debug)]
pub struct ZfsExtension {
    manifest: Manifest,
    datasets: Arc<Datasets>,
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
        Self::with_zfs(Arc::new(SystemZfs::new()))
    }

    /// The extension over another `zfs`: a [`ScriptedZfs`] in tests.
    #[must_use]
    pub fn with_zfs(zfs: Arc<dyn Zfs>) -> Self {
        Self::with_datasets(Arc::new(Datasets::new(zfs)))
    }

    /// The extension over a dataset cache of the caller's, which tests
    /// build with a short TTL.
    #[must_use]
    pub fn with_datasets(datasets: Arc<Datasets>) -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "ZFS",
                description: "Shows the snapshots of OpenZFS datasets as read-only folders, lists the versions of a file, restores from a snapshot, manages snapshots and edits a dataset's safe properties.",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PROVIDER | Points::DECORATOR | Points::ACTION,
                platforms: Platforms::LINUX,
                previews: &[],
                commands: &[],
            },
            datasets,
        }
    }

    /// The dataset cache every contribution shares.
    #[must_use]
    pub fn datasets(&self) -> &Arc<Datasets> {
        &self.datasets
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
        reg.provider(
            Arc::new(ZfsProvider::new(Arc::clone(&self.datasets))) as Arc<dyn ext::Provider>
        );
        reg.decorator(
            Arc::new(decorator::ZfsDecorator::new(Arc::clone(&self.datasets)))
                as Arc<dyn ext::Decorator>,
        );
        let shared = Arc::new(actions::Shared {
            datasets: Arc::clone(&self.datasets),
            platform: reg.platform().cloned(),
            elevate: reg
                .platform()
                .filter(|_| zfs::find_program("pkexec").is_some())
                .map(|p| Arc::clone(p) as Arc<dyn ext::platform::Elevate>),
            helper: helper::path(),
        });
        for action in actions::all(&shared) {
            reg.action(action);
        }
        Ok(())
    }
}

ext::plugin!(|| Arc::new(ZfsExtension::new()) as Arc<dyn Extension>);

#[cfg(test)]
mod tests;
