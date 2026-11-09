//! Acceptance (issue 60, install dialogue): the dialogue's header, the
//! *Install for* control, the *WHAT RICK WILL DO* card, the *Advanced
//! options* disclosure, the footer's note and buttons, and the warning of
//! a missing version.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::ActionId;
use ext_installer::fixtures::Fixture;
use model::SelectionChange;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::{EguiHost, Intent, RickUi};

fn harness(start: &Path) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(ext_installer::InstallerExtension::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
}

/// Runs frames until the app has nothing in flight and a quiet moment
/// passed.
fn settle(harness: &mut Harness<'_, RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for _ in 0..3 {
            harness.step();
        }
        if !harness.state().app().is_busy() {
            std::thread::sleep(Duration::from_millis(50));
            harness.step();
            if !harness.state().app().is_busy() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "did not settle");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Steps frames until `done` holds, within `limit`.
fn step_until(
    harness: &mut Harness<'_, RickUi>,
    limit: Duration,
    mut done: impl FnMut(&Harness<'_, RickUi>) -> bool,
) -> bool {
    let deadline = Instant::now() + limit;
    while !done(harness) {
        if Instant::now() > deadline {
            return false;
        }
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

/// Selects the archive and runs *Install as application…*; the dialogue
/// opens once the archive is read, its header titled `name`.
fn open_dialog(harness: &mut Harness<'_, RickUi>, archive: &str, name: &str) {
    let pane = harness.state().app().model().focused;
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| t.rows().find(|e| e.name == archive).map(|e| e.id))
        .unwrap_or_else(|| panic!("no row {archive}"));
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    settle(harness);
    harness
        .state_mut()
        .queue(Intent::Action(ActionId("installer.install")), pane);
    assert!(
        step_until(harness, Duration::from_secs(15), |h| {
            h.query_by_label("Install for").is_some()
        }),
        "the dialogue opens"
    );
    harness.get_by_label(name);
}

/// Scrolls the hovered area to its end: a touch-phased wheel, which egui
/// applies at once instead of smoothing it over frames.
fn scroll_to_end(harness: &mut Harness<'_, RickUi>) {
    for (phase, delta) in [
        (egui::TouchPhase::Start, 0.0),
        (egui::TouchPhase::Move, -4000.0),
        (egui::TouchPhase::End, 0.0),
    ] {
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, delta),
            modifiers: egui::Modifiers::default(),
            phase,
        });
        harness.step();
    }
    harness.step();
}

fn click_at(harness: &mut Harness<'_, RickUi>, at: egui::Pos2) {
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
    harness.step();
}

#[test]
fn the_dialogue_has_the_header_the_cards_the_disclosure_and_the_footer() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    Fixture::GuiWithDesktop
        .write(dir.path())
        .unwrap_or_else(|e| panic!("{e}"));
    let archive = Fixture::GuiWithDesktop.file_name();
    let mut harness = harness(dir.path());
    open_dialog(&mut harness, archive, "Fixture GUI");
    // The header's secondary line is the version; the fields, the
    // segmented control with *Just me* chosen, the caps heading over the
    // effects and the first effect.
    harness.get_by_label("1.5.0");
    for label in ["Name", "App id", "Version", "Install for", "Kind", "Icon"] {
        harness.get_by_label(label);
    }
    harness.get_by_role_and_label(egui::accesskit::Role::RadioButton, "Just me");
    harness.get_by_role_and_label(egui::accesskit::Role::RadioButton, "All users");
    harness.get_by_label("WHAT RICK WILL DO");
    harness.get_by_label_contains("App files ·");
    // The footer: the archive's name, Cancel and the primary.
    harness.get_by_label(&format!("Archive: {archive}"));
    harness.get_by_label("Cancel");
    harness.get_by_role_and_label(egui::accesskit::Role::Button, "Install");
    // Advanced options is collapsed: its rows appear on a click. The body
    // scrolls, so the card is brought into view first.
    assert!(harness.query_by_label("Category (GUI)").is_none());
    let body = harness.get_by_label("Install for").rect().center();
    harness.event(egui::Event::PointerMoved(body));
    harness.step();
    scroll_to_end(&mut harness);
    let advanced = harness
        .get_by_role_and_label(egui::accesskit::Role::CheckBox, "Advanced options")
        .rect()
        .center();
    click_at(&mut harness, advanced);
    harness.get_by_label("Category (GUI)");
    harness.get_by_label("Expected SHA-256");
    // Escape cancels: the dialogue goes and nothing was installed.
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Install for").is_none());
}

#[test]
fn a_missing_version_warns_under_the_field() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    Fixture::Tarbomb
        .write(dir.path())
        .unwrap_or_else(|e| panic!("{e}"));
    let archive = Fixture::Tarbomb.file_name();
    let mut harness = harness(dir.path());
    open_dialog(&mut harness, archive, "tarbomb");
    harness.get_by_label("Couldn't read the version from the archive — please enter it.");
    harness.get_by_label("?");
    harness.key_press(egui::Key::Escape);
    harness.step();
}
