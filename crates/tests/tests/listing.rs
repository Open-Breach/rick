//! Acceptance: listing the 200k fixture is fast, and cancelling a listing
//! ends it within one batch.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ext::{ListOptions, Provider};
use futures::StreamExt;
use tests::fixture::{Config, SizeDistribution};
use tests::local;
use types::CancelToken;
use vfs_local::{BATCH, LocalProvider};

fn provider() -> LocalProvider {
    LocalProvider::new(Arc::new(
        platform::Fake::new().unwrap_or_else(|e| panic!("{e}")),
    ))
}

/// A flat directory of `entries` empty files: the layout of the performance
/// target.
fn flat(entries: usize) -> tests::fixture::Fixture {
    Config {
        entries,
        max_depth: 0,
        sizes: SizeDistribution::Empty,
        ..Config::new(entries, 11)
    }
    .generate()
    .unwrap_or_else(|e| panic!("{e}"))
}

/// The Decisions-table numbers hold on Linux CI; macOS and Windows record
/// their timings until targets are set from measurements (issue 2).
#[test]
#[ignore = "lists 200 000 entries; run with --ignored in release"]
fn listing_200k_entries_first_batch_under_50ms_complete_under_400ms() {
    let fixture = flat(200_000);
    let provider = provider();
    // Warm the runtime and the compute pool so the measurement is the listing.
    runtime::block_on(async {
        let warm = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let mut stream = provider.list(
            &local(warm.path()),
            ListOptions::default(),
            CancelToken::new(),
        );
        while stream.next().await.is_some() {}
    });

    let (first_batch, complete, count) = runtime::block_on(async {
        let started = Instant::now();
        let mut stream = provider.list(
            &local(fixture.root()),
            ListOptions::default(),
            CancelToken::new(),
        );
        let first = stream
            .next()
            .await
            .unwrap_or_else(|| panic!("empty listing"))
            .unwrap_or_else(|e| panic!("{e}"));
        let first_batch = started.elapsed();
        let mut count = first.len();
        while let Some(batch) = stream.next().await {
            count += batch.unwrap_or_else(|e| panic!("{e}")).len();
        }
        (first_batch, started.elapsed(), count)
    });

    println!(
        "listing 200 000 entries on {}: first batch {first_batch:?}, complete {complete:?}",
        std::env::consts::OS
    );
    assert_eq!(count, 200_000);
    if std::env::consts::OS == "linux" {
        assert!(
            first_batch < Duration::from_millis(50),
            "first batch took {first_batch:?}"
        );
        assert!(
            complete < Duration::from_millis(400),
            "complete listing took {complete:?}"
        );
    }
}

#[test]
fn cancelling_a_listing_ends_it_within_one_batch() {
    let fixture = flat(10 * BATCH);
    let provider = provider();
    let cancel = CancelToken::new();
    let batches_after_cancel = runtime::block_on(async {
        let mut stream = provider.list(
            &local(fixture.root()),
            ListOptions::default(),
            cancel.clone(),
        );
        let first = stream
            .next()
            .await
            .unwrap_or_else(|| panic!("empty listing"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(first.len(), BATCH);
        cancel.cancel();
        let mut after = 0usize;
        while let Some(batch) = stream.next().await {
            batch.unwrap_or_else(|e| panic!("{e}"));
            after += 1;
        }
        after
    });
    assert!(
        batches_after_cancel <= 1,
        "{batches_after_cancel} batches arrived after cancelling"
    );
}
