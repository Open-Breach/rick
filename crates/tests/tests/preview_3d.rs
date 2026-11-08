//! Acceptance (issue 19): the 3D previewer plugin. It loads through the
//! host and claims the fixtures by sniff over the other previewers; the
//! pointer gestures a document declares reach it through `set_control`
//! from the preview pane and from the viewer, with no 3D code in either;
//! the preview keeps its frame while it orbits; a 12-triangle STL, a
//! 500 000-triangle binary STL, an OBJ with three objects and a 3MF in
//! millimetres each show a frame within 300 ms (release; recorded as
//! `3d-first`); the bounding box of a 10 mm cube reads 10 × 10 × 10 mm;
//! and a 3 000 000-triangle STL decimates for the preview and says so.
//! Frames render on the runner's software adapter (lavapipe on Linux, WARP
//! on Windows, `RICK_REQUIRE_WGPU`); a runner without one skips the frame
//! checks with a note.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use ext::Text;
use ext::{
    API_VERSION, Capabilities, ControlId, ControlValue, Extension, Frame, Gesture, Icon,
    InfoSection, Manifest, Platforms, Points, PreviewControl, PreviewDoc, Previewer, Provider,
    Registry, RenderRequest, TextLayer, Transport, Version, async_trait,
};
use ext_preview_3d::fixtures;
use model::{SelectionChange, TabKind};
use platform::{ConfigDir, Fake};
use plugins_native::{Host, Library, MANIFEST_FILE, Options, PluginStatus, PluginToml, Source};
use tests::local;
use types::{CancelToken, Entry, EntryKind, Error, ExtId, Location};
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi};

/// The plugins installed for the sniff test: the 3D previewer and the two
/// that would claim its files otherwise (hex claims everything, the
/// archive previewer every zip).
const PLUGINS: [(&str, &str); 3] = [
    ("ext_preview_3d", "preview-3d"),
    ("ext_preview_hex", "preview-hex"),
    ("ext_preview_archive", "preview-archive"),
];

fn deps_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    exe.parent()
        .unwrap_or_else(|| panic!("no parent of {}", exe.display()))
        .to_owned()
}

fn library_of(stem: &str) -> PathBuf {
    let path = deps_dir().join(format!(
        "{}{stem}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(path.is_file(), "not built: {}", path.display());
    path
}

/// Installs the library of the crate `stem` into `plugins_dir` with the
/// manifest the bundler would write.
fn install(plugins_dir: &Path, stem: &str) {
    let library = library_of(stem);
    let mut toml: PluginToml = plugins_native::describe(&library).unwrap_or_else(|e| panic!("{e}"));
    toml.library = Library::for_crate(stem);
    let dir = plugins_dir.join(&toml.id);
    toml.write(&dir.join(MANIFEST_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    let file = toml
        .library
        .for_os(std::env::consts::OS)
        .unwrap_or_else(|| panic!("no library name for this OS"));
    fs::copy(&library, dir.join(file)).unwrap_or_else(|e| panic!("{e}"));
}

/// The plugins installed into a temporary plugins directory and loaded by
/// the host, which stays alive (and so mapped) for the process.
fn load_plugins() -> &'static Host {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    for (stem, _) in PLUGINS {
        install(&plugins, stem);
    }
    let host = Host::load(&Options {
        dirs: vec![(Source::BesideBinary, plugins)],
        disabled: Vec::new(),
        compiled_in: Vec::new(),
        platform: None,
        prompts: None,
    });
    for (_, id) in PLUGINS {
        let row = host
            .plugins()
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("no plugin {id}: {:?}", host.plugins()));
        assert_eq!(row.status, PluginStatus::Loaded, "{id}: {:?}", row.status);
    }
    let _ = dir.keep();
    Box::leak(Box::new(host))
}

fn built_ins() -> Vec<Arc<dyn Extension>> {
    vec![
        Arc::new(ext_preview_text::TextPreview::new()),
        Arc::new(ext_preview_code::CodePreview::new()),
        Arc::new(ext_preview_image::ImagePreview::new()),
        Arc::new(ext_preview_pdf::PdfPreview::new()),
    ]
}

fn provider() -> Arc<dyn Provider> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    Arc::new(vfs_local::LocalProvider::new(fake))
}

/// The entry and the first 4 KiB of `path`.
fn sniff(provider: &Arc<dyn Provider>, path: &Path) -> (Entry, Vec<u8>) {
    let loc = local(path);
    let entry = runtime::block_on(provider.stat(&loc)).unwrap_or_else(|e| panic!("{e}"));
    let bytes = runtime::block_on(ext::read_bytes(
        &**provider,
        &loc,
        Some(4096),
        &CancelToken::new(),
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    (entry, bytes)
}

fn open(
    previewer: &Arc<dyn Previewer>,
    provider: &Arc<dyn Provider>,
    path: &Path,
) -> Box<dyn PreviewDoc> {
    runtime::block_on(previewer.open(&local(path), provider.clone(), CancelToken::new()))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Whether the process has a graphics adapter, saying so when not; fails
/// where one is required (`RICK_REQUIRE_WGPU`: CI on Linux and Windows).
fn gpu_or_skip(what: &str) -> bool {
    match ext_preview_3d::render::gpu() {
        Ok(_) => true,
        Err(error) => {
            eprintln!("no graphics adapter ({error}); {what} is not checked here");
            assert!(
                std::env::var_os("RICK_REQUIRE_WGPU").is_none(),
                "a graphics adapter is required here but none was found: {error}"
            );
            false
        }
    }
}

/// The four fixtures of the issue in `dir`: name and what the status bar
/// must say.
fn fixtures(dir: &Path) -> Vec<(&'static str, Vec<&'static str>)> {
    fs::write(dir.join("cube.stl"), fixtures::cube_stl_ascii(10.0))
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.join("torus.stl"), fixtures::torus_stl(500_000))
        .unwrap_or_else(|e| panic!("{e}"));
    let (obj, mtl) = fixtures::three_cubes_obj();
    fs::write(dir.join("cubes.obj"), obj).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.join(fixtures::OBJ_MATERIALS), mtl).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.join("cube.3mf"), fixtures::cube_3mf(10.0, "millimeter"))
        .unwrap_or_else(|e| panic!("{e}"));
    vec![
        (
            "cube.stl",
            vec!["STL (ASCII)", "12 triangles", "10 × 10 × 10 mm"],
        ),
        ("torus.stl", vec!["STL (binary)", "500 000 triangles"]),
        (
            "cubes.obj",
            vec!["OBJ", "36 triangles", "40 × 10 × 10 mm", "3 objects"],
        ),
        ("cube.3mf", vec!["3MF", "12 triangles", "10 × 10 × 10 mm"]),
    ]
}

#[test]
fn the_3d_plugin_loads_through_the_host_and_claims_the_fixtures_by_sniff() {
    let host = load_plugins();
    let mut registry = Registry::new();
    for extension in built_ins().into_iter().chain(host.extensions()) {
        registry.load(&*extension).unwrap_or_else(|e| panic!("{e}"));
    }
    assert!(
        registry
            .settings_schemas()
            .iter()
            .any(|s| s.ext == ext_preview_3d::ID && s.fields[0].key == "triangle_budget"),
        "the triangle budget is declared"
    );
    let previewers = registry.previewers().to_vec();
    let provider = provider();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for (name, status) in fixtures(dir.path()) {
        let path = dir.path().join(name);
        let (entry, head) = sniff(&provider, &path);
        let (best, score) = previewers
            .iter()
            .map(|p| (p, p.score(&entry, &head)))
            .max_by_key(|(_, s)| *s)
            .unwrap_or_else(|| panic!("no previewers"));
        assert_eq!(best.id(), ext_preview_3d::ID, "{name}: score {score}");
        assert!(score >= 200, "{name}: {score}");
        let doc = open(best, &provider, &path);
        let facts = doc.status();
        for fact in status {
            assert!(
                facts.iter().any(|f| f == fact),
                "{name}: {fact:?} in {facts:?}"
            );
        }
        assert!(
            doc.controls().iter().any(|c| matches!(
                c,
                PreviewControl::Pointer {
                    gesture: Gesture::Drag,
                    ..
                }
            )),
            "{name}: orbit is a pointer control"
        );
        if name == "cube.3mf" {
            let rows = &doc.sections()[0].rows;
            assert!(
                rows.contains(&("Units".to_owned(), "millimeter".to_owned())),
                "{rows:?}"
            );
            assert!(
                rows.contains(&("Title".to_owned(), "Cube".to_owned())),
                "{rows:?}"
            );
        }
    }
    // Text, hex and the archive previewer are there and lost.
    let ids: Vec<ExtId> = previewers.iter().map(|p| p.id()).collect();
    assert!(
        ids.contains(&ExtId("preview-hex")) && ids.contains(&ExtId("preview-archive")),
        "{ids:?}"
    );
}

/// A previewer whose document records every control it is set, for the
/// gesture routing of the panes: it claims `.gst` files and renders a
/// plain frame of the requested size.
struct GestureExtension {
    manifest: Manifest,
    log: Arc<Mutex<Vec<(ControlId, ControlValue)>>>,
}

const DRAG: ControlId = ControlId("gst.drag");
const WHEEL: ControlId = ControlId("gst.wheel");
const DOUBLE: ControlId = ControlId("gst.double");

impl GestureExtension {
    fn new(log: Arc<Mutex<Vec<(ControlId, ControlValue)>>>) -> Self {
        Self {
            manifest: Manifest {
                id: ExtId("gestures"),
                name: "Gestures",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PREVIEWER,
                platforms: Platforms::ALL,
            },
            log,
        }
    }
}

impl Extension for GestureExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.previewer(Arc::new(GesturePreviewer {
            log: self.log.clone(),
        }));
        Ok(())
    }
}

struct GesturePreviewer {
    log: Arc<Mutex<Vec<(ControlId, ControlValue)>>>,
}

#[async_trait]
impl Previewer for GesturePreviewer {
    fn id(&self) -> ExtId {
        ExtId("gestures")
    }
    fn score(&self, entry: &Entry, _sniff: &[u8]) -> u8 {
        let gesture_file = std::path::Path::new(&entry.name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gst"));
        u8::from(entry.kind == EntryKind::File && gesture_file) * 250
    }
    async fn open(
        &self,
        _loc: &Location,
        _provider: Arc<dyn Provider>,
        _cancel: CancelToken,
    ) -> Result<Box<dyn PreviewDoc>, Error> {
        Ok(Box::new(GestureDoc {
            log: self.log.clone(),
            renders: 0,
        }))
    }
}

struct GestureDoc {
    log: Arc<Mutex<Vec<(ControlId, ControlValue)>>>,
    renders: u32,
}

impl PreviewDoc for GestureDoc {
    fn pages(&self) -> Option<usize> {
        None
    }
    fn render(&mut self, _page: usize, req: RenderRequest) -> Result<Frame, Error> {
        self.renders += 1;
        let shade = u8::try_from(40 + self.renders * 20).unwrap_or(255);
        let data: Vec<u8> = (0..req.width * req.height)
            .flat_map(|_| [shade, 120, 200, 255])
            .collect();
        Ok(Frame::Rgba {
            width: req.width,
            height: req.height,
            data: data.into(),
        })
    }
    fn text_layer(&self) -> Option<&TextLayer> {
        None
    }
    fn sections(&self) -> Vec<InfoSection> {
        vec![InfoSection {
            id: "file",
            title: Text("File info"),
            icon: Icon::Info,
            rows: vec![("Renders".to_owned(), self.renders.to_string())],
        }]
    }
    fn status(&self) -> Vec<String> {
        vec![format!("{} renders", self.renders)]
    }
    fn controls(&self) -> Vec<PreviewControl> {
        vec![
            PreviewControl::Pointer {
                id: DRAG,
                gesture: Gesture::Drag,
                label: "Orbit",
            },
            PreviewControl::Pointer {
                id: WHEEL,
                gesture: Gesture::Wheel,
                label: "Zoom",
            },
            PreviewControl::Pointer {
                id: DOUBLE,
                gesture: Gesture::DoubleClick,
                label: "Reset",
            },
        ]
    }
    fn set_control(&mut self, id: ControlId, value: ControlValue) -> Result<(), Error> {
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((id, value));
        Ok(())
    }
    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

/// A window on `start` with the Preview tab open in a 400-point pane.
fn window(
    fake: &Arc<Fake>,
    start: &Path,
    extensions: Vec<Arc<dyn Extension>>,
) -> Harness<'static, RickUi> {
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions,
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .with_pixels_per_point(1.0)
        // A frame every 16 ms of simulated time, as on a display, so the
        // two clicks of a double-click fall inside egui's 300 ms window
        // (kittest's default step is a quarter of a second).
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| {
            let mut ui = RickUi::new(app, host, &cc.egui_ctx);
            ui.set_inspector_width(400.0);
            ui
        });
    settle(&mut harness);
    if !harness.state().inspector_open() {
        let pane = harness.state().app().model().focused;
        harness.state_mut().queue(Intent::ToggleInspector, pane);
        harness.step();
    }
    harness.step();
    harness.step();
    assert!(harness.state().inspector_open());
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
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn id_of(harness: &Harness<'_, RickUi>, name: &str) -> types::EntryId {
    let pane = harness.state().app().model().focused;
    harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| t.listing.rows().find(|e| e.name == name).map(|e| e.id))
        .unwrap_or_else(|| panic!("no entry {name}"))
}

/// Steps until the focused pane's slot has a frame or an error.
fn wait_frame(harness: &mut Harness<'_, RickUi>, what: &str) {
    let started = Instant::now();
    loop {
        harness.step();
        let pane = harness.state().app().model().focused;
        let slot = harness.state().previews().get(pane);
        if slot.is_some_and(|s| s.has_frame() || s.error().is_some()) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "{what}: no frame: {slot:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    harness.step();
    harness.step();
}

/// Steps until the focused pane's slot has no render in flight: a gesture
/// made while one runs is applied when it returns, and the next frame
/// renders.
fn wait_idle(harness: &mut Harness<'_, RickUi>) {
    let started = Instant::now();
    loop {
        harness.step();
        let pane = harness.state().app().model().focused;
        if harness
            .state()
            .previews()
            .get(pane)
            .is_none_or(|s| !s.rendering())
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "still rendering"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    harness.step();
}

/// Selects `name` in the focused pane and waits for its preview.
fn select(harness: &mut Harness<'_, RickUi>, name: &str) {
    let pane = harness.state().app().model().focused;
    let id = id_of(harness, name);
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    wait_frame(harness, name);
}

/// Opens `name` in a viewer tab with Enter and waits for its frame.
fn open_in_viewer(harness: &mut Harness<'_, RickUi>, name: &str) {
    select(harness, name);
    harness.key_press(egui::Key::Enter);
    let started = Instant::now();
    loop {
        harness.step();
        let model = harness.state().app().model();
        let viewer = model
            .active_tab(model.focused)
            .is_some_and(|(_, tab)| tab.kind == TabKind::Viewer);
        if viewer {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "no viewer tab");
        std::thread::sleep(Duration::from_millis(2));
    }
    wait_frame(harness, name);
}

/// Where the current frame is drawn.
fn frame_centre(harness: &Harness<'_, RickUi>) -> egui::Pos2 {
    let pane = harness.state().app().model().focused;
    harness
        .state()
        .previews()
        .get(pane)
        .and_then(ui_egui::preview::Slot::frame_rect)
        .map_or_else(|| panic!("no frame rect"), |r| r.center())
}

/// The rectangle of the focused pane.
fn pane_centre(harness: &Harness<'_, RickUi>) -> egui::Pos2 {
    let pane = harness.state().app().model().focused;
    harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(id, _)| *id == pane)
        .map_or_else(|| panic!("no pane rect"), |(_, rect)| rect.center())
}

fn button(harness: &mut Harness<'_, RickUi>, pos: egui::Pos2, pressed: bool) {
    harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
}

/// Presses at `from`, moves to `to` in small steps and releases.
fn drag(harness: &mut Harness<'_, RickUi>, from: egui::Pos2, to: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(from));
    harness.step();
    button(harness, from, true);
    for i in 1..=8u8 {
        let pos = from + (to - from) * (f32::from(i) / 8.0);
        harness.event(egui::Event::PointerMoved(pos));
        harness.step();
    }
    button(harness, to, false);
    harness.step();
}

fn wheel(harness: &mut Harness<'_, RickUi>, at: egui::Pos2, dy: f32) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    harness.step();
}

fn double_click(harness: &mut Harness<'_, RickUi>, at: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    for _ in 0..2 {
        button(harness, at, true);
        button(harness, at, false);
    }
    harness.step();
}

/// What the document was set, summed per control: the drag's and the
/// wheel's deltas, and the number of presses.
fn received(log: &Mutex<Vec<(ControlId, ControlValue)>>) -> (egui::Vec2, egui::Vec2, usize) {
    let mut drag = egui::Vec2::ZERO;
    let mut wheel = egui::Vec2::ZERO;
    let mut presses = 0;
    for (id, value) in log
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
    {
        match (id, value) {
            (&DRAG, ControlValue::Delta { x, y }) => drag += egui::vec2(*x, *y),
            (&WHEEL, ControlValue::Delta { x, y }) => wheel += egui::vec2(*x, *y),
            (&DOUBLE, ControlValue::Pressed) => presses += 1,
            other => panic!("unexpected control {other:?}"),
        }
    }
    (drag, wheel, presses)
}

#[test]
fn pointer_gestures_reach_the_document_from_the_preview_pane_and_the_viewer() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("spin.gst"), b"spin").unwrap_or_else(|e| panic!("{e}"));
    let log = Arc::new(Mutex::new(Vec::new()));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut extensions = built_ins();
    extensions.push(Arc::new(GestureExtension::new(log.clone())));
    let mut harness = window(&fake, dir.path(), extensions);
    let pane = harness.state().app().model().focused;

    // The preview pane: a drag orbits instead of panning, the wheel zooms,
    // a double-click resets, and the frame stays on screen throughout.
    select(&mut harness, "spin.gst");
    assert!(
        harness
            .state()
            .previews()
            .get(pane)
            .is_some_and(ui_egui::preview::Slot::is_raster)
    );
    let centre = frame_centre(&harness);
    let before = harness
        .state()
        .previews()
        .get(pane)
        .map_or(egui::Vec2::ZERO, ui_egui::preview::Slot::pan_offset);
    drag(&mut harness, centre, centre + egui::vec2(-90.0, 30.0));
    wait_idle(&mut harness);
    let (dragged, _, _) = received(&log);
    assert!(
        dragged.x < -60.0 && dragged.y > 20.0,
        "the drag reached the document: {dragged:?}"
    );
    assert_eq!(
        harness
            .state()
            .previews()
            .get(pane)
            .map_or(egui::Vec2::ZERO, ui_egui::preview::Slot::pan_offset),
        before,
        "the pane did not pan"
    );
    assert!(
        harness
            .state()
            .previews()
            .get(pane)
            .is_some_and(ui_egui::preview::Slot::has_frame),
        "the frame stayed"
    );
    wheel(&mut harness, centre, 48.0);
    wait_idle(&mut harness);
    let (_, wheeled, _) = received(&log);
    assert!(
        wheeled.y > 40.0,
        "the wheel reached the document: {wheeled:?}"
    );
    double_click(&mut harness, centre);
    wait_idle(&mut harness);
    let (_, _, presses) = received(&log);
    assert_eq!(presses, 1, "the double-click reached the document");
    assert!(
        !harness
            .state()
            .previews()
            .get(pane)
            .is_some_and(|s| s.actual_size),
        "the double-click did not switch to 1:1"
    );
    wait_frame(&mut harness, "after the gestures");

    // The viewer: the same three, with the pane's own zoom untouched.
    log.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    open_in_viewer(&mut harness, "spin.gst");
    let centre = pane_centre(&harness);
    drag(&mut harness, centre, centre + egui::vec2(70.0, -40.0));
    wait_idle(&mut harness);
    wheel(&mut harness, centre, -32.0);
    wait_idle(&mut harness);
    double_click(&mut harness, centre);
    wait_idle(&mut harness);
    let (dragged, wheeled, presses) = received(&log);
    assert!(
        dragged.x > 50.0 && dragged.y < -25.0,
        "the viewer forwarded the drag: {dragged:?}"
    );
    assert!(
        wheeled.y < -25.0,
        "the viewer forwarded the wheel: {wheeled:?}"
    );
    assert_eq!(presses, 1, "the viewer forwarded the double-click");
    let model = harness.state().app().model();
    let tab = model
        .active_tab(model.focused)
        .map_or_else(|| panic!("tab"), |(id, _)| id);
    let zoom = harness.state().viewer(tab).map(|v| v.zoom);
    assert_eq!(
        zoom,
        Some(ui_egui::viewer::Zoom::Fit),
        "the wheel without a modifier is not the viewer's zoom"
    );
}

#[test]
fn the_3d_preview_keeps_its_frame_while_it_orbits() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("cube.stl"), fixtures::cube_stl_ascii(10.0))
        .unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut extensions = built_ins();
    extensions.push(Arc::new(ext_preview_3d::Preview3d::new()));
    let mut harness = window(&fake, dir.path(), extensions);
    let pane = harness.state().app().model().focused;
    select(&mut harness, "cube.stl");
    let status = harness
        .state()
        .previews()
        .get(pane)
        .map(|s| s.status().to_vec())
        .unwrap_or_default();
    assert!(status.iter().any(|f| f == "10 × 10 × 10 mm"), "{status:?}");
    assert!(status.iter().any(|f| f == "12 triangles"), "{status:?}");
    if !gpu_or_skip("the frame in the pane") {
        assert!(
            harness
                .state()
                .previews()
                .get(pane)
                .is_some_and(ui_egui::preview::Slot::is_text),
            "the facts as text"
        );
        return;
    }
    assert!(
        harness
            .state()
            .previews()
            .get(pane)
            .is_some_and(ui_egui::preview::Slot::is_raster)
    );
    let centre = frame_centre(&harness);
    // Every step of a drag keeps a frame on screen, and the document is
    // rendered again for the new view.
    let from = centre;
    harness.event(egui::Event::PointerMoved(from));
    harness.step();
    button(&mut harness, from, true);
    let mut rendered_again = false;
    for i in 1..=12u8 {
        let pos = from + egui::vec2(f32::from(i) * 8.0, 0.0);
        harness.event(egui::Event::PointerMoved(pos));
        harness.step();
        let slot = harness
            .state()
            .previews()
            .get(pane)
            .unwrap_or_else(|| panic!("slot"));
        assert!(slot.has_frame(), "step {i}: {slot:?}");
        rendered_again |= slot.rendering();
        std::thread::sleep(Duration::from_millis(5));
    }
    button(&mut harness, from + egui::vec2(96.0, 0.0), false);
    assert!(rendered_again, "the orbit rendered the document again");
    wait_frame(&mut harness, "after the orbit");
    let slot = harness
        .state()
        .previews()
        .get(pane)
        .unwrap_or_else(|| panic!("slot"));
    assert!(slot.has_frame() && slot.error().is_none(), "{slot:?}");
}

#[test]
#[ignore = "timing; run with --ignored in release"]
fn each_fixture_shows_a_frame_within_300_ms() {
    if !gpu_or_skip("the first frame") {
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fixtures = fixtures(dir.path());
    let previewer: Arc<dyn Previewer> = Arc::new(ext_preview_3d::Previewer3d::new(None));
    let provider = provider();
    let request = RenderRequest {
        width: 800,
        height: 600,
        scale: 1.0,
        dark: false,
    };
    // The device is opened once, before anything is timed.
    let mut warm = open(&previewer, &provider, &dir.path().join("cube.stl"));
    assert!(matches!(warm.render(0, request), Ok(Frame::Rgba { .. })));
    let target = tests::bench::target("3d-first");
    let assert = !cfg!(debug_assertions) && std::env::var_os("RICK_BENCH_RECORD").is_none();
    let mut samples = Vec::new();
    for run in 0..tests::bench::runs() {
        for (name, _) in &fixtures {
            let path = dir.path().join(name);
            let started = Instant::now();
            let mut doc = open(&previewer, &provider, &path);
            let frame = doc
                .render(0, request)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let elapsed = started.elapsed();
            assert!(
                matches!(
                    frame,
                    Frame::Rgba {
                        width: 800,
                        height: 600,
                        ..
                    }
                ),
                "{name}: {frame:?}"
            );
            println!("{name}: first frame in {elapsed:?} (run {run})");
            if *name == "torus.stl" {
                samples.push(elapsed);
            } else if assert {
                assert!(elapsed <= target, "{name}: {elapsed:?} over {target:?}");
            }
        }
    }
    tests::bench::record("3d-first", &samples);
}

#[test]
#[ignore = "writes a 150 MB STL; run with --ignored in release"]
fn a_3_million_triangle_stl_decimates_for_the_preview_and_says_so() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("big.stl");
    fs::write(&path, fixtures::torus_stl(3_000_000)).unwrap_or_else(|e| panic!("{e}"));
    let previewer: Arc<dyn Previewer> = Arc::new(ext_preview_3d::Previewer3d::new(None));
    let provider = provider();
    let started = Instant::now();
    let mut doc = open(&previewer, &provider, &path);
    println!(
        "3 000 000 triangles opened and decimated in {:?}",
        started.elapsed()
    );
    let status = doc.status();
    assert!(
        status.iter().any(|f| f == "3 000 000 triangles"),
        "{status:?}"
    );
    let note = status
        .iter()
        .find(|f| f.starts_with("decimated to "))
        .unwrap_or_else(|| panic!("no decimation note in {status:?}"));
    let shown: usize = note
        .trim_start_matches("decimated to ")
        .split(" triangles")
        .next()
        .unwrap_or_default()
        .replace(' ', "")
        .parse()
        .unwrap_or_else(|e| panic!("{note}: {e}"));
    assert!(shown <= 2_000_000 && shown > 100_000, "{note}");
    assert!(
        doc.sections()[0]
            .rows
            .iter()
            .any(|(k, v)| k == "Preview" && v.contains("budget 2 000 000")),
        "{:?}",
        doc.sections()[0].rows
    );
    if gpu_or_skip("the decimated frame") {
        let started = Instant::now();
        let frame = doc
            .render(
                0,
                RenderRequest {
                    width: 800,
                    height: 600,
                    scale: 1.0,
                    dark: true,
                },
            )
            .unwrap_or_else(|e| panic!("{e}"));
        println!("decimated frame in {:?}", started.elapsed());
        assert!(matches!(frame, Frame::Rgba { .. }), "{frame:?}");
    }
}
