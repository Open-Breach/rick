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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use app::{App, Config, Event};
    use ext::PaneId;
    use platform::{ConfigDir as _, Fake, SingleInstance as _};
    use types::Location;

    fn location(path: &Path) -> Location {
        Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path))
    }

    /// The location a launch opens for `path`: canonical, as `cli::resolve`
    /// makes a path that exists (`/private/var/…` for a macOS temporary
    /// directory, the `\\?\` form on Windows).
    fn opened(path: &Path) -> Location {
        location(&path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
    }

    /// A folder with three files and a home, the app on the fake platform
    /// started at the home, and the endpoint of the test's config
    /// directory served by the app's external-work channel.
    struct Instance {
        dir: tempfile::TempDir,
        fake: Arc<Fake>,
        app: App,
        /// What the session bus's calls are queued through, as a second
        /// launch's are.
        #[cfg(target_os = "linux")]
        externals: app::ExternalSender,
        receiver: app::ExternalReceiver,
        events: Vec<Event>,
        /// How often a launch handed `--dbus-service` over.
        asked_for_the_bus: Arc<AtomicUsize>,
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
            let asked_for_the_bus = Arc::new(AtomicUsize::new(0));
            let asked = Arc::clone(&asked_for_the_bus);
            // Served for the test's lifetime; the runtime drops it with the
            // process.
            let _serving = rick::instance::serve(
                listener,
                externals.clone(),
                Some(Arc::new(move || {
                    asked.fetch_add(1, Ordering::SeqCst);
                })),
            );
            Self {
                dir,
                fake,
                app,
                #[cfg(target_os = "linux")]
                externals,
                receiver,
                events: Vec::new(),
                asked_for_the_bus,
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
            self.run(command).await
        }

        /// Runs `command` to its end while the app is driven.
        async fn run(&mut self, mut command: Command) -> std::process::Output {
            let running = tokio::task::spawn_blocking(move || command.output());
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                self.drive().await;
                if running.is_finished() {
                    break;
                }
                assert!(Instant::now() < deadline, "the process did not end");
            }
            running
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
                    "{what} did not happen: {:?}, tabs {:?}",
                    self.app,
                    self.tabs()
                );
                self.drive().await;
            }
        }

        /// Each pane's active tab's location, for a failure's message.
        fn tabs(&self) -> Vec<(PaneId, Option<String>)> {
            let model = self.app.model();
            model
                .panes()
                .map(|(pane, _)| {
                    let location = model
                        .active_tab(pane)
                        .map(|(_, tab)| tab.location.to_string());
                    (pane, location)
                })
                .collect()
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
            let folder = opened(&instance.dir.path().join("folder"));
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
            let folder = opened(&instance.dir.path().join("folder"));
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

    /// A click on the launcher entry or on a dock favourite
    /// (specs/system-integration.md §4): the desktop entry runs
    /// `rick --activate`, and a running Rick answers by bringing its
    /// window forward and opening nothing, whatever directory the
    /// launcher started the process in.
    #[test]
    fn a_launcher_click_raises_the_window_and_opens_no_tab() {
        runtime::block_on(async {
            let mut instance = Instance::start().await;
            let windows = instance.app.model().windows().count();
            let panes = instance.app.model().panes().count();
            let focused = instance.app.model().focused;
            let shown = instance
                .app
                .model()
                .active_tab(focused)
                .map(|(id, tab)| (id, tab.location.clone()));
            let out = instance.launch(&["--activate"]).await;
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&out.stdout), "");
            for _ in 0..5 {
                instance.drive().await;
            }
            let model = instance.app.model();
            assert_eq!(model.windows().count(), windows);
            assert_eq!(model.panes().count(), panes, "{:?}", instance.tabs());
            assert_eq!(model.focused, focused);
            assert_eq!(
                model
                    .active_tab(focused)
                    .map(|(id, tab)| (id, tab.location.clone())),
                shown
            );
            assert!(
                instance.events.iter().any(|e| matches!(
                    e,
                    Event::Activate { window, token: None } if *window == model.focused_window
                )),
                "{:?}",
                instance.events
            );
            // The bus was not asked for: this is not its activation.
            assert_eq!(instance.asked_for_the_bus.load(Ordering::SeqCst), 0);

            // What the launcher hands over still opens: a folder dropped
            // on the icon arrives as the entry's `%U`.
            let out = instance.launch(&["--activate", "folder"]).await;
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let folder = opened(&instance.dir.path().join("folder"));
            instance
                .wait_for("the dropped folder's tab", |app| {
                    app.model()
                        .active_tab(app.model().focused)
                        .is_some_and(|(_, tab)| tab.location == folder)
                })
                .await;
        });
    }

    #[test]
    fn a_launch_for_the_bus_opens_nothing_and_tells_the_instance() {
        /// The focused pane's tab and where it is.
        fn focused(app: &App) -> Option<(model::TabId, Location)> {
            app.model()
                .active_tab(app.model().focused)
                .map(|(id, tab)| (id, tab.location.clone()))
        }
        runtime::block_on(async {
            let mut instance = Instance::start().await;
            let before = focused(&instance.app);
            let windows = instance.app.model().windows().count();
            let out = instance
                .launch(&[platform::file_manager::SERVICE_ARGUMENT])
                .await;
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&out.stdout), "");
            // The instance was told the bus is asking for it, once…
            assert_eq!(instance.asked_for_the_bus.load(Ordering::SeqCst), 1);
            // …and nothing was opened for the launch: no tab of the
            // working directory, no window, nothing raised.
            for _ in 0..5 {
                instance.drive().await;
            }
            assert_eq!(instance.app.model().windows().count(), windows);
            assert_eq!(focused(&instance.app), before);
            assert!(
                !instance
                    .events
                    .iter()
                    .any(|e| matches!(e, Event::Activate { .. })),
                "{:?}",
                instance.events
            );
        });
    }

    /// *"Show in folder" from other apps* (specs/system-integration.md
    /// §7.1, issue 120): the calls of `org.freedesktop.FileManager1`,
    /// made with `gdbus` as another application makes them, against a
    /// session bus of the test's own — a `dbus-daemon` on a socket in the
    /// test's directory, with that directory as its home, so neither the
    /// user's bus nor a file of theirs is touched. Where `dbus-daemon` or
    /// `gdbus` is not installed the tests say so and pass; Linux CI
    /// requires both (`RICK_REQUIRE_GDBUS`).
    #[cfg(target_os = "linux")]
    mod bus {
        use std::path::{Path, PathBuf};
        use std::process::{Child, Command, Stdio};
        use std::sync::{Arc, Mutex, PoisonError};
        use std::time::{Duration, Instant};

        use app::Event;
        use platform::Integration as _;
        use platform::file_manager::{
            BUS_NAME, Call, OBJECT_PATH, SERVICE_ARGUMENT, Service, Show,
        };
        use platform::integration::{Actual, Item, Receipt, Request, Scope};

        use super::{Instance, location};

        /// Whether `name` is a program on the `PATH`.
        fn installed(name: &str) -> bool {
            std::env::var_os("PATH").is_some_and(|path| {
                std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
            })
        }

        /// Whether the bus daemon and `gdbus` are here; a note where they
        /// are not, and a failure where they are required.
        fn tools() -> bool {
            let missing: Vec<&str> = ["dbus-daemon", "gdbus"]
                .into_iter()
                .filter(|name| !installed(name))
                .collect();
            if missing.is_empty() {
                return true;
            }
            assert!(
                std::env::var_os("RICK_REQUIRE_GDBUS").is_none(),
                "{missing:?} required here and not installed"
            );
            eprintln!("{missing:?} not installed; skipping");
            false
        }

        /// A session bus of the test's own.
        struct Bus {
            daemon: Child,
            address: String,
        }

        impl Bus {
            /// Starts the daemon on a socket in `dir`, with `dir` as its
            /// home; it starts the services whose files are in
            /// `services`, in that same environment.
            fn start(dir: &Path, services: Option<&Path>) -> Self {
                let socket = dir.join("bus");
                let servicedir = services.map_or_else(String::new, |services| {
                    format!("<servicedir>{}</servicedir>", services.display())
                });
                let config = dir.join("bus.conf");
                std::fs::write(
                    &config,
                    format!(
                        "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration \
                         1.0//EN\" \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n\
                         <busconfig>\n<type>session</type>\n\
                         <listen>unix:path={}</listen>\n{servicedir}\n\
                         <policy context=\"default\">\n\
                         <allow send_destination=\"*\" eavesdrop=\"true\"/>\n\
                         <allow eavesdrop=\"true\"/>\n<allow own=\"*\"/>\n</policy>\n\
                         </busconfig>\n",
                        socket.display()
                    ),
                )
                .unwrap_or_else(|e| panic!("{e}"));
                let home = dir.join("bus-home");
                for sub in ["data", "config", "run"] {
                    std::fs::create_dir_all(home.join(sub)).unwrap_or_else(|e| panic!("{e}"));
                }
                let daemon = Command::new("dbus-daemon")
                    .arg(format!("--config-file={}", config.display()))
                    .arg("--nofork")
                    .env("HOME", &home)
                    .env("XDG_DATA_HOME", home.join("data"))
                    .env("XDG_CONFIG_HOME", home.join("config"))
                    .env("XDG_RUNTIME_DIR", home.join("run"))
                    .env_remove("DBUS_SESSION_BUS_ADDRESS")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap_or_else(|e| panic!("dbus-daemon: {e}"));
                let deadline = Instant::now() + Duration::from_secs(10);
                while std::os::unix::net::UnixStream::connect(&socket).is_err() {
                    assert!(Instant::now() < deadline, "the bus did not come up");
                    std::thread::sleep(Duration::from_millis(10));
                }
                Self {
                    daemon,
                    address: format!("unix:path={}", socket.display()),
                }
            }

            /// The acceptance line's call, word for word but for the
            /// method and the files: `gdbus call --session --dest
            /// org.freedesktop.FileManager1 --object-path
            /// /org/freedesktop/FileManager1 --method
            /// org.freedesktop.FileManager1.<method> "['file:///…']"
            /// "<startup id>"`, the session being this bus.
            fn call(&self, method: &str, files: &[PathBuf], startup_id: &str) -> Command {
                let uris: Vec<String> = files
                    .iter()
                    .map(|file| format!("'{}'", platform::paths::file_uri(file)))
                    .collect();
                let mut command = Command::new("gdbus");
                command
                    .args(["call", "--session", "--dest", BUS_NAME])
                    .args(["--object-path", OBJECT_PATH, "--method"])
                    .arg(format!("{BUS_NAME}.{method}"))
                    .arg(format!("[{}]", uris.join(", ")))
                    .arg(startup_id)
                    .env("DBUS_SESSION_BUS_ADDRESS", &self.address);
                command
            }
        }

        impl Bus {
            /// The connection that owns the file manager's name now, as
            /// the bus names it; `None` while nobody does.
            fn owner(&self) -> Option<String> {
                let out = Command::new("gdbus")
                    .args(["call", "--session", "--dest", "org.freedesktop.DBus"])
                    .args(["--object-path", "/org/freedesktop/DBus", "--method"])
                    .args(["org.freedesktop.DBus.GetNameOwner", BUS_NAME])
                    .env("DBUS_SESSION_BUS_ADDRESS", &self.address)
                    .output()
                    .unwrap_or_else(|e| panic!("gdbus: {e}"));
                out.status
                    .success()
                    .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            }
        }

        impl Drop for Bus {
            fn drop(&mut self) {
                let _ = self.daemon.kill();
                let _ = self.daemon.wait();
            }
        }

        /// The instance takes the name on `bus`, as the running Rick
        /// does: the calls go to the application the way a second
        /// launch's command line does.
        async fn serve(instance: &Instance, bus: &Bus) -> Service {
            platform::file_manager::serve(
                Some(&bus.address),
                rick::instance::bus_calls(instance.externals.clone()),
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"))
        }

        /// Waits for `folder` as the focused pane's tab with exactly
        /// `names` selected.
        async fn shown(instance: &mut Instance, folder: &Path, names: &[&str]) {
            let folder = location(folder);
            instance
                .wait_for("the folder's tab with the selection", |app| {
                    app.model()
                        .active_tab(app.model().focused)
                        .is_some_and(|(_, tab)| {
                            tab.location == folder
                                && tab.selected().map(|e| e.name.as_str()).collect::<Vec<_>>()
                                    == names
                        })
                })
                .await;
        }

        fn succeeded(out: &std::process::Output) {
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        #[test]
        fn show_items_opens_the_folder_with_the_item_selected() {
            if !tools() {
                return;
            }
            runtime::block_on(async {
                let mut instance = Instance::start().await;
                let bus = Bus::start(instance.dir.path(), None);
                let _service = serve(&instance, &bus).await;
                let windows = instance.app.model().windows().count();
                let folder = instance.dir.path().join("folder");
                let out = instance
                    .run(bus.call("ShowItems", &[folder.join("b.txt")], ""))
                    .await;
                succeeded(&out);
                shown(&mut instance, &folder, &["b.txt"]).await;
                assert_eq!(instance.app.model().windows().count(), windows);
                // The window comes forward; the caller gave no startup id.
                assert!(
                    instance.events.iter().any(|e| matches!(
                        e,
                        Event::Activate { window, token: None }
                            if *window == instance.app.model().focused_window
                    )),
                    "{:?}",
                    instance.events
                );
            });
        }

        #[test]
        fn show_folders_opens_each_folder_and_the_startup_id_is_the_activation_token() {
            if !tools() {
                return;
            }
            runtime::block_on(async {
                let mut instance = Instance::start().await;
                let bus = Bus::start(instance.dir.path(), None);
                let _service = serve(&instance, &bus).await;
                let folder = instance.dir.path().join("folder");
                let out = instance
                    .run(bus.call("ShowFolders", std::slice::from_ref(&folder), "token-7"))
                    .await;
                succeeded(&out);
                // The folder itself, nothing selected in it.
                shown(&mut instance, &folder, &[]).await;
                assert!(
                    instance.events.iter().any(|e| matches!(
                        e,
                        Event::Activate { token: Some(token), .. } if token == "token-7"
                    )),
                    "{:?}",
                    instance.events
                );
            });
        }

        #[test]
        fn show_item_properties_selects_the_item_and_opens_properties() {
            if !tools() {
                return;
            }
            runtime::block_on(async {
                let mut instance = Instance::start().await;
                let bus = Bus::start(instance.dir.path(), None);
                let _service = serve(&instance, &bus).await;
                let folder = instance.dir.path().join("folder");
                let out = instance
                    .run(bus.call("ShowItemProperties", &[folder.join("c.txt")], ""))
                    .await;
                succeeded(&out);
                shown(&mut instance, &folder, &["c.txt"]).await;
                let deadline = Instant::now() + Duration::from_secs(20);
                while !instance
                    .events
                    .iter()
                    .any(|e| matches!(e, Event::Properties(info) if info.entry.name == "c.txt"))
                {
                    assert!(
                        Instant::now() < deadline,
                        "no Properties: {:?}",
                        instance.events
                    );
                    instance.drive().await;
                }
            });
        }

        /// Rick neither takes the name from a file manager that has it nor
        /// gives up: it waits, and the calls are its own once the other
        /// lets go.
        #[test]
        fn behind_another_file_manager_rick_waits_for_the_name() {
            if !tools() {
                return;
            }
            runtime::block_on(async {
                let mut instance = Instance::start().await;
                let bus = Bus::start(instance.dir.path(), None);
                let others: Arc<Mutex<Vec<Call>>> = Arc::default();
                let seen = Arc::clone(&others);
                let other = platform::file_manager::serve(
                    Some(&bus.address),
                    Arc::new(move |call| {
                        seen.lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(call);
                    }),
                )
                .await
                .unwrap_or_else(|e| panic!("{e}"));
                let _service = serve(&instance, &bus).await;
                let folder = instance.dir.path().join("folder");

                // The first owner answers while it is there…
                let out = instance
                    .run(bus.call("ShowItems", &[folder.join("a.txt")], ""))
                    .await;
                succeeded(&out);
                let taken: Vec<Call> = others
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                assert_eq!(
                    taken,
                    [Call {
                        show: Show::Items,
                        uris: vec![platform::paths::file_uri(&folder.join("a.txt"))],
                        startup_id: String::new(),
                    }]
                );
                assert!(
                    !instance
                        .events
                        .iter()
                        .any(|e| matches!(e, Event::Activate { .. })),
                    "the call was the other file manager's"
                );

                // …and once it is gone the name is Rick's.
                let first = bus.owner();
                assert!(first.is_some(), "the other file manager owns the name");
                drop(other);
                let deadline = Instant::now() + Duration::from_secs(20);
                loop {
                    let owner = bus.owner();
                    if owner.is_some() && owner != first {
                        break;
                    }
                    assert!(Instant::now() < deadline, "the name did not pass on");
                    instance.drive().await;
                }
                let out = instance
                    .run(bus.call("ShowItems", &[folder.join("b.txt")], ""))
                    .await;
                succeeded(&out);
                shown(&mut instance, &folder, &["b.txt"]).await;
            });
        }

        /// A desktop's settings nobody asks in these tests.
        struct NoSettings;

        impl platform::Gsettings for NoSettings {
            fn get(&self, _schema: &str, _key: &str) -> Result<String, types::Error> {
                Err(types::Error::Unsupported(
                    "no desktop settings in this test",
                ))
            }

            fn set(&self, _schema: &str, _key: &str, _value: &str) -> Result<(), types::Error> {
                Ok(())
            }
        }

        /// The second half of the acceptance line: *with Rick not
        /// running, the same call starts it*. The activation file is the
        /// one the integration item writes, into a home of the test's
        /// own; the bus reads that home's services, finds nobody owning
        /// the name and starts what the file names. No window can open
        /// here, so the file names a stand-in that records how it was
        /// started; the test then does what the started Rick does with
        /// `--dbus-service` — takes the name — and the call that started
        /// it is answered like any other.
        #[test]
        fn the_same_call_starts_rick_through_the_activation_file() {
            if !tools() {
                return;
            }
            runtime::block_on(async {
                let mut instance = Instance::start().await;
                let dir = instance.dir.path().to_path_buf();
                let record = dir.join("started-with");
                let stand_in = dir.join("rick-stand-in");
                std::fs::write(
                    &stand_in,
                    format!(
                        "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
                        record.display()
                    ),
                )
                .unwrap_or_else(|e| panic!("{e}"));
                std::fs::set_permissions(
                    &stand_in,
                    std::os::unix::fs::PermissionsExt::from_mode(0o755),
                )
                .unwrap_or_else(|e| panic!("{e}"));

                // The bus is up before the item is set up, as a session's
                // is, and the folder of the user's services is not there
                // yet.
                let home = dir.join("item-home");
                std::fs::create_dir_all(&home).unwrap_or_else(|e| panic!("{e}"));
                let bus = Bus::start(&dir, Some(&home.join(".local/share/dbus-1/services")));

                // The item, set up in a home of the test's own, on this
                // bus, which is asked to read its services again
                // (`dbus-broker` needs that; `dbus-daemon` also looks by
                // itself when a name has no owner).
                let mut env = platform::LinuxEnv::under(&home, &stand_in, "GNOME");
                env.session_bus = Some(bus.address.clone());
                env.path = std::env::var_os("PATH")
                    .map(|path| std::env::split_paths(&path).collect())
                    .unwrap_or_default();
                let integration = platform::LinuxIntegration::new(env, Arc::new(NoSettings));
                let request = Request {
                    scope: Scope::User,
                    binary: Some(stand_in.clone()),
                    receipt: Receipt::default(),
                    replace: false,
                };
                let receipt = integration
                    .perform_integration(Item::ShowInFolder, &request)
                    .unwrap_or_else(|e| panic!("{e}"));
                assert_eq!(
                    receipt.files,
                    ["~/.local/share/dbus-1/services/org.freedesktop.FileManager1.service"]
                );
                assert_eq!(
                    integration
                        .check_integration(Item::ShowInFolder, &request)
                        .actual,
                    Actual::SetUp
                );

                // Nobody owns the name on this bus: the call starts what
                // the file names, with the argument that opens no folder.
                let folder = dir.join("folder");
                let mut call = bus.call("ShowItems", &[folder.join("a.txt")], "");
                let calling = tokio::task::spawn_blocking(move || call.output());
                let deadline = Instant::now() + Duration::from_secs(20);
                while !record.is_file() {
                    assert!(Instant::now() < deadline, "the bus started nothing");
                    assert!(
                        !calling.is_finished(),
                        "the call ended before Rick was started"
                    );
                    instance.drive().await;
                }
                let started_with = loop {
                    // The stand-in may still be writing.
                    let text = std::fs::read_to_string(&record).unwrap_or_default();
                    if text.ends_with('\n') {
                        break text;
                    }
                    assert!(Instant::now() < deadline, "the stand-in wrote nothing");
                    instance.drive().await;
                };
                assert_eq!(started_with, format!("{SERVICE_ARGUMENT}\n"));

                // The started Rick takes the name, and the call that
                // started it reaches it.
                let _service = serve(&instance, &bus).await;
                while !calling.is_finished() {
                    assert!(Instant::now() < deadline, "the call was not answered");
                    instance.drive().await;
                }
                let out = calling
                    .await
                    .unwrap_or_else(|e| panic!("{e}"))
                    .unwrap_or_else(|e| panic!("{e}"));
                succeeded(&out);
                shown(&mut instance, &folder, &["a.txt"]).await;
            });
        }
    }
}
