//! The requests: which addresses are asked at all, and the two for the
//! index and its signature.
//!
//! One rule holds for every request of this crate, the index, its
//! signature and a package alike (specs/plugin-distribution.md §6):
//! `https://` is asked of any host, plain `http://` only of this machine,
//! so that no plain-HTTP request ever leaves it. [`Clients::get`] is the
//! one door, and it holds every address against the rule before it
//! connects: the one it is given and each one a redirect names.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use crate::{Error, Index, Source};

/// More than any index weighs: a body beyond it is not read to its end.
const MAX_INDEX: usize = 16 * 1024 * 1024;
/// A minisign signature is four short lines.
const MAX_SIGNATURE: usize = 16 * 1024;
/// How long a connection may take to open.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one of the two requests may take in all, and how long any
/// request may stand still.
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

/// How an address may be asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Way {
    /// `https://`, any host.
    Secure,
    /// Plain `http://`, which stays on this machine.
    Local,
}

/// The rule of which addresses are asked, and how: `https://` with a
/// host, any host; plain `http://` only when the host is this machine.
/// Anything else is `None` and is never requested.
///
/// This machine is a loopback address written as one — `127.0.0.0/8`
/// and `::1`, in whatever spelling the address parser takes for them —
/// or the name `localhost`, exactly. No other name is, whatever it
/// resolves to: finding that out is a question to a resolver, which may
/// itself be elsewhere, and the answer may be another one by the time
/// the connection is made. `localhost` is not looked up either; the
/// client of plain HTTP takes it for the loopback addresses itself.
pub(crate) fn way(url: &reqwest::Url) -> Option<Way> {
    let host = url.host_str().filter(|host| !host.is_empty())?;
    match url.scheme() {
        "https" => Some(Way::Secure),
        "http" => {
            // An IPv6 address stands in brackets in an address.
            let loopback = host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback());
            (loopback || host == LOCALHOST).then_some(Way::Local)
        }
        _ => None,
    }
}

/// The one name that is this machine.
const LOCALHOST: &str = "localhost";
/// What [`LOCALHOST`] is, whatever a hosts file or a resolver says. The
/// port is the address's own.
const LOOPBACK: [SocketAddr; 2] = [
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
    SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0),
];
/// How many times one request is sent on before it is given up.
const MAX_REDIRECTS: usize = 10;

/// The clients of every request, one for each [`Way`]: `rick/<version>`
/// as the user agent, no referrer, no cookies, TLS through rustls with
/// ring. Neither follows a redirect by itself; [`Clients::get`] does.
pub(crate) struct Clients {
    /// For `https://`, through the proxy the system names, if any.
    pub(crate) secure: reqwest::Client,
    /// For plain `http://`, which goes to this machine only: never
    /// through a proxy, which would be another machine, and with
    /// `localhost` as the loopback addresses without a lookup.
    pub(crate) local: reqwest::Client,
}

impl Clients {
    /// The clients of a Rick of version `rick`. A request may take
    /// `whole` in all for each address it is sent on to, or as long as
    /// it likes while something arrives every minute.
    pub(crate) fn new(rick: &str, whole: Option<Duration>) -> Result<Self, Error> {
        Self::build(rick, whole, |builder| builder)
    }

    /// [`Clients::new`] for a test: both clients take `name` for
    /// `address`, so a request to `name` arrives at a server of the
    /// test, should one be made.
    #[cfg(test)]
    pub(crate) fn resolving(rick: &str, name: &str, address: SocketAddr) -> Result<Self, Error> {
        Self::build(rick, None, |builder| builder.resolve(name, address))
    }

    fn build(
        rick: &str,
        whole: Option<Duration>,
        with: impl Fn(reqwest::ClientBuilder) -> reqwest::ClientBuilder,
    ) -> Result<Self, Error> {
        // Once for the process; a second caller is told it is done already.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let builder = || {
            let builder = reqwest::Client::builder()
                .user_agent(user_agent(rick))
                .referer(false)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(REQUEST_TIMEOUT);
            with(match whole {
                Some(whole) => builder.timeout(whole),
                None => builder,
            })
        };
        let built = |builder: reqwest::ClientBuilder| {
            builder
                .build()
                .map_err(|error| Error::Fetch(error.to_string()))
        };
        Ok(Self {
            secure: built(builder())?,
            local: built(builder().no_proxy().resolve_to_addrs(LOCALHOST, &LOOPBACK))?,
        })
    }

    /// One GET of `address`, sent on along its redirects, and the answer
    /// of the last address: the caller reads its status and its body.
    /// Every address is held against [`way`] before anything is sent to
    /// it, the first and each one a redirect names, whatever address the
    /// request started from; one that fails is not connected to.
    ///
    /// # Errors
    ///
    /// [`Error::Refused`] for an address the rule does not take, before
    /// any request to it; [`Error::Fetch`] when a request fails, when a
    /// redirect names no address and after [`MAX_REDIRECTS`] of them.
    pub(crate) async fn get(&self, address: &str) -> Result<reqwest::Response, Error> {
        let refused = |url: String, redirected| Error::Refused { url, redirected };
        let mut url = reqwest::Url::parse(address.trim())
            .map_err(|_| refused(address.trim().to_owned(), false))?;
        for hop in 0..=MAX_REDIRECTS {
            let client = match way(&url) {
                Some(Way::Secure) => &self.secure,
                Some(Way::Local) => &self.local,
                None => return Err(refused(url.into(), hop > 0)),
            };
            let response = client.get(url).send().await.map_err(failed)?;
            match redirect(&response)? {
                Some(next) => url = next,
                None => return Ok(response),
            }
        }
        Err(Error::Fetch(format!(
            "{address}: more than {MAX_REDIRECTS} redirects"
        )))
    }
}

/// The address `response` sends its request on to, when it does: a 301,
/// 302, 303, 307 or 308 with a `Location`, read against the address that
/// answered.
fn redirect(response: &reqwest::Response) -> Result<Option<reqwest::Url>, Error> {
    use reqwest::StatusCode;
    let sends_on = matches!(
        response.status(),
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    );
    let Some(location) = response
        .headers()
        .get(reqwest::header::LOCATION)
        .filter(|_| sends_on)
    else {
        return Ok(None);
    };
    location
        .to_str()
        .ok()
        .and_then(|location| response.url().join(location).ok())
        .map(Some)
        .ok_or_else(|| Error::Fetch(format!("{}: a redirect to no address", response.url())))
}

/// What a request that failed is reported as: reqwest's text without the
/// address, which the caller names itself.
pub(crate) fn failed(error: reqwest::Error) -> Error {
    Error::Fetch(error.without_url().to_string())
}

/// One GET of `url`, its body read up to `limit` bytes.
async fn get(clients: &Clients, url: &str, limit: usize) -> Result<Vec<u8>, Error> {
    let mut response = clients.get(url).await?;
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
/// [`Error::Refused`] when the index's address, or one it is sent on to,
/// is plain HTTP to another machine; [`Error::Fetch`] when either file
/// does not arrive;
/// [`Error::Unverified`] when the signature is not the key's signature of
/// the index; [`Error::NewerSchema`] and [`Error::Unreadable`] as
/// [`Index::parse`] has them.
pub async fn fetch(source: &Source, rick: &str) -> Result<Fetched, Error> {
    let key = source.key.as_ref().ok_or(Error::NotSetUp)?;
    let clients = Clients::new(rick, Some(REQUEST_TIMEOUT))?;
    let bytes = get(&clients, &source.url, MAX_INDEX).await?;
    let signature = get(&clients, &source.signature_url(), MAX_SIGNATURE).await?;
    let signature = String::from_utf8(signature).map_err(|_| Error::Unverified)?;
    key.verify(&bytes, &signature)?;
    let index = Index::parse(&bytes)?;
    Ok(Fetched {
        index,
        bytes,
        signature,
    })
}
