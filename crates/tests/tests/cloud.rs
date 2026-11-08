//! Acceptance (issue 21): providers as plugins, SFTP and the clouds.
//!
//! The kernel's side is proven on every OS with the in-memory provider in
//! its native mode: a move within one account, and one across two buckets
//! of it, goes through `transfer` and streams nothing through `open_read`,
//! while a copy to the local disk streams every byte; the side panel's
//! *Add connection…* opens the plugin's dialogue, and a connection's row
//! connects and removes through its menu, the secret in the keychain and
//! never in `rick.toml`; the four plugins load through the host like the
//! others. Against the containers (`RICK_S3_ENDPOINT` and friends, set by
//! CI on Linux; skipped with a note elsewhere) a 100 MB upload streams
//! with progress and, cancelled half-way, leaves no object, and the
//! plugins' own round trips (list, download, upload, rename, delete, moves
//! within and across buckets) run in their crates.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event, TransferMode};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::{Extension, Points};
use model::JobState;
use platform::{ConfigDir, Fake, Secrets as _};
use plugins_native::{Host, Library, MANIFEST_FILE, Options, PluginStatus, Source};
use tests::local;
use tests::mem::{MemExtension, MemProvider};
use types::{Location, ProviderId};
use ui_egui::{EguiHost, RickUi};

/// A UI whose answers are fixed.
#[derive(Default)]
struct Scripted;

impl ext::UiHost for Scripted {
    fn notify(&self, _level: ext::Level, _text: &str) {}
    fn confirm(&self, _title: &str, _text: &str) -> bool {
        true
    }
    fn prompt(&self, _title: &str, default: &str) -> Option<String> {
        Some(default.to_owned())
    }
    fn show(&self, _title: &str, _text: &str) {}
    fn form(&self, form: ext::Form) -> Option<ext::Values> {
        Some(form.defaults())
    }
}

/// An application over `fake` with `extensions` besides the core, driven
/// without a window.
struct Driver {
    app: App,
    fake: Arc<Fake>,
    events: Vec<Event>,
}

impl Driver {
    fn new(start: &Path, extensions: Vec<Arc<dyn Extension>>) -> Self {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let app = App::new(Config {
            platform: fake.clone(),
            ui: Arc::new(Scripted),
            extensions,
            config_dir: Some(fake.config_dir()),
            start: Some(local(start)),
        });
        let mut driver = Self {
            app,
            fake,
            events: Vec::new(),
        };
        driver.settle();
        driver
    }

    fn send(&mut self, command: Command) {
        self.app.handle(command, &mut self.events);
    }

    /// Waits until nothing has arrived for a while and nothing runs.
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
                assert!(Instant::now() < deadline, "did not settle: {:?}", self.app);
            }
        });
    }

    /// Runs the loop until `done`, within `limit`.
    fn run_until(&mut self, limit: Duration, mut done: impl FnMut(&App) -> bool) -> bool {
        let deadline = Instant::now() + limit;
        runtime::block_on(async {
            while Instant::now() < deadline {
                let _ = tokio::time::timeout(
                    Duration::from_millis(50),
                    self.app.tick(&mut self.events),
                )
                .await;
                if done(&self.app) {
                    return true;
                }
            }
            done(&self.app)
        })
    }
}

/// A stand-in cloud account: `s3` with two buckets and a tree in one.
fn store() -> (Arc<MemProvider>, Arc<dyn Extension>) {
    let s3 = Arc::new(MemProvider::new(ProviderId("s3"), Duration::from_millis(5)));
    s3.set_native(true);
    s3.dir("/bucket-a/photos/raw");
    s3.file("/bucket-a/photos/a.jpg", &vec![1u8; 300_000]);
    s3.file("/bucket-a/photos/raw/b.raw", &vec![2u8; 200_000]);
    s3.file("/bucket-a/notes.txt", b"hello");
    s3.dir("/bucket-a/archive");
    s3.dir("/bucket-b");
    let ext: Arc<dyn Extension> = Arc::new(MemExtension::new(Arc::clone(&s3)));
    (s3, ext)
}

fn at(path: &str) -> Location {
    Location::new(ProviderId("s3"), path)
}

#[test]
fn a_move_within_an_account_streams_nothing_within_and_across_buckets() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (s3, ext) = store();
    let mut d = Driver::new(dir.path(), vec![ext]);

    // A folder and a file moved into another folder of the same bucket.
    d.send(Command::Receive {
        items: vec![at("/bucket-a/photos"), at("/bucket-a/notes.txt")],
        to: at("/bucket-a/archive"),
        mode: TransferMode::Move,
    });
    d.settle();
    assert_eq!(s3.read_bytes(), 0, "nothing streamed through the machine");
    assert_eq!(
        s3.transfers(),
        [(
            ext::TransferMode::Move,
            vec![
                "/bucket-a/photos".to_owned(),
                "/bucket-a/notes.txt".to_owned()
            ],
            "/bucket-a/archive".to_owned()
        )],
        "one call for the whole selection"
    );
    assert!(
        s3.paths()
            .contains(&"/bucket-a/archive/photos/raw/b.raw".to_owned())
    );
    assert!(
        s3.paths()
            .contains(&"/bucket-a/archive/notes.txt".to_owned())
    );
    assert!(!s3.paths().contains(&"/bucket-a/photos/a.jpg".to_owned()));

    // The undo moves back natively too.
    d.send(Command::Undo);
    d.settle();
    assert_eq!(s3.read_bytes(), 0);
    assert_eq!(s3.transfers().len(), 2);
    assert!(s3.paths().contains(&"/bucket-a/photos/a.jpg".to_owned()));

    // Across two buckets of the account: the same.
    d.send(Command::Receive {
        items: vec![at("/bucket-a/photos")],
        to: at("/bucket-b"),
        mode: TransferMode::Move,
    });
    d.settle();
    assert_eq!(s3.read_bytes(), 0);
    assert_eq!(s3.transfers().len(), 3);
    assert_eq!(s3.transfers()[2].2, "/bucket-b");
    assert!(
        s3.paths()
            .contains(&"/bucket-b/photos/raw/b.raw".to_owned())
    );
    assert!(!s3.paths().contains(&"/bucket-a/photos/a.jpg".to_owned()));

    // A copy to the local disk streams every byte: the cross-provider path.
    let down = dir.path().join("down");
    fs::create_dir_all(&down).unwrap_or_else(|e| panic!("{e}"));
    d.send(Command::Receive {
        items: vec![at("/bucket-b/photos")],
        to: local(&down),
        mode: TransferMode::Copy,
    });
    d.settle();
    assert_eq!(s3.read_bytes(), 500_000, "both files streamed down");
    assert_eq!(
        fs::read(down.join("photos").join("a.jpg"))
            .map(|b| b.len())
            .ok(),
        Some(300_000)
    );
    assert_eq!(
        s3.transfers().len(),
        3,
        "no native call for another provider"
    );
    assert!(
        d.app
            .model()
            .jobs
            .values()
            .all(|j| j.state == JobState::Done),
        "{:?}",
        d.app.model().jobs
    );
}

/// The directory the test binary runs from, where cargo puts every
/// dependency's `cdylib` under its plain name.
fn library_of(stem: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    let path = exe
        .parent()
        .unwrap_or_else(|| panic!("no parent of {}", exe.display()))
        .join(format!(
            "{}{stem}{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ));
    assert!(path.is_file(), "not built: {}", path.display());
    path
}

/// Installs the library of the crate `stem` as a plugin into `plugins_dir`
/// with the manifest the bundler would write.
fn install(plugins_dir: &Path, stem: &str) {
    let library = library_of(stem);
    let mut toml = plugins_native::describe(&library).unwrap_or_else(|e| panic!("{stem}: {e}"));
    toml.library = Library::for_crate(stem);
    let dir = plugins_dir.join(&toml.id);
    toml.write(&dir.join(MANIFEST_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    let file = toml
        .library
        .for_os(std::env::consts::OS)
        .unwrap_or_else(|| panic!("no library name for this OS"));
    fs::copy(&library, dir.join(file)).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn the_four_plugins_load_through_the_host_with_a_provider_and_a_connector_each() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    for stem in ["ext_vfs_sftp", "ext_vfs_s3", "ext_vfs_azure", "ext_vfs_gcs"] {
        install(&plugins, stem);
    }
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Host::load(&Options {
        dirs: vec![(Source::BesideBinary, plugins)],
        platform: Some(fake.clone()),
        ..Options::default()
    });
    let mut ids: Vec<&str> = host.plugins().iter().map(|p| p.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["vfs-azure", "vfs-gcs", "vfs-s3", "vfs-sftp"]);
    for plugin in host.plugins() {
        assert_eq!(plugin.status, PluginStatus::Loaded, "{plugin:?}");
        assert_eq!(plugin.points, Points::PROVIDER, "{}", plugin.id);
    }
    let mut registry = ext::Registry::with_platform(fake);
    for extension in host.extensions() {
        registry.load(&*extension).unwrap_or_else(|e| panic!("{e}"));
    }
    let mut providers: Vec<&str> = registry.providers().iter().map(|p| p.id().0).collect();
    providers.sort_unstable();
    assert_eq!(providers, ["azure", "gcs", "s3", "sftp"]);
    let mut titles: Vec<String> = registry
        .connectors()
        .iter()
        .map(|c| c.schema().title.get())
        .collect();
    titles.sort_unstable();
    assert_eq!(titles, ["Azure Blob", "Google Cloud Storage", "S3", "SFTP"]);
    for connector in registry.connectors() {
        assert!(
            connector.schema().secret_fields().next().is_some(),
            "{} keeps a secret in the keychain",
            connector.schema().title
        );
    }
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

/// The side panel rows whose label contains `text`: left of the focused
/// pane.
fn panel_rects(harness: &Harness<'_, RickUi>, text: &str) -> Vec<egui::Rect> {
    let pane = harness.state().app().model().focused;
    let pane_left = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or(400.0, |(_, r)| r.left());
    let mut rects: Vec<egui::Rect> = harness
        .query_all_by_label_contains(text)
        .map(|n| n.rect())
        .filter(|r| r.center().x < pane_left)
        .collect();
    rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
    rects
}

fn click_at(harness: &mut Harness<'_, RickUi>, at: egui::Pos2, button: egui::PointerButton) {
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
    harness.step();
}

#[test]
// One scenario through the panel, start to finish: the dialogue, the
// command, the row's menu.
#[allow(clippy::too_many_lines)]
fn add_connection_opens_the_dialogue_and_a_row_connects_and_removes_from_its_menu() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![
            Arc::new(ext_vfs_s3::S3Extension::new()),
            Arc::new(ext_vfs_trash::TrashExtension::new()),
        ],
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_step_dt(0.05)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);

    // The Cloud section is there, with the plugin's Add connection… row.
    assert_eq!(
        ui_egui::side_panel::sections_shown(harness.state().app()),
        [
            model::Section::Machine,
            model::Section::Network,
            model::Section::Cloud
        ]
    );
    let add = panel_rects(&harness, "Add connection");
    assert_eq!(add.len(), 1, "{add:?}");
    click_at(&mut harness, add[0].center(), egui::PointerButton::Primary);
    harness.step();
    assert!(
        harness
            .query_by_label_contains("Add S3 connection")
            .is_some(),
        "the dialogue names the provider"
    );
    assert!(harness.query_by_label_contains("Access key *").is_some());
    assert!(harness.query_by_label_contains("Secret key *").is_some());
    let cancel = harness.get_by_label("Cancel").rect().center();
    click_at(&mut harness, cancel, egui::PointerButton::Primary);
    assert!(
        harness
            .query_by_label_contains("Add S3 connection")
            .is_none()
    );

    // The dialogue's OK sends this command; the row appears with the name.
    harness.state_mut().app_mut().handle(
        Command::AddConnection {
            provider: ProviderId("s3"),
            name: "acme-prod".into(),
            values: vec![
                ("endpoint".into(), "http://127.0.0.1:1".into()),
                ("access_key".into(), "minio".into()),
                ("secret_key".into(), "hunter2".into()),
            ],
        },
        &mut Vec::new(),
    );
    assert!(step_until(&mut harness, Duration::from_secs(10), |h| {
        h.state()
            .app()
            .model()
            .cloud
            .iter()
            .any(|e| e.name == "acme-prod")
    }));
    settle(&mut harness);
    let settings = fs::read_to_string(fake.config_dir().join(app::SETTINGS_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(settings.contains("[[connections]]"), "{settings}");
    assert!(settings.contains("access_key = \"minio\""), "{settings}");
    assert!(
        !settings.contains("hunter2"),
        "the secret stays out: {settings}"
    );
    assert_eq!(
        fake.secrets()
            .get("s3/acme-prod/secret_key")
            .map(String::as_str),
        Some("hunter2")
    );
    // The topmost match is the Cloud row; the job's row in the operations
    // area may still name the connection below it.
    let row = panel_rects(&harness, "acme-prod");
    assert!(!row.is_empty(), "{row:?}");

    // The row's menu: Connect (not connected yet), then Remove connection.
    click_at(
        &mut harness,
        row[0].center(),
        egui::PointerButton::Secondary,
    );
    harness.step();
    assert!(harness.query_by_label_contains("Connect").is_some());
    assert!(harness.query_by_label_contains("Disconnect").is_none());
    let remove = harness
        .get_by_label_contains("Remove connection")
        .rect()
        .center();
    click_at(&mut harness, remove, egui::PointerButton::Primary);
    assert!(step_until(&mut harness, Duration::from_secs(10), |h| {
        h.state().app().model().cloud.is_empty()
    }));
    settle(&mut harness);
    let settings = fs::read_to_string(fake.config_dir().join(app::SETTINGS_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!settings.contains("acme-prod"), "{settings}");
    assert!(fake.secrets().is_empty());
    // The empty section offers Add connection… again (the finished job's
    // row in the operations area may still carry the words).
    assert_ne!(
        panel_rects(&harness, "Add connection"),
        Vec::<egui::Rect>::new()
    );
}

/// The whole pipeline against a real S3 store (issue 21): a 100 MB file
/// uploads from the local disk through the kernel's streamed copy with
/// progress, and the same upload cancelled half-way leaves no object.
/// `RICK_S3_ENDPOINT`, `RICK_S3_ACCESS_KEY` and `RICK_S3_SECRET_KEY` name
/// the store; without them the test skips with a note, and fails instead
/// under `RICK_REQUIRE_CONTAINERS`.
#[test]
// One scenario against one store, start to finish.
#[allow(clippy::too_many_lines)]
fn a_100_mb_upload_streams_with_progress_and_cancelled_half_way_leaves_no_object() {
    // The kernel's and the plugin's log lines go with a failure's output.
    let _ = tracing_subscriber::fmt()
        .with_env_filter("vfs=debug,ext_vfs_s3=debug")
        .with_test_writer()
        .try_init();
    let Ok(endpoint) = std::env::var("RICK_S3_ENDPOINT") else {
        eprintln!("RICK_S3_ENDPOINT is not set: no S3 store to test against; skipped");
        assert!(
            std::env::var_os("RICK_REQUIRE_CONTAINERS").is_none(),
            "RICK_REQUIRE_CONTAINERS is set but RICK_S3_ENDPOINT is not"
        );
        return;
    };
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let big = dir.path().join("big.bin");
    {
        use std::io::Write as _;
        let mut file = fs::File::create(&big).unwrap_or_else(|e| panic!("{e}"));
        let chunk: Vec<u8> = (0..1024 * 1024u32).map(|i| (i % 251) as u8).collect();
        for _ in 0..100 {
            file.write_all(&chunk).unwrap_or_else(|e| panic!("{e}"));
        }
    }
    let mut d = Driver::new(dir.path(), vec![Arc::new(ext_vfs_s3::S3Extension::new())]);
    d.fake
        .store_secret(
            &ext::secret_key(ProviderId("s3"), "test", "secret_key"),
            &std::env::var("RICK_S3_SECRET_KEY").unwrap_or_else(|_| "minioadmin".into()),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    d.send(Command::AddConnection {
        provider: ProviderId("s3"),
        name: "test".into(),
        values: vec![
            ("endpoint".into(), endpoint),
            (
                "access_key".into(),
                std::env::var("RICK_S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".into()),
            ),
            (
                "secret_key".into(),
                std::env::var("RICK_S3_SECRET_KEY").unwrap_or_else(|_| "minioadmin".into()),
            ),
        ],
    });
    d.settle();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |t| t.as_millis());
    let bucket = format!("rick-test-up-{stamp}");
    let provider = d
        .app
        .mounts()
        .provider(ProviderId("s3"))
        .unwrap_or_else(|| panic!("s3 mounted"));
    runtime::block_on(provider.create_dir(&at(&format!("/test/{bucket}"))))
        .unwrap_or_else(|e| panic!("create bucket: {e}"));

    // The whole file: progress reaches the size, the object has it.
    d.events.clear();
    d.send(Command::Receive {
        items: vec![local(&big)],
        to: at(&format!("/test/{bucket}")),
        mode: TransferMode::Copy,
    });
    d.settle();
    let mut peaks: Vec<u64> = d
        .events
        .iter()
        .filter_map(|e| match e {
            Event::JobProgress {
                progress: types::Progress::Bytes { done, .. },
                ..
            } => Some(*done),
            _ => None,
        })
        .collect();
    peaks.sort_unstable();
    assert!(
        peaks.len() >= 2,
        "progress was reported along the way: {peaks:?}"
    );
    assert_eq!(peaks.last().copied(), Some(100 * 1024 * 1024));
    for event in &d.events {
        if let Event::JobDone { result: Err(e), .. } = event {
            panic!("the upload job failed: {e}");
        }
    }
    let uploaded = runtime::block_on(provider.stat(&at(&format!("/test/{bucket}/big.bin"))))
        .unwrap_or_else(|e| panic!("stat: {e}"));
    assert_eq!(uploaded.size, Some(100 * 1024 * 1024));

    // The same, cancelled once a part is on its way: no object remains.
    let partial = dir.path().join("partial.bin");
    fs::copy(&big, &partial).unwrap_or_else(|e| panic!("{e}"));
    d.events.clear();
    d.send(Command::Receive {
        items: vec![local(&partial)],
        to: at(&format!("/test/{bucket}")),
        mode: TransferMode::Copy,
    });
    assert!(d.run_until(Duration::from_mins(1), |app| {
        app.model().jobs.values().any(|j| {
            matches!(
                j.state,
                JobState::Running(types::Progress::Bytes { done, .. }) if done > 16 * 1024 * 1024
            )
        })
    }));
    let id = d
        .app
        .model()
        .jobs
        .iter()
        .find(|(_, j)| matches!(j.state, JobState::Running(_)))
        .map_or_else(|| panic!("a running job"), |(id, _)| *id);
    d.send(Command::CancelJob(id));
    d.settle();
    assert_eq!(
        d.app.model().jobs.get(&id).map(|j| j.state.clone()),
        Some(JobState::Cancelled)
    );
    std::thread::sleep(Duration::from_millis(500));
    assert!(matches!(
        runtime::block_on(provider.stat(&at(&format!("/test/{bucket}/partial.bin")))),
        Err(types::Error::NotFound)
    ));
    runtime::block_on(provider.remove(&at(&format!("/test/{bucket}")), ext::RemoveMode::Permanent))
        .unwrap_or_else(|e| panic!("remove bucket: {e}"));
}
