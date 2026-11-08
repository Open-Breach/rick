use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ext::platform::Fake;
use ext::{
    Action, ActionCtx, ActionId, ActionOutcome, Availability, Invocation, Level, PaneId, Provider,
    Registry, UiHost,
};
use ext_vfs_archive::{Format, create};
use runtime::JobRunner;
use types::{ConflictAnswer, Entry, Location, Resolution};

use super::*;

#[derive(Default)]
struct Scripted {
    form: Mutex<Option<Vec<(&'static str, ext::Value)>>>,
}

impl UiHost for Scripted {
    fn notify(&self, _: Level, _: &str) {}
    fn confirm(&self, _: &str, _: &str) -> bool {
        true
    }
    fn prompt(&self, _: &str, default: &str) -> Option<String> {
        Some(default.to_owned())
    }
    fn show(&self, _: &str, _: &str) {}
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

struct Harness {
    dir: tempfile::TempDir,
    registry: Registry,
    jobs: JobRunner,
    ui: Scripted,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut registry = Registry::new();
    registry
        .load(&CompressExtension::new())
        .unwrap_or_else(|e| panic!("{e}"));
    Harness {
        dir,
        registry,
        jobs: JobRunner::new(),
        ui: Scripted {
            form: Mutex::new(Some(Vec::new())),
        },
    }
}

fn loc(path: &Path) -> Location {
    Location::new(
        vfs_local::PROVIDER_ID,
        ext::platform::paths::from_path(path),
    )
}

fn entries(paths: &[PathBuf]) -> Vec<Entry> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider = vfs_local::LocalProvider::new(fake);
    runtime::block_on(async {
        let mut out = Vec::new();
        for path in paths {
            out.push(
                provider
                    .stat(&loc(path))
                    .await
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display())),
            );
        }
        out
    })
}

impl Harness {
    fn action(&self, id: ActionId) -> Arc<dyn Action> {
        self.registry
            .actions()
            .iter()
            .find(|a| a.id() == id)
            .cloned()
            .unwrap_or_else(|| panic!("no action {}", id.0))
    }

    #[allow(clippy::unused_self)]
    fn ctx(&self, location: &Path, selection: &[PathBuf]) -> ActionCtx {
        ActionCtx {
            pane: PaneId(1),
            location: loc(location),
            selection: entries(selection),
            invoked_from: Invocation::Palette,
            other: None,
        }
    }

    fn run(&self, id: ActionId, ctx: ActionCtx) -> ActionOutcome {
        let action = self.action(id);
        assert_eq!(action.availability(&ctx), Availability::Enabled, "{}", id.0);
        let outcome = action
            .run(ctx, &self.jobs, &self.ui)
            .unwrap_or_else(|e| panic!("{}: {e}", id.0));
        self.settle();
        outcome
    }

    fn settle(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.jobs.running().is_empty() {
            assert!(Instant::now() < deadline, "jobs still running");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// A tree to compress: `docs/` with two files and a folder, and a file
/// beside it.
fn tree(root: &Path) -> (PathBuf, PathBuf) {
    let docs = root.join("docs");
    fs::create_dir_all(docs.join("inner")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(docs.join("a.txt"), "alpha").unwrap_or_else(|e| panic!("{e}"));
    fs::write(docs.join("inner/b.txt"), "beta").unwrap_or_else(|e| panic!("{e}"));
    let note = root.join("note.md");
    fs::write(&note, "# note").unwrap_or_else(|e| panic!("{e}"));
    (docs, note)
}

fn contents(dir: &Path) -> Vec<(String, String)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{e}")) {
            let path = entry.unwrap_or_else(|e| panic!("{e}")).path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push((rel, fs::read_to_string(&path).unwrap_or_default()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn compress_makes_a_zip_next_to_the_selection_and_extract_here_unpacks_it() {
    let h = harness();
    let root = h.dir.path().to_path_buf();
    let (docs, note) = tree(&root);
    let compress = h.action(COMPRESS);
    assert!(matches!(
        compress.availability(&h.ctx(&root, &[])),
        Availability::Disabled(_)
    ));
    let outcome = h.run(COMPRESS, h.ctx(&root, &[docs.clone(), note.clone()]));
    assert!(matches!(outcome, ActionOutcome::Started(_)), "{outcome:?}");
    let archive = root.join(format!(
        "{}.zip",
        root.file_name().unwrap_or_default().to_string_lossy()
    ));
    assert!(archive.is_file(), "{}", archive.display());
    // A second run numbers the name.
    h.run(COMPRESS, h.ctx(&root, std::slice::from_ref(&note)));
    assert!(root.join("note.zip").is_file());
    h.run(COMPRESS, h.ctx(&root, std::slice::from_ref(&note)));
    assert!(root.join("note (2).zip").is_file());

    // Extract here into an empty folder gives the tree back.
    let into = root.join("into");
    fs::create_dir(&into).unwrap_or_else(|e| panic!("{e}"));
    let moved = into.join("x.zip");
    fs::rename(&archive, &moved).unwrap_or_else(|e| panic!("{e}"));
    let extract = h.action(EXTRACT_HERE);
    assert_eq!(
        extract.availability(&h.ctx(&root, std::slice::from_ref(&note))),
        Availability::Hidden,
        "a .md is no archive"
    );
    h.run(EXTRACT_HERE, h.ctx(&into, std::slice::from_ref(&moved)));
    assert_eq!(
        fs::read_to_string(into.join("docs/inner/b.txt"))
            .ok()
            .as_deref(),
        Some("beta")
    );
    assert_eq!(
        fs::read_to_string(into.join("note.md")).ok().as_deref(),
        Some("# note")
    );

    // Extract to folder: a folder named after the archive.
    h.run(EXTRACT_FOLDER, h.ctx(&into, std::slice::from_ref(&moved)));
    assert_eq!(contents(&into.join("x")).len(), 3);
    assert_eq!(
        fs::read_to_string(into.join("x/docs/a.txt"))
            .ok()
            .as_deref(),
        Some("alpha")
    );
}

#[test]
fn compress_to_round_trips_every_format_and_extract_to_takes_a_folder() {
    let h = harness();
    let root = h.dir.path().to_path_buf();
    let (docs, note) = tree(&root);
    let selection = [docs.clone(), note.clone()];
    let expected = {
        let mut all = contents(&docs)
            .into_iter()
            .map(|(rel, text)| (format!("docs/{rel}"), text))
            .collect::<Vec<_>>();
        all.push(("note.md".to_owned(), "# note".to_owned()));
        all.sort();
        all
    };
    for (index, (ext, format)) in create::CREATABLE.iter().enumerate() {
        if matches!(format, Format::Single(_)) {
            continue;
        }
        let out_dir = root.join(format!("out-{ext}"));
        h.ui.form
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(vec![
                ("name", ext::Value::Text("bundle".into())),
                ("format", ext::Value::Choice(index)),
                ("into", ext::Value::Text(out_dir.display().to_string())),
            ]);
        h.run(COMPRESS_AS, h.ctx(&root, &selection));
        let archive = out_dir.join(format!("bundle.{ext}"));
        assert!(archive.is_file(), "{ext}: {}", archive.display());

        let into = root.join(format!("into-{ext}"));
        h.ui.form
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(vec![("into", ext::Value::Text(into.display().to_string()))]);
        h.run(EXTRACT_TO, h.ctx(&out_dir, &[archive]));
        assert_eq!(contents(&into), expected, "{ext}");
    }
    // One file compresses to a single codec file and back.
    let formats: Vec<&str> = create::CREATABLE.iter().map(|(e, _)| *e).collect();
    let gz = formats
        .iter()
        .position(|e| *e == "gz")
        .unwrap_or_else(|| panic!("gz offered"));
    let out_dir = root.join("single");
    h.ui.form
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(vec![
            ("name", ext::Value::Text("note.md".into())),
            ("format", ext::Value::Choice(gz)),
            ("into", ext::Value::Text(out_dir.display().to_string())),
        ]);
    h.run(COMPRESS_AS, h.ctx(&root, std::slice::from_ref(&note)));
    let gz_file = out_dir.join("note.md.gz");
    assert!(gz_file.is_file());
    let into = root.join("single-into");
    h.ui.form
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(vec![("into", ext::Value::Text(into.display().to_string()))]);
    h.run(EXTRACT_TO, h.ctx(&out_dir, &[gz_file]));
    assert_eq!(
        fs::read_to_string(into.join("note.md")).ok().as_deref(),
        Some("# note")
    );
}

#[test]
fn an_existing_name_asks_through_the_conflict_queue_and_a_folder_merges() {
    let h = harness();
    let root = h.dir.path().to_path_buf();
    let (docs, _) = tree(&root);
    h.run(COMPRESS, h.ctx(&root, std::slice::from_ref(&docs)));
    let archive = root.join("docs.zip");
    assert!(archive.is_file());
    // The folder already exists with one differing file and one more.
    fs::write(docs.join("a.txt"), "changed").unwrap_or_else(|e| panic!("{e}"));
    fs::write(docs.join("extra.txt"), "extra").unwrap_or_else(|e| panic!("{e}"));

    let conflicts = Arc::clone(h.jobs.conflicts());
    let asked = Arc::new(Mutex::new(Vec::new()));
    {
        let asked = Arc::clone(&asked);
        let queue = Arc::clone(&conflicts);
        conflicts.set_waker(Arc::new(move || {
            for conflict in queue.take_new() {
                asked
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(conflict.clone());
                // Skip a.txt, overwrite everything else.
                let name =
                    String::from_utf8_lossy(conflict.target.path.file_name().unwrap_or_default())
                        .into_owned();
                let answer = if name == "a.txt" {
                    ConflictAnswer::once(Resolution::Skip)
                } else {
                    ConflictAnswer::once(Resolution::Overwrite)
                };
                queue.answer(conflict.id, answer);
            }
        }));
    }
    h.run(EXTRACT_HERE, h.ctx(&root, &[archive]));
    let asked = asked.lock().unwrap_or_else(PoisonError::into_inner);
    let names: Vec<String> = asked
        .iter()
        .map(|c| {
            String::from_utf8_lossy(c.target.path.file_name().unwrap_or_default()).into_owned()
        })
        .collect();
    // The folder itself merged without asking; both files asked.
    assert_eq!(names, ["a.txt", "b.txt"], "{names:?}");
    assert!(
        asked
            .iter()
            .all(|c| c.source.provider == types::ProviderId::ARCHIVE)
    );
    assert_eq!(
        fs::read_to_string(docs.join("a.txt")).ok().as_deref(),
        Some("changed"),
        "skipped"
    );
    assert_eq!(
        fs::read_to_string(docs.join("inner/b.txt")).ok().as_deref(),
        Some("beta"),
        "overwritten"
    );
    assert!(docs.join("extra.txt").is_file(), "merged, not replaced");
}

#[test]
fn the_extension_registers_five_actions() {
    let h = harness();
    assert_eq!(h.registry.actions().len(), 5);
    assert_eq!(h.registry.extensions(), &[ID]);
    for action in h.registry.actions() {
        assert!(action.id().0.starts_with("compress."));
        assert_eq!(action.group(), ext::ActionGroup::Core);
    }
}
