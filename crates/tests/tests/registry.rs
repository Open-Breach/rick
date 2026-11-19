//! Acceptance (issue 251, item 1, specs/plugin-distribution.md §6): the
//! registry client behind the window, against a registry of the test's
//! own — a server on this machine that counts what it is asked, and a key
//! made here. No test asks get.openbreach.dev.
//!
//! Rick reads the plugin list once its first frame is drawn. With *Look
//! for plugins and updates online* off and nothing cached, starting Rick
//! and selecting files makes no request; with it on, the start makes one
//! for each of the two files and the next start, the same day, none.

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
fn the_start_fetches_the_list_once_the_first_frame_is_drawn_and_once_a_day() {
    let (server, signer) = (Server::start(), Signer::new(1));
    server.publish(&signer, &index());
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let dir = folder();

    let harness = window(&fake, &server, &signer, true, dir.path());
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

    // The next start reads what was cached and asks nobody.
    let again = window(&fake, &server, &signer, true, dir.path());
    assert!(again.state().app().plugin_registry().index.is_some());
    assert_eq!(server.requests().len(), 2, "at most once a day");
}
