use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ext::platform::Fake;
use ext::{
    ActionCtx, ActionOutcome, Availability, Invocation, Level, PaneId, Provider, Registry, UiHost,
};
use runtime::JobRunner;
use types::{Entry, Location};

use super::*;

#[derive(Default)]
struct Scripted {
    form: Mutex<Option<Vec<(&'static str, ext::Value)>>>,
    /// The preview rows the last form computed for the values given.
    rows: Mutex<Vec<PreviewRow>>,
}

impl UiHost for Scripted {
    fn notify(&self, _: Level, _: &str) {}
    fn confirm(&self, _: &str, _: &str) -> bool {
        true
    }
    fn prompt(&self, _: &str, default: &str) -> Option<String> {
        Some(default.to_owned())
    }
    fn show(&self, _: &str, _: &str) {}
    fn form(&self, form: ext::Form) -> Option<ext::Values> {
        let overrides = self
            .form
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()?;
        let mut values = form.defaults();
        for (key, value) in overrides {
            values.set(key, value);
        }
        *self.rows.lock().unwrap_or_else(PoisonError::into_inner) = form.preview_rows(&values);
        Some(values)
    }
}

fn loc(path: &Path) -> Location {
    Location::new(
        vfs_local::PROVIDER_ID,
        ext::platform::paths::from_path(path),
    )
}

fn entries(paths: &[PathBuf]) -> Vec<Entry> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider = vfs_local::LocalProvider::new(fake);
    runtime::block_on(async {
        let mut out = Vec::new();
        for path in paths {
            out.push(
                provider
                    .stat(&loc(path))
                    .await
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display())),
            );
        }
        out
    })
}

fn ctx(folder: &Path, selection: &[PathBuf]) -> ActionCtx {
    ActionCtx {
        pane: PaneId(1),
        location: loc(folder),
        selection: entries(selection),
        invoked_from: Invocation::Shortcut,
        other: None,
    }
}

fn settle(jobs: &JobRunner) {
    let deadline = Instant::now() + Duration::from_mins(1);
    while !jobs.running().is_empty() {
        assert!(Instant::now() < deadline, "jobs still running");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{e}"))
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_thousand_files_rename_by_number_as_one_undo() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let paths: Vec<PathBuf> = (0..1000)
        .map(|i| {
            let path = dir.path().join(format!("shot {i}.jpg"));
            fs::write(&path, i.to_string()).unwrap_or_else(|e| panic!("{e}"));
            path
        })
        .collect();
    let before = names(dir.path());
    let ui = Scripted {
        form: Mutex::new(Some(vec![(
            "pattern",
            ext::Value::Text("holiday_{n}{ext}".into()),
        )])),
        rows: Mutex::new(Vec::new()),
    };
    let jobs = JobRunner::new();
    let c = ctx(dir.path(), &paths);
    assert_eq!(BulkRename.availability(&c), Availability::Enabled);
    let outcome = BulkRename
        .run(c, &jobs, &ui)
        .unwrap_or_else(|e| panic!("{e}"));
    settle(&jobs);
    let ActionOutcome::Undoable(command) = outcome else {
        panic!("not undoable: {outcome:?}");
    };
    assert_eq!(command.label(), "Rename 1000 entries");
    let rows = ui
        .rows
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(rows.len(), 1000);
    assert!(rows.iter().all(|r| r.problem.is_none()));
    let after = names(dir.path());
    assert_eq!(after.len(), 1000);
    assert!(
        after.contains(&"holiday_0001.jpg".to_owned()),
        "{:?}",
        &after[..3]
    );
    assert!(after.contains(&"holiday_1000.jpg".to_owned()));
    // The first selected entry became number one and kept its contents.
    assert_eq!(
        fs::read_to_string(dir.path().join("holiday_0001.jpg"))
            .ok()
            .as_deref(),
        Some("0")
    );

    command.undo(&jobs).unwrap_or_else(|e| panic!("{e}"));
    settle(&jobs);
    assert_eq!(names(dir.path()), before, "one undo brings every name back");
    command.redo(&jobs).unwrap_or_else(|e| panic!("{e}"));
    settle(&jobs);
    assert_eq!(names(dir.path()), after);
}

#[test]
fn swaps_work_and_clashes_refuse_the_rename() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    fs::write(&a, "A").unwrap_or_else(|e| panic!("{e}"));
    fs::write(&b, "B").unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("c.txt"), "C").unwrap_or_else(|e| panic!("{e}"));
    let jobs = JobRunner::new();
    // a → b and b → a at once.
    let ui = Scripted {
        form: Mutex::new(Some(vec![
            ("find", ext::Value::Text("^([ab])".into())),
            ("replace", ext::Value::Text("${1}x".into())),
            ("regex", ext::Value::Check(true)),
        ])),
        rows: Mutex::new(Vec::new()),
    };
    let outcome = BulkRename
        .run(ctx(dir.path(), &[a.clone(), b.clone()]), &jobs, &ui)
        .unwrap_or_else(|e| panic!("{e}"));
    settle(&jobs);
    assert!(matches!(outcome, ActionOutcome::Undoable(_)));
    assert_eq!(
        fs::read_to_string(dir.path().join("ax.txt"))
            .ok()
            .as_deref(),
        Some("A")
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("bx.txt"))
            .ok()
            .as_deref(),
        Some("B")
    );
    // Swap them: ax ↔ bx through a pattern that numbers them in reverse.
    let ax = dir.path().join("ax.txt");
    let bx = dir.path().join("bx.txt");
    let ui = Scripted {
        form: Mutex::new(Some(vec![
            ("pattern", ext::Value::Text("{n}{ext}".into())),
            ("start", ext::Value::Text("1".into())),
        ])),
        rows: Mutex::new(Vec::new()),
    };
    BulkRename
        .run(ctx(dir.path(), &[bx.clone(), ax.clone()]), &jobs, &ui)
        .unwrap_or_else(|e| panic!("{e}"));
    settle(&jobs);
    assert_eq!(
        fs::read_to_string(dir.path().join("1.txt")).ok().as_deref(),
        Some("B")
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("2.txt")).ok().as_deref(),
        Some("A")
    );
    let one = dir.path().join("1.txt");
    let two = dir.path().join("2.txt");
    let ui = Scripted {
        form: Mutex::new(Some(vec![("pattern", ext::Value::Text("{n}{ext}".into()))])),
        rows: Mutex::new(Vec::new()),
    };
    // 2 → 1 and 1 → 2: a swap.
    BulkRename
        .run(ctx(dir.path(), &[two, one]), &jobs, &ui)
        .unwrap_or_else(|e| panic!("{e}"));
    settle(&jobs);
    assert_eq!(
        fs::read_to_string(dir.path().join("1.txt")).ok().as_deref(),
        Some("A")
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("2.txt")).ok().as_deref(),
        Some("B")
    );

    // A clash with c.txt is refused, and the preview says so.
    let ui = Scripted {
        form: Mutex::new(Some(vec![("pattern", ext::Value::Text("c{ext}".into()))])),
        rows: Mutex::new(Vec::new()),
    };
    let result = BulkRename.run(ctx(dir.path(), &[dir.path().join("1.txt")]), &jobs, &ui);
    assert!(result.is_err(), "{result:?}");
    let rows = ui
        .rows
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(
        rows[0].problem.as_deref(),
        Some("another entry has this name")
    );
    assert!(dir.path().join("1.txt").is_file(), "nothing renamed");
    // A dismissed form does nothing.
    let ui = Scripted::default();
    let outcome = BulkRename
        .run(ctx(dir.path(), &[dir.path().join("1.txt")]), &jobs, &ui)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(outcome, ActionOutcome::Done));
}

#[test]
fn the_extension_registers_the_action() {
    let mut registry = Registry::new();
    registry
        .load(&RenameExtension::new())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(registry.actions().len(), 1);
    assert_eq!(registry.actions()[0].id(), BULK);
    assert!(registry.actions()[0].default_shortcut().is_some());
}
