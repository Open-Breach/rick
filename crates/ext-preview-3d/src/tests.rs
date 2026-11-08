//! The plugin's own tests: scoring, the three parsers, the budget, the
//! camera and its controls, the settings file, and, where a graphics
//! adapter exists, the frames.

// The fixtures are whole millimetres and the constants are exact, so the
// tests compare them as they are.
#![allow(clippy::float_cmp)]

use std::collections::HashMap;
use std::sync::Arc;

use ext::{ControlValue, Frame, PreviewControl, PreviewDoc, Previewer, Provider, RenderRequest};
use platform::{ConfigDir as _, Fake};
use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Location, ProviderId};

use super::*;
use crate::fixtures;

fn entry(name: &str, size: u64) -> Entry {
    Entry {
        id: EntryId::new(ProviderId("local"), 1),
        name: name.into(),
        path: format!("/{name}").into(),
        kind: EntryKind::File,
        size: Some(size),
        created: None,
        modified: None,
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    }
}

fn request(width: u32, height: u32, dark: bool) -> RenderRequest {
    RenderRequest {
        width,
        height,
        scale: 1.0,
        dark,
    }
}

/// The model of `bytes` as `format`, through the parser alone.
fn model(format: Format, bytes: Vec<u8>) -> Model {
    Model::parse(format, bytes, &HashMap::new()).unwrap_or_else(|e| panic!("{e}"))
}

/// Whether the process has a graphics adapter; says so when it has none,
/// and fails when one is required (`RICK_REQUIRE_WGPU`, CI on Linux and
/// Windows).
fn gpu_or_skip() -> bool {
    match render::gpu() {
        Ok(_) => true,
        Err(error) => {
            eprintln!("no graphics adapter ({error}); the frames are not rendered here");
            assert!(
                std::env::var_os("RICK_REQUIRE_WGPU").is_none(),
                "a graphics adapter is required here but none was found: {error}"
            );
            false
        }
    }
}

#[test]
fn scores_models_by_name_and_content() {
    let p = Previewer3d::default();
    let ascii = fixtures::cube_stl_ascii(10.0);
    let binary = fixtures::cube_stl_binary(10.0);
    let threemf = fixtures::cube_3mf(10.0, "millimeter");
    assert_eq!(p.score(&entry("cube.stl", ascii.len() as u64), &ascii), 200);
    assert_eq!(
        p.score(&entry("cube.STL", binary.len() as u64), &binary[..84]),
        200,
        "a binary STL is told by its size"
    );
    assert_eq!(p.score(&entry("cube.stl", 0), b""), 150);
    assert_eq!(p.score(&entry("cube.stl", 7), b"garbage"), 150);
    assert_eq!(p.score(&entry("model.obj", 20), b"v 0 0 0\nf 1 1 1\n"), 200);
    assert_eq!(p.score(&entry("model.obj", 3), b"\x00\x01\x02"), 60);
    assert_eq!(
        p.score(&entry("cube.3mf", threemf.len() as u64), &threemf[..64]),
        200
    );
    assert_eq!(p.score(&entry("cube.3mf", 5), b"hello"), 60);
    // Content alone claims a little, and only when it is unmistakable.
    assert_eq!(p.score(&entry("cube", ascii.len() as u64), &ascii), 120);
    assert_eq!(
        p.score(&entry("cube.bin", binary.len() as u64), &binary),
        120
    );
    assert_eq!(p.score(&entry("notes.txt", 12), b"solid advice"), 0);
    assert_eq!(p.score(&entry("photo.jpg", 100), b"\xFF\xD8\xFF"), 0);
    let mut dir = entry("models.stl", 0);
    dir.kind = EntryKind::Dir;
    assert_eq!(p.score(&dir, b""), 0);
    assert_eq!(p.id(), ID);
}

#[test]
fn an_ascii_cube_parses_to_twelve_triangles_of_ten_millimetres() {
    let model = model(Format::StlAscii, fixtures::cube_stl_ascii(10.0));
    assert_eq!(model.format, Format::StlAscii);
    assert_eq!(model.mesh.triangles(), 12);
    assert_eq!(model.mesh.bounds.min, [0.0, 0.0, 0.0]);
    assert_eq!(model.mesh.bounds.max, [10.0, 10.0, 10.0]);
    let doc = Doc3d::new(model, settings::DEFAULT_BUDGET, Some(1000));
    assert_eq!(doc.dimensions(), "10 × 10 × 10 mm");
    assert_eq!(
        doc.status(),
        ["STL (ASCII)", "12 triangles", "10 × 10 × 10 mm"]
    );
    let sections = doc.sections();
    assert_eq!(sections[0].title.key(), "preview-3d-file-info");
    assert!(
        sections[0]
            .rows
            .contains(&("Bounding box".to_owned(), "10 × 10 × 10 mm".to_owned())),
        "{:?}",
        sections[0].rows
    );
    assert!(doc.text_layer().is_none());
    assert!(doc.pages().is_none());
    assert!(doc.decimated().is_none());
}

#[test]
fn a_binary_stl_with_a_solid_header_parses_as_binary() {
    let bytes = fixtures::cube_stl_binary(12.5);
    assert!(bytes.starts_with(b"solid"));
    assert_eq!(
        stl::detect(&bytes[..84], bytes.len() as u64),
        Some(stl::Kind::Binary)
    );
    let model = model(Format::StlAscii, bytes);
    assert_eq!(
        model.format,
        Format::StlBinary,
        "the size decides, not the name"
    );
    assert_eq!(model.mesh.triangles(), 12);
    assert_eq!(model.mesh.bounds.max, [12.5, 12.5, 12.5]);
    assert_eq!(
        Doc3d::new(model, 100, None).dimensions(),
        "12.5 × 12.5 × 12.5 mm"
    );
    let torus = fixtures::torus_stl(1234);
    assert_eq!(stl::binary_count(&torus, torus.len() as u64), Some(1234));
    assert_eq!(self::model(Format::StlBinary, torus).mesh.triangles(), 1234);
    assert!(stl::parse(b"not a model at all".to_vec()).is_err());
}

#[test]
fn an_obj_with_three_objects_counts_them_and_takes_the_material_colours() {
    let (obj, mtl) = fixtures::three_cubes_obj();
    assert_eq!(obj::material_libraries(&obj), [fixtures::OBJ_MATERIALS]);
    assert_eq!(obj::count_objects(&obj), 3);
    let mut materials = HashMap::new();
    materials.insert(fixtures::OBJ_MATERIALS.to_owned(), mtl);
    let model =
        Model::parse(Format::Obj, obj.clone(), &materials).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(model.objects, Some(3));
    assert_eq!(model.mesh.triangles(), 36);
    assert_eq!(model.mesh.bounds.max, [40.0, 10.0, 10.0]);
    let colours: std::collections::BTreeSet<u32> =
        model.mesh.vertices.iter().map(|v| v.color).collect();
    assert_eq!(
        colours,
        [
            mesh::pack([255, 0, 0, 255]),
            mesh::pack([0, 255, 0, 255]),
            mesh::pack([0, 0, 255, 255])
        ]
        .into_iter()
        .collect()
    );
    let doc = Doc3d::new(model, settings::DEFAULT_BUDGET, None);
    assert_eq!(
        doc.status(),
        ["OBJ", "36 triangles", "40 × 10 × 10 mm", "3 objects"]
    );
    // Without the library the cubes are neutral and still three.
    let plain = Model::parse(Format::Obj, obj, &HashMap::new()).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        plain
            .mesh
            .vertices
            .iter()
            .all(|v| v.color == mesh::pack(mesh::NEUTRAL))
    );
    assert_eq!(obj::count_objects(b"g one\ng two\nv 0 0 0\n"), 2);
    assert_eq!(obj::count_objects(b"v 0 0 0\nf 1 1 1\n"), 1);
    assert_eq!(
        obj::material_libraries(b"mtllib ../x.mtl a/b.mtl ok.mtl\n"),
        ["ok.mtl"]
    );
}

#[test]
fn a_3mf_reads_its_unit_its_metadata_and_its_components() {
    let model = model(Format::ThreeMf, fixtures::cube_3mf(10.0, "millimeter"));
    assert_eq!(model.unit, Some(Unit::Millimeter));
    assert_eq!(model.mesh.triangles(), 12);
    assert_eq!(model.mesh.bounds.max, [10.0, 10.0, 10.0]);
    assert_eq!(model.objects, Some(1));
    assert_eq!(
        model.metadata,
        [
            ("Title".to_owned(), "Cube".to_owned()),
            ("Designer".to_owned(), "Rick".to_owned())
        ]
    );
    let doc = Doc3d::new(model, settings::DEFAULT_BUDGET, None);
    assert_eq!(doc.status(), ["3MF", "12 triangles", "10 × 10 × 10 mm"]);
    let rows = doc.sections()[0].rows.clone();
    assert!(
        rows.contains(&("Units".to_owned(), "millimeter".to_owned())),
        "{rows:?}"
    );
    assert!(
        rows.contains(&("Title".to_owned(), "Cube".to_owned())),
        "{rows:?}"
    );
    assert!(
        rows.contains(&("Designer".to_owned(), "Rick".to_owned())),
        "{rows:?}"
    );

    let inches = self::model(Format::ThreeMf, fixtures::cube_3mf(1.0, "inch"));
    assert_eq!(inches.unit, Some(Unit::Inch));
    assert_eq!(
        Doc3d::new(inches, settings::DEFAULT_BUDGET, None).dimensions(),
        "25.4 × 25.4 × 25.4 mm"
    );

    let components = self::model(Format::ThreeMf, fixtures::components_3mf());
    assert_eq!(components.mesh.triangles(), 24);
    assert_eq!(components.mesh.bounds.min, [0.0, 0.0, 0.0]);
    assert_eq!(components.mesh.bounds.max, [10.0, 30.0, 10.0]);

    assert!(matches!(
        threemf::parse(b"PK\x03\x04 not really"),
        Err(threemf::ThreeMfError::Zip(_))
    ));
    assert!(matches!(
        threemf::parse(&fixtures::package_3mf("<other/>")),
        Err(threemf::ThreeMfError::NotAModel)
    ));
    assert_eq!(Unit::parse("furlong"), None);
    assert_eq!(Unit::Meter.millimetres(), 1000.0);
}

#[test]
fn a_model_over_the_budget_is_decimated_and_says_so() {
    let model = model(Format::StlBinary, fixtures::torus_stl(20_000));
    assert_eq!(model.mesh.triangles(), 20_000);
    let bounds = model.mesh.bounds;
    assert!(model.mesh.decimate(20_000).is_none(), "within the budget");
    let smaller = model
        .mesh
        .decimate(5_000)
        .unwrap_or_else(|| panic!("over the budget"));
    assert!(smaller.triangles() <= 5_000, "{}", smaller.triangles());
    assert!(smaller.triangles() > 500, "{}", smaller.triangles());
    // The clustered mesh stays inside the original's box.
    for axis in 0..3 {
        assert!(smaller.bounds.min[axis] >= bounds.min[axis] - 1e-3);
        assert!(smaller.bounds.max[axis] <= bounds.max[axis] + 1e-3);
    }
    let doc = Doc3d::new(model, 5_000, None);
    assert_eq!(doc.triangles(), 20_000, "the count shown is the file's");
    let shown = doc.decimated().unwrap_or_else(|| panic!("decimated"));
    assert!(shown <= 5_000);
    let status = doc.status();
    assert_eq!(status[1], "20 000 triangles");
    assert_eq!(
        status[3],
        format!("decimated to {} triangles for the preview", count(shown))
    );
    assert!(
        doc.sections()[0]
            .rows
            .iter()
            .any(|(k, v)| k == "Preview" && v.contains("budget 5 000")),
        "{:?}",
        doc.sections()[0].rows
    );
}

#[test]
fn the_camera_orbits_zooms_and_resets_through_the_controls() {
    let mut doc = Doc3d::new(
        model(Format::StlAscii, fixtures::cube_stl_ascii(10.0)),
        settings::DEFAULT_BUDGET,
        None,
    );
    assert_eq!(
        doc.controls(),
        [
            PreviewControl::Pointer {
                id: ORBIT,
                gesture: ext::Gesture::Drag,
                label: "Orbit"
            },
            PreviewControl::Pointer {
                id: ZOOM,
                gesture: ext::Gesture::Wheel,
                label: "Zoom"
            },
            PreviewControl::Pointer {
                id: RESET,
                gesture: ext::Gesture::DoubleClick,
                label: "Reset view"
            },
        ]
    );
    let start = doc.camera();
    assert_eq!(start, Camera::default());
    doc.set_control(ORBIT, ControlValue::Delta { x: 50.0, y: -20.0 })
        .unwrap_or_else(|e| panic!("{e}"));
    let turned = doc.camera();
    assert!(
        turned.yaw < start.yaw,
        "dragging right turns the model right"
    );
    assert!(turned.pitch > start.pitch, "dragging up shows the top");
    doc.set_control(ZOOM, ControlValue::Delta { x: 0.0, y: 100.0 })
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(doc.camera().zoom > 1.0, "scrolling up comes closer");
    doc.set_control(ZOOM, ControlValue::Delta { x: 0.0, y: -200.0 })
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(doc.camera().zoom < 1.0);
    doc.set_control(RESET, ControlValue::Pressed)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(doc.camera(), Camera::default());
    assert!(doc.set_control(ORBIT, ControlValue::Pressed).is_err());
    assert!(
        doc.set_control(ext::ControlId("other"), ControlValue::Bool(true))
            .is_err()
    );
    // The pitch never passes the pole and the zoom stays in bounds.
    let mut camera = Camera::default();
    camera.orbit(0.0, -100_000.0);
    assert!(camera.pitch <= 1.56);
    for _ in 0..100 {
        camera.zoom_by(10_000.0);
    }
    assert_eq!(camera.zoom, camera::MAX_ZOOM);
    // The eye looks down at the centre from the distance that fits.
    let eye = camera::Camera::default().eye([5.0; 3], 8.66);
    assert!(eye[2] > 5.0, "above the centre: {eye:?}");
    assert!(math::length(math::sub(eye, [5.0; 3])) > 8.66);
}

/// Whether two frames differ in any pixel.
fn differs(a: &Frame, b: &Frame) -> bool {
    match (a, b) {
        (Frame::Rgba { data: x, .. }, Frame::Rgba { data: y, .. }) => x != y,
        _ => true,
    }
}

/// The pixel at `(x, y)` of a frame.
fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 4] {
    let Frame::Rgba { width, data, .. } = frame else {
        panic!("not pixels: {frame:?}");
    };
    let at = ((y * width + x) * 4) as usize;
    [data[at], data[at + 1], data[at + 2], data[at + 3]]
}

#[test]
fn a_frame_has_the_requested_size_and_orbiting_changes_it() {
    if !gpu_or_skip() {
        return;
    }
    let mut doc = Doc3d::new(
        model(Format::StlAscii, fixtures::cube_stl_ascii(10.0)),
        settings::DEFAULT_BUDGET,
        None,
    );
    let frame = doc
        .render(0, request(320, 200, false))
        .unwrap_or_else(|e| panic!("{e}"));
    let Frame::Rgba {
        width,
        height,
        data,
    } = &frame
    else {
        panic!("not pixels: {frame:?} ({:?})", doc.failure());
    };
    assert_eq!((*width, *height), (320, 200));
    assert_eq!(data.len(), 320 * 200 * 4);
    assert!(doc.failure().is_none());
    // The corner is the light background, the middle the shaded cube.
    let corner = pixel(&frame, 1, 1);
    let middle = pixel(&frame, 160, 100);
    assert!(corner[0] > 230 && corner[3] == 255, "{corner:?}");
    assert!(middle != corner, "the cube covers the middle: {middle:?}");
    assert!(
        middle[0] < 220 && middle[0] > 40,
        "a grey, lit solid: {middle:?}"
    );
    // The same view renders the same; another size, another frame.
    let again = doc
        .render(0, request(320, 200, false))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!differs(&frame, &again));
    let dark = doc
        .render(0, request(320, 200, true))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(pixel(&dark, 1, 1)[0] < 60, "{:?}", pixel(&dark, 1, 1));
    let small = doc
        .render(0, request(64, 64, false))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        small,
        Frame::Rgba {
            width: 64,
            height: 64,
            ..
        }
    ));
    // Orbit: a different frame; reset: the first one again.
    doc.set_control(ORBIT, ControlValue::Delta { x: 60.0, y: -30.0 })
        .unwrap_or_else(|e| panic!("{e}"));
    let orbited = doc
        .render(0, request(320, 200, false))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(differs(&frame, &orbited), "orbit changes the frame");
    doc.set_control(ZOOM, ControlValue::Delta { x: 0.0, y: 120.0 })
        .unwrap_or_else(|e| panic!("{e}"));
    let zoomed = doc
        .render(0, request(320, 200, false))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(differs(&orbited, &zoomed), "zoom changes the frame");
    doc.set_control(RESET, ControlValue::Pressed)
        .unwrap_or_else(|e| panic!("{e}"));
    let reset = doc
        .render(0, request(320, 200, false))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!differs(&frame, &reset), "reset brings the first view back");
    assert!(doc.render(1, request(10, 10, false)).is_err());
    assert!(
        doc.sections()[0]
            .rows
            .iter()
            .any(|(k, v)| k == "Renderer" && v.contains("through")),
        "{:?}",
        doc.sections()[0].rows
    );
}

#[test]
fn the_obj_and_3mf_render_too_and_an_empty_model_shows_its_grid() {
    if !gpu_or_skip() {
        return;
    }
    let (obj, mtl) = fixtures::three_cubes_obj();
    let mut materials = HashMap::new();
    materials.insert(fixtures::OBJ_MATERIALS.to_owned(), mtl);
    let cubes = Model::parse(Format::Obj, obj, &materials).unwrap_or_else(|e| panic!("{e}"));
    let mut doc = Doc3d::new(cubes, settings::DEFAULT_BUDGET, None);
    let frame = doc
        .render(0, request(300, 120, false))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        frame,
        Frame::Rgba {
            width: 300,
            height: 120,
            ..
        }
    ));
    // Three colours somewhere along the middle row.
    let Frame::Rgba { data, .. } = &frame else {
        panic!("not pixels")
    };
    let row: Vec<[i32; 3]> = (0..300)
        .map(|x| pixel(&frame, x, 60))
        .map(|p| [i32::from(p[0]), i32::from(p[1]), i32::from(p[2])])
        .collect();
    let reddish = row.iter().any(|p| p[0] > p[1] + 60 && p[0] > p[2] + 60);
    let greenish = row.iter().any(|p| p[1] > p[0] + 60 && p[1] > p[2] + 60);
    let bluish = row.iter().any(|p| p[2] > p[0] + 60 && p[2] > p[1] + 60);
    assert!(reddish && greenish && bluish, "{}", data.len());

    let mut threemf = Doc3d::new(
        model(Format::ThreeMf, fixtures::cube_3mf(10.0, "millimeter")),
        settings::DEFAULT_BUDGET,
        None,
    );
    let frame = threemf
        .render(0, request(100, 100, true))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        frame,
        Frame::Rgba {
            width: 100,
            height: 100,
            ..
        }
    ));

    let empty = Model {
        format: Format::Obj,
        mesh: Mesh::default(),
        objects: Some(0),
        unit: None,
        metadata: Vec::new(),
    };
    let mut doc = Doc3d::new(empty, settings::DEFAULT_BUDGET, None);
    assert_eq!(doc.dimensions(), "0 × 0 × 0 mm");
    let frame = doc
        .render(0, request(80, 60, false))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        frame,
        Frame::Rgba {
            width: 80,
            height: 60,
            ..
        }
    ));
}

#[test]
fn a_request_is_capped_at_4096_a_side_keeping_the_aspect() {
    assert_eq!(capped(800, 600), (800, 600));
    assert_eq!(capped(0, 0), (1, 1));
    assert_eq!(capped(8192, 8192), (4096, 4096));
    assert_eq!(capped(8192, 2048), (4096, 1024));
    assert_eq!(capped(1000, 10_000), (410, 4096));
}

#[test]
fn the_triangle_budget_comes_from_the_settings_file() {
    assert_eq!(
        settings::parse_budget("triangle_budget = 500000\n"),
        Some(500_000)
    );
    assert_eq!(
        settings::parse_budget("triangle_budget = 1\n"),
        Some(1_000),
        "clamped to the minimum"
    );
    assert_eq!(
        settings::parse_budget("triangle_budget = 99_999_999_999\n"),
        Some(20_000_000),
        "clamped to the maximum"
    );
    assert_eq!(settings::parse_budget("triangle_budget = \"many\"\n"), None);
    assert_eq!(settings::parse_budget("other = 3\n"), None);
    assert_eq!(settings::parse_budget("not toml ="), None);
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        settings::triangle_budget(Some(dir.path())),
        settings::DEFAULT_BUDGET,
        "no file"
    );
    std::fs::write(
        dir.path().join(settings::FILE),
        "triangle_budget = 250000\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(settings::triangle_budget(Some(dir.path())), 250_000);
    assert_eq!(settings::triangle_budget(None), settings::DEFAULT_BUDGET);
    assert_eq!(
        Previewer3d::new(Some(dir.path().to_owned())).triangle_budget(),
        250_000
    );
    assert_eq!(Previewer3d::with_budget(9_000).triangle_budget(), 9_000);
    let schema = settings::schema();
    assert_eq!(schema.ext, ID);
    assert_eq!(schema.fields[0].key, settings::KEY);
    assert!(matches!(
        schema.fields[0].kind,
        ext::SettingKind::Integer {
            default: 2_000_000,
            ..
        }
    ));
}

#[test]
fn numbers_are_shown_the_human_way() {
    assert_eq!(millimetres(10.0), "10");
    assert_eq!(millimetres(12.5), "12.5");
    assert_eq!(millimetres(0.126), "0.13");
    assert_eq!(millimetres(-0.001), "0");
    assert_eq!(count(999), "999");
    assert_eq!(count(1_000), "1 000");
    assert_eq!(count(2_000_000), "2 000 000");
    assert_eq!(render::nice_step(0.9), 1.0);
    assert_eq!(render::nice_step(1.4), 1.0);
    assert_eq!(render::nice_step(2.9), 2.0);
    assert_eq!(render::nice_step(6.0), 5.0);
    assert_eq!(render::nice_step(80.0), 100.0);
    let grid = Grid::under(&Aabb {
        min: [0.0; 3],
        max: [10.0; 3],
    });
    assert_eq!(grid.step, 1.0);
    assert!(grid.half >= 10.0, "{}", grid.half);
    assert!(grid.vertices.iter().all(|v| v.position[2] == 0.0));
    assert!(grid.vertices.iter().any(|v| v.color == 1));
}

#[test]
fn the_matrices_project_as_expected() {
    let view = math::Mat4::look_at([0.0, -10.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0]);
    let at = view.transform([0.0; 3]);
    assert!(
        (at[2] + 10.0).abs() < 1e-5,
        "{at:?}: the centre is 10 ahead"
    );
    let proj = math::Mat4::perspective(camera::FOV_Y, 1.0, 1.0, 100.0);
    let clip = (proj * view).transform([0.0; 3]);
    let depth = clip[2] / clip[3];
    assert!((0.0..=1.0).contains(&depth), "{depth}");
    assert_eq!(math::Mat4::IDENTITY * view, view);
    assert_eq!(math::normalize([0.0; 3]), [0.0; 3]);
    assert_eq!(
        math::cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        [0.0, 0.0, 1.0]
    );
}

/// Opens `name` in `dir` through the local provider.
fn open_local(previewer: &Previewer3d, dir: &std::path::Path, name: &str) -> Box<dyn PreviewDoc> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
    let loc = Location::new(
        vfs_local::PROVIDER_ID,
        platform::paths::from_path(&dir.join(name)),
    );
    runtime::block_on(previewer.open(&loc, provider, CancelToken::new()))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn files_open_through_the_provider_with_their_material_libraries() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (obj, mtl) = fixtures::three_cubes_obj();
    std::fs::write(dir.path().join("cubes.obj"), obj).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join(fixtures::OBJ_MATERIALS), mtl).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("cube.stl"), fixtures::cube_stl_ascii(10.0))
        .unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(
        dir.path().join("cube.3mf"),
        fixtures::cube_3mf(2.0, "centimeter"),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("torus.stl"), fixtures::torus_stl(3_000))
        .unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("notes.stl"), b"nothing like a model")
        .unwrap_or_else(|e| panic!("{e}"));

    let previewer = Previewer3d::with_budget(1_000);
    let doc = open_local(&previewer, dir.path(), "cubes.obj");
    assert_eq!(
        doc.status(),
        ["OBJ", "36 triangles", "40 × 10 × 10 mm", "3 objects"]
    );
    let doc = open_local(&previewer, dir.path(), "cube.stl");
    assert_eq!(doc.status()[1], "12 triangles");
    let doc = open_local(&previewer, dir.path(), "cube.3mf");
    assert_eq!(doc.status()[2], "20 × 20 × 20 mm");
    let doc = open_local(&previewer, dir.path(), "torus.stl");
    assert_eq!(doc.status()[1], "3 000 triangles");
    assert!(
        doc.status()[3].starts_with("decimated to"),
        "{:?}",
        doc.status()
    );

    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
    let loc = Location::new(
        vfs_local::PROVIDER_ID,
        platform::paths::from_path(&dir.path().join("notes.stl")),
    );
    let error = runtime::block_on(previewer.open(&loc, provider, CancelToken::new()))
        .err()
        .unwrap_or_else(|| panic!("opened"));
    assert!(
        matches!(error, types::Error::Extension { ext, .. } if ext == ID),
        "{error}"
    );
}

#[test]
fn the_extension_registers_a_previewer_and_the_setting() {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let _ = fake.config_dir();
    let mut registry = ext::Registry::with_platform(fake);
    registry
        .load(&Preview3d::new())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(registry.previewers().len(), 1);
    assert_eq!(registry.settings_schemas().len(), 1);
    assert_eq!(registry.extensions(), &[ID]);
    assert_eq!(Preview3d::default().manifest().points, Points::PREVIEWER);
}
