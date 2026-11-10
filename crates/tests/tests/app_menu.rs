//! Acceptance (burger menu): the actions an extension puts in the
//! application menu ([`ext::Action::app_menu`]) are listed after the
//! Layouts submenu under one submenu per extension and run on the focused
//! pane. With the installer loaded, Installer ▸ Installed Apps… takes the
//! pane to `rick://apps`; off Linux, where the installer's actions hide,
//! the menu has no Installer submenu.
//!
//! Acceptance (issue 139, item 2, SPEC.md *Menu*): the menu button has a
//! name; its card opens under it, left edges aligned, 6 px below, at
//! least 280 px wide with 32 px entries; the entries are New window,
//! Layouts, the extension submenus, Settings, Keyboard shortcuts,
//! Extensions and About Rick, none ending in "…" and none a theme switch;
//! a hint is the keymap's chord as the platform spells it and nothing
//! when the entry is unbound.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::{EguiHost, RickUi};

fn harness(start: &Path, extensions: Vec<Arc<dyn ext::Extension>>) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions,
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 720.0))
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

#[test]
fn the_burger_menu_lists_installed_apps_under_installer() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("readme.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(
        dir.path(),
        vec![Arc::new(ext_installer::InstallerExtension::new())],
    );
    harness.get_by_label("Menu").click();
    harness.step();
    harness.step();
    // The menu is open: its own entries are there.
    harness.get_by_label("Settings");
    if !ext_installer::supported() {
        assert!(
            harness.query_by_label_contains("Installer").is_none(),
            "the installer's actions hide off Linux, and so does its submenu"
        );
        return;
    }
    // The submenu button's label carries egui's arrow: "Installer ⏵".
    harness.get_by_label_contains("Installer").hover();
    harness.step();
    harness.step();
    harness.get_by_label("Installed Apps…").click();
    harness.step();
    settle(&mut harness);
    let pane = harness.state().app().model().focused;
    let location = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map(|(_, tab)| tab.location.clone());
    assert_eq!(location, Some(ext_installer::location()));
    assert!(
        harness.query_by_label("Installed Apps…").is_none(),
        "the menu closed"
    );
}

#[test]
fn the_application_menu_opens_under_its_button_with_its_three_groups() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("readme.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(dir.path(), Vec::new());
    // The icon-only button is named for a screen reader.
    let button = harness.get_by_label("Menu").rect();
    harness.get_by_label("Menu").click();
    harness.step();
    harness.step();

    // The entries, top to bottom, in their three groups.
    let labels = [
        "New window",
        "Layouts",
        "Settings",
        "Keyboard shortcuts",
        "Extensions",
        "About Rick",
    ];
    let rows: Vec<egui::Rect> = labels
        .iter()
        .map(|label| harness.get_by_label_contains(label).rect())
        .collect();
    for pair in rows.windows(2) {
        assert!(pair[0].top() < pair[1].top(), "the entries are in order");
    }
    for (label, row) in labels.iter().zip(&rows) {
        assert!(
            (row.height() - 32.0).abs() < 0.5,
            "{label} is 32 px tall, not {}",
            row.height()
        );
        // The card is 280 px at least: 7 px of padding on each side.
        assert!(
            row.width() >= 280.0 - 14.0 - 0.5,
            "{label}: {}",
            row.width()
        );
    }
    // A hairline with 4 px of air on each side parts the groups: 9 px.
    assert!((rows[2].top() - rows[1].bottom() - 9.0).abs() < 0.5);
    assert!((rows[5].top() - rows[4].bottom() - 9.0).abs() < 0.5);
    assert!((rows[1].top() - rows[0].bottom()).abs() < 0.5);

    // Under the button: left edges aligned, 6 px below it.
    let first = rows[0];
    assert!(
        (first.left() - 7.0 - button.left()).abs() < 1.0,
        "the card's left edge is the button's: {} and {}",
        first.left() - 7.0,
        button.left()
    );
    assert!(
        (first.top() - 7.0 - button.bottom() - 6.0).abs() < 1.0,
        "the card opens 6 px under the button: {}",
        first.top() - 7.0 - button.bottom()
    );

    // No entry ends in an ellipsis, and the theme is not switched here.
    for label in ["Settings…", "Keyboard shortcuts…", "Extensions…"] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }
    for label in ["Theme", "Light", "Dark"] {
        assert!(harness.query_by_label_contains(label).is_none(), "{label}");
    }

    // An entry runs what it names and closes the menu.
    harness.get_by_label("Keyboard shortcuts").click();
    harness.step();
    harness.step();
    assert!(harness.query_by_label("About Rick").is_none(), "closed");
    assert!(harness.state().shortcuts_open(), "the screen opened");
}

#[test]
fn a_menu_hint_is_the_keymap_s_chord_and_nothing_when_unbound() {
    use ui_egui::keys::{Intent, KeyMap};
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(dir.path(), Vec::new());
    let app = harness.state().app();
    let keys = KeyMap::new(app.registry(), &app.model().settings);
    // Bound by default: spelled with the platform's key, not "Mod".
    let hint = keys.text_for(Intent::NewWindow);
    assert_eq!(hint, Some(format!("{}+N", platform::MOD_KEY)));
    assert_eq!(
        keys.text_for(Intent::Settings),
        Some(format!("{}+,", platform::MOD_KEY))
    );
    // Unbound by default: no hint.
    assert_eq!(keys.text_for(Intent::Shortcuts), None);
    assert_eq!(keys.text_for(Intent::Extensions), None);

    // The hint follows a rebinding, and goes when the entry is unbound.
    for (binding, expected) in [
        (Some("Alt+F9".to_owned()), Some("Alt+F9".to_owned())),
        (Some(String::new()), None),
    ] {
        harness.state_mut().app_mut().handle(
            app::Command::Settings(model::SettingsChange::Shortcut {
                action: "window.new".to_owned(),
                binding,
            }),
            &mut Vec::new(),
        );
        let app = harness.state().app();
        let keys = KeyMap::new(app.registry(), &app.model().settings);
        assert_eq!(keys.text_for(Intent::NewWindow), expected);
    }
}
