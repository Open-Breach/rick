//! The connections: what the application hands the connector, and one
//! client per account, built on first use.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ext::platform::Platform;
use ext::{
    Connection, ConnectionField, ConnectionFieldKind, ConnectionSchema, Connector, async_trait,
    secret_key,
};
use types::{Error, ProviderId};

use crate::PROVIDER_ID;
use crate::client::{S3Client, message};

/// The field keys of the *Add connection…* dialogue and `rick.toml`.
pub mod field {
    /// The endpoint: scheme, host and port; AWS's own when empty.
    pub const ENDPOINT: &str = "endpoint";
    /// The region, `us-east-1` by default; `auto` for R2.
    pub const REGION: &str = "region";
    /// The access key id.
    pub const ACCESS_KEY: &str = "access_key";
    /// The secret access key, in the keychain.
    pub const SECRET_KEY: &str = "secret_key";
    /// Path-style addressing (`endpoint/bucket/key`) rather than the
    /// bucket as a host name; on by default, what `MinIO` and R2 take.
    pub const PATH_STYLE: &str = "path_style";
}

/// The fields of the dialogue, in display order.
#[must_use]
pub fn schema() -> ConnectionSchema {
    ConnectionSchema {
        title: "S3",
        fields: vec![
            ConnectionField {
                key: field::ENDPOINT,
                label: "Endpoint (empty for AWS)",
                kind: ConnectionFieldKind::Text {
                    default: String::new(),
                },
                required: false,
            },
            ConnectionField {
                key: field::REGION,
                label: "Region",
                kind: ConnectionFieldKind::Text {
                    default: "us-east-1".to_owned(),
                },
                required: false,
            },
            ConnectionField {
                key: field::ACCESS_KEY,
                label: "Access key",
                kind: ConnectionFieldKind::Text {
                    default: String::new(),
                },
                required: true,
            },
            ConnectionField {
                key: field::SECRET_KEY,
                label: "Secret key",
                kind: ConnectionFieldKind::Secret,
                required: true,
            },
            ConnectionField {
                key: field::PATH_STYLE,
                label: "Path-style addressing",
                kind: ConnectionFieldKind::Bool { default: true },
                required: false,
            },
        ],
    }
}

/// One connection as the user made it, and its client once built.
struct Conn {
    settings: Connection,
    client: tokio::sync::Mutex<Option<Arc<S3Client>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The connector: the connections the application handed over and their
/// clients.
pub struct S3Connections {
    platform: Arc<dyn Platform>,
    schema: ConnectionSchema,
    conns: Mutex<BTreeMap<String, Arc<Conn>>>,
}

impl std::fmt::Debug for S3Connections {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Connections")
            .field("names", &self.names())
            .finish_non_exhaustive()
    }
}

impl S3Connections {
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

    /// The client of `name`, built on first use with the secret from the
    /// keychain.
    pub(crate) async fn client(&self, name: &str) -> Result<Arc<S3Client>, Error> {
        let conn = self.conn(name)?;
        let mut slot = conn.client.lock().await;
        if let Some(client) = slot.as_ref() {
            return Ok(Arc::clone(client));
        }
        let settings = &conn.settings;
        let access_key = settings
            .get(field::ACCESS_KEY)
            .ok_or_else(|| message(format!("connection {name} has no access key")))?;
        let platform = Arc::clone(&self.platform);
        let key = secret_key(PROVIDER_ID, name, field::SECRET_KEY);
        let secret = ext::runtime::handle()
            .spawn_blocking(move || platform.get_secret(&key))
            .await
            .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))))?
            .ok_or_else(|| {
                message(format!(
                    "connection {name} has no secret key in the keychain"
                ))
            })?;
        let client = Arc::new(S3Client::new(
            settings.get(field::ENDPOINT).unwrap_or_default(),
            settings.get(field::REGION).unwrap_or_default(),
            access_key,
            &secret,
            settings.get(field::PATH_STYLE) != Some("false"),
        )?);
        *slot = Some(Arc::clone(&client));
        Ok(client)
    }
}

#[async_trait]
impl Connector for S3Connections {
    fn provider(&self) -> ProviderId {
        PROVIDER_ID
    }

    fn schema(&self) -> &ConnectionSchema {
        &self.schema
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

    /// Builds the client and lists the buckets, so wrong credentials are
    /// reported now.
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
