//! Rick, a god-like file manager: the binary.
//!
//! Wires the native platform, the local provider, the built-in operations,
//! the previewers compiled in as features and the plugins found beside the
//! binary and in the config directory into the `app`, and runs the egui
//! shell. `rick --version` prints the build's details — the text the About
//! dialog's *Copy details* copies (specs/about-dialog.md) — and exits;
//! `rick --list-plugins` prints the table the Extensions screen
//! shows and exits; every other argument is a folder to open, each in a
//! tab of its own (`rick <folder>…`, as the desktop entry's `%F` passes the
//! selected folders; specs/app-icons.md).

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use app::Command;
use platform::ConfigDir as _;
use plugins_native::{Host, Options, PluginStatus};
use types::Location;

/// What this binary knows about its own build, for the About dialog and
/// `--version`: the values `build.rs` records, the toolchain the plugin
/// host was built with, the contributors, the component list of the
/// release beside the binary (`None` in a build without one), and the
/// licence lists of the plugins loaded from outside the release's folder.
fn build(exe: &Path, plugins: &[app::Plugin]) -> ui_egui::about::Build {
    let plugins_dir = exe
        .parent()
        .map(|dir| dir.join(plugins_native::PLUGINS_DIR));
    ui_egui::about::Build {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        commit: env!("RICK_GIT_HASH").to_owned(),
        target: env!("RICK_TARGET").to_owned(),
        date: env!("RICK_BUILD_DATE").to_owned(),
        toolchain: plugins_native::HOST_TOOLCHAIN.to_owned(),
        contributors: include_str!(env!("RICK_CONTRIBUTORS")),
        roles: include_str!(env!("RICK_CONTRIBUTOR_ROLES")),
        third_party: ui_egui::about::load_third_party(exe),
        plugin_licences: ui_egui::about::load_plugin_licences(plugins, plugins_dir.as_deref()),
        plugins_dir,
    }
}

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
    #[cfg(feature = "preview-markup")]
    extensions.push(Arc::new(ext_preview_markup::MarkupPreview::new()));
    #[cfg(feature = "preview-3d")]
    extensions.push(Arc::new(ext_preview_3d::Preview3d::new()));
    #[cfg(feature = "preview-icons")]
    extensions.push(Arc::new(ext_preview_icons::IconsPreview::new()));
    #[cfg(feature = "git")]
    extensions.push(Arc::new(ext_git::GitExtension::new()));
    #[cfg(feature = "vfs-trash")]
    extensions.push(Arc::new(ext_vfs_trash::TrashExtension::new()));
    #[cfg(feature = "vfs-archive")]
    extensions.push(Arc::new(ext_vfs_archive::ArchiveExtension::new()));
    #[cfg(feature = "svn")]
    extensions.push(Arc::new(ext_svn::SvnExtension::new()));
    #[cfg(feature = "search-walk")]
    extensions.push(Arc::new(ext_search_walk::WalkExtension::new()));
    #[cfg(feature = "search-windows")]
    extensions.push(Arc::new(ext_search_windows::WindowsSearchExtension::new()));
    #[cfg(feature = "search-spotlight")]
    extensions.push(Arc::new(ext_search_spotlight::SpotlightExtension::new()));
    #[cfg(feature = "search-tracker")]
    extensions.push(Arc::new(ext_search_tracker::TrackerExtension::new()));
    #[cfg(feature = "search-baloo")]
    extensions.push(Arc::new(ext_search_baloo::BalooExtension::new()));
    #[cfg(feature = "ssh")]
    extensions.push(Arc::new(ext_ssh::SshExtension::new()));
    #[cfg(feature = "vfs-s3")]
    extensions.push(Arc::new(ext_vfs_s3::S3Extension::new()));
    #[cfg(feature = "vfs-azure")]
    extensions.push(Arc::new(ext_vfs_azure::AzureExtension::new()));
    #[cfg(feature = "vfs-gcs")]
    extensions.push(Arc::new(ext_vfs_gcs::GcsExtension::new()));
    #[cfg(feature = "action-compress")]
    extensions.push(Arc::new(ext_action_compress::CompressExtension::new()));
    #[cfg(feature = "action-compare")]
    extensions.push(Arc::new(ext_action_compare::CompareExtension::new()));
    #[cfg(feature = "action-terminal")]
    extensions.push(Arc::new(ext_action_terminal::TerminalExtension::new()));
    #[cfg(feature = "action-rename")]
    extensions.push(Arc::new(ext_action_rename::RenameExtension::new()));
    #[cfg(feature = "installer")]
    extensions.push(Arc::new(ext_installer::InstallerExtension::new()));
    extensions
}

fn main() -> anyhow::Result<()> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let first = args.first().map(OsString::as_os_str);
    // Both print and exit once the plugins are known: the version text
    // lists them with their status.
    let version = first.is_some_and(|arg| arg == "--version" || arg == "-V");
    let list_plugins = version || first.is_some_and(|arg| arg == "--list-plugins");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    // The folders on the command line each open in a tab of the restored
    // session's focused window, the first as the start location and the
    // others after it; without any the session comes back as it was. An
    // argument that is no folder is handed on as a location all the same:
    // the pane says what it cannot list, and the start never fails on it.
    let mut folders = if list_plugins {
        Vec::new()
    } else {
        folders(&args)
    }
    .into_iter();
    let start = folders.next();
    if let Some(start) = &start {
        tracing::info!(start = %start.display(), "starting");
    } else {
        tracing::info!("starting");
    }

    let platform = Arc::new(platform::Native::new());
    let config_dir = platform.config_dir();
    let built_ins = built_ins();
    // The question queue the plugins ask the user through (a host key, a
    // passphrase), and the settings they declared: one of each for the
    // plugins and the app alike.
    let prompts = ext::Prompts::new();
    let ext_settings = Arc::new(app::ExtSettings::new(&config_dir));

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
        prompts: Some(Arc::clone(&prompts)),
        settings: Some(Arc::clone(&ext_settings) as Arc<dyn ext::SettingsStore>),
    });
    let rows: Vec<app::Plugin> = host.plugins().iter().map(row).collect();
    let build = build(&exe, &rows);
    if version {
        writeln!(
            std::io::stdout().lock(),
            "{}",
            ui_egui::about::details(&build, &rows)
        )
        .context("writing the version")?;
        return Ok(());
    }
    if list_plugins {
        write!(std::io::stdout().lock(), "{}", table(host.plugins()))
            .context("writing the plugins table")?;
        return Ok(());
    }
    let mut extensions = built_ins;
    extensions.extend(host.extensions());
    // The loaded libraries stay mapped for the process lifetime: every
    // contribution the app holds points into them.
    Box::leak(Box::new(host));

    let ui = Arc::new(ui_egui::EguiHost::new());
    let mut app = app::App::with_services(
        app::Config {
            platform,
            ui: ui.clone(),
            extensions,
            config_dir: None,
            start: start.as_deref().map(location),
        },
        prompts,
        ext_settings,
    );
    app.set_plugins(rows);
    // The further folders, one tab each, before the first frame; a new tab
    // reports nothing the UI would miss (its listing arrives through the
    // app's polling like the start location's).
    let mut events = Vec::new();
    for folder in folders {
        app.handle(Command::NewTab(location(&folder)), &mut events);
    }
    if !events.is_empty() {
        tracing::debug!(count = events.len(), "events of the start-up tabs dropped");
    }
    ui_egui::run("Rick", build, app, ui).context("the window could not be opened")
}

/// The folders of the command line, in order: every argument, made
/// absolute where it names something (a relative path from a launcher
/// resolves against its working directory), kept as given otherwise.
fn folders(args: &[OsString]) -> Vec<PathBuf> {
    args.iter()
        .map(PathBuf::from)
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
        .collect()
}

/// A folder as a location of the local provider.
fn location(path: &Path) -> Location {
    Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path))
}

/// A plugin as the Extensions screen lists it.
fn row(plugin: &plugins_native::Plugin) -> app::Plugin {
    app::Plugin {
        id: plugin.id.clone(),
        name: plugin.name.clone(),
        version: plugin.version.clone(),
        points: plugin.points,
        previews: plugin.previews.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The build's details `crates/rick/build.rs` records for the About
    /// dialog (specs/about-dialog.md): a `YYYY-MM-DD` date, a target
    /// triple, and the contributors' names one per line in a file the
    /// binary embeds, empty outside a checkout (QUESTIONS.md 258).
    #[test]
    fn the_build_records_its_date_target_and_contributors() {
        let date = env!("RICK_BUILD_DATE");
        assert_eq!(date.len(), 10, "{date}");
        let digits: Vec<bool> = date.chars().map(|c| c.is_ascii_digit()).collect();
        assert!(digits[0] && digits[1] && digits[2] && digits[3], "{date}");
        assert_eq!(&date[4..5], "-", "{date}");
        assert!(
            digits[5] && digits[6] && date[7..8] == *"-" && digits[8] && digits[9],
            "{date}"
        );
        let target = env!("RICK_TARGET");
        assert!(target.split('-').count() >= 3, "{target}");
        let names = include_str!(env!("RICK_CONTRIBUTORS"));
        for name in names.lines() {
            assert_eq!(name, name.trim());
            assert_ne!(name, "");
        }
        // The roles file is there, text, and empty until CONTRIBUTORS.md exists.
        let roles = include_str!(env!("RICK_CONTRIBUTOR_ROLES"));
        assert!(roles.lines().all(|line| line.len() < 10_000));
    }

    /// Every argument is a folder to open, in order, made absolute where
    /// it exists and kept where it does not; nothing is dropped and
    /// nothing fails.
    #[test]
    fn every_argument_is_a_folder_in_order() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap_or_else(|e| panic!("{e}"));
        std::fs::create_dir_all(&b).unwrap_or_else(|e| panic!("{e}"));
        let missing = dir.path().join("nowhere");
        let args: Vec<OsString> = [&a, &b, &missing]
            .into_iter()
            .map(|p| p.as_os_str().to_owned())
            .collect();
        let found = folders(&args);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0], a.canonicalize().unwrap_or(a));
        assert_eq!(found[1], b.canonicalize().unwrap_or(b));
        assert_eq!(found[2], missing);
        assert_eq!(folders(&[]), Vec::<PathBuf>::new());
        let location = location(&found[0]);
        assert_eq!(location.provider, vfs_local::PROVIDER_ID);
    }
}
