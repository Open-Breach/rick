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

/// The relaunch from the installed copy (specs/system-integration.md
/// §6.1, by the single instance's rules of §3): the old process starts
/// the copy and is done only once the new instance answers on the
/// endpoint. Here the copy is the built binary asked for its version,
/// which exits at once, and the test itself plays the new instance: it
/// binds the endpoint a moment later and answers the one request.
#[test]
fn the_relaunch_returns_once_the_new_instance_answers() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
    let endpoint = platform::Native::new().instance_endpoint(&config);
    let args: Vec<std::ffi::OsString> = vec![
        "--config-dir".into(),
        config.clone().into(),
        "--version".into(),
    ];
    runtime::block_on(async {
        let late = Duration::from_millis(400);
        let new_instance = async {
            tokio::time::sleep(late).await;
            let mut listener = endpoint.bind().await.unwrap_or_else(|e| panic!("{e}"));
            let mut conn = listener.accept().await.unwrap_or_else(|e| panic!("{e}"));
            let request = conn
                .request(PATIENCE)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            conn.reply(&Reply::ok(Some("help".to_owned())))
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            request
        };
        let started = Instant::now();
        let (outcome, request) = tokio::join!(
            rick::instance::relaunch(&endpoint, rick(), &args, Duration::from_secs(10)),
            new_instance
        );
        outcome.unwrap_or_else(|e| panic!("{e}"));
        assert!(
            started.elapsed() >= late,
            "not before the new instance answered: {:?}",
            started.elapsed()
        );
        // What is asked of the new instance changes nothing in its window.
        assert_eq!(request.argv, ["--help"]);
    });
    endpoint.unbind();
}

/// A copy that never becomes the instance: the relaunch gives up after
/// its patience and says so, for the old process to stay.
#[test]
fn the_relaunch_gives_up_when_nothing_answers() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
    let endpoint = platform::Native::new().instance_endpoint(&config);
    let args: Vec<std::ffi::OsString> = vec![
        "--config-dir".into(),
        config.clone().into(),
        "--version".into(),
    ];
    runtime::block_on(async {
        let patience = Duration::from_millis(600);
        let started = Instant::now();
        let outcome = rick::instance::relaunch(&endpoint, rick(), &args, patience).await;
        let error = outcome.err().unwrap_or_else(|| panic!("nothing answered"));
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut, "{error}");
        assert!(started.elapsed() >= patience);
        // A binary that is not there is the caller's error at once.
        let missing =
            rick::instance::relaunch(&endpoint, &dir.path().join("no-such-rick"), &args, patience)
                .await;
        assert_eq!(
            missing.err().map(|e| e.kind()),
            Some(std::io::ErrorKind::NotFound)
        );
    });
}

/// The instance's half against a second process (issue 103, item 8): a
/// headless app on the fake platform serves the endpoint of a config
/// directory of its own; the built binary, run as the newcomer, sends
/// `reveal` and `--new-window`, and the app routes them — the file's
/// folder as a new tab of the focused window with the file selected once
/// listed, the path in a window of its own — and asks for the window to
/// be raised.
mod routed {
    use std::path::Path;
    use std::process::Command;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use app::{App, Config, Event};
    use platform::{ConfigDir as _, Fake, SingleInstance as _};
    use types::Location;

    fn location(path: &Path) -> Location {
        Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path))
    }

    /// A folder with three files and a home, the app on the fake platform
    /// started at the home, and the endpoint of the test's config
    /// directory served by the app's external-work channel.
    struct Instance {
        dir: tempfile::TempDir,
        fake: Arc<Fake>,
        app: App,
        receiver: app::ExternalReceiver,
        events: Vec<Event>,
    }

    impl Instance {
        async fn start() -> Self {
            let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
            let config = dir.path().join("config");
            let home = dir.path().join("home");
            let folder = dir.path().join("folder");
            std::fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
            std::fs::create_dir_all(&home).unwrap_or_else(|e| panic!("{e}"));
            std::fs::create_dir_all(&folder).unwrap_or_else(|e| panic!("{e}"));
            for name in ["a.txt", "b.txt", "c.txt"] {
                std::fs::write(folder.join(name), name).unwrap_or_else(|e| panic!("{e}"));
            }
            let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
            let app = App::new(Config {
                platform: fake.clone(),
                ui: Arc::new(ui_egui::EguiHost::new()),
                extensions: Vec::new(),
                config_dir: Some(fake.config_dir()),
                start: Some(location(&home)),
            });
            let endpoint = platform::Native::new().instance_endpoint(&config);
            let listener = endpoint.bind().await.unwrap_or_else(|e| panic!("{e}"));
            let (externals, receiver) = app::external_channel();
            // Served for the test's lifetime; the runtime drops it with the
            // process.
            let _serving = rick::instance::serve(listener, externals);
            Self {
                dir,
                fake,
                app,
                receiver,
                events: Vec::new(),
            }
        }

        fn config(&self) -> std::path::PathBuf {
            self.dir.path().join("config")
        }

        /// Runs `rick --config-dir <config> <args…>` from the test's
        /// directory while the app is driven, until the process ends.
        async fn launch(&mut self, args: &[&str]) -> std::process::Output {
            let mut command = Command::new(super::rick());
            command
                .arg("--config-dir")
                .arg(self.config())
                .args(args)
                .current_dir(self.dir.path())
                .env("RUST_LOG", "warn")
                .env_remove("XDG_ACTIVATION_TOKEN")
                .env_remove("DESKTOP_STARTUP_ID");
            let launched = tokio::task::spawn_blocking(move || command.output());
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                self.drive().await;
                if launched.is_finished() {
                    break;
                }
                assert!(Instant::now() < deadline, "the launch did not end");
            }
            launched
                .await
                .unwrap_or_else(|e| panic!("{e}"))
                .unwrap_or_else(|e| panic!("{e}"))
        }

        /// One round of the application's loop: the deliveries from the
        /// endpoint, then what the listings sent.
        async fn drive(&mut self) {
            self.receiver.drain(&mut self.app, &mut self.events);
            let _ =
                tokio::time::timeout(Duration::from_millis(20), self.app.tick(&mut self.events))
                    .await;
        }

        /// Drives the app until `check` holds.
        async fn wait_for(&mut self, what: &str, check: impl Fn(&App) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !check(&self.app) {
                assert!(
                    Instant::now() < deadline,
                    "{what} did not happen: {:?}",
                    self.app
                );
                self.drive().await;
            }
        }
    }

    #[test]
    fn reveal_from_a_second_process_opens_the_folder_and_selects_the_file() {
        runtime::block_on(async {
            let mut instance = Instance::start().await;
            let windows = instance.app.model().windows().count();
            let out = instance.launch(&["reveal", "folder/b.txt"]).await;
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let folder = location(&instance.dir.path().join("folder"));
            // The folder is the focused pane's tab, in the same window…
            instance
                .wait_for("the folder's tab", |app| {
                    app.model()
                        .active_tab(app.model().focused)
                        .is_some_and(|(_, tab)| tab.location == folder)
                })
                .await;
            assert_eq!(instance.app.model().windows().count(), windows);
            // …and the file is selected once listed.
            instance
                .wait_for("the selection", |app| {
                    app.model()
                        .active_tab(app.model().focused)
                        .is_some_and(|(_, tab)| {
                            tab.selected().map(|e| e.name.as_str()).collect::<Vec<_>>() == ["b.txt"]
                        })
                })
                .await;
            // The window was asked to come forward, without a token (the
            // launch had none in its environment).
            assert!(
                instance.events.iter().any(|e| matches!(
                    e,
                    Event::Activate { window, token: None } if *window == instance.app.model().focused_window
                )),
                "{:?}",
                instance.events
            );
            assert!(
                instance.fake.activations().is_empty(),
                "the UI raises, not the app"
            );
        });
    }

    #[test]
    fn new_window_from_a_second_process_opens_a_window_of_its_own() {
        runtime::block_on(async {
            let mut instance = Instance::start().await;
            let windows = instance.app.model().windows().count();
            let out = instance.launch(&["--new-window", "folder"]).await;
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let folder = location(&instance.dir.path().join("folder"));
            instance
                .wait_for("the new window", |app| {
                    app.model().windows().count() == windows + 1
                        && app
                            .model()
                            .active_tab(app.model().focused)
                            .is_some_and(|(_, tab)| tab.location == folder)
                })
                .await;
            let new_window = instance.app.model().focused_window;
            assert_eq!(
                instance.app.model().workspaces(new_window).len(),
                1,
                "one tab in the new window"
            );
        });
    }
}
