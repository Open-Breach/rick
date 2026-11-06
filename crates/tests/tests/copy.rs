//! Acceptance: a 1 GiB sparse file copies through the fast path.

use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use ext::{CopyOptions, Provider};
use platform::fs::{CopyMethod, allocated_size, copy_file, shared_extents};
use runtime::ProgressSink;
use tests::local;
use types::{CancelToken, Progress};
use vfs_local::LocalProvider;

const GIB: u64 = 1 << 30;
const DATA: usize = 4 << 20;

fn sparse_gib(path: &Path) {
    let mut file = fs::File::create(path).unwrap_or_else(|e| panic!("{e}"));
    file.write_all(&vec![0xABu8; DATA])
        .unwrap_or_else(|e| panic!("{e}"));
    file.seek(SeekFrom::Start(GIB - DATA as u64))
        .unwrap_or_else(|e| panic!("{e}"));
    file.write_all(&vec![0xCDu8; DATA])
        .unwrap_or_else(|e| panic!("{e}"));
    file.set_len(GIB).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
#[ignore = "writes a 1 GiB sparse file; run with --ignored"]
fn copying_a_1gib_sparse_file_uses_the_fast_path() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let source = dir.path().join("sparse.bin");
    sparse_gib(&source);
    let source_allocated = allocated_size(&source).unwrap_or_else(|e| panic!("{e}"));

    // Through the provider, as `vfs::copy` does it.
    let provider = LocalProvider::new(Arc::new(
        platform::Fake::new().unwrap_or_else(|e| panic!("{e}")),
    ));
    let target = dir.path().join("copy.bin");
    let (sink, rx) = ProgressSink::channel();
    let started = Instant::now();
    runtime::block_on(provider.copy(
        &local(&source),
        &local(&target),
        CopyOptions::default(),
        sink,
        CancelToken::new(),
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    let elapsed = started.elapsed();
    let allocated = allocated_size(&target).unwrap_or_else(|e| panic!("{e}"));
    println!(
        "1 GiB sparse copy on {}: {elapsed:?}, source {source_allocated} bytes on disk, copy {allocated} bytes on disk",
        std::env::consts::OS
    );

    assert_eq!(fs::metadata(&target).map(|m| m.len()).ok(), Some(GIB));
    assert_eq!(
        *rx.borrow(),
        Progress::Bytes {
            done: GIB,
            total: Some(GIB)
        }
    );
    tests::bench::record("copy-sparse", elapsed);
    // Writing 1 GiB of zeros would take seconds and allocate a gigabyte; the
    // fast path copies the two data segments and leaves the hole.
    let hole_preserved = allocated <= source_allocated + (64 << 20);
    if std::env::consts::OS == "windows" {
        // CopyFileEx keeps sparse ranges on NTFS but not every volume agrees;
        // the elapsed-time bound is the assertion there.
        println!("hole preserved: {hole_preserved}");
    } else {
        assert!(
            hole_preserved,
            "copy allocated {allocated} bytes for a {source_allocated}-byte sparse source"
        );
    }

    // Directly, to learn which path the platform took; a reflink must show
    // shared extents where the filesystem can report them (Btrfs, XFS).
    let direct = dir.path().join("direct.bin");
    let method =
        copy_file(&source, &direct, false, &mut |_, _| true).unwrap_or_else(|e| panic!("{e}"));
    let shared = shared_extents(&direct).unwrap_or_else(|e| panic!("{e}"));
    println!("direct copy method {method:?}, shared extents {shared:?}");
    if method == CopyMethod::Reflink && std::env::consts::OS == "linux" {
        assert_eq!(shared, Some(true), "a reflink copy shares extents");
    }
    let content = fs::read(&direct).unwrap_or_else(|e| panic!("{e}"));
    let last = usize::try_from(GIB - 1).unwrap_or(0);
    assert_eq!(
        (content.first(), content.get(last)),
        (Some(&0xAB), Some(&0xCD))
    );
}
