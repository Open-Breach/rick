//! Acceptance (issue 44): the tarball installer, headless through the app
//! against the generated fixture archives on a temporary home.
//!
//! A GUI app with a desktop entry and an icon installs, its desktop
//! entry's `Exec` resolves through `current` and runs; a second version
//! updates it with `current` moving and the previous version kept; a
//! command link not created by Rick is refused until Overwrite is ticked;
//! a toolchain in PATH mode writes the shell snippet; uninstalling from
//! the Installed Apps view removes exactly the record's files and the app
//! folder, which a diff of the home asserts; a tarbomb, a wrong-machine
//! binary, a bad checksum and a source tree are handled as the spec says;
//! reading a 300 MB archive is a job the action returns from at once,
//! cancellable without a trace; and the plugin loads through the host
//! with its three points.
//!
//! Issue 223: *Run* on an AppImage, and Enter on it, set the execute bit
//! and start the file in its folder; one built for another machine is
//! refused untouched. *Install as application…* on an AppImage reads its
//! name, icon and version out of the file, installs the one file under
//! `current`, updates it from a file of another name without touching the
//! desktop entry, and uninstalls it without a trace, the downloads left
//! as they were.

use std::fs;
use std::io::{self, Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event};
use ext::{ActionId, Form, Invocation, Level, UiHost, Value, Values};
use ext_installer::fixtures::{
    AppImageFixture, Fixture, other_machine, this_machine, tiny_appimage, tiny_elf,
};
use ext_installer::record::{Layout, Record};
use model::{JobState, SelectionChange};
use platform::{ConfigDir, Fake};
use plugins_native::{Host, Library, MANIFEST_FILE, Options, PluginStatus, Source};
use tests::local;
use types::Progress;

/// A UI whose form answers the script sets in advance; confirms everything.
#[derive(Default)]
struct Scripted {
    form: Mutex<Vec<(&'static str, Value)>>,
    forms: AtomicUsize,
}

impl Scripted {
    fn answer_form(&self, values: Vec<(&'static str, Value)>) {
        *self.form.lock().unwrap_or_else(PoisonError::into_inner) = values;
    }

    /// How many forms were shown.
    fn forms_shown(&self) -> usize {
        self.forms.load(Ordering::SeqCst)
    }
}

impl UiHost for Scripted {
    fn notify(&self, _level: Level, _text: &str) {}
    fn confirm(&self, _title: &str, _text: &str) -> bool {
        true
    }
    fn prompt(&self, _title: &str, default: &str) -> Option<String> {
        Some(default.to_owned())
    }
    fn show(&self, _title: &str, _text: &str) {}
    fn form(&self, form: Form) -> Option<Values> {
        self.forms.fetch_add(1, Ordering::SeqCst);
        let mut values = form.defaults();
        for (key, value) in self
            .form
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            values.set(key, value);
        }
        Some(values)
    }
}

struct Driver {
    app: App,
    fake: Arc<Fake>,
    ui: Arc<Scripted>,
    events: Vec<Event>,
    pane: model::PaneId,
}

impl Driver {
    fn new(start: &Path) -> Self {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let ui = Arc::new(Scripted::default());
        let app = App::new(Config {
            platform: fake.clone(),
            ui: ui.clone(),
            extensions: vec![Arc::new(ext_installer::InstallerExtension::new())],
            config_dir: Some(fake.config_dir()),
            start: Some(local(start)),
        });
        let pane = app.model().focused;
        let mut driver = Self {
            app,
            fake,
            ui,
            events: Vec::new(),
            pane,
        };
        driver.settle();
        driver
    }

    fn step(&mut self, command: Command) -> Vec<Event> {
        self.events.clear();
        self.app.handle(command, &mut self.events);
        self.settle();
        std::mem::take(&mut self.events)
    }

    fn run(&mut self, action: &'static str) -> Vec<Event> {
        let ctx = self
            .app
            .action_ctx(self.pane, Invocation::ContextMenu)
            .unwrap_or_else(|| panic!("no active tab"));
        self.step(Command::Run {
            action: ActionId(action),
            ctx,
        })
    }

    /// Runs the loop until nothing arrives for a while and nothing runs.
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_mins(1);
        runtime::block_on(async {
            loop {
                let quiet = tokio::time::timeout(
                    Duration::from_millis(100),
                    self.app.tick(&mut self.events),
                )
                .await
                .is_err();
                if quiet && !self.app.is_busy() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "did not settle: {:?}",
                    self.app.model().jobs
                );
            }
        });
    }

    fn tab(&self) -> &model::Tab {
        self.app
            .model()
            .active_tab(self.pane)
            .map_or_else(|| panic!("no active tab"), |(_, tab)| tab)
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tab().listing.rows().map(|e| e.name.clone()).collect();
        names.sort();
        names
    }

    fn select(&mut self, names: &[&str]) {
        let ids: Vec<_> = names
            .iter()
            .map(|name| {
                self.tab()
                    .listing
                    .rows()
                    .find(|e| e.name == *name)
                    .map_or_else(|| panic!("no entry {name} in {:?}", self.names()), |e| e.id)
            })
            .collect();
        self.step(Command::Select {
            pane: self.pane,
            change: SelectionChange::Set(ids),
        });
    }

    /// Opens the entry named `name`, as Enter and a double-click do.
    fn open(&mut self, name: &str) -> Vec<Event> {
        let id = self
            .tab()
            .listing
            .rows()
            .find(|e| e.name == name)
            .map_or_else(|| panic!("no entry {name} in {:?}", self.names()), |e| e.id);
        self.step(Command::Open {
            pane: self.pane,
            id,
        })
    }

    /// Whether `action` is offered for the selection, as the context menu
    /// asks.
    fn offers(&self, action: &'static str) -> bool {
        let ctx = self
            .app
            .action_ctx(self.pane, Invocation::ContextMenu)
            .unwrap_or_else(|| panic!("no active tab"));
        self.app
            .registry()
            .actions()
            .iter()
            .find(|a| a.id() == ActionId(action))
            .is_some_and(|a| a.availability(&ctx) == ext::Availability::Enabled)
    }

    fn navigate(&mut self, to: types::Location) {
        self.step(Command::Navigate {
            pane: self.pane,
            to,
        });
    }

    fn layout(&self) -> Layout {
        Layout::user(&*self.fake)
    }

    fn current(&self, id: &str) -> String {
        fs::read_link(self.layout().current(id))
            .unwrap_or_else(|e| panic!("{id}: {e}"))
            .to_string_lossy()
            .into_owned()
    }

    fn record(&self, id: &str) -> Record {
        Record::read(&self.layout().record_path(id)).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Installs the selected archive with the form's defaults and the
    /// overrides, returning the events.
    fn install(&mut self, fixture: Fixture, overrides: Vec<(&'static str, Value)>) -> Vec<Event> {
        self.select(&[fixture.file_name()]);
        self.ui.answer_form(overrides);
        self.run("installer.install")
    }
}

/// The error notifications among `events`.
fn errors(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Notify {
                level: Level::Error,
                text,
            } => Some(text.clone()),
            Event::JobDone {
                result: Err(error), ..
            } => Some(error.to_string()),
            _ => None,
        })
        .collect()
}

/// Every regular file and link under `dir`, relative to it, sorted.
fn files_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).unwrap_or_else(|e| panic!("{e}"));
            if meta.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == "mimeinfo.cache") {
                // `update-desktop-database`'s own cache, which Rick asks it to
                // rebuild after a desktop entry comes or goes.
            } else {
                out.push(
                    path.strip_prefix(dir)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    out.sort();
    out
}

#[test]
#[allow(clippy::too_many_lines)]
fn install_launch_update_collide_path_mode_and_uninstall_through_the_app() {
    if !ext_installer::supported() {
        eprintln!("the installer is Linux only; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    ext_installer::fixtures::write_all(dir.path()).unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(dir.path());
    let home = d.fake.home_dir();
    let before = files_under(&home);
    let layout = d.layout();

    // Install the GUI app: the folder, the record, current, the desktop
    // entry, the icon and the command link.
    let events = d.install(Fixture::GuiWithDesktop, Vec::new());
    assert!(errors(&events).is_empty(), "{:?}", errors(&events));
    assert_eq!(d.current("fixture-gui"), "1.5.0");
    let record = d.record("fixture-gui");
    assert_eq!(record.name, "Fixture GUI");
    assert_eq!(record.external_files.len(), 3);
    let desktop_path = layout.desktop_path("fixture-gui");
    let desktop = fs::read_to_string(&desktop_path).unwrap_or_else(|e| panic!("{e}"));
    let entry = ext_installer::desktop::DesktopEntry::parse(&desktop);
    let exec = entry.get("Exec").unwrap_or_default().to_owned();
    assert!(
        exec.starts_with(&format!("\"{}", layout.current("fixture-gui").display())),
        "{exec}"
    );
    // Launch: resolve the Exec line's program and run it headlessly.
    let program = exec
        .trim_start_matches('"')
        .split('"')
        .next()
        .unwrap_or_default()
        .to_owned();
    let status = std::process::Command::new(&program)
        .status()
        .unwrap_or_else(|e| panic!("running {program}: {e}"));
    assert!(status.success(), "{status}");
    assert!(layout.icon_path("fixture-gui", "64x64", "png").is_file());
    let link = layout.bin_dir().join("fixture-gui");
    assert_eq!(
        fs::read_link(&link).ok(),
        Some(layout.current("fixture-gui").join("fixgui"))
    );

    // Update to a second version: current moves, the previous is kept.
    let events = d.install(Fixture::UpdateV2, Vec::new());
    assert!(errors(&events).is_empty(), "{:?}", errors(&events));
    assert_eq!(d.current("fixture-gui"), "2.0.0");
    let record = d.record("fixture-gui");
    assert_eq!(record.previous_version.as_deref(), Some("1.5.0"));
    assert!(layout.version_dir("fixture-gui", "1.5.0").is_dir());
    assert_eq!(
        fs::read_to_string(&desktop_path).unwrap_or_default(),
        desktop,
        "an update leaves the desktop entry alone"
    );

    // A command link of another origin is refused, then replaced.
    let foreign = layout.bin_dir().join("fixa");
    fs::write(&foreign, "#!/bin/sh\necho mine\n").unwrap_or_else(|e| panic!("{e}"));
    // Its three commands are equal, so the main executable is the user's
    // pick: the first.
    let events = d.install(Fixture::Toolchain, vec![("exe", Value::Choice(1))]);
    let refused = errors(&events);
    assert!(
        refused
            .iter()
            .any(|e| e.contains("was not created by Rick")),
        "{refused:?}"
    );
    assert!(!layout.app_dir("fixtool").exists());
    let events = d.install(
        Fixture::Toolchain,
        vec![("exe", Value::Choice(1)), ("overwrite", Value::Check(true))],
    );
    assert!(errors(&events).is_empty(), "{:?}", errors(&events));
    assert!(fs::read_link(&foreign).is_ok(), "replaced by the link");
    assert_eq!(d.record("fixtool").commands().len(), 4);

    // A tarbomb lands directly in its version folder.
    let events = d.install(
        Fixture::Tarbomb,
        vec![("version", Value::Text("3.1".into()))],
    );
    assert!(errors(&events).is_empty(), "{:?}", errors(&events));
    assert!(
        layout
            .version_dir("tarbomb", "3.1")
            .join("fixbomb")
            .is_file()
    );

    // Refused: the other machine's binary, a bad checksum, a source tree.
    let events = d.install(Fixture::WrongArch, Vec::new());
    assert!(
        errors(&events).iter().any(|e| e.contains("built for")),
        "{events:?}"
    );
    assert!(!layout.app_dir("fixarch").exists());
    let events = d.install(
        Fixture::GuiNoDesktop,
        vec![("sha256", Value::Text("f".repeat(64)))],
    );
    assert!(
        errors(&events).iter().any(|e| e.contains("SHA-256")),
        "{events:?}"
    );
    assert!(!layout.app_dir("fixnodesk").exists());
    let events = d.install(Fixture::Source, Vec::new());
    assert!(
        errors(&events).iter().any(|e| e.contains("source code")),
        "{events:?}"
    );

    // The Installed Apps view lists the three apps; uninstall them from it.
    let events = d.run("installer.apps");
    assert!(errors(&events).is_empty(), "{:?}", errors(&events));
    assert_eq!(d.tab().location, ext_installer::location());
    assert_eq!(d.names(), vec!["Fixture GUI", "fixtool", "tarbomb"]);
    for name in ["Fixture GUI", "fixtool", "tarbomb"] {
        d.select(&[name]);
        let events = d.run("installer.uninstall");
        assert!(errors(&events).is_empty(), "{name}: {:?}", errors(&events));
    }
    assert!(d.names().is_empty(), "{:?}", d.names());
    assert!(!layout.app_dir("fixture-gui").exists());
    assert!(!desktop_path.exists());
    assert!(fs::symlink_metadata(&link).is_err());

    // PATH mode writes the shell snippet; its uninstall removes it.
    d.navigate(local(dir.path()));
    let events = d.install(
        Fixture::Toolchain,
        vec![
            ("exe", Value::Choice(1)),
            ("path_mode", Value::Choice(1)),
            ("env_var", Value::Text("FIXTOOL_HOME".into())),
        ],
    );
    assert!(errors(&events).is_empty(), "{:?}", errors(&events));
    let snippet = fs::read_to_string(layout.path_file()).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        snippet.contains(&format!(
            "export PATH=\"{}:$PATH\"",
            layout.current("fixtool").join("bin").display()
        )),
        "{snippet}"
    );
    assert!(snippet.contains("export FIXTOOL_HOME="), "{snippet}");
    assert!(
        fs::read_to_string(home.join(".profile"))
            .unwrap_or_default()
            .contains("path.sh")
    );
    d.navigate(ext_installer::location());
    d.select(&["fixtool"]);
    let events = d.run("installer.uninstall");
    assert!(errors(&events).is_empty(), "{:?}", errors(&events));

    // The filesystem diff: the home holds what it held before.
    assert_eq!(files_under(&home), before, "nothing of Rick's remains");
}

/// Acceptance (issue 223, item 2), through the app with the plugin
/// loaded. An AppImage without the execute bit has *Run* in its menu, by
/// its name or by the magic in its header alone; *Run* starts it detached
/// in its folder, and the bit is set afterwards. Enter and a double-click
/// do exactly what *Run* does (decided 2026-11-15). A program that is no
/// AppImage and an archive have no *Run*, and an AppImage built for
/// another machine is refused: nothing is started and its bit stays off.
#[test]
fn run_and_enter_start_an_appimage_through_the_app() {
    if !ext_installer::supported() {
        eprintln!("the installer is Linux only; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().canonicalize().unwrap_or_else(|e| panic!("{e}"));
    let write = |name: &str, bytes: &[u8], mode: u32| {
        let path = root.join(name);
        fs::write(&path, bytes).unwrap_or_else(|e| panic!("{e}"));
        platform::fs::set_unix_mode(&path, mode).unwrap_or_else(|e| panic!("{e}"));
        path
    };
    let mode = |path: &Path| {
        platform::fs::unix_mode(path)
            .unwrap_or_else(|e| panic!("{e}"))
            .map(|mode| mode & 0o7777)
    };
    let image = tiny_appimage(this_machine(), 2, &[]);
    let downloaded = write("Fix.AppImage", &image, 0o644);
    let entered = write("fix-entered", &image, 0o600);
    let other = write(
        "Other.AppImage",
        &tiny_appimage(other_machine(), 2, &[]),
        0o644,
    );
    write("fix-tool", &tiny_elf(this_machine(), &[]), 0o644);
    Fixture::GuiWithDesktop
        .write(&root)
        .unwrap_or_else(|e| panic!("{e}"));
    let mut driver = Driver::new(&root);

    // Run, from the menu.
    driver.select(&["fix-tool"]);
    assert!(!driver.offers("installer.run"), "no AppImage");
    driver.select(&[Fixture::GuiWithDesktop.file_name()]);
    assert!(!driver.offers("installer.run"), "an archive");
    driver.select(&["Fix.AppImage"]);
    assert!(driver.offers("installer.run"));
    let events = driver.run("installer.run");
    assert!(errors(&events).is_empty(), "{:?}", errors(&events));
    assert_eq!(
        driver.fake.ran(),
        [(downloaded.clone(), Vec::new(), root.clone())]
    );
    assert_eq!(mode(&downloaded), Some(0o744));
    assert_eq!(fs::read(&downloaded).ok(), Some(image));

    // Enter, on one the magic alone tells.
    driver.select(&["fix-entered"]);
    assert!(driver.offers("installer.run"));
    let events = driver.open("fix-entered");
    assert!(errors(&events).is_empty(), "{:?}", errors(&events));
    assert_eq!(driver.fake.ran().len(), 2);
    assert_eq!(
        driver.fake.ran()[1],
        (entered.clone(), Vec::new(), root.clone())
    );
    assert_eq!(mode(&entered), Some(0o700));
    assert!(
        driver.fake.opened().is_empty(),
        "nothing went to the system"
    );

    // Another machine's: said so, and nothing more.
    for events in [
        {
            driver.select(&["Other.AppImage"]);
            driver.run("installer.run")
        },
        driver.open("Other.AppImage"),
    ] {
        let said = errors(&events);
        assert!(
            said.iter().any(|text| text.contains("built for")),
            "{said:?}"
        );
    }
    assert_eq!(driver.fake.ran().len(), 2, "nothing more was started");
    assert_eq!(mode(&other), Some(0o644));
}

/// Acceptance (issue 223, item 3), through the app with the plugin
/// loaded. An AppImage has *Run* and *Install as application…* in its
/// menu. The install reads the file in the job *Reading AppImage…* and
/// lands where an archive's lands: the one file under
/// `~/.local/opt/<id>/<version>/`, `current`, the record, the AppImage's
/// own desktop entry and icon. The launcher's entry starts the app
/// through `current`, the app shows in Installed Apps, an update from a
/// file of another name leaves the desktop entry as it is, and Uninstall
/// leaves the home as it was. The downloaded files keep their bytes and
/// stay without the execute bit.
#[test]
#[allow(clippy::too_many_lines)]
fn an_appimage_installs_updates_and_uninstalls_through_the_app() {
    if !ext_installer::supported() {
        eprintln!("the installer is Linux only; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().canonicalize().unwrap_or_else(|e| panic!("{e}"));
    let first = AppImageFixture::Entry
        .write(&root)
        .unwrap_or_else(|e| panic!("{e}"));
    let second = AppImageFixture::UpdateV2
        .write(&root)
        .unwrap_or_else(|e| panic!("{e}"));
    let downloaded = fs::read(&first).unwrap_or_else(|e| panic!("{e}"));
    let mode = |path: &Path| {
        platform::fs::unix_mode(path)
            .unwrap_or_else(|e| panic!("{e}"))
            .map(|mode| mode & 0o7777)
    };
    let mut d = Driver::new(&root);
    let home = d.fake.home_dir();
    let before = files_under(&home);
    let layout = d.layout();

    // Both entries, and the install.
    d.select(&[AppImageFixture::Entry.file_name()]);
    assert!(d.offers("installer.run") && d.offers("installer.install"));
    d.ui.answer_form(Vec::new());
    let events = d.run("installer.install");
    assert!(errors(&events).is_empty(), "{:?}", errors(&events));
    assert!(
        d.app
            .model()
            .jobs
            .values()
            .any(|job| job.name == ext_installer::actions::READING_APPIMAGE),
        "the file is read in a job of its own: {:?}",
        d.app.model().jobs
    );
    assert_eq!(d.current("fixapp"), "2.1.0");
    let installed = layout
        .version_dir("fixapp", "2.1.0")
        .join("fixapp.AppImage");
    assert_eq!(fs::read(&installed).ok().as_ref(), Some(&downloaded));
    assert_eq!(mode(&installed), Some(0o755));
    let record = d.record("fixapp");
    assert_eq!(record.name, "Fixapp");
    assert_eq!(record.main_executable, "fixapp.AppImage");
    assert_eq!(record.external_files.len(), 2, "the entry and the icon");
    assert!(layout.icon_path("fixapp", "64x64", "png").is_file());

    // The launcher's entry starts the app, through current.
    let desktop_path = layout.desktop_path("fixapp");
    let desktop = fs::read(&desktop_path).unwrap_or_else(|e| panic!("{e}"));
    let entry = ext_installer::desktop::DesktopEntry::parse(&String::from_utf8_lossy(&desktop));
    let exec = entry.get("Exec").unwrap_or_default().to_owned();
    let through = layout.current("fixapp").join("fixapp.AppImage");
    assert_eq!(exec, format!("\"{}\" %U", through.display()));
    assert_eq!(entry.get("Icon"), Some("fixapp"));
    let status = std::process::Command::new(&through)
        .status()
        .unwrap_or_else(|e| panic!("running {}: {e}", through.display()));
    assert!(status.success(), "{status}");

    // The download is where it was and as it was.
    assert_eq!(fs::read(&first).ok().as_ref(), Some(&downloaded));
    assert_eq!(mode(&first), Some(0o644));
    assert!(d.fake.ran().is_empty(), "the install ran nothing");

    // Installed Apps lists it.
    d.navigate(ext_installer::location());
    assert_eq!(d.names(), ["Fixapp"]);

    // The next version, from a file with another name: an update, and the
    // desktop entry is not touched.
    d.navigate(local(&root));
    d.select(&[AppImageFixture::UpdateV2.file_name()]);
    d.ui.answer_form(Vec::new());
    let events = d.run("installer.install");
    assert!(errors(&events).is_empty(), "{:?}", errors(&events));
    assert_eq!(d.current("fixapp"), "2.2.0");
    let record = d.record("fixapp");
    assert_eq!(record.previous_version.as_deref(), Some("2.1.0"));
    assert_eq!(record.source_archive, "update-v2.AppImage");
    assert_eq!(fs::read(&desktop_path).ok().as_ref(), Some(&desktop));
    assert!(installed.is_file(), "one previous version is kept");

    // Uninstall: the home is what it was, the downloads are still there.
    d.navigate(ext_installer::location());
    d.select(&["Fixapp"]);
    let events = d.run("installer.uninstall");
    assert!(errors(&events).is_empty(), "{:?}", errors(&events));
    assert_eq!(files_under(&home), before, "nothing of Rick's remains");
    assert!(first.is_file() && second.is_file());
    assert_eq!(mode(&second), Some(0o644));
}

/// Pseudo-random bytes, so a stored gzip stays as large as its contents.
struct Noise {
    left: u64,
    state: u64,
}

impl Read for Noise {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = usize::try_from(self.left.min(buf.len() as u64)).unwrap_or(0);
        for chunk in buf[..n].chunks_mut(8) {
            self.state ^= self.state << 13;
            self.state ^= self.state >> 7;
            self.state ^= self.state << 17;
            let bytes = self.state.to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        self.left -= n as u64;
        Ok(n)
    }
}

/// A gzip tar of about 300 MB: the shape of a GUI application (top folder
/// `Big App-1.0`, `bin/app`, `bin/app.png`), then three blobs of 100 MiB
/// of noise stored without compression, so the file on disk is as large as
/// its contents and reading it takes long enough to cancel. Returns the
/// path and the file's size.
fn big_archive(dir: &Path) -> (PathBuf, u64) {
    const BLOB: u64 = 100 << 20;
    let path = dir.join("Big.App-1.0.tar.gz");
    let file = fs::File::create(&path).unwrap_or_else(|e| panic!("{e}"));
    let encoder = flate2::write::GzEncoder::new(
        io::BufWriter::with_capacity(1 << 20, file),
        flate2::Compression::none(),
    );
    let mut tar = tar::Builder::new(encoder);
    let top = "Big App-1.0";
    let mut entry =
        |path: String, kind: tar::EntryType, mode: u32, size: u64, data: &mut dyn Read| {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(kind);
            header.set_mode(mode);
            header.set_size(size);
            header.set_mtime(1_700_000_000);
            tar.append_data(&mut header, path, data)
                .unwrap_or_else(|e| panic!("{e}"));
        };
    entry(
        format!("{top}/"),
        tar::EntryType::Directory,
        0o755,
        0,
        &mut io::empty(),
    );
    entry(
        format!("{top}/bin/"),
        tar::EntryType::Directory,
        0o755,
        0,
        &mut io::empty(),
    );
    let elf = ext_installer::fixtures::tiny_elf(ext_installer::fixtures::this_machine(), &[]);
    entry(
        format!("{top}/bin/app"),
        tar::EntryType::Regular,
        0o755,
        elf.len() as u64,
        &mut elf.as_slice(),
    );
    let png = ext_installer::fixtures::tiny_png(64);
    entry(
        format!("{top}/bin/app.png"),
        tar::EntryType::Regular,
        0o644,
        png.len() as u64,
        &mut png.as_slice(),
    );
    for i in 0..3u64 {
        entry(
            format!("{top}/data/blob{i}"),
            tar::EntryType::Regular,
            0o644,
            BLOB,
            &mut Noise {
                left: BLOB,
                state: 0x9e37_79b9_7f4a_7c15 ^ (i + 1),
            },
        );
    }
    let encoder = tar.into_inner().unwrap_or_else(|e| panic!("{e}"));
    let mut file = encoder.finish().unwrap_or_else(|e| panic!("{e}"));
    file.flush().unwrap_or_else(|e| panic!("{e}"));
    drop(file);
    let size = fs::metadata(&path).unwrap_or_else(|e| panic!("{e}")).len();
    (path, size)
}

/// Reading the archive is a job (specs/ext-installer.md "Detection is a
/// job"): *Install as application* returns at once, *Reading archive…*
/// shows in the operations table with byte progress against the file's
/// size, cancelling it leaves nothing behind and opens no form; left to
/// finish, the form opens and the install runs.
#[test]
fn reading_the_archive_is_a_job_that_returns_at_once_and_cancels_cleanly() {
    if !ext_installer::supported() {
        eprintln!("the installer is Linux only; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (archive, size) = big_archive(dir.path());
    assert!(size > 290 << 20, "{size} bytes");
    let mut d = Driver::new(dir.path());
    let home = d.fake.home_dir();
    let before = files_under(&home);
    let name = archive
        .file_name()
        .map_or_default(|n| n.to_string_lossy().into_owned());
    d.select(&[&name]);

    // The action returns at once; the reading is the job.
    let ctx = d
        .app
        .action_ctx(d.pane, Invocation::ContextMenu)
        .unwrap_or_else(|| panic!("no active tab"));
    let started = Instant::now();
    let mut events = Vec::new();
    d.app.handle(
        Command::Run {
            action: ActionId("installer.install"),
            ctx,
        },
        &mut events,
    );
    let took = started.elapsed();
    assert!(
        took < Duration::from_millis(16),
        "Install as application took {took:?}"
    );
    assert_eq!(d.ui.forms_shown(), 0, "the form waits for the job");
    let (id, info) = d
        .app
        .model()
        .jobs
        .iter()
        .find(|(_, job)| job.name == ext_installer::actions::READING_ARCHIVE)
        .map_or_else(
            || panic!("no Reading archive… job in {:?}", d.app.model().jobs),
            |(id, job)| (*id, job.clone()),
        );
    assert!(matches!(info.state, JobState::Running(_)), "{info:?}");

    // Cancelled: the job ends cancelled, no form opens, nothing is written.
    let events = d.step(Command::CancelJob(id));
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::JobDone { id: done, result: Err(types::Error::Cancelled) } if *done == id
        )),
        "{events:?}"
    );
    assert_eq!(
        d.app.model().jobs.get(&id).map(|job| job.state.clone()),
        Some(JobState::Cancelled)
    );
    assert_eq!(d.ui.forms_shown(), 0, "a cancelled read opens no form");
    assert_eq!(
        files_under(&home),
        before,
        "cancelling leaves nothing behind"
    );

    // Left to finish: byte progress against the file's size, then the
    // form, then the install.
    let events = d.run("installer.install");
    assert!(errors(&events).is_empty(), "{:?}", errors(&events));
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::JobProgress {
                progress: Progress::Bytes { done, total: Some(total) },
                ..
            } if *total == size && *done > 0
        )),
        "no byte progress against the file's size"
    );
    assert_eq!(d.ui.forms_shown(), 1);
    assert_eq!(d.current("big-app"), "1.0");
    assert_eq!(d.record("big-app").main_executable, "bin/app");
    assert!(
        d.layout()
            .version_dir("big-app", "1.0")
            .join("data/blob2")
            .is_file()
    );
}

/// The directory the test binary runs from, where cargo puts every
/// dependency's `cdylib` under its plain name.
fn deps_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    exe.parent()
        .unwrap_or_else(|| panic!("no parent of {}", exe.display()))
        .to_owned()
}

#[test]
fn the_installer_plugin_loads_through_the_host_on_every_platform() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    let library = deps_dir().join(format!(
        "{}ext_installer{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(library.is_file(), "not built: {}", library.display());
    let mut toml = plugins_native::describe(&library).unwrap_or_else(|e| panic!("{e}"));
    toml.library = Library::for_crate("ext_installer");
    let plugin_dir = plugins.join(&toml.id);
    toml.write(&plugin_dir.join(MANIFEST_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    let file = toml
        .library
        .for_os(std::env::consts::OS)
        .unwrap_or_else(|| panic!("no library name for this OS"));
    fs::copy(&library, plugin_dir.join(file)).unwrap_or_else(|e| panic!("{e}"));

    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Host::load(&Options {
        dirs: vec![(Source::BesideBinary, plugins)],
        platform: Some(fake),
        ..Options::default()
    });
    let plugin = host
        .plugins()
        .iter()
        .find(|p| p.id == "installer")
        .unwrap_or_else(|| panic!("no installer plugin in {:?}", host.plugins()));
    assert_eq!(plugin.status, PluginStatus::Loaded);
    assert_eq!(
        plugin.points,
        ext::Points::PROVIDER | ext::Points::DECORATOR | ext::Points::ACTION
    );
    let mut registry =
        ext::Registry::with_platform(Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}"))));
    for extension in host.extensions() {
        registry.load(&*extension).unwrap_or_else(|e| panic!("{e}"));
    }
    assert_eq!(registry.views().len(), 1);
    let ids: Vec<&str> = registry.actions().iter().map(|a| a.id().0).collect();
    assert!(
        ids.contains(&"installer.install") && ids.contains(&"installer.apps"),
        "{ids:?}"
    );
    // Off Linux the actions hide and the view says so.
    let ctx = ext::ActionCtx {
        pane: ext::PaneId(1),
        location: ext_installer::location(),
        selection: Vec::new(),
        invoked_from: Invocation::Palette,
        other: None,
    };
    let apps = registry
        .actions()
        .iter()
        .find(|a| a.id().0 == "installer.apps")
        .cloned()
        .unwrap_or_else(|| panic!("no installer.apps"));
    let mut elsewhere = ctx.clone();
    elsewhere.location = local(dir.path());
    if ext_installer::supported() {
        assert_eq!(apps.availability(&elsewhere), ext::Availability::Enabled);
    } else {
        assert_eq!(apps.availability(&elsewhere), ext::Availability::Hidden);
        let error = apps
            .run(elsewhere, &runtime::JobRunner::new(), &Scripted::default())
            .err()
            .map(|e| e.to_string());
        assert_eq!(
            error.as_deref(),
            Some("extension installer: unsupported on this platform")
        );
    }
}
