//! Acceptance (issue 120, Settings → Integration of
//! specs/system-integration.md §6.1): the category after *Appearance*
//! with the line naming the system and the last check, *Check again*,
//! every item with its pill — *Set up*, *Reset by system* with
//! *Restore*, *Skipped* with *Add*, *Unsupported* — the switches for
//! shell integration and the check at startup, and *Reset choices*.
//! Against a scripted platform, so it runs the same on every system.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use model::{ItemState, ItemStatus};
use platform::integration::{Call, Check, Desktop, Item, Reason};
use platform::{ConfigDir as _, Fake, FakeIntegration};
use tests::local;
use ui_egui::{EguiHost, RickUi};

/// A Rick installed and set up on a GNOME desktop, but for three rows:
/// the default file manager is no longer Rick, the dock favourite is not
/// pinned, and the service file cannot be written here.
fn scripted() -> (Arc<Fake>, FakeIntegration) {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let home = fake.home_dir();
    let integration = FakeIntegration::new("Fedora Linux 43 · GNOME", Some(Desktop::Gnome));
    integration.set(
        Item::InstallLocation,
        Check {
            place: Some(home.join(".local/opt/rick/0.3.0")),
            ..Check::set_up()
        },
    );
    integration.set(
        Item::LauncherEntry,
        Check {
            place: Some(home.join(".local/share")),
            ..Check::set_up()
        },
    );
    integration.set(
        Item::DefaultFileManager,
        Check {
            name: Some("Files".to_owned()),
            ..Check::missing()
        },
    );
    integration.set(
        Item::ShowInFolder,
        Check::unsupported(Reason::Program("dbus-daemon".to_owned())),
    );
    integration.set(
        Item::RickCommand,
        Check {
            place: Some(home.join(".local/bin/rick")),
            target: Some(home.join(".local/opt/rick/current/rick")),
            ..Check::set_up()
        },
    );
    integration.set(Item::DockFavourite, Check::missing());
    fake.set_integration(Arc::new(integration.clone()));
    (fake, integration)
}

/// What `rick.toml` holds of an earlier start: the default file manager
/// was set up, the dock favourite was switched off on 8 November, and
/// the dialog is silenced, so nothing opens over the settings.
const EARLIER: &str = "[integration]\nask = false\n\n\
    [integration.items.default-file-manager]\nstate = \"set-up\"\nchecked = 2026-11-08\n\n\
    [integration.items.dock-favourite]\nstate = \"declined\"\nchecked = 2026-11-08\n";

/// One start of Rick's window on `config`, with `settings` as its
/// `rick.toml` when given.
fn start(fake: &Arc<Fake>, config: &Path, settings: Option<&str>) -> Harness<'static, RickUi> {
    if let Some(settings) = settings {
        std::fs::write(config.join(app::SETTINGS_FILE), settings).unwrap_or_else(|e| panic!("{e}"));
    }
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

/// Scrolls the page to its end: the settings window is shorter than the
/// page with every item listed.
fn scroll_to_the_end(harness: &mut Harness<'_, RickUi>) {
    let over = harness.get_by_label("Terminal and links").rect().center();
    harness.event(egui::Event::PointerMoved(over));
    harness.step();
    for _ in 0..4 {
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -400.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    for _ in 0..3 {
        harness.step();
    }
}

/// Opens the settings on the Integration page.
fn open_page(harness: &mut Harness<'_, RickUi>) {
    harness.state_mut().open_settings();
    for _ in 0..3 {
        harness.step();
    }
    click(harness, Role::RadioButton, "Integration");
    settle(harness);
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

fn count(harness: &Harness<'_, RickUi>, label: &str) -> usize {
    harness.query_all_by_label(label).count()
}

fn calls(integration: &FakeIntegration, wanted: impl Fn(&Call) -> Option<Item>) -> Vec<Item> {
    integration.calls().iter().filter_map(wanted).collect()
}

fn performed(integration: &FakeIntegration) -> Vec<Item> {
    calls(integration, |call| match call {
        Call::Perform(item, _) => Some(*item),
        _ => None,
    })
}

fn checks(integration: &FakeIntegration) -> usize {
    calls(integration, |call| match call {
        Call::Check(item) => Some(*item),
        _ => None,
    })
    .len()
}

/// Every item of the scripted platform, with the line its state words,
/// its pill and, where its state has one, its button.
fn every_item_has_its_line_and_pill(harness: &Harness<'_, RickUi>) {
    // Every item, with the line its state words and its pill.
    for (title, line) in [
        ("Install location", "~/.local/opt/rick/0.3.0 · current"),
        (
            "App launcher entry",
            "Desktop entry and icon in ~/.local/share",
        ),
        (
            "Default file manager",
            "Was set up by Rick and has been changed or removed since",
        ),
        (
            "“Show in folder” from other apps",
            "Needs dbus-daemon, which is not installed",
        ),
        ("Dock favourite", "Skipped in the setup dialog on 8 Nov"),
        (
            "rick command",
            "~/.local/bin/rick → ~/.local/opt/rick/current/rick",
        ),
    ] {
        harness.get_by_label(title);
        harness.get_by_label(line);
    }
    assert_eq!(count(harness, "Set up"), 3, "three rows are set up");
    harness.get_by_label("Reset by system");
    harness.get_by_label("Skipped");
    harness.get_by_label("Unsupported");
    harness.get_by_role_and_label(Role::Button, "Restore: Default file manager");
    harness.get_by_role_and_label(Role::Button, "Add: Dock favourite");
    assert_eq!(
        harness
            .query_all_by_role(Role::Button)
            .filter(|node| {
                node.accesskit_node().label().is_some_and(|label| {
                    label.starts_with("Add: ") || label.starts_with("Restore: ")
                })
            })
            .count(),
        2,
        "what is set up or cannot be has no button"
    );
}

#[test]
fn the_page_shows_every_item_with_its_pill_and_restore_and_add_set_them_up() {
    let (fake, integration) = scripted();
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = start(&fake, config.path(), Some(EARLIER));
    assert!(
        !harness.state().integration_dialog().is_open(),
        "the dialog is silenced"
    );
    // The category sits after Appearance.
    harness.state_mut().open_settings();
    for _ in 0..3 {
        harness.step();
    }
    let appearance = harness
        .get_by_role_and_label(Role::RadioButton, "Appearance")
        .rect();
    let category = harness
        .get_by_role_and_label(Role::RadioButton, "Integration")
        .rect();
    assert!(category.top() >= appearance.bottom() - 1.0);
    assert!(
        category.top() < appearance.bottom() + 12.0,
        "right after it"
    );
    click(&mut harness, Role::RadioButton, "Integration");
    settle(&mut harness);

    // The line naming the system and the check, and *Check again*.
    harness.get_by_label_contains("Fedora Linux 43 · GNOME · checked at start, ");
    harness.get_by_role_and_label(Role::Button, "Check again");
    for group in ["This system", "Terminal and links", "Startup"] {
        harness.get_by_label(group);
    }

    every_item_has_its_line_and_pill(&harness);
    assert_eq!(performed(&integration), [] as [Item; 0]);

    // Restore sets the drifted row up again.
    click(&mut harness, Role::Button, "Restore: Default file manager");
    settle(&mut harness);
    assert_eq!(performed(&integration), [Item::DefaultFileManager]);
    assert_eq!(
        status(&harness, Item::DefaultFileManager),
        Some(ItemStatus::SetUp)
    );
    assert_eq!(count(&harness, "Set up"), 4);
    assert_eq!(count(&harness, "Reset by system"), 0);
    harness.get_by_label("Rick opens folders (inode/directory) and rick:// links");

    // Add sets the skipped row up, and it is watched like any other.
    click(&mut harness, Role::Button, "Add: Dock favourite");
    settle(&mut harness);
    assert_eq!(
        performed(&integration),
        [Item::DefaultFileManager, Item::DockFavourite]
    );
    assert_eq!(
        status(&harness, Item::DockFavourite),
        Some(ItemStatus::SetUp)
    );
    assert_eq!(
        harness
            .state()
            .app()
            .model()
            .settings
            .integration
            .record(Item::DockFavourite)
            .map(|record| record.state),
        Some(ItemState::SetUp)
    );
    assert_eq!(count(&harness, "Set up"), 5);
    assert_eq!(count(&harness, "Skipped"), 0);
    harness.get_by_label("Pinned to the GNOME dash");
}

#[test]
fn check_again_and_the_switches_do_what_they_say() {
    let (fake, integration) = scripted();
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = start(&fake, config.path(), Some(EARLIER));
    open_page(&mut harness);

    // Check again: every item that is not skipped is looked at once more,
    // and the line says the check was no longer the one at the start.
    let before = checks(&integration);
    click(&mut harness, Role::Button, "Check again");
    settle(&mut harness);
    assert_eq!(
        checks(&integration),
        before + 5,
        "the declined row is not checked"
    );
    assert_eq!(
        count(&harness, "Reset by system"),
        1,
        "what it found is the same"
    );
    assert!(
        harness
            .query_by_label_contains("checked at start")
            .is_none()
    );
    harness.get_by_label_contains("Fedora Linux 43 · GNOME · checked ");

    // The two preferences.
    let settings =
        |harness: &Harness<'_, RickUi>| harness.state().app().model().settings.integration.clone();
    assert!(settings(&harness).shell_integration);
    assert!(settings(&harness).check_at_startup);
    harness
        .get_by_label("The console follows the tab’s folder, and cd in the console moves the tab");
    harness.get_by_label("Shows the setup dialog when something is missing or was reset");
    click(&mut harness, Role::CheckBox, "Shell integration");
    assert!(!settings(&harness).shell_integration);
    scroll_to_the_end(&mut harness);
    click(&mut harness, Role::CheckBox, "Check integration at startup");
    assert!(!settings(&harness).check_at_startup);
    let path = harness.state().app().settings_path().to_path_buf();
    let text = tests::read_until(&path, |text| text.contains("check_at_startup = false"));
    assert!(text.contains("shell_integration = false"), "{text}");
    drop(harness);

    // With the check at startup off nothing is checked until the page is
    // opened, which asks for a check of its own.
    let before = checks(&integration);
    let mut harness = start(&fake, config.path(), None);
    assert_eq!(checks(&integration), before, "no check at this start");
    assert!(
        harness
            .state()
            .app()
            .model()
            .integration
            .checked_at
            .is_none()
    );
    open_page(&mut harness);
    assert!(checks(&integration) > before, "the page asked for one");
    harness.get_by_label_contains("Fedora Linux 43 · GNOME · checked ");
    harness.get_by_label("Reset by system");
}

#[test]
fn reset_choices_makes_the_dialog_ask_again() {
    let (fake, _integration) = scripted();
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = start(&fake, config.path(), Some(EARLIER));
    open_page(&mut harness);
    harness.get_by_label("Skipped items");
    harness.get_by_label("Ask again about items switched off in the setup dialog");
    scroll_to_the_end(&mut harness);
    click(&mut harness, Role::Button, "Reset choices");
    settle(&mut harness);
    let settings = harness.state().app().model().settings.integration.clone();
    assert!(settings.ask, "the dialog may ask again");
    assert!(!settings.declined(Item::DockFavourite));
    assert_eq!(
        status(&harness, Item::DockFavourite),
        Some(ItemStatus::Missing)
    );
    // The row is one that is simply not set up now.
    harness.get_by_label("Not set up");
    harness.get_by_label("Pin Rick to the GNOME dash");
    harness.get_by_role_and_label(Role::Button, "Add: Dock favourite");
    assert_eq!(count(&harness, "Skipped"), 0);
    let path = harness.state().app().settings_path().to_path_buf();
    // Reset is two changes, the rows and then `ask`, and the file is
    // written behind each: wait for the second.
    let text = tests::read_until(&path, |text| !text.contains("ask = false"));
    assert!(!text.contains("declined"), "{text}");
    drop(harness);

    // The next start asks about the two rows that are not set up.
    let harness = start(&fake, config.path(), None);
    assert!(harness.state().integration_dialog().is_open());
    for row in ["Default file manager", "Dock favourite"] {
        harness.get_by_role_and_label(Role::CheckBox, row);
    }
}

#[test]
fn a_file_of_another_origin_is_replaced_by_its_rows_button() {
    let (fake, integration) = scripted();
    let link: PathBuf = fake.home_dir().join(".local/bin/rick");
    integration.set(
        Item::RickCommand,
        Check {
            place: Some(link.clone()),
            foreign: vec![link],
            ..Check::missing()
        },
    );
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = start(&fake, config.path(), Some(EARLIER));
    open_page(&mut harness);
    // The row says what is in the way, and its button is the answer.
    harness.get_by_label("~/.local/bin/rick is already there and was not written by Rick");
    harness.get_by_role_and_label(Role::Button, "Replace: rick command");
    click(&mut harness, Role::Button, "Replace: rick command");
    settle(&mut harness);
    let replaced: Vec<(Item, bool)> = integration
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Perform(item, request) => Some((item, request.replace)),
            _ => None,
        })
        .collect();
    assert_eq!(replaced, [(Item::RickCommand, true)]);
    assert_eq!(status(&harness, Item::RickCommand), Some(ItemStatus::SetUp));
}

#[test]
fn without_items_the_page_has_the_status_line_and_the_preferences() {
    // Windows and macOS until their milestones: the check lists nothing.
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = start(&fake, config.path(), None);
    open_page(&mut harness);
    harness.get_by_label_contains("checked at start, ");
    harness.get_by_role_and_label(Role::Button, "Check again");
    assert!(harness.query_by_label("This system").is_none(), "no item");
    assert_eq!(count(&harness, "Set up"), 0);
    harness.get_by_label("Terminal and links");
    harness.get_by_role_and_label(Role::CheckBox, "Shell integration");
    harness.get_by_label("Startup");
    harness.get_by_role_and_label(Role::CheckBox, "Check integration at startup");
    harness.get_by_role_and_label(Role::Button, "Reset choices");
}
