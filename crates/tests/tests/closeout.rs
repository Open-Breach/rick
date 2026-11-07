//! Acceptance (0.1 close-out, issue A): the side panel's Recent, Network and
//! Trash sections, the operations area, Tree view, inline rename and new
//! entries, the conflict dialogue, search in subdirectories and Open
//! containing folder, driven through the egui shell with `egui_kittest`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use ext::ActionId;
use model::{JobState, SelectionChange, ViewMode};
use platform::{ConfigDir, Fake, Trash as _};
use tests::local;
use ui_egui::keys::Intent;
use ui_egui::{EditKind, EguiHost, RickUi, icons};

/// A harness whose settings live in `config`, so a second one can start
/// from what the first one saved; `fake` is handed in so a test can prepare
/// it (volumes) before the app starts.
fn harness_with(
    start: &Path,
    config: &Path,
    fake: Arc<Fake>,
) -> (Harness<'static, RickUi>, Arc<Fake>) {
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(config.to_path_buf()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_step_dt(0.05)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    (harness, fake)
}

fn harness(start: &Path) -> (Harness<'static, RickUi>, Arc<Fake>) {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let config = fake.config_dir();
    harness_with(start, &config, fake)
}

/// Runs frames until the app has nothing in flight and a quiet moment
/// passed.
fn settle(harness: &mut Harness<'_, RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for _ in 0..3 {
            harness.step();
        }
        if !harness.state().app().is_busy() {
            std::thread::sleep(Duration::from_millis(50));
            harness.step();
            if !harness.state().app().is_busy() {
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "did not settle: {:?} jobs {:?}",
            harness.state().app(),
            harness.state().app().model().jobs
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Steps frames until `done` holds, within `limit`.
fn step_until(
    harness: &mut Harness<'_, RickUi>,
    limit: Duration,
    mut done: impl FnMut(&Harness<'_, RickUi>) -> bool,
) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if done(harness) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn command(harness: &mut Harness<'_, RickUi>, command: app::Command) {
    harness
        .state_mut()
        .app_mut()
        .handle(command, &mut Vec::new());
    settle(harness);
}

fn pane(harness: &Harness<'_, RickUi>) -> model::PaneId {
    harness.state().app().model().focused
}

fn tab(harness: &Harness<'_, RickUi>) -> (model::TabId, model::Tab) {
    let pane = pane(harness);
    harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map_or_else(|| panic!("no tab"), |(id, t)| (id, t.clone()))
}

fn location(harness: &Harness<'_, RickUi>) -> PathBuf {
    platform::paths::to_path(&tab(harness).1.location.path).unwrap_or_else(|e| panic!("{e}"))
}

fn row_names(harness: &Harness<'_, RickUi>) -> Vec<String> {
    tab(harness).1.rows().map(|e| e.name.clone()).collect()
}

fn selected_names(harness: &Harness<'_, RickUi>) -> Vec<String> {
    tab(harness).1.selected().map(|e| e.name.clone()).collect()
}

fn select(harness: &mut Harness<'_, RickUi>, name: &str) {
    let pane = pane(harness);
    let id = tab(harness).1.rows().find(|e| e.name == name).map_or_else(
        || panic!("no row {name} in {:?}", row_names(harness)),
        |e| e.id,
    );
    command(
        harness,
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
    );
}

/// The one node whose label contains every piece of `parts`.
fn node_with<'a>(harness: &'a Harness<'_, RickUi>, parts: &'a [&'a str]) -> egui_kittest::Node<'a> {
    let mut found: Vec<egui_kittest::Node<'a>> = harness
        .query_all_by_label_contains(parts[0])
        .filter(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| parts.iter().all(|p| l.contains(p)))
        })
        .collect();
    assert_eq!(found.len(), 1, "nodes with {parts:?}: {}", found.len());
    found.remove(0)
}

/// The one node labelled `label` inside the focused pane's file list, below
/// its toolbar.
fn node_in_pane<'a>(harness: &'a Harness<'_, RickUi>, label: &'a str) -> egui_kittest::Node<'a> {
    let pane = pane(harness);
    let (tab_id, _) = tab(harness);
    let rect = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("no pane rect"), |(_, r)| r);
    let toolbar_bottom = harness
        .state()
        .toolbar_row(tab_id)
        .map_or(rect.top(), |r| r.bottom());
    let mut found: Vec<egui_kittest::Node<'a>> = harness
        .query_all_by_label(label)
        .filter(|n| rect.contains(n.rect().center()) && n.rect().top() > toolbar_bottom)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "nodes labelled {label:?} in the pane: {}",
        found.len()
    );
    found.remove(0)
}

fn run(harness: &mut Harness<'_, RickUi>, action: &'static str) {
    let pane = pane(harness);
    harness
        .state_mut()
        .queue(Intent::Action(ActionId(action)), pane);
    harness.step();
    harness.step();
}

/// Types into the focused text field.
fn type_text(harness: &mut Harness<'_, RickUi>, text: &str) {
    harness.event(egui::Event::Text(text.to_owned()));
    harness.step();
}

/// A folder with three files and a `sub` folder holding two more.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::create_dir_all(dir.path().join("sub/deeper")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("sub/inner.txt"), "inner").unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("sub/deeper/leaf.txt"), "leaf").unwrap_or_else(|e| panic!("{e}"));
    dir
}

#[test]
fn recent_shows_a_touched_file_first_and_survives_a_restart() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    // The config directory is written at start; inside the listed folder it
    // would be the first recent entry itself.
    let config_dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let config = config_dir.path().join("config");
    let home = fake.home_dir();
    let touched = &["first.txt", "second.txt", "third.txt"][..tests::bench::runs().max(2)];
    let last = touched[touched.len() - 1];
    {
        let (mut harness, _) = harness_with(dir.path(), &config, fake.clone());
        harness.get_by_label_contains("Nothing changed recently");
        // Each touched file is the first row within the budget; at least two
        // touches, so the order after a restart means something.
        let mut samples = Vec::new();
        for (i, name) in touched.iter().enumerate() {
            if i > 0 {
                std::thread::sleep(Duration::from_millis(30));
            }
            fs::write(home.join(name), name).unwrap_or_else(|e| panic!("{e}"));
            let started = Instant::now();
            assert!(
                step_until(&mut harness, Duration::from_millis(500), |h| {
                    h.state()
                        .app()
                        .model()
                        .recent
                        .entries()
                        .first()
                        .is_some_and(|e| e.name == *name)
                }),
                "{name} is the first row within 500 ms"
            );
            samples.push(started.elapsed());
            harness.get_by_label_contains(name);
        }
        tests::bench::record("recent", &samples[..tests::bench::runs()]);
        settle(&mut harness);
        let file = config.join(app::RECENT_FILE);
        assert!(step_until(&mut harness, Duration::from_secs(3), |_| {
            fs::read_to_string(&file).is_ok_and(|t| t.contains(last))
        }));

        // A click selects the file in the focused pane; a double-click opens.
        harness.get_by_label_contains(last).click();
        harness.step();
        settle(&mut harness);
        assert_eq!(location(&harness), home);
        assert_eq!(selected_names(&harness), [last]);
    }
    let (harness, _) = harness_with(dir.path(), &config, fake);
    let rows: Vec<String> = harness
        .state()
        .app()
        .model()
        .recent
        .entries()
        .iter()
        .map(|e| e.name.clone())
        .collect();
    let newest_first: Vec<&str> = touched.iter().rev().copied().collect();
    assert_eq!(rows, newest_first, "the same rows after a restart");
    harness.get_by_label_contains(last);
}

#[test]
fn network_lists_a_registered_volume_and_clicking_navigates_there() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mount = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(mount.path().join("on-the-stick.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    fake.add_volume(platform::Volume {
        name: "STICK".into(),
        mount_point: mount.path().to_path_buf(),
        kind: platform::VolumeKind::Removable,
        total: Some(16 << 30),
        free: Some(4 << 30),
    });
    let config = fake.config_dir();
    let (mut harness, fake) = harness_with(dir.path(), &config, fake);
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().volumes.len() == 1
    }));
    let row = harness.get_by_label_contains("STICK");
    row.click();
    harness.step();
    settle(&mut harness);
    assert_eq!(location(&harness), mount.path());
    harness.get_by_label("on-the-stick.txt");
    harness.get_by_label_contains("4.0 GiB free of 16.0 GiB");

    // The volume goes away: the section follows on the next poll.
    fake.remove_volume(mount.path());
    command(&mut harness, app::Command::RefreshVolumes);
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().volumes.is_empty()
    }));
    harness.get_by_label_contains("No removable media");
}

#[test]
fn trash_lists_restores_undoes_and_empties() {
    let dir = tree();
    let (mut harness, fake) = harness(dir.path());
    harness.get_by_label_contains("The trash is empty");
    select(&mut harness, "b.txt");
    run(&mut harness, "core.trash");
    settle(&mut harness);
    assert!(!dir.path().join("b.txt").exists());
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().trash.items.len() == 1
    }));
    harness.step();
    // The row names the file and its original folder.
    let folder = dir.path().display().to_string();
    let parts = ["b.txt", folder.as_str()];
    let row = node_with(&harness, &parts);

    // Select the row; Enter restores (Restore trash items on the keyboard).
    row.click();
    harness.step();
    assert_eq!(
        harness.state().trash_selection(),
        [fake
            .list()
            .ok()
            .and_then(|l| l.first().map(|i| i.trashed.clone()))
            .unwrap_or_else(|| panic!("trashed"))]
    );
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(&mut harness);
    assert!(dir.path().join("b.txt").exists(), "restored");
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().trash.items.is_empty()
    }));
    assert_eq!(
        harness.state().app().undo_label().as_deref(),
        Some("Restore b.txt")
    );

    // Undo trashes it again.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.step();
    settle(&mut harness);
    assert!(!dir.path().join("b.txt").exists());
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().trash.items.len() == 1
    }));

    // Empty trash asks first, then leaves the section empty.
    harness.step();
    harness.get_by_label_contains("Empty trash").click();
    harness.step();
    harness.step();
    assert!(
        matches!(harness.state().asking(), Some(ui_egui::Ask::Confirm { title, .. }) if title == "Empty trash"),
        "{:?}",
        harness.state().asking()
    );
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(&mut harness);
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().trash.items.is_empty()
    }));
    assert_eq!(fake.list().map(|l| l.len()).ok(), Some(0));
    harness.step();
    harness.get_by_label_contains("The trash is empty");
}

#[test]
fn a_finished_job_leaves_the_operations_area_after_five_seconds() {
    let dir = tree();
    let (mut harness, _) = harness(dir.path());
    select(&mut harness, "a.txt");
    run(&mut harness, "core.copy");
    let pane = pane(&harness);
    command(
        &mut harness,
        app::Command::Navigate {
            pane,
            to: local(&dir.path().join("sub")),
        },
    );
    run(&mut harness, "core.paste");
    settle(&mut harness);
    assert!(dir.path().join("sub/a.txt").exists());
    let jobs = &harness.state().app().model().jobs;
    assert_eq!(jobs.len(), 1);
    assert!(jobs.values().all(|j| j.state == JobState::Done));
    harness.get_by_label_contains("Copy a.txt");
    let started = Instant::now();
    assert!(
        step_until(&mut harness, Duration::from_secs(8), |h| {
            h.state().app().model().jobs.is_empty()
        }),
        "the row is dismissed"
    );
    let gone_after = started.elapsed();
    assert!(
        gone_after >= Duration::from_secs(4) && gone_after < Duration::from_secs(7),
        "the row stayed {gone_after:?}"
    );
    harness.step();
    assert!(harness.query_by_label_contains("Copy a.txt").is_none());
}

/// A 1 GiB file with 4 MiB of data at each end and a hole between.
fn sparse_gib(path: &Path) {
    use std::io::{Seek, SeekFrom, Write};
    let mut file = fs::File::create(path).unwrap_or_else(|e| panic!("{e}"));
    file.write_all(&vec![0xABu8; 4 << 20])
        .unwrap_or_else(|e| panic!("{e}"));
    file.seek(SeekFrom::Start((1 << 30) - (4 << 20)))
        .unwrap_or_else(|e| panic!("{e}"));
    file.write_all(&vec![0xCDu8; 4 << 20])
        .unwrap_or_else(|e| panic!("{e}"));
    file.set_len(1 << 30).unwrap_or_else(|e| panic!("{e}"));
}

/// A 1 GiB sparse copy shows its row with progress, pause holds it, cancel
/// ends it within one batch.
#[test]
#[ignore = "writes a 1 GiB sparse file; run with --ignored"]
fn a_1gib_sparse_copy_shows_progress_pauses_and_cancels() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    sparse_gib(&dir.path().join("sparse.bin"));
    // The target folders exist before the window opens, so the listed folder
    // does not change under the test.
    for run_index in 0..tests::bench::runs() {
        fs::create_dir(dir.path().join(format!("dst{run_index}")))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let (mut harness, _) = harness(dir.path());
    let pane = pane(&harness);
    let running = |h: &Harness<'_, RickUi>| {
        h.state()
            .app()
            .model()
            .jobs
            .iter()
            .find(|(_, j)| matches!(j.state, JobState::Running(_)))
            .map(|(id, _)| *id)
    };
    let row_shown = |h: &Harness<'_, RickUi>| {
        assert!(
            h.query_all_by_label_contains("Copy sparse.bin").count() >= 1,
            "the copy's row is in the operations area"
        );
    };
    let mut samples = Vec::new();
    // One copy per run, each into a folder of its own.
    for run_index in 0..tests::bench::runs() {
        let dst = dir.path().join(format!("dst{run_index}"));
        if run_index > 0 {
            command(
                &mut harness,
                app::Command::Navigate {
                    pane,
                    to: local(dir.path()),
                },
            );
        }
        select(&mut harness, "sparse.bin");
        run(&mut harness, "core.copy");
        command(
            &mut harness,
            app::Command::Navigate {
                pane,
                to: local(&dst),
            },
        );
        run(&mut harness, "core.paste");
        let Some(id) = running(&harness) else {
            // The fast path finished within a frame; the row is still there
            // and there is nothing left to cancel.
            row_shown(&harness);
            continue;
        };
        row_shown(&harness);
        assert!(harness.state().app().model().jobs[&id].pausable);
        // Pause holds the job at its next gate; a single file through the
        // fast path may finish before one comes, so the flag is checked only
        // while it still runs.
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::PauseJob(id), &mut Vec::new());
        harness.step();
        let info = harness.state().app().model().jobs[&id].clone();
        if matches!(info.state, JobState::Running(_)) {
            assert!(info.paused, "{info:?}");
            harness.get_by_label(icons::PLAY);
        }
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::ResumeJob(id), &mut Vec::new());
        harness.step();
        let cancelled_at = Instant::now();
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::CancelJob(id), &mut Vec::new());
        assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
            !matches!(
                h.state().app().model().jobs[&id].state,
                JobState::Running(_)
            )
        }));
        let cancel_took = cancelled_at.elapsed();
        let state = harness.state().app().model().jobs[&id].state.clone();
        println!("cancel took {cancel_took:?}; the job ended {state:?}");
        assert!(
            matches!(state, JobState::Cancelled | JobState::Done),
            "{state:?}"
        );
        samples.push(cancel_took);
        settle(&mut harness);
    }
    if !samples.is_empty() {
        tests::bench::record("cancel", &samples);
    }
}

#[test]
fn tree_view_expands_folders_in_place_and_keeps_them_across_a_refresh() {
    let dir = tree();
    let (mut harness, _) = harness(dir.path());
    let pane = pane(&harness);
    command(
        &mut harness,
        app::Command::SetView {
            pane,
            mode: ViewMode::Tree,
        },
    );
    assert_eq!(row_names(&harness), ["sub", "a.txt", "b.txt", "c.txt"]);
    // The chevron expands; the children come indented under the folder.
    node_in_pane(&harness, icons::TREE_CLOSED).click();
    harness.step();
    settle(&mut harness);
    assert_eq!(
        row_names(&harness),
        ["sub", "deeper", "inner.txt", "a.txt", "b.txt", "c.txt"]
    );
    let (_, tab) = tab(&harness);
    assert_eq!(tab.depth(1), 1);
    assert_eq!(tab.depth(3), 0);
    harness.get_by_label("inner.txt");
    // A refresh keeps the expanded set.
    harness.key_press(egui::Key::F5);
    harness.step();
    settle(&mut harness);
    assert_eq!(row_names(&harness).len(), 6);
    // Collapse through the open chevron; the children go, the set shrinks.
    node_in_pane(&harness, icons::TREE_OPEN).click();
    harness.step();
    settle(&mut harness);
    assert_eq!(row_names(&harness), ["sub", "a.txt", "b.txt", "c.txt"]);
    assert_eq!(tab_expanded(&harness).len(), 0, "nothing stays expanded");
}

fn tab_expanded(harness: &Harness<'_, RickUi>) -> Vec<types::EntryId> {
    tab(harness).1.tree.expanded.iter().copied().collect()
}

/// Tree view on the 200k fixture: expanding a folder lists its children
/// lazily, the first screen within the budget (50 ms on Linux; BENCHMARKS.md
/// for the others).
#[test]
#[ignore = "lists 200 000 entries; run with --ignored in release"]
fn tree_view_on_the_200k_fixture_expands_within_the_budget() {
    let fixture = tests::fixture::generate(200_000, 7).unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _) = harness(fixture.root());
    let pane = pane(&harness);
    command(
        &mut harness,
        app::Command::SetView {
            pane,
            mode: ViewMode::Tree,
        },
    );
    // One folder per run, each expanded for the first time.
    let folders: Vec<types::EntryId> = tab(&harness)
        .1
        .rows()
        .filter(|e| e.kind == types::EntryKind::Dir)
        .take(tests::bench::runs())
        .map(|e| e.id)
        .collect();
    assert_eq!(
        folders.len(),
        tests::bench::runs(),
        "the fixture has folders"
    );
    let mut samples = Vec::new();
    for folder in &folders {
        let before = tab(&harness).1.row_count();
        let started = Instant::now();
        harness.state_mut().app_mut().handle(
            app::Command::TreeExpand { pane, id: *folder },
            &mut Vec::new(),
        );
        let mut first_screen = None;
        while first_screen.is_none() {
            harness.step();
            if tab(&harness).1.row_count() > before {
                first_screen = Some(started.elapsed());
            }
            assert!(started.elapsed() < Duration::from_secs(10), "no children");
        }
        settle(&mut harness);
        samples.push(first_screen.unwrap_or_default());
    }
    tests::bench::record("tree-first", &samples);
    let folder = folders[0];
    assert!(tab(&harness).1.tree.expanded.contains(&folder));
    harness.key_press(egui::Key::F5);
    harness.step();
    settle(&mut harness);
    assert!(
        tab(&harness).1.tree.expanded.contains(&folder),
        "kept across refresh"
    );
}

// One story: rename, refuse, rename, new folder, new file.
#[allow(clippy::too_many_lines)]
#[test]
fn f2_edits_the_name_in_place_and_new_folder_inserts_an_editable_row() {
    let dir = tree();
    let (mut harness, _) = harness(dir.path());
    let (tab_id, _) = tab(&harness);
    select(&mut harness, "a.txt");
    harness.key_press(egui::Key::F2);
    harness.step();
    harness.step();
    let edit = harness
        .state()
        .editing(tab_id)
        .cloned()
        .unwrap_or_else(|| panic!("no inline edit"));
    let selected = tab(&harness)
        .1
        .selected()
        .next()
        .map_or_else(|| panic!("selection"), |e| e.id);
    assert_eq!(edit.kind, EditKind::Rename(selected));
    assert_eq!(edit.text, "a.txt");
    // The base name is selected, the extension left alone.
    let state = egui::TextEdit::load_state(&harness.ctx, ui_egui::inline::edit_id(edit.kind))
        .unwrap_or_else(|| panic!("edit state"));
    let range = state
        .cursor
        .char_range()
        .unwrap_or_else(|| panic!("selection"));
    let (a, b) = (range.primary.index.0, range.secondary.index.0);
    assert_eq!((a.min(b), a.max(b)), (0, 1), "`a` selected, `.txt` not");

    // Escape leaves the name.
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert!(harness.state().editing(tab_id).is_none());
    assert!(dir.path().join("a.txt").exists());

    // Typing an existing name is refused inline: the row shakes red.
    harness.key_press(egui::Key::F2);
    harness.step();
    harness.step();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.step();
    type_text(&mut harness, "b.txt");
    harness.key_press(egui::Key::Enter);
    harness.step();
    let edit = harness
        .state()
        .editing(tab_id)
        .cloned()
        .unwrap_or_else(|| panic!("the edit stays open"));
    assert_eq!(edit.text, "b.txt");
    assert!(edit.is_shaking(harness.ctx.input(|i| i.time)), "refused");
    harness.step();
    assert!(dir.path().join("a.txt").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("b.txt")).ok(),
        Some("b.txt".into())
    );

    // A free name commits on Enter.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.step();
    type_text(&mut harness, "renamed.md");
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(&mut harness);
    assert!(harness.state().editing(tab_id).is_none());
    assert!(dir.path().join("renamed.md").exists());
    assert!(!dir.path().join("a.txt").exists());
    assert!(
        step_until(&mut harness, Duration::from_secs(3), |h| {
            selected_names(h) == ["renamed.md"]
        }),
        "the renamed entry is selected"
    );

    // New folder: an editable row at the top, committed the same way.
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::N,
    );
    harness.step();
    harness.step();
    let edit = harness
        .state()
        .editing(tab_id)
        .cloned()
        .unwrap_or_else(|| panic!("no new-folder edit"));
    assert_eq!(edit.kind, EditKind::NewFolder);
    assert_eq!(edit.text, "New folder");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.step();
    type_text(&mut harness, "made here");
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(&mut harness);
    assert!(dir.path().join("made here").is_dir());
    assert!(step_until(&mut harness, Duration::from_secs(3), |h| {
        selected_names(h) == ["made here"]
    }));

    // New file too; it has no default shortcut since Mod+N opens a new
    // window (QUESTIONS.md 66).
    run(&mut harness, "core.new_file");
    assert_eq!(
        harness.state().editing(tab_id).map(|e| e.kind),
        Some(EditKind::NewFile)
    );
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(&mut harness);
    assert!(dir.path().join("New file.txt").is_file());
}

/// A long name in the side panel is truncated to the panel: pinning a folder
/// with a 90-character name leaves the panel, and so the pane, where they
/// were.
#[test]
fn the_side_panel_keeps_its_width_under_a_long_name() {
    let dir = tree();
    let long = "a".repeat(90);
    fs::create_dir(dir.path().join(&long)).unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _) = harness(dir.path());
    let pane = pane(&harness);
    let pane_left = |h: &Harness<'_, RickUi>| {
        h.state()
            .pane_rects()
            .into_iter()
            .find(|(p, _)| *p == pane)
            .map_or_else(|| panic!("no pane rect"), |(_, r)| r.left())
    };
    let before = pane_left(&harness);
    command(
        &mut harness,
        app::Command::Settings(model::SettingsChange::Pin {
            items: vec![local(&dir.path().join(&long))],
            at: None,
        }),
    );
    harness.step();
    harness.step();
    // The pinned row and the pane's row both show the name.
    assert_eq!(harness.query_all_by_label_contains(&long[..20]).count(), 2);
    let after = pane_left(&harness);
    assert!(
        (after - before).abs() < 1.0,
        "the pane moved from {before} to {after}: the panel grew to fit the name"
    );
}

#[test]
fn pasting_over_an_existing_name_opens_the_dialogue_and_cancel_touches_nothing() {
    let dir = tree();
    fs::write(dir.path().join("sub/a.txt"), "old").unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _) = harness(dir.path());
    select(&mut harness, "a.txt");
    run(&mut harness, "core.copy");
    let pane = pane(&harness);
    command(
        &mut harness,
        app::Command::Navigate {
            pane,
            to: local(&dir.path().join("sub")),
        },
    );
    run(&mut harness, "core.paste");
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        !h.state().app().model().conflicts.is_empty()
    }));
    harness.step();
    harness.step();
    harness.get_by_label_contains("a.txt already exists");
    harness.get_by_label("Cancel").click();
    harness.step();
    settle(&mut harness);
    assert_eq!(
        harness.state().app().model().conflicts.len(),
        0,
        "no conflict is pending"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("sub/a.txt")).ok(),
        Some("old".into())
    );
    let states: Vec<JobState> = harness
        .state()
        .app()
        .model()
        .jobs
        .values()
        .map(|j| j.state.clone())
        .collect();
    assert_eq!(states, [JobState::Cancelled], "the job ends Cancelled");

    // Skip leaves the existing file; the dialogue proposes `a (2).txt`.
    run(&mut harness, "core.paste");
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        !h.state().app().model().conflicts.is_empty()
    }));
    assert_eq!(
        harness.state().app().model().conflicts[0].proposed,
        "a (2).txt"
    );
    harness.step();
    harness.step();
    harness.get_by_label("Skip").click();
    harness.step();
    settle(&mut harness);
    assert_eq!(
        fs::read_to_string(dir.path().join("sub/a.txt")).ok(),
        Some("old".into())
    );
    assert!(!dir.path().join("sub/a (2).txt").exists());
}

/// Twenty nested levels with one `foo` each, plus a wide level of `wide`
/// matches so the walk outlives a frame and a second text can cancel it.
fn search_tree(wide: usize) -> tempfile::TempDir {
    let dir = tree();
    let mut deep = dir.path().join("sub");
    for level in 0..20 {
        deep.push(format!("l{level}"));
        fs::create_dir_all(&deep).unwrap_or_else(|e| panic!("{e}"));
        fs::write(deep.join(format!("foo{level}.txt")), "x").unwrap_or_else(|e| panic!("{e}"));
    }
    let wide_dir = dir.path().join("sub/wide");
    fs::create_dir_all(&wide_dir).unwrap_or_else(|e| panic!("{e}"));
    for i in 0..wide {
        fs::write(wide_dir.join(format!("food-{i:05}.txt")), "x").unwrap_or_else(|e| panic!("{e}"));
    }
    dir
}

/// Turns the subtree toggle on with Mod+Shift+F, types `foo` and returns
/// how long the first matches took to show.
fn search_foo(harness: &mut Harness<'_, RickUi>) -> Duration {
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::F,
    );
    harness.step();
    harness.step();
    assert!(tab(harness).1.subtree);
    let started = Instant::now();
    type_text(harness, "foo");
    assert!(
        step_until(harness, Duration::from_secs(10), |h| !row_names(h)
            .is_empty()),
        "matches stream in"
    );
    started.elapsed()
}

/// The states of the search jobs in the job table.
fn search_states(harness: &Harness<'_, RickUi>) -> Vec<JobState> {
    harness
        .state()
        .app()
        .model()
        .jobs
        .values()
        .filter(|j| j.name.starts_with("Search"))
        .map(|j| j.state.clone())
        .collect()
}

/// A new text cancels the walk still running for the old one: on the 200k
/// fixture the walk outlives the second text by far, so the first job ends
/// Cancelled and the second runs to Done.
#[test]
#[ignore = "lists 200 000 entries; run with --ignored in release"]
fn a_new_text_cancels_the_running_walk_on_the_200k_fixture() {
    let fixture = tests::fixture::generate(200_000, 7).unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _) = harness(fixture.root());
    let pane = pane(&harness);
    command(&mut harness, app::Command::Subtree { pane, on: true });
    for text in ["f0", "f00"] {
        harness.state_mut().app_mut().handle(
            app::Command::Filter {
                pane,
                text: text.to_owned(),
            },
            &mut Vec::new(),
        );
    }
    settle(&mut harness);
    let states = search_states(&harness);
    assert_eq!(
        states,
        [JobState::Cancelled, JobState::Done],
        "the first walk is cancelled, the second runs through"
    );
}

/// Typing `foo` with the subtree toggle on streams matches from 20 nested
/// levels within the budget for the first batch (100 ms on Linux;
/// BENCHMARKS.md for the others). Timed in release on CI, as the other
/// budgets are.
#[test]
#[ignore = "timing; run with --ignored in release"]
fn the_first_search_batch_arrives_within_100_ms() {
    let dir = search_tree(12_000);
    let mut samples = Vec::new();
    // A fresh window per run, so every run's walk starts from nothing.
    for _ in 0..tests::bench::runs() {
        let (mut harness, _) = harness(dir.path());
        samples.push(search_foo(&mut harness));
        settle(&mut harness);
        assert_eq!(row_names(&harness).len(), 12_020);
    }
    tests::bench::record("search-first", &samples);
}

#[test]
fn search_in_subdirectories_streams_matches_and_a_new_text_supersedes_the_walk() {
    let dir = search_tree(12_000);
    let (mut harness, _) = harness(dir.path());
    let (tab_id, _) = tab(&harness);
    let first_batch = search_foo(&mut harness);
    println!("first search batch after {first_batch:?}");
    settle(&mut harness);
    assert_eq!(row_names(&harness).len(), 12_020);
    // Rows show their relative path, in the platform's separator.
    let deep = format!("l19{}foo19.txt", std::path::MAIN_SEPARATOR);
    assert!(
        tab(&harness)
            .1
            .rows()
            .any(|e| e.path.to_string_lossy().ends_with(&deep))
    );
    let _ = tab_id;

    // A changed text supersedes the walk and starts another: the two texts
    // go straight to the app, as a typist faster than a frame would send
    // them. Whether the first walk is cancelled or had already finished
    // depends on the machine; that it is cancelled while running is shown
    // on the 200k fixture below. Either way only the last text's rows show.
    let pane = pane(&harness);
    for text in ["foo19", "foo1"] {
        harness.state_mut().app_mut().handle(
            app::Command::Filter {
                pane,
                text: text.to_owned(),
            },
            &mut Vec::new(),
        );
    }
    settle(&mut harness);
    let states = search_states(&harness);
    assert_eq!(states.len(), 3, "{states:?}");
    assert!(
        states
            .iter()
            .all(|s| matches!(s, JobState::Done | JobState::Cancelled)),
        "{states:?}"
    );
    assert!(row_names(&harness).iter().all(|n| n.starts_with("foo1")));
    assert_eq!(row_names(&harness).len(), 11, "foo1 and foo10 to foo19");

    // The toggle off ends the search; the folder's own entries are back.
    harness.get_by_label(icons::SUBTREE).click();
    harness.step();
    settle(&mut harness);
    assert!(!tab(&harness).1.subtree);
}

#[test]
fn open_containing_folder_from_a_flattened_view_selects_the_item() {
    let dir = tree();
    let (mut harness, _) = harness(dir.path());
    let pane = pane(&harness);
    command(&mut harness, app::Command::Flatten { pane, on: true });
    let names = row_names(&harness);
    assert!(names.contains(&"leaf.txt".to_owned()), "{names:?}");
    let sep = std::path::MAIN_SEPARATOR;
    harness.get_by_label(&format!("sub{sep}deeper{sep}leaf.txt"));
    select(&mut harness, "leaf.txt");
    run(&mut harness, "core.open_containing");
    settle(&mut harness);
    assert!(step_until(&mut harness, Duration::from_secs(3), |h| {
        selected_names(h) == ["leaf.txt"]
    }));
    assert_eq!(location(&harness), dir.path().join("sub/deeper"));
    harness.get_by_label("leaf.txt");

    // In a new tab, from the context menu's action.
    command(
        &mut harness,
        app::Command::Navigate {
            pane,
            to: local(dir.path()),
        },
    );
    command(&mut harness, app::Command::Flatten { pane, on: true });
    select(&mut harness, "inner.txt");
    run(&mut harness, "core.open_containing_tab");
    settle(&mut harness);
    assert_eq!(location(&harness), dir.path().join("sub"));
    assert!(step_until(&mut harness, Duration::from_secs(3), |h| {
        selected_names(h) == ["inner.txt"]
    }));
    assert_eq!(
        harness
            .state()
            .app()
            .model()
            .pane(pane)
            .map(|p| p.tabs().len()),
        Some(2)
    );
}
