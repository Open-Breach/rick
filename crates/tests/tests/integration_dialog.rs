//! Acceptance (issue 120, the integration dialog of
//! specs/system-integration.md §6.1 and §6.2): the window asks for the
//! check after its first frame and shows *Set up Rick on this system*
//! once, with a switch per row; *Apply N items* performs the rows that
//! are on and records the ones switched off as declined, *Not now*
//! changes nothing and *Don't ask again* stores `ask = false`. Against a
//! scripted platform, so it runs the same on every system.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use model::{ItemState, ItemStatus};
use platform::integration::{Call, Check, Desktop, Item};
use platform::{Fake, FakeIntegration};
use tests::{local, read_until};
use ui_egui::{EguiHost, RickUi};

/// A platform whose check finds four items missing and one set up, on a
/// GNOME desktop where *Files* opens folders now.
fn scripted() -> (Arc<Fake>, FakeIntegration) {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let integration = FakeIntegration::new("Fedora Linux 43 · GNOME", Some(Desktop::Gnome));
    integration.set(Item::LauncherEntry, Check::missing());
    integration.set(
        Item::DefaultFileManager,
        Check {
            name: Some("Files".to_owned()),
            ..Check::missing()
        },
    );
    integration.set(Item::ShowInFolder, Check::set_up());
    integration.set(
        Item::RickCommand,
        Check {
            place: Some(PathBuf::from("/opt/tools/bin/rick")),
            ..Check::missing()
        },
    );
    integration.set(Item::DockFavourite, Check::missing());
    fake.set_integration(Arc::new(integration.clone()));
    (fake, integration)
}

/// One start of Rick's window on `config`.
fn start(fake: &Arc<Fake>, config: &Path) -> Harness<'static, RickUi> {
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(config.to_path_buf()),
        start: Some(local(config)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
}

/// Runs frames until no job, no check and no apply is in flight and a
/// quiet moment passed. A job that has ended is not enough to go by: the
/// answer it sent may still be waiting for the frame that folds it into
/// the model, and the check that follows an apply starts only then.
fn settle(harness: &mut Harness<'_, RickUi>) {
    let quiet = |harness: &Harness<'_, RickUi>| {
        let app = harness.state().app();
        let integration = &app.model().integration;
        !app.is_busy() && !integration.checking && integration.applying.is_none()
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for _ in 0..3 {
            harness.step();
        }
        if quiet(harness) {
            std::thread::sleep(Duration::from_millis(50));
            harness.step();
            if quiet(harness) {
                break;
            }
        }
        assert!(Instant::now() < deadline, "did not settle");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn click(harness: &mut Harness<'_, RickUi>, role: Role, label: &str) {
    let at = harness.get_by_role_and_label(role, label).rect().center();
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
}

fn open(harness: &Harness<'_, RickUi>) -> bool {
    harness.state().integration_dialog().is_open()
}

fn status(harness: &Harness<'_, RickUi>, item: Item) -> Option<ItemStatus> {
    harness
        .state()
        .app()
        .model()
        .integration
        .get(item)
        .map(|row| row.status.clone())
}

fn performed(integration: &FakeIntegration) -> Vec<Item> {
    integration
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Perform(item, _) => Some(item),
            _ => None,
        })
        .collect()
}

/// The titles of the four rows the scripted check lists, in order.
const ROWS: [&str; 4] = [
    "App launcher entry",
    "Default file manager",
    "rick command in the terminal",
    "Dock favourite",
];

#[test]
fn the_dialog_opens_once_and_apply_sets_up_the_rows_that_are_on() {
    let (fake, integration) = scripted();
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = start(&fake, config.path());
    assert!(open(&harness), "the dialog opens after the first frames");

    // The frame: the header, what the dialog is for, *Install for*, a
    // row per missing item with its line, the note and the footer.
    harness.get_by_label("Set up Rick on this system");
    harness.get_by_label("Fedora Linux 43 · GNOME — a few things are not set up yet");
    harness.get_by_label_contains("Choose what Rick may set up.");
    harness.get_by_label("Install for");
    let just_me = harness.get_by_role_and_label(Role::RadioButton, "Just me");
    assert_eq!(
        just_me.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    let all_users = harness.get_by_role_and_label(Role::RadioButton, "All users");
    assert!(
        all_users.accesskit_node().is_disabled(),
        "All users cannot be chosen yet"
    );
    for row in ROWS {
        let switch = harness.get_by_role_and_label(Role::CheckBox, row);
        assert_eq!(
            switch.accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::True),
            "{row} is on at first"
        );
    }
    assert!(
        harness
            .query_by_role_and_label(Role::CheckBox, "“Show in folder” from other apps")
            .is_none(),
        "what is set up is not asked about"
    );
    harness.get_by_label("Desktop entry and icon, so Rick appears in the Activities overview");
    harness.get_by_label("Open folders with Rick instead of Files (inode/directory)");
    harness.get_by_label(
        "Link in /opt/tools/bin, plus a shell helper so rick opens the current folder",
    );
    harness.get_by_label("Pin Rick to the GNOME dash");
    harness.get_by_label("Best effort");
    harness.get_by_label_contains("Nothing here needs a password.");
    let dont_ask = harness.get_by_role_and_label(Role::CheckBox, "Don’t ask again");
    assert_eq!(
        dont_ask.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    harness.get_by_role_and_label(Role::Button, "Not now");
    harness.get_by_role_and_label(Role::Button, "Apply 4 items");
    // Nothing was written by looking.
    assert_eq!(performed(&integration), [] as [Item; 0]);

    // All users takes no click.
    click(&mut harness, Role::RadioButton, "All users");
    let just_me = harness.get_by_role_and_label(Role::RadioButton, "Just me");
    assert_eq!(
        just_me.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );

    // A row switched off leaves the count, and is declined by Apply.
    click(&mut harness, Role::CheckBox, "Dock favourite");
    harness.get_by_role_and_label(Role::Button, "Apply 3 items");
    click(&mut harness, Role::Button, "Apply 3 items");
    settle(&mut harness);
    assert!(!open(&harness), "the dialog closes when the rows are done");
    assert_eq!(
        performed(&integration),
        [
            Item::LauncherEntry,
            Item::DefaultFileManager,
            Item::RickCommand
        ]
    );
    for item in [
        Item::LauncherEntry,
        Item::DefaultFileManager,
        Item::RickCommand,
    ] {
        assert_eq!(status(&harness, item), Some(ItemStatus::SetUp), "{item:?}");
    }
    assert_eq!(
        status(&harness, Item::DockFavourite),
        Some(ItemStatus::Declined)
    );
    let settings = harness.state().app().settings_path().to_path_buf();
    let text = read_until(&settings, |text| text.contains("declined"));
    assert!(
        text.contains("[integration.items.dock-favourite]\nstate = \"declined\""),
        "{text}"
    );
    drop(harness);

    // Started again: nothing to ask, the declined row included.
    let harness = start(&fake, config.path());
    assert!(!open(&harness), "no dialog at the next start");
    assert_eq!(
        harness
            .state()
            .app()
            .model()
            .settings
            .integration
            .record(Item::DockFavourite)
            .map(|record| record.state),
        Some(ItemState::Declined)
    );
}

#[test]
fn not_now_changes_nothing_and_dont_ask_again_silences_the_dialog() {
    let (fake, integration) = scripted();
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));

    // Not now: closed, nothing performed or declined, asked again.
    let mut harness = start(&fake, config.path());
    assert!(open(&harness));
    click(&mut harness, Role::Button, "Not now");
    settle(&mut harness);
    assert!(!open(&harness));
    assert_eq!(performed(&integration), [] as [Item; 0]);
    assert!(harness.state().app().model().settings.integration.ask);
    assert_eq!(
        status(&harness, Item::DockFavourite),
        Some(ItemStatus::Missing)
    );
    drop(harness);

    // The next start asks again; Don't ask again with Not now stores
    // `ask = false` and nothing else.
    let mut harness = start(&fake, config.path());
    assert!(open(&harness), "asked again at the next start");
    click(&mut harness, Role::CheckBox, "Don’t ask again");
    let dont_ask = harness.get_by_role_and_label(Role::CheckBox, "Don’t ask again");
    assert_eq!(
        dont_ask.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    click(&mut harness, Role::Button, "Not now");
    settle(&mut harness);
    assert!(!open(&harness));
    assert!(!harness.state().app().model().settings.integration.ask);
    assert_eq!(performed(&integration), [] as [Item; 0]);
    let settings = harness.state().app().settings_path().to_path_buf();
    let text = read_until(&settings, |text| text.contains("ask = false"));
    assert!(text.contains("[integration]"), "{text}");
    drop(harness);

    // From then on the dialog stays closed, with rows still missing.
    let harness = start(&fake, config.path());
    assert!(!open(&harness), "the dialog stays closed");
    assert_eq!(
        status(&harness, Item::LauncherEntry),
        Some(ItemStatus::Missing),
        "the check still ran, for Settings"
    );
}

#[test]
fn a_file_of_another_origin_is_named_and_replaced_only_when_ticked() {
    let (fake, integration) = scripted();
    let home = platform::ConfigDir::home_dir(&*fake);
    integration.set(
        Item::RickCommand,
        Check {
            place: Some(home.join(".local/bin/rick")),
            foreign: vec![home.join(".local/bin/rick")],
            ..Check::missing()
        },
    );
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = start(&fake, config.path());
    assert!(open(&harness));
    // The row says what is in its way and asks; until the answer it is
    // not among the rows Apply sets up.
    harness.get_by_label("~/.local/bin/rick is already there and was not written by Rick");
    let replace = harness.get_by_role_and_label(Role::CheckBox, "Replace");
    assert_eq!(
        replace.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    harness.get_by_role_and_label(Role::Button, "Apply 3 items");
    click(&mut harness, Role::CheckBox, "Replace");
    harness.get_by_role_and_label(Role::Button, "Apply 4 items");
    click(&mut harness, Role::Button, "Apply 4 items");
    settle(&mut harness);
    assert!(!open(&harness));
    let replaced: Vec<(Item, bool)> = integration
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Perform(item, request) => Some((item, request.replace)),
            _ => None,
        })
        .collect();
    assert_eq!(
        replaced,
        [
            (Item::LauncherEntry, false),
            (Item::DefaultFileManager, false),
            (Item::RickCommand, true),
            (Item::DockFavourite, false),
        ]
    );
    assert_eq!(status(&harness, Item::RickCommand), Some(ItemStatus::SetUp));
}

#[test]
fn a_row_left_unanswered_is_asked_about_again() {
    let (fake, integration) = scripted();
    let home = platform::ConfigDir::home_dir(&*fake);
    integration.set(
        Item::RickCommand,
        Check {
            place: Some(home.join(".local/bin/rick")),
            foreign: vec![home.join(".local/bin/rick")],
            ..Check::missing()
        },
    );
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = start(&fake, config.path());
    click(&mut harness, Role::Button, "Apply 3 items");
    settle(&mut harness);
    assert!(!open(&harness));
    assert_eq!(
        performed(&integration),
        [
            Item::LauncherEntry,
            Item::DefaultFileManager,
            Item::DockFavourite
        ],
        "the row with the file in its way was not touched"
    );
    assert_eq!(
        status(&harness, Item::RickCommand),
        Some(ItemStatus::Missing),
        "neither set up nor declined"
    );
    drop(harness);
    let harness = start(&fake, config.path());
    assert!(open(&harness), "that one row is asked about again");
    harness.get_by_role_and_label(Role::CheckBox, "rick command in the terminal");
    assert!(
        harness
            .query_by_role_and_label(Role::CheckBox, "App launcher entry")
            .is_none()
    );
}

#[test]
fn without_items_the_dialog_never_opens() {
    // What Windows and macOS have until their milestones: no item.
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let harness = start(&fake, config.path());
    assert!(!open(&harness));
    assert_eq!(
        harness.state().app().model().integration.items.len(),
        0,
        "nothing is listed"
    );
    assert!(
        harness
            .state()
            .app()
            .model()
            .integration
            .checked_at
            .is_some(),
        "the check ran after the first frame"
    );
}
