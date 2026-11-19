//! The client of the plugin registry (specs/plugin-distribution.md §3 and
//! §6): the signed list of the plugins a release of Rick publishes.
//!
//! The registry is one JSON document, the **index**, with a minisign
//! signature beside it (`docs/spec.md` 7.2 and 7.5 in
//! Open-Breach/trebuchet). This crate reads the two and downloads the
//! packages the index names, and nothing else of it reaches the network:
//!
//! - [`Source`] says where the index is and which key signs it: the
//!   address of the specification and the key compiled in from
//!   `keys/registry.pub`, or what `[registry] url` and `public_key` of
//!   `rick.toml` name. Without a key the registry is off.
//! - [`fetch`] asks for the two files, one GET each, with `rick/<version>`
//!   as the user agent and nothing else, and hands back an index only
//!   when its signature verifies and its `schema` is one this Rick reads.
//! - [`Cache`] keeps the last index that verified, with its signature, and
//!   verifies it again on every read. [`refresh`] is a fetch written to
//!   the cache; [`due`] says whether the daily check in the background is
//!   due.
//! - [`Index::offer`] is the rule of what fits: a build is for one
//!   platform and one release of Rick, compared as whole strings.
//! - [`download`] fetches a package to a file and holds it against the
//!   size and the SHA-256 the index names; a package that is another one
//!   is deleted.
//!
//! Nothing here decides *when* to ask or whether the user allows it: that
//! is the application's, with the switch of the settings.
//!
//! ```no_run
//! # async fn check(cache_dir: &std::path::Path) -> Result<(), plugins_registry::Error> {
//! use plugins_registry::{Cache, Offer, RICK, Source, platform, refresh};
//!
//! // The address and key of this build, or those of `rick.toml`.
//! let source = Source::new(None, None);
//! let cache = Cache::new(cache_dir);
//! // Two GETs; the index is verified before it is used or cached.
//! let cached = refresh(&source, &cache, RICK).await?;
//! if let Offer::Plugins(offered) = cached.index.offer(&platform(), RICK) {
//!     for plugin in offered {
//!         println!("{} {}", plugin.plugin.id, plugin.build.url);
//!     }
//! }
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]

mod cache;
mod download;
mod fetch;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
mod index;
mod key;
mod source;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

pub use cache::{CACHE_DIR, CHECK_EVERY, Cache, Cached, INDEX_FILE, SIGNATURE_FILE, due, refresh};
pub use download::{Downloaded, Expected, download};
pub use fetch::{Fetched, fetch, user_agent};
pub use index::{
    Build, Connection, Index, Meta, Offer, Offered, Plugin, SCHEMA, Triggers, is_pre_release,
    platform, platform_name,
};
pub use key::Key;
pub use source::{INDEX_URL, Source};

/// The version of this Rick: what a build's `app_version` is compared
/// with, and what the user agent names.
pub const RICK: &str = env!("CARGO_PKG_VERSION");

/// Why there is no index to use.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No public key: the registry is off, and nothing is fetched.
    #[error("the plugin list is not set up in this build")]
    NotSetUp,
    /// A request was not answered with the file: no network, a refused
    /// connection, another status than success, a body beyond any size
    /// an index has.
    #[error("the plugin list could not be fetched: {0}")]
    Fetch(String),
    /// The signature is missing, is no minisign signature, or is not the
    /// key's signature of these bytes.
    #[error("the plugin list could not be verified")]
    Unverified,
    /// The index says a `schema` above [`SCHEMA`]; the number is the one
    /// it says.
    #[error("the plugin list needs a newer Rick: schema {0}")]
    NewerSchema(u32),
    /// The signature verified and the bytes are still no index.
    #[error("the plugin list could not be read: {0}")]
    Unreadable(String),
    /// A package arrived and is not the one the index describes: another
    /// size or another SHA-256. It was deleted.
    #[error("the download did not match the plugin list")]
    Mismatch,
    /// A text that should be a minisign public key is none.
    #[error("not a minisign public key: {0}")]
    Key(String),
    /// Reading or writing the cache failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or folder.
        path: PathBuf,
        /// What the system said.
        source: std::io::Error,
    },
}
