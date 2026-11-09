//! Bulk rename, as a plugin: a pattern, find and replace, a case change,
//! a live preview with clash detection, one undoable job.
//!
//! The action shows a form ([`ext::Form`]) whose preview is computed by
//! [`plan()`] from the selected entries as the rules are typed; a problem in
//! any row keeps the form from being confirmed. The renames then run as
//! one job through temporary names, so swapping two names or changing
//! only the case on a case-insensitive file system works, and come back
//! the same way on undo.
//!
//! The crate builds as a `cdylib` the plugin host loads from
//! `plugins/action-rename/` and as an `rlib` for the binary's opt-in
//! `action-rename` feature and the tests.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

pub mod plan;

use ext::t;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::{
    API_VERSION, Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability,
    Capabilities, Extension, Field, Form, JobRunner, Key, Manifest, Modifiers, Platforms, Points,
    PreviewRow, Registry, Shortcut, UiHost, UndoCommand, Values, Version,
};
use types::{Entry, Error, ExtId, Location, Progress, ProviderId};

pub use plan::{Case, Planned, Rules, plan};

/// The extension id.
pub const ID: ExtId = ExtId("action-rename");

/// The action id.
pub const BULK: ActionId = ActionId("rename.bulk");

/// The extension: one action.
#[derive(Debug)]
pub struct RenameExtension {
    manifest: Manifest,
}

impl Default for RenameExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl RenameExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Bulk rename",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::RENAME,
                points: Points::ACTION,
                platforms: Platforms::ALL,
                previews: &[],
                commands: &[],
            },
        }
    }
}

impl Extension for RenameExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[
            ("en", include_str!("../locales/en.ftl")),
            ("de", include_str!("../locales/de.ftl")),
        ]);
        reg.action(Arc::new(BulkRename));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(RenameExtension::new()) as Arc<dyn Extension>);

/// The OS path of a local location.
fn local_path(loc: &Location) -> Option<PathBuf> {
    (loc.provider == ProviderId("local"))
        .then(|| ext::platform::paths::to_path(&loc.path).ok())
        .flatten()
}

/// The rules the form's values describe.
fn rules_of(values: &Values) -> Rules {
    Rules {
        pattern: values.text("pattern").to_owned(),
        find: values.text("find").to_owned(),
        replace: values.text("replace").to_owned(),
        regex: values.check("regex"),
        case: Case::from_index(values.choice("case")),
        start: values.text("start").trim().parse().unwrap_or(1),
    }
}

/// The preview rows for `entries` under the form's values.
fn preview(entries: &[Entry], folder: Option<&Path>, values: &Values) -> Vec<PreviewRow> {
    let rules = rules_of(values);
    match plan(entries, &rules, folder) {
        Ok(planned) => planned
            .into_iter()
            .map(|p| PreviewRow {
                from: p.entry.name.clone(),
                to: p.to,
                problem: p.problem,
            })
            .collect(),
        Err(error) => entries
            .iter()
            .map(|e| PreviewRow {
                from: e.name.clone(),
                to: e.name.clone(),
                problem: Some(format!("the pattern does not parse: {error}")),
            })
            .collect(),
    }
}

/// Renames the selection by a pattern.
pub struct BulkRename;

impl Action for BulkRename {
    fn id(&self) -> ActionId {
        BULK
    }
    fn label(&self) -> String {
        t!("rename-bulk-rename")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD,
            key: Key::F(2),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if local_path(&ctx.location).is_none()
            || !ctx
                .selection
                .iter()
                .all(|e| e.id.provider == ProviderId("local"))
        {
            return Availability::Hidden;
        }
        if ctx.selection.is_empty() {
            Availability::Disabled(t!("rename-select-the-entries-to-rename-first"))
        } else {
            Availability::Enabled
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        if ctx.selection.is_empty() {
            return Ok(ActionOutcome::Done);
        }
        let folder = local_path(&ctx.location).ok_or(Error::Unsupported(
            "bulk rename works in a folder on the local disk",
        ))?;
        let entries = ctx.selection.clone();
        let preview_entries = entries.clone();
        let preview_folder = folder.clone();
        let form = Form::new(
            format!("Rename {} entries", entries.len()),
            t!("rename-rename"),
        )
        .field(Field::text(
            "pattern",
            t!("rename-pattern"),
            "{name}{ext}",
            "{name}, {ext}, {n}, {n:4}, {date}",
        ))
        .field(Field::text(
            "find",
            t!("rename-find"),
            "",
            "text to replace",
        ))
        .field(Field::text(
            "replace",
            t!("rename-replace-with"),
            "",
            "$1 for a group",
        ))
        .field(Field::check(
            "regex",
            t!("rename-find-is-a-regular-expression"),
            false,
        ))
        .field(Field::choice(
            "case",
            t!("rename-case"),
            Case::OPTIONS.iter().map(|o| (*o).to_owned()).collect(),
            0,
        ))
        .field(Field::text(
            "start",
            t!("rename-numbering-starts-at"),
            "1",
            "",
        ))
        .preview(Arc::new(move |values: &Values| {
            preview(&preview_entries, Some(&preview_folder), values)
        }));
        let Some(values) = ui.form(form) else {
            return Ok(ActionOutcome::Done);
        };
        let rules = rules_of(&values);
        let planned = plan(&entries, &rules, Some(&folder)).map_err(|e| Error::Extension {
            ext: ID,
            source: e.into(),
        })?;
        if let Some(problem) = planned.iter().find_map(|p| p.problem.clone()) {
            return Err(Error::Extension {
                ext: ID,
                source: problem.into(),
            });
        }
        let pairs: Vec<(String, String)> = planned
            .into_iter()
            .filter(|p| !p.unchanged())
            .map(|p| (p.entry.name, p.to))
            .collect();
        if pairs.is_empty() {
            return Ok(ActionOutcome::Done);
        }
        let command = RenameCommand {
            label: format!("Rename {} entries", pairs.len()),
            folder,
            pairs,
        };
        command.redo(jobs)?;
        Ok(ActionOutcome::Undoable(Box::new(command)))
    }
}

/// Renames in `folder`: (old name, new name) pairs forward, the reverse
/// back.
struct RenameCommand {
    folder: PathBuf,
    pairs: Vec<(String, String)>,
    label: String,
}

/// Renames every pair in `folder` through temporary names, so two entries
/// may swap and a name may change only its case. Blocking.
fn rename_all(
    folder: &Path,
    pairs: &[(String, String)],
    progress: &ext::ProgressSink,
    cancel: &types::CancelToken,
) -> Result<(), Error> {
    let total = pairs.len() as u64;
    let stamp = std::process::id();
    let temp = |i: usize| folder.join(format!(".rick-rename-{stamp}-{i}"));
    let mut moved = 0usize;
    for (i, (from, _)) in pairs.iter().enumerate() {
        if cancel.is_cancelled() {
            // Put back what was moved to a temporary name so far.
            for (j, (from, _)) in pairs.iter().enumerate().take(moved) {
                let _ = std::fs::rename(temp(j), folder.join(from));
            }
            return Err(Error::Cancelled);
        }
        std::fs::rename(folder.join(from), temp(i)).map_err(Error::from_io)?;
        moved = i + 1;
    }
    for (i, (_, to)) in pairs.iter().enumerate() {
        std::fs::rename(temp(i), folder.join(to)).map_err(Error::from_io)?;
        progress.report(Progress::Items {
            done: i as u64 + 1,
            total: Some(total),
        });
    }
    Ok(())
}

impl UndoCommand for RenameCommand {
    fn undo(&self, jobs: &JobRunner) -> Result<(), Error> {
        let folder = self.folder.clone();
        let pairs: Vec<(String, String)> = self
            .pairs
            .iter()
            .map(|(from, to)| (to.clone(), from.clone()))
            .collect();
        jobs.spawn_named(format!("Undo {}", self.label), move |job| async move {
            ext::blocking(move || rename_all(&folder, &pairs, &job.progress, &job.cancel)).await
        });
        Ok(())
    }

    fn redo(&self, jobs: &JobRunner) -> Result<(), Error> {
        let folder = self.folder.clone();
        let pairs = self.pairs.clone();
        jobs.spawn_named(self.label.clone(), move |job| async move {
            ext::blocking(move || rename_all(&folder, &pairs, &job.progress, &job.cancel)).await
        });
        Ok(())
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}

#[cfg(test)]
mod tests;
