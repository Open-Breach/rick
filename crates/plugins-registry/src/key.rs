//! The registry's public key and the check of a signature against it.

use minisign_verify::{PublicKey, Signature};

use crate::Error;

/// The first words of the comment line of a minisign file.
const COMMENT: &str = "untrusted comment:";

/// A minisign public key: the Ed25519 key an index must be signed with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key(PublicKey);

impl Key {
    /// Reads a public key as minisign writes it: the base64 line, with or
    /// without the comment line a `.pub` file has above it. An empty text
    /// is no key, `None`: that is the committed `keys/registry.pub` until
    /// the key pair exists.
    ///
    /// # Errors
    ///
    /// [`Error::Key`] for a text that is neither empty nor a key.
    pub fn parse(text: &str) -> Result<Option<Self>, Error> {
        let mut lines = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with(COMMENT));
        let Some(line) = lines.next() else {
            return Ok(None);
        };
        if lines.next().is_some() {
            return Err(Error::Key("more than one line".to_owned()));
        }
        PublicKey::from_base64(line)
            .map(|key| Some(Self(key)))
            .map_err(|error| Error::Key(error.to_string()))
    }

    /// Checks that `signature`, the text of a `.minisig` file, is this
    /// key's signature of `bytes`. Only the signatures minisign makes
    /// today are taken, over the file's hash; the legacy form is refused.
    ///
    /// # Errors
    ///
    /// [`Error::Unverified`] for a text that is no signature, a signature
    /// of another key, and a signature of other bytes.
    pub fn verify(&self, bytes: &[u8], signature: &str) -> Result<(), Error> {
        let signature = Signature::decode(signature).map_err(|error| {
            tracing::warn!(%error, "the index's signature could not be read");
            Error::Unverified
        })?;
        self.0.verify(bytes, &signature, false).map_err(|error| {
            tracing::warn!(%error, "the index's signature does not verify");
            Error::Unverified
        })
    }
}
