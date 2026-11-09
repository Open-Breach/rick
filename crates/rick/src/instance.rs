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
//! frames, through the application's external-work channel, parses it as
//! the command line ([`cli`], the parser) with the loaded plugins'
//! commands known, and replies with what it made of it: paths open as
//! tabs, `reveal` opens the files' folders, a plugin's command runs in the
//! extension that declared it, `--help` is the help, and a command line
//! the parser refuses is answered with its message and [`EXIT_USAGE`].

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use app::{App, Command, Event, ExternalSender};
use ext::CommandSpec;
use platform::instance::{
    Connection, EXIT_FAILED, EXIT_USAGE, Endpoint, Handover, Listener, PATIENCE, Reply, Request,
};
use runtime::oneshot;
use types::Location;

use crate::cli::{self, Parsed, Usage};

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
pub fn hand_over(endpoint: &Endpoint, argv: Vec<String>, cwd: PathBuf) -> io::Result<Outcome> {
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
                Err(error) => Err(error),
            },
        }
    })
}

/// A folder as a location of the local provider.
#[must_use]
pub fn location(path: &Path) -> Location {
    Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path))
}

/// The plugin commands the loaded extensions declare, for the parser and
/// the help.
#[must_use]
pub fn plugin_commands(app: &App) -> Vec<CommandSpec> {
    app.registry()
        .manifests()
        .iter()
        .flat_map(|m| m.commands.iter().copied())
        .collect()
}

/// The activation token a launch received from its launcher, for the
/// platform to raise the window with: `XDG_ACTIVATION_TOKEN` (Wayland) or
/// `DESKTOP_STARTUP_ID` (X11 startup notification).
pub fn activation_token(request: &Request) -> Option<&str> {
    request
        .env
        .get("XDG_ACTIVATION_TOKEN")
        .or_else(|| request.env.get("DESKTOP_STARTUP_ID"))
        .map(String::as_str)
        .filter(|token| !token.is_empty())
}

/// Runs a request against the application: parses it as the command line
/// with the loaded plugins' commands known and applies it; the reply is
/// what the launch prints and exits with.
pub fn dispatch(app: &mut App, events: &mut Vec<Event>, request: &Request) -> Reply {
    let plugins = plugin_commands(app);
    match cli::parse(&request.argv, &request.cwd, &plugins) {
        Err(Usage(text)) => Reply::usage(text),
        Ok(parsed) => apply(app, events, parsed, &request.cwd, activation_token(request)),
    }
}

/// Applies a parsed command line to the application
/// (specs/system-integration.md §3): the paths open as tabs of the most
/// recently focused window, the first in a window of its own with
/// `--new-window`; `reveal` opens each file's folder the same way and
/// selects the files once listed; a plugin's command runs in the
/// extension that declared it, its text or error going back to the
/// caller; the help and the version are text for the caller. Whatever
/// changed the window is followed by its activation, with the launcher's
/// `token`, so the caller sees what happened.
pub fn apply(
    app: &mut App,
    events: &mut Vec<Event>,
    parsed: Parsed,
    cwd: &Path,
    token: Option<&str>,
) -> Reply {
    let activate = |app: &mut App, events: &mut Vec<Event>| {
        app.handle(
            Command::Activate {
                token: token.map(str::to_owned),
            },
            events,
        );
    };
    match parsed {
        Parsed::Open { paths, new_window } => {
            tracing::info!(new_window, ?paths, "open from the command line");
            let mut paths = paths.into_iter();
            if new_window && let Some(first) = paths.next() {
                app.handle(Command::NewWindowAt(location(&first)), events);
            }
            for path in paths {
                app.handle(Command::NewTab(location(&path)), events);
            }
            activate(app, events);
            Reply::ok(None)
        }
        Parsed::Reveal { files, new_window } => {
            tracing::info!(new_window, ?files, "reveal from the command line");
            app.handle(
                Command::Reveal {
                    items: files.iter().map(|f| location(f)).collect(),
                    new_window,
                },
                events,
            );
            activate(app, events);
            Reply::ok(None)
        }
        Parsed::Plugin { name, args } => {
            tracing::info!(command = %name, ?args, "plugin command from the command line");
            match app.run_cli(&name, args, cwd.to_path_buf(), events) {
                None => Reply::usage(format!(
                    "rick: unknown command `{name}`\n\n{}",
                    cli::help(&plugin_commands(app))
                )),
                Some(Ok(Some(text))) => Reply::ok(Some(text)),
                Some(Ok(None)) => {
                    // The command did something in the window: show it.
                    activate(app, events);
                    Reply::ok(None)
                }
                Some(Err(error)) => Reply::failed(EXIT_FAILED, format!("rick {name}: {error}")),
            }
        }
        Parsed::Help(text) | Parsed::Version(text) => Reply::ok(Some(text)),
    }
}

/// Serves the endpoint for the rest of the process: every connection's
/// request goes to the application's thread through `externals`, and the
/// reply comes back the same way. A connection that sends nothing within
/// [`PATIENCE`] is dropped. Returns the task, to abort at exit.
pub fn serve(mut listener: Listener, externals: ExternalSender) -> tokio::task::JoinHandle<()> {
    runtime::handle().spawn(async move {
        loop {
            match listener.accept().await {
                Ok(connection) => {
                    let externals = externals.clone();
                    runtime::handle().spawn(answer(connection, externals));
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
async fn answer(mut connection: Connection, externals: ExternalSender) {
    let request = match connection.request(PATIENCE).await {
        Ok(request) => request,
        Err(error) => {
            tracing::debug!(%error, "a launch connected without a readable request");
            return;
        }
    };
    let (tx, rx) = oneshot::channel();
    let queued = externals.send(Box::new(move |app, events| {
        let reply = dispatch(app, events, &request);
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
    use std::sync::{Arc, Mutex, PoisonError};

    use ext::{
        API_VERSION, Capabilities, CliCommand, CliCtx, CliOutcome, Extension, JobRunner, Manifest,
        Platforms, Points, Registry, UiHost, Version,
    };
    use platform::{ConfigDir as _, Fake};
    use types::{Error, ExtId};

    use super::*;

    /// A plugin with two commands: `echo`, which answers its arguments
    /// and the working directory as text, and `navigate`, which takes the
    /// focused pane to its argument through an action's outcome.
    struct Echoer(Manifest);

    const SPECS: &[CommandSpec] = &[
        CommandSpec {
            name: "echo",
            help: "Echo the arguments",
        },
        CommandSpec {
            name: "navigate",
            help: "Take the focused pane somewhere",
        },
    ];

    struct Echo;

    impl CliCommand for Echo {
        fn name(&self) -> &'static str {
            "echo"
        }
        fn run(&self, ctx: CliCtx, _: &JobRunner, _: &dyn UiHost) -> Result<CliOutcome, Error> {
            if ctx.args.first().is_some_and(|a| a == "--fail") {
                return Err(Error::Unsupported("asked to fail"));
            }
            Ok(CliOutcome::Text(format!(
                "{} from {}",
                ctx.args.join(" "),
                ctx.cwd.display()
            )))
        }
    }

    struct NavigateTo(Arc<Mutex<Option<PathBuf>>>);

    impl CliCommand for NavigateTo {
        fn name(&self) -> &'static str {
            "navigate"
        }
        fn run(&self, ctx: CliCtx, _: &JobRunner, _: &dyn UiHost) -> Result<CliOutcome, Error> {
            let [target] = ctx.args.as_slice() else {
                return Err(Error::Unsupported("usage: rick navigate <folder>"));
            };
            let path = cli::resolve(&ctx.cwd, Path::new(target));
            *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(path.clone());
            Ok(CliOutcome::Outcome(ext::ActionOutcome::Navigate(location(
                &path,
            ))))
        }
    }

    impl Extension for Echoer {
        fn manifest(&self) -> &Manifest {
            &self.0
        }
        fn register(&self, reg: &mut Registry) -> Result<(), Error> {
            reg.command(Arc::new(Echo));
            reg.command(Arc::new(NavigateTo(Arc::new(Mutex::new(None)))));
            Ok(())
        }
    }

    fn echoer() -> Arc<dyn Extension> {
        Arc::new(Echoer(Manifest {
            id: ExtId("echoer"),
            name: "Echoer",
            version: Version::new(0, 1, 0),
            api_version: API_VERSION,
            requires: Capabilities::empty(),
            points: Points::empty(),
            platforms: Platforms::ALL,
            previews: &[],
            commands: SPECS,
        }))
    }

    fn app(fake: &Arc<Fake>, start: &Path, extensions: Vec<Arc<dyn Extension>>) -> App {
        App::new(app::Config {
            platform: fake.clone(),
            ui: Arc::new(ui_egui::EguiHost::new()),
            extensions,
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
    fn paths_open_tabs_a_new_window_opens_first_and_an_unknown_argument_gets_the_help() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let home = dir.path().join("home");
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&home).unwrap_or_else(|e| panic!("{e}"));
        std::fs::create_dir_all(&sub).unwrap_or_else(|e| panic!("{e}"));
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let mut app = app(&fake, &home, Vec::new());
        let mut events = Vec::new();
        let windows_before = app.model().windows().count();
        let panes_before = app.model().panes().count();

        let mut with_token = request(&["sub"], dir.path());
        with_token
            .env
            .insert("XDG_ACTIVATION_TOKEN".to_owned(), "tok-1".to_owned());
        let reply = dispatch(&mut app, &mut events, &with_token);
        assert_eq!(reply, Reply::ok(None));
        // A tab of its own, in a workspace of its own, in the same window,
        // which is then raised with the launcher's token.
        assert_eq!(app.model().panes().count(), panes_before + 1);
        assert_eq!(app.model().windows().count(), windows_before);
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::Activate { window, token } if *window == app.model().focused_window && token.as_deref() == Some("tok-1")
            )),
            "{events:?}"
        );
        let focused = app.model().focused;
        let (_, tab) = app
            .model()
            .active_tab(focused)
            .unwrap_or_else(|| panic!("a focused tab"));
        assert_eq!(
            tab.location,
            location(&cli::resolve(dir.path(), Path::new("sub")))
        );

        let reply = dispatch(
            &mut app,
            &mut events,
            &request(&["--new-window", "sub"], dir.path()),
        );
        assert_eq!(reply.status, 0);
        assert_eq!(app.model().windows().count(), windows_before + 1);
        // The new window holds that one tab, and is the focused one.
        let new_window = app.model().focused_window;
        assert_eq!(app.model().workspaces(new_window).len(), 1);

        // Reveal opens the file's folder, once for two files in it.
        let panes = app.model().panes().count();
        let reply = dispatch(
            &mut app,
            &mut events,
            &request(&["reveal", "sub/a.txt", "sub/b.txt"], dir.path()),
        );
        assert_eq!(reply, Reply::ok(None));
        assert_eq!(app.model().panes().count(), panes + 1);

        let reply = dispatch(&mut app, &mut events, &request(&["--bogus"], dir.path()));
        assert_eq!(reply.status, EXIT_USAGE);
        let text = reply.text.unwrap_or_default();
        assert!(
            text.contains("--bogus") && text.contains("reveal"),
            "{text}"
        );

        let reply = dispatch(&mut app, &mut events, &request(&["--help"], dir.path()));
        assert_eq!(reply.status, 0);
        assert!(
            reply
                .text
                .as_deref()
                .is_some_and(|t| t.contains("--new-window")),
            "{reply:?}"
        );
    }

    #[test]
    fn a_plugin_command_runs_in_its_extension_and_answers_the_caller() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let home = dir.path().join("home");
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&home).unwrap_or_else(|e| panic!("{e}"));
        std::fs::create_dir_all(&sub).unwrap_or_else(|e| panic!("{e}"));
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let mut app = app(&fake, &home, vec![echoer()]);
        let mut events = Vec::new();
        assert_eq!(plugin_commands(&app), SPECS);

        // Text goes back to the caller, with the arguments as typed.
        let reply = dispatch(
            &mut app,
            &mut events,
            &request(&["echo", "--loud", "hi"], dir.path()),
        );
        assert_eq!(
            reply,
            Reply::ok(Some(format!("--loud hi from {}", dir.path().display())))
        );

        // An error is the caller's exit status 1 with the text.
        let reply = dispatch(
            &mut app,
            &mut events,
            &request(&["echo", "--fail"], dir.path()),
        );
        assert_eq!(reply.status, EXIT_FAILED);
        assert!(
            reply
                .text
                .as_deref()
                .is_some_and(|t| t.contains("asked to fail")),
            "{reply:?}"
        );

        // An action's outcome applies to the focused pane: here a
        // navigation, resolved against the caller's directory.
        let reply = dispatch(
            &mut app,
            &mut events,
            &request(&["navigate", "sub"], dir.path()),
        );
        assert_eq!(reply, Reply::ok(None));
        let focused = app.model().focused;
        let (_, tab) = app
            .model()
            .active_tab(focused)
            .unwrap_or_else(|| panic!("a focused tab"));
        assert_eq!(
            tab.location,
            location(&cli::resolve(dir.path(), Path::new("sub")))
        );

        // The help lists the plugin's commands.
        let reply = dispatch(&mut app, &mut events, &request(&["--help"], dir.path()));
        let text = reply.text.unwrap_or_default();
        assert!(
            text.contains("echo") && text.contains("Echo the arguments"),
            "{text}"
        );

        // An unknown command is a usage error with the help.
        let reply = dispatch(
            &mut app,
            &mut events,
            &request(&["frobnicate", "x"], dir.path()),
        );
        assert_eq!(reply.status, EXIT_USAGE);
        let text = reply.text.unwrap_or_default();
        assert!(
            text.contains("frobnicate") && text.contains("echo"),
            "{text}"
        );
    }
}
