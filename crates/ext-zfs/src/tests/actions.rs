//! The navigation actions and Versions.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use ext::{Action, ActionOutcome, Availability, JobRunner};
use types::Location;

use super::*;
use crate::actions::{self, Browse, Live, Shared, Versions, local};
use crate::provider::{Target, snapshot_location, target, versions_location};

fn shared(fx: &Fixture) -> Arc<Shared> {
    Arc::new(Shared {
        datasets: Arc::clone(&fx.datasets),
        platform: None,
        elevate: None,
        helper: None,
    })
}

/// What the decorator's job does while the folder `rel` of `tank/home` is
/// shown: find the newest snapshot holding it.
fn shown(fx: &Fixture, rel: &str) {
    let fs = fx
        .datasets
        .named("tank/home")
        .cloned()
        .unwrap_or_else(|| panic!("no tank/home"));
    fx.datasets
        .newest_holding(&fs, Path::new(rel))
        .unwrap_or_else(|e| panic!("{e}"));
}

fn navigated(outcome: Result<ActionOutcome, types::Error>) -> Location {
    match outcome {
        Ok(ActionOutcome::Navigate(to)) => to,
        other => panic!("not a navigation: {other:?}"),
    }
}

#[test]
fn browse_goes_into_the_newest_snapshot_holding_the_folder() {
    let fx = Fixture::new();
    let browse = Browse(shared(&fx));
    let docs = ctx(local(&fx.home().join("ann/docs")), Vec::new());
    shown(&fx, "ann/docs");
    assert_eq!(browse.availability(&docs), Availability::Enabled);
    let to = navigated(browse.run(docs, &JobRunner::new(), &Scripted::default()));
    assert_eq!(
        to,
        snapshot_location("tank/home", "b", Path::new("ann/docs"))
    );

    // A folder only the older snapshot holds.
    fs::create_dir_all(fx.snapshot_dir("a").join("ann/kept")).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(fx.home().join("ann/kept")).unwrap_or_else(|e| panic!("{e}"));
    let kept = ctx(local(&fx.home().join("ann/kept")), Vec::new());
    shown(&fx, "ann/kept");
    let to = navigated(browse.run(kept, &JobRunner::new(), &Scripted::default()));
    assert_eq!(
        to,
        snapshot_location("tank/home", "a", Path::new("ann/kept"))
    );

    // The dataset's own root.
    let root = ctx(local(&fx.home()), Vec::new());
    shown(&fx, "");
    let to = navigated(browse.run(root, &JobRunner::new(), &Scripted::default()));
    assert_eq!(to, snapshot_location("tank/home", "b", Path::new("")));
}

#[test]
fn browse_is_disabled_for_a_folder_no_snapshot_holds() {
    let fx = Fixture::new();
    fs::create_dir_all(fx.home().join("ann/fresh")).unwrap_or_else(|e| panic!("{e}"));
    let browse = Browse(shared(&fx));
    let fresh = ctx(local(&fx.home().join("ann/fresh")), Vec::new());
    shown(&fx, "ann/fresh");
    assert!(matches!(
        browse.availability(&fresh),
        Availability::Disabled(_)
    ));
    assert!(
        browse
            .run(fresh, &JobRunner::new(), &Scripted::default())
            .is_err()
    );
}

#[test]
fn browse_runs_no_zfs_and_looks_into_no_snapshot_on_the_app_thread() {
    if !crate::supported() {
        return;
    }
    let fx = Fixture::new();
    let browse = Browse(shared(&fx));
    let docs = ctx(local(&fx.home().join("ann/docs")), Vec::new());
    // The file systems are read once per start, as every contribution
    // needs them; everything after that belongs in a job.
    let _ = fx.datasets.filesystems();
    let before = fx.zfs.calls().len();

    // Nothing read yet: disabled while the snapshots are read, and no
    // `zfs list -t snapshot` from the menu or the run.
    assert!(matches!(
        browse.availability(&docs),
        Availability::Disabled(_)
    ));
    assert!(
        browse
            .run(docs.clone(), &JobRunner::new(), &Scripted::default())
            .is_err()
    );
    assert_eq!(
        fx.zfs.calls().len(),
        before,
        "zfs ran on the app thread: {:?}",
        &fx.zfs.calls()[before..]
    );

    // Once a job found the newest snapshot holding the folder (the
    // decorator's, while the folder is shown), the menu answers at once.
    let fs = fx
        .datasets
        .named("tank/home")
        .cloned()
        .unwrap_or_else(|| panic!("no tank/home"));
    let found = fx
        .datasets
        .newest_holding(&fs, Path::new("ann/docs"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(found.as_deref(), Some("b"));
    let after = fx.zfs.calls().len();
    assert_eq!(browse.availability(&docs), Availability::Enabled);
    let to = navigated(browse.run(docs, &JobRunner::new(), &Scripted::default()));
    assert_eq!(
        to,
        snapshot_location("tank/home", "b", Path::new("ann/docs"))
    );
    assert_eq!(fx.zfs.calls().len(), after, "no zfs once the job ran");
}

/// The dataset's list is read again by whoever asks next (the time slider,
/// another pane) and dropped when a snapshot job ends. What was found for a
/// folder stays, as nothing reads for a folder that is merely shown; its
/// next read looks again.
#[test]
fn browse_keeps_its_answer_when_the_list_is_read_again() {
    if !crate::supported() {
        return;
    }
    let fx = Fixture::new();
    let browse = Browse(shared(&fx));
    let docs = ctx(local(&fx.home().join("ann/docs")), Vec::new());
    let current = || {
        fx.datasets
            .holding_is_current("tank/home", Path::new("ann/docs"))
    };
    assert!(!current(), "nothing found yet");
    shown(&fx, "ann/docs");
    assert_eq!(browse.availability(&docs), Availability::Enabled);
    assert!(current());

    fx.datasets.invalidate("tank/home");
    fx.datasets
        .snapshots("tank/home")
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        browse.availability(&docs),
        Availability::Enabled,
        "left reading with nobody to read"
    );
    assert!(!current(), "found against the list before");

    shown(&fx, "ann/docs");
    assert!(current());
}

#[test]
fn browse_is_hidden_off_zfs_and_inside_a_snapshot() {
    let fx = Fixture::new();
    let browse = Browse(shared(&fx));
    let off = ctx(local(Path::new("/definitely/not/zfs")), Vec::new());
    assert_eq!(browse.availability(&off), Availability::Hidden);
    let inside = ctx(
        snapshot_location("tank/home", "b", Path::new("ann")),
        Vec::new(),
    );
    assert_eq!(browse.availability(&inside), Availability::Hidden);
}

#[test]
fn back_to_live_goes_to_the_folder_or_its_nearest_parent() {
    if !crate::supported() {
        return;
    }
    let fx = Fixture::new();
    let live = Live(shared(&fx));
    let docs = ctx(
        snapshot_location("tank/home", "b", Path::new("ann/docs")),
        Vec::new(),
    );
    assert_eq!(live.availability(&docs), Availability::Enabled);
    let to = navigated(live.run(docs, &JobRunner::new(), &Scripted::default()));
    assert_eq!(to, local(&fx.home().join("ann/docs")));

    // A folder gone live: its nearest existing parent.
    fs::create_dir_all(fx.snapshot_dir("b").join("ann/gone/deeper"))
        .unwrap_or_else(|e| panic!("{e}"));
    let gone = ctx(
        snapshot_location("tank/home", "b", Path::new("ann/gone/deeper")),
        Vec::new(),
    );
    let to = navigated(live.run(gone, &JobRunner::new(), &Scripted::default()));
    assert_eq!(to, local(&fx.home().join("ann")));

    // From a versions view: the file's folder.
    let report = fx.home().join("ann/docs/report.txt");
    let versions = ctx(versions_location(&report), Vec::new());
    let to = navigated(live.run(versions, &JobRunner::new(), &Scripted::default()));
    assert_eq!(to, local(&fx.home().join("ann/docs")));

    let outside = ctx(local(&fx.home()), Vec::new());
    assert_eq!(live.availability(&outside), Availability::Hidden);
}

#[test]
fn versions_opens_for_one_live_or_snapshot_file() {
    if !crate::supported() {
        return;
    }
    let fx = Fixture::new();
    let versions = Versions(shared(&fx));
    let report = fx.home().join("ann/docs/report.txt");
    let folder = local(&fx.home().join("ann/docs"));

    let none = ctx(folder.clone(), Vec::new());
    assert!(matches!(
        versions.availability(&none),
        Availability::Disabled(_)
    ));
    let dir = ctx(
        folder.clone(),
        vec![live_entry(&fx.home().join("ann/docs"))],
    );
    assert!(matches!(
        versions.availability(&dir),
        Availability::Disabled(_)
    ));

    let one = ctx(folder, vec![live_entry(&report)]);
    assert_eq!(versions.availability(&one), Availability::Enabled);
    let to = navigated(versions.run(one, &JobRunner::new(), &Scripted::default()));
    assert_eq!(to, versions_location(&report));

    // A file in a snapshot that no longer exists live.
    let provider = fx.provider();
    let old = snapshot_location("tank/home", "b", Path::new("ann/old.txt"));
    let in_snapshot = ctx(
        snapshot_location("tank/home", "b", Path::new("ann")),
        vec![zfs_entry(&provider, &old)],
    );
    assert_eq!(versions.availability(&in_snapshot), Availability::Enabled);
    let to = navigated(versions.run(in_snapshot, &JobRunner::new(), &Scripted::default()));
    assert_eq!(
        target(&to).ok(),
        Some(Target::Versions(fx.home().join("ann/old.txt")))
    );

    let off = ctx(local(Path::new("/definitely/not/zfs")), Vec::new());
    assert_eq!(versions.availability(&off), Availability::Hidden);
}

#[test]
fn the_group_registers_with_the_extension() {
    let fx = Fixture::new();
    let mut registry = ext::Registry::new();
    registry
        .load(&crate::ZfsExtension::with_datasets(Arc::clone(
            &fx.datasets,
        )))
        .unwrap_or_else(|e| panic!("{e}"));
    let ids: Vec<&str> = registry.actions().iter().map(|a| a.id().0).collect();
    for id in [actions::BROWSE.0, actions::LIVE.0, actions::VERSIONS.0] {
        assert!(ids.contains(&id), "{id} in {ids:?}");
    }
    assert!(
        registry
            .actions()
            .iter()
            .all(|a| a.group() == actions::GROUP)
    );
    assert_eq!(registry.providers().len(), 1);
}
