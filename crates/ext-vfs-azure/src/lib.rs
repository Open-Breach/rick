//! Azure Blob Storage as locations: the `azure` provider.
//!
//! The provider's root lists its connections, one folder per storage
//! account the user added from the side panel's Cloud section; below
//! `azure:/<account>` the containers are folders, and below a container
//! the flat blob names show as folders by prefix. Listings stream page by
//! page as the service returns them; a download is a streamed `GET`, an
//! upload a block upload fed from the machine, whose blocks stay
//! uncommitted when the writer is dropped before it is finished, so a
//! cancelled upload leaves no blob behind; a new folder is a zero-byte blob
//! with a trailing slash. There is no rename: a move within the account is
//! a *Copy Blob* on the service per blob, polled for its progress and
//! aborted on cancel where the service still allows, then a delete,
//! through [`Provider::transfer`] under
//! `NATIVE_TRANSFER`, within a container or across two containers of the
//! account, so nothing streams through the machine. Blob storage notifies
//! nothing: `watch` is `Unsupported` and the kernel polls; the account is
//! a remote-latency mount (`REMOTE`).
//!
//! Connections are the application's: it keeps the account name and the
//! endpoint in `rick.toml` and the account key in the platform keychain,
//! and hands the list to the [`Connector`] this plugin registers
//! ([`AzureConnections`]). Requests are signed with the *Shared Key*
//! scheme ([`sign`]) and travel over rustls with ring.
//!
//! The extension takes the platform from the registry it registers with
//! ([`Registry::platform`]) for the keychain; without one it refuses to
//! register. It builds as a `cdylib` the plugin host loads from
//! `plugins/vfs-azure/` and as an `rlib` for the binary's opt-in
//! `vfs-azure` feature and the tests.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

mod client;
mod connection;
mod provider;

use std::sync::Arc;

use ext::{
    API_VERSION, Capabilities, Connector, Extension, Manifest, Platforms, Points, Provider,
    Registry, Version,
};
use types::{Error, ExtId, Location, ProviderId};

pub use client::{AzureClient, AzureError, sign};
pub use connection::{AzureConnections, schema};
pub use provider::AzureProvider;

/// The extension id.
pub const ID: ExtId = ExtId("vfs-azure");

/// The provider id; `vfs-rick` lists it under Cloud.
pub const PROVIDER_ID: ProviderId = ProviderId("azure");

/// Where a location points: the root, an account, a container or a blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// `azure:/`: the connections.
    Root,
    /// `azure:/<account>`: the account's containers.
    Account(String),
    /// `azure:/<account>/<container>`: the container's top-level blobs.
    Container {
        /// The connection name.
        account: String,
        /// The container.
        container: String,
    },
    /// `azure:/<account>/<container>/<name>`: a blob, or the folder of the
    /// blobs whose names start with `<name>/`.
    Blob {
        /// The connection name.
        account: String,
        /// The container.
        container: String,
        /// The blob name, with `/` separators and no trailing slash.
        name: String,
    },
}

impl Place {
    /// Where `loc` points.
    #[must_use]
    pub fn of(loc: &Location) -> Self {
        let text = loc.path.to_string_lossy();
        let trimmed = text.trim_matches(['/', '\\']);
        if trimmed.is_empty() {
            return Self::Root;
        }
        let mut parts = trimmed.splitn(3, ['/', '\\']);
        let account = parts.next().unwrap_or_default().to_owned();
        let Some(container) = parts.next().filter(|c| !c.is_empty()) else {
            return Self::Account(account);
        };
        let container = container.to_owned();
        match parts
            .next()
            .map(|n| n.trim_matches(['/', '\\']).replace('\\', "/"))
        {
            Some(name) if !name.is_empty() => Self::Blob {
                account,
                container,
                name,
            },
            _ => Self::Container { account, container },
        }
    }

    /// The connection name, for everything but the root.
    #[must_use]
    pub fn account(&self) -> Option<&str> {
        match self {
            Self::Root => None,
            Self::Account(a)
            | Self::Container { account: a, .. }
            | Self::Blob { account: a, .. } => Some(a),
        }
    }

    /// The location of a blob, container or account.
    #[must_use]
    pub fn location(&self) -> Location {
        match self {
            Self::Root => Location::new(PROVIDER_ID, "/"),
            Self::Account(a) => Location::new(PROVIDER_ID, format!("/{a}")),
            Self::Container { account, container } => {
                Location::new(PROVIDER_ID, format!("/{account}/{container}"))
            }
            Self::Blob {
                account,
                container,
                name,
            } => Location::new(PROVIDER_ID, format!("/{account}/{container}/{name}")),
        }
    }
}

/// The extension: the provider and its connector.
#[derive(Debug)]
pub struct AzureExtension {
    manifest: Manifest,
}

impl Default for AzureExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl AzureExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Azure Blob",
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

impl Extension for AzureExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        let platform = reg
            .platform()
            .ok_or(Error::Unsupported(
                "the Azure provider needs the platform services the host lends",
            ))?
            .clone();
        let connections = Arc::new(AzureConnections::new(platform));
        reg.provider(Arc::new(AzureProvider::new(Arc::clone(&connections))) as Arc<dyn Provider>);
        reg.connector(connections as Arc<dyn Connector>);
        Ok(())
    }
}

ext::plugin!(|| Arc::new(AzureExtension::new()) as Arc<dyn Extension>);

#[cfg(test)]
mod tests;
