//! The second launch, end to end (specs/system-integration.md §3): the
//! built binary, run with a config directory of the test's own, hands its
//! command line to the instance the test holds on that directory's
//! endpoint, prints the reply and exits with the instance's status within
//! the second; an instance that accepts and never answers is reported as
//! unresponsive, and the launch exits non-zero without opening a window.

use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use platform::SingleInstance as _;
use platform::instance::{EXIT_FAILED, PATIENCE, Reply};

/// The binary this build produced.
fn rick() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_rick"))
}

/// Runs `rick --config-dir <config> <args…>` from `cwd`, on the blocking
/// pool so the instance in the test can answer meanwhile; with how long
/// the launch took.
async fn launch(config: &Path, cwd: &Path, args: &[&str]) -> (Output, Duration) {
    let mut command = Command::new(rick());
    command
        .arg("--config-dir")
        .arg(config)
        .args(args)
        .current_dir(cwd)
        .env("RUST_LOG", "warn");
    let (out, took) = tokio::task::spawn_blocking(move || {
        let started = Instant::now();
        let out = command.output();
        (out, started.elapsed())
    })
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    (out.unwrap_or_else(|e| panic!("{e}")), took)
}

#[test]
fn a_second_launch_hands_its_arguments_over_and_exits_with_the_instances_status() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
    let endpoint = platform::Native::new().instance_endpoint(&config);
    runtime::block_on(async {
        let mut listener = endpoint.bind().await.unwrap_or_else(|e| panic!("{e}"));
        let instance = async {
            let mut conn = listener.accept().await.unwrap_or_else(|e| panic!("{e}"));
            let request = conn
                .request(PATIENCE)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            // The launch's own `--config-dir` stays with the launch; the
            // rest arrives as typed, with the working directory it was
            // typed in.
            assert_eq!(request.argv, ["--new-window", "sub"]);
            assert_eq!(
                request.cwd.canonicalize().ok(),
                dir.path().canonicalize().ok(),
                "{request:?}"
            );
            conn.reply(&Reply::failed(7, "seven from the instance"))
                .await
                .unwrap_or_else(|e| panic!("{e}"));
        };
        let ((out, took), ()) = tokio::join!(
            launch(&config, dir.path(), &["--new-window", "sub"]),
            instance
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(7), "{stderr}");
        assert!(stderr.contains("seven from the instance"), "{stderr}");
        assert!(out.stdout.is_empty(), "a failure prints on standard error");
        // Well within the second, plus what starting a process costs.
        assert!(took < Duration::from_secs(5), "{took:?}");
    });
}

#[test]
fn an_instance_that_does_not_answer_is_reported_and_the_launch_exits_non_zero() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
    let endpoint = platform::Native::new().instance_endpoint(&config);
    runtime::block_on(async {
        let mut listener = endpoint.bind().await.unwrap_or_else(|e| panic!("{e}"));
        let instance = async {
            // Accepts, and holds the connection without a word.
            let conn = listener.accept().await.unwrap_or_else(|e| panic!("{e}"));
            tokio::time::sleep(PATIENCE * 3).await;
            drop(conn);
        };
        let ((out, took), ()) = tokio::join!(launch(&config, dir.path(), &[]), instance);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(EXIT_FAILED), "{stderr}");
        assert!(stderr.contains("did not answer"), "{stderr}");
        assert!(
            took >= PATIENCE && took < PATIENCE * 3,
            "the launch waits its second and gives up before the instance lets go: {took:?}"
        );
    });
}

#[test]
fn the_launch_answers_version_and_help_without_an_instance() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let config = dir.path().join("config");
    runtime::block_on(async {
        let (out, _) = launch(&config, dir.path(), &["--version"]).await;
        assert!(out.status.success());
        let stdout = String::from_utf8_lossy(&out.stdout);
        // The About dialog's details (specs/about-dialog.md): the name and
        // version first, the build line second.
        assert!(
            stdout.starts_with(&format!("Rick {}\n", env!("CARGO_PKG_VERSION"))),
            "{stdout}"
        );
        assert!(stdout.contains("plugin ABI "), "{stdout}");
        let (out, _) = launch(&config, dir.path(), &["--help"]).await;
        assert!(out.status.success());
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("--new-window") && stdout.contains("--list-plugins"),
            "{stdout}"
        );
    });
}
