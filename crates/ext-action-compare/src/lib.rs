//! The Compare action, as a plugin: two selected files as a text diff,
//! two folders entry by entry.
//!
//! The action opens a tab at once with an [`ext::Compare`] that has no
//! result yet and spawns the job that reads the two sides and fills it;
//! the UI shows the diff when it is there. Text is diffed line by line
//! with the `similar` crate (Myers, with a patience pass); two files that
//! are not text are compared byte for byte; two folders are listed one
//! level deep and matched by name, then by size and modification time.
//!
//! The crate builds as a `cdylib` the plugin host loads from
//! `plugins/action-compare/` and as an `rlib` for the binary's opt-in
//! `action-compare` feature and the tests.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

mod diff;

use ext::t;
use std::path::PathBuf;
use std::sync::Arc;

use ext::{
    API_VERSION, Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability,
    Capabilities, Compare, Extension, JobRunner, Manifest, Platforms, Points, Registry, Shortcut,
    UiHost, Version, View,
};
use types::{Entry, EntryKind, Error, ExtId, Location, ProviderId};

pub use diff::{MAX_TEXT_BYTES, compare_files, compare_folders, text_diff};

/// The extension id.
pub const ID: ExtId = ExtId("action-compare");

/// The action id.
pub const COMPARE: ActionId = ActionId("compare.compare");

/// The extension: one action.
#[derive(Debug)]
pub struct CompareExtension {
    manifest: Manifest,
}

impl Default for CompareExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl CompareExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Compare",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::ACTION,
                platforms: Platforms::ALL,
                previews: &[],
            },
        }
    }
}

impl Extension for CompareExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[
            ("en", include_str!("../locales/en.ftl")),
            ("de", include_str!("../locales/de.ftl")),
        ]);
        reg.action(Arc::new(CompareAction));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(CompareExtension::new()) as Arc<dyn Extension>);

/// What a context offers to compare.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Sides {
    /// Two selected files.
    Files(PathBuf, PathBuf),
    /// Two folders: two selected, or the pane's and the other pane's.
    Folders(Location, Location),
}

/// The OS path of a local location.
fn local_path(loc: &Location) -> Option<PathBuf> {
    (loc.provider == ProviderId("local"))
        .then(|| ext::platform::paths::to_path(&loc.path).ok())
        .flatten()
}

fn location_of(entry: &Entry) -> Location {
    Location::new(entry.id.provider, entry.path.clone())
}

/// What the context compares, if it can: two files, two folders, or the
/// two panes' folders with nothing selected.
fn sides(ctx: &ActionCtx) -> Option<Sides> {
    match ctx.selection.as_slice() {
        [a, b] => {
            let (la, lb) = (location_of(a), location_of(b));
            match (&a.kind, &b.kind) {
                (EntryKind::Dir, EntryKind::Dir) => {
                    local_path(&la)?;
                    local_path(&lb)?;
                    Some(Sides::Folders(la, lb))
                }
                (EntryKind::Dir, _) | (_, EntryKind::Dir) => None,
                _ => Some(Sides::Files(local_path(&la)?, local_path(&lb)?)),
            }
        }
        [] => {
            let other = ctx.other.clone()?;
            local_path(&ctx.location)?;
            local_path(&other)?;
            (other != ctx.location).then(|| Sides::Folders(ctx.location.clone(), other))
        }
        _ => None,
    }
}

fn name_of(loc: &Location) -> String {
    loc.path.file_name().map_or_else(
        || loc.path.to_string(),
        |n| String::from_utf8_lossy(n).into_owned(),
    )
}

/// Compare the two selected files, or two folders.
pub struct CompareAction;

impl Action for CompareAction {
    fn id(&self) -> ActionId {
        COMPARE
    }
    fn label(&self) -> String {
        t!("compare-compare")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        match sides(ctx) {
            Some(_) => Availability::Enabled,
            None if ctx.selection.len() == 2 => {
                Availability::Disabled(t!("compare-select-two-files-or-two-folders"))
            }
            None if ctx.selection.is_empty() && ctx.location.provider == ProviderId("local") => {
                Availability::Disabled(t!("compare-select-two-files-or-two-folders-2"))
            }
            None => Availability::Hidden,
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let Some(sides) = sides(&ctx) else {
            return Ok(ActionOutcome::Done);
        };
        let (title, left, right) = match &sides {
            Sides::Files(a, b) => {
                let (a, b) = (
                    a.file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                    b.file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                );
                (format!("{a} ↔ {b}"), a, b)
            }
            Sides::Folders(a, b) => {
                let (a, b) = (name_of(a), name_of(b));
                (format!("{a} ↔ {b}"), a, b)
            }
        };
        let compare = Compare::pending(title.clone(), left, right);
        let target = Arc::clone(&compare);
        jobs.spawn_named(format!("Compare {title}"), move |job| async move {
            let cancel = job.cancel.clone();
            let result = ext::blocking(move || match sides {
                Sides::Files(a, b) => compare_files(&a, &b, &cancel),
                Sides::Folders(a, b) => compare_folders(&a, &b, &cancel),
            })
            .await;
            match result {
                Err(Error::Cancelled) => Err(Error::Cancelled),
                other => {
                    let failed = other.is_err();
                    target.finish(other);
                    if failed {
                        Err(Error::Unsupported("the comparison failed; see the tab"))
                    } else {
                        Ok(())
                    }
                }
            }
        });
        Ok(ActionOutcome::View(View::Compare(compare)))
    }
}

#[cfg(test)]
mod tests;
