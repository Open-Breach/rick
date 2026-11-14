//! The actions: *Install as application…* on an archive with its form,
//! *Installed Apps…* opening the view (from the palette and the burger
//! menu's *Installer* submenu), and the per-row actions of the view
//! (Open install folder, Launch, Update from archive…, Revert to previous
//! version, Edit launcher…, Uninstall…, Repair, Forget, Remove staging
//! folder). Everything long runs as a job: reading the archive is the job
//! *Reading archive…*, whose end opens the install form
//! ([`ActionOutcome::Later`]), and confirming the form starts the install
//! job.

use ext::t;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, CliCommand, CliCtx,
    CliOutcome, CommandSpec, Continuation, Field, Form, FormLater, Header, HeaderIcon, JobRunner,
    PreviewRow, Shortcut, UiHost, Values,
};
use types::{CancelToken, Entry, EntryKind, Error, Location, Progress, ProviderId};

use crate::archive::{Format, OnProgress};
use crate::detect::{self, CATEGORIES, Detected};
use crate::error::{InstallError, err};
use crate::install::{self, CommandLink, IconSource, Plan};
use crate::provider::AppsProvider;
use crate::record::{Installed, Kind, Layout, PathMode, Scan, glob_matches};
use crate::{is_view, location};

/// Install the selected archive as an application.
pub const INSTALL: ActionId = ActionId("installer.install");
/// Open the Installed Apps view.
pub const APPS: ActionId = ActionId("installer.apps");
/// Open the selected app's install folder.
pub const OPEN_FOLDER: ActionId = ActionId("installer.open_folder");
/// Launch the selected GUI app through its desktop entry.
pub const LAUNCH: ActionId = ActionId("installer.launch");
/// Update the selected app from an archive.
pub const UPDATE: ActionId = ActionId("installer.update");
/// Flip the selected app back to its previous version.
pub const REVERT: ActionId = ActionId("installer.revert");
/// Edit the selected app's launcher entry.
pub const EDIT_LAUNCHER: ActionId = ActionId("installer.edit_launcher");
/// Uninstall the selected app.
pub const UNINSTALL: ActionId = ActionId("installer.uninstall");
/// Rewrite what the selected app's record says is missing.
pub const REPAIR: ActionId = ActionId("installer.repair");
/// Delete the selected app's record only.
pub const FORGET: ActionId = ActionId("installer.forget");
/// Delete an orphaned staging folder.
pub const REMOVE_STAGING: ActionId = ActionId("installer.remove_staging");

const GROUP: ActionGroup = ActionGroup::Extension("Installer");

/// The form's field keys.
const K_NAME: &str = "name";
const K_ID: &str = "id";
const K_VERSION: &str = "version";
const K_KIND: &str = "kind";
const K_EXE: &str = "exe";
const K_ICON: &str = "icon";
const K_ICON_FILE: &str = "icon_file";
const K_CATEGORY: &str = "category";
const K_PATH_MODE: &str = "path_mode";
const K_ENV: &str = "env_var";
const K_SCOPE: &str = "scope";
const K_SHA: &str = "sha256";
const K_UPDATE_OF: &str = "update_of";
const K_OVERWRITE: &str = "overwrite";
const K_DOWNGRADE: &str = "downgrade";
const K_WM_CLASS: &str = "wm_class";
/// The two disclosure cards' open state.
const K_COMMANDS_OPEN: &str = "commands_open";
const K_ADVANCED_OPEN: &str = "advanced_open";
/// One checkbox per command the archive offers, up to this many.
const CMD_KEYS: [&str; 16] = [
    "cmd0", "cmd1", "cmd2", "cmd3", "cmd4", "cmd5", "cmd6", "cmd7", "cmd8", "cmd9", "cmd10",
    "cmd11", "cmd12", "cmd13", "cmd14", "cmd15",
];

/// The file's size and modification time: when they change, detection
/// runs again.
type Stamp = (u64, Option<SystemTime>);

/// The actions' shared state: the view's provider and the detections of
/// the archives looked at, so a second look at an unchanged file opens
/// the form without reading it again.
pub struct Installer {
    /// The view, whose last scan the actions read.
    pub provider: Arc<AppsProvider>,
    detections: Mutex<HashMap<PathBuf, (Stamp, Arc<Detected>)>>,
}

impl std::fmt::Debug for Installer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Installer")
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Installer {
    /// The actions' state over the view's provider.
    #[must_use]
    pub fn new(provider: Arc<AppsProvider>) -> Self {
        Self {
            provider,
            detections: Mutex::new(HashMap::new()),
        }
    }

    /// The detection kept from an earlier look at the file at `path`, when
    /// its size and modification time have not changed since.
    #[must_use]
    pub fn cached(&self, path: &Path) -> Option<Arc<Detected>> {
        let meta = std::fs::metadata(path).ok()?;
        let stamp: Stamp = (meta.len(), meta.modified().ok());
        lock(&self.detections)
            .get(path)
            .filter(|(kept, _)| *kept == stamp)
            .map(|(_, detected)| Arc::clone(detected))
    }

    /// Detects the archive at `path`, reading it once with `cancel`
    /// checked between entries and `progress` told the bytes read so far,
    /// and keeps the result for the next look at the same file. Blocking:
    /// a job's work.
    ///
    /// # Errors
    ///
    /// The errors of [`detect::detect_with`].
    pub fn detect_with(
        &self,
        path: &Path,
        cancel: &CancelToken,
        progress: &mut OnProgress<'_>,
    ) -> Result<Arc<Detected>, Error> {
        let meta = std::fs::metadata(path).map_err(Error::from_io)?;
        let stamp: Stamp = (meta.len(), meta.modified().ok());
        let detected = Arc::new(detect::detect_with(path, cancel, progress)?);
        lock(&self.detections).insert(path.to_path_buf(), (stamp, Arc::clone(&detected)));
        Ok(detected)
    }
}

/// The subcommand `rick install <archive>`.
pub const INSTALL_COMMAND: &str = "install";

/// The subcommand `rick install-rick <folder> <version>`: what the
/// integration dialog's *Install to the standard location* runs
/// (specs/system-integration.md §6.1).
pub const INSTALL_RICK_COMMAND: &str = "install-rick";

/// What the manifest declares for the command line: `install` and
/// `install-rick`.
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: INSTALL_COMMAND,
        help: "Install a tar or zip archive as an application (Linux)",
    },
    CommandSpec {
        name: INSTALL_RICK_COMMAND,
        help: "Install the Rick unpacked in a folder to the standard location, as a version (Linux)",
    },
];

/// Every command-line subcommand over `installer`.
#[must_use]
pub fn commands(installer: &Arc<Installer>) -> Vec<Arc<dyn CliCommand>> {
    vec![
        Arc::new(InstallCommand(Arc::clone(installer))),
        Arc::new(InstallRickCommand(Arc::clone(installer))),
    ]
}

/// Every action over `installer`.
#[must_use]
pub fn all(installer: &Arc<Installer>) -> Vec<Arc<dyn Action>> {
    vec![
        Arc::new(Install(Arc::clone(installer))),
        Arc::new(Apps),
        Arc::new(OpenFolder(Arc::clone(installer))),
        Arc::new(Launch(Arc::clone(installer))),
        Arc::new(Update(Arc::clone(installer))),
        Arc::new(Revert(Arc::clone(installer))),
        Arc::new(EditLauncher(Arc::clone(installer))),
        Arc::new(Uninstall(Arc::clone(installer))),
        Arc::new(Repair(Arc::clone(installer))),
        Arc::new(Forget(Arc::clone(installer))),
        Arc::new(RemoveStaging(Arc::clone(installer))),
    ]
}

/// The OS path of a local location.
fn local_path(loc: &Location) -> Option<PathBuf> {
    (loc.provider == ProviderId::LOCAL)
        .then(|| ext::platform::paths::to_path(&loc.path).ok())
        .flatten()
}

/// The one selected local archive, when the selection is exactly that.
fn selected_archive(ctx: &ActionCtx) -> Option<PathBuf> {
    let [entry] = ctx.selection.as_slice() else {
        return None;
    };
    if entry.id.provider != ProviderId::LOCAL || entry.kind != EntryKind::File {
        return None;
    }
    Format::by_name(&entry.name)?;
    local_path(&Location::new(entry.id.provider, entry.path.clone()))
}

/// The one selected row of the view, as an installed app.
fn selected_app(installer: &Installer, ctx: &ActionCtx) -> Option<Installed> {
    if !is_view(&ctx.location) {
        return None;
    }
    let [entry] = ctx.selection.as_slice() else {
        return None;
    };
    installer.provider.installed_of(entry)
}

fn selected_entry(ctx: &ActionCtx) -> Option<&Entry> {
    match ctx.selection.as_slice() {
        [entry] => Some(entry),
        _ => None,
    }
}

/// The installed app a detected archive matches: by app id, then by the
/// source pattern against the file name, then by the main executable's
/// path inside the archive.
#[must_use]
pub fn matching(scan: &Scan, detected: &Detected) -> Option<Installed> {
    let id = detected.app_id();
    let name = detected.archive_name();
    scan.apps
        .iter()
        .find(|a| a.record.id == id)
        .or_else(|| {
            scan.apps
                .iter()
                .find(|a| glob_matches(&a.record.source_pattern, &name))
        })
        .or_else(|| {
            scan.apps.iter().find(|a| {
                detected
                    .candidates
                    .iter()
                    .any(|c| c.rel == a.record.main_executable)
            })
        })
        .cloned()
}

/// The commands the archive offers to link for `kind`: one named after
/// the app id pointing at the main executable for a GUI application, the
/// `bin/` contents for a toolchain (or the same one command when it has
/// no `bin/`).
fn offered_commands(detected: &Detected, kind: Kind, id: &str, main: &str) -> Vec<CommandLink> {
    if kind == Kind::Gui || detected.bins.is_empty() {
        return vec![CommandLink {
            name: id.to_owned(),
            target: main.to_owned(),
        }];
    }
    detected
        .bins
        .iter()
        .map(|rel| CommandLink {
            name: rel.rsplit('/').next().unwrap_or(rel).to_owned(),
            target: rel.clone(),
        })
        .collect()
}

/// The text of an icon in the form's list: its path and pixel size.
fn icon_option(icon: &detect::IconFile) -> String {
    match (icon.ext.as_str(), icon.size) {
        ("svg", _) => format!("{} · scalable", icon.rel),
        (_, Some((w, h))) => format!("{} · {w}×{h}", icon.rel),
        (_, None) => icon.rel.clone(),
    }
}

/// What the form's values make of the archive: the plan when every row is
/// fine, and the rows of the summary either way.
#[derive(Debug, Default)]
pub struct Assembled {
    /// The plan, when nothing blocks it.
    pub plan: Option<Plan>,
    /// The summary rows, problems included.
    pub rows: Vec<PreviewRow>,
}

impl Assembled {
    /// The first problem, if any.
    #[must_use]
    pub fn problem(&self) -> Option<&str> {
        self.rows.iter().find_map(|r| r.problem.as_deref())
    }
}

/// The index of the main executable the values name, into
/// `detected.candidates`; `None` when the user still has to pick.
fn chosen_main(values: &Values, detected: &Detected) -> Option<usize> {
    let choice = values.choice(K_EXE);
    if detected.main.is_none() && detected.candidates.len() > 1 {
        // The list starts with "(choose one)".
        choice.checked_sub(1)
    } else {
        Some(choice)
    }
    .filter(|i| *i < detected.candidates.len())
}

/// Builds the plan and the summary from the form's values.
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn assemble(
    values: &Values,
    detected: &Detected,
    scan: &Scan,
    layout: &Layout,
    forced_update_of: Option<&Installed>,
) -> Assembled {
    let mut rows: Vec<PreviewRow> = Vec::new();
    let mut row = |from: &str, to: String, problem: Option<String>| {
        rows.push(PreviewRow {
            from: from.to_owned(),
            to,
            problem,
        });
    };

    // The archive itself.
    let archive_problem = if detected.source {
        Some(InstallError::Source.to_string())
    } else if let Some(found) = detected.arch_mismatch {
        Some(
            InstallError::Architecture {
                found: crate::elf::machine_name(found),
                running: crate::elf::current_machine()
                    .map_or_else(|| "unknown".to_owned(), crate::elf::machine_name),
            }
            .to_string(),
        )
    } else if detected.candidates.is_empty() {
        Some("the archive holds no program this machine can run".to_owned())
    } else {
        None
    };
    let mut archive_text = format!("{} ({})", detected.archive_name(), detected.format.name());
    if !detected.hostile.is_empty() {
        let _ = write!(
            archive_text,
            "; {} entries with unsafe paths will be left out",
            detected.hostile.len()
        );
    }
    row("Archive", archive_text, archive_problem);

    // Update or new install.
    let update_of: Option<Installed> = forced_update_of.cloned().or_else(|| {
        values
            .choice(K_UPDATE_OF)
            .checked_sub(1)
            .and_then(|i| scan.apps.get(i).cloned())
    });
    let name = values.text(K_NAME).trim().to_owned();
    let id = update_of.as_ref().map_or_else(
        || values.text(K_ID).trim().to_owned(),
        |u| u.record.id.clone(),
    );
    let version = values.text(K_VERSION).trim().to_owned();
    match &update_of {
        Some(installed) => {
            let old = &installed.record.version;
            let order = detect::compare_versions(&version, old);
            let problem = (order == std::cmp::Ordering::Less && !values.check(K_DOWNGRADE)).then(
                || {
                    format!(
                        "{} {old} is installed and {version} is older; tick Allow downgrade to continue",
                        installed.record.name
                    )
                },
            );
            row(
                "Mode",
                format!(
                    "update: {} {old} is installed, update to {version}",
                    installed.record.name
                ),
                problem,
            );
        }
        None => row("Mode", "new install".to_owned(), None),
    }

    let kind = if values.choice(K_KIND) == 1 {
        Kind::Cli
    } else {
        Kind::Gui
    };
    let scope_problem =
        (values.choice(K_SCOPE) == 1).then(|| InstallError::SystemUnavailable.to_string());
    row("Scope", "user space".to_owned(), scope_problem);

    let files_problem = if name.is_empty() {
        Some("the name is empty".to_owned())
    } else if !detect::valid_id(&id) {
        Some(format!(
            "the app id {id:?} must be lowercase letters, digits and hyphens"
        ))
    } else if !detect::valid_version(&version) {
        Some(format!("the version {version:?} cannot name a folder"))
    } else {
        None
    };
    row(
        "App files",
        format!(
            "{}/{}/",
            layout
                .app_dir(if id.is_empty() { "<id>" } else { &id })
                .display(),
            if version.is_empty() {
                "<version>"
            } else {
                &version
            }
        ),
        files_problem,
    );

    let main_index = chosen_main(values, detected);
    let main = main_index.map(|i| detected.candidates[i].rel.clone());
    row(
        "Main executable",
        main.clone()
            .map_or_else(|| "none chosen".to_owned(), |m| format!("current/{m}")),
        main.is_none()
            .then(|| "pick the main executable".to_owned()),
    );

    // Desktop entry and icon: the icon is one of the archive's or none.
    let mut collisions = Vec::new();
    let icon = values
        .choice(K_ICON)
        .checked_sub(1)
        .and_then(|i| detected.icons.get(i).cloned())
        .map_or(IconSource::None, IconSource::Archive);
    let categories = CATEGORIES
        .get(values.choice(K_CATEGORY))
        .copied()
        .unwrap_or("Utility")
        .to_owned();
    let wm_class = values.text(K_WM_CLASS).trim().to_owned();
    if kind == Kind::Gui {
        let desktop_path = layout.desktop_path(if id.is_empty() { "<id>" } else { &id });
        let own = update_of.as_ref().is_some_and(|u| {
            u.record
                .external_files
                .iter()
                .any(|f| layout.expand(&f.path) == desktop_path)
        });
        let collides =
            std::fs::symlink_metadata(&desktop_path).is_ok() && !scan.owns(&desktop_path) && !own;
        if collides {
            collisions.push(desktop_path.clone());
        }
        let problem = (collides && !values.check(K_OVERWRITE)).then(|| {
            InstallError::Collision {
                what: desktop_path.display().to_string(),
            }
            .to_string()
        });
        row(
            "Desktop entry",
            format!(
                "{} ({})",
                desktop_path.display(),
                if detected.desktop.is_some() {
                    "the vendor's, with Exec and Icon through current"
                } else {
                    "generated"
                }
            ),
            problem,
        );
        row(
            "Icon",
            match &icon {
                IconSource::Archive(file) => format!(
                    "{} → hicolor/{}/apps/{id}.{}",
                    file.rel,
                    file.size_dir(),
                    file.ext
                ),
                IconSource::File(path) => format!("{} → hicolor theme as {id}", path.display()),
                IconSource::None => "none".to_owned(),
            },
            None,
        );
    }

    // Commands.
    let path_mode = if kind == Kind::Cli && values.choice(K_PATH_MODE) == 1 {
        PathMode::Path
    } else {
        PathMode::Symlinks
    };
    // The command defaults follow the chosen kind. The checkboxes were
    // laid out for the detected kind; when the user switched the kind they
    // no longer describe the offer and the kind's defaults are taken
    // whole, which the summary rows show (QUESTIONS.md 212).
    let offered = offered_commands(detected, kind, &id, main.as_deref().unwrap_or(""));
    let commands: Vec<CommandLink> = if kind == detected.kind {
        offered
            .iter()
            .enumerate()
            .filter(|(i, _)| CMD_KEYS.get(*i).is_some_and(|k| values.check(k)))
            .map(|(_, c)| c.clone())
            .collect()
    } else {
        offered
    };
    let bin_dir = detected
        .bins
        .first()
        .and_then(|b| b.rsplit_once('/'))
        .map(|(dir, _)| dir.to_owned());
    let env_var = values.text(K_ENV).trim().to_owned();
    match path_mode {
        PathMode::Symlinks => {
            for command in &commands {
                let link = layout.bin_dir().join(&command.name);
                let own = update_of.as_ref().is_some_and(|u| {
                    u.record
                        .external_files
                        .iter()
                        .any(|f| layout.expand(&f.path) == link)
                });
                let collides =
                    std::fs::symlink_metadata(&link).is_ok() && !scan.owns(&link) && !own;
                if collides {
                    collisions.push(link.clone());
                }
                let problem = (collides && !values.check(K_OVERWRITE)).then(|| {
                    InstallError::Collision {
                        what: link.display().to_string(),
                    }
                    .to_string()
                });
                row(
                    "Command",
                    format!("{} → current/{}", link.display(), command.target),
                    problem,
                );
            }
            if !commands.is_empty() && !install::on_path(&layout.bin_dir()) {
                row(
                    "Hint",
                    format!(
                        "{} is not on your PATH; Rick does not edit shell profiles",
                        layout.bin_dir().display()
                    ),
                    None,
                );
            }
        }
        PathMode::Path => {
            let bin = bin_dir.clone().unwrap_or_else(|| "bin".to_owned());
            row(
                "PATH",
                format!(
                    "{} adds current/{bin} to the PATH of new shells",
                    layout.path_file().display()
                ),
                None,
            );
            let env_problem = env_var
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || c == '_'))
                .then(|| format!("{env_var:?} is not a variable name"));
            if !env_var.is_empty() {
                row(
                    "Variable",
                    format!("{env_var} = {}", layout.current(&id).display()),
                    env_problem,
                );
            }
        }
    }

    // Checksum.
    let sha = values.text(K_SHA).trim().to_ascii_lowercase();
    if !sha.is_empty() {
        let valid = sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit());
        row(
            "Checksum",
            "verified before extraction".to_owned(),
            (!valid).then(|| "the expected SHA-256 is 64 hex digits".to_owned()),
        );
    }
    if let Some(script) = &detected.install_script {
        row(
            "Not run",
            format!("{script} is in the archive and will not be run"),
            None,
        );
    }

    let plan = (rows.iter().all(|r| r.problem.is_none()) && main.is_some()).then(|| Plan {
        layout: layout.clone(),
        archive: detected.archive.clone(),
        format: detected.format,
        index: Arc::clone(&detected.index),
        id: id.clone(),
        name,
        version,
        kind,
        main: main.clone().unwrap_or_default(),
        icon,
        categories,
        wm_class: if wm_class.is_empty() {
            None
        } else {
            Some(wm_class)
        },
        vendor_desktop: detected.desktop.as_ref().map(|d| d.rel.clone()),
        commands,
        path_mode,
        bin_dir: (path_mode == PathMode::Path).then_some(bin_dir).flatten(),
        env_var: (path_mode == PathMode::Path && !env_var.is_empty()).then_some(env_var),
        expected_sha256: (!sha.is_empty()).then_some(sha),
        source_pattern: detected.source_pattern.clone(),
        update_of: update_of.map(|u| u.record),
        overwrite: values.check(K_OVERWRITE) || collisions.is_empty(),
        bins: detected.bins.clone(),
        launcher: true,
    });
    Assembled { plan, rows }
}

/// The install dialogue (SPEC.md *Install dialogue*): the header with the
/// app's icon, the mode pill and the version change; Name, App ID,
/// Version and *Install for*; the card with Kind, Main executable and
/// Icon; the *WHAT RICK WILL DO* card from the live summary; for a
/// toolchain the *Commands to link* disclosure; the *Advanced options*
/// disclosure with the rest. The archive is named in the footer.
#[allow(clippy::too_many_lines)]
pub fn install_form(
    detected: &Arc<Detected>,
    scan: &Scan,
    layout: &Layout,
    update_of: Option<&Installed>,
    forced: bool,
) -> Form {
    let matched = update_of;
    let id = matched.map_or_else(|| detected.app_id(), |m| m.record.id.clone());
    let title = match matched {
        Some(m) => format!(
            "{} {} is installed — update to {}?",
            m.record.name,
            m.record.version,
            if detected.version.is_empty() {
                "a new version"
            } else {
                &detected.version
            }
        ),
        None => format!("Install {} as application", detected.archive_name()),
    };
    let version_text = if detected.version.is_empty() {
        "?"
    } else {
        detected.version.as_str()
    };
    let (pill, subtitle) = match matched {
        Some(m) if !detected.version.is_empty() && m.record.version == detected.version => (
            Some(t!("installer-reinstall")),
            format!("{} → {version_text}", m.record.version),
        ),
        Some(m) => (
            Some(t!("installer-update")),
            format!("{} → {version_text}", m.record.version),
        ),
        None => (None, version_text.to_owned()),
    };
    let mut header = Header::new(header_icon(detected, &id))
        .title(&detected.name)
        .subtitle(subtitle);
    if let Some(pill) = pill {
        header = header.pill(pill);
    }
    let ok = match (matched, pill_is_reinstall(matched, detected)) {
        (Some(_), true) => t!("installer-reinstall"),
        (Some(_), false) => t!("installer-update"),
        (None, _) => t!("installer-install"),
    };
    let mut form = Form::new(title, ok)
        .header(header)
        .note(t!(
            "installer-archive-note",
            name = detected.archive_name().as_str()
        ))
        .preview_title(t!("installer-what-rick-will-do"))
        .field(Field::text(
            K_NAME,
            t!("installer-name"),
            &detected.name,
            "The name in the launcher",
        ))
        .field(Field::text(K_ID, t!("installer-app-id"), &id, "lowercase-with-hyphens").mono())
        .field(
            Field::text(
                K_VERSION,
                t!("installer-version"),
                &detected.version,
                t!("installer-version-hint"),
            )
            .required(t!("installer-version-missing")),
        )
        .field(Field::segmented(
            K_SCOPE,
            t!("installer-install-for"),
            vec![t!("installer-just-me"), t!("installer-all-users")],
            0,
        ))
        .field(Field::section(None, true))
        .field(Field::choice(
            K_KIND,
            t!("installer-kind"),
            vec!["GUI application".to_owned(), "Command-line tool".to_owned()],
            usize::from(detected.kind == Kind::Cli),
        ));
    let mut exe_options: Vec<String> = detected.candidates.iter().map(|c| c.rel.clone()).collect();
    let exe_selected = match detected.main {
        Some(i) => i,
        None if exe_options.len() > 1 => {
            exe_options.insert(0, "(choose one)".to_owned());
            0
        }
        None => {
            if exe_options.is_empty() {
                exe_options.push("(no program found)".to_owned());
            }
            0
        }
    };
    form = form.field(
        Field::choice(
            K_EXE,
            t!("installer-main-executable"),
            exe_options,
            exe_selected,
        )
        .mono(),
    );
    // One list: (none) — or what a missing icon becomes — then every PNG
    // and SVG of the archive, best first, the best preselected only when
    // detection trusts it.
    let none = if detected.icons.is_empty() {
        t!("installer-no-icon-generated")
    } else {
        "(none)".to_owned()
    };
    let mut icon_options = vec![none];
    icon_options.extend(detected.icons.iter().map(icon_option));
    form = form.field(
        Field::choice(
            K_ICON,
            t!("installer-icon"),
            icon_options,
            usize::from(detected.icon_trusted && !detected.icons.is_empty()),
        )
        .mono(),
    );
    if detected.icons.is_empty() {
        form = form.field(Field::file(
            K_ICON_FILE,
            t!("installer-icon-file"),
            "",
            vec![("Images", &["svg", "png"][..])],
        ));
    }
    form = form.field(Field::preview_here());
    // The commands: a toolchain's `bin/` under *Commands to link*, a GUI
    // application's one command under Advanced options.
    let main = detected.main_rel().unwrap_or("");
    let offered = offered_commands(detected, detected.kind, &id, main);
    let mut command_fields: Vec<Field> = Vec::new();
    for (i, command) in offered.iter().enumerate() {
        let Some(key) = CMD_KEYS.get(i) else {
            break;
        };
        let target = if command.target.is_empty() {
            "<main executable>"
        } else {
            &command.target
        };
        command_fields.push(Field::check(
            key,
            format!("Link command {} → current/{target}", command.name),
            true,
        ));
    }
    if offered.len() > CMD_KEYS.len() {
        command_fields.push(Field::check(
            "cmd_more",
            format!(
                "{} more commands in bin/ are not linked; PATH mode puts them all on the PATH",
                offered.len() - CMD_KEYS.len()
            ),
            false,
        ));
    }
    let toolchain = detected.kind == Kind::Cli;
    if toolchain && !command_fields.is_empty() {
        form = form.field(
            Field::disclosure(K_COMMANDS_OPEN, t!("installer-commands-to-link"), "", false)
                .counted(),
        );
        for field in command_fields.drain(..) {
            form = form.field(field);
        }
    }
    let category_index = detected
        .desktop
        .as_ref()
        .and_then(|d| d.entry.get("Categories"))
        .and_then(|cats| {
            cats.split(';')
                .find_map(|c| CATEGORIES.iter().position(|known| *known == c.trim()))
        })
        .unwrap_or(0);
    form = form
        .field(Field::disclosure(
            K_ADVANCED_OPEN,
            t!("installer-advanced-options"),
            t!("installer-advanced-summary"),
            false,
        ))
        .field(Field::choice(
            K_CATEGORY,
            t!("installer-category-gui"),
            CATEGORIES.iter().map(|c| (*c).to_owned()).collect(),
            category_index,
        ))
        .field(Field::text(
            K_WM_CLASS,
            t!("installer-startupwmclass-gui"),
            detected.wm_class().unwrap_or_default(),
            "editable later in Installed Apps",
        ));
    for field in command_fields {
        form = form.field(field);
    }
    form = form
        .field(Field::choice(
            K_PATH_MODE,
            t!("installer-commands-cli"),
            vec![
                format!("Symlinks in {}", layout.bin_dir().display()),
                "Add the bin folder to the PATH of new shells".to_owned(),
            ],
            0,
        ))
        .field(Field::text(
            K_ENV,
            t!("installer-variable-path-mode"),
            "",
            "JAVA_HOME, pointing at the version folder",
        ))
        .field(
            Field::text(
                K_SHA,
                t!("installer-expected-sha-256"),
                "",
                "optional; the hash the vendor publishes",
            )
            .mono(),
        );
    if !forced {
        let mut update_options = vec!["New install".to_owned()];
        update_options.extend(
            scan.apps
                .iter()
                .map(|a| format!("{} ({}) {}", a.record.name, a.record.id, a.record.version)),
        );
        let selected = matched
            .and_then(|m| scan.apps.iter().position(|a| a.record.id == m.record.id))
            .map_or(0, |i| i + 1);
        form = form.field(Field::choice(
            K_UPDATE_OF,
            t!("installer-this-is-an-update-of"),
            update_options,
            selected,
        ));
    }
    form = form
        .field(Field::check(
            K_OVERWRITE,
            t!("installer-overwrite-command-links-and-desktop-entries"),
            false,
        ))
        .field(Field::check(
            K_DOWNGRADE,
            t!("installer-allow-downgrade"),
            false,
        ));
    let preview_detected = Arc::clone(detected);
    let preview_scan = scan.clone();
    let preview_layout = layout.clone();
    let preview_forced = forced.then(|| update_of.cloned()).flatten();
    form.preview(Arc::new(move |values: &Values| {
        assemble(
            values,
            &preview_detected,
            &preview_scan,
            &preview_layout,
            preview_forced.as_ref(),
        )
        .rows
    }))
}

/// Whether the dialogue is a reinstall: the installed version is the
/// archive's.
fn pill_is_reinstall(matched: Option<&Installed>, detected: &Detected) -> bool {
    matched.is_some_and(|m| !detected.version.is_empty() && m.record.version == detected.version)
}

/// The header's icon: the trusted PNG icon of the archive when its bytes
/// were kept, else a monogram of the name on a colour from the app id.
fn header_icon(detected: &Detected, id: &str) -> HeaderIcon {
    let png = detected
        .icons
        .iter()
        .filter(|_| detected.icon_trusted)
        .find(|icon| icon.ext == "png" && icon.data.is_some())
        .and_then(|icon| icon.data.clone());
    match png {
        Some(bytes) => HeaderIcon::Image { bytes, svg: false },
        None => HeaderIcon::Monogram {
            letter: detected
                .name
                .chars()
                .find(|c| c.is_alphanumeric())
                .unwrap_or('?'),
            seed: id.to_owned(),
        },
    }
}

/// Spawns the install job for `plan`.
fn spawn_install(
    installer: &Arc<Installer>,
    plan: Plan,
    scan: Scan,
    jobs: &JobRunner,
) -> ActionOutcome {
    let name = if plan.is_update() {
        format!("Update {} to {}", plan.name, plan.version)
    } else {
        format!("Install {} {}", plan.name, plan.version)
    };
    let staging = plan.layout.staging(&plan.id, &plan.version);
    installer.provider.set_busy(&staging, true);
    let platform = Arc::clone(installer.provider.platform());
    let installer = Arc::clone(installer);
    let handle = jobs.spawn_named(name, move |job| async move {
        let progress = job.progress.clone();
        let cancel = job.cancel.clone();
        let result = ext::blocking(move || {
            install::install(&plan, &scan, &*platform, &progress, &cancel).map(drop)
        })
        .await;
        installer.provider.set_busy(&staging, false);
        result
    });
    ActionOutcome::Started(handle.id)
}

/// The name of the job that reads an archive for its install form.
pub const READING_ARCHIVE: &str = "Reading archive…";

/// Opens the install form once the archive is read and, on confirmation,
/// runs the install job. Reading the archive is the job *Reading archive…*
/// with byte progress over the file and a cancel; a detection kept from
/// an earlier look at the same file opens the form at once. The form and
/// the rest of the action travel through the [`FormLater`] the outcome
/// carries: the host shows the form, the application runs the rest.
fn install_through_form(
    installer: &Arc<Installer>,
    archive: &Path,
    update_of: Option<&Installed>,
    jobs: &JobRunner,
) -> ActionOutcome {
    let (sender, later) = FormLater::channel();
    let label = if update_of.is_some() {
        t!("installer-update-from-archive")
    } else {
        t!("installer-install-as-application")
    };
    // From the detection to the form and the rest of the action.
    let make = {
        let installer = Arc::clone(installer);
        let jobs = jobs.clone();
        let update_of = update_of.cloned();
        move |detected: Arc<Detected>| -> (Form, Continuation) {
            let scan = installer.provider.rescan();
            let layout = installer.provider.user_layout();
            let forced = update_of.is_some();
            let matched = update_of.clone().or_else(|| matching(&scan, &detected));
            let form = install_form(&detected, &scan, &layout, matched.as_ref(), forced);
            let then = Continuation::new(label, move |values| {
                let Some(values) = values else {
                    return Ok(ActionOutcome::Done);
                };
                let assembled = assemble(&values, &detected, &scan, &layout, update_of.as_ref());
                if let Some(problem) = assembled.problem() {
                    return Err(err(InstallError::Invalid(problem.to_owned())));
                }
                let Some(plan) = assembled.plan else {
                    return Err(err(InstallError::Invalid(
                        "pick the main executable".to_owned(),
                    )));
                };
                Ok(spawn_install(&installer, plan, scan, &jobs))
            });
            (form, then)
        }
    };
    if let Some(detected) = installer.cached(archive) {
        let _ = sender.send(make(detected));
        return ActionOutcome::Later(later);
    }
    let installer = Arc::clone(installer);
    let archive = archive.to_path_buf();
    jobs.spawn_named(READING_ARCHIVE, move |job| async move {
        let total = std::fs::metadata(&archive).ok().map(|m| m.len());
        let progress = job.progress.clone();
        let cancel = job.cancel.clone();
        progress.report(Progress::Bytes { done: 0, total });
        let detected = ext::blocking(move || {
            installer.detect_with(&archive, &cancel, &mut |done| {
                progress.report(Progress::Bytes { done, total });
            })
        })
        .await?;
        let _ = sender.send(make(detected));
        Ok(())
    });
    ActionOutcome::Later(later)
}

/// `rick install <archive>`: *Install as application…* from the command
/// line. The archive, resolved against the caller's working directory, is
/// read by the job *Reading archive…* and the install form opens in the
/// running Rick with every guess editable; the caller's terminal gets
/// exit status zero at once, and an error (no such file, not an archive,
/// another platform) with status 1.
pub struct InstallCommand(Arc<Installer>);

impl CliCommand for InstallCommand {
    fn name(&self) -> &'static str {
        INSTALL_COMMAND
    }

    fn run(&self, ctx: CliCtx, jobs: &JobRunner, _ui: &dyn UiHost) -> Result<CliOutcome, Error> {
        if !crate::supported() {
            return Err(err(InstallError::Unsupported));
        }
        let [archive] = ctx.args.as_slice() else {
            return Err(Error::Unsupported("usage: rick install <archive>"));
        };
        let path = Path::new(archive);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            ctx.cwd.join(path)
        };
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let name = path
            .file_name()
            .map_or_default(|n| n.to_string_lossy().into_owned());
        if Format::by_name(&name).is_none() {
            return Err(Error::Unsupported(
                "the archive must be a tar (gz, xz, bz2, zst) or zip file",
            ));
        }
        if !std::fs::metadata(&path).map_err(Error::from_io)?.is_file() {
            return Err(Error::Unsupported("the archive must be a file"));
        }
        Ok(CliOutcome::Outcome(install_through_form(
            &self.0, &path, None, jobs,
        )))
    }
}

/// `rick install-rick <folder> <version>`: installs the Rick unpacked in
/// `folder` — the folder that holds the binary, its libraries, `plugins/`
/// and `share/` — to the standard location, `~/.local/opt/rick/<version>`
/// with the `current` link, as one job with progress and a cancel. No
/// form: there is nothing to guess about Rick. The folder is copied as it
/// is, never packed first; nothing is written outside the app folder (the
/// launcher entry and the `rick` command are integration items of their
/// own); a Rick installed before is updated, its version kept as the
/// previous one. This is what the integration dialog's *Install to the
/// standard location* runs (specs/system-integration.md §6.1), and the
/// caller's terminal gets exit status zero once the job has started.
pub struct InstallRickCommand(Arc<Installer>);

impl CliCommand for InstallRickCommand {
    fn name(&self) -> &'static str {
        INSTALL_RICK_COMMAND
    }

    fn run(&self, ctx: CliCtx, jobs: &JobRunner, _ui: &dyn UiHost) -> Result<CliOutcome, Error> {
        if !crate::supported() {
            return Err(err(InstallError::Unsupported));
        }
        let [folder, version] = ctx.args.as_slice() else {
            return Err(Error::Unsupported(
                "usage: rick install-rick <folder> <version>",
            ));
        };
        let folder = Path::new(folder);
        let folder = if folder.is_absolute() {
            folder.to_path_buf()
        } else {
            ctx.cwd.join(folder)
        };
        let folder = std::fs::canonicalize(&folder).unwrap_or(folder);
        if !folder.join(install::RICK_MAIN).is_file() {
            return Err(Error::Unsupported("the folder must hold the rick binary"));
        }
        if !detect::valid_version(version) {
            return Err(err(InstallError::Invalid(format!(
                "the version {version:?} cannot name a folder"
            ))));
        }
        let version = version.clone();
        let installer = Arc::clone(&self.0);
        let layout = installer.provider.user_layout();
        let staging = layout.staging(install::RICK_ID, &version);
        installer.provider.set_busy(&staging, true);
        let platform = Arc::clone(installer.provider.platform());
        let name = format!("Install Rick {version}");
        let handle = jobs.spawn_named(name, move |job| async move {
            let progress = job.progress.clone();
            let cancel = job.cancel.clone();
            let provider = Arc::clone(&installer.provider);
            let result = ext::blocking(move || {
                let index = crate::archive::scan(
                    &folder,
                    Format::Folder,
                    &cancel,
                    &mut |_| {},
                    &mut |_, _| Ok(()),
                )?;
                let scan = provider.rescan();
                let update_of = scan
                    .apps
                    .iter()
                    .find(|app| {
                        app.layout.scope == layout.scope && app.record.id == install::RICK_ID
                    })
                    .map(|app| app.record.clone());
                let plan =
                    install::rick_plan(layout, &folder, &version, Arc::new(index), update_of);
                install::install(&plan, &scan, &*platform, &progress, &cancel).map(drop)
            })
            .await;
            installer.provider.set_busy(&staging, false);
            result
        });
        Ok(CliOutcome::Outcome(ActionOutcome::Started(handle.id)))
    }
}

/// *Install as application…* on a selected archive.
pub struct Install(Arc<Installer>);

impl Action for Install {
    fn id(&self) -> ActionId {
        INSTALL
    }
    fn label(&self) -> String {
        t!("installer-install-as-application")
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !crate::supported() || selected_archive(ctx).is_none() {
            Availability::Hidden
        } else {
            Availability::Enabled
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        if !crate::supported() {
            return Err(err(InstallError::Unsupported));
        }
        let archive = selected_archive(&ctx).ok_or(Error::Unsupported(
            "select one tar or zip archive on the local disk",
        ))?;
        Ok(install_through_form(&self.0, &archive, None, jobs))
    }
}

/// *Installed Apps…*: opens the view; listed in the application menu
/// under *Installer* as well.
pub struct Apps;

impl Action for Apps {
    fn id(&self) -> ActionId {
        APPS
    }
    fn label(&self) -> String {
        t!("installer-installed-apps")
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn app_menu(&self) -> bool {
        true
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !crate::supported() || is_view(&ctx.location) {
            Availability::Hidden
        } else {
            Availability::Enabled
        }
    }
    fn run(
        &self,
        _ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        if !crate::supported() {
            return Err(err(InstallError::Unsupported));
        }
        Ok(ActionOutcome::Navigate(location()))
    }
}

/// The availability of a per-row action: hidden outside the view and
/// without exactly one app row selected.
fn row_availability(installer: &Installer, ctx: &ActionCtx) -> Option<Installed> {
    if !crate::supported() {
        return None;
    }
    selected_app(installer, ctx)
}

/// *Open install folder*.
pub struct OpenFolder(Arc<Installer>);

impl Action for OpenFolder {
    fn id(&self) -> ActionId {
        OPEN_FOLDER
    }
    fn label(&self) -> String {
        t!("installer-open-install-folder")
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if row_availability(&self.0, ctx).is_some() {
            Availability::Enabled
        } else {
            Availability::Hidden
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let installed = selected_app(&self.0, &ctx).ok_or(Error::NotFound)?;
        Ok(ActionOutcome::Navigate(Location::new(
            ProviderId::LOCAL,
            ext::platform::paths::from_path(&installed.dir),
        )))
    }
}

/// *Launch*, for GUI apps.
pub struct Launch(Arc<Installer>);

impl Action for Launch {
    fn id(&self) -> ActionId {
        LAUNCH
    }
    fn label(&self) -> String {
        t!("installer-launch")
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        match row_availability(&self.0, ctx) {
            Some(installed) if installed.record.kind == Kind::Gui => Availability::Enabled,
            _ => Availability::Hidden,
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let installed = selected_app(&self.0, &ctx).ok_or(Error::NotFound)?;
        install::launch(&installed)?;
        Ok(ActionOutcome::Done)
    }
}

/// *Update from archive…*: asks for the archive, then runs the install
/// form in update mode.
pub struct Update(Arc<Installer>);

impl Action for Update {
    fn id(&self) -> ActionId {
        UPDATE
    }
    fn label(&self) -> String {
        t!("installer-update-from-archive")
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if row_availability(&self.0, ctx).is_some() {
            Availability::Enabled
        } else {
            Availability::Hidden
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let installed = selected_app(&self.0, &ctx).ok_or(Error::NotFound)?;
        let Some(text) = ui.prompt(
            &format!("Update {} from archive", installed.record.name),
            "",
        ) else {
            return Ok(ActionOutcome::Done);
        };
        let text = text.trim();
        if text.is_empty() {
            return Ok(ActionOutcome::Done);
        }
        let archive = installed.layout.expand(text);
        if !archive.is_file() {
            return Err(Error::NotFound);
        }
        if Format::by_name(text).is_none() {
            return Err(Error::Unsupported(
                "the archive must be a tar (gz, xz, bz2, zst) or zip file",
            ));
        }
        Ok(install_through_form(
            &self.0,
            &archive,
            Some(&installed),
            jobs,
        ))
    }
}

/// *Revert to previous version*.
pub struct Revert(Arc<Installer>);

impl Action for Revert {
    fn id(&self) -> ActionId {
        REVERT
    }
    fn label(&self) -> String {
        t!("installer-revert-to-previous-version")
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        match row_availability(&self.0, ctx) {
            None => Availability::Hidden,
            Some(installed) => match &installed.record.previous_version {
                Some(previous) if installed.dir.join(previous).is_dir() => Availability::Enabled,
                _ => Availability::Disabled(t!("installer-no-previous-version-is-kept")),
            },
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let installed = selected_app(&self.0, &ctx).ok_or(Error::NotFound)?;
        let previous = installed
            .record
            .previous_version
            .clone()
            .ok_or_else(|| err(InstallError::NoPrevious(installed.record.name.clone())))?;
        let name = format!("Revert {} to {previous}", installed.record.name);
        let handle = jobs.spawn_named(name.clone(), move |job| async move {
            job.progress.report(Progress::Message(name));
            ext::blocking(move || install::revert(&installed).map(drop)).await
        });
        Ok(ActionOutcome::Started(handle.id))
    }
}

/// *Edit launcher…*: name, icon, category and `StartupWMClass`.
pub struct EditLauncher(Arc<Installer>);

impl Action for EditLauncher {
    fn id(&self) -> ActionId {
        EDIT_LAUNCHER
    }
    fn label(&self) -> String {
        t!("installer-edit-launcher")
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        match row_availability(&self.0, ctx) {
            Some(installed) if installed.record.kind == Kind::Gui => Availability::Enabled,
            _ => Availability::Hidden,
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let installed = selected_app(&self.0, &ctx).ok_or(Error::NotFound)?;
        let current = installed
            .desktop_entry()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| crate::desktop::DesktopEntry::parse(&t));
        let category_index = current
            .as_ref()
            .and_then(|e| e.get("Categories"))
            .and_then(|cats| {
                cats.split(';')
                    .find_map(|c| CATEGORIES.iter().position(|known| *known == c.trim()))
            })
            .unwrap_or(0);
        let form = Form::new(
            format!("Edit launcher of {}", installed.record.name),
            "Save",
        )
        .field(Field::text(
            K_NAME,
            t!("installer-name"),
            &installed.record.name,
            "",
        ))
        .field(Field::text(
            K_ICON_FILE,
            t!("installer-icon-file"),
            "",
            "a PNG or SVG path, to replace the icon",
        ))
        .field(Field::choice(
            K_CATEGORY,
            t!("installer-category"),
            CATEGORIES.iter().map(|c| (*c).to_owned()).collect(),
            category_index,
        ))
        .field(Field::text(
            K_WM_CLASS,
            t!("installer-startupwmclass"),
            installed
                .record
                .startup_wm_class
                .clone()
                .unwrap_or_default(),
            "the window class, once the app has run",
        ));
        let Some(values) = ui.form(form) else {
            return Ok(ActionOutcome::Done);
        };
        let icon_file = values.text(K_ICON_FILE).trim().to_owned();
        let icon = (!icon_file.is_empty()).then(|| installed.layout.expand(&icon_file));
        install::edit_launcher(
            &installed,
            values.text(K_NAME),
            icon.as_deref(),
            CATEGORIES
                .get(values.choice(K_CATEGORY))
                .unwrap_or(&"Utility"),
            values.text(K_WM_CLASS),
        )?;
        self.0.provider.rescan();
        Ok(ActionOutcome::Done)
    }
}

/// *Uninstall…*, after a confirmation that lists what goes.
pub struct Uninstall(Arc<Installer>);

impl Action for Uninstall {
    fn id(&self) -> ActionId {
        UNINSTALL
    }
    fn label(&self) -> String {
        t!("installer-uninstall")
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if row_availability(&self.0, ctx).is_some() {
            Availability::Enabled
        } else {
            Availability::Hidden
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let installed = selected_app(&self.0, &ctx).ok_or(Error::NotFound)?;
        let scan = self.0.provider.scan();
        let others: Vec<_> = scan
            .apps
            .iter()
            .filter(|a| a.record.id != installed.record.id)
            .map(|a| a.record.clone())
            .collect();
        let last_path_mode = installed.record.path_mode == Some(PathMode::Path)
            && !others.iter().any(|r| r.path_mode == Some(PathMode::Path))
            && install::has_profile_line(&installed.layout);
        let mut text = format!(
            "Uninstall {} {}? This removes exactly:\n",
            installed.record.name, installed.record.version
        );
        for path in install::uninstall_list(&installed) {
            let _ = writeln!(text, "  {}", path.display());
        }
        text.push_str(
            "The app's own settings and data, for instance under ~/.config and ~/.local/share, are left untouched.",
        );
        if last_path_mode {
            text.push_str(
                "\nThis is the last PATH-mode toolchain: the line Rick added to ~/.profile is removed too.",
            );
        }
        if !ui.confirm(&t!("installer-uninstall-2"), &text) {
            return Ok(ActionOutcome::Done);
        }
        let name = format!("Uninstall {}", installed.record.name);
        let handle = jobs.spawn_named(name.clone(), move |job| async move {
            job.progress.report(Progress::Message(name));
            ext::blocking(move || install::uninstall(&installed, &others, last_path_mode)).await
        });
        Ok(ActionOutcome::Started(handle.id))
    }
}

/// *Repair*: rewrites what the record says is missing.
pub struct Repair(Arc<Installer>);

impl Action for Repair {
    fn id(&self) -> ActionId {
        REPAIR
    }
    fn label(&self) -> String {
        t!("installer-repair")
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        match row_availability(&self.0, ctx) {
            None => Availability::Hidden,
            Some(installed) if installed.problems.is_empty() => {
                Availability::Disabled(t!("installer-nothing-to-repair"))
            }
            Some(_) => Availability::Enabled,
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let installed = selected_app(&self.0, &ctx).ok_or(Error::NotFound)?;
        let name = format!("Repair {}", installed.record.name);
        let handle = jobs.spawn_named(name, move |job| async move {
            job.progress
                .report(Progress::Message(install::repair_text(&installed.problems)));
            let done = ext::blocking(move || install::repair(&installed)).await?;
            job.progress
                .report(Progress::Message(format!("Repaired: {}", done.join(", "))));
            Ok(())
        });
        Ok(ActionOutcome::Started(handle.id))
    }
}

/// *Forget*: deletes the record and nothing else.
pub struct Forget(Arc<Installer>);

impl Action for Forget {
    fn id(&self) -> ActionId {
        FORGET
    }
    fn label(&self) -> String {
        t!("installer-forget")
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if row_availability(&self.0, ctx).is_some() {
            Availability::Enabled
        } else {
            Availability::Hidden
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let installed = selected_app(&self.0, &ctx).ok_or(Error::NotFound)?;
        if !ui.confirm(
            "Forget",
            &format!(
                "Forget {}? Only its record is deleted; the files stay where they are.",
                installed.record.name
            ),
        ) {
            return Ok(ActionOutcome::Done);
        }
        install::forget(&installed)?;
        self.0.provider.rescan();
        Ok(ActionOutcome::Done)
    }
}

/// *Remove staging folder*, on an orphaned staging folder's row.
pub struct RemoveStaging(Arc<Installer>);

impl Action for RemoveStaging {
    fn id(&self) -> ActionId {
        REMOVE_STAGING
    }
    fn label(&self) -> String {
        t!("installer-remove-staging-folder")
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        let orphan = crate::supported()
            && is_view(&ctx.location)
            && selected_entry(ctx).is_some_and(|e| self.0.provider.orphan_of(e).is_some());
        if orphan {
            Availability::Enabled
        } else {
            Availability::Hidden
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let orphan = selected_entry(&ctx)
            .and_then(|e| self.0.provider.orphan_of(e))
            .ok_or(Error::NotFound)?;
        if !ui.confirm(
            "Remove staging folder",
            &format!(
                "Remove {}? It is what an interrupted install of {} {} left behind.",
                orphan.dir.display(),
                orphan.app_id,
                orphan.version
            ),
        ) {
            return Ok(ActionOutcome::Done);
        }
        std::fs::remove_dir_all(&orphan.dir).map_err(Error::from_io)?;
        self.0.provider.rescan();
        Ok(ActionOutcome::Done)
    }
}
