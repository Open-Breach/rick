use std::fs;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ext::platform::{Fake, Trash as _};
use ext::{
    ActionCtx, ActionOutcome, Availability, CellValue, DecorateCtx, Depth, Invocation, Level,
    ListOptions, PaneId, Provider, Registry, RemoveMode, UiHost,
};
use futures::StreamExt;
use runtime::JobRunner;
use tokio::io::AsyncReadExt as _;
use types::{CancelToken, Entry, EntryKind, Error, Location, ProviderId};

use super::*;

#[derive(Default)]
struct Scripted {
    confirm: Mutex<bool>,
}

impl UiHost for Scripted {
    fn notify(&self, _: Level, _: &str) {}
    fn confirm(&self, _: &str, _: &str) -> bool {
        *self.confirm.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn prompt(&self, _: &str, default: &str) -> Option<String> {
        Some(default.to_owned())
    }
    fn show(&self, _: &str, _: &str) {}
}

struct Harness {
    dir: tempfile::TempDir,
    fake: Arc<Fake>,
    registry: Registry,
    jobs: JobRunner,
    ui: Scripted,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut registry = Registry::with_platform(fake.clone());
    registry
        .load(&TrashExtension::new())
        .unwrap_or_else(|e| panic!("{e}"));
    Harness {
        dir,
        fake,
        registry,
        jobs: JobRunner::new(),
        ui: Scripted::default(),
    }
}

impl Harness {
    fn view(&self) -> Arc<dyn Provider> {
        Arc::clone(&self.registry.views()[0].1)
    }

    fn list(&self, loc: &Location) -> Vec<Entry> {
        let view = self.view();
        runtime::block_on(async {
            let mut stream = view.list(
                loc,
                ListOptions {
                    hidden: false,
                    system: false,
                    depth: Depth::One,
                },
                CancelToken::new(),
            );
            let mut out = Vec::new();
            while let Some(batch) = stream.next().await {
                out.extend(batch.unwrap_or_else(|e| panic!("{e}")));
            }
            out
        })
    }

    #[allow(clippy::unused_self)]
    fn ctx(&self, location: Location, selection: Vec<Entry>) -> ActionCtx {
        ActionCtx {
            pane: PaneId(1),
            location,
            selection,
            invoked_from: Invocation::ContextMenu,
        }
    }

    fn action(&self, id: &str) -> Arc<dyn ext::Action> {
        self.registry
            .actions()
            .iter()
            .find(|a| a.id().0 == id)
            .cloned()
            .unwrap_or_else(|| panic!("no action {id}"))
    }

    fn run(&self, id: &str, ctx: ActionCtx) -> ActionOutcome {
        let outcome = self
            .action(id)
            .run(ctx, &self.jobs, &self.ui)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        self.settle();
        outcome
    }

    /// Waits for every job to finish.
    fn settle(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.jobs.running().is_empty() {
            assert!(Instant::now() < deadline, "jobs did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn trash(&self, name: &str) -> std::path::PathBuf {
        let path = self.dir.path().join(name);
        self.fake.trash(&path).unwrap_or_else(|e| panic!("{e}"));
        path
    }
}

#[test]
fn the_extension_needs_the_platform_and_registers_the_view() {
    let h = harness();
    assert_eq!(h.registry.extensions(), &[ID]);
    assert_eq!(h.registry.views().len(), 1);
    assert_eq!(h.registry.views()[0].0, VIEW);
    assert_eq!(h.registry.decorators().len(), 1);
    assert_eq!(h.registry.actions().len(), 3);
    assert_eq!(
        h.registry.points(),
        Points::PROVIDER | Points::DECORATOR | Points::ACTION
    );
    assert_eq!(location(), Location::new(ProviderId::RICK, "/trash"));
    assert_eq!(h.view().id(), ProviderId::RICK);
    assert_eq!(h.view().capabilities(&location()), Capabilities::empty());

    let mut bare = Registry::new();
    assert!(matches!(
        bare.load(&TrashExtension::new()),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn the_view_lists_trashed_items_browses_a_folder_reads_and_deletes() {
    let h = harness();
    fs::write(h.dir.path().join("a.txt"), "alpha").unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(h.dir.path().join("folder/sub")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(h.dir.path().join("folder/inner.txt"), "inner").unwrap_or_else(|e| panic!("{e}"));
    fs::write(h.dir.path().join(".hidden"), "h").unwrap_or_else(|e| panic!("{e}"));
    h.trash("a.txt");
    h.trash("folder");
    h.trash(".hidden");

    // The root: one entry per item, named as before, newest first.
    let root = h.list(&location());
    let names: Vec<&str> = root.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["folder", "a.txt"], "hidden items need the flag");
    let a = root
        .iter()
        .find(|e| e.name == "a.txt")
        .unwrap_or_else(|| panic!("a"));
    assert_eq!(a.kind, EntryKind::File);
    assert_eq!(a.size, Some(5));
    assert!(a.modified.is_some());
    assert!(a.path.is_inside(&location().path));
    let folder = root
        .iter()
        .find(|e| e.name == "folder")
        .unwrap_or_else(|| panic!("folder"));
    assert_eq!(folder.kind, EntryKind::Dir);
    let view = h.view();
    let stat = runtime::block_on(view.stat(&location())).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(stat.name, "Trash");
    assert_eq!(stat.size, Some(3), "the root counts every item");
    assert_eq!(
        runtime::block_on(view.stat(&Location::new(ProviderId::RICK, a.path.clone())))
            .map(|e| e.id)
            .ok(),
        Some(a.id)
    );
    // Ids are stable across listings.
    assert_eq!(h.list(&location())[1].id, a.id);

    // Below a trashed folder: the folder's own contents.
    let below = h.list(&Location::new(ProviderId::RICK, folder.path.clone()));
    let mut names: Vec<&str> = below.iter().map(|e| e.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["inner.txt", "sub"]);
    let inner = below
        .iter()
        .find(|e| e.name == "inner.txt")
        .unwrap_or_else(|| panic!("inner"));
    assert!(inner.path.is_inside(&folder.path));

    // Reading, for the preview.
    let text = runtime::block_on(async {
        let mut reader = view
            .open_read(&Location::new(ProviderId::RICK, inner.path.clone()))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let mut text = String::new();
        reader
            .read_to_string(&mut text)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        text
    });
    assert_eq!(text, "inner");

    // The decorator names the original folder.
    let decorator = &h.registry.decorators()[0];
    assert!(decorator.applies_to(&location()));
    assert!(!decorator.applies_to(&Location::new(ProviderId::RICK, folder.path.clone())));
    let ctx = DecorateCtx::new(location(), CancelToken::new(), JobRunner::new(), |_| {});
    let decorated = decorator.decorate(&root, &ctx);
    assert_eq!(decorated.len(), 2);
    assert_eq!(
        decorated[1].1.cells[0],
        (
            ORIGINAL_COLUMN,
            CellValue::Text(h.dir.path().display().to_string())
        )
    );
    assert!(
        decorator
            .panel(a)
            .is_some_and(|p| p.rows[0].1.contains(&h.dir.path().display().to_string()))
    );

    // Nothing can be written; a trashed item can be deleted for good.
    let a_loc = Location::new(ProviderId::RICK, a.path.clone());
    assert!(matches!(
        runtime::block_on(view.remove(&a_loc, RemoveMode::Trash)),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        runtime::block_on(view.create_dir(&Location::new(ProviderId::RICK, "/trash/new"))),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        view.watch(&location()),
        Err(Error::Unsupported(_))
    ));
    runtime::block_on(view.remove(&a_loc, RemoveMode::Permanent)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(h.fake.list().map(|l| l.len()).ok(), Some(2));
    assert_eq!(h.list(&location()).len(), 1);
    assert!(matches!(
        runtime::block_on(view.stat(&a_loc)),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        runtime::block_on(view.stat(&Location::new(ProviderId::LOCAL, "/trash"))),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn restore_undo_and_empty_work_in_the_view_only() {
    let h = harness();
    for name in ["one.txt", "two.txt", "three.txt"] {
        fs::write(h.dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
        h.trash(name);
    }
    let entries = h.list(&location());
    assert_eq!(entries.len(), 3);
    let entry = |name: &str| {
        entries
            .iter()
            .find(|e| e.name == name)
            .cloned()
            .unwrap_or_else(|| panic!("{name}"))
    };
    let elsewhere = h.ctx(
        Location::new(ProviderId::LOCAL, "/"),
        vec![entry("one.txt")],
    );
    for id in ["trash.restore", "trash.restore_items", "trash.empty"] {
        assert_eq!(
            h.action(id).availability(&elsewhere),
            Availability::Hidden,
            "{id}"
        );
        assert_eq!(h.action(id).group(), ext::ActionGroup::Extension("Trash"));
    }
    assert_eq!(
        h.action("trash.restore")
            .availability(&h.ctx(location(), vec![entry("one.txt"), entry("two.txt")])),
        Availability::Disabled("Restore trash items puts several back")
    );
    assert_eq!(
        h.action("trash.restore_items")
            .availability(&h.ctx(location(), Vec::new())),
        Availability::Disabled("select trashed items first")
    );

    // Restore one; undo trashes it again; redo restores it.
    let outcome = h.run("trash.restore", h.ctx(location(), vec![entry("one.txt")]));
    let ActionOutcome::Undoable(restore) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(restore.label(), "Restore one.txt");
    assert_eq!(
        fs::read_to_string(h.dir.path().join("one.txt")).ok(),
        Some("one.txt".into())
    );
    assert_eq!(h.fake.list().map(|l| l.len()).ok(), Some(2));
    restore.undo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert!(!h.dir.path().join("one.txt").exists());
    assert_eq!(h.fake.list().map(|l| l.len()).ok(), Some(3));
    restore.redo(&h.jobs).unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert!(h.dir.path().join("one.txt").exists());

    // Several at once.
    let entries = h.list(&location());
    assert_eq!(entries.len(), 2);
    let outcome = h.run("trash.restore_items", h.ctx(location(), entries.clone()));
    assert!(matches!(outcome, ActionOutcome::Undoable(_)));
    assert!(h.dir.path().join("two.txt").exists());
    assert!(h.dir.path().join("three.txt").exists());
    assert_eq!(h.fake.list().map(|l| l.len()).ok(), Some(0));

    // Empty: refused without confirmation, then everything goes.
    h.trash("two.txt");
    let _ = h.list(&location());
    assert_eq!(
        h.action("trash.empty")
            .availability(&h.ctx(location(), Vec::new())),
        Availability::Enabled
    );
    assert!(matches!(
        h.run("trash.empty", h.ctx(location(), Vec::new())),
        ActionOutcome::Done
    ));
    assert_eq!(h.fake.list().map(|l| l.len()).ok(), Some(1), "refused");
    *h.ui.confirm.lock().unwrap_or_else(PoisonError::into_inner) = true;
    assert!(matches!(
        h.run("trash.empty", h.ctx(location(), Vec::new())),
        ActionOutcome::Started(_)
    ));
    assert_eq!(h.fake.list().map(|l| l.len()).ok(), Some(0));
    assert!(h.list(&location()).is_empty());
    assert_eq!(
        h.action("trash.empty")
            .availability(&h.ctx(location(), Vec::new())),
        Availability::Disabled("the trash is empty")
    );
}
