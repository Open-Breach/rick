//! Acceptance (issue 149, item 5, the window), with the monitors the
//! platform reports scripted (SPEC.md *Window geometry*).
//!
//! A first launch opens centred on the primary monitor at the default
//! size. Quit with the window on a second monitor and start with that
//! monitor still there: the window comes back exactly; start with it
//! unplugged: the window is centred on the primary monitor, at most
//! 1920 × 1200 and no wider than 16:9 of its height. A second window is
//! offset from the first by a title row's height.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::Command;
use egui_kittest::Harness;
use model::Geometry;
use model::placement::{Area, default_size};
use platform::{ConfigDir, Fake, Monitor};
use tests::local;
use ui_egui::{EguiHost, RickUi};

/// The title row's height in the default density.
const TITLE: f32 = 40.0;

/// A laptop's screen with a panel of 32 px at its top: the primary one.
fn laptop() -> Monitor {
    Monitor {
        id: "eDP-1".into(),
        primary: true,
        bounds: [0.0, 0.0, 1920.0, 1080.0],
        work: [0.0, 32.0, 1920.0, 1048.0],
    }
}

/// A larger monitor to its right.
fn second() -> Monitor {
    Monitor {
        id: "DP-2".into(),
        primary: false,
        bounds: [1920.0, 0.0, 2560.0, 1440.0],
        work: [1920.0, 0.0, 2560.0, 1440.0],
    }
}

/// A window of Rick over `fake`, whose settings and session are `fake`'s.
fn window(fake: &Arc<Fake>, start: &Path) -> Harness<'static, RickUi> {
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
        .with_pixels_per_point(1.0)
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

/// The focused window's geometry and monitor, as the model has them.
fn placed(harness: &Harness<'_, RickUi>) -> (Geometry, Option<String>) {
    let model = harness.state().app().model();
    let window = model
        .window(model.focused_window)
        .unwrap_or_else(|| panic!("no window"));
    (
        window.geometry.unwrap_or_else(|| panic!("no geometry")),
        window.monitor.clone(),
    )
}

/// Whether `geometry` is the default size, centred in `work`.
fn centred_at_the_default_size(geometry: &Geometry, work: [f32; 4]) -> bool {
    let area = Area::new(work[0], work[1], work[2], work[3]);
    let (width, height) = default_size(&area);
    (geometry.width - width).abs() < 0.5
        && (geometry.height - height).abs() < 0.5
        && (geometry.x + geometry.width / 2.0 - (area.x + area.width / 2.0)).abs() < 0.5
        && (geometry.y + geometry.height / 2.0 - (area.y + area.height / 2.0)).abs() < 0.5
}

/// Waits until the session on disk names `needle`.
fn saved(harness: &mut Harness<'_, RickUi>, fake: &Fake, needle: &str) {
    let path = fake.config_dir().join(app::SESSION_FILE);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        settle(harness);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        if text.contains(needle) {
            return;
        }
        assert!(Instant::now() < deadline, "not saved: {text}");
    }
}

/// A first launch: centred in the working area of the primary monitor, at
/// the default size, whichever monitor the system lists first.
#[test]
fn a_first_launch_opens_centred_on_the_primary_monitor_at_the_default_size() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    fake.set_monitors(vec![second(), laptop()]);
    let harness = window(&fake, dir.path());
    let (geometry, monitor) = placed(&harness);
    assert!(
        centred_at_the_default_size(&geometry, laptop().work),
        "{geometry:?}"
    );
    assert_eq!(monitor.as_deref(), Some("eDP-1"));
    assert!(!geometry.maximised);
}

/// Quit with the window on the second monitor. With that monitor still
/// there it comes back exactly; with it unplugged it is centred on the
/// primary monitor, at most 1920 × 1200 and no wider than 16:9 of its
/// height.
#[test]
fn a_window_comes_back_on_its_monitor_and_on_the_primary_one_when_that_is_gone() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    fake.set_monitors(vec![laptop(), second()]);
    let on_the_second = Geometry {
        x: 2300.0,
        y: 120.0,
        width: 2000.0,
        height: 1250.0,
        maximised: false,
    };
    {
        let mut harness = window(&fake, dir.path());
        let root = harness.state().app().model().focused_window;
        harness.state_mut().app_mut().handle(
            Command::WindowGeometry {
                window: root,
                geometry: on_the_second,
                monitor: Some("DP-2".into()),
            },
            &mut Vec::new(),
        );
        saved(&mut harness, &fake, "DP-2");
    }

    // The monitor is still there: exactly as it was.
    {
        let harness = window(&fake, dir.path());
        let (geometry, monitor) = placed(&harness);
        assert_eq!(geometry, on_the_second);
        assert_eq!(monitor.as_deref(), Some("DP-2"));
    }

    // Unplugged: centred on the primary monitor at the default size.
    fake.set_monitors(vec![laptop()]);
    let harness = window(&fake, dir.path());
    let (geometry, monitor) = placed(&harness);
    assert!(
        centred_at_the_default_size(&geometry, laptop().work),
        "{geometry:?}"
    );
    assert!(geometry.width <= 1920.0 && geometry.height <= 1200.0);
    assert!(geometry.width <= geometry.height * 16.0 / 9.0 + 1.0);
    assert_eq!(monitor.as_deref(), Some("eDP-1"));
}

/// A second window opens at the size of the one it was opened from, a
/// title row's height down and to the right of it.
#[test]
fn a_second_window_is_offset_from_the_first_by_a_title_rows_height() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    fake.set_monitors(vec![laptop()]);
    let mut harness = window(&fake, dir.path());
    let (first, _) = placed(&harness);
    let root = harness.state().app().model().focused_window;

    harness
        .state_mut()
        .app_mut()
        .handle(Command::NewWindow, &mut Vec::new());
    settle(&mut harness);
    assert_eq!(harness.state().app().model().windows().count(), 2);
    assert_ne!(harness.state().app().model().focused_window, root);
    let (second, monitor) = placed(&harness);
    assert_eq!(
        (second.x, second.y),
        (first.x + TITLE, first.y + TITLE),
        "{first:?} then {second:?}"
    );
    assert_eq!((second.width, second.height), (first.width, first.height));
    assert_eq!(monitor.as_deref(), Some("eDP-1"));

    // The first window is where it was.
    let model = harness.state().app().model();
    assert_eq!(model.window(root).and_then(|w| w.geometry), Some(first));
}

/// Where the platform reports no monitors — a Wayland session — nothing is
/// placed: the saved size comes back and the rest is the system's.
#[test]
fn without_monitors_the_window_is_left_to_the_system() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let harness = window(&fake, dir.path());
    let model = harness.state().app().model();
    let window = model
        .window(model.focused_window)
        .unwrap_or_else(|| panic!("no window"));
    assert_eq!(window.monitor, None);
}
