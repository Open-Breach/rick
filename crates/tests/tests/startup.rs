//! Acceptance (release): startup to the first frame.
//!
//! The binary's start-up, headlessly: the app built with every built-in
//! extension on a small folder, then the shell's first frames in the
//! `egui_kittest` harness, which runs until the window is stable. Creating
//! the window and the GPU surface is not part of it; the platform is the
//! fake. There is no spec target; the targets in `tests::bench` were set
//! from measurements (BENCHMARKS.md).

use std::sync::Arc;
use std::time::Instant;

use egui_kittest::Harness;
use ext::Extension;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::{EguiHost, RickUi};

/// The extensions the binary compiles in.
fn built_ins() -> Vec<Arc<dyn Extension>> {
    vec![
        Arc::new(ext_preview_text::TextPreview::new()),
        Arc::new(ext_preview_code::CodePreview::new()),
        Arc::new(ext_preview_image::ImagePreview::new()),
        Arc::new(ext_preview_pdf::PdfPreview::new()),
        Arc::new(ext_git::GitExtension::new()),
    ]
}

#[test]
#[ignore = "timing; run with --ignored in release"]
fn startup_to_the_first_frame() {
    let fixture = tests::fixture::generate(100, 3).unwrap_or_else(|e| panic!("{e}"));
    let mut samples = Vec::new();
    for _ in 0..tests::bench::runs() {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let config_dir = fake.config_dir();

        let started = Instant::now();
        let host = Arc::new(EguiHost::new());
        let app = app::App::new(app::Config {
            platform: fake,
            ui: host.clone(),
            extensions: built_ins(),
            config_dir: Some(config_dir),
            start: Some(local(fixture.root())),
        });
        let harness = Harness::builder()
            .with_size(egui::vec2(1100.0, 720.0))
            .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
        samples.push(started.elapsed());

        assert!(harness.state().app().model().panes().next().is_some());
        drop(harness);
    }
    tests::bench::record("startup", &samples);
}
