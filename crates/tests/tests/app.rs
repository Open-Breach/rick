//! Acceptance (issue 3): a script of `Command`s drives `app` headlessly
//! against a generated fixture; the `Event` stream and the final model are
//! what the UI would have seen.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event};
use ext::{ActionId, Invocation, Level, UiHost};
use model::{Column, PaneId, SelectionChange, SortDir, type_of};
use platform::{ConfigDir, Fake, Trash};
use tests::local;
use types::{Entry, EntryId, EntryKind};

/// A UI whose answers the script sets in advance.
#[derive(Default)]
struct Scripted {
    prompt: Mutex<Option<String>>,
    confirm: Mutex<bool>,
    notices: Mutex<Vec<(Level, String)>>,
}

impl UiHost for Scripted {
    fn notify(&self, level: Level, text: &str) {
        self.notices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((level, text.to_owned()));
    }
    fn confirm(&self, _title: &str, _text: &str) -> bool {
        *self.confirm.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn prompt(&self, _title: &str, _default: &str) -> Option<String> {
        self.prompt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct Driver {
    app: App,
    ui: Arc<Scripted>,
    fake: Arc<Fake>,
    events: Vec<Event>,
    pane: PaneId,
}

impl Driver {
    fn new(start: &Path) -> Self {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let ui = Arc::new(Scripted::default());
        let app = App::new(Config {
            platform: fake.clone(),
            ui: ui.clone(),
            extensions: Vec::new(),
            config_dir: Some(fake.config_dir()),
            start: local(start),
        });
        let pane = app.model().focused;
        let mut driver = Self {
            app,
            ui,
            fake,
            events: Vec::new(),
            pane,
        };
        driver.settle();
        driver
    }

    /// Sends one command and lets everything it started finish. Returns the
    /// events the step produced.
    fn step(&mut self, command: Command) -> Vec<Event> {
        self.events.clear();
        self.app.handle(command, &mut self.events);
        self.settle();
        std::mem::take(&mut self.events)
    }

    fn run(&mut self, action: &'static str) -> Vec<Event> {
        let ctx = self
            .app
            .action_ctx(self.pane, Invocation::Shortcut)
            .unwrap_or_else(|| panic!("no active tab"));
        self.step(Command::Run {
            action: ActionId(action),
            ctx,
        })
    }

    /// Waits until nothing has arrived for a while and nothing runs.
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
                assert!(Instant::now() < deadline, "did not settle: {:?}", self.app);
            }
        });
    }

    fn tab(&self) -> &model::Tab {
        self.app
            .model()
            .active_tab(self.pane)
            .map_or_else(|| panic!("no active tab"), |(_, tab)| tab)
    }

    fn location(&self) -> PathBuf {
        platform::paths::to_path(&self.tab().location.path).unwrap_or_else(|e| panic!("{e}"))
    }

    fn rows(&self) -> Vec<&Entry> {
        self.tab().listing.rows().collect()
    }

    fn names(&self) -> Vec<String> {
        self.rows().iter().map(|e| e.name.clone()).collect()
    }

    fn id_of(&self, name: &str) -> EntryId {
        self.rows()
            .iter()
            .find(|e| e.name == name)
            .map_or_else(|| panic!("no entry {name} in {:?}", self.names()), |e| e.id)
    }

    fn select(&mut self, names: &[&str]) -> Vec<Event> {
        let ids = names.iter().map(|n| self.id_of(n)).collect();
        self.step(Command::Select {
            pane: self.pane,
            change: SelectionChange::Set(ids),
        })
    }

    fn navigate(&mut self, to: &Path) -> Vec<Event> {
        self.step(Command::Navigate {
            pane: self.pane,
            to: local(to),
        })
    }
}

/// Names on disk, sorted the way the default view sorts them.
fn on_disk(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{e}"))
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort_unstable();
    names
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort_unstable();
    names
}

fn listed_done(events: &[Event], pane: PaneId) -> bool {
    events
        .iter()
        .any(|e| matches!(e, Event::Listed { pane: p, done: true, .. } if *p == pane))
}

fn job_results(events: &[Event]) -> Vec<bool> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::JobDone { result, .. } => Some(result.is_ok()),
            _ => None,
        })
        .collect()
}

/// Whether `rows` are in the order `by`/`dir` asks for, folders first.
fn in_order(rows: &[&Entry], by: Column, dir: SortDir) -> bool {
    rows.windows(2).all(|pair| {
        let (a, b) = (pair[0], pair[1]);
        match (a.kind == EntryKind::Dir, b.kind == EntryKind::Dir) {
            (true, false) => return true,
            (false, true) => return false,
            _ => {}
        }
        let ord = match by {
            Column::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            Column::Type => type_of(a).cmp(&type_of(b)),
            Column::Modified => a.modified.cmp(&b.modified),
            Column::Created => a.created.cmp(&b.created),
            Column::Accessed => a.accessed.cmp(&b.accessed),
            Column::Size | Column::Items => a.size.cmp(&b.size),
        };
        match dir {
            SortDir::Ascending => ord.is_le(),
            SortDir::Descending => ord.is_ge(),
        }
    })
}

// The script is one story, read top to bottom.
#[allow(clippy::too_many_lines)]
#[test]
fn a_command_script_drives_the_app_headlessly() {
    let fixture = tests::fixture::generate(3_000, 3).unwrap_or_else(|e| panic!("{e}"));
    let root = fixture.root().to_path_buf();
    let mut d = Driver::new(&root);
    let pane = d.pane;

    // Start: the root is listed and complete.
    assert!(d.tab().listing.complete);
    assert_eq!(sorted(d.names()), on_disk(&root));
    assert!(
        d.app.model().jobs.is_empty(),
        "listings are not jobs the user sees"
    );

    // Navigate into the first folder.
    let first_dir = d
        .rows()
        .iter()
        .find(|e| e.kind == EntryKind::Dir)
        .map_or_else(|| panic!("the fixture has folders"), |e| e.name.clone());
    let dir_a = root.join(&first_dir);
    let events = d.navigate(&dir_a);
    assert!(listed_done(&events, pane), "{events:?}");
    assert_eq!(d.location(), dir_a);
    assert_eq!(sorted(d.names()), on_disk(&dir_a));
    assert_eq!(d.tab().history.back, [local(&root)]);

    // Back and forward.
    let events = d.step(Command::Back(pane));
    assert!(listed_done(&events, pane));
    assert_eq!(d.location(), root);
    assert_eq!(d.tab().history.forward, [local(&dir_a)]);
    let events = d.step(Command::Forward(pane));
    assert!(listed_done(&events, pane));
    assert_eq!(d.location(), dir_a);
    assert_eq!(d.tab().history.forward, [] as [types::Location; 0]);

    // Sort by every column, both ways; no I/O, just the view.
    let all = sorted(d.names());
    for by in Column::ALL {
        for dir in [SortDir::Ascending, SortDir::Descending] {
            let events = d.step(Command::Sort { pane, by, dir });
            assert!(events.is_empty(), "sorting is silent: {events:?}");
            assert_eq!(d.tab().sort, model::Sort { by, dir });
            assert!(
                in_order(&d.rows(), by, dir),
                "{by:?} {dir:?}: {:?}",
                d.names()
            );
            assert_eq!(sorted(d.names()), all, "sorting keeps every row");
        }
    }
    d.step(Command::Sort {
        pane,
        by: Column::Name,
        dir: SortDir::Ascending,
    });

    // Filter, then clear it.
    let needle = "f0";
    d.step(Command::Filter {
        pane,
        text: needle.to_owned(),
    });
    assert_eq!(d.tab().filter, needle);
    assert_ne!(d.rows().len(), 0);
    assert!(
        d.names().iter().all(|n| n.contains(needle)),
        "{:?}",
        d.names()
    );
    let expected: Vec<String> = all.iter().filter(|n| n.contains(needle)).cloned().collect();
    assert_eq!(sorted(d.names()), expected);
    d.step(Command::Filter {
        pane,
        text: String::new(),
    });
    assert_eq!(sorted(d.names()), all);

    // Select all.
    let events = d.step(Command::Select {
        pane,
        change: SelectionChange::All,
    });
    let ids = d.tab().listing.row_ids();
    assert!(
        matches!(events.as_slice(), [Event::SelectionChanged { pane: p, ids: got }] if *p == pane && *got == ids),
        "{events:?}"
    );
    assert_eq!(d.tab().selection.len(), all.len());

    // Copy two files from this folder, paste them into a new folder.
    let files: Vec<String> = d
        .rows()
        .iter()
        .filter(|e| e.kind == EntryKind::File)
        .take(2)
        .map(|e| e.name.clone())
        .collect();
    assert_eq!(files.len(), 2, "{:?}", d.names());
    let picked: Vec<&str> = files.iter().map(String::as_str).collect();
    let events = d.select(&picked);
    assert!(matches!(events.as_slice(), [Event::SelectionChanged { ids, .. }] if ids.len() == 2));
    assert!(d.run("core.copy").is_empty());

    let dir_b = root.join("pasted");
    fs::create_dir(&dir_b).unwrap_or_else(|e| panic!("{e}"));
    d.navigate(&dir_b);
    assert_eq!(d.names(), [] as [String; 0]);
    let events = d.run("core.paste");
    assert_eq!(job_results(&events), [true], "{events:?}");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::JobProgress { .. }))
    );
    assert_eq!(sorted(d.names()), sorted(files.clone()));
    assert_eq!(on_disk(&dir_b), sorted(files.clone()));
    assert!(dir_a.join(&files[0]).exists(), "a copy leaves the source");
    assert!(d.app.undo_label().is_none(), "a copy is not undoable");

    // Cut one of the copies and paste it into the root.
    d.select(&[&files[0]]);
    d.run("core.cut");
    d.navigate(&root);
    let events = d.run("core.paste");
    assert_eq!(job_results(&events), [true], "{events:?}");
    assert!(root.join(&files[0]).exists());
    assert!(!dir_b.join(&files[0]).exists());
    assert!(d.names().contains(&files[0]));
    assert_eq!(
        d.app.undo_label().as_deref(),
        Some(format!("Move {}", files[0]).as_str())
    );

    // Rename it.
    *d.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = Some("renamed.bin".into());
    d.select(&[&files[0]]);
    let events = d.run("core.rename");
    assert_eq!(job_results(&events), [true], "{events:?}");
    assert!(root.join("renamed.bin").exists());
    assert!(!root.join(&files[0]).exists());
    assert!(d.names().contains(&"renamed.bin".to_owned()));
    assert_eq!(
        d.app.undo_label().as_deref(),
        Some(format!("Rename {} to renamed.bin", files[0]).as_str())
    );

    // Trash it.
    d.select(&["renamed.bin"]);
    let events = d.run("core.trash");
    assert_eq!(job_results(&events), [true], "{events:?}");
    assert!(!root.join("renamed.bin").exists());
    assert!(!d.names().contains(&"renamed.bin".to_owned()));
    assert_eq!(d.fake.list().map(|l| l.len()).ok(), Some(1));
    assert_eq!(
        d.app.undo_label().as_deref(),
        Some("Move to trash renamed.bin")
    );
    assert!(
        d.tab().selection.is_empty(),
        "the trashed entry leaves the selection"
    );

    // Undo three times: restore, un-rename, move back.
    let events = d.step(Command::Undo);
    assert_eq!(job_results(&events), [true], "{events:?}");
    assert!(root.join("renamed.bin").exists());
    assert!(d.names().contains(&"renamed.bin".to_owned()));
    assert_eq!(d.fake.list().map(|l| l.len()).ok(), Some(0));
    assert_eq!(
        d.app.redo_label().as_deref(),
        Some("Move to trash renamed.bin")
    );

    let events = d.step(Command::Undo);
    assert_eq!(job_results(&events), [true], "{events:?}");
    assert!(root.join(&files[0]).exists());
    assert!(!root.join("renamed.bin").exists());

    let events = d.step(Command::Undo);
    assert_eq!(job_results(&events), [true], "{events:?}");
    assert!(dir_b.join(&files[0]).exists());
    assert!(!root.join(&files[0]).exists());
    assert!(!d.names().contains(&files[0]));
    assert!(d.app.undo_label().is_none());

    // Redo the move.
    let events = d.step(Command::Redo);
    assert_eq!(job_results(&events), [true], "{events:?}");
    assert!(root.join(&files[0]).exists());
    assert!(d.names().contains(&files[0]));
    assert_eq!(
        d.app.undo_label().as_deref(),
        Some(format!("Move {}", files[0]).as_str())
    );
    assert_eq!(
        d.app.redo_label().as_deref(),
        Some(format!("Rename {} to renamed.bin", files[0]).as_str())
    );

    // Final state: one pane, one tab at the root, every job done, nothing
    // running, and the model agrees with the disk.
    assert_eq!(d.app.model().dock.panes(), [pane]);
    assert_eq!(d.location(), root);
    assert_eq!(d.tab().history.back.len(), 3);
    assert_eq!(sorted(d.names()), on_disk(&root));
    assert!(d.tab().listing.complete);
    assert!(!d.app.is_busy());
    let states: Vec<&model::JobState> = d.app.model().jobs.values().map(|j| &j.state).collect();
    assert_eq!(states.len(), 8, "{states:?}");
    assert!(
        states.iter().all(|s| **s == model::JobState::Done),
        "{states:?}"
    );
    let notices = d.ui.notices.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(notices.is_empty(), "{notices:?}");
}

#[test]
fn settings_edited_on_disk_reach_the_model_within_500_ms() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(dir.path());
    assert_eq!(d.app.model().settings, model::Settings::default());
    let path = d.app.settings_path().to_path_buf();

    fs::write(&path, "theme = \"dark\"\nfolders_first = false\n").unwrap_or_else(|e| panic!("{e}"));
    let started = Instant::now();
    runtime::block_on(async {
        while d.app.model().settings.theme != model::Theme::Dark {
            assert!(
                started.elapsed() < Duration::from_millis(500),
                "settings not reflected after {:?}",
                started.elapsed()
            );
            let _ =
                tokio::time::timeout(Duration::from_millis(10), d.app.tick(&mut d.events)).await;
        }
    });
    assert!(!d.app.model().settings.folders_first);
    assert!(
        d.app.model().settings.show_extensions,
        "unset keys keep their defaults"
    );
}
