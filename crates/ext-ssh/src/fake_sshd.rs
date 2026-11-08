//! An SSH server in the test process, for the plugin's own tests and the
//! acceptance tests: `russh`'s server side over a loopback port, which
//! authenticates with a password, the authorized keys and
//! keyboard-interactive, runs every `exec` and `shell` request through the
//! local `sh` with the request's standard streams on the channel, refuses
//! the session channel after a limit like `MaxSessions`, and connects a
//! `direct-tcpip` channel to the host and port asked for, so a second
//! server stands in for a host behind a bastion. The local userland is the
//! remote one, which on Linux CI is GNU; the `busybox` and `bsd` profiles
//! are tested against the containers and recorded output.
//!
//! Only built for the tests (`cfg(test)`) and for the `fake-sshd` feature
//! the `tests` crate turns on.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Response, Server as _, Session};
use russh::{Channel, ChannelId, ChannelOpenFailure};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

/// What the server accepts and does.
#[derive(Debug, Clone)]
pub struct Options {
    /// The password `rick` logs in with, when any.
    pub password: Option<String>,
    /// The public keys that log in.
    pub authorized: Vec<PublicKey>,
    /// Session channels per connection before the next is refused, as
    /// `MaxSessions`.
    pub max_sessions: usize,
    /// The directory commands start in: the "remote home".
    pub home: PathBuf,
    /// Whether `direct-tcpip` channels are connected (a bastion) or
    /// refused.
    pub forwarding: bool,
    /// Answer keyboard-interactive with a password prompt.
    pub keyboard_interactive: bool,
    /// The loopback port to listen on; 0 picks a free one.
    pub port: u16,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            password: Some("rickpass".to_owned()),
            authorized: Vec::new(),
            max_sessions: 10,
            home: std::env::temp_dir(),
            forwarding: true,
            keyboard_interactive: false,
            port: 0,
        }
    }
}

/// A deterministic Ed25519 key for the tests, from a seed byte.
#[must_use]
pub fn test_key(seed: u8) -> PrivateKey {
    let seed = [seed; 32];
    PrivateKey::from(Ed25519Keypair::from_seed(&seed))
}

/// The server, listening until dropped.
pub struct FakeSshd {
    /// The loopback port it listens on.
    pub port: u16,
    /// Its host key.
    pub host_key: PublicKey,
    /// Connections accepted so far.
    pub connections: Arc<AtomicUsize>,
    /// Session channels opened so far, refusals not counted.
    pub sessions: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for FakeSshd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeSshd")
            .field("port", &self.port)
            .field(
                "host_key",
                &self.host_key.fingerprint(russh::keys::HashAlg::default()),
            )
            .finish_non_exhaustive()
    }
}

impl Drop for FakeSshd {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeSshd {
    /// Starts a server on a free loopback port with `options`, keyed with
    /// [`test_key`] of `key_seed`.
    ///
    /// # Errors
    ///
    /// The I/O error of binding the port.
    pub async fn start(options: Options, key_seed: u8) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", options.port)).await?;
        let port = listener.local_addr()?.port();
        let key = test_key(key_seed);
        let host_key = key.public_key().clone();
        let config = Arc::new(server::Config {
            keys: vec![key],
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            inactivity_timeout: None,
            ..server::Config::default()
        });
        let connections = Arc::new(AtomicUsize::new(0));
        let sessions = Arc::new(AtomicUsize::new(0));
        let mut sshd = Sshd {
            options: Arc::new(options),
            connections: Arc::clone(&connections),
            sessions: Arc::clone(&sessions),
        };
        let task = tokio::spawn(async move {
            let running = sshd.run_on_socket(config, &listener);
            let _ = running.await;
        });
        Ok(Self {
            port,
            host_key,
            connections,
            sessions,
            task,
        })
    }
}

/// The server: one [`Conn`] per connection.
struct Sshd {
    options: Arc<Options>,
    connections: Arc<AtomicUsize>,
    sessions: Arc<AtomicUsize>,
}

impl server::Server for Sshd {
    type Handler = Conn;

    fn new_client(&mut self, _peer: Option<std::net::SocketAddr>) -> Conn {
        self.connections.fetch_add(1, Ordering::Relaxed);
        Conn {
            options: Arc::clone(&self.options),
            sessions: Arc::clone(&self.sessions),
            open: Arc::default(),
            stdin: HashMap::new(),
        }
    }
}

/// The session channels open on a connection, shared with the tasks that
/// close them when their command ends.
type Open = Arc<std::sync::Mutex<HashSet<ChannelId>>>;

/// One connection: the session channels open and the standard inputs of
/// the commands running on them.
struct Conn {
    options: Arc<Options>,
    sessions: Arc<AtomicUsize>,
    open: Open,
    stdin: HashMap<ChannelId, tokio::process::ChildStdin>,
}

fn open_count(open: &Open) -> usize {
    open.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .len()
}

fn forget(open: &Open, channel: ChannelId) {
    open.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&channel);
}

impl Conn {
    fn authorized(&self, key: &PublicKey) -> bool {
        self.options
            .authorized
            .iter()
            .any(|k| k.key_data() == key.key_data())
    }

    /// Runs `command` through `sh -c` in the home, its streams on the
    /// channel; the exit status, EOF and close follow when it ends.
    fn spawn(&mut self, channel: ChannelId, command: Option<Vec<u8>>, session: &Session) {
        let mut process = tokio::process::Command::new("sh");
        if let Some(command) = command {
            process.arg("-c");
            process.arg(std::ffi::OsStr::new(
                std::str::from_utf8(&command).unwrap_or("exit 127"),
            ));
        }
        process
            .current_dir(&self.options.home)
            .env("HOME", &self.options.home)
            .env("SHELL", "/bin/sh")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let handle = session.handle();
        let mut child = match process.spawn() {
            Ok(child) => child,
            Err(error) => {
                tokio::spawn(async move {
                    let _ = handle
                        .extended_data(channel, 1, format!("sh: {error}\n").into_bytes())
                        .await;
                    let _ = handle.exit_status_request(channel, 127).await;
                    let _ = handle.eof(channel).await;
                    let _ = handle.close(channel).await;
                });
                return;
            }
        };
        if let Some(stdin) = child.stdin.take() {
            self.stdin.insert(channel, stdin);
        }
        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let open = Arc::clone(&self.open);
        tokio::spawn(async move {
            let out = handle.clone();
            let err = handle.clone();
            let pump_out = async {
                if let Some(stdout) = stdout.as_mut() {
                    let mut buffer = vec![0u8; 64 * 1024];
                    loop {
                        match tokio::io::AsyncReadExt::read(stdout, &mut buffer).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if out.data(channel, buffer[..n].to_vec()).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
            };
            let pump_err = async {
                if let Some(stderr) = stderr.as_mut() {
                    let mut buffer = vec![0u8; 16 * 1024];
                    loop {
                        match tokio::io::AsyncReadExt::read(stderr, &mut buffer).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if err
                                    .extended_data(channel, 1, buffer[..n].to_vec())
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                    }
                }
            };
            tokio::join!(pump_out, pump_err);
            let status = child
                .wait()
                .await
                .ok()
                .and_then(|s| s.code())
                .and_then(|c| u32::try_from(c).ok())
                .unwrap_or(255);
            let _ = handle.exit_status_request(channel, status).await;
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
            // The session is free again, as sshd frees one when its
            // command ends.
            forget(&open, channel);
        });
    }
}

impl server::Handler for Conn {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if self.options.password.as_deref() == Some(password) {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey_offered(
        &mut self,
        _user: &str,
        key: server::PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        Ok(if self.authorized(key.public_key) {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey(
        &mut self,
        _user: &str,
        key: server::VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        Ok(if self.authorized(key.public_key) {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        if !self.options.keyboard_interactive {
            return Ok(Auth::reject());
        }
        match response {
            None => Ok(Auth::Partial {
                name: Cow::Borrowed("Rick test server"),
                instructions: Cow::Borrowed("Say the password."),
                prompts: Cow::Owned(vec![(Cow::Borrowed("Password: "), false)]),
            }),
            Some(mut answers) => {
                let typed = answers.next().unwrap_or_default();
                Ok(
                    if self.options.password.as_deref().map(str::as_bytes) == Some(&typed[..]) {
                        Auth::Accept
                    } else {
                        Auth::reject()
                    },
                )
            }
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if open_count(&self.open) >= self.options.max_sessions {
            reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        self.open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(channel.id());
        self.sessions.fetch_add(1, Ordering::Relaxed);
        // The callbacks below carry the channel's traffic; the channel
        // object itself is dropped so its queue never fills.
        drop(channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.stdin.remove(&channel);
        forget(&self.open, channel);
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host: &str,
        port: u32,
        _originator: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if !self.options.forwarding {
            reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        let port = u16::try_from(port).unwrap_or(22);
        match tokio::net::TcpStream::connect((host, port)).await {
            Ok(mut tcp) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
                });
            }
            Err(_) => reply.reject(ChannelOpenFailure::ConnectFailed).await,
        }
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _cols: u32,
        _rows: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        _channel: ChannelId,
        _cols: u32,
        _rows: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        self.spawn(channel, None, session);
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        self.spawn(channel, Some(data.to_vec()), session);
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(stdin) = self.stdin.get_mut(&channel)
            && stdin.write_all(data).await.is_err()
        {
            self.stdin.remove(&channel);
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Dropping the pipe is the command's EOF.
        self.stdin.remove(&channel);
        Ok(())
    }
}

/// The OpenSSH text of a private key, for an identity file.
#[must_use]
pub fn key_text(key: &PrivateKey) -> String {
    key.to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .map(|z| z.to_string())
        .unwrap_or_default()
}

/// Writes `key` as an identity file at `path`, readable by the owner only.
///
/// # Errors
///
/// The I/O error.
pub fn write_identity(path: &Path, key: &PrivateKey) -> std::io::Result<()> {
    std::fs::write(path, key_text(key))
}

/// An Ed25519 private key encrypted with the passphrase `test`, from
/// russh's own fixtures, for the passphrase prompt.
pub const ENCRYPTED_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABD1phlku5
A2G7Q9iP+DcOc9AAAAEAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHeLC1lWiCYrXsf/
85O/pkbUFZ6OGIt49PX3nw8iRoXEAAAAkKRF0st5ZI7xxo9g6A4m4l6NarkQre3mycqNXQ
dP3jryYgvsCIBAA5jMWSjrmnOTXhidqcOy4xYCrAttzSnZ/cUadfBenL+DQq6neffw7j8r
0tbCxVGp6yCQlKrgSZf6c0Hy7dNEIU2bJFGxLe6/kWChcUAt/5Ll5rI7DVQPJdLgehLzvv
sJWR7W+cGvJ/vLsw==
-----END OPENSSH PRIVATE KEY-----
";

/// The passphrase of [`ENCRYPTED_KEY`].
pub const ENCRYPTED_KEY_PASSPHRASE: &str = "test";
