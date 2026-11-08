//! The tarball installer: *Install as application…* on a tar or zip
//! archive, a versioned layout under `~/.local/opt/<app-id>/` with a
//! `current` symlink, desktop entry, icon and commands on the PATH, update
//! with one previous version kept, revert, uninstall, repair, and the
//! Installed Apps view `rick://apps` (specs/ext-installer.md).
//!
//! Linux only in this version: the plugin loads everywhere, its actions
//! are hidden and its view answers "unsupported on this platform" on macOS
//! and Windows. System-wide installs through the privileged helper are the
//! spec's own later milestone; the form shows the scope as unavailable.
//!
//! The crate builds as a `cdylib` the plugin host loads from
//! `plugins/installer/` and as an `rlib` for the binary's opt-in
//! `installer` feature and the tests.

// `deny`, not `forbid`: the entry point's `#[export_name]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

pub mod actions;
pub mod archive;
pub mod desktop;
pub mod detect;
pub mod elf;
pub mod error;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub mod install;
pub mod provider;
pub mod record;

use std::sync::Arc;

use ext::{
    API_VERSION, Capabilities, Decorator, Extension, Manifest, Platforms, Points, Provider,
    Registry, Version,
};
use types::{Error, ExtId, Location, ProviderId};

pub use actions::Installer;
pub use provider::{AppsDecorator, AppsProvider};

/// The extension id.
pub const ID: ExtId = ExtId("installer");

/// The view's name: `rick://apps`.
pub const VIEW: &str = "apps";

/// The location of the view, `rick://apps`, the path being the bare view
/// name as the `rick` provider writes its locations.
#[must_use]
pub fn location() -> Location {
    Location::new(ProviderId::RICK, VIEW)
}

/// Whether `loc` is the view itself (a leading slash, as a hand-edited
/// file might carry, is tolerated).
#[must_use]
pub fn is_view(loc: &Location) -> bool {
    loc.provider == ProviderId::RICK && view_path(loc) == Some(&[][..])
}

/// The path below the view, with its separators: empty for the view
/// itself, `None` for a location outside it.
pub(crate) fn view_path(loc: &Location) -> Option<&[u8]> {
    let bytes = loc.path.as_bytes();
    let start = bytes
        .iter()
        .take_while(|b| matches!(b, b'/' | b'\\'))
        .count();
    let rest = bytes[start..].strip_prefix(VIEW.as_bytes())?;
    if rest.first().is_some_and(|b| !matches!(b, b'/' | b'\\')) {
        return None;
    }
    Some(rest)
}

/// Whether the installer works on this platform: Linux only in this
/// version.
#[must_use]
pub fn supported() -> bool {
    Platforms::current() == Platforms::LINUX
}

/// The extension: the view, its columns and the installer's actions.
#[derive(Debug)]
pub struct InstallerExtension {
    manifest: Manifest,
}

impl Default for InstallerExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl InstallerExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Installer",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PROVIDER | Points::DECORATOR | Points::ACTION,
                platforms: Platforms::ALL,
            },
        }
    }
}

impl Extension for InstallerExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[
            ("en", include_str!("../locales/en.ftl")),
            ("de", include_str!("../locales/de.ftl")),
        ]);
        let platform = reg
            .platform()
            .ok_or(Error::Unsupported(
                "the installer needs the platform services the host lends",
            ))?
            .clone();
        let provider = Arc::new(AppsProvider::new(platform));
        reg.view(VIEW, Arc::clone(&provider) as Arc<dyn Provider>);
        reg.decorator(Arc::new(AppsDecorator::new(Arc::clone(&provider))) as Arc<dyn Decorator>);
        let installer = Arc::new(Installer::new(provider));
        for action in actions::all(&installer) {
            reg.action(action);
        }
        Ok(())
    }
}

ext::plugin!(|| Arc::new(InstallerExtension::new()) as Arc<dyn Extension>);

#[cfg(test)]
mod tests;
