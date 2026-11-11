//! Acceptance (issue 149, item 2, remote providers), in the running
//! window, on a stand-in for an object store's account (SPEC.md
//! *Capabilities decide what is offered*, *Object storage*, *Remote
//! files*, *Footer*).
//!
//! At the account's root *New folder* is offered and creates a container,
//! whose name is checked before anything is asked of the store, and *New
//! file* is greyed with *Files live inside a container*. Inside a
//! container both are there. Enter on a remote file leaves it in
//! Downloads, shows the job while it runs and opens nothing; *Download…*
//! asks for the folder and brings two files and a folder down. The search
//! in subfolders finds a blob three levels below the pane's folder, and
//! where a location cannot be searched beneath the toggle is disabled
//! with its reason and no error appears anywhere.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use ext::{ActionId, Extension, Level};
use model::SelectionChange;
use platform::{ConfigDir, Fake};
use tests::local;
use tests::mem::{MemExtension, MemProvider};
use types::{Location, ProviderId};
use ui_egui::inline::EditKind;
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi};

const STORE: ProviderId = ProviderId("azure");
const ACCOUNT: &str = "/flowcptstorage";
const CONTAINER: &str = "/flowcptstorage/photos";

/// The account: one container with two pictures, a folder beside them and
/// a blob three levels below the container.
fn account() -> Arc<MemProvider> {
    let mem = Arc::new(MemProvider::new(STORE, Duration::ZERO));
    mem.set_store(true);
    mem.file(&format!("{CONTAINER}/a.jpg"), b"picture a");
    mem.file(&format!("{CONTAINER}/b.jpg"), b"picture b");
    mem.file(&format!("{CONTAINER}/raw/c.dng"), b"negative c");
    mem.file(&format!("{CONTAINER}/2026/may/trip/needle.jpg"), b"needle");
    mem
}

struct Window {
    harness: Harness<'static, RickUi>,
    fake: Arc<Fake>,
    /// The folder the window starts in, kept for the test's length.
    _start: tempfile::TempDir,
}

/// A window with the stores mounted, in the store's folder `at`.
fn window(stores: &[&Arc<MemProvider>], at: &Location) -> Window {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let extensions: Vec<Arc<dyn Extension>> = stores
        .iter()
        .map(|mem| Arc::new(MemExtension::new(Arc::clone(mem))) as Arc<dyn Extension>)
        .collect();
    let start = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions,
        config_dir: Some(fake.config_dir()),
        start: Some(local(start.path())),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 700.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let mut window = Window {
        harness,
        fake,
        _start: start,
    };
    navigate(&mut window.harness, at);
    window
}

fn pane(harness: &Harness<'_, RickUi>) -> model::PaneId {
    harness.state().app().model().focused
}

fn tab<'a>(harness: &'a Harness<'_, RickUi>) -> (model::TabId, &'a model::Tab) {
    harness
        .state()
        .app()
        .model()
        .active_tab(pane(harness))
        .unwrap_or_else(|| panic!("no tab"))
}

fn names(harness: &Harness<'_, RickUi>) -> Vec<String> {
    tab(harness).1.rows().map(|e| e.name.clone()).collect()
}

fn command(harness: &mut Harness<'_, RickUi>, command: app::Command) {
    harness
        .state_mut()
        .app_mut()
        .handle(command, &mut Vec::new());
    harness.step();
}

fn navigate(harness: &mut Harness<'_, RickUi>, to: &Location) {
    let pane = pane(harness);
    command(
        harness,
        app::Command::Navigate {
            pane,
            to: to.clone(),
        },
    );
    settle(harness);
}

fn select(harness: &mut Harness<'_, RickUi>, wanted: &[&str]) {
    let ids = tab(harness)
        .1
        .rows()
        .filter(|e| wanted.contains(&e.name.as_str()))
        .map(|e| e.id)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), wanted.len(), "{:?}", names(harness));
    let pane = pane(harness);
    command(
        harness,
        app::Command::Select {
            pane,
            change: SelectionChange::Set(ids),
        },
    );
}

fn run(harness: &mut Harness<'_, RickUi>, action: &'static str) {
    let pane = pane(harness);
    harness
        .state_mut()
        .queue(Intent::Action(ActionId(action)), pane);
    harness.step();
    harness.step();
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

/// Runs frames until `done` holds.
fn step_until(
    harness: &mut Harness<'_, RickUi>,
    what: &str,
    mut done: impl FnMut(&Harness<'_, RickUi>) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done(harness) {
        harness.step();
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..3 {
        harness.step();
    }
}

fn click_at(harness: &mut Harness<'_, RickUi>, at: egui::Pos2, button: egui::PointerButton) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.step();
    harness.step();
}

/// Opens the menu of the list's empty area, well below the last row.
fn open_empty_menu(harness: &mut Harness<'_, RickUi>) {
    let tab_id = tab(harness).0;
    let toolbar = harness
        .state()
        .toolbar_row(tab_id)
        .unwrap_or_else(|| panic!("no toolbar"));
    let empty = egui::pos2(toolbar.center().x, toolbar.bottom() + 420.0);
    click_at(harness, empty, egui::PointerButton::Secondary);
}

/// The toasts that are no plain notice: warnings and errors.
fn complaints(harness: &Harness<'_, RickUi>) -> Vec<(Level, String)> {
    harness
        .state()
        .toasts()
        .into_iter()
        .filter(|(level, _)| *level != Level::Info)
        .collect()
}

/// The jobs the operations drawer knows, by name, with whether each is
/// still running.
fn jobs(harness: &Harness<'_, RickUi>) -> Vec<(String, bool)> {
    harness
        .state()
        .app()
        .model()
        .jobs
        .values()
        .map(|job| {
            (
                job.name.clone(),
                matches!(job.state, model::JobState::Running(_)),
            )
        })
        .collect()
}

/// At the account's root the menu offers New folder and greys New file,
/// with the reason as its tooltip; inside a container, at any depth, both
/// are there to use.
#[test]
fn at_an_account_new_folder_is_offered_and_new_file_is_greyed_with_its_reason() {
    let mem = account();
    let mut w = window(&[&mem], &Location::new(STORE, ACCOUNT));
    let harness = &mut w.harness;
    assert_eq!(names(harness), ["photos"]);

    open_empty_menu(harness);
    let new_folder = harness.get_by_label("New folder");
    assert!(!new_folder.accesskit_node().is_disabled());
    let new_file = harness.get_by_label("New file");
    assert!(
        new_file.accesskit_node().is_disabled(),
        "New file is greyed"
    );
    // It says why, and a click on it starts nothing.
    new_file.hover();
    step_until(harness, "the reason as the tooltip", |h| {
        h.query_by_label("Files live inside a container").is_some()
    });
    let at = harness.get_by_label("New file").rect().center();
    click_at(harness, at, egui::PointerButton::Primary);
    assert!(harness.state().editing(tab(harness).0).is_none());
    harness.key_press(egui::Key::Escape);
    harness.step();

    // By its (rebindable) action too, it runs nothing and says why.
    run(harness, "core.new_file");
    settle(harness);
    assert!(harness.state().editing(tab(harness).0).is_none());
    assert_eq!(
        complaints(harness),
        [(
            Level::Warning,
            "New file: Files live inside a container".to_owned()
        )]
    );
    assert_eq!(
        mem.paths()
            .iter()
            .filter(|p| p.contains("New file"))
            .count(),
        0
    );

    // Inside the container, and two levels below it, both are offered.
    for inside in [CONTAINER.to_owned(), format!("{CONTAINER}/2026/may")] {
        navigate(harness, &Location::new(STORE, inside.as_str()));
        open_empty_menu(harness);
        for label in ["New folder", "New file"] {
            assert!(
                !harness.get_by_label(label).accesskit_node().is_disabled(),
                "{label} in {inside}"
            );
        }
        harness.key_press(egui::Key::Escape);
        harness.step();
    }
}

/// New folder at the account's root creates a container. A name the
/// naming rules refuse — the suggested *New folder* is one — is refused in
/// place, before the store is asked for anything: the row stays in edit
/// and a notice names the rule. A name they take is created.
#[test]
fn a_container_is_created_at_the_root_under_a_name_the_rules_take() {
    let mem = account();
    let mut w = window(&[&mem], &Location::new(STORE, ACCOUNT));
    let harness = &mut w.harness;
    let tab_id = tab(harness).0;

    run(harness, "core.new_folder");
    let edit = harness
        .state()
        .editing(tab_id)
        .cloned()
        .unwrap_or_else(|| panic!("no new-folder edit"));
    assert_eq!(edit.kind, EditKind::NewFolder);
    harness.key_press(egui::Key::Enter);
    harness.step();
    harness.step();
    harness.step();
    let refused = harness
        .state()
        .editing(tab_id)
        .cloned()
        .unwrap_or_else(|| panic!("the edit is gone"));
    assert_eq!(
        refused.text, "New folder",
        "the name is kept for another try"
    );
    assert_eq!(
        complaints(harness),
        [(
            Level::Warning,
            "A container name has lowercase letters, digits and hyphens only".to_owned()
        )]
    );
    assert_eq!(mem.made(), Vec::<String>::new(), "the store was not asked");

    // Another name, in the same edit.
    step_until(harness, "the editor has the keyboard again", |h| {
        h.state().editing(tab_id).is_some_and(|e| !e.focus)
    });
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.step();
    harness.event(egui::Event::Text("archive-2026".to_owned()));
    harness.step();
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(harness);
    assert!(harness.state().editing(tab_id).is_none());
    assert_eq!(mem.made(), [format!("{ACCOUNT}/archive-2026")]);
    step_until(harness, "the container is listed", |h| {
        names(h) == ["archive-2026", "photos"]
    });
}

/// Inside a container a new folder and a new file appear two levels deep,
/// under the names the inline edit suggests, and are there after a
/// refresh.
#[test]
fn a_folder_and_a_file_appear_two_levels_deep_and_survive_a_refresh() {
    let mem = account();
    let deep = format!("{CONTAINER}/2026/may");
    let mut w = window(&[&mem], &Location::new(STORE, deep.as_str()));
    let harness = &mut w.harness;
    assert_eq!(names(harness), ["trip"]);

    run(harness, "core.new_folder");
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(harness);
    run(harness, "core.new_file");
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(harness);
    assert_eq!(complaints(harness), Vec::new());
    step_until(harness, "both are listed", |h| {
        names(h) == ["New folder", "trip", "New file.txt"]
    });

    let pane_id = pane(harness);
    harness.state_mut().queue(Intent::Refresh, pane_id);
    harness.step();
    settle(harness);
    assert_eq!(names(harness), ["New folder", "trip", "New file.txt"]);
    assert!(mem.paths().contains(&format!("{deep}/New folder")));
    assert!(mem.paths().contains(&format!("{deep}/New file.txt")));
}

/// Enter on a remote file starts a download into Downloads: the job shows
/// while it runs, the file is there when it ends and nothing is opened.
/// A second Enter meets the name and asks with the paste-conflict
/// dialogue.
#[test]
fn enter_on_a_remote_file_leaves_it_in_downloads_and_opens_nothing() {
    let mem = account();
    let mut w = window(&[&mem], &Location::new(STORE, CONTAINER));
    let downloads = w.fake.home_dir().join("Downloads");
    fs::create_dir_all(&downloads).unwrap_or_else(|e| panic!("{e}"));
    let harness = &mut w.harness;

    // The link is slow: the read stops half way, so the job can be seen.
    mem.hold_reads_at(4);
    select(harness, &["a.jpg"]);
    let focused = pane(harness);
    harness.state_mut().queue(Intent::Open, focused);
    step_until(harness, "the download runs as a job", |h| {
        jobs(h).contains(&("Download a.jpg to Downloads".to_owned(), true))
    });
    mem.release_reads();
    settle(harness);
    assert_eq!(
        fs::read(downloads.join("a.jpg")).unwrap_or_else(|e| panic!("{e}")),
        b"picture a"
    );
    assert_eq!(w.fake.opened(), Vec::<PathBuf>::new(), "nothing is opened");
    assert_eq!(complaints(harness), Vec::new());
    assert_eq!(tab(harness).1.location, Location::new(STORE, CONTAINER));

    // Once more: the name is there, and the question is the paste
    // conflict's, whose Enter keeps both.
    let focused = pane(harness);
    harness.state_mut().queue(Intent::Open, focused);
    step_until(harness, "the paste-conflict dialogue", |h| {
        h.query_by_label("Keep both").is_some()
    });
    harness.get_by_label("Keep both").click();
    harness.step();
    settle(harness);
    assert!(downloads.join("a (2).jpg").is_file());
    assert_eq!(w.fake.opened(), Vec::<PathBuf>::new());
}

/// Download… on two files and a folder asks for the target folder with
/// the system's dialogue and brings all of it down, the folder with what
/// is beneath it. A dismissed dialogue downloads nothing.
#[test]
fn download_asks_for_the_target_and_brings_two_files_and_a_folder_down() {
    let mem = account();
    let mut w = window(&[&mem], &Location::new(STORE, CONTAINER));
    let target = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let harness = &mut w.harness;

    // The entry is in the file's menu on a remote location.
    let row = harness.get_by_label("a.jpg").rect().center();
    click_at(harness, row, egui::PointerButton::Secondary);
    assert!(
        !harness
            .get_by_label("Download…")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness.query_by_label("Open with…").is_none(),
        "nothing remote is handed to another program"
    );
    harness.key_press(egui::Key::Escape);
    harness.step();

    select(harness, &["a.jpg", "b.jpg", "raw"]);
    // Dismissed: nothing happens.
    w.fake.answer_folder(None);
    run(harness, "core.download");
    settle(harness);
    assert_eq!(jobs(harness), Vec::new());

    w.fake.answer_folder(Some(target.path().to_path_buf()));
    run(harness, "core.download");
    step_until(harness, "the download ran", |h| {
        jobs(h)
            .iter()
            .any(|(name, running)| name.starts_with("Download 3 items") && !running)
    });
    settle(harness);
    let read =
        |name: &str| fs::read(target.path().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(read("a.jpg"), b"picture a");
    assert_eq!(read("b.jpg"), b"picture b");
    assert_eq!(read("raw/c.dng"), b"negative c");
    assert_eq!(w.fake.opened(), Vec::<PathBuf>::new());
    assert_eq!(complaints(harness), Vec::new());
}

/// With the subfolder toggle on, the filter finds a blob three levels
/// below the pane's folder on the store. On a remote location that cannot
/// be searched beneath, the toggle is disabled with its reason, its
/// shortcut does nothing, and no error appears anywhere.
#[test]
fn the_search_in_subfolders_finds_a_blob_three_levels_below_and_is_disabled_where_it_cannot() {
    let mem = account();
    // A host like an SSH one: remote, with no listing beneath.
    let host = Arc::new(MemProvider::new(ProviderId("nas"), Duration::ZERO));
    host.set_native(true);
    host.file("/srv/2026/may/trip/needle.jpg", b"needle");
    let mut w = window(&[&mem, &host], &Location::new(STORE, CONTAINER));
    let harness = &mut w.harness;

    let toggle = harness.get_by_label("Include subfolders");
    assert!(!toggle.accesskit_node().is_disabled());
    let focused = pane(harness);
    harness.state_mut().queue(Intent::ToggleSubtree, focused);
    harness.step();
    assert!(tab(harness).1.subtree);
    let pane_id = pane(harness);
    command(
        harness,
        app::Command::Filter {
            pane: pane_id,
            text: "needle".to_owned(),
        },
    );
    settle(harness);
    assert_eq!(names(harness), ["needle.jpg"]);
    let found = tab(harness)
        .1
        .rows()
        .next()
        .map(|e| e.path.to_string())
        .unwrap_or_default();
    assert_eq!(found, format!("{CONTAINER}/2026/may/trip/needle.jpg"));
    assert_eq!(complaints(harness), Vec::new());

    // On the host the pane lists the folder, the toggle is off and
    // disabled, and it says why.
    navigate(harness, &Location::new(ProviderId("nas"), "/srv"));
    assert!(!tab(harness).1.subtree, "the search ended with the move");
    let toggle = harness.get_by_label("Include subfolders");
    assert!(toggle.accesskit_node().is_disabled());
    toggle.hover();
    step_until(harness, "the reason as the tooltip", |h| {
        h.query_by_label("Not available in this location").is_some()
    });
    let focused = pane(harness);
    harness.state_mut().queue(Intent::ToggleSubtree, focused);
    harness.step();
    command(
        harness,
        app::Command::Filter {
            pane: pane_id,
            text: "needle".to_owned(),
        },
    );
    settle(harness);
    assert!(!tab(harness).1.subtree);
    assert_eq!(names(harness), Vec::<String>::new(), "the folder, filtered");
    assert_eq!(complaints(harness), Vec::new(), "no error anywhere");
}
