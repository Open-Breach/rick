//! The hosts: the user's OpenSSH configuration merged with Rick's own,
//! and the plugin's settings file.
//!
//! The configuration hosts come from `~/.ssh/config` ([`config`]), re-read
//! when the file's modification time changes; Rick's own are the
//! connections the application hands the connector (`[[connections]]`
//! tables of `rick.toml` with `provider = "ssh"`), whose fields are the
//! host editor's ([`field`]). A Rick host with the alias of a configuration
//! host hides it. The settings (`ssh.toml` in the config directory) say
//! whether the configuration is read at all, how long an idle connection
//! stays open and the preview size cap.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use ext::Text;
use ext::platform::Platform;
use ext::{Connection, ConnectionField, ConnectionFieldKind, ConnectionSchema};

use crate::config::{self, ConfigHost, Env, Hop, IdentityAgent};

/// The field keys of the host editor and the `[[connections]]` table.
pub mod field {
    /// The host name or address.
    pub const HOST: &str = "host";
    /// The port, 22 by default.
    pub const PORT: &str = "port";
    /// The user name; the local user when empty.
    pub const USER: &str = "user";
    /// The remote folder the host opens at; the remote home when empty.
    pub const DEFAULT_DIR: &str = "default_dir";
    /// An OpenSSH private key file, tried before the default ones.
    pub const IDENTITY_FILE: &str = "identity_file";
    /// The alias of the host to jump through, Rick-defined or from the
    /// configuration; empty for none.
    pub const JUMP: &str = "jump";
    /// Whether a password typed into the prompt is kept in the keychain.
    pub const REMEMBER_PASSWORD: &str = "remember_password";
    /// The preview size cap for this host in MiB; 0 for the setting's.
    pub const PREVIEW_CAP: &str = "preview_size_cap_mib";
    /// The password, in the keychain.
    pub const PASSWORD: &str = "password";
    /// The passphrase of the identity file, in the keychain.
    pub const PASSPHRASE: &str = "passphrase";
}

/// The keys of `ssh.toml`, the settings the plugin declares.
pub mod setting {
    /// Whether `~/.ssh/config` is read.
    pub const READ_OPENSSH_CONFIG: &str = "read_openssh_config";
    /// Minutes an idle connection stays open.
    pub const IDLE_TIMEOUT_MINUTES: &str = "idle_timeout_minutes";
    /// The size above which a whole-file preview asks first, in MiB.
    pub const PREVIEW_CAP_MIB: &str = "preview_size_cap_mib";
}

/// The plugin's settings, as `ssh.toml` holds them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Whether the OpenSSH configuration's hosts are listed.
    pub read_openssh_config: bool,
    /// How long a connection without a channel stays open.
    pub idle_timeout: Duration,
    /// The default preview size cap in bytes.
    pub preview_cap: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            read_openssh_config: true,
            idle_timeout: Duration::from_mins(10),
            preview_cap: 4 * 1024 * 1024,
        }
    }
}

impl Settings {
    /// The settings from the text of `ssh.toml`; a key that is missing or
    /// unreadable takes its default.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let table = text.parse::<toml::Table>().unwrap_or_default();
        let mut settings = Self::default();
        if let Some(on) = table
            .get(setting::READ_OPENSSH_CONFIG)
            .and_then(toml::Value::as_bool)
        {
            settings.read_openssh_config = on;
        }
        if let Some(minutes) = table
            .get(setting::IDLE_TIMEOUT_MINUTES)
            .and_then(toml::Value::as_integer)
            .and_then(|m| u64::try_from(m).ok())
        {
            settings.idle_timeout = Duration::from_mins(minutes);
        }
        if let Some(mib) = table
            .get(setting::PREVIEW_CAP_MIB)
            .and_then(toml::Value::as_integer)
            .and_then(|m| u64::try_from(m).ok())
        {
            settings.preview_cap = mib.saturating_mul(1024 * 1024);
        }
        settings
    }
}

/// Where a host comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `~/.ssh/config`: read-only in Rick.
    Config,
    /// Rick's own list: editable from the Network section.
    Rick,
}

/// One host, from either source, with what a connection needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    /// The alias: the row's name and the first component of its locations.
    pub alias: String,
    /// The host name or address connected to.
    pub host: String,
    /// The port.
    pub port: u16,
    /// The user; the local user when `None`.
    pub user: Option<String>,
    /// The remote folder the host opens at; the remote home when `None`.
    pub default_dir: Option<String>,
    /// The identity files to try before the default `~/.ssh/id_*` set.
    pub identity_files: Vec<PathBuf>,
    /// The agent to use, when the configuration says.
    pub identity_agent: Option<IdentityAgent>,
    /// The hops to jump through, in order.
    pub jump: Vec<Hop>,
    /// Where it comes from.
    pub source: Source,
    /// Whether a password typed into the prompt is kept in the keychain.
    pub remember_password: bool,
    /// The preview size cap for this host, when it overrides the setting.
    pub preview_cap: Option<u64>,
}

impl Host {
    /// The host as a configuration entry describes it.
    fn from_config(entry: &ConfigHost) -> Self {
        Self {
            alias: entry.alias.clone(),
            host: entry.host().to_owned(),
            port: entry.port.unwrap_or(22),
            user: entry.user.clone(),
            default_dir: None,
            identity_files: entry.identity_files.clone(),
            identity_agent: entry.identity_agent.clone(),
            jump: entry.proxy_jump.iter().map(|h| Hop::parse(h)).collect(),
            source: Source::Config,
            remember_password: false,
            preview_cap: None,
        }
    }

    /// The host as a Rick connection describes it; `None` without a host
    /// name.
    fn from_connection(connection: &Connection, env: &Env) -> Option<Self> {
        let host = connection.get(field::HOST)?.to_owned();
        let port = connection
            .get(field::PORT)
            .and_then(|p| p.parse().ok())
            .unwrap_or(22);
        let identity_files = connection
            .get(field::IDENTITY_FILE)
            .map(|f| vec![expand_home(f, env)])
            .unwrap_or_default();
        let jump = connection
            .get(field::JUMP)
            .map(|j| {
                j.split(',')
                    .map(str::trim)
                    .filter(|h| !h.is_empty())
                    .map(Hop::parse)
                    .collect()
            })
            .unwrap_or_default();
        let preview_cap = connection
            .get(field::PREVIEW_CAP)
            .and_then(|c| c.parse::<u64>().ok())
            .filter(|c| *c > 0)
            .map(|mib| mib.saturating_mul(1024 * 1024));
        Some(Self {
            alias: connection.name.clone(),
            host,
            port,
            user: connection.get(field::USER).map(str::to_owned),
            default_dir: connection.get(field::DEFAULT_DIR).map(str::to_owned),
            identity_files,
            identity_agent: None,
            jump,
            source: Source::Rick,
            remember_password: connection.get(field::REMEMBER_PASSWORD) == Some("true"),
            preview_cap,
        })
    }

    /// The `ssh` command line that reaches this host from a terminal:
    /// `ssh <alias>` for a configuration host, which `ssh` resolves itself;
    /// the explicit form for a Rick host.
    #[must_use]
    pub fn ssh_command(&self) -> Vec<String> {
        let mut words = vec!["ssh".to_owned()];
        if self.source == Source::Config {
            words.push(self.alias.clone());
            return words;
        }
        if self.port != 22 {
            words.push("-p".to_owned());
            words.push(self.port.to_string());
        }
        for file in &self.identity_files {
            words.push("-i".to_owned());
            words.push(file.to_string_lossy().into_owned());
        }
        if !self.jump.is_empty() {
            words.push("-J".to_owned());
            words.push(
                self.jump
                    .iter()
                    .map(|hop| {
                        let mut text = String::new();
                        if let Some(user) = &hop.user {
                            text.push_str(user);
                            text.push('@');
                        }
                        text.push_str(&hop.host);
                        if let Some(port) = hop.port {
                            text.push(':');
                            text.push_str(&port.to_string());
                        }
                        text
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        match &self.user {
            Some(user) => words.push(format!("{user}@{}", self.host)),
            None => words.push(self.host.clone()),
        }
        words
    }
}

/// `~` and `~/x` as the home directory.
fn expand_home(value: &str, env: &Env) -> PathBuf {
    if value == "~" {
        env.home.clone()
    } else if let Some(rest) = value.strip_prefix("~/") {
        env.home.join(rest)
    } else {
        PathBuf::from(value)
    }
}

/// The local user name, from the environment.
fn local_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

/// The configuration file as last read, with the modification time it
/// had.
#[derive(Debug, Default)]
struct ConfigCache {
    modified: Option<SystemTime>,
    hosts: Vec<ConfigHost>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The host list: the configuration's hosts and Rick's, with the settings.
pub struct Hosts {
    platform: Arc<dyn Platform>,
    rick: Mutex<Vec<Connection>>,
    config: Mutex<ConfigCache>,
    settings: Mutex<(Option<SystemTime>, Settings)>,
}

impl std::fmt::Debug for Hosts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hosts")
            .field("rick", &lock(&self.rick).len())
            .field("config", &lock(&self.config).hosts.len())
            .finish_non_exhaustive()
    }
}

impl Hosts {
    /// An empty list over `platform`, whose home holds `~/.ssh/config` and
    /// whose config directory holds `ssh.toml`.
    #[must_use]
    pub fn new(platform: Arc<dyn Platform>) -> Self {
        Self {
            platform,
            rick: Mutex::new(Vec::new()),
            config: Mutex::new(ConfigCache::default()),
            settings: Mutex::new((None, Settings::default())),
        }
    }

    /// The platform the hosts were read from.
    #[must_use]
    pub fn platform(&self) -> &Arc<dyn Platform> {
        &self.platform
    }

    /// The token environment: the home directory and the local user.
    #[must_use]
    pub fn env(&self) -> Env {
        Env {
            home: self.platform.home_dir(),
            user: local_user(),
        }
    }

    /// `~/.ssh/config`.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.platform.home_dir().join(".ssh").join("config")
    }

    /// `~/.ssh/known_hosts`.
    #[must_use]
    pub fn known_hosts_path(&self) -> PathBuf {
        self.platform.home_dir().join(".ssh").join("known_hosts")
    }

    /// The settings, re-read when `ssh.toml` changed on disk.
    #[must_use]
    pub fn settings(&self) -> Settings {
        let path = self
            .platform
            .config_dir()
            .join(format!("{}.toml", crate::ID.0));
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let mut cached = lock(&self.settings);
        if cached.0 != modified || cached.0.is_none() {
            let settings = std::fs::read_to_string(&path)
                .map(|text| Settings::parse(&text))
                .unwrap_or_default();
            *cached = (modified, settings);
        }
        cached.1
    }

    /// Replaces Rick's own hosts with the connections the application
    /// keeps.
    pub fn update(&self, connections: Vec<Connection>) {
        *lock(&self.rick) = connections;
    }

    /// The configuration's hosts, re-read when the file changed on disk;
    /// none when the setting says not to read it.
    #[must_use]
    pub fn config_hosts(&self) -> Vec<ConfigHost> {
        if !self.settings().read_openssh_config {
            return Vec::new();
        }
        let path = self.config_path();
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let mut cache = lock(&self.config);
        if cache.modified != modified || cache.modified.is_none() {
            cache.hosts = config::load(&path, &self.env());
            cache.modified = modified;
        }
        cache.hosts.clone()
    }

    /// Every host, Rick's first in the order the settings hold them, then
    /// the configuration's in file order, an alias listed once.
    #[must_use]
    pub fn all(&self) -> Vec<Host> {
        let env = self.env();
        let mut hosts: Vec<Host> = lock(&self.rick)
            .iter()
            .filter_map(|c| Host::from_connection(c, &env))
            .collect();
        for entry in self.config_hosts() {
            if !hosts.iter().any(|h| h.alias == entry.alias) {
                hosts.push(Host::from_config(&entry));
            }
        }
        hosts
    }

    /// The host called `alias`, if any.
    #[must_use]
    pub fn get(&self, alias: &str) -> Option<Host> {
        self.all().into_iter().find(|h| h.alias == alias)
    }

    /// The host a `ProxyJump` hop names: the host of that alias with the
    /// hop's user and port over it, or a host made from the hop alone.
    #[must_use]
    pub fn hop_host(&self, hop: &Hop) -> Host {
        let mut host = self.get(&hop.host).unwrap_or_else(|| Host {
            alias: hop.host.clone(),
            host: hop.host.clone(),
            port: 22,
            user: None,
            default_dir: None,
            identity_files: Vec::new(),
            identity_agent: None,
            jump: Vec::new(),
            source: Source::Config,
            remember_password: false,
            preview_cap: None,
        });
        if let Some(user) = &hop.user {
            host.user = Some(user.clone());
        }
        if let Some(port) = hop.port {
            host.port = port;
        }
        host
    }

    /// The fields of the host editor, with the jump host chosen from the
    /// hosts known right now.
    #[must_use]
    pub fn schema(&self) -> ConnectionSchema {
        let mut jumps: Vec<String> = vec![String::new()];
        jumps.extend(self.all().into_iter().map(|h| h.alias));
        ConnectionSchema {
            title: Text("ssh-ssh"),
            fields: vec![
                ConnectionField {
                    key: field::HOST,
                    label: Text("ssh-host-name"),
                    kind: ConnectionFieldKind::Text {
                        default: String::new(),
                    },
                    required: true,
                },
                ConnectionField {
                    key: field::PORT,
                    label: Text("ssh-port"),
                    kind: ConnectionFieldKind::Integer {
                        default: 22,
                        min: 1,
                        max: 65535,
                    },
                    required: false,
                },
                ConnectionField {
                    key: field::USER,
                    label: Text("ssh-user"),
                    kind: ConnectionFieldKind::Text {
                        default: String::new(),
                    },
                    required: false,
                },
                ConnectionField {
                    key: field::DEFAULT_DIR,
                    label: Text("ssh-default-directory"),
                    kind: ConnectionFieldKind::Text {
                        default: String::new(),
                    },
                    required: false,
                },
                ConnectionField {
                    key: field::IDENTITY_FILE,
                    label: Text("ssh-identity-file"),
                    kind: ConnectionFieldKind::Path,
                    required: false,
                },
                ConnectionField {
                    key: field::JUMP,
                    label: Text("ssh-jump-host"),
                    kind: ConnectionFieldKind::Choice {
                        options: jumps,
                        default: 0,
                    },
                    required: false,
                },
                ConnectionField {
                    key: field::PASSWORD,
                    label: Text("ssh-password"),
                    kind: ConnectionFieldKind::Secret,
                    required: false,
                },
                ConnectionField {
                    key: field::REMEMBER_PASSWORD,
                    label: Text("ssh-remember-password-in-the-os-keychain"),
                    kind: ConnectionFieldKind::Bool { default: false },
                    required: false,
                },
                ConnectionField {
                    key: field::PREVIEW_CAP,
                    label: Text("ssh-preview-size-cap-mib-0-for"),
                    kind: ConnectionFieldKind::Integer {
                        default: 0,
                        min: 0,
                        max: 1_000_000,
                    },
                    required: false,
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ext::platform::{ConfigDir as _, Fake};

    use super::*;

    fn rick(name: &str, fields: &[(&str, &str)]) -> Connection {
        Connection {
            name: name.into(),
            settings: fields
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn rick_hosts_and_config_hosts_merge_with_rick_first() {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let ssh = fake.home_dir().join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(
            ssh.join("config"),
            "Host nas\n  HostName 10.0.0.2\n  User serge\nHost bastion\n  Port 2200\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let hosts = Hosts::new(fake.clone());
        assert_eq!(
            hosts.all().iter().map(|h| &h.alias).collect::<Vec<_>>(),
            ["nas", "bastion"]
        );
        hosts.update(vec![
            rick(
                "prod-db",
                &[
                    ("host", "10.0.3.12"),
                    ("user", "ops"),
                    ("jump", "bastion"),
                    ("default_dir", "/srv"),
                    ("identity_file", "~/.ssh/id_prod"),
                    ("remember_password", "true"),
                    ("preview_size_cap_mib", "1"),
                ],
            ),
            rick("nas", &[("host", "192.168.1.20"), ("port", "2222")]),
            rick("broken", &[("user", "nobody")]),
        ]);
        let all = hosts.all();
        let aliases: Vec<&str> = all.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(
            aliases,
            ["prod-db", "nas", "bastion"],
            "a Rick host hides the config host of its alias; one without a host is dropped"
        );
        let db = &all[0];
        assert_eq!(db.source, Source::Rick);
        assert_eq!(db.jump, [Hop::parse("bastion")]);
        assert_eq!(db.default_dir.as_deref(), Some("/srv"));
        assert_eq!(db.identity_files, [fake.home_dir().join(".ssh/id_prod")]);
        assert!(db.remember_password);
        assert_eq!(db.preview_cap, Some(1024 * 1024));
        assert_eq!(
            db.ssh_command(),
            [
                "ssh",
                "-i",
                &fake.home_dir().join(".ssh/id_prod").to_string_lossy(),
                "-J",
                "bastion",
                "ops@10.0.3.12"
            ]
        );
        let nas = &all[1];
        assert_eq!(nas.port, 2222);
        assert_eq!(nas.user, None);
        assert_eq!(nas.ssh_command(), ["ssh", "-p", "2222", "192.168.1.20"]);
        let bastion = &all[2];
        assert_eq!(bastion.source, Source::Config);
        assert_eq!(bastion.ssh_command(), ["ssh", "bastion"]);
        let hop = hosts.hop_host(&Hop::parse("ops@bastion:2201"));
        assert_eq!(
            (hop.alias.as_str(), hop.user.as_deref(), hop.port),
            ("bastion", Some("ops"), 2201)
        );
        let unknown = hosts.hop_host(&Hop::parse("jump.example.com"));
        assert_eq!(unknown.host, "jump.example.com");
        assert_eq!(unknown.port, 22);

        // The schema offers the known aliases as jump hosts.
        let schema = hosts.schema();
        assert_eq!(schema.title.key(), "ssh-ssh");
        assert!(matches!(
            schema.field("jump").map(|f| &f.kind),
            Some(ConnectionFieldKind::Choice { options, .. })
                if options == &["", "prod-db", "nas", "bastion"]
        ));
        assert_eq!(schema.secret_fields().collect::<Vec<_>>(), ["password"]);

        // The config file is re-read when it changes.
        std::fs::write(ssh.join("config"), "Host other\n").unwrap_or_else(|e| panic!("{e}"));
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
        std::fs::File::options()
            .write(true)
            .open(ssh.join("config"))
            .and_then(|f| f.set_modified(later))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            hosts.all().iter().map(|h| &h.alias).collect::<Vec<_>>(),
            ["prod-db", "nas", "other"]
        );
    }

    #[test]
    fn settings_read_from_the_plugin_file_with_defaults() {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let hosts = Hosts::new(fake.clone());
        assert_eq!(hosts.settings(), Settings::default());
        std::fs::create_dir_all(fake.config_dir()).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(
            fake.config_dir().join("ssh.toml"),
            "read_openssh_config = false\nidle_timeout_minutes = 3\npreview_size_cap_mib = 1\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let settings = hosts.settings();
        assert!(!settings.read_openssh_config);
        assert_eq!(settings.idle_timeout, Duration::from_mins(3));
        assert_eq!(settings.preview_cap, 1024 * 1024);
        std::fs::write(fake.home_dir().join(".ssh").join("config"), "Host hidden\n").ok();
        assert!(
            hosts.config_hosts().is_empty(),
            "the configuration is not read when the setting says so"
        );
        assert_eq!(Settings::parse("not toml ["), Settings::default());
    }
}
