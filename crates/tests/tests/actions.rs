//! Acceptance (issue 23): the action plugins, headless through the app.
//!
//! Compress and extract round-trip through the operations area with the
//! conflict dialogue; a two-file diff finds the changed lines and two
//! panes' folders diff by name, size and date; a bulk rename of 1 000
//! files with a numbering pattern is one undo; Open with lists at least
//! the default application; Paste as link creates a symlink; Select by
//! pattern selects the expected set; the console opens at the location
//! and runs `pwd` (`cd` on Windows); all four plugins load through the
//! host; and no "not part of version 0.1" stub remains in the sources.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event};
use ext::platform::OpenWith as _;
use ext::{
    ActionId, Diff, Form, HunkKind, Invocation, Level, RowStatus, UiHost, Value, Values, View,
};
use model::{SelectionChange, SplitDir, TabKind};
use platform::{ConfigDir, Fake};
use plugins_native::{Host, Library, MANIFEST_FILE, Options, PluginStatus, Source};
use tests::local;
use types::{ConflictAnswer, EntryKind, Resolution};

/// A UI whose form answers the script sets in advance.
#[derive(Default)]
struct Scripted {
    form: Mutex<Option<Vec<(&'static str, Value)>>>,
}

impl Scripted {
    fn answer_form(&self, values: Vec<(&'static str, Value)>) {
        *self.form.lock().unwrap_or_else(PoisonError::into_inner) = Some(values);
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
        let overrides = self
            .form
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()?;
        let mut values = form.defaults();
        for (key, value) in overrides {
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

/// The four action plugins plus the archive provider the compress plugin
/// needs for its conflict sources.
fn plugins() -> Vec<Arc<dyn ext::Extension>> {
    vec![
        Arc::new(ext_vfs_archive::ArchiveExtension::new()),
        Arc::new(ext_action_compress::CompressExtension::new()),
        Arc::new(ext_action_compare::CompareExtension::new()),
        Arc::new(ext_action_terminal::TerminalExtension::new()),
        Arc::new(ext_action_rename::RenameExtension::new()),
    ]
}

impl Driver {
    fn new(start: &Path) -> Self {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let ui = Arc::new(Scripted {
            form: Mutex::new(Some(Vec::new())),
        });
        let app = App::new(Config {
            platform: fake.clone(),
            ui: ui.clone(),
            extensions: plugins(),
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
            .action_ctx(self.pane, Invocation::Shortcut)
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
                // A job waiting on the conflict dialogue counts as settled.
                let waiting = !self.app.model().conflicts.is_empty();
                if quiet && (!self.app.is_busy() || waiting) {
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

    fn navigate(&mut self, to: &Path) {
        self.step(Command::Navigate {
            pane: self.pane,
            to: local(to),
        });
    }

    /// The view the last events opened, if any.
    fn opened_view(events: &[Event]) -> Option<(model::TabId, View)> {
        events.iter().find_map(|e| match e {
            Event::ViewOpened { tab, view, .. } => Some((*tab, view.clone())),
            _ => None,
        })
    }
}

fn tree(root: &Path) {
    fs::create_dir_all(root.join("docs/inner")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join("docs/a.txt"), "alpha").unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join("docs/inner/b.txt"), "beta").unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join("note.md"), "# note").unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn compress_and_extract_round_trip_through_the_app_with_the_conflict_dialogue() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("work");
    tree(&root);
    let mut d = Driver::new(&root);
    d.select(&["docs", "note.md"]);
    let events = d.run("compress.compress");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::JobDone { result: Ok(()), .. })),
        "{events:?}"
    );
    assert!(
        d.names().contains(&"work.zip".to_owned()),
        "{:?}",
        d.names()
    );

    // Extract to folder: a folder named after the archive, listed after
    // the job's refresh.
    d.select(&["work.zip"]);
    d.run("compress.extract_folder");
    assert!(d.names().contains(&"work".to_owned()), "{:?}", d.names());
    assert_eq!(
        fs::read_to_string(root.join("work/docs/inner/b.txt"))
            .ok()
            .as_deref(),
        Some("beta")
    );

    // Extract here over the existing tree asks through the conflict
    // dialogue; the first answer skips, the second overwrites all.
    fs::write(root.join("docs/a.txt"), "changed").unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join("note.md"), "edited").unwrap_or_else(|e| panic!("{e}"));
    d.select(&["work.zip"]);
    let events = d.run("compress.extract_here");
    let conflict = events
        .iter()
        .find_map(|e| match e {
            Event::Conflict(c) => Some(c.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no conflict asked: {events:?}"));
    assert_eq!(conflict.source.provider, types::ProviderId::ARCHIVE);
    let first =
        String::from_utf8_lossy(conflict.target.path.file_name().unwrap_or_default()).into_owned();
    let events = d.step(Command::ResolveConflict {
        id: conflict.id,
        answer: ConflictAnswer::once(Resolution::Skip),
    });
    let second = events
        .iter()
        .find_map(|e| match e {
            Event::Conflict(c) => Some(c.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no second conflict: {events:?}"));
    let events = d.step(Command::ResolveConflict {
        id: second.id,
        answer: ConflictAnswer {
            resolution: Resolution::Overwrite,
            apply_to_all: true,
        },
    });
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::JobDone { result: Ok(()), .. })),
        "{events:?}"
    );
    // One of the two files kept its edit (skipped), the other came back
    // from the archive (overwritten).
    let a = fs::read_to_string(root.join("docs/a.txt")).unwrap_or_default();
    let note = fs::read_to_string(root.join("note.md")).unwrap_or_default();
    if first == "a.txt" {
        assert_eq!((a.as_str(), note.as_str()), ("changed", "# note"));
    } else {
        assert_eq!((a.as_str(), note.as_str()), ("alpha", "edited"));
    }
}

// Two files, then two panes, in one story.
#[allow(clippy::too_many_lines)]
#[test]
fn two_files_diff_and_two_panes_folders_diff() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("work");
    fs::create_dir_all(&root).unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join("b.txt"), "one\n2\nthree\nfour\n").unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(&root);
    d.select(&["a.txt", "b.txt"]);
    let events = d.run("compare.compare");
    let (tab, view) = Driver::opened_view(&events).unwrap_or_else(|| panic!("no view: {events:?}"));
    let View::Compare(compare) = view else {
        panic!("not a comparison: {view:?}");
    };
    assert_eq!(
        d.app.model().tab(tab).map(|t| t.kind),
        Some(TabKind::Compare)
    );
    assert_eq!(
        d.app
            .model()
            .tab(tab)
            .and_then(|t| t.title.clone())
            .as_deref(),
        Some("a.txt ↔ b.txt")
    );
    let result = compare
        .result()
        .and_then(Result::ok)
        .unwrap_or_else(|| panic!("no result"));
    let Diff::Text(diff) = &*result else {
        panic!("not text: {result:?}");
    };
    assert_eq!(diff.changed_lines(), 2);
    assert!(diff.hunks.iter().any(|h| h.kind == HunkKind::Replace));
    // The compare tab is not part of the session.
    let session = d.app.model().session();
    assert!(
        session
            .windows
            .iter()
            .flat_map(|w| w.panes.iter())
            .flat_map(|p| p.tabs.iter())
            .all(|t| t.kind == TabKind::Explorer)
    );

    // Two panes: the other pane's folder is the right side.
    let other_root = dir.path().join("other");
    fs::create_dir_all(&other_root).unwrap_or_else(|e| panic!("{e}"));
    fs::write(other_root.join("a.txt"), "one\ntwo\nthree\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(other_root.join("c.txt"), "c").unwrap_or_else(|e| panic!("{e}"));
    d.step(Command::CloseTab(tab));
    let first = d.pane;
    d.step(Command::Split {
        pane: first,
        dir: SplitDir::Right,
    });
    let window = d
        .app
        .model()
        .window_of(first)
        .unwrap_or_else(|| panic!("window"));
    let panes = d
        .app
        .model()
        .window(window)
        .map_or_default(|w| w.dock().panes());
    let second = panes
        .into_iter()
        .find(|p| *p != first)
        .unwrap_or_else(|| panic!("no second pane"));
    d.pane = second;
    d.navigate(&other_root);
    d.pane = first;
    d.step(Command::FocusPane(first));
    d.step(Command::Select {
        pane: first,
        change: SelectionChange::Clear,
    });
    let ctx = d
        .app
        .action_ctx(first, Invocation::Palette)
        .unwrap_or_else(|| panic!("ctx"));
    assert_eq!(ctx.other, Some(local(&other_root)));
    let events = d.run("compare.compare");
    let (_, view) = Driver::opened_view(&events).unwrap_or_else(|| panic!("no view: {events:?}"));
    let View::Compare(compare) = view else {
        panic!("not a comparison: {view:?}");
    };
    let result = compare
        .result()
        .and_then(Result::ok)
        .unwrap_or_else(|| panic!("no result"));
    let Diff::Folders(folders) = &*result else {
        panic!("not folders: {result:?}");
    };
    let rows: Vec<(String, RowStatus)> = folders
        .rows
        .iter()
        .map(|r| (r.name.clone(), r.status))
        .collect();
    assert_eq!(
        rows,
        [
            ("a.txt".to_owned(), RowStatus::Same),
            ("b.txt".to_owned(), RowStatus::Removed),
            ("c.txt".to_owned(), RowStatus::Added),
        ]
    );
}

#[test]
fn bulk_rename_of_a_thousand_files_is_one_undo() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("shots");
    fs::create_dir_all(&root).unwrap_or_else(|e| panic!("{e}"));
    for i in 0..1000 {
        fs::write(root.join(format!("img {i}.jpg")), i.to_string())
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let mut d = Driver::new(&root);
    let before = d.names();
    assert_eq!(before.len(), 1000);
    d.step(Command::Select {
        pane: d.pane,
        change: SelectionChange::All,
    });
    d.ui.answer_form(vec![("pattern", Value::Text("holiday_{n}{ext}".into()))]);
    let events = d.run("rename.bulk");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::JobDone { result: Ok(()), .. })),
        "{events:?}"
    );
    let after = d.names();
    assert_eq!(after.len(), 1000);
    assert!(
        after.contains(&"holiday_0001.jpg".to_owned()),
        "{:?}",
        &after[..3]
    );
    assert!(after.contains(&"holiday_1000.jpg".to_owned()));
    assert_eq!(d.app.undo_label().as_deref(), Some("Rename 1000 entries"));
    d.step(Command::Undo);
    assert_eq!(d.names(), before, "one undo brings every name back");
    assert_eq!(d.app.undo_label(), None);
    d.step(Command::Redo);
    assert_eq!(d.names(), after);
}

#[test]
fn open_with_lists_at_least_the_default_application() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("work");
    fs::create_dir_all(&root).unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join("notes.txt"), "text").unwrap_or_else(|e| panic!("{e}"));
    // Through the app with the fake platform: the second application.
    let mut d = Driver::new(&root);
    d.select(&["notes.txt"]);
    d.ui.answer_form(vec![("app", Value::Choice(1))]);
    d.run("core.open_with");
    let opened = d.fake.opened_with();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].1.name, platform::FAKE_APPLICATIONS[1].0);

    // The real platform knows at least one application for a text file,
    // the default first, wherever a desktop is installed.
    let native = platform::Native::new();
    let apps = native
        .applications(&root.join("notes.txt"))
        .unwrap_or_else(|e| panic!("{e}"));
    let has_desktop = std::env::consts::OS != "linux"
        || std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|p| p.join("xdg-open").is_file()))
        || Path::new("/usr/share/applications/mimeinfo.cache").is_file();
    if has_desktop {
        assert!(!apps.is_empty(), "no application for a text file");
        assert!(apps.iter().all(|a| !a.name.is_empty() && !a.id.is_empty()));
    }
}

#[test]
fn paste_as_link_creates_a_symlink() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("work");
    fs::create_dir_all(root.join("links")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join("target.txt"), "T").unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(&root);
    d.select(&["target.txt"]);
    d.run("core.copy");
    d.navigate(&root.join("links"));
    let events = d.run("core.paste_link");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::JobDone { result: Ok(()), .. })),
        "{events:?}"
    );
    let link = root.join("links/target.txt");
    let meta = fs::symlink_metadata(&link).unwrap_or_else(|e| panic!("{e}"));
    assert!(meta.file_type().is_symlink());
    assert_eq!(fs::read_to_string(&link).ok().as_deref(), Some("T"));
    let listed = d
        .tab()
        .listing
        .rows()
        .find(|e| e.name == "target.txt")
        .map(|e| e.kind.clone());
    assert!(
        matches!(listed, Some(EntryKind::Symlink { .. })),
        "{listed:?}"
    );
    assert_eq!(
        d.app.undo_label().as_deref(),
        Some("Link target.txt into links")
    );
    d.step(Command::Undo);
    assert!(
        fs::symlink_metadata(&link).is_err(),
        "undo removed the link"
    );
}

#[test]
fn select_by_pattern_selects_the_expected_set() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("work");
    fs::create_dir_all(&root).unwrap_or_else(|e| panic!("{e}"));
    for name in [
        "report.pdf",
        "notes.TXT",
        "draft.txt",
        "Image.png",
        "readme",
    ] {
        fs::write(root.join(name), "x").unwrap_or_else(|e| panic!("{e}"));
    }
    let mut d = Driver::new(&root);
    d.ui.answer_form(vec![("pattern", Value::Text("*.txt".into()))]);
    let events = d.run("core.select_pattern");
    let selected: Vec<String> = events
        .iter()
        .find_map(|e| match e {
            Event::SelectionChanged { ids, .. } => Some(ids.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no selection change: {events:?}"))
        .into_iter()
        .map(|id| d.tab().entry(id).map_or_default(|e| e.name.clone()))
        .collect();
    assert_eq!(selected, ["draft.txt", "notes.TXT"]);
    // Adding a regular expression's matches.
    d.ui.answer_form(vec![
        ("pattern", Value::Text("^[a-z]+\\.p(df|ng)$".into())),
        ("kind", Value::Choice(1)),
        ("mode", Value::Choice(1)),
    ]);
    d.run("core.select_pattern");
    let mut names: Vec<String> = d.tab().selected().map(|e| e.name.clone()).collect();
    names.sort();
    assert_eq!(names, ["draft.txt", "notes.TXT", "report.pdf"]);
}

#[test]
fn the_console_opens_at_the_location_and_runs_pwd() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = fs::canonicalize(dir.path()).unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(&root);
    let events = d.run("terminal.console");
    let (tab, view) = Driver::opened_view(&events).unwrap_or_else(|| panic!("no view: {events:?}"));
    // The event was this test's copy; the UI would hold the view.
    drop(events);
    let View::Console(console) = view else {
        panic!("not a console: {view:?}");
    };
    assert_eq!(
        d.app.model().tab(tab).map(|t| t.kind),
        Some(TabKind::Console)
    );
    // A view opens in a workspace tab of its own, shown and focused.
    let console_pane = d.app.model().focused;
    assert_ne!(console_pane, d.pane);
    assert_eq!(
        d.app.model().active_tab(console_pane).map(|(id, _)| id),
        Some(tab),
        "the console tab is active"
    );
    assert!(
        d.app
            .action_ctx(console_pane, Invocation::Shortcut)
            .is_none()
    );
    console.resize(120, 30);
    let command: &[u8] = if std::env::consts::OS == "windows" {
        b"cd\r\n"
    } else {
        b"pwd\n"
    };
    console.write(command);
    let wanted = root.display().to_string();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut text = String::new();
    while Instant::now() < deadline {
        let screen = console.screen(0);
        text = screen.cells.iter().map(|c| c.ch).collect();
        if text.contains(&wanted) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        text.contains(&wanted),
        "the location did not appear:\n{text}"
    );
    // Closing the tab drops the console, which ends the shell.
    let weak = Arc::downgrade(&console);
    drop(console);
    d.step(Command::CloseTab(tab));
    assert!(weak.upgrade().is_none(), "the session is gone with its tab");
}

/// The directory the test binary runs from, where cargo puts every
/// dependency's `cdylib` under its plain name.
fn deps_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    exe.parent()
        .unwrap_or_else(|| panic!("no parent of {}", exe.display()))
        .to_owned()
}

/// Installs the library of the crate `stem` as a plugin into `plugins_dir`
/// with the manifest the bundler would write.
fn install(plugins_dir: &Path, stem: &str) {
    let library = deps_dir().join(format!(
        "{}{stem}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(library.is_file(), "not built: {}", library.display());
    let mut toml = plugins_native::describe(&library).unwrap_or_else(|e| panic!("{e}"));
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
fn all_four_action_plugins_load_through_the_host() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    for stem in [
        "ext_action_compress",
        "ext_action_compare",
        "ext_action_terminal",
        "ext_action_rename",
    ] {
        install(&plugins, stem);
    }
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Host::load(&Options {
        dirs: vec![(Source::BesideBinary, plugins)],
        platform: Some(fake),
        ..Options::default()
    });
    for id in [
        "action-compress",
        "action-compare",
        "action-terminal",
        "action-rename",
    ] {
        let plugin = host
            .plugins()
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("no plugin {id} in {:?}", host.plugins()));
        assert_eq!(plugin.status, PluginStatus::Loaded, "{id}");
        assert_eq!(plugin.points, ext::Points::ACTION, "{id}");
    }
    let mut registry = ext::Registry::new();
    for extension in host.extensions() {
        registry.load(&*extension).unwrap_or_else(|e| panic!("{e}"));
    }
    let ids: Vec<&str> = registry.actions().iter().map(|a| a.id().0).collect();
    for wanted in [
        "compress.compress",
        "compress.compress_as",
        "compress.extract_here",
        "compress.extract_folder",
        "compress.extract_to",
        "compare.compare",
        "terminal.console",
        "rename.bulk",
    ] {
        assert!(ids.contains(&wanted), "{wanted} missing from {ids:?}");
    }
    assert_eq!(
        registry.settings_schemas().len(),
        1,
        "the console's setting"
    );
}

/// Every `.rs` file under `dir` that still carries a stub's text.
fn stub_hits(dir: &Path, hits: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.unwrap_or_else(|e| panic!("{e}")).path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            stub_hits(&path, hits);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = fs::read_to_string(&path).unwrap_or_default();
            if text.contains("is not part of version 0.1") || text.contains("fn unavailable(") {
                hits.push(path);
            }
        }
    }
}

#[test]
fn no_stub_notification_remains() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("crates");
    let mut hits = Vec::new();
    stub_hits(&root, &mut hits);
    // This test names the text it looks for.
    hits.retain(|p| !p.ends_with("tests/tests/actions.rs"));
    assert!(hits.is_empty(), "stubs remain: {hits:?}");
}
