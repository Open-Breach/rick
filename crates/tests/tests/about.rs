//! Acceptance (issue 101, the About dialog): the menu's request opens the
//! modal with the hero, the build line, the two buttons, the three cards
//! and the footer; *Copy details* puts the details text on the clipboard,
//! the same text `rick --version` prints; *What's new* and *Licence* open
//! the read-only text pages and the back button returns; *Loaded plugins*
//! opens the Extensions screen; the close button closes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::about::{self, Build};
use ui_egui::{EguiHost, RickUi};

fn build() -> Build {
    Build {
        version: "0.3.0-dev".to_owned(),
        commit: "a1b2c3d".to_owned(),
        target: "x86_64-unknown-linux-gnu".to_owned(),
        date: "2026-11-09".to_owned(),
        toolchain: "rustc 1.97.0 (test)".to_owned(),
        contributors: "Claude\nSerge\nAda Lovelace\n",
        roles: "- Ada Lovelace — Analytical engines\n",
        plugins_dir: Some(PathBuf::from("/opt/rick/plugins")),
        third_party: Some(vec![about::Component {
            name: "egui".to_owned(),
            version: "0.36.2".to_owned(),
            license: "MIT".to_owned(),
            copyright: vec!["Emil Ernerfeldt".to_owned()],
            repository: "https://github.com/emilk/egui".to_owned(),
            text: "MIT License".to_owned(),
        }]),
    }
}

fn plugins() -> Vec<app::Plugin> {
    vec![
        app::Plugin {
            id: "git".to_owned(),
            name: "Git".to_owned(),
            version: "0.3.0-dev".to_owned(),
            points: ext::Points::empty(),
            previews: Vec::new(),
            status: app::PluginStatus::Loaded,
            path: PathBuf::from("/opt/rick/plugins/git"),
        },
        app::Plugin {
            id: "probe".to_owned(),
            name: "Probe".to_owned(),
            version: "1.0.0".to_owned(),
            points: ext::Points::empty(),
            previews: Vec::new(),
            status: app::PluginStatus::Disabled,
            path: PathBuf::from("/home/me/.config/rick/plugins/probe"),
        },
    ]
}

fn harness(start: &Path) -> (Harness<'static, RickUi>, Arc<Fake>) {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let mut app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    app.set_plugins(plugins());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 1000.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx).with_build(build()));
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

/// Clicks at `at`, then runs two more frames: a page change re-centres
/// the modal on the frame after it, as any size change of an egui area.
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
    harness.step();
}

fn click_button(harness: &mut Harness<'_, RickUi>, label: &str) {
    let at = harness
        .get_by_role_and_label(Role::Button, label)
        .rect()
        .center();
    click_at(harness, at);
}

fn open(harness: &mut Harness<'_, RickUi>) {
    harness.state_mut().open_about();
    for _ in 0..3 {
        harness.step();
    }
    assert!(harness.state().about().is_open());
}

#[test]
fn the_dialog_shows_the_hero_the_cards_and_the_footer() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _fake) = harness(dir.path());
    assert!(!harness.state().about().is_open());
    open(&mut harness);
    // The hero: name, tagline, version pill, build line.
    harness.get_by_label("a god-like file manager");
    harness.get_by_label("0.3.0-dev");
    let build_line = about::build_line(&build());
    assert!(build_line.contains(&format!("plugin ABI {}", ext::API_VERSION)));
    harness.get_by_label(&build_line);
    for button in ["Copy details", "What's new", "Close"] {
        harness.get_by_role_and_label(Role::Button, button);
    }
    // The cards: three links, three licence rows, the credits.
    for row in [
        "Website",
        "Source code",
        "Report an issue",
        "Licence",
        "Third-party licences",
        "Loaded plugins",
    ] {
        harness.get_by_role_and_label(Role::Button, row);
    }
    harness.get_by_label("Credits");
    harness.get_by_label("Serge, Specification, design and direction");
    harness.get_by_label_contains("Claude (Anthropic)");
    harness.get_by_label("Ada Lovelace, Analytical engines");
    harness.get_by_label_contains("use with care");
    harness.get_by_label("© 2026 Open-Breach contributors");

    // The close button closes.
    click_button(&mut harness, "Close");
    assert!(!harness.state().about().is_open());
}

#[test]
fn copy_details_puts_the_version_text_on_the_clipboard() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, fake) = harness(dir.path());
    open(&mut harness);
    click_button(&mut harness, "Copy details");
    let expected = about::details(&build(), &plugins());
    assert_eq!(fake.clipboard_text().as_deref(), Some(expected.as_str()));
    let lines: Vec<&str> = expected.lines().collect();
    assert_eq!(lines[0], "Rick 0.3.0-dev");
    assert_eq!(lines[1], about::build_line(&build()));
    assert_eq!(lines[2], "rustc 1.97.0 (test)");
    assert_ne!(lines[3], "", "the OS line");
    assert_eq!(lines[4], "git 0.3.0-dev loaded");
    assert_eq!(lines[5], "probe 1.0.0 disabled");
    assert!(
        harness
            .state()
            .toasts()
            .iter()
            .any(|(_, text)| text == "Details copied"),
        "a toast says so"
    );
    // The issue URL carries the same text.
    assert!(about::issue_url(&expected).contains("body=Rick%200%2E3%2E0%2Ddev%0A"));
}

#[test]
fn whats_new_and_licence_open_the_text_pages_and_back_returns() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _fake) = harness(dir.path());
    open(&mut harness);
    click_button(&mut harness, "What's new");
    assert_eq!(
        harness.state().about().text_page(),
        Some("What's new in 0.3.0-dev")
    );
    harness.get_by_label("What's new in 0.3.0-dev");
    harness.get_by_label_contains("## [Unreleased]");
    click_button(&mut harness, "Back");
    assert_eq!(harness.state().about().text_page(), None);
    assert!(harness.state().about().is_open());

    click_button(&mut harness, "Licence");
    assert_eq!(harness.state().about().text_page(), Some("Licence"));
    harness.get_by_label_contains("Apache License");
    // Escape on a text page goes back; on the About page it closes.
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert_eq!(harness.state().about().text_page(), None);
    assert!(harness.state().about().is_open());
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert!(!harness.state().about().is_open());
}

#[test]
fn the_licence_card_counts_and_loaded_plugins_opens_the_extensions_screen() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _fake) = harness(dir.path());
    open(&mut harness);
    // One crate and the eight bundled runtimes; two plugins, one from
    // outside the release's plugins folder.
    assert_eq!(harness.state().about().component_count(), Some(9));
    harness.get_by_label_contains("9 components");
    harness.get_by_label_contains("2 plugins · 1 third-party");
    assert!(!harness.state().extensions_open());
    click_button(&mut harness, "Loaded plugins");
    assert!(harness.state().extensions_open());
    assert!(!harness.state().about().is_open());
}

/// Without the generated file the row says so and shows no count.
#[test]
fn a_build_without_the_generated_list_says_so() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 1000.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    open(&mut harness);
    assert_eq!(harness.state().about().component_count(), None);
    harness.get_by_label_contains("not available in this build");
}
