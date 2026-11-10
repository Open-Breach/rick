//! Acceptance (issue 84): the icon previewer over the fixtures in
//! `tests/fixtures/`. A multi-entry ICO previews largest first with every
//! variant a page, each at a whole-number magnification in Fit and at
//! actual pixels for the 1:1 request; a CUR shows its hotspot crosshair at
//! the right pixel and the toggle hides it; an ICNS with a JPEG 2000 entry
//! previews its other variants and lists and counts the skipped one; an
//! XPM with named colours and `None` renders with transparency and its
//! source is the text layer; a single-variant ICO has no pager; a
//! truncated ICO does not panic and a file with no decodable variant gives
//! a clear error. The *Plugin available* placeholder is checked in the
//! `tests` crate, where the pane is.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::{ControlValue, Frame, PreviewControl, PreviewDoc, Previewer, Provider, RenderRequest};
use ext_preview_icons::{ACTUAL_SIZE, Format, HOTSPOT, IconDoc, IconsPreviewer, Variant};
use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Location, ProviderId};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn bytes(name: &str) -> Vec<u8> {
    fs::read(fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn open(name: &str) -> IconDoc {
    IconDoc::new(&bytes(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn req(width: u32, height: u32) -> RenderRequest {
    RenderRequest {
        width,
        height,
        scale: 1.0,
        dark: false,
    }
}

/// The pane's request for actual pixels.
fn actual() -> RenderRequest {
    req(ACTUAL_SIZE, ACTUAL_SIZE)
}

fn pixels(doc: &mut IconDoc, page: usize, req: RenderRequest) -> (u32, u32, Arc<[u8]>) {
    match doc.render(page, req) {
        Ok(Frame::Rgba {
            width,
            height,
            data,
        }) => (width, height, data),
        other => panic!("page {page}: not pixels: {other:?}"),
    }
}

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
fn open_through_provider(path: &Path) -> Result<Box<dyn PreviewDoc>, types::Error> {
    let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
    let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path));
    runtime::block_on(IconsPreviewer.open(&loc, provider, CancelToken::new()))
}

/// Every frame pixel equals the source pixel it magnifies: crisp squares,
/// no filtering.
fn assert_magnified(frame: &[u8], fw: u32, source: &[u8], sw: u32, sh: u32, factor: u32) {
    assert_eq!(fw, sw * factor);
    assert_eq!(frame.len(), (sw * sh * factor * factor * 4) as usize);
    for y in 0..sh * factor {
        for x in 0..fw {
            assert_eq!(
                at(frame, fw, x, y),
                at(source, sw, x / factor, y / factor),
                "frame pixel {x}, {y} at factor {factor}"
            );
        }
    }
}

/// A multi-entry ICO with BMP and PNG entries previews with the largest
/// variant first; the pager steps through every variant; each is drawn
/// at a whole-number magnification with crisp pixels in Fit and at actual
/// pixels in 1:1.
#[test]
fn a_multi_entry_ico_previews_largest_first_with_every_variant_a_page_magnified_whole() {
    let ico = bytes("multi.ico");
    assert_eq!(IconsPreviewer.score(&entry("multi.ico"), &ico[..64]), 200);
    assert_eq!(
        IconsPreviewer.score(&entry("multi.bin"), &ico[..64]),
        200,
        "by magic, whatever the name"
    );
    assert_eq!(
        IconsPreviewer.score(&entry("x.ico"), b"not an icon at all!!"),
        0
    );
    let mut doc = open("multi.ico");
    assert_eq!(doc.format(), Format::Ico);
    assert_eq!(doc.pages(), Some(3), "every variant is a page");
    let rows: Vec<String> = doc.variants().iter().map(Variant::describe).collect();
    assert_eq!(
        rows,
        [
            "32 × 32 · 32-bit · PNG",
            "16 × 16 · 32-bit · BMP",
            "8 × 8 · 4-bit · BMP"
        ],
        "largest first, whatever the file's order"
    );
    assert_eq!(
        doc.status(),
        ["ICO", "3 variants", "32 × 32", "32-bit", "PNG", "1.5 KiB"]
    );
    let file = &doc.sections()[0];
    assert_eq!(file.id, "file");
    assert!(
        file.rows
            .iter()
            .any(|(k, v)| k == "Variant 3" && v == "8 × 8 · 4-bit · BMP"),
        "{:?}",
        file.rows
    );
    assert!(doc.text_layer().is_none(), "a binary container has no Copy");
    assert_eq!(doc.controls(), Vec::new(), "no controls of its own");

    // Actual pixels: each variant at its own size.
    let (w, h, red) = pixels(&mut doc, 0, actual());
    assert_eq!((w, h), (32, 32));
    assert_eq!(at(&red, w, 16, 16), [255, 0, 0, 255], "the PNG's red disc");
    assert_eq!(at(&red, w, 0, 0)[3], 0, "transparent corner");
    let (w, h, green) = pixels(&mut doc, 1, actual());
    assert_eq!((w, h), (16, 16));
    assert_eq!(
        at(&green, w, 8, 8),
        [0, 255, 0, 255],
        "the 32-bit BMP's green disc"
    );
    assert_eq!(at(&green, w, 0, 0)[3], 0);
    let (w, h, blue) = pixels(&mut doc, 2, actual());
    assert_eq!((w, h), (8, 8));
    assert_eq!(
        at(&blue, w, 0, 0)[3],
        0,
        "the 4-bit BMP's frame is masked out"
    );
    assert_eq!(at(&blue, w, 1, 1), [255, 255, 0, 255], "the diagonal");
    assert_eq!(at(&blue, w, 1, 2), [0, 0, 255, 255]);
    assert_eq!(
        doc.status()[2..5],
        ["8 × 8", "4-bit", "BMP"],
        "the footer names the shown variant"
    );

    // Fit: the largest whole factor up to 8, nearest-neighbour.
    let (w, _, frame) = pixels(&mut doc, 0, req(300, 300));
    assert_eq!(w, 256, "32 × 8");
    assert_magnified(&frame, w, &red, 32, 32, 8);
    let (w, _, frame) = pixels(&mut doc, 1, req(300, 300));
    assert_eq!(w, 128, "16 × 8, the cap");
    assert_magnified(&frame, w, &green, 16, 16, 8);
    let (w, _, frame) = pixels(&mut doc, 0, req(100, 100));
    assert_eq!(w, 96, "32 × 3");
    assert_magnified(&frame, w, &red, 32, 32, 3);
    let (w, _, _) = pixels(&mut doc, 0, req(40, 20));
    assert_eq!(w, 32, "no factor fits: the pane scales down");
    assert!(doc.render(3, actual()).is_err(), "no fourth page");
}

/// A CUR shows its hotspot crosshair at the right pixel; the toggle hides
/// it.
#[test]
fn a_cur_shows_its_hotspot_crosshair_and_the_toggle_hides_it() {
    let cur = bytes("arrow.cur");
    assert_eq!(IconsPreviewer.score(&entry("arrow.cur"), &cur[..64]), 200);
    let mut doc = open("arrow.cur");
    assert_eq!(doc.format(), Format::Cur);
    assert_eq!(doc.pages(), None);
    assert_eq!(
        doc.variants()[0].describe(),
        "16 × 16 · 32-bit · BMP · hotspot 3, 4"
    );
    assert_eq!(
        doc.status(),
        ["CUR", "1 variant", "16 × 16", "32-bit", "BMP", "1.1 KiB"]
    );
    assert!(matches!(
        doc.controls().as_slice(),
        [PreviewControl::Toggle {
            id: HOTSPOT,
            label: "Hotspot",
            on: true,
            ..
        }]
    ));

    // At factor 8 the hotspot pixel (3, 4) is the square from (24, 32); the
    // crosshair runs through its centre (28, 36): white over the black
    // arrow, black where the arrow is not.
    let (w, _, frame) = pixels(&mut doc, 0, req(200, 200));
    assert_eq!(w, 128);
    assert_eq!(
        at(&frame, w, 28, 36),
        [255, 255, 255, 255],
        "white over black"
    );
    assert_eq!(
        at(&frame, w, 28 + 24, 36),
        [0, 0, 0, 255],
        "black over nothing"
    );
    assert_eq!(at(&frame, w, 28, 36 + 24), [255, 255, 255, 255]);
    assert_eq!(at(&frame, w, 28 + 25, 36)[3], 0, "the arm ends");
    assert_eq!(
        at(&frame, w, 29, 37),
        [0, 0, 0, 255],
        "beside the cross: the arrow"
    );

    // At actual pixels the crosshair is three pixels each way through (3, 4).
    let (w, _, frame) = pixels(&mut doc, 0, actual());
    assert_eq!(w, 16);
    assert_eq!(at(&frame, w, 3, 4), [255, 255, 255, 255]);
    assert_eq!(at(&frame, w, 6, 4), [0, 0, 0, 255]);
    assert_eq!(at(&frame, w, 7, 4)[3], 0);

    doc.set_control(HOTSPOT, ControlValue::Bool(false))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!doc.hotspot_shown());
    let (w, _, frame) = pixels(&mut doc, 0, req(200, 200));
    assert_eq!(at(&frame, w, 28, 36), [0, 0, 0, 255], "the arrow alone");
    assert_eq!(at(&frame, w, 28 + 24, 36)[3], 0);
    assert!(matches!(
        doc.controls()[0],
        PreviewControl::Toggle { on: false, .. }
    ));
    assert!(doc.set_control(HOTSPOT, ControlValue::Pressed).is_err());
    let mut ico = open("single.ico");
    assert!(
        ico.set_control(HOTSPOT, ControlValue::Bool(false)).is_err(),
        "only a cursor has the toggle"
    );
}

/// An ICNS containing a JPEG 2000 entry previews its other variants; the
/// skipped entry is listed as not decodable and counted in the footer.
#[test]
fn an_icns_with_a_jpeg_2000_entry_previews_the_rest_and_counts_the_skipped() {
    let icns = bytes("mixed.icns");
    assert_eq!(IconsPreviewer.score(&entry("mixed.icns"), &icns[..64]), 200);
    let mut doc = open("mixed.icns");
    assert_eq!(doc.format(), Format::Icns);
    let rows: Vec<String> = doc.variants().iter().map(Variant::describe).collect();
    assert_eq!(
        rows,
        [
            "256 × 256 · ic08 · not decodable (JPEG 2000)",
            "128 × 128 · ic07 · PNG",
            "16 × 16 · is32 · RLE",
            "zzzz · not decodable (unknown type)",
        ]
    );
    assert_eq!(doc.pages(), Some(2), "the two decodable variants");
    assert_eq!(doc.skipped(), 2);
    assert_eq!(
        doc.status(),
        [
            "ICNS",
            "4 variants",
            "128 × 128",
            "ic07",
            "PNG",
            "1.1 KiB",
            "2 not shown"
        ]
    );
    let file = &doc.sections()[0];
    assert!(
        file.rows
            .iter()
            .any(|(k, v)| k == "Variants" && v == "4 (2 not decodable)"),
        "{:?}",
        file.rows
    );
    assert!(
        file.rows
            .iter()
            .any(|(_, v)| v.ends_with("not decodable (JPEG 2000)")),
        "{:?}",
        file.rows
    );

    let (w, h, blue) = pixels(&mut doc, 0, actual());
    assert_eq!((w, h), (128, 128));
    assert_eq!(
        at(&blue, w, 64, 64),
        [0, 0, 255, 255],
        "the PNG's blue disc"
    );
    assert_eq!(at(&blue, w, 0, 0)[3], 0);
    let (w, h, orange) = pixels(&mut doc, 1, actual());
    assert_eq!((w, h), (16, 16));
    assert_eq!(at(&orange, w, 8, 8), [255, 128, 0, 255], "the RLE disc");
    assert_eq!(
        at(&orange, w, 0, 0)[3],
        0,
        "the s8mk mask makes the corner transparent"
    );
    assert_eq!(doc.status()[2..5], ["16 × 16", "is32", "RLE"]);
    let (w, _, frame) = pixels(&mut doc, 1, req(300, 300));
    assert_magnified(&frame, w, &orange, 16, 16, 8);
}

/// An XPM with named colours and `None` renders with transparency; its
/// source is the text layer, what *Copy* puts on the clipboard.
#[test]
fn an_xpm_with_named_colours_and_none_renders_with_transparency_and_copies_its_source() {
    let path = fixture("named.xpm");
    let xpm = bytes("named.xpm");
    assert_eq!(IconsPreviewer.score(&entry("named.xpm"), &xpm[..64]), 200);
    let mut doc = open_through_provider(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(doc.pages(), None, "one picture, no pager");
    assert_eq!(doc.status(), ["XPM", "8 × 4", "4 colours", "262 B"]);
    assert_eq!(
        doc.text_layer().map(|l| l.pages.clone()),
        Some(vec![String::from_utf8_lossy(&xpm).into_owned()]),
        "the source"
    );
    assert_eq!(doc.controls(), Vec::new(), "no controls of its own");
    let file = &doc.sections()[0];
    for (key, value) in [
        ("Format", "XPM"),
        ("Dimensions", "8 × 4"),
        ("Colours", "4"),
        ("Chars per pixel", "2"),
    ] {
        assert!(
            file.rows.iter().any(|(k, v)| k == key && v == value),
            "{key}: {:?}",
            file.rows
        );
    }
    let Ok(Frame::Rgba { width, data, .. }) = doc.render(0, actual()) else {
        panic!("not pixels")
    };
    assert_eq!(width, 8);
    assert_eq!(at(&data, width, 0, 0), [0, 0, 0, 0], "None is transparent");
    assert_eq!(at(&data, width, 2, 0), [255, 0, 0, 255], "red");
    assert_eq!(
        at(&data, width, 3, 1),
        [132, 112, 255, 255],
        "light slate blue"
    );
    assert_eq!(
        at(&data, width, 4, 2),
        [0, 255, 0, 255],
        "#00FF00, the m key ignored"
    );
    let Ok(Frame::Rgba { width, height, .. }) = doc.render(0, req(200, 200)) else {
        panic!("not pixels")
    };
    assert_eq!((width, height), (64, 32), "magnified eightfold in Fit");
}

/// A single-variant ICO has no pager.
#[test]
fn a_single_variant_ico_has_no_pager() {
    let mut doc = open("single.ico");
    assert_eq!(doc.pages(), None);
    assert_eq!(doc.variants().len(), 1);
    assert_eq!(
        doc.status(),
        ["ICO", "1 variant", "16 × 16", "32-bit", "PNG", "158 B"]
    );
    let (w, h, data) = pixels(&mut doc, 0, actual());
    assert_eq!((w, h), (16, 16));
    assert_eq!(at(&data, w, 8, 8), [0, 0, 255, 255]);
    assert!(doc.render(1, actual()).is_err());
}

/// A truncated ICO does not panic; a file whose every variant is
/// undecodable gives a clear error, not an empty pane.
#[test]
fn a_truncated_ico_does_not_panic_and_no_decodable_variant_is_a_clear_error() {
    let truncated = IconDoc::new(&bytes("truncated.ico"));
    assert!(
        truncated.is_err(),
        "the cut-off entry is refused: {truncated:?}"
    );
    let error = open_through_provider(&fixture("garbage.ico"))
        .err()
        .unwrap_or_else(|| panic!("garbage decoded"))
        .to_string();
    assert!(
        error.contains("no variant of the ICO can be decoded"),
        "a clear error: {error}"
    );
    // Every prefix of a good file, and bytes that only start like one: no
    // panic anywhere.
    let multi = bytes("multi.ico");
    for len in (0..multi.len()).step_by(7) {
        let _ = IconDoc::new(&multi[..len]);
    }
    let icns = bytes("mixed.icns");
    for len in (0..icns.len()).step_by(5) {
        let _ = IconDoc::new(&icns[..len]);
    }
    let _ = IconDoc::new(b"icns\0\0\0\x08");
    let _ = IconDoc::new(b"/* XPM */ static char *x[] = { \"1 1 1 1\", \"a c red\" ");
    assert!(
        IconDoc::new(b"\x89PNG\r\n\x1a\n").is_err(),
        "not a container"
    );
}
