use std::sync::Arc;
use std::time::{Duration, Instant};

use ext::platform::{ConfigDir as _, Fake};
use ext::{
    ActionCtx, ActionOutcome, Availability, Invocation, Level, PaneId, Registry, UiHost, View,
};
use runtime::JobRunner;
use types::Location;

use super::*;

struct Headless;

impl UiHost for Headless {
    fn notify(&self, _: Level, _: &str) {}
    fn confirm(&self, _: &str, _: &str) -> bool {
        true
    }
    fn prompt(&self, _: &str, default: &str) -> Option<String> {
        Some(default.to_owned())
    }
    fn show(&self, _: &str, _: &str) {}
    fn form(&self, form: ext::Form) -> Option<ext::Values> {
        Some(form.defaults())
    }
}

fn ctx(dir: &std::path::Path) -> ActionCtx {
    ActionCtx {
        pane: PaneId(1),
        location: Location::new(
            types::ProviderId("local"),
            ext::platform::paths::from_path(dir),
        ),
        selection: Vec::new(),
        invoked_from: Invocation::Shortcut,
        other: None,
    }
}

/// Steps until `done` holds or the limit passes.
fn wait_for(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    done()
}

/// The command that prints the working directory in the default shell.
fn pwd_command() -> &'static [u8] {
    if std::env::consts::OS == "windows" {
        b"cd\r\n"
    } else {
        b"pwd\n"
    }
}

#[test]
fn the_console_opens_at_the_location_and_runs_pwd() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let dir = std::fs::canonicalize(dir.path()).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut registry = Registry::with_platform(fake.clone());
    registry
        .load(&TerminalExtension::new())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(registry.settings_schemas().len(), 1);
    let action = Arc::clone(&registry.actions()[0]);
    assert_eq!(action.id(), CONSOLE);
    assert_eq!(action.availability(&ctx(&dir)), Availability::Enabled);
    let outcome = action
        .run(ctx(&dir), &JobRunner::new(), &Headless)
        .unwrap_or_else(|e| panic!("{e}"));
    let ActionOutcome::View(View::Console(console)) = outcome else {
        panic!("not a console: {outcome:?}");
    };
    assert_eq!(console.exit_status(), None);
    console.resize(100, 30);
    let wanted = dir.display().to_string();
    console.write(pwd_command());
    let shown = wait_for(Duration::from_secs(20), || {
        let screen = console.screen(0);
        let text: String = screen.cells.iter().map(|c| c.ch).collect();
        text.contains(&wanted)
    });
    let screen = console.screen(0);
    let text: String = screen
        .cells
        .chunks(usize::from(screen.cols))
        .map(|row| {
            row.iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(shown, "the working directory did not appear:\n{text}");
    assert_eq!(screen.cols, 100);
    assert_eq!(screen.rows, 30);
    assert_ne!(console.title(), "");
    // Leaving the shell ends the session.
    console.write(b"exit\r\n");
    assert!(
        wait_for(Duration::from_secs(20), || console.exit_status().is_some()),
        "the shell did not exit"
    );
}

#[test]
fn the_setting_sends_the_console_to_the_system_terminal() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let config = fake.config_dir();
    std::fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(
        config.join(format!("{}.toml", ID.0)),
        "console = \"system\"\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let mut registry = Registry::with_platform(fake.clone());
    registry
        .load(&TerminalExtension::new())
        .unwrap_or_else(|e| panic!("{e}"));
    let action = Arc::clone(&registry.actions()[0]);
    let outcome = action
        .run(ctx(dir.path()), &JobRunner::new(), &Headless)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(outcome, ActionOutcome::Done));
    let opened = fake.terminals();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].0, dir.path());
    assert_eq!(opened[0].1, None);
    // A location of another provider offers no console.
    let elsewhere = ActionCtx {
        location: Location::new(types::ProviderId("mem"), "/x"),
        ..ctx(dir.path())
    };
    assert_eq!(action.availability(&elsewhere), Availability::Hidden);
}
