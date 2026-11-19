//! Acceptance (issue 251, item 1, specs/plugin-distribution.md §6): the
//! registry client behind the window, against a registry of the test's
//! own — a server on this machine that counts what it is asked, and a key
//! made here. No test asks get.openbreach.dev.
//!
//! Rick reads the cached plugin list once its first frame is drawn, and
//! asks nobody then. With *Look for plugins and updates online* off and
//! nothing cached, starting Rick and selecting files makes no request;
//! with it on, the background check a minute after the start makes one
//! for each of the two files, and the next start, the same day, none.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use platform::{ConfigDir, Fake};
use plugins_registry::fixtures::{INDEX_PATH, Server, Signer};
use serde_json::json;
use tests::local;
use ui_egui::{EguiHost, RickUi};

/// The Rick the tests ask as.
const RICK: &str = "0.6.0";

/// An index with Git for [`RICK`] on this platform.
fn index() -> serde_json::Value {
    json!({
        "schema": 1,
        "generated": "2026-11-19T02:00:00Z",
        "apps": { "rick": { "plugins": [{
            "id": "git",
            "name": "Git",
            "description": "Status badges and the Git actions of the context menu.",
            "publisher": "Open-Breach",
            "meta": { "points": ["decorator", "action"], "triggers": { "markers": [".git"] } },
            "builds": [{
                "version": RICK,
                "app_version": RICK,
                "api_version": ext::API_VERSION,
                "platform": plugins_registry::platform(),
                "url": "https://example.invalid/git.rkp",
                "size": 1024,
                "sha256": "aa".repeat(32),
            }],
        }] } },
    })
}

/// A window of a Rick of version [`RICK`] on `fake`'s config directory,
/// whose `rick.toml` names `server` and `signer` and has the switch as
/// `check` says, showing `folder`. Its first frames are drawn.
fn window(
    fake: &Arc<Fake>,
    server: &Server,
    signer: &Signer,
    check: bool,
    folder: &Path,
) -> Harness<'static, RickUi> {
    let config = fake.config_dir();
    fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
    let settings = format!(
        "[registry]\ncheck = {check}\nurl = \"{}\"\npublic_key = \"{}\"\n",
        server.index_url(),
        signer.public_key()
    );
    fs::write(config.join(app::SETTINGS_FILE), settings).unwrap_or_else(|e| panic!("{e}"));
    let host = Arc::new(EguiHost::new());
    let mut app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(config),
        start: Some(local(folder)),
    });
    app.set_rick_version(RICK);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 820.0))
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

/// A folder with a few files to select.
fn folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    dir
}

#[test]
fn with_the_switch_off_and_nothing_cached_starting_and_selecting_asks_nobody() {
    let (server, signer) = (Server::start(), Signer::new(1));
    server.publish(&signer, &index());
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let dir = folder();
    let mut harness = window(&fake, &server, &signer, false, dir.path());

    // Select a file, then all of them.
    let pane = harness.state().app().model().focused;
    let first: Vec<types::EntryId> = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, tab)| tab.listing.rows().next().map(|entry| entry.id))
        .into_iter()
        .collect();
    assert_eq!(first.len(), 1, "the folder is listed");
    for change in [
        model::SelectionChange::Set(first),
        model::SelectionChange::All,
    ] {
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::Select { pane, change }, &mut Vec::new());
        settle(&mut harness);
    }
    let app = harness.state().app();
    let selected = app
        .model()
        .active_tab(pane)
        .map_or(0, |(_, tab)| tab.selected().count());
    assert_eq!(selected, 3, "the files are selected");

    let registry = app.plugin_registry();
    assert!(registry.set_up, "the registry is set up, and still");
    assert_eq!(registry.index, None);
    assert_eq!(server.requests(), [], "not one request was made");
}

#[test]
fn the_first_frame_asks_nobody_and_the_background_check_fetches_once_a_day() {
    let (server, signer) = (Server::start(), Signer::new(1));
    server.publish(&signer, &index());
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let dir = folder();

    // The first frames: nothing is cached, and nobody is asked for it.
    let mut harness = window(&fake, &server, &signer, true, dir.path());
    assert_eq!(server.requests(), [], "the start asks nobody");
    assert_eq!(harness.state().app().plugin_registry().index, None);

    // A minute later the background check comes.
    tests::registry::background_check(harness.state_mut().app_mut());
    settle(&mut harness);
    let requests = server.requests();
    let paths: Vec<&str> = requests.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, [INDEX_PATH, "registry/index.json.minisig"]);
    for request in &requests {
        assert_eq!(request.header("user-agent"), Some("rick/0.6.0"));
    }
    let registry = harness.state().app().plugin_registry();
    let listed = registry
        .index
        .as_ref()
        .is_some_and(|index| index.plugin("git").is_some());
    assert!(listed, "the list is the application's");
    assert_eq!(registry.problem, None);
    drop(harness);

    // The next start reads what was cached, and its background check
    // finds a list that is not a day old: nobody is asked.
    let mut again = window(&fake, &server, &signer, true, dir.path());
    assert!(again.state().app().plugin_registry().index.is_some());
    tests::registry::background_check(again.state_mut().app_mut());
    settle(&mut again);
    assert_eq!(server.requests().len(), 2, "at most once a day");
}

/// The background check comes by itself, through the window's own loop:
/// a Rick whose minute is a moment asks without anything being done.
#[test]
fn the_background_check_comes_by_itself_a_while_after_the_first_frame() {
    let (server, signer) = (Server::start(), Signer::new(1));
    server.publish(&signer, &index());
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let dir = folder();
    let config = fake.config_dir();
    fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
    let settings = tests::registry::rick_toml(&server, &signer, true);
    fs::write(config.join(app::SETTINGS_FILE), settings).unwrap_or_else(|e| panic!("{e}"));
    let host = Arc::new(EguiHost::new());
    let mut app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(config),
        start: Some(local(dir.path())),
    });
    app.set_rick_version(RICK);
    let after_start = Duration::from_millis(500);
    app.set_registry_schedule(after_start, Duration::from_hours(24));
    let started = Instant::now();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 820.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    // The requests first: what they hold happened before the clock is
    // read.
    let asked = server.requests();
    if started.elapsed() < after_start {
        assert_eq!(asked, [], "the first frames ask nobody");
    }

    // Nothing is done but drawing frames.
    let deadline = Instant::now() + Duration::from_secs(30);
    while server.requests().len() < 2 {
        assert!(Instant::now() < deadline, "the check did not come");
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(started.elapsed() >= after_start);
    settle(&mut harness);
    assert!(harness.state().app().plugin_registry().index.is_some());
    assert_eq!(server.requests().len(), 2);
}

/// Issue 251, item 5: *Look for plugins and updates online* is on the
/// Behaviour page of the settings, on by default, and writes `[registry]
/// check`. Off, nothing is fetched by itself; *Check now* still is.
#[test]
fn the_behaviour_page_switches_the_looking_for_plugins_online_off() {
    use egui::accesskit::{Role, Toggled};
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    let (server, signer) = (Server::start(), Signer::new(1));
    server.publish(&signer, &index());
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let dir = folder();
    // A rick.toml that says nothing of the switch: it is on.
    let config = fake.config_dir();
    fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
    let settings = format!(
        "[registry]\nurl = \"{}\"\npublic_key = \"{}\"\n",
        server.index_url(),
        signer.public_key()
    );
    fs::write(config.join(app::SETTINGS_FILE), settings).unwrap_or_else(|e| panic!("{e}"));
    let host = Arc::new(EguiHost::new());
    let mut app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(config.clone()),
        start: Some(local(dir.path())),
    });
    app.set_rick_version(RICK);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    assert!(harness.state().app().model().settings.registry.check);
    tests::registry::background_check(harness.state_mut().app_mut());
    settle(&mut harness);
    assert_eq!(server.requests().len(), 2, "the background check looked");

    harness.state_mut().open_settings();
    for _ in 0..4 {
        harness.step();
    }
    harness
        .get_by_role_and_label(Role::RadioButton, "Behaviour")
        .click();
    for _ in 0..4 {
        harness.step();
    }
    let switch =
        harness.get_by_role_and_label(Role::CheckBox, "Look for plugins and updates online");
    assert_eq!(
        switch.accesskit_node().toggled(),
        Some(Toggled::True),
        "on by default"
    );
    harness.get_by_label_contains("It sends its version and nothing else");
    switch.click();
    for _ in 0..4 {
        harness.step();
    }
    assert!(!harness.state().app().model().settings.registry.check);
    let written = tests::read_until(&config.join(app::SETTINGS_FILE), |text| {
        text.contains("check = false")
    });
    assert!(written.contains("[registry]\ncheck = false"), "{written}");
    // The address and the key the file named are still there.
    assert!(written.contains(&server.index_url()), "{written}");

    // Off: the Available view opened asks nobody; Check now does.
    let send = |harness: &mut Harness<'_, RickUi>, command: app::RegistryCommand| {
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::Registry(command), &mut Vec::new());
        settle(harness);
    };
    send(&mut harness, app::RegistryCommand::Opened);
    assert_eq!(server.requests().len(), 2, "nothing is fetched by itself");
    send(&mut harness, app::RegistryCommand::CheckNow);
    assert_eq!(server.requests().len(), 4);
}
