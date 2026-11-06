//! Acceptance: trash and restore round-trip on the platform the tests run
//! on, through the native platform layer and through the local provider.

use std::fs;

use ext::{Provider, RemoveMode};
use platform::{Native, Trash};
use tests::local;
use types::Error;
use vfs_local::LocalProvider;

fn unique_name(stem: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("rick-{stem}-{}-{nanos}.txt", std::process::id())
}

#[test]
fn trash_and_restore_round_trip_with_the_native_platform() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let file = dir.path().join(unique_name("roundtrip"));
    fs::write(&file, b"keep me").unwrap_or_else(|e| panic!("{e}"));
    let native = Native::new();

    let item = native.trash(&file).unwrap_or_else(|e| {
        // Show what the platform's trash holds, so a mismatch can be read
        // from the CI log.
        println!("trash contents after the failure: {:#?}", native.list());
        panic!("trash: {e}")
    });
    assert!(!file.exists(), "the file is gone from its folder");
    assert_eq!(
        item.original,
        std::path::absolute(&file).unwrap_or_else(|e| panic!("{e}"))
    );
    assert!(
        item.trashed.exists(),
        "the trash holds it at {}",
        item.trashed.display()
    );

    native
        .restore(&item)
        .unwrap_or_else(|e| panic!("restore: {e}"));
    assert_eq!(fs::read(&file).ok(), Some(b"keep me".to_vec()));
    assert!(matches!(native.restore(&item), Err(Error::NotFound)));
}

#[test]
fn the_local_provider_trashes_through_the_platform() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let file = dir.path().join(unique_name("provider"));
    fs::write(&file, b"bye").unwrap_or_else(|e| panic!("{e}"));
    let provider = LocalProvider::native();

    runtime::block_on(provider.remove(&local(&file), RemoveMode::Trash))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!file.exists());

    // Put it back so nothing accumulates in the developer's trash. macOS
    // cannot list its trash, so the round-trip test above is the one that
    // restores there.
    if let Ok(items) = Native::new().list() {
        let name = file.file_name().map(std::ffi::OsStr::to_os_string);
        if let Some(item) = items
            .into_iter()
            .find(|i| i.original.file_name().map(std::ffi::OsStr::to_os_string) == name)
        {
            Native::new()
                .restore(&item)
                .unwrap_or_else(|e| panic!("restore: {e}"));
            assert_eq!(fs::read(&file).ok(), Some(b"bye".to_vec()));
        }
    }
}
