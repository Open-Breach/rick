//! Acceptance (issue 149, item 3, removable media and connections), in
//! the running window (SPEC.md *Removed and unreachable locations*, *This
//! Machine*).
//!
//! With a tab three folders deep on a stick, pulling the stick sends the
//! tab Home with the toast *<drive> was removed*, and the card's Eject
//! does the same before it unmounts. A remote account that stops
//! answering keeps its tab and shows *Connection lost to <account>* with
//! *Retry*, which lists the folder again once the account answers.
//! *Format…* is in the menu of a removable and of a fixed drive's card,
//! opens the system's tool, and is disabled with *No disk utility is
//! installed* where the system has none.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT as _, Queryable as _};
use ext::{Extension, Level};
use platform::{ConfigDir, Fake};
use tests::local;
use tests::mem::{MemExtension, MemProvider};
use types::{Location, ProviderId};
use ui_egui::{EguiHost, RickUi};

struct Window {
    harness: Harness<'static, RickUi>,
    fake: Arc<Fake>,
    /// The folder the window starts in, kept for the test's length.
    _start: tempfile::TempDir,
}

/// A window with `extensions` loaded, in a folder of its own.
fn window(extensions: Vec<Arc<dyn Extension>>) -> Window {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
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
    Window {
        harness,
        fake,
        _start: start,
    }
}

fn pane(harness: &Harness<'_, RickUi>) -> model::PaneId {
    harness.state().app().model().focused
}

fn tab<'a>(harness: &'a Harness<'_, RickUi>) -> &'a model::Tab {
    harness
        .state()
        .app()
        .model()
        .active_tab(pane(harness))
        .map_or_else(|| panic!("no tab"), |(_, tab)| tab)
}

fn names(harness: &Harness<'_, RickUi>) -> Vec<String> {
    tab(harness).rows().map(|e| e.name.clone()).collect()
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

fn volume(name: &str, mount: &Path, kind: platform::VolumeKind) -> platform::Volume {
    platform::Volume {
        name: name.into(),
        mount_point: mount.to_path_buf(),
        kind,
        fs_type: "vfat".into(),
        device: None,
        total: Some(100 << 20),
        free: Some(40 << 20),
        read_only: false,
    }
}

/// Mounts `volumes` and waits for the side panel to list them.
fn mount(w: &mut Window, volumes: &[platform::Volume]) {
    for volume in volumes {
        w.fake.add_volume(volume.clone());
    }
    w.fake.mounts_changed();
    let count = w.fake_volume_count();
    step_until(&mut w.harness, "the drives are listed", |h| {
        h.state().app().model().volumes.len() == count
    });
}

impl Window {
    fn fake_volume_count(&self) -> usize {
        platform::Volumes::volumes(&*self.fake).map_or(0, |volumes| volumes.len())
    }
}

/// The centre of the drive card named `name`, in the side panel: the
/// card's name starts with the drive's (and carries its percentage), and
/// the panel sits left of the pane.
fn card(harness: &Harness<'_, RickUi>, name: &str) -> egui::Pos2 {
    let pane = pane(harness);
    let pane_left = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("no pane rect"), |(_, r)| r.left());
    harness
        .query_all(
            By::new()
                .label_contains(name)
                .predicate(|n| n.label().is_some_and(|l| l.starts_with(name))),
        )
        .map(|n| n.rect())
        .find(|r| r.center().x < pane_left)
        .map_or_else(|| panic!("no card of {name}"), |r| r.center())
}

/// The toasts that say something was removed.
fn removal_toasts(harness: &Harness<'_, RickUi>) -> Vec<(Level, String)> {
    harness
        .state()
        .toasts()
        .into_iter()
        .filter(|(_, text)| text.contains("was removed"))
        .collect()
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

/// With a tab three folders deep on a stick, pulling the stick sends the
/// tab Home with the toast; the card's Eject sends it Home too, and the
/// stick is unmounted after.
#[test]
fn pulling_or_ejecting_a_stick_sends_its_tab_home_with_a_toast() {
    let mut w = window(Vec::new());
    let stick = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let deep = stick.path().join("one").join("two").join("three");
    fs::create_dir_all(&deep).unwrap_or_else(|e| panic!("{e}"));
    fs::write(deep.join("note.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    let home = local(&w.fake.home_dir());

    mount(
        &mut w,
        &[volume(
            "STICK",
            stick.path(),
            platform::VolumeKind::Removable,
        )],
    );
    navigate(&mut w.harness, &local(&deep));
    assert_eq!(names(&w.harness), ["note.txt"]);

    // Pulled out: the platform's list no longer has it.
    w.fake.remove_volume(stick.path());
    w.fake.mounts_changed();
    step_until(&mut w.harness, "the tab goes Home", |h| {
        tab(h).location == home
    });
    assert_eq!(
        removal_toasts(&w.harness),
        [(Level::Info, "STICK was removed".to_owned())]
    );
    assert_eq!(complaints(&w.harness), [], "a pulled stick is no error");
    assert!(
        w.harness.query_by_label("Eject STICK").is_none(),
        "the card went with the stick"
    );

    // Plugged in again and ejected from its card, three folders deep.
    mount(
        &mut w,
        &[volume(
            "STICK",
            stick.path(),
            platform::VolumeKind::Removable,
        )],
    );
    navigate(&mut w.harness, &local(&deep));
    w.harness.get_by_label("Eject STICK").click();
    w.harness.step();
    w.harness.step();
    assert_eq!(
        tab(&w.harness).location,
        home,
        "Home before the unmount is reported"
    );
    let fake = Arc::clone(&w.fake);
    let mount_point = stick.path().to_path_buf();
    step_until(&mut w.harness, "the stick is ejected", |h| {
        fake.ejected() == [mount_point.clone()] && h.state().app().model().volumes.is_empty()
    });
    settle(&mut w.harness);
    assert_eq!(tab(&w.harness).location, home);
    assert_eq!(complaints(&w.harness), []);
}

/// A remote account that stops answering keeps the tab where it is and
/// shows the card, *Connection lost to <account>* with the provider's
/// message and *Retry*; *Retry* lists the folder again once the account
/// answers.
#[test]
fn a_lost_connection_shows_the_card_and_retry_lists_the_folder_again() {
    const STORE: ProviderId = ProviderId("azure");
    let mem = Arc::new(MemProvider::new(STORE, Duration::ZERO));
    mem.set_store(true);
    mem.file("/work/photos/a.jpg", b"picture a");
    mem.file("/work/photos/b.jpg", b"picture b");
    // The stand-in's name, as a plugin's own strings would give it.
    ext::i18n::add_bundled("en", "removed-media-test-provider = Azure Blob");
    let mut w = window(vec![Arc::new(MemExtension::with_accounts(
        Arc::clone(&mem),
        "removed-media-test-provider",
    ))]);
    let folder = Location::new(STORE, "/work/photos");
    navigate(&mut w.harness, &folder);
    assert_eq!(names(&w.harness), ["a.jpg", "b.jpg"]);
    assert!(w.harness.query_by_label("Retry").is_none());

    // The connection drops: the next listing fails with its error.
    mem.set_unreachable(Some("connection timed out"));
    let pane_id = pane(&w.harness);
    command(&mut w.harness, app::Command::Refresh(pane_id));
    step_until(
        &mut w.harness,
        "the card says the connection is lost",
        |h| h.query_by_label("Connection lost to work").is_some(),
    );
    let harness = &mut w.harness;
    assert_eq!(tab(harness).location, folder, "the tab stays where it is");
    harness.get_by_label("Azure Blob");
    harness.get_by_label("connection timed out");
    assert!(
        harness.query_by_label("a.jpg").is_none(),
        "the card stands in place of the list"
    );
    assert_eq!(complaints(harness), [], "shown in the pane, not as a toast");

    // The account answers again, and Retry lists the folder where the tab
    // was.
    mem.set_unreachable(None);
    harness.get_by_label("Retry").click();
    step_until(harness, "the listing is back", |h| {
        h.query_by_label("Connection lost to work").is_none() && names(h) == ["a.jpg", "b.jpg"]
    });
    assert_eq!(tab(harness).location, folder);
    assert!(harness.query_by_label("Retry").is_none());
}

/// *Format…* is in the menu of a removable and of a fixed drive's card
/// and opens the system's tool on the drive; a network share has none;
/// and without a disk utility the entry is disabled and says so.
#[test]
fn format_is_in_a_drives_menu_and_disabled_without_a_disk_utility() {
    let mut w = window(Vec::new());
    let stick = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let disk = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let share = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    mount(
        &mut w,
        &[
            volume("STICK", stick.path(), platform::VolumeKind::Removable),
            volume("data", disk.path(), platform::VolumeKind::Fixed),
            volume("nas", share.path(), platform::VolumeKind::Network),
        ],
    );
    let fake = Arc::clone(&w.fake);
    let harness = &mut w.harness;

    for (name, mount_point) in [("STICK", stick.path()), ("data", disk.path())] {
        let at = card(harness, name);
        click_at(harness, at, egui::PointerButton::Secondary);
        let entry = harness.get_by_label("Format…");
        assert!(!entry.accesskit_node().is_disabled(), "{name}");
        entry.click();
        let wanted = mount_point.to_path_buf();
        step_until(harness, "the system's tool is opened on the drive", |_| {
            fake.format_opened().last() == Some(&wanted)
        });
        assert!(
            harness.query_by_label("Format…").is_none(),
            "the menu closed"
        );
    }
    assert_eq!(fake.format_opened().len(), 2);

    // A network share is no drive of the machine.
    let at = card(harness, "nas");
    click_at(harness, at, egui::PointerButton::Secondary);
    harness.get_by_label("Open in new tab");
    assert!(harness.query_by_label("Format…").is_none());
    harness.key_press(egui::Key::Escape);
    harness.step();

    // Without a disk utility the entry is there, disabled, and says why.
    fake.set_format_tool(false);
    step_until(harness, "the missing tool is noticed", |h| {
        !h.state().app().model().format_tool
    });
    let at = card(harness, "STICK");
    click_at(harness, at, egui::PointerButton::Secondary);
    let entry = harness.get_by_label("Format…");
    assert!(entry.accesskit_node().is_disabled());
    entry.hover();
    step_until(harness, "the reason as the tooltip", |h| {
        h.query_by_label("No disk utility is installed").is_some()
    });
    let at = harness.get_by_label("Format…").rect().center();
    click_at(harness, at, egui::PointerButton::Primary);
    assert_eq!(fake.format_opened().len(), 2, "nothing is opened");
}
