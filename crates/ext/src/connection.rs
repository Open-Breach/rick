//! Connections: how a provider that is reached over a network (an SFTP
//! host, an object-storage account) is set up from the side panel.
//!
//! Such a provider lists its connections as the folders at its root, one
//! row per connection in the panel's Network or Cloud section, and
//! registers a [`Connector`] besides its `Provider`: the fields its *Add
//! connection…* and *Edit…* dialogues ask for ([`ConnectionSchema`],
//! computed on each call, so a choice among the other connections stays
//! current), and the methods the application calls when the list changes
//! or the user picks *Connect*, *Disconnect* or *Remove* on a row. The application owns the
//! list: the non-secret fields go into `rick.toml`, a field of
//! [`ConnectionFieldKind::Secret`] into the platform keychain under
//! [`secret_key`], and the connector is handed the list through
//! [`Connector::update`] at start and after every change. The connector
//! reads a secret through `platform::Secrets` when it needs one, never
//! from a file.
//!
//! The dialogue is one generic form, and a connector can make it react
//! (API 24): after each edit the dialogue hands [`Connector::react`] the
//! form as it stands, a [`ConnectionForm`] that also says whether the user
//! is still typing, and applies the
//! [`ConnectionReaction`] that comes back. A reaction can *fill* fields,
//! which only takes on a field the user has not edited by hand; *rewrite*
//! fields, for input the user gave in another shape (an address pasted
//! whole, of which one field keeps a part and another the rest); and put a
//! short *note* by a field, such as what the connector recognised. A
//! connector that does not override `react` has a form that stays as
//! typed.
//!
//! # Worked example
//!
//! A connector whose provider knows one field and holds no session.
//!
//! ```
//! use std::collections::BTreeMap;
//! use std::sync::{Arc, Mutex};
//!
//! use ext::{
//!     Connection, ConnectionField, ConnectionSchema, Connector, ConnectionFieldKind, Text,
//!     async_trait, secret_key,
//! };
//! use types::{Error, ProviderId};
//!
//! const ID: ProviderId = ProviderId("demo");
//!
//! struct Demo {
//!     schema: ConnectionSchema,
//!     connections: Mutex<Vec<Connection>>,
//! }
//!
//! #[async_trait]
//! impl Connector for Demo {
//!     fn provider(&self) -> ProviderId {
//!         ID
//!     }
//!     fn schema(&self) -> ConnectionSchema {
//!         self.schema.clone()
//!     }
//!     fn update(&self, connections: Vec<Connection>) {
//!         *self.connections.lock().unwrap() = connections;
//!     }
//!     async fn connect(&self, name: &str) -> Result<(), Error> {
//!         let known = self.connections.lock().unwrap().iter().any(|c| c.name == name);
//!         if known { Ok(()) } else { Err(Error::NotFound) }
//!     }
//!     fn disconnect(&self, _name: &str) {}
//!     fn is_connected(&self, _name: &str) -> bool {
//!         false
//!     }
//! }
//!
//! let demo = Demo {
//!     schema: ConnectionSchema {
//!         title: Text("demo"),
//!         fields: vec![
//!             ConnectionField { key: "host", label: Text("host"), kind: ConnectionFieldKind::Text { default: String::new() }, required: true },
//!             ConnectionField { key: "token", label: Text("token"), kind: ConnectionFieldKind::Secret, required: false },
//!         ],
//!     },
//!     connections: Mutex::new(Vec::new()),
//! };
//! demo.update(vec![Connection {
//!     name: "work".into(),
//!     settings: BTreeMap::from([("host".to_owned(), "example.org".to_owned())]),
//! }]);
//! assert_eq!(secret_key(ID, "work", "token"), "demo/work/token");
//! assert!(demo.schema().secret_fields().eq(["token"]));
//! runtime::block_on(async {
//!     assert!(demo.connect("work").await.is_ok());
//!     assert!(matches!(demo.connect("home").await, Err(Error::NotFound)));
//! });
//! let _ = Arc::new(demo);
//! ```
//!
//! # Worked example: a form that reacts
//!
//! A connector that knows the port of a host it recognises: it fills the
//! port, says by the host what it saw, and takes a port typed onto the
//! host (`example.org:8443`) into the port field.
//!
//! ```
//! use std::collections::{BTreeMap, BTreeSet};
//!
//! use ext::{
//!     Connection, ConnectionForm, ConnectionReaction, ConnectionSchema, Connector, Text,
//!     async_trait,
//! };
//! use types::{Error, ProviderId};
//!
//! struct Demo;
//!
//! #[async_trait]
//! impl Connector for Demo {
//!     fn provider(&self) -> ProviderId {
//!         ProviderId("demo")
//!     }
//!     fn schema(&self) -> ConnectionSchema {
//!         ConnectionSchema { title: Text("demo"), fields: Vec::new() }
//!     }
//!     fn update(&self, _connections: Vec<Connection>) {}
//!     async fn connect(&self, _name: &str) -> Result<(), Error> {
//!         Ok(())
//!     }
//!     fn disconnect(&self, _name: &str) {}
//!     fn is_connected(&self, _name: &str) -> bool {
//!         false
//!     }
//!
//!     fn react(&self, form: &ConnectionForm<'_>) -> ConnectionReaction {
//!         let mut reaction = ConnectionReaction::default();
//!         let host = form.get("host");
//!         // Input in another shape: the port typed onto the host, taken
//!         // apart once the user has finished with the field.
//!         if let Some((name, port)) = host.split_once(':').filter(|_| form.settled) {
//!             reaction.rewrite.push(("host", name.to_owned()));
//!             reaction.rewrite.push(("port", port.to_owned()));
//!             return reaction;
//!         }
//!         if host.ends_with(".example.org") {
//!             // Taken only while the user has not typed a port.
//!             reaction.fill.push(("port", "8443".to_owned()));
//!             // A real connector words this with `t!`.
//!             reaction.notes.push(("host", "Example Cloud".to_owned()));
//!         }
//!         reaction
//!     }
//! }
//!
//! let mut values = BTreeMap::from([("host".to_owned(), "eu.example.org".to_owned())]);
//! let edited = BTreeSet::from(["host".to_owned()]);
//! let react = |values: &BTreeMap<String, String>, edited: &BTreeSet<String>| {
//!     Demo.react(&ConnectionForm { values, edited, changed: Some("host"), settled: true })
//! };
//! let reaction = react(&values, &edited);
//! assert_eq!(reaction.fill, [("port", "8443".to_owned())]);
//! assert_eq!(reaction.notes, [("host", "Example Cloud".to_owned())]);
//!
//! // The dialogue applies it: a fill only where the user has not typed.
//! assert_eq!(reaction.apply(&mut values, &edited), ["port"]);
//! assert_eq!(values["port"], "8443");
//! let typed = BTreeSet::from(["host".to_owned(), "port".to_owned()]);
//! values.insert("port".to_owned(), "22".to_owned());
//! assert!(react(&values, &typed).apply(&mut values, &typed).is_empty());
//! assert_eq!(values["port"], "22");
//!
//! // A port typed onto the host is the user's own input, taken always.
//! values.insert("host".to_owned(), "eu.example.org:9000".to_owned());
//! assert_eq!(react(&values, &typed).apply(&mut values, &typed), ["host", "port"]);
//! assert_eq!(values["port"], "9000");
//! ```

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use types::{Error, ProviderId};

/// The section of the side panel a kind of connection is added in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionSection {
    /// Network: hosts reached over a network protocol, such as SSH.
    Network,
    /// Cloud: accounts of an object store.
    Cloud,
}

impl ConnectionSection {
    /// Both sections, in the side panel's order.
    pub const ALL: [Self; 2] = [Self::Network, Self::Cloud];

    /// The section's name in `plugin.toml`: `network` or `cloud`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Cloud => "cloud",
        }
    }

    /// The section `name` spells; `None` for anything else.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.name() == name)
    }
}

/// A kind of connection an extension adds to *Add…* in the side panel, as
/// its manifest declares it: one of the triggers by which Rick can say
/// what a plugin is for without having it (specs/plugin-distribution.md
/// §5). `plugin.toml` carries each as a `[[connections]]` table.
///
/// ```
/// use ext::{ConnectionSection, ConnectionSpec};
///
/// const S3: ConnectionSpec = ConnectionSpec {
///     section: ConnectionSection::Cloud,
///     title: "S3",
/// };
/// assert_eq!(S3.section.name(), "cloud");
/// assert_eq!(ConnectionSection::named("network"), Some(ConnectionSection::Network));
/// assert_eq!(ConnectionSection::named("usb"), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionSpec {
    /// The section whose *Add…* lists the kind.
    pub section: ConnectionSection,
    /// The kind's title there, in English: what the entry reads once the
    /// plugin is loaded (the English of its [`ConnectionSchema::title`]).
    pub title: &'static str,
}

/// How a field of the *Add connection…* dialogue is edited and stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionFieldKind {
    /// A line of text, kept in `rick.toml`.
    Text {
        /// The value the dialogue starts with.
        default: String,
    },
    /// A whole number within bounds, kept in `rick.toml` as text.
    Integer {
        /// The value the dialogue starts with.
        default: i64,
        /// The smallest accepted value.
        min: i64,
        /// The largest accepted value.
        max: i64,
    },
    /// A checkbox, kept in `rick.toml` as `true` or `false`.
    Bool {
        /// The value the dialogue starts with.
        default: bool,
    },
    /// A path on the local disk (a key file), kept in `rick.toml`.
    Path,
    /// A password, key or token: typed masked, kept in the platform
    /// keychain under [`secret_key`], never in a file.
    Secret,
    /// One of a list, kept in `rick.toml` as the option's text; an empty
    /// option means none. The options are computed when the schema is
    /// asked for, so they can name the other connections (a jump host).
    Choice {
        /// The options, in display order.
        options: Vec<String>,
        /// Index into `options` of the one the dialogue starts with.
        default: usize,
    },
}

/// One field of a provider's connection dialogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionField {
    /// The key in `rick.toml` and in [`Connection::settings`], or the
    /// last part of the keychain key for a secret.
    pub key: &'static str,
    /// The label in the dialogue: a Fluent key, read live.
    pub label: crate::Text,
    /// How it is edited and stored.
    pub kind: ConnectionFieldKind,
    /// Whether the dialogue refuses an empty value.
    pub required: bool,
}

/// What a provider's *Add connection…* dialogue asks for. The connection's
/// name is asked for by the application itself and is not a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSchema {
    /// The provider's name in the *Add connection…* menu ("S3", "SFTP"): a
    /// Fluent key, read live.
    pub title: crate::Text,
    /// The fields, in display order.
    pub fields: Vec<ConnectionField>,
}

impl ConnectionSchema {
    /// The keys of the fields kept in the keychain.
    pub fn secret_fields(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.fields
            .iter()
            .filter(|f| f.kind == ConnectionFieldKind::Secret)
            .map(|f| f.key)
    }

    /// The field called `key`.
    #[must_use]
    pub fn field(&self, key: &str) -> Option<&ConnectionField> {
        self.fields.iter().find(|f| f.key == key)
    }
}

/// One connection as the application keeps it: its name and the non-secret
/// fields by key. The secrets are in the keychain under [`secret_key`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Connection {
    /// The name the user gave it; unique within the provider, and the
    /// folder the provider lists it as at its root.
    pub name: String,
    /// The non-secret fields, by [`ConnectionField::key`]; a field left
    /// empty is absent.
    pub settings: BTreeMap<String, String>,
}

impl Connection {
    /// The field `key`, when set and not empty.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.settings
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
}

/// The form of a connection dialogue as it stands, handed to
/// [`Connector::react`] when the dialogue opens and after each edit.
#[derive(Debug, Clone, Copy)]
pub struct ConnectionForm<'a> {
    /// The fields by [`ConnectionField::key`] with their values as typed;
    /// a `Bool` holds `true` or `false`. The fields of
    /// [`ConnectionFieldKind::Secret`] are not in it: a secret reaches no
    /// reaction.
    pub values: &'a BTreeMap<String, String>,
    /// The keys of the fields the user edited by hand, in this dialogue or,
    /// in *Edit…*, when the connection was saved. A reaction's `fill`
    /// leaves them alone.
    pub edited: &'a BTreeSet<String>,
    /// The field the user just edited; `None` when the dialogue opens and
    /// when it is about to save.
    pub changed: Option<&'a str>,
    /// Whether the input is whole: `false` while the user is typing in
    /// `changed`, `true` once the text was pasted, the field was left, a
    /// box was ticked or an option picked, and when the dialogue opens or
    /// is about to save. A `rewrite` belongs to whole input: taking a
    /// half-typed address apart would move the text under the cursor.
    pub settled: bool,
}

impl ConnectionForm<'_> {
    /// The value of the field `key`, trimmed; empty when it has none.
    #[must_use]
    pub fn get(&self, key: &str) -> &str {
        self.values.get(key).map_or("", |value| value.trim())
    }

    /// Whether the user edited the field `key` by hand.
    #[must_use]
    pub fn is_edited(&self, key: &str) -> bool {
        self.edited.contains(key)
    }
}

/// What a connector does to its dialogue after an edit: the answer of
/// [`Connector::react`]. Empty, the default, leaves the form as typed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConnectionReaction {
    /// Values the connector settles, by field key: what it knows from the
    /// other fields (the region of a recognised host). The dialogue takes
    /// one only for a field the user has not edited by hand, and the field
    /// stays unedited, so a later reaction can settle it again.
    pub fill: Vec<(&'static str, String)>,
    /// The user's own input in the shape the fields want, by field key: an
    /// address pasted whole, of which one field keeps a part and another
    /// the rest. The dialogue takes these whatever the field's state, and
    /// the fields count as edited by hand from then on.
    pub rewrite: Vec<(&'static str, String)>,
    /// A short note by a field, by field key, already worded in the
    /// user's language (`t!`): what the connector recognised. The notes of
    /// one reaction replace those of the one before.
    pub notes: Vec<(&'static str, String)>,
}

impl ConnectionReaction {
    /// Applies the reaction to a form's values as the dialogue does, and
    /// returns the keys whose value changed: every `rewrite`, then each
    /// `fill` whose field is not in `edited`. A key of no field is taken
    /// as it comes; the dialogue ignores it.
    pub fn apply(
        &self,
        values: &mut BTreeMap<String, String>,
        edited: &BTreeSet<String>,
    ) -> Vec<&'static str> {
        let mut changed = Vec::new();
        let rewritten = |key: &str| self.rewrite.iter().any(|(k, _)| *k == key);
        let fills = self
            .fill
            .iter()
            .filter(|(key, _)| !edited.contains(*key) && !rewritten(key));
        for (key, value) in self.rewrite.iter().chain(fills) {
            if values.get(*key) != Some(value) {
                values.insert((*key).to_owned(), value.clone());
                changed.push(*key);
            }
        }
        changed
    }
}

/// The keychain key of the secret field `field` of connection
/// `connection` of `provider`: `s3/work/secret_key`.
#[must_use]
pub fn secret_key(provider: ProviderId, connection: &str, field: &str) -> String {
    format!("{}/{connection}/{field}", provider.0)
}

/// How a network provider's connections are made and kept: registered with
/// `Registry::connector` beside the provider, it counts as part of the
/// `Points::PROVIDER` contribution.
#[async_trait]
pub trait Connector: Send + Sync + 'static {
    /// The provider whose connections these are.
    fn provider(&self) -> ProviderId;

    /// The fields of the *Add connection…* and *Edit…* dialogues, as they
    /// are now: a connector whose fields depend on its state (a choice
    /// among the other connections) computes them on each call.
    fn schema(&self) -> ConnectionSchema;

    /// The connections as the application keeps them, once at start and
    /// after every change: the connector keeps them for its provider's
    /// root listing and path resolution, drops the state of a connection
    /// that went, and fetches a secret through `platform::Secrets` when a
    /// connection is used.
    fn update(&self, connections: Vec<Connection>);

    /// Opens or verifies the connection `name`: authenticates, lists the
    /// root. A wrong password is reported here rather than on the first
    /// listing.
    ///
    /// # Errors
    ///
    /// `Error::NotFound` for a name the connector does not know; the
    /// service's error otherwise.
    async fn connect(&self, name: &str) -> Result<(), Error>;

    /// Drops the connection's session and anything cached for it; the next
    /// operation connects again.
    fn disconnect(&self, name: &str);

    /// Whether `name` has an open session, or a verified client, right now.
    fn is_connected(&self, name: &str) -> bool;

    /// What the dialogue does after an edit: asked when it opens, each
    /// time the user changes a field, when a field typed in is left and
    /// before the dialogue saves, with the form as it stands. The
    /// default reacts to nothing, and the form stays as typed.
    ///
    /// It runs on the interface's thread between two frames: it reads the
    /// form and answers, and does no I/O.
    fn react(&self, form: &ConnectionForm<'_>) -> ConnectionReaction {
        let _ = form;
        ConnectionReaction::default()
    }
}
