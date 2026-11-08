//! Open with…: the platform's applications for the selection's type plus
//! any executable, with the choice remembered per file type in
//! `core.toml` (Mod+O).

use ext::t;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Field, Form, Key,
    Modifiers, Shortcut, UiHost,
};
use platform::Application;
use runtime::JobRunner;
use types::Error;

use crate::{State, local_path, location_of, needs_entries};

/// The file in the config directory that remembers the choice per type.
pub const FILE: &str = crate::core_toml::FILE;
/// The table in it.
const TABLE: &str = "open_with";
/// The last option of the application choice.
const OTHER: &str = "Another program…";

/// Opens the selection with a chosen application.
pub(crate) struct OpenWith(pub(crate) Arc<State>);

/// The lower-case extension a choice is remembered for; `none` for a name
/// without one, `folder` for a folder.
fn type_key(path: &Path) -> String {
    if path.is_dir() {
        return "folder".to_owned();
    }
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "none".to_owned())
}

/// The remembered choices, by type key: the application's id.
fn remembered(file: &Path) -> toml::Table {
    crate::core_toml::table(file, TABLE)
}

/// Writes one remembered choice, keeping the rest of the file.
fn remember(file: &Path, key: &str, app: &Application) -> Result<(), Error> {
    crate::core_toml::set(file, TABLE, key, toml::Value::String(app.id.clone()))
}

impl OpenWith {
    fn file(&self) -> PathBuf {
        self.0.platform.config_dir().join(FILE)
    }
}

impl Action for OpenWith {
    fn id(&self) -> ActionId {
        ActionId("core.open_with")
    }
    fn label(&self) -> String {
        t!("action-open-with")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD,
            key: Key::Char('o'),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        match needs_entries(ctx) {
            Availability::Enabled
                if !ctx
                    .selection
                    .iter()
                    .all(|e| e.id.provider == vfs_local::PROVIDER_ID) =>
            {
                Availability::Disabled(t!("reason-only-local-files-open-elsewhere"))
            }
            other => other,
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let Some(first) = ctx.selection.first() else {
            return Ok(ActionOutcome::Done);
        };
        let first_path = local_path(&location_of(first))?;
        let key = type_key(&first_path);
        let mut apps = self.0.platform.applications(&first_path)?;
        let file = self.file();
        let remembered_id = remembered(&file)
            .get(&key)
            .and_then(|v| v.as_str().map(str::to_owned));
        // The remembered program goes first, listed even when the platform
        // no longer offers it for the type.
        if let Some(id) = &remembered_id {
            if let Some(index) = apps.iter().position(|a| a.id == *id) {
                let app = apps.remove(index);
                apps.insert(0, app);
            } else {
                apps.insert(0, Application::executable(id));
            }
        }
        let mut options: Vec<String> = apps.iter().map(|a| a.name.clone()).collect();
        options.push(OTHER.to_owned());
        let other_index = options.len() - 1;
        let label = match ctx.selection.len() {
            1 => format!("Open {} with", first.name),
            n => format!("Open {n} items with"),
        };
        let form = Form::new(label, "Open")
            .field(Field::choice("app", "Application", options, 0))
            .field(Field::text(
                "command",
                "Program",
                "",
                "a program on the PATH or its full path, for \"Another program\"",
            ))
            .field(Field::check(
                "remember",
                format!(
                    "Remember for {}",
                    match key.as_str() {
                        "folder" => "folders".to_owned(),
                        "none" => "files without an extension".to_owned(),
                        ext => format!(".{ext} files"),
                    }
                ),
                false,
            ));
        let Some(values) = ui.form(form) else {
            return Ok(ActionOutcome::Done);
        };
        let chosen = values.choice("app");
        let command = values.text("command").trim().to_owned();
        let app = if chosen >= other_index || apps.is_empty() {
            if command.is_empty() {
                return Err(Error::Unsupported("name the program to open with"));
            }
            Application::executable(&command)
        } else {
            apps.get(chosen)
                .cloned()
                .ok_or(Error::Unsupported("no such application"))?
        };
        for entry in &ctx.selection {
            let path = local_path(&location_of(entry))?;
            self.0.platform.open_with(&path, &app)?;
        }
        if values.check("remember") {
            remember(&file, &key, &app)?;
        }
        Ok(ActionOutcome::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_keys_and_the_remembered_file() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join("a.TXT"), "a").unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join("README"), "a").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(type_key(&dir.path().join("a.TXT")), "txt");
        assert_eq!(type_key(&dir.path().join("README")), "none");
        assert_eq!(type_key(dir.path()), "folder");
        let file = dir.path().join("config").join(FILE);
        assert!(remembered(&file).is_empty());
        std::fs::create_dir_all(dir.path().join("config")).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(&file, "[other]\nkeep = 1\n").unwrap_or_else(|e| panic!("{e}"));
        remember(&file, "txt", &Application::executable("/usr/bin/editor"))
            .unwrap_or_else(|e| panic!("{e}"));
        let table = remembered(&file);
        assert_eq!(
            table.get("txt").and_then(|v| v.as_str()),
            Some("/usr/bin/editor")
        );
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        assert!(text.contains("[other]"), "{text}");
    }
}
