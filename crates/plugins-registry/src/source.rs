//! Where the index is and which key signs it.

use crate::Key;

/// The address of the registry's index (specs/plugin-distribution.md §2,
/// decision 3). Its signature is beside it, under the same name with
/// `.minisig` appended.
pub const INDEX_URL: &str = "https://get.openbreach.dev/registry/index.json";

/// The registry's public key, Trebuchet's `keys/registry.pub`, as
/// committed in this crate's `keys/` folder. Empty until the key pair
/// exists, and the registry is off for as long.
const COMPILED_KEY: &str = include_str!("../keys/registry.pub");

/// The index to read and the key it must be signed with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The address of the index.
    pub url: String,
    /// The key the index is verified with; `None` while there is none,
    /// and then nothing is fetched.
    pub key: Option<Key>,
    /// `rick.toml` names another address or another key than this build's
    /// own: the Extensions dialogue then says which address it reads.
    pub custom: bool,
}

impl Source {
    /// The source of this build, [`INDEX_URL`] and the key compiled in,
    /// unless `rick.toml` says otherwise: `url` is its `[registry] url`
    /// and `public_key` its `[registry] public_key`, each `None` or empty
    /// when the file does not set it. A `public_key` that is no key
    /// leaves the source without one; the key compiled in never stands in
    /// for a key that was asked for.
    #[must_use]
    pub fn new(url: Option<&str>, public_key: Option<&str>) -> Self {
        Self::with_compiled(COMPILED_KEY, url, public_key)
    }

    /// [`Source::new`] with `compiled` as the text of the key file.
    pub(crate) fn with_compiled(
        compiled: &str,
        url: Option<&str>,
        public_key: Option<&str>,
    ) -> Self {
        let url = url.map(str::trim).filter(|url| !url.is_empty());
        let public_key = public_key.map(str::trim).filter(|key| !key.is_empty());
        let key = Key::parse(public_key.unwrap_or(compiled)).unwrap_or_else(|error| {
            tracing::warn!(%error, "the registry's public key is not used");
            None
        });
        Self {
            url: url.unwrap_or(INDEX_URL).to_owned(),
            key,
            custom: url.is_some() || public_key.is_some(),
        }
    }

    /// The address of the index's signature.
    #[must_use]
    pub fn signature_url(&self) -> String {
        format!("{}.minisig", self.url)
    }

    /// The registry's host, with its port when the address names one:
    /// what the details of an available plugin show as its source. The
    /// whole address when it has no host to show.
    #[must_use]
    pub fn host(&self) -> String {
        reqwest::Url::parse(&self.url)
            .ok()
            .and_then(|url| {
                let host = url.host_str()?.to_owned();
                Some(match url.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host,
                })
            })
            .unwrap_or_else(|| self.url.clone())
    }
}
