//! One Rick per user session and config directory
//! (specs/system-integration.md §3): the newcomer's half, which hands the
//! command line to the running instance and exits with its status, and the
//! instance's half, which serves the endpoint and runs what arrives against
//! the application.
//!
//! A launch connects to the endpoint the platform names for the config
//! directory and sends its arguments, working directory and display
//! variables as one [`Request`]; the instance answers within a second or
//! the launch reports it unresponsive and exits non-zero. Nothing listening
//! (or a socket file a crash left, which nothing answers on) makes the
//! launch the instance: it binds the endpoint and opens the window as
//! before. The instance runs every request on the UI thread between two
//! frames, through the application's external-work channel, and replies
//! with what it made of it: a path opens as a tab, `--new-window` opens a
//! window first, an argument it does not know is answered with the help
//! and [`EXIT_USAGE`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use app::{App, Command, Event, ExternalSender};
use platform::instance::{
    Connection, EXIT_FAILED, EXIT_USAGE, Endpoint, Handover, Listener, PATIENCE, Reply, Request,
};
use runtime::oneshot;
use types::Location;

/// What the launch does next, once it has looked for an instance.
pub enum Outcome {
    /// An instance took the command line, or none could be reached and
    /// the launch should not open a window: print `text` (standard output
    /// for status zero, standard error otherwise) and exit with `status`.
    Exit {
        /// The process's exit status.
        status: i32,
        /// What to print, if anything.
        text: Option<String>,
    },
    /// Nothing listens: this launch is the instance, holding the endpoint.
    Become(Listener),
}

/// Looks for the running instance and hands `argv` over, from the working
/// directory `cwd`; binds the endpoint when there is none.
///
/// # Errors
///
/// The I/O error of the endpoint when it can be neither reached nor
/// bound: the runtime directory is not writable, the pipe refuses the
/// user.
pub fn hand_over(endpoint: &Endpoint, argv: Vec<String>, cwd: PathBuf) -> anyhow::Result<Outcome> {
    let request = Request::new(argv, cwd);
    runtime::block_on(async {
        match endpoint.hand_over(&request, PATIENCE).await? {
            Handover::Answered(reply) => {
                tracing::debug!(status = reply.status, "the running instance answered");
                Ok(Outcome::Exit {
                    status: reply.status,
                    text: reply.text,
                })
            }
            Handover::Unresponsive => Ok(Outcome::Exit {
                status: EXIT_FAILED,
                text: Some(format!(
                    "rick: a Rick is running but did not answer within {} second; it may be \
                     busy or hung",
                    PATIENCE.as_secs()
                )),
            }),
            Handover::NoInstance => match endpoint.bind().await {
                Ok(listener) => {
                    tracing::debug!(endpoint = %endpoint.path().display(), "this launch is the instance");
                    Ok(Outcome::Become(listener))
                }
                // Another launch took the endpoint between our two looks:
                // it is the instance now, so hand over to it.
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                    match endpoint.hand_over(&request, PATIENCE).await? {
                        Handover::Answered(reply) => Ok(Outcome::Exit {
                            status: reply.status,
                            text: reply.text,
                        }),
                        _ => Ok(Outcome::Exit {
                            status: EXIT_FAILED,
                            text: Some(
                                "rick: another Rick took the endpoint and did not answer"
                                    .to_owned(),
                            ),
                        }),
                    }
                }
                Err(error) => Err(error.into()),
            },
        }
    })
}

/// What the instance makes of a request's arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// `--new-window` was given: the paths open in a window of their own.
    pub new_window: bool,
    /// The folders to open, resolved against the caller's working
    /// directory; the working directory itself when none was given.
    pub paths: Vec<PathBuf>,
}

/// Parses a request's arguments: `--new-window` and paths, resolved
/// against `cwd` (made canonical where they exist, kept as given
/// otherwise).
///
/// # Errors
///
/// The unknown argument, for the usage reply.
pub fn parse(argv: &[String], cwd: &Path) -> Result<Parsed, String> {
    let mut new_window = false;
    let mut paths = Vec::new();
    let mut only_paths = false;
    for arg in argv {
        if !only_paths && arg == "--" {
            only_paths = true;
        } else if !only_paths && arg == "--new-window" {
            new_window = true;
        } else if !only_paths && arg.starts_with('-') && arg != "-" {
            return Err(arg.clone());
        } else {
            paths.push(resolve(cwd, Path::new(arg)));
        }
    }
    if paths.is_empty() {
        paths.push(resolve(cwd, Path::new(".")));
    }
    Ok(Parsed { new_window, paths })
}

/// `path` against `cwd`: canonical where it names something, so a launcher's
/// relative path and a symlink both reach the folder itself; as joined
/// otherwise, for the pane to say what it cannot list.
pub fn resolve(cwd: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    std::fs::canonicalize(&joined).unwrap_or(joined)
}

/// A folder as a location of the local provider.
pub fn location(path: &Path) -> Location {
    Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path))
}

/// Runs a request against the application: the paths open as tabs of the
/// focused window, in a new window first when asked; an argument the
/// instance does not know is answered with the help and [`EXIT_USAGE`].
pub fn dispatch(app: &mut App, events: &mut Vec<Event>, request: &Request, help: &str) -> Reply {
    let parsed = match parse(&request.argv, &request.cwd) {
        Ok(parsed) => parsed,
        Err(unknown) => {
            return Reply::usage(format!("rick: unknown argument `{unknown}`\n\n{help}"));
        }
    };
    tracing::info!(
        new_window = parsed.new_window,
        paths = ?parsed.paths,
        "command line from a second launch"
    );
    if parsed.new_window {
        app.handle(Command::NewWindow, events);
    }
    for path in &parsed.paths {
        app.handle(Command::NewTab(location(path)), events);
    }
    Reply::ok(None)
}

/// Serves the endpoint for the rest of the process: every connection's
/// request goes to the application's thread through `externals`, and the
/// reply comes back the same way. A connection that sends nothing within
/// [`PATIENCE`] is dropped. Returns the task, to abort at exit.
pub fn serve(
    mut listener: Listener,
    externals: ExternalSender,
    help: Arc<str>,
) -> tokio::task::JoinHandle<()> {
    runtime::handle().spawn(async move {
        loop {
            match listener.accept().await {
                Ok(connection) => {
                    let externals = externals.clone();
                    let help = Arc::clone(&help);
                    runtime::handle().spawn(answer(connection, externals, help));
                }
                Err(error) => {
                    tracing::warn!(%error, "the single-instance endpoint failed to accept");
                    // Not a hot loop on a broken listener.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    })
}

/// One launch's connection: its request run on the application's thread,
/// its reply written back.
async fn answer(mut connection: Connection, externals: ExternalSender, help: Arc<str>) {
    let request = match connection.request(PATIENCE).await {
        Ok(request) => request,
        Err(error) => {
            tracing::debug!(%error, "a launch connected without a readable request");
            return;
        }
    };
    let (tx, rx) = oneshot::channel();
    let queued = externals.send(Box::new(move |app, events| {
        let reply = dispatch(app, events, &request, &help);
        let _ = tx.send(reply);
    }));
    let reply = if queued {
        match rx.await {
            Ok(reply) => reply,
            Err(_) => Reply::failed(
                EXIT_FAILED,
                "rick: the running instance dropped the request",
            ),
        }
    } else {
        Reply::failed(EXIT_FAILED, "rick: the running instance is shutting down")
    };
    if let Err(error) = connection.reply(&reply).await {
        tracing::debug!(%error, "the launch went away before the reply");
    }
}

/// The exit status of a usage error, re-exported for the binary's own
/// parsing.
pub const USAGE: i32 = EXIT_USAGE;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use platform::{ConfigDir as _, Fake};

    use super::*;

    fn app(fake: &Arc<Fake>, start: &Path) -> App {
        App::new(app::Config {
            platform: fake.clone(),
            ui: Arc::new(ui_egui::EguiHost::new()),
            extensions: Vec::new(),
            config_dir: Some(fake.config_dir()),
            start: Some(location(start)),
        })
    }

    fn request(argv: &[&str], cwd: &Path) -> Request {
        Request {
            protocol_version: platform::instance::PROTOCOL_VERSION,
            argv: argv.iter().map(|a| (*a).to_owned()).collect(),
            cwd: cwd.to_path_buf(),
            env: std::collections::BTreeMap::default(),
        }
    }

    #[test]
    fn paths_resolve_against_the_callers_directory_and_none_is_the_directory_itself() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap_or_else(|e| panic!("{e}"));
        let canonical = dir.path().canonicalize().unwrap_or_else(|e| panic!("{e}"));
        let parsed = parse(&["sub".to_owned(), "nowhere".to_owned()], dir.path())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!parsed.new_window);
        assert_eq!(
            parsed.paths,
            [canonical.join("sub"), dir.path().join("nowhere")]
        );
        let none = parse(&[], dir.path()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(none.paths, std::slice::from_ref(&canonical));
        let window = parse(
            &["--new-window".to_owned(), "--".to_owned(), "-x".to_owned()],
            dir.path(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(window.new_window);
        assert_eq!(window.paths, [dir.path().join("-x")]);
        assert_eq!(
            parse(&["--frobnicate".to_owned()], dir.path()),
            Err("--frobnicate".to_owned())
        );
    }

    #[test]
    fn a_dispatched_path_opens_a_tab_and_an_unknown_argument_gets_the_help() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let home = dir.path().join("home");
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&home).unwrap_or_else(|e| panic!("{e}"));
        std::fs::create_dir_all(&sub).unwrap_or_else(|e| panic!("{e}"));
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let mut app = app(&fake, &home);
        let mut events = Vec::new();
        let windows_before = app.model().windows().count();
        let panes_before = app.model().panes().count();

        let reply = dispatch(
            &mut app,
            &mut events,
            &request(&["sub"], dir.path()),
            "HELP",
        );
        assert_eq!(reply, Reply::ok(None));
        // A tab of its own, in a workspace of its own, in the same window.
        assert_eq!(app.model().panes().count(), panes_before + 1);
        assert_eq!(app.model().windows().count(), windows_before);
        let focused = app.model().focused;
        let (_, tab) = app
            .model()
            .active_tab(focused)
            .unwrap_or_else(|| panic!("a focused tab"));
        assert_eq!(
            tab.location,
            location(&resolve(dir.path(), Path::new("sub")))
        );

        let reply = dispatch(
            &mut app,
            &mut events,
            &request(&["--new-window", "sub"], dir.path()),
            "HELP",
        );
        assert_eq!(reply.status, 0);
        assert_eq!(app.model().windows().count(), windows_before + 1);

        let reply = dispatch(
            &mut app,
            &mut events,
            &request(&["--bogus"], dir.path()),
            "HELP",
        );
        assert_eq!(reply.status, EXIT_USAGE);
        let text = reply.text.unwrap_or_default();
        assert!(text.contains("--bogus") && text.ends_with("HELP"), "{text}");
    }
}
