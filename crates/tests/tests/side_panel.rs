//! Acceptance (0.2 milestone 2, the side panel rework): Places first, then
//! This Machine, Network and, with a cloud provider, Cloud; the drop rule;
//! places that persist their order, titles and icons; Favorites, Recent and
//! Trash as views in the active tab; utilisation, the overview and Eject;
//! live mount and unmount; open in new tab by middle click, Mod+click and
//! the context menu; the Tree section gone; the keyboard; the Trash view's
//! restore and empty; per-volume scheduling with the *Waiting* state; the
//! operations area under many jobs; and progress throttling over the 200k
//! fixture.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::{ActionId, Extension};
use model::{JobState, Section, SelectionChange};
use platform::{ConfigDir, Fake, Trash as _};
use tests::local;
use tests::mem::{MemExtension, MemProvider};
use types::{Location, ProviderId};
use ui_egui::{EguiHost, RickUi, icons};
use vfs_rick::Virtual;

/// A harness over `fake` whose settings live in `config`, with
/// `extensions` loaded besides the core.
fn harness_with(
    start: &Path,
    config: &Path,
    fake: Arc<Fake>,
    extensions: Vec<Arc<dyn Extension>>,
) -> Harness<'static, RickUi> {
    let host = Arc::new(EguiHost::new());
    // The Trash view is the trash plugin's (issue 20).
    let mut extensions = extensions;
    extensions.push(Arc::new(ext_vfs_trash::TrashExtension::new()));
    let app = app::App::new(app::Config {
        platform: fake,
        ui: host.clone(),
        extensions,
        config_dir: Some(config.to_path_buf()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_step_dt(0.05)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
}

fn harness(start: &Path) -> (Harness<'static, RickUi>, Arc<Fake>) {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let config = fake.config_dir();
    let harness = harness_with(start, &config, fake.clone(), Vec::new());
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

/// Steps until `done`, within `limit`.
fn step_until(
    harness: &mut Harness<'_, RickUi>,
    limit: Duration,
    mut done: impl FnMut(&Harness<'_, RickUi>) -> bool,
) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        harness.step();
        if done(harness) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    done(harness)
}

fn command(harness: &mut Harness<'_, RickUi>, command: app::Command) {
    harness
        .state_mut()
        .app_mut()
        .handle(command, &mut Vec::new());
    harness.step();
}

fn pane(harness: &Harness<'_, RickUi>) -> model::PaneId {
    harness.state().app().model().focused
}

/// The focused pane's active tab.
fn tab(harness: &Harness<'_, RickUi>) -> model::Tab {
    let app = harness.state().app();
    app.model()
        .active_tab(app.model().focused)
        .map_or_else(|| panic!("no tab"), |(_, t)| t.clone())
}

/// The focused pane's location.
fn location(harness: &Harness<'_, RickUi>) -> Location {
    tab(harness).location
}

/// The focused pane's location as an OS path.
fn local_location(harness: &Harness<'_, RickUi>) -> PathBuf {
    platform::paths::to_path(&location(harness).path).unwrap_or_else(|e| panic!("{e}"))
}

/// The workspace tabs of the focused window: *Open in new tab* opens a
/// workspace (SPEC.md *Workspace tabs*).
fn tab_count(harness: &Harness<'_, RickUi>) -> usize {
    let model = harness.state().app().model();
    model.workspaces(model.focused_window).len()
}

fn row_names(harness: &Harness<'_, RickUi>) -> Vec<String> {
    tab(harness).rows().map(|e| e.name.clone()).collect()
}

fn pane_rect(harness: &Harness<'_, RickUi>) -> egui::Rect {
    let pane = pane(harness);
    harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("no pane rect"), |(_, r)| r)
}

/// The side panel rows whose label contains `text`, top to bottom: the
/// panel sits left of the focused pane.
fn panel_rects(harness: &Harness<'_, RickUi>, text: &str) -> Vec<egui::Rect> {
    let rect = pane_rect(harness);
    let mut rects: Vec<egui::Rect> = harness
        .query_all_by_label_contains(text)
        .map(|n| n.rect())
        .filter(|r| r.center().x < rect.left())
        .collect();
    rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
    rects
}

/// The centre of the one side panel row whose label contains `text`.
fn panel_row(harness: &Harness<'_, RickUi>, text: &str) -> egui::Pos2 {
    let rects = panel_rects(harness, text);
    assert_eq!(rects.len(), 1, "{text}: {rects:?}");
    rects[0].center()
}

fn click_at(harness: &mut Harness<'_, RickUi>, at: egui::Pos2, button: egui::PointerButton) {
    click_with(harness, at, button, egui::Modifiers::NONE);
}

/// Clicks the side panel row whose label contains `text`.
fn click_row(harness: &mut Harness<'_, RickUi>, text: &str, button: egui::PointerButton) {
    let at = panel_row(harness, text);
    click_at(harness, at, button);
}

/// Clicks the side panel row whose label contains `text` with `modifiers`.
fn click_row_with(
    harness: &mut Harness<'_, RickUi>,
    text: &str,
    button: egui::PointerButton,
    modifiers: egui::Modifiers,
) {
    let at = panel_row(harness, text);
    click_with(harness, at, button, modifiers);
}

fn click_with(
    harness: &mut Harness<'_, RickUi>,
    at: egui::Pos2,
    button: egui::PointerButton,
    modifiers: egui::Modifiers,
) {
    harness.event(egui::Event::PointerMoved(at));
    harness.event(egui::Event::ModifiersChanged(modifiers));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: at,
        button,
        pressed: true,
        modifiers,
    });
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: at,
        button,
        pressed: false,
        modifiers,
    });
    harness.step();
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    harness.step();
}

fn select(harness: &mut Harness<'_, RickUi>, name: &str) {
    let pane = pane(harness);
    let id = tab(harness).rows().find(|e| e.name == name).map_or_else(
        || panic!("no row {name} in {:?}", row_names(harness)),
        |e| e.id,
    );
    command(
        harness,
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
    );
}

fn run(harness: &mut Harness<'_, RickUi>, action: &'static str) {
    let pane = pane(harness);
    harness
        .state_mut()
        .queue(ui_egui::Intent::Action(ActionId(action)), pane);
    harness.step();
    harness.step();
}

/// Waits until the session file contains `needle`.
fn session_contains(harness: &mut Harness<'_, RickUi>, needle: &str) -> String {
    let path = harness.state().app().session_path().to_path_buf();
    wait_for(harness, &path, needle)
}

fn wait_for(harness: &mut Harness<'_, RickUi>, path: &Path, needle: &str) -> String {
    wait_until(harness, path, &format!("contain {needle:?}"), |text| {
        text.contains(needle)
    })
}

/// Steps until the file at `path` satisfies `done`, within ten seconds,
/// and returns its text. The application writes its files from a
/// background task that keeps only the latest value, so a test that made
/// several changes waits for the last one's mark, not the first's.
fn wait_until(
    harness: &mut Harness<'_, RickUi>,
    path: &Path,
    what: &str,
    done: impl Fn(&str) -> bool,
) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = fs::read_to_string(path).unwrap_or_default();
        if done(&text) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "{} never came to {what}:\n{text}",
            path.display()
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A folder with three files and a `sub` folder.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::create_dir_all(dir.path().join("sub/deeper")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("sub/inner.txt"), "inner").unwrap_or_else(|e| panic!("{e}"));
    dir
}

fn volume(name: &str, mount: &Path, kind: platform::VolumeKind) -> platform::Volume {
    platform::Volume {
        name: name.into(),
        mount_point: mount.to_path_buf(),
        kind,
        fs_type: "ext4".into(),
        device: None,
        total: Some(100 << 20),
        free: Some(40 << 20),
    }
}

/// A stand-in cloud provider: `s3` with two buckets.
fn cloud() -> (Arc<MemProvider>, Arc<dyn Extension>) {
    let s3 = Arc::new(MemProvider::new(
        ProviderId("s3"),
        Duration::from_millis(400),
    ));
    s3.dir("/bucket-a");
    s3.dir("/bucket-b");
    s3.file("/bucket-a/report.pdf", b"pdf");
    let ext: Arc<dyn Extension> = Arc::new(MemExtension::new(Arc::clone(&s3)));
    (s3, ext)
}

#[test]
fn the_panel_shows_places_then_the_sections_and_cloud_only_with_a_cloud_provider() {
    let dir = tree();
    let (harness, _fake) = harness(dir.path());
    // Places, in the spec's order as far as the fake has the folders.
    let places: Vec<String> = harness
        .state()
        .app()
        .model()
        .places
        .rows()
        .iter()
        .map(|p| p.title.clone())
        .collect();
    assert_eq!(
        places,
        [
            "Home",
            "Documents",
            "Pictures",
            "Downloads",
            "Favorites",
            "Recent",
            "Trash"
        ]
    );
    for title in &places {
        assert_eq!(panel_rects(&harness, title).len(), 1, "{title}");
    }
    // Then the two sections; Cloud is absent without a cloud provider, and
    // so are 0.1's Pinned and Tree.
    assert_eq!(
        ui_egui::side_panel::sections_shown(harness.state().app()),
        [Section::Machine, Section::Network]
    );
    let machine = panel_row(&harness, "This Machine");
    let network = panel_row(&harness, "Network");
    assert!(
        panel_rects(&harness, "Cloud").is_empty(),
        "no cloud provider"
    );
    for gone in ["Pinned", "Tree"] {
        assert!(panel_rects(&harness, gone).is_empty(), "{gone} is gone");
    }
    let last_place = panel_row(&harness, "Trash");
    assert!(
        last_place.y < machine.y && machine.y < network.y,
        "top to bottom"
    );
    assert!(
        harness
            .query_by_label_contains("Coming in a later")
            .is_none(),
        "nothing is announced"
    );

    // With a cloud provider mounted the Cloud section shows its rows.
    let (_s3, ext) = cloud();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = harness_with(dir.path(), &fake.config_dir(), fake, vec![ext]);
    assert_eq!(
        ui_egui::side_panel::sections_shown(harness.state().app()),
        [Section::Machine, Section::Network, Section::Cloud]
    );
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().cloud.len() == 2
    }));
    let cloud_header = panel_row(&harness, "Cloud");
    assert!(cloud_header.y > network.y);
    let bucket = panel_row(&harness, "bucket-a");
    click_at(&mut harness, bucket, egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(
        location(&harness),
        Location::new(ProviderId("s3"), "/bucket-a")
    );
    assert_eq!(row_names(&harness), ["report.pdf"]);
    // The header opens the Cloud overview, which lists the same rows.
    click_at(&mut harness, cloud_header, egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(location(&harness), Virtual::Cloud.location());
    assert_eq!(row_names(&harness), ["bucket-a", "bucket-b"]);
}

#[test]
fn mod_b_hides_and_shows_the_panel() {
    let dir = tree();
    let (mut harness, _fake) = harness(dir.path());
    panel_row(&harness, "This Machine");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    harness.step();
    harness.step();
    assert!(harness.query_by_label("This Machine").is_none(), "hidden");
    session_contains(&mut harness, "visible = false");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    harness.step();
    harness.step();
    panel_row(&harness, "This Machine");
    session_contains(&mut harness, "visible = true");
}

#[test]
fn places_open_in_the_pane_and_the_views_are_never_listed_inline() {
    let dir = tree();
    let (mut harness, fake) = harness(dir.path());
    // A folder place.
    click_row(&mut harness, "Documents", egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(local_location(&harness), fake.home_dir().join("Documents"));
    assert_eq!(tab_count(&harness), 1, "the active tab moved");

    // Favorites: a view in the tab; the panel lists none of the pins.
    command(
        &mut harness,
        app::Command::Pin {
            items: vec![local(&dir.path().join("sub"))],
            at: None,
        },
    );
    click_row(&mut harness, "Favorites", egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(location(&harness), Virtual::Favorites.location());
    assert_eq!(row_names(&harness), ["sub"]);
    assert!(
        panel_rects(&harness, "sub").is_empty(),
        "the panel never lists the pins"
    );
    assert!(
        harness.query_all_by_label_contains("Favorites").count() >= 2,
        "the row and the tab's title"
    );

    // Recent: the system's list, newest first.
    fake.set_recent(vec![
        platform::RecentItem {
            path: dir.path().join("a.txt"),
            used_at: std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(10),
        },
        platform::RecentItem {
            path: dir.path().join("c.txt"),
            used_at: std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(20),
        },
    ]);
    click_row(&mut harness, "Recent", egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(location(&harness), Virtual::Recent.location());
    assert_eq!(row_names(&harness), ["c.txt", "a.txt"]);
    assert_eq!(panel_rects(&harness, "c.txt"), Vec::<egui::Rect>::new());

    // Trash: the view, empty.
    click_row(&mut harness, "Trash", egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(location(&harness), Virtual::Trash.location());
    assert_eq!(row_names(&harness), Vec::<String>::new());
    // Back and forward work across views like folders.
    let focused = pane(&harness);
    command(&mut harness, app::Command::Back(focused));
    settle(&mut harness);
    assert_eq!(location(&harness), Virtual::Recent.location());
}

#[test]
fn middle_click_mod_click_and_the_context_menu_open_a_row_in_a_new_tab() {
    let dir = tree();
    let (mut harness, fake) = harness(dir.path());
    assert_eq!(tab_count(&harness), 1);
    click_row(&mut harness, "Documents", egui::PointerButton::Middle);
    settle(&mut harness);
    assert_eq!(tab_count(&harness), 2, "a middle click opened a tab");
    assert_eq!(local_location(&harness), fake.home_dir().join("Documents"));

    click_row_with(
        &mut harness,
        "Downloads",
        egui::PointerButton::Primary,
        egui::Modifiers::COMMAND,
    );
    settle(&mut harness);
    assert_eq!(tab_count(&harness), 3, "Mod+click opened a tab");
    assert_eq!(local_location(&harness), fake.home_dir().join("Downloads"));

    click_row(&mut harness, "This Machine", egui::PointerButton::Secondary);
    harness.get_by_label("Open in new tab").click();
    harness.step();
    settle(&mut harness);
    assert_eq!(tab_count(&harness), 4, "the context menu opened a tab");
    assert_eq!(location(&harness), Virtual::Machine.location());
}

#[test]
fn places_can_be_reordered_removed_and_edited_and_survive_a_restart() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let config = fake.config_dir();
    let titles = |harness: &Harness<'_, RickUi>| -> Vec<String> {
        harness
            .state()
            .app()
            .model()
            .places
            .rows()
            .iter()
            .map(|p| p.title.clone())
            .collect()
    };
    {
        let mut harness = harness_with(dir.path(), &config, fake.clone(), Vec::new());
        // Remove Pictures from its context menu.
        click_row(&mut harness, "Pictures", egui::PointerButton::Secondary);
        harness.get_by_label("Remove").click();
        harness.step();
        harness.step();
        assert!(!titles(&harness).iter().any(|t| t == "Pictures"));
        assert_eq!(panel_rects(&harness, "Pictures"), Vec::<egui::Rect>::new());

        // Edit Downloads: a new title and the star icon.
        click_row(&mut harness, "Downloads", egui::PointerButton::Secondary);
        harness.get_by_label_contains("Edit…").click();
        harness.step();
        harness.step();
        harness.get_by_label("Title");
        // The dialogue's text field has the focus; replace the text.
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.event(egui::Event::Text("Inbox".to_owned()));
        harness.step();
        harness.get_by_label(icons::STAR).click();
        harness.step();
        harness.get_by_label("OK").click();
        harness.step();
        harness.step();
        let rows = harness.state().app().model().places.rows().to_vec();
        let inbox = rows
            .iter()
            .find(|p| p.title == "Inbox")
            .unwrap_or_else(|| panic!("edited: {rows:?}"));
        assert_eq!(inbox.icon, "STAR");
        assert_eq!(inbox.location, local(&fake.home_dir().join("Downloads")));
        panel_row(&harness, "Inbox");

        // Reorder by dragging Trash above Home.
        let from = panel_row(&harness, "Trash");
        let home = panel_row(&harness, "Home");
        let to = egui::pos2(home.x, home.y - 6.0);
        harness.event(egui::Event::PointerMoved(from));
        harness.step();
        harness.event(egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
        for i in 1..=6u8 {
            let t = f32::from(i) / 6.0;
            harness.event(egui::Event::PointerMoved(from + (to - from) * t));
            harness.step();
        }
        harness.event(egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
        harness.step();
        assert_eq!(
            titles(&harness),
            ["Trash", "Home", "Documents", "Inbox", "Favorites", "Recent"]
        );
        let file = config.join(app::PLACES_FILE);
        // The reorder was the last change, so its order marks the final file.
        let text = wait_until(&mut harness, &file, "Trash before Home", |text| {
            text.contains("Inbox") && text.find("Trash") < text.find("Home")
        });
        assert!(text.contains("icon = \"STAR\""), "{text}");
        assert!(!text.contains("Pictures"), "{text}");
    }
    // A fresh start from the same config has the edited list.
    let harness = harness_with(dir.path(), &config, fake, Vec::new());
    assert_eq!(
        titles(&harness),
        ["Trash", "Home", "Documents", "Inbox", "Favorites", "Recent"]
    );
    panel_row(&harness, "Inbox");
    assert_eq!(panel_rects(&harness, "Pictures"), Vec::<egui::Rect>::new());
}

#[test]
fn this_machine_shows_utilisation_the_header_opens_the_overview_and_eject_works() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let stick = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(stick.path().join("on-the-stick.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    fake.add_volume(volume("data", dir.path(), platform::VolumeKind::Fixed));
    let mut removable = volume("STICK", stick.path(), platform::VolumeKind::Removable);
    removable.fs_type = "vfat".into();
    removable.total = Some(16 << 30);
    removable.free = Some(4 << 30);
    fake.add_volume(removable);
    let mut harness = harness_with(dir.path(), &fake.config_dir(), fake.clone(), Vec::new());
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().volumes.len() == 2
    }));
    // Each row: the name and the utilisation (60 of 100 MiB used, 12 of 16
    // GiB).
    panel_row(&harness, "data");
    harness.get_by_label_contains("60%");
    harness.get_by_label_contains("75%");
    // A row opens the drive's root.
    click_row(&mut harness, "STICK", egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(local_location(&harness), stick.path());
    harness.get_by_label("on-the-stick.txt");

    // The header opens the overview with the full table.
    click_row(&mut harness, "This Machine", egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(location(&harness), Virtual::Machine.location());
    for head in [
        "Mount point",
        "File system",
        "Total",
        "Used",
        "Free",
        "Utilisation",
        "Removable",
    ] {
        harness.get_by_label(head);
    }
    harness.get_by_label("vfat");
    harness.get_by_label("16.0 GiB");
    harness.get_by_label("12.0 GiB");
    harness.get_by_label("4.0 GiB");
    assert_eq!(row_names(&harness), ["data", "STICK"]);

    // Eject from the overview: the platform ejects and the list follows.
    harness.get_by_label_contains("Eject").click();
    harness.step();
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().volumes.len() == 1
    }));
    assert_eq!(fake.ejected(), [stick.path().to_path_buf()]);
    // The overview lists again without the stick.
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        row_names(h) == ["data"]
    }));
    // No Eject button in the table (the job's row in the operations area
    // still says Eject STICK for a few seconds).
    let table = pane_rect(&harness);
    assert!(
        harness
            .query_all_by_label_contains("Eject")
            .all(|n| !table.contains(n.rect().center())),
        "the fixed disk has none"
    );
    // The row is gone from the panel (once the job's row, which names the
    // stick too, is dismissed).
    let ejects: Vec<types::JobId> = harness.state().app().model().jobs.keys().copied().collect();
    for id in ejects {
        command(&mut harness, app::Command::DismissJob(id));
    }
    assert_eq!(panel_rects(&harness, "STICK"), Vec::<egui::Rect>::new());

    // Eject from the row's context menu too.
    fake.add_volume(volume(
        "STICK",
        stick.path(),
        platform::VolumeKind::Removable,
    ));
    command(&mut harness, app::Command::RefreshVolumes);
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().volumes.len() == 2
    }));
    click_row(&mut harness, "STICK", egui::PointerButton::Secondary);
    // The menu's Eject, not the table's button.
    let table = pane_rect(&harness);
    let item = harness
        .query_all_by_label_contains("Eject")
        .map(|n| n.rect())
        .find(|r| !table.contains(r.center()))
        .map_or_else(|| panic!("no Eject in the menu"), |r| r.center());
    click_at(&mut harness, item, egui::PointerButton::Primary);
    assert!(step_until(&mut harness, Duration::from_secs(5), |_| {
        fake.ejected().len() == 2
    }));
}

#[test]
fn a_mounted_volume_appears_and_an_unmounted_one_goes_without_a_refresh() {
    let dir = tree();
    let (mut harness, fake) = harness(dir.path());
    assert_eq!(panel_rects(&harness, "USB"), Vec::<egui::Rect>::new());
    assert_eq!(
        panel_rects(&harness, "Add…").len(),
        2,
        "both sections empty"
    );
    let mount = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fake.add_volume(volume("USB", mount.path(), platform::VolumeKind::Removable));
    let share = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fake.add_volume(volume("nas", share.path(), platform::VolumeKind::Network));
    // No command, no refresh: the poll picks both up.
    assert!(
        step_until(&mut harness, Duration::from_secs(6), |h| {
            !panel_rects(h, "USB").is_empty() && !panel_rects(h, "nas").is_empty()
        }),
        "the mount shows up by itself"
    );
    let usb = panel_row(&harness, "USB");
    let nas = panel_row(&harness, "nas");
    assert!(
        usb.y < panel_row(&harness, "Network").y,
        "USB under This Machine"
    );
    assert!(
        nas.y > panel_row(&harness, "Network").y,
        "nas under Network"
    );
    assert!(
        harness.query_by_label_contains("Add…").is_none(),
        "nothing to add"
    );
    fake.remove_volume(mount.path());
    assert!(
        step_until(&mut harness, Duration::from_secs(6), |h| {
            panel_rects(h, "USB").is_empty()
        }),
        "the unmount shows up by itself"
    );
    panel_row(&harness, "nas");
}

#[test]
fn sections_fold_with_the_chevron_and_the_fold_survives_a_restart() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let config = fake.config_dir();
    {
        let mut harness = harness_with(dir.path(), &config, fake.clone(), Vec::new());
        // The empty Network section shows its Add… row under the header.
        let add = panel_rects(&harness, "Add…");
        assert_eq!(add.len(), 2, "This Machine and Network are empty");
        let network = panel_row(&harness, "Network");
        // The chevron sits left of the title.
        let chevron = egui::pos2(network.x - 60.0, network.y);
        let chevrons: Vec<egui::Rect> = panel_rects(&harness, icons::TREE_OPEN);
        let on_row = chevrons
            .iter()
            .find(|r| (r.center().y - network.y).abs() < 4.0)
            .map_or(chevron, egui::Rect::center);
        click_at(&mut harness, on_row, egui::PointerButton::Primary);
        harness.step();
        assert_eq!(panel_rects(&harness, "Add…").len(), 1, "Network folded");
        assert_eq!(
            location(&harness),
            local(dir.path()),
            "the chevron did not navigate"
        );
        let window = harness.state().app().model().focused_window;
        assert!(
            ui_egui::side_panel::folded(harness.state().app(), window).contains(&Section::Network)
        );
        session_contains(&mut harness, "folded = [\"network\"]");
    }
    let harness = harness_with(dir.path(), &config, fake, Vec::new());
    assert_eq!(panel_rects(&harness, "Add…").len(), 1, "still folded");
    panel_row(&harness, "Network");
}

#[test]
fn with_the_panel_focused_up_down_enter_and_mod_enter_navigate() {
    let dir = tree();
    let (mut harness, fake) = harness(dir.path());
    click_row(&mut harness, "Home", egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(local_location(&harness), fake.home_dir());
    assert!(harness.state().side_panel_state().focused());
    harness.key_press(egui::Key::ArrowDown);
    harness.step();
    assert_eq!(
        harness.state().side_panel_state().focused_location(),
        Some(local(&fake.home_dir().join("Documents")))
    );
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(&mut harness);
    assert_eq!(local_location(&harness), fake.home_dir().join("Documents"));
    assert_eq!(tab_count(&harness), 1);
    harness.key_press(egui::Key::ArrowDown);
    harness.step();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    harness.step();
    settle(&mut harness);
    assert_eq!(tab_count(&harness), 2, "Mod+Enter opened a tab");
    assert_eq!(local_location(&harness), fake.home_dir().join("Pictures"));
    harness.key_press(egui::Key::ArrowUp);
    harness.key_press(egui::Key::ArrowUp);
    harness.step();
    assert_eq!(
        harness.state().side_panel_state().focused_location(),
        Some(local(&fake.home_dir()))
    );
    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(!harness.state().side_panel_state().focused());
}

#[test]
fn the_trash_view_lists_restores_undoes_and_empties() {
    let dir = tree();
    let (mut harness, fake) = harness(dir.path());
    select(&mut harness, "b.txt");
    run(&mut harness, "core.trash");
    settle(&mut harness);
    assert!(!dir.path().join("b.txt").exists());
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().trash.items.len() == 1
    }));
    harness.step();
    // The Trash row carries the count as a quiet hint.
    harness.get_by_label_contains("Trash");
    click_row(&mut harness, "Trash", egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(location(&harness), Virtual::Trash.location());
    assert_eq!(row_names(&harness), ["b.txt"]);

    // A click on the row takes the keyboard from the panel to the list;
    // Enter restores (Restore trash items on the keyboard).
    harness.get_by_label("b.txt").click();
    harness.step();
    harness.step();
    assert!(!harness.state().side_panel_state().focused());
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(&mut harness);
    assert!(dir.path().join("b.txt").exists(), "restored");
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().trash.items.is_empty() && row_names(h).is_empty()
    }));
    assert_eq!(
        harness.state().app().undo_label().as_deref(),
        Some("Restore b.txt")
    );

    // Undo trashes it again; the view follows.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.step();
    settle(&mut harness);
    assert!(!dir.path().join("b.txt").exists());
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        row_names(h) == ["b.txt"]
    }));

    // Empty trash lives in the view's menu, in the trash plugin's own
    // group, asks first, then empties.
    let rect = pane_rect(&harness);
    let empty_space = egui::pos2(rect.center().x, rect.bottom() - 80.0);
    click_at(&mut harness, empty_space, egui::PointerButton::Secondary);
    harness.get_by_label_contains("Trash ⏵").hover();
    harness.step();
    harness.step();
    harness.get_by_label("Empty trash").click();
    harness.step();
    harness.step();
    assert!(
        matches!(harness.state().asking(), Some(ui_egui::Ask::Confirm { title, .. }) if title == "Empty trash"),
        "{:?}",
        harness.state().asking()
    );
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(&mut harness);
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().trash.items.is_empty() && row_names(h).is_empty()
    }));
    assert_eq!(fake.list().map(|l| l.len()).ok(), Some(0));
    // Away from the view the group is hidden: the folder's menu has New
    // folder and no Trash group.
    let focused = pane(&harness);
    command(&mut harness, app::Command::Back(focused));
    settle(&mut harness);
    click_at(&mut harness, empty_space, egui::PointerButton::Secondary);
    harness.step();
    harness.get_by_label_contains("New folder");
    assert!(harness.query_by_label_contains("Trash ⏵").is_none());
    harness.key_press(egui::Key::Escape);
    harness.step();
}

#[test]
fn a_virtual_location_survives_the_session() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let config = fake.config_dir();
    {
        let mut harness = harness_with(dir.path(), &config, fake.clone(), Vec::new());
        click_row(&mut harness, "Favorites", egui::PointerButton::Primary);
        settle(&mut harness);
        session_contains(&mut harness, "rick:favorites");
        harness
            .state_mut()
            .app_mut()
            .save_session()
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake,
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(config),
        start: None,
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    assert_eq!(location(&harness), Virtual::Favorites.location());
    assert!(tab(&harness).note.is_none());
    // The tab shows the view's title and the breadcrumb its one segment.
    assert!(harness.query_all_by_label_contains("Favorites").count() >= 2);
}

/// Two copies that share a volume run one after the other, the second
/// shown as Waiting; copies on disjoint volumes run at the same time.
// One scenario after the other, in one window.
#[allow(clippy::too_many_lines)]
#[test]
fn copies_on_one_volume_run_in_turn_and_on_disjoint_volumes_together() {
    let dir = tree();
    let s3 = Arc::new(MemProvider::new(
        ProviderId("s3"),
        Duration::from_millis(600),
    ));
    let azure = Arc::new(MemProvider::new(
        ProviderId("azure"),
        Duration::from_millis(600),
    ));
    for provider in [&s3, &azure] {
        provider.file("/src/one.bin", b"1111");
        provider.file("/src/two.bin", b"2222");
        provider.dir("/dst");
        provider.dir("/other");
    }
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = harness_with(
        dir.path(),
        &fake.config_dir(),
        fake,
        vec![
            Arc::new(MemExtension::new(Arc::clone(&s3))),
            Arc::new(MemExtension::new(Arc::clone(&azure))),
        ],
    );
    let receive = |harness: &mut Harness<'_, RickUi>, provider: &str, name: &str, to: &str| {
        command(
            harness,
            app::Command::Receive {
                items: vec![Location::new(
                    ProviderId::intern(provider),
                    format!("/src/{name}"),
                )],
                to: Location::new(ProviderId::intern(provider), to),
                mode: app::TransferMode::Copy,
            },
        );
    };
    let states = |harness: &Harness<'_, RickUi>| -> Vec<(String, JobState)> {
        harness
            .state()
            .app()
            .model()
            .jobs
            .values()
            .map(|j| (j.name.clone(), j.state.clone()))
            .collect()
    };

    // Disjoint volumes: both run at once and both finish within one delay.
    let started = Instant::now();
    receive(&mut harness, "s3", "one.bin", "/dst");
    receive(&mut harness, "azure", "one.bin", "/dst");
    harness.step();
    assert!(
        states(&harness)
            .iter()
            .all(|(_, s)| matches!(s, JobState::Running(_))),
        "{:?}",
        states(&harness)
    );
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        states(h).iter().all(|(_, s)| *s == JobState::Done) && states(h).len() == 2
    }));
    let together = started.elapsed();
    assert!(
        together < Duration::from_millis(1_100),
        "two 600 ms copies on disjoint volumes took {together:?}"
    );
    assert!(s3.paths().contains(&"/dst/one.bin".to_owned()));
    assert!(azure.paths().contains(&"/dst/one.bin".to_owned()));
    let finished: Vec<types::JobId> = harness.state().app().model().jobs.keys().copied().collect();
    for id in finished {
        command(&mut harness, app::Command::DismissJob(id));
    }

    // One volume: the second waits for the first by name, then runs.
    let started = Instant::now();
    receive(&mut harness, "s3", "two.bin", "/dst");
    receive(&mut harness, "s3", "one.bin", "/other");
    assert!(
        step_until(&mut harness, Duration::from_secs(2), |h| {
            states(h).iter().any(|(name, s)| {
                name == "Copy one.bin to other"
                    && *s == JobState::Waiting("Copy two.bin to dst".into())
            })
        }),
        "{:?}",
        states(&harness)
    );
    harness.get_by_label_contains("Waiting for Copy two.bin to dst");
    assert!(
        !s3.paths().contains(&"/other/one.bin".to_owned()),
        "not yet"
    );
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        states(h).iter().all(|(_, s)| *s == JobState::Done) && states(h).len() == 2
    }));
    let in_turn = started.elapsed();
    assert!(
        in_turn >= Duration::from_millis(1_100),
        "two 600 ms copies on one volume took only {in_turn:?}"
    );
    assert!(s3.paths().contains(&"/other/one.bin".to_owned()));
}

/// Cancelling the running job starts the one queued behind it.
#[test]
fn cancelling_the_running_copy_starts_the_waiting_one() {
    let dir = tree();
    let s3 = Arc::new(MemProvider::new(ProviderId("s3"), Duration::from_secs(30)));
    s3.file("/src/slow.bin", b"slow");
    s3.file("/src/next.bin", b"next");
    s3.dir("/dst");
    s3.dir("/other");
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = harness_with(
        dir.path(),
        &fake.config_dir(),
        fake,
        vec![Arc::new(MemExtension::new(Arc::clone(&s3)))],
    );
    for (name, to) in [("slow.bin", "/dst"), ("next.bin", "/other")] {
        command(
            &mut harness,
            app::Command::Receive {
                items: vec![Location::new(ProviderId("s3"), format!("/src/{name}"))],
                to: Location::new(ProviderId("s3"), to),
                mode: app::TransferMode::Copy,
            },
        );
    }
    let job_named = |h: &Harness<'_, RickUi>, name: &str| -> Option<(types::JobId, JobState)> {
        h.state()
            .app()
            .model()
            .jobs
            .iter()
            .find(|(_, j)| j.name == name)
            .map(|(id, j)| (*id, j.state.clone()))
    };
    assert!(step_until(&mut harness, Duration::from_secs(2), |h| {
        matches!(
            job_named(h, "Copy next.bin to other"),
            Some((_, JobState::Waiting(_)))
        )
    }));
    let (slow, _) = job_named(&harness, "Copy slow.bin to dst").unwrap_or_else(|| panic!("slow"));
    command(&mut harness, app::Command::CancelJob(slow));
    assert!(step_until(&mut harness, Duration::from_secs(2), |h| {
        matches!(
            job_named(h, "Copy slow.bin to dst"),
            Some((_, JobState::Cancelled))
        )
    }));
    // The waiting job was cancelled too? No: it runs now, waiting on its
    // own 30 s delay.
    assert!(step_until(&mut harness, Duration::from_secs(2), |h| {
        matches!(
            job_named(h, "Copy next.bin to other"),
            Some((_, JobState::Running(_)))
        )
    }));
    let (next, _) = job_named(&harness, "Copy next.bin to other").unwrap_or_else(|| panic!("next"));
    command(&mut harness, app::Command::CancelJob(next));
    settle(&mut harness);
}

/// Many jobs at once: every row has its bar, the area keeps its cap and
/// the frames stay quick.
#[test]
fn the_operations_area_shows_a_bar_per_row_and_stays_responsive_with_many_jobs() {
    let dir = tree();
    let (mut harness, _fake) = harness(dir.path());
    let (go_tx, go_rx) = runtime::watch::channel(false);
    let jobs = harness.state().app().jobs().clone();
    let mut ids = Vec::new();
    for n in 0..8u64 {
        let mut go = go_rx.clone();
        let handle = jobs.spawn_named(format!("job {n}"), move |ctx| async move {
            let total = 2_000u64;
            let report = |done| {
                ctx.progress.report(types::Progress::Items {
                    done,
                    total: Some(total),
                });
            };
            // Paced by iterations, not by the clock: a 1 ms sleep per item
            // took the macOS runner, three cores shared by sixteen test
            // harnesses, past the budget below (CI run 42401751687).
            for done in 0..total / 2 {
                report(done);
                tokio::task::yield_now().await;
            }
            // Held at the midpoint while the frames are measured, reporting
            // the way a running job does.
            while !*go.borrow_and_update() {
                tokio::select! {
                    changed = go.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    () = tokio::time::sleep(Duration::from_millis(5)) => report(total / 2),
                }
            }
            for done in total / 2..total {
                report(done);
                tokio::task::yield_now().await;
            }
            Ok::<(), types::Error>(())
        });
        ids.push(handle.id);
    }
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().jobs.len() == 8
    }));
    // Frames while eight jobs report: each well under a tenth of a second,
    // even in a debug build.
    let mut worst = Duration::ZERO;
    for _ in 0..20 {
        let started = Instant::now();
        harness.step();
        worst = worst.max(started.elapsed());
    }
    assert!(worst < Duration::from_millis(100), "worst frame {worst:?}");
    // The header counts them and every job has a row with a bar of
    // `done of total`.
    harness.get_by_label_contains("8 jobs running");
    assert!(
        harness.query_all_by_label_contains("of 2000").count() >= 5,
        "the rows show done of total (at least the capped five are visible)"
    );
    let _ = go_tx.send(true);
    assert!(step_until(&mut harness, Duration::from_secs(20), |h| {
        h.state()
            .app()
            .model()
            .jobs
            .values()
            .all(|j| j.state == JobState::Done)
    }));
}

/// A job over the 200 000-entry fixture reports progress per entry; the
/// application's progress events, and so the repaints, come at most twenty
/// times a second per job, and polling never stalls the loop.
#[test]
#[ignore = "generates the 200k fixture; run with --ignored"]
fn progress_over_the_200k_fixture_is_throttled_to_twenty_events_a_second() {
    use futures::StreamExt as _;

    let fixture = tests::fixture::generate(tests::fixture::DEFAULT_ENTRIES, 7)
        .unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: Arc::new(EguiHost::new()),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(fixture.root())),
    });
    let provider = app
        .mounts()
        .provider(vfs_local::PROVIDER_ID)
        .unwrap_or_else(|| panic!("local"));
    let root = local(fixture.root());
    let handle = app
        .jobs()
        .spawn_named("Count the fixture", move |ctx| async move {
            let opts = ext::ListOptions {
                hidden: true,
                system: true,
                depth: ext::Depth::Unlimited,
            };
            let mut stream = provider.list(&root, opts, ctx.cancel.clone());
            let mut count = 0u64;
            while let Some(batch) = stream.next().await {
                for _ in batch? {
                    count += 1;
                    ctx.progress.report(types::Progress::Items {
                        done: count,
                        total: Some(tests::fixture::DEFAULT_ENTRIES as u64),
                    });
                }
            }
            Ok::<(), types::Error>(())
        });
    let started = Instant::now();
    let mut events = Vec::new();
    let mut progress_events = 0u64;
    let mut polls = 0u64;
    let mut worst_poll = Duration::ZERO;
    let mut done = false;
    let mut last_done = 0u64;
    while !done {
        let before = Instant::now();
        app.poll(&mut events);
        worst_poll = worst_poll.max(before.elapsed());
        polls += 1;
        for event in events.drain(..) {
            match event {
                app::Event::JobProgress { id, progress } if id == handle.id => {
                    progress_events += 1;
                    if let types::Progress::Items { done, .. } = progress {
                        last_done = done;
                    }
                }
                app::Event::JobDone { id, .. } if id == handle.id => done = true,
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
        assert!(
            started.elapsed() < Duration::from_mins(2),
            "the count never ended"
        );
    }
    let elapsed = started.elapsed();
    let allowed = u64::try_from(elapsed.as_millis() / 50).unwrap_or(u64::MAX) + 3;
    eprintln!(
        "{progress_events} progress events over {elapsed:?} ({polls} polls, worst {worst_poll:?}); last {last_done}"
    );
    assert!(
        progress_events <= allowed,
        "{progress_events} progress events in {elapsed:?}; at most {allowed} allowed"
    );
    assert!(progress_events >= 1);
    assert_eq!(
        last_done,
        tests::fixture::DEFAULT_ENTRIES as u64,
        "the last report arrived"
    );
    assert!(
        worst_poll < Duration::from_millis(50),
        "a poll took {worst_poll:?}"
    );
}

/// Opens the context menu on the file row `name` and says whether
/// *Mount* is in it, then closes the menu.
fn mount_offered(harness: &mut Harness<'_, RickUi>, name: &str) -> bool {
    let at = harness.get_by_label(name).rect().center();
    click_at(harness, at, egui::PointerButton::Secondary);
    harness.step();
    let offered = harness.query_by_label("Mount").is_some();
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    offered
}

/// The side panel rows whose label contains `text` but not the image
/// file's name: the volume's row, not the "Mount disc.iso" job row of
/// the operations area nor the file in the list.
fn volume_rows(harness: &Harness<'_, RickUi>, text: &str) -> Vec<egui::Rect> {
    let rect = pane_rect(harness);
    let file: Vec<egui::Rect> = harness
        .query_all_by_label_contains(&format!("{text}.iso"))
        .map(|n| n.rect())
        .collect();
    harness
        .query_all_by_label_contains(text)
        .map(|n| n.rect())
        .filter(|r| r.center().x < rect.left() && !file.contains(r))
        .collect()
}

/// Issue 51: *Mount* in a disk image's context menu, shown on an ISO on
/// every platform, on a VHDX only where Windows would show it, on a DMG
/// only as macOS would, never on a QCOW2, through the fake platform's
/// mountable formats; mounting an ISO adds a volume to This Machine, where
/// that row's Eject takes it away again.
#[test]
fn mount_shows_on_the_images_this_os_mounts_and_a_mounted_iso_is_a_volume_until_eject() {
    use platform::ImageFormat;

    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["disc.iso", "disk.vhdx", "disk.dmg", "disk.qcow2"] {
        fs::write(dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    let (mut harness, fake) = harness(dir.path());

    let as_platform: [(&str, &'static [ImageFormat], [bool; 4]); 3] = [
        (
            "Linux",
            &[ImageFormat::Iso, ImageFormat::Img],
            [true, false, false, false],
        ),
        (
            "Windows",
            &[ImageFormat::Iso, ImageFormat::Vhd, ImageFormat::Vhdx],
            [true, true, false, false],
        ),
        (
            "macOS",
            &[ImageFormat::Dmg, ImageFormat::Iso, ImageFormat::Img],
            [true, false, true, false],
        ),
    ];
    for (platform, mountable, expected) in as_platform {
        fake.set_mountable(mountable);
        let offered: Vec<bool> = ["disc.iso", "disk.vhdx", "disk.dmg", "disk.qcow2"]
            .iter()
            .map(|name| mount_offered(&mut harness, name))
            .collect();
        assert_eq!(offered, expected, "as {platform}: iso, vhdx, dmg, qcow2");
    }

    // Mount the ISO: the fake mounts, the live volume watch shows the
    // volume under This Machine within its poll.
    fake.set_mountable(&[ImageFormat::Iso, ImageFormat::Img]);
    assert_eq!(volume_rows(&harness, "disc"), Vec::<egui::Rect>::new());
    let at = harness.get_by_label("disc.iso").rect().center();
    click_at(&mut harness, at, egui::PointerButton::Secondary);
    harness.step();
    harness.get_by_label("Mount").click();
    harness.step();
    assert!(
        step_until(&mut harness, Duration::from_secs(6), |h| {
            h.state().app().model().volumes.len() == 1 && !volume_rows(h, "disc").is_empty()
        }),
        "the mounted image shows up as a volume"
    );
    assert_eq!(fake.mounted(), [dir.path().join("disc.iso")]);
    let volumes = harness.state().app().model().volumes.clone();
    assert_eq!(volumes.len(), 1);
    assert!(
        volumes[0].is_removable(),
        "a removable volume: Eject applies"
    );

    // Eject from the row's context menu unmounts it; no Unmount action.
    let row = volume_rows(&harness, "disc");
    assert_eq!(row.len(), 1, "{row:?}");
    click_at(
        &mut harness,
        row[0].center(),
        egui::PointerButton::Secondary,
    );
    harness.step();
    assert!(harness.query_by_label_contains("Unmount").is_none());
    harness.get_by_label_contains("Eject").click();
    harness.step();
    assert!(
        step_until(&mut harness, Duration::from_secs(6), |h| {
            h.state().app().model().volumes.is_empty() && volume_rows(h, "disc").is_empty()
        }),
        "the volume goes with Eject"
    );
    assert_eq!(fake.ejected(), [volumes[0].mount_point.clone()]);
}
