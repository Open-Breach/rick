//! The registry's test binary: reads a plugin list the way Rick does, and
//! installs what the list offers.
//!
//! ```text
//! registry_check <config dir> [--install] [--rick <version>]
//! ```
//!
//! `<config dir>` is a config directory of Rick's whose `rick.toml` names
//! the registry to read, an index and the key it is signed with
//! (specs/plugin-distribution.md §6, *Another registry*):
//!
//! ```toml
//! [registry]
//! url = "http://127.0.0.1:8123/registry/index.json"
//! public_key = "RWQ…"
//! ```
//!
//! The binary makes the application without a window on that directory and
//! sends it what the Extensions dialogue sends: *Check now*, and with
//! `--install` the install of every plugin on offer. So the list is
//! fetched, verified against the key and held to its schema by the
//! registry client, and a package is downloaded, compared with the size
//! and the SHA-256 the list names and unpacked into the per-user plugins
//! folder by the same job the **Install** button starts. It prints what
//! the *Available* view would show, one line per plugin, and where a list
//! is refused the line the view says. A plugin that is in the per-user
//! folder already is not on offer. `--rick` asks as a Rick of another
//! version than this build's.
//!
//! Like every application that is not the released binary, it knows only
//! the key its `rick.toml` names: without one it asks nobody.
//!
//! It is how a registry is checked from Rick's side before anything is
//! published: Trebuchet's publisher writes an index for a release's
//! packages with a key of the test's own and serves it on this machine,
//! and this binary reads it (`docs/building.md`, *Checking a registry*).
//! A plugin it installed loads in the Rick it was built for at that
//! Rick's next start, as after **Install**.
//!
//! ```text
//! cargo run -p app --example registry_check -- /tmp/rick-config --install
//! ```

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::plugins::{Origin, Record};
use app::{App, Available, Command, Config, Event, Offering, RegistryCommand, RegistryProblem};
use ext::{Level, UiHost};
use types::Location;

const USAGE: &str = "usage: registry_check <config dir> [--install] [--rick <version>]";
/// How long the list may take to arrive.
const FETCH_TIMEOUT: Duration = Duration::from_mins(2);
/// How long one package may take to arrive and to be unpacked.
const INSTALL_TIMEOUT: Duration = Duration::from_mins(15);

/// What the binary was asked for.
struct Args {
    config: PathBuf,
    install: bool,
    rick: Option<String>,
}

fn args() -> Result<Args, String> {
    let mut config = None;
    let mut install = false;
    let mut rick = None;
    let mut given = std::env::args_os().skip(1);
    while let Some(arg) = given.next() {
        if arg == "--install" {
            install = true;
        } else if arg == "--rick" {
            let version = given.next().and_then(|version| version.into_string().ok());
            rick = Some(version.ok_or(USAGE)?);
        } else if config.is_none() {
            config = Some(PathBuf::from(arg));
        } else {
            return Err(USAGE.to_owned());
        }
    }
    Ok(Args {
        config: config.ok_or(USAGE)?,
        install,
        rick,
    })
}

/// Nobody is there to ask: every prompt gets its default.
struct Headless;

impl UiHost for Headless {
    fn notify(&self, _level: Level, _text: &str) {}
    fn confirm(&self, _title: &str, _text: &str) -> bool {
        true
    }
    fn prompt(&self, _title: &str, default: &str) -> Option<String> {
        Some(default.to_owned())
    }
    fn show(&self, _title: &str, _text: &str) {}
    fn form(&self, form: ext::Form) -> Option<ext::Values> {
        Some(form.defaults())
    }
}

/// Runs the application's loop until nothing has arrived for a while and
/// nothing runs, the fetch of the list included.
fn settle(app: &mut App, events: &mut Vec<Event>, limit: Duration) -> Result<(), String> {
    let deadline = Instant::now() + limit;
    runtime::block_on(async {
        loop {
            let quiet = tokio::time::timeout(Duration::from_millis(200), app.tick(events))
                .await
                .is_err();
            if quiet && !app.is_busy() && !app.plugin_registry().checking {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(format!("not done after {} seconds", limit.as_secs()));
            }
        }
    })
}

/// The line the *Available* view says where it has no list.
fn said(problem: RegistryProblem) -> &'static str {
    match problem {
        RegistryProblem::Offline => "The plugin list could not be fetched.",
        RegistryProblem::Unverified => "The plugin list could not be verified",
        RegistryProblem::NewerRick => "This list needs a newer Rick",
        RegistryProblem::Unreadable => "The plugin list could not be read",
    }
}

/// The application on the config directory of `args`, without a window,
/// and the first line of the report: which registry it reads, and as
/// which Rick.
fn open(args: &Args) -> Result<(App, String), String> {
    let settings = args.config.join(app::SETTINGS_FILE);
    if !settings.is_file() {
        return Err(format!(
            "no {}: it names the registry to read",
            settings.display()
        ));
    }
    let config = std::fs::canonicalize(&args.config)
        .map_err(|e| format!("{}: {e}", args.config.display()))?;
    let mut app = App::new(Config {
        platform: Arc::new(platform::Native::new()),
        ui: Arc::new(Headless),
        extensions: Vec::new(),
        config_dir: Some(config.clone()),
        start: Some(Location::new(
            vfs_local::PROVIDER_ID,
            platform::paths::from_path(&config),
        )),
    });
    if let Some(rick) = &args.rick {
        app.set_rick_version(rick);
    }
    // What the per-user plugins folder holds is not on offer, as in the
    // dialogue. The folder beside a binary is not this program's to know:
    // it has none.
    let installed = std::fs::read_dir(app.plugins_dir())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|dir| dir.join("plugin.toml").is_file())
        .map(|dir| app::plugins::describe(&dir))
        .collect();
    app.set_plugins(installed);
    let registry = app.plugin_registry();
    if !registry.set_up {
        return Err(format!(
            "{} names no public key: the plugin list is not set up, and nobody is asked",
            settings.display()
        ));
    }
    let out = format!(
        "reading {} as Rick {} on {}\n",
        registry.custom.as_deref().unwrap_or(&registry.host),
        registry.rick,
        registry.platform
    );
    Ok((app, out))
}

/// *Check now*: fetches the list, and adds to `out` what it says of
/// Rick's newest release and which plugins the *Available* view would
/// offer. Those are returned.
fn check(
    app: &mut App,
    events: &mut Vec<Event>,
    out: &mut String,
) -> Result<Vec<Available>, String> {
    app.handle(Command::Registry(RegistryCommand::CheckNow), events);
    settle(app, events, FETCH_TIMEOUT).map_err(|e| format!("the list: {e}"))?;
    let registry = app.plugin_registry().clone();
    if let Some(problem) = registry.problem {
        return Err(said(problem).to_owned());
    }
    let index = registry
        .index
        .ok_or("no plugin list, and no reason given")?;
    // Writing into a String cannot fail.
    let _ = writeln!(
        out,
        "the list verified: schema {}, generated {}",
        index.schema, index.generated
    );
    match index.latest() {
        Some(latest) => {
            let _ = writeln!(out, "latest: Rick {} at {}", latest.version, latest.url);
        }
        None => out.push_str("latest: not named\n"),
    }
    match app.rick_update() {
        Some(update) => {
            let _ = writeln!(out, "update notice: Rick {} is available", update.version);
        }
        None => out.push_str("update notice: none\n"),
    }
    let offered = match app.offering() {
        Offering::Plugins(offered) => offered,
        Offering::NotSetUp => return Err("the plugin list is not set up".to_owned()),
        Offering::Nothing => return Err("no plugin list".to_owned()),
        Offering::PreRelease => {
            out.push_str("offered: nothing, this Rick's version has a pre-release part\n");
            Vec::new()
        }
        Offering::TooOld => {
            out.push_str("offered: nothing, the plugins for this Rick are no longer published\n");
            Vec::new()
        }
    };
    for plugin in &offered {
        let _ = writeln!(
            out,
            "offered: {} {} ({}), {} bytes",
            plugin.id, plugin.version, plugin.name, plugin.size
        );
    }
    Ok(offered)
}

/// **Install** for every plugin of `offered`, one after the other: each
/// has to arrive in the per-user plugins folder with a record that says
/// it came from the registry.
fn install(
    app: &mut App,
    events: &mut Vec<Event>,
    out: &mut String,
    offered: &[Available],
) -> Result<(), String> {
    if offered.is_empty() {
        return Err("nothing is on offer to install".to_owned());
    }
    let plugins_dir = app.plugins_dir();
    for plugin in offered {
        events.clear();
        app.handle(Command::InstallAvailable(plugin.id.clone()), events);
        settle(app, events, INSTALL_TIMEOUT).map_err(|e| format!("{}: {e}", plugin.id))?;
        let folder = plugins_dir.join(&plugin.id);
        let unpacked = events
            .iter()
            .any(|event| matches!(event, Event::PluginInstalled { path } if *path == folder));
        if !unpacked || !folder.join("plugin.toml").is_file() {
            return Err(format!(
                "{} was not installed: its package did not arrive, or did not match the plugin list",
                plugin.id
            ));
        }
        let record = Record::read(&plugins_dir);
        let entry = record
            .get(&folder)
            .filter(|entry| entry.source == Origin::Registry)
            .ok_or_else(|| format!("{}: the record does not say registry", plugin.id))?;
        let _ = writeln!(
            out,
            "installed: {} {} from the registry, sha256 {}, in {}",
            plugin.id,
            entry.version,
            entry.sha256,
            folder.display()
        );
    }
    // Installed, a plugin is no longer on offer.
    let left = match app.offering() {
        Offering::Plugins(left) => left.len(),
        _ => 0,
    };
    if left != 0 {
        return Err(format!(
            "{left} plugin(s) are still on offer after the install"
        ));
    }
    Ok(())
}

fn run(args: &Args) -> Result<String, String> {
    let (mut app, mut out) = open(args)?;
    let mut events = Vec::new();
    // What was found out before something failed is said with the failure.
    let offered = match check(&mut app, &mut events, &mut out) {
        Ok(offered) => offered,
        Err(error) => return Err(format!("{out}{error}")),
    };
    if !args.install {
        let _ = writeln!(out, "registry ok: {} offered", offered.len());
        return Ok(out);
    }
    if let Err(error) = install(&mut app, &mut events, &mut out, &offered) {
        return Err(format!("{out}{error}"));
    }
    let _ = writeln!(
        out,
        "registry ok: {} offered, {} installed",
        offered.len(),
        offered.len()
    );
    Ok(out)
}

fn main() -> ExitCode {
    // The binary's output is its purpose.
    let mut stdout = std::io::stdout().lock();
    let (report, code) = match args().and_then(|args| run(&args)) {
        Ok(report) => (report, ExitCode::SUCCESS),
        Err(report) => (
            format!("registry check failed: {report}\n"),
            ExitCode::FAILURE,
        ),
    };
    let _ = std::io::Write::write_all(&mut stdout, report.as_bytes());
    code
}
