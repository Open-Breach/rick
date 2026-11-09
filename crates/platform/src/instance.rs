//! The single-instance endpoint (specs/system-integration.md §3): one Rick
//! process per user session and config directory, and the wire the second
//! launch hands its command line over.
//!
//! The first process binds an [`Endpoint`] named after the user and the
//! config directory, so two installs with different config directories
//! coexist: a Unix socket in `$XDG_RUNTIME_DIR` on Linux and in the user's
//! temporary directory on macOS, a named pipe under `\\.\pipe\` on
//! Windows. A second launch connects, sends one [`Request`] — *run command
//! with arguments*: the arguments, the caller's working directory and the
//! environment the window needs to be raised in the caller's session — and
//! reads one [`Reply`] with the exit status and an optional text. Frames
//! are a four-byte big-endian length and JSON; each message carries
//! [`PROTOCOL_VERSION`], and a request of another version is answered with
//! [`EXIT_USAGE`] and a line saying so, without being understood.
//!
//! [`Endpoint::hand_over`] is the newcomer's half: connect, send, wait
//! [`PATIENCE`] for the reply, or report that nothing listens or that the
//! instance is unresponsive. [`Endpoint::bind`] is the instance's: a socket
//! file left behind by a crash, which nothing answers on, is removed and
//! the endpoint taken; one another process answers on is [`AddrInUse`].
//!
//! [`AddrInUse`]: io::ErrorKind::AddrInUse

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// The version of the request and reply, in every frame. A change to the
/// fields bumps it; the instance answers another version with
/// [`EXIT_USAGE`] and a line naming both.
pub const PROTOCOL_VERSION: u32 = 1;

/// The exit status of a command the instance does not know, or a request
/// it cannot read: the usage error of the shell conventions, with the help
/// in the reply's text.
pub const EXIT_USAGE: i32 = 2;

/// The exit status the newcomer ends with when the instance did not answer
/// within [`PATIENCE`], or when the handover failed otherwise.
pub const EXIT_FAILED: i32 = 1;

/// How long the newcomer waits for the reply, and the instance for the
/// request, before it gives the other side up.
pub const PATIENCE: Duration = Duration::from_secs(1);

/// The environment variables a request carries: the display the caller
/// is on and the activation token the launcher gave it, so the instance
/// raises its window in the caller's session.
pub const ENVIRONMENT: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_ACTIVATION_TOKEN",
    "DESKTOP_STARTUP_ID",
];

/// A frame longer than this is refused unread: no command line is that
/// long, and a stray client must not make the instance allocate for it.
const MAX_FRAME: usize = 16 * 1024 * 1024;

/// *Run command with arguments*: the one message a launch sends the
/// instance, the same from every entry point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// [`PROTOCOL_VERSION`] of the sender.
    pub protocol_version: u32,
    /// The arguments after the program name, as the caller typed them;
    /// paths are resolved against `cwd` by the receiver.
    pub argv: Vec<String>,
    /// The caller's working directory.
    pub cwd: PathBuf,
    /// The caller's values of [`ENVIRONMENT`], the ones it has.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl Request {
    /// A request for `argv` from `cwd`, with this process's values of
    /// [`ENVIRONMENT`].
    #[must_use]
    pub fn new(argv: Vec<String>, cwd: PathBuf) -> Self {
        let env = ENVIRONMENT
            .iter()
            .filter_map(|name| {
                std::env::var(name)
                    .ok()
                    .filter(|value| !value.is_empty())
                    .map(|value| ((*name).to_owned(), value))
            })
            .collect();
        Self {
            protocol_version: PROTOCOL_VERSION,
            argv,
            cwd,
            env,
        }
    }
}

/// What the instance answers: the exit status for the newcomer and a text
/// for it to print, when there is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    /// [`PROTOCOL_VERSION`] of the sender.
    pub protocol_version: u32,
    /// The exit status the newcomer ends with.
    pub status: i32,
    /// What the newcomer prints: on its standard output when `status` is
    /// zero, on its standard error otherwise.
    pub text: Option<String>,
}

impl Reply {
    /// Success, with `text` to print when there is any.
    #[must_use]
    pub fn ok(text: Option<String>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            status: 0,
            text,
        }
    }

    /// A failure with `status` and the text that says why.
    #[must_use]
    pub fn failed(status: i32, text: impl Into<String>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            status,
            text: Some(text.into()),
        }
    }

    /// A command the instance does not know: [`EXIT_USAGE`] and the help.
    #[must_use]
    pub fn usage(help: impl Into<String>) -> Self {
        Self::failed(EXIT_USAGE, help)
    }
}

/// Writes `value` as one frame: the length, big-endian, then the JSON.
///
/// # Errors
///
/// The I/O error of the write, or `InvalidInput` for a value whose JSON is
/// longer than a frame may be.
pub async fn write_frame<T: Serialize>(
    io: &mut (impl AsyncWrite + Unpin),
    value: &T,
) -> io::Result<()> {
    let body = serde_json::to_vec(value).map_err(io::Error::other)?;
    let len = u32::try_from(body.len())
        .ok()
        .filter(|len| *len as usize <= MAX_FRAME)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "frame too long"))?;
    io.write_all(&len.to_be_bytes()).await?;
    io.write_all(&body).await?;
    io.flush().await
}

/// Reads one frame and parses its JSON as a `T`.
///
/// # Errors
///
/// `UnexpectedEof` when the other side closed before a whole frame
/// arrived, `InvalidData` for a frame too long to be one of ours or whose
/// JSON is not a `T`, else the I/O error of the read.
pub async fn read_frame<T: DeserializeOwned>(io: &mut (impl AsyncRead + Unpin)) -> io::Result<T> {
    let mut len = [0u8; 4];
    io.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a frame of {len} bytes is not a Rick message"),
        ));
    }
    let mut body = vec![0u8; len];
    io.read_exact(&mut body).await?;
    serde_json::from_slice(&body).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// A byte stream between the newcomer and the instance.
pub trait InstanceIo: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin> InstanceIo for T {}

/// The stream, boxed so one type serves every platform.
pub type Stream = Box<dyn InstanceIo>;

/// What a launch found when it handed its request over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handover {
    /// An instance answered; print the text and exit with the status.
    Answered(Reply),
    /// Nothing listens on the endpoint: this launch becomes the instance.
    NoInstance,
    /// An instance accepted the connection but did not answer within
    /// [`PATIENCE`].
    Unresponsive,
}

/// Where the instance of one user and config directory listens.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Endpoint {
    /// The socket file, or the pipe's `\\.\pipe\…` name.
    path: PathBuf,
}

/// FNV-1a over the bytes: a short, stable name for a path, the same from
/// every build of Rick, which the standard library's hasher does not
/// promise.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The config directory as a stable name: canonical where it exists (so
/// `~/.config/rick` and a symlink to it are one instance), then hashed.
fn config_tag(config_dir: &Path) -> String {
    let canonical = std::fs::canonicalize(config_dir)
        .or_else(|_| std::path::absolute(config_dir))
        .unwrap_or_else(|_| config_dir.to_path_buf());
    format!("{:016x}", fnv1a(canonical.as_os_str().as_encoded_bytes()))
}

impl Endpoint {
    /// The endpoint of this user's Rick for `config_dir`:
    /// `$XDG_RUNTIME_DIR/rick-<tag>.sock` on Linux (`/tmp/rick-<uid>-<tag>.sock`
    /// without a runtime directory), `$TMPDIR/rick-<tag>.sock` on macOS,
    /// `\\.\pipe\rick-<user>-<tag>` on Windows, the tag naming the config
    /// directory.
    #[must_use]
    pub fn for_config_dir(config_dir: &Path) -> Self {
        Self {
            path: imp::endpoint_path(&config_tag(config_dir)),
        }
    }

    /// An endpoint of a test's own inside `dir`: a socket file there, or a
    /// pipe named after the directory.
    #[must_use]
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            path: imp::endpoint_in_dir(dir),
        }
    }

    /// The socket file or pipe name.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Connects to the instance listening here.
    ///
    /// # Errors
    ///
    /// `NotFound` or `ConnectionRefused` when nothing listens (the second
    /// for a socket file whose process is gone), else the I/O error.
    pub async fn connect(&self) -> io::Result<Stream> {
        imp::connect(&self.path).await
    }

    /// Takes the endpoint for this process. A socket file nothing answers
    /// on, left by a crash, is removed first.
    ///
    /// # Errors
    ///
    /// `AddrInUse` when another process answers on the endpoint, else the
    /// I/O error of binding.
    pub async fn bind(&self) -> io::Result<Listener> {
        let inner = imp::bind(&self.path).await?;
        Ok(Listener {
            inner,
            endpoint: self.clone(),
        })
    }

    /// Removes what [`bind`](Self::bind) left on disk, for the instance
    /// that bound the endpoint and stops listening at exit while its
    /// [`Listener`] lives on another thread; nothing on Windows, where a
    /// pipe dies with its process. Never for an endpoint another process
    /// holds.
    pub fn unbind(&self) {
        imp::unbind(&self.path);
    }

    /// The newcomer's half: connects, sends `request` and waits `patience`
    /// for the reply.
    ///
    /// # Errors
    ///
    /// An I/O error other than nothing listening: the socket's directory
    /// is not readable, the pipe refuses the user. Nothing listening is
    /// [`Handover::NoInstance`], not an error.
    pub async fn hand_over(&self, request: &Request, patience: Duration) -> io::Result<Handover> {
        let mut stream = match tokio::time::timeout(patience, self.connect()).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                return Ok(Handover::NoInstance);
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => return Ok(Handover::Unresponsive),
        };
        let exchange = async {
            write_frame(&mut stream, request).await?;
            read_frame::<Reply>(&mut stream).await
        };
        match tokio::time::timeout(patience, exchange).await {
            Ok(Ok(reply)) => Ok(Handover::Answered(reply)),
            // The instance closed without a word: it is there, but not
            // listening to us; the same as no answer.
            Ok(Err(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                Ok(Handover::Unresponsive)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Ok(Handover::Unresponsive),
        }
    }
}

/// The instance's end of the endpoint: accepts one connection per launch.
pub struct Listener {
    inner: imp::Listener,
    endpoint: Endpoint,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl Listener {
    /// The endpoint this listens on.
    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Waits for the next launch to connect.
    ///
    /// # Errors
    ///
    /// The I/O error of accepting; the listener stays usable.
    pub async fn accept(&mut self) -> io::Result<Connection> {
        let stream = self.inner.accept().await?;
        Ok(Connection { stream })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        imp::unbind(&self.endpoint.path);
    }
}

/// One launch's connection to the instance: its request, then the reply.
pub struct Connection {
    stream: Stream,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection").finish_non_exhaustive()
    }
}

impl Connection {
    /// Reads the request, waiting at most `patience` for it. A request of
    /// another [`PROTOCOL_VERSION`] is answered on the spot with
    /// [`EXIT_USAGE`] and a line naming both versions, and reported as
    /// `Unsupported`.
    ///
    /// # Errors
    ///
    /// `TimedOut` when nothing whole arrived in time, `Unsupported` for
    /// another protocol version (already answered), `InvalidData` for a
    /// frame that is no request, else the I/O error of the read.
    pub async fn request(&mut self, patience: Duration) -> io::Result<Request> {
        let value: serde_json::Value = tokio::time::timeout(patience, read_frame(&mut self.stream))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no request arrived in time"))??;
        let version = value
            .get("protocol_version")
            .and_then(serde_json::Value::as_u64);
        if version != Some(u64::from(PROTOCOL_VERSION)) {
            let text = match version {
                Some(version) => format!(
                    "rick: the running instance speaks protocol version {PROTOCOL_VERSION}, this \
                     launch version {version}; start both from the same Rick"
                ),
                None => "rick: the request names no protocol version".to_owned(),
            };
            let _ = self.reply(&Reply::usage(text.clone())).await;
            return Err(io::Error::new(io::ErrorKind::Unsupported, text));
        }
        serde_json::from_value(value)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// Sends the reply and closes the connection's writing end.
    ///
    /// # Errors
    ///
    /// The I/O error of the write; the launch may be gone already.
    pub async fn reply(&mut self, reply: &Reply) -> io::Result<()> {
        write_frame(&mut self.stream, reply).await?;
        self.stream.shutdown().await
    }
}

#[cfg(unix)]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};

    use tokio::net::{UnixListener, UnixStream};

    use super::Stream;

    /// The socket file's name for the config directory's `tag`.
    pub(super) fn endpoint_path(tag: &str) -> PathBuf {
        #[cfg(target_os = "linux")]
        {
            // The runtime directory is the user's own; without one (a
            // bare TTY session, a container) the shared temporary
            // directory, with the user in the name.
            if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
                return PathBuf::from(dir).join(format!("rick-{tag}.sock"));
            }
            let uid = rustix::process::getuid().as_raw();
            std::env::temp_dir().join(format!("rick-{uid}-{tag}.sock"))
        }
        #[cfg(not(target_os = "linux"))]
        {
            // macOS gives every user a temporary directory of their own
            // (`$TMPDIR`, under /var/folders); the user goes into the
            // name all the same, for a shell that unset it.
            let uid = rustix::process::getuid().as_raw();
            std::env::temp_dir().join(format!("rick-{uid}-{tag}.sock"))
        }
    }

    pub(super) fn endpoint_in_dir(dir: &Path) -> PathBuf {
        dir.join("instance.sock")
    }

    pub(super) async fn connect(path: &Path) -> io::Result<Stream> {
        let stream = UnixStream::connect(path).await?;
        Ok(Box::new(stream))
    }

    pub(super) struct Listener(UnixListener);

    impl Listener {
        pub(super) async fn accept(&mut self) -> io::Result<Stream> {
            let (stream, _) = self.0.accept().await?;
            Ok(Box::new(stream))
        }
    }

    pub(super) async fn bind(path: &Path) -> io::Result<Listener> {
        if std::fs::symlink_metadata(path).is_ok() {
            // A file is there: another instance's, or a crash's leftover.
            // Nothing answers on a leftover, so it goes; an answer means
            // the endpoint is taken.
            match UnixStream::connect(path).await {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        format!("another Rick listens on {}", path.display()),
                    ));
                }
                Err(error) => {
                    tracing::info!(path = %path.display(), %error, "stale socket removed");
                    std::fs::remove_file(path)?;
                }
            }
        } else if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let listener = UnixListener::bind(path)?;
        // The endpoint is this user's: nobody else connects.
        if let Err(error) =
            std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
        {
            tracing::debug!(path = %path.display(), %error, "socket permissions not set");
        }
        Ok(Listener(listener))
    }

    pub(super) fn unbind(path: &Path) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};

    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    use super::Stream;

    /// `ERROR_PIPE_BUSY`: every instance of the pipe is taken for the
    /// moment; wait and try again.
    const ERROR_PIPE_BUSY: i32 = 231;
    /// `ERROR_ACCESS_DENIED`: `first_pipe_instance` found the pipe taken.
    const ERROR_ACCESS_DENIED: i32 = 5;
    /// `ERROR_FILE_NOT_FOUND`: no server has the pipe open.
    const ERROR_FILE_NOT_FOUND: i32 = 2;

    /// The pipe's name for the config directory's `tag`: the user in it
    /// too, since the pipe namespace is the machine's.
    pub(super) fn endpoint_path(tag: &str) -> PathBuf {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".to_owned());
        let user: String = user
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        PathBuf::from(format!(r"\\.\pipe\rick-{user}-{tag}"))
    }

    pub(super) fn endpoint_in_dir(dir: &Path) -> PathBuf {
        let tag = super::fnv1a(dir.as_os_str().as_encoded_bytes());
        PathBuf::from(format!(r"\\.\pipe\rick-test-{tag:016x}"))
    }

    pub(super) async fn connect(path: &Path) -> io::Result<Stream> {
        loop {
            match ClientOptions::new().open(path) {
                Ok(client) => return Ok(Box::new(client)),
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Err(error) if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => {
                    return Err(io::Error::new(io::ErrorKind::NotFound, error));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// The pipe: one server instance waits for a client; once one is
    /// connected, the next instance is created before the connected one
    /// is handed out, so a launch always finds the pipe.
    pub(super) struct Listener {
        path: PathBuf,
        server: NamedPipeServer,
    }

    impl Listener {
        pub(super) async fn accept(&mut self) -> io::Result<Stream> {
            self.server.connect().await?;
            let next = ServerOptions::new().create(&self.path)?;
            let connected = std::mem::replace(&mut self.server, next);
            Ok(Box::new(connected))
        }
    }

    pub(super) async fn bind(path: &Path) -> io::Result<Listener> {
        // A named pipe dies with its process: there is no stale pipe to
        // remove, and a pipe that exists has an owner.
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(path)
            .map_err(|error| {
                if error.raw_os_error() == Some(ERROR_ACCESS_DENIED) {
                    io::Error::new(
                        io::ErrorKind::AddrInUse,
                        format!("another Rick listens on {}", path.display()),
                    )
                } else {
                    error
                }
            })?;
        Ok(Listener {
            path: path.to_path_buf(),
            server,
        })
    }

    pub(super) fn unbind(_path: &Path) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(argv: &[&str]) -> Request {
        Request {
            protocol_version: PROTOCOL_VERSION,
            argv: argv.iter().map(|a| (*a).to_owned()).collect(),
            cwd: PathBuf::from("/work"),
            env: BTreeMap::from([("DISPLAY".to_owned(), ":1".to_owned())]),
        }
    }

    #[test]
    fn a_frame_round_trips_and_a_long_one_is_refused() {
        runtime::block_on(async {
            let (mut a, mut b) = tokio::io::duplex(4096);
            let sent = request(&["reveal", "a b.txt"]);
            write_frame(&mut a, &sent)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            let got: Request = read_frame(&mut b).await.unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(got, sent);

            // A length that is no Rick message is refused before any body.
            let (mut a, mut b) = tokio::io::duplex(64);
            a.write_all(&u32::MAX.to_be_bytes())
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            let got = read_frame::<Request>(&mut b).await;
            assert_eq!(
                got.as_ref().err().map(io::Error::kind),
                Some(io::ErrorKind::InvalidData),
                "{got:?}"
            );

            // A closed stream before a whole frame is the end of the
            // conversation, not data.
            let (mut a, mut b) = tokio::io::duplex(64);
            a.write_all(&8u32.to_be_bytes())
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            drop(a);
            let got = read_frame::<Request>(&mut b).await;
            assert_eq!(
                got.as_ref().err().map(io::Error::kind),
                Some(io::ErrorKind::UnexpectedEof),
                "{got:?}"
            );
        });
    }

    #[test]
    fn a_request_carries_the_display_variables_of_the_process() {
        let request = Request::new(vec!["x".to_owned()], PathBuf::from("/w"));
        assert_eq!(request.protocol_version, PROTOCOL_VERSION);
        assert_eq!(request.argv, ["x"]);
        assert!(
            request
                .env
                .keys()
                .all(|k| ENVIRONMENT.contains(&k.as_str())),
            "{request:?}"
        );
        for name in ENVIRONMENT {
            assert_eq!(
                request.env.get(*name).cloned(),
                std::env::var(name).ok().filter(|v| !v.is_empty())
            );
        }
        let reply = Reply::usage("help");
        assert_eq!(reply.status, EXIT_USAGE);
        assert_eq!(reply.text.as_deref(), Some("help"));
        assert_eq!(Reply::ok(None).status, 0);
    }

    #[test]
    fn the_endpoint_names_the_config_directory_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let a = Endpoint::for_config_dir(&dir.path().join("a"));
        let b = Endpoint::for_config_dir(&dir.path().join("b"));
        assert_ne!(a, b);
        assert_eq!(a, Endpoint::for_config_dir(&dir.path().join("a")));
        let name = a.path().to_string_lossy().into_owned();
        assert!(name.contains("rick-"), "{name}");
        assert!(!name.contains("/a"), "no path in the name: {name}");
        // The directory's canonical path names it, so a symlink to the
        // config directory is the same instance.
        #[cfg(unix)]
        {
            std::fs::create_dir(dir.path().join("a")).unwrap_or_else(|e| panic!("{e}"));
            std::os::unix::fs::symlink(dir.path().join("a"), dir.path().join("link"))
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(
                Endpoint::for_config_dir(&dir.path().join("a")),
                Endpoint::for_config_dir(&dir.path().join("link"))
            );
        }
    }

    #[test]
    fn a_request_is_answered_and_another_protocol_version_is_refused() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let endpoint = Endpoint::in_dir(dir.path());
        runtime::block_on(async {
            let mut listener = endpoint.bind().await.unwrap_or_else(|e| panic!("{e}"));
            let server = async {
                // The first launch: answered with its arguments echoed.
                let mut conn = listener.accept().await.unwrap_or_else(|e| panic!("{e}"));
                let got = conn
                    .request(PATIENCE)
                    .await
                    .unwrap_or_else(|e| panic!("{e}"));
                assert_eq!(got, request(&["a", "b"]));
                conn.reply(&Reply::ok(Some(got.argv.join(" "))))
                    .await
                    .unwrap_or_else(|e| panic!("{e}"));
                // The second: another protocol version, refused unread.
                let mut conn = listener.accept().await.unwrap_or_else(|e| panic!("{e}"));
                let refused = conn.request(PATIENCE).await;
                assert_eq!(
                    refused.as_ref().err().map(io::Error::kind),
                    Some(io::ErrorKind::Unsupported),
                    "{refused:?}"
                );
            };
            let client = async {
                let first = endpoint
                    .hand_over(&request(&["a", "b"]), PATIENCE)
                    .await
                    .unwrap_or_else(|e| panic!("{e}"));
                assert_eq!(first, Handover::Answered(Reply::ok(Some("a b".to_owned()))));
                let mut other = request(&["a"]);
                other.protocol_version = PROTOCOL_VERSION + 1;
                let second = endpoint
                    .hand_over(&other, PATIENCE)
                    .await
                    .unwrap_or_else(|e| panic!("{e}"));
                match second {
                    Handover::Answered(reply) => {
                        assert_eq!(reply.status, EXIT_USAGE);
                        let text = reply.text.unwrap_or_default();
                        assert!(
                            text.contains(&format!("version {PROTOCOL_VERSION}"))
                                && text.contains(&format!("version {}", PROTOCOL_VERSION + 1)),
                            "{text}"
                        );
                    }
                    other => panic!("{other:?}"),
                }
            };
            tokio::join!(server, client);
        });
    }

    #[test]
    fn nothing_listening_is_no_instance_and_silence_is_unresponsive() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let endpoint = Endpoint::in_dir(dir.path());
        runtime::block_on(async {
            let found = endpoint
                .hand_over(&request(&[]), PATIENCE)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(found, Handover::NoInstance);

            // An instance that accepts and never answers.
            let mut listener = endpoint.bind().await.unwrap_or_else(|e| panic!("{e}"));
            let server = async {
                let conn = listener.accept().await.unwrap_or_else(|e| panic!("{e}"));
                tokio::time::sleep(Duration::from_millis(400)).await;
                drop(conn);
            };
            let client = async {
                let started = std::time::Instant::now();
                let found = endpoint
                    .hand_over(&request(&[]), Duration::from_millis(150))
                    .await
                    .unwrap_or_else(|e| panic!("{e}"));
                assert_eq!(found, Handover::Unresponsive);
                assert!(
                    started.elapsed() < Duration::from_secs(1),
                    "{:?}",
                    started.elapsed()
                );
            };
            tokio::join!(server, client);
        });
    }

    #[test]
    fn a_second_bind_is_refused_while_the_first_listens() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let endpoint = Endpoint::in_dir(dir.path());
        runtime::block_on(async {
            let first = endpoint.bind().await.unwrap_or_else(|e| panic!("{e}"));
            let second = endpoint.bind().await;
            assert_eq!(
                second.as_ref().err().map(io::Error::kind),
                Some(io::ErrorKind::AddrInUse),
                "{second:?}"
            );
            drop(first);
            // Gone with the listener: the next bind is a fresh one.
            let again = endpoint.bind().await;
            assert!(again.is_ok(), "{again:?}");
        });
    }

    /// A socket file left by a crash: nothing answers on it, so the next
    /// instance removes it and takes the endpoint.
    #[cfg(unix)]
    #[test]
    fn a_stale_socket_is_reclaimed() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let endpoint = Endpoint::in_dir(dir.path());
        // A listener whose process died: the file stays, nobody accepts.
        let stale = std::os::unix::net::UnixListener::bind(endpoint.path())
            .unwrap_or_else(|e| panic!("{e}"));
        // Closing the listener without removing the file is what a crash
        // leaves behind; `std` removes nothing on drop.
        drop(stale);
        assert!(endpoint.path().exists(), "the stale file is there");
        runtime::block_on(async {
            let found = endpoint
                .hand_over(&request(&[]), PATIENCE)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(found, Handover::NoInstance, "a stale socket is no instance");
            let listener = endpoint.bind().await.unwrap_or_else(|e| panic!("{e}"));
            assert!(endpoint.path().exists());
            let client = async {
                endpoint
                    .hand_over(&request(&["x"]), PATIENCE)
                    .await
                    .unwrap_or_else(|e| panic!("{e}"))
            };
            let mut listener = listener;
            let server = async {
                let mut conn = listener.accept().await.unwrap_or_else(|e| panic!("{e}"));
                let got = conn
                    .request(PATIENCE)
                    .await
                    .unwrap_or_else(|e| panic!("{e}"));
                conn.reply(&Reply::ok(Some(got.argv.join(","))))
                    .await
                    .unwrap_or_else(|e| panic!("{e}"));
            };
            let (found, ()) = tokio::join!(client, server);
            assert_eq!(found, Handover::Answered(Reply::ok(Some("x".to_owned()))));
            drop(listener);
            assert!(
                !endpoint.path().exists(),
                "the listener takes its file along"
            );
        });
    }
}
