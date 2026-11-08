//! The "SSH" action group: *Open console here* and *Open in external
//! terminal*, on a remote folder and on a host row of the Network section.

use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, JobRunner, Shortcut,
    UiHost, View,
};
use types::{EntryKind, Error, Location};

use crate::connection::Pool;
use crate::console::SshConsole;
use crate::{message, split};

/// The group every action belongs to.
pub const GROUP: ActionGroup = ActionGroup::Extension("SSH");
/// *Open console here*.
pub const CONSOLE: ActionId = ActionId("ssh.console");
/// *Open in external terminal*.
pub const TERMINAL: ActionId = ActionId("ssh.terminal");

/// The size a console opens at before the tab tells it its own.
const INITIAL_SIZE: (u16, u16) = (80, 24);

/// The two actions over `pool`.
#[must_use]
pub fn all(pool: &Arc<Pool>) -> Vec<Arc<dyn Action>> {
    vec![
        Arc::new(OpenConsole {
            pool: Arc::clone(pool),
        }),
        Arc::new(OpenTerminal {
            pool: Arc::clone(pool),
        }),
    ]
}

/// The remote folder an action works in: the one selected folder, else
/// the pane's location, when it is on an SSH host; `None` elsewhere.
fn target(ctx: &ActionCtx) -> Option<(String, Vec<u8>)> {
    let location = match ctx.selection.as_slice() {
        [one] if one.kind == EntryKind::Dir && one.id.provider == crate::PROVIDER_ID => {
            Location::new(one.id.provider, one.path.clone())
        }
        _ => ctx.location.clone(),
    };
    if location.provider != crate::PROVIDER_ID {
        return None;
    }
    split(&location)
}

fn availability(ctx: &ActionCtx) -> Availability {
    if target(ctx).is_some() {
        Availability::Enabled
    } else {
        Availability::Hidden
    }
}

/// *Open console here*: a shell on the host in the folder, in the console
/// pane.
pub struct OpenConsole {
    pool: Arc<Pool>,
}

impl Action for OpenConsole {
    fn id(&self) -> ActionId {
        CONSOLE
    }
    fn label(&self) -> &'static str {
        "Open console here"
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        availability(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let (alias, remote) = target(&ctx).ok_or(Error::Unsupported("not an SSH location"))?;
        // The tab opens at once; the channel, and the connection when
        // there is none, open on the runtime, so a host-key or password
        // dialogue can wait for the user without holding the window.
        let console = SshConsole::start(
            Arc::clone(&self.pool),
            &alias,
            remote,
            INITIAL_SIZE.0,
            INITIAL_SIZE.1,
        );
        Ok(ActionOutcome::View(View::Console(console)))
    }
}

/// *Open in external terminal*: the user's terminal with `ssh <alias>`.
pub struct OpenTerminal {
    pool: Arc<Pool>,
}

impl Action for OpenTerminal {
    fn id(&self) -> ActionId {
        TERMINAL
    }
    fn label(&self) -> &'static str {
        "Open in external terminal"
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        availability(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let (alias, remote) = target(&ctx).ok_or(Error::Unsupported("not an SSH location"))?;
        let host = self
            .pool
            .hosts()
            .get(&alias)
            .ok_or_else(|| message(format!("no host called {alias}")))?;
        let mut command = host.ssh_command();
        if !remote.is_empty() {
            // `ssh host command` runs the command instead of a shell: the
            // shell is started again inside the folder.
            command.push("-t".to_owned());
            let mut cd = b"cd ".to_vec();
            cd.extend_from_slice(&crate::shell::quote(&remote));
            cd.extend_from_slice(b" && exec \"${SHELL:-sh}\" -l");
            command.push(String::from_utf8_lossy(&cd).into_owned());
        }
        let platform = self.pool.platform();
        platform.open_terminal(&platform.home_dir(), Some(&command))?;
        Ok(ActionOutcome::Done)
    }
}
