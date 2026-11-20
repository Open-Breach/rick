//! What a test needs to stand in for the registry: a key pair made on the
//! spot that signs as minisign does, and a server on this machine that
//! serves what it is given and remembers what it was asked.
//!
//! No test reaches the real registry. The public key goes into
//! `[registry] public_key` and the server's address into `[registry]
//! url`, which is what those two settings are for.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use blake2::{Blake2b512, Digest as _};
use ring::signature::{Ed25519KeyPair, KeyPair as _};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// The path of the index on the [`Server`], as on the real registry.
pub const INDEX_PATH: &str = "registry/index.json";

/// The two bytes that open a minisign public key: Ed25519.
const KEY_ALGORITHM: &[u8; 2] = b"Ed";
/// The two bytes that open a signature of the file's `BLAKE2b` hash, the
/// only kind minisign makes today.
const HASHED_ALGORITHM: &[u8; 2] = b"ED";

/// A minisign key pair of a test's own.
pub struct Signer {
    pair: Ed25519KeyPair,
    /// The eight bytes by which a signature names its key.
    id: [u8; 8],
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Signer {
    /// The key pair of `seed`: the same pair for the same seed, another
    /// one for another.
    ///
    /// # Panics
    ///
    /// Never: 32 bytes are an Ed25519 seed.
    #[must_use]
    pub fn new(seed: u8) -> Self {
        let pair = Ed25519KeyPair::from_seed_unchecked(&[seed; 32])
            .unwrap_or_else(|error| panic!("an Ed25519 seed: {error}"));
        Self {
            pair,
            id: [seed; 8],
        }
    }

    /// The public key as `[registry] public_key` and a `.pub` file's
    /// second line have it.
    #[must_use]
    pub fn public_key(&self) -> String {
        let mut key = KEY_ALGORITHM.to_vec();
        key.extend_from_slice(&self.id);
        key.extend_from_slice(self.pair.public_key().as_ref());
        STANDARD.encode(key)
    }

    /// The text of the `.minisig` file for `bytes`: the signature of
    /// their `BLAKE2b` hash, and the signature of that and the trusted
    /// comment.
    #[must_use]
    pub fn sign(&self, bytes: &[u8]) -> String {
        let trusted = "timestamp:0\tfile:index.json\thashed";
        let signature = self.pair.sign(&Blake2b512::digest(bytes));
        let mut first = HASHED_ALGORITHM.to_vec();
        first.extend_from_slice(&self.id);
        first.extend_from_slice(signature.as_ref());
        let mut global = signature.as_ref().to_vec();
        global.extend_from_slice(trusted.as_bytes());
        format!(
            "untrusted comment: signature from a test's key\n{}\ntrusted comment: {trusted}\n{}\n",
            STANDARD.encode(first),
            STANDARD.encode(self.pair.sign(&global).as_ref()),
        )
    }
}

/// One request the [`Server`] received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The path asked for, without the leading slash.
    pub path: String,
    /// The headers in the order they were sent, the names in lower case.
    pub headers: Vec<(String, String)>,
}

impl Request {
    /// The value of the header `name` (lower case), when it was sent.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }
}

/// What the [`Server`] answers a path with.
#[derive(Debug, Clone)]
enum Served {
    /// These bytes.
    Bytes(Vec<u8>),
    /// A 302 to this address.
    Elsewhere(String),
}

type Files = Arc<Mutex<BTreeMap<String, Served>>>;
type Requests = Arc<Mutex<Vec<Request>>>;

/// An HTTP server on this machine: it answers a GET for a path it was
/// given with those bytes, or with the redirect it was given for it, and
/// any other with 404, and counts the lot. Once it has [fallen
/// silent](Server::fall_silent) it answers nothing at all.
#[derive(Debug)]
pub struct Server {
    address: SocketAddr,
    files: Files,
    requests: Requests,
    connections: Arc<AtomicUsize>,
    silent: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    /// Starts a server on a free port of the loopback interface, on
    /// Rick's runtime.
    ///
    /// # Panics
    ///
    /// When no port can be bound.
    #[must_use]
    pub fn start() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| {
                listener.set_nonblocking(true)?;
                Ok(listener)
            })
            .unwrap_or_else(|error| panic!("a port for the test registry: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("the test registry's address: {error}"));
        let files = Files::default();
        let requests = Requests::default();
        let connections = Arc::new(AtomicUsize::new(0));
        let silent = Arc::new(AtomicBool::new(false));
        let handle = runtime::handle();
        let task = {
            let (files, requests) = (Arc::clone(&files), Arc::clone(&requests));
            let (connections, silent) = (Arc::clone(&connections), Arc::clone(&silent));
            let _context = handle.enter();
            let listener = tokio::net::TcpListener::from_std(listener)
                .unwrap_or_else(|error| panic!("the test registry's listener: {error}"));
            handle.spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    connections.fetch_add(1, Ordering::SeqCst);
                    if silent.load(Ordering::SeqCst) {
                        // Taken and closed, without a word.
                        continue;
                    }
                    let (files, requests) = (Arc::clone(&files), Arc::clone(&requests));
                    runtime::handle().spawn(answer(stream, files, requests));
                }
            })
        };
        Self {
            address,
            files,
            requests,
            connections,
            silent,
            task,
        }
    }

    /// The address the server listens on: a loopback address and its
    /// port.
    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The address of `path` on this server.
    #[must_use]
    pub fn url(&self, path: &str) -> String {
        format!("http://{}/{}", self.address, path.trim_start_matches('/'))
    }

    /// The address of the index: what `[registry] url` is set to.
    #[must_use]
    pub fn index_url(&self) -> String {
        self.url(INDEX_PATH)
    }

    /// Serves `bytes` at `path` from now on.
    pub fn put(&self, path: &str, bytes: impl Into<Vec<u8>>) {
        lock(&self.files).insert(
            path.trim_start_matches('/').to_owned(),
            Served::Bytes(bytes.into()),
        );
    }

    /// Answers `path` with a redirect to the address `to` from now on: a
    /// 302 with `to` as its `Location`, written as given.
    pub fn redirect(&self, path: &str, to: &str) {
        lock(&self.files).insert(
            path.trim_start_matches('/').to_owned(),
            Served::Elsewhere(to.to_owned()),
        );
    }

    /// Serves `index`, written as `to_string` writes it, at
    /// [`INDEX_PATH`] with `signer`'s signature beside it, as the
    /// publisher does.
    pub fn publish(&self, signer: &Signer, index: &serde_json::Value) {
        let bytes = index.to_string().into_bytes();
        self.put(&format!("{INDEX_PATH}.minisig"), signer.sign(&bytes));
        self.put(INDEX_PATH, bytes);
    }

    /// Every request so far, in the order they arrived.
    #[must_use]
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.requests).clone()
    }

    /// How many connections were opened to the server so far, whether or
    /// not a request came over them. They are counted in the order they
    /// were opened, so one opened before another that is counted is
    /// counted too.
    #[must_use]
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    /// Says nothing from now on: a connection is taken and closed again
    /// without an answer, as by a machine that is there and serves
    /// nothing. No request is recorded; [`Server::connections`] still
    /// counts.
    pub fn fall_silent(&self) {
        self.silent.store(true, Ordering::SeqCst);
    }

    /// Stops listening: a request from now on is refused, as without a
    /// network.
    pub async fn stop(mut self) {
        self.task.abort();
        // The listener is closed once the task is gone.
        let _ = (&mut self.task).await;
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Reads one request from `stream`, records it and answers it; the
/// connection is closed after the answer.
async fn answer(mut stream: tokio::net::TcpStream, files: Files, requests: Requests) {
    let mut head = Vec::new();
    let mut buffer = [0u8; 2048];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") && head.len() < 64 * 1024 {
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => head.extend_from_slice(&buffer[..read]),
        }
    }
    let head = String::from_utf8_lossy(&head);
    let mut lines = head.lines();
    let path = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .map_or_default(|path| path.trim_start_matches('/').to_owned());
    let headers = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let served = lock(&files).get(&path).cloned();
    lock(&requests).push(Request { path, headers });
    let (status, location, body) = match served {
        Some(Served::Bytes(body)) => ("200 OK", String::new(), body),
        Some(Served::Elsewhere(to)) => ("302 Found", format!("Location: {to}\r\n"), Vec::new()),
        None => ("404 Not Found", String::new(), Vec::new()),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    if stream.write_all(head.as_bytes()).await.is_ok() {
        let _ = stream.write_all(&body).await;
    }
    let _ = stream.shutdown().await;
}
