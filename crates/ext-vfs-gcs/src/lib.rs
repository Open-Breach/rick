//! Google Cloud Storage as locations: the `gcs` provider.
//!
//! The provider's root lists its connections, one folder per project the
//! user added from the side panel's Cloud section; below `gcs:/<project>`
//! the buckets are folders, and below a bucket the flat object names show
//! as folders by prefix. Listings stream page by page as the service
//! returns them; a download is a streamed media `GET`, an upload a
//! resumable upload fed from the machine in chunks, whose session is
//! abandoned when the writer is dropped before it is finished, so a
//! cancelled upload leaves no object behind; a new folder is a zero-byte
//! object with a trailing slash. There is no rename: a move within the
//! account is a `Rewrite` on the service per object, continued token by
//! token with the bytes rewritten as the progress, then a delete, through
//! [`Provider::transfer`] under
//! `NATIVE_TRANSFER`, within a bucket or across two buckets of the account,
//! so nothing streams through the machine. The store notifies nothing:
//! `watch` is `Unsupported` and the kernel polls; the account is a
//! remote-latency mount (`REMOTE`).
//!
//! Connections are the application's: it keeps the project and the
//! endpoint in `rick.toml` and the service account's key (the JSON file's
//! contents) in the platform keychain, and hands the list to the
//! [`Connector`] this plugin registers ([`GcsConnections`]). The key signs
//! a JWT ([`auth`]) that the token endpoint exchanges for a bearer token;
//! without a key the requests go unauthenticated, which an emulator takes.
//! Requests travel over rustls with ring.
//!
//! The extension takes the platform from the registry it registers with
//! ([`Registry::platform`]) for the keychain; without one it refuses to
//! register. It builds as a `cdylib` the plugin host loads from
//! `plugins/vfs-gcs/` and as an `rlib` for the binary's opt-in `vfs-gcs`
//! feature and the tests.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

pub mod auth;
mod client;
mod connection;
mod provider;

use std::sync::Arc;

use ext::{
    API_VERSION, Capabilities, Connector, Extension, Manifest, Platforms, Points, Provider,
    Registry, Version,
};
use types::{Error, ExtId, Location, ProviderId};

pub use client::{GcsClient, GcsError};
pub use connection::{GcsConnections, schema};
pub use provider::GcsProvider;

/// The extension id.
pub const ID: ExtId = ExtId("vfs-gcs");

/// The provider id; `vfs-rick` lists it under Cloud.
pub const PROVIDER_ID: ProviderId = ProviderId("gcs");

/// Where a location points: the root, a project, a bucket or an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// `gcs:/`: the connections.
    Root,
    /// `gcs:/<project>`: the project's buckets.
    Project(String),
    /// `gcs:/<project>/<bucket>`: the bucket's top-level objects.
    Bucket {
        /// The connection name.
        project: String,
        /// The bucket.
        bucket: String,
    },
    /// `gcs:/<project>/<bucket>/<name>`: an object, or the folder of the
    /// objects whose names start with `<name>/`.
    Object {
        /// The connection name.
        project: String,
        /// The bucket.
        bucket: String,
        /// The object name, with `/` separators and no trailing slash.
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
        let project = parts.next().unwrap_or_default().to_owned();
        let Some(bucket) = parts.next().filter(|b| !b.is_empty()) else {
            return Self::Project(project);
        };
        let bucket = bucket.to_owned();
        match parts
            .next()
            .map(|n| n.trim_matches(['/', '\\']).replace('\\', "/"))
        {
            Some(name) if !name.is_empty() => Self::Object {
                project,
                bucket,
                name,
            },
            _ => Self::Bucket { project, bucket },
        }
    }

    /// The connection name, for everything but the root.
    #[must_use]
    pub fn project(&self) -> Option<&str> {
        match self {
            Self::Root => None,
            Self::Project(p)
            | Self::Bucket { project: p, .. }
            | Self::Object { project: p, .. } => Some(p),
        }
    }

    /// The location of an object, bucket or project.
    #[must_use]
    pub fn location(&self) -> Location {
        match self {
            Self::Root => Location::new(PROVIDER_ID, "/"),
            Self::Project(p) => Location::new(PROVIDER_ID, format!("/{p}")),
            Self::Bucket { project, bucket } => {
                Location::new(PROVIDER_ID, format!("/{project}/{bucket}"))
            }
            Self::Object {
                project,
                bucket,
                name,
            } => Location::new(PROVIDER_ID, format!("/{project}/{bucket}/{name}")),
        }
    }
}

/// The extension: the provider and its connector.
#[derive(Debug)]
pub struct GcsExtension {
    manifest: Manifest,
}

impl Default for GcsExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl GcsExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Google Cloud Storage",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PROVIDER,
                platforms: Platforms::ALL,
                previews: &[],
            },
        }
    }
}

impl Extension for GcsExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        let platform = reg
            .platform()
            .ok_or(Error::Unsupported(
                "the Google Cloud Storage provider needs the platform services the host lends",
            ))?
            .clone();
        let connections = Arc::new(GcsConnections::new(platform));
        reg.provider(Arc::new(GcsProvider::new(Arc::clone(&connections))) as Arc<dyn Provider>);
        reg.connector(connections as Arc<dyn Connector>);
        Ok(())
    }
}

ext::plugin!(|| Arc::new(GcsExtension::new()) as Arc<dyn Extension>);

#[cfg(test)]
mod tests;
