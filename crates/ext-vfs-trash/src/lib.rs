//! The trash as a browsable location: the view `trash` of the `rick`
//! provider, `rick://trash`.
//!
//! [`TrashProvider`] lists what the platform's [`Trash`] holds, one entry per
//! trashed item named as it was before it was trashed, with its size, its
//! deletion date as the modification time and a folder browsable below the
//! view like any other; `open_read` serves the preview and `remove` with
//! `RemoveMode::Permanent` deletes an item for good. Nothing else is
//! possible in the view and [`Capabilities`] say so; `watch` is
//! `Unsupported`, the kernel polls. A [`Decorator`] adds the *Original
//! location* column, and the "Trash" action group (`trash.restore`,
//! `trash.restore_items`, `trash.empty`) is available in the view only.
//!
//! The extension takes the platform from the registry it registers with
//! ([`Registry::platform`]); without one it refuses to register. It builds
//! as a `cdylib` the plugin host loads from `plugins/vfs-trash/` and as an
//! `rlib` for the binary's opt-in `vfs-trash` feature and the tests.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

mod actions;
mod provider;

use std::sync::Arc;

use ext::platform::Trash;
use ext::{
    API_VERSION, Capabilities, Decorator, Extension, Manifest, Platforms, Points, Registry, Version,
};
use types::{Error, ExtId, Location, ProviderId};

pub use actions::{EMPTY, RESTORE, RESTORE_ITEMS};
pub use provider::{ORIGINAL_COLUMN, TrashDecorator, TrashProvider};

/// The extension id.
pub const ID: ExtId = ExtId("vfs-trash");

/// The view's name: `rick://trash`.
pub const VIEW: &str = "trash";

/// The location of the view: `rick://trash`, the path being the bare
/// view name as the `rick` provider writes its locations.
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

/// The extension: the view, its decorator and the trash actions.
#[derive(Debug)]
pub struct TrashExtension {
    manifest: Manifest,
}

impl Default for TrashExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl TrashExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Trash",
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

impl Extension for TrashExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        let platform = reg
            .platform()
            .ok_or(Error::Unsupported(
                "the trash provider needs the platform services the host lends",
            ))?
            .clone();
        let trash: Arc<dyn Trash> = platform;
        let provider = Arc::new(TrashProvider::new(trash));
        reg.view(VIEW, Arc::clone(&provider) as Arc<dyn ext::Provider>);
        reg.decorator(Arc::new(TrashDecorator::new(Arc::clone(&provider))) as Arc<dyn Decorator>);
        for action in actions::all(&provider) {
            reg.action(action);
        }
        Ok(())
    }
}

ext::plugin!(|| Arc::new(TrashExtension::new()) as Arc<dyn Extension>);

#[cfg(test)]
mod tests;
