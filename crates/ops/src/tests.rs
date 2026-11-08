use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ext::{
    Action, ActionCtx, ActionId, ActionOutcome, Availability, Extension, Invocation, Level, PaneId,
    Registry, UiHost,
};
use platform::{ConfigDir as _, Fake, Trash as _};
use runtime::JobRunner;
use types::{Entry, Location};
use vfs::Mounts;
use vfs_local::LocalProvider;

use super::{ClipMode, Conflicts, Core, PinStore, Scheduler, TransferKind};

/// A host whose answers the test scripts.
#[derive(Default)]
struct Scripted {
    confirm: Mutex<bool>,
    prompt: Mutex<Option<String>>,
    notices: Mutex<Vec<(Level, String)>>,
    /// Values laid over a form's defaults; `None` dismisses the form.
    form: Mutex<Option<Vec<(&'static str, ext::Value)>>>,
}

impl Scripted {
    fn answer_form(&self, values: Vec<(&'static str, ext::Value)>) {
        *self.form.lock().unwrap_or_else(PoisonError::into_inner) = Some(values);
    }
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
    fn show(&self, _title: &str, _text: &str) {}
    fn form(&self, form: ext::Form) -> Option<ext::Values> {
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

/// A pinned list in memory.
#[derive(Default)]
struct Pins(Mutex<Vec<Location>>);

impl PinStore for Pins {
    fn pinned(&self) -> Vec<Location> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    fn pin(&self, items: Vec<Location>, at: Option<usize>) {
        let mut pinned = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut index = at.map_or(pinned.len(), |at| at.min(pinned.len()));
        for item in items {
            if !pinned.contains(&item) {
                pinned.insert(index, item);
                index += 1;
            }
        }
    }
    fn unpin(&self, item: &Location) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|p| p != item);
    }
}

struct Harness {
    dir: tempfile::TempDir,
    fake: Arc<Fake>,
    pins: Arc<Pins>,
    conflicts: Arc<Conflicts>,
    scheduler: Arc<Scheduler>,
    mounts: Arc<Mounts>,
    registry: Registry,
    core_clipboard: Arc<dyn Fn() -> Option<super::Clipboard> + Send + Sync>,
    jobs: JobRunner,
    ui: Scripted,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mounts = Arc::new(Mounts::new());
    mounts.mount(Arc::new(LocalProvider::new(fake.clone())));
    let pins = Arc::new(Pins::default());
    let conflicts = Conflicts::new();
    let scheduler = Scheduler::new();
    let core = Arc::new(Core::new(
        Arc::clone(&mounts),
        fake.clone(),
        pins.clone(),
        Arc::clone(&conflicts),
        Arc::clone(&scheduler),
    ));
    let mut registry = Registry::new();
    registry.load(&*core).unwrap_or_else(|e| panic!("{e}"));
    let clip = Arc::clone(&core);
    Harness {
        dir,
        fake,
        pins,
        conflicts,
        scheduler,
        mounts,
        registry,
        core_clipboard: Arc::new(move || clip.clipboard()),
        jobs: JobRunner::new(),
        ui: Scripted {
            form: Mutex::new(Some(Vec::new())),
            ..Scripted::default()
        },
    }
}

impl Harness {
    fn action(&self, id: &'static str) -> Arc<dyn Action> {
        self.registry
            .actions()
            .iter()
            .find(|a| a.id() == ActionId(id))
            .cloned()
            .unwrap_or_else(|| panic!("no action {id}"))
    }

    fn entries(&self, paths: &[&Path]) -> Vec<Entry> {
        let provider = self
            .mounts
            .provider(vfs_local::PROVIDER_ID)
            .unwrap_or_else(|| panic!("local"));
        runtime::block_on(async {
            let mut out = Vec::new();
            for path in paths {
                out.push(
                    provider
                        .stat(&tests_local(path))
                        .await
                        .unwrap_or_else(|e| panic!("{e}")),
                );
            }
            out
        })
    }

    fn ctx(&self, location: &Path, selection: &[&Path]) -> ActionCtx {
        ActionCtx {
            pane: PaneId(1),
            location: tests_local(location),
            selection: self.entries(selection),
            invoked_from: Invocation::Palette,
            other: None,
        }
    }

    fn run(&self, id: &'static str, ctx: ActionCtx) -> ActionOutcome {
        let action = self.action(id);
        assert_eq!(action.availability(&ctx), Availability::Enabled, "{id}");
        let outcome = action
            .run(ctx, &self.jobs, &self.ui)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        self.settle();
        outcome
    }

    /// Waits for every job the runner knows to finish.
    fn settle(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.jobs.running().is_empty() {
            assert!(
                Instant::now() < deadline,
                "jobs still running: {:?}",
                self.jobs.running()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn tests_local(path: &Path) -> Location {
    Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path))
}

fn write(path: &Path, content: &str) {
    fs::write(path, content).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn copy_then_paste_copies_and_keeps_the_clipboard() {
    let h = harness();
    let (src, dst) = (h.dir.path().join("src"), h.dir.path().join("dst"));
    fs::create_dir_all(&src).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(&dst).unwrap_or_else(|e| panic!("{e}"));
    write(&src.join("a.txt"), "A");
    fs::create_dir(src.join("sub")).unwrap_or_else(|e| panic!("{e}"));
    write(&src.join("sub/b.txt"), "B");

    let paste = h.action("core.paste");
    assert!(matches!(
        paste.availability(&h.ctx(&dst, &[])),
        Availability::Disabled(_)
    ));

    let outcome = h.run(
        "core.copy",
        h.ctx(&src, &[&src.join("a.txt"), &src.join("sub")]),
    );
    assert!(matches!(outcome, ActionOutcome::Done));
    assert_eq!((h.core_clipboard)().map(|c| c.mode), Some(ClipMode::Copy));

    let outcome = h.run("core.paste", h.ctx(&dst, &[]));
    assert!(matches!(outcome, ActionOutcome::Started(_)), "{outcome:?}");
    assert_eq!(
        fs::read_to_string(dst.join("a.txt")).ok().as_deref(),
        Some("A")
    );
    assert_eq!(
        fs::read_to_string(dst.join("sub/b.txt")).ok().as_deref(),
        Some("B")
    );
    assert!(src.join("a.txt").exists(), "a copy leaves the source");
    assert!((h.core_clipboard)().is_some(), "copy keeps the clipboard");
}

#[test]
fn cut_then_paste_moves_and_undoes() {
    let h = harness();
    let (src, dst) = (h.dir.path().join("src"), h.dir.path().join("dst"));
    fs::create_dir_all(&src).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(&dst).unwrap_or_else(|e| panic!("{e}"));
    write(&src.join("a.txt"), "A");

    h.run("core.cut", h.ctx(&src, &[&src.join("a.txt")]));
    let outcome = h.run("core.paste", h.ctx(&dst, &[]));
    let ActionOutcome::Undoable(command) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(!src.join("a.txt").exists());
    assert!(dst.join("a.txt").exists());
    assert!(
        (h.core_clipboard)().is_none(),
        "cut empties the clipboard after paste"
    );
    assert_eq!(command.label(), "Move a.txt");

    command.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert!(src.join("a.txt").exists());
    assert!(!dst.join("a.txt").exists());

    command.redo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert!(dst.join("a.txt").exists());
}

#[test]
fn rename_and_undo() {
    let h = harness();
    let dir = h.dir.path().join("d");
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    write(&dir.join("old.txt"), "x");
    write(&dir.join("other.txt"), "y");

    let rename = h.action("core.rename");
    assert!(matches!(
        rename.availability(&h.ctx(&dir, &[&dir.join("old.txt"), &dir.join("other.txt")])),
        Availability::Disabled(_)
    ));
    *h.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = Some("new.txt".into());
    let outcome = h.run("core.rename", h.ctx(&dir, &[&dir.join("old.txt")]));
    let ActionOutcome::Undoable(command) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(dir.join("new.txt").exists());
    assert!(!dir.join("old.txt").exists());
    command.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert!(dir.join("old.txt").exists());

    *h.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = None;
    assert!(matches!(
        h.run("core.rename", h.ctx(&dir, &[&dir.join("old.txt")])),
        ActionOutcome::Done
    ));
}

#[test]
fn trash_and_undo_restore() {
    let h = harness();
    let dir = h.dir.path().join("d");
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    write(&dir.join("gone.txt"), "x");
    write(&dir.join("too.txt"), "y");

    let outcome = h.run(
        "core.trash",
        h.ctx(&dir, &[&dir.join("gone.txt"), &dir.join("too.txt")]),
    );
    let ActionOutcome::Undoable(command) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(!dir.join("gone.txt").exists());
    assert_eq!(h.fake.list().map(|l| l.len()).ok(), Some(2));
    assert_eq!(command.label(), "Move to trash 2 items");

    command.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert_eq!(
        fs::read_to_string(dir.join("gone.txt")).ok().as_deref(),
        Some("x")
    );
    assert_eq!(
        fs::read_to_string(dir.join("too.txt")).ok().as_deref(),
        Some("y")
    );
    assert_eq!(h.fake.list().map(|l| l.len()).ok(), Some(0));
}

#[test]
fn delete_asks_first() {
    let h = harness();
    let dir = h.dir.path().join("d");
    fs::create_dir_all(dir.join("folder")).unwrap_or_else(|e| panic!("{e}"));
    write(&dir.join("folder/inner.txt"), "x");

    assert!(matches!(
        h.run("core.delete", h.ctx(&dir, &[&dir.join("folder")])),
        ActionOutcome::Done
    ));
    assert!(dir.join("folder").exists());

    *h.ui.confirm.lock().unwrap_or_else(PoisonError::into_inner) = true;
    assert!(matches!(
        h.run("core.delete", h.ctx(&dir, &[&dir.join("folder")])),
        ActionOutcome::Started(_)
    ));
    assert!(!dir.join("folder").exists());
}

#[test]
fn new_file_and_folder_then_undo() {
    let h = harness();
    let dir = h.dir.path().join("d");
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));

    *h.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = Some("notes.txt".into());
    let ActionOutcome::Undoable(file) = h.run("core.new_file", h.ctx(&dir, &[])) else {
        panic!("new file")
    };
    assert_eq!(fs::read(dir.join("notes.txt")).ok(), Some(Vec::new()));

    *h.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = Some("Stuff".into());
    let ActionOutcome::Undoable(folder) = h.run("core.new_folder", h.ctx(&dir, &[])) else {
        panic!("new folder")
    };
    assert!(dir.join("Stuff").is_dir());

    file.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    folder.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert!(!dir.join("notes.txt").exists());
    assert!(!dir.join("Stuff").exists());
}

#[test]
fn duplicate_picks_free_names() {
    let h = harness();
    let dir = h.dir.path().join("d");
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    write(&dir.join("a.txt"), "A");
    h.run("core.duplicate", h.ctx(&dir, &[&dir.join("a.txt")]));
    h.run("core.duplicate", h.ctx(&dir, &[&dir.join("a.txt")]));
    assert_eq!(
        fs::read_to_string(dir.join("a (copy).txt")).ok().as_deref(),
        Some("A")
    );
    assert_eq!(
        fs::read_to_string(dir.join("a (copy 2).txt"))
            .ok()
            .as_deref(),
        Some("A")
    );
}

#[test]
fn copy_path_open_and_reveal_go_to_the_platform() {
    let h = harness();
    let dir = h.dir.path().join("d");
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    write(&dir.join("a.txt"), "A");
    write(&dir.join("b.txt"), "B");
    let both = [dir.join("a.txt"), dir.join("b.txt")];
    let refs: Vec<&Path> = both.iter().map(Path::new).collect();

    // Copy name: the names, one per line.
    h.run("core.copy_name", h.ctx(&dir, &refs));
    assert_eq!(h.fake.clipboard_text().unwrap_or_default(), "a.txt\nb.txt");
    // Copy path: the folders, duplicates removed; the location itself when
    // nothing is selected.
    h.run("core.copy_path", h.ctx(&dir, &refs));
    assert_eq!(
        h.fake.clipboard_text().unwrap_or_default(),
        dir.display().to_string()
    );
    h.run("core.copy_path", h.ctx(&dir, &[]));
    assert_eq!(
        h.fake.clipboard_text().unwrap_or_default(),
        dir.display().to_string()
    );
    // Copy path and name: the full paths.
    h.run("core.copy_path_and_name", h.ctx(&dir, &refs));
    assert_eq!(
        h.fake.clipboard_text().unwrap_or_default(),
        format!("{}\n{}", both[0].display(), both[1].display())
    );
    assert_eq!(
        h.action("core.copy_name").availability(&h.ctx(&dir, &[])),
        Availability::Disabled(ext::t!("reason-select-something-first"))
    );

    h.run("core.open_default", h.ctx(&dir, &refs));
    assert_eq!(h.fake.opened().len(), 2);
    h.run("core.reveal", h.ctx(&dir, &refs));
    assert_eq!(h.fake.revealed(), [dir.join("a.txt")]);
    h.run("core.reveal", h.ctx(&dir, &[]));
    assert_eq!(
        h.fake.opened().len(),
        3,
        "without a selection the location opens"
    );
    assert_eq!(h.fake.opened()[2], dir);
}

#[test]
fn every_action_has_an_id_label_and_core_group() {
    let h = harness();
    assert_eq!(h.registry.actions().len(), 23);
    for action in h.registry.actions() {
        assert!(action.id().0.starts_with("core."), "{}", action.id().0);
        assert_ne!(action.label(), "");
        assert_eq!(action.group(), ext::ActionGroup::Core);
        // Pin, Unpin, Open containing folder and Compute hash have no
        // shortcut in the spec's tables; New file lost Mod+N to New window
        // (QUESTIONS.md 66).
        let unbound = matches!(
            action.id().0,
            "core.pin"
                | "core.unpin"
                | "core.new_file"
                | "core.open_containing"
                | "core.open_containing_tab"
                | "core.create_link"
                | "core.hash"
        );
        assert_eq!(
            action.default_shortcut().is_some(),
            !unbound,
            "{}",
            action.id().0
        );
    }
    for always in ["core.new_file", "core.new_folder", "core.reveal"] {
        assert_eq!(
            h.action(always).availability(&h.ctx(h.dir.path(), &[])),
            Availability::Enabled,
            "{always}"
        );
    }
    let core = Core::new(
        Arc::clone(&h.mounts),
        h.fake.clone(),
        h.pins.clone(),
        Arc::clone(&h.conflicts),
        Arc::clone(&h.scheduler),
    );
    assert_eq!(core.manifest().id, types::ExtId("core"));
    assert!(format!("{core:?}").contains("core"));
}

#[test]
fn pin_and_unpin_flip_and_undo() {
    let h = harness();
    let (file, folder) = (h.dir.path().join("a.txt"), h.dir.path().join("sub"));
    write(&file, "A");
    fs::create_dir(&folder).unwrap_or_else(|e| panic!("{e}"));
    let ctx = h.ctx(h.dir.path(), &[&file, &folder]);
    assert_eq!(
        h.action("core.unpin").availability(&ctx),
        Availability::Hidden,
        "nothing pinned yet"
    );

    let outcome = h.run("core.pin", ctx.clone());
    let ActionOutcome::Undoable(pin) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(pin.label(), "Pin 2 items");
    assert_eq!(
        h.pins.pinned(),
        [tests_local(&file), tests_local(&folder)],
        "both pinned, at the end, in order"
    );
    assert_eq!(
        h.action("core.pin").availability(&ctx),
        Availability::Hidden
    );
    assert_eq!(
        h.action("core.unpin").availability(&ctx),
        Availability::Enabled,
        "the label flips once everything is pinned"
    );
    // A mixed selection offers Pin again.
    let mixed = h.ctx(h.dir.path(), &[&file]);
    write(&h.dir.path().join("b.txt"), "B");
    let mixed_ctx = h.ctx(h.dir.path(), &[&file, &h.dir.path().join("b.txt")]);
    assert_eq!(
        h.action("core.pin").availability(&mixed_ctx),
        Availability::Enabled
    );
    assert_eq!(
        h.action("core.unpin").availability(&mixed_ctx),
        Availability::Hidden
    );

    pin.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    assert!(h.pins.pinned().is_empty(), "undo removes what pin added");
    pin.redo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(h.pins.pinned().len(), 2);

    let outcome = h.run("core.unpin", mixed);
    let ActionOutcome::Undoable(unpin) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(unpin.label(), "Unpin a.txt");
    assert_eq!(h.pins.pinned(), [tests_local(&folder)]);
    unpin.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        h.pins.pinned(),
        [tests_local(&file), tests_local(&folder)],
        "undo puts it back at its old index"
    );
}

#[test]
fn pin_command_inserts_at_a_position_and_skips_what_is_pinned() {
    let h = harness();
    let pins: Arc<dyn PinStore> = h.pins.clone();
    let a = tests_local(&h.dir.path().join("a"));
    let b = tests_local(&h.dir.path().join("b"));
    let c = tests_local(&h.dir.path().join("c"));
    super::pin_command(Arc::clone(&pins), vec![a.clone(), b.clone()], None)
        .redo(&h.jobs)
        .unwrap_or_else(|e| panic!("{e}"));
    let command = super::pin_command(Arc::clone(&pins), vec![c.clone(), a.clone()], Some(1));
    command.redo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(h.pins.pinned(), [a.clone(), c.clone(), b.clone()]);
    assert_eq!(command.label(), "Pin c", "only what was not pinned counts");
    command.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(h.pins.pinned(), [a.clone(), b.clone()], "a stays pinned");
    let unpin = super::unpin_command(Arc::clone(&pins), a.clone());
    unpin.redo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(h.pins.pinned(), std::slice::from_ref(&b));
    unpin.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(h.pins.pinned(), [a, b]);
}

#[test]
fn transfer_moves_with_undo_and_copies_without() {
    let h = harness();
    let (src, dst) = (h.dir.path().join("src"), h.dir.path().join("dst"));
    fs::create_dir_all(src.join("folder")).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(&dst).unwrap_or_else(|e| panic!("{e}"));
    write(&src.join("a.txt"), "A");
    write(&src.join("folder/inner.txt"), "I");

    let outcome = super::transfer(
        &h.jobs,
        Arc::clone(&h.mounts),
        Arc::clone(&h.conflicts),
        Arc::clone(&h.scheduler),
        &[
            tests_local(&src.join("a.txt")),
            tests_local(&src.join("folder")),
        ],
        &tests_local(&dst),
        TransferKind::Move,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    let ActionOutcome::Undoable(command) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(command.label(), "Move 2 items to dst");
    assert!(!src.join("a.txt").exists());
    assert_eq!(fs::read_to_string(dst.join("a.txt")).ok(), Some("A".into()));
    assert!(dst.join("folder/inner.txt").exists());
    command.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert!(src.join("a.txt").exists());
    assert!(src.join("folder/inner.txt").exists());
    assert!(!dst.join("a.txt").exists());

    let outcome = super::transfer(
        &h.jobs,
        Arc::clone(&h.mounts),
        Arc::clone(&h.conflicts),
        Arc::clone(&h.scheduler),
        &[tests_local(&src.join("a.txt"))],
        &tests_local(&dst),
        TransferKind::Copy,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert!(matches!(outcome, ActionOutcome::Started(_)), "{outcome:?}");
    assert!(src.join("a.txt").exists(), "a copy leaves the source");
    assert_eq!(fs::read_to_string(dst.join("a.txt")).ok(), Some("A".into()));

    assert!(matches!(
        super::transfer(
            &h.jobs,
            Arc::clone(&h.mounts),
            Arc::clone(&h.conflicts),
            Arc::clone(&h.scheduler),
            &[],
            &tests_local(&dst),
            TransferKind::Copy,
        ),
        Err(types::Error::Unsupported(_))
    ));
}

/// Answers the first open conflict with `answer` once a job asks, within
/// two seconds.
fn answer_conflict(h: &Harness, answer: types::ConflictAnswer) -> types::Conflict {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let fresh = h.conflicts.take_new();
        if let Some(conflict) = fresh.into_iter().next() {
            assert!(h.conflicts.is_waiting(conflict.id));
            assert!(h.conflicts.answer(conflict.id, answer));
            return conflict;
        }
        assert!(Instant::now() < deadline, "no conflict was asked");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A source folder with `a.txt`, `b.txt` and `sub/inner.txt`, and a target
/// folder that already holds `a.txt`, `b.txt` and `sub/other.txt`.
fn conflicting_tree(h: &Harness) -> (std::path::PathBuf, std::path::PathBuf) {
    let (src, dst) = (h.dir.path().join("src"), h.dir.path().join("dst"));
    fs::create_dir_all(src.join("sub")).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(dst.join("sub")).unwrap_or_else(|e| panic!("{e}"));
    write(&src.join("a.txt"), "new a");
    write(&src.join("b.txt"), "new b");
    write(&src.join("sub/inner.txt"), "inner");
    write(&dst.join("a.txt"), "old a");
    write(&dst.join("b.txt"), "old b");
    write(&dst.join("sub/other.txt"), "other");
    (src, dst)
}

fn paste_all(h: &Harness, src: &Path, dst: &Path) -> ActionOutcome {
    let ctx = h.ctx(
        src,
        &[&src.join("a.txt"), &src.join("b.txt"), &src.join("sub")],
    );
    h.run("core.copy", ctx);
    let action = h.action("core.paste");
    action
        .run(h.ctx(dst, &[]), &h.jobs, &h.ui)
        .unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn a_paste_over_existing_names_asks_and_skip_leaves_the_target_alone() {
    let h = harness();
    let (src, dst) = conflicting_tree(&h);
    let outcome = paste_all(&h, &src, &dst);
    let ActionOutcome::Started(id) = outcome else {
        panic!("{outcome:?}");
    };
    let first = answer_conflict(&h, types::ConflictAnswer::once(types::Resolution::Skip));
    assert_eq!(first.job, id);
    assert!(!first.target_is_dir);
    assert_eq!(first.proposed, "a (2).txt");
    assert_eq!(first.remaining, 2);
    let second = answer_conflict(&h, types::ConflictAnswer::once(types::Resolution::Skip));
    assert_eq!(second.proposed, "b (2).txt");
    let third = answer_conflict(&h, types::ConflictAnswer::once(types::Resolution::Skip));
    assert!(third.target_is_dir);
    h.settle();
    assert_eq!(
        fs::read_to_string(dst.join("a.txt")).ok(),
        Some("old a".into())
    );
    assert_eq!(
        fs::read_to_string(dst.join("b.txt")).ok(),
        Some("old b".into())
    );
    assert!(!dst.join("sub/inner.txt").exists());
    assert_eq!(h.conflicts.take_new().len(), 0, "no new conflict");
}

#[test]
fn overwrite_replaces_files_and_merges_folders() {
    let h = harness();
    let (src, dst) = conflicting_tree(&h);
    paste_all(&h, &src, &dst);
    answer_conflict(
        &h,
        types::ConflictAnswer::for_all(types::Resolution::Overwrite),
    );
    h.settle();
    assert_eq!(
        fs::read_to_string(dst.join("a.txt")).ok(),
        Some("new a".into())
    );
    assert_eq!(
        fs::read_to_string(dst.join("b.txt")).ok(),
        Some("new b".into())
    );
    assert!(dst.join("sub/inner.txt").exists(), "merged in");
    assert!(
        dst.join("sub/other.txt").exists(),
        "merge keeps what was there"
    );
    assert!(src.join("a.txt").exists(), "a copy leaves the source");
}

#[test]
fn rename_puts_the_item_under_the_proposed_name_and_apply_to_all_numbers_the_rest() {
    let h = harness();
    let (src, dst) = conflicting_tree(&h);
    paste_all(&h, &src, &dst);
    answer_conflict(
        &h,
        types::ConflictAnswer::once(types::Resolution::Rename("fresh.txt".into())),
    );
    answer_conflict(
        &h,
        types::ConflictAnswer::for_all(types::Resolution::Rename(String::new())),
    );
    h.settle();
    assert_eq!(
        fs::read_to_string(dst.join("fresh.txt")).ok(),
        Some("new a".into())
    );
    assert_eq!(
        fs::read_to_string(dst.join("a.txt")).ok(),
        Some("old a".into())
    );
    assert_eq!(
        fs::read_to_string(dst.join("b (2).txt")).ok(),
        Some("new b".into())
    );
    assert!(
        dst.join("sub (2)/inner.txt").exists(),
        "folders are numbered too"
    );
    assert!(dst.join("sub/other.txt").exists());
}

#[test]
fn cancel_leaves_everything_untouched_and_ends_the_job_cancelled() {
    let h = harness();
    let (src, dst) = conflicting_tree(&h);
    let (jobs, mut events) = JobRunner::with_events();
    let ctx = h.ctx(
        src.as_path(),
        &[&src.join("a.txt"), &src.join("b.txt"), &src.join("sub")],
    );
    h.run("core.copy", ctx);
    h.action("core.paste")
        .run(h.ctx(&dst, &[]), &jobs, &h.ui)
        .unwrap_or_else(|e| panic!("{e}"));
    answer_conflict(&h, types::ConflictAnswer::once(types::Resolution::Cancel));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !jobs.running().is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut finished = None;
    while let Ok(event) = events.try_recv() {
        if let runtime::JobEvent::Finished { result, .. } = event {
            finished = Some(result);
        }
    }
    assert!(
        matches!(finished, Some(Err(types::Error::Cancelled))),
        "{finished:?}"
    );
    assert_eq!(
        fs::read_to_string(dst.join("a.txt")).ok(),
        Some("old a".into())
    );
    assert_eq!(
        fs::read_to_string(dst.join("b.txt")).ok(),
        Some("old b".into())
    );
    assert!(!dst.join("sub/inner.txt").exists());
}

#[test]
fn a_move_that_overwrites_merges_and_its_undo_moves_back() {
    let h = harness();
    let (src, dst) = conflicting_tree(&h);
    let ctx = h.ctx(src.as_path(), &[&src.join("a.txt"), &src.join("sub")]);
    h.run("core.cut", ctx);
    let outcome = h
        .action("core.paste")
        .run(h.ctx(&dst, &[]), &h.jobs, &h.ui)
        .unwrap_or_else(|e| panic!("{e}"));
    let ActionOutcome::Undoable(command) = outcome else {
        panic!("{outcome:?}");
    };
    answer_conflict(
        &h,
        types::ConflictAnswer::for_all(types::Resolution::Overwrite),
    );
    h.settle();
    assert_eq!(
        fs::read_to_string(dst.join("a.txt")).ok(),
        Some("new a".into())
    );
    assert!(!src.join("a.txt").exists(), "moved");
    assert!(!src.join("sub").exists(), "the folder was merged away");
    assert!(dst.join("sub/inner.txt").exists());
    assert!(dst.join("sub/other.txt").exists());
    command.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert_eq!(
        fs::read_to_string(src.join("a.txt")).ok(),
        Some("new a".into())
    );
    assert!(src.join("sub/inner.txt").exists());
    assert!(!dst.join("a.txt").exists());
}

#[test]
fn open_containing_folder_is_available_away_from_the_selection_folder() {
    let h = harness();
    let dir = h.dir.path().join("d");
    fs::create_dir_all(dir.join("sub")).unwrap_or_else(|e| panic!("{e}"));
    write(&dir.join("sub/deep.txt"), "x");
    write(&dir.join("here.txt"), "y");
    let action = h.action("core.open_containing");
    assert!(matches!(
        action.availability(&h.ctx(&dir, &[])),
        Availability::Disabled(_)
    ));
    assert!(matches!(
        action.availability(&h.ctx(&dir, &[&dir.join("here.txt")])),
        Availability::Disabled(_)
    ));
    assert_eq!(
        action.availability(&h.ctx(&dir, &[&dir.join("sub/deep.txt")])),
        Availability::Enabled
    );
    assert_eq!(
        h.action("core.open_containing_tab")
            .availability(&h.ctx(&dir, &[&dir.join("sub/deep.txt")])),
        Availability::Enabled
    );
    assert!(matches!(
        h.run(
            "core.open_containing",
            h.ctx(&dir, &[&dir.join("sub/deep.txt")])
        ),
        ActionOutcome::Done
    ));
}

/// In the Trash view the file actions are hidden and Delete permanently
/// stays (the trash plugin's own actions join it there); in another virtual
/// location the writing actions are hidden and the rest work on the real
/// entries listed there.
#[test]
fn virtual_locations_hide_the_actions_that_do_not_apply() {
    let h = harness();
    let dir = h.dir.path().join("d");
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    write(&dir.join("one.txt"), "1");
    let entries = h.entries(&[&dir.join("one.txt")]);
    let trash_ctx = ActionCtx {
        pane: PaneId(1),
        location: vfs_rick::trash_location(),
        selection: entries.clone(),
        invoked_from: Invocation::ContextMenu,
        other: None,
    };
    for hidden in [
        "core.copy",
        "core.cut",
        "core.paste",
        "core.duplicate",
        "core.rename",
        "core.trash",
        "core.new_file",
        "core.new_folder",
        "core.copy_path",
        "core.reveal",
        "core.open_default",
        "core.pin",
        "core.unpin",
    ] {
        assert_eq!(
            h.action(hidden).availability(&trash_ctx),
            Availability::Hidden,
            "{hidden} in the trash"
        );
    }
    assert_eq!(
        h.action("core.delete").availability(&trash_ctx),
        Availability::Enabled,
        "Delete permanently goes through the view's own remove"
    );
    let favorites_ctx = ActionCtx {
        pane: PaneId(1),
        location: vfs_rick::Virtual::Favorites.location(),
        selection: entries,
        invoked_from: Invocation::ContextMenu,
        other: None,
    };
    for hidden in [
        "core.paste",
        "core.rename",
        "core.new_file",
        "core.new_folder",
    ] {
        assert_eq!(
            h.action(hidden).availability(&favorites_ctx),
            Availability::Hidden,
            "{hidden} in Favorites"
        );
    }
    for shown in [
        "core.copy",
        "core.open_default",
        "core.copy_path",
        "core.pin",
        "core.trash",
    ] {
        assert_eq!(
            h.action(shown).availability(&favorites_ctx),
            Availability::Enabled,
            "{shown} in Favorites"
        );
    }
    assert!(super::is_virtual(&favorites_ctx));
    assert!(!super::is_trash(&favorites_ctx));
}

/// An in-memory provider with `NATIVE_TRANSFER`: files by path, a
/// `transfer` that moves or copies the keys itself and records the call,
/// and an `open_read` that counts the bytes handed out, so a test can
/// prove a transfer streamed nothing. Copies answer `Unsupported` when
/// `copies` is off, as an SFTP server would.
mod native {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

    use ext::{
        Capabilities, ChangeStream, CopyOptions, EntryStream, LinkKind, ListOptions, Provider,
        ReadHandle, RemoveMode, TransferMode, WriteHandle, WriteOptions, async_trait,
    };
    use futures::StreamExt as _;
    use futures::stream;
    use runtime::ProgressSink;
    use types::{
        Attrs, CancelToken, Entry, EntryId, EntryKind, Error, Location, Progress, ProviderId, VPath,
    };

    pub(super) const ID: ProviderId = ProviderId("store");

    pub(super) struct Native {
        files: Mutex<BTreeMap<String, Vec<u8>>>,
        copies: bool,
        /// Bytes handed out through `open_read`.
        pub(super) read_bytes: Arc<AtomicU64>,
        /// Every `transfer` call: the mode, the items, the target folder.
        pub(super) transfers: Mutex<Vec<(TransferMode, Vec<String>, String)>>,
    }

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn key(loc: &Location) -> String {
        loc.path.to_string_lossy().trim_end_matches('/').to_owned()
    }

    impl Native {
        pub(super) fn new(copies: bool, files: &[(&str, &[u8])]) -> Arc<Self> {
            Arc::new(Self {
                files: Mutex::new(
                    files
                        .iter()
                        .map(|(p, b)| ((*p).to_owned(), b.to_vec()))
                        .collect(),
                ),
                copies,
                read_bytes: Arc::new(AtomicU64::new(0)),
                transfers: Mutex::new(Vec::new()),
            })
        }

        pub(super) fn paths(&self) -> Vec<String> {
            lock(&self.files).keys().cloned().collect()
        }

        fn entry(path: &str, size: Option<u64>) -> Entry {
            Entry {
                id: EntryId::new(ID, u128::from(path.len() as u64)),
                name: path.rsplit('/').next().unwrap_or(path).to_owned(),
                path: VPath::from(path),
                kind: if size.is_some() {
                    EntryKind::File
                } else {
                    EntryKind::Dir
                },
                size,
                created: None,
                modified: None,
                accessed: None,
                attrs: Attrs::empty(),
                mime: None,
            }
        }
    }

    /// A reader that adds what it hands out to the provider's count.
    struct Counted {
        inner: std::io::Cursor<Vec<u8>>,
        count: Arc<AtomicU64>,
    }

    impl tokio::io::AsyncRead for Counted {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let n = std::io::Read::read(&mut self.inner, buf.initialize_unfilled())?;
            buf.advance(n);
            self.count.fetch_add(n as u64, Ordering::SeqCst);
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[async_trait]
    impl Provider for Native {
        fn id(&self) -> ProviderId {
            ID
        }
        fn capabilities(&self, _loc: &Location) -> Capabilities {
            Capabilities::WRITE | Capabilities::NATIVE_TRANSFER | Capabilities::REMOTE
        }
        fn list(&self, loc: &Location, _opts: ListOptions, _cancel: CancelToken) -> EntryStream {
            let dir = key(loc);
            let entries: Vec<Entry> = lock(&self.files)
                .iter()
                .filter(|(p, _)| p.rsplit_once('/').is_some_and(|(parent, _)| parent == dir))
                .map(|(p, b)| Self::entry(p, Some(b.len() as u64)))
                .collect();
            stream::once(async move { Ok(entries) }).boxed()
        }
        async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
            let path = key(loc);
            let files = lock(&self.files);
            if let Some(bytes) = files.get(&path) {
                return Ok(Self::entry(&path, Some(bytes.len() as u64)));
            }
            let prefix = format!("{path}/");
            if path.is_empty() || files.keys().any(|k| k.starts_with(&prefix)) {
                return Ok(Self::entry(&path, None));
            }
            Err(Error::NotFound)
        }
        async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
            let bytes = lock(&self.files)
                .get(&key(loc))
                .cloned()
                .ok_or(Error::NotFound)?;
            Ok(Box::new(Counted {
                inner: std::io::Cursor::new(bytes),
                count: Arc::clone(&self.read_bytes),
            }))
        }
        async fn open_write(
            &self,
            loc: &Location,
            _opts: WriteOptions,
        ) -> Result<WriteHandle, Error> {
            lock(&self.files).insert(key(loc), Vec::new());
            Ok(Box::new(tokio::io::sink()))
        }
        async fn create_dir(&self, loc: &Location) -> Result<Entry, Error> {
            Ok(Self::entry(&key(loc), None))
        }
        async fn rename(&self, _from: &Location, _to: &Location) -> Result<Entry, Error> {
            Err(Error::Unsupported("no rename in the store"))
        }
        async fn remove(&self, loc: &Location, _mode: RemoveMode) -> Result<(), Error> {
            let path = key(loc);
            let prefix = format!("{path}/");
            lock(&self.files).retain(|k, _| *k != path && !k.starts_with(&prefix));
            Ok(())
        }
        async fn link(&self, _t: &Location, _at: &Location, _k: LinkKind) -> Result<Entry, Error> {
            Err(Error::Unsupported("no links in the store"))
        }
        async fn copy(
            &self,
            from: &Location,
            to: &Location,
            _opts: CopyOptions,
            progress: ProgressSink,
            _cancel: CancelToken,
        ) -> Result<(), Error> {
            let bytes = lock(&self.files)
                .get(&key(from))
                .cloned()
                .ok_or(Error::NotFound)?;
            let total = bytes.len() as u64;
            lock(&self.files).insert(key(to), bytes);
            progress.report(Progress::Bytes {
                done: total,
                total: Some(total),
            });
            Ok(())
        }
        async fn transfer(
            &self,
            items: &[Location],
            to: &Location,
            mode: TransferMode,
            _progress: ProgressSink,
            _cancel: CancelToken,
        ) -> Result<(), Error> {
            if mode == TransferMode::Copy && !self.copies {
                return Err(Error::Unsupported("the store cannot copy by itself"));
            }
            lock(&self.transfers).push((mode, items.iter().map(key).collect(), key(to)));
            let dir = key(to);
            let mut files = lock(&self.files);
            for item in items {
                let from = key(item);
                let name = from.rsplit('/').next().unwrap_or(&from).to_owned();
                let moved: Vec<(String, Vec<u8>)> = files
                    .iter()
                    .filter(|(k, _)| **k == from || k.starts_with(&format!("{from}/")))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                for (k, v) in moved {
                    let rest = &k[from.len()..];
                    files.insert(format!("{dir}/{name}{rest}"), v);
                    if mode == TransferMode::Move {
                        files.remove(&k);
                    }
                }
            }
            Ok(())
        }
        fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
            Err(Error::Unsupported("no watches in the store"))
        }
    }
}

/// Acceptance (issue 21): a move or copy within one provider instance
/// with `NATIVE_TRANSFER` goes through the provider's `transfer`, in one
/// call per target folder, and streams nothing through `open_read`; the
/// undo of the move goes back the same way.
#[test]
fn a_transfer_within_a_native_provider_streams_nothing() {
    let h = harness();
    let store = native::Native::new(
        true,
        &[
            ("/bucket/a.bin", &[1u8; 4096][..]),
            ("/bucket/tree/inner.bin", &[2u8; 2048][..]),
            ("/other/.keep", b""),
        ],
    );
    h.mounts.mount(Arc::clone(&store) as Arc<dyn ext::Provider>);
    let loc = |path: &str| Location::new(native::ID, path);

    let outcome = super::transfer(
        &h.jobs,
        Arc::clone(&h.mounts),
        Arc::clone(&h.conflicts),
        Arc::clone(&h.scheduler),
        &[loc("/bucket/a.bin"), loc("/bucket/tree")],
        &loc("/other"),
        TransferKind::Move,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    let ActionOutcome::Undoable(command) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(
        store.paths(),
        ["/other/.keep", "/other/a.bin", "/other/tree/inner.bin"]
    );
    {
        let transfers = store
            .transfers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        assert_eq!(
            *transfers,
            [(
                ext::TransferMode::Move,
                vec!["/bucket/a.bin".to_owned(), "/bucket/tree".to_owned()],
                "/other".to_owned()
            )],
            "one call for the whole selection"
        );
    }
    assert_eq!(
        store.read_bytes.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "nothing streamed through the machine"
    );

    // The undo moves back natively too: the items of one target folder
    // in one call.
    command.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert_eq!(
        store.paths(),
        ["/bucket/a.bin", "/bucket/tree/inner.bin", "/other/.keep"]
    );
    assert_eq!(
        store
            .transfers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        2
    );
    assert_eq!(
        store.read_bytes.load(std::sync::atomic::Ordering::SeqCst),
        0
    );

    // A copy goes the same way.
    let outcome = super::transfer(
        &h.jobs,
        Arc::clone(&h.mounts),
        Arc::clone(&h.conflicts),
        Arc::clone(&h.scheduler),
        &[loc("/bucket/a.bin")],
        &loc("/other"),
        TransferKind::Copy,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert!(matches!(outcome, ActionOutcome::Started(_)), "{outcome:?}");
    assert!(store.paths().contains(&"/other/a.bin".to_owned()));
    assert!(store.paths().contains(&"/bucket/a.bin".to_owned()));
    assert_eq!(
        store.read_bytes.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

/// A provider that answers `Unsupported` for a mode (an SFTP server
/// renames but cannot copy) gets the kernel's path for it; a transfer to
/// another provider is never native.
#[test]
fn a_mode_the_provider_cannot_transfer_falls_back_to_the_kernel() {
    let h = harness();
    let store = native::Native::new(false, &[("/bucket/a.bin", &[7u8; 1024][..])]);
    h.mounts.mount(Arc::clone(&store) as Arc<dyn ext::Provider>);
    let loc = |path: &str| Location::new(native::ID, path);
    super::transfer(
        &h.jobs,
        Arc::clone(&h.mounts),
        Arc::clone(&h.conflicts),
        Arc::clone(&h.scheduler),
        &[loc("/bucket/a.bin")],
        &loc("/other"),
        TransferKind::Copy,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert_eq!(store.paths(), ["/bucket/a.bin", "/other/a.bin"]);
    assert!(
        store
            .transfers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty(),
        "the refused call is not recorded as a transfer"
    );

    // Into the local disk: the streamed path, every byte read.
    let dst = h.dir.path().join("down");
    fs::create_dir_all(&dst).unwrap_or_else(|e| panic!("{e}"));
    super::transfer(
        &h.jobs,
        Arc::clone(&h.mounts),
        Arc::clone(&h.conflicts),
        Arc::clone(&h.scheduler),
        &[loc("/bucket/a.bin")],
        &tests_local(&dst),
        TransferKind::Copy,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert_eq!(
        fs::read(dst.join("a.bin")).map(|b| b.len()).ok(),
        Some(1024)
    );
    assert_eq!(
        store.read_bytes.load(std::sync::atomic::Ordering::SeqCst),
        1024
    );
}
#[test]
fn open_with_lists_the_platforms_applications_and_remembers_the_choice() {
    let h = harness();
    let dir = h.dir.path().join("docs");
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    write(&dir.join("a.txt"), "A");
    let action = h.action("core.open_with");
    assert!(matches!(
        action.availability(&h.ctx(&dir, &[])),
        Availability::Disabled(_)
    ));
    // The second fake application, remembered for .txt files.
    h.ui.answer_form(vec![
        ("app", ext::Value::Choice(1)),
        ("remember", ext::Value::Check(true)),
    ]);
    h.run("core.open_with", h.ctx(&dir, &[&dir.join("a.txt")]));
    let opened = h.fake.opened_with();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].0, dir.join("a.txt"));
    assert_eq!(opened[0].1.id, platform::FAKE_APPLICATIONS[1].1);
    let file = h.fake.config_dir().join(super::OPEN_WITH_FILE);
    let text = fs::read_to_string(&file).unwrap_or_default();
    assert!(text.contains("txt = \"fake-viewer\""), "{text}");

    // Another program, typed.
    h.ui.answer_form(vec![
        ("app", ext::Value::Choice(9)),
        ("command", ext::Value::Text("/opt/tool/view --fast".into())),
    ]);
    h.run("core.open_with", h.ctx(&dir, &[&dir.join("a.txt")]));
    let opened = h.fake.opened_with();
    assert_eq!(opened.len(), 2);
    assert_eq!(opened[1].1.id, "/opt/tool/view --fast");
    assert_eq!(opened[1].1.name, "view");
}

#[test]
fn paste_as_link_makes_symlinks_and_undo_removes_them() {
    let h = harness();
    let (src, dst) = (h.dir.path().join("src"), h.dir.path().join("dst"));
    fs::create_dir_all(&src).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(&dst).unwrap_or_else(|e| panic!("{e}"));
    write(&src.join("a.txt"), "A");
    fs::create_dir(src.join("sub")).unwrap_or_else(|e| panic!("{e}"));
    let paste_link = h.action("core.paste_link");
    assert!(matches!(
        paste_link.availability(&h.ctx(&dst, &[])),
        Availability::Disabled(_)
    ));
    h.run(
        "core.copy",
        h.ctx(&src, &[&src.join("a.txt"), &src.join("sub")]),
    );
    let outcome = h.run("core.paste_link", h.ctx(&dst, &[]));
    let ActionOutcome::Undoable(command) = outcome else {
        panic!("not undoable: {outcome:?}");
    };
    let link = dst.join("a.txt");
    let meta = fs::symlink_metadata(&link).unwrap_or_else(|e| panic!("{e}"));
    assert!(meta.file_type().is_symlink(), "a symlink was pasted");
    assert_eq!(fs::read_link(&link).ok(), Some(src.join("a.txt")));
    assert_eq!(fs::read_to_string(&link).ok().as_deref(), Some("A"));
    assert!(fs::symlink_metadata(dst.join("sub")).is_ok_and(|m| m.file_type().is_symlink()));
    assert!(src.join("a.txt").is_file(), "the source stays");

    // A second paste takes a numbered name.
    h.run("core.paste_link", h.ctx(&dst, &[]));
    assert!(fs::symlink_metadata(dst.join("a (2).txt")).is_ok());

    command.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert!(
        fs::symlink_metadata(&link).is_err(),
        "undo removed the link"
    );
    assert!(fs::symlink_metadata(dst.join("sub")).is_err());
    assert!(src.join("a.txt").is_file(), "undo leaves the source");
    command.redo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert!(fs::symlink_metadata(&link).is_ok());

    // Create link… into a chosen folder.
    let elsewhere = h.dir.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap_or_else(|e| panic!("{e}"));
    h.ui.answer_form(vec![(
        "folder",
        ext::Value::Text(elsewhere.display().to_string()),
    )]);
    let outcome = h.run("core.create_link", h.ctx(&src, &[&src.join("a.txt")]));
    assert!(matches!(outcome, ActionOutcome::Undoable(_)));
    assert_eq!(
        fs::read_link(elsewhere.join("a.txt")).ok(),
        Some(src.join("a.txt"))
    );
}

#[test]
fn select_by_pattern_matches_globs_and_regular_expressions() {
    let h = harness();
    let dir = h.dir.path().join("docs");
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    for name in ["report.pdf", "notes.TXT", "draft.txt", "Image.png"] {
        write(&dir.join(name), "x");
    }
    let all: Vec<_> = ["report.pdf", "notes.TXT", "draft.txt", "Image.png"]
        .iter()
        .map(|n| dir.join(n))
        .collect();
    let refs: Vec<&Path> = all.iter().map(PathBuf::as_path).collect();
    let entries = h.entries(&refs);
    let names = |selector: &ext::Selector| -> Vec<String> {
        entries
            .iter()
            .filter(|e| (selector.matches)(e))
            .map(|e| e.name.clone())
            .collect()
    };

    h.ui.answer_form(vec![("pattern", ext::Value::Text("*.txt".into()))]);
    let outcome = h.run("core.select_pattern", h.ctx(&dir, &[]));
    let ActionOutcome::Select(selector) = outcome else {
        panic!("not a selection: {outcome:?}");
    };
    assert!(selector.replace);
    assert_eq!(names(&selector), ["notes.TXT", "draft.txt"]);

    h.ui.answer_form(vec![
        ("pattern", ext::Value::Text("^[a-z]+\\.(pdf|png)$".into())),
        ("kind", ext::Value::Choice(1)),
        ("mode", ext::Value::Choice(1)),
    ]);
    let outcome = h.run("core.select_pattern", h.ctx(&dir, &[]));
    let ActionOutcome::Select(selector) = outcome else {
        panic!("not a selection: {outcome:?}");
    };
    assert!(!selector.replace);
    assert_eq!(names(&selector), ["report.pdf"]);

    // A pattern that does not parse is reported, not applied.
    h.ui.answer_form(vec![
        ("pattern", ext::Value::Text("(".into())),
        ("kind", ext::Value::Choice(1)),
    ]);
    let action = h.action("core.select_pattern");
    assert!(action.run(h.ctx(&dir, &[]), &h.jobs, &h.ui).is_err());
    // An empty pattern does nothing.
    h.ui.answer_form(vec![("pattern", ext::Value::Text("  ".into()))]);
    assert!(matches!(
        h.run("core.select_pattern", h.ctx(&dir, &[])),
        ActionOutcome::Done
    ));
}
