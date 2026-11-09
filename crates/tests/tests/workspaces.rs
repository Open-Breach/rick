//! Acceptance (issue 60, workspace tabs): the title row holds one tab per
//! workspace, titled after the folder in its focused pane; the new-tab
//! button, a click, a middle click, Mod+Tab and a drag along the row open,
//! show, close, cycle and reorder them; a pane dropped on the row becomes
//! a workspace of its own; and every tab and close button carries a name
//! for a screen reader.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::{EguiHost, RickUi};

fn harness(start: &Path, size: egui::Vec2) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(size)
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

fn command(harness: &mut Harness<'_, RickUi>, command: app::Command) {
    harness
        .state_mut()
        .app_mut()
        .handle(command, &mut Vec::new());
    settle(harness);
}

/// The titles of the focused window's workspace tabs, in row order.
fn titles(harness: &Harness<'_, RickUi>) -> Vec<String> {
    let model = harness.state().app().model();
    model
        .workspaces(model.focused_window)
        .iter()
        .map(|ws| {
            model
                .active_tab(ws.focused)
                .and_then(|(_, t)| platform::paths::to_path(&t.location.path).ok())
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default()
        })
        .collect()
}

/// The shown workspace's position in the row.
fn shown(harness: &Harness<'_, RickUi>) -> usize {
    let model = harness.state().app().model();
    let w = model
        .window(model.focused_window)
        .unwrap_or_else(|| panic!("window"));
    w.position(w.active).unwrap_or_else(|| panic!("active"))
}

/// Presses `button` at `pos` and releases it.
fn click_at(harness: &mut Harness<'_, RickUi>, pos: egui::Pos2, button: egui::PointerButton) {
    harness.event(egui::Event::PointerMoved(pos));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.step();
}

/// Presses the primary button at `from`, moves to `to` in small steps so
/// a drag is recognised, releases, and lets the app settle.
fn drag(harness: &mut Harness<'_, RickUi>, from: egui::Pos2, to: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(from));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    for i in 1..=12u8 {
        let t = f32::from(i) / 12.0;
        let pos = from + (to - from) * t;
        harness.event(egui::Event::PointerMoved(pos));
        harness.step();
    }
    harness.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    harness.step();
    settle(harness);
}

/// The centre of the workspace tab labelled `title` in the title row: the
/// row's tabs are the buttons with that name above the panes.
fn tab_centre(harness: &Harness<'_, RickUi>, title: &str) -> egui::Pos2 {
    let row = harness
        .state()
        .tab_row()
        .unwrap_or_else(|| panic!("no tab row"));
    harness
        .query_all_by_label(title)
        .map(|n| n.rect())
        .find(|r| row.contains(r.center()))
        .unwrap_or_else(|| panic!("no tab {title} in the row {row:?}"))
        .center()
}

#[test]
fn the_title_row_opens_shows_closes_cycles_and_reorders_workspace_tabs() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let alpha = dir.path().join("alpha-folder");
    let beta = dir.path().join("beta-folder");
    fs::create_dir(&alpha).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(&beta).unwrap_or_else(|e| panic!("{e}"));
    fs::write(alpha.join("a.txt"), "a").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(&alpha, egui::vec2(1400.0, 800.0));

    // One tab, titled after the folder, with a close button named for a
    // screen reader; the new-tab button after it.
    assert_eq!(titles(&harness), ["alpha-folder"]);
    tab_centre(&harness, "alpha-folder");
    assert_eq!(harness.query_all_by_label("Close tab").count(), 1);

    // The new-tab button opens a workspace at the focused pane's folder,
    // right after the shown one, and shows it.
    harness.get_by_label("New tab").click();
    harness.step();
    settle(&mut harness);
    assert_eq!(titles(&harness), ["alpha-folder", "alpha-folder"]);
    assert_eq!(shown(&harness), 1);
    assert_eq!(harness.query_all_by_label("Close tab").count(), 2);

    // The title follows the folder of the workspace's focused pane.
    let pane = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Navigate {
            pane,
            to: local(&beta),
        },
    );
    assert_eq!(titles(&harness), ["alpha-folder", "beta-folder"]);

    // A click shows a tab; Mod+Tab cycles.
    let alpha_tab = tab_centre(&harness, "alpha-folder");
    click_at(&mut harness, alpha_tab, egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(shown(&harness), 0);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Tab);
    harness.step();
    settle(&mut harness);
    assert_eq!(shown(&harness), 1, "Mod+Tab shows the next tab");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Tab);
    harness.step();
    settle(&mut harness);
    assert_eq!(shown(&harness), 0, "and wraps around");

    // A third tab, dragged to the front of the row.
    harness.get_by_label("New tab").click();
    harness.step();
    settle(&mut harness);
    let gamma = dir.path().join("gamma-folder");
    fs::create_dir(&gamma).unwrap_or_else(|e| panic!("{e}"));
    let pane = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Navigate {
            pane,
            to: local(&gamma),
        },
    );
    assert_eq!(
        titles(&harness),
        ["alpha-folder", "gamma-folder", "beta-folder"],
        "a new tab opens right after the shown one"
    );
    let from = tab_centre(&harness, "gamma-folder");
    let first = tab_centre(&harness, "alpha-folder");
    drag(&mut harness, from, egui::pos2(first.x - 60.0, first.y));
    assert_eq!(
        titles(&harness),
        ["gamma-folder", "alpha-folder", "beta-folder"],
        "dragged to the front"
    );
    assert_eq!(shown(&harness), 0, "the dragged tab stays shown");

    // A middle click closes a tab; the neighbour shows.
    let alpha_tab = tab_centre(&harness, "alpha-folder");
    click_at(&mut harness, alpha_tab, egui::PointerButton::Middle);
    settle(&mut harness);
    assert_eq!(titles(&harness), ["gamma-folder", "beta-folder"]);
    // The close button of the shown tab closes it too.
    let shown_before = shown(&harness);
    let row = harness
        .state()
        .tab_row()
        .unwrap_or_else(|| panic!("no tab row"));
    let close = harness
        .query_all_by_label("Close tab")
        .map(|n| n.rect())
        .filter(|r| row.contains(r.center()))
        .nth(shown_before)
        .unwrap_or_else(|| panic!("no close button"));
    click_at(&mut harness, close.center(), egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(titles(&harness), ["beta-folder"]);
    // Mod+W on the last tab of the last window leaves it.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
    harness.step();
    settle(&mut harness);
    assert_eq!(titles(&harness), ["beta-folder"]);
}

#[test]
fn a_pane_dropped_on_the_tab_row_becomes_a_workspace_of_its_own() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let left = dir.path().join("left-pane");
    let right = dir.path().join("right-pane");
    fs::create_dir(&left).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(&right).unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(&left, egui::vec2(1400.0, 800.0));
    let pane_a = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Split {
            pane: pane_a,
            dir: model::SplitDir::Right,
        },
    );
    let pane_b = harness.state().app().model().focused;
    assert_ne!(pane_b, pane_a);
    command(
        &mut harness,
        app::Command::Navigate {
            pane: pane_b,
            to: local(&right),
        },
    );
    assert_eq!(titles(&harness), ["right-pane"], "one workspace, two panes");

    // Pane B's tab, dragged from its bar onto the tab row after the one
    // tab: a second workspace holding pane B, shown.
    let rect_b = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane_b)
        .map_or_else(|| panic!("no rect"), |(_, r)| r);
    let row = harness
        .state()
        .tab_row()
        .unwrap_or_else(|| panic!("no tab row"));
    let from = egui::pos2(rect_b.left() + 50.0, rect_b.top() + 14.0);
    let to = egui::pos2(row.left() + 240.0, row.center().y);
    drag(&mut harness, from, to);
    let model = harness.state().app().model();
    let window = model.focused_window;
    assert_eq!(
        model.workspaces(window).len(),
        2,
        "{:?}",
        model.workspaces(window)
    );
    assert_eq!(model.workspaces(window)[0].dock, model::Dock::Pane(pane_a));
    assert_eq!(model.workspaces(window)[1].dock, model::Dock::Pane(pane_b));
    assert_eq!(shown(&harness), 1);
    assert_eq!(titles(&harness), ["left-pane", "right-pane"]);
}
