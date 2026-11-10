//! Acceptance (issue 120, the integration check of
//! specs/system-integration.md §6): what every platform shares. The items
//! the platform lists, checked without a change, and the check, the
//! declined rows and the choices as the application keeps them across
//! restarts, against a scripted platform; the Linux items against a
//! temporary home are in `integration_linux.rs`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event, IntegrationCommand};
use ext::{Level, UiHost};
use model::{ItemState, ItemStatus, SettingsChange};
use platform::integration::{Actual, Call, Check, Desktop, Item, Request, Scope};
use platform::{Fake, FakeIntegration, Integration as _, Native};
use tests::read_until;

/// Nobody to ask: the integration asks through its own dialog.
struct Quiet;

impl UiHost for Quiet {
    fn notify(&self, _level: Level, _text: &str) {}
    fn confirm(&self, _title: &str, _text: &str) -> bool {
        false
    }
    fn prompt(&self, _title: &str, _default: &str) -> Option<String> {
        None
    }
    fn show(&self, _title: &str, _text: &str) {}
    fn form(&self, _form: ext::Form) -> Option<ext::Values> {
        None
    }
}

/// One start of Rick on a config directory: the application, headless,
/// on a platform whose integration the test scripted.
struct Start {
    app: App,
    events: Vec<Event>,
}

impl Start {
    fn new(fake: &Arc<Fake>, config: &Path) -> Self {
        let app = App::new(Config {
            platform: fake.clone(),
            ui: Arc::new(Quiet),
            extensions: Vec::new(),
            config_dir: Some(config.to_path_buf()),
            start: Some(tests::local(config)),
        });
        let mut start = Self {
            app,
            events: Vec::new(),
        };
        start.settle();
        start
    }

    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        runtime::block_on(async {
            loop {
                let quiet = tokio::time::timeout(
                    Duration::from_millis(80),
                    self.app.tick(&mut self.events),
                )
                .await
                .is_err();
                if quiet && !self.app.is_busy() {
                    break;
                }
                assert!(Instant::now() < deadline, "did not settle");
            }
        });
    }

    fn send(&mut self, command: IntegrationCommand) -> Vec<Event> {
        self.events.clear();
        self.app
            .handle(Command::Integration(command), &mut self.events);
        self.settle();
        std::mem::take(&mut self.events)
    }

    /// The check after the first frame; whether it says to open the
    /// dialog, `None` when no check ran.
    fn startup_check(&mut self) -> Option<bool> {
        self.send(IntegrationCommand::Check { startup: true })
            .iter()
            .find_map(|event| match event {
                Event::IntegrationChecked { ask } => Some(*ask),
                _ => None,
            })
    }

    fn status(&self, item: Item) -> Option<ItemStatus> {
        self.app
            .model()
            .integration
            .get(item)
            .map(|row| row.status.clone())
    }

    /// Ends this start with the settings on disk, as an exit does.
    fn exit(self, written: impl Fn(&str) -> bool) -> String {
        let path = self.app.settings_path().to_path_buf();
        let text = read_until(&path, written);
        drop(self);
        text
    }
}

/// A platform with the launcher entry and the dock favourite missing and
/// the `rick` command set up.
fn scripted() -> (Arc<Fake>, FakeIntegration) {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let integration = FakeIntegration::new("Fedora Linux 43 · GNOME", Some(Desktop::Gnome));
    integration.set(Item::LauncherEntry, Check::missing());
    integration.set(Item::RickCommand, Check::set_up());
    integration.set(Item::DockFavourite, Check::missing());
    fake.set_integration(Arc::new(integration.clone()));
    (fake, integration)
}

/// Linux has the six items of §7.1; Windows and macOS have none yet
/// (milestones 4 and 5), so nothing is listed there and nothing can be
/// performed.
#[test]
fn the_native_platform_lists_the_items_of_its_system() {
    let listed = Native::new().integration_items();
    assert_ne!(listed.system, "", "the line naming the system");
    if ext::Platforms::current() == ext::Platforms::LINUX {
        assert_eq!(listed.items, Item::ALL);
    } else {
        assert!(listed.items.is_empty(), "{listed:?}");
        assert_eq!(listed.desktop, None);
        for item in Item::ALL {
            assert!(
                Native::new()
                    .perform_integration(item, &Request::default())
                    .is_err(),
                "{item:?}"
            );
        }
    }
}

/// The fake platform lists nothing by itself, and answers with whatever
/// integration a test puts in its place.
#[test]
fn the_fake_platform_lists_what_a_test_scripts() {
    let fake = Fake::new().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(fake.integration_items().items, [] as [Item; 0]);

    let scripted = FakeIntegration::new("Fedora Linux 43 · GNOME", Some(Desktop::Gnome));
    scripted.set(Item::LauncherEntry, Check::missing());
    scripted.set(Item::RickCommand, Check::set_up());
    fake.set_integration(Arc::new(scripted.clone()));
    let listed = fake.integration_items();
    assert_eq!(listed.system, "Fedora Linux 43 · GNOME");
    assert_eq!(listed.desktop, Some(Desktop::Gnome));
    assert_eq!(listed.items, [Item::LauncherEntry, Item::RickCommand]);
    assert_eq!(
        fake.check_integration(Item::RickCommand, &Request::default())
            .actual,
        Actual::SetUp
    );
    // A check is a check: the scripted state is what it was.
    assert_eq!(scripted.actual(Item::LauncherEntry), Some(Actual::Missing));
    assert_eq!(scripted.calls(), [Call::Check(Item::RickCommand)]);
}

/// A row switched off is recorded as *declined*, is not asked about at
/// the next start and shows as *Skipped*; *Add* sets it up; *Reset
/// choices* makes the dialog ask again. Everything survives the restart
/// in `rick.toml`.
#[test]
fn a_row_switched_off_is_declined_across_restarts_until_added_or_reset() {
    let (fake, integration) = scripted();
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));

    // First start: two rows to ask about. One is applied, one switched off.
    let mut first = Start::new(&fake, config.path());
    assert_eq!(first.startup_check(), Some(true));
    let asked: Vec<Item> = first
        .app
        .model()
        .integration
        .asked()
        .map(|row| row.item)
        .collect();
    assert_eq!(asked, [Item::LauncherEntry, Item::DockFavourite]);
    first.send(IntegrationCommand::Apply {
        scope: Scope::User,
        perform: vec![Item::LauncherEntry],
        decline: vec![Item::DockFavourite],
        replace: Vec::new(),
    });
    assert_eq!(first.status(Item::LauncherEntry), Some(ItemStatus::SetUp));
    assert_eq!(
        first.status(Item::DockFavourite),
        Some(ItemStatus::Declined)
    );
    let text = first.exit(|text| text.contains("declined") && text.contains("fake/launcher-entry"));
    assert!(
        text.contains("[integration.items.dock-favourite]\nstate = \"declined\""),
        "{text}"
    );
    assert!(
        text.contains("[integration.items.launcher-entry]\nstate = \"set-up\""),
        "{text}"
    );

    // Second start: nothing to ask, the declined row is not even read.
    let checks_before = integration.calls().len();
    let mut second = Start::new(&fake, config.path());
    assert_eq!(second.startup_check(), Some(false), "no dialog");
    assert_eq!(
        second.status(Item::DockFavourite),
        Some(ItemStatus::Declined)
    );
    assert!(
        !integration.calls()[checks_before..].contains(&Call::Check(Item::DockFavourite)),
        "declined is not watched"
    );
    // *Add* in Settings → Integration.
    second.send(IntegrationCommand::Add {
        item: Item::DockFavourite,
        replace: false,
    });
    assert_eq!(second.status(Item::DockFavourite), Some(ItemStatus::SetUp));
    assert_eq!(integration.actual(Item::DockFavourite), Some(Actual::SetUp));

    // Declined once more, then *Reset choices*: asked at the next start.
    integration.set(Item::DockFavourite, Check::missing());
    second.send(IntegrationCommand::Apply {
        scope: Scope::User,
        perform: Vec::new(),
        decline: vec![Item::DockFavourite],
        replace: Vec::new(),
    });
    assert_eq!(second.startup_check(), Some(false));
    second.send(IntegrationCommand::Reset);
    assert_eq!(
        second.status(Item::DockFavourite),
        Some(ItemStatus::Missing)
    );
    second.exit(|text| !text.contains("declined"));
    let mut third = Start::new(&fake, config.path());
    assert_eq!(third.startup_check(), Some(true), "the dialog asks again");
}

/// *Don't ask again* stores `ask = false`; the dialog stays closed at the
/// next start while the check, and with it Settings → Integration, still
/// works. *Check integration at startup* off leaves the start alone.
#[test]
fn dont_ask_again_keeps_the_dialog_closed_and_the_check_working() {
    let (fake, integration) = scripted();
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut first = Start::new(&fake, config.path());
    assert_eq!(first.startup_check(), Some(true));
    first.app.handle(
        Command::Settings(SettingsChange::IntegrationAsk(false)),
        &mut Vec::new(),
    );
    let text = first.exit(|text| text.contains("ask = false"));
    assert!(text.contains("[integration]\nask = false"), "{text}");

    let mut second = Start::new(&fake, config.path());
    assert_eq!(
        second.startup_check(),
        Some(false),
        "the dialog stays closed"
    );
    assert_eq!(
        second.status(Item::LauncherEntry),
        Some(ItemStatus::Missing)
    );
    // Settings → Integration still works: Add on the missing row.
    second.send(IntegrationCommand::Add {
        item: Item::LauncherEntry,
        replace: false,
    });
    assert_eq!(second.status(Item::LauncherEntry), Some(ItemStatus::SetUp));
    second.app.handle(
        Command::Settings(SettingsChange::IntegrationCheckAtStartup(false)),
        &mut Vec::new(),
    );
    second.exit(|text| text.contains("check_at_startup = false"));

    let checks_before = integration.calls().len();
    let mut third = Start::new(&fake, config.path());
    assert_eq!(third.startup_check(), None, "no check at startup");
    assert_eq!(integration.calls().len(), checks_before);
    // *Check again* is asked for, not automatic.
    let events = third.send(IntegrationCommand::Check { startup: false });
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::IntegrationChecked { ask: false }))
    );
    assert!(third.app.model().integration.checked_at.is_some());
}

/// With everything set up, a start shows no dialog; an item removed behind
/// Rick's back is listed as drifted at the next start, and the record
/// keeps saying what was reached.
#[test]
fn a_set_up_system_is_not_asked_and_drift_is() {
    let (fake, integration) = scripted();
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut first = Start::new(&fake, config.path());
    first.startup_check();
    first.send(IntegrationCommand::Apply {
        scope: Scope::User,
        perform: vec![Item::LauncherEntry, Item::DockFavourite],
        decline: Vec::new(),
        replace: Vec::new(),
    });
    first.exit(|text| text.contains("fake/dock-favourite"));

    let mut second = Start::new(&fake, config.path());
    assert_eq!(second.startup_check(), Some(false), "nothing missing");
    second.exit(|_| true);

    integration.set(Item::LauncherEntry, Check::missing());
    let mut third = Start::new(&fake, config.path());
    assert_eq!(third.startup_check(), Some(true));
    let asked: Vec<Item> = third
        .app
        .model()
        .integration
        .asked()
        .map(|row| row.item)
        .collect();
    assert_eq!(asked, [Item::LauncherEntry], "that one row");
    assert_eq!(third.status(Item::LauncherEntry), Some(ItemStatus::Drifted));
    assert_eq!(
        third
            .app
            .model()
            .settings
            .integration
            .record(Item::LauncherEntry)
            .map(|record| record.state),
        Some(ItemState::SetUp)
    );
    third.send(IntegrationCommand::Restore {
        item: Item::LauncherEntry,
        replace: false,
    });
    assert_eq!(third.status(Item::LauncherEntry), Some(ItemStatus::SetUp));
}

/// A platform without items (Windows and macOS until their milestones):
/// the check answers an empty list under the system's line and never asks.
#[test]
fn without_items_the_check_is_empty_and_the_dialog_never_opens() {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let config = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut start = Start::new(&fake, config.path());
    assert_eq!(start.startup_check(), Some(false));
    let integration = &start.app.model().integration;
    assert_eq!(integration.items, []);
    assert_ne!(integration.system, "");
    assert!(integration.checked_at.is_some());
}
