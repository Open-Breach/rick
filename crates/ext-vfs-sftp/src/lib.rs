//! Remote hosts over SSH as locations: the `sftp` provider.
//!
//! The provider's root lists its connections, one folder per connection
//! the user made from the side panel's Network section; below
//! `sftp:/<connection>` the remote host's files appear, relative to the
//! connection's folder (the login folder by default), listed in the
//! batches the server returns them in, read and written through the SFTP
//! subsystem's own file handles, renamed, removed and linked in place.
//! A move within one host is a remote rename through
//! [`Provider::transfer`] under
//! `NATIVE_TRANSFER`, so nothing streams through the machine; a copy
//! within a host answers `Unsupported` there and the kernel streams it,
//! since the SFTP protocol has no copy. The host is a remote-latency
//! mount (`REMOTE`): no thumbnails, no watch (the kernel polls).
//!
//! Connections are the application's: it keeps host, port, user, key file
//! and folder in `rick.toml` and the password and the key's passphrase in
//! the platform keychain, and hands the list to the [`Connector`] this
//! plugin registers ([`SftpConnections`]). One SSH session per connection
//! is opened on first use and kept until *Disconnect* or an error closes
//! it. The server's host key is checked against the user's
//! `~/.ssh/known_hosts` with OpenSSH semantics: an unknown host is learnt
//! and recorded, a changed key is refused with its fingerprint
//! (QUESTIONS.md 138).
//!
//! The extension takes the platform from the registry it registers with
//! ([`Registry::platform`]), for the keychain and the home directory;
//! without one it refuses to register. It builds as a `cdylib` the plugin
//! host loads from `plugins/vfs-sftp/` and as an `rlib` for the binary's
//! opt-in `vfs-sftp` feature and the tests.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

mod connection;
mod provider;

use std::sync::Arc;

use ext::{
    API_VERSION, Capabilities, Connector, Extension, Manifest, Platforms, Points, Provider,
    Registry, Version,
};
use types::{Error, ExtId, Location, ProviderId};

pub use connection::{SftpConnections, schema};
pub use provider::SftpProvider;

/// The extension id.
pub const ID: ExtId = ExtId("vfs-sftp");

/// The provider id; `vfs-rick` lists it under Network.
pub const PROVIDER_ID: ProviderId = ProviderId("sftp");

/// A location's connection name and the path below it, with `/`
/// separators and no leading slash: `sftp:/nas/photos/a.jpg` is
/// `("nas", "photos/a.jpg")`, `sftp:/nas` is `("nas", "")`. `None` for
/// the root, which lists the connections.
#[must_use]
pub fn split(loc: &Location) -> Option<(String, String)> {
    let text = loc.path.to_string_lossy();
    let trimmed = text.trim_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    let (name, rest) = trimmed.split_once(['/', '\\']).unwrap_or((trimmed, ""));
    let rest = rest.trim_matches(['/', '\\']).replace('\\', "/");
    Some((name.to_owned(), rest))
}

/// The location of `rest` under the connection `name`.
#[must_use]
pub fn location(name: &str, rest: &str) -> Location {
    let rest = rest.trim_matches('/');
    if rest.is_empty() {
        Location::new(PROVIDER_ID, format!("/{name}"))
    } else {
        Location::new(PROVIDER_ID, format!("/{name}/{rest}"))
    }
}

/// The extension: the provider and its connector.
#[derive(Debug)]
pub struct SftpExtension {
    manifest: Manifest,
}

impl Default for SftpExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl SftpExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "SFTP",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PROVIDER,
                platforms: Platforms::ALL,
            },
        }
    }
}

impl Extension for SftpExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        let platform = reg
            .platform()
            .ok_or(Error::Unsupported(
                "the SFTP provider needs the platform services the host lends",
            ))?
            .clone();
        let connections = Arc::new(SftpConnections::new(platform));
        reg.provider(Arc::new(SftpProvider::new(Arc::clone(&connections))) as Arc<dyn Provider>);
        reg.connector(connections as Arc<dyn Connector>);
        Ok(())
    }
}

ext::plugin!(|| Arc::new(SftpExtension::new()) as Arc<dyn Extension>);

#[cfg(test)]
mod tests;
