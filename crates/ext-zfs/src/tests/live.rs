//! The opt-in test against a real pool.
//!
//! It runs only when `RICK_REQUIRE_ZFS=1` is set and `RICK_ZFS_TEST_DATASET`
//! names a mounted dataset the user may snapshot, rename and destroy
//! snapshots of and set `compression` and `quota` on without elevation
//! (`zfs allow <user> snapshot,rename,mount,create,destroy,compression,quota
//! <dataset>`); otherwise it prints a
//! note and passes. CI has no pool and never sets it. The test works in a
//! folder of its own under the mount point and on a snapshot of its own,
//! and removes both.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use ext::platform::Fake;

use super::*;
use crate::datasets::Datasets;
use crate::manage::{Batch, FirstRun, run_first};
use crate::provider::{dataset_location, snapshot_location};
use crate::restore::{items, restore};
use crate::zfs::SystemZfs;

#[test]
fn a_real_pool_creates_browses_restores_renames_and_destroys() {
    if std::env::var("RICK_REQUIRE_ZFS").as_deref() != Ok("1") {
        println!("RICK_REQUIRE_ZFS is not 1; the real-pool test is skipped");
        return;
    }
    let dataset = std::env::var("RICK_ZFS_TEST_DATASET")
        .unwrap_or_else(|_| panic!("RICK_REQUIRE_ZFS=1 needs RICK_ZFS_TEST_DATASET"));
    let datasets = Arc::new(Datasets::new(Arc::new(SystemZfs::new())));
    let fs_ = datasets
        .named(&dataset)
        .unwrap_or_else(|| panic!("{dataset} is not a mounted file system"))
        .clone();
    let stamp = crate::parse::default_name();
    let work = fs_.mountpoint.join(format!(".{stamp}-test"));
    let rel = work
        .strip_prefix(&fs_.mountpoint)
        .unwrap_or_else(|e| panic!("{e}"))
        .to_path_buf();
    fs::create_dir(&work).unwrap_or_else(|e| panic!("{}: {e}", work.display()));
    write(&work.join("report.txt"), "as it was\n");

    let run = |commands: Vec<Vec<String>>| {
        let outcome = run_first(&datasets, &Batch::new(dataset.clone(), commands));
        assert!(matches!(outcome, FirstRun::Done), "{outcome:?}");
    };
    let name = format!("{stamp}-test");
    let full = format!("{dataset}@{name}");
    run(vec![vec!["snapshot".to_owned(), full.clone()]]);

    let provider = ZfsProvider::new(Arc::clone(&datasets));
    let snaps = list(&provider, &dataset_location(&dataset));
    assert!(snaps.iter().any(|e| e.name == name), "{:?}", names(&snaps));
    let inside = list(&provider, &snapshot_location(&dataset, &name, &rel));
    assert_eq!(names(&inside), ["report.txt"]);

    fs::remove_file(work.join("report.txt")).unwrap_or_else(|e| panic!("{e}"));
    let shared = crate::actions::Shared {
        datasets: Arc::clone(&datasets),
        platform: None,
        elevate: None,
        helper: None,
    };
    let wanted = items(
        &shared,
        &[snapshot_location(&dataset, &name, &rel.join("report.txt"))],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let trash = Fake::new().unwrap_or_else(|e| panic!("{e}"));
    restore(&wanted, &trash, &types::CancelToken::new(), &mut |_| {})
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        fs::read_to_string(work.join("report.txt")).ok().as_deref(),
        Some("as it was\n")
    );

    let renamed = format!("{name}-renamed");
    run(vec![vec![
        "rename".to_owned(),
        full,
        format!("{dataset}@{renamed}"),
    ]]);
    let snaps = list(&provider, &dataset_location(&dataset));
    assert!(snaps.iter().any(|e| e.name == renamed));
    run(vec![vec![
        "destroy".to_owned(),
        format!("{dataset}@{renamed}"),
    ]]);
    let snaps = list(&provider, &dataset_location(&dataset));
    assert!(!snaps.iter().any(|e| e.name == renamed));
    fs::remove_dir_all(&work).unwrap_or_else(|e| panic!("{e}"));
    assert!(!Path::new(&work).exists());

    properties_round_trip(&datasets, &dataset);
}

/// The property editor's commands (change `add-zfs-property-editor`): set
/// `compression` and `quota`, read them back, and reset them as the editor
/// does: `compression` inherited, `quota` back to `none`.
fn properties_round_trip(datasets: &Arc<Datasets>, dataset: &str) {
    let ordered = |commands: Vec<Vec<String>>| {
        let outcome = run_first(datasets, &Batch::ordered(dataset.to_owned(), commands));
        assert!(matches!(outcome, FirstRun::Done), "{outcome:?}");
    };
    let before = crate::edit::read(&**datasets.zfs(), dataset).unwrap_or_else(|e| panic!("{e}"));
    let value = |current: &crate::edit::Current, name: &str| {
        current
            .properties
            .iter()
            .find(|p| p.name == name)
            .map(|p| (p.value.clone(), p.source.clone()))
    };
    let quota = before.used + 1024 * 1024 * 1024;
    ordered(vec![vec![
        "set".to_owned(),
        "compression=zle".to_owned(),
        format!("quota={quota}"),
        dataset.to_owned(),
    ]]);
    let after = crate::edit::read(&**datasets.zfs(), dataset).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        value(&after, "compression"),
        Some(("zle".to_owned(), crate::props::Source::Local))
    );
    assert_eq!(
        value(&after, "quota"),
        Some((quota.to_string(), crate::props::Source::Local))
    );
    // The resets as the editor builds them: compression inherited, the
    // quota set to none, since ZFS never inherits a quota.
    let after = Arc::new(after);
    let mut values = after.form().defaults();
    values.set("compression", ext::Value::Choice(0));
    values.set("quota", ext::Value::Text(String::new()));
    ordered(after.commands(&values).unwrap_or_else(|e| panic!("{e}")));
    let back = crate::edit::read(&**datasets.zfs(), dataset).unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(
        value(&back, "compression").map(|(_, s)| s),
        Some(crate::props::Source::Local)
    );
    assert_eq!(value(&back, "quota").map(|(v, _)| v), Some("0".to_owned()));
}
