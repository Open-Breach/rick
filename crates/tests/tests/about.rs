//! Acceptance (issue 101, the About dialog): the menu's request opens the
//! modal with the hero, the build line, the two buttons, the three cards
//! and the footer; *Copy details* puts the details text on the clipboard,
//! the same text `rick --version` prints; *What's new* and *Licence* open
//! the read-only text pages and the back button returns; *Loaded plugins*
//! opens the Extensions screen; the close button closes. The Third-party
//! licences page lists the bundled runtimes, the crates and a plugin's
//! entries, filters by name, opens one row at a time and exports the whole
//! list through the save dialog.

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
            text: "MIT License\n\nCopyright (c) 2018 Emil Ernerfeldt\n\nPermission is hereby granted, free of charge, to any person".to_owned(),
        }]),
        plugin_licences: Vec::new(),
    }
}

/// A build with two crates and a plugin's licences, for the page.
fn build_with_licences() -> Build {
    let mut build = build();
    build.third_party = Some(vec![
        about::Component {
            name: "serde".to_owned(),
            version: "1.0.228".to_owned(),
            license: "Apache-2.0".to_owned(),
            copyright: vec!["David Tolnay <dtolnay@gmail.com>".to_owned()],
            repository: "https://github.com/serde-rs/serde".to_owned(),
            text: "Apache License\nVersion 2.0, January 2004".to_owned(),
        },
        build
            .third_party
            .as_ref()
            .and_then(|l| l.first().cloned())
            .unwrap_or_else(|| panic!("the egui entry")),
    ]);
    build.plugin_licences = vec![about::PluginLicences {
        name: "Probe".to_owned(),
        components: vec![about::Component {
            name: "probe-dep".to_owned(),
            version: "2.0.0".to_owned(),
            license: "ISC".to_owned(),
            copyright: vec!["Someone".to_owned()],
            repository: "https://example.org/probe-dep".to_owned(),
            text: "ISC License\n\nPermission to use, copy, modify".to_owned(),
        }],
    }];
    build
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
    harness_with(start, build())
}

fn harness_with(start: &Path, build: Build) -> (Harness<'static, RickUi>, Arc<Fake>) {
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
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx).with_build(build));
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

/// The dialog's search field: the text input inside the modal, apart from
/// the title row's search box and the footer's filter field.
fn licences_search<'h>(harness: &'h Harness<'_, RickUi>) -> egui_kittest::Node<'h> {
    let title = harness.get_by_label("Third-party licences").rect();
    harness
        .get_all_by_role(Role::TextInput)
        .find(|node| {
            let rect = node.rect();
            rect.top() > title.bottom() && rect.top() < title.bottom() + 60.0
        })
        .unwrap_or_else(|| panic!("no search field under the title"))
}

#[test]
fn the_third_party_page_lists_filters_expands_and_exports() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, fake) = harness_with(dir.path(), build_with_licences());
    open(&mut harness);
    // Two crates, eight bundled runtimes and the plugin's one entry.
    assert_eq!(harness.state().about().component_count(), Some(11));
    click_button(&mut harness, "Third-party licences");
    assert!(harness.state().about().licences_page().is_some());
    harness.get_by_label("Third-party licences");
    for section in ["BUNDLED RUNTIMES", "RUST CRATES", "PROBE"] {
        harness.get_by_label(section);
    }
    for row in [
        "pdfium chromium/8138",
        "Phosphor Icons 2.1",
        "egui 0.36.2",
        "serde 1.0.228",
        "probe-dep 2.0.0",
    ] {
        harness.get_by_role_and_label(Role::Button, row);
    }
    harness.get_by_label_contains("11 components, 7 licence types");

    // A row opens in place with its copyright line and text; another row
    // replaces it (one at the top, so the open text does not scroll the
    // second row out of the list); the same click closes it.
    assert!(
        harness
            .query_by_label_contains("Permission is hereby granted")
            .is_none()
    );
    click_button(&mut harness, "egui 0.36.2");
    assert!(
        harness
            .state()
            .about()
            .licences_page()
            .is_some_and(|p| p.open_row().is_some())
    );
    harness.get_by_label("Copyright (c) 2018 Emil Ernerfeldt");
    harness.get_by_label_contains("Permission is hereby granted");
    click_button(&mut harness, "pdfium chromium/8138");
    assert!(
        harness
            .query_by_label_contains("Permission is hereby granted")
            .is_none()
    );
    harness.get_by_label("Copyright 2014 The PDFium Authors");
    harness.get_by_label_contains("Redistribution and use in source and binary forms");
    click_button(&mut harness, "pdfium chromium/8138");
    assert!(
        harness
            .state()
            .about()
            .licences_page()
            .is_some_and(|p| p.open_row().is_none())
    );
    assert!(
        harness
            .query_by_label_contains("Redistribution and use")
            .is_none()
    );

    // The search narrows the rows to the names that contain the text.
    let search = licences_search(&harness);
    search.focus();
    search.type_text("pdf");
    for _ in 0..3 {
        harness.step();
    }
    harness.get_by_role_and_label(Role::Button, "pdfium chromium/8138");
    assert!(harness.query_by_label("egui 0.36.2").is_none());
    assert!(harness.query_by_label("RUST CRATES").is_none());
    assert_eq!(
        harness
            .state()
            .about()
            .licences_page()
            .map(|p| p.search.as_str()),
        Some("pdf")
    );

    // Export writes the whole list, filters or not, where the save dialog
    // says, and a toast names the file.
    let out = dir.path().join("licences.txt");
    fake.answer_save(Some(out.clone()));
    click_button(&mut harness, "Export…");
    let deadline = Instant::now() + Duration::from_secs(20);
    while harness
        .state()
        .about()
        .licences_page()
        .is_some_and(|p| p.exporting())
    {
        assert!(Instant::now() < deadline, "the export did not finish");
        std::thread::sleep(Duration::from_millis(20));
        harness.step();
    }
    let text = std::fs::read_to_string(&out).unwrap_or_else(|e| panic!("{e}"));
    assert!(text.starts_with("Third-party licences of Rick 0.3.0-dev\n"));
    for needle in [
        "== Bundled runtimes ==",
        "== Rust crates ==",
        "== Probe ==",
        "--- pdfium chromium/8138 (BSD-3-Clause) ---",
        "--- egui 0.36.2 (MIT) ---",
        "--- serde 1.0.228 (Apache-2.0) ---",
        "--- probe-dep 2.0.0 (ISC) ---",
        "Permission is hereby granted, free of charge",
        "GNU LESSER GENERAL PUBLIC LICENSE",
        "UBUNTU FONT LICENCE",
    ] {
        assert!(text.contains(needle), "the export lacks {needle}");
    }
    assert!(
        harness
            .state()
            .toasts()
            .iter()
            .any(|(_, t)| t.starts_with("Exported to ")),
        "{:?}",
        harness.state().toasts()
    );

    // Back returns to the About page.
    click_button(&mut harness, "Back");
    assert!(harness.state().about().licences_page().is_none());
    assert!(harness.state().about().is_open());
}
