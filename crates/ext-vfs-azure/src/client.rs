//! The Blob Storage REST client: the Shared Key signature, the requests
//! the provider needs, and the XML they answer.

use std::time::SystemTime;

use base64::Engine as _;
use bytes::Bytes;
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::{Method, StatusCode, Url};
use sha2::Sha256;
use tokio::io::AsyncRead;
use tracing::debug;
use types::Error;

use crate::ID;

/// The REST version every request names.
const VERSION: &str = "2021-12-02";

/// The characters a blob name keeps in a URI, the slashes included.
const PATH: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~')
    .remove(b'/');

/// The characters a query value keeps in a URI: a space travels as `%20`,
/// never as the `+` of a form, which a service may read as a plus and
/// which would then not be the value that was signed.
const QUERY: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// How many blobs one listing page asks for.
const PAGE: &str = "5000";

/// Installs ring as the process's rustls crypto provider, once; a second
/// plugin doing the same is told it is done already and that is fine.
pub(crate) fn install_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// What went wrong with a request, as the service said it.
#[derive(Debug)]
pub struct AzureError {
    /// The HTTP status.
    pub status: u16,
    /// The service's error code, `BlobNotFound`, when it sent one.
    pub code: String,
    /// The service's message.
    pub message: String,
}

impl std::fmt::Display for AzureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.code.is_empty() {
            write!(f, "HTTP {}: {}", self.status, self.message)
        } else {
            write!(f, "{} ({}): {}", self.code, self.status, self.message)
        }
    }
}

impl std::error::Error for AzureError {}

/// The kernel error for a failed request.
fn failed(status: StatusCode, body: &str) -> Error {
    let (code, message) = parse_error(body);
    match (status, code.as_str()) {
        (StatusCode::NOT_FOUND, _)
        | (_, "BlobNotFound" | "ContainerNotFound" | "ResourceNotFound") => Error::NotFound,
        (StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED, _)
        | (_, "AuthenticationFailed" | "AuthorizationFailure") => Error::PermissionDenied,
        _ => Error::Extension {
            ext: ID,
            source: Box::new(AzureError {
                status: status.as_u16(),
                code,
                message,
            }),
        },
    }
}

/// A transport error as the kernel error. A request that never reached the
/// service, or went unanswered, is a lost connection
/// ([`Error::unreachable`]): the pane keeps its place and tries again.
pub(crate) fn transport(error: reqwest::Error) -> Error {
    if error.is_connect() || error.is_timeout() {
        // The cause is in the chain, not in the error's own text.
        let mut text = error.to_string();
        let mut cause = std::error::Error::source(&error);
        while let Some(inner) = cause {
            text.push_str(": ");
            text.push_str(&inner.to_string());
            cause = inner.source();
        }
        return Error::unreachable(text);
    }
    Error::Extension {
        ext: ID,
        source: Box::new(error),
    }
}

/// An error with a message of this plugin's own.
pub(crate) fn message(text: impl Into<String>) -> Error {
    Error::Extension {
        ext: ID,
        source: text.into().into(),
    }
}

/// `<Error><Code>…</Code><Message>…</Message></Error>`, when the body is one.
fn parse_error(body: &str) -> (String, String) {
    let Ok(doc) = roxmltree::Document::parse(body) else {
        return (String::new(), body.trim().chars().take(200).collect());
    };
    let text = |name: &str| {
        doc.descendants()
            .find(|n| n.has_tag_name(name))
            .and_then(|n| n.text())
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned()
    };
    (text("Code"), text("Message"))
}

/// Percent-encodes a blob name for a URI path, keeping the slashes.
#[must_use]
pub fn encode_path(text: &str) -> String {
    utf8_percent_encode(text, PATH).to_string()
}

/// The query string of `pairs`, each name and value percent-encoded.
#[must_use]
pub fn encode_query(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                utf8_percent_encode(k, QUERY),
                utf8_percent_encode(v, QUERY)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// A request before signing.
pub struct Unsigned<'a> {
    /// `GET`, `PUT`, …
    pub method: &'a str,
    /// The URI path, not encoded: `/account/container/blob` in path
    /// style, `/container/blob` with the account as the host.
    pub path: &'a str,
    /// The query parameters, raw, in any order.
    pub query: &'a [(&'a str, &'a str)],
    /// The `x-ms-*` headers, lower-case names, raw values.
    pub ms_headers: &'a [(&'a str, &'a str)],
    /// The body's length; empty for none.
    pub content_length: &'a str,
    /// The `Content-Type` header; empty for none.
    pub content_type: &'a str,
}

/// The `Authorization` header value of a request: the Shared Key scheme
/// over the service's string to sign.
#[must_use]
pub fn sign(req: &Unsigned<'_>, account: &str, key: &[u8]) -> String {
    let mut ms: Vec<(String, String)> = req
        .ms_headers
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.trim().to_owned()))
        .collect();
    ms.sort();
    let mut canonical_headers = String::new();
    for (k, v) in &ms {
        canonical_headers.push_str(k);
        canonical_headers.push(':');
        canonical_headers.push_str(v);
        canonical_headers.push('\n');
    }
    let mut query: Vec<(String, String)> = req
        .query
        .iter()
        .map(|(k, v)| (k.to_lowercase(), (*v).to_owned()))
        .collect();
    query.sort();
    let mut canonical_resource = format!("/{account}{}", req.path);
    for (k, v) in &query {
        canonical_resource.push('\n');
        canonical_resource.push_str(k);
        canonical_resource.push(':');
        canonical_resource.push_str(v);
    }
    let string_to_sign = format!(
        "{}\n\n\n{}\n\n{}\n\n\n\n\n\n\n{canonical_headers}{canonical_resource}",
        req.method, req.content_length, req.content_type
    );
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap_or_else(|_| unreachable!());
    mac.update(string_to_sign.as_bytes());
    let signature = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    format!("SharedKey {account}:{signature}")
}

/// The current time as the `x-ms-date` header writes it.
fn date_now() -> String {
    chrono::DateTime::<chrono::Utc>::from(SystemTime::now())
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

fn parse_time(text: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc2822(text)
        .ok()
        .map(SystemTime::from)
}

/// A container of the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    /// Its name.
    pub name: String,
    /// When it last changed, when the service says.
    pub modified: Option<SystemTime>,
}

/// A blob as a listing or its properties describe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    /// The full name.
    pub name: String,
    /// Its size in bytes.
    pub size: u64,
    /// When it was last written.
    pub modified: Option<SystemTime>,
}

/// One page of a listing with a delimiter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    /// The blob prefixes: the folders, each ending in `/`.
    pub prefixes: Vec<String>,
    /// The blobs at this level.
    pub blobs: Vec<Blob>,
    /// The marker of the next page, when there is one.
    pub next: Option<String>,
}

/// Where a *Copy Blob* stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyState {
    /// The copy's id, for an abort.
    pub id: String,
    /// `success`, `pending`, `failed` or `aborted`.
    pub status: String,
    /// Bytes copied so far, and the total, when the service says.
    pub progress: Option<(u64, u64)>,
}

/// One request before it is sent.
struct Req<'a> {
    method: Method,
    container: Option<&'a str>,
    blob: Option<&'a str>,
    query: &'a [(&'a str, &'a str)],
    ms_headers: &'a [(&'a str, &'a str)],
    body: Option<(Bytes, &'a str)>,
}

impl<'a> Req<'a> {
    fn new(method: Method, container: Option<&'a str>, blob: Option<&'a str>) -> Self {
        Self {
            method,
            container,
            blob,
            query: &[],
            ms_headers: &[],
            body: None,
        }
    }

    fn query(mut self, query: &'a [(&'a str, &'a str)]) -> Self {
        self.query = query;
        self
    }

    fn ms_headers(mut self, headers: &'a [(&'a str, &'a str)]) -> Self {
        self.ms_headers = headers;
        self
    }

    fn body(mut self, body: Bytes, content_type: &'a str) -> Self {
        self.body = Some((body, content_type));
        self
    }
}

/// The client of one storage account.
pub struct AzureClient {
    http: reqwest::Client,
    account: String,
    key: Vec<u8>,
    /// The URL everything hangs off: `https://account.blob.core.windows.net/`
    /// or, for an emulator, `http://host:port/account/`.
    base: Url,
}

impl std::fmt::Debug for AzureClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AzureClient")
            .field("account", &self.account)
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

impl AzureClient {
    /// A client for `account` with its base64 `key`; `endpoint` names an
    /// emulator (scheme, host and port), the public service when empty.
    ///
    /// # Errors
    ///
    /// The plugin's error when the key is not base64, the endpoint not a
    /// URL, or the HTTP client cannot be built.
    pub fn new(account: &str, key: &str, endpoint: &str) -> Result<Self, Error> {
        let key = base64::engine::general_purpose::STANDARD
            .decode(key.trim())
            .map_err(|e| message(format!("the account key is not base64: {e}")))?;
        let base = if endpoint.is_empty() {
            format!("https://{account}.blob.core.windows.net/")
        } else {
            let endpoint = if endpoint.contains("://") {
                endpoint.trim_end_matches('/').to_owned()
            } else {
                format!("https://{}", endpoint.trim_end_matches('/'))
            };
            format!("{endpoint}/{account}/")
        };
        let base = Url::parse(&base).map_err(|e| message(format!("endpoint {base}: {e}")))?;
        install_crypto();
        let http = reqwest::Client::builder().build().map_err(transport)?;
        Ok(Self {
            http,
            account: account.to_owned(),
            key,
            base,
        })
    }

    /// The absolute URL of a blob, as `x-ms-copy-source` wants it.
    pub(crate) fn blob_url(&self, container: &str, blob: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(&self.path_of(Some(container), Some(blob)));
        url
    }

    /// The URI path of the account, a container or a blob, percent-encoded
    /// as it is sent: the path the signature is made over, too.
    fn path_of(&self, container: Option<&str>, blob: Option<&str>) -> String {
        let mut path = self.base.path().to_owned();
        if let Some(container) = container {
            path.push_str(&encode_path(container));
            if let Some(blob) = blob {
                path.push('/');
                path.push_str(&encode_path(blob));
            }
        }
        path
    }

    /// Sends one signed request and returns the response, or the error the
    /// service described.
    async fn send(&self, req: Req<'_>) -> Result<reqwest::Response, Error> {
        let Req {
            method,
            container,
            blob,
            query,
            ms_headers,
            body,
        } = req;
        let mut url = self.base.clone();
        let path = self.path_of(container, blob);
        url.set_path(&path);
        if !query.is_empty() {
            url.set_query(Some(&encode_query(query)));
        }
        let date = date_now();
        let mut ms: Vec<(&str, &str)> =
            vec![("x-ms-date", date.as_str()), ("x-ms-version", VERSION)];
        ms.extend_from_slice(ms_headers);
        let (content_length, content_type) = match &body {
            Some((bytes, kind)) if !bytes.is_empty() => (bytes.len().to_string(), *kind),
            Some((_, kind)) => (String::new(), *kind),
            None => (String::new(), ""),
        };
        // The signed path is the URI's own: in path style the account is
        // its first segment, and the canonical resource adds the account
        // once more in front, as the service does for an emulator's URL.
        let authorization = sign(
            &Unsigned {
                method: method.as_str(),
                path: &path,
                query,
                ms_headers: &ms,
                content_length: &content_length,
                content_type,
            },
            &self.account,
            &self.key,
        );
        let mut request = self.http.request(method.clone(), url.clone());
        for (k, v) in &ms {
            request = request.header(*k, *v);
        }
        request = request.header("authorization", authorization);
        if let Some((bytes, kind)) = body {
            if !kind.is_empty() {
                request = request.header("content-type", kind);
            }
            request = request
                .header("content-length", bytes.len().to_string())
                .body(bytes);
        } else if method == Method::PUT {
            // The service wants a length on every PUT, and the HTTP client
            // sends none for a request without a body: creating a
            // container, copying a blob and aborting a copy were answered
            // `MissingContentLengthHeader` (411).
            request = request.header("content-length", "0");
        }
        debug!(%method, %url, "azure request");
        let response = request.send().await.map_err(transport)?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        Err(failed(status, &body))
    }

    /// The account's containers.
    pub(crate) async fn list_containers(&self) -> Result<Vec<Container>, Error> {
        let mut out = Vec::new();
        let mut marker: Option<String> = None;
        loop {
            let mut query: Vec<(&str, &str)> = vec![("comp", "list")];
            if let Some(m) = marker.as_deref() {
                query.push(("marker", m));
            }
            let text = self
                .send(Req::new(Method::GET, None, None).query(&query))
                .await?
                .text()
                .await
                .map_err(transport)?;
            let (containers, next) = parse_containers(&text)?;
            out.extend(containers);
            match next {
                Some(next) => marker = Some(next),
                None => return Ok(out),
            }
        }
    }

    /// One page of the blobs under `prefix` with `/` as the delimiter.
    pub(crate) async fn list(
        &self,
        container: &str,
        prefix: &str,
        marker: Option<&str>,
        max: Option<&str>,
    ) -> Result<Page, Error> {
        let mut query: Vec<(&str, &str)> = vec![
            ("restype", "container"),
            ("comp", "list"),
            ("delimiter", "/"),
            ("prefix", prefix),
            ("maxresults", max.unwrap_or(PAGE)),
        ];
        if let Some(m) = marker {
            query.push(("marker", m));
        }
        let text = self
            .send(Req::new(Method::GET, Some(container), None).query(&query))
            .await?
            .text()
            .await
            .map_err(transport)?;
        parse_page(&text)
    }

    /// One page of every blob under `prefix`, however deep: the listing
    /// without a delimiter, which answers for a whole subtree.
    pub(crate) async fn list_flat(
        &self,
        container: &str,
        prefix: &str,
        marker: Option<&str>,
    ) -> Result<Page, Error> {
        let mut query: Vec<(&str, &str)> = vec![
            ("restype", "container"),
            ("comp", "list"),
            ("prefix", prefix),
            ("maxresults", PAGE),
        ];
        if let Some(m) = marker {
            query.push(("marker", m));
        }
        let text = self
            .send(Req::new(Method::GET, Some(container), None).query(&query))
            .await?
            .text()
            .await
            .map_err(transport)?;
        parse_page(&text)
    }

    /// Every blob under `prefix`, however deep.
    pub(crate) async fn list_all(&self, container: &str, prefix: &str) -> Result<Vec<Blob>, Error> {
        let mut out = Vec::new();
        let mut marker: Option<String> = None;
        loop {
            let page = self.list_flat(container, prefix, marker.as_deref()).await?;
            out.extend(page.blobs);
            match page.next {
                Some(next) => marker = Some(next),
                None => return Ok(out),
            }
        }
    }

    /// The blob's properties, with the state of a copy to it when one is
    /// or was under way.
    pub(crate) async fn head(
        &self,
        container: &str,
        blob: &str,
    ) -> Result<(Blob, Option<CopyState>), Error> {
        let response = self
            .send(Req::new(Method::HEAD, Some(container), Some(blob)))
            .await?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let size = header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let modified = header("last-modified").as_deref().and_then(parse_time);
        let copy = header("x-ms-copy-status").map(|status| CopyState {
            id: header("x-ms-copy-id").unwrap_or_default(),
            status,
            progress: header("x-ms-copy-progress")
                .as_deref()
                .and_then(parse_progress),
        });
        Ok((
            Blob {
                name: blob.to_owned(),
                size,
                modified,
            },
            copy,
        ))
    }

    /// Whether `container` exists.
    pub(crate) async fn head_container(&self, container: &str) -> Result<(), Error> {
        self.send(Req::new(Method::HEAD, Some(container), None).query(&[("restype", "container")]))
            .await
            .map(|_| ())
    }

    /// The blob's bytes, streamed.
    pub(crate) async fn get(
        &self,
        container: &str,
        blob: &str,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin>, Error> {
        use futures::TryStreamExt as _;
        let response = self
            .send(Req::new(Method::GET, Some(container), Some(blob)))
            .await?;
        let stream = response.bytes_stream().map_err(std::io::Error::other);
        Ok(Box::new(tokio_util::io::StreamReader::new(stream)))
    }

    /// Writes a whole block blob at once.
    pub(crate) async fn put(&self, container: &str, blob: &str, body: Bytes) -> Result<(), Error> {
        self.send(
            Req::new(Method::PUT, Some(container), Some(blob))
                .ms_headers(&[("x-ms-blob-type", "BlockBlob")])
                .body(body, "application/octet-stream"),
        )
        .await
        .map(|_| ())
    }

    /// Uploads one block of a blob, uncommitted until [`put_block_list`](Self::put_block_list).
    pub(crate) async fn put_block(
        &self,
        container: &str,
        blob: &str,
        block_id: &str,
        body: Bytes,
    ) -> Result<(), Error> {
        self.send(
            Req::new(Method::PUT, Some(container), Some(blob))
                .query(&[("comp", "block"), ("blockid", block_id)])
                .body(body, "application/octet-stream"),
        )
        .await
        .map(|_| ())
    }

    /// Commits the blocks, in order, as the blob.
    pub(crate) async fn put_block_list(
        &self,
        container: &str,
        blob: &str,
        block_ids: &[String],
    ) -> Result<(), Error> {
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?><BlockList>");
        for id in block_ids {
            xml.push_str("<Latest>");
            xml.push_str(id);
            xml.push_str("</Latest>");
        }
        xml.push_str("</BlockList>");
        self.send(
            Req::new(Method::PUT, Some(container), Some(blob))
                .query(&[("comp", "blocklist")])
                .body(Bytes::from(xml), "application/xml"),
        )
        .await
        .map(|_| ())
    }

    /// Creates a container.
    pub(crate) async fn create_container(&self, container: &str) -> Result<(), Error> {
        self.send(Req::new(Method::PUT, Some(container), None).query(&[("restype", "container")]))
            .await
            .map(|_| ())
    }

    /// Deletes a container and everything in it.
    pub(crate) async fn delete_container(&self, container: &str) -> Result<(), Error> {
        self.send(
            Req::new(Method::DELETE, Some(container), None).query(&[("restype", "container")]),
        )
        .await
        .map(|_| ())
    }

    /// Deletes one blob; one that is not there is no error.
    pub(crate) async fn delete(&self, container: &str, blob: &str) -> Result<(), Error> {
        match self
            .send(Req::new(Method::DELETE, Some(container), Some(blob)))
            .await
        {
            Ok(_) | Err(Error::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Starts a *Copy Blob* on the service and returns its state: done
    /// already (`success`) or `pending`, to be polled with [`head`](Self::head).
    pub(crate) async fn copy(
        &self,
        from_container: &str,
        from_blob: &str,
        to_container: &str,
        to_blob: &str,
    ) -> Result<CopyState, Error> {
        let source = self.blob_url(from_container, from_blob).to_string();
        let response = self
            .send(
                Req::new(Method::PUT, Some(to_container), Some(to_blob))
                    .ms_headers(&[("x-ms-copy-source", source.as_str())]),
            )
            .await?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        Ok(CopyState {
            id: header("x-ms-copy-id").unwrap_or_default(),
            status: header("x-ms-copy-status").unwrap_or_else(|| "success".to_owned()),
            progress: None,
        })
    }

    /// Aborts a pending copy; one that finished meanwhile is no error.
    pub(crate) async fn abort_copy(
        &self,
        container: &str,
        blob: &str,
        copy_id: &str,
    ) -> Result<(), Error> {
        match self
            .send(
                Req::new(Method::PUT, Some(container), Some(blob))
                    .query(&[("comp", "copy"), ("copyid", copy_id)])
                    .ms_headers(&[("x-ms-copy-action", "abort")]),
            )
            .await
        {
            Ok(_) | Err(Error::NotFound) => Ok(()),
            Err(Error::Extension { source, .. })
                if source.to_string().contains("NoPendingCopyOperation") =>
            {
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

/// `bytesCopied/bytesTotal`.
fn parse_progress(text: &str) -> Option<(u64, u64)> {
    let (done, total) = text.split_once('/')?;
    Some((done.trim().parse().ok()?, total.trim().parse().ok()?))
}

/// The containers of a *List Containers* answer, and the next marker.
pub(crate) fn parse_containers(text: &str) -> Result<(Vec<Container>, Option<String>), Error> {
    let doc = roxmltree::Document::parse(text)
        .map_err(|e| message(format!("List Containers answered no XML: {e}")))?;
    let containers = doc
        .descendants()
        .filter(|n| n.has_tag_name("Container"))
        .filter_map(|c| {
            let name = c
                .children()
                .find(|n| n.has_tag_name("Name"))
                .and_then(|n| n.text())?
                .to_owned();
            let modified = c
                .descendants()
                .find(|n| n.has_tag_name("Last-Modified"))
                .and_then(|n| n.text())
                .and_then(parse_time);
            Some(Container { name, modified })
        })
        .collect();
    Ok((containers, next_marker(&doc)))
}

fn next_marker(doc: &roxmltree::Document<'_>) -> Option<String> {
    doc.root_element()
        .children()
        .find(|n| n.has_tag_name("NextMarker"))
        .and_then(|n| n.text())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// One *List Blobs* answer.
pub(crate) fn parse_page(text: &str) -> Result<Page, Error> {
    let doc = roxmltree::Document::parse(text)
        .map_err(|e| message(format!("List Blobs answered no XML: {e}")))?;
    let mut page = Page::default();
    for node in doc.descendants() {
        if node.has_tag_name("BlobPrefix") {
            if let Some(name) = node
                .children()
                .find(|c| c.has_tag_name("Name"))
                .and_then(|c| c.text())
            {
                page.prefixes.push(name.to_owned());
            }
        } else if node.has_tag_name("Blob") {
            let name = node
                .children()
                .find(|c| c.has_tag_name("Name"))
                .and_then(|c| c.text());
            let property = |what: &str| {
                node.children()
                    .find(|c| c.has_tag_name("Properties"))
                    .and_then(|p| p.children().find(|c| c.has_tag_name(what)))
                    .and_then(|c| c.text())
            };
            if let Some(name) = name {
                page.blobs.push(Blob {
                    name: name.to_owned(),
                    size: property("Content-Length")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                    modified: property("Last-Modified").and_then(parse_time),
                });
            }
        }
    }
    page.next = next_marker(&doc);
    Ok(page)
}
