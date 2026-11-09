//! Acceptance (issue 60, side panel): rows `sidebar/item-height` tall in
//! both densities, the section headers named with their carets, the Trash
//! row's pill with the trash's size, the drive cards' heights, and the
//! operations drawer's summary that collapses and expands.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use model::{Density, SettingsChange};
use platform::{ConfigDir, Fake, Trash as _};
use tests::local;
use ui_egui::{EguiHost, RickUi};

fn harness(start: &Path, fake: &Arc<Fake>) -> Harness<'static, RickUi> {
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 700.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
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
        assert!(Instant::now() < deadline, "did not settle");
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
    while !done(harness) {
        if Instant::now() > deadline {
            return false;
        }
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

fn density(harness: &mut Harness<'_, RickUi>, density: Density) {
    harness.state_mut().app_mut().handle(
        app::Command::Settings(SettingsChange::Density(density)),
        &mut Vec::new(),
    );
    settle(harness);
}

/// The one widget whose accessible name is `text`, left of the pane.
fn panel_rect(harness: &Harness<'_, RickUi>, text: &str) -> egui::Rect {
    let pane = harness.state().app().model().focused;
    let tab = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map_or_else(|| panic!("no tab"), |(id, _)| id);
    let edge = harness.state().toolbar_row(tab).map_or(0.0, |r| r.left());
    let rects: Vec<egui::Rect> = harness
        .query_all_by_label(text)
        .map(|n| n.rect())
        .filter(|r| r.center().x < edge)
        .collect();
    assert_eq!(rects.len(), 1, "{text}: {rects:?}");
    rects[0]
}

/// A primary click at `at`.
fn click_at(harness: &mut Harness<'_, RickUi>, at: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.step();
}

fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("a.txt"), vec![b'x'; 1200]).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("b.txt"), vec![b'y'; 300]).unwrap_or_else(|e| panic!("{e}"));
    dir
}

fn volume(fake: &Fake, mount: &Path) {
    fake.add_volume(platform::Volume {
        name: "System".into(),
        mount_point: mount.to_path_buf(),
        kind: platform::VolumeKind::Fixed,
        fs_type: "ext4".into(),
        device: None,
        total: Some(100 << 30),
        free: Some(60 << 30),
    });
}

#[test]
fn rows_headers_and_drive_cards_take_the_densitys_sizes() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    volume(&fake, dir.path());
    let mut harness = harness(dir.path(), &fake);
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().volumes.len() == 1
    }));
    // A Places row is `sidebar/item-height` tall: 34 in Comfortable, 22 in
    // Compact.
    let home = panel_rect(&harness, "Home");
    assert!((home.height() - 34.0).abs() < 0.5, "{}", home.height());
    // The header is named after its section, the caret after what it does.
    panel_rect(&harness, "This Machine");
    panel_rect(&harness, "Fold This Machine");
    panel_rect(&harness, "Fold Network");
    // The drive card: 44 px with the name and the percentage.
    let card = panel_rect(&harness, "System · 40%");
    assert!((card.height() - 44.0).abs() < 0.5, "{}", card.height());
    density(&mut harness, Density::Compact);
    let home = panel_rect(&harness, "Home");
    assert!((home.height() - 22.0).abs() < 0.5, "{}", home.height());
    let card = panel_rect(&harness, "System · 40%");
    assert!((card.height() - 26.0).abs() < 0.5, "{}", card.height());
    // The caret folds the section: its name flips and the card goes.
    harness.get_by_label("Fold This Machine").click();
    harness.step();
    settle(&mut harness);
    panel_rect(&harness, "Unfold This Machine");
    assert!(harness.query_by_label_contains("System · 40%").is_none());
}

#[test]
fn the_trash_row_carries_the_trashs_size_in_a_pill() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    // An empty trash: the row alone.
    let harness = harness(dir.path(), &fake);
    panel_rect(&harness, "Trash");
    assert!(harness.query_by_label_contains("Trash ·").is_none());
    drop(harness);
    // Two files trashed, 1.5 KiB together: the pill says so.
    fake.trash(&dir.path().join("a.txt"))
        .unwrap_or_else(|e| panic!("{e}"));
    fake.trash(&dir.path().join("b.txt"))
        .unwrap_or_else(|e| panic!("{e}"));
    let harness = self::harness(dir.path(), &fake);
    assert_eq!(harness.state().app().model().trash.size, 1500);
    panel_rect(&harness, "Trash · 1.5 KiB");
}

#[test]
fn the_operations_summary_collapses_to_its_line_and_expands_again() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = harness(dir.path(), &fake);
    let (go_tx, go_rx) = runtime::watch::channel(false);
    let jobs = harness.state().app().jobs().clone();
    for name in ["Copy one.bin to raw", "Extract archive.tar.gz"] {
        let mut go = go_rx.clone();
        jobs.spawn_named(name, move |ctx| async move {
            let total = 2_000u64;
            let report = |done| {
                ctx.progress.report(types::Progress::Items {
                    done,
                    total: Some(total),
                });
            };
            for done in 0..total / 2 {
                report(done);
                tokio::task::yield_now().await;
            }
            while !*go.borrow_and_update() {
                tokio::select! {
                    changed = go.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    () = tokio::time::sleep(Duration::from_millis(5)) => report(total / 2),
                }
            }
            Ok::<(), types::Error>(())
        });
    }
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().jobs.len() == 2
            && h.query_all_by_label_contains("of 2000").count() == 2
    }));
    // The summary (the title row has a job chip of the same words, so the
    // panel's one), then collapsed: the blocks go, the summary stays.
    let summary = panel_rect(&harness, "2 running").center();
    click_at(&mut harness, summary);
    assert_eq!(harness.query_all_by_label_contains("of 2000").count(), 0);
    let summary = panel_rect(&harness, "2 running").center();
    click_at(&mut harness, summary);
    assert_eq!(harness.query_all_by_label_contains("of 2000").count(), 2);
    let _ = go_tx.send(true);
    assert!(step_until(&mut harness, Duration::from_secs(20), |h| {
        h.state().app().model().jobs.is_empty()
    }));
}
