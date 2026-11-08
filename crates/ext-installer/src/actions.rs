//! The actions: *Install as application…* on an archive with its form,
//! *Installed apps* opening the view, and the per-row actions of the view
//! (Open install folder, Launch, Update from archive…, Revert to previous
//! version, Edit launcher…, Uninstall…, Repair, Forget, Remove staging
//! folder). Everything long runs as a job; the form is one dialogue per
//! action run, as the UI host's protocol asks.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Field, Form, JobRunner,
    PreviewRow, Shortcut, UiHost, Values,
};
use types::{Entry, EntryKind, Error, Location, Progress, ProviderId};

use crate::archive::Format;
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
/// One checkbox per command the archive offers, up to this many.
const CMD_KEYS: [&str; 16] = [
    "cmd0", "cmd1", "cmd2", "cmd3", "cmd4", "cmd5", "cmd6", "cmd7", "cmd8", "cmd9", "cmd10",
    "cmd11", "cmd12", "cmd13", "cmd14", "cmd15",
];

/// The file's size and modification time: when they change, detection
/// runs again.
type Stamp = (u64, Option<SystemTime>);

/// The actions' shared state: the view's provider, the detections of the
/// archives looked at (the form protocol runs an action twice), and the
/// archive an *Update from archive…* is in the middle of.
pub struct Installer {
    /// The view, whose last scan the actions read.
    pub provider: Arc<AppsProvider>,
    detections: Mutex<HashMap<PathBuf, (Stamp, Arc<Detected>)>>,
    pending_updates: Mutex<HashMap<String, PathBuf>>,
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
            pending_updates: Mutex::new(HashMap::new()),
        }
    }

    /// Detects the archive at `path`, or returns the detection kept from
    /// an earlier look at the same file.
    ///
    /// # Errors
    ///
    /// The errors of [`detect::detect`].
    pub fn detect(&self, path: &Path) -> Result<Arc<Detected>, Error> {
        let meta = std::fs::metadata(path).map_err(Error::from_io)?;
        let stamp: Stamp = (meta.len(), meta.modified().ok());
        if let Some((kept, detected)) = lock(&self.detections).get(path)
            && *kept == stamp
        {
            return Ok(Arc::clone(detected));
        }
        let detected = Arc::new(detect::detect(path)?);
        lock(&self.detections).insert(path.to_path_buf(), (stamp, Arc::clone(&detected)));
        Ok(detected)
    }

    fn pending_update(&self, id: &str) -> Option<PathBuf> {
        let mut pending = lock(&self.pending_updates);
        let path = pending.get(id)?.clone();
        if path.is_file() {
            Some(path)
        } else {
            pending.remove(id);
            None
        }
    }

    fn set_pending_update(&self, id: &str, path: &Path) {
        lock(&self.pending_updates).insert(id.to_owned(), path.to_path_buf());
    }

    fn clear_pending_update(&self, id: &str) {
        lock(&self.pending_updates).remove(id);
    }
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

/// The commands the archive offers to link: the `bin/` contents of a
/// toolchain, or one named after the app id for everything else.
fn offered_commands(detected: &Detected, id: &str, main: &str) -> Vec<CommandLink> {
    if detected.bins.is_empty() {
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

    // Desktop entry and icon.
    let mut collisions = Vec::new();
    let icon_file = values.text(K_ICON_FILE).trim().to_owned();
    let icon = if icon_file.is_empty() {
        values
            .choice(K_ICON)
            .checked_sub(1)
            .and_then(|i| detected.icons.get(i).cloned())
            .map_or(IconSource::None, IconSource::Archive)
    } else {
        IconSource::File(layout.expand(&icon_file))
    };
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
        let icon_problem = match &icon {
            IconSource::File(path) if !path.is_file() => {
                Some(format!("the icon file {} does not exist", path.display()))
            }
            _ => None,
        };
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
            icon_problem,
        );
    }

    // Commands.
    let path_mode = if kind == Kind::Cli && values.choice(K_PATH_MODE) == 1 {
        PathMode::Path
    } else {
        PathMode::Symlinks
    };
    let offered = offered_commands(detected, &id, main.as_deref().unwrap_or(""));
    let commands: Vec<CommandLink> = offered
        .iter()
        .enumerate()
        .filter(|(i, _)| CMD_KEYS.get(*i).is_some_and(|k| values.check(k)))
        .map(|(_, c)| c.clone())
        .collect();
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
    });
    Assembled { plan, rows }
}

/// The install form for `detected`, prefilled with detection's defaults
/// and, for an update, the installed app's.
#[allow(clippy::too_many_lines)]
#[must_use]
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
    let mut form = Form::new(
        title,
        if matched.is_some() {
            "Update"
        } else {
            "Install"
        },
    )
    .field(Field::text(
        K_NAME,
        "Name",
        &detected.name,
        "The name in the launcher",
    ))
    .field(Field::text(K_ID, "App id", &id, "lowercase-with-hyphens"))
    .field(Field::text(
        K_VERSION,
        "Version",
        &detected.version,
        "as the archive names it",
    ))
    .field(Field::choice(
        K_KIND,
        "Kind",
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
    form = form.field(Field::choice(
        K_EXE,
        "Main executable",
        exe_options,
        exe_selected,
    ));
    let mut icon_options = vec!["(none)".to_owned()];
    icon_options.extend(detected.icons.iter().map(|i| {
        i.size
            .map_or_else(|| i.rel.clone(), |(w, h)| format!("{} ({w}×{h})", i.rel))
    }));
    form = form
        .field(Field::choice(
            K_ICON,
            "Icon",
            icon_options,
            usize::from(!detected.icons.is_empty()),
        ))
        .field(Field::text(
            K_ICON_FILE,
            "Icon file",
            "",
            "a PNG or SVG path, instead of the choice above",
        ));
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
        .field(Field::choice(
            K_CATEGORY,
            "Category (GUI)",
            CATEGORIES.iter().map(|c| (*c).to_owned()).collect(),
            category_index,
        ))
        .field(Field::text(
            K_WM_CLASS,
            "StartupWMClass (GUI)",
            detected.wm_class().unwrap_or_default(),
            "editable later in Installed Apps",
        ));
    let main = detected.main_rel().unwrap_or("");
    let offered = offered_commands(detected, &id, main);
    for (i, command) in offered.iter().enumerate() {
        let Some(key) = CMD_KEYS.get(i) else {
            break;
        };
        form = form.field(Field::check(
            key,
            format!("Link command {} → current/{}", command.name, command.target),
            true,
        ));
    }
    if offered.len() > CMD_KEYS.len() {
        form = form.field(Field::check(
            "cmd_more",
            format!(
                "{} more commands in bin/ are not linked; PATH mode puts them all on the PATH",
                offered.len() - CMD_KEYS.len()
            ),
            false,
        ));
    }
    form = form
        .field(Field::choice(
            K_PATH_MODE,
            "Commands (CLI)",
            vec![
                format!("Symlinks in {}", layout.bin_dir().display()),
                "Add the bin folder to the PATH of new shells".to_owned(),
            ],
            0,
        ))
        .field(Field::text(
            K_ENV,
            "Variable (PATH mode)",
            "",
            "JAVA_HOME, pointing at the version folder",
        ))
        .field(Field::choice(
            K_SCOPE,
            "Scope",
            vec![
                "User space (recommended)".to_owned(),
                "System-wide (unavailable: the privileged helper is not part of this version)"
                    .to_owned(),
            ],
            0,
        ))
        .field(Field::text(
            K_SHA,
            "Expected SHA-256",
            "",
            "optional; the hash the vendor publishes",
        ));
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
            "This is an update of…",
            update_options,
            selected,
        ));
    }
    form = form
        .field(Field::check(
            K_OVERWRITE,
            "Overwrite command links and desktop entries Rick did not create",
            false,
        ))
        .field(Field::check(K_DOWNGRADE, "Allow downgrade", false));
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

/// Runs the form and, on confirmation, the install job.
fn install_through_form(
    installer: &Arc<Installer>,
    archive: &Path,
    update_of: Option<&Installed>,
    jobs: &JobRunner,
    ui: &dyn UiHost,
) -> Result<ActionOutcome, Error> {
    let detected = installer.detect(archive)?;
    let scan = installer.provider.rescan();
    let layout = installer.provider.user_layout();
    let matched = update_of.cloned().or_else(|| matching(&scan, &detected));
    let form = install_form(
        &detected,
        &scan,
        &layout,
        matched.as_ref(),
        update_of.is_some(),
    );
    let Some(values) = ui.form(form) else {
        return Ok(ActionOutcome::Done);
    };
    if let Some(u) = update_of {
        installer.clear_pending_update(&u.record.id);
    }
    let assembled = assemble(&values, &detected, &scan, &layout, update_of);
    if let Some(problem) = assembled.problem() {
        return Err(err(InstallError::Invalid(problem.to_owned())));
    }
    let Some(plan) = assembled.plan else {
        return Err(err(InstallError::Invalid(
            "pick the main executable".to_owned(),
        )));
    };
    Ok(spawn_install(installer, plan, scan, jobs))
}

/// *Install as application…* on a selected archive.
pub struct Install(Arc<Installer>);

impl Action for Install {
    fn id(&self) -> ActionId {
        INSTALL
    }
    fn label(&self) -> &'static str {
        "Install as application…"
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
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        if !crate::supported() {
            return Err(err(InstallError::Unsupported));
        }
        let archive = selected_archive(&ctx).ok_or(Error::Unsupported(
            "select one tar or zip archive on the local disk",
        ))?;
        install_through_form(&self.0, &archive, None, jobs, ui)
    }
}

/// *Installed apps*: opens the view.
pub struct Apps;

impl Action for Apps {
    fn id(&self) -> ActionId {
        APPS
    }
    fn label(&self) -> &'static str {
        "Installed apps"
    }
    fn group(&self) -> ActionGroup {
        GROUP
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
    fn label(&self) -> &'static str {
        "Open install folder"
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
    fn label(&self) -> &'static str {
        "Launch"
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
    fn label(&self) -> &'static str {
        "Update from archive…"
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
        let id = installed.record.id.clone();
        let archive = if let Some(path) = self.0.pending_update(&id) {
            path
        } else {
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
            let path = installed.layout.expand(text);
            if !path.is_file() {
                return Err(Error::NotFound);
            }
            if Format::by_name(text).is_none() {
                return Err(Error::Unsupported(
                    "the archive must be a tar (gz, xz, bz2, zst) or zip file",
                ));
            }
            self.0.set_pending_update(&id, &path);
            path
        };
        install_through_form(&self.0, &archive, Some(&installed), jobs, ui)
    }
}

/// *Revert to previous version*.
pub struct Revert(Arc<Installer>);

impl Action for Revert {
    fn id(&self) -> ActionId {
        REVERT
    }
    fn label(&self) -> &'static str {
        "Revert to previous version"
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
                _ => Availability::Disabled("no previous version is kept"),
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
    fn label(&self) -> &'static str {
        "Edit launcher…"
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
        .field(Field::text(K_NAME, "Name", &installed.record.name, ""))
        .field(Field::text(
            K_ICON_FILE,
            "Icon file",
            "",
            "a PNG or SVG path, to replace the icon",
        ))
        .field(Field::choice(
            K_CATEGORY,
            "Category",
            CATEGORIES.iter().map(|c| (*c).to_owned()).collect(),
            category_index,
        ))
        .field(Field::text(
            K_WM_CLASS,
            "StartupWMClass",
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
    fn label(&self) -> &'static str {
        "Uninstall…"
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
        if !ui.confirm("Uninstall", &text) {
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
    fn label(&self) -> &'static str {
        "Repair"
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
                Availability::Disabled("nothing to repair")
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
    fn label(&self) -> &'static str {
        "Forget"
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
    fn label(&self) -> &'static str {
        "Remove staging folder"
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
