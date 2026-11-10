//! Acceptance (issue 139, item 7, SPEC.md *Remote sources*): the size
//! limit of whole-file previews on a remote source, on a stand-in for a
//! cloud account, which is no SSH host.
//!
//! A picture above the limit shows the File info card, centred, with the
//! source in its head, *Not previewed automatically* and the sentence that
//! names the limit. *Preview this file* clears the card at once and the
//! pane shows the download with its progress; Escape cancels it and the
//! card is back. *Always preview on ‹account›* fetches the file, and from
//! then on every file above the limit on that account previews without
//! asking — in this window and after a restart.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::Extension;
use model::SelectionChange;
use platform::{ConfigDir, Fake};
use tests::mem::{MemExtension, MemProvider};
use tests::{local, media};
use types::{Location, ProviderId};
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi};

const MIB: usize = 1024 * 1024;
const FOLDER: &str = "/flowcptstorage/photos";

/// A small JPEG padded to `bytes`: a picture as large as a photo, which
/// decodes as the small one it starts with.
fn padded_jpeg(bytes: usize) -> Vec<u8> {
    let mut jpeg = media::photo_jpeg(64, 48);
    jpeg.resize(bytes.max(jpeg.len()), 0);
    jpeg
}

/// The account: two pictures above the 4 MiB limit and one under it.
fn account() -> Arc<MemProvider> {
    let mem = Arc::new(MemProvider::new(ProviderId("azure"), Duration::ZERO));
    // A cloud provider's capabilities, `REMOTE` among them.
    mem.set_native(true);
    mem.dir(FOLDER);
    mem.file(&format!("{FOLDER}/a.jpg"), &padded_jpeg(5 * MIB));
    mem.file(&format!("{FOLDER}/b.jpg"), &padded_jpeg(6 * MIB));
    mem.file(&format!("{FOLDER}/small.jpg"), &padded_jpeg(200 * 1024));
    mem
}

/// A window on the account's folder with the Preview tab open, sharing
/// `fake` — and so the settings file — with earlier windows.
fn window(fake: &Arc<Fake>, mem: &Arc<MemProvider>, start: &Path) -> Harness<'static, RickUi> {
    let host = Arc::new(EguiHost::new());
    let extensions: Vec<Arc<dyn Extension>> = vec![
        Arc::new(ext_preview_image::ImagePreview::new()),
        Arc::new(MemExtension::with_accounts(
            Arc::clone(mem),
            "remote-cap-test-provider",
        )),
    ];
    // The stand-in's name, as a plugin's own strings would give it.
    ext::i18n::add_bundled("en", "remote-cap-test-provider = Azure Blob");
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions,
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 700.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| {
            let mut ui = RickUi::new(app, host, &cc.egui_ctx);
            ui.set_inspector_width(560.0);
            ui
        });
    settle(&mut harness);
    let pane = harness.state().app().model().focused;
    if !harness.state().inspector_open() {
        harness.state_mut().queue(Intent::ToggleInspector, pane);
        harness.step();
    }
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane,
            to: Location::new(ProviderId("azure"), FOLDER),
        },
        &mut Vec::new(),
    );
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
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Runs frames until `done` holds.
fn step_until(
    harness: &mut Harness<'_, RickUi>,
    what: &str,
    mut done: impl FnMut(&Harness<'_, RickUi>) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done(harness) {
        harness.step();
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..3 {
        harness.step();
    }
}

/// Selects `name` in the focused pane.
fn select(harness: &mut Harness<'_, RickUi>, name: &str) {
    let pane = harness.state().app().model().focused;
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| t.listing.rows().find(|e| e.name == name).map(|e| e.id))
        .unwrap_or_else(|| panic!("no entry {name}"));
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    harness.step();
}

/// The pane's slot: whether the size-limit card is on show, whether a
/// download is, and whether a picture is.
fn state(harness: &Harness<'_, RickUi>) -> (bool, bool, bool) {
    let pane = harness.state().app().model().focused;
    harness
        .state()
        .previews()
        .get(pane)
        .map_or((false, false, false), |slot| {
            (
                slot.capped().is_some(),
                slot.downloading(),
                slot.has_frame(),
            )
        })
}

fn click(harness: &mut Harness<'_, RickUi>, label: &str) {
    harness.get_by_label(label).click();
    for _ in 0..3 {
        harness.step();
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_file_above_the_limit_shows_the_card_and_always_preview_holds_for_the_account() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mem = account();
    let mut harness = window(&fake, &mem, dir.path());

    // Above the limit: the card, and nothing was read beyond the sniff.
    select(&mut harness, "a.jpg");
    step_until(&mut harness, "no size-limit card", |h| state(h).0);
    harness.get_by_label("JPEG image  ·  5.0 MiB  ·  Azure Blob, flowcptstorage");
    harness.get_by_label("Not previewed automatically");
    harness.get_by_label_contains("larger than the 4 MiB limit for automatic previews");
    let preview = harness.get_by_label("Preview this file").rect();
    let always = harness
        .get_by_label("Always preview on flowcptstorage")
        .rect();
    // Both in the card's action row, left-aligned; a long account name
    // takes the second button onto a line of its own.
    let beside = always.left() > preview.right() && (preview.top() - always.top()).abs() < 1.0;
    let below = (always.left() - preview.left()).abs() < 1.0 && always.top() > preview.bottom();
    assert!(beside || below, "{preview:?}, {always:?}");
    assert!(mem.read_bytes() < 64 * 1024, "{} read", mem.read_bytes());

    // Preview this file: the card clears at once, and while the file
    // comes the pane shows how far it is.
    mem.hold_reads_at(2 * MIB as u64);
    click(&mut harness, "Preview this file");
    assert!(
        harness
            .query_by_label("Not previewed automatically")
            .is_none(),
        "the card goes with the click"
    );
    step_until(&mut harness, "no download half way", |h| {
        let pane = h.state().app().model().focused;
        h.state()
            .previews()
            .get(pane)
            .and_then(ui_egui::preview::Slot::download)
            .is_some_and(|(read, _)| read >= 2 * MIB as u64)
    });
    assert_eq!(state(&harness), (false, true, false));
    harness.get_by_label("Downloading…  2.0 of 5.0 MiB");

    // Escape cancels the download, before it does anything else, and the
    // card is back.
    let selected = |h: &Harness<'_, RickUi>| {
        let model = h.state().app().model();
        model
            .active_tab(model.focused)
            .map_or(0, |(_, tab)| tab.selected().count())
    };
    harness.key_press(egui::Key::Escape);
    step_until(&mut harness, "the card did not come back", |h| state(h).0);
    assert_eq!(selected(&harness), 1, "the selection is as it was");
    harness.get_by_label("Not previewed automatically");
    mem.release_reads();
    settle(&mut harness);
    assert_eq!(
        state(&harness),
        (true, false, false),
        "nothing arrives late"
    );

    // Always preview on the account: this file is fetched and shown.
    click(&mut harness, "Always preview on flowcptstorage");
    step_until(&mut harness, "no picture", |h| state(h).2);
    assert_eq!(state(&harness), (false, false, true));

    // The next file above the limit on the same account previews without
    // asking.
    select(&mut harness, "b.jpg");
    step_until(&mut harness, "the second picture asked again", |h| {
        state(h).2 && !state(h).0
    });
    assert!(
        harness
            .query_by_label("Not previewed automatically")
            .is_none()
    );
    let settings = harness.state().app().settings_path().to_path_buf();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        settle(&mut harness);
        let text = std::fs::read_to_string(&settings).unwrap_or_default();
        if text.contains("preview_always = \"true\"") {
            assert!(text.contains("name = \"flowcptstorage\""), "{text}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the switch was not saved: {text}"
        );
    }

    // And after a restart: a new window, the same settings.
    drop(harness);
    let mem = account();
    let mut harness = window(&fake, &mem, dir.path());
    select(&mut harness, "b.jpg");
    step_until(
        &mut harness,
        "the picture asked again after a restart",
        |h| state(h).2 && !state(h).0,
    );
    assert_eq!(mem.read_bytes() / MIB as u64, 6, "the whole file was read");
}

#[test]
fn a_file_under_the_limit_previews_at_once_with_the_download_shown() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mem = account();
    let mut harness = window(&fake, &mem, dir.path());
    // Held before its end, past the first look at the file: the download
    // is on show without a card first.
    mem.hold_reads_at(100 * 1024);
    select(&mut harness, "small.jpg");
    step_until(&mut harness, "no download", |h| state(h).1);
    assert!(!state(&harness).0, "no card under the limit");
    assert!(
        harness
            .query_all_by_label_contains("Downloading…")
            .next()
            .is_some(),
        "the progress under the burst"
    );
    mem.release_reads();
    step_until(&mut harness, "no picture", |h| state(h).2);
    assert_eq!(state(&harness), (false, false, true));
}
