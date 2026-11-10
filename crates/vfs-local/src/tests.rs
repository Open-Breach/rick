use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ext::{
    CopyOptions, Depth, FsChange, LinkKind, ListOptions, Provider, RemoveMode, WriteOptions,
};
use futures::StreamExt;
use platform::Trash as _;
use runtime::ProgressSink;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use types::{CancelToken, EntryKind, Error, Location, Progress};

use super::{LocalProvider, PROVIDER_ID};

fn provider() -> (LocalProvider, Arc<platform::Fake>) {
    let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
    (LocalProvider::new(fake.clone()), fake)
}

fn loc(path: &Path) -> Location {
    Location::new(PROVIDER_ID, platform::paths::from_path(path))
}

async fn collect(provider: &LocalProvider, dir: &Path, opts: ListOptions) -> Vec<types::Entry> {
    let mut stream = provider.list(&loc(dir), opts, CancelToken::new());
    let mut all = Vec::new();
    while let Some(batch) = stream.next().await {
        all.extend(batch.unwrap_or_else(|e| panic!("{e}")));
    }
    all.sort_by(|a, b| a.path.cmp(&b.path));
    all
}

#[test]
fn listing_reports_kinds_sizes_and_hidden_filtering() {
    let (provider, _) = provider();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("a.txt"), b"hello").unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(dir.path().join("sub")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("sub/deep.txt"), b"x").unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join(".hidden"), b"").unwrap_or_else(|e| panic!("{e}"));

    runtime::block_on(async {
        let visible = collect(&provider, dir.path(), ListOptions::default()).await;
        let names: Vec<&str> = visible.iter().map(|e| e.name.as_str()).collect();
        // A dot-file is hidden on every platform (SPEC.md *Hidden files*);
        // on Windows the hidden attribute hides a file too.
        assert_eq!(names, ["a.txt", "sub"]);
        let file = visible
            .iter()
            .find(|e| e.name == "a.txt")
            .unwrap_or_else(|| panic!("a.txt"));
        assert_eq!(file.kind, EntryKind::File);
        assert_eq!(file.size, Some(5));
        assert!(file.modified.is_some());
        assert_eq!(file.id.provider, PROVIDER_ID);
        assert!(file.path.to_string_lossy().ends_with("a.txt"));

        let everything = collect(
            &provider,
            dir.path(),
            ListOptions {
                hidden: true,
                system: true,
                depth: Depth::One,
            },
        )
        .await;
        assert_eq!(everything.len(), 3);

        let flattened = collect(
            &provider,
            dir.path(),
            ListOptions {
                hidden: true,
                system: true,
                depth: Depth::Unlimited,
            },
        )
        .await;
        assert_eq!(flattened.len(), 4, "{flattened:?}");
        assert!(flattened.iter().any(|e| e.name == "deep.txt"));

        let missing = provider
            .list(
                &loc(&dir.path().join("nope")),
                ListOptions::default(),
                CancelToken::new(),
            )
            .next()
            .await;
        assert!(matches!(missing, Some(Err(Error::NotFound))), "{missing:?}");
    });
}

#[test]
fn stat_create_rename_remove_round_trip() {
    let (provider, fake) = provider();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    runtime::block_on(async {
        let sub = dir.path().join("made");
        let created = provider
            .create_dir(&loc(&sub))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(created.kind, EntryKind::Dir);
        assert_eq!(created.name, "made");

        let file = sub.join("f.txt");
        {
            let mut writer = provider
                .open_write(&loc(&file), WriteOptions::default())
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            writer
                .write_all(b"content")
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            writer.shutdown().await.unwrap_or_else(|e| panic!("{e}"));
        }
        let refused = provider
            .open_write(&loc(&file), WriteOptions::default())
            .await;
        assert!(
            matches!(refused, Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists)
        );

        let mut text = String::new();
        provider
            .open_read(&loc(&file))
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .read_to_string(&mut text)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(text, "content");

        let before = provider
            .stat(&loc(&file))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let renamed_path = sub.join("g.txt");
        let after = provider
            .rename(&loc(&file), &loc(&renamed_path))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(after.id, before.id, "a rename keeps the identity");
        assert_eq!(after.name, "g.txt");
        assert!(matches!(
            provider.stat(&loc(&file)).await,
            Err(Error::NotFound)
        ));

        provider
            .remove(&loc(&renamed_path), RemoveMode::Trash)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!renamed_path.exists());
        assert_eq!(fake.list().map(|l| l.len()).ok(), Some(1));

        provider
            .remove(&loc(&sub), RemoveMode::Permanent)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!sub.exists());
    });
}

#[test]
fn links_and_copies() {
    let (provider, _) = provider();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let target = dir.path().join("target.bin");
    fs::write(&target, vec![9u8; 200_000]).unwrap_or_else(|e| panic!("{e}"));
    runtime::block_on(async {
        let hard = dir.path().join("hard.bin");
        let entry = provider
            .link(&loc(&target), &loc(&hard), LinkKind::Hard)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(entry.kind, EntryKind::File);

        let soft = dir.path().join("soft.bin");
        match provider
            .link(&loc(&target), &loc(&soft), LinkKind::Symbolic)
            .await
        {
            Ok(entry) => assert!(
                matches!(entry.kind, EntryKind::Symlink { broken: false, .. }),
                "{entry:?}"
            ),
            // Windows without developer mode refuses symbolic links.
            Err(Error::PermissionDenied) => {}
            Err(e) => panic!("{e}"),
        }

        let copy = dir.path().join("copy.bin");
        let (sink, rx) = ProgressSink::channel();
        provider
            .copy(
                &loc(&target),
                &loc(&copy),
                CopyOptions::default(),
                sink,
                CancelToken::new(),
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fs::read(&copy).map(|c| c.len()).ok(), Some(200_000));
        assert_eq!(
            *rx.borrow(),
            Progress::Bytes {
                done: 200_000,
                total: Some(200_000)
            }
        );

        let cancel = CancelToken::new();
        cancel.cancel();
        let cancelled = provider
            .copy(
                &loc(&target),
                &loc(&dir.path().join("never.bin")),
                CopyOptions::default(),
                ProgressSink::channel().0,
                cancel,
            )
            .await;
        assert!(matches!(cancelled, Err(Error::Cancelled)), "{cancelled:?}");
        assert!(!dir.path().join("never.bin").exists());

        let dir_copy = provider
            .copy(
                &loc(dir.path()),
                &loc(&dir.path().join("x")),
                CopyOptions::default(),
                ProgressSink::channel().0,
                CancelToken::new(),
            )
            .await;
        assert!(
            matches!(dir_copy, Err(Error::Unsupported(_))),
            "{dir_copy:?}"
        );
    });
}

#[test]
fn foreign_locations_are_refused() {
    let (provider, _) = provider();
    let foreign = Location::new(types::ProviderId("sftp"), "/x");
    runtime::block_on(async {
        assert!(matches!(
            provider.stat(&foreign).await,
            Err(Error::Unsupported(_))
        ));
        let mut listing = provider.list(&foreign, ListOptions::default(), CancelToken::new());
        assert!(matches!(
            listing.next().await,
            Some(Err(Error::Unsupported(_)))
        ));
        assert!(matches!(
            provider.watch(&foreign),
            Err(Error::Unsupported(_))
        ));
    });
}

/// Waits for a change matching `accept`, skipping unrelated ones.
async fn expect_change(
    stream: &mut ext::ChangeStream,
    what: &str,
    accept: impl Fn(&FsChange) -> bool,
) -> FsChange {
    let deadline = Duration::from_secs(10);
    let found = tokio::time::timeout(deadline, async {
        while let Some(change) = stream.next().await {
            if accept(&change) {
                return Some(change);
            }
            tracing::debug!(?change, "skipping change while waiting for {what}");
        }
        None
    })
    .await;
    match found {
        Ok(Some(change)) => change,
        Ok(None) => panic!("watch ended while waiting for {what}"),
        Err(elapsed) => panic!("no {what} within {deadline:?}: {elapsed}"),
    }
}

#[test]
fn watching_reports_create_modify_rename_and_remove_with_stable_ids() {
    let (provider, _) = provider();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let existing = dir.path().join("existing.txt");
    fs::write(&existing, b"old").unwrap_or_else(|e| panic!("{e}"));

    runtime::block_on(async {
        let mut changes = provider
            .watch(&loc(dir.path()))
            .unwrap_or_else(|e| panic!("{e}"));
        // Give the watcher a moment to arm on platforms that register lazily.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let fresh = dir.path().join("fresh.txt");
        fs::write(&fresh, b"new").unwrap_or_else(|e| panic!("{e}"));
        let created = expect_change(
            &mut changes,
            "Created(fresh.txt)",
            |c| matches!(c, FsChange::Created(e) if e.name == "fresh.txt"),
        )
        .await;
        let FsChange::Created(fresh_entry) = created else {
            unreachable!()
        };

        fs::write(&existing, b"changed").unwrap_or_else(|e| panic!("{e}"));
        let existing_id = platform::fs::stat(&existing).map_or_else(|e| panic!("{e}"), |r| r.id);
        expect_change(&mut changes, "Modified(existing.txt)", |c| {
            matches!(c, FsChange::Modified(e) if e.name == "existing.txt" && e.id.key == existing_id)
        })
        .await;

        let renamed = dir.path().join("renamed.txt");
        fs::rename(&fresh, &renamed).unwrap_or_else(|e| panic!("{e}"));
        expect_change(&mut changes, "Renamed(fresh.txt → renamed.txt)", |c| {
            matches!(c, FsChange::Renamed { from, to } if *from == fresh_entry.id && to.name == "renamed.txt")
                || matches!(c, FsChange::Created(e) if e.name == "renamed.txt")
        })
        .await;

        fs::remove_file(&renamed).unwrap_or_else(|e| panic!("{e}"));
        expect_change(&mut changes, "Removed(renamed.txt)", |c| {
            matches!(c, FsChange::Removed(id) if *id == fresh_entry.id)
                || matches!(c, FsChange::Rescan)
        })
        .await;
    });
}
