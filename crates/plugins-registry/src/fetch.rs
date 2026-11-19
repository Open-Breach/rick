//! The two requests: the index and its signature.

use std::time::Duration;

use crate::{Error, Index, Source};

/// More than any index weighs: a body beyond it is not read to its end.
const MAX_INDEX: usize = 16 * 1024 * 1024;
/// A minisign signature is four short lines.
const MAX_SIGNATURE: usize = 16 * 1024;
/// How long a connection may take to open.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one of the two requests may take in all.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// An index that was fetched and whose signature verified, with the bytes
/// as they were served: what [`Cache::write`](crate::Cache::write) keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// The index.
    pub index: Index,
    /// The bytes of `index.json`, which the signature is of.
    pub bytes: Vec<u8>,
    /// The text of `index.json.minisig`.
    pub signature: String,
}

/// What Rick calls itself in a request: `rick/<version>`, and nothing
/// that tells one install from another.
#[must_use]
pub fn user_agent(rick: &str) -> String {
    format!("rick/{rick}")
}

/// The client of the two requests: `rick/<version>` as the user agent, no
/// referrer, no cookies, TLS through rustls with ring.
fn client(rick: &str) -> Result<reqwest::Client, Error> {
    // Once for the process; a second caller is told it is done already.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(user_agent(rick))
        .referer(false)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| Error::Fetch(error.to_string()))
}

/// One GET of `url`, its body read up to `limit` bytes.
async fn get(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>, Error> {
    let failed = |error: reqwest::Error| {
        // Without the address: reqwest's own text names it again.
        Error::Fetch(error.without_url().to_string())
    };
    let mut response = client.get(url).send().await.map_err(failed)?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Fetch(format!("{url}: HTTP {}", status.as_u16())));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(failed)? {
        if body.len() + chunk.len() > limit {
            return Err(Error::Fetch(format!("{url}: more than {limit} bytes")));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Fetches the index of `source` and its signature, one GET each, as a
/// Rick of version `rick`, and verifies the one with the other.
///
/// # Errors
///
/// [`Error::NotSetUp`] without a key, before any request;
/// [`Error::Fetch`] when either file does not arrive;
/// [`Error::Unverified`] when the signature is not the key's signature of
/// the index; [`Error::NewerSchema`] and [`Error::Unreadable`] as
/// [`Index::parse`] has them.
pub async fn fetch(source: &Source, rick: &str) -> Result<Fetched, Error> {
    let key = source.key.as_ref().ok_or(Error::NotSetUp)?;
    let client = client(rick)?;
    let bytes = get(&client, &source.url, MAX_INDEX).await?;
    let signature = get(&client, &source.signature_url(), MAX_SIGNATURE).await?;
    let signature = String::from_utf8(signature).map_err(|_| Error::Unverified)?;
    key.verify(&bytes, &signature)?;
    let index = Index::parse(&bytes)?;
    Ok(Fetched {
        index,
        bytes,
        signature,
    })
}
