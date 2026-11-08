//! Issue 51, the real thing on Linux: an ISO mounted through
//! `platform::Mount`, that is `udisksctl loop-setup` and `udisksctl mount`,
//! appears among the mounted volumes with its files readable, and Eject
//! unmounts it and deletes the loop device. The test runs where udisks
//! answers (CI installs udisks2 with a polkit rule for the runner and sets
//! `RICK_REQUIRE_UDISKS`); elsewhere it skips with a note, and the fake
//! platform's test in `side_panel.rs` covers the action.

#![cfg(target_os = "linux")]

use std::fs;
use std::process::{Command, Stdio};

use platform::{Mount as _, Native, Volumes as _};

/// Whether udisks is installed and its daemon answers.
fn udisks_answers() -> bool {
    Command::new("udisksctl")
        .arg("status")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A small ISO 9660 image with a file at the top and one in a folder.
fn iso(path: &std::path::Path) {
    use hadris_fs::{Content, Node, Tree};
    let mut tree = Tree::new();
    tree.insert("docs", Node::dir())
        .unwrap_or_else(|e| panic!("{e}"));
    tree.insert(
        "docs/notes.txt",
        Node::file(Content::bytes("hello from the disc\n")),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    tree.insert("README", Node::file(Content::bytes("top")))
        .unwrap_or_else(|e| panic!("{e}"));
    let mut image: Vec<u8> = Vec::new();
    hadris_iso::sync::write(
        &mut image,
        &tree,
        &hadris_iso::IsoOptions::new()
            .with_rock_ridge()
            .with_joliet(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fs::write(path, image).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn an_iso_mounted_through_udisks_is_a_volume_until_eject() {
    if !udisks_answers() {
        eprintln!("udisks does not answer here; the real mount test is skipped");
        assert!(
            std::env::var_os("RICK_REQUIRE_UDISKS").is_none(),
            "udisks is required here but does not answer"
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("rick-mount-test.iso");
    iso(&path);
    let native = Native::new();
    assert!(native.mountable().contains(&platform::ImageFormat::Iso));
    let volume = native
        .mount_image(&path)
        .unwrap_or_else(|e| panic!("mounting {}: {e}", path.display()));
    println!(
        "mounted {} at {} ({})",
        path.display(),
        volume.mount_point.display(),
        volume.fs_type
    );
    assert!(volume.is_removable(), "{volume:?}: Eject applies to it");
    let volumes = native.volumes().unwrap_or_else(|e| panic!("{e}"));
    assert!(
        volumes.iter().any(|v| v.mount_point == volume.mount_point),
        "the volume list has the mount: {volumes:?}"
    );
    let readme = fs::read_to_string(volume.mount_point.join("README"))
        .unwrap_or_else(|e| panic!("reading the mounted README: {e}"));
    assert_eq!(readme, "top");
    native
        .eject(&volume)
        .unwrap_or_else(|e| panic!("ejecting: {e}"));
    let volumes = native.volumes().unwrap_or_else(|e| panic!("{e}"));
    assert!(
        !volumes.iter().any(|v| v.mount_point == volume.mount_point),
        "the volume is gone after Eject: {volumes:?}"
    );
    let loop_devices = fs::read_dir("/sys/block").map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("loop"))
            .filter(|e| {
                fs::read_to_string(e.path().join("loop/backing_file"))
                    .is_ok_and(|f| f.trim() == path.display().to_string())
            })
            .count()
    });
    assert_eq!(loop_devices, 0, "the loop device was deleted");
}
