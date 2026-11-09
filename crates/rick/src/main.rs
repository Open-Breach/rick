//! Rick, a god-like file manager: the binary.
//!
//! Wires the native platform, the local provider, the built-in operations,
//! the previewers compiled in as features and the plugins found beside the
//! binary and in the config directory into the `app`, and runs the egui
//! shell. The command line ([`cli`], specs/system-integration.md §4):
//! `rick --version` prints the build's details — the text the About
//! dialog's *Copy details* copies (specs/about-dialog.md) — and exits;
//! `rick --list-plugins` prints the table the Extensions screen shows and
//! exits; `rick [<path>…]` opens each folder in a tab of its own (as the
//! desktop entry's `%F` passes the selected folders; specs/app-icons.md),
//! the working directory without any; `rick reveal <file>…` opens the
//! files' folders; `--new-window` puts them in a window of their own;
//! `rick <command> <args…>` runs a plugin's subcommand, and `--help` lists
//! the core and plugin commands.
//!
//! One Rick runs per user session and config directory
//! (specs/system-integration.md §3, [`instance`]): a launch first hands
//! its command line to the Rick already running on the endpoint the
//! platform names for the config directory (`--config-dir <dir>` picks
//! another, so two installs coexist), prints the reply and exits with the
//! instance's status, within a second; only when nothing listens does it
//! bind the endpoint and open the window itself.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context as _;
use platform::{Activation as _, ConfigDir as _, SingleInstance as _};
use plugins_native::{Host, Options, PluginStatus};
use rick::{cli, instance};

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

/// The arguments the launch itself acts on, before any instance is asked;
/// everything else is the instance's, parsed by [`cli`] with the loaded
/// plugins' commands known, so a plugin's option never fails here.
#[derive(Debug, Default, PartialEq, Eq)]
struct Launch {
    /// `--version`.
    version: bool,
    /// `--list-plugins`.
    list_plugins: bool,
    /// `--config-dir <dir>`.
    config_dir: Option<PathBuf>,
    /// Everything else, for the instance: the paths, the subcommands,
    /// `--new-window`, `--help`.
    rest: Vec<OsString>,
}

/// Splits the command line into what the launch does itself and what it
/// hands the instance.
///
/// # Errors
///
/// `--config-dir` without a directory.
fn launch(args: &[OsString]) -> Result<Launch, String> {
    let mut launch = Launch::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--version" || arg == "-V" {
            launch.version = true;
        } else if arg == "--list-plugins" {
            launch.list_plugins = true;
        } else if arg == "--config-dir" {
            let dir = args
                .next()
                .filter(|d| !d.is_empty())
                .ok_or_else(|| "--config-dir takes a directory".to_owned())?;
            launch.config_dir = Some(PathBuf::from(dir));
        } else if let Some(dir) = arg
            .to_str()
            .and_then(|a| a.strip_prefix("--config-dir="))
            .filter(|d| !d.is_empty())
        {
            launch.config_dir = Some(PathBuf::from(dir));
        } else {
            launch.rest.push(arg.clone());
        }
    }
    Ok(launch)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            // The binary's own report of why it stopped.
            let _ = writeln!(std::io::stderr().lock(), "rick: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Prints a reply's text where it belongs and turns its status into the
/// process's exit code.
fn exit_with(status: i32, text: Option<String>) -> anyhow::Result<ExitCode> {
    if let Some(text) = text {
        if status == 0 {
            writeln!(std::io::stdout().lock(), "{text}").context("writing the reply")?;
        } else {
            writeln!(std::io::stderr().lock(), "{text}").context("writing the reply")?;
        }
    }
    Ok(u8::try_from(status).map_or(ExitCode::FAILURE, ExitCode::from))
}

// Start-up is one straight line: the launch's own arguments, the handover
// or the endpoint, the plugins, the app, the window.
#[allow(clippy::too_many_lines)]
fn run() -> anyhow::Result<ExitCode> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let launch = match launch(&args) {
        Ok(launch) => launch,
        Err(message) => {
            return exit_with(
                instance::USAGE,
                Some(format!("rick: {message}\n\n{}", cli::help(&[]))),
            );
        }
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let platform = Arc::new(platform::Native::new());
    let config_dir = launch
        .config_dir
        .clone()
        .unwrap_or_else(|| platform.config_dir());

    // One Rick per user and config directory: a running one takes the
    // command line and this launch exits with its status; none, and this
    // launch holds the endpoint from here on.
    let cwd = std::env::current_dir().context("the working directory")?;
    let for_instance: Vec<String> = launch
        .rest
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    // `--version` and `--list-plugins` print and exit once the plugins are
    // known: the version text lists them with their status.
    let listener = if launch.list_plugins || launch.version {
        None
    } else {
        let endpoint = platform.instance_endpoint(&config_dir);
        // The instance may take the foreground for the window it opens.
        platform.allow_activation();
        match instance::hand_over(&endpoint, for_instance.clone(), cwd.clone()).with_context(
            || format!("the single-instance endpoint {}", endpoint.path().display()),
        )? {
            instance::Outcome::Exit { status, text } => return exit_with(status, text),
            instance::Outcome::Become(listener) => Some(listener),
        }
    };

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
    if launch.version {
        writeln!(
            std::io::stdout().lock(),
            "{}",
            ui_egui::about::details(&build, &rows)
        )
        .context("writing the version")?;
        return Ok(ExitCode::SUCCESS);
    }
    if launch.list_plugins {
        write!(std::io::stdout().lock(), "{}", table(host.plugins()))
            .context("writing the plugins table")?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut extensions = built_ins;
    extensions.extend(host.extensions());
    // The loaded libraries stay mapped for the process lifetime: every
    // contribution the app holds points into them.
    Box::leak(Box::new(host));

    // The command line, with the plugins' commands known. The help needs
    // no window; a command line the parser refuses ends here with its
    // message. The first folder of a plain `rick <folder>…` is the start
    // location: a tab in the restored session's focused window, or the
    // one tab when there is no session to restore; the rest is applied
    // once the app is up, as a second launch's would be.
    let specs: Vec<ext::CommandSpec> = extensions
        .iter()
        .flat_map(|e| e.manifest().commands.iter().copied())
        .collect();
    let parsed = match cli::parse(&for_instance, &cwd, &specs) {
        Ok(parsed) => parsed,
        Err(cli::Usage(text)) => return exit_with(instance::USAGE, Some(text)),
    };
    let (start, parsed) = match parsed {
        cli::Parsed::Help(text) | cli::Parsed::Version(text) => {
            return exit_with(0, Some(text));
        }
        cli::Parsed::Open {
            mut targets,
            new_window: false,
        } => {
            // A `rick://` URL is resolved once the app is up, like the
            // rest; a plain first folder is the start location.
            let start = match targets.first() {
                Some(cli::Target::Path(_)) => match targets.remove(0) {
                    cli::Target::Path(path) => Some(path),
                    cli::Target::Url(_) => None,
                },
                _ => None,
            };
            (
                start,
                cli::Parsed::Open {
                    targets,
                    new_window: false,
                },
            )
        }
        other => (None, other),
    };
    if let Some(start) = &start {
        tracing::info!(start = %start.display(), "starting");
    } else {
        tracing::info!("starting");
    }

    let ui = Arc::new(ui_egui::EguiHost::new());
    let mut app = app::App::with_services(
        app::Config {
            platform,
            ui: ui.clone(),
            extensions,
            config_dir: launch.config_dir.clone(),
            start: start.as_deref().map(instance::location),
        },
        prompts,
        ext_settings,
    );
    app.set_plugins(rows);
    // The rest of the command line before the first frame; a new tab
    // reports nothing the UI would miss (its listing arrives through the
    // app's polling like the start location's), and a plugin command's
    // reply has nobody to print it, so it is logged.
    let mut events = Vec::new();
    let reply = instance::apply(&mut app, &mut events, parsed, &cwd, None);
    if reply.status != 0 {
        return exit_with(reply.status, reply.text);
    }
    if let Some(text) = reply.text {
        writeln!(std::io::stdout().lock(), "{text}").context("writing the reply")?;
    }
    if !events.is_empty() {
        tracing::debug!(count = events.len(), "events of the start-up tabs dropped");
    }

    // The endpoint, served for as long as the window is open: a second
    // launch's command line runs on the UI thread between two frames.
    let (externals, receiver) = app::external_channel();
    let endpoint = listener.as_ref().map(|l| l.endpoint().clone());
    let serving = listener.map(|listener| instance::serve(listener, externals));
    let result = ui_egui::run("Rick", build, app, ui, Some(receiver))
        .context("the window could not be opened");
    if let Some(serving) = serving {
        serving.abort();
    }
    if let Some(endpoint) = endpoint {
        endpoint.unbind();
    }
    result.map(|()| ExitCode::SUCCESS)
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

    /// The launch's own arguments come out, the rest stays for the
    /// instance (`--help` among it, since only the instance knows every
    /// command); `--config-dir` takes a directory either way.
    #[test]
    fn the_launch_takes_its_own_arguments_and_keeps_the_rest() {
        let parsed = launch(&[
            "--config-dir".into(),
            "/c".into(),
            "--new-window".into(),
            "x".into(),
            "--help".into(),
        ])
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parsed.config_dir, Some(PathBuf::from("/c")));
        assert_eq!(parsed.rest, ["--new-window", "x", "--help"]);
        assert!(!parsed.version && !parsed.list_plugins);
        let parsed =
            launch(&["--config-dir=/d".into(), "-V".into()]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parsed.config_dir, Some(PathBuf::from("/d")));
        assert!(parsed.version);
        let parsed = launch(&["--list-plugins".into()]).unwrap_or_else(|e| panic!("{e}"));
        assert!(parsed.list_plugins);
        assert!(launch(&["--config-dir".into()]).is_err());
    }
}
