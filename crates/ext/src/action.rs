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
//!     Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Form, Invocation,
//!     Key, Level, Modifiers, PaneId, Shortcut, UiHost, Values,
//! };
//! use runtime::JobRunner;
//! use std::sync::Mutex;
//! use types::{Error, Location, ProviderId};
//!
//! struct CountSelection;
//!
//! impl Action for CountSelection {
//!     fn id(&self) -> ActionId {
//!         ActionId("demo.count")
//!     }
//!
//!     fn label(&self) -> String {
//!         t!("count-selection")
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
//! struct Recorder(Mutex<Vec<String>>);
//!
//! impl UiHost for Recorder {
//!     fn notify(&self, _level: Level, text: &str) {
//!         self.0.lock().unwrap().push(text.to_owned());
//!     }
//!     fn confirm(&self, _title: &str, _text: &str) -> bool {
//!         true
//!     }
//!     fn prompt(&self, _title: &str, default: &str) -> Option<String> {
//!         Some(default.to_owned())
//!     }
//!     fn show(&self, title: &str, _text: &str) {
//!         self.0.lock().unwrap().push(title.to_owned());
//!     }
//!     fn form(&self, form: Form) -> Option<Values> {
//!         Some(form.defaults())
//!     }
//! }
//!
//! let ctx = ActionCtx {
//!     pane: PaneId(1),
//!     location: Location::new(ProviderId("local"), "/"),
//!     selection: Vec::new(),
//!     invoked_from: Invocation::Palette,
//!     other: None,
//! };
//! assert!(matches!(CountSelection.availability(&ctx), Availability::Disabled(_)));
//!
//! let host = Recorder::default();
//! let outcome = CountSelection.run(ctx, &JobRunner::new(), &host).unwrap();
//! assert!(matches!(outcome, ActionOutcome::Done));
//! assert_eq!(host.0.lock().unwrap()[0], "0 entries selected");
//! ```

use std::fmt;
use std::sync::Arc;

use runtime::JobRunner;
use types::{Entry, Error, JobId, Location};

use crate::form::{Form, Values};
use crate::view::View;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Not shown at all.
    Hidden,
    /// Shown greyed out, with the reason as tooltip, in the language in
    /// use.
    Disabled(String),
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
    /// The location of the other explorer pane of the window, when there is
    /// one: what Compare puts beside the pane's own folder.
    pub other: Option<Location>,
}

/// What `run` produced.
pub enum ActionOutcome {
    /// Finished synchronously.
    Done,
    /// A job was spawned; the UI follows it by id.
    Started(JobId),
    /// Finished, and can be undone.
    Undoable(Box<dyn UndoCommand>),
    /// Change the pane's selection: the application applies the selector
    /// to the pane's rows.
    Select(Selector),
    /// Open this in a tab of its own, next to the pane's active tab.
    View(View),
    /// Take the pane to this location, as a click on a side-panel row
    /// does: what an action that opens a view of its own returns (the
    /// installer's Installed Apps).
    Navigate(Location),
}

impl fmt::Debug for ActionOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Done => f.write_str("Done"),
            Self::Started(id) => f.debug_tuple("Started").field(id).finish(),
            Self::Undoable(cmd) => f.debug_tuple("Undoable").field(&cmd.label()).finish(),
            Self::Select(selector) => f
                .debug_struct("Select")
                .field("replace", &selector.replace)
                .finish_non_exhaustive(),
            Self::View(view) => f.debug_tuple("View").field(view).finish(),
            Self::Navigate(to) => f.debug_tuple("Navigate").field(to).finish(),
        }
    }
}

/// Which rows of the pane an action wants selected.
#[derive(Clone)]
pub struct Selector {
    /// Whether a row is wanted.
    pub matches: Arc<dyn Fn(&Entry) -> bool + Send + Sync>,
    /// Replace the selection with the matches, or add them to it.
    pub replace: bool,
}

impl fmt::Debug for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Selector")
            .field("replace", &self.replace)
            .finish_non_exhaustive()
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

impl Shortcut {
    /// The spec's text form, e.g. `Mod+Shift+C`, `F2`, `Alt+Left`, `Mod+,`.
    /// Letters are upper case; `+` and `-` are written `Plus` and `Minus`.
    #[must_use]
    pub fn text(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.contains(Modifiers::MOD) {
            f.write_str("Mod+")?;
        }
        if self.modifiers.contains(Modifiers::SHIFT) {
            f.write_str("Shift+")?;
        }
        if self.modifiers.contains(Modifiers::ALT) {
            f.write_str("Alt+")?;
        }
        match self.key {
            Key::Char('+') => f.write_str("Plus"),
            Key::Char('-') => f.write_str("Minus"),
            Key::Char(c) => write!(f, "{}", c.to_ascii_uppercase()),
            Key::F(n) => write!(f, "F{n}"),
            Key::Enter => f.write_str("Enter"),
            Key::Escape => f.write_str("Escape"),
            Key::Tab => f.write_str("Tab"),
            Key::Backspace => f.write_str("Backspace"),
            Key::Delete => f.write_str("Delete"),
            Key::Insert => f.write_str("Insert"),
            Key::Home => f.write_str("Home"),
            Key::End => f.write_str("End"),
            Key::PageUp => f.write_str("PageUp"),
            Key::PageDown => f.write_str("PageDown"),
            Key::Up => f.write_str("Up"),
            Key::Down => f.write_str("Down"),
            Key::Left => f.write_str("Left"),
            Key::Right => f.write_str("Right"),
            Key::Space => f.write_str("Space"),
        }
    }
}

/// Why a shortcut text did not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShortcutParseError {
    /// The text has no key, e.g. `Mod+`.
    #[error("no key in shortcut")]
    NoKey,
    /// A part is neither a modifier nor a key.
    #[error("unknown key or modifier `{0}`")]
    Unknown(String),
}

impl std::str::FromStr for Shortcut {
    type Err = ShortcutParseError;

    /// Parses the text form. `Ctrl` and `Cmd` are accepted for `Mod`,
    /// `Option` for `Alt`; key names are matched ignoring case; a single
    /// character stands for itself, so `Mod+,` and `Mod+Comma` are the same.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut modifiers = Modifiers::empty();
        let mut key = None;
        // A trailing `+` is the key itself ("Mod++").
        let body = text.trim();
        let parts: Vec<&str> = if let Some(stem) = body.strip_suffix("++") {
            stem.split('+').chain(std::iter::once("+")).collect()
        } else {
            body.split('+').collect()
        };
        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match part.to_ascii_lowercase().as_str() {
                "mod" | "ctrl" | "cmd" | "control" | "command" => modifiers |= Modifiers::MOD,
                "shift" => modifiers |= Modifiers::SHIFT,
                "alt" | "option" => modifiers |= Modifiers::ALT,
                lower => {
                    if key.is_some() {
                        return Err(ShortcutParseError::Unknown(part.to_owned()));
                    }
                    key = Some(parse_key(lower, part)?);
                }
            }
        }
        key.map(|key| Shortcut { modifiers, key })
            .ok_or(ShortcutParseError::NoKey)
    }
}

fn parse_key(lower: &str, original: &str) -> Result<Key, ShortcutParseError> {
    Ok(match lower {
        "enter" | "return" => Key::Enter,
        "escape" | "esc" => Key::Escape,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "insert" | "ins" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "space" => Key::Space,
        "plus" => Key::Char('+'),
        "minus" => Key::Char('-'),
        "comma" => Key::Char(','),
        "period" => Key::Char('.'),
        "backslash" => Key::Char('\\'),
        "slash" => Key::Char('/'),
        "backtick" | "grave" => Key::Char('`'),
        _ => {
            if let Some(n) = lower.strip_prefix('f')
                && let Ok(n) = n.parse::<u8>()
                && (1..=24).contains(&n)
            {
                return Ok(Key::F(n));
            }
            let mut chars = original.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if !c.is_whitespace() => Key::Char(c.to_ascii_lowercase()),
                _ => return Err(ShortcutParseError::Unknown(original.to_owned())),
            }
        }
    })
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
pub trait UiHost: Send + Sync {
    /// Shows a transient notification.
    fn notify(&self, level: Level, text: &str);

    /// Asks a yes/no question and blocks for the answer.
    fn confirm(&self, title: &str, text: &str) -> bool;

    /// Asks for a line of text and blocks for it; `None` when dismissed.
    fn prompt(&self, title: &str, default: &str) -> Option<String>;

    /// Shows a read-only text, such as a log, under `title`; returns at
    /// once.
    fn show(&self, title: &str, text: &str);

    /// Shows a form and blocks for its values; `None` when dismissed. A
    /// host that cannot block runs the action again with the values once
    /// the user confirmed, the way it does for `prompt`.
    fn form(&self, form: Form) -> Option<Values>;
}

/// A command the user can invoke.
pub trait Action: Send + Sync + 'static {
    /// Unique id.
    fn id(&self) -> ActionId;

    /// Text shown in menus and the palette, in the language in use:
    /// `t!("count-selection")`.
    fn label(&self) -> String;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(modifiers: Modifiers, key: Key) -> Shortcut {
        Shortcut { modifiers, key }
    }

    #[test]
    fn shortcut_text_round_trips() {
        let cases = [
            (
                "Mod+Shift+C",
                sc(Modifiers::MOD | Modifiers::SHIFT, Key::Char('c')),
            ),
            ("F2", sc(Modifiers::empty(), Key::F(2))),
            ("Alt+Left", sc(Modifiers::ALT, Key::Left)),
            ("Mod+,", sc(Modifiers::MOD, Key::Char(','))),
            ("Mod+Plus", sc(Modifiers::MOD, Key::Char('+'))),
            ("Mod+Minus", sc(Modifiers::MOD, Key::Char('-'))),
            ("Mod+\\", sc(Modifiers::MOD, Key::Char('\\'))),
            ("Mod+`", sc(Modifiers::MOD, Key::Char('`'))),
            (
                "Mod+Shift+Home",
                sc(Modifiers::MOD | Modifiers::SHIFT, Key::Home),
            ),
            ("Delete", sc(Modifiers::empty(), Key::Delete)),
            ("Mod+1", sc(Modifiers::MOD, Key::Char('1'))),
            ("Space", sc(Modifiers::empty(), Key::Space)),
        ];
        for (text, shortcut) in cases {
            assert_eq!(text.parse::<Shortcut>().ok(), Some(shortcut), "{text}");
            assert_eq!(shortcut.text(), text, "{text}");
        }
    }

    #[test]
    fn shortcut_parsing_accepts_aliases_and_rejects_nonsense() {
        assert_eq!(
            "ctrl+shift+n".parse::<Shortcut>().ok(),
            Some(sc(Modifiers::MOD | Modifiers::SHIFT, Key::Char('n')))
        );
        assert_eq!(
            "Cmd+Option+Up".parse::<Shortcut>().ok(),
            Some(sc(Modifiers::MOD | Modifiers::ALT, Key::Up))
        );
        assert_eq!(
            "Mod+Comma".parse::<Shortcut>().ok(),
            Some(sc(Modifiers::MOD, Key::Char(',')))
        );
        assert_eq!(
            "Mod++".parse::<Shortcut>().ok(),
            Some(sc(Modifiers::MOD, Key::Char('+')))
        );
        assert_eq!("Mod+".parse::<Shortcut>(), Err(ShortcutParseError::NoKey));
        assert_eq!(
            "Mod+Bogus".parse::<Shortcut>(),
            Err(ShortcutParseError::Unknown("Bogus".to_owned()))
        );
        assert_eq!(
            "Mod+A+B".parse::<Shortcut>(),
            Err(ShortcutParseError::Unknown("B".to_owned()))
        );
        assert_eq!(
            "F25".parse::<Shortcut>(),
            Err(ShortcutParseError::Unknown("F25".to_owned()))
        );
    }
}
