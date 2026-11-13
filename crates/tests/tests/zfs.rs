//! Acceptance (change add-zfs-snapshots): the ZFS plugin's read-only
//! provider inside the application.
//!
//! A scripted `zfs` reports one dataset mounted at a temporary directory
//! with one snapshot, whose files lie under `.zfs/snapshot/<name>/` as on a
//! real pool. Inside the snapshot the context menu offers no rename, trash or
//! new item; *Delete permanently*, which the core offers wherever there is a
//! selection, fails and changes nothing; and a file copied out of the
//! snapshot into a live folder arrives through the normal copy job. A file
//! in a snapshot opens like any file: the system is handed its path under
//! `.zfs/snapshot/`.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event, TransferMode};
use ext::{ActionId, Availability, Invocation, Level, UiHost};
use ext_zfs::{Output, ScriptedZfs, ZfsExtension, snapshot_location};
use model::SelectionChange;
use platform::{ConfigDir, Fake};
use tests::local;

/// A UI that says yes to every question and dismisses every form.
struct Agreeing;

impl UiHost for Agreeing {
    fn notify(&self, _level: Level, _text: &str) {}
    fn confirm(&self, _title: &str, _text: &str) -> bool {
        true
    }
    fn prompt(&self, _title: &str, _default: &str) -> Option<String> {
        None
    }
    fn show(&self, _title: &str, _text: &str) {}
    fn form(&self, _form: ext::Form) -> Option<ext::Values> {
        None
    }
}

/// `tank/home` mounted at `<dir>/home` with snapshot `b` holding
/// `notes/report.txt`; the live folder `<dir>/home/notes` is empty.
fn pool(dir: &Path) -> Arc<ScriptedZfs> {
    let home = dir.join("home");
    let snap = home.join(".zfs/snapshot/b/notes");
    fs::create_dir_all(&snap).unwrap_or_else(|e| panic!("{e}"));
    fs::write(snap.join("report.txt"), "as it was\n").unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(home.join("notes")).unwrap_or_else(|e| panic!("{e}"));
    let zfs = Arc::new(ScriptedZfs::new());
    zfs.on(
        &[
            "list",
            "-H",
            "-p",
            "-o",
            "name,mountpoint,mounted",
            "-t",
            "filesystem",
        ],
        Output::ok(format!("tank/home\t{}\tyes\n", home.display())),
    );
    zfs.on(
        &[
            "list",
            "-H",
            "-p",
            "-t",
            "snapshot",
            "-o",
            "name,creation,used",
            "-s",
            "creation",
            "-d",
            "1",
            "tank/home",
        ],
        Output::ok("tank/home@b\t1790877375\t4096\n"),
    );
    zfs
}

struct Driver {
    app: App,
    fake: Arc<Fake>,
    events: Vec<Event>,
    pane: model::PaneId,
}

impl Driver {
    fn new(dir: &Path, zfs: Arc<ScriptedZfs>) -> Self {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let app = App::new(Config {
            platform: Arc::clone(&fake) as Arc<dyn platform::Platform>,
            ui: Arc::new(Agreeing),
            extensions: vec![Arc::new(ZfsExtension::with_zfs(zfs))],
            config_dir: Some(fake.config_dir()),
            start: Some(local(dir)),
        });
        let pane = app.model().focused;
        let mut driver = Self {
            app,
            fake,
            events: Vec::new(),
            pane,
        };
        driver.settle();
        driver
    }

    fn step(&mut self, command: Command) {
        self.events.clear();
        self.app.handle(command, &mut self.events);
        self.settle();
    }

    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
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
                assert!(Instant::now() < deadline, "did not settle");
            }
        });
    }

    fn rows(&self) -> Vec<types::Entry> {
        self.app
            .model()
            .active_tab(self.pane)
            .map_or_else(|| panic!("no active tab"), |(_, tab)| tab)
            .listing
            .rows()
            .cloned()
            .collect()
    }

    fn select(&mut self, name: &str) {
        let id = self
            .rows()
            .iter()
            .find(|e| e.name == name)
            .map_or_else(|| panic!("no {name}"), |e| e.id);
        self.step(Command::Select {
            pane: self.pane,
            change: SelectionChange::Set(vec![id]),
        });
    }

    fn enabled(&self, id: &str) -> bool {
        let ctx = self
            .app
            .action_ctx(self.pane, Invocation::ContextMenu)
            .unwrap_or_else(|| panic!("no active tab"));
        self.app
            .registry()
            .actions()
            .iter()
            .find(|a| a.id().0 == id)
            .is_some_and(|a| a.availability(&ctx) == Availability::Enabled)
    }
}

#[test]
fn the_context_menu_offers_no_change_inside_a_snapshot() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(dir.path(), pool(dir.path()));
    let pane = d.pane;
    d.step(Command::Navigate {
        pane,
        to: snapshot_location("tank/home", "b", Path::new("notes")),
    });
    let names: Vec<String> = d.rows().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["report.txt"]);
    d.select("report.txt");
    assert!(d.enabled("core.copy"), "copying out stays possible");
    for id in [
        "core.rename",
        "core.trash",
        "core.new_file",
        "core.new_folder",
    ] {
        assert!(!d.enabled(id), "{id} is offered inside a snapshot");
    }

    // Delete permanently is the core's and checks no capability; its job
    // fails on the read-only provider and the file stays.
    let ctx = d
        .app
        .action_ctx(pane, Invocation::ContextMenu)
        .unwrap_or_else(|| panic!("no active tab"));
    d.step(Command::Run {
        action: ActionId("core.delete"),
        ctx,
    });
    let failed = d.events.iter().any(|event| {
        matches!(event, Event::JobDone { result: Err(types::Error::Unsupported(why)), .. } if why.contains("read-only"))
    });
    assert!(failed, "{:?}", d.events);
    assert!(
        dir.path()
            .join("home/.zfs/snapshot/b/notes/report.txt")
            .exists()
    );
}

#[test]
fn a_file_copied_out_of_a_snapshot_arrives_through_the_copy_job() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(dir.path(), pool(dir.path()));
    let live = dir.path().join("home/notes");
    d.step(Command::Receive {
        items: vec![snapshot_location(
            "tank/home",
            "b",
            Path::new("notes/report.txt"),
        )],
        to: local(&live),
        mode: TransferMode::Copy,
    });
    assert_eq!(
        fs::read_to_string(live.join("report.txt")).ok().as_deref(),
        Some("as it was\n")
    );
    assert!(
        dir.path()
            .join("home/.zfs/snapshot/b/notes/report.txt")
            .exists(),
        "the snapshot keeps its file"
    );
}

/// specs/ext-zfs.md: a snapshot "opens, previews and copies out like any
/// folder". Enter hands the system the file's path in the snapshot
/// directory, and the editor is offered for it.
#[test]
fn a_file_in_a_snapshot_opens_from_its_snapshot_directory() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(dir.path(), pool(dir.path()));
    let pane = d.pane;
    d.step(Command::Navigate {
        pane,
        to: snapshot_location("tank/home", "b", Path::new("notes")),
    });
    d.select("report.txt");
    assert!(d.enabled("core.open_in_editor"), "the editor is offered");
    let id = d
        .rows()
        .iter()
        .find(|e| e.name == "report.txt")
        .map_or_else(|| panic!("no report.txt"), |e| e.id);
    d.step(Command::Open { pane, id });
    let failed: Vec<&Event> = d
        .events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Notify {
                    level: Level::Error,
                    ..
                }
            )
        })
        .collect();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(
        d.fake.opened(),
        [dir.path().join("home/.zfs/snapshot/b/notes/report.txt")]
    );
}
