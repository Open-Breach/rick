//! `rick-perms`, the privileged helper, end to end: the protocol round
//! trip through a fake platform that runs the helper as the current user,
//! and, where Linux CI set `RICK_ELEVATE_FIXTURE` and installed the polkit
//! stand-in, a change to a root-owned fixture that the unprivileged pass
//! refuses and the helper, run as root through `pkexec`, makes.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use platform::perms::{self, helper};
use platform::{Fake, PermissionChange, Permissions as _, Platform, Principal};

/// The helper this build produced.
fn built_helper() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_rick-perms"))
}

#[test]
fn the_helper_applies_a_request_and_reports_what_the_entries_had() {
    let fake = Fake::new().unwrap_or_else(|e| panic!("{e}"));
    fake.set_elevate_runs(true);
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let file = dir.path().join("f");
    std::fs::write(&file, b"x").unwrap_or_else(|e| panic!("{e}"));
    let before = fake.permissions(&file).unwrap_or_else(|e| panic!("{e}"));
    let items = vec![
        helper::Item {
            path: file.clone(),
            change: PermissionChange {
                mode: before.mode.map(|m| (m & 0o7000) | 0o600),
                ..PermissionChange::default()
            },
        },
        helper::Item {
            path: dir.path().join("missing"),
            change: PermissionChange {
                mode: Some(0o600),
                ..PermissionChange::default()
            },
        },
    ];
    let reply =
        perms::apply_elevated_with(&fake, built_helper(), &items).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(reply.done.len(), 1, "{reply:?}");
    assert_eq!(reply.done[0].path, file);
    assert_eq!(reply.done[0].previous, before);
    assert_eq!(reply.failed.len(), 1);
    assert_eq!(reply.failed[0].error, "not found");
    let after = fake.permissions(&file).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(after.mode.map(|m| m & 0o777), before.mode.map(|_| 0o600));
    // The helper was the program handed to the platform's prompt.
    let elevated = fake.elevated();
    assert_eq!(elevated.len(), 1);
    assert_eq!(elevated[0].0, built_helper());
    assert_eq!(elevated[0].1[0], "apply");
}

#[test]
fn a_helper_that_is_not_installed_is_reported_not_run() {
    let fake = Fake::new().unwrap_or_else(|e| panic!("{e}"));
    let result = perms::apply_elevated_with(&fake, Path::new("/nonexistent/rick-perms"), &[]);
    assert!(
        matches!(result, Err(types::Error::Unsupported(_))),
        "{result:?}"
    );
    assert!(fake.elevated().is_empty(), "nothing ran");
}

#[test]
fn the_helper_is_found_beside_the_binary_or_one_directory_up() {
    // This test binary lives in target/<profile>/deps; the helper one up.
    assert_eq!(perms::helper_path(), built_helper());
}

/// Runs `sudo` for the fixture; `None` when the runner has no passwordless
/// sudo.
fn sudo(args: &[&str]) -> Option<()> {
    Command::new("sudo")
        .arg("-n")
        .args(args)
        .status()
        .ok()
        .filter(std::process::ExitStatus::success)
        .map(|_| ())
}

fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::symlink_metadata(path)
        .unwrap_or_else(|e| panic!("{e}"))
        .permissions()
        .mode()
        & 0o7777
}

/// Linux CI only (`RICK_ELEVATE_FIXTURE`): a file root owns and nobody
/// else may write is changed through the job, which turns to the helper
/// under the polkit stand-in, and undone the same way.
#[cfg(unix)]
#[test]
fn a_change_to_a_root_owned_fixture_goes_through_the_helper() {
    if std::env::var_os("RICK_ELEVATE_FIXTURE").is_none() {
        eprintln!("RICK_ELEVATE_FIXTURE is not set; the elevated change is not checked here");
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let file = dir.path().join("root-owned");
    std::fs::write(&file, b"x").unwrap_or_else(|e| panic!("{e}"));
    let path = file.to_string_lossy().into_owned();
    assert!(
        sudo(&["chown", "root:root", &path]).is_some() && sudo(&["chmod", "0644", &path]).is_some(),
        "the fixture needs passwordless sudo"
    );
    assert_eq!(mode_of(&file), 0o644);

    let platform: Arc<dyn Platform> = Arc::new(platform::Native::new());
    let jobs = runtime::JobRunner::new();
    let request = ops::PermissionRequest {
        targets: vec![file.clone()],
        change: PermissionChange {
            mode: Some(0o600),
            ..PermissionChange::default()
        },
        recursive: None,
    };
    let label = ops::perms::label(&request);
    let handle = ops::perms::spawn(&jobs, Arc::clone(&platform), label.clone(), request.clone());
    let applied = runtime::block_on(handle.result)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        applied.elevated,
        "the unprivileged pass was refused: {applied:?}"
    );
    assert!(applied.failed.is_empty(), "{:?}", applied.failed);
    assert_eq!(applied.previous.len(), 1);
    assert_eq!(applied.previous[0].1.mode, Some(0o644));
    assert_eq!(applied.previous[0].1.owner, Principal::named("0", "root"));
    assert_eq!(mode_of(&file), 0o600);

    // Undo runs as root the same way and puts the mode back.
    let command = ops::PermissionsCommand::new(Arc::clone(&platform), label, &request, &applied);
    ext::UndoCommand::undo(&command, &jobs).unwrap_or_else(|e| panic!("{e}"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !jobs.running().is_empty() {
        assert!(std::time::Instant::now() < deadline, "the undo job hangs");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(mode_of(&file), 0o644);
    let _ = sudo(&["rm", "-f", &path]);
}
