use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ext::platform::Fake;
use ext::{
    Action, ActionCtx, ActionOutcome, Availability, Diff, HunkKind, Invocation, Level, PaneId,
    Provider, Registry, RowStatus, UiHost, View,
};
use runtime::JobRunner;
use types::{CancelToken, Entry, Location};

use super::*;

struct Headless;

impl UiHost for Headless {
    fn notify(&self, _: Level, _: &str) {}
    fn confirm(&self, _: &str, _: &str) -> bool {
        true
    }
    fn prompt(&self, _: &str, default: &str) -> Option<String> {
        Some(default.to_owned())
    }
    fn show(&self, _: &str, _: &str) {}
    fn form(&self, form: ext::Form) -> Option<ext::Values> {
        Some(form.defaults())
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

fn ctx(location: &Path, selection: &[PathBuf], other: Option<&Path>) -> ActionCtx {
    ActionCtx {
        pane: PaneId(1),
        location: loc(location),
        selection: entries(selection),
        invoked_from: Invocation::Palette,
        other: other.map(loc),
    }
}

/// Runs the action and waits for its job; the comparison it opened.
fn run(ctx: ActionCtx) -> Arc<ext::Compare> {
    let jobs = JobRunner::new();
    let outcome = CompareAction
        .run(ctx, &jobs, &Headless)
        .unwrap_or_else(|e| panic!("{e}"));
    let ActionOutcome::View(View::Compare(compare)) = outcome else {
        panic!("not a compare view: {outcome:?}");
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while compare.result().is_none() {
        assert!(Instant::now() < deadline, "no result");
        std::thread::sleep(Duration::from_millis(5));
    }
    compare
}

#[test]
fn two_files_diff_line_by_line_and_the_changed_lines_are_found() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    fs::write(&a, "one\ntwo\nthree\nfour\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(&b, "one\n2\nthree\nfour\nfive\r\n").unwrap_or_else(|e| panic!("{e}"));
    let c = ctx(dir.path(), &[a.clone(), b.clone()], None);
    assert_eq!(CompareAction.availability(&c), Availability::Enabled);
    let compare = run(c);
    assert_eq!(compare.title, "a.txt ↔ b.txt");
    let result = compare
        .result()
        .and_then(Result::ok)
        .unwrap_or_else(|| panic!("ok"));
    let Diff::Text(diff) = &*result else {
        panic!("not text: {result:?}");
    };
    assert_eq!(diff.left.len(), 4);
    assert_eq!(diff.right.len(), 5);
    assert_eq!(diff.changed_lines(), 2);
    let kinds: Vec<HunkKind> = diff.hunks.iter().map(|h| h.kind).collect();
    assert_eq!(
        kinds,
        [
            HunkKind::Equal,
            HunkKind::Replace,
            HunkKind::Equal,
            HunkKind::Insert
        ]
    );
    assert_eq!(diff.hunks[1].left, 1..2);
    assert_eq!(diff.hunks[1].right, 1..2);
    assert_eq!(diff.hunks[3].right, 4..5);
    assert_eq!(diff.right[4], "five", "a CR LF ending is stripped");

    // Identical texts: one equal hunk, nothing changed.
    let same = text_diff("x\ny\n", "x\ny");
    assert_eq!(same.changed_lines(), 0);
    assert_eq!(same.hunks.len(), 1);
}

#[test]
fn binary_files_are_compared_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let a = dir.path().join("a.bin");
    let b = dir.path().join("b.bin");
    let c = dir.path().join("c.bin");
    fs::write(&a, [0u8, 1, 2, 3]).unwrap_or_else(|e| panic!("{e}"));
    fs::write(&b, [0u8, 1, 2, 3]).unwrap_or_else(|e| panic!("{e}"));
    fs::write(&c, [0u8, 1, 2, 4]).unwrap_or_else(|e| panic!("{e}"));
    let cancel = CancelToken::new();
    assert_eq!(
        compare_files(&a, &b, &cancel).unwrap_or_else(|e| panic!("{e}")),
        Diff::Binary { identical: true }
    );
    assert_eq!(
        compare_files(&a, &c, &cancel).unwrap_or_else(|e| panic!("{e}")),
        Diff::Binary { identical: false }
    );
}

#[test]
fn two_folders_list_added_removed_and_changed_and_the_other_pane_counts() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let left = dir.path().join("left");
    let right = dir.path().join("right");
    fs::create_dir_all(left.join("both")).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(right.join("both")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(left.join("same.txt"), "same").unwrap_or_else(|e| panic!("{e}"));
    fs::write(right.join("same.txt"), "same").unwrap_or_else(|e| panic!("{e}"));
    fs::write(left.join("changed.txt"), "short").unwrap_or_else(|e| panic!("{e}"));
    fs::write(right.join("changed.txt"), "a longer text").unwrap_or_else(|e| panic!("{e}"));
    fs::write(left.join("removed.txt"), "gone").unwrap_or_else(|e| panic!("{e}"));
    fs::write(right.join("added.txt"), "new").unwrap_or_else(|e| panic!("{e}"));
    // Same size and content but another time still differs.
    fs::write(left.join("older.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    fs::write(right.join("older.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    let old = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    fs::File::options()
        .write(true)
        .open(left.join("older.txt"))
        .and_then(|f| f.set_modified(old))
        .unwrap_or_else(|e| panic!("{e}"));

    // Two selected folders.
    let compare = run(ctx(dir.path(), &[left.clone(), right.clone()], None));
    let result = compare
        .result()
        .and_then(Result::ok)
        .unwrap_or_else(|| panic!("ok"));
    let Diff::Folders(folders) = &*result else {
        panic!("not folders: {result:?}");
    };
    let statuses: Vec<(String, RowStatus)> = folders
        .rows
        .iter()
        .map(|r| (r.name.clone(), r.status))
        .collect();
    assert_eq!(
        statuses,
        [
            ("added.txt".to_owned(), RowStatus::Added),
            ("both".to_owned(), RowStatus::Same),
            ("changed.txt".to_owned(), RowStatus::Changed),
            ("older.txt".to_owned(), RowStatus::Changed),
            ("removed.txt".to_owned(), RowStatus::Removed),
            ("same.txt".to_owned(), RowStatus::Same),
        ]
    );
    assert_eq!(folders.left, loc(&left));
    let added = &folders.rows[0];
    assert!(added.left.is_none());
    assert_eq!(
        added.right.as_ref().map(|s| s.location.clone()),
        Some(loc(&right.join("added.txt")))
    );

    // Nothing selected, the other pane's folder: the same comparison.
    let c = ctx(&left, &[], Some(&right));
    assert_eq!(CompareAction.availability(&c), Availability::Enabled);
    let compare = run(c);
    assert_eq!(compare.title, "left ↔ right");
    // Nothing selected and no other pane: disabled, with a reason.
    assert!(matches!(
        CompareAction.availability(&ctx(&left, &[], None)),
        Availability::Disabled(_)
    ));
    // A file and a folder: disabled.
    assert!(matches!(
        CompareAction.availability(&ctx(
            dir.path(),
            &[left.clone(), right.join("added.txt")],
            None
        )),
        Availability::Disabled(_)
    ));
    // One file: hidden.
    assert_eq!(
        CompareAction.availability(&ctx(dir.path(), std::slice::from_ref(&left), None)),
        Availability::Hidden
    );
}

#[test]
fn the_extension_registers_the_action() {
    let mut registry = Registry::new();
    registry
        .load(&CompareExtension::new())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(registry.actions().len(), 1);
    assert_eq!(registry.actions()[0].id(), COMPARE);
    assert_eq!(registry.extensions(), &[ID]);
}
