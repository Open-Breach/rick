//! Rick, a god-like file manager: the binary.
//!
//! Wires the native platform, the local provider, the built-in operations,
//! the previewers compiled in as features and the plugins found beside the
//! binary and in the config directory into the `app`, and runs the egui
//! shell. `rick --version` prints the version and the git hash of the build
//! and exits; `rick --list-plugins` prints the table the Extensions screen
//! shows and exits; the only other argument is the folder to start in.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use platform::ConfigDir as _;
use plugins_native::{Host, Options, PluginStatus};
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
    #[cfg(feature = "preview-hex")]
    extensions.push(Arc::new(ext_preview_hex::HexPreview::new()));
    #[cfg(feature = "preview-office")]
    extensions.push(Arc::new(ext_preview_office::OfficePreview::new()));
    #[cfg(feature = "preview-media")]
    extensions.push(Arc::new(ext_preview_media::MediaPreview::new()));
    #[cfg(feature = "preview-archive")]
    extensions.push(Arc::new(ext_preview_archive::ArchivePreview::new()));
    #[cfg(feature = "preview-csv")]
    extensions.push(Arc::new(ext_preview_csv::CsvPreview::new()));
    #[cfg(feature = "git")]
    extensions.push(Arc::new(ext_git::GitExtension::new()));
    #[cfg(feature = "vfs-trash")]
    extensions.push(Arc::new(ext_vfs_trash::TrashExtension::new()));
    #[cfg(feature = "vfs-archive")]
    extensions.push(Arc::new(ext_vfs_archive::ArchiveExtension::new()));
    #[cfg(feature = "vfs-sftp")]
    extensions.push(Arc::new(ext_vfs_sftp::SftpExtension::new()));
    #[cfg(feature = "vfs-s3")]
    extensions.push(Arc::new(ext_vfs_s3::S3Extension::new()));
    #[cfg(feature = "vfs-azure")]
    extensions.push(Arc::new(ext_vfs_azure::AzureExtension::new()));
    #[cfg(feature = "vfs-gcs")]
    extensions.push(Arc::new(ext_vfs_gcs::GcsExtension::new()));
    extensions
}

fn main() -> anyhow::Result<()> {
    let arg = std::env::args_os().nth(1);
    if arg
        .as_deref()
        .is_some_and(|arg| arg == "--version" || arg == "-V")
    {
        writeln!(std::io::stdout().lock(), "{VERSION}").context("writing the version")?;
        return Ok(());
    }
    let list_plugins = arg.as_deref().is_some_and(|arg| arg == "--list-plugins");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    // A path on the command line opens in a new tab of the restored
    // session; without one the session comes back as it was.
    let start = arg
        .filter(|_| !list_plugins)
        .map(PathBuf::from)
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p));
    if let Some(start) = &start {
        tracing::info!(start = %start.display(), "starting");
    } else {
        tracing::info!("starting");
    }

    let platform = Arc::new(platform::Native::new());
    let config_dir = platform.config_dir();
    let built_ins = built_ins();

    // The plugins: beside the binary and per user, minus the ids rick.toml
    // disables. A settings file that does not read leaves every plugin
    // enabled; the app reports the file itself.
    let settings = app::settings::load(&config_dir.join(app::SETTINGS_FILE)).unwrap_or_default();
    let exe = std::env::current_exe().context("the binary's own path")?;
    let host = Host::load(&Options {
        dirs: Options::dirs(&exe, &config_dir),
        disabled: settings.plugins.disabled,
        compiled_in: built_ins
            .iter()
            .map(|e| e.manifest().id.0.to_owned())
            .collect(),
        platform: Some(platform.clone()),
    });
    if list_plugins {
        write!(std::io::stdout().lock(), "{}", table(host.plugins()))
            .context("writing the plugins table")?;
        return Ok(());
    }
    let rows: Vec<app::Plugin> = host.plugins().iter().map(row).collect();
    let mut extensions = built_ins;
    extensions.extend(host.extensions());
    // The loaded libraries stay mapped for the process lifetime: every
    // contribution the app holds points into them.
    Box::leak(Box::new(host));

    let ui = Arc::new(ui_egui::EguiHost::new());
    let mut app = app::App::new(app::Config {
        platform,
        ui: ui.clone(),
        extensions,
        config_dir: None,
        start: start.map(|p| Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&p))),
    });
    app.set_plugins(rows);
    ui_egui::run("Rick", VERSION, app, ui).context("the window could not be opened")
}

/// A plugin as the Extensions screen lists it.
fn row(plugin: &plugins_native::Plugin) -> app::Plugin {
    app::Plugin {
        id: plugin.id.clone(),
        name: plugin.name.clone(),
        version: plugin.version.clone(),
        points: plugin.points,
        status: match &plugin.status {
            PluginStatus::Loaded => app::PluginStatus::Loaded,
            PluginStatus::Disabled => app::PluginStatus::Disabled,
            PluginStatus::Refused(reason) => app::PluginStatus::Refused(reason.clone()),
        },
        path: plugin.dir.clone(),
    }
}

/// The table `--list-plugins` prints: one header, one row per plugin found,
/// columns padded to their widest cell.
fn table(plugins: &[plugins_native::Plugin]) -> String {
    let header = ["id", "name", "version", "points", "status", "path"];
    let rows: Vec<[String; 6]> = plugins
        .iter()
        .map(|p| {
            [
                p.id.clone(),
                p.name.clone(),
                p.version.clone(),
                p.points.to_string(),
                match &p.status {
                    PluginStatus::Loaded => "loaded".to_owned(),
                    PluginStatus::Disabled => "disabled".to_owned(),
                    PluginStatus::Refused(reason) => format!("refused: {reason}"),
                },
                p.dir.display().to_string(),
            ]
        })
        .collect();
    let mut widths: Vec<usize> = header.iter().map(|h| h.len()).collect();
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: &[&str]| -> String {
        let mut out = String::new();
        for (i, (cell, width)) in cells.iter().zip(&widths).enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            if i + 1 == cells.len() {
                out.push_str(cell);
            } else {
                // Writing into a String cannot fail.
                let _ = write!(out, "{cell:<width$}");
            }
        }
        out.trim_end().to_owned() + "\n"
    };
    let mut out = line(&header);
    for row in &rows {
        let cells: Vec<&str> = row.iter().map(String::as_str).collect();
        out.push_str(&line(&cells));
    }
    if rows.is_empty() {
        out.push_str("(no plugins found)\n");
    }
    out
}
