//! Rick, a god-like file manager: the binary.
//!
//! Wires the native platform, the local provider, the built-in operations
//! and the previewers compiled in as features into the `app`, and runs the
//! egui shell.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use types::Location;

/// The extensions compiled in, one per enabled feature.
// Each push sits behind its feature, which `vec![]` cannot express.
#[allow(clippy::vec_init_then_push)]
fn built_ins() -> Vec<Arc<dyn ext::Extension>> {
    let mut extensions: Vec<Arc<dyn ext::Extension>> = Vec::new();
    #[cfg(feature = "preview-text")]
    extensions.push(Arc::new(ext_preview_text::TextPreview::new()));
    #[cfg(feature = "preview-code")]
    extensions.push(Arc::new(ext_preview_code::CodePreview::new()));
    #[cfg(feature = "preview-image")]
    extensions.push(Arc::new(ext_preview_image::ImagePreview::new()));
    #[cfg(feature = "preview-pdf")]
    extensions.push(Arc::new(ext_preview_pdf::PdfPreview::new()));
    extensions
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let start = std::env::args_os().nth(1).map(PathBuf::from).map_or_else(
        || std::env::home_dir().unwrap_or_else(|| PathBuf::from(".")),
        |p| std::fs::canonicalize(&p).unwrap_or(p),
    );
    tracing::info!(start = %start.display(), "starting");

    let platform = Arc::new(platform::Native::new());
    let host = Arc::new(ui_egui::EguiHost::new());
    let app = app::App::new(app::Config {
        platform,
        ui: host.clone(),
        extensions: built_ins(),
        config_dir: None,
        start: Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&start)),
    });
    ui_egui::run("Rick", app, host).context("the window could not be opened")
}
