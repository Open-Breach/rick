use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ext::{
    Action, ActionCtx, ActionId, ActionOutcome, Availability, Extension, Invocation, Level, PaneId,
    Registry, UiHost,
};
use platform::{Fake, Trash as _};
use runtime::JobRunner;
use types::{Entry, Location};
use vfs::Mounts;
use vfs_local::LocalProvider;

use super::{ClipMode, Core};

/// A host whose answers the test scripts.
#[derive(Default)]
struct Scripted {
    confirm: Mutex<bool>,
    prompt: Mutex<Option<String>>,
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
    fn show(&self, _title: &str, _text: &str) {}
}

struct Harness {
    dir: tempfile::TempDir,
    fake: Arc<Fake>,
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
    let core = Arc::new(Core::new(Arc::clone(&mounts), fake.clone()));
    let mut registry = Registry::new();
    registry.load(&*core).unwrap_or_else(|e| panic!("{e}"));
    let clip = Arc::clone(&core);
    Harness {
        dir,
        fake,
        mounts,
        registry,
        core_clipboard: Arc::new(move || clip.clipboard()),
        jobs: JobRunner::new(),
        ui: Scripted::default(),
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

    h.run("core.copy_path", h.ctx(&dir, &refs));
    let text = h.fake.clipboard_text().unwrap_or_default();
    assert_eq!(text.lines().count(), 2);
    assert!(text.lines().all(|l| l.contains(".txt")), "{text}");

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
    assert_eq!(h.registry.actions().len(), 12);
    for action in h.registry.actions() {
        assert!(action.id().0.starts_with("core."), "{}", action.id().0);
        assert_ne!(action.label(), "");
        assert_eq!(action.group(), ext::ActionGroup::Core);
        assert!(action.default_shortcut().is_some(), "{}", action.id().0);
    }
    for always in ["core.new_file", "core.new_folder", "core.reveal"] {
        assert_eq!(
            h.action(always).availability(&h.ctx(h.dir.path(), &[])),
            Availability::Enabled,
            "{always}"
        );
    }
    let core = Core::new(Arc::clone(&h.mounts), h.fake.clone());
    assert_eq!(core.manifest().id, types::ExtId("core"));
    assert!(format!("{core:?}").contains("core"));
}
