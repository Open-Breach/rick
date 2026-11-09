//! Acceptance (issue 24, permissions): Properties on a fixture file shows
//! its owner, group and mode bits, a change through the dialogue's command
//! is read back, and a recursive change on a thousand files is one job
//! with progress that one Undo reverts; through the egui harness, the
//! dialogue opens on the selection with its rows and the Permissions
//! section. The elevated change and the helper are the binary crate's
//! tests (`crates/rick/tests/perms.rs`).

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event, PermissionRequest, PropertiesInfo, Recursion};
use ext::platform::{ConfigDir as _, Fake};
use model::{JobState, SelectionChange};
use platform::PermissionChange;
use tests::local;
use types::Progress;

/// A UI whose answers are fixed.
struct Quiet;

impl ext::UiHost for Quiet {
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

/// The application over a fake platform, driven without a window.
struct Driver {
    app: App,
    events: Vec<Event>,
}

impl Driver {
    fn new(fake: &Arc<Fake>, start: &Path) -> Self {
        let app = App::new(Config {
            platform: fake.clone(),
            ui: Arc::new(Quiet),
            extensions: Vec::new(),
            config_dir: Some(fake.config_dir()),
            start: Some(local(start)),
        });
        let mut driver = Self {
            app,
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
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let quiet = runtime::block_on(async {
                tokio::time::timeout(Duration::from_millis(100), self.app.tick(&mut self.events))
                    .await
                    .is_err()
            });
            if quiet && !self.app.is_busy() {
                break;
            }
            assert!(Instant::now() < deadline, "did not settle");
        }
    }

    /// Runs the loop until `done`, within `limit`.
    fn run_until(&mut self, limit: Duration, mut done: impl FnMut(&App, &[Event]) -> bool) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            runtime::block_on(async {
                let _ = tokio::time::timeout(
                    Duration::from_millis(50),
                    self.app.tick(&mut self.events),
                )
                .await;
            });
            if done(&self.app, &self.events) {
                return true;
            }
        }
        done(&self.app, &self.events)
    }

    fn pane(&self) -> model::PaneId {
        self.app.model().focused
    }

    fn select(&mut self, name: &str) {
        let pane = self.pane();
        let id = self
            .app
            .model()
            .active_tab(pane)
            .and_then(|(_, t)| t.listing.rows().find(|e| e.name == name).map(|e| e.id))
            .unwrap_or_else(|| panic!("no entry {name}"));
        self.send(Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        });
    }

    /// Opens Properties on the selection and returns what the dialogue
    /// gets.
    fn properties(&mut self) -> PropertiesInfo {
        let before = self.events.len();
        let pane = self.pane();
        self.send(Command::ShowProperties(pane));
        assert!(
            self.run_until(Duration::from_secs(10), |_, events| {
                events[before..]
                    .iter()
                    .any(|e| matches!(e, Event::Properties(_)))
            }),
            "no properties arrived"
        );
        self.events[before..]
            .iter()
            .rev()
            .find_map(|e| match e {
                Event::Properties(info) => Some((**info).clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("properties"))
    }

    /// Sends the change and waits for its job to end.
    fn set_permissions(&mut self, request: PermissionRequest) -> types::JobId {
        let before: Vec<types::JobId> = self.app.model().jobs.keys().copied().collect();
        let pane = self.pane();
        self.send(Command::SetPermissions { pane, request });
        let mut id = None;
        assert!(
            self.run_until(Duration::from_mins(1), |app, _| {
                let new: Vec<types::JobId> = app
                    .model()
                    .jobs
                    .keys()
                    .filter(|k| !before.contains(k))
                    .copied()
                    .collect();
                id = new.first().copied();
                new.len() == 1
                    && app.model().jobs[&new[0]].state == JobState::Done
                    && !app.is_busy()
            }),
            "the permission job did not finish: {:?}",
            self.app.model().jobs
        );
        self.settle();
        id.unwrap_or_else(|| panic!("a job"))
    }
}

fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path)
        .unwrap_or_else(|e| panic!("{e}"))
        .permissions()
        .mode()
        & 0o7777
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap_or_else(|e| panic!("{e}"));
}

fn mode_change(mode: u32) -> PermissionChange {
    PermissionChange {
        mode: Some(mode),
        ..PermissionChange::default()
    }
}

#[test]
fn properties_show_the_mode_bits_and_a_change_is_read_back_and_undone() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let file = dir.path().join("notes.txt");
    fs::write(&file, b"hello").unwrap_or_else(|e| panic!("{e}"));
    chmod(&file, 0o644);
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut d = Driver::new(&fake, dir.path());
    d.select("notes.txt");

    let info = d.properties();
    assert_eq!(info.entry.name, "notes.txt");
    assert_eq!(info.path.as_deref(), Some(file.as_path()));
    assert!(
        info.rows.iter().any(|(k, v)| k == "Size" && v == "5 bytes"),
        "{:?}",
        info.rows
    );
    let perms = info
        .permissions
        .clone()
        .unwrap_or_else(|| panic!("permissions"));
    assert_eq!(perms.mode, Some(0o644));
    assert!(
        perms.owner.name.is_some(),
        "the owner has a name: {:?}",
        perms.owner
    );
    assert!(!info.principals.users.is_empty(), "the users are listed");

    // The change: one job, the mode read back, the open dialogue told.
    let events_before = d.events.len();
    let job = d.set_permissions(PermissionRequest {
        targets: vec![file.clone()],
        change: mode_change(0o4600),
        recursive: None,
    });
    assert_eq!(mode_of(&file), 0o4600);
    assert_eq!(
        d.app.model().jobs[&job].name,
        "Change permissions of notes.txt"
    );
    let refreshed = d.events[events_before..]
        .iter()
        .find_map(|e| match e {
            Event::Properties(info) => Some((**info).clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the open dialogue gets the new state"));
    assert_eq!(refreshed.permissions.and_then(|p| p.mode), Some(0o4600));

    // Undo is one step and runs as a job too.
    d.send(Command::Undo);
    assert!(d.run_until(Duration::from_secs(30), |_, _| mode_of(&file) == 0o644));
    d.settle();
    assert_eq!(mode_of(&file), 0o644);
    d.send(Command::Redo);
    assert!(d.run_until(Duration::from_secs(30), |_, _| mode_of(&file) == 0o4600));
    d.settle();

    // Closing the dialogue stops the reports.
    d.send(Command::PropertiesClosed);
    let events_before = d.events.len();
    d.set_permissions(PermissionRequest {
        targets: vec![file.clone()],
        change: mode_change(0o640),
        recursive: None,
    });
    assert!(
        !d.events[events_before..]
            .iter()
            .any(|e| matches!(e, Event::Properties(_))),
        "a closed dialogue is told nothing"
    );
}

#[test]
fn a_recursive_change_on_a_thousand_files_is_one_job_with_progress_and_one_undo() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let tree = dir.path().join("tree");
    let mut files: Vec<PathBuf> = Vec::new();
    for folder in 0..10 {
        let sub = tree.join(format!("d{folder}"));
        fs::create_dir_all(&sub).unwrap_or_else(|e| panic!("{e}"));
        for file in 0..100 {
            let path = sub.join(format!("f{file}.txt"));
            fs::write(&path, b"x").unwrap_or_else(|e| panic!("{e}"));
            chmod(&path, 0o644);
            files.push(path);
        }
        chmod(&sub, 0o755);
    }
    chmod(&tree, 0o755);
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut d = Driver::new(&fake, dir.path());
    d.select("tree");

    let jobs_before = d.app.model().jobs.len();
    let events_before = d.events.len();
    // The target folder gets the change as well, so the mode keeps its
    // execute bit for the checks that read through it.
    let job = d.set_permissions(PermissionRequest {
        targets: vec![tree.clone()],
        change: mode_change(0o700),
        recursive: Some(Recursion {
            files: true,
            folders: false,
        }),
    });
    assert_eq!(
        d.app.model().jobs.len(),
        jobs_before + 1,
        "one job for the whole tree"
    );
    assert!(
        files.iter().all(|f| mode_of(f) == 0o700),
        "every file changed"
    );
    assert_eq!(mode_of(&tree), 0o700, "the folder itself too");
    assert_eq!(
        mode_of(&tree.join("d3")),
        0o755,
        "the folders inside were not asked for"
    );
    // Progress counted the entries: the folder and its thousand files.
    let totals: Vec<u64> = d.events[events_before..]
        .iter()
        .filter_map(|e| match e {
            Event::JobProgress {
                id,
                progress:
                    Progress::Items {
                        total: Some(total), ..
                    },
            } if *id == job => Some(*total),
            _ => None,
        })
        .collect();
    assert!(totals.contains(&1001), "{totals:?}");

    // One Undo, one job, everything back.
    let jobs_before = d.app.model().jobs.len();
    d.send(Command::Undo);
    assert!(d.run_until(Duration::from_mins(1), |app, _| {
        app.model().jobs.len() == jobs_before + 1 && !app.is_busy()
    }));
    d.settle();
    assert!(files.iter().all(|f| mode_of(f) == 0o644), "every file back");
    assert_eq!(mode_of(&tree), 0o755);
}

#[test]
fn the_dialogue_opens_on_the_selection_with_its_rows_and_the_permissions_section() {
    use egui_kittest::kittest::Queryable as _;

    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let file = dir.path().join("photo.jpg");
    fs::write(&file, vec![0u8; 2048]).unwrap_or_else(|e| panic!("{e}"));
    chmod(&file, 0o640);
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(ui_egui::EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .with_step_dt(0.05)
        .build_eframe(move |cc| ui_egui::RickUi::new(app, host, &cc.egui_ctx));
    let deadline = Instant::now() + Duration::from_secs(30);
    while harness.state().app().is_busy() {
        harness.step();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.step();
    let pane = harness.state().app().model().focused;
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| {
            t.listing
                .rows()
                .find(|e| e.name == "photo.jpg")
                .map(|e| e.id)
        })
        .unwrap_or_else(|| panic!("the file is listed"));
    harness.state_mut().app_mut().handle(
        Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    harness.step();
    harness
        .state_mut()
        .app_mut()
        .handle(Command::ShowProperties(pane), &mut Vec::new());
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness
        .query_by_label_contains("Properties of photo.jpg")
        .is_none()
    {
        harness.step();
        assert!(Instant::now() < deadline, "the dialogue did not open");
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.step();
    for label in [
        "Permissions",
        "Owner",
        "Group",
        "Read",
        "Write",
        "Execute",
        "Octal",
        "Apply",
    ] {
        assert!(
            harness.query_all_by_label_contains(label).next().is_some(),
            "{label} in the dialogue"
        );
    }
    assert!(
        harness.query_by_label("2048 bytes").is_some(),
        "the size row"
    );
    assert!(
        harness.state().properties().pending_change().is_none(),
        "nothing changed yet"
    );
}
