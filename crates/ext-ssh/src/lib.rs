//! Remote hosts over SSH: the `ssh` provider, the hosts of `~/.ssh/config`
//! and Rick's own in the side panel's Network section, and the "SSH"
//! action group (`specs/ext-ssh.md`).
//!
//! Every operation is a command run on an SSH `exec` channel through
//! `russh`, never the SFTP subsystem, so any host with a POSIX shell works.
//! On the first connection the host is probed once and a command profile
//! chosen (`gnu`, `bsd` or `busybox`, [`profile`]); every command is a
//! fixed template with shell-quoted arguments ([`shell`]) whose output is
//! NUL-separated where the tool allows it, so names with spaces, newlines
//! and bytes that are no UTF-8 survive. Connections are pooled per host
//! ([`connection`]): channels multiplex on one, a second opens at the
//! server's session limit, idle ones close after the setting's timeout and
//! come back on the next operation; jump hosts are connected like any host
//! and the next hop runs over a `direct-tcpip` channel. Authentication
//! tries the agent, the identity files and then keyboard-interactive and
//! password; secrets live in the platform keychain and prompts go through
//! the `Prompts` queue the host lends, as do the two host-key dialogues
//! over `~/.ssh/known_hosts` ([`known_hosts`]).
//!
//! Locations are `ssh:/<alias>/<remote path>`; `ssh:/<alias>` alone is the
//! host's default directory and the provider's root lists the hosts.
//!
//! The crate builds as a `cdylib` the plugin host loads from
//! `plugins/ssh/` and as an `rlib` for the binary's opt-in `ssh` feature
//! and the tests.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

pub mod actions;
pub mod config;
pub mod connection;
pub mod console;
#[cfg(any(test, feature = "fake-sshd"))]
pub mod fake_sshd;
pub mod hosts;
pub mod known_hosts;
pub mod profile;
pub mod provider;
pub mod shell;

use std::sync::Arc;

use ext::{
    API_VERSION, Capabilities, Connector, Extension, Manifest, Platforms, Points, Provider,
    Registry, SettingField, SettingKind, SettingsSchema, Version,
};
use types::{Error, ExtId, Location, ProviderId, VPath};

pub use connection::{Pool, SshConnector};
pub use hosts::{Host, Hosts, Settings, Source};
pub use provider::SshProvider;

/// The extension id, and the name of its settings file.
pub const ID: ExtId = ExtId("ssh");

/// The provider id; `vfs-rick` lists it under Network.
pub const PROVIDER_ID: ProviderId = ProviderId("ssh");

/// An error with a message of this plugin's own.
pub(crate) fn message(text: impl Into<String>) -> Error {
    Error::Extension {
        ext: ID,
        source: text.into().into(),
    }
}

/// An error of the SSH library, as the kernel error.
pub(crate) fn ssh_error(error: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::Extension {
        ext: ID,
        source: Box::new(error),
    }
}

/// A location's alias and the absolute remote path below it, as bytes:
/// `ssh:/nas/srv/a b` is `("nas", b"/srv/a b")`, `ssh:/nas` and `ssh:/nas/`
/// are `("nas", b"")`, the host's default directory. `None` for the root,
/// which lists the hosts.
#[must_use]
pub fn split(loc: &Location) -> Option<(String, Vec<u8>)> {
    let bytes = loc.path.as_bytes();
    let mut start = 0;
    while start < bytes.len() && matches!(bytes[start], b'/' | b'\\') {
        start += 1;
    }
    let rest = &bytes[start..];
    if rest.is_empty() {
        return None;
    }
    let (alias, remote) = match rest.iter().position(|b| matches!(b, b'/' | b'\\')) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, &rest[rest.len()..]),
    };
    let alias = String::from_utf8_lossy(alias).into_owned();
    let mut remote: Vec<u8> = remote
        .iter()
        .map(|b| if *b == b'\\' { b'/' } else { *b })
        .collect();
    while remote.len() > 1 && remote.ends_with(b"/") {
        remote.pop();
    }
    if remote == b"/" {
        remote.clear();
    }
    Some((alias, remote))
}

/// The location of the absolute remote path `remote` on the host `alias`;
/// an empty path is the host's default directory.
#[must_use]
pub fn location(alias: &str, remote: &[u8]) -> Location {
    let mut path = Vec::with_capacity(alias.len() + remote.len() + 2);
    path.push(b'/');
    path.extend_from_slice(alias.as_bytes());
    let trimmed = {
        let mut end = remote.len();
        while end > 1 && remote[end - 1] == b'/' {
            end -= 1;
        }
        &remote[..end]
    };
    if !trimmed.is_empty() && trimmed != b"/" {
        if !trimmed.starts_with(b"/") {
            path.push(b'/');
        }
        path.extend_from_slice(trimmed);
    }
    Location::new(PROVIDER_ID, VPath::from(path))
}

/// The extension: the provider, its connector, the two actions and the
/// settings.
#[derive(Debug)]
pub struct SshExtension {
    manifest: Manifest,
}

impl Default for SshExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl SshExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "SSH",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PROVIDER | Points::ACTION,
                platforms: Platforms::ALL,
            },
        }
    }

    /// The settings the Settings window shows for `ssh.toml`.
    #[must_use]
    pub fn settings_schema() -> SettingsSchema {
        SettingsSchema {
            ext: ID,
            fields: vec![
                SettingField {
                    key: hosts::setting::READ_OPENSSH_CONFIG,
                    label: "Hosts from ~/.ssh/config",
                    kind: SettingKind::Bool { default: true },
                },
                SettingField {
                    key: hosts::setting::IDLE_TIMEOUT_MINUTES,
                    label: "Close idle connections after (minutes)",
                    kind: SettingKind::Integer {
                        default: 10,
                        min: 1,
                        max: 24 * 60,
                    },
                },
                SettingField {
                    key: hosts::setting::PREVIEW_CAP_MIB,
                    label: "Preview size cap for whole-file previews (MiB)",
                    kind: SettingKind::Integer {
                        default: 4,
                        min: 1,
                        max: 1_000_000,
                    },
                },
            ],
        }
    }
}

impl Extension for SshExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        let platform = reg
            .platform()
            .ok_or(Error::Unsupported(
                "the SSH plugin needs the platform services the host lends",
            ))?
            .clone();
        let hosts = Arc::new(Hosts::new(platform));
        let pool = Arc::new(Pool::new(Arc::clone(&hosts), reg.prompts().cloned()));
        reg.provider(Arc::new(SshProvider::new(Arc::clone(&pool))) as Arc<dyn Provider>);
        reg.connector(Arc::new(SshConnector::new(Arc::clone(&pool))) as Arc<dyn Connector>);
        for action in actions::all(&pool) {
            reg.action(action);
        }
        reg.settings(Self::settings_schema());
        Ok(())
    }
}

ext::plugin!(|| Arc::new(SshExtension::new()) as Arc<dyn Extension>);

#[cfg(test)]
mod tests;
