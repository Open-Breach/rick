use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ext::platform::{ConfigDir, Fake, fs as pfs};
use ext::runtime::JobEvent;
use ext::{
    ActionCtx, ActionOutcome, Availability, CellValue, CliCtx, CliOutcome, DecorateCtx, Depth,
    Form, Invocation, Level, ListOptions, PaneId, Provider, Registry, UiHost, Value, Values,
};
use futures::StreamExt;
use runtime::JobRunner;
use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, Location, ProviderId};

use super::*;
use crate::actions::{
    APPS, EDIT_LAUNCHER, FORGET, INSTALL, INSTALL_COMMAND, INSTALL_RICK_COMMAND, LAUNCH,
    OPEN_FOLDER, REMOVE_STAGING, REPAIR, REVERT, RUN, UNINSTALL, UPDATE,
};
use crate::appimage::{self, Type};
use crate::archive::{Format, ItemKind};
use crate::detect::{compare_versions, slug, split_version, strip_version};
use crate::fixtures::{
    AppImageFixture, Fixture, Inside, other_machine, squashfs, this_machine, tiny_appimage,
    tiny_elf, tiny_png,
};
use crate::provider::{KIND_COLUMN, STATE_COLUMN, VERSION_COLUMN};
use crate::record::{Kind, Layout, PathMode, Record};

/// A host whose form, prompt and confirm answers the test sets.
#[derive(Default)]
struct Scripted {
    form: Mutex<Vec<(&'static str, Value)>>,
    prompt: Mutex<Option<String>>,
    confirm: Mutex<bool>,
    forms_seen: Mutex<Vec<Form>>,
    /// The summary rows of each form as the user saw them, before the
    /// install changed the disk.
    rows_seen: Mutex<Vec<Vec<ext::PreviewRow>>>,
    /// The texts shown, as (title, text).
    shown: Mutex<Vec<(String, String)>>,
}

impl Scripted {
    fn answer_form(&self, values: Vec<(&'static str, Value)>) {
        *self.form.lock().unwrap_or_else(PoisonError::into_inner) = values;
    }

    fn last_form(&self) -> Form {
        self.forms_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last()
            .cloned()
            .unwrap_or_else(|| panic!("no form was shown"))
    }

    fn last_rows(&self) -> Vec<ext::PreviewRow> {
        self.rows_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last()
            .cloned()
            .unwrap_or_else(|| panic!("no form was shown"))
    }
}

impl UiHost for Scripted {
    fn notify(&self, _: Level, _: &str) {}
    fn confirm(&self, _: &str, _: &str) -> bool {
        *self.confirm.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn prompt(&self, _: &str, _default: &str) -> Option<String> {
        self.prompt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    fn show(&self, title: &str, text: &str) {
        self.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((title.to_owned(), text.to_owned()));
    }
    fn form(&self, form: Form) -> Option<Values> {
        let mut values = form.defaults();
        for (key, value) in self
            .form
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            values.set(key, value);
        }
        self.rows_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(form.preview_rows(&values));
        self.forms_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(form);
        Some(values)
    }
}

struct Harness {
    dir: tempfile::TempDir,
    fake: Arc<Fake>,
    registry: Registry,
    jobs: JobRunner,
    events: Mutex<tokio::sync::mpsc::UnboundedReceiver<JobEvent>>,
    ui: Scripted,
    fixtures: HashMap<Fixture, PathBuf>,
    /// The messages the jobs reported, oldest first: the last one of an
    /// install is its summary.
    messages: Mutex<Vec<String>>,
}

fn harness() -> Harness {
    harness_on(&InstallerExtension::new())
}

/// A harness on a machine that has exactly the libraries of these names,
/// in place of the system's.
fn harness_with_libraries(names: &[&str]) -> Harness {
    harness_on(&InstallerExtension::with_libraries(install::Libraries::of(
        names,
    )))
}

fn harness_on(extension: &InstallerExtension) -> Harness {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut registry = Registry::with_platform(fake.clone());
    registry.load(extension).unwrap_or_else(|e| panic!("{e}"));
    let fixtures = Fixture::ALL
        .iter()
        .map(|f| (*f, f.write(dir.path()).unwrap_or_else(|e| panic!("{e}"))))
        .collect();
    let (jobs, events) = JobRunner::with_events();
    Harness {
        dir,
        fake,
        registry,
        jobs,
        events: Mutex::new(events),
        ui: Scripted {
            confirm: Mutex::new(true),
            ..Scripted::default()
        },
        fixtures,
        messages: Mutex::new(Vec::new()),
    }
}

impl Harness {
    fn home(&self) -> PathBuf {
        self.fake.home_dir()
    }

    fn layout(&self) -> Layout {
        Layout::user(&*self.fake)
    }

    fn archive(&self, fixture: Fixture) -> &Path {
        &self.fixtures[&fixture]
    }

    fn archive_entry(&self, fixture: Fixture) -> Entry {
        let path = self.archive(fixture);
        let meta = fs::metadata(path).unwrap_or_else(|e| panic!("{e}"));
        Entry {
            id: EntryId::new(ProviderId::LOCAL, u128::from(meta.len()) + 1),
            name: fixture.file_name().to_owned(),
            path: ext::platform::paths::from_path(path),
            kind: EntryKind::File,
            size: Some(meta.len()),
            created: None,
            modified: None,
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        }
    }

    fn ctx_archive(&self, fixture: Fixture) -> ActionCtx {
        ActionCtx {
            pane: PaneId(1),
            location: Location::new(
                ProviderId::LOCAL,
                ext::platform::paths::from_path(self.dir.path()),
            ),
            selection: vec![self.archive_entry(fixture)],
            invoked_from: Invocation::ContextMenu,
            other: None,
        }
    }

    /// Writes `bytes` to `name` beside the fixtures, with the permission
    /// bits `mode` where the platform has them.
    fn file(&self, name: &str, bytes: &[u8], mode: u32) -> PathBuf {
        let path = self.dir.path().join(name);
        fs::write(&path, bytes).unwrap_or_else(|e| panic!("{e}"));
        pfs::set_unix_mode(&path, mode).unwrap_or_else(|e| panic!("{e}"));
        path
    }

    /// Writes the AppImage fixture beside the others, without the execute
    /// bit.
    fn appimage(&self, fixture: AppImageFixture) -> PathBuf {
        fixture
            .write(self.dir.path())
            .unwrap_or_else(|e| panic!("{e}"))
    }

    /// Installs the file at `path` through *Install as application…* with
    /// the form's defaults and the overrides.
    fn install_file(&self, path: &Path, overrides: Vec<(&'static str, Value)>) {
        self.ui.answer_form(overrides);
        self.ok(INSTALL, self.ctx_file(path));
    }

    /// The local file at `path` as a listing shows it.
    #[allow(clippy::unused_self)]
    fn file_entry(&self, path: &Path) -> Entry {
        let meta = fs::metadata(path).unwrap_or_else(|e| panic!("{e}"));
        Entry {
            id: EntryId::new(ProviderId::LOCAL, u128::from(meta.len()) + 1),
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: ext::platform::paths::from_path(path),
            kind: if meta.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            },
            size: Some(meta.len()),
            created: None,
            modified: meta.modified().ok(),
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        }
    }

    /// The fixtures' folder with `selection` selected in it.
    fn ctx_selection(&self, selection: Vec<Entry>) -> ActionCtx {
        ActionCtx {
            pane: PaneId(1),
            location: Location::new(
                ProviderId::LOCAL,
                ext::platform::paths::from_path(self.dir.path()),
            ),
            selection,
            invoked_from: Invocation::ContextMenu,
            other: None,
        }
    }

    /// The fixtures' folder with the file at `path` selected.
    fn ctx_file(&self, path: &Path) -> ActionCtx {
        self.ctx_selection(vec![self.file_entry(path)])
    }

    #[allow(clippy::unused_self)]
    fn ctx_view(&self, selection: Vec<Entry>) -> ActionCtx {
        ActionCtx {
            pane: PaneId(1),
            location: location(),
            selection,
            invoked_from: Invocation::ContextMenu,
            other: None,
        }
    }

    fn action(&self, id: ext::ActionId) -> Arc<dyn ext::Action> {
        self.registry
            .actions()
            .iter()
            .find(|a| a.id() == id)
            .cloned()
            .unwrap_or_else(|| panic!("no action {}", id.0))
    }

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

    fn rows(&self) -> Vec<Entry> {
        self.list(&location())
    }

    fn decorations(&self, entries: &[Entry]) -> HashMap<EntryId, Vec<(ext::ColumnId, CellValue)>> {
        let decorator = &self.registry.decorators()[0];
        let ctx = DecorateCtx::new(location(), CancelToken::new(), self.jobs.clone(), |_| {});
        decorator
            .decorate(entries, &ctx)
            .into_iter()
            .map(|(id, d)| (id, d.cells.into_iter().collect()))
            .collect()
    }

    fn cell(&self, entry: &Entry, column: ext::ColumnId) -> String {
        match self.decorations(std::slice::from_ref(entry))[&entry.id]
            .iter()
            .find(|(c, _)| *c == column)
        {
            Some((_, CellValue::Text(text))) => text.clone(),
            other => panic!("no text in column {}: {other:?}", column.0),
        }
    }

    /// Runs the action, waits for its jobs and returns the outcome and the
    /// jobs' results. A form the action asks for later is shown through
    /// the host as the application would, and the rest of the action's
    /// outcome stands for the action's.
    fn run(
        &self,
        id: ext::ActionId,
        ctx: ActionCtx,
    ) -> (Result<ActionOutcome, Error>, Vec<Result<(), Error>>) {
        let pane = ctx.pane;
        let outcome = match self.action(id).run(ctx, &self.jobs, &self.ui) {
            Ok(ActionOutcome::Later(later)) => match runtime::block_on(later.wait()) {
                Some((form, then)) => self
                    .ui
                    .form_later(pane, form, then)
                    .unwrap_or_else(|| panic!("the scripted host answers at once")),
                // The job failed: its result says why.
                None => Ok(ActionOutcome::Done),
            },
            outcome => outcome,
        };
        self.settle();
        let mut results = Vec::new();
        let mut events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        while let Ok(event) = events.try_recv() {
            match event {
                JobEvent::Finished { result, .. } => results.push(result),
                JobEvent::Progress {
                    progress: types::Progress::Message(message),
                    ..
                } => self
                    .messages
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(message),
                _ => {}
            }
        }
        (outcome, results)
    }

    /// Runs an action that must succeed, with its jobs.
    fn ok(&self, id: ext::ActionId, ctx: ActionCtx) -> ActionOutcome {
        let (outcome, results) = self.run(id, ctx);
        let outcome = outcome.unwrap_or_else(|e| panic!("{}: {e}", id.0));
        for result in results {
            result.unwrap_or_else(|e| panic!("{} job: {e}", id.0));
        }
        outcome
    }

    fn settle(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.jobs.running().is_empty() {
            assert!(Instant::now() < deadline, "jobs did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn install(&self, fixture: Fixture, overrides: Vec<(&'static str, Value)>) {
        self.ui.answer_form(overrides);
        self.ok(INSTALL, self.ctx_archive(fixture));
    }

    fn row(&self, name: &str) -> Entry {
        self.rows()
            .into_iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("no row {name} in {:?}", self.rows()))
    }

    fn record(&self, id: &str) -> Record {
        Record::read(&self.layout().record_path(id)).unwrap_or_else(|e| panic!("{e}"))
    }

    fn current(&self, id: &str) -> String {
        fs::read_link(self.layout().current(id))
            .unwrap_or_else(|e| panic!("{e}"))
            .to_string_lossy()
            .into_owned()
    }
}

/// Every regular file and link under `dir`, relative to it.
fn files_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).unwrap_or_else(|e| panic!("{e}"));
            if meta.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == "mimeinfo.cache") {
                // `update-desktop-database`'s own cache, which Rick asks it to
                // rebuild after a desktop entry comes or goes.
            } else {
                out.push(
                    path.strip_prefix(dir)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    out.sort();
    out
}

fn text_of(error: &Error) -> String {
    error.to_string()
}

#[test]
fn version_parser_slug_and_comparison() {
    let stem = detect::archive_stem("Postman-linux-x64-11.3.0.tar.gz");
    assert_eq!(stem, "Postman-linux-x64-11.3.0");
    let (version, range) = split_version(&stem).unwrap_or_else(|| panic!("no version"));
    assert_eq!(version, "11.3.0");
    assert_eq!(&stem[range], "11.3.0");
    assert_eq!(strip_version(&stem), "Postman-linux-x64");
    assert_eq!(
        split_version("app-v2.3").map(|(v, r)| (v, r.start)),
        Some(("2.3".to_owned(), 4))
    );
    assert_eq!(
        split_version("jdk-21.0.2_linux-x64_bin").map(|(v, _)| v),
        Some("21.0.2".to_owned())
    );
    assert_eq!(split_version("release-2024"), None);
    assert_eq!(
        split_version("x86_64-1.2.3").map(|(v, _)| v),
        Some("1.2.3".to_owned())
    );
    assert_eq!(slug("Fixture GUI"), "fixture-gui");
    assert_eq!(slug("  Postman!! "), "postman");
    assert_eq!(slug("JetBrains_Toolbox 2.1"), "jetbrains-toolbox-2-1");
    assert!(detect::valid_id("fixture-gui") && !detect::valid_id("-x") && !detect::valid_id("A"));
    assert!(detect::valid_version("1.5.0") && !detect::valid_version("a/b"));
    assert!(!detect::valid_version("current") && !detect::valid_version(""));
    assert_eq!(
        compare_versions("2.0.0", "1.10.3"),
        std::cmp::Ordering::Greater
    );
    assert_eq!(compare_versions("1.2", "1.2.1"), std::cmp::Ordering::Less);
    assert_eq!(compare_versions("v1.2", "1.2"), std::cmp::Ordering::Equal);
    assert_eq!(
        detect::archive_stem("tool.tgz") + &detect::archive_stem("a.zip"),
        "toola"
    );
    assert!(record::glob_matches(
        "Postman-linux-x64-*.tar.gz",
        "postman-linux-x64-11.4.0.tar.gz"
    ));
    assert!(!record::glob_matches(
        "Postman-*.tar.gz",
        "other-1.0.tar.gz"
    ));
}

#[test]
fn the_extension_registers_the_view_the_columns_and_the_actions() {
    let h = harness();
    assert_eq!(h.registry.extensions(), &[ID]);
    assert_eq!(h.registry.views().len(), 1);
    assert_eq!(h.registry.views()[0].0, VIEW);
    assert_eq!(h.registry.decorators().len(), 1);
    assert_eq!(h.registry.decorators()[0].columns().len(), 5);
    assert_eq!(h.registry.actions().len(), 12);
    assert_eq!(
        h.registry.points(),
        Points::PROVIDER | Points::DECORATOR | Points::ACTION
    );
    assert_eq!(location(), Location::new(ProviderId::RICK, "apps"));
    assert!(is_view(&Location::new(ProviderId::RICK, "/apps")));
    assert!(!is_view(&Location::new(ProviderId::RICK, "apps/x")));
    assert!(!is_view(&Location::new(ProviderId::RICK, "apps2")));
    assert_eq!(h.view().id(), ProviderId::RICK);
    assert_eq!(h.view().capabilities(&location()), Capabilities::empty());
    let mut bare = Registry::new();
    assert!(bare.load(&InstallerExtension::new()).is_err());

    // From here on Linux only: elsewhere the view answers "unsupported on
    // this platform" and the actions hide (the host test checks that).
    if !supported() {
        return;
    }
    // The view is empty at first and the archive action is enabled on an
    // archive only.
    let rows = h.rows();
    assert!(rows.is_empty(), "{rows:?}");
    let install = h.action(INSTALL);
    assert_eq!(
        install.availability(&h.ctx_archive(Fixture::GuiWithDesktop)),
        Availability::Enabled
    );
    assert_eq!(
        install.availability(&h.ctx_view(Vec::new())),
        Availability::Hidden
    );
    let apps = h.action(APPS);
    assert_eq!(
        apps.availability(&h.ctx_archive(Fixture::GuiWithDesktop)),
        Availability::Enabled
    );
    assert_eq!(
        apps.availability(&h.ctx_view(Vec::new())),
        Availability::Hidden
    );
    let outcome = h.ok(APPS, h.ctx_archive(Fixture::GuiWithDesktop));
    assert!(matches!(outcome, ActionOutcome::Navigate(to) if to == location()));
}

/// `rick install <archive>` (specs/system-integration.md §4): the
/// manifest declares the command, the registry holds it, and it opens the
/// install form on the archive named relative to the caller's directory;
/// wrong arguments and a file that is no archive are the caller's errors.
#[test]
fn the_install_command_opens_the_form_on_an_archive_from_the_command_line() {
    let h = harness();
    let manifest = InstallerExtension::new().manifest().clone();
    assert_eq!(manifest.commands.len(), 2);
    assert_eq!(manifest.commands[0].name, INSTALL_COMMAND);
    assert_eq!(manifest.commands[1].name, INSTALL_RICK_COMMAND);
    let command = h
        .registry
        .commands()
        .iter()
        .find(|c| c.name() == INSTALL_COMMAND)
        .cloned()
        .unwrap_or_else(|| panic!("the install command is registered"));
    let cwd = h.dir.path().to_path_buf();
    let ctx = |args: &[&str]| CliCtx {
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        cwd: cwd.clone(),
        pane: PaneId(1),
        location: Location::new(ProviderId::LOCAL, ext::platform::paths::from_path(&cwd)),
    };
    let name = Fixture::GuiWithDesktop.file_name();
    if !supported() {
        let refused = command.run(ctx(&[name]), &h.jobs, &h.ui);
        assert!(refused.is_err(), "Linux only: {refused:?}");
        return;
    }
    assert!(matches!(
        command.run(ctx(&[]), &h.jobs, &h.ui),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        command.run(ctx(&["nowhere.tar.gz"]), &h.jobs, &h.ui),
        Err(Error::NotFound)
    ));
    fs::write(h.dir.path().join("notes.txt"), b"x").unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        command.run(ctx(&["notes.txt"]), &h.jobs, &h.ui),
        Err(Error::Unsupported(_))
    ));
    // The archive by its name relative to the caller's directory: the
    // detection job reads it and the form arrives for the application to
    // show, as *Install as application…* would.
    let outcome = command
        .run(ctx(&[name]), &h.jobs, &h.ui)
        .unwrap_or_else(|e| panic!("{e}"));
    let CliOutcome::Outcome(ActionOutcome::Later(later)) = outcome else {
        panic!("the form comes later: {outcome:?}");
    };
    let (form, _then) =
        runtime::block_on(later.wait()).unwrap_or_else(|| panic!("the detection fills the form"));
    assert!(
        !form.title.is_empty() && !form.fields.is_empty(),
        "{form:?}"
    );
    h.settle();
}

#[test]
fn the_elf_reader_reads_the_tiny_programs() {
    let program = tiny_elf(this_machine(), &["libgtk-3.so.0", "libc.so.6"]);
    let elf = elf::read_bytes(&program).unwrap_or_else(|| panic!("not elf"));
    assert_eq!(elf.machine, this_machine());
    assert!(elf.class64 && elf.is_executable() && !elf.interp);
    assert_eq!(elf.needed, vec!["libgtk-3.so.0", "libc.so.6"]);
    let plain =
        elf::read_bytes(&tiny_elf(other_machine(), &[])).unwrap_or_else(|| panic!("not elf"));
    assert_eq!(plain.machine, other_machine());
    assert!(plain.needed.is_empty(), "{:?}", plain.needed);
    assert_eq!(elf::read_bytes(b"#!/bin/sh\n"), None);
    assert!(elf::read_bytes(&program[..40]).is_some_and(|e| e.needed.is_empty()));
}

#[test]
#[allow(clippy::too_many_lines)]
fn detection_answers_the_five_questions_for_each_fixture() {
    let h = harness();
    let detect = |f: Fixture| {
        detect::detect(h.archive(f)).unwrap_or_else(|e| panic!("{}: {e}", f.file_name()))
    };

    let gui = detect(Fixture::GuiWithDesktop);
    assert_eq!(gui.format, Format::Tar(archive::Codec::Gzip));
    assert!(!gui.source);
    assert_eq!(gui.kind, Kind::Gui);
    assert_eq!(gui.main_rel(), Some("fixgui"));
    assert_eq!(gui.name, "Fixture GUI");
    assert_eq!(gui.version, "1.5.0");
    assert_eq!(gui.app_id(), "fixture-gui");
    assert_eq!(gui.wm_class().as_deref(), Some("FixGui"));
    assert_eq!(gui.icons[0].rel, "fixgui.png");
    assert_eq!(gui.icons[0].size, Some((64, 64)));
    assert_eq!(gui.icons[0].size_dir(), "64x64");
    assert_eq!(gui.index.top_level.as_deref(), Some("fixgui-1.5.0"));
    assert_eq!(gui.source_pattern, "gui-with-desktop.tar.gz");
    assert!(
        gui.desktop
            .as_ref()
            .is_some_and(|d| d.rel == "fixgui.desktop")
    );
    assert_eq!(gui.arch_mismatch, None);

    let no_desktop = detect(Fixture::GuiNoDesktop);
    assert_eq!(no_desktop.format, Format::Tar(archive::Codec::Xz));
    assert_eq!(no_desktop.kind, Kind::Gui);
    assert!(no_desktop.kind_reason.contains("toolkit") || no_desktop.kind_reason.contains("icon"));
    assert_eq!(no_desktop.main_rel(), Some("fixnodesk"));
    assert_eq!(no_desktop.icons[0].rel, "resources/icon.svg");
    assert_eq!(no_desktop.icons[0].size_dir(), "scalable");
    assert_eq!(no_desktop.version, "0.3.0");
    assert_eq!(no_desktop.name, "fixnodesk");
    assert!(no_desktop.desktop.is_none());

    let tool = detect(Fixture::Toolchain);
    assert_eq!(tool.kind, Kind::Cli);
    assert_eq!(
        tool.bins,
        vec!["bin/fixa", "bin/fixb", "bin/fixc", "bin/fixscript"]
    );
    assert_eq!(
        tool.main, None,
        "three equal commands, none named like the toolchain: the user picks"
    );
    assert_eq!(tool.candidates.len(), 3, "the .so.1 file is no candidate");
    assert_eq!(tool.name, "fixtool");
    assert_eq!(tool.version, "1.2.0");
    assert!(tool.icons.is_empty(), "{:?}", tool.icons);

    let bomb = detect(Fixture::Tarbomb);
    assert_eq!(bomb.format, Format::Zip);
    assert_eq!(bomb.index.top_level, None);
    assert_eq!(bomb.main_rel(), Some("fixbomb"));
    assert_eq!(bomb.kind, Kind::Cli);
    assert_eq!(bomb.version, "");
    assert_eq!(bomb.name, "tarbomb");
    assert!(
        bomb.index
            .items
            .iter()
            .any(|i| i.path == "fixbomb-link" && i.kind == ItemKind::Symlink)
    );

    let source = detect(Fixture::Source);
    assert!(source.source);
    assert!(source.candidates.is_empty(), "{:?}", source.candidates);

    let arch = detect(Fixture::WrongArch);
    assert_eq!(arch.arch_mismatch, Some(other_machine()));

    let ambiguous = detect(Fixture::Ambiguous);
    assert_eq!(ambiguous.main, None);
    assert_eq!(ambiguous.candidates.len(), 2);

    let hostile = detect(Fixture::Hostile);
    assert_eq!(hostile.hostile, vec!["../evil", "abs/evil"]);
    assert_eq!(hostile.install_script.as_deref(), Some("install.sh"));
    assert_eq!(hostile.main_rel(), Some("fixhostile"));
    assert!(
        hostile
            .index
            .items
            .iter()
            .any(|i| i.path == "fixhostile-1.0.0/pipe" && i.kind == ItemKind::Other)
    );
    assert!(
        hostile
            .index
            .items
            .iter()
            .any(|i| i.path == "fixhostile-1.0.0/suid" && i.mode == 0o4755)
    );

    let v2 = detect(Fixture::UpdateV2);
    assert_eq!(
        (v2.name.as_str(), v2.version.as_str()),
        ("Fixture GUI", "2.0.0")
    );
    let v1 = detect(Fixture::UpdateV1);
    assert_eq!(v1.version, "1.0.0");

    // The JetBrains shape: no desktop entry, the launcher named like the
    // app among helpers in bin/, its icons beside it.
    let ide = detect(Fixture::JetBrains);
    assert_eq!(ide.name, "Vendor App");
    assert_eq!(ide.app_id(), "vendor-app");
    assert_eq!(ide.version, "1.2.3");
    assert_eq!(ide.source_pattern, "Vendor.App-*.tar.gz");
    assert_eq!(ide.kind, Kind::Gui);
    assert_eq!(ide.kind_reason, "the archive ships an icon");
    assert_eq!(ide.main_rel(), Some("bin/app"), "{:?}", ide.candidates);
    assert_eq!(
        ide.icons.iter().map(|i| i.rel.as_str()).collect::<Vec<_>>(),
        vec!["bin/app.svg", "bin/app.png"]
    );
    assert!(ide.icon_trusted);
    assert_eq!(ide.icons[1].size, Some((128, 128)));
    assert_eq!(
        ide.bins,
        vec![
            "bin/app",
            "bin/app.sh",
            "bin/format.sh",
            "bin/fsnotifier",
            "bin/inspect.sh",
            "bin/restarter"
        ]
    );
    assert_eq!(ide.arch_mismatch, None);
}

/// A gzip tar of `files` (path, mode, bytes) under the folder `top`, for
/// a shape the fixtures do not cover.
fn gz_tar(top: &str, files: &[(&str, u32, &[u8])]) -> Vec<u8> {
    use std::io::Write as _;
    let mut builder = tar::Builder::new(Vec::new());
    let mut dir = tar::Header::new_ustar();
    dir.set_entry_type(tar::EntryType::Directory);
    dir.set_mode(0o755);
    dir.set_size(0);
    dir.set_path(format!("{top}/"))
        .unwrap_or_else(|e| panic!("{e}"));
    dir.set_cksum();
    builder
        .append(&dir, std::io::empty())
        .unwrap_or_else(|e| panic!("{e}"));
    for (path, mode, bytes) in files {
        let mut header = tar::Header::new_ustar();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(*mode);
        header.set_size(bytes.len() as u64);
        header
            .set_path(format!("{top}/{path}"))
            .unwrap_or_else(|e| panic!("{e}"));
        header.set_cksum();
        builder
            .append(&header, *bytes)
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let plain = builder.into_inner().unwrap_or_else(|e| panic!("{e}"));
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&plain).unwrap_or_else(|e| panic!("{e}"));
    encoder.finish().unwrap_or_else(|e| panic!("{e}"))
}

/// The case that exposed the rule: a helper sorting before the launcher
/// must not become the main executable, and the launcher's icons beside
/// it make the archive a GUI application.
#[test]
fn a_helper_sorting_before_the_launcher_does_not_win() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let elf = tiny_elf(this_machine(), &[]);
    let png = crate::fixtures::tiny_png(32);
    let path = dir.path().join("Vendor.Rider-2026.2.4.1.tar.gz");
    fs::write(
        &path,
        gz_tar(
            "Vendor Rider-2026.2.4.1",
            &[
                ("bin/fsnotifier", 0o755, &elf),
                ("bin/remote-dev-server", 0o755, &elf),
                ("bin/restarter", 0o755, &elf),
                ("bin/rider", 0o755, &elf),
                ("bin/rider.sh", 0o755, b"#!/bin/sh\nexit 0\n"),
                ("bin/rider.png", 0o644, &png),
                ("bin/rider.svg", 0o644, crate::fixtures::ICON_SVG.as_bytes()),
                ("lib/app.jar", 0o644, b"PK\x05\x06"),
            ],
        ),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let detected = detect::detect(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(detected.name, "Vendor Rider");
    assert_eq!(detected.app_id(), "vendor-rider");
    assert_eq!(detected.version, "2026.2.4.1");
    assert_eq!(detected.main_rel(), Some("bin/rider"));
    assert_eq!(detected.kind, Kind::Gui);
    assert_eq!(detected.kind_reason, "the archive ships an icon");
    assert_eq!(detected.icons[0].rel, "bin/rider.svg");
    assert_eq!(detected.icons[1].rel, "bin/rider.png");
    assert!(detected.icon_trusted);
}

/// The field `key` of a form.
fn field<'a>(form: &'a Form, key: &str) -> &'a ext::FieldKind {
    &form
        .fields
        .iter()
        .find(|f| f.key == key)
        .unwrap_or_else(|| panic!("no field {key} in {form:?}"))
        .kind
}

/// The command defaults follow the chosen kind: a GUI application links
/// one command named after the app id at the main executable, a
/// toolchain its `bin/` contents, whichever kind detection proposed.
#[test]
#[allow(clippy::too_many_lines)]
fn the_form_defaults_follow_the_kind() {
    if !supported() {
        return;
    }
    let h = harness();
    let layout = h.layout();
    h.install(Fixture::JetBrains, Vec::new());
    let form = h.ui.last_form();
    assert!(matches!(
        field(&form, "kind"),
        ext::FieldKind::Choice { selected: 0, .. }
    ));
    assert!(matches!(
        field(&form, "exe"),
        ext::FieldKind::Choice { options, selected: 0 } if options[0] == "bin/app"
    ));
    let commands: Vec<&ext::Field> = form
        .fields
        .iter()
        .filter(|f| f.key.starts_with("cmd"))
        .collect();
    assert_eq!(commands.len(), 1, "{commands:?}");
    assert_eq!(
        commands[0].label,
        "Link command vendor-app → current/bin/app"
    );
    // What was written: one link named after the id, the desktop entry
    // running the chosen executable through current.
    let record = h.record("vendor-app");
    assert_eq!(record.kind, Kind::Gui);
    assert_eq!(record.main_executable, "bin/app");
    assert_eq!(record.commands(), vec!["vendor-app"]);
    let expected = layout.current("vendor-app").join("bin/app");
    assert_eq!(
        fs::read_link(layout.bin_dir().join("vendor-app")).ok(),
        Some(expected.clone())
    );
    assert!(
        fs::symlink_metadata(layout.bin_dir().join("fsnotifier")).is_err(),
        "a helper is not linked"
    );
    let desktop =
        fs::read_to_string(layout.desktop_path("vendor-app")).unwrap_or_else(|e| panic!("{e}"));
    let entry = desktop::DesktopEntry::parse(&desktop);
    assert_eq!(
        entry.get("Exec"),
        Some(format!("\"{}\"", expected.display()).as_str()),
        "{desktop}"
    );
    assert_eq!(entry.get("Icon"), Some("vendor-app"));
    assert!(layout.icon_path("vendor-app", "scalable", "svg").is_file());
    // Switched to a command-line tool in the dialogue, the summary lists
    // the bin/ contents and no desktop entry.
    let mut values = form.defaults();
    values.set("kind", Value::Choice(1));
    let rows = form.preview_rows(&values);
    let linked: Vec<&str> = rows
        .iter()
        .filter(|r| r.from == "Command")
        .map(|r| r.to.as_str())
        .collect();
    assert_eq!(linked.len(), 6, "{linked:?}");
    assert!(
        linked
            .iter()
            .any(|l| l.ends_with("fsnotifier → current/bin/fsnotifier")),
        "{linked:?}"
    );
    assert!(rows.iter().all(|r| r.from != "Desktop entry"), "{rows:?}");
    // And a toolchain installed as a GUI application links one command
    // named after its id at the executable the user picked.
    h.install(
        Fixture::Toolchain,
        vec![("kind", Value::Choice(0)), ("exe", Value::Choice(2))],
    );
    let record = h.record("fixtool");
    assert_eq!(record.kind, Kind::Gui);
    assert_eq!(record.main_executable, "bin/fixb");
    assert_eq!(record.commands(), vec!["fixtool"]);
    assert_eq!(
        fs::read_link(layout.bin_dir().join("fixtool")).ok(),
        Some(layout.current("fixtool").join("bin/fixb"))
    );
    assert!(layout.desktop_path("fixtool").is_file());
}

/// The Icon field is one list: `(none)`, then every PNG and SVG of the
/// archive with its path and pixel size, best first, the best one
/// preselected only when detection trusts it; there is no file picker.
#[test]
fn the_icon_field_is_one_list_of_the_archives_icons() {
    let h = harness();
    let scan = crate::record::Scan::default();
    let layout = h.layout();
    let detected =
        Arc::new(detect::detect(h.archive(Fixture::JetBrains)).unwrap_or_else(|e| panic!("{e}")));
    let form = actions::install_form(
        &detected,
        &scan,
        &layout,
        None,
        false,
        &install::Libraries::default(),
    );
    assert!(
        matches!(
            field(&form, "icon"),
            ext::FieldKind::Choice { options, selected: 1 }
                if *options == ["(none)", "bin/app.svg · scalable", "bin/app.png · 128×128"]
        ),
        "{:?}",
        field(&form, "icon")
    );
    assert!(
        form.fields.iter().all(|f| f.key != "icon_file"),
        "no file picker"
    );
    // Choosing the PNG instead puts it in the plan.
    let mut values = form.defaults();
    values.set("icon", Value::Choice(2));
    let rows = form.preview_rows(&values);
    let icon = rows
        .iter()
        .find(|r| r.from == "Icon")
        .unwrap_or_else(|| panic!("no Icon row in {rows:?}"));
    assert_eq!(icon.to, "bin/app.png → hicolor/128x128/apps/vendor-app.png");
    assert!(icon.problem.is_none());

    // A stray PNG is listed but not preselected, and does not make the
    // archive a GUI application.
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("fixplain-1.0.tar.gz");
    fs::write(
        &path,
        gz_tar(
            "fixplain-1.0",
            &[
                ("fixplain", 0o755, &tiny_elf(this_machine(), &[])),
                ("docs/shot.png", 0o644, &crate::fixtures::tiny_png(32)),
            ],
        ),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let detected = Arc::new(detect::detect(&path).unwrap_or_else(|e| panic!("{e}")));
    assert_eq!(detected.kind, Kind::Cli);
    assert!(!detected.icon_trusted);
    assert_eq!(detected.icons.len(), 1);
    let form = actions::install_form(
        &detected,
        &scan,
        &layout,
        None,
        false,
        &install::Libraries::default(),
    );
    assert!(
        matches!(
            field(&form, "icon"),
            ext::FieldKind::Choice { options, selected: 0 }
                if *options == ["(none)", "docs/shot.png · 32×32"]
        ),
        "{:?}",
        field(&form, "icon")
    );
}

/// Detection reads the archive in one streaming pass: the index and the
/// heads it collects are what the two passes (the index, then the heads
/// over it) collected, with byte progress against the file and a cancel
/// between entries.
#[test]
fn the_single_pass_reads_what_the_two_passes_did() {
    let h = harness();
    for fixture in Fixture::ALL {
        let path = h.archive(fixture);
        let name = fixture.file_name();
        let format = Format::detect(path).unwrap_or_else(|e| panic!("{name}: {e}"));
        // The two passes as they were.
        let index = archive::index(path, format).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut heads = detect::Heads::default();
        archive::visit(path, format, &mut |item, reader| {
            if item.kind == ItemKind::File && !item.hostile {
                heads.probe(item, reader)?;
            }
            Ok(archive::Visit::Next)
        })
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        // The one pass.
        let mut reports: Vec<u64> = Vec::new();
        let (one_index, one_heads) =
            detect::read_archive(path, format, &CancelToken::new(), &mut |done| {
                reports.push(done);
            })
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(one_index, index, "{name}");
        assert_eq!(one_heads, heads, "{name}");
        assert!(
            !heads.elves.is_empty() || fixture == Fixture::Source,
            "{name}"
        );
        // Progress: a report per entry, in bytes of the file, never above
        // its size.
        let size = fs::metadata(path).unwrap_or_else(|e| panic!("{e}")).len();
        assert_eq!(reports.len(), index.items.len(), "{name}");
        assert!(
            reports.windows(2).all(|w| w[0] <= w[1]),
            "{name}: {reports:?}"
        );
        assert!(
            reports
                .last()
                .is_some_and(|last| *last > 0 && *last <= size),
            "{name}: {reports:?} against {size}"
        );
        // Cancelled before the first entry: nothing comes back.
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(
            matches!(
                detect::detect_with(path, &cancel, &mut |_| {}),
                Err(Error::Cancelled)
            ),
            "{name}"
        );
    }
}

#[test]
fn the_name_rule_takes_a_word_of_the_name_or_a_suffix_of_the_id() {
    let names = ["JetBrains Rider", "JetBrains Rider"];
    assert!(detect::name_matches("rider", &names));
    assert!(detect::name_matches("Rider", &names));
    assert!(detect::name_matches("jetbrains-rider", &names));
    assert!(detect::name_matches("rider.png", &names));
    assert!(!detect::name_matches("fsnotifier", &names));
    assert!(
        !detect::name_matches("ider", &names),
        "a suffix of a word is not a word"
    );
    assert!(!detect::name_matches("rider64", &names));
    assert!(detect::name_matches("code", &["Code - OSS"]));
    assert!(detect::name_matches(
        "idea",
        &["IntelliJ IDEA Community Edition"]
    ));
    assert!(!detect::name_matches("fixa", &["fixtool"]));
    assert!(detect::name_matches("fixtool", &["fixtool"]));
}

/// The acceptance round trip: install, launch through the desktop entry,
/// update with `current` moving, downgrade warning, revert, uninstall, and
/// the home left without a file of Rick's.
#[test]
#[allow(clippy::too_many_lines)]
fn install_update_revert_and_uninstall_leave_the_home_clean() {
    if !supported() {
        return;
    }
    let h = harness();
    let layout = h.layout();
    let before = files_under(&h.home());

    h.install(Fixture::GuiWithDesktop, Vec::new());
    let form = h.ui.last_form();
    assert!(form.title.starts_with("Install gui-with-desktop.tar.gz"));
    let rows = h.ui.last_rows();
    assert!(rows.iter().all(|r| r.problem.is_none()), "{rows:?}");
    assert!(rows.iter().any(|r| r.from == "Desktop entry"));

    let version_dir = layout.version_dir("fixture-gui", "1.5.0");
    assert!(version_dir.join("fixgui").is_file());
    assert!(version_dir.join("README").is_file());
    assert_eq!(h.current("fixture-gui"), "1.5.0");
    let record = h.record("fixture-gui");
    assert_eq!(record.name, "Fixture GUI");
    assert_eq!(record.kind, Kind::Gui);
    assert_eq!(record.main_executable, "fixgui");
    assert_eq!(record.previous_version, None);
    assert_eq!(record.startup_wm_class.as_deref(), Some("FixGui"));
    assert_eq!(record.source_archive, "gui-with-desktop.tar.gz");
    assert_eq!(record.external_files.len(), 3);
    let text = record.to_toml();
    assert!(text.contains("installed_at = 20"), "{text}");
    assert!(text.contains("[[external_files]]"), "{text}");
    assert!(text.contains("path = \"~/.local/share/applications/fixture-gui.desktop\""));

    // The desktop entry: the vendor's, with Exec and Icon through current.
    let desktop_path = layout.desktop_path("fixture-gui");
    let desktop = fs::read_to_string(&desktop_path).unwrap_or_else(|e| panic!("{e}"));
    let entry = desktop::DesktopEntry::parse(&desktop);
    let exec = entry.get("Exec").unwrap_or_default().to_owned();
    let expected = layout.current("fixture-gui").join("fixgui");
    assert_eq!(exec, format!("\"{}\" %U", expected.display()), "{desktop}");
    assert_eq!(entry.get("Icon"), Some("fixture-gui"));
    assert_eq!(entry.get("StartupWMClass"), Some("FixGui"));
    assert_eq!(entry.get("Comment"), Some("A fixture application"));
    assert_eq!(entry.get("MimeType"), Some("text/plain;"));
    let icon = layout.icon_path("fixture-gui", "64x64", "png");
    assert!(icon.is_file(), "{}", icon.display());
    let link = layout.bin_dir().join("fixture-gui");
    assert_eq!(
        fs::read_link(&link).unwrap_or_else(|e| panic!("{e}")),
        expected
    );
    // Launch: resolve the Exec line and run the program headlessly.
    let program = desktop::exec_program(&exec).unwrap_or_else(|| panic!("no program"));
    assert_eq!(program, "fixgui");
    let status = std::process::Command::new(&expected)
        .status()
        .unwrap_or_else(|e| panic!("running {}: {e}", expected.display()));
    assert!(status.success(), "{status}");
    let through_link = std::process::Command::new(&link)
        .status()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(through_link.success());

    // The view shows the row with its columns and the launch action.
    let rows = h.rows();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.name, "Fixture GUI");
    assert_eq!(h.cell(row, VERSION_COLUMN), "1.5.0");
    assert_eq!(h.cell(row, KIND_COLUMN), "GUI application");
    assert_eq!(h.cell(row, STATE_COLUMN), "ok");
    assert_eq!(
        h.action(LAUNCH)
            .availability(&h.ctx_view(vec![row.clone()])),
        Availability::Enabled
    );
    assert_eq!(
        h.action(REVERT)
            .availability(&h.ctx_view(vec![row.clone()])),
        Availability::Disabled("no previous version is kept".to_owned())
    );
    let below = h.list(&Location::new(ProviderId::RICK, "apps/fixture-gui"));
    assert!(
        below
            .iter()
            .any(|e| e.name == "1.5.0" && e.kind == EntryKind::Dir)
    );
    assert!(below.iter().any(|e| e.name == "rick-install.toml"));
    let opened = h.ok(OPEN_FOLDER, h.ctx_view(vec![row.clone()]));
    assert!(matches!(opened, ActionOutcome::Navigate(to) if to.provider == ProviderId::LOCAL));

    // Update: matched by app id, current moves, the previous version stays,
    // the desktop entry is not rewritten.
    let desktop_before = fs::metadata(&desktop_path)
        .and_then(|m| m.modified())
        .unwrap_or_else(|e| panic!("{e}"));
    std::thread::sleep(Duration::from_millis(20));
    h.install(Fixture::UpdateV2, Vec::new());
    let form = h.ui.last_form();
    assert_eq!(
        form.title,
        "Fixture GUI 1.5.0 is installed — update to 2.0.0?"
    );
    assert_eq!(form.ok, "Update");
    assert_eq!(h.current("fixture-gui"), "2.0.0");
    let record = h.record("fixture-gui");
    assert_eq!(record.version, "2.0.0");
    assert_eq!(record.previous_version.as_deref(), Some("1.5.0"));
    assert!(layout.version_dir("fixture-gui", "1.5.0").is_dir());
    assert!(layout.version_dir("fixture-gui", "2.0.0").is_dir());
    assert_eq!(
        fs::metadata(&desktop_path)
            .and_then(|m| m.modified())
            .unwrap_or_else(|e| panic!("{e}")),
        desktop_before,
        "the desktop entry is left untouched by an update"
    );
    assert_eq!(
        fs::read_link(&link).unwrap_or_else(|e| panic!("{e}")),
        expected
    );

    // A lower version warns; ticking Allow downgrade goes ahead and the
    // oldest version goes.
    h.ui.answer_form(Vec::new());
    let (outcome, _) = h.run(INSTALL, h.ctx_archive(Fixture::UpdateV1));
    let error = outcome
        .err()
        .unwrap_or_else(|| panic!("the downgrade was not refused"));
    assert!(text_of(&error).contains("is older"), "{error}");
    assert_eq!(h.current("fixture-gui"), "2.0.0");
    h.install(Fixture::UpdateV1, vec![("downgrade", Value::Check(true))]);
    assert_eq!(h.current("fixture-gui"), "1.0.0");
    let record = h.record("fixture-gui");
    assert_eq!(record.previous_version.as_deref(), Some("2.0.0"));
    assert!(
        !layout.version_dir("fixture-gui", "1.5.0").exists(),
        "one previous version is kept"
    );

    // Revert flips back and swaps the two fields.
    let row = h.row("Fixture GUI");
    assert_eq!(
        h.action(REVERT)
            .availability(&h.ctx_view(vec![row.clone()])),
        Availability::Enabled
    );
    h.ok(REVERT, h.ctx_view(vec![row]));
    assert_eq!(h.current("fixture-gui"), "2.0.0");
    let record = h.record("fixture-gui");
    assert_eq!(record.version, "2.0.0");
    assert_eq!(record.previous_version.as_deref(), Some("1.0.0"));

    // Edit launcher changes the name, the category and the class.
    let row = h.row("Fixture GUI");
    h.ui.answer_form(vec![
        ("name", Value::Text("Fixture App".into())),
        ("category", Value::Choice(1)),
        ("wm_class", Value::Text("fixture".into())),
    ]);
    h.ok(EDIT_LAUNCHER, h.ctx_view(vec![row]));
    let desktop = fs::read_to_string(&desktop_path).unwrap_or_else(|e| panic!("{e}"));
    let entry = desktop::DesktopEntry::parse(&desktop);
    assert_eq!(entry.get("Name"), Some("Fixture App"));
    assert_eq!(entry.get("Categories"), Some("Development;"));
    assert_eq!(entry.get("StartupWMClass"), Some("fixture"));
    assert_eq!(h.record("fixture-gui").name, "Fixture App");
    assert_eq!(h.rows()[0].name, "Fixture App");

    // Uninstall removes exactly the record's files and the app folder.
    let row = h.row("Fixture App");
    h.ok(UNINSTALL, h.ctx_view(vec![row]));
    assert!(!layout.app_dir("fixture-gui").exists());
    assert!(!desktop_path.exists() && !icon.exists());
    assert!(fs::symlink_metadata(&link).is_err());
    let rows = h.rows();
    assert!(rows.is_empty(), "{rows:?}");
    assert_eq!(files_under(&h.home()), before, "nothing of Rick's remains");
}

#[test]
fn a_checksum_mismatch_refuses_before_extracting() {
    if !supported() {
        return;
    }
    let h = harness();
    h.ui.answer_form(vec![("sha256", Value::Text("0".repeat(64)))]);
    let (outcome, results) = h.run(INSTALL, h.ctx_archive(Fixture::GuiWithDesktop));
    assert!(
        matches!(outcome, Ok(ActionOutcome::Started(_))),
        "{outcome:?}"
    );
    // Two jobs ran: reading the archive, then the install, which failed.
    assert_eq!(results.len(), 2, "{results:?}");
    let error = results
        .iter()
        .find_map(|r| r.as_ref().err())
        .unwrap_or_else(|| panic!("the job did not fail"));
    let text = text_of(error);
    assert!(
        text.contains("SHA-256") && text.contains(&"0".repeat(64)),
        "{text}"
    );
    assert!(!h.layout().app_dir("fixture-gui").exists());
    // A short hash is refused in the form.
    h.ui.answer_form(vec![("sha256", Value::Text("abc".into()))]);
    let (outcome, _) = h.run(INSTALL, h.ctx_archive(Fixture::GuiWithDesktop));
    assert!(outcome.is_err());
    // The right hash passes.
    let actual = archive::sha256(
        h.archive(Fixture::GuiWithDesktop),
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap_or_else(|e| panic!("{e}"));
    h.install(
        Fixture::GuiWithDesktop,
        vec![("sha256", Value::Text(actual.to_uppercase()))],
    );
    assert_eq!(h.current("fixture-gui"), "1.5.0");
}

#[test]
fn source_and_wrong_architecture_are_refused() {
    if !supported() {
        return;
    }
    let h = harness();
    h.ui.answer_form(Vec::new());
    let (outcome, _) = h.run(INSTALL, h.ctx_archive(Fixture::Source));
    let error = outcome
        .err()
        .unwrap_or_else(|| panic!("source not refused"));
    assert!(text_of(&error).contains("source code"), "{error}");
    let (outcome, _) = h.run(INSTALL, h.ctx_archive(Fixture::WrongArch));
    let error = outcome
        .err()
        .unwrap_or_else(|| panic!("architecture not refused"));
    assert!(text_of(&error).contains("built for"), "{error}");
    let (outcome, _) = h.run(INSTALL, h.ctx_archive(Fixture::Ambiguous));
    let error = outcome
        .err()
        .unwrap_or_else(|| panic!("the pick was not asked"));
    assert!(
        text_of(&error).contains("pick the main executable"),
        "{error}"
    );
    // Picking beta installs it.
    h.install(Fixture::Ambiguous, vec![("exe", Value::Choice(2))]);
    assert_eq!(h.record("fixamb").main_executable, "beta");
    assert!(h.rows().iter().any(|r| r.name == "fixamb"));
}

#[test]
fn collisions_are_refused_unless_overwrite_is_ticked() {
    if !supported() {
        return;
    }
    let h = harness();
    let layout = h.layout();
    fs::create_dir_all(layout.bin_dir()).unwrap_or_else(|e| panic!("{e}"));
    let foreign = layout.bin_dir().join("fixture-gui");
    fs::write(&foreign, "#!/bin/sh\necho mine\n").unwrap_or_else(|e| panic!("{e}"));
    h.ui.answer_form(Vec::new());
    let (outcome, _) = h.run(INSTALL, h.ctx_archive(Fixture::GuiWithDesktop));
    let error = outcome
        .err()
        .unwrap_or_else(|| panic!("collision not refused"));
    assert!(
        text_of(&error).contains("was not created by Rick"),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(&foreign).unwrap_or_default(),
        "#!/bin/sh\necho mine\n"
    );
    assert!(!layout.app_dir("fixture-gui").exists());
    h.install(
        Fixture::GuiWithDesktop,
        vec![("overwrite", Value::Check(true))],
    );
    assert!(fs::read_link(&foreign).is_ok(), "replaced by the symlink");
}

#[test]
fn a_toolchain_links_its_commands_or_goes_on_the_path() {
    if !supported() {
        return;
    }
    let h = harness();
    let layout = h.layout();
    // Three equal commands: the form opens on "(choose one)" and the
    // user picks the first.
    h.ui.answer_form(Vec::new());
    let (outcome, _) = h.run(INSTALL, h.ctx_archive(Fixture::Toolchain));
    let error = outcome
        .err()
        .unwrap_or_else(|| panic!("the pick was not asked"));
    assert!(
        text_of(&error).contains("pick the main executable"),
        "{error}"
    );
    h.install(Fixture::Toolchain, vec![("exe", Value::Choice(1))]);
    for name in ["fixa", "fixb", "fixc", "fixscript"] {
        let link = layout.bin_dir().join(name);
        assert_eq!(
            fs::read_link(&link).unwrap_or_else(|e| panic!("{name}: {e}")),
            layout.current("fixtool").join("bin").join(name)
        );
    }
    let script = layout.version_dir("fixtool", "1.2.0").join("bin/fixscript");
    let mode = ext::platform::fs::unix_mode(&script)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_default();
    assert_eq!(
        mode & 0o111,
        0o111,
        "the shebang script got its execute bit"
    );
    let record = h.record("fixtool");
    assert_eq!(record.kind, Kind::Cli);
    assert_eq!(record.path_mode, Some(PathMode::Symlinks));
    assert_eq!(record.commands(), vec!["fixa", "fixb", "fixc", "fixscript"]);
    let row = h.row("fixtool");
    assert_eq!(h.cell(&row, KIND_COLUMN), "command-line tool");
    assert_eq!(
        h.action(LAUNCH)
            .availability(&h.ctx_view(vec![row.clone()])),
        Availability::Hidden
    );
    h.ok(UNINSTALL, h.ctx_view(vec![row]));
    assert!(fs::symlink_metadata(layout.bin_dir().join("fixa")).is_err());

    // PATH mode: the managed file, the profile line, no symlinks.
    h.install(
        Fixture::Toolchain,
        vec![
            ("exe", Value::Choice(1)),
            ("path_mode", Value::Choice(1)),
            ("env_var", Value::Text("FIXTOOL_HOME".into())),
        ],
    );
    let path_file = layout.path_file();
    let sh = fs::read_to_string(&path_file).unwrap_or_else(|e| panic!("{e}"));
    let bin = layout.current("fixtool").join("bin");
    assert!(
        sh.contains(&format!("export PATH=\"{}:$PATH\"", bin.display())),
        "{sh}"
    );
    assert!(
        sh.contains(&format!(
            "export FIXTOOL_HOME=\"{}\"",
            layout.current("fixtool").display()
        )),
        "{sh}"
    );
    let profile = fs::read_to_string(h.home().join(".profile")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(profile.lines().count(), 1);
    assert!(
        profile.contains(&path_file.display().to_string()) && profile.contains("# added by rick")
    );
    assert!(fs::symlink_metadata(layout.bin_dir().join("fixa")).is_err());
    let record = h.record("fixtool");
    assert_eq!(record.path_mode, Some(PathMode::Path));
    assert_eq!(record.bin_dir.as_deref(), Some("bin"));
    assert_eq!(record.env_var.as_deref(), Some("FIXTOOL_HOME"));
    let row = h.row("fixtool");
    assert_eq!(
        h.cell(&row, crate::provider::COMMANDS_COLUMN),
        "bin on PATH"
    );
    // The last PATH-mode uninstall removes the file and the profile line.
    h.ok(UNINSTALL, h.ctx_view(vec![row]));
    assert!(!path_file.exists());
    assert!(
        !h.home().join(".profile").exists(),
        "the profile Rick made for its one line goes with it"
    );
}

#[test]
fn hostile_entries_are_left_out_and_nothing_runs() {
    if !supported() {
        return;
    }
    let h = harness();
    let layout = h.layout();
    h.install(Fixture::Hostile, Vec::new());
    let rows = h.ui.last_rows();
    assert!(
        rows.iter()
            .any(|r| r.from == "Archive" && r.to.contains("2 entries with unsafe paths"))
    );
    assert!(
        rows.iter()
            .any(|r| r.from == "Not run" && r.to.contains("install.sh"))
    );
    let version_dir = layout.version_dir("fixhostile", "1.0.0");
    assert!(version_dir.join("fixhostile").is_file());
    assert!(
        version_dir.join("install.sh").is_file(),
        "present, never run"
    );
    assert!(!layout.app_dir("fixhostile").join("evil").exists());
    assert!(!version_dir.join("evil").exists());
    assert!(!version_dir.join("abs").exists());
    assert!(!Path::new("/abs/evil").exists());
    assert!(
        fs::symlink_metadata(version_dir.join("etc-link")).is_err(),
        "the link to /etc is skipped"
    );
    assert_eq!(
        fs::read_link(version_dir.join("self-link")).ok(),
        Some(PathBuf::from("fixhostile")),
        "a link inside the folder is kept"
    );
    assert!(
        fs::symlink_metadata(version_dir.join("pipe")).is_err(),
        "the FIFO is skipped"
    );
    let suid = ext::platform::fs::unix_mode(&version_dir.join("suid"))
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_default();
    assert_eq!(suid & 0o4000, 0, "set-uid stripped: {suid:o}");
    assert!(!Path::new("/tmp/rick-fixture-ran").exists());
}

#[test]
fn a_tarbomb_lands_directly_in_the_version_folder() {
    if !supported() {
        return;
    }
    let h = harness();
    let layout = h.layout();
    h.ui.answer_form(Vec::new());
    let (outcome, _) = h.run(INSTALL, h.ctx_archive(Fixture::Tarbomb));
    assert!(outcome.is_err(), "an empty version is a problem row");
    h.install(
        Fixture::Tarbomb,
        vec![("version", Value::Text("3.1".into()))],
    );
    let version_dir = layout.version_dir("tarbomb", "3.1");
    assert!(version_dir.join("fixbomb").is_file());
    assert!(version_dir.join("README.md").is_file());
    assert!(version_dir.join("data/notes.txt").is_file());
    assert_eq!(
        fs::read_link(version_dir.join("fixbomb-link")).ok(),
        Some(PathBuf::from("fixbomb"))
    );
    let link = layout.bin_dir().join("tarbomb");
    assert_eq!(
        fs::read_link(&link).ok(),
        Some(layout.current("tarbomb").join("fixbomb"))
    );
    assert!(
        std::process::Command::new(&link)
            .status()
            .is_ok_and(|s| s.success())
    );
}

#[test]
fn update_from_archive_asks_for_the_archive_then_shows_the_form() {
    if !supported() {
        return;
    }
    let h = harness();
    h.install(Fixture::GuiWithDesktop, Vec::new());
    let row = h.row("Fixture GUI");
    // No answer: nothing happens.
    *h.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = None;
    let outcome = h.ok(UPDATE, h.ctx_view(vec![row.clone()]));
    assert!(matches!(outcome, ActionOutcome::Done));
    assert_eq!(h.current("fixture-gui"), "1.5.0");
    // The archive's path: the form opens in update mode and the update runs.
    *h.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) =
        Some(h.archive(Fixture::UpdateV2).display().to_string());
    h.ui.answer_form(Vec::new());
    h.ok(UPDATE, h.ctx_view(vec![row]));
    let form = h.ui.last_form();
    assert_eq!(form.ok, "Update");
    assert!(
        form.fields.iter().all(|f| f.key != "update_of"),
        "the target is fixed"
    );
    assert_eq!(h.current("fixture-gui"), "2.0.0");
    assert_eq!(
        h.record("fixture-gui").previous_version.as_deref(),
        Some("1.5.0")
    );
}

#[test]
fn repair_rewrites_what_is_missing_and_forget_drops_the_record() {
    if !supported() {
        return;
    }
    let h = harness();
    let layout = h.layout();
    h.install(Fixture::GuiWithDesktop, Vec::new());
    let desktop_path = layout.desktop_path("fixture-gui");
    let link = layout.bin_dir().join("fixture-gui");
    fs::remove_file(&desktop_path).unwrap_or_else(|e| panic!("{e}"));
    fs::remove_file(&link).unwrap_or_else(|e| panic!("{e}"));
    fs::remove_file(layout.current("fixture-gui")).unwrap_or_else(|e| panic!("{e}"));
    let row = h.row("Fixture GUI");
    let state = h.cell(&row, STATE_COLUMN);
    assert!(
        state.contains("current symlink dangling")
            && state.contains("desktop entry missing")
            && state.contains("symlink fixture-gui broken"),
        "{state}"
    );
    assert_eq!(
        h.action(REPAIR)
            .availability(&h.ctx_view(vec![row.clone()])),
        Availability::Enabled
    );
    assert_eq!(
        h.action(LAUNCH)
            .run(h.ctx_view(vec![row.clone()]), &h.jobs, &h.ui)
            .err()
            .map(|e| text_of(&e)),
        Some(
            "unsupported: the app has no desktop entry to launch through; Repair writes one"
                .to_owned()
        )
    );
    h.ok(REPAIR, h.ctx_view(vec![row]));
    assert_eq!(h.current("fixture-gui"), "1.5.0");
    assert!(desktop_path.is_file());
    let entry = desktop::DesktopEntry::parse(
        &fs::read_to_string(&desktop_path).unwrap_or_else(|e| panic!("{e}")),
    );
    assert_eq!(entry.get("Name"), Some("Fixture GUI"));
    assert_eq!(entry.get("Icon"), Some("fixture-gui"));
    assert_eq!(
        fs::read_link(&link).ok(),
        Some(layout.current("fixture-gui").join("fixgui"))
    );
    let row = h.row("Fixture GUI");
    assert_eq!(h.cell(&row, STATE_COLUMN), "ok");
    assert_eq!(
        h.action(REPAIR)
            .availability(&h.ctx_view(vec![row.clone()])),
        Availability::Disabled("nothing to repair".to_owned())
    );

    // Forget deletes the record and leaves the files.
    h.ok(FORGET, h.ctx_view(vec![row]));
    let rows = h.rows();
    assert!(rows.is_empty(), "{rows:?}");
    assert!(!layout.record_path("fixture-gui").exists());
    assert!(
        layout
            .version_dir("fixture-gui", "1.5.0")
            .join("fixgui")
            .is_file()
    );
    assert!(desktop_path.is_file());
}

#[test]
fn orphaned_staging_folders_are_listed_and_removed() {
    if !supported() {
        return;
    }
    let h = harness();
    let layout = h.layout();
    let staging = layout.staging("ghost", "9.9");
    fs::create_dir_all(staging.join("bin")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(staging.join("bin/ghost"), b"x").unwrap_or_else(|e| panic!("{e}"));
    let rows = h.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "ghost (unfinished 9.9)");
    assert_eq!(h.cell(&rows[0], STATE_COLUMN), "orphaned staging folder");
    assert_eq!(
        h.action(REMOVE_STAGING)
            .availability(&h.ctx_view(vec![rows[0].clone()])),
        Availability::Enabled
    );
    assert_eq!(
        h.action(UNINSTALL)
            .availability(&h.ctx_view(vec![rows[0].clone()])),
        Availability::Hidden
    );
    h.ok(REMOVE_STAGING, h.ctx_view(vec![rows[0].clone()]));
    assert!(!staging.exists());
    let rows = h.rows();
    assert!(rows.is_empty(), "{rows:?}");
}

#[test]
fn the_library_check_resolves_the_system_libraries() {
    if !supported() {
        return;
    }
    let h = harness();
    h.install(Fixture::GuiNoDesktop, Vec::new());
    let layout = h.layout();
    let version_dir = layout.version_dir("fixnodesk", "0.3.0");
    let unresolved = install::unresolved_libraries(
        &version_dir,
        &[version_dir.join("fixnodesk")],
        &install::Libraries::system(),
    );
    assert!(
        !unresolved.contains(&"libc.so.6".to_owned()),
        "{unresolved:?}"
    );
    assert!(
        unresolved.is_empty() || unresolved == vec!["libgtk-3.so.0".to_owned()],
        "{unresolved:?}"
    );
    // The generated desktop entry and the SVG in scalable/apps.
    let desktop =
        fs::read_to_string(layout.desktop_path("fixnodesk")).unwrap_or_else(|e| panic!("{e}"));
    let entry = desktop::DesktopEntry::parse(&desktop);
    assert_eq!(entry.get("Type"), Some("Application"));
    assert_eq!(entry.get("Name"), Some("fixnodesk"));
    assert_eq!(entry.get("Icon"), Some("fixnodesk"));
    assert_eq!(entry.get("Terminal"), Some("false"));
    assert_eq!(entry.get("Categories"), Some("Utility;"));
    assert!(layout.icon_path("fixnodesk", "scalable", "svg").is_file());
}

#[test]
fn desktop_entries_validate_with_desktop_file_validate() {
    if !supported() {
        return;
    }
    let h = harness();
    h.install(Fixture::GuiWithDesktop, Vec::new());
    h.install(Fixture::GuiNoDesktop, Vec::new());
    let layout = h.layout();
    for id in ["fixture-gui", "fixnodesk"] {
        let path = layout.desktop_path(id);
        match desktop::validate(&path) {
            Some(Ok(())) => {}
            Some(Err(output)) => panic!("{}: {output}", path.display()),
            None => {
                assert!(
                    std::env::var_os("RICK_REQUIRE_DESKTOP_FILE_VALIDATE").is_none(),
                    "desktop-file-validate is required here and missing"
                );
                eprintln!("desktop-file-validate is not installed; skipping");
            }
        }
    }
}

#[test]
fn desktop_entry_helpers() {
    let mut entry = desktop::DesktopEntry::parse(
        "# a comment\n[Desktop Entry]\nName=App\nExec=\"/opt/app/bin/app\" --flag %F\nIcon=/opt/app/app.png\n\n[Desktop Action New]\nName=New\nExec=app --new\n",
    );
    assert_eq!(
        desktop::exec_program("\"/opt/app/bin/app\" --flag %F"),
        Some("app".to_owned())
    );
    assert_eq!(
        desktop::exec_program("fixgui %U"),
        Some("fixgui".to_owned())
    );
    assert_eq!(
        desktop::exec_arguments("fixgui --a \"b c\" %U"),
        vec!["--a", "b c", "%U"]
    );
    entry.set(
        "Exec",
        &desktop::exec_through(Path::new("/x/current/app"), entry.get("Exec")),
    );
    entry.set("Icon", "app");
    entry.set("StartupWMClass", "App");
    assert_eq!(entry.get("Exec"), Some("\"/x/current/app\" \"--flag\" %F"));
    assert_eq!(entry.get("Icon"), Some("app"));
    let text = entry.text();
    assert!(text.starts_with("# a comment\n[Desktop Entry]\n"), "{text}");
    assert!(
        text.contains("StartupWMClass=App\n\n[Desktop Action New]"),
        "{text}"
    );
    assert!(
        text.contains("Exec=app --new\n"),
        "the action group is untouched"
    );
    entry.remove("StartupWMClass");
    assert_eq!(entry.get("StartupWMClass"), None);
    assert_eq!(
        desktop::quote(Path::new("/a b/$x\"y%")),
        "\"/a b/\\$x\\\"y%%\""
    );
    let generated = desktop::DesktopEntry::generate("N", "\"/c/app\"", "n", "Utility", Some("W"));
    assert_eq!(
        generated.text(),
        "[Desktop Entry]\nType=Application\nName=N\nExec=\"/c/app\"\nTerminal=false\nIcon=n\nCategories=Utility;\nStartupWMClass=W\n"
    );
}

#[test]
fn records_round_trip_and_the_layout_expands_tildes() {
    let text = r#"id = "postman"
name = "Postman"
kind = "gui"
scope = "user"
version = "11.3.0"
previous_version = "11.2.0"
installed_at = 2026-11-07T18:42:00Z
source_archive = "Postman-linux-x64-11.3.0.tar.gz"
source_pattern = "Postman-linux-x64-*.tar.gz"
main_executable = "Postman"
startup_wm_class = "Postman"

[[external_files]]
path = "~/.local/share/applications/postman.desktop"
kind = "desktop-entry"

[[external_files]]
path = "~/.local/bin/postman"
kind = "symlink"
"#;
    let record = Record::parse(text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(record.kind, Kind::Gui);
    assert_eq!(record.external_files.len(), 2);
    assert_eq!(record.commands(), vec!["postman"]);
    assert!(record.installed_at_time().is_some());
    let again = Record::parse(&record.to_toml()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(again, record);
    assert!(Record::parse("id = 1").is_err());
    let fake = Fake::new().unwrap_or_else(|e| panic!("{e}"));
    let layout = Layout::user(&fake);
    let home = fake.home_dir();
    assert_eq!(layout.apps_root(), home.join(".local/opt"));
    assert_eq!(layout.tilde(&home.join(".local/bin/x")), "~/.local/bin/x");
    assert_eq!(layout.expand("~/.local/bin/x"), home.join(".local/bin/x"));
    assert_eq!(layout.expand("/opt/x"), PathBuf::from("/opt/x"));
    let system = Layout::system(&fake);
    assert_eq!(system.apps_root(), PathBuf::from("/opt"));
    assert_eq!(system.bin_dir(), PathBuf::from("/usr/local/bin"));
    assert_eq!(
        system.icon_path("x", "scalable", "svg"),
        PathBuf::from("/usr/share/icons/hicolor/scalable/apps/x.svg")
    );
}

#[test]
fn the_system_scope_is_unavailable_in_this_version() {
    if !supported() {
        return;
    }
    let h = harness();
    h.ui.answer_form(vec![("scope", Value::Choice(1))]);
    let (outcome, _) = h.run(INSTALL, h.ctx_archive(Fixture::GuiWithDesktop));
    let error = outcome
        .err()
        .unwrap_or_else(|| panic!("system-wide was not refused"));
    assert!(text_of(&error).contains("privileged helper"), "{error}");
    let form = h.ui.last_form();
    let scope = form
        .fields
        .iter()
        .find(|f| f.key == "scope")
        .unwrap_or_else(|| panic!("no scope field"));
    assert!(
        matches!(&scope.kind, ext::FieldKind::Segmented { options, selected: 0 } if options[1] == "All users")
    );
}

/// With Rick itself among the installed apps, `path.sh` and the fish
/// drop-in define the `rick` function over its `current` binary
/// (specs/system-integration.md §4); without it and without a PATH-mode
/// app the files go.
#[test]
fn the_path_file_defines_the_rick_function_when_rick_is_installed() {
    let h = harness();
    if !supported() {
        return;
    }
    let layout = h.layout();
    fs::create_dir_all(h.home().join(".config/fish")).unwrap_or_else(|e| panic!("{e}"));
    h.install(Fixture::GuiWithDesktop, Vec::new());
    let mut record = h.record("fixture-gui");
    record.id = "rick".to_owned();
    record.main_executable = "rick".to_owned();
    install::write_path_file(&layout, &[record.clone()]).unwrap_or_else(|e| panic!("{e}"));
    let sh = fs::read_to_string(layout.path_file()).unwrap_or_else(|e| panic!("{e}"));
    let binary = layout.current("rick").join("rick");
    assert!(
        sh.contains(&format!("rick() {{ \"{}\" \"$@\"; }}", binary.display())),
        "{sh}"
    );
    assert!(!sh.contains("export PATH"), "no PATH-mode app: {sh}");
    let fish = fs::read_to_string(h.home().join(".config/fish/conf.d/rick.fish"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        fish.contains("function rick") && fish.contains(&format!("\"{}\" $argv", binary.display())),
        "{fish}"
    );
    let profile = fs::read_to_string(h.home().join(".profile")).unwrap_or_else(|e| panic!("{e}"));
    assert!(profile.contains(&layout.path_file().display().to_string()));
    // Rick uninstalled (the record gone), nothing else on the PATH: the
    // files go.
    install::write_path_file(&layout, &[]).unwrap_or_else(|e| panic!("{e}"));
    assert!(!layout.path_file().exists());
    assert!(!h.home().join(".config/fish/conf.d/rick.fish").exists());
}

/// The `rick` command of the integration items keeps its shell function
/// in a marked block of `path.sh` and the fish drop-in
/// (specs/system-integration.md §7.1): regenerating the files from the
/// install records carries the block over, in place of the records' own
/// function, and a file the records no longer need stays for the block.
#[test]
fn the_path_file_keeps_the_rick_commands_block() {
    let h = harness();
    if !supported() {
        return;
    }
    let layout = h.layout();
    let (begin, end) = ext::platform::integration::COMMAND_BLOCK;
    let block = format!("{begin}\nrick() {{ \"/elsewhere/rick\" \"$@\"; }}\n{end}\n");
    fs::create_dir_all(h.home().join(".config/fish/conf.d")).unwrap_or_else(|e| panic!("{e}"));
    let fish_file = h.home().join(".config/fish/conf.d/rick.fish");
    let fish_block = format!("{begin}\nfunction rick\n    \"/elsewhere/rick\" $argv\nend\n{end}\n");
    let path_file = layout.path_file();
    if let Some(parent) = path_file.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::write(layout.path_file(), format!("# old\n{block}")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(&fish_file, &fish_block).unwrap_or_else(|e| panic!("{e}"));

    // Rick among the records: the block stands in for the records' function.
    h.install(Fixture::GuiWithDesktop, Vec::new());
    let mut record = h.record("fixture-gui");
    record.id = "rick".to_owned();
    record.main_executable = "rick".to_owned();
    install::write_path_file(&layout, &[record]).unwrap_or_else(|e| panic!("{e}"));
    let sh = fs::read_to_string(layout.path_file()).unwrap_or_else(|e| panic!("{e}"));
    assert!(sh.ends_with(&block), "{sh}");
    assert_eq!(sh.matches("rick() {").count(), 1, "one definition: {sh}");
    let fish = fs::read_to_string(&fish_file).unwrap_or_else(|e| panic!("{e}"));
    assert!(fish.ends_with(&fish_block), "{fish}");
    assert_eq!(fish.matches("function rick").count(), 1, "{fish}");

    // No record needs the files any more: they stay, for the block.
    install::write_path_file(&layout, &[]).unwrap_or_else(|e| panic!("{e}"));
    let sh = fs::read_to_string(layout.path_file()).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        sh.starts_with("# Generated by Rick") && sh.ends_with(&block),
        "{sh}"
    );
    assert!(fs::read_to_string(&fish_file).is_ok_and(|fish| fish.ends_with(&fish_block)));
}

/// A Rick unpacked into `folder`, as the release archive unpacks: the
/// binary, a library, a plugin and `share/`, with one link that stays
/// inside the folder and one that leaves it.
fn unpacked_rick(folder: &Path) {
    fs::create_dir_all(folder.join("plugins/git")).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(folder.join("share/applications")).unwrap_or_else(|e| panic!("{e}"));
    let write = |rel: &str, bytes: &[u8]| {
        fs::write(folder.join(rel), bytes).unwrap_or_else(|e| panic!("{rel}: {e}"));
    };
    write("rick", &tiny_elf(this_machine(), &[]));
    write("librick_kernel.so", b"kernel");
    write("plugins/git/plugin.toml", b"id = \"git\"\n");
    write("plugins/git/libext_git.so", b"plugin");
    write(
        "share/applications/rick.desktop",
        b"[Desktop Entry]\nName=Rick\nExec=rick %U\n",
    );
    ext::platform::fs::symlink(Path::new("rick"), &folder.join("rick-again"))
        .unwrap_or_else(|e| panic!("{e}"));
    ext::platform::fs::symlink(Path::new("/etc/hostname"), &folder.join("outside"))
        .unwrap_or_else(|e| panic!("{e}"));
}

/// An unpacked folder reads as one more archive format: every file below
/// it an entry relative to it, each folder before what it holds, nothing
/// hoisted, the links as links.
#[test]
fn a_folder_reads_as_an_archive_of_its_files() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let folder = dir.path().join("rick-0.3.0-linux-x64");
    unpacked_rick(&folder);
    let index = archive::index(&folder, Format::Folder).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(index.top_level, None, "the folder is taken as it is");
    let paths: Vec<&str> = index.items.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "librick_kernel.so",
            "outside",
            "plugins",
            "rick",
            "rick-again",
            "share",
            "plugins/git",
            "plugins/git/libext_git.so",
            "plugins/git/plugin.toml",
            "share/applications",
            "share/applications/rick.desktop",
        ]
    );
    assert!(index.has_file("rick") && index.has_file("plugins/git/plugin.toml"));
    assert_eq!(index.at("plugins").map(|i| i.kind), Some(ItemKind::Dir));
    let link = index.at("rick-again").unwrap_or_else(|| panic!("the link"));
    assert_eq!(link.kind, ItemKind::Symlink);
    assert_eq!(link.link.as_deref(), Some("rick"));
    // The files' bytes; links and folders weigh nothing.
    let files: u64 = index
        .items
        .iter()
        .filter(|item| item.kind == ItemKind::File)
        .map(|item| fs::metadata(folder.join(&item.path)).map_or_default(|meta| meta.len()))
        .sum();
    assert_eq!(index.total_size, files);
    assert!(files > tiny_elf(this_machine(), &[]).len() as u64);
    assert_eq!(Format::Folder.name(), "folder");
    assert_eq!(Format::by_name("rick-0.3.0-linux-x64"), None);
}

/// The registered `install-rick` command and what a caller in `cwd`
/// hands it.
fn install_rick_command(h: &Harness) -> (Arc<dyn ext::CliCommand>, impl Fn(&[&str]) -> CliCtx) {
    let command = h
        .registry
        .commands()
        .iter()
        .find(|c| c.name() == INSTALL_RICK_COMMAND)
        .cloned()
        .unwrap_or_else(|| panic!("the install-rick command is registered"));
    let cwd = h.dir.path().to_path_buf();
    let ctx = move |args: &[&str]| CliCtx {
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        cwd: cwd.clone(),
        pane: PaneId(1),
        location: Location::new(ProviderId::LOCAL, ext::platform::paths::from_path(&cwd)),
    };
    (command, ctx)
}

/// Runs `rick install-rick <args>` and waits for its one job to succeed.
fn install_rick(h: &Harness, args: &[&str]) {
    let (command, ctx) = install_rick_command(h);
    let outcome = command
        .run(ctx(args), &h.jobs, &h.ui)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        matches!(outcome, CliOutcome::Outcome(ActionOutcome::Started(_))),
        "one job, no form: {outcome:?}"
    );
    h.settle();
    let mut events = h.events.lock().unwrap_or_else(PoisonError::into_inner);
    while let Ok(event) = events.try_recv() {
        if let JobEvent::Finished { result, .. } = event {
            result.unwrap_or_else(|e| panic!("the install job: {e}"));
        }
    }
}

/// `rick install-rick` takes a folder that holds the binary and a version
/// that can name a folder; anything else is the caller's error, and
/// nothing is installed. Linux only, like the rest of the installer.
#[test]
fn the_install_rick_command_refuses_what_is_no_rick() {
    let h = harness();
    let (command, ctx) = install_rick_command(&h);
    unpacked_rick(&h.dir.path().join("rick-0.3.0-linux-x64"));
    if !supported() {
        let refused = command.run(ctx(&["rick-0.3.0-linux-x64", "0.3.0"]), &h.jobs, &h.ui);
        assert!(refused.is_err(), "Linux only: {refused:?}");
        return;
    }
    assert!(matches!(
        command.run(ctx(&["rick-0.3.0-linux-x64"]), &h.jobs, &h.ui),
        Err(Error::Unsupported(_))
    ));
    assert!(
        command.run(ctx(&[".", "0.3.0"]), &h.jobs, &h.ui).is_err(),
        "a folder without the binary"
    );
    assert!(
        command
            .run(ctx(&["rick-0.3.0-linux-x64", "../up"]), &h.jobs, &h.ui)
            .is_err(),
        "a version that cannot name a folder"
    );
    assert!(!h.layout().app_dir("rick").exists());
}

/// `rick install-rick <folder> <version>`, what the integration dialog's
/// *Install to the standard location* runs: the folder is copied as it is
/// into `~/.local/opt/rick/<version>` with the `current` link and the
/// record, and nothing is written outside the app folder — no launcher
/// entry, no icon, no command link. A later version updates and keeps the
/// one before.
#[test]
fn the_install_rick_command_copies_the_folder_to_the_standard_location() {
    let h = harness();
    if !supported() {
        return;
    }
    let folder = h.dir.path().join("rick-0.3.0-linux-x64");
    unpacked_rick(&folder);
    // A launcher entry and a command of another origin are not this
    // install's business: it writes neither, so neither is in its way.
    let layout = h.layout();
    let foreign_entry = layout.desktop_path("rick");
    fs::create_dir_all(layout.applications_dir()).unwrap_or_else(|e| panic!("{e}"));
    fs::write(&foreign_entry, b"[Desktop Entry]\nName=Other\n").unwrap_or_else(|e| panic!("{e}"));

    install_rick(&h, &["rick-0.3.0-linux-x64", "0.3.0"]);
    let app = layout.app_dir("rick");
    let version = app.join("0.3.0");
    assert_eq!(
        fs::read(version.join("rick")).ok(),
        Some(tiny_elf(this_machine(), &[]))
    );
    let mode = ext::platform::fs::unix_mode(&version.join("rick"))
        .ok()
        .flatten()
        .unwrap_or(0);
    assert_eq!(mode & 0o111, 0o111, "the binary is executable: {mode:o}");
    assert!(version.join("librick_kernel.so").is_file());
    assert!(version.join("plugins/git/libext_git.so").is_file());
    assert!(version.join("share/applications/rick.desktop").is_file());
    assert_eq!(
        fs::read_link(version.join("rick-again")).ok().as_deref(),
        Some(Path::new("rick"))
    );
    assert!(
        fs::symlink_metadata(version.join("outside")).is_err(),
        "a link out of the folder is left out"
    );
    assert_eq!(
        fs::read_link(layout.current("rick")).ok().as_deref(),
        Some(Path::new("0.3.0"))
    );
    assert!(
        folder.join("rick").is_file(),
        "the folder is copied, not moved"
    );
    let record = h.record("rick");
    assert_eq!(record.name, "Rick");
    assert_eq!(record.kind, Kind::Gui);
    assert_eq!(record.version, "0.3.0");
    assert_eq!(record.main_executable, "rick");
    assert_eq!(record.source_archive, "rick-0.3.0-linux-x64");
    assert_eq!(record.external_files, []);
    assert_eq!(
        fs::read(&foreign_entry).ok().as_deref(),
        Some(&b"[Desktop Entry]\nName=Other\n"[..]),
        "nothing outside the app folder is touched"
    );
    assert!(fs::symlink_metadata(layout.bin_dir().join("rick")).is_err());
    assert!(!layout.staging("rick", "0.3.0").exists());

    // The next version, unpacked elsewhere: an update with one kept.
    let newer = h.dir.path().join("rick-0.3.1-linux-x64");
    unpacked_rick(&newer);
    fs::write(newer.join("NEW"), b"new").unwrap_or_else(|e| panic!("{e}"));
    install_rick(&h, &["rick-0.3.1-linux-x64", "0.3.1"]);
    assert_eq!(
        fs::read_link(layout.current("rick")).ok().as_deref(),
        Some(Path::new("0.3.1"))
    );
    assert!(app.join("0.3.1/NEW").is_file());
    assert!(
        app.join("0.3.0/rick").is_file(),
        "the previous version is kept"
    );
    let record = h.record("rick");
    assert_eq!(record.version, "0.3.1");
    assert_eq!(record.previous_version.as_deref(), Some("0.3.0"));
}

/// An AppImage is told by its name, which ends in `.AppImage` in any
/// case, or by the magic in its ELF header, `AI` and then 1 or 2 at
/// offset 8 (specs/ext-installer.md "AppImages"). An ELF without the
/// magic, the magic without an ELF header and a name that only resembles
/// the extension are none.
#[test]
fn an_appimage_is_told_by_its_name_or_by_the_magic_in_its_header() {
    for name in [
        "Fix.AppImage",
        "fix.appimage",
        "FIX.APPIMAGE",
        "Fix-1.2.3-x86_64.AppImage",
        "Füx.AppImage",
        ".AppImage",
    ] {
        assert!(appimage::has_extension(name), "{name}");
    }
    for name in ["Fix.AppImage.zsync", "appimage", "Fix.AppImag", "Füx", ""] {
        assert!(!appimage::has_extension(name), "{name}");
    }

    let two = tiny_appimage(this_machine(), 2, &[]);
    assert_eq!(appimage::magic(&two), Some(Type::Two));
    assert_eq!(
        appimage::magic(&tiny_appimage(other_machine(), 1, &[])),
        Some(Type::One)
    );
    assert_eq!(
        appimage::magic(&tiny_elf(this_machine(), &[])),
        None,
        "an ELF without the magic"
    );
    assert_eq!(
        appimage::magic(&tiny_appimage(this_machine(), 3, &[])),
        None,
        "no type 3"
    );
    assert_eq!(
        appimage::magic(b"\x7fELX\x02\x01\x01\0AI\x02"),
        None,
        "the magic without an ELF header"
    );
    assert_eq!(appimage::magic(&two[..10]), None, "a header cut short");

    // On disk the header is read only where the name does not say it.
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.path().join(name);
        fs::write(&path, bytes).unwrap_or_else(|e| panic!("{e}"));
        path
    };
    assert!(appimage::is_appimage(&write("fix", &two), "fix"));
    assert_eq!(appimage::magic_of(&dir.path().join("fix")), Some(Type::Two));
    assert!(!appimage::is_appimage(
        &write("tool", &tiny_elf(this_machine(), &[])),
        "tool"
    ));
    assert!(!appimage::is_appimage(&write("short", b"\x7fELF"), "short"));
    assert!(appimage::is_appimage(
        &write("notes.AppImage", b"text\n"),
        "notes.AppImage"
    ));
    assert!(!appimage::is_appimage(&dir.path().join("gone"), "gone"));
}

/// *Run* is in the menu of one AppImage on the local file system and of
/// nothing else: not of another program, an archive, a folder, a
/// selection of two, a file on another provider; and not at all outside
/// Linux (specs/ext-installer.md "AppImages").
#[test]
fn run_is_offered_on_one_local_appimage_and_nowhere_else() {
    let h = harness();
    let run = h.action(RUN);
    let image = tiny_appimage(this_machine(), 2, &[]);
    let named = h.file("Fix.AppImage", &image, 0o644);
    if !supported() {
        assert_eq!(run.availability(&h.ctx_file(&named)), Availability::Hidden);
        return;
    }
    assert_eq!(run.availability(&h.ctx_file(&named)), Availability::Enabled);
    let magic_only = h.file("fix-magic", &image, 0o644);
    assert_eq!(
        run.availability(&h.ctx_file(&magic_only)),
        Availability::Enabled,
        "the magic under a name without the extension"
    );
    let program = h.file("fix-tool", &tiny_elf(this_machine(), &[]), 0o755);
    assert_eq!(
        run.availability(&h.ctx_file(&program)),
        Availability::Hidden
    );
    assert_eq!(
        run.availability(&h.ctx_archive(Fixture::GuiWithDesktop)),
        Availability::Hidden
    );
    let folder = h.dir.path().join("Folder.AppImage");
    fs::create_dir(&folder).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(run.availability(&h.ctx_file(&folder)), Availability::Hidden);
    assert_eq!(
        run.availability(&h.ctx_selection(Vec::new())),
        Availability::Hidden
    );
    assert_eq!(
        run.availability(&h.ctx_selection(vec![h.file_entry(&named), h.file_entry(&magic_only)])),
        Availability::Hidden,
        "one file, not two"
    );
    let mut remote = h.file_entry(&named);
    remote.id = EntryId::new(ProviderId("ssh"), 7);
    assert_eq!(
        run.availability(&h.ctx_selection(vec![remote])),
        Availability::Hidden,
        "a file that is not on this machine"
    );

    // A file that becomes an AppImage is seen as one once it is listed
    // anew, with its new size or time.
    let grows = h.file("fix-grows", b"not yet", 0o644);
    assert_eq!(run.availability(&h.ctx_file(&grows)), Availability::Hidden);
    fs::write(&grows, &image).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(run.availability(&h.ctx_file(&grows)), Availability::Enabled);
}

/// Acceptance (issue 223): *Run* on an AppImage without the execute bit
/// starts it, detached from Rick and in its folder, and the bit is set
/// afterwards, for the owner and nobody else. The file's bytes are as
/// they were, and a file that has the bit keeps its mode.
#[test]
fn run_sets_the_execute_bit_and_starts_the_appimage_in_its_folder() {
    let h = harness();
    if !supported() {
        return;
    }
    let image = tiny_appimage(this_machine(), 2, &[]);
    let path = h.file("Fix.AppImage", &image, 0o644);
    let mode = |path: &Path| {
        pfs::unix_mode(path)
            .unwrap_or_else(|e| panic!("{e}"))
            .map(|mode| mode & 0o7777)
    };
    let outcome = h.ok(RUN, h.ctx_file(&path));
    assert!(matches!(outcome, ActionOutcome::Done), "{outcome:?}");
    assert_eq!(
        h.fake.ran(),
        [(path.clone(), Vec::new(), h.dir.path().to_path_buf())]
    );
    assert_eq!(mode(&path), Some(0o744), "the owner's bit, no other");
    assert_eq!(fs::read(&path).ok(), Some(image));

    pfs::set_unix_mode(&path, 0o500).unwrap_or_else(|e| panic!("{e}"));
    h.ok(RUN, h.ctx_file(&path));
    assert_eq!(h.fake.ran().len(), 2);
    assert_eq!(mode(&path), Some(0o500), "a mode with the bit is kept");
}

/// *Run* on an AppImage built for another machine says so and stops, as
/// the bonus check does: nothing is started and the execute bit is not
/// set. A file that is only named like an AppImage is refused the same
/// way.
#[test]
fn run_refuses_another_machines_appimage_before_touching_it() {
    let h = harness();
    if !supported() {
        return;
    }
    let refused = |name: &str, bytes: &[u8]| {
        let path = h.file(name, bytes, 0o644);
        let (outcome, _) = h.run(RUN, h.ctx_file(&path));
        let error = match outcome {
            Err(error) => error.to_string(),
            Ok(outcome) => panic!("{name} ran: {outcome:?}"),
        };
        assert!(h.fake.ran().is_empty(), "{name} was started");
        assert_eq!(
            pfs::unix_mode(&path)
                .ok()
                .flatten()
                .map(|mode| mode & 0o777),
            Some(0o644),
            "{name} was made executable"
        );
        error
    };
    let error = refused("Other.AppImage", &tiny_appimage(other_machine(), 2, &[]));
    assert!(
        error.contains(&elf::machine_name(other_machine()))
            && error.contains(&elf::machine_name(this_machine())),
        "{error}"
    );
    let error = refused("notes.AppImage", b"only named like one\n");
    assert!(error.contains("not a program"), "{error}");
}

/// The permission bits of the entry at `path`.
fn mode_of(path: &Path) -> Option<u32> {
    pfs::unix_mode(path)
        .unwrap_or_else(|e| panic!("{e}"))
        .map(|mode| mode & 0o7777)
}

/// The versions of an AppStream metainfo's releases are read off its
/// `<release>` tags, in the file's order: both quotes, attributes in any
/// order, a tag cut short; a comment and `<releases>` are no release.
#[test]
fn the_release_versions_of_a_metainfo_are_read_off_its_tags() {
    let xml = "<?xml version=\"1.0\"?>\n<component>\n <releases>\n\
        <!-- <release version=\"9.9\"/> -->\n\
        <release date=\"2026-02-01\" version=\"1.4.0\" />\n\
        <release version='1.3.2'>\n <description/>\n </release>\n\
        <release>\n</release>\n\
        <release type=\"stable\" urgency = \"low\" version = \"1.3.0\"/>\n\
        </releases>\n</component>\n<release version=\"0.1";
    assert_eq!(appimage::release_versions(xml), ["1.4.0", "1.3.2", "1.3.0"]);
    let none: [&str; 0] = [];
    assert_eq!(appimage::release_versions("no tags at all"), none);
    assert_eq!(appimage::release_versions("<releases/><release"), none);
}

/// Acceptance (issue 223): name, icon and version are read out of the
/// AppImage, by the spec's order of the version sources — the desktop
/// entry, then the newest release of the metainfo, then the file name —
/// and with none of them the version stays empty. Nothing is run, and an
/// AppImage never goes through the archive detection.
#[test]
#[allow(clippy::too_many_lines)]
fn an_appimage_is_read_for_its_name_icon_and_version_without_being_run() {
    let h = harness();
    let detected = |fixture: AppImageFixture| {
        detect::detect(&h.appimage(fixture)).unwrap_or_else(|e| panic!("{fixture:?}: {e}"))
    };

    // The desktop entry's version wins over the metainfo's.
    let entry = detected(AppImageFixture::Entry);
    assert_eq!(entry.format, Format::AppImage);
    assert_eq!(entry.name, "Fixapp");
    assert_eq!(entry.app_id(), "fixapp");
    assert_eq!(entry.version, "2.1.0");
    assert_eq!(entry.kind, Kind::Gui);
    assert_eq!(entry.main, Some(0));
    assert_eq!(entry.candidates.len(), 1);
    assert_eq!(entry.candidates[0].elf.machine, this_machine());
    assert_eq!(entry.arch_mismatch, None);
    assert_eq!(entry.source_pattern, "entry.AppImage");
    let shipped = entry.desktop.as_ref().unwrap_or_else(|| panic!("an entry"));
    assert_eq!(shipped.rel, "fixapp.desktop");
    assert_eq!(shipped.entry.get("Exec"), Some("fixapp %U"));
    assert_eq!(entry.wm_class().as_deref(), Some("Fixapp"));
    // `.DirIcon` is a link to the PNG beside it, followed inside the image.
    assert_eq!(entry.icons.len(), 1);
    let icon = &entry.icons[0];
    assert_eq!(
        (icon.rel.as_str(), icon.ext.as_str(), icon.size),
        (".DirIcon", "png", Some((64, 64)))
    );
    assert_eq!(icon.data.as_deref(), Some(&tiny_png(64)));
    assert!(entry.icon_trusted);

    // Then the newest release of the metainfo, wherever it is listed.
    let metainfo = detected(AppImageFixture::Metainfo);
    assert_eq!(
        (metainfo.name.as_str(), metainfo.version.as_str()),
        ("Fixmeta", "1.4.0")
    );
    // Then the file's name, whose version is masked in the pattern.
    let named = detected(AppImageFixture::FileName);
    assert_eq!(
        (named.name.as_str(), named.version.as_str()),
        ("Fixname", "3.2.1")
    );
    assert_eq!(named.source_pattern, "Fixname-*-x86_64.AppImage");
    // With none of the three the version is left to the user.
    let none = detected(AppImageFixture::NoVersion);
    assert_eq!((none.name.as_str(), none.version.as_str()), ("Fixnone", ""));

    // The magic alone tells one, whatever its name.
    let magic = detected(AppImageFixture::MagicOnly);
    assert_eq!(magic.format, Format::AppImage);
    assert_eq!(
        (magic.name.as_str(), magic.version.as_str()),
        ("Fixmagic", "0.9.0")
    );
    // The architecture is the AppImage's own header's.
    assert_eq!(
        detected(AppImageFixture::WrongArch).arch_mismatch,
        Some(other_machine())
    );
    // A link that points out of the image is not followed: no icon.
    let links = detected(AppImageFixture::OutsideLinks);
    assert_eq!(links.name, "Fixlinks");
    assert!(links.icons.is_empty(), "{:?}", links.icons);
    assert!(!links.icon_trusted);

    // An image that cannot be read is no error: the file's name gives the
    // name and the version, and there is neither entry nor icon.
    let type1 = detected(AppImageFixture::Type1);
    assert_eq!(type1.format, Format::AppImage);
    assert_eq!((type1.name.as_str(), type1.version.as_str()), ("type1", ""));
    assert!(type1.desktop.is_none() && type1.icons.is_empty());
    let bytes = AppImageFixture::Entry
        .bytes()
        .unwrap_or_else(|e| panic!("{e}"));
    let cut = h.file(
        "Fixcut-0.4.2-aarch64.AppImage",
        &bytes[..bytes.len() - 200],
        0o644,
    );
    let cut = detect::detect(&cut).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        (cut.name.as_str(), cut.version.as_str()),
        ("Fixcut", "0.4.2")
    );
    assert!(cut.desktop.is_none() && cut.icons.is_empty());
    let bare = h.file(
        "fixbare-x86_64.AppImage",
        &tiny_appimage(this_machine(), 2, &[]),
        0o644,
    );
    let bare = detect::detect(&bare).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((bare.name.as_str(), bare.version.as_str()), ("fixbare", ""));
    // A file that is only named like one is no program.
    let text = h.file("notes.AppImage", b"text\n", 0o644);
    let error = detect::detect(&text).err().map(|e| text_of(&e));
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("not a program")),
        "{error:?}"
    );

    assert!(h.fake.ran().is_empty(), "nothing was run");
}

/// The image is read whatever it is compressed with among gzip, xz, zstd
/// and lz4, and stored without compression.
#[test]
fn an_image_is_read_through_every_compression_the_reader_has() {
    use backhand::compression::Compressor;
    let h = harness();
    let desktop = crate::fixtures::appimage_desktop("Fixzip", "fixzip", Some("7.0"));
    let png = tiny_png(48);
    for compressor in [
        Compressor::Gzip,
        Compressor::Xz,
        Compressor::Zstd,
        Compressor::Lz4,
        Compressor::Uncompressed,
    ] {
        let mut bytes = tiny_appimage(this_machine(), 2, &[]);
        bytes.extend(
            squashfs(
                &[
                    ("fixzip.desktop", Inside::File(desktop.as_bytes())),
                    ("fixzip.png", Inside::File(&png)),
                ],
                compressor,
            )
            .unwrap_or_else(|e| panic!("{compressor:?}: {e}")),
        );
        let path = h.file("fixzip.AppImage", &bytes, 0o644);
        let bundled = appimage::read_bundled(&path)
            .unwrap_or_else(|| panic!("{compressor:?}: the image is not read"));
        assert_eq!(
            bundled.desktop.as_ref().map(|(name, _)| name.as_str()),
            Some("fixzip.desktop"),
            "{compressor:?}"
        );
        // No `.DirIcon` here: the icon is the one the entry names.
        let icon = bundled
            .icon
            .unwrap_or_else(|| panic!("{compressor:?}: an icon"));
        assert_eq!((icon.name.as_str(), icon.ext), ("fixzip.png", "png"));
        assert_eq!(icon.data, png, "{compressor:?}");
        assert_eq!(bundled.release, None);
    }
}

/// Acceptance (issue 223): *Install as application…* on an AppImage reads
/// name, icon and version from the file and installs it on the archive
/// model. The version folder holds the AppImage itself under the fixed
/// name `<app-id>.AppImage` with the execute bit, `current` points at it,
/// the desktop entry is the AppImage's own with `Exec` through `current`
/// and its arguments, without `TryExec` and with the installed icon's
/// name, and no command is linked. The downloaded file is left as it was:
/// the same bytes, still without the execute bit. Nothing was run.
#[test]
#[allow(clippy::too_many_lines)]
fn an_appimage_installs_as_one_file_and_the_download_is_left_as_it_was() {
    let h = harness();
    let path = h.appimage(AppImageFixture::Entry);
    let install = h.action(INSTALL);
    if !supported() {
        assert_eq!(
            install.availability(&h.ctx_file(&path)),
            Availability::Hidden
        );
        return;
    }
    let layout = h.layout();
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(mode_of(&path), Some(0o644));
    assert_eq!(
        install.availability(&h.ctx_file(&path)),
        Availability::Enabled
    );
    assert_eq!(
        install.availability(&h.ctx_file(&h.appimage(AppImageFixture::MagicOnly))),
        Availability::Enabled,
        "by the magic alone"
    );

    h.install_file(&path, Vec::new());

    // The dialogue, in AppImage mode.
    let form = h.ui.last_form();
    assert_eq!(form.title, "Install entry.AppImage as application");
    assert_eq!(form.note.as_deref(), Some("AppImage: entry.AppImage"));
    assert!(matches!(
        field(&form, "name"),
        ext::FieldKind::Text { value, .. } if value == "Fixapp"
    ));
    assert!(matches!(
        field(&form, "version"),
        ext::FieldKind::Text { value, .. } if value == "2.1.0"
    ));
    assert!(
        form.fields.iter().all(|f| f.key != "exe"),
        "there is no executable to pick"
    );
    assert!(matches!(
        field(&form, "icon"),
        ext::FieldKind::Choice { options, selected: 1 } if options[1] == ".DirIcon · 64×64"
    ));
    let commands: Vec<&ext::Field> = form
        .fields
        .iter()
        .filter(|f| f.key.starts_with("cmd"))
        .collect();
    assert_eq!(commands.len(), 1, "{commands:?}");
    assert_eq!(
        commands[0].label,
        "Link command fixapp → current/fixapp.AppImage"
    );
    assert!(
        matches!(commands[0].kind, ext::FieldKind::Check { on: false }),
        "a command is offered and not ticked"
    );
    let rows = h.ui.last_rows();
    let row = |from: &str| {
        rows.iter()
            .find(|r| r.from == from)
            .map_or_else(|| panic!("no row {from} in {rows:?}"), |r| r.to.clone())
    };
    assert_eq!(
        row("AppImage"),
        "entry.AppImage, copied as it is; nothing in it is run"
    );
    assert_eq!(row("Mode"), "new install");
    assert_eq!(row("Main executable"), "current/fixapp.AppImage");
    assert!(row("Desktop entry").contains("the AppImage's own"));
    assert!(
        rows.iter()
            .all(|r| r.from != "Command" && r.problem.is_none())
    );

    // What was written.
    let version_dir = layout.version_dir("fixapp", "2.1.0");
    let installed = version_dir.join("fixapp.AppImage");
    assert_eq!(
        files_under(&version_dir),
        ["fixapp.AppImage"],
        "the one file and nothing unpacked"
    );
    assert_eq!(fs::read(&installed).ok().as_ref(), Some(&bytes));
    assert_eq!(mode_of(&installed), Some(0o755));
    assert_eq!(h.current("fixapp"), "2.1.0");
    let record = h.record("fixapp");
    assert_eq!(record.name, "Fixapp");
    assert_eq!(record.kind, Kind::Gui);
    assert_eq!(record.version, "2.1.0");
    assert_eq!(record.previous_version, None);
    assert_eq!(record.main_executable, "fixapp.AppImage");
    assert_eq!(record.source_archive, "entry.AppImage");
    assert_eq!(record.startup_wm_class.as_deref(), Some("Fixapp"));
    assert!(record.commands().is_empty(), "{:?}", record.commands());
    assert_eq!(
        record.external_files.len(),
        2,
        "{:?}",
        record.external_files
    );

    let desktop_path = layout.desktop_path("fixapp");
    let desktop = fs::read_to_string(&desktop_path).unwrap_or_else(|e| panic!("{e}"));
    let entry = desktop::DesktopEntry::parse(&desktop);
    let through = layout.current("fixapp").join("fixapp.AppImage");
    assert_eq!(
        entry.get("Exec"),
        Some(format!("\"{}\" %U", through.display()).as_str()),
        "{desktop}"
    );
    assert_eq!(entry.get("TryExec"), None, "{desktop}");
    assert_eq!(entry.get("Icon"), Some("fixapp"));
    assert_eq!(entry.get("Name"), Some("Fixapp"));
    assert_eq!(entry.get("Comment"), Some("A fixture"));
    assert_eq!(entry.get("StartupWMClass"), Some("Fixapp"));
    if let Some(valid) = desktop::validate(&desktop_path) {
        valid.unwrap_or_else(|e| panic!("desktop-file-validate: {e}\n{desktop}"));
    }
    assert_eq!(
        fs::read(layout.icon_path("fixapp", "64x64", "png")).ok(),
        Some(tiny_png(64))
    );
    assert!(fs::symlink_metadata(layout.bin_dir().join("fixapp")).is_err());

    // The row in Installed Apps.
    let row = h.row("Fixapp");
    assert_eq!(h.cell(&row, VERSION_COLUMN), "2.1.0");

    // The download: where it was and as it was.
    assert_eq!(fs::read(&path).ok(), Some(bytes));
    assert_eq!(mode_of(&path), Some(0o644));
    assert!(h.fake.ran().is_empty(), "nothing was run");
}

/// Acceptance (issue 223): an update from a file with another name leaves
/// the desktop entry untouched, because the path through `current` is the
/// same; one previous version is kept and Revert goes back to it; Repair
/// reads a lost icon out of the installed AppImage again; and Uninstall
/// removes everything the install created, the downloads staying where
/// they were.
#[test]
#[allow(clippy::too_many_lines)]
fn an_appimage_updates_reverts_repairs_and_uninstalls_like_an_archive() {
    let h = harness();
    if !supported() {
        return;
    }
    let layout = h.layout();
    let home_before = files_under(&h.home());
    let first = h.appimage(AppImageFixture::Entry);
    let second = h.appimage(AppImageFixture::UpdateV2);
    h.install_file(&first, Vec::new());
    let desktop_path = layout.desktop_path("fixapp");
    let desktop = fs::read(&desktop_path).unwrap_or_else(|e| panic!("{e}"));
    let written = fs::metadata(&desktop_path)
        .and_then(|m| m.modified())
        .unwrap_or_else(|e| panic!("{e}"));

    // The same id is offered as an update.
    h.install_file(&second, Vec::new());
    let form = h.ui.last_form();
    assert_eq!(form.title, "Fixapp 2.1.0 is installed — update to 2.2.0?");
    assert_eq!(h.current("fixapp"), "2.2.0");
    let record = h.record("fixapp");
    assert_eq!(record.version, "2.2.0");
    assert_eq!(record.previous_version.as_deref(), Some("2.1.0"));
    assert_eq!(record.source_archive, "update-v2.AppImage");
    assert_eq!(record.main_executable, "fixapp.AppImage");
    let app_dir = layout.app_dir("fixapp");
    assert_eq!(
        fs::read(app_dir.join("2.2.0/fixapp.AppImage")).ok(),
        fs::read(&second).ok()
    );
    assert!(
        app_dir.join("2.1.0/fixapp.AppImage").is_file(),
        "the previous version is kept"
    );
    assert_eq!(
        fs::read(&desktop_path).ok().as_ref(),
        Some(&desktop),
        "the desktop entry is not rewritten"
    );
    assert_eq!(
        fs::metadata(&desktop_path).and_then(|m| m.modified()).ok(),
        Some(written),
        "nor written again"
    );

    // Revert flips back; the entry still runs through current.
    h.ok(REVERT, h.ctx_view(vec![h.row("Fixapp")]));
    assert_eq!(h.current("fixapp"), "2.1.0");
    assert_eq!(fs::read(&desktop_path).ok().as_ref(), Some(&desktop));

    // A lost icon comes back out of the installed AppImage.
    let icon = layout.icon_path("fixapp", "64x64", "png");
    fs::remove_file(&icon).unwrap_or_else(|e| panic!("{e}"));
    h.ok(REPAIR, h.ctx_view(vec![h.row("Fixapp")]));
    assert_eq!(fs::read(&icon).ok(), Some(tiny_png(64)));

    // Uninstall: exactly what the install created.
    h.ok(UNINSTALL, h.ctx_view(vec![h.row("Fixapp")]));
    assert_eq!(files_under(&h.home()), home_before);
    assert!(first.is_file() && second.is_file(), "the downloads stay");
    assert_eq!(mode_of(&first), Some(0o644));
    assert!(h.fake.ran().is_empty(), "nothing was run");
}

/// Acceptance (issue 223): with no version in the desktop entry, the
/// metainfo or the file name, *Install* waits for one. The field is
/// empty and required, the summary names the problem, and nothing is
/// written until the user has typed a version; no version is made up.
#[test]
fn an_appimage_without_a_version_waits_for_the_user_to_type_one() {
    let h = harness();
    if !supported() {
        return;
    }
    let layout = h.layout();
    let path = h.appimage(AppImageFixture::NoVersion);
    h.ui.answer_form(Vec::new());
    let (outcome, _) = h.run(INSTALL, h.ctx_file(&path));
    let error = outcome.err().map(|e| text_of(&e));
    assert!(
        error.as_deref().is_some_and(|e| e.contains("version")),
        "{error:?}"
    );
    let form = h.ui.last_form();
    let version = form
        .fields
        .iter()
        .find(|f| f.key == "version")
        .unwrap_or_else(|| panic!("a version field"));
    assert!(matches!(
        &version.kind,
        ext::FieldKind::Text { value, .. } if value.is_empty()
    ));
    assert_eq!(
        version.required.as_deref(),
        Some("Couldn't read the version from the AppImage — please enter it.")
    );
    assert!(
        h.ui.last_rows().iter().any(|r| r.problem.is_some()),
        "the summary holds the install back"
    );
    assert!(!layout.app_dir("fixnone").exists(), "nothing was written");

    h.install_file(&path, vec![("version", Value::Text("0.1.0".to_owned()))]);
    assert_eq!(h.current("fixnone"), "0.1.0");
    assert!(
        layout
            .version_dir("fixnone", "0.1.0")
            .join("fixnone.AppImage")
            .is_file()
    );
}

/// Where the image cannot be read — an AppImage of type 1 — the dialogue
/// opens with a minimal entry from the file name and no icon, as for an
/// archive without one, and the install generates the desktop entry.
#[test]
fn an_appimage_whose_image_cannot_be_read_installs_with_a_minimal_entry() {
    let h = harness();
    if !supported() {
        return;
    }
    let layout = h.layout();
    let path = h.appimage(AppImageFixture::Type1);
    h.install_file(&path, vec![("version", Value::Text("1.0".to_owned()))]);
    let form = h.ui.last_form();
    assert!(matches!(
        field(&form, "name"),
        ext::FieldKind::Text { value, .. } if value == "type1"
    ));
    assert!(matches!(
        field(&form, "icon"),
        ext::FieldKind::Choice { options, selected: 0 } if options.len() == 1
    ));
    assert!(
        h.ui.last_rows()
            .iter()
            .any(|r| r.from == "Desktop entry" && r.to.contains("generated"))
    );
    let record = h.record("type1");
    assert_eq!(record.main_executable, "type1.AppImage");
    let desktop =
        fs::read_to_string(layout.desktop_path("type1")).unwrap_or_else(|e| panic!("{e}"));
    let entry = desktop::DesktopEntry::parse(&desktop);
    assert_eq!(entry.get("Name"), Some("type1"));
    assert_eq!(
        entry.get("Exec"),
        Some(
            format!(
                "\"{}\"",
                layout.current("type1").join("type1.AppImage").display()
            )
            .as_str()
        ),
        "{desktop}"
    );
    assert_eq!(entry.get("Icon"), None, "{desktop}");
    assert_eq!(
        record
            .external_files
            .iter()
            .filter(|f| f.kind == crate::record::FileKind::Icon)
            .count(),
        0
    );
}

/// The architecture check reads the AppImage's own ELF header: one built
/// for another machine is refused in the dialogue, before anything is
/// copied.
#[test]
fn an_appimage_of_another_machine_is_refused_by_the_dialogue() {
    let h = harness();
    if !supported() {
        return;
    }
    let path = h.appimage(AppImageFixture::WrongArch);
    h.ui.answer_form(Vec::new());
    let (outcome, _) = h.run(INSTALL, h.ctx_file(&path));
    let error = outcome.err().map(|e| text_of(&e));
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("this AppImage is built for")),
        "{error:?}"
    );
    assert!(!h.layout().app_dir("fixarch").exists());
}

/// `rick install <file>` and *Update from archive…* take an AppImage as
/// they take an archive, by its name or by its magic.
#[test]
fn the_command_line_and_update_from_archive_take_an_appimage() {
    let h = harness();
    if !supported() {
        return;
    }
    let command = h
        .registry
        .commands()
        .iter()
        .find(|c| c.name() == INSTALL_COMMAND)
        .cloned()
        .unwrap_or_else(|| panic!("the install command is registered"));
    let cwd = h.dir.path().to_path_buf();
    h.appimage(AppImageFixture::Entry);
    h.appimage(AppImageFixture::MagicOnly);
    for name in ["entry.AppImage", "magic-only"] {
        let outcome = command
            .run(
                CliCtx {
                    args: vec![name.to_owned()],
                    cwd: cwd.clone(),
                    pane: PaneId(1),
                    location: Location::new(
                        ProviderId::LOCAL,
                        ext::platform::paths::from_path(&cwd),
                    ),
                },
                &h.jobs,
                &h.ui,
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            matches!(outcome, CliOutcome::Outcome(ActionOutcome::Later(_))),
            "{name}: {outcome:?}"
        );
        h.settle();
    }

    // Update from archive…, on the installed app's row, with the path of
    // the next AppImage typed.
    h.install_file(&h.appimage(AppImageFixture::Entry), Vec::new());
    let next = h.appimage(AppImageFixture::UpdateV2);
    *h.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) =
        Some(next.to_string_lossy().into_owned());
    h.ui.answer_form(Vec::new());
    h.ok(UPDATE, h.ctx_view(vec![h.row("Fixapp")]));
    assert_eq!(h.current("fixapp"), "2.2.0");
    assert_eq!(
        h.record("fixapp").previous_version.as_deref(),
        Some("2.1.0")
    );
}

/// Whether an AppImage needs FUSE 2 is read from its runtime, the ELF at
/// the head of the file, and never by running it: the older runtime
/// carries the library's name, as the string it opens it by or among its
/// needed libraries, and the static one does not. What the image behind
/// the runtime holds changes nothing.
#[test]
fn the_need_of_fuse2_is_read_from_the_runtime() {
    use backhand::compression::Compressor;
    let h = harness();
    let needs = |path: &Path| appimage::needs_fuse2(path).unwrap_or_else(|e| panic!("{e}"));
    assert!(needs(&h.appimage(AppImageFixture::Fuse2)));
    assert!(
        detect::detect(&h.appimage(AppImageFixture::Fuse2))
            .unwrap_or_else(|e| panic!("{e}"))
            .needs_fuse2
    );
    // A runtime that lists it as a needed library.
    let listed = h.file(
        "fixlisted.AppImage",
        &tiny_appimage(this_machine(), 1, &["libz.so.1", "libfuse.so.2"]),
        0o644,
    );
    assert!(needs(&listed));

    // The fixtures on a runtime without the name are never held back.
    for fixture in [
        AppImageFixture::Entry,
        AppImageFixture::Type1,
        AppImageFixture::MagicOnly,
    ] {
        let path = h.appimage(fixture);
        assert!(!needs(&path), "{fixture:?}");
        let detected = detect::detect(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(!detected.needs_fuse2, "{fixture:?}");
    }
    // The name inside the image, stored as it is, is not the runtime's.
    let mut inside = tiny_appimage(this_machine(), 2, &[]);
    inside.extend(
        squashfs(
            &[("README", Inside::File(b"this app bundles libfuse.so.2\n"))],
            Compressor::Uncompressed,
        )
        .unwrap_or_else(|e| panic!("{e}")),
    );
    assert!(
        inside
            .windows(appimage::FUSE2.len())
            .any(|w| w == appimage::FUSE2.as_bytes()),
        "the name is in the file"
    );
    assert!(!needs(&h.file("fixinside.AppImage", &inside, 0o644)));
    // No program at all.
    assert!(!needs(&h.file("notes.AppImage", b"libfuse.so.2\n", 0o644)));
    assert!(appimage::needs_fuse2(&h.dir.path().join("gone")).is_err());

    // An archive is never asked.
    let archive =
        detect::detect(h.archive(Fixture::GuiWithDesktop)).unwrap_or_else(|e| panic!("{e}"));
    assert!(!archive.needs_fuse2);
}

/// The machine's libraries are the loader cache's names and the files of
/// the library folders; a test's stand-in has exactly the names it is
/// given.
#[test]
fn the_libraries_of_a_machine_are_looked_up_by_file_name() {
    let none = install::Libraries::of(&[]);
    assert!(!none.has(appimage::FUSE2));
    let some = install::Libraries::of(&["libfuse.so.2", "libz.so.1"]);
    assert!(some.has(appimage::FUSE2) && some.has("libz.so.1"));
    assert!(!some.has("libfuse.so.3"));
    // The system's are read without a panic wherever the tests run, a
    // machine without a loader cache included.
    let _ = install::Libraries::system().has(appimage::FUSE2);
}

/// Acceptance (issue 223): an AppImage on the older runtime, on a machine
/// without `libfuse.so.2`. *Run* stops with the warning and starts
/// nothing: *FUSE 2 is required to run this AppImage*, the install line
/// of each distribution and the link to the AppImage documentation. With
/// the library installed the same file runs, and an AppImage on a runtime
/// that does not need it runs with no warning on either machine.
#[test]
fn run_stops_with_the_fuse2_warning_where_the_library_is_missing() {
    if !supported() {
        return;
    }
    let h = harness_with_libraries(&[]);
    let old = h.appimage(AppImageFixture::Fuse2);
    let outcome = h.ok(RUN, h.ctx_file(&old));
    assert!(matches!(outcome, ActionOutcome::Done), "{outcome:?}");
    assert!(h.fake.ran().is_empty(), "nothing is started");
    let shown =
        h.ui.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
    let [(title, text)] = shown.as_slice() else {
        panic!("one warning: {shown:?}");
    };
    assert_eq!(title, "FUSE 2 is required to run this AppImage");
    for line in [
        "libfuse.so.2",
        "sudo apt install libfuse2\n",
        "Ubuntu 24.04 and later:   sudo apt install libfuse2t64",
        "sudo dnf install fuse-libs",
        "sudo pacman -S fuse2",
        "https://docs.appimage.org/user-guide/troubleshooting/fuse.html",
    ] {
        assert!(text.contains(line), "{line:?} is not in:\n{text}");
    }
    // The static runtime is never held back, FUSE 2 or not.
    let new = h.appimage(AppImageFixture::Entry);
    h.ok(RUN, h.ctx_file(&new));
    assert_eq!(
        h.fake.ran(),
        [(new, Vec::new(), h.dir.path().to_path_buf())]
    );
    assert_eq!(
        h.ui.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        1
    );

    // Where the library is installed, the older runtime runs.
    let h = harness_with_libraries(&["libfuse.so.2"]);
    let old = h.appimage(AppImageFixture::Fuse2);
    h.ok(RUN, h.ctx_file(&old));
    assert_eq!(
        h.fake.ran(),
        [(old, Vec::new(), h.dir.path().to_path_buf())]
    );
    assert!(
        h.ui.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
}

/// Acceptance (issue 223): *Install* works without FUSE 2 and carries the
/// warning in its summary, so nobody installs an app that then does not
/// start without being told why: as a row of the dialogue that holds
/// nothing back, and in the job's closing summary, where the library
/// check lists a missing library. With the library installed, and for an
/// AppImage that does not need it, there is no warning.
#[test]
fn an_install_without_fuse2_works_and_carries_the_warning_in_its_summary() {
    if !supported() {
        return;
    }
    let warning = |rows: &[ext::PreviewRow]| {
        rows.iter()
            .find(|r| r.from == "Warning")
            .map(|r| (r.to.clone(), r.problem.clone()))
    };
    let summary = |h: &Harness| {
        h.messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last()
            .cloned()
            .unwrap_or_default()
    };

    let h = harness_with_libraries(&[]);
    h.install_file(&h.appimage(AppImageFixture::Fuse2), Vec::new());
    let (text, problem) = warning(&h.ui.last_rows()).unwrap_or_else(|| panic!("a warning row"));
    assert_eq!(text, appimage::FUSE2_WARNING);
    assert_eq!(problem, None, "the warning holds nothing back");
    assert_eq!(h.current("fixfuse"), "1.0.0", "the install works");
    let said = summary(&h);
    assert!(said.starts_with("Fixfuse 1.0.0 installed."), "{said}");
    assert!(said.ends_with(appimage::FUSE2_WARNING), "{said}");
    for line in [
        "FUSE 2 is required to run this AppImage",
        "sudo apt install libfuse2 (libfuse2t64 on Ubuntu 24.04 and later)",
        "sudo dnf install fuse-libs",
        "sudo pacman -S fuse2",
        "https://docs.appimage.org/user-guide/troubleshooting/fuse.html",
    ] {
        assert!(said.contains(line), "{line:?} is not in: {said}");
    }
    assert!(
        !said.contains("This app needs libfuse.so.2"),
        "said once: {said}"
    );
    // An AppImage that does not need it, on the same machine.
    h.install_file(&h.appimage(AppImageFixture::Entry), Vec::new());
    assert_eq!(warning(&h.ui.last_rows()), None);
    assert!(!summary(&h).contains("FUSE"), "{}", summary(&h));
    assert!(h.fake.ran().is_empty(), "nothing was run");

    // With the library installed there is nothing to warn of.
    let h = harness_with_libraries(&["libfuse.so.2"]);
    h.install_file(&h.appimage(AppImageFixture::Fuse2), Vec::new());
    assert_eq!(warning(&h.ui.last_rows()), None);
    assert_eq!(summary(&h), "Fixfuse 1.0.0 installed.");
}

/// The AppImage fixtures are the ten of the spec's table, under its
/// names and in its order (specs/ext-installer.md "Test fixtures"): each
/// is the tiny ELF with the AppImage magic in its header, of type 2 with
/// a SquashFS image where the ELF ends but for `type1.AppImage`, built
/// anew with the same bytes every time, and written without the execute
/// bit. The two rows no other test asks of the fixture itself: the magic
/// alone offers both entries, and another machine's is refused by *Run*.
#[test]
fn the_appimage_fixtures_are_the_ones_of_the_specs_table() {
    let names: Vec<&str> = AppImageFixture::ALL.iter().map(|f| f.file_name()).collect();
    assert_eq!(
        names,
        [
            "entry.AppImage",
            "metainfo.AppImage",
            "Fixname-3.2.1-x86_64.AppImage",
            "no-version.AppImage",
            "type1.AppImage",
            "magic-only",
            "fuse2.AppImage",
            "wrong-arch.AppImage",
            "outside-links.AppImage",
            "update-v2.AppImage",
        ]
    );
    let h = harness();
    for fixture in AppImageFixture::ALL {
        let bytes = fixture
            .bytes()
            .unwrap_or_else(|e| panic!("{fixture:?}: {e}"));
        assert_eq!(
            fixture.bytes().ok().as_ref(),
            Some(&bytes),
            "{fixture:?} is the same every time"
        );
        let wanted = if fixture == AppImageFixture::Type1 {
            Type::One
        } else {
            Type::Two
        };
        assert_eq!(appimage::magic(&bytes), Some(wanted), "{fixture:?}");
        let header = elf::read_bytes(&bytes).unwrap_or_else(|| panic!("{fixture:?}: no ELF"));
        let end = usize::try_from(header.end).unwrap_or(usize::MAX);
        let image = bytes.get(end..end + 4);
        if wanted == Type::Two {
            assert_eq!(image, Some(&b"hsqs"[..]), "{fixture:?}: the image");
        } else {
            assert_ne!(image, Some(&b"hsqs"[..]), "{fixture:?}: no image");
        }
        let path = h.appimage(fixture);
        assert_eq!(fs::read(&path).ok(), Some(bytes), "{fixture:?}");
        if supported() {
            assert_eq!(mode_of(&path), Some(0o644), "{fixture:?}");
        }
    }
    if !supported() {
        return;
    }
    let magic_only = h.appimage(AppImageFixture::MagicOnly);
    for action in [RUN, INSTALL] {
        assert_eq!(
            h.action(action).availability(&h.ctx_file(&magic_only)),
            Availability::Enabled,
            "{}",
            action.0
        );
    }
    let other = h.appimage(AppImageFixture::WrongArch);
    let (outcome, _) = h.run(RUN, h.ctx_file(&other));
    let error = outcome.err().map(|e| text_of(&e));
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("this AppImage is built for")),
        "{error:?}"
    );
    assert!(h.fake.ran().is_empty(), "nothing is started");
}
