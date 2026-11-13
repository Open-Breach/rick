//! Copy name, Copy path, Copy path and name, Open with the system default,
//! Open in the system file manager.

use ext::t;
use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Capabilities, Key,
    Modifiers, Shortcut, UiHost,
};
use runtime::JobRunner;
use types::{Entry, EntryKind, Error};

use crate::{State, is_trash, local_path, location_of, needs_entries};

/// The clipboard text for a selection: one line per item, `f` picking what
/// each item contributes; duplicates dropped when `dedup`.
fn lines(ctx: &ActionCtx, dedup: bool, f: impl Fn(&types::Entry) -> String) -> String {
    let mut out: Vec<String> = ctx.selection.iter().map(f).collect();
    if dedup {
        let mut seen = std::collections::BTreeSet::new();
        out.retain(|line| seen.insert(line.clone()));
    }
    out.join("\n")
}

/// The OS form of an entry's path.
fn full_path(entry: &types::Entry) -> String {
    let loc = location_of(entry);
    local_path(&loc).map_or_else(
        |_| loc.path.to_string_lossy().into_owned(),
        |path| path.display().to_string(),
    )
}

/// The OS form of the folder holding an entry.
fn folder_path(entry: &types::Entry) -> String {
    let loc = location_of(entry);
    if let Ok(path) = local_path(&loc) {
        return path
            .parent()
            .map_or_else(|| path.display().to_string(), |p| p.display().to_string());
    }
    let text = loc.path.to_string_lossy();
    text.rfind(['/', '\\'])
        .map_or_else(|| text.to_string(), |i| text[..i.max(1)].to_owned())
}

/// Copies the file name of each selected item, one per line (Mod+Alt+C).
pub(crate) struct CopyName(pub(crate) Arc<State>);

impl Action for CopyName {
    fn id(&self) -> ActionId {
        ActionId("core.copy_name")
    }
    fn label(&self) -> String {
        t!("action-copy-name")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD | Modifiers::ALT,
            key: Key::Char('c'),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        needs_entries(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let text = lines(&ctx, false, |entry| entry.name.clone());
        self.0.platform.set_text(&text)?;
        Ok(ActionOutcome::Done)
    }
}

/// Copies the folder path of each selected item, without the name, one per
/// line with duplicates removed (Mod+Alt+Shift+C); with nothing selected,
/// the current folder's path.
pub(crate) struct CopyPath(pub(crate) Arc<State>);

impl Action for CopyPath {
    fn id(&self) -> ActionId {
        ActionId("core.copy_path")
    }
    fn label(&self) -> String {
        t!("action-copy-path")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD | Modifiers::ALT | Modifiers::SHIFT,
            key: Key::Char('c'),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if is_trash(ctx) {
            Availability::Hidden
        } else {
            Availability::Enabled
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let text = if ctx.selection.is_empty() {
            local_path(&ctx.location).map_or_else(
                |_| ctx.location.path.to_string_lossy().into_owned(),
                |path| path.display().to_string(),
            )
        } else {
            lines(&ctx, true, folder_path)
        };
        self.0.platform.set_text(&text)?;
        Ok(ActionOutcome::Done)
    }
}

/// Copies the full path of each selected item, one per line (Mod+Shift+C).
pub(crate) struct CopyPathAndName(pub(crate) Arc<State>);

impl Action for CopyPathAndName {
    fn id(&self) -> ActionId {
        ActionId("core.copy_path_and_name")
    }
    fn label(&self) -> String {
        t!("action-copy-path-and-name")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD | Modifiers::SHIFT,
            key: Key::Char('c'),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        needs_entries(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let text = lines(&ctx, false, full_path);
        self.0.platform.set_text(&text)?;
        Ok(ActionOutcome::Done)
    }
}

/// Opens each selected entry with the system default application.
pub(crate) struct OpenDefault(pub(crate) Arc<State>);

impl Action for OpenDefault {
    fn id(&self) -> ActionId {
        ActionId("core.open_default")
    }
    fn label(&self) -> String {
        t!("action-open")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD,
            key: Key::Enter,
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        needs_entries(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        // A remote file is not on the machine: nothing is handed to the
        // system, and opening it brings it down to Downloads (SPEC.md,
        // *Remote files*). Without `DOWNLOAD` it does nothing.
        let caps = self.0.capabilities(&ctx.location);
        if caps.contains(Capabilities::REMOTE) {
            let files: Vec<Entry> = ctx
                .selection
                .iter()
                .filter(|e| e.kind != EntryKind::Dir)
                .cloned()
                .collect();
            if files.is_empty() || !caps.contains(Capabilities::DOWNLOAD) {
                return Ok(ActionOutcome::Done);
            }
            let to = crate::download::downloads(&*self.0.platform);
            return Ok(crate::download::start(&self.0, jobs, &files, &to));
        }
        for entry in &ctx.selection {
            let path = local_path(&location_of(entry))?;
            self.0.platform.open(&path)?;
        }
        Ok(ActionOutcome::Done)
    }
}

/// What the open actions read of the user's settings. The application owns
/// the settings and hands the actions this view.
pub trait OpenSettings: Send + Sync {
    /// The command of *Open in editor*, with its arguments; empty for the
    /// system's default text editor.
    fn editor(&self) -> String;
}

/// An editor command as the setting holds it, split into its words: at
/// spaces outside double quotes, the quotes themselves dropped, so that
/// `"C:\Program Files\Editor\edit.exe" --wait` is a program and one
/// argument.
#[must_use]
pub fn split_command(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut any) = (false, false);
    for c in command.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    words.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            c => {
                word.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(word);
    }
    words
}

/// Opens the selected files in the editor (SPEC.md *Open in editor*): the
/// command the user named in Settings → Behaviour → Editor, with the files'
/// paths appended, or the system's default text editor where they named
/// none. Offered for local files only: a file on a remote location or in
/// an archive has no path on the machine.
pub(crate) struct OpenInEditor(pub(crate) Arc<State>);

impl Action for OpenInEditor {
    fn id(&self) -> ActionId {
        ActionId("core.open_in_editor")
    }
    fn label(&self) -> String {
        t!("action-open-in-editor")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::empty(),
            key: Key::F(4),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        let local = ctx.location.provider == vfs_local::PROVIDER_ID;
        let files = !ctx.selection.is_empty()
            && ctx.selection.iter().all(|entry| {
                entry.kind != EntryKind::Dir && entry.id.provider == vfs_local::PROVIDER_ID
            });
        if local && files {
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
        let paths = ctx
            .selection
            .iter()
            .map(|entry| local_path(&location_of(entry)))
            .collect::<Result<Vec<_>, _>>()?;
        let command = self
            .0
            .open_settings
            .as_ref()
            .map_or_default(|settings| settings.editor());
        let mut words = split_command(&command).into_iter();
        let Some(program) = words.next() else {
            self.0.platform.open_in_text_editor(&paths)?;
            return Ok(ActionOutcome::Done);
        };
        let mut args: Vec<std::ffi::OsString> = words.map(Into::into).collect();
        args.extend(paths.iter().map(|path| path.as_os_str().to_owned()));
        // The editor starts where the first file is.
        let dir = paths.first().and_then(|path| path.parent()).map_or_else(
            || std::path::PathBuf::from("."),
            std::path::Path::to_path_buf,
        );
        self.0
            .platform
            .run_detached(std::path::Path::new(&program), &args, &dir)?;
        Ok(ActionOutcome::Done)
    }
}

/// Shows the current location in the system file manager, with the first
/// selected entry selected when there is one.
pub(crate) struct Reveal(pub(crate) Arc<State>);

impl Action for Reveal {
    fn id(&self) -> ActionId {
        ActionId("core.reveal")
    }
    fn label(&self) -> String {
        t!("action-reveal")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD | Modifiers::SHIFT,
            key: Key::Char('o'),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if crate::is_virtual(ctx) && ctx.selection.is_empty() || is_trash(ctx) {
            Availability::Hidden
        } else {
            Availability::Enabled
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        match ctx.selection.first() {
            Some(entry) => self.0.platform.reveal(&local_path(&location_of(entry))?)?,
            None => self.0.platform.open(&local_path(&ctx.location)?)?,
        }
        Ok(ActionOutcome::Done)
    }
}
