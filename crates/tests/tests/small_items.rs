//! Acceptance (issue 139, item 11): the small items of the test session
//! that have no frame of their own.
//!
//! - The window controls sit wider apart: 16 px icons 12 px apart, each
//!   answering over its whole 28 px step, with the divider in a 25 px gap
//!   before the layout toggles; every one has an accessible name.
//! - The settings page has no close button of its own where the settings
//!   are a window; drawn inside the main window it keeps its one.
//! - The Hash dialogue's secondary line is cut before the close button.
//! - A console tab is titled *Console*, not the folder it was opened in.
//! - An operation's error is never lost to the ellipsis: the drawer's row
//!   unfolds to the whole text on a click, and the failure is reported
//!   where the user acted, in a notice that stays until it is dismissed.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use ext::{
    API_VERSION, Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability,
    Capabilities, Extension, Level, Manifest, Platforms, Points, Registry, Shortcut, UiHost,
    Version,
};
use model::SelectionChange;
use platform::{ConfigDir, Fake};
use runtime::JobRunner;
use tests::local;
use types::{Error, ExtId};
use ui_egui::keys::Intent;
use ui_egui::theme;
use ui_egui::{EguiHost, RickUi};

/// What the failing job says: longer than any row or notice is wide.
const LONG_ERROR: &str = "Invalid argument: the destination \
    /mnt/archive/2026/photos/holiday/day-one/edited/IMG_1204-final-retouched.png could not be \
    written, because the volume it is on was taken away while the copy was running";

const SYNC: ActionId = ActionId("test.sync");

/// An action whose job fails with [`LONG_ERROR`].
struct Sync;

impl Action for Sync {
    fn id(&self) -> ActionId {
        SYNC
    }
    fn label(&self) -> String {
        "Sync photos".to_owned()
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, _: &ActionCtx) -> Availability {
        Availability::Enabled
    }
    fn run(
        &self,
        _ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let handle = jobs.spawn_named("Sync photos", |_job| async move {
            Err::<(), Error>(Error::Io(std::io::Error::other(LONG_ERROR)))
        });
        Ok(ActionOutcome::Started(handle.id))
    }
}

struct SyncExtension(Manifest);

impl SyncExtension {
    fn new() -> Self {
        Self(Manifest {
            id: ExtId("test-sync"),
            name: "Sync",
            description: "",
            version: Version::new(1, 0, 0),
            api_version: API_VERSION,
            requires: Capabilities::empty(),
            points: Points::ACTION,
            platforms: Platforms::ALL,
            previews: &[],
            commands: &[],
        })
    }
}

impl Extension for SyncExtension {
    fn manifest(&self) -> &Manifest {
        &self.0
    }
    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.action(Arc::new(Sync));
        Ok(())
    }
}

fn app(fake: &Arc<Fake>, start: &Path, host: &Arc<EguiHost>) -> app::App {
    app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(SyncExtension::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    })
}

fn window(start: &Path) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app(&fake, start, &host);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 800.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
}

fn settle(harness: &mut Harness<'_, RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for _ in 0..3 {
            harness.step();
        }
        if !harness.state().app().is_busy() {
            std::thread::sleep(Duration::from_millis(30));
            harness.step();
            if !harness.state().app().is_busy() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn steps(harness: &mut Harness<'_, RickUi>) {
    for _ in 0..4 {
        harness.step();
    }
}

#[test]
fn the_window_controls_sit_a_step_apart_and_each_has_a_name() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = window(dir.path());
    // Right to left: close, maximise, minimise; then the divider's gap;
    // then the preview and the side panel toggles.
    let rect = |harness: &Harness<'_, RickUi>, role: Role, name: &str| {
        harness
            .get_all_by_role_and_label(role, name)
            .map(|node| node.rect())
            .find(|rect| rect.top() < 44.0 && rect.left() > 900.0)
            .unwrap_or_else(|| panic!("no {name} in the title row"))
    };
    let close = rect(&harness, Role::Button, "Close window");
    let maximise = rect(&harness, Role::Button, "Maximise");
    let minimise = rect(&harness, Role::Button, "Minimise");
    let preview = rect(&harness, Role::CheckBox, "Toggle preview pane");
    let side = rect(&harness, Role::CheckBox, "Toggle side panel");

    // Each control is a 28 px step, as tall as a tool button: the 16 px
    // icon in its middle is 12 px from the next one.
    for control in [close, maximise, minimise, preview, side] {
        assert!((control.width() - 28.0).abs() < 0.5, "{control:?}");
        assert!((control.height() - 34.0).abs() < 0.5, "{control:?}");
    }
    assert!(
        (close.left() - maximise.right()).abs() < 0.5,
        "no dead space"
    );
    assert!((maximise.left() - minimise.right()).abs() < 0.5);
    assert!((side.right() - preview.left()).abs() < 0.5);
    // Between the toggles' icons and the window controls': 25 px, where
    // 12 px is the rule inside a group.
    let icon_gap = (minimise.center().x - 8.0) - (preview.center().x + 8.0);
    assert!((icon_gap - 25.0).abs() < 0.5, "{icon_gap}");
    // The last icon ends at the row's margin, inside the window's frame.
    let window = harness.state().app().model().focused_window;
    let content = harness
        .state()
        .content_of(window)
        .unwrap_or_else(|| panic!("the window's content"));
    let margin = content.right() - (close.center().x + 8.0);
    assert!((margin - 12.0).abs() < 0.5, "{margin}");

    // The toggles say whether they are on, and take the click over the
    // whole step.
    let toggled = |harness: &Harness<'_, RickUi>, name: &str| {
        harness
            .get_all_by_role_and_label(Role::CheckBox, name)
            .find(|node| node.rect().top() < 44.0)
            .map_or_else(
                || panic!("no {name}"),
                |node| node.accesskit_node().toggled(),
            )
    };
    assert_eq!(
        toggled(&harness, "Toggle side panel"),
        Some(egui::accesskit::Toggled::True)
    );
    let edge = egui::pos2(side.left() + 2.0, side.center().y);
    harness.event(egui::Event::PointerMoved(edge));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: edge,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    steps(&mut harness);
    assert_eq!(
        toggled(&harness, "Toggle side panel"),
        Some(egui::accesskit::Toggled::False),
        "a click at the step's edge is the control's"
    );
}

#[test]
fn the_settings_page_has_a_close_button_only_where_it_is_embedded() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app(&fake, dir.path(), &host);
    for embedded in [false, true] {
        let mut settings = ui_egui::settings_ui::SettingsWindow::default();
        let mut requests = Vec::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(880.0, 640.0))
            .build_ui(|ui| {
                let rect = ui.max_rect();
                settings.content(ui, rect, embedded, &app, &mut requests);
            });
        harness.step();
        harness.step();
        // The page is there either way.
        harness.get_by_role_and_label(Role::RadioButton, "Appearance");
        let closes = harness.query_all_by_label("Close").count();
        assert_eq!(
            closes,
            usize::from(embedded),
            "embedded: {embedded} — a window of its own is closed by its own control"
        );
    }
}

#[test]
fn the_hash_dialogue_s_secondary_line_ends_before_the_close_button() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let name = format!("{}.bin", "a-very-long-file-name-".repeat(6));
    std::fs::write(dir.path().join(&name), vec![7_u8; 4096]).unwrap_or_else(|e| panic!("{e}"));
    let mut harness = window(dir.path());
    let pane = harness.state().app().model().focused;
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, tab)| tab.listing.rows().find(|e| e.name == name).map(|e| e.id))
        .unwrap_or_else(|| panic!("the file is listed"));
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    harness.step();
    harness
        .state_mut()
        .queue(Intent::Action(ActionId("core.hash")), pane);
    steps(&mut harness);
    assert!(harness.state().hash_dialog().is_open());

    // The line carries the whole text for a screen reader and as its
    // tooltip, and is drawn only up to the close button.
    let line = harness.get_by_label(&format!("{name} · 4.0 KiB")).rect();
    let title = harness.get_by_label("File hashes").rect();
    let close = harness
        .get_all_by_label("Close")
        .map(|node| node.rect())
        .find(|rect| {
            (rect.center().y - (title.center().y + 12.0)).abs() < 30.0 && rect.left() > line.left()
        })
        .unwrap_or_else(|| panic!("the dialogue's close button"));
    assert!(
        line.right() <= close.left() - 8.0,
        "the line {line:?} ends before the button {close:?}"
    );
    assert!(line.width() > 200.0, "and has the room up to it");
}

#[test]
fn a_console_tab_is_titled_console_not_after_its_folder() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let folder = dir.path().join("projects");
    std::fs::create_dir_all(&folder).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(ext_action_terminal::TerminalExtension::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&folder)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 800.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    // The workspace tabs of the title row, by their names.
    let tabs = |harness: &Harness<'_, RickUi>, name: &str| {
        harness
            .query_all_by_role_and_label(Role::Button, name)
            .filter(|node| node.rect().top() < 44.0 && node.rect().width() > 100.0)
            .count()
    };
    assert_eq!(tabs(&harness, "projects"), 1, "the folder's own tab");
    assert_eq!(tabs(&harness, "Console"), 0);

    // A console opened in the folder gets a tab of its own, and that tab
    // says what it is.
    let pane = harness.state().app().model().focused;
    harness
        .state_mut()
        .queue(Intent::Action(ActionId("terminal.console")), pane);
    let deadline = Instant::now() + Duration::from_secs(20);
    while tabs(&harness, "Console") == 0 {
        harness.step();
        assert!(Instant::now() < deadline, "no console tab");
        std::thread::sleep(Duration::from_millis(5));
    }
    steps(&mut harness);
    assert_eq!(tabs(&harness, "Console"), 1);
    assert_eq!(
        tabs(&harness, "projects"),
        1,
        "the console's tab is not named after the folder it was opened in"
    );
}

#[test]
fn a_failed_job_s_error_unfolds_in_the_drawer_and_stays_as_a_notice() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = window(dir.path());
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(Intent::Action(SYNC), pane);
    let deadline = Instant::now() + Duration::from_secs(20);
    while harness.state().toasts().is_empty() {
        harness.step();
        assert!(Instant::now() < deadline, "the job did not fail");
        std::thread::sleep(Duration::from_millis(5));
    }
    steps(&mut harness);
    // The error as the job reports it.
    let error = Error::Io(std::io::Error::other(LONG_ERROR)).to_string();
    assert!(error.ends_with(LONG_ERROR));
    let whole = format!("Sync photos: {error}");
    let row = format!("Sync photos · {error}");

    // Where the user acted: a notice with the job's name and the whole
    // error, wrapped over several lines and not cut.
    assert_eq!(harness.state().toasts(), [(Level::Error, whole.clone())]);
    let notice = harness.get_by_label(&whole).rect();
    assert!(notice.height() > 40.0, "wrapped: {notice:?}");
    assert!(notice.width() <= 341.0, "{notice:?}");
    // At the top right of the window, under the title row and not on it.
    let window = harness.state().app().model().focused_window;
    let content = harness
        .state()
        .content_of(window)
        .unwrap_or_else(|| panic!("the window's content"));
    let under = content.top() + theme::metrics(&harness.ctx).titlebar_height;
    assert!(
        notice.right() > content.right() - 80.0 && notice.right() < content.right(),
        "at the right: {notice:?} in {content:?}"
    );
    assert!(
        notice.top() > under && notice.top() < under + 40.0,
        "under the title row at {under}: {notice:?}"
    );

    // It stays: an error is not a notice that leaves after five seconds.
    for _ in 0..40 {
        harness.step();
    }
    std::thread::sleep(Duration::from_millis(50));
    steps(&mut harness);
    assert_eq!(harness.state().toasts().len(), 1);

    // The drawer's row: one line at rest, the whole text behind a click.
    let at_rest = harness.get_by_role_and_label(Role::Button, &row).rect();
    assert!(at_rest.height() <= 28.5, "{at_rest:?}");
    harness.get_by_role_and_label(Role::Button, &row).click();
    steps(&mut harness);
    let unfolded = harness.get_by_role_and_label(Role::Button, &row).rect();
    assert!(
        unfolded.height() > at_rest.height() + 14.0,
        "the row grew to hold the text: {unfolded:?}"
    );
    // A second click folds it again.
    harness.get_by_role_and_label(Role::Button, &row).click();
    steps(&mut harness);
    let folded = harness.get_by_role_and_label(Role::Button, &row).rect();
    assert!(folded.height() <= 28.5, "{folded:?}");

    // The notice goes with its close button, and only then.
    let dismiss = harness
        .get_all_by_label("Dismiss")
        .map(|node| node.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap_or_else(|| panic!("the notice's close button"));
    assert!(dismiss.top() < 140.0, "{dismiss:?}");
    let at = dismiss.center();
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    steps(&mut harness);
    assert_eq!(harness.state().toasts(), Vec::new(), "the notice is gone");
    // The drawer still has the failed job until it is dismissed there.
    harness.get_by_role_and_label(Role::Button, &row);
}
