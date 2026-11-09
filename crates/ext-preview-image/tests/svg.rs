//! Acceptance (issue 83): SVG and SVGZ in the image previewer. A document
//! is rasterised at the size the pane asks for and again at a new size,
//! never from an upscaled bitmap, and at the intrinsic size times the
//! display scale for actual pixels; a viewBox-only document shows its
//! viewBox and one with neither renders at 300 × 150; an SVGZ renders and
//! its text layer is the decompressed XML; *Rendered* off shows the
//! highlighted source and on again the picture; a malformed document falls
//! to the source with the error in the footer; references outside the file
//! are not followed; and a hundred-thousand-unit viewBox renders at the
//! cap with the footer saying so.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::{ControlValue, Frame, PreviewControl, PreviewDoc, Previewer, Provider, RenderRequest};
use ext_preview_image::ImagePreviewer;
use ext_preview_image::svg::{ACTUAL_SIZE, DEFAULT_SIZE, Intrinsic, MAX_RASTER, RENDERED, SvgDoc};
use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Location, ProviderId};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn open_fixture(name: &str) -> SvgDoc {
    let bytes = fs::read(fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    SvgDoc::new(name, &bytes).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn req(width: u32, height: u32) -> RenderRequest {
    RenderRequest {
        width,
        height,
        scale: 1.0,
        dark: false,
    }
}

/// The pane's request for actual pixels at `scale`.
fn actual(scale: f32) -> RenderRequest {
    RenderRequest {
        width: ACTUAL_SIZE,
        height: ACTUAL_SIZE,
        scale,
        dark: false,
    }
}

/// The pixels of a rendered frame.
fn pixels(doc: &mut SvgDoc, req: RenderRequest) -> (u32, u32, Arc<[u8]>) {
    match doc.render(0, req) {
        Ok(Frame::Rgba {
            width,
            height,
            data,
        }) => (width, height, data),
        other => panic!("not pixels: {other:?}"),
    }
}

/// The RGBA of pixel (`x`, `y`) of a frame `width` wide.
fn at(data: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    [data[i], data[i + 1], data[i + 2], data[i + 3]]
}

fn entry(name: &str) -> Entry {
    Entry {
        id: EntryId::new(ProviderId("t"), 1),
        name: name.to_owned(),
        path: format!("/{name}").into(),
        kind: EntryKind::File,
        size: Some(1),
        created: None,
        modified: None,
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    }
}

/// Opens `path` through the local provider, as the pane does.
fn open_through_provider(path: &Path) -> Box<dyn PreviewDoc> {
    let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
    let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path));
    runtime::block_on(ImagePreviewer.open(&loc, provider, CancelToken::new()))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Selecting an `.svg` shows the picture filling the pane in Fit, sharp;
/// a new pane size is a new rasterisation at that size; 1:1 is the
/// intrinsic size at the display scale; nothing comes from an upscaled
/// texture.
#[test]
fn fit_rasterises_at_the_requested_size_and_again_at_a_new_one_and_actual_size_at_the_intrinsic() {
    let bytes = fs::read(fixture("disc.svg")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        ImagePreviewer.score(&entry("disc.svg"), &bytes),
        200,
        "claimed by content"
    );
    assert_eq!(
        ImagePreviewer.score(&entry("disc.svg"), b"<html><body/></html>"),
        0,
        "never by the extension alone"
    );
    let mut doc = open_fixture("disc.svg");
    assert_eq!(doc.intrinsic(), &Intrinsic::Size(64.0, 64.0));
    assert_eq!(doc.status(), ["SVG", "64 × 64 px", "336 B"]);

    // Fit into a 128 × 96 pane: the limiting axis fills the request.
    let (w, h, data) = pixels(&mut doc, req(128, 96));
    assert_eq!((w, h), (96, 96));
    assert_eq!(at(&data, w, 48, 48), [255, 0, 0, 255], "the disc's centre");
    assert_eq!(at(&data, w, 1, 1)[3], 0, "transparent outside the disc");
    let corner = at(&data, w, 90, 90);
    assert!(
        corner[2] > 200 && (100..=160).contains(&corner[3]),
        "the translucent square keeps its alpha: {corner:?}"
    );

    // A bigger pane: rasterised again at the new size, not scaled up. At
    // 512 px the disc's radius is 192 px around 256: a pixel four short of
    // the edge is solid, one four past it is empty, which no 8× stretch of
    // a 96 px bitmap would give.
    let (w, h, data) = pixels(&mut doc, req(512, 512));
    assert_eq!((w, h), (512, 512));
    assert_eq!(at(&data, w, 256 + 188, 256), [255, 0, 0, 255]);
    assert_eq!(at(&data, w, 256 + 196, 256)[3], 0);
    assert!(!doc.capped());

    // Actual pixels: the intrinsic size at the display scale.
    let (w, h, _) = pixels(&mut doc, actual(1.0));
    assert_eq!((w, h), (64, 64));
    let (w, h, _) = pixels(&mut doc, actual(2.0));
    assert_eq!((w, h), (128, 128));
    assert!(doc.text_layer().is_some(), "the source is the text layer");
    assert_eq!(doc.pages(), None);
}

/// A viewBox-only document renders and the footer shows the viewBox; a
/// document with neither renders at 300 × 150.
#[test]
fn a_viewbox_only_document_shows_its_viewbox_and_one_without_any_size_renders_at_300_by_150() {
    let mut doc = open_fixture("viewbox-only.svg");
    assert_eq!(
        doc.intrinsic(),
        &Intrinsic::ViewBox("0 0 24 24".to_owned(), 24.0, 24.0)
    );
    assert_eq!(doc.status()[..2], ["SVG", "viewBox 0 0 24 24"]);
    let (w, h, data) = pixels(&mut doc, actual(1.0));
    assert_eq!((w, h), (24, 24));
    assert_eq!(at(&data, w, 12, 12), [0, 170, 0, 255]);
    let (w, h, _) = pixels(&mut doc, req(240, 480));
    assert_eq!((w, h), (240, 240), "fitted by its viewBox's aspect");
    let image = &doc.sections()[1];
    assert_eq!(image.id, "image");
    assert!(
        image
            .rows
            .iter()
            .any(|(k, v)| k == "viewBox" && v == "0 0 24 24"),
        "{:?}",
        image.rows
    );

    let mut doc = open_fixture("no-size.svg");
    assert_eq!(doc.intrinsic(), &Intrinsic::Default);
    assert_eq!(doc.status()[1], "300 × 150 px");
    let (w, h, data) = pixels(&mut doc, actual(1.0));
    assert_eq!((w, h), (300, 150));
    assert_eq!(
        (f64::from(w), f64::from(h)),
        (f64::from(DEFAULT_SIZE.0), f64::from(DEFAULT_SIZE.1))
    );
    assert_eq!(at(&data, w, 150, 75), [0x33, 0x66, 0x99, 255]);
}

/// An `.svgz` renders; its text layer, what *Copy* puts on the clipboard,
/// is the decompressed XML.
#[test]
fn an_svgz_renders_and_its_text_layer_is_the_decompressed_xml() {
    let path = fixture("disc.svgz");
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{e}"));
    assert!(bytes.starts_with(&[0x1f, 0x8b]), "the fixture is gzip");
    assert_eq!(ImagePreviewer.score(&entry("disc.svgz"), &bytes[..64]), 200);
    assert_eq!(
        ImagePreviewer.score(&entry("disc.gz"), &bytes[..64]),
        0,
        "gzip alone is not an SVGZ"
    );

    let mut doc = open_through_provider(&path);
    let xml = fs::read_to_string(fixture("disc.svg")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        doc.text_layer().map(|l| l.pages.clone()),
        Some(vec![xml.clone()]),
        "the decompressed source"
    );
    let Ok(Frame::Rgba { width, height, .. }) = doc.render(0, actual(1.0)) else {
        panic!("not pixels")
    };
    assert_eq!((width, height), (64, 64));
    assert_eq!(doc.status()[0], "SVGZ");
    assert_eq!(doc.status()[1], "64 × 64 px");
    assert_eq!(doc.status()[2], "255 B", "the file's size on disk");
    let image = &doc.sections()[1];
    assert!(
        image
            .rows
            .iter()
            .any(|(k, v)| k == "Compressed" && v == "255 B"),
        "{:?}",
        image.rows
    );
    assert!(
        image
            .rows
            .iter()
            .any(|(k, v)| k == "Uncompressed" && *v == format!("{} B", xml.len())),
        "{:?}",
        image.rows
    );
    // Rendered off: the same decompressed text, highlighted.
    doc.set_control(RENDERED, ControlValue::Bool(false))
        .unwrap_or_else(|e| panic!("{e}"));
    let Ok(Frame::Text(block)) = doc.render(0, req(400, 400)) else {
        panic!("not text")
    };
    assert_eq!(block.text, xml);
}

/// *Rendered* off shows the highlighted XML; on again, the picture.
#[test]
fn rendered_off_shows_the_highlighted_xml_and_on_again_the_picture() {
    let mut doc = open_fixture("disc.svg");
    assert!(doc.rendered(), "on for every newly shown file");
    assert!(matches!(
        doc.controls().as_slice(),
        [PreviewControl::Toggle {
            id: RENDERED,
            label: "Rendered",
            on: true,
            ..
        }]
    ));
    assert!(matches!(
        doc.render(0, req(100, 100)),
        Ok(Frame::Rgba { .. })
    ));

    doc.set_control(RENDERED, ControlValue::Bool(false))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!doc.rendered());
    let Ok(Frame::Text(block)) = doc.render(0, req(100, 100)) else {
        panic!("not text")
    };
    assert_eq!(block.text, doc.text());
    assert!(block.text.contains("<circle"));
    assert_eq!(block.language.as_deref(), Some("XML"));
    assert!(!block.spans.is_empty(), "highlighted");
    let controls = doc.controls();
    assert!(matches!(
        controls[0],
        PreviewControl::Toggle {
            id: RENDERED,
            on: false,
            ..
        }
    ));
    assert!(
        controls.len() > 1,
        "the source's own controls while it shows: {controls:?}"
    );

    doc.set_control(RENDERED, ControlValue::Bool(true))
        .unwrap_or_else(|e| panic!("{e}"));
    let (w, h, _) = pixels(&mut doc, req(100, 100));
    assert_eq!((w, h), (100, 100));
    assert!(
        doc.set_control(RENDERED, ControlValue::Pressed).is_err(),
        "a toggle takes on or off"
    );
}

/// A malformed document shows the source with the error in the footer and
/// no panic; *Rendered* stays available.
#[test]
fn a_malformed_document_shows_the_source_with_the_error_in_the_footer() {
    let mut doc = open_fixture("broken.svg");
    let error = doc
        .parse_error()
        .unwrap_or_else(|| panic!("usvg accepted a cut-off document"))
        .to_owned();
    assert!(error.starts_with("not rendered: "), "{error}");
    assert!(
        error.contains("line ") && error.contains("column "),
        "the parser's position: {error}"
    );
    assert_eq!(
        doc.status(),
        ["SVG".to_owned(), "117 B".to_owned(), error.clone()],
        "no size claimed for a document that did not parse"
    );
    assert!(
        !doc.sections()[0]
            .rows
            .iter()
            .any(|(k, _)| k == "Dimensions"),
        "{:?}",
        doc.sections()
    );
    let Ok(Frame::Text(block)) = doc.render(0, req(200, 200)) else {
        panic!("not text")
    };
    assert!(block.text.contains("<rect"));
    assert!(matches!(
        doc.controls()[0],
        PreviewControl::Toggle {
            id: RENDERED,
            on: true,
            ..
        }
    ));
    // Switching the toggle changes nothing and breaks nothing.
    doc.set_control(RENDERED, ControlValue::Bool(false))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(doc.render(0, req(200, 200)), Ok(Frame::Text(_))));
    doc.set_control(RENDERED, ControlValue::Bool(true))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(doc.render(0, req(200, 200)), Ok(Frame::Text(_))));

    // Not XML at all, and empty: no panic either.
    for bytes in [&b"<svg><"[..], b"<svg", b"", b"\xff\xfe\x00"] {
        let mut doc = SvgDoc::new("x.svg", bytes).unwrap_or_else(|e| panic!("{e}"));
        assert!(doc.parse_error().is_some());
        assert!(matches!(doc.render(0, req(50, 50)), Ok(Frame::Text(_))));
    }
}

/// An SVG referencing `file:///etc/passwd`, an `https://` image, an
/// external style sheet or a picture beside it renders without any of them.
#[test]
fn references_outside_the_file_are_not_followed() {
    let mut doc = open_fixture("external.svg");
    assert!(doc.parse_error().is_none());
    let (w, h, data) = pixels(&mut doc, actual(1.0));
    assert_eq!((w, h), (32, 32));
    assert_eq!(at(&data, w, 4, 16), [0, 0, 255, 255], "the rect is drawn");
    assert_eq!(at(&data, w, 28, 16)[3], 0, "nothing where the images go");

    // A real picture on this disk, referenced by its absolute path, by a
    // file URL and by a relative name: none of it is read.
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let png = dir.path().join("red.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
        .save(&png)
        .unwrap_or_else(|e| panic!("{e}"));
    let absolute = png.to_string_lossy().replace('\\', "/");
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"8\" height=\"8\">\
         <image width=\"8\" height=\"8\" href=\"{absolute}\"/>\
         <image width=\"8\" height=\"8\" xlink:href=\"file://{absolute}\"/>\
         <image width=\"8\" height=\"8\" href=\"red.png\"/>\
         <image width=\"8\" height=\"8\" href=\"https://example.invalid/red.png\"/>\
         </svg>"
    );
    let svg_path = dir.path().join("refs.svg");
    fs::write(&svg_path, &svg).unwrap_or_else(|e| panic!("{e}"));
    let mut doc = open_through_provider(&svg_path);
    let Ok(Frame::Rgba { width, data, .. }) = doc.render(0, actual(1.0)) else {
        panic!("not pixels")
    };
    assert!(
        data.chunks_exact(4).all(|px| px[3] == 0),
        "a referenced picture was drawn: {:?}",
        at(&data, width, 4, 4)
    );

    // Embedded data is honoured: the same picture as a data URI shows.
    let mut encoded = Vec::new();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Png,
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let base64 = {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in encoded.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(char::from(TABLE[((n >> (18 - 6 * i)) & 63) as usize]));
                } else {
                    out.push('=');
                }
            }
        }
        out
    };
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\">\
         <image width=\"8\" height=\"8\" href=\"data:image/png;base64,{base64}\"/></svg>"
    );
    let mut doc = SvgDoc::new("data.svg", svg.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    let (w, _, data) = pixels(&mut doc, actual(1.0));
    assert_eq!(
        at(&data, w, 4, 4),
        [255, 0, 0, 255],
        "the data URI is drawn"
    );
}

/// A `viewBox="0 0 100000 100000"` document renders at the cap, the footer
/// says so, and memory stays bounded by the cap.
#[test]
fn a_huge_viewbox_renders_at_the_cap_and_the_footer_says_so() {
    let mut doc = open_fixture("huge.svg");
    assert_eq!(doc.status()[1], "viewBox 0 0 100000 100000");
    // Fitted into a pane: nothing to cap.
    let (w, h, data) = pixels(&mut doc, req(400, 300));
    assert_eq!((w, h), (300, 300));
    assert_eq!(at(&data, w, 150, 150), [0, 0x33, 0x66, 255]);
    assert!(!doc.capped());
    assert!(!doc.status().iter().any(|s| s.contains("cap")));

    // Actual size: the long edge at the cap, no more.
    let (w, h, data) = pixels(&mut doc, actual(1.0));
    assert_eq!((w, h), (MAX_RASTER, MAX_RASTER));
    assert_eq!(data.len(), (MAX_RASTER * MAX_RASTER * 4) as usize);
    assert_eq!(at(&data, w, 4096, 4096), [0, 0x33, 0x66, 255]);
    assert_eq!(at(&data, w, 10, 10), [255, 0xcc, 0, 255]);
    assert!(doc.capped());
    assert!(
        doc.status()
            .contains(&format!("rendered at {MAX_RASTER} px cap")),
        "{:?}",
        doc.status()
    );
    drop(data);

    // A wide document is capped on its long edge alone.
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20000\" height=\"100\"/>";
    let mut doc = SvgDoc::new("wide.svg", svg.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    let (w, h, _) = pixels(&mut doc, actual(1.0));
    assert_eq!((w, h), (MAX_RASTER, 41));
    assert!(doc.capped());
}

/// Text is set through the system fonts without a panic, whatever fonts
/// this machine has.
#[test]
fn text_renders_through_the_system_fonts() {
    let mut doc = open_fixture("text.svg");
    assert!(doc.parse_error().is_none());
    let (w, h, _) = pixels(&mut doc, actual(1.0));
    assert_eq!((w, h), (120, 40));
}
