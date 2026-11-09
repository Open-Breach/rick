//! Plugin subcommands of the `rick` command line
//! (specs/system-integration.md §4): a plugin declares its commands in its
//! manifest, registers one [`CliCommand`] per name, and the host dispatches
//! `rick <command> <args…>` to it — in the running instance when there is
//! one, so the command works on the Rick the user sees.
//!
//! The manifest's [`CommandSpec`]s (name and help line) are what
//! `plugin.toml` carries as `[[commands]]` and what `rick --help` lists,
//! so the help names a plugin's commands without loading the plugin. The
//! registry refuses a registered command whose name the manifest does not
//! declare, so the manifest stays honest. A command runs on the
//! application thread like an action: it returns at once with text for
//! the caller's terminal ([`CliOutcome::Text`]) or with what an action
//! would return ([`CliOutcome::Outcome`]), which the application applies
//! to the focused pane — a job starts, a form opens, a view or a tab
//! opens — while the caller gets exit status zero. An error becomes the
//! caller's exit status 1 with the error's text.
//!
//! # Worked example
//!
//! A plugin whose `greet <name>` prints a greeting, and whose manifest
//! declares it.
//!
//! ```
//! use std::path::PathBuf;
//! use std::sync::Arc;
//!
//! use ext::{
//!     API_VERSION, Capabilities, CliCommand, CliCtx, CliOutcome, CommandSpec, Extension,
//!     Manifest, PaneId, Platforms, Points, Registry, UiHost, Version,
//! };
//! use runtime::JobRunner;
//! use types::{Error, ExtId, Location, ProviderId};
//!
//! struct Greet;
//!
//! impl CliCommand for Greet {
//!     fn name(&self) -> &'static str {
//!         "greet"
//!     }
//!
//!     fn run(&self, ctx: CliCtx, _jobs: &JobRunner, _ui: &dyn UiHost) -> Result<CliOutcome, Error> {
//!         match ctx.args.as_slice() {
//!             [name] => Ok(CliOutcome::Text(format!("hello, {name}"))),
//!             _ => Err(Error::Unsupported("usage: rick greet <name>")),
//!         }
//!     }
//! }
//!
//! struct Greeter(Manifest);
//!
//! impl Extension for Greeter {
//!     fn manifest(&self) -> &Manifest {
//!         &self.0
//!     }
//!     fn register(&self, reg: &mut Registry) -> Result<(), Error> {
//!         reg.command(Arc::new(Greet));
//!         Ok(())
//!     }
//! }
//!
//! let greeter = Greeter(Manifest {
//!     id: ExtId("greeter"),
//!     name: "Greeter",
//!     version: Version::new(0, 1, 0),
//!     api_version: API_VERSION,
//!     requires: Capabilities::empty(),
//!     points: Points::empty(),
//!     platforms: Platforms::ALL,
//!     previews: &[],
//!     commands: &[CommandSpec { name: "greet", help: "Say hello to someone" }],
//! });
//! let mut registry = Registry::new();
//! registry.load(&greeter).unwrap();
//!
//! // What the host does with `rick greet world` from /work.
//! let command = registry.commands().iter().find(|c| c.name() == "greet").unwrap();
//! let ctx = CliCtx {
//!     args: vec!["world".to_owned()],
//!     cwd: PathBuf::from("/work"),
//!     pane: PaneId(1),
//!     location: Location::new(ProviderId("local"), "/work"),
//! };
//! struct Quiet;
//! impl UiHost for Quiet {
//!     fn notify(&self, _: ext::Level, _: &str) {}
//!     fn confirm(&self, _: &str, _: &str) -> bool { false }
//!     fn prompt(&self, _: &str, _: &str) -> Option<String> { None }
//!     fn show(&self, _: &str, _: &str) {}
//!     fn form(&self, _: ext::Form) -> Option<ext::Values> { None }
//! }
//! let outcome = command.run(ctx, &JobRunner::new(), &Quiet).unwrap();
//! assert!(matches!(outcome, CliOutcome::Text(text) if text == "hello, world"));
//! ```

use std::fmt;
use std::path::PathBuf;

use runtime::JobRunner;
use types::{Error, Location};

use crate::action::{ActionOutcome, PaneId, UiHost};

/// A command a plugin's manifest declares: what `plugin.toml` carries as
/// `[[commands]]` and `rick --help` lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    /// The subcommand, as typed after `rick`: lower case, no spaces.
    pub name: &'static str,
    /// The one line `rick --help` shows beside it, in English like the
    /// rest of the command line's help.
    pub help: &'static str,
}

/// What a plugin command sees when it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliCtx {
    /// The arguments after the command's name, as typed.
    pub args: Vec<String>,
    /// The caller's working directory, which relative paths among the
    /// arguments are resolved against.
    pub cwd: PathBuf,
    /// The focused pane of the instance's most recently focused window,
    /// where an [`ActionOutcome`] the command returns applies.
    pub pane: PaneId,
    /// That pane's location.
    pub location: Location,
}

/// What a plugin command produced.
pub enum CliOutcome {
    /// Text for the caller's terminal; nothing changes in the window.
    Text(String),
    /// What an action would return, applied to the context's pane: a job
    /// that starts, a form that opens, a view or a selection. The caller
    /// gets exit status zero and no text.
    Outcome(ActionOutcome),
}

impl fmt::Debug for CliOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => f.debug_tuple("Text").field(text).finish(),
            Self::Outcome(outcome) => f.debug_tuple("Outcome").field(outcome).finish(),
        }
    }
}

/// A subcommand of the `rick` command line a plugin provides.
pub trait CliCommand: Send + Sync {
    /// The subcommand's name, one of the manifest's [`CommandSpec`]s.
    fn name(&self) -> &'static str;

    /// Runs on the application thread, like an action: returns at once,
    /// anything long spawned as a job through `jobs`.
    ///
    /// # Errors
    ///
    /// The error the caller's terminal shows, with exit status 1; a
    /// wrong argument list is an `Error::Unsupported` naming the usage.
    fn run(&self, ctx: CliCtx, jobs: &JobRunner, ui: &dyn UiHost) -> Result<CliOutcome, Error>;
}
