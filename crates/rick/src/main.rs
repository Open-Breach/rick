//! Rick, a god-like file manager: the binary.
//!
//! Wires the native platform, the local provider, the built-in operations
//! and the previewers compiled in as features into the `app`, and runs the
//! egui shell. `rick --version` prints the version and the git hash of the
//! build and exits; the only other argument is the folder to start in.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use types::Location;

/// The version and the git hash of this build, as `--version` prints them
/// and the About window shows them.
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("RICK_GIT_HASH"), ")");

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
    #[cfg(feature = "git")]
    extensions.push(Arc::new(ext_git::GitExtension::new()));
    extensions
}

fn main() -> anyhow::Result<()> {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--version" || arg == "-V")
    {
        writeln!(std::io::stdout().lock(), "{VERSION}").context("writing the version")?;
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    // A path on the command line opens in a new tab of the restored
    // session; without one the session comes back as it was.
    let start = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p));
    if let Some(start) = &start {
        tracing::info!(start = %start.display(), "starting");
    } else {
        tracing::info!("starting");
    }

    let platform = Arc::new(platform::Native::new());
    let host = Arc::new(ui_egui::EguiHost::new());
    let app = app::App::new(app::Config {
        platform,
        ui: host.clone(),
        extensions: built_ins(),
        config_dir: None,
        start: start.map(|p| Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&p))),
    });
    ui_egui::run("Rick", VERSION, app, host).context("the window could not be opened")
}
