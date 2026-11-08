//! The connections: what the application hands the connector, and one
//! client per project, built on first use.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ext::platform::Platform;
use ext::{
    Connection, ConnectionField, ConnectionFieldKind, ConnectionSchema, Connector, async_trait,
    secret_key,
};
use types::{Error, ProviderId};

use crate::PROVIDER_ID;
use crate::auth::ServiceAccount;
use crate::client::{GcsClient, message};

/// The field keys of the *Add connection…* dialogue and `rick.toml`.
pub mod field {
    /// The project id.
    pub const PROJECT: &str = "project";
    /// The service account's key: the JSON file's contents, or the path
    /// of the file, in the keychain; empty for an emulator.
    pub const KEY: &str = "key";
    /// An emulator's endpoint (scheme, host and port); the service when
    /// empty.
    pub const ENDPOINT: &str = "endpoint";
}

/// The fields of the dialogue, in display order.
#[must_use]
pub fn schema() -> ConnectionSchema {
    ConnectionSchema {
        title: "Google Cloud Storage",
        fields: vec![
            ConnectionField {
                key: field::PROJECT,
                label: "Project",
                kind: ConnectionFieldKind::Text {
                    default: String::new(),
                },
                required: true,
            },
            ConnectionField {
                key: field::KEY,
                label: "Service account key (JSON, or its file)",
                kind: ConnectionFieldKind::Secret,
                required: false,
            },
            ConnectionField {
                key: field::ENDPOINT,
                label: "Endpoint (empty for Google)",
                kind: ConnectionFieldKind::Text {
                    default: String::new(),
                },
                required: false,
            },
        ],
    }
}

/// One connection as the user made it, and its client once built.
struct Conn {
    settings: Connection,
    client: tokio::sync::Mutex<Option<Arc<GcsClient>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The connector: the connections the application handed over and their
/// clients.
pub struct GcsConnections {
    platform: Arc<dyn Platform>,
    schema: ConnectionSchema,
    conns: Mutex<BTreeMap<String, Arc<Conn>>>,
}

impl std::fmt::Debug for GcsConnections {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcsConnections")
            .field("names", &self.names())
            .finish_non_exhaustive()
    }
}

impl GcsConnections {
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

    /// The client of `name`, built on first use with the key from the
    /// keychain: the JSON itself, or the path of the file holding it.
    pub(crate) async fn client(&self, name: &str) -> Result<Arc<GcsClient>, Error> {
        let conn = self.conn(name)?;
        let mut slot = conn.client.lock().await;
        if let Some(client) = slot.as_ref() {
            return Ok(Arc::clone(client));
        }
        let settings = &conn.settings;
        let project = settings
            .get(field::PROJECT)
            .ok_or_else(|| message(format!("connection {name} has no project")))?;
        let platform = Arc::clone(&self.platform);
        let key = secret_key(PROVIDER_ID, name, field::KEY);
        let secret = ext::runtime::handle()
            .spawn_blocking(move || -> Result<Option<String>, Error> {
                let Some(secret) = platform.get_secret(&key)? else {
                    return Ok(None);
                };
                let trimmed = secret.trim();
                if trimmed.starts_with('{') {
                    Ok(Some(trimmed.to_owned()))
                } else {
                    std::fs::read_to_string(trimmed)
                        .map(Some)
                        .map_err(|e| message(format!("key file {trimmed}: {e}")))
                }
            })
            .await
            .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))))?;
        let account = secret
            .map(|json| ServiceAccount::parse(&json))
            .transpose()?;
        let client = Arc::new(GcsClient::new(
            project,
            settings.get(field::ENDPOINT).unwrap_or_default(),
            account,
        )?);
        *slot = Some(Arc::clone(&client));
        Ok(client)
    }
}

#[async_trait]
impl Connector for GcsConnections {
    fn provider(&self) -> ProviderId {
        PROVIDER_ID
    }

    fn schema(&self) -> ConnectionSchema {
        self.schema.clone()
    }

    /// Keeps the client of a connection whose settings did not change,
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
                    client: tokio::sync::Mutex::new(None),
                })
            });
            conns.insert(connection.name, conn);
        }
    }

    /// Builds the client and lists the buckets, so a wrong key is reported
    /// now.
    async fn connect(&self, name: &str) -> Result<(), Error> {
        let client = self.client(name).await?;
        client.list_buckets().await.map(|_| ())
    }

    fn disconnect(&self, name: &str) {
        if let Ok(conn) = self.conn(name)
            && let Ok(mut slot) = conn.client.try_lock()
        {
            *slot = None;
        }
    }

    fn is_connected(&self, name: &str) -> bool {
        self.conn(name)
            .is_ok_and(|conn| conn.client.try_lock().is_ok_and(|slot| slot.is_some()))
    }
}
