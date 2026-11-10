//! The provider: paths, the overview levels, a snapshot's tree, versions,
//! and that nothing can be changed.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ext::{
    Capabilities, CopyOptions, Depth, LinkKind, ListOptions, Provider, RemoveMode, WriteOptions,
};
use tokio::io::AsyncReadExt as _;
use types::{Attrs, CancelToken, EntryKind, Error, Location, ProviderId};

use super::*;
use crate::provider::{
    PROVIDER_ID, Target, dataset_location, snapshot_location, target, versions_location,
};

fn loc(path: &str) -> Location {
    Location::new(PROVIDER_ID, path)
}

fn at(path: &str) -> Option<Target> {
    target(&loc(path)).ok()
}

fn not_found(path: &str) -> bool {
    matches!(target(&loc(path)), Err(Error::NotFound))
}

#[test]
fn paths_name_the_root_datasets_snapshots_and_versions() {
    assert_eq!(at("/"), Some(Target::Root));
    assert_eq!(at(""), Some(Target::Root));
    assert_eq!(
        at("/tank/home"),
        Some(Target::Dataset("tank/home".to_owned()))
    );
    assert_eq!(
        at("/tank/home/@b/ann/docs"),
        Some(Target::Snapshot {
            dataset: "tank/home".to_owned(),
            snapshot: "b".to_owned(),
            rel: PathBuf::from("ann/docs"),
        })
    );
    assert_eq!(
        at("/tank/home/@b"),
        Some(Target::Snapshot {
            dataset: "tank/home".to_owned(),
            snapshot: "b".to_owned(),
            rel: PathBuf::new(),
        })
    );
    // Only the first `@` component is the snapshot; later ones are names.
    assert_eq!(
        at("/tank/@a/x@y/@z"),
        Some(Target::Snapshot {
            dataset: "tank".to_owned(),
            snapshot: "a".to_owned(),
            rel: PathBuf::from("x@y/@z"),
        })
    );
    assert_eq!(
        at("/~versions/home/ann/report.txt"),
        Some(Target::Versions(PathBuf::from("/home/ann/report.txt")))
    );
    assert!(not_found("/@a"));
    assert!(not_found("/tank/@"));
    assert!(not_found("/~versions"));
    assert!(matches!(
        target(&Location::new(ProviderId("local"), "/")),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn locations_round_trip_and_a_snapshot_s_parent_is_its_dataset() {
    let snap = snapshot_location("tank/home", "b", Path::new("ann/docs"));
    assert_eq!(snap.path.to_string_lossy(), "/tank/home/@b/ann/docs");
    let root = snapshot_location("tank/home", "b", Path::new(""));
    assert_eq!(root.path.to_string_lossy(), "/tank/home/@b");
    assert_eq!(
        root.path.parent().map(|p| p.to_string_lossy().into_owned()),
        Some("/tank/home".to_owned())
    );
    assert_eq!(
        dataset_location("tank/home").path.to_string_lossy(),
        "/tank/home"
    );
    let versions = versions_location(Path::new("/home/ann/report.txt"));
    assert_eq!(
        target(&versions).ok(),
        Some(Target::Versions(PathBuf::from("/home/ann/report.txt")))
    );
}

#[test]
fn the_root_lists_the_datasets_with_snapshots() {
    let fx = Fixture::new();
    let provider = fx.provider();
    let root = list(&provider, &loc("/"));
    assert_eq!(names(&root), ["tank/home"]);
    assert_eq!(root[0].kind, EntryKind::Dir);
    assert_eq!(root[0].path.to_string_lossy(), "/tank/home");
}

#[test]
fn a_dataset_lists_its_snapshots_newest_first() {
    let fx = Fixture::new();
    let provider = fx.provider();
    let snaps = list(&provider, &dataset_location("tank/home"));
    let order: Vec<&str> = snaps.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(order, ["b", "a"]);
    assert_eq!(
        snaps[0].modified,
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(200))
    );
    assert_eq!(snaps[0].size, Some(20));
    assert_eq!(snaps[0].kind, EntryKind::Dir);
    assert_eq!(snaps[0].path.to_string_lossy(), "/tank/home/@b");
    // A dataset that is not mounted names nothing.
    let mut stream = provider.list(
        &dataset_location("tank/gone"),
        ListOptions::default(),
        CancelToken::new(),
    );
    let first = runtime::block_on(stream.next());
    assert!(matches!(first, Some(Err(Error::NotFound))));
}

#[test]
fn a_snapshot_lists_its_files_as_they_were() {
    let fx = Fixture::new();
    let provider = fx.provider();
    let ann = snapshot_location("tank/home", "b", Path::new("ann"));
    let shown = list(&provider, &ann);
    assert_eq!(names(&shown), ["docs", "link", "old.txt"]);
    let with_hidden = list_with(
        &provider,
        &ann,
        ListOptions {
            hidden: true,
            system: false,
            depth: Depth::One,
        },
    );
    assert_eq!(names(&with_hidden), [".hidden", "docs", "link", "old.txt"]);
    let hidden = with_hidden.iter().find(|e| e.name == ".hidden");
    assert!(hidden.is_some_and(|e| e.attrs.contains(Attrs::HIDDEN)));
    let link = shown.iter().find(|e| e.name == "link");
    assert!(
        link.is_some_and(|e| matches!(&e.kind, EntryKind::Symlink { broken: false, .. })),
        "{link:?}"
    );
    let docs = shown.iter().find(|e| e.name == "docs");
    assert_eq!(
        docs.map(|e| e.path.to_string_lossy().into_owned()),
        Some("/tank/home/@b/ann/docs".to_owned())
    );
    // Entry ids come from the snapshot file's device and inode.
    let raw = ext::platform::fs::stat(&fx.snapshot_dir("b").join("ann/docs"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(docs.map(|e| e.id.key), Some(raw.id));

    let deep = list_with(
        &provider,
        &ann,
        ListOptions {
            hidden: false,
            system: false,
            depth: Depth::Unlimited,
        },
    );
    assert!(deep.iter().any(|e| e.name == "report.txt"));
}

#[test]
fn stat_and_read_inside_a_snapshot() {
    let fx = Fixture::new();
    let provider = fx.provider();
    let file = snapshot_location("tank/home", "a", Path::new("ann/docs/report.txt"));
    runtime::block_on(async {
        let entry = provider.stat(&file).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(entry.name, "report.txt");
        assert_eq!(entry.size, Some(4));
        assert_eq!(entry.kind, EntryKind::File);
        let mut text = String::new();
        provider
            .open_read(&file)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .read_to_string(&mut text)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(text, "same");
        let snap = provider
            .stat(&snapshot_location("tank/home", "b", Path::new("")))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(snap.name, "b");
        assert_eq!(snap.kind, EntryKind::Dir);
        assert!(matches!(
            provider
                .stat(&snapshot_location("tank/home", "zz", Path::new("")))
                .await,
            Err(Error::NotFound)
        ));
        assert!(provider.stat(&dataset_location("tank/home")).await.is_ok());
        assert!(provider.stat(&loc("/")).await.is_ok());
    });
}

#[test]
fn nothing_can_be_changed() {
    let fx = Fixture::new();
    let provider = fx.provider();
    let file = snapshot_location("tank/home", "b", Path::new("ann/old.txt"));
    let dir = snapshot_location("tank/home", "b", Path::new("ann"));
    for at in [
        loc("/"),
        dataset_location("tank/home"),
        file.clone(),
        dir.clone(),
    ] {
        assert_eq!(provider.capabilities(&at), Capabilities::empty());
    }
    let unsupported = |result: Result<(), Error>| matches!(result, Err(Error::Unsupported(_)));
    runtime::block_on(async {
        assert!(unsupported(
            provider
                .open_write(&file, WriteOptions::default())
                .await
                .map(|_| ())
        ));
        assert!(unsupported(provider.create_dir(&dir).await.map(|_| ())));
        assert!(unsupported(provider.rename(&file, &dir).await.map(|_| ())));
        assert!(unsupported(provider.remove(&file, RemoveMode::Trash).await));
        assert!(unsupported(
            provider.remove(&file, RemoveMode::Permanent).await
        ));
        assert!(unsupported(
            provider
                .link(&file, &dir, LinkKind::Symbolic)
                .await
                .map(|_| ())
        ));
        assert!(unsupported(
            provider
                .copy(
                    &file,
                    &dir,
                    CopyOptions::default(),
                    ext::ProgressSink::channel().0,
                    CancelToken::new()
                )
                .await
        ));
    });
    assert!(matches!(provider.watch(&dir), Err(Error::Unsupported(_))));
    assert!(fx.snapshot_dir("b").join("ann/old.txt").exists());
}

#[test]
fn unchanged_versions_collapse_into_the_newest() {
    let fx = Fixture::new();
    // c holds a changed report; b and a the same older one.
    super::on_snapshots(
        &fx.zfs,
        "tank/home",
        "tank/home@a\t100\t10\ntank/home@b\t200\t20\ntank/home@c\t300\t30\n",
    );
    let c = fx.snapshot_dir("c").join("ann/docs/report.txt");
    write(&c, "changed");
    set_mtime(&c, 2_000);
    let provider = fx.provider();
    let live = fx.home().join("ann/docs/report.txt");
    let versions = list(&provider, &versions_location(&live));
    let order: Vec<&str> = versions.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(order, ["c — report.txt", "b — report.txt"]);
    assert_eq!(
        versions[1].path.to_string_lossy(),
        "/tank/home/@b/ann/docs/report.txt"
    );
    assert_eq!(versions[0].size, Some(7));
}

#[test]
fn a_file_deleted_live_still_has_its_versions() {
    let fx = Fixture::new();
    let provider = fx.provider();
    let live = fx.home().join("ann/old.txt");
    assert!(!live.exists());
    let versions = list(&provider, &versions_location(&live));
    let order: Vec<&str> = versions.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(order, ["b — old.txt"]);
    // Outside every dataset there is nothing to list.
    let mut stream = provider.list(
        &versions_location(Path::new("/nowhere/x")),
        ListOptions::default(),
        CancelToken::new(),
    );
    let first = runtime::block_on(stream.next());
    assert!(matches!(first, Some(Err(Error::NotFound))), "{first:?}");
}

#[test]
fn a_cancelled_versions_listing_ends() {
    let fx = Fixture::new();
    let provider = fx.provider();
    let cancel = CancelToken::new();
    cancel.cancel();
    let live = fx.home().join("ann/docs/report.txt");
    let mut stream = provider.list(&versions_location(&live), ListOptions::default(), cancel);
    assert!(runtime::block_on(stream.next()).is_none());
}
