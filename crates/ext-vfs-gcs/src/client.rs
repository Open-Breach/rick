//! The JSON API client: the requests the provider needs and the JSON they
//! answer, with a bearer token from the service account when there is one.

use std::time::SystemTime;

use bytes::Bytes;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::{Method, StatusCode, Url};
use serde::Deserialize;
use tokio::io::AsyncRead;
use tracing::debug;
use types::Error;

use crate::ID;
use crate::auth::{ServiceAccount, Token, fetch_token};

/// The characters kept in a path segment: an object name is one segment,
/// its slashes encoded.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// How many objects one listing page asks for.
const PAGE: &str = "1000";

/// Installs ring as the process's rustls crypto provider, once; a second
/// plugin doing the same is told it is done already and that is fine.
pub(crate) fn install_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// What went wrong with a request, as the service said it.
#[derive(Debug)]
pub struct GcsError {
    /// The HTTP status.
    pub status: u16,
    /// The service's message.
    pub message: String,
}

impl std::fmt::Display for GcsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}: {}", self.status, self.message)
    }
}

impl std::error::Error for GcsError {}

/// The kernel error for a failed request.
fn failed(status: StatusCode, body: &str) -> Error {
    match status {
        StatusCode::NOT_FOUND => Error::NotFound,
        StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED => Error::PermissionDenied,
        _ => Error::Extension {
            ext: ID,
            source: Box::new(GcsError {
                status: status.as_u16(),
                message: error_message(body),
            }),
        },
    }
}

/// `{"error": {"message": …}}`, or the body's first line.
fn error_message(body: &str) -> String {
    #[derive(Deserialize)]
    struct Envelope {
        error: Inner,
    }
    #[derive(Deserialize)]
    struct Inner {
        message: String,
    }
    serde_json::from_str::<Envelope>(body).map_or_else(
        |_| body.trim().chars().take(200).collect(),
        |e| e.error.message,
    )
}

/// A transport error as the kernel error.
pub(crate) fn transport(error: reqwest::Error) -> Error {
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

/// Percent-encodes an object name as one path segment.
#[must_use]
pub fn encode(text: &str) -> String {
    utf8_percent_encode(text, SEGMENT).to_string()
}

fn parse_time(text: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(SystemTime::from)
}

/// A bucket of the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    /// Its name.
    pub name: String,
    /// When it was created, when the service says.
    pub created: Option<SystemTime>,
}

/// An object as a listing or its metadata describe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
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
    /// The prefixes: the folders, each ending in `/`.
    pub prefixes: Vec<String>,
    /// The objects at this level.
    pub objects: Vec<Object>,
    /// The token of the next page, when there is one.
    pub next: Option<String>,
}

/// One step of a `Rewrite`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    /// Whether the object is complete at the target.
    pub done: bool,
    /// Bytes rewritten so far.
    pub written: u64,
    /// The object's size.
    pub size: u64,
    /// The token for the next step while not done.
    pub token: Option<String>,
}

#[derive(Deserialize)]
struct ObjectJson {
    name: String,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    updated: Option<String>,
}

impl From<ObjectJson> for Object {
    fn from(o: ObjectJson) -> Self {
        Self {
            name: o.name,
            size: o.size.and_then(|s| s.parse().ok()).unwrap_or(0),
            modified: o.updated.as_deref().and_then(parse_time),
        }
    }
}

#[derive(Deserialize)]
struct ListJson {
    #[serde(default)]
    prefixes: Vec<String>,
    #[serde(default)]
    items: Vec<ObjectJson>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct BucketJson {
    name: String,
    #[serde(default, rename = "timeCreated")]
    time_created: Option<String>,
}

#[derive(Deserialize)]
struct BucketsJson {
    #[serde(default)]
    items: Vec<BucketJson>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct RewriteJson {
    done: bool,
    #[serde(default, rename = "totalBytesRewritten")]
    total_bytes_rewritten: Option<String>,
    #[serde(default, rename = "objectSize")]
    object_size: Option<String>,
    #[serde(default, rename = "rewriteToken")]
    rewrite_token: Option<String>,
}

/// The buckets of one `buckets.list` page.
pub(crate) fn parse_buckets(json: &str) -> Result<(Vec<Bucket>, Option<String>), Error> {
    let parsed: BucketsJson = serde_json::from_str(json)
        .map_err(|e| message(format!("buckets.list answered no JSON: {e}")))?;
    Ok((
        parsed
            .items
            .into_iter()
            .map(|b| Bucket {
                name: b.name,
                created: b.time_created.as_deref().and_then(parse_time),
            })
            .collect(),
        parsed.next_page_token,
    ))
}

/// One `objects.list` page.
pub(crate) fn parse_page(json: &str) -> Result<Page, Error> {
    let parsed: ListJson = serde_json::from_str(json)
        .map_err(|e| message(format!("objects.list answered no JSON: {e}")))?;
    Ok(Page {
        prefixes: parsed.prefixes,
        objects: parsed.items.into_iter().map(Object::from).collect(),
        next: parsed.next_page_token,
    })
}

/// One `objects.rewrite` answer.
pub(crate) fn parse_rewrite(json: &str) -> Result<Rewrite, Error> {
    let parsed: RewriteJson = serde_json::from_str(json)
        .map_err(|e| message(format!("objects.rewrite answered no JSON: {e}")))?;
    Ok(Rewrite {
        done: parsed.done,
        written: parsed
            .total_bytes_rewritten
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        size: parsed.object_size.and_then(|s| s.parse().ok()).unwrap_or(0),
        token: parsed.rewrite_token.filter(|t| !t.is_empty()),
    })
}

/// The client of one project.
pub struct GcsClient {
    http: reqwest::Client,
    /// `https://storage.googleapis.com/` or an emulator.
    base: Url,
    project: String,
    account: Option<ServiceAccount>,
    token: tokio::sync::Mutex<Option<Token>>,
}

impl std::fmt::Debug for GcsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcsClient")
            .field("base", &self.base.as_str())
            .field("project", &self.project)
            .field("authenticated", &self.account.is_some())
            .finish_non_exhaustive()
    }
}

impl GcsClient {
    /// A client for `project`, at `endpoint` (an emulator; the service
    /// when empty), as `account` when there is one.
    ///
    /// # Errors
    ///
    /// The plugin's error when the endpoint is not a URL or the HTTP
    /// client cannot be built.
    pub fn new(
        project: &str,
        endpoint: &str,
        account: Option<ServiceAccount>,
    ) -> Result<Self, Error> {
        let base = if endpoint.is_empty() {
            "https://storage.googleapis.com/".to_owned()
        } else if endpoint.contains("://") {
            format!("{}/", endpoint.trim_end_matches('/'))
        } else {
            format!("https://{}/", endpoint.trim_end_matches('/'))
        };
        let base = Url::parse(&base).map_err(|e| message(format!("endpoint {base}: {e}")))?;
        install_crypto();
        let http = reqwest::Client::builder().build().map_err(transport)?;
        Ok(Self {
            http,
            base,
            project: project.to_owned(),
            account,
            token: tokio::sync::Mutex::new(None),
        })
    }

    /// The bearer token, renewed when it is about to run out; `None`
    /// without a service account.
    async fn bearer(&self) -> Result<Option<String>, Error> {
        let Some(account) = &self.account else {
            return Ok(None);
        };
        let mut slot = self.token.lock().await;
        let now = SystemTime::now();
        if let Some(token) = slot.as_ref()
            && token.is_fresh(now)
        {
            return Ok(Some(token.access_token.clone()));
        }
        let token = fetch_token(&self.http, account).await?;
        let access = token.access_token.clone();
        *slot = Some(token);
        Ok(Some(access))
    }

    fn url(&self, path: &str, query: &[(&str, &str)]) -> Url {
        let mut url = self.base.clone();
        url.set_path(&format!("{}{path}", self.base.path().trim_end_matches('/')));
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in query {
                pairs.append_pair(k, v);
            }
        }
        url
    }

    /// Sends one request with the bearer token and returns the response,
    /// or the error the service described.
    async fn send(
        &self,
        method: Method,
        url: Url,
        headers: &[(&str, &str)],
        body: Option<Bytes>,
    ) -> Result<reqwest::Response, Error> {
        let mut request = self.http.request(method.clone(), url.clone());
        if let Some(token) = self.bearer().await? {
            request = request.bearer_auth(token);
        }
        for (k, v) in headers {
            request = request.header(*k, *v);
        }
        if let Some(body) = body {
            request = request
                .header("content-length", body.len().to_string())
                .body(body);
        }
        debug!(%method, %url, "gcs request");
        let response = request.send().await.map_err(transport)?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        Err(failed(status, &body))
    }

    async fn text(
        &self,
        method: Method,
        url: Url,
        headers: &[(&str, &str)],
        body: Option<Bytes>,
    ) -> Result<String, Error> {
        self.send(method, url, headers, body)
            .await?
            .text()
            .await
            .map_err(transport)
    }

    /// The project's buckets.
    pub(crate) async fn list_buckets(&self) -> Result<Vec<Bucket>, Error> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query: Vec<(&str, &str)> = vec![("project", &self.project)];
            if let Some(t) = token.as_deref() {
                query.push(("pageToken", t));
            }
            let json = self
                .text(Method::GET, self.url("/storage/v1/b", &query), &[], None)
                .await?;
            let (buckets, next) = parse_buckets(&json)?;
            out.extend(buckets);
            match next {
                Some(next) => token = Some(next),
                None => return Ok(out),
            }
        }
    }

    /// Whether `bucket` exists and may be seen.
    pub(crate) async fn get_bucket(&self, bucket: &str) -> Result<(), Error> {
        self.send(
            Method::GET,
            self.url(&format!("/storage/v1/b/{}", encode(bucket)), &[]),
            &[],
            None,
        )
        .await
        .map(|_| ())
    }

    /// Creates a bucket in the project.
    pub(crate) async fn create_bucket(&self, bucket: &str) -> Result<(), Error> {
        let body = serde_json::json!({ "name": bucket }).to_string();
        self.send(
            Method::POST,
            self.url("/storage/v1/b", &[("project", &self.project)]),
            &[("content-type", "application/json")],
            Some(Bytes::from(body)),
        )
        .await
        .map(|_| ())
    }

    /// Deletes an empty bucket.
    pub(crate) async fn delete_bucket(&self, bucket: &str) -> Result<(), Error> {
        self.send(
            Method::DELETE,
            self.url(&format!("/storage/v1/b/{}", encode(bucket)), &[]),
            &[],
            None,
        )
        .await
        .map(|_| ())
    }

    /// One page of the objects under `prefix` with `/` as the delimiter.
    pub(crate) async fn list(
        &self,
        bucket: &str,
        prefix: &str,
        page_token: Option<&str>,
        max: Option<&str>,
    ) -> Result<Page, Error> {
        let mut query: Vec<(&str, &str)> = vec![
            ("delimiter", "/"),
            ("prefix", prefix),
            ("maxResults", max.unwrap_or(PAGE)),
        ];
        if let Some(t) = page_token {
            query.push(("pageToken", t));
        }
        let json = self
            .text(
                Method::GET,
                self.url(&format!("/storage/v1/b/{}/o", encode(bucket)), &query),
                &[],
                None,
            )
            .await?;
        parse_page(&json)
    }

    /// One page of every object under `prefix`, however deep: the listing
    /// without a delimiter, which answers for a whole subtree.
    pub(crate) async fn list_flat(
        &self,
        bucket: &str,
        prefix: &str,
        page_token: Option<&str>,
    ) -> Result<Page, Error> {
        let mut query: Vec<(&str, &str)> = vec![("prefix", prefix), ("maxResults", PAGE)];
        if let Some(t) = page_token {
            query.push(("pageToken", t));
        }
        let json = self
            .text(
                Method::GET,
                self.url(&format!("/storage/v1/b/{}/o", encode(bucket)), &query),
                &[],
                None,
            )
            .await?;
        parse_page(&json)
    }

    /// Every object under `prefix`, however deep.
    pub(crate) async fn list_all(&self, bucket: &str, prefix: &str) -> Result<Vec<Object>, Error> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self.list_flat(bucket, prefix, token.as_deref()).await?;
            out.extend(page.objects);
            match page.next {
                Some(next) => token = Some(next),
                None => return Ok(out),
            }
        }
    }

    /// The object's metadata.
    pub(crate) async fn get(&self, bucket: &str, name: &str) -> Result<Object, Error> {
        let json = self
            .text(
                Method::GET,
                self.url(
                    &format!("/storage/v1/b/{}/o/{}", encode(bucket), encode(name)),
                    &[],
                ),
                &[],
                None,
            )
            .await?;
        let object: ObjectJson = serde_json::from_str(&json)
            .map_err(|e| message(format!("objects.get answered no JSON: {e}")))?;
        Ok(object.into())
    }

    /// The object's bytes, streamed.
    pub(crate) async fn download(
        &self,
        bucket: &str,
        name: &str,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin>, Error> {
        use futures::TryStreamExt as _;
        let response = self
            .send(
                Method::GET,
                self.url(
                    &format!(
                        "/download/storage/v1/b/{}/o/{}",
                        encode(bucket),
                        encode(name)
                    ),
                    &[("alt", "media")],
                ),
                &[],
                None,
            )
            .await?;
        let stream = response.bytes_stream().map_err(std::io::Error::other);
        Ok(Box::new(tokio_util::io::StreamReader::new(stream)))
    }

    /// Writes a whole object at once.
    pub(crate) async fn upload(&self, bucket: &str, name: &str, body: Bytes) -> Result<(), Error> {
        self.send(
            Method::POST,
            self.url(
                &format!("/upload/storage/v1/b/{}/o", encode(bucket)),
                &[("uploadType", "media"), ("name", name)],
            ),
            &[("content-type", "application/octet-stream")],
            Some(body),
        )
        .await
        .map(|_| ())
    }

    /// Starts a resumable upload and returns its session URL.
    pub(crate) async fn start_resumable(&self, bucket: &str, name: &str) -> Result<Url, Error> {
        let body = serde_json::json!({ "name": name }).to_string();
        let response = self
            .send(
                Method::POST,
                self.url(
                    &format!("/upload/storage/v1/b/{}/o", encode(bucket)),
                    &[("uploadType", "resumable")],
                ),
                &[
                    ("content-type", "application/json; charset=UTF-8"),
                    ("x-upload-content-type", "application/octet-stream"),
                ],
                Some(Bytes::from(body)),
            )
            .await?;
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| message("the resumable upload answered without a session URL"))?;
        Url::parse(location)
            .or_else(|_| self.base.join(location))
            .map_err(|e| message(format!("session URL {location}: {e}")))
    }

    /// Sends one chunk of a resumable upload: bytes `offset..offset+len`
    /// of the object, whose total is `total` once the last chunk goes.
    pub(crate) async fn upload_chunk(
        &self,
        session: &Url,
        offset: u64,
        body: Bytes,
        total: Option<u64>,
    ) -> Result<(), Error> {
        let end = offset + body.len() as u64;
        let range = match total {
            Some(total) if body.is_empty() => format!("bytes */{total}"),
            Some(total) => format!("bytes {offset}-{}/{total}", end - 1),
            None => format!("bytes {offset}-{}/*", end - 1),
        };
        let mut request = self
            .http
            .put(session.clone())
            .header("content-range", range)
            .header("content-length", body.len().to_string())
            .body(body);
        if let Some(token) = self.bearer().await? {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(transport)?;
        let status = response.status();
        // 308 says "so far, so good"; the last chunk gets 200 or 201.
        if status.is_success() || status.as_u16() == 308 {
            return Ok(());
        }
        let text = response.text().await.unwrap_or_default();
        Err(failed(status, &text))
    }

    /// Abandons a resumable upload; a service without the call is no
    /// error, since an unfinished session holds no object.
    pub(crate) async fn abandon_resumable(&self, session: &Url) {
        let mut request = self
            .http
            .delete(session.clone())
            .header("content-length", "0");
        if let Ok(Some(token)) = self.bearer().await {
            request = request.bearer_auth(token);
        }
        let _ = request.send().await;
    }

    /// Deletes one object; one that is not there is no error.
    pub(crate) async fn delete(&self, bucket: &str, name: &str) -> Result<(), Error> {
        match self
            .send(
                Method::DELETE,
                self.url(
                    &format!("/storage/v1/b/{}/o/{}", encode(bucket), encode(name)),
                    &[],
                ),
                &[],
                None,
            )
            .await
        {
            Ok(_) | Err(Error::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// One step of a `Rewrite` from one object to another, continued with
    /// the token of the step before.
    pub(crate) async fn rewrite(
        &self,
        from_bucket: &str,
        from_name: &str,
        to_bucket: &str,
        to_name: &str,
        token: Option<&str>,
    ) -> Result<Rewrite, Error> {
        let path = format!(
            "/storage/v1/b/{}/o/{}/rewriteTo/b/{}/o/{}",
            encode(from_bucket),
            encode(from_name),
            encode(to_bucket),
            encode(to_name)
        );
        let query: Vec<(&str, &str)> = token.map(|t| ("rewriteToken", t)).into_iter().collect();
        let json = self
            .text(
                Method::POST,
                self.url(&path, &query),
                &[("content-type", "application/json")],
                Some(Bytes::from_static(b"{}")),
            )
            .await?;
        parse_rewrite(&json)
    }
}
