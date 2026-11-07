//! Acceptance (side panel): the panel shows its six sections by default,
//! Mod+B hides and shows it, Home navigates, pins persist across a restart
//! and a folded section stays folded (through the session file, where the
//! panel's state lives since issue B).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::{EguiHost, RickUi, icons};

/// A harness whose settings live in `config`, so a second one can start
/// from what the first one saved.
fn harness(start: &Path, config: &Path) -> (Harness<'static, RickUi>, Arc<Fake>) {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(config.to_path_buf()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 720.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    (harness, fake)
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

fn location(harness: &Harness<'_, RickUi>) -> PathBuf {
    let app = harness.state().app();
    let pane = app.model().focused;
    app.model()
        .active_tab(pane)
        .and_then(|(_, t)| platform::paths::to_path(&t.location.path).ok())
        .unwrap_or_else(|| panic!("no location"))
}

/// Waits until the session file contains `needle`: the panel's state is
/// session state, written within a second of the change.
fn session_contains(harness: &mut Harness<'_, RickUi>, needle: &str) -> String {
    let path = harness.state().app().session_path().to_path_buf();
    wait_for(harness, &path, needle)
}

/// Waits until the settings file contains `needle`.
fn settings_contain(harness: &mut Harness<'_, RickUi>, needle: &str) -> String {
    let path = harness.state().app().settings_path().to_path_buf();
    wait_for(harness, &path, needle)
}

fn wait_for(harness: &mut Harness<'_, RickUi>, path: &Path, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = fs::read_to_string(path).unwrap_or_default();
        if text.contains(needle) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "{} never contained {needle:?}:\n{text}",
            path.display()
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_panel_shows_its_sections_and_mod_b_hides_and_shows_it() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _fake) = harness(dir.path(), &dir.path().join("config"));
    for title in ["Home", "Recent", "Network", "Trash", "Pinned", "Tree"] {
        harness.get_by_label(title);
    }
    assert_eq!(
        harness
            .query_all_by_label_contains("Coming in a later issue")
            .count(),
        0,
        "every section has its contents now"
    );
    harness.get_by_label_contains("The trash is empty");
    harness.get_by_label_contains("No removable media");

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Pinned").is_none(), "hidden");
    session_contains(&mut harness, "visible = false");

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    harness.step();
    harness.step();
    harness.get_by_label("Pinned");
    session_contains(&mut harness, "visible = true");
}

#[test]
fn home_navigates_the_focused_pane() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, fake) = harness(dir.path(), &dir.path().join("config"));
    assert_eq!(location(&harness), dir.path());
    harness
        .get_by_label(&format!("{} Home", icons::HOME))
        .click();
    harness.step();
    settle(&mut harness);
    assert_eq!(location(&harness), fake.home_dir());
}

#[test]
fn pins_survive_a_restart_and_unpin_removes_them() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let work = dir.path().join("project");
    fs::create_dir(&work).unwrap_or_else(|e| panic!("{e}"));
    let config = dir.path().join("config");
    {
        let (mut harness, _fake) = harness(&work, &config);
        assert!(
            harness
                .query_by_label(&format!("{} project", icons::PIN))
                .is_none()
        );
        harness.get_by_label("Pin current folder").click();
        harness.step();
        harness.step();
        harness.get_by_label(&format!("{} project", icons::PIN));
        settings_contain(&mut harness, "project");
    }
    // A fresh start from the same settings file has the pin.
    let (mut harness, fake) = harness(dir.path(), &config);
    harness.get_by_label(&format!("{} project", icons::PIN));
    assert_eq!(location(&harness), dir.path());
    harness
        .get_by_label(&format!("{} project", icons::PIN))
        .click();
    harness.step();
    settle(&mut harness);
    assert_eq!(location(&harness), work);
    // Away from the pinned folder, Home still works and the pin unpins.
    harness
        .get_by_label(&format!("{} Home", icons::HOME))
        .click();
    harness.step();
    settle(&mut harness);
    assert_eq!(location(&harness), fake.home_dir());
    harness.get_by_label(icons::UNPIN).click();
    harness.step();
    harness.step();
    assert!(
        harness
            .query_by_label(&format!("{} project", icons::PIN))
            .is_none()
    );
    let text = settings_contain(&mut harness, "pinned = []");
    assert!(!text.contains("project"), "{text}");
}

#[test]
fn a_folded_section_stays_folded_across_a_restart() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let config = dir.path().join("config");
    {
        let (mut harness, _fake) = harness(dir.path(), &config);
        harness.get_by_label_contains("Nothing pinned yet");
        harness.get_by_label("Pinned").click();
        harness.step();
        harness.step();
        assert!(
            harness
                .query_by_label_contains("Nothing pinned yet")
                .is_none(),
            "folded"
        );
        session_contains(&mut harness, "folded = [\"pinned\"]");
    }
    let (harness, _fake) = harness(dir.path(), &config);
    harness.get_by_label("Pinned");
    assert!(
        harness
            .query_by_label_contains("Nothing pinned yet")
            .is_none(),
        "still folded after a restart"
    );
    harness.get_by_label("Home");
    harness.get_by_label(&format!("{} Home", icons::HOME));
}
