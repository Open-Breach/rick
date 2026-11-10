//! Acceptance (issue 120, specs/system-integration.md §6.1 and §7.1): the
//! Linux integration items against a home of the test's own. Nothing here
//! touches the real system: every path is under a temporary directory
//! with its own data and config homes, `gsettings` is a map in memory,
//! and nothing runs `pkexec`.

#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event, IntegrationCommand};
use ext::{Level, UiHost};
use model::{ItemState, ItemStatus};
use platform::integration::{Actual, Item, Reason, Request, Scope};
use platform::{ConfigDir as _, Fake, Gsettings, Integration as _, LinuxEnv, LinuxIntegration};
use tests::read_until;
use types::Error;

/// `gsettings` in memory: the keys a test set.
#[derive(Debug, Default)]
struct Settings {
    values: Mutex<BTreeMap<String, String>>,
    /// Whether a write is refused, as on a desktop whose settings are
    /// locked down.
    refuse: AtomicBool,
    /// How many writes were asked for.
    writes: AtomicUsize,
}

impl Settings {
    fn value(&self, schema: &str, key: &str) -> Option<String> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&format!("{schema} {key}"))
            .cloned()
    }

    fn put(&self, schema: &str, key: &str, value: &str) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(format!("{schema} {key}"), value.to_owned());
    }
}

impl Gsettings for Settings {
    fn get(&self, schema: &str, key: &str) -> Result<String, Error> {
        self.value(schema, key)
            .ok_or(Error::Unsupported("no such key"))
    }

    fn set(&self, schema: &str, key: &str, value: &str) -> Result<(), Error> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        if self.refuse.load(Ordering::SeqCst) {
            return Err(Error::PermissionDenied);
        }
        self.put(schema, key, value);
        Ok(())
    }
}

/// A scratch folder with a Rick unpacked into it, as the release archive
/// unpacks — the binary, a library, a plugin and `share/` — and a home
/// of the test's own: the fake platform's, which is also where the
/// installer plugin installs to.
struct Scratch {
    dir: tempfile::TempDir,
    fake: Arc<Fake>,
    env: LinuxEnv,
    settings: Arc<Settings>,
}

impl Scratch {
    fn new(desktop: &str) -> Self {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let home = fake.home_dir();
        let unpacked = dir.path().join("Downloads/rick-0.3.0");
        fs::create_dir_all(unpacked.join("plugins/git")).unwrap_or_else(|e| panic!("{e}"));
        fs::create_dir_all(unpacked.join("share/applications")).unwrap_or_else(|e| panic!("{e}"));
        let write = |rel: &str, bytes: &[u8]| {
            fs::write(unpacked.join(rel), bytes).unwrap_or_else(|e| panic!("{rel}: {e}"));
        };
        // A program of this machine, as the installer verifies.
        write(
            "rick",
            &ext_installer::fixtures::tiny_elf(ext_installer::fixtures::this_machine(), &[]),
        );
        write("librick_kernel.so", b"kernel");
        write("plugins/git/plugin.toml", b"id = \"git\"\n");
        write(
            "share/applications/rick.desktop",
            b"[Desktop Entry]\nName=Rick\n",
        );
        let env = LinuxEnv::under(&home, &unpacked.join("rick"), desktop);
        Self {
            dir,
            fake,
            env,
            settings: Arc::default(),
        }
    }

    fn integration(&self) -> LinuxIntegration {
        LinuxIntegration::new(self.env.clone(), self.settings.clone())
    }

    /// Everything the test could have written to: the scratch folder and
    /// the home.
    fn everything(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut all = snapshot(self.dir.path());
        all.extend(snapshot(&self.env.home));
        all
    }

    /// The Rick this scratch starts from now on: the copy in the standard
    /// location, as after the relaunch.
    fn run_installed(&mut self, version: &str) {
        self.env.exe = self
            .env
            .home
            .join(".local/opt/rick")
            .join(version)
            .join("rick");
    }
}

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

/// One start of Rick from the scratch folder: the application, headless,
/// its platform the fake with the real Linux items on the scratch
/// environment, its config directory the scratch home's.
struct Start {
    app: App,
    events: Vec<Event>,
}

impl Start {
    fn new(scratch: &Scratch) -> Self {
        Self::with(scratch, Vec::new())
    }

    /// A start with the installer plugin loaded, as a release has it.
    fn with_installer(scratch: &Scratch) -> Self {
        Self::with(
            scratch,
            vec![Arc::new(ext_installer::InstallerExtension::new())],
        )
    }

    fn with(scratch: &Scratch, extensions: Vec<Arc<dyn ext::Extension>>) -> Self {
        scratch
            .fake
            .set_integration(Arc::new(scratch.integration()));
        let config = scratch.env.config_home.join("rick");
        let app = App::new(Config {
            platform: scratch.fake.clone(),
            ui: Arc::new(Quiet),
            extensions,
            config_dir: Some(config),
            start: Some(tests::local(scratch.dir.path())),
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

    /// The check after the first frame: whether it says to open the
    /// dialog.
    fn startup_check(&mut self) -> Option<bool> {
        self.send(IntegrationCommand::Check { startup: true })
            .iter()
            .find_map(|event| match event {
                Event::IntegrationChecked { ask } => Some(*ask),
                _ => None,
            })
    }

    /// The rows the dialog would list.
    fn asked(&self) -> Vec<Item> {
        self.app
            .model()
            .integration
            .asked()
            .map(|row| row.item)
            .collect()
    }

    fn status(&self, item: Item) -> Option<ItemStatus> {
        self.app
            .model()
            .integration
            .get(item)
            .map(|row| row.status.clone())
    }

    /// Ends this start once `rick.toml` holds what `written` waits for.
    fn exit(self, written: impl Fn(&str) -> bool) -> String {
        let path = self.app.settings_path().to_path_buf();
        let text = read_until(&path, written);
        drop(self);
        text
    }
}

/// The three rows this file applies; the others are switched off until
/// their own tests further down.
const ROWS: [Item; 4] = [
    Item::LauncherEntry,
    Item::DefaultFileManager,
    Item::ShowInFolder,
    Item::RickCommand,
];

fn apply(start: &mut Start, perform: &[Item], decline: &[Item], replace: &[Item]) -> Vec<Event> {
    start.send(IntegrationCommand::Apply {
        scope: Scope::User,
        perform: perform.to_vec(),
        decline: decline.to_vec(),
        replace: replace.to_vec(),
    })
}

/// Every file and link under `dir` with its contents, directories left
/// out: what an undo must bring back to what it was.
fn files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    snapshot(dir)
        .into_iter()
        .filter(|(path, _)| !path.is_dir() || path.is_symlink())
        .collect()
}

/// Every path under `dir`, with what a file holds or a link points at.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{e}"))
            .flatten()
        {
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).unwrap_or_else(|e| panic!("{e}"));
            if meta.is_dir() {
                out.push((path.clone(), Vec::new()));
                stack.push(path);
            } else if meta.is_symlink() {
                let target = fs::read_link(&path).unwrap_or_else(|e| panic!("{e}"));
                out.push((path, target.to_string_lossy().into_owned().into_bytes()));
            } else {
                out.push((
                    path.clone(),
                    fs::read(&path).unwrap_or_else(|e| panic!("{e}")),
                ));
            }
        }
    }
    out.sort();
    out
}

/// A Rick unpacked into a scratch folder has nothing set up: every item
/// the desktop supports is missing, the install location names where
/// Rick is and where it would go, and the check wrote nothing.
#[test]
fn the_check_of_an_unpacked_rick_finds_everything_missing_and_changes_nothing() {
    let scratch = Scratch::new("GNOME");
    let integration = scratch.integration();
    assert_eq!(integration.integration_items().items, Item::ALL);
    let before = scratch.everything();
    for item in [
        Item::InstallLocation,
        Item::LauncherEntry,
        Item::DefaultFileManager,
        Item::ShowInFolder,
        Item::RickCommand,
    ] {
        let check = integration.check_integration(item, &Request::default());
        assert_eq!(check.actual, Actual::Missing, "{item:?}");
        assert!(check.foreign.is_empty(), "{item:?}");
    }
    let location = integration.check_integration(Item::InstallLocation, &Request::default());
    assert_eq!(
        location.place,
        Some(scratch.env.home.join(".local/opt/rick"))
    );
    assert_eq!(
        location.target,
        Some(scratch.dir.path().join("Downloads/rick-0.3.0"))
    );
    assert_eq!(scratch.everything(), before, "the check is read-only");
}

/// On a desktop outside the four the dock row is *Unsupported*; the other
/// rows are what they are anywhere.
#[test]
fn on_a_desktop_outside_the_four_the_dock_row_is_unsupported() {
    let scratch = Scratch::new("XFCE");
    let integration = scratch.integration();
    assert_eq!(integration.integration_items().desktop, None);
    assert_eq!(
        integration
            .check_integration(Item::DockFavourite, &Request::default())
            .actual,
        Actual::Unsupported(Reason::Desktop)
    );
    assert_eq!(
        integration
            .check_integration(Item::LauncherEntry, &Request::default())
            .actual,
        Actual::Missing
    );
}

/// On GNOME the dock row is the last of the dialog; *Apply* with every
/// row on appends Rick to the dash's favourites, after the launcher
/// entry the favourite names, and the record says where.
#[test]
fn on_gnome_apply_pins_rick_to_the_dash() {
    let scratch = Scratch::new("GNOME");
    scratch.settings.put(
        "org.gnome.shell",
        "favorite-apps",
        "['org.gnome.Nautilus.desktop', 'firefox.desktop']",
    );
    let mut first = Start::new(&scratch);
    assert_eq!(first.startup_check(), Some(true));
    assert_eq!(first.asked().last(), Some(&Item::DockFavourite));
    assert_eq!(first.status(Item::DockFavourite), Some(ItemStatus::Missing));
    let mut rows = ROWS.to_vec();
    rows.push(Item::DockFavourite);
    let events = apply(&mut first, &rows, &[], &[]);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::IntegrationApplied { failed, .. } if failed.is_empty())),
        "{events:?}"
    );
    assert_eq!(
        scratch
            .settings
            .value("org.gnome.shell", "favorite-apps")
            .as_deref(),
        Some("['org.gnome.Nautilus.desktop', 'firefox.desktop', 'rick.desktop']")
    );
    assert_eq!(first.status(Item::DockFavourite), Some(ItemStatus::SetUp));
    let text = first.exit(|text| text.contains("dock-favourite"));
    assert!(
        text.contains("[integration.items.dock-favourite]\nstate = \"set-up\""),
        "{text}"
    );
    assert!(text.contains("\"org.gnome.shell favorite-apps\""), "{text}");

    // Started again there is nothing to ask; unpinned by hand, the row
    // has drifted and *Restore* pins Rick again.
    let mut second = Start::new(&scratch);
    assert_eq!(second.startup_check(), Some(false), "no dialog");
    drop(second);
    scratch
        .settings
        .put("org.gnome.shell", "favorite-apps", "['firefox.desktop']");
    let mut third = Start::new(&scratch);
    assert_eq!(third.startup_check(), Some(true));
    assert_eq!(third.asked(), [Item::DockFavourite]);
    assert_eq!(third.status(Item::DockFavourite), Some(ItemStatus::Drifted));
    third.send(IntegrationCommand::Restore {
        item: Item::DockFavourite,
        replace: false,
    });
    assert_eq!(
        scratch
            .settings
            .value("org.gnome.shell", "favorite-apps")
            .as_deref(),
        Some("['firefox.desktop', 'rick.desktop']")
    );
}

/// *Best effort*: a dock that cannot be pinned to is reported once, as a
/// warning, is not tried again, and the other rows are set up all the
/// same.
#[test]
fn a_dock_that_refuses_is_reported_and_blocks_nothing() {
    let scratch = Scratch::new("GNOME");
    scratch
        .settings
        .put("org.gnome.shell", "favorite-apps", "['firefox.desktop']");
    scratch.settings.refuse.store(true, Ordering::SeqCst);
    let mut start = Start::new(&scratch);
    start.startup_check();
    let mut rows = ROWS.to_vec();
    rows.push(Item::DockFavourite);
    let events = apply(&mut start, &rows, &[], &[]);
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::IntegrationApplied { failed, .. } if *failed == [Item::DockFavourite]
        )),
        "{events:?}"
    );
    let warnings: Vec<&String> = events
        .iter()
        .filter_map(|e| match e {
            Event::Notify {
                level: Level::Warning,
                text,
            } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(warnings.len(), 1, "{events:?}");
    assert!(
        !events.iter().any(|e| matches!(
            e,
            Event::Notify {
                level: Level::Error,
                ..
            }
        )),
        "{events:?}"
    );
    for item in ROWS {
        assert_eq!(start.status(item), Some(ItemStatus::SetUp), "{item:?}");
    }
    assert_in_place(&scratch.env.home, &scratch.env.exe);
    assert_eq!(start.status(Item::DockFavourite), Some(ItemStatus::Missing));
    assert_eq!(
        scratch.settings.writes.load(Ordering::SeqCst),
        1,
        "tried once"
    );
    assert_eq!(
        scratch
            .settings
            .value("org.gnome.shell", "favorite-apps")
            .as_deref(),
        Some("['firefox.desktop']")
    );
}

/// What the four rows leave in `home` for the Rick at `rick`: the
/// launcher entry with its icons, the defaults, the activation file, the
/// link and the shell function.
fn assert_in_place(home: &Path, rick: &Path) {
    // The launcher entry starts this Rick and is a valid desktop entry.
    let entry = home.join(".local/share/applications/rick.desktop");
    let text = fs::read_to_string(&entry).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        text.contains(&format!("Exec=\"{}\" %U\n", rick.display())),
        "{text}"
    );
    match ext_installer::desktop::validate(&entry) {
        Some(verdict) => assert_eq!(verdict, Ok(()), "desktop-file-validate"),
        None => assert!(
            std::env::var_os("RICK_REQUIRE_DESKTOP_FILE_VALIDATE").is_none(),
            "desktop-file-validate is required here but not installed"
        ),
    }
    for size in ["16x16", "48x48", "256x256", "512x512"] {
        assert!(
            home.join(format!(".local/share/icons/hicolor/{size}/apps/rick.png"))
                .is_file(),
            "{size}"
        );
    }
    assert!(
        home.join(".local/share/icons/hicolor/scalable/apps/rick.svg")
            .is_file()
    );
    // The defaults, as xdg-mime would have written them.
    assert_eq!(
        fs::read_to_string(home.join(".config/mimeapps.list")).unwrap_or_default(),
        "[Default Applications]\ninode/directory=rick.desktop\nx-scheme-handler/rick=rick.desktop\n"
    );
    // The activation file other apps' "Show in folder" starts Rick with.
    assert_eq!(
        fs::read_to_string(
            home.join(".local/share/dbus-1/services/org.freedesktop.FileManager1.service")
        )
        .unwrap_or_default(),
        format!(
            "[D-BUS Service]\nName=org.freedesktop.FileManager1\nExec={} --dbus-service\n",
            rick.display()
        )
    );
    // The command: the link, the function, the profile that sources it.
    assert_eq!(
        fs::read_link(home.join(".local/bin/rick")).ok(),
        Some(rick.to_path_buf())
    );
    let path_sh = fs::read_to_string(home.join(".config/rick/path.sh")).unwrap_or_default();
    assert!(
        path_sh.contains(&format!("rick() {{ \"{}\" \"$@\"; }}", rick.display())),
        "{path_sh}"
    );
    assert!(
        fs::read_to_string(home.join(".profile"))
            .unwrap_or_default()
            .contains("path.sh")
    );
}

/// *Apply* with the launcher entry, the default file manager, "Show in
/// folder" and the `rick` command switched on leaves the desktop entry,
/// the icons, the `mimeapps.list` defaults, the D-Bus service file, the
/// link and the shell function in place, each pointing at the Rick that
/// runs, and on record in `rick.toml`; starting again shows no dialog.
#[test]
fn apply_leaves_the_launcher_the_defaults_and_the_command_in_place() {
    let scratch = Scratch::new("XFCE");
    let rick = scratch.env.exe.clone();
    let home = scratch.env.home.clone();
    let mut first = Start::new(&scratch);
    assert_eq!(first.startup_check(), Some(true), "the dialog opens once");
    assert_eq!(
        first.asked(),
        [
            Item::LauncherEntry,
            Item::DefaultFileManager,
            Item::ShowInFolder,
            Item::RickCommand
        ],
        "no installer plugin and no dock Rick knows: both unsupported, the rest missing"
    );
    assert_eq!(
        first.status(Item::InstallLocation),
        Some(ItemStatus::Unsupported(Reason::Plugin(
            "Installer".to_owned()
        )))
    );
    let events = apply(&mut first, &ROWS, &[], &[]);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::IntegrationApplied { failed, .. } if failed.is_empty())),
        "{events:?}"
    );
    for item in ROWS {
        assert_eq!(first.status(item), Some(ItemStatus::SetUp), "{item:?}");
    }

    assert_in_place(&home, &rick);

    // On record, each with what it wrote.
    let text = first.exit(|text| text.contains("path.sh") && text.contains("dbus-1"));
    assert!(
        text.contains("[integration.items.launcher-entry]\nstate = \"set-up\""),
        "{text}"
    );
    assert!(
        text.contains("\"~/.local/share/dbus-1/services/org.freedesktop.FileManager1.service\""),
        "{text}"
    );
    assert!(
        text.contains("\"~/.local/share/applications/rick.desktop\""),
        "{text}"
    );
    assert!(text.contains("\"~/.local/bin/rick\""), "{text}");
    assert!(
        text.contains("[integration.items.default-file-manager.previous]"),
        "{text}"
    );

    // The same Rick started again: nothing to ask.
    let mut second = Start::new(&scratch);
    assert_eq!(second.startup_check(), Some(false), "no dialog");
    for item in ROWS {
        assert_eq!(second.status(item), Some(ItemStatus::SetUp), "{item:?}");
    }
}

/// Removing the launcher entry by hand and starting Rick lists that one
/// row as drifted; *Restore* puts it back.
#[test]
fn a_launcher_entry_removed_by_hand_is_drifted_until_restored() {
    let scratch = Scratch::new("XFCE");
    let mut first = Start::new(&scratch);
    first.startup_check();
    apply(&mut first, &ROWS, &[], &[]);
    first.exit(|text| text.contains("path.sh") && text.contains("dbus-1"));

    let entry = scratch
        .env
        .home
        .join(".local/share/applications/rick.desktop");
    fs::remove_file(&entry).unwrap_or_else(|e| panic!("{e}"));
    let mut second = Start::new(&scratch);
    assert_eq!(second.startup_check(), Some(true), "the dialog opens again");
    assert_eq!(second.asked(), [Item::LauncherEntry], "that one row");
    assert_eq!(
        second.status(Item::LauncherEntry),
        Some(ItemStatus::Drifted)
    );
    assert_eq!(
        second
            .app
            .model()
            .settings
            .integration
            .record(Item::LauncherEntry)
            .map(|record| record.state),
        Some(ItemState::SetUp)
    );
    second.send(IntegrationCommand::Restore {
        item: Item::LauncherEntry,
        replace: false,
    });
    assert_eq!(second.status(Item::LauncherEntry), Some(ItemStatus::SetUp));
    assert!(entry.is_file());
}

/// A `rick.desktop` and a `rick` link from somewhere else are never
/// overwritten silently: the rows know what is in their way, an apply
/// without the user's word leaves both as they are and says so, and with
/// it replaces them.
#[test]
fn files_of_another_origin_are_replaced_only_when_the_user_agreed() {
    let scratch = Scratch::new("XFCE");
    let home = scratch.env.home.clone();
    let entry = home.join(".local/share/applications/rick.desktop");
    let link = home.join(".local/bin/rick");
    fs::create_dir_all(entry.parent().unwrap_or(&home)).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(link.parent().unwrap_or(&home)).unwrap_or_else(|e| panic!("{e}"));
    let foreign = "[Desktop Entry]\nType=Application\nName=Rick\nExec=/opt/other/rick %U\n";
    fs::write(&entry, foreign).unwrap_or_else(|e| panic!("{e}"));
    std::os::unix::fs::symlink("/opt/other/rick", &link).unwrap_or_else(|e| panic!("{e}"));

    let mut start = Start::new(&scratch);
    start.startup_check();
    let in_the_way = |start: &Start, item: Item| {
        start
            .app
            .model()
            .integration
            .get(item)
            .map(|row| row.check.foreign.clone())
            .unwrap_or_default()
    };
    assert_eq!(
        in_the_way(&start, Item::LauncherEntry),
        std::slice::from_ref(&entry)
    );
    assert_eq!(
        in_the_way(&start, Item::RickCommand),
        std::slice::from_ref(&link)
    );

    let rows = [Item::LauncherEntry, Item::RickCommand];
    let events = apply(&mut start, &rows, &[], &[]);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::IntegrationApplied { failed, .. } if failed == &rows)),
        "{events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::Notify { level: Level::Error, text } if text.contains("was not written by Rick")
        )),
        "{events:?}"
    );
    assert_eq!(fs::read_to_string(&entry).unwrap_or_default(), foreign);
    assert_eq!(
        fs::read_link(&link).ok().as_deref(),
        Some(Path::new("/opt/other/rick"))
    );
    assert_eq!(start.status(Item::LauncherEntry), Some(ItemStatus::Missing));

    apply(&mut start, &rows, &[], &rows);
    assert_eq!(start.status(Item::LauncherEntry), Some(ItemStatus::SetUp));
    assert_eq!(start.status(Item::RickCommand), Some(ItemStatus::SetUp));
    assert_eq!(fs::read_link(&link).ok(), Some(scratch.env.exe.clone()));
    assert_eq!(in_the_way(&start, Item::LauncherEntry), [] as [PathBuf; 0]);
}

/// Every write is on record so it can be undone: undoing each item with
/// the receipt `rick.toml` keeps leaves the home as it was before the
/// apply, a profile and a `mimeapps.list` of the user's own included.
#[test]
fn what_apply_wrote_is_on_record_and_undo_removes_exactly_that() {
    let scratch = Scratch::new("XFCE");
    let home = scratch.env.home.clone();
    fs::create_dir_all(home.join(".config")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(home.join(".profile"), "export EDITOR=vi\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(
        home.join(".config/mimeapps.list"),
        "[Default Applications]\ninode/directory=thunar.desktop\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let before = files(&home);

    let mut start = Start::new(&scratch);
    start.startup_check();
    apply(&mut start, &ROWS, &[], &[]);
    for item in ROWS {
        assert_eq!(start.status(item), Some(ItemStatus::SetUp), "{item:?}");
    }
    let records = start.app.model().settings.integration.clone();
    start.exit(|text| text.contains("path.sh"));
    assert_ne!(
        files(&home)
            .into_iter()
            .filter(|(path, _)| !path.starts_with(home.join(".config/rick")))
            .collect::<Vec<_>>(),
        before
    );

    let integration = scratch.integration();
    for item in ROWS {
        let request = Request {
            receipt: records.receipt(item),
            ..Request::default()
        };
        assert!(!request.receipt.is_empty(), "{item:?} left a receipt");
        integration
            .undo_integration(item, &request)
            .unwrap_or_else(|e| panic!("{item:?}: {e}"));
        assert_eq!(
            integration.check_integration(item, &request).actual,
            Actual::Missing,
            "{item:?}"
        );
    }
    // Rick's own settings aside, the home is what it was.
    let after: Vec<(PathBuf, Vec<u8>)> = files(&home)
        .into_iter()
        .filter(|(path, _)| !path.starts_with(home.join(".config/rick")))
        .collect();
    assert_eq!(after, before);
}

/// A Rick unpacked from the archive into a scratch folder and started
/// shows the dialog once; *Apply* with the rows on leaves Rick under
/// `~/.local/opt/rick/<version>` with `current` pointing at it, and the
/// launcher entry, the defaults, the D-Bus service file and the `rick`
/// link in place, each on the installed copy; the apply ends naming that
/// copy to continue from, and the installed copy, started, shows no
/// dialog.
#[test]
fn apply_with_the_install_row_leaves_rick_in_the_standard_location() {
    let version = env!("CARGO_PKG_VERSION");
    let mut scratch = Scratch::new("XFCE");
    let home = scratch.env.home.clone();
    let mut first = Start::with_installer(&scratch);
    assert_eq!(first.startup_check(), Some(true), "the dialog opens once");
    assert_eq!(
        first.asked().first(),
        Some(&Item::InstallLocation),
        "the first row"
    );
    let rows = [
        Item::InstallLocation,
        Item::LauncherEntry,
        Item::DefaultFileManager,
        Item::ShowInFolder,
        Item::RickCommand,
    ];
    let events = apply(&mut first, &rows, &[], &[]);
    let installed = home.join(".local/opt/rick/current/rick");
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::IntegrationApplied { failed, relaunch }
                if failed.is_empty() && relaunch.as_deref() == Some(installed.as_path())
        )),
        "{events:?}"
    );

    // Rick, its library, its plugins and share/ in the version folder,
    // `current` pointing at it, the installer's own record beside them.
    let app_dir = home.join(".local/opt/rick");
    let folder = app_dir.join(version);
    for rel in [
        "rick",
        "librick_kernel.so",
        "plugins/git/plugin.toml",
        "share/applications/rick.desktop",
    ] {
        assert!(folder.join(rel).is_file(), "{rel}");
    }
    assert_eq!(
        fs::read_link(app_dir.join("current")).ok().as_deref(),
        Some(Path::new(version))
    );
    let record = fs::read_to_string(app_dir.join("rick-install.toml")).unwrap_or_default();
    assert!(record.contains("id = \"rick\""), "{record}");
    assert!(!record.contains("external_files"), "{record}");
    assert!(
        scratch.env.exe.is_file(),
        "the folder Rick ran from is copied, not moved"
    );

    // The other rows act on the installed copy, through `current`.
    assert_in_place(&home, &installed);
    // Until the relaunch, the copy stands for the running Rick.
    for item in rows {
        assert_eq!(first.status(item), Some(ItemStatus::SetUp), "{item:?}");
    }
    let text = first.exit(|text| text.contains("install-location") && text.contains("path.sh"));
    assert!(
        text.contains("[integration.items.install-location]\nstate = \"set-up\""),
        "{text}"
    );

    // The installed copy, started: nothing to ask.
    scratch.run_installed(version);
    let mut second = Start::with_installer(&scratch);
    assert_eq!(second.startup_check(), Some(false), "no dialog");
    for item in rows {
        assert_eq!(second.status(item), Some(ItemStatus::SetUp), "{item:?}");
    }
}

/// The installer plugin's command is the one the core asks for.
#[test]
fn the_core_and_the_installer_agree_on_the_command() {
    assert_eq!(
        app::INSTALL_RICK_COMMAND,
        ext_installer::actions::INSTALL_RICK_COMMAND
    );
    assert!(
        ext_installer::actions::COMMANDS
            .iter()
            .any(|spec| spec.name == app::INSTALL_RICK_COMMAND)
    );
    // The fake platform's home is where both sides put Rick.
    let scratch = Scratch::new("XFCE");
    assert_eq!(scratch.env.home, scratch.fake.home_dir());
}

/// What the relaunch hook was called with, and whether the session was
/// on disk by then.
type Relaunched = Arc<Mutex<Vec<(PathBuf, bool)>>>;

/// Rick's window on the scratch home, with the installer plugin and a
/// relaunch hook that records its calls.
fn window(
    scratch: &Scratch,
    relaunched: &Relaunched,
) -> egui_kittest::Harness<'static, ui_egui::RickUi> {
    scratch
        .fake
        .set_integration(Arc::new(scratch.integration()));
    let config = scratch.env.config_home.join("rick");
    let host = Arc::new(ui_egui::EguiHost::new());
    let app = App::new(Config {
        platform: scratch.fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(ext_installer::InstallerExtension::new())],
        config_dir: Some(config.clone()),
        start: Some(tests::local(scratch.dir.path())),
    });
    let seen = Arc::clone(relaunched);
    let session = config.join(app::SESSION_FILE);
    let hook: ui_egui::Relaunch = Arc::new(move |binary: &Path| {
        seen.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((binary.to_path_buf(), session.is_file()));
    });
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_eframe(move |cc| ui_egui::RickUi::new(app, host, &cc.egui_ctx).with_relaunch(hook));
    settle_window(&mut harness);
    harness
}

/// Runs frames until no check and no apply is in flight and a quiet
/// moment passed.
fn settle_window(harness: &mut egui_kittest::Harness<'_, ui_egui::RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for _ in 0..3 {
            harness.step();
        }
        let integration = &harness.state().app().model().integration;
        let idle = !integration.checking && integration.applying.is_none();
        if idle && !harness.state().app().is_busy() {
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

/// The first two acceptance lines, through the window: a Rick unpacked
/// into a scratch folder and started shows the dialog once; *Apply* with
/// every row on leaves Rick in the standard location with `current`
/// pointing at it and the launcher entry, the icons, the D-Bus service
/// file, the defaults, the `rick` link and the dock favourite in place,
/// each on the installed copy; the shell saves the session and hands the
/// installed binary to the relaunch hook, which in the binary starts the
/// copy and exits when it answers. The installed copy, started, shows no
/// dialog.
#[test]
fn through_the_dialog_every_row_on_installs_rick_and_continues_from_the_copy() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;

    let version = env!("CARGO_PKG_VERSION");
    let mut scratch = Scratch::new("GNOME");
    scratch
        .settings
        .put("org.gnome.shell", "favorite-apps", "['firefox.desktop']");
    let home = scratch.env.home.clone();
    let relaunched = Relaunched::default();

    let mut harness = window(&scratch, &relaunched);
    assert!(
        harness.state().integration_dialog().is_open(),
        "the dialog opens at the first start"
    );
    // Six rows, the install location first, every switch on.
    harness.get_by_label("Set up Rick on this system");
    let mut last = f32::NEG_INFINITY;
    for row in [
        "Install to the standard location",
        "App launcher entry",
        "Default file manager",
        "“Show in folder” from other apps",
        "rick command in the terminal",
        "Dock favourite",
    ] {
        let top = harness
            .get_by_role_and_label(Role::CheckBox, row)
            .rect()
            .top();
        assert!(top > last, "{row} follows the row before it");
        last = top;
    }
    harness.get_by_label_contains("to ~/.local/opt/rick and run it from there");
    let at = harness
        .get_by_role_and_label(Role::Button, "Apply 6 items")
        .rect()
        .center();
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
    settle_window(&mut harness);
    assert!(!harness.state().integration_dialog().is_open());

    // Rick in the standard location, `current` pointing at the version,
    // and every other row on the installed copy.
    let app_dir = home.join(".local/opt/rick");
    assert!(app_dir.join(version).join("rick").is_file());
    assert_eq!(
        fs::read_link(app_dir.join("current")).ok().as_deref(),
        Some(Path::new(version))
    );
    let installed = app_dir.join("current/rick");
    assert_in_place(&home, &installed);
    assert_eq!(
        scratch
            .settings
            .value("org.gnome.shell", "favorite-apps")
            .as_deref(),
        Some("['firefox.desktop', 'rick.desktop']")
    );
    // Rick continues from the installed copy: once, the session saved
    // before.
    let calls = |relaunched: &Relaunched| {
        relaunched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    };
    assert_eq!(calls(&relaunched), [(installed, true)]);
    let settings = harness.state().app().settings_path().to_path_buf();
    let text = read_until(&settings, |text| text.contains("dock-favourite"));
    assert!(
        text.contains("[integration.items.install-location]\nstate = \"set-up\""),
        "{text}"
    );
    drop(harness);

    // The installed copy, started: no dialog, and nothing more to hand
    // over.
    scratch.run_installed(version);
    let harness = window(&scratch, &relaunched);
    assert!(
        !harness.state().integration_dialog().is_open(),
        "starting the installed copy shows no dialog"
    );
    for item in Item::ALL {
        let status = harness.state().app().model().integration.get(item);
        assert_eq!(
            status.map(|row| row.status.clone()),
            Some(ItemStatus::SetUp),
            "{item:?}"
        );
    }
    assert_eq!(calls(&relaunched).len(), 1);
}
