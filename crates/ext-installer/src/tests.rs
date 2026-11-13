use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ext::platform::{ConfigDir, Fake};
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
    OPEN_FOLDER, REMOVE_STAGING, REPAIR, REVERT, UNINSTALL, UPDATE,
};
use crate::archive::{Format, ItemKind};
use crate::detect::{compare_versions, slug, split_version, strip_version};
use crate::fixtures::{Fixture, other_machine, this_machine, tiny_elf};
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
    fn show(&self, _: &str, _: &str) {}
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
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut registry = Registry::with_platform(fake.clone());
    registry
        .load(&InstallerExtension::new())
        .unwrap_or_else(|e| panic!("{e}"));
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
            if let JobEvent::Finished { result, .. } = event {
                results.push(result);
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
    assert_eq!(h.registry.actions().len(), 11);
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
    let form = actions::install_form(&detected, &scan, &layout, None, false);
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
    let form = actions::install_form(&detected, &scan, &layout, None, false);
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
    let unresolved = install::unresolved_libraries(&version_dir, &[version_dir.join("fixnodesk")]);
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
