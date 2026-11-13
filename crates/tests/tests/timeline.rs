//! The time slider in the real UI (`add-zfs-time-slider`): the bar under
//! the toolbar, a click on a tick, the keys while focused, a drag with its
//! release, *Back to now*, the view options entry and the palette command.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use model::{PaneId, TabId};
use platform::{ConfigDir, Fake};
use tests::local;
use tests::timeline::{SNAPSHOTS, Snapshots, write_tree};
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi};

fn harness(start: &Path, sources: Vec<Arc<dyn ext::Extension>>) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: sources,
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 720.0))
        // Frames of a sixtieth of a second, so a drag's 200 ms rest is
        // twelve frames, not one.
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
}

fn settle(harness: &mut Harness<'_, RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for _ in 0..3 {
            harness.step();
        }
        if !harness.state().app().is_busy() {
            std::thread::sleep(Duration::from_millis(30));
            harness.step();
            if !harness.state().app().is_busy() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "did not settle");
        std::thread::sleep(Duration::from_millis(10));
    }
}

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
    settle(harness);
}

/// A folder with its snapshots, the harness on it, its source, pane and tab.
struct Scene {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    source: Arc<Snapshots>,
    harness: Harness<'static, RickUi>,
    pane: PaneId,
    tab: TabId,
}

fn scene() -> Scene {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("docs");
    write_tree(&root, &SNAPSHOTS);
    let source = Snapshots::new(&root, &SNAPSHOTS, Duration::ZERO);
    let harness = harness(&root, vec![source.extension()]);
    let pane = harness.state().app().model().focused;
    let tab = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map_or_else(|| panic!("no tab"), |(id, _)| id);
    Scene {
        _dir: dir,
        root,
        source,
        harness,
        pane,
        tab,
    }
}

impl Scene {
    fn toggle(&mut self) {
        self.harness
            .state_mut()
            .queue(Intent::ToggleTimeline, self.pane);
        settle(&mut self.harness);
    }

    fn location(&self) -> types::Location {
        self.harness
            .state()
            .app()
            .model()
            .active_tab(self.pane)
            .map_or_else(|| panic!("no tab"), |(_, t)| t.location.clone())
    }

    fn axis(&self) -> egui::Rect {
        self.harness
            .state()
            .timeline_bar(self.tab)
            .and_then(|(_, axis)| axis)
            .unwrap_or_else(|| panic!("no axis"))
    }

    /// The x of point `index` on the axis, as the bar places it.
    fn x_of(&self, index: usize) -> f32 {
        let model = self.harness.state().app().model();
        let (timeline, _) = model
            .timeline_points(self.pane)
            .unwrap_or_else(|| panic!("no timeline"));
        let axis = self.axis();
        ui_egui::timeline_bar::positions(timeline, axis.left(), axis.right())[index]
    }

    fn at(&self, label: &str) -> types::Location {
        self.source.point(label, Path::new(""))
    }

    fn back_entries(&self) -> usize {
        self.harness
            .state()
            .app()
            .model()
            .active_tab(self.pane)
            .map_or(0, |(_, t)| t.history.back.len())
    }
}

#[test]
fn the_bar_appears_under_the_toolbar_when_switched_on() {
    let mut s = scene();
    assert!(
        s.harness.state().timeline_bar(s.tab).is_none(),
        "off at first"
    );
    s.toggle();
    let (bar, _) = s
        .harness
        .state()
        .timeline_bar(s.tab)
        .unwrap_or_else(|| panic!("no bar"));
    let toolbar = s
        .harness
        .state()
        .toolbar_row(s.tab)
        .unwrap_or_else(|| panic!("no toolbar"));
    assert!((bar.top() - toolbar.bottom()).abs() < 1.0, "directly below");
    assert!((bar.height() - toolbar.height()).abs() < 1.0, "as tall");
    s.harness.get_by_label("Timeline");
    assert_eq!(s.source.calls(), 1);
}

#[test]
fn a_click_on_a_tick_goes_there_and_back_to_now_returns() {
    let mut s = scene();
    s.toggle();
    let y = s.axis().center().y;
    let x = s.x_of(1);
    click_at(&mut s.harness, egui::pos2(x, y));
    assert_eq!(s.location(), s.at("weekly-45"));
    assert!(s.harness.state().app().model().in_the_past(s.pane));
    let back = s.harness.get_by_label("Back to now").rect().center();
    click_at(&mut s.harness, back);
    assert_eq!(s.location(), local(&s.root));
    assert_eq!(s.back_entries(), 1, "one run, one step");
}

#[test]
fn the_focused_bar_steps_with_the_keys() {
    let mut s = scene();
    s.toggle();
    let y = s.axis().center().y;
    let x = s.x_of(2);
    click_at(&mut s.harness, egui::pos2(x, y));
    assert_eq!(s.location(), s.at("daily-mon"));
    for (key, label) in [
        (egui::Key::ArrowLeft, Some("weekly-45")),
        (egui::Key::ArrowRight, Some("daily-mon")),
        (egui::Key::Home, Some("weekly-44")),
        (egui::Key::End, None),
    ] {
        s.harness.key_press(key);
        settle(&mut s.harness);
        let want = label.map_or_else(|| local(&s.root), |l| s.at(l));
        assert_eq!(s.location(), want, "{key:?}");
    }
    assert_eq!(s.back_entries(), 1, "the whole run is one step");
    s.harness.state_mut().queue(Intent::Back, s.pane);
    settle(&mut s.harness);
    assert_eq!(s.location(), local(&s.root));
}

#[test]
fn a_drag_goes_where_it_rests_or_is_released() {
    let mut s = scene();
    s.toggle();
    let y = s.axis().center().y;
    let press = |s: &mut Scene, pos: egui::Pos2, pressed: bool| {
        s.harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        s.harness.step();
    };
    let glide = |s: &mut Scene, from: egui::Pos2, to: egui::Pos2| {
        for step in 1..=6u8 {
            let t = f32::from(step) / 6.0;
            s.harness.event(egui::Event::PointerMoved(from.lerp(to, t)));
            s.harness.step();
        }
    };
    let start = egui::pos2(s.axis().right(), y);
    s.harness.event(egui::Event::PointerMoved(start));
    s.harness.step();
    press(&mut s, start, true);
    let rest = egui::pos2(s.x_of(3), y);
    glide(&mut s, start, rest);
    assert_eq!(s.location(), local(&s.root), "no travel while moving");
    // Resting on a point for the 200 ms takes the pane there.
    for _ in 0..16 {
        s.harness.step();
    }
    settle(&mut s.harness);
    assert_eq!(s.location(), s.at("daily-tue"));
    // On, and released: the pane goes where the handle was let go.
    let end = egui::pos2(s.x_of(5), y);
    glide(&mut s, rest, end);
    press(&mut s, end, false);
    settle(&mut s.harness);
    assert_eq!(s.location(), s.at("daily-thu"));
    assert_eq!(s.back_entries(), 1, "the drag is one step");
}

#[test]
fn the_menu_entry_and_the_palette_follow_the_sources() {
    // No source: no entry, the palette's command disabled.
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut plain = harness(dir.path(), Vec::new());
    let pane = plain.state().app().model().focused;
    let button = plain.get_by_label("View options").rect().center();
    click_at(&mut plain, button);
    assert!(
        plain.query_by_label("Timeline").is_none(),
        "no source, no entry"
    );
    let item = ui_egui::palette::items(
        plain.state().app(),
        &ui_egui::keys::KeyMap::new(
            plain.state().app().registry(),
            &plain.state().app().model().settings,
        ),
        pane,
        &[],
    )
    .into_iter()
    .find(|i| i.intent == Intent::ToggleTimeline)
    .unwrap_or_else(|| panic!("no palette entry"));
    assert_eq!(
        item.disabled.as_deref(),
        Some("No timeline for this folder")
    );

    // A source with nothing for the folder: disabled once read.
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let empty = Snapshots::new(dir.path(), &[], Duration::ZERO);
    let mut h = harness(dir.path(), vec![empty.extension()]);
    let pane = h.state().app().model().focused;
    h.state_mut().queue(Intent::ToggleTimeline, pane);
    settle(&mut h);
    let button = h.get_by_label("View options").rect().center();
    click_at(&mut h, button);
    let entry = h.get_by_label("Timeline");
    assert!(entry.accesskit_node().is_disabled(), "no earlier states");
    h.get_by_label("No earlier states of this folder");
}

#[test]
fn keys_work_when_the_bar_got_the_focus_without_a_click() {
    let mut s = scene();
    s.toggle();
    s.harness
        .ctx
        .memory_mut(|m| m.request_focus(ui_egui::timeline_bar::id(s.pane)));
    s.harness.step();
    assert!(ui_egui::timeline_bar::focused(&s.harness.ctx, s.pane));
    s.harness.key_press(egui::Key::Home);
    settle(&mut s.harness);
    assert_eq!(s.location(), s.at("weekly-44"));
}
