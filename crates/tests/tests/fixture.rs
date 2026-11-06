//! Acceptance: the fixture is deterministic and fast enough.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use tests::fixture::{self, Config, SizeDistribution};

/// Relative path → (`is_dir`, size, content hash).
type Snapshot = BTreeMap<String, (bool, u64, u64)>;

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3)
    })
}

fn snapshot(root: &Path) -> Snapshot {
    fn walk(root: &Path, dir: &Path, out: &mut Snapshot) {
        let entries =
            fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|e| panic!("entry in {}: {e}", dir.display()));
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .unwrap_or_else(|e| panic!("{} outside {}: {e}", path.display(), root.display()))
                .to_string_lossy()
                .replace('\\', "/");
            let meta = entry
                .metadata()
                .unwrap_or_else(|e| panic!("metadata {}: {e}", path.display()));
            if meta.is_dir() {
                out.insert(relative, (true, 0, 0));
                walk(root, &path, out);
            } else {
                let content =
                    fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                out.insert(relative, (false, meta.len(), fnv1a(&content)));
            }
        }
    }
    let mut out = Snapshot::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn same_seed_same_tree_different_seed_different_tree() {
    let config = Config {
        entries: 2_000,
        max_depth: 3,
        entries_per_dir: 32,
        sizes: SizeDistribution::LogUniform { min: 0, max: 2048 },
        seed: 7,
    };
    let a = config.generate().unwrap_or_else(|e| panic!("{e}"));
    let b = config.generate().unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(a.root(), b.root());
    assert_eq!(a.entries(), 2_000);
    assert_eq!(a.dirs(), 2_000 / 32);
    assert_eq!(a.files(), 2_000 - 2_000 / 32);

    let snap_a = snapshot(a.root());
    let snap_b = snapshot(b.root());
    assert_eq!(snap_a.len(), 2_000);
    assert_eq!(snap_a, snap_b);
    assert_eq!(
        snap_a
            .values()
            .filter(|(_, _, _)| true)
            .map(|(_, size, _)| size)
            .sum::<u64>(),
        a.bytes()
    );

    let other = Config { seed: 8, ..config }
        .generate()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(snapshot(other.root()), snap_a);
}

#[test]
fn the_fixture_is_removed_when_dropped() {
    let fixture = fixture::generate(50, 1).unwrap_or_else(|e| panic!("{e}"));
    let root = fixture.root().to_path_buf();
    assert!(root.is_dir());
    drop(fixture);
    assert!(!root.exists());
}

#[test]
fn flat_layout_has_no_directories() {
    let fixture = Config {
        entries: 300,
        max_depth: 0,
        sizes: SizeDistribution::Empty,
        ..Config::new(300, 5)
    }
    .generate()
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(fixture.dirs(), 0);
    assert_eq!(fixture.files(), 300);
    assert_eq!(fixture.bytes(), 0);
    assert_eq!(
        fs::read_dir(fixture.root()).map(Iterator::count).ok(),
        Some(300)
    );
}

/// The full-size acceptance test. Writes 200 000 entries, so it is skipped in
/// the default run and executed in CI with `cargo test -- --ignored`.
#[test]
#[ignore = "writes 200 000 entries; run with --ignored"]
fn two_hundred_thousand_entries_in_under_thirty_seconds() {
    let started = Instant::now();
    let fixture =
        fixture::generate(fixture::DEFAULT_ENTRIES, 2026).unwrap_or_else(|e| panic!("{e}"));
    let elapsed = started.elapsed();
    assert_eq!(fixture.entries(), 200_000);
    assert!(
        elapsed < Duration::from_secs(30),
        "generating 200 000 entries took {elapsed:?}"
    );
}
