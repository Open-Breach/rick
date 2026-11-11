//! The S3 REST client: Signature Version 4, the handful of requests the
//! provider needs, and the XML they answer.

use std::time::SystemTime;

use bytes::Bytes;
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::{Method, StatusCode, Url};
use sha2::{Digest, Sha256};
use tokio::io::AsyncRead;
use tracing::debug;
use types::Error;

use crate::ID;

/// The characters S3 leaves as they are in a URI: the unreserved set.
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Like [`UNRESERVED`], keeping the `/` between key segments.
const PATH: &AsciiSet = &UNRESERVED.remove(b'/');

/// The SHA-256 of nothing, the payload hash of a request without a body.
const EMPTY_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// What a request with a streamed body declares instead of its hash.
const UNSIGNED: &str = "UNSIGNED-PAYLOAD";

/// How many keys one listing page asks for.
const PAGE: &str = "1000";

/// Installs ring as the process's rustls crypto provider, once; a second
/// plugin doing the same is told it is done already and that is fine.
pub(crate) fn install_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// What went wrong with a request, as the service said it.
#[derive(Debug)]
pub struct S3Error {
    /// The HTTP status.
    pub status: u16,
    /// The service's error code, `NoSuchKey`, when it sent one.
    pub code: String,
    /// The service's message.
    pub message: String,
}

impl std::fmt::Display for S3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.code.is_empty() {
            write!(f, "HTTP {}: {}", self.status, self.message)
        } else {
            write!(f, "{} ({}): {}", self.code, self.status, self.message)
        }
    }
}

impl std::error::Error for S3Error {}

/// The kernel error for a failed request: a missing key or bucket is
/// `NotFound`, a refusal `PermissionDenied`, the rest the service's words.
fn failed(status: StatusCode, body: &str) -> Error {
    let (code, message) = parse_error(body);
    match (status, code.as_str()) {
        (StatusCode::NOT_FOUND, _) | (_, "NoSuchKey" | "NoSuchBucket" | "NoSuchUpload") => {
            Error::NotFound
        }
        (StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED, _)
        | (_, "AccessDenied" | "InvalidAccessKeyId" | "SignatureDoesNotMatch") => {
            Error::PermissionDenied
        }
        _ => Error::Extension {
            ext: ID,
            source: Box::new(S3Error {
                status: status.as_u16(),
                code,
                message,
            }),
        },
    }
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
            .to_owned()
    };
    (text("Code"), text("Message"))
}

/// Percent-encodes `text` for a URI path, keeping the slashes.
#[must_use]
pub fn encode_path(text: &str) -> String {
    utf8_percent_encode(text, PATH).to_string()
}

/// Percent-encodes `text` for a query string or a single path segment.
#[must_use]
pub fn encode(text: &str) -> String {
    utf8_percent_encode(text, UNRESERVED).to_string()
}

fn hex_sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap_or_else(|_| unreachable!());
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// A request before signing.
pub struct Unsigned<'a> {
    /// `GET`, `PUT`, …
    pub method: &'a str,
    /// The host header: `127.0.0.1:9000`, `bucket.s3.amazonaws.com`.
    pub host: &'a str,
    /// The path, already percent-encoded: `/bucket/a%20b`.
    pub path: &'a str,
    /// The query parameters, raw; empty values allowed (`uploads`).
    pub query: &'a [(&'a str, &'a str)],
    /// Extra headers to sign, lower-case names, raw values.
    pub headers: &'a [(&'a str, &'a str)],
    /// The payload's SHA-256 in hex, or `UNSIGNED-PAYLOAD`.
    pub payload_hash: &'a str,
    /// The request time, `yyyymmddThhmmssZ`.
    pub amz_date: &'a str,
}

/// The signed headers of a request: `authorization`, `x-amz-date` and
/// `x-amz-content-sha256`, as `(name, value)` pairs.
#[must_use]
pub fn sign(
    req: &Unsigned<'_>,
    access_key: &str,
    secret_key: &str,
    region: &str,
) -> Vec<(String, String)> {
    let mut query: Vec<(String, String)> = req
        .query
        .iter()
        .map(|(k, v)| (encode(k), encode(v)))
        .collect();
    query.sort();
    let canonical_query = query
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let mut headers: Vec<(String, String)> = vec![
        ("host".to_owned(), req.host.to_owned()),
        (
            "x-amz-content-sha256".to_owned(),
            req.payload_hash.to_owned(),
        ),
        ("x-amz-date".to_owned(), req.amz_date.to_owned()),
    ];
    for (k, v) in req.headers {
        headers.push((k.to_lowercase(), v.trim().to_owned()));
    }
    headers.sort();
    let mut canonical_headers = String::new();
    for (k, v) in &headers {
        canonical_headers.push_str(k);
        canonical_headers.push(':');
        canonical_headers.push_str(v);
        canonical_headers.push('\n');
    }
    let signed_headers = headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical_request = format!(
        "{}\n{}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{}",
        req.method, req.path, req.payload_hash
    );
    let date = &req.amz_date[..8];
    let scope = format!("{date}/{region}/s3/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        req.amz_date,
        hex_sha256(canonical_request.as_bytes())
    );
    let k_date = hmac(format!("AWS4{secret_key}").as_bytes(), date);
    let k_region = hmac(&k_date, region);
    let k_service = hmac(&k_region, "s3");
    let k_signing = hmac(&k_service, "aws4_request");
    let signature = hex::encode(hmac(&k_signing, &string_to_sign));
    vec![
        (
            "authorization".to_owned(),
            format!(
                "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
            ),
        ),
        ("x-amz-date".to_owned(), req.amz_date.to_owned()),
        (
            "x-amz-content-sha256".to_owned(),
            req.payload_hash.to_owned(),
        ),
    ]
}

/// The current time as Signature Version 4 writes it.
fn amz_now() -> String {
    chrono::DateTime::<chrono::Utc>::from(SystemTime::now())
        .format("%Y%m%dT%H%M%SZ")
        .to_string()
}

/// A bucket of the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    /// Its name.
    pub name: String,
    /// When it was created, when the service says.
    pub created: Option<SystemTime>,
}

/// An object as a listing or a `HEAD` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// The full key.
    pub key: String,
    /// Its size in bytes.
    pub size: u64,
    /// When it was last written.
    pub modified: Option<SystemTime>,
}

/// One page of a listing with a delimiter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    /// The common prefixes: the folders, each ending in `/`.
    pub prefixes: Vec<String>,
    /// The objects at this level.
    pub objects: Vec<Object>,
    /// The token of the next page, when there is one.
    pub next: Option<String>,
}

/// One request before it is signed and sent.
struct Req<'a> {
    method: Method,
    bucket: Option<&'a str>,
    key: Option<&'a str>,
    query: &'a [(&'a str, &'a str)],
    headers: &'a [(&'a str, &'a str)],
    body: Option<reqwest::Body>,
    payload_hash: &'a str,
}

impl<'a> Req<'a> {
    /// A request without a body.
    fn new(method: Method, bucket: Option<&'a str>, key: Option<&'a str>) -> Self {
        Self {
            method,
            bucket,
            key,
            query: &[],
            headers: &[],
            body: None,
            payload_hash: EMPTY_HASH,
        }
    }

    fn query(mut self, query: &'a [(&'a str, &'a str)]) -> Self {
        self.query = query;
        self
    }

    fn headers(mut self, headers: &'a [(&'a str, &'a str)]) -> Self {
        self.headers = headers;
        self
    }

    /// A body with its hash, or `UNSIGNED-PAYLOAD` for a large one.
    fn body(mut self, body: Bytes, hash: &'a str) -> Self {
        self.body = Some(reqwest::Body::from(body));
        self.payload_hash = hash;
        self
    }
}

/// The client of one account.
pub struct S3Client {
    http: reqwest::Client,
    endpoint: Url,
    region: String,
    access_key: String,
    secret_key: String,
    path_style: bool,
}

impl std::fmt::Debug for S3Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Client")
            .field("endpoint", &self.endpoint.as_str())
            .field("region", &self.region)
            .field("path_style", &self.path_style)
            .finish_non_exhaustive()
    }
}

impl S3Client {
    /// A client for `endpoint` (scheme, host and port; the AWS endpoint of
    /// `region` when empty) with the given credentials.
    ///
    /// # Errors
    ///
    /// The plugin's error when the endpoint is not a URL or the HTTP
    /// client cannot be built.
    pub fn new(
        endpoint: &str,
        region: &str,
        access_key: &str,
        secret_key: &str,
        path_style: bool,
    ) -> Result<Self, Error> {
        let region = if region.is_empty() {
            "us-east-1".to_owned()
        } else {
            region.to_owned()
        };
        let endpoint = if endpoint.is_empty() {
            format!("https://s3.{region}.amazonaws.com")
        } else if endpoint.contains("://") {
            endpoint.to_owned()
        } else {
            format!("https://{endpoint}")
        };
        let endpoint =
            Url::parse(&endpoint).map_err(|e| message(format!("endpoint {endpoint}: {e}")))?;
        if endpoint.host_str().is_none() {
            return Err(message(format!("endpoint {endpoint} has no host")));
        }
        install_crypto();
        let http = reqwest::Client::builder().build().map_err(transport)?;
        Ok(Self {
            http,
            endpoint,
            region,
            access_key: access_key.to_owned(),
            secret_key: secret_key.to_owned(),
            path_style,
        })
    }

    /// The URL and the host header of a bucket, a key or the service
    /// itself.
    fn target(
        &self,
        bucket: Option<&str>,
        key: Option<&str>,
    ) -> Result<(Url, String, String), Error> {
        let mut url = self.endpoint.clone();
        let encoded_key = key.map(encode_path).unwrap_or_default();
        let path = match (bucket, self.path_style) {
            (None, _) => "/".to_owned(),
            (Some(bucket), true) => format!("/{bucket}/{encoded_key}"),
            (Some(bucket), false) => {
                let host = url.host_str().unwrap_or_default().to_owned();
                url.set_host(Some(&format!("{bucket}.{host}")))
                    .map_err(|e| message(format!("bucket {bucket} as a host name: {e}")))?;
                format!("/{encoded_key}")
            }
        };
        url.set_path(&path);
        let mut host = url.host_str().unwrap_or_default().to_owned();
        if let Some(port) = url.port() {
            host = format!("{host}:{port}");
        }
        Ok((url, host, path))
    }

    /// Sends one signed request and returns the response, or the error the
    /// service described.
    async fn send(&self, req: Req<'_>) -> Result<reqwest::Response, Error> {
        let Req {
            method,
            bucket,
            key,
            query,
            headers,
            body,
            payload_hash,
        } = req;
        let (mut url, host, path) = self.target(bucket, key)?;
        if !query.is_empty() {
            let text = {
                let mut sorted: Vec<(String, String)> =
                    query.iter().map(|(k, v)| (encode(k), encode(v))).collect();
                sorted.sort();
                sorted
                    .iter()
                    .map(|(k, v)| {
                        if v.is_empty() {
                            k.clone()
                        } else {
                            format!("{k}={v}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("&")
            };
            url.set_query(Some(&text));
        }
        let amz_date = amz_now();
        let signed = sign(
            &Unsigned {
                method: method.as_str(),
                host: &host,
                path: &path,
                query,
                headers,
                payload_hash,
                amz_date: &amz_date,
            },
            &self.access_key,
            &self.secret_key,
            &self.region,
        );
        let mut request = self.http.request(method.clone(), url.clone());
        for (k, v) in headers {
            request = request.header(*k, *v);
        }
        for (k, v) in &signed {
            request = request.header(k.as_str(), v.as_str());
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        debug!(%method, %url, "s3 request");
        let response = request.send().await.map_err(transport)?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        Err(failed(status, &body))
    }

    /// The account's buckets.
    pub(crate) async fn list_buckets(&self) -> Result<Vec<Bucket>, Error> {
        let text = self
            .send(Req::new(Method::GET, None, None))
            .await?
            .text()
            .await
            .map_err(transport)?;
        parse_buckets(&text)
    }

    /// One page of the keys under `prefix` with `/` as the delimiter.
    pub(crate) async fn list(
        &self,
        bucket: &str,
        prefix: &str,
        continuation: Option<&str>,
        max_keys: Option<&str>,
    ) -> Result<Page, Error> {
        let mut query: Vec<(&str, &str)> = vec![
            ("list-type", "2"),
            ("delimiter", "/"),
            ("prefix", prefix),
            ("max-keys", max_keys.unwrap_or(PAGE)),
        ];
        if let Some(token) = continuation {
            query.push(("continuation-token", token));
        }
        let text = self
            .send(Req::new(Method::GET, Some(bucket), None).query(&query))
            .await?
            .text()
            .await
            .map_err(transport)?;
        parse_page(&text)
    }

    /// One page of every object under `prefix`, however deep: the listing
    /// without a delimiter, which answers for a whole subtree.
    pub(crate) async fn list_flat(
        &self,
        bucket: &str,
        prefix: &str,
        continuation: Option<&str>,
    ) -> Result<Page, Error> {
        let mut query: Vec<(&str, &str)> =
            vec![("list-type", "2"), ("prefix", prefix), ("max-keys", PAGE)];
        if let Some(token) = continuation {
            query.push(("continuation-token", token));
        }
        let text = self
            .send(Req::new(Method::GET, Some(bucket), None).query(&query))
            .await?
            .text()
            .await
            .map_err(transport)?;
        parse_page(&text)
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

    /// The object `key`, by its headers.
    pub(crate) async fn head(&self, bucket: &str, key: &str) -> Result<Object, Error> {
        let response = self
            .send(Req::new(Method::HEAD, Some(bucket), Some(key)))
            .await?;
        let size = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let modified = response
            .headers()
            .get(reqwest::header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| chrono::DateTime::parse_from_rfc2822(v).ok())
            .map(SystemTime::from);
        Ok(Object {
            key: key.to_owned(),
            size,
            modified,
        })
    }

    /// Whether `bucket` exists and the credentials may see it.
    pub(crate) async fn head_bucket(&self, bucket: &str) -> Result<(), Error> {
        self.send(Req::new(Method::HEAD, Some(bucket), None))
            .await
            .map(|_| ())
    }

    /// The object's bytes, streamed.
    pub(crate) async fn get(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin>, Error> {
        use futures::TryStreamExt as _;
        let response = self
            .send(Req::new(Method::GET, Some(bucket), Some(key)))
            .await?;
        let stream = response.bytes_stream().map_err(std::io::Error::other);
        Ok(Box::new(tokio_util::io::StreamReader::new(stream)))
    }

    /// Writes a whole object at once.
    pub(crate) async fn put(&self, bucket: &str, key: &str, body: Bytes) -> Result<(), Error> {
        let hash = hex_sha256(&body);
        let length = body.len().to_string();
        self.send(
            Req::new(Method::PUT, Some(bucket), Some(key))
                .headers(&[("content-length", length.as_str())])
                .body(body, &hash),
        )
        .await
        .map(|_| ())
    }

    /// Creates a bucket.
    pub(crate) async fn create_bucket(&self, bucket: &str) -> Result<(), Error> {
        let body = if self.region == "us-east-1" {
            Bytes::new()
        } else {
            Bytes::from(format!(
                "<CreateBucketConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><LocationConstraint>{}</LocationConstraint></CreateBucketConfiguration>",
                self.region
            ))
        };
        let hash = hex_sha256(&body);
        let length = body.len().to_string();
        self.send(
            Req::new(Method::PUT, Some(bucket), None)
                .headers(&[("content-length", length.as_str())])
                .body(body, &hash),
        )
        .await
        .map(|_| ())
    }

    /// Deletes an empty bucket.
    pub(crate) async fn delete_bucket(&self, bucket: &str) -> Result<(), Error> {
        self.send(Req::new(Method::DELETE, Some(bucket), None))
            .await
            .map(|_| ())
    }

    /// Deletes one object; a key that is not there is no error, as the
    /// service has it.
    pub(crate) async fn delete(&self, bucket: &str, key: &str) -> Result<(), Error> {
        match self
            .send(Req::new(Method::DELETE, Some(bucket), Some(key)))
            .await
        {
            Ok(_) | Err(Error::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Copies an object on the service, within or across buckets of the
    /// account.
    pub(crate) async fn copy(
        &self,
        from_bucket: &str,
        from_key: &str,
        to_bucket: &str,
        to_key: &str,
    ) -> Result<(), Error> {
        let source = format!("/{from_bucket}/{}", encode_path(from_key));
        self.send(
            Req::new(Method::PUT, Some(to_bucket), Some(to_key))
                .headers(&[("x-amz-copy-source", source.as_str())]),
        )
        .await
        .map(|_| ())
    }

    /// Starts a multipart upload and returns its id.
    pub(crate) async fn create_multipart(&self, bucket: &str, key: &str) -> Result<String, Error> {
        let text = self
            .send(Req::new(Method::POST, Some(bucket), Some(key)).query(&[("uploads", "")]))
            .await?
            .text()
            .await
            .map_err(transport)?;
        let doc = roxmltree::Document::parse(&text)
            .map_err(|e| message(format!("InitiateMultipartUpload answered no XML: {e}")))?;
        doc.descendants()
            .find(|n| n.has_tag_name("UploadId"))
            .and_then(|n| n.text())
            .map(str::to_owned)
            .ok_or_else(|| message("InitiateMultipartUpload answered without an UploadId"))
    }

    /// Uploads one part and returns its `ETag`.
    pub(crate) async fn upload_part(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        number: u32,
        body: Bytes,
    ) -> Result<String, Error> {
        let number = number.to_string();
        let length = body.len().to_string();
        let response = self
            .send(
                Req::new(Method::PUT, Some(bucket), Some(key))
                    .query(&[("partNumber", number.as_str()), ("uploadId", upload_id)])
                    .headers(&[("content-length", length.as_str())])
                    .body(body, UNSIGNED),
            )
            .await?;
        response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .ok_or_else(|| message("UploadPart answered without an ETag"))
    }

    /// Completes a multipart upload from its parts' numbers and `ETag`s.
    pub(crate) async fn complete_multipart(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        parts: &[(u32, String)],
    ) -> Result<(), Error> {
        use std::fmt::Write as _;
        let mut xml = String::from("<CompleteMultipartUpload>");
        for (number, etag) in parts {
            let _ = write!(
                xml,
                "<Part><PartNumber>{number}</PartNumber><ETag>{}</ETag></Part>",
                etag.replace('"', "&quot;")
            );
        }
        xml.push_str("</CompleteMultipartUpload>");
        let body = Bytes::from(xml);
        let hash = hex_sha256(&body);
        let length = body.len().to_string();
        let text = self
            .send(
                Req::new(Method::POST, Some(bucket), Some(key))
                    .query(&[("uploadId", upload_id)])
                    .headers(&[("content-length", length.as_str())])
                    .body(body, &hash),
            )
            .await?
            .text()
            .await
            .map_err(transport)?;
        // A 200 may still carry an error document.
        if text.contains("<Error>") {
            let (code, msg) = parse_error(&text);
            return Err(message(format!("CompleteMultipartUpload: {code}: {msg}")));
        }
        Ok(())
    }

    /// Abandons a multipart upload, freeing its parts.
    pub(crate) async fn abort_multipart(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
    ) -> Result<(), Error> {
        match self
            .send(
                Req::new(Method::DELETE, Some(bucket), Some(key)).query(&[("uploadId", upload_id)]),
            )
            .await
        {
            Ok(_) | Err(Error::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

fn parse_time(text: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(SystemTime::from)
}

/// The buckets of a `ListAllMyBuckets` answer.
pub(crate) fn parse_buckets(text: &str) -> Result<Vec<Bucket>, Error> {
    let doc = roxmltree::Document::parse(text)
        .map_err(|e| message(format!("ListBuckets answered no XML: {e}")))?;
    Ok(doc
        .descendants()
        .filter(|n| n.has_tag_name("Bucket"))
        .filter_map(|bucket| {
            let name = bucket
                .children()
                .find(|c| c.has_tag_name("Name"))
                .and_then(|c| c.text())?
                .to_owned();
            let created = bucket
                .children()
                .find(|c| c.has_tag_name("CreationDate"))
                .and_then(|c| c.text())
                .and_then(parse_time);
            Some(Bucket { name, created })
        })
        .collect())
}

/// One `ListObjectsV2` answer.
pub(crate) fn parse_page(text: &str) -> Result<Page, Error> {
    let doc = roxmltree::Document::parse(text)
        .map_err(|e| message(format!("ListObjectsV2 answered no XML: {e}")))?;
    let root = doc.root_element();
    let mut page = Page::default();
    for node in root.children() {
        if node.has_tag_name("CommonPrefixes") {
            if let Some(prefix) = node
                .children()
                .find(|c| c.has_tag_name("Prefix"))
                .and_then(|c| c.text())
            {
                page.prefixes.push(prefix.to_owned());
            }
        } else if node.has_tag_name("Contents") {
            let text_of = |name: &str| {
                node.children()
                    .find(|c| c.has_tag_name(name))
                    .and_then(|c| c.text())
            };
            if let Some(key) = text_of("Key") {
                page.objects.push(Object {
                    key: key.to_owned(),
                    size: text_of("Size").and_then(|s| s.parse().ok()).unwrap_or(0),
                    modified: text_of("LastModified").and_then(parse_time),
                });
            }
        } else if node.has_tag_name("NextContinuationToken") {
            page.next = node.text().map(str::to_owned);
        }
    }
    Ok(page)
}
