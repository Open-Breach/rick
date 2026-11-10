//! Open console, as a plugin: an embedded terminal tab, or the system
//! terminal when the setting says so.
//!
//! [`Session`] runs the user's shell in a pseudo-terminal and keeps its
//! output as a [`Screen`] of cells through the kernel's terminal emulator
//! ([`ext::terminal`]); the UI draws the screen and writes what is typed. The action returns the
//! session as an [`ext::View::Console`], and the application opens the tab.
//! The setting `console` of `action-terminal.toml` (`embedded` or
//! `system`) picks the system terminal instead, through
//! [`platform::Terminal`](ext::platform::Terminal).
//!
//! The crate builds as a `cdylib` the plugin host loads from
//! `plugins/action-terminal/` and as an `rlib` for the binary's opt-in
//! `action-terminal` feature and the tests.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

pub mod session;

use ext::t;
use std::path::PathBuf;
use std::sync::Arc;

use ext::platform::Platform;
use ext::{
    API_VERSION, Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability,
    Capabilities, Extension, JobRunner, Key, Manifest, Modifiers, Platforms, Points, Registry,
    SettingField, SettingKind, SettingsSchema, Shortcut, Text, UiHost, Version, View,
};
use types::{Error, ExtId, Location, ProviderId};

pub use ext::terminal::Screen;
pub use session::Session;

/// The extension id, and the name of its settings file.
pub const ID: ExtId = ExtId("action-terminal");

/// The action id.
pub const CONSOLE: ActionId = ActionId("terminal.console");

/// The setting that picks the console: `embedded` or `system`.
pub const CONSOLE_SETTING: &str = "console";
/// The setting's options, in the order the Settings window lists them.
pub const CONSOLE_OPTIONS: [&str; 2] = ["embedded", "system"];

/// The extension: one action and one setting.
#[derive(Debug)]
pub struct TerminalExtension {
    manifest: Manifest,
}

impl Default for TerminalExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Console",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::ACTION,
                platforms: Platforms::ALL,
                previews: &[],
                commands: &[],
            },
        }
    }
}

impl Extension for TerminalExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[
            ("en", include_str!("../locales/en.ftl")),
            ("de", include_str!("../locales/de.ftl")),
        ]);
        reg.action(Arc::new(OpenConsole {
            platform: reg.platform().cloned(),
        }));
        reg.settings(SettingsSchema {
            ext: ID,
            fields: vec![SettingField {
                key: CONSOLE_SETTING,
                label: Text("terminal-open-console-in"),
                kind: SettingKind::Choice {
                    options: CONSOLE_OPTIONS.to_vec(),
                    default: 0,
                },
            }],
        });
        Ok(())
    }
}

ext::plugin!(|| Arc::new(TerminalExtension::new()) as Arc<dyn Extension>);

/// Opens a terminal at the pane's location.
pub struct OpenConsole {
    platform: Option<Arc<dyn Platform>>,
}

/// The OS path of a local location.
fn local_path(loc: &Location) -> Option<PathBuf> {
    (loc.provider == ProviderId("local"))
        .then(|| ext::platform::paths::to_path(&loc.path).ok())
        .flatten()
}

/// Whether the setting sends the console to the system terminal: the
/// `console` key of `action-terminal.toml` in the config directory.
fn wants_system_terminal(platform: Option<&Arc<dyn Platform>>) -> bool {
    let Some(platform) = platform else {
        return false;
    };
    let file = platform.config_dir().join(format!("{}.toml", ID.0));
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|table| table.get(CONSOLE_SETTING)?.as_str().map(str::to_owned))
        .is_some_and(|value| value == CONSOLE_OPTIONS[1])
}

impl Action for OpenConsole {
    fn id(&self) -> ActionId {
        CONSOLE
    }
    fn label(&self) -> String {
        t!("terminal-open-console")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD,
            key: Key::Char('`'),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        let folder = match ctx.selection.as_slice() {
            [one] if one.kind == types::EntryKind::Dir => {
                Location::new(one.id.provider, one.path.clone())
            }
            _ => ctx.location.clone(),
        };
        if local_path(&folder).is_some() {
            Availability::Enabled
        } else {
            Availability::Hidden
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        // One selected folder opens the console in that folder.
        let folder = match ctx.selection.as_slice() {
            [one] if one.kind == types::EntryKind::Dir => {
                Location::new(one.id.provider, one.path.clone())
            }
            _ => ctx.location.clone(),
        };
        let dir = local_path(&folder).ok_or(Error::Unsupported(
            "the console opens in a folder on the local disk",
        ))?;
        if wants_system_terminal(self.platform.as_ref()) {
            let platform = self
                .platform
                .as_ref()
                .ok_or(Error::Unsupported("no platform to open a terminal with"))?;
            platform.open_terminal(&dir, None)?;
            return Ok(ActionOutcome::Done);
        }
        let session = Session::spawn(&dir, 80, 24)?;
        Ok(ActionOutcome::View(View::Console(session)))
    }
}

#[cfg(test)]
mod tests;
