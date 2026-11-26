//! Acceptance (issue 21): providers as plugins, the clouds.
//!
//! The kernel's side is proven on every OS with the in-memory provider in
//! its native mode: a move within one account, and one across two buckets
//! of it, goes through `transfer` and streams nothing through `open_read`,
//! while a copy to the local disk streams every byte; the side panel's
//! *Add connection…* opens the plugin's dialogue, and a connection's row
//! connects and removes through its menu, the secret in the keychain and
//! never in `rick.toml`; the three plugins load through the host like the
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
fn the_three_plugins_load_through_the_host_with_a_provider_and_a_connector_each() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    for stem in ["ext_vfs_s3", "ext_vfs_azure", "ext_vfs_gcs"] {
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
    assert_eq!(ids, ["vfs-azure", "vfs-gcs", "vfs-s3"]);
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
    assert_eq!(providers, ["azure", "gcs", "s3"]);
    let mut titles: Vec<String> = registry
        .connectors()
        .iter()
        .map(|c| c.schema().title.get())
        .collect();
    titles.sort_unstable();
    assert_eq!(titles, ["Azure Blob", "Google Cloud Storage", "S3"]);
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
    let settings = tests::read_until(&fake.config_dir().join(app::SETTINGS_FILE), |t| {
        t.contains("access_key = \"minio\"")
    });
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
    let settings = tests::read_until(&fake.config_dir().join(app::SETTINGS_FILE), |t| {
        !t.contains("acme-prod")
    });
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
    // The job reports its item count right after the copy, and reports
    // closer together than the runtime's window go out as the last one
    // (`runtime::PROGRESS_EVERY`), so the final byte count is seen unless
    // the item count took its window; then that count is the complete one.
    let last = d.events.iter().rev().find_map(|e| match e {
        Event::JobProgress { progress, .. } => Some(progress.clone()),
        _ => None,
    });
    match last {
        Some(types::Progress::Items { done, total }) => {
            assert_eq!(Some(done), total, "the job's final item count is complete");
        }
        _ => assert_eq!(peaks.last().copied(), Some(100 * 1024 * 1024)),
    }
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

/// A point inside the control of the connection dialogue's row whose
/// label starts with `label`: the controls share a column right of the
/// longest label.
fn dialogue_field(harness: &Harness<'_, RickUi>, label: &str) -> egui::Pos2 {
    let right = [
        "Endpoint",
        "Region",
        "Access key",
        "Secret key",
        "Buckets",
        "Path-style",
    ]
    .iter()
    .filter_map(|text| harness.query_all_by_label_contains(text).next())
    .map(|node| node.rect().right())
    .fold(0.0, f32::max);
    let row = harness
        .query_all_by_label_contains(label)
        .next()
        .unwrap_or_else(|| panic!("the row {label}"))
        .rect();
    egui::pos2(right + 60.0, row.center().y)
}

/// Issue 309, `specs/s3-connection.md` 1.4, through the real dialogue:
/// pasting the address of an R2 bucket leaves the endpoint clean, puts the
/// bucket into *Buckets*, sets the region to `auto`, shows *Cloudflare R2*
/// by the endpoint, and a region typed by hand is left alone afterwards.
#[test]
fn pasting_an_r2_bucket_address_fills_the_s3_dialogue() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(ext_vfs_s3::S3Extension::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_step_dt(0.05)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let shows = |harness: &Harness<'_, RickUi>, value: &str| {
        harness.query_all_by_value(value).next().is_some()
    };
    let add = panel_rects(&harness, "Add connection");
    click_at(&mut harness, add[0].center(), egui::PointerButton::Primary);
    harness.step();
    // An empty endpoint is AWS, and says so.
    assert!(harness.query_by_label("Amazon S3").is_some());
    assert!(shows(&harness, "us-east-1"));

    let endpoint = dialogue_field(&harness, "Endpoint");
    let region = dialogue_field(&harness, "Region");
    click_at(&mut harness, endpoint, egui::PointerButton::Primary);
    harness.event(egui::Event::Paste(
        "https://abc123.r2.cloudflarestorage.com/packages".to_owned(),
    ));
    harness.step();
    harness.step();
    assert!(
        harness.query_by_label("Cloudflare R2").is_some(),
        "clicked {endpoint:?}: {}",
        dialogue_state(&harness)
    );
    assert!(harness.query_by_label("Amazon S3").is_none());
    assert!(
        shows(&harness, "https://abc123.r2.cloudflarestorage.com"),
        "the endpoint is left clean"
    );
    assert!(shows(&harness, "packages"));
    assert!(shows(&harness, "auto"));
    assert!(!shows(&harness, "us-east-1"));

    // A region typed by hand stays when the endpoint changes again.
    click_at(&mut harness, region, egui::PointerButton::Primary);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.step();
    harness.event(egui::Event::Text("weur".to_owned()));
    harness.step();
    click_at(&mut harness, endpoint, egui::PointerButton::Primary);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.step();
    harness.event(egui::Event::Paste(
        "https://s3.eu-central-1.wasabisys.com".to_owned(),
    ));
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Wasabi").is_some());
    assert!(shows(&harness, "weur"));
    assert!(!shows(&harness, "eu-central-1"));
}

/// Issue 309, `specs/s3-connection.md` 1.4, last line: a connection that
/// 0.6.0 wrote into `rick.toml`, without the fields 0.6.1 adds, loads as
/// it is and is written back unchanged.
#[test]
fn a_connection_saved_before_the_buckets_field_loads_unchanged() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let saved = "[[connections]]\nprovider = \"s3\"\nname = \"old\"\naccess_key = \"minio\"\n\
                 endpoint = \"http://127.0.0.1:1\"\npath_style = \"true\"\nregion = \"us-east-1\"\n";
    fs::create_dir_all(fake.config_dir()).unwrap_or_else(|e| panic!("{e}"));
    fs::write(fake.config_dir().join(app::SETTINGS_FILE), saved).unwrap_or_else(|e| panic!("{e}"));
    let s3 = Arc::new(ext_vfs_s3::S3Extension::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: Arc::new(EguiHost::new()),
        extensions: vec![s3],
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let entry = app
        .model()
        .settings
        .connections
        .iter()
        .find(|c| c.name == "old")
        .unwrap_or_else(|| panic!("the connection loaded"));
    assert_eq!(
        entry
            .settings
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["access_key", "endpoint", "path_style", "region"]
    );
    let connector = app
        .registry()
        .connector_of(ProviderId("s3"))
        .unwrap_or_else(|| panic!("the S3 connector"));
    // Its root would list the buckets, as before: it names none.
    let dialog = ui_egui::side_panel::ConnectionDialog::edit(&**connector, entry);
    let value = |key: &str| {
        dialog
            .values
            .iter()
            .find(|(field, _)| field.key == key)
            .map_or("", |(_, value)| value.as_str())
    };
    assert_eq!(value("buckets"), "");
    assert_eq!(value("region"), "us-east-1");
    assert_eq!(value("endpoint"), "http://127.0.0.1:1");
}

/// A local stand-in for an S3 service with a bucket-scoped key: listing
/// the buckets is refused, the bucket `packages` lists, every other
/// bucket does not exist. Returns its endpoint. No real account.
fn scoped_s3_service() -> String {
    use std::io::{BufRead as _, BufReader, Write as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("{e}"));
    let port = listener
        .local_addr()
        .unwrap_or_else(|e| panic!("{e}"))
        .port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let Ok(clone) = stream.try_clone() else {
                continue;
            };
            let mut reader = BufReader::new(clone);
            let mut request = String::new();
            if reader.read_line(&mut request).is_err() {
                continue;
            }
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line != "\r\n" {
                line.clear();
            }
            let target = request.split_whitespace().nth(1).unwrap_or_default();
            let error =
                |code: &str| format!("<Error><Code>{code}</Code><Message>{code}</Message></Error>");
            let (status, body) = if target == "/" {
                (403, error("AccessDenied"))
            } else if target.starts_with("/packages/") {
                (
                    200,
                    "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>"
                        .to_owned(),
                )
            } else {
                (404, error("NoSuchBucket"))
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\ncontent-type: application/xml\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// What the open connection dialogue holds, for an assertion's message:
/// the name, the fields (none of the tests' values is a real secret), the
/// refusal and where the test stands; and where the pointer is.
fn dialogue_state(harness: &Harness<'_, RickUi>) -> String {
    let pointer = harness.ctx.input(|i| i.pointer.latest_pos());
    harness
        .state()
        .side_panel_state()
        .connection
        .as_ref()
        .map_or_else(
            || format!("no dialogue; pointer {pointer:?}"),
            |dialog| {
                let values: Vec<String> = dialog
                    .values
                    .iter()
                    .map(|(field, value)| format!("{}={value:?}", field.key))
                    .collect();
                format!(
                    "name={:?} {} error={:?} test={:?} pointer={pointer:?}",
                    dialog.name,
                    values.join(" "),
                    dialog.error,
                    dialog.test
                )
            },
        )
}

/// Pastes `text` over what the field of the row `label` holds.
fn fill_field(harness: &mut Harness<'_, RickUi>, label: &str, text: &str) {
    // The dialogue moves when a row comes or goes: let it rest.
    for _ in 0..4 {
        harness.step();
    }
    let at = dialogue_field(harness, label);
    click_at(harness, at, egui::PointerButton::Primary);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.step();
    harness.event(egui::Event::Paste(text.to_owned()));
    harness.step();
    harness.step();
}

fn click_label(harness: &mut Harness<'_, RickUi>, label: &str) {
    for _ in 0..4 {
        harness.step();
    }
    let at = harness.get_by_label(label).rect().center();
    click_at(harness, at, egui::PointerButton::Primary);
}

fn shows(harness: &Harness<'_, RickUi>, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

fn saved_connections(harness: &Harness<'_, RickUi>) -> usize {
    harness.state().app().model().settings.connections.len()
}

/// The *Add S3 connection* dialogue, filled in for a bucket-scoped key on
/// a local stand-in, with *Buckets* as given.
fn s3_dialogue(dir: &Path, buckets: &str) -> (Harness<'static, RickUi>, Arc<Fake>) {
    let endpoint = scoped_s3_service();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(ext_vfs_s3::S3Extension::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_step_dt(0.05)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let add = panel_rects(&harness, "Add connection");
    click_at(&mut harness, add[0].center(), egui::PointerButton::Primary);
    harness.step();
    assert!(shows(&harness, "Test"), "the S3 dialogue has a Test button");
    // The name has the focus when the dialogue opens.
    harness.event(egui::Event::Paste("scoped".to_owned()));
    harness.step();
    fill_field(&mut harness, "Endpoint", &endpoint);
    fill_field(&mut harness, "Access key", "key");
    fill_field(&mut harness, "Secret key", "typed-secret");
    fill_field(&mut harness, "Buckets", buckets);
    (harness, fake)
}

/// *Add* was clicked: steps until the test that is not good holds the save.
fn held(harness: &mut Harness<'_, RickUi>) {
    let held = step_until(harness, Duration::from_secs(10), |h| {
        shows(h, "Save anyway")
    });
    assert!(held, "{}", dialogue_state(harness));
    assert!(shows(harness, "The test did not pass"));
}

/// Issue 310, `specs/s3-connection.md` 3.6, through the real dialogue:
/// *Add* with a test that is not good saves nothing, and neither does
/// going back to the fields or the *Test* button; with the bucket's name
/// put right the test is good and *Add* saves at once. The typed secret
/// reaches the keychain only then, and no file.
#[test]
fn a_test_that_is_not_good_saves_nothing_and_going_back_saves_nothing() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, fake) = s3_dialogue(dir.path(), "pakages");

    click_label(&mut harness, "Add");
    held(&mut harness);
    assert!(shows(&harness, "The bucket pakages can be opened"));
    assert!(shows(&harness, "NoSuchBucket"));
    assert_eq!(saved_connections(&harness), 0);
    assert_eq!(fake.secrets().len(), 0, "{:?}", fake.secrets());

    click_label(&mut harness, "Back to the fields");
    settle(&mut harness);
    assert!(!shows(&harness, "Save anyway"));
    assert!(shows(&harness, "Add S3 connection"), "the dialogue stays");
    assert_eq!(saved_connections(&harness), 0);

    // The Test button shows the result and offers no save.
    click_label(&mut harness, "Test");
    assert!(step_until(&mut harness, Duration::from_secs(10), |h| {
        shows(h, "NoSuchBucket")
    }));
    assert!(!shows(&harness, "Save anyway"));
    assert_eq!(saved_connections(&harness), 0);
    assert_eq!(fake.secrets().len(), 0, "{:?}", fake.secrets());

    // The name put right: the result of the old form goes, the test is
    // good, and Add saves and closes.
    fill_field(&mut harness, "Buckets", "packages");
    assert!(
        !shows(&harness, "NoSuchBucket"),
        "an edit forgets the result"
    );
    click_label(&mut harness, "Add");
    assert!(step_until(&mut harness, Duration::from_secs(10), |h| {
        saved_connections(h) == 1
    }));
    assert!(!shows(&harness, "Add S3 connection"), "the dialogue closed");
    settle(&mut harness);
    assert_eq!(
        fake.secrets()
            .get("s3/scoped/secret_key")
            .map(String::as_str),
        Some("typed-secret")
    );
    for file in fs::read_dir(fake.config_dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        let text = fs::read(file.path()).unwrap_or_default();
        assert!(
            !String::from_utf8_lossy(&text).contains("typed-secret"),
            "the secret is in {}",
            file.path().display()
        );
    }
}

/// The other choice of 3.4: *Save anyway* saves the connection as typed,
/// mistyped name and all, which is also how one is added while offline.
#[test]
fn save_anyway_saves_a_connection_whose_test_is_not_good() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, fake) = s3_dialogue(dir.path(), "pakages");
    click_label(&mut harness, "Add");
    held(&mut harness);
    assert_eq!(saved_connections(&harness), 0);
    click_label(&mut harness, "Save anyway");
    assert!(step_until(&mut harness, Duration::from_secs(10), |h| {
        saved_connections(h) == 1
    }));
    assert!(!shows(&harness, "Add S3 connection"), "the dialogue closed");
    let entry = harness.state().app().model().settings.connections[0].clone();
    assert_eq!(entry.name, "scoped");
    assert_eq!(
        entry.settings.get("buckets").map(String::as_str),
        Some("pakages")
    );
    settle(&mut harness);
    assert_eq!(
        fake.secrets()
            .get("s3/scoped/secret_key")
            .map(String::as_str),
        Some("typed-secret")
    );
}

/// Issue 311: the connection dialogue has the keyboard while it is open.
/// Escape is *Cancel*, with the focus in a field too. Before the fix the
/// side panel behind the dialogue took the key, and the dialogue stayed.
#[test]
fn escape_closes_the_connection_dialogue() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _fake) = s3_dialogue(dir.path(), "packages");
    assert!(shows(&harness, "Add S3 connection"));
    harness.key_press(egui::Key::Escape);
    for _ in 0..4 {
        harness.step();
    }
    assert!(!shows(&harness, "Add S3 connection"), "Escape left it open");
    assert_eq!(saved_connections(&harness), 0);
}
