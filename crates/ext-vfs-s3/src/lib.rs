//! Object storage over the S3 API as locations: the `s3` provider, for
//! Amazon S3, Cloudflare R2, `MinIO` and every other S3-compatible store,
//! one endpoint setting apart.
//!
//! The provider's root lists its connections, one folder per account the
//! user added from the side panel's Cloud section; below `s3:/<account>`
//! the buckets are folders, and below a bucket the flat keys show as
//! folders by prefix (`photos/2026/a.jpg` is the file `a.jpg` in the folder
//! `2026` in the folder `photos`). Listings stream page by page as the
//! service returns them; a download is a streamed `GET`, an upload a
//! multipart upload fed from the machine, aborted when the writer is
//! dropped before it is finished so that a cancelled upload leaves no
//! object behind; a new folder is a zero-byte key with a trailing slash.
//! There is no rename: a move within the account is a `CopyObject` per
//! object then a delete, through [`Provider::transfer`]
//! under `NATIVE_TRANSFER`, within a bucket or across two buckets of the
//! account, so nothing streams through the machine. Object stores notify
//! nothing: `watch` is `Unsupported` and the kernel polls; the account is
//! a remote-latency mount (`REMOTE`).
//!
//! Connections are the application's: it keeps endpoint, region, access
//! key and addressing style in `rick.toml` and the secret key in the
//! platform keychain, and hands the list to the [`Connector`] this plugin
//! registers ([`S3Connections`]). Requests are signed with Signature
//! Version 4 ([`sign`]) and travel over rustls with ring.
//!
//! The extension takes the platform from the registry it registers with
//! ([`Registry::platform`]) for the keychain; without one it refuses to
//! register. It builds as a `cdylib` the plugin host loads from
//! `plugins/vfs-s3/` and as an `rlib` for the binary's opt-in `vfs-s3`
//! feature and the tests.

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

pub use client::{S3Client, S3Error, sign};
pub use connection::{S3Connections, schema};
pub use provider::S3Provider;

/// The extension id.
pub const ID: ExtId = ExtId("vfs-s3");

/// The provider id; `vfs-rick` lists it under Cloud.
pub const PROVIDER_ID: ProviderId = ProviderId("s3");

/// Where a location points: the root, an account, a bucket or a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// `s3:/`: the connections.
    Root,
    /// `s3:/<account>`: the account's buckets.
    Account(String),
    /// `s3:/<account>/<bucket>`: the bucket's top-level keys.
    Bucket {
        /// The connection name.
        account: String,
        /// The bucket.
        bucket: String,
    },
    /// `s3:/<account>/<bucket>/<key>`: an object, or the folder of the
    /// keys starting with `<key>/`.
    Key {
        /// The connection name.
        account: String,
        /// The bucket.
        bucket: String,
        /// The key, with `/` separators and no trailing slash.
        key: String,
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
        let Some(bucket) = parts.next().filter(|b| !b.is_empty()) else {
            return Self::Account(account);
        };
        let bucket = bucket.to_owned();
        match parts
            .next()
            .map(|k| k.trim_matches(['/', '\\']).replace('\\', "/"))
        {
            Some(key) if !key.is_empty() => Self::Key {
                account,
                bucket,
                key,
            },
            _ => Self::Bucket { account, bucket },
        }
    }

    /// The connection name, for everything but the root.
    #[must_use]
    pub fn account(&self) -> Option<&str> {
        match self {
            Self::Root => None,
            Self::Account(a) | Self::Bucket { account: a, .. } | Self::Key { account: a, .. } => {
                Some(a)
            }
        }
    }

    /// The location of a key, bucket or account.
    #[must_use]
    pub fn location(&self) -> Location {
        match self {
            Self::Root => Location::new(PROVIDER_ID, "/"),
            Self::Account(a) => Location::new(PROVIDER_ID, format!("/{a}")),
            Self::Bucket { account, bucket } => {
                Location::new(PROVIDER_ID, format!("/{account}/{bucket}"))
            }
            Self::Key {
                account,
                bucket,
                key,
            } => Location::new(PROVIDER_ID, format!("/{account}/{bucket}/{key}")),
        }
    }
}

/// The extension: the provider and its connector.
#[derive(Debug)]
pub struct S3Extension {
    manifest: Manifest,
}

impl Default for S3Extension {
    fn default() -> Self {
        Self::new()
    }
}

impl S3Extension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "S3",
                description: "Browse Amazon S3 and S3-compatible stores as folders. Buckets appear at the root; copy and move inside one account run server-side.",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PROVIDER,
                platforms: Platforms::ALL,
                previews: &[],
                commands: &[],
            },
        }
    }
}

impl Extension for S3Extension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        let platform = reg
            .platform()
            .ok_or(Error::Unsupported(
                "the S3 provider needs the platform services the host lends",
            ))?
            .clone();
        let connections = Arc::new(S3Connections::new(platform));
        reg.provider(Arc::new(S3Provider::new(Arc::clone(&connections))) as Arc<dyn Provider>);
        reg.connector(connections as Arc<dyn Connector>);
        Ok(())
    }
}

ext::plugin!(|| Arc::new(S3Extension::new()) as Arc<dyn Extension>);

#[cfg(test)]
mod tests;
