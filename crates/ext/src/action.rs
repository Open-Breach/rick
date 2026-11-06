//! The Action extension point: a command the user can invoke.
//!
//! An action has an id, a label, a group that decides where it appears
//! (core menus or an extension's submenu), an optional default shortcut, an
//! availability check against the current selection, and `run`. `run` is
//! called on the app thread and must return quickly: anything long is spawned
//! as a job through the [`JobRunner`] it receives, and it reports
//! [`ActionOutcome::Started`] with the job's id.
//!
//! # Worked example
//!
//! An action that counts the selected entries and tells the user.
//!
//! ```
//! use ext::{
//!     Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Invocation, Key,
//!     Level, Modifiers, PaneId, Shortcut, UiHost,
//! };
//! use runtime::JobRunner;
//! use std::cell::RefCell;
//! use types::{Error, Location, ProviderId};
//!
//! struct CountSelection;
//!
//! impl Action for CountSelection {
//!     fn id(&self) -> ActionId {
//!         ActionId("demo.count")
//!     }
//!
//!     fn label(&self) -> &str {
//!         "Count selection"
//!     }
//!
//!     fn group(&self) -> ActionGroup {
//!         ActionGroup::Extension("Demo")
//!     }
//!
//!     fn default_shortcut(&self) -> Option<Shortcut> {
//!         Some(Shortcut { modifiers: Modifiers::MOD | Modifiers::SHIFT, key: Key::Char('c') })
//!     }
//!
//!     fn availability(&self, ctx: &ActionCtx) -> Availability {
//!         if ctx.selection.is_empty() {
//!             Availability::Disabled("select something first")
//!         } else {
//!             Availability::Enabled
//!         }
//!     }
//!
//!     fn run(&self, ctx: ActionCtx, _jobs: &JobRunner, ui: &dyn UiHost) -> Result<ActionOutcome, Error> {
//!         ui.notify(Level::Info, &format!("{} entries selected", ctx.selection.len()));
//!         Ok(ActionOutcome::Done)
//!     }
//! }
//!
//! /// A host that records what the action said.
//! #[derive(Default)]
//! struct Recorder(RefCell<Vec<String>>);
//!
//! impl UiHost for Recorder {
//!     fn notify(&self, _level: Level, text: &str) {
//!         self.0.borrow_mut().push(text.to_owned());
//!     }
//!     fn confirm(&self, _title: &str, _text: &str) -> bool {
//!         true
//!     }
//!     fn prompt(&self, _title: &str, default: &str) -> Option<String> {
//!         Some(default.to_owned())
//!     }
//! }
//!
//! let ctx = ActionCtx {
//!     pane: PaneId(1),
//!     location: Location::new(ProviderId("local"), "/"),
//!     selection: Vec::new(),
//!     invoked_from: Invocation::Palette,
//! };
//! assert!(matches!(CountSelection.availability(&ctx), Availability::Disabled(_)));
//!
//! let host = Recorder::default();
//! let outcome = CountSelection.run(ctx, &JobRunner::new(), &host).unwrap();
//! assert!(matches!(outcome, ActionOutcome::Done));
//! assert_eq!(host.0.borrow()[0], "0 entries selected");
//! ```

use std::fmt;

use runtime::JobRunner;
use types::{Entry, Error, JobId, Location};

/// Identity of a pane in a window; `app` assigns them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(pub u64);

/// Identity of an action: `"core.copy"`, `"git.commit"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActionId(pub &'static str);

/// Where an action appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionGroup {
    /// The core menus and the top of the context menu.
    Core,
    /// A submenu of the given name in the Extensions part of the context menu
    /// and the command palette.
    Extension(&'static str),
}

/// Whether an action can run for the current context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// Not shown at all.
    Hidden,
    /// Shown greyed out, with the reason as tooltip.
    Disabled(&'static str),
    /// Ready to run.
    Enabled,
}

/// How the user invoked an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invocation {
    /// From the right-click menu.
    ContextMenu,
    /// From the command palette.
    Palette,
    /// From its keyboard shortcut.
    Shortcut,
    /// From the application menu.
    Menu,
}

/// What an action sees when asked whether it can run, and when it runs.
#[derive(Debug, Clone)]
pub struct ActionCtx {
    /// The pane the action was invoked in.
    pub pane: PaneId,
    /// The pane's current location.
    pub location: Location,
    /// The selected entries, possibly empty.
    pub selection: Vec<Entry>,
    /// How the user got here.
    pub invoked_from: Invocation,
}

/// What `run` produced.
pub enum ActionOutcome {
    /// Finished synchronously.
    Done,
    /// A job was spawned; the UI follows it by id.
    Started(JobId),
    /// Finished, and can be undone.
    Undoable(Box<dyn UndoCommand>),
}

impl fmt::Debug for ActionOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Done => f.write_str("Done"),
            Self::Started(id) => f.debug_tuple("Started").field(id).finish(),
            Self::Undoable(cmd) => f.debug_tuple("Undoable").field(&cmd.label()).finish(),
        }
    }
}

bitflags::bitflags! {
    /// Modifier keys of a shortcut. `MOD` is Ctrl on Linux and Windows and
    /// Cmd on macOS.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Modifiers: u8 {
        /// Ctrl, or Cmd on macOS.
        const MOD = 1;
        /// Shift.
        const SHIFT = 2;
        /// Alt, or Option on macOS.
        const ALT = 4;
    }
}

/// The non-modifier part of a shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable character, lower case for letters.
    Char(char),
    /// A function key, 1 to 24.
    F(u8),
    /// Enter / Return.
    Enter,
    /// Escape.
    Escape,
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Insert.
    Insert,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Space.
    Space,
}

/// A keyboard shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    /// Modifier keys held.
    pub modifiers: Modifiers,
    /// The key pressed.
    pub key: Key,
}

/// Severity of a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    /// Informational.
    Info,
    /// Something to look at.
    Warning,
    /// Something failed.
    Error,
}

/// What an action may ask of the UI while it runs on the app thread.
pub trait UiHost {
    /// Shows a transient notification.
    fn notify(&self, level: Level, text: &str);

    /// Asks a yes/no question and blocks for the answer.
    fn confirm(&self, title: &str, text: &str) -> bool;

    /// Asks for a line of text and blocks for it; `None` when dismissed.
    fn prompt(&self, title: &str, default: &str) -> Option<String>;
}

/// A command the user can invoke.
pub trait Action: Send + Sync + 'static {
    /// Unique id.
    fn id(&self) -> ActionId;

    /// Text shown in menus and the palette.
    fn label(&self) -> &str;

    /// Where the action appears.
    fn group(&self) -> ActionGroup;

    /// The shortcut bound unless the user rebinds it.
    fn default_shortcut(&self) -> Option<Shortcut>;

    /// Whether the action applies to `ctx`.
    fn availability(&self, ctx: &ActionCtx) -> Availability;

    /// Runs on the app thread; anything long is spawned as a job through
    /// `jobs`.
    ///
    /// # Errors
    ///
    /// Returns the error the UI should show; a job's own failure arrives
    /// through the job instead.
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error>;
}

/// A reversible step on the undo stack.
pub trait UndoCommand: Send {
    /// Reverts the step.
    ///
    /// # Errors
    ///
    /// Fails when the state has moved on and the step cannot be reverted.
    fn undo(&self, jobs: &JobRunner) -> Result<(), Error>;

    /// Applies the step again after an undo.
    ///
    /// # Errors
    ///
    /// Fails when the step cannot be applied again.
    fn redo(&self, jobs: &JobRunner) -> Result<(), Error>;

    /// Text for the Undo and Redo menu items.
    fn label(&self) -> String;
}
