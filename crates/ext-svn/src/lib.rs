//! The Subversion extension: one manifest, two contributions, the shape of
//! `ext-git`. A [`Decorator`] puts status badges on entries, an SVN column
//! next to them, a roll-up mark on folders with changes inside, and the
//! working copy's revision, URL and last commit into the info panel; the
//! "SVN" action group runs Update, Commit, Revert, Log and Add through the
//! `svn` command line; both only inside a working copy.
//!
//! Status is read through `svn status --xml` and `svn info --xml` once per
//! working copy and cached in [`WorkingCopies`] until the kernel reports a
//! change in the working copy or its `.svn` directory. A batch whose
//! working copy has no cached status gets [`CellValue::Pending`] at once
//! and its decorations through [`DecorateCtx::publish`] when the job that
//! reads the status is done.
//!
//! The crate builds as a `cdylib` that the plugin host loads from
//! `plugins/svn/` through the entry point [`ext::plugin!`] declares, and
//! as an `rlib` for the binary's opt-in `svn` feature and the tests.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

pub mod actions;
pub mod wc;

use ext::t;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use ext::Text;
use ext::{
    API_VERSION, Badge, Capabilities, CellValue, ColumnDef, ColumnId, DecorateCtx, Decoration,
    Decorator, Extension, Manifest, PanelSection, Platforms, Points, Registry, Version,
};
use types::{Entry, EntryId, Error, ExtId, Location};

pub use wc::{Commit, FileStatus, Snapshot, SvnError, WorkingCopies};

/// The extension id.
pub const ID: ExtId = ExtId("svn");

/// The decorator's column.
pub const STATUS_COLUMN: ColumnId = ColumnId("svn.status");

const COLUMNS: [ColumnDef; 1] = [ColumnDef {
    id: STATUS_COLUMN,
    label: Text("svn-svn"),
}];

/// The OS path of a local location; `None` for other providers.
#[must_use]
pub fn local_path(loc: &Location) -> Option<PathBuf> {
    (loc.provider.0 == "local").then(|| {
        let text = loc.path.to_string_lossy().into_owned();
        PathBuf::from(text)
    })
}

/// The extension: registers the decorator and the action group.
#[derive(Debug)]
pub struct SvnExtension {
    manifest: Manifest,
    copies: Arc<WorkingCopies>,
}

impl Default for SvnExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl SvnExtension {
    /// The extension with an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "SVN",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::DECORATOR | Points::ACTION,
                platforms: Platforms::ALL,
            },
            copies: Arc::new(WorkingCopies::default()),
        }
    }

    /// The working-copy cache the contributions share.
    #[must_use]
    pub fn copies(&self) -> &Arc<WorkingCopies> {
        &self.copies
    }
}

impl Extension for SvnExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[
            ("en", include_str!("../locales/en.ftl")),
            ("de", include_str!("../locales/de.ftl")),
        ]);
        reg.decorator(Arc::new(SvnDecorator::new(Arc::clone(&self.copies))));
        let shared = Arc::new(actions::Shared {
            copies: Arc::clone(&self.copies),
        });
        for action in actions::all(&shared) {
            reg.action(action);
        }
        Ok(())
    }
}

ext::plugin!(|| Arc::new(SvnExtension::new()) as Arc<dyn Extension>);

/// Batches waiting for a working copy's status to be read, with the
/// context to publish them through.
type Waiting = Vec<(Vec<Entry>, DecorateCtx)>;

/// Status badges, the SVN column and the panel section.
#[derive(Debug)]
pub struct SvnDecorator {
    copies: Arc<WorkingCopies>,
    /// Working copies whose status a job is reading right now, with the
    /// batches that arrived meanwhile; one read serves them all.
    reading: Arc<Mutex<HashMap<PathBuf, Waiting>>>,
}

impl SvnDecorator {
    /// A decorator over its own cache, for tests.
    #[must_use]
    pub fn new(copies: Arc<WorkingCopies>) -> Self {
        Self {
            copies,
            reading: Arc::default(),
        }
    }

    /// The decoration of one entry under `root` from `snapshot`.
    fn decoration(root: &Path, snapshot: &Snapshot, entry: &Entry) -> Decoration {
        let path = local_path(&Location::new(
            types::ProviderId("local"),
            entry.path.clone(),
        ));
        let status = path
            .as_deref()
            .and_then(|p| p.strip_prefix(root).ok())
            .map(|rela| rela.to_string_lossy().into_owned())
            .and_then(|rela| snapshot.status_of(&rela));
        let mut decoration = Decoration::default();
        match status {
            Some(status) => {
                decoration.badge = Some(Badge {
                    glyph: status.glyph(),
                    color: status.color(),
                });
                if status != FileStatus::Contains {
                    decoration.tint = Some(status.color());
                }
                decoration
                    .cells
                    .push((STATUS_COLUMN, CellValue::Text(status.label().to_owned())));
            }
            None => decoration
                .cells
                .push((STATUS_COLUMN, CellValue::Text(String::new()))),
        }
        decoration
    }

    fn decorate_with(
        root: &Path,
        snapshot: &Snapshot,
        entries: &[Entry],
    ) -> Vec<(EntryId, Decoration)> {
        entries
            .iter()
            .map(|entry| (entry.id, Self::decoration(root, snapshot, entry)))
            .collect()
    }
}

impl Decorator for SvnDecorator {
    fn id(&self) -> ExtId {
        ID
    }

    fn applies_to(&self, loc: &Location) -> bool {
        local_path(loc).is_some_and(|dir| self.copies.working_copy(&dir).is_some())
    }

    fn columns(&self) -> &[ColumnDef] {
        &COLUMNS
    }

    fn decorate(&self, entries: &[Entry], ctx: &DecorateCtx) -> Vec<(EntryId, Decoration)> {
        let Some(root) = local_path(&ctx.location).and_then(|dir| self.copies.working_copy(&dir))
        else {
            return Vec::new();
        };
        if let Some(snapshot) = self.copies.snapshot(&root) {
            return Self::decorate_with(&root, &snapshot, entries);
        }
        // Nothing cached: answer Pending now and publish when the job that
        // reads the status is done. Batches that arrive while it runs wait
        // for the same read.
        let pending: Vec<(EntryId, Decoration)> = entries
            .iter()
            .map(|entry| {
                let mut decoration = Decoration::default();
                decoration.cells.push((STATUS_COLUMN, CellValue::Pending));
                (entry.id, decoration)
            })
            .collect();
        let waiter = (entries.to_vec(), ctx.clone());
        {
            let mut reading = self.reading.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(waiting) = reading.get_mut(&root) {
                waiting.push(waiter);
                return pending;
            }
            reading.insert(root.clone(), vec![waiter]);
        }
        let copies = Arc::clone(&self.copies);
        let reading = Arc::clone(&self.reading);
        ctx.jobs.spawn_named(
            format!("SVN status {}", root.display()),
            move |_job| async move {
                let read_root = root.clone();
                let result =
                    ext::blocking(move || copies.refresh(&read_root).map_err(wc::to_error)).await;
                let waiting = reading
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&root)
                    .unwrap_or_default();
                let snapshot = match result {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        tracing::warn!(root = %root.display(), %error, "svn status not read");
                        // Clear the pending marks; the badges stay empty.
                        for (entries, ctx) in waiting {
                            if !ctx.cancel.is_cancelled() {
                                ctx.publish(Self::decorate_with(
                                    &root,
                                    &Snapshot::default(),
                                    &entries,
                                ));
                            }
                        }
                        return Err(error);
                    }
                };
                for (entries, ctx) in waiting {
                    if !ctx.cancel.is_cancelled() {
                        ctx.publish(Self::decorate_with(&root, &snapshot, &entries));
                    }
                }
                Ok(())
            },
        );
        pending
    }

    fn panel(&self, entry: &Entry) -> Option<PanelSection> {
        let path = local_path(&Location::new(
            types::ProviderId("local"),
            entry.path.clone(),
        ))?;
        let dir = if entry.kind == types::EntryKind::Dir {
            path.clone()
        } else {
            path.parent()?.to_path_buf()
        };
        let root = self.copies.working_copy(&dir)?;
        let snapshot = self.copies.snapshot(&root)?;
        let mut rows = vec![("Revision".to_owned(), snapshot.revision_text())];
        if let Some(url) = &snapshot.url {
            rows.push(("URL".to_owned(), url.clone()));
        }
        if let Some(repository) = &snapshot.repository {
            rows.push(("Repository".to_owned(), repository.clone()));
        }
        if let Some(commit) = &snapshot.last_commit {
            rows.push((
                "Last commit".to_owned(),
                format!(
                    "r{} by {} on {}",
                    commit.revision, commit.author, commit.date
                ),
            ));
        }
        let rela = path
            .strip_prefix(&root)
            .ok()
            .map(|r| r.to_string_lossy().into_owned());
        let status = rela
            .as_deref()
            .and_then(|r| snapshot.status_of(r))
            .map_or("Unchanged", FileStatus::label);
        rows.push(("Status".to_owned(), status.to_owned()));
        Some(PanelSection {
            title: t!("svn-svn"),
            rows,
        })
    }

    fn invalidates_on(&self, loc: &Location) -> Vec<Location> {
        let Some(root) = local_path(loc).and_then(|dir| self.copies.working_copy(&dir)) else {
            return Vec::new();
        };
        let svn_dir = root.join(".svn");
        let path = svn_dir.to_string_lossy().into_owned();
        vec![Location::new(loc.provider, path.as_str())]
    }

    fn invalidate(&self, loc: &Location) {
        if let Some(path) = local_path(loc) {
            self.copies.invalidate(&path);
        }
    }
}

#[cfg(test)]
mod tests;
