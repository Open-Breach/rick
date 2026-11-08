//! Acceptance (burger menu): the actions an extension puts in the
//! application menu ([`ext::Action::app_menu`]) are listed after the
//! Layouts submenu under one submenu per extension and run on the focused
//! pane. With the installer loaded, Installer ▸ Installed Apps… takes the
//! pane to `rick://apps`; off Linux, where the installer's actions hide,
//! the menu has no Installer submenu.

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
    harness.get_by_label(ui_egui::icons::MENU).click();
    harness.step();
    harness.step();
    // The menu is open: its own entries are there.
    harness.get_by_label("Settings…");
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
