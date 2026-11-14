//! The ZFS section of the info panel.

use std::sync::Arc;
use std::time::Duration;

use ext::{DecorateCtx, Decorator, JobRunner};
use runtime::JobEvent;
use types::{CancelToken, Location};

use super::*;
use crate::actions::local;
use crate::decorator::ZfsDecorator;
use crate::props::{PROPERTIES, get_args};
use crate::zfs::Output;

fn script_properties(fx: &Fixture) {
    let args = get_args(&PROPERTIES, "tank/home");
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    fx.zfs.on(
        &args,
        Output::ok(
            "guid\t2701085171283325975\t-\ncompression\tzstd\tinherited from tank\nencryption\toff\tdefault\nkeystatus\t-\t-\n",
        ),
    );
}

fn decorate(decorator: &ZfsDecorator, at: Location, entries: &[types::Entry]) {
    let (jobs, mut events) = JobRunner::with_events();
    let ctx = DecorateCtx::new(at, CancelToken::new(), jobs, |_| {});
    assert!(
        decorator.decorate(entries, &ctx).is_empty(),
        "no decorations"
    );
    runtime::block_on(async {
        loop {
            match tokio::time::timeout(Duration::from_secs(30), events.recv()).await {
                Ok(Some(JobEvent::Finished { result, .. })) => {
                    assert!(result.is_ok(), "{result:?}");
                    return;
                }
                Ok(Some(_)) => {}
                other => panic!("no read: {other:?}"),
            }
        }
    });
}

#[test]
fn the_section_shows_the_dataset_its_snapshots_and_properties() {
    let fx = Fixture::new();
    script_properties(&fx);
    let decorator = ZfsDecorator::new(Arc::clone(&fx.datasets));
    let folder = local(&fx.home().join("ann/docs"));
    assert!(decorator.applies_to(&folder));
    assert_eq!(decorator.columns().len(), 0);
    let file = live_entry(&fx.home().join("ann/docs/report.txt"));

    // Before the read: the dataset and a loading row.
    let before = decorator
        .panel(&file)
        .unwrap_or_else(|| panic!("no section"));
    assert_eq!(before.title, "ZFS");
    assert_eq!(
        before.rows[0],
        ("Dataset".to_owned(), "tank/home".to_owned())
    );
    assert_eq!(before.rows[1].0, "Loading…");

    decorate(&decorator, folder, std::slice::from_ref(&file));
    let after = decorator
        .panel(&file)
        .unwrap_or_else(|| panic!("no section"));
    let row = |label: &str| {
        after
            .rows
            .iter()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(row("Dataset").as_deref(), Some("tank/home"));
    assert_eq!(row("Snapshots").as_deref(), Some("2"));
    assert!(
        row("Newest snapshot").is_some_and(|v| v.starts_with("b (")),
        "{after:?}"
    );
    assert_eq!(row("guid").as_deref(), Some("2701085171283325975"));
    assert_eq!(
        row("compression · inherited from tank").as_deref(),
        Some("zstd")
    );
    assert_eq!(row("encryption · default").as_deref(), Some("off"));
    assert!(!after.rows.iter().any(|(l, _)| l.starts_with("keystatus")));
    // The order of the fixed list.
    let labels: Vec<&str> = after.rows.iter().map(|(l, _)| l.as_str()).collect();
    let guid = labels.iter().position(|l| *l == "guid");
    let compression = labels.iter().position(|l| l.starts_with("compression"));
    assert!(guid < compression, "{labels:?}");
}

#[test]
fn an_entry_outside_a_dataset_has_no_section() {
    let fx = Fixture::new();
    let decorator = ZfsDecorator::new(Arc::clone(&fx.datasets));
    let outside = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let file = outside.path().join("x.txt");
    write(&file, "x");
    assert!(decorator.panel(&live_entry(&file)).is_none());
    assert!(!decorator.applies_to(&local(outside.path())));
    // Snapshot entries are the provider's, not live: no section either.
    let provider = fx.provider();
    let snap = zfs_entry(
        &provider,
        &crate::provider::snapshot_location("tank/home", "b", std::path::Path::new("ann/old.txt")),
    );
    assert!(decorator.panel(&snap).is_none());
}

#[test]
fn without_zfs_nothing_appears() {
    let datasets = Arc::new(crate::datasets::Datasets::new(Arc::new(
        crate::zfs::ScriptedZfs::missing(),
    )));
    let decorator = ZfsDecorator::new(datasets);
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    write(&dir.path().join("x"), "x");
    assert!(!decorator.applies_to(&local(dir.path())));
    assert!(
        decorator
            .panel(&live_entry(&dir.path().join("x")))
            .is_none()
    );
}

/// The time slider's timeline (spec requirement "ZFS timeline").
mod timeline {
    use std::path::Path;

    use super::*;
    use crate::parse;
    use crate::provider::snapshot_location;

    #[test]
    fn a_live_folder_has_one_point_per_snapshot() {
        let fx = Fixture::new();
        let decorator = ZfsDecorator::new(Arc::clone(&fx.datasets));
        assert!(decorator.offers_timelines());
        let folder = local(&fx.home().join("ann/docs"));
        let timeline = decorator
            .timeline(&folder)
            .unwrap_or_else(|| panic!("no timeline"));
        assert_eq!(timeline.live, folder);
        let points: Vec<(&str, &str, Location)> = timeline
            .points
            .iter()
            .map(|p| (p.label.as_str(), p.source.as_str(), p.location.clone()))
            .collect();
        let rel = Path::new("ann/docs");
        assert_eq!(
            points,
            [
                ("a", "ZFS", snapshot_location("tank/home", "a", rel)),
                ("b", "ZFS", snapshot_location("tank/home", "b", rel)),
            ],
            "oldest first"
        );
        assert!(timeline.points[0].time < timeline.points[1].time);
        assert_eq!(timeline.point_at(&folder), None, "the live folder is Now");
    }

    #[test]
    fn inside_a_snapshot_the_timeline_is_the_live_folders() {
        let fx = Fixture::new();
        let decorator = ZfsDecorator::new(Arc::clone(&fx.datasets));
        let inside = snapshot_location("tank/home", "a", Path::new("ann/docs"));
        assert!(decorator.applies_to(&inside));
        let from_inside = decorator
            .timeline(&inside)
            .unwrap_or_else(|| panic!("no timeline"));
        let live = decorator
            .timeline(&local(&fx.home().join("ann/docs")))
            .unwrap_or_else(|| panic!("no timeline"));
        assert_eq!(from_inside, live);
        assert_eq!(from_inside.point_at(&inside), Some(0), "the pane is at a");
    }

    #[test]
    fn snapshots_of_the_same_second_keep_zfs_order() {
        let fx = Fixture::new();
        // ZFS lists by creation; two taken in one second come in the order
        // they were taken.
        on_snapshots(
            &fx.zfs,
            "tank/home",
            "tank/home@a\t100\t10\ntank/home@b\t100\t20\n",
        );
        let decorator = ZfsDecorator::new(Arc::clone(&fx.datasets));
        let timeline = decorator
            .timeline(&local(&fx.home()))
            .unwrap_or_else(|| panic!("no timeline"));
        let labels: Vec<&str> = timeline.points.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["a", "b"], "a was taken first");
    }

    #[test]
    fn a_snapshots_root_leads_back_to_the_mount_point() {
        let fx = Fixture::new();
        let decorator = ZfsDecorator::new(Arc::clone(&fx.datasets));
        let root = snapshot_location("tank/home", "b", Path::new(""));
        let timeline = decorator
            .timeline(&root)
            .unwrap_or_else(|| panic!("no timeline"));
        assert_eq!(timeline.live, local(&fx.home()), "no trailing separator");
        assert_eq!(timeline.point_at(&root), Some(1));
    }

    #[test]
    fn a_dataset_without_snapshots_offers_none() {
        let fx = Fixture::new();
        let decorator = ZfsDecorator::new(Arc::clone(&fx.datasets));
        let in_tank = local(fx.dir.path());
        assert!(decorator.applies_to(&in_tank));
        assert_eq!(decorator.timeline(&in_tank), None);
        let elsewhere = local(Path::new("/nowhere/at/all"));
        assert_eq!(decorator.timeline(&elsewhere), None);
    }

    #[test]
    fn three_folders_of_a_dataset_read_the_list_once() {
        let fx = Fixture::new();
        let decorator = ZfsDecorator::new(Arc::clone(&fx.datasets));
        for rel in ["", "ann", "ann/docs"] {
            assert!(decorator.timeline(&local(&fx.home().join(rel))).is_some());
        }
        let list = parse::list_snapshots_args("tank/home");
        let reads = fx.zfs.calls().iter().filter(|call| **call == list).count();
        assert_eq!(reads, 1);
    }
}
