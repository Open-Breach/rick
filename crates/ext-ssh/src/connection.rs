//! The connection pool: one pool per host, channels multiplexed on a
//! connection, a second connection at the server's session limit, idle
//! connections closed after the setting's timeout and reopened on the next
//! operation, jump hosts connected like any host with the next hop over a
//! `direct-tcpip` channel. The first connection to a host authenticates:
//! the agent, then the identity files, then keyboard-interactive and
//! password, with the keychain and the `Prompts` queue for whatever the
//! chain needs from the user, and the two host-key dialogues over
//! `known_hosts`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use ext::platform::Platform;
use ext::{Connection, ConnectionSchema, Connector, Prompts, async_trait, secret_key};
use futures::future::BoxFuture;
use russh::client::{self, KeyboardInteractiveAuthResponse, Msg};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{Channel, ChannelMsg, ChannelStream};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tracing::{debug, info, warn};
use types::{Error, ProviderId};

use crate::config::IdentityAgent;
use crate::hosts::{Host, Hosts, field};
use crate::known_hosts::{self, Verdict};
use crate::profile::{self, Probe, Profile};
use crate::{PROVIDER_ID, message, ssh_error};

/// The server's session limit assumed before a channel open is refused:
/// OpenSSH's `MaxSessions` default.
pub const MAX_CHANNELS: usize = 10;
/// How long the TCP connection to a host may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How often idle connections are looked for.
const SWEEP_EVERY: Duration = Duration::from_secs(15);
/// How often a wrong secret may be typed again before the method is given
/// up.
const TRIES: usize = 3;
/// The identity files tried after the host's, under `~/.ssh/`.
const DEFAULT_IDENTITIES: [&str; 4] = ["id_ed25519", "id_ecdsa", "id_rsa", "id_dsa"];

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the host-key check decided when it refused, for the error.
type Refusal = Arc<Mutex<Option<String>>>;

/// The russh client handler: checks the server's host key against
/// `known_hosts` and asks the user about an unknown or a changed one.
struct Handler {
    /// The host as the dialogues and errors name it: the alias, and the
    /// hop it is when it is one.
    label: String,
    host: String,
    port: u16,
    known_hosts: PathBuf,
    prompts: Option<Arc<Prompts>>,
    refusal: Refusal,
}

impl Handler {
    fn refuse(&self, reason: String) {
        warn!(host = %self.label, %reason, "connection refused");
        *lock(&self.refusal) = Some(reason);
    }

    /// The host-key decision: a known key passes; an unknown one asks
    /// *Trust and save*, *Trust once* or *Cancel*; a changed one shows the
    /// warning with *Cancel* as its default and replaces the entry only
    /// after a second confirmation; a revoked one is refused. Nothing is
    /// trusted silently: without a `Prompts` queue an unknown key is
    /// refused too.
    async fn verify(&self, key: &PublicKey) -> bool {
        let verdict = match known_hosts::check(&self.known_hosts, &self.host, self.port, key) {
            Ok(verdict) => verdict,
            Err(error) => {
                self.refuse(format!(
                    "{} could not be read: {error}",
                    self.known_hosts.display()
                ));
                return false;
            }
        };
        let where_ = format!("{} ({}:{})", self.label, self.host, self.port);
        match verdict {
            Verdict::Known => true,
            Verdict::Revoked { line } => {
                self.refuse(format!(
                    "the {} host key {} of {where_} is revoked at line {line} of {}",
                    key.algorithm(),
                    known_hosts::fingerprint(key),
                    self.known_hosts.display()
                ));
                false
            }
            Verdict::Unknown => self.verify_unknown(key, &where_).await,
            Verdict::Changed {
                line,
                old_fingerprint,
            } => {
                self.verify_changed(key, &where_, line, &old_fingerprint)
                    .await
            }
        }
    }

    /// The unknown-host dialogue.
    async fn verify_unknown(&self, key: &PublicKey, where_: &str) -> bool {
        let fingerprint = known_hosts::fingerprint(key);
        let Some(prompts) = &self.prompts else {
            self.refuse(format!(
                "the host key of {where_} is not in {} and nobody can be asked",
                self.known_hosts.display()
            ));
            return false;
        };
        let text = format!(
            "{where_} is not in {}.\n\nKey type: {}\nSHA256 fingerprint: {fingerprint}\n\n\
             Trust and save adds the key to known_hosts; Trust once connects without saving it.",
            self.known_hosts.display(),
            key.algorithm()
        );
        let choice = prompts
            .choose(
                "Unknown host key",
                text,
                vec![
                    "Trust and save".to_owned(),
                    "Trust once".to_owned(),
                    "Cancel".to_owned(),
                ],
                0,
                false,
            )
            .await;
        match choice {
            Some(0) => {
                if let Err(error) =
                    known_hosts::append(&self.known_hosts, &self.host, self.port, key)
                {
                    warn!(%error, "host key not saved");
                } else {
                    info!(host = %self.label, %fingerprint, "host key saved");
                }
                true
            }
            Some(1) => {
                info!(host = %self.label, %fingerprint, "host key trusted once");
                true
            }
            _ => {
                self.refuse(format!("the host key of {where_} was not trusted"));
                false
            }
        }
    }

    /// The changed-key warning, with *Cancel* as its default, and the
    /// confirmation before the saved entry is replaced.
    async fn verify_changed(
        &self,
        key: &PublicKey,
        where_: &str,
        line: usize,
        old_fingerprint: &str,
    ) -> bool {
        let fingerprint = known_hosts::fingerprint(key);
        let key_type = key.algorithm().to_string();
        let Some(prompts) = &self.prompts else {
            self.refuse(format!(
                "the host key of {where_} changed (line {line} of {} holds another) and nobody can be asked",
                self.known_hosts.display()
            ));
            return false;
        };
        let text = format!(
            "WARNING: the host key of {where_} has changed.\n\n\
             Saved key (line {line} of {}): {key_type} {old_fingerprint}\n\
             Offered key: {key_type} {fingerprint}\n\n\
             Someone could be eavesdropping, or the host's key was replaced. \
             Cancel unless you know the key changed.",
            self.known_hosts.display()
        );
        let choice = prompts
            .choose(
                "Host key changed",
                text,
                vec![
                    "Cancel".to_owned(),
                    "Connect once".to_owned(),
                    "Replace saved key…".to_owned(),
                ],
                0,
                true,
            )
            .await;
        match choice {
            Some(1) => {
                warn!(host = %self.label, %fingerprint, "changed host key accepted once");
                true
            }
            Some(2) => {
                let confirm = prompts
                    .choose(
                        "Replace the saved host key?",
                        format!(
                            "Line {line} of {} will be replaced with the {key_type} key {fingerprint}.",
                            self.known_hosts.display()
                        ),
                        vec!["Cancel".to_owned(), "Replace".to_owned()],
                        0,
                        true,
                    )
                    .await;
                if confirm != Some(1) {
                    self.refuse(format!("the changed host key of {where_} was not accepted"));
                    return false;
                }
                if let Err(error) =
                    known_hosts::replace(&self.known_hosts, &self.host, self.port, key)
                {
                    warn!(%error, "host key not replaced");
                } else {
                    warn!(host = %self.label, %fingerprint, "saved host key replaced");
                }
                true
            }
            _ => {
                self.refuse(format!("the changed host key of {where_} was not accepted"));
                false
            }
        }
    }
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => cert.public_key().clone().into(),
        };
        Ok(self.verify(&key).await)
    }
}

/// One open connection to a host.
pub struct Live {
    alias: String,
    handle: client::Handle<Handler>,
    /// Channels open on it right now.
    channels: AtomicUsize,
    /// The server refused a channel: this connection takes no more.
    full: AtomicBool,
    /// When the last channel closed, for the idle timeout.
    last_used: Mutex<Instant>,
    /// The hops this connection runs through, for the error messages.
    via: Vec<String>,
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Live")
            .field("alias", &self.alias)
            .field("channels", &self.channels.load(Ordering::Relaxed))
            .field("open", &self.is_open())
            .finish_non_exhaustive()
    }
}

impl Live {
    /// Whether the session is still open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        !self.handle.is_closed()
    }

    /// Channels open right now.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels.load(Ordering::Relaxed)
    }

    /// Whether another channel may open here.
    fn has_room(&self) -> bool {
        self.is_open() && !self.full.load(Ordering::Relaxed) && self.channels() < MAX_CHANNELS
    }

    /// How long no channel has been open, `None` while one is.
    fn idle_for(&self) -> Option<Duration> {
        (self.channels() == 0).then(|| lock(&self.last_used).elapsed())
    }

    /// Opens a `direct-tcpip` channel to `host:port` through this
    /// connection: the next hop's transport.
    async fn tunnel(self: &Arc<Self>, host: &str, port: u16) -> Result<Tunnel, Error> {
        let token = Token::new(self);
        let channel = self
            .handle
            .channel_open_direct_tcpip(host, u32::from(port), "127.0.0.1", 0)
            .await
            .map_err(|e| message(format!("{}: no tunnel to {host}:{port}: {e}", self.alias)))?;
        Ok(Tunnel {
            stream: channel.into_stream(),
            _token: token,
        })
    }

    /// Tells the server goodbye and closes the session.
    async fn close(&self) {
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "", "English")
            .await;
    }
}

/// Counts a channel open on a connection; dropping it counts the channel
/// closed and marks the time.
pub struct Token {
    live: Arc<Live>,
}

impl Token {
    fn new(live: &Arc<Live>) -> Self {
        live.channels.fetch_add(1, Ordering::Relaxed);
        Self {
            live: Arc::clone(live),
        }
    }

    /// The connection the channel is on.
    #[must_use]
    pub fn live(&self) -> &Arc<Live> {
        &self.live
    }
}

impl Drop for Token {
    fn drop(&mut self) {
        self.live.channels.fetch_sub(1, Ordering::Relaxed);
        *lock(&self.live.last_used) = Instant::now();
    }
}

/// A session channel on a connection, counted while it lives.
pub struct Lease {
    /// The channel.
    pub channel: Channel<Msg>,
    /// The count.
    pub token: Token,
}

/// The stream of a `direct-tcpip` channel, the transport of the next hop;
/// it holds its channel counted on the bastion, so the bastion is never
/// idle while a target behind it is connected.
struct Tunnel {
    stream: ChannelStream<Msg>,
    _token: Token,
}

impl AsyncRead for Tunnel {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for Tunnel {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

/// What a command printed and how it ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// The exit status, 255 when the server sent none.
    pub status: u32,
    /// Standard output.
    pub stdout: Vec<u8>,
    /// Standard error.
    pub stderr: Vec<u8>,
}

impl Output {
    /// The standard error as text, trimmed.
    #[must_use]
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_owned()
    }

    /// The standard output when the command succeeded, else the kernel
    /// error the standard error points at: `NotFound` for a missing path,
    /// `PermissionDenied` for a refusal, the text otherwise.
    ///
    /// # Errors
    ///
    /// As above, for a non-zero status.
    pub fn ok(self) -> Result<Vec<u8>, Error> {
        if self.status == 0 {
            return Ok(self.stdout);
        }
        Err(self.error())
    }

    /// The kernel error for a failed command.
    #[must_use]
    pub fn error(&self) -> Error {
        let text = self.stderr_text();
        let lower = text.to_ascii_lowercase();
        if lower.contains("no such file") || lower.contains("not found") {
            Error::NotFound
        } else if lower.contains("permission denied") || lower.contains("operation not permitted") {
            Error::PermissionDenied
        } else if text.is_empty() {
            message(format!("the command exited with status {}", self.status))
        } else {
            message(text)
        }
    }
}

/// What the pool keeps per host: its connections, its command profile and
/// its default directory once known.
#[derive(Default)]
struct Slot {
    /// The connections, oldest first; the lock is held while one opens,
    /// so a second operation waits for the first's connection instead of
    /// asking the user twice.
    live: tokio::sync::Mutex<Vec<Arc<Live>>>,
    profile: Mutex<Option<Profile>>,
    default_dir: Mutex<Option<Vec<u8>>>,
}

/// The pool: the hosts, the queue to ask the user through, and the
/// connections by alias.
pub struct Pool {
    hosts: Arc<Hosts>,
    prompts: Option<Arc<Prompts>>,
    slots: Mutex<BTreeMap<String, Arc<Slot>>>,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("hosts", &lock(&self.slots).keys().collect::<Vec<_>>())
            .field("prompts", &self.prompts.is_some())
            .finish()
    }
}

impl Pool {
    /// An empty pool over `hosts`, asking the user through `prompts` when
    /// there is one; without one, nothing that needs a question succeeds.
    /// The idle sweeper runs on the runtime for as long as the pool lives.
    #[must_use]
    pub fn new(hosts: Arc<Hosts>, prompts: Option<Arc<Prompts>>) -> Arc<Self> {
        let pool = Arc::new(Self {
            hosts,
            prompts,
            slots: Mutex::new(BTreeMap::new()),
        });
        let weak = Arc::downgrade(&pool);
        ext::runtime::handle().spawn(sweep(weak));
        pool
    }

    /// The hosts.
    #[must_use]
    pub fn hosts(&self) -> &Arc<Hosts> {
        &self.hosts
    }

    /// The platform: the keychain, the home directory.
    #[must_use]
    pub fn platform(&self) -> &Arc<dyn Platform> {
        self.hosts.platform()
    }

    /// The queue the pool asks the user through, when it has one.
    #[must_use]
    pub fn prompts(&self) -> Option<&Arc<Prompts>> {
        self.prompts.as_ref()
    }

    fn slot(&self, alias: &str) -> Arc<Slot> {
        Arc::clone(lock(&self.slots).entry(alias.to_owned()).or_default())
    }

    /// Every connection to `alias` right now, for the tests.
    #[must_use]
    pub fn connections(&self, alias: &str) -> Vec<Arc<Live>> {
        let Some(slot) = lock(&self.slots).get(alias).cloned() else {
            return Vec::new();
        };
        slot.live.try_lock().map(|l| l.clone()).unwrap_or_default()
    }

    /// Whether `alias` has an open connection.
    #[must_use]
    pub fn is_connected(&self, alias: &str) -> bool {
        self.connections(alias).iter().any(|l| l.is_open())
    }

    /// Closes every connection to `alias` and forgets what was learnt
    /// about the host.
    pub fn disconnect(&self, alias: &str) {
        let Some(slot) = lock(&self.slots).remove(alias) else {
            return;
        };
        ext::runtime::handle().spawn(async move {
            let lives = std::mem::take(&mut *slot.live.lock().await);
            for live in lives {
                live.close().await;
            }
        });
    }

    /// Closes the connections of every host not in `keep`, after the host
    /// list changed.
    pub fn retain(&self, keep: &[String]) {
        let gone: Vec<String> = lock(&self.slots)
            .keys()
            .filter(|alias| !keep.contains(alias))
            .cloned()
            .collect();
        for alias in gone {
            self.disconnect(&alias);
        }
    }

    /// A connection to `alias` with room for a channel, opened when there
    /// is none.
    ///
    /// # Errors
    ///
    /// `Error::NotFound` for an alias no source lists; the connection or
    /// authentication error otherwise, naming the hop it happened at.
    pub async fn live(&self, alias: &str) -> Result<Arc<Live>, Error> {
        let host = self.hosts.get(alias).ok_or(Error::NotFound)?;
        self.live_for(&host, Vec::new()).await
    }

    /// [`Pool::live`] for a resolved host; `chain` is the aliases on the
    /// way here, so a `ProxyJump` loop is refused.
    fn live_for(&self, host: &Host, chain: Vec<String>) -> BoxFuture<'_, Result<Arc<Live>, Error>> {
        let host = host.clone();
        Box::pin(async move {
            if chain.contains(&host.alias) {
                return Err(message(format!(
                    "ProxyJump loops back to {}: {}",
                    host.alias,
                    chain.join(" → ")
                )));
            }
            let slot = self.slot(&host.alias);
            let mut lives = slot.live.lock().await;
            lives.retain(|l| l.is_open());
            if let Some(live) = lives.iter().find(|l| l.has_room()) {
                return Ok(Arc::clone(live));
            }
            let live = Arc::new(self.connect(&host, chain).await?);
            lives.push(Arc::clone(&live));
            Ok(live)
        })
    }

    /// Connects and authenticates to `host`, through its jump hosts.
    async fn connect(&self, host: &Host, mut chain: Vec<String>) -> Result<Live, Error> {
        chain.push(host.alias.clone());
        let label = if chain.len() > 1 {
            format!(
                "{} (reached through {})",
                host.alias,
                chain[..chain.len() - 1].join(", ")
            )
        } else {
            host.alias.clone()
        };
        let span = tracing::info_span!("ssh connect", host = %host.alias, address = %host.host, port = host.port);
        let _enter = span.enter();
        let refusal: Refusal = Arc::default();
        let handler = Handler {
            label: label.clone(),
            host: host.host.clone(),
            port: host.port,
            known_hosts: self.hosts.known_hosts_path(),
            prompts: self.prompts.clone(),
            refusal: Arc::clone(&refusal),
        };
        let config = Arc::new(client::Config {
            inactivity_timeout: None,
            keepalive_interval: Some(Duration::from_secs(30)),
            ..client::Config::default()
        });
        let connected = if host.jump.is_empty() {
            let address = (host.host.as_str(), host.port);
            let tcp =
                tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(address))
                    .await
                    .map_err(|_| {
                        message(format!(
                            "{label}: {}:{} did not answer in time",
                            host.host, host.port
                        ))
                    })?
                    .map_err(|e| message(format!("{label}: {}:{}: {e}", host.host, host.port)))?;
            client::connect_stream(config, tcp, handler).await
        } else {
            // Each hop runs over the one before it: the first hop is
            // connected as its own configuration says (its own ProxyJump
            // included), every later one through the hops before it.
            let hops: Vec<Host> = host.jump.iter().map(|h| self.hosts.hop_host(h)).collect();
            let mut bastion = None;
            for (index, hop) in hops.iter().enumerate() {
                let mut effective = hop.clone();
                if index > 0 {
                    effective.jump = host.jump[..index].to_vec();
                }
                let live = self
                    .live_for(&effective, chain.clone())
                    .await
                    .map_err(|e| message(format!("jump host {}: {e}", hop.alias)))?;
                bastion = Some(live);
            }
            let bastion = bastion.ok_or_else(|| message("no jump host"))?;
            let tunnel = bastion.tunnel(&host.host, host.port).await?;
            client::connect_stream(config, tunnel, handler).await
        };
        let mut handle = match connected {
            Ok(handle) => handle,
            Err(error) => {
                if let Some(reason) = lock(&refusal).take() {
                    return Err(message(reason));
                }
                return Err(message(format!("{label}: {error}")));
            }
        };
        if let Err(error) = self.authenticate(&mut handle, host).await {
            let _ = handle
                .disconnect(russh::Disconnect::ByApplication, "", "English")
                .await;
            return Err(error);
        }
        info!("connected");
        Ok(Live {
            alias: host.alias.clone(),
            handle,
            channels: AtomicUsize::new(0),
            full: AtomicBool::new(false),
            last_used: Mutex::new(Instant::now()),
            via: chain,
        })
    }

    /// A secret of the host from the keychain, on the blocking pool.
    async fn secret(&self, alias: &str, field: &'static str) -> Option<String> {
        let platform = Arc::clone(self.platform());
        let key = secret_key(PROVIDER_ID, alias, field);
        ext::runtime::handle()
            .spawn_blocking(move || platform.get_secret(&key))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
            .filter(|s| !s.is_empty())
    }

    /// Stores a secret of the host in the keychain, on the blocking pool.
    async fn remember(&self, alias: &str, field: &'static str, secret: String) {
        let platform = Arc::clone(self.platform());
        let key = secret_key(PROVIDER_ID, alias, field);
        let stored = ext::runtime::handle()
            .spawn_blocking(move || platform.store_secret(&key, &secret))
            .await;
        match stored {
            Ok(Ok(())) => debug!(alias, field, "secret remembered"),
            Ok(Err(error)) => warn!(%error, "secret not stored in the keychain"),
            Err(error) => warn!(%error, "keychain task failed"),
        }
    }

    /// Authenticates `handle` as the host's user: the agent, the identity
    /// files, keyboard-interactive, password.
    async fn authenticate(
        &self,
        handle: &mut client::Handle<Handler>,
        host: &Host,
    ) -> Result<(), Error> {
        let user = host.user.clone().unwrap_or_else(|| self.hosts.env().user);
        if user.is_empty() {
            return Err(message(format!(
                "{}: no user name; set User in ~/.ssh/config or the host editor",
                host.alias
            )));
        }
        let mut refused: Vec<&str> = Vec::new();
        if self.agent_auth(handle, host, &user).await? {
            return Ok(());
        }
        refused.push("agent");
        if self.identity_auth(handle, host, &user).await? {
            return Ok(());
        }
        refused.push("identity files");
        if self.keyboard_interactive_auth(handle, host, &user).await? {
            return Ok(());
        }
        refused.push("keyboard-interactive");
        if self.password_auth(handle, host, &user).await? {
            return Ok(());
        }
        refused.push("password");
        Err(message(format!(
            "{}:{} refused every authentication of {user}: {}",
            host.host,
            host.port,
            refused.join(", ")
        )))
    }

    /// The agent's identities, one by one; `false` without an agent or a
    /// key the server takes.
    async fn agent_auth(
        &self,
        handle: &mut client::Handle<Handler>,
        host: &Host,
        user: &str,
    ) -> Result<bool, Error> {
        let endpoint = match &host.identity_agent {
            Some(IdentityAgent::None) => return Ok(false),
            Some(IdentityAgent::Socket(path)) => Some(path.clone()),
            None => None,
        };
        let stream = match ext::platform::agent::connect(endpoint.as_deref()).await {
            Ok(stream) => stream,
            Err(error) => {
                debug!(%error, "no agent");
                return Ok(false);
            }
        };
        let mut agent = AgentClient::connect(stream);
        let identities = match agent.request_identities().await {
            Ok(identities) => identities,
            Err(error) => {
                debug!(%error, "agent listed no identities");
                return Ok(false);
            }
        };
        for identity in identities {
            let AgentIdentity::PublicKey { key, comment } = identity else {
                continue;
            };
            let hash = if key.algorithm().is_rsa() {
                handle
                    .best_supported_rsa_hash()
                    .await
                    .map_err(ssh_error)?
                    .flatten()
            } else {
                None
            };
            match handle
                .authenticate_publickey_with(user, key.clone(), hash, &mut agent)
                .await
            {
                Ok(result) if result.success() => {
                    info!(comment, "authenticated through the agent");
                    return Ok(true);
                }
                Ok(_) => debug!(comment, "agent key refused"),
                Err(error) => debug!(comment, %error, "agent did not sign"),
            }
        }
        Ok(false)
    }

    /// The host's identity files, then the default ones; an encrypted key
    /// asks for its passphrase, from the keychain first.
    async fn identity_auth(
        &self,
        handle: &mut client::Handle<Handler>,
        host: &Host,
        user: &str,
    ) -> Result<bool, Error> {
        let ssh_dir = self.hosts.env().home.join(".ssh");
        let mut files = host.identity_files.clone();
        for name in DEFAULT_IDENTITIES {
            let path = ssh_dir.join(name);
            if !files.contains(&path) {
                files.push(path);
            }
        }
        for file in files.iter().filter(|f| f.is_file()) {
            let Some(key) = self.load_key(host, file).await else {
                continue;
            };
            let hash = if key.algorithm().is_rsa() {
                handle
                    .best_supported_rsa_hash()
                    .await
                    .map_err(ssh_error)?
                    .flatten()
            } else {
                None
            };
            let result = handle
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map_err(ssh_error)?;
            if result.success() {
                info!(file = %file.display(), "authenticated with an identity file");
                return Ok(true);
            }
            debug!(file = %file.display(), "identity file refused");
        }
        Ok(false)
    }

    /// Reads a private key, asking for its passphrase when it is
    /// encrypted: the keychain's first, then the user's, remembered when
    /// asked to.
    async fn load_key(&self, host: &Host, file: &std::path::Path) -> Option<PrivateKey> {
        let path = file.to_path_buf();
        let plain = ext::runtime::handle()
            .spawn_blocking(move || russh::keys::load_secret_key(&path, None))
            .await
            .ok()?;
        match plain {
            Ok(key) => return Some(key),
            Err(russh::keys::Error::KeyIsEncrypted) => {}
            Err(error) => {
                debug!(file = %file.display(), %error, "identity file not read");
                return None;
            }
        }
        let mut passphrase = self.secret(&host.alias, field::PASSPHRASE).await;
        let mut from_keychain = passphrase.is_some();
        for _ in 0..TRIES {
            let candidate = if let Some(candidate) = passphrase.take() {
                (candidate, false)
            } else {
                let prompts = self.prompts.as_ref()?;
                prompts
                    .secret(
                        format!("Passphrase for {}", file.display()),
                        format!(
                            "The identity file for {} is encrypted. Enter its passphrase.",
                            host.alias
                        ),
                        Some(host.remember_password),
                    )
                    .await?
            };
            let path = file.to_path_buf();
            let (secret, remember) = candidate;
            let attempt = secret.clone();
            let loaded = ext::runtime::handle()
                .spawn_blocking(move || russh::keys::load_secret_key(&path, Some(&attempt)))
                .await
                .ok()?;
            match loaded {
                Ok(key) => {
                    if remember {
                        self.remember(&host.alias, field::PASSPHRASE, secret).await;
                    }
                    return Some(key);
                }
                Err(error) => {
                    debug!(file = %file.display(), %error, from_keychain, "passphrase refused");
                    from_keychain = false;
                }
            }
        }
        None
    }

    /// Keyboard-interactive: the server's prompts answered from the
    /// keychain's password where one fits, else by the user.
    async fn keyboard_interactive_auth(
        &self,
        handle: &mut client::Handle<Handler>,
        host: &Host,
        user: &str,
    ) -> Result<bool, Error> {
        let mut response = handle
            .authenticate_keyboard_interactive_start(user, None)
            .await
            .map_err(ssh_error)?;
        let mut password = self.secret(&host.alias, field::PASSWORD).await;
        let mut rounds = 0;
        loop {
            match response {
                KeyboardInteractiveAuthResponse::Success => {
                    info!("authenticated keyboard-interactively");
                    return Ok(true);
                }
                KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
                KeyboardInteractiveAuthResponse::InfoRequest {
                    name,
                    instructions,
                    prompts,
                } => {
                    rounds += 1;
                    if rounds > TRIES * 2 {
                        return Ok(false);
                    }
                    let mut answers = Vec::with_capacity(prompts.len());
                    for prompt in prompts {
                        let looks_like_password =
                            !prompt.echo && prompt.prompt.to_ascii_lowercase().contains("password");
                        if looks_like_password && let Some(stored) = password.take() {
                            answers.push(stored);
                            continue;
                        }
                        let Some(queue) = &self.prompts else {
                            return Ok(false);
                        };
                        let title = if name.trim().is_empty() {
                            format!("{} asks", host.alias)
                        } else {
                            name.clone()
                        };
                        let mut text = String::new();
                        if !instructions.trim().is_empty() {
                            text.push_str(instructions.trim());
                            text.push_str("\n\n");
                        }
                        text.push_str(prompt.prompt.trim());
                        let remember = looks_like_password.then_some(host.remember_password);
                        let Some((typed, remember_it)) = queue.secret(title, text, remember).await
                        else {
                            return Err(Error::Cancelled);
                        };
                        if remember_it && looks_like_password {
                            self.remember(&host.alias, field::PASSWORD, typed.clone())
                                .await;
                        }
                        answers.push(typed);
                    }
                    response = handle
                        .authenticate_keyboard_interactive_respond(answers)
                        .await
                        .map_err(ssh_error)?;
                }
            }
        }
    }

    /// Password: the keychain's, then the user's, remembered when asked to.
    async fn password_auth(
        &self,
        handle: &mut client::Handle<Handler>,
        host: &Host,
        user: &str,
    ) -> Result<bool, Error> {
        let mut stored = self.secret(&host.alias, field::PASSWORD).await;
        for _ in 0..TRIES {
            let (password, remember) = if let Some(password) = stored.take() {
                (password, false)
            } else {
                let Some(prompts) = &self.prompts else {
                    return Ok(false);
                };
                let Some(typed) = prompts
                    .secret(
                        format!("Password for {user}@{}", host.host),
                        format!(
                            "{} ({}:{}) asks for the password of {user}.",
                            host.alias, host.host, host.port
                        ),
                        Some(host.remember_password),
                    )
                    .await
                else {
                    return Err(Error::Cancelled);
                };
                typed
            };
            let result = handle
                .authenticate_password(user, password.clone())
                .await
                .map_err(ssh_error)?;
            if result.success() {
                info!("authenticated with a password");
                if remember {
                    self.remember(&host.alias, field::PASSWORD, password).await;
                }
                return Ok(true);
            }
            debug!("password refused");
        }
        Ok(false)
    }

    /// Opens a session channel on a connection to `alias`: a second
    /// connection when the first is at the session limit or refuses the
    /// open.
    ///
    /// # Errors
    ///
    /// The connection error, or the server's refusal of the channel.
    pub async fn lease(&self, alias: &str) -> Result<Lease, Error> {
        let mut last = None;
        for _ in 0..2 {
            let live = self.live(alias).await?;
            let token = Token::new(&live);
            match live.handle.channel_open_session().await {
                Ok(channel) => return Ok(Lease { channel, token }),
                Err(russh::Error::ChannelOpenFailure(failure)) => {
                    debug!(
                        alias,
                        ?failure,
                        "channel refused; a second connection opens"
                    );
                    live.full.store(true, Ordering::Relaxed);
                    drop(token);
                    last = Some(message(format!("{alias}: channel refused: {failure:?}")));
                }
                Err(error) => return Err(ssh_error(error)),
            }
        }
        Err(last.unwrap_or_else(|| message(format!("{alias}: no channel"))))
    }

    /// Runs `command` on `alias` and collects what it printed and its exit
    /// status.
    ///
    /// # Errors
    ///
    /// The connection or channel error; a non-zero status is in the
    /// output, not an error.
    pub async fn run(&self, alias: &str, command: &[u8]) -> Result<Output, Error> {
        let Lease { mut channel, token } = self.lease(alias).await?;
        channel
            .exec(true, command.to_vec())
            .await
            .map_err(ssh_error)?;
        let mut output = Output {
            status: 255,
            ..Output::default()
        };
        let mut status = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => output.stdout.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, .. } => output.stderr.extend_from_slice(&data),
                ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                ChannelMsg::ExitSignal { signal_name, .. } => {
                    status = Some(128);
                    debug!(?signal_name, "command killed by a signal");
                }
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        if let Some(status) = status {
            output.status = status;
        }
        drop(token);
        Ok(output)
    }

    /// Runs `command` and returns its standard output when it succeeded.
    ///
    /// # Errors
    ///
    /// [`Output::error`] for a failed command; the connection error.
    pub async fn run_ok(&self, alias: &str, command: &[u8]) -> Result<Vec<u8>, Error> {
        self.run(alias, command).await?.ok()
    }

    /// The command profile of `alias`, probed on first use.
    ///
    /// # Errors
    ///
    /// The connection error, or `Error::Unsupported` for a host whose
    /// userland the plugin does not know.
    pub async fn profile(&self, alias: &str) -> Result<Profile, Error> {
        let slot = self.slot(alias);
        if let Some(profile) = *lock(&slot.profile) {
            return Ok(profile);
        }
        let output = self.run(alias, profile::PROBE.as_bytes()).await?;
        let probe = Probe::parse(&output.stdout);
        let profile = probe.profile()?;
        info!(alias, uname = %probe.uname, %profile, "host probed");
        *lock(&slot.profile) = Some(profile);
        Ok(profile)
    }

    /// Forgets the profile of `alias`, so the next operation probes again:
    /// what a command failing in a way the profile does not expect asks
    /// for.
    pub fn reprobe(&self, alias: &str) {
        *lock(&self.slot(alias).profile) = None;
    }

    /// The directory `ssh:/<alias>` stands for: the host's default
    /// directory, or the remote home when it has none, absolute.
    ///
    /// # Errors
    ///
    /// The connection error.
    pub async fn default_dir(&self, alias: &str) -> Result<Vec<u8>, Error> {
        let slot = self.slot(alias);
        if let Some(dir) = lock(&slot.default_dir).clone() {
            return Ok(dir);
        }
        let host = self.hosts.get(alias).ok_or(Error::NotFound)?;
        let configured = host
            .default_dir
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty());
        let dir = match configured {
            Some(dir) if dir.starts_with('/') => dir.as_bytes().to_vec(),
            other => {
                let home = self
                    .run_ok(alias, b"cd && pwd -P")
                    .await?
                    .into_iter()
                    .take_while(|b| *b != b'\n')
                    .collect::<Vec<u8>>();
                let home = if home.is_empty() { b"/".to_vec() } else { home };
                match other {
                    Some(relative) => {
                        crate::shell::join(&home, relative.trim_start_matches("~/").as_bytes())
                    }
                    None => home,
                }
            }
        };
        *lock(&slot.default_dir) = Some(dir.clone());
        Ok(dir)
    }
}

/// Closes the connections that have had no channel for longer than the
/// idle timeout, every so often, for as long as the pool lives.
async fn sweep(pool: Weak<Pool>) {
    loop {
        tokio::time::sleep(SWEEP_EVERY).await;
        let Some(pool) = pool.upgrade() else {
            return;
        };
        let timeout = pool.hosts.settings().idle_timeout;
        let slots: Vec<Arc<Slot>> = lock(&pool.slots).values().cloned().collect();
        for slot in slots {
            let Ok(mut lives) = slot.live.try_lock() else {
                continue;
            };
            let mut closing = Vec::new();
            lives.retain(|live| {
                let idle = live.idle_for().is_some_and(|idle| idle >= timeout) || !live.is_open();
                if idle {
                    closing.push(Arc::clone(live));
                }
                !idle
            });
            drop(lives);
            for live in closing {
                debug!(alias = %live.alias, via = ?live.via, "idle connection closed");
                live.close().await;
            }
        }
    }
}

/// The connector the application talks to: the host editor's fields, the
/// list of Rick's hosts, and *Connect* and *Disconnect* on a row.
pub struct SshConnector {
    pool: Arc<Pool>,
}

impl std::fmt::Debug for SshConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshConnector")
            .field("pool", &self.pool)
            .finish()
    }
}

impl SshConnector {
    /// The connector over `pool`.
    #[must_use]
    pub fn new(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Connector for SshConnector {
    fn provider(&self) -> ProviderId {
        PROVIDER_ID
    }

    fn schema(&self) -> ConnectionSchema {
        let mut schema = self.pool.hosts().schema();
        schema.fields.push(ext::ConnectionField {
            key: field::PASSPHRASE,
            label: "Identity file passphrase",
            kind: ext::ConnectionFieldKind::Secret,
            required: false,
        });
        schema
    }

    /// Replaces Rick's hosts; the connections of hosts that went, or whose
    /// settings changed, are closed.
    fn update(&self, connections: Vec<Connection>) {
        let before = self.pool.hosts().all();
        self.pool.hosts().update(connections);
        let after = self.pool.hosts().all();
        let keep: Vec<String> = after
            .iter()
            .filter(|h| before.iter().any(|b| b == *h))
            .map(|h| h.alias.clone())
            .collect();
        self.pool.retain(&keep);
    }

    async fn connect(&self, name: &str) -> Result<(), Error> {
        self.pool.live(name).await?;
        self.pool.profile(name).await?;
        Ok(())
    }

    fn disconnect(&self, name: &str) {
        self.pool.disconnect(name);
    }

    fn is_connected(&self, name: &str) -> bool {
        self.pool.is_connected(name)
    }
}
