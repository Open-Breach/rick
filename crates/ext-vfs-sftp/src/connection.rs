//! The connections: what the application hands the connector, and one SSH
//! session per connection, opened on first use.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use ext::Text;
use ext::platform::Platform;
use ext::{
    Connection, ConnectionField, ConnectionFieldKind, ConnectionSchema, Connector, async_trait,
    secret_key,
};
use russh::client;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;
use russh_sftp::client::rawsession::RawSftpSession;
use tracing::{debug, info, warn};
use types::{Error, ProviderId};

use crate::{ID, PROVIDER_ID};

/// The field keys of the *Add connection…* dialogue and `rick.toml`.
pub mod field {
    /// The host name or address.
    pub const HOST: &str = "host";
    /// The port, 22 by default.
    pub const PORT: &str = "port";
    /// The user name.
    pub const USER: &str = "user";
    /// An OpenSSH private key file; the password is used without one.
    pub const KEY_FILE: &str = "key_file";
    /// The key's passphrase, in the keychain.
    pub const PASSPHRASE: &str = "passphrase";
    /// The password, in the keychain.
    pub const PASSWORD: &str = "password";
    /// The remote folder the connection opens at; the login folder when
    /// empty.
    pub const FOLDER: &str = "folder";
}

/// How long the connection may take to be established.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// The fields of the dialogue, in display order.
#[must_use]
pub fn schema() -> ConnectionSchema {
    ConnectionSchema {
        title: Text("sftp-sftp"),
        fields: vec![
            ConnectionField {
                key: field::HOST,
                label: Text("sftp-host"),
                kind: ConnectionFieldKind::Text {
                    default: String::new(),
                },
                required: true,
            },
            ConnectionField {
                key: field::PORT,
                label: Text("sftp-port"),
                kind: ConnectionFieldKind::Integer {
                    default: 22,
                    min: 1,
                    max: 65535,
                },
                required: false,
            },
            ConnectionField {
                key: field::USER,
                label: Text("sftp-user"),
                kind: ConnectionFieldKind::Text {
                    default: String::new(),
                },
                required: true,
            },
            ConnectionField {
                key: field::KEY_FILE,
                label: Text("sftp-private-key-file"),
                kind: ConnectionFieldKind::Path,
                required: false,
            },
            ConnectionField {
                key: field::PASSPHRASE,
                label: Text("sftp-key-passphrase"),
                kind: ConnectionFieldKind::Secret,
                required: false,
            },
            ConnectionField {
                key: field::PASSWORD,
                label: Text("sftp-password"),
                kind: ConnectionFieldKind::Secret,
                required: false,
            },
            ConnectionField {
                key: field::FOLDER,
                label: Text("sftp-remote-folder"),
                kind: ConnectionFieldKind::Text {
                    default: String::new(),
                },
                required: false,
            },
        ],
    }
}

/// An error of the SSH or SFTP libraries, as the kernel error.
pub(crate) fn ext_error(error: impl std::error::Error + Send + Sync + 'static) -> Error {
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

/// What the host-key check decided, for the error message when it
/// refused.
#[derive(Default)]
struct KeyCheck {
    refused: Option<String>,
}

/// The russh client handler: checks the server's host key against the
/// user's `known_hosts` with OpenSSH semantics (hashed entries,
/// port-qualified names), learning an unknown host and refusing a changed
/// key.
struct Handler {
    host: String,
    port: u16,
    known_hosts: PathBuf,
    check: Arc<Mutex<KeyCheck>>,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        std::future::ready(Ok(self.verify(server_public_key)))
    }
}

impl Handler {
    /// The host-key decision of [`client::Handler::check_server_key`]: a
    /// known key passes, an unknown one is learnt and recorded, a changed
    /// one or an unreadable `known_hosts` is refused with the reason kept
    /// for the connector to report. Nothing here awaits.
    fn verify(&mut self, server_public_key: &PublicKeyOrCertificate) -> bool {
        let key = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => cert.public_key().clone().into(),
        };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let known = if self.known_hosts.is_file() {
            russh::keys::check_known_hosts_path(&self.host, self.port, &key, &self.known_hosts)
        } else {
            Ok(false)
        };
        match known {
            Ok(true) => true,
            Ok(false) => {
                info!(host = %self.host, port = self.port, %fingerprint, "new host key learnt");
                if let Some(dir) = self.known_hosts.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(error) = russh::keys::known_hosts::learn_known_hosts_path(
                    &self.host,
                    self.port,
                    &key,
                    &self.known_hosts,
                ) {
                    warn!(%error, "host key not recorded");
                }
                true
            }
            Err(russh::keys::Error::KeyChanged { line }) => {
                lock(&self.check).refused = Some(format!(
                    "the host key of {}:{} changed (now {fingerprint}); line {line} of {} holds another key",
                    self.host,
                    self.port,
                    self.known_hosts.display()
                ));
                false
            }
            Err(error) => {
                lock(&self.check).refused = Some(format!("known_hosts could not be read: {error}"));
                false
            }
        }
    }
}

/// One open connection: the SSH session and two SFTP sessions on it, the
/// high-level one for files and metadata and a raw one whose `readdir`
/// streams a directory packet by packet.
pub(crate) struct Live {
    handle: client::Handle<Handler>,
    pub(crate) sftp: SftpSession,
    pub(crate) raw: RawSftpSession,
    /// The remote folder the connection's root stands for, absolute.
    pub(crate) root: String,
}

impl Live {
    /// Whether the SSH session is still open.
    fn is_open(&self) -> bool {
        !self.handle.is_closed()
    }

    /// The absolute remote path of `rest`, a path below the connection's
    /// root with `/` separators.
    pub(crate) fn remote(&self, rest: &str) -> String {
        if rest.is_empty() {
            self.root.clone()
        } else if self.root == "/" {
            format!("/{rest}")
        } else {
            format!("{}/{rest}", self.root)
        }
    }
}

/// One connection as the user made it, and its session when open.
struct Conn {
    settings: Connection,
    live: tokio::sync::Mutex<Option<Arc<Live>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The connector: the connections the application handed over and their
/// sessions.
pub struct SftpConnections {
    platform: Arc<dyn Platform>,
    schema: ConnectionSchema,
    conns: Mutex<BTreeMap<String, Arc<Conn>>>,
}

impl std::fmt::Debug for SftpConnections {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SftpConnections")
            .field("names", &self.names())
            .finish_non_exhaustive()
    }
}

impl SftpConnections {
    /// No connections yet; the application hands them over through
    /// [`Connector::update`].
    #[must_use]
    pub fn new(platform: Arc<dyn Platform>) -> Self {
        Self {
            platform,
            schema: schema(),
            conns: Mutex::new(BTreeMap::new()),
        }
    }

    /// The connections' names, sorted.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        lock(&self.conns).keys().cloned().collect()
    }

    fn conn(&self, name: &str) -> Result<Arc<Conn>, Error> {
        lock(&self.conns).get(name).cloned().ok_or(Error::NotFound)
    }

    /// The open session of `name`, connecting first when there is none or
    /// the last one closed.
    pub(crate) async fn session(&self, name: &str) -> Result<Arc<Live>, Error> {
        let conn = self.conn(name)?;
        let mut slot = conn.live.lock().await;
        if let Some(live) = slot.as_ref()
            && live.is_open()
        {
            return Ok(Arc::clone(live));
        }
        let live = Arc::new(self.open(&conn.settings).await?);
        *slot = Some(Arc::clone(&live));
        Ok(live)
    }

    /// A secret of the connection from the keychain, on the blocking pool.
    async fn secret(&self, name: &str, field: &'static str) -> Result<Option<String>, Error> {
        let platform = Arc::clone(&self.platform);
        let key = secret_key(PROVIDER_ID, name, field);
        ext::runtime::handle()
            .spawn_blocking(move || platform.get_secret(&key))
            .await
            .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))))
    }

    /// Connects and authenticates, then opens the SFTP sessions.
    async fn open(&self, settings: &Connection) -> Result<Live, Error> {
        let name = settings.name.as_str();
        let host = settings
            .get(field::HOST)
            .ok_or_else(|| message(format!("connection {name} has no host")))?
            .to_owned();
        let port: u16 = settings
            .get(field::PORT)
            .map_or(Ok(22), str::parse)
            .map_err(|_| message(format!("connection {name} has a port that is not a number")))?;
        let user = settings
            .get(field::USER)
            .ok_or_else(|| message(format!("connection {name} has no user")))?
            .to_owned();
        let span = tracing::info_span!("sftp connect", connection = name, %host, port);
        let _enter = span.enter();

        let check = Arc::new(Mutex::new(KeyCheck::default()));
        let handler = Handler {
            host: host.clone(),
            port,
            known_hosts: self.platform.home_dir().join(".ssh").join("known_hosts"),
            check: Arc::clone(&check),
        };
        let config = Arc::new(client::Config {
            inactivity_timeout: None,
            keepalive_interval: Some(Duration::from_secs(30)),
            ..client::Config::default()
        });
        let connecting = client::connect(config, (host.as_str(), port), handler);
        let mut handle = match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
            Ok(Ok(handle)) => handle,
            Ok(Err(error)) => {
                if let Some(refused) = lock(&check).refused.take() {
                    return Err(message(refused));
                }
                return Err(ext_error(error));
            }
            Err(_) => return Err(message(format!("{host}:{port} did not answer in time"))),
        };

        let authenticated = if let Some(key_file) = settings.get(field::KEY_FILE) {
            let passphrase = self.secret(name, field::PASSPHRASE).await?;
            let key = russh::keys::load_secret_key(key_file, passphrase.as_deref())
                .map_err(|e| message(format!("key file {key_file}: {e}")))?;
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .map_err(ext_error)?
                .flatten();
            handle
                .authenticate_publickey(
                    user.clone(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await
                .map_err(ext_error)?
                .success()
        } else {
            let password = self.secret(name, field::PASSWORD).await?.ok_or_else(|| {
                message(format!(
                    "connection {name} has neither a key file nor a password"
                ))
            })?;
            handle
                .authenticate_password(user.clone(), password)
                .await
                .map_err(ext_error)?
                .success()
        };
        if !authenticated {
            let _ = handle
                .disconnect(russh::Disconnect::ByApplication, "", "English")
                .await;
            return Err(message(format!(
                "{host}:{port} refused the credentials of {user}"
            )));
        }

        let sftp = sftp_channel(&handle).await?;
        let sftp = SftpSession::new(sftp).await.map_err(ext_error)?;
        let raw = RawSftpSession::new(sftp_channel(&handle).await?);
        raw.init().await.map_err(ext_error)?;
        let root = match settings.get(field::FOLDER) {
            Some(folder) => sftp.canonicalize(folder).await.map_err(ext_error)?,
            None => sftp.canonicalize(".").await.map_err(ext_error)?,
        };
        debug!(%root, "connected");
        Ok(Live {
            handle,
            sftp,
            raw,
            root,
        })
    }
}

/// A channel with the SFTP subsystem requested, as a stream.
async fn sftp_channel(
    handle: &client::Handle<Handler>,
) -> Result<russh::ChannelStream<client::Msg>, Error> {
    let channel = handle.channel_open_session().await.map_err(ext_error)?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(ext_error)?;
    Ok(channel.into_stream())
}

#[async_trait]
impl Connector for SftpConnections {
    fn provider(&self) -> ProviderId {
        PROVIDER_ID
    }

    fn schema(&self) -> ConnectionSchema {
        self.schema.clone()
    }

    /// Keeps the session of a connection whose settings did not change,
    /// drops the rest.
    fn update(&self, connections: Vec<Connection>) {
        let mut conns = lock(&self.conns);
        let old = std::mem::take(&mut *conns);
        for connection in connections {
            let kept = old
                .get(&connection.name)
                .filter(|c| c.settings == connection)
                .cloned();
            let conn = kept.unwrap_or_else(|| {
                Arc::new(Conn {
                    settings: connection.clone(),
                    live: tokio::sync::Mutex::new(None),
                })
            });
            conns.insert(connection.name, conn);
        }
        for (name, conn) in old {
            if !conns.contains_key(&name) {
                close(&conn);
            }
        }
    }

    async fn connect(&self, name: &str) -> Result<(), Error> {
        self.session(name).await.map(|_| ())
    }

    fn disconnect(&self, name: &str) {
        if let Ok(conn) = self.conn(name) {
            close(&conn);
        }
    }

    fn is_connected(&self, name: &str) -> bool {
        self.conn(name).is_ok_and(|conn| {
            conn.live
                .try_lock()
                .is_ok_and(|slot| slot.as_ref().is_some_and(|live| live.is_open()))
        })
    }
}

/// Drops a connection's session and tells the server, off the caller's
/// thread.
fn close(conn: &Arc<Conn>) {
    let conn = Arc::clone(conn);
    ext::runtime::handle().spawn(async move {
        let live = conn.live.lock().await.take();
        if let Some(live) = live {
            let _ = live.sftp.close().await;
            let _ = live
                .handle
                .disconnect(russh::Disconnect::ByApplication, "", "English")
                .await;
        }
    });
}
