//! Restore, with the platform's fake trash.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ext::platform::{Fake, Trash as _};
use ext::{Action, ActionOutcome, Availability, JobRunner};
use runtime::JobEvent;
use types::{CancelToken, Error, Location};

use super::*;
use crate::actions::{Shared, local};
use crate::provider::{snapshot_location, versions_location};
use crate::restore::{Restore, items, restore};

struct Bench {
    fx: Fixture,
    fake: Arc<Fake>,
    shared: Arc<Shared>,
}

fn bench() -> Bench {
    let fx = Fixture::new();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let shared = Arc::new(Shared {
        datasets: Arc::clone(&fx.datasets),
        platform: Some(fake.clone()),
        elevate: None,
        helper: None,
    });
    Bench { fx, fake, shared }
}

impl Bench {
    /// The entry at `rel` in snapshot `snap`.
    fn entry(&self, snap: &str, rel: &str) -> types::Entry {
        zfs_entry(
            &self.fx.provider(),
            &snapshot_location("tank/home", snap, Path::new(rel)),
        )
    }

    /// Runs Restore on `selection` in the folder `at` and waits for its
    /// job; `None` when no job was started.
    fn run(
        &self,
        at: Location,
        selection: Vec<types::Entry>,
        ui: &Scripted,
    ) -> Option<Result<(), Error>> {
        let (jobs, mut events) = JobRunner::with_events();
        let outcome = Restore(Arc::clone(&self.shared))
            .run(ctx(at, selection), &jobs, ui)
            .unwrap_or_else(|e| panic!("{e}"));
        let ActionOutcome::Started(id) = outcome else {
            return None;
        };
        runtime::block_on(async {
            loop {
                match tokio::time::timeout(Duration::from_secs(30), events.recv()).await {
                    Ok(Some(JobEvent::Finished { id: done, result })) if done == id => {
                        return Some(result);
                    }
                    Ok(Some(_)) => {}
                    other => panic!("no end of the restore job: {other:?}"),
                }
            }
        })
    }

    fn trashed(&self) -> Vec<std::path::PathBuf> {
        self.fake
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|i| i.original)
            .collect()
    }
}

fn ann(b: &Bench, snap: &str) -> Location {
    let _ = b;
    snapshot_location("tank/home", snap, Path::new("ann"))
}

#[test]
fn a_deleted_file_is_restored_with_its_snapshot_content() {
    let b = bench();
    let live = b.fx.home().join("ann/old.txt");
    let result = b.run(
        ann(&b, "b"),
        vec![b.entry("b", "ann/old.txt")],
        &Scripted::confirming(true),
    );
    assert!(matches!(result, Some(Ok(()))), "{result:?}");
    assert_eq!(fs::read_to_string(&live).ok().as_deref(), Some("gone live"));
    assert!(b.trashed().is_empty(), "nothing was in the way");
}

#[test]
fn the_question_names_the_snapshot_even_with_nothing_in_the_way() {
    let b = bench();
    let live = b.fx.home().join("ann/old.txt");
    let ui = Scripted::confirming(true);
    let result = b.run(ann(&b, "b"), vec![b.entry("b", "ann/old.txt")], &ui);
    assert!(matches!(result, Some(Ok(()))), "{result:?}");
    let asked = ui.asked();
    assert_eq!(asked.len(), 1, "one confirmation: {asked:?}");
    assert!(asked[0].contains("tank/home@b"), "{asked:?}");
    assert!(asked[0].contains(&live.display().to_string()), "{asked:?}");
    assert!(
        !asked[0].contains("trash"),
        "nothing is replaced: {asked:?}"
    );
}

#[test]
fn a_declined_restore_changes_nothing() {
    let b = bench();
    let live = b.fx.home().join("ann/old.txt");
    let ui = Scripted::confirming(false);
    let result = b.run(ann(&b, "b"), vec![b.entry("b", "ann/old.txt")], &ui);
    assert!(result.is_none(), "no job: {result:?}");
    assert!(!live.exists(), "nothing restored");
}

#[test]
fn a_live_file_is_trashed_and_replaced_after_confirmation() {
    let b = bench();
    let live = b.fx.home().join("ann/docs/report.txt");
    let ui = Scripted::confirming(true);
    let docs = snapshot_location("tank/home", "b", Path::new("ann/docs"));
    let result = b.run(docs, vec![b.entry("b", "ann/docs/report.txt")], &ui);
    assert!(matches!(result, Some(Ok(()))), "{result:?}");
    assert_eq!(fs::read_to_string(&live).ok().as_deref(), Some("same"));
    assert_eq!(b.trashed(), std::slice::from_ref(&live));
    let asked = ui.asked();
    assert_eq!(asked.len(), 1, "one confirmation: {asked:?}");
    assert!(asked[0].contains(&live.display().to_string()), "{asked:?}");
    assert!(asked[0].contains("tank/home@b"), "{asked:?}");
    assert!(asked[0].contains("trash"), "{asked:?}");
    // The snapshot's modification time came along.
    let modified = fs::metadata(&live).and_then(|m| m.modified()).ok();
    assert_eq!(
        modified,
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000))
    );
}

#[test]
fn a_declined_replacement_changes_nothing() {
    let b = bench();
    let live = b.fx.home().join("ann/docs/report.txt");
    let ui = Scripted::confirming(false);
    let docs = snapshot_location("tank/home", "b", Path::new("ann/docs"));
    let result = b.run(docs, vec![b.entry("b", "ann/docs/report.txt")], &ui);
    assert!(result.is_none(), "no job: {result:?}");
    assert_eq!(fs::read_to_string(&live).ok().as_deref(), Some("live"));
    assert_eq!(b.trashed(), Vec::<std::path::PathBuf>::new());
    assert_eq!(ui.asked().len(), 1);
}

#[test]
fn missing_live_parents_are_created() {
    let b = bench();
    write(
        &b.fx.snapshot_dir("b").join("ann/gone/deeper/f.txt"),
        "deep",
    );
    let at = snapshot_location("tank/home", "b", Path::new("ann/gone/deeper"));
    let result = b.run(
        at,
        vec![b.entry("b", "ann/gone/deeper/f.txt")],
        &Scripted::confirming(true),
    );
    assert!(matches!(result, Some(Ok(()))), "{result:?}");
    assert_eq!(
        fs::read_to_string(b.fx.home().join("ann/gone/deeper/f.txt"))
            .ok()
            .as_deref(),
        Some("deep")
    );
}

#[test]
fn a_folder_comes_back_with_its_files_and_links() {
    let b = bench();
    let proj = b.fx.snapshot_dir("b").join("ann/proj");
    write(&proj.join("a.txt"), "a");
    write(&proj.join("sub/b.txt"), "b");
    ext::platform::fs::symlink(Path::new("a.txt"), &proj.join("ln"))
        .unwrap_or_else(|e| panic!("{e}"));
    if let Ok(Some(_)) = ext::platform::fs::unix_mode(&proj.join("a.txt")) {
        ext::platform::fs::set_unix_mode(&proj.join("a.txt"), 0o640)
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let result = b.run(
        ann(&b, "b"),
        vec![b.entry("b", "ann/proj")],
        &Scripted::confirming(true),
    );
    assert!(matches!(result, Some(Ok(()))), "{result:?}");
    let live = b.fx.home().join("ann/proj");
    assert_eq!(
        fs::read_to_string(live.join("a.txt")).ok().as_deref(),
        Some("a")
    );
    assert_eq!(
        fs::read_to_string(live.join("sub/b.txt")).ok().as_deref(),
        Some("b")
    );
    assert_eq!(
        fs::read_link(live.join("ln")).ok(),
        Some(std::path::PathBuf::from("a.txt"))
    );
    if let Ok(Some(mode)) = ext::platform::fs::unix_mode(&live.join("a.txt")) {
        assert_eq!(mode & 0o777, 0o640);
    }
}

#[test]
fn a_version_from_the_versions_view_is_restored() {
    let b = bench();
    let live = b.fx.home().join("ann/old.txt");
    let version = b.entry("b", "ann/old.txt");
    let result = b.run(
        versions_location(&live),
        vec![version],
        &Scripted::confirming(true),
    );
    assert!(matches!(result, Some(Ok(()))), "{result:?}");
    assert!(live.exists());
}

#[test]
fn an_item_whose_live_version_cannot_be_trashed_is_skipped() {
    let b = bench();
    let docs = b.fx.home().join("ann/docs");
    let Ok(Some(mode)) = ext::platform::fs::unix_mode(&docs) else {
        return; // No permission bits to take away here.
    };
    ext::platform::fs::set_unix_mode(&docs, 0o555).unwrap_or_else(|e| panic!("{e}"));
    let writable = fs::write(docs.join("probe"), "x").is_ok();
    let _ = fs::remove_file(docs.join("probe"));
    if writable {
        // Running as root: permissions do not stop anything.
        ext::platform::fs::set_unix_mode(&docs, mode).unwrap_or_else(|e| panic!("{e}"));
        return;
    }
    let restore_items = items(
        &b.shared,
        &[
            snapshot_location("tank/home", "b", Path::new("ann/docs/report.txt")),
            snapshot_location("tank/home", "b", Path::new("ann/old.txt")),
        ],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let result = restore(&restore_items, &*b.fake, &CancelToken::new(), &mut |_| {});
    ext::platform::fs::set_unix_mode(&docs, mode).unwrap_or_else(|e| panic!("{e}"));
    let message = result.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(message.contains("report.txt"), "{message}");
    assert_eq!(
        fs::read_to_string(docs.join("report.txt")).ok().as_deref(),
        Some("live"),
        "the live file stays"
    );
    assert!(
        b.fx.home().join("ann/old.txt").exists(),
        "the other item is restored"
    );
}

#[test]
fn a_cancelled_restore_stops_before_copying() {
    let b = bench();
    let restore_items = items(
        &b.shared,
        &[snapshot_location(
            "tank/home",
            "b",
            Path::new("ann/old.txt"),
        )],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let cancel = CancelToken::new();
    cancel.cancel();
    let result = restore(&restore_items, &*b.fake, &cancel, &mut |_| {});
    assert!(matches!(result, Err(Error::Cancelled)));
    assert!(!b.fx.home().join("ann/old.txt").exists());
}

#[test]
fn restore_is_offered_inside_snapshots_and_versions_only() {
    let b = bench();
    let action = Restore(Arc::clone(&b.shared));
    let selected = vec![b.entry("b", "ann/old.txt")];
    assert_eq!(
        action.availability(&ctx(ann(&b, "b"), selected.clone())),
        Availability::Enabled
    );
    assert!(matches!(
        action.availability(&ctx(ann(&b, "b"), Vec::new())),
        Availability::Disabled(_)
    ));
    let live = b.fx.home().join("ann/old.txt");
    assert_eq!(
        action.availability(&ctx(versions_location(&live), selected.clone())),
        Availability::Enabled
    );
    assert_eq!(
        action.availability(&ctx(local(&b.fx.home()), selected.clone())),
        Availability::Hidden
    );
    assert_eq!(
        action.availability(&ctx(
            crate::provider::dataset_location("tank/home"),
            selected.clone()
        )),
        Availability::Hidden
    );
    let without_platform = Restore(Arc::new(Shared {
        datasets: Arc::clone(&b.fx.datasets),
        platform: None,
        elevate: None,
        helper: None,
    }));
    assert_eq!(
        without_platform.availability(&ctx(ann(&b, "b"), selected)),
        Availability::Hidden
    );
    // A whole snapshot is never restored over its dataset.
    assert!(
        items(
            &b.shared,
            &[snapshot_location("tank/home", "b", Path::new(""))]
        )
        .is_err()
    );
}
