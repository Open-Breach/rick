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

use std::fs;
use std::io::{self, Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event};
use ext::{ActionId, Form, Invocation, Level, UiHost, Value, Values};
use ext_installer::fixtures::Fixture;
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
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
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
