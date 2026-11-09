//! The install engine: the plan the form produced, the six steps of an
//! install or update with their rollback, and revert, uninstall, repair,
//! the launcher edit and the PATH-mode file. Everything here is
//! synchronous and runs on the blocking pool inside a job.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use ext::ProgressSink;
use ext::platform::{Platform, fs as pfs};
use types::{CancelToken, Error, Progress};

use crate::archive::{self, Format, Index};
use crate::desktop::{self, DesktopEntry};
use crate::detect::{self, IconFile};
use crate::elf;
use crate::error::{InstallError, bytes, err, io_write};
use crate::record::{
    ExternalFile, FileKind, Installed, Kind, Layout, PathMode, Problem, Record, STAGED_MARKER,
    Scan, Scope,
};

/// Where the icon comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconSource {
    /// A file inside the archive.
    Archive(IconFile),
    /// A file the user picked.
    File(PathBuf),
    /// No icon.
    None,
}

/// One command symlink: its name in the bin folder and its target relative
/// to the version folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLink {
    /// The name in `~/.local/bin`.
    pub name: String,
    /// The target relative to `current`.
    pub target: String,
}

/// Everything an install or update needs, settled by the form.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The scope's directories.
    pub layout: Layout,
    /// The archive.
    pub archive: PathBuf,
    /// Its format.
    pub format: Format,
    /// Its index.
    pub index: Arc<Index>,
    /// The app id.
    pub id: String,
    /// The display name.
    pub name: String,
    /// The version to install.
    pub version: String,
    /// GUI or CLI.
    pub kind: Kind,
    /// The main executable, relative to the version folder.
    pub main: String,
    /// The icon.
    pub icon: IconSource,
    /// The desktop entry's categories, `Utility` or several with `;`.
    pub categories: String,
    /// The window class for the desktop entry.
    pub wm_class: Option<String>,
    /// The vendor's desktop entry inside the archive, reused when present.
    pub vendor_desktop: Option<String>,
    /// The command symlinks to make, in symlink mode.
    pub commands: Vec<CommandLink>,
    /// Symlinks or PATH mode.
    pub path_mode: PathMode,
    /// The bin folder on the PATH in PATH mode, relative to `current`.
    pub bin_dir: Option<String>,
    /// The variable pointing at `current` in PATH mode.
    pub env_var: Option<String>,
    /// The expected SHA-256, lower-case hex, when the user filled it.
    pub expected_sha256: Option<String>,
    /// The archive file name with the version masked.
    pub source_pattern: String,
    /// The record this is an update of, when it is one.
    pub update_of: Option<Record>,
    /// Replace command links and desktop entries of another origin.
    pub overwrite: bool,
    /// The commands under `bin/`, which get the execute bit.
    pub bins: Vec<String>,
}

impl Plan {
    /// The archive's file name.
    #[must_use]
    pub fn archive_name(&self) -> String {
        self.archive
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Whether this is an update of an installed app.
    #[must_use]
    pub fn is_update(&self) -> bool {
        self.update_of.is_some()
    }

    /// The desktop entry's path, for a GUI app.
    #[must_use]
    pub fn desktop_path(&self) -> Option<PathBuf> {
        (self.kind == Kind::Gui).then(|| self.layout.desktop_path(&self.id))
    }

    /// The icon's path in the hicolor theme, for a GUI app with an icon.
    #[must_use]
    pub fn icon_path(&self) -> Option<PathBuf> {
        if self.kind != Kind::Gui {
            return None;
        }
        let (size, ext) = match &self.icon {
            IconSource::Archive(icon) => (icon.size_dir(), icon.ext.clone()),
            IconSource::File(path) => icon_file_size(path)?,
            IconSource::None => return None,
        };
        Some(self.layout.icon_path(&self.id, &size, &ext))
    }

    /// The command symlinks' paths, in symlink mode.
    #[must_use]
    pub fn symlink_paths(&self) -> Vec<PathBuf> {
        if self.path_mode != PathMode::Symlinks {
            return Vec::new();
        }
        self.commands
            .iter()
            .map(|c| self.layout.bin_dir().join(&c.name))
            .collect()
    }

    /// The files this plan writes outside the app folder that exist already
    /// and were not created by Rick.
    #[must_use]
    pub fn collisions(&self, scan: &Scan) -> Vec<PathBuf> {
        let own: HashSet<PathBuf> = self
            .update_of
            .iter()
            .flat_map(|r| r.external_files.iter())
            .map(|f| self.layout.expand(&f.path))
            .collect();
        self.desktop_path()
            .into_iter()
            .chain(self.symlink_paths())
            .filter(|path| {
                fs::symlink_metadata(path).is_ok() && !scan.owns(path) && !own.contains(path)
            })
            .collect()
    }

    /// The external files the install will have written, as the record
    /// lists them.
    fn external_files(&self) -> Vec<ExternalFile> {
        let mut out: Vec<ExternalFile> = Vec::new();
        if let Some(path) = self.desktop_path() {
            out.push(ExternalFile {
                path: self.layout.tilde(&path),
                kind: FileKind::DesktopEntry,
            });
        }
        if let Some(path) = self.icon_path() {
            out.push(ExternalFile {
                path: self.layout.tilde(&path),
                kind: FileKind::Icon,
            });
        }
        for path in self.symlink_paths() {
            out.push(ExternalFile {
                path: self.layout.tilde(&path),
                kind: FileKind::Symlink,
            });
        }
        out
    }
}

/// The hicolor size folder and extension of an icon file on disk.
fn icon_file_size(path: &Path) -> Option<(String, String)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)?;
    match ext.as_str() {
        "svg" => Some(("scalable".to_owned(), ext)),
        "png" => {
            let mut head = [0u8; 24];
            let mut file = fs::File::open(path).ok()?;
            let got = read_up_to(&mut file, &mut head).ok()?;
            let size = detect::png_size(&head[..got])
                .map_or_else(|| "48x48".to_owned(), |(w, h)| format!("{w}x{h}"));
            Some((size, ext))
        }
        _ => None,
    }
}

fn read_up_to(reader: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// What an install did beyond the plan, for the summary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    /// Entries the archive-safety rules left out, with the reason.
    pub skipped: Vec<(String, String)>,
    /// A top-level install script the archive carried, not run.
    pub install_script: Option<String>,
    /// Libraries the main executable and the commands need and the machine
    /// does not have.
    pub unresolved: Vec<String>,
    /// Anything else worth a line.
    pub notes: Vec<String>,
}

impl Report {
    /// The plain-language summary the job ends with.
    #[must_use]
    pub fn summary(&self, name: &str, version: &str, updated: bool) -> String {
        let mut out = if updated {
            format!("{name} updated to {version}.")
        } else {
            format!("{name} {version} installed.")
        };
        if let Some(script) = &self.install_script {
            let _ = write!(
                out,
                " The archive's {script} was not run; run it by hand if the app needs it."
            );
        }
        for lib in &self.unresolved {
            let _ = write!(out, " This app needs {lib}, which is not installed.");
        }
        if !self.skipped.is_empty() {
            let _ = write!(out, " {} entries were left out:", self.skipped.len());
            for (path, why) in self.skipped.iter().take(5) {
                let _ = write!(out, " {path} ({why});");
            }
            if self.skipped.len() > 5 {
                out.push_str(" …");
            }
        }
        for note in &self.notes {
            out.push(' ');
            out.push_str(note);
        }
        out
    }
}

/// Deletes the staging folder on drop unless disarmed.
struct StagingGuard {
    dir: PathBuf,
    armed: bool,
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

/// The free bytes on the volume holding `path` (or its nearest existing
/// ancestor), when the platform knows the volume.
fn free_space_at(platform: &dyn Platform, path: &Path) -> Option<u64> {
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let probe = fs::canonicalize(&probe).unwrap_or(probe);
    let volumes = platform.volumes().ok()?;
    volumes
        .iter()
        .filter(|v| probe.starts_with(&v.mount_point))
        .max_by_key(|v| v.mount_point.as_os_str().len())
        .and_then(|v| v.free)
}

/// Sets the execute bit on `path`, keeping its other bits.
fn make_executable(path: &Path) {
    if let Ok(Some(mode)) = pfs::unix_mode(path) {
        let _ = pfs::set_unix_mode(path, mode | 0o111);
    }
}

/// Points `link` at `target` atomically: a temporary link renamed over it.
fn flip_symlink(link: &Path, target: &Path) -> Result<(), Error> {
    let tmp = link.with_file_name(format!(
        "{}.tmp-{}",
        link.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id()
    ));
    let _ = fs::remove_file(&tmp);
    pfs::symlink(target, &tmp).map_err(io_write)?;
    fs::rename(&tmp, link).map_err(io_write)
}

/// The processes running an executable under `dir`, as `/proc/*/exe` says;
/// those under `except` do not count.
#[must_use]
pub fn running_under(dir: &Path, except: Option<&Path>) -> Vec<u32> {
    let dir = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let except = except.map(|e| fs::canonicalize(e).unwrap_or_else(|_| e.to_path_buf()));
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(exe) = fs::read_link(entry.path().join("exe")) else {
            continue;
        };
        if exe.starts_with(&dir) && !except.as_ref().is_some_and(|e| exe.starts_with(e)) {
            out.push(pid);
        }
    }
    out
}

/// Installs or updates by `plan`, reporting progress and honouring
/// `cancel`; `scan` is the state of the installed apps before it.
///
/// # Errors
///
/// The installer's errors by case (source tree, architecture, checksum,
/// disk full, collision, running app), `Error::Cancelled`, or the I/O
/// error. Before the version folder exists nothing is left behind; after
/// it, the record shows what remains to repair.
#[allow(clippy::too_many_lines)]
pub fn install(
    plan: &Plan,
    scan: &Scan,
    platform: &dyn Platform,
    progress: &ProgressSink,
    cancel: &CancelToken,
) -> Result<Report, Error> {
    if !crate::supported() {
        return Err(err(InstallError::Unsupported));
    }
    if plan.layout.scope == Scope::System {
        return Err(err(InstallError::SystemUnavailable));
    }
    if !detect::valid_id(&plan.id) {
        return Err(err(InstallError::Invalid(format!(
            "the app id {:?} must be lowercase letters, digits and hyphens",
            plan.id
        ))));
    }
    if !detect::valid_version(&plan.version) {
        return Err(err(InstallError::Invalid(format!(
            "the version {:?} cannot name a folder",
            plan.version
        ))));
    }
    if plan.name.trim().is_empty() {
        return Err(err(InstallError::Invalid("the name is empty".to_owned())));
    }
    if !plan.overwrite
        && let Some(path) = plan.collisions(scan).first()
    {
        return Err(err(InstallError::Collision {
            what: path.display().to_string(),
        }));
    }
    let mut report = Report {
        install_script: plan
            .index
            .items
            .iter()
            .filter_map(|i| plan.index.relative(&i.path))
            .find(|rel| {
                !rel.contains('/')
                    && matches!(
                        *rel,
                        "install.sh" | "install" | "INSTALL.sh" | "setup.sh" | "postinst"
                    )
            })
            .map(str::to_owned),
        ..Report::default()
    };
    let _entered = tracing::info_span!("install", id = %plan.id, version = %plan.version).entered();

    // Checksum, before anything is extracted.
    if let Some(expected) = plan
        .expected_sha256
        .as_deref()
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| !e.is_empty())
    {
        progress.report(Progress::Message(format!(
            "Checking {}",
            plan.archive_name()
        )));
        let total = fs::metadata(&plan.archive).map(|m| m.len()).ok();
        let actual = archive::sha256(&plan.archive, cancel, &mut |done| {
            progress.report(Progress::Bytes { done, total });
        })?;
        if actual != expected {
            return Err(err(InstallError::Checksum { expected, actual }));
        }
    }

    // Free space at the destination.
    let app_dir = plan.layout.app_dir(&plan.id);
    if let Some(free) = free_space_at(platform, &app_dir)
        && free < plan.index.total_size.saturating_add(16 << 20)
    {
        return Err(err(InstallError::DiskFull {
            needed: bytes(plan.index.total_size),
            free: bytes(free),
            at: plan.layout.apps_root().display().to_string(),
        }));
    }

    // Step 1: extract into the staging folder beside the destination.
    let staging = plan.layout.staging(&plan.id, &plan.version);
    let resumed = staging.join(STAGED_MARKER).is_file();
    let mut guard = StagingGuard {
        dir: staging.clone(),
        armed: true,
    };
    if resumed {
        report
            .notes
            .push("The files extracted last time were used.".to_owned());
    } else {
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(io_write)?;
        }
        fs::create_dir_all(&staging).map_err(io_write)?;
        progress.report(Progress::Message(format!(
            "Extracting {}",
            plan.archive_name()
        )));
        let total = Some(plan.index.total_size);
        let skipped = archive::extract(
            &plan.archive,
            plan.format,
            &plan.index,
            &staging,
            cancel,
            &mut |done| progress.report(Progress::Bytes { done, total }),
        )?;
        report.skipped = skipped.entries;
        for rel in std::iter::once(plan.main.as_str())
            .chain(plan.commands.iter().map(|c| c.target.as_str()))
            .chain(plan.bins.iter().map(String::as_str))
        {
            make_executable(&staging.join(rel));
        }
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }

    // Step 2: verify the main executable.
    let main_path = staging.join(&plan.main);
    let elf = elf::read_file(&main_path)
        .ok()
        .flatten()
        .ok_or_else(|| err(InstallError::NoMainExecutable(plan.main.clone())))?;
    if let Some(running) = elf::current_machine()
        && running != elf.machine
    {
        return Err(err(InstallError::Architecture {
            found: elf::machine_name(elf.machine),
            running: elf::machine_name(running),
        }));
    }
    fs::write(staging.join(STAGED_MARKER), b"").map_err(io_write)?;

    // A running app keeps the staging folder for the next attempt.
    if plan.is_update() && !running_under(&app_dir, Some(&staging)).is_empty() {
        guard.armed = false;
        return Err(err(InstallError::Running {
            name: plan.name.clone(),
        }));
    }

    // Step 3: the staging folder becomes the version folder.
    progress.report(Progress::Message(format!(
        "Installing {} {}",
        plan.name, plan.version
    )));
    let version_dir = plan.layout.version_dir(&plan.id, &plan.version);
    if version_dir.exists() {
        fs::remove_dir_all(&version_dir).map_err(io_write)?;
    }
    fs::rename(&staging, &version_dir).map_err(io_write)?;
    guard.armed = false;
    let _ = fs::remove_file(version_dir.join(STAGED_MARKER));

    // Step 4: the record.
    let previous = plan
        .update_of
        .as_ref()
        .map(|r| r.version.clone())
        .filter(|v| *v != plan.version);
    let mut external_files = plan.external_files();
    if let Some(old) = &plan.update_of {
        // An update keeps what it wrote before and adds what is new.
        for file in &old.external_files {
            if !external_files.iter().any(|f| f.path == file.path) {
                external_files.push(file.clone());
            }
        }
    }
    let record = Record {
        id: plan.id.clone(),
        name: plan.name.clone(),
        kind: plan.kind,
        scope: plan.layout.scope,
        version: plan.version.clone(),
        previous_version: previous,
        installed_at: Record::now(),
        source_archive: plan.archive_name(),
        source_pattern: plan.source_pattern.clone(),
        main_executable: plan.main.clone(),
        startup_wm_class: plan.wm_class.clone(),
        path_mode: (plan.kind == Kind::Cli).then_some(plan.path_mode),
        bin_dir: plan.bin_dir.clone(),
        env_var: plan.env_var.clone(),
        external_files,
    };
    record.write(&plan.layout.record_path(&plan.id))?;

    // Step 5: flip `current`.
    flip_symlink(&plan.layout.current(&plan.id), Path::new(&plan.version))?;

    // Step 6: the external files.
    let current = plan.layout.current(&plan.id);
    let main_changed = plan
        .update_of
        .as_ref()
        .is_some_and(|old| old.main_executable != plan.main);
    let mut wrote_desktop = false;
    if plan.kind == Kind::Gui {
        let icon_path = plan.icon_path();
        let desktop_path = plan.layout.desktop_path(&plan.id);
        if !plan.is_update() || !desktop_path.exists() {
            if let Some(icon_path) = &icon_path {
                write_icon(plan, &version_dir, icon_path)?;
            }
            write_desktop(
                plan,
                &version_dir,
                &current,
                &desktop_path,
                icon_path.is_some(),
            )?;
            wrote_desktop = true;
        } else if main_changed {
            let text = fs::read_to_string(&desktop_path).map_err(Error::from_io)?;
            let mut entry = DesktopEntry::parse(&text);
            let original = entry.get("Exec").map(str::to_owned);
            entry.set(
                "Exec",
                &desktop::exec_through(&current.join(&plan.main), original.as_deref()),
            );
            fs::write(&desktop_path, entry.text()).map_err(io_write)?;
            wrote_desktop = true;
        }
    }
    match plan.path_mode {
        PathMode::Symlinks => {
            for command in &plan.commands {
                let link = plan.layout.bin_dir().join(&command.name);
                let target = current.join(&command.target);
                let exists = fs::symlink_metadata(&link).is_ok();
                let same = fs::read_link(&link).is_ok_and(|t| t == target);
                if exists && same && !main_changed {
                    continue;
                }
                if let Some(parent) = link.parent() {
                    fs::create_dir_all(parent).map_err(io_write)?;
                }
                flip_symlink(&link, &target)?;
            }
        }
        PathMode::Path => {
            let mut records: Vec<Record> = scan
                .apps
                .iter()
                .filter(|a| a.layout.scope == plan.layout.scope && a.record.id != plan.id)
                .map(|a| a.record.clone())
                .collect();
            records.push(record.clone());
            write_path_file(&plan.layout, &records)?;
            report
                .notes
                .push("The PATH change applies to new shells.".to_owned());
        }
    }
    if wrote_desktop {
        desktop::update_database(&plan.layout.applications_dir());
    }

    // One previous version is kept; anything older goes.
    if let Ok(entries) = fs::read_dir(&app_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let keep = name == plan.version
                || Some(name.as_str()) == record.previous_version.as_deref()
                || name == "current"
                || name == crate::record::RECORD_FILE
                || name.starts_with('.');
            if !keep && entry.path().is_dir() {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }

    // The library check.
    let mut exes = vec![version_dir.join(&plan.main)];
    exes.extend(plan.commands.iter().map(|c| version_dir.join(&c.target)));
    report.unresolved = unresolved_libraries(&version_dir, &exes);
    progress.report(Progress::Message(report.summary(
        &plan.name,
        &plan.version,
        plan.is_update(),
    )));
    tracing::info!(summary = %report.summary(&plan.name, &plan.version, plan.is_update()), "done");
    Ok(report)
}

/// Copies the plan's icon into the hicolor theme at `icon_path`.
fn write_icon(plan: &Plan, version_dir: &Path, icon_path: &Path) -> Result<(), Error> {
    let source = match &plan.icon {
        IconSource::Archive(icon) => version_dir.join(&icon.rel),
        IconSource::File(path) => path.clone(),
        IconSource::None => return Ok(()),
    };
    if let Some(parent) = icon_path.parent() {
        fs::create_dir_all(parent).map_err(io_write)?;
    }
    fs::copy(&source, icon_path).map(drop).map_err(io_write)
}

/// Writes the desktop entry: the vendor's with `Exec` and `Icon` through
/// `current`, or a generated one.
fn write_desktop(
    plan: &Plan,
    version_dir: &Path,
    current: &Path,
    desktop_path: &Path,
    has_icon: bool,
) -> Result<(), Error> {
    let program = current.join(&plan.main);
    let vendor = plan
        .vendor_desktop
        .as_ref()
        .and_then(|rel| fs::read_to_string(version_dir.join(rel)).ok());
    let mut entry = match vendor {
        Some(text) => {
            let mut entry = DesktopEntry::parse(&text);
            let original = entry.get("Exec").map(str::to_owned);
            entry.set(
                "Exec",
                &desktop::exec_through(&program, original.as_deref()),
            );
            if entry.get("TryExec").is_some() {
                entry.set("TryExec", &program.to_string_lossy());
            }
            if has_icon {
                entry.set("Icon", &plan.id);
            }
            if entry.get("Type").is_none() {
                entry.set("Type", "Application");
            }
            entry
        }
        None => DesktopEntry::generate(
            &plan.name,
            &desktop::exec_through(&program, None),
            if has_icon { &plan.id } else { "" },
            &plan.categories,
            plan.wm_class.as_deref(),
        ),
    };
    if let Some(class) = plan.wm_class.as_deref().filter(|c| !c.is_empty()) {
        entry.set("StartupWMClass", class);
    }
    if let Some(parent) = desktop_path.parent() {
        fs::create_dir_all(parent).map_err(io_write)?;
    }
    fs::write(desktop_path, entry.text()).map_err(io_write)
}

/// The marker on the line Rick adds to a shell profile.
const PROFILE_MARK: &str = "# added by rick";

/// The installed Rick among `records`, by its main executable's name, and
/// the binary the shell function runs.
fn rick_binary(layout: &Layout, records: &[Record]) -> Option<PathBuf> {
    records
        .iter()
        .find(|r| {
            Path::new(&r.main_executable)
                .file_name()
                .is_some_and(|n| n == "rick")
        })
        .map(|r| layout.current(&r.id).join(&r.main_executable))
}

/// Regenerates `~/.config/rick/path.sh` (and the fish drop-in when fish is
/// configured) from the PATH-mode records of `records`, with the `rick`
/// shell function when Rick itself is installed (so `rick` with no
/// argument opens the shell's current directory from a login shell that
/// lacks `~/.local/bin` on its PATH; specs/system-integration.md §4), and
/// makes sure the profiles source it; removes the files when no record
/// needs them. The block the `rick` command of the integration items
/// keeps in either file (specs/system-integration.md §7.1) is carried
/// over as it is, in place of the records' function, and keeps a file
/// alive that the records no longer need.
///
/// # Errors
///
/// The I/O error from writing.
pub fn write_path_file(layout: &Layout, records: &[Record]) -> Result<(), Error> {
    let path_mode: Vec<&Record> = records
        .iter()
        .filter(|r| r.path_mode == Some(PathMode::Path))
        .collect();
    let rick = rick_binary(layout, records);
    let file = layout.path_file();
    let fish_dir = layout.home.join(".config/fish");
    let fish_file = fish_dir.join("conf.d/rick.fish");
    // The `rick` command of the integration items keeps the function in a
    // marked block of its own in both files (it works without an install
    // record, for a Rick that runs from where it was unpacked): carried
    // over as found, in place of the function the records would give.
    let kept = |path: &Path| {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| ext::platform::integration::command_block(&text))
    };
    let (kept_sh, kept_fish) = (kept(&file), kept(&fish_file));
    let header = "# Generated by Rick from its install records; edit nothing here.\n";
    if path_mode.is_empty() && rick.is_none() {
        for (path, block) in [(&file, &kept_sh), (&fish_file, &kept_fish)] {
            match block {
                Some(block) => fs::write(path, format!("{header}{block}")).map_err(io_write)?,
                None => {
                    let _ = fs::remove_file(path);
                }
            }
        }
        return Ok(());
    }
    let mut sh = String::from(header);
    let mut fish = sh.clone();
    for record in path_mode {
        let current = layout.current(&record.id);
        let bin = current.join(record.bin_dir.as_deref().unwrap_or("bin"));
        let bin = bin.to_string_lossy();
        let _ = writeln!(sh, "export PATH=\"{bin}:$PATH\"");
        let _ = writeln!(fish, "set -gx PATH \"{bin}\" $PATH");
        if let Some(var) = record.env_var.as_deref().filter(|v| !v.is_empty()) {
            let dir = current.to_string_lossy();
            let _ = writeln!(sh, "export {var}=\"{dir}\"");
            let _ = writeln!(fish, "set -gx {var} \"{dir}\"");
        }
    }
    let binary = rick.map(|binary| binary.to_string_lossy().into_owned());
    match (&kept_sh, &binary) {
        (Some(block), _) => sh.push_str(block),
        (None, Some(binary)) => {
            let _ = writeln!(
                sh,
                "# `rick` for a login shell without ~/.local/bin on its PATH: no argument opens the shell's directory.\nrick() {{ \"{binary}\" \"$@\"; }}"
            );
        }
        (None, None) => {}
    }
    match (&kept_fish, &binary) {
        (Some(block), _) => fish.push_str(block),
        (None, Some(binary)) => {
            let _ = writeln!(
                fish,
                "# `rick` for a login shell without ~/.local/bin on its PATH: no argument opens the shell's directory.\nfunction rick\n    \"{binary}\" $argv\nend"
            );
        }
        (None, None) => {}
    }
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(io_write)?;
    }
    fs::write(&file, sh).map_err(io_write)?;
    let line = format!(
        "[ -f \"{0}\" ] && . \"{0}\" {PROFILE_MARK}",
        file.to_string_lossy()
    );
    ensure_profile_line(&layout.home.join(".profile"), &line, true)?;
    let zprofile = layout.home.join(".zprofile");
    if zprofile.is_file() {
        ensure_profile_line(&zprofile, &line, false)?;
    }
    if fish_dir.is_dir() {
        fs::create_dir_all(fish_dir.join("conf.d")).map_err(io_write)?;
        fs::write(&fish_file, fish).map_err(io_write)?;
    }
    Ok(())
}

/// Adds `line` to the profile at `path` once; `create` makes the file when
/// it does not exist.
fn ensure_profile_line(path: &Path, line: &str, create: bool) -> Result<(), Error> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => String::new(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Error::from_io(e)),
    };
    if text.lines().any(|l| l.contains(PROFILE_MARK)) {
        return Ok(());
    }
    let mut out = text;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    fs::write(path, out).map_err(io_write)
}

/// Removes Rick's line from the shell profiles: what the last PATH-mode
/// uninstall does once the user agreed.
///
/// # Errors
///
/// The I/O error from rewriting a profile.
pub fn remove_profile_lines(layout: &Layout) -> Result<(), Error> {
    for name in [".profile", ".zprofile"] {
        let path = layout.home.join(name);
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if !text.contains(PROFILE_MARK) {
            continue;
        }
        let kept: Vec<&str> = text.lines().filter(|l| !l.contains(PROFILE_MARK)).collect();
        let mut out = kept.join("\n");
        if out.trim().is_empty() {
            // Rick's line was all the file held, so Rick made the file.
            fs::remove_file(&path).map_err(io_write)?;
            continue;
        }
        out.push('\n');
        fs::write(&path, out).map_err(io_write)?;
    }
    Ok(())
}

/// Whether the shell profiles carry Rick's line.
#[must_use]
pub fn has_profile_line(layout: &Layout) -> bool {
    [".profile", ".zprofile"].iter().any(|name| {
        fs::read_to_string(layout.home.join(name)).is_ok_and(|t| t.contains(PROFILE_MARK))
    })
}

/// Whether `dir` is on the PATH of this process.
#[must_use]
pub fn on_path(dir: &Path) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|p| p == dir))
}

/// The library names from `/etc/ld.so.cache`, when it is readable: the
/// `glibc-ld.so.cache1.1` format every glibc of the last decade writes.
fn loader_cache() -> HashSet<String> {
    const MAGIC: &[u8] = b"glibc-ld.so.cache1.1";
    let mut out = HashSet::new();
    let Ok(data) = fs::read("/etc/ld.so.cache") else {
        return out;
    };
    let Some(start) = data
        .windows(MAGIC.len())
        .position(|w| w == MAGIC)
        .filter(|s| s % 8 == 0 || *s == 0)
    else {
        return out;
    };
    let header = &data[start..];
    if header.len() < 48 {
        return out;
    }
    let nlibs = u32::from_le_bytes([header[20], header[21], header[22], header[23]]) as usize;
    let entries = &header[48..];
    for i in 0..nlibs.min(1 << 20) {
        let Some(entry) = entries.get(i * 24..i * 24 + 24) else {
            break;
        };
        let key = u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]) as usize;
        let Some(rest) = header.get(key..) else {
            continue;
        };
        let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
        let name = String::from_utf8_lossy(&rest[..end]);
        out.insert(name.rsplit('/').next().unwrap_or(&name).to_owned());
    }
    out
}

/// The shared libraries `exes` need that neither the app's own folders nor
/// the system provide, by file name, each once.
#[must_use]
pub fn unresolved_libraries(version_dir: &Path, exes: &[PathBuf]) -> Vec<String> {
    let cache = loader_cache();
    let machine = elf::current_machine();
    let mut system_dirs: Vec<PathBuf> =
        ["/lib", "/usr/lib", "/lib64", "/usr/lib64", "/usr/local/lib"]
            .iter()
            .map(PathBuf::from)
            .collect();
    if let Some(triplet) = machine.and_then(elf::multiarch_dir) {
        system_dirs.push(PathBuf::from("/lib").join(triplet));
        system_dirs.push(PathBuf::from("/usr/lib").join(triplet));
    }
    let mut unresolved: Vec<String> = Vec::new();
    for exe in exes {
        let Ok(Some(elf)) = elf::read_file(exe) else {
            continue;
        };
        if elf.needed.is_empty() {
            continue;
        }
        let origin = exe.parent().unwrap_or(version_dir).to_path_buf();
        let mut dirs: Vec<PathBuf> = elf
            .search_paths()
            .iter()
            .map(|p| {
                let p = p
                    .replace("$ORIGIN", &origin.to_string_lossy())
                    .replace("${ORIGIN}", &origin.to_string_lossy());
                let p = p.replace("$LIB", "lib").replace("${LIB}", "lib");
                let path = PathBuf::from(p);
                if path.is_absolute() {
                    path
                } else {
                    origin.join(path)
                }
            })
            .collect();
        dirs.push(origin.clone());
        dirs.push(version_dir.join("lib"));
        dirs.push(version_dir.join("lib64"));
        if let Some(triplet) = machine.and_then(elf::multiarch_dir) {
            dirs.push(version_dir.join("lib").join(triplet));
        }
        dirs.extend(system_dirs.iter().cloned());
        for lib in &elf.needed {
            let found = cache.contains(lib) || dirs.iter().any(|d| d.join(lib).is_file());
            if !found && !unresolved.contains(lib) {
                unresolved.push(lib.clone());
            }
        }
    }
    unresolved
}

/// Flips `current` back to the previous version and swaps the record's two
/// version fields.
///
/// # Errors
///
/// The installer's `NoPrevious` error when there is none or its folder is
/// gone; otherwise the I/O error.
pub fn revert(installed: &Installed) -> Result<Record, Error> {
    let record = &installed.record;
    let Some(previous) = record.previous_version.clone() else {
        return Err(err(InstallError::NoPrevious(record.name.clone())));
    };
    if !installed.dir.join(&previous).is_dir() {
        return Err(err(InstallError::NoPrevious(record.name.clone())));
    }
    flip_symlink(&installed.current(), Path::new(&previous))?;
    let mut updated = record.clone();
    updated.previous_version = Some(record.version.clone());
    updated.version = previous;
    updated.installed_at = Record::now();
    updated.write(&installed.record_path())?;
    Ok(updated)
}

/// What an uninstall will remove, for the confirmation and the job.
#[must_use]
pub fn uninstall_list(installed: &Installed) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = installed
        .record
        .external_files
        .iter()
        .map(|f| installed.layout.expand(&f.path))
        .collect();
    out.push(installed.dir.clone());
    out
}

/// Deletes every external file the record lists, then the app folder;
/// nothing else. `others` are the other installed apps, whose PATH-mode
/// entries the regenerated file keeps; `drop_profile_line` removes Rick's
/// line from the shell profiles when no PATH-mode app remains.
///
/// # Errors
///
/// The I/O error from a deletion; what was deleted before it stays
/// deleted.
pub fn uninstall(
    installed: &Installed,
    others: &[Record],
    drop_profile_line: bool,
) -> Result<(), Error> {
    let _entered = tracing::info_span!("uninstall", id = %installed.record.id).entered();
    let mut touched_desktop = false;
    for file in &installed.record.external_files {
        let path = installed.layout.expand(&file.path);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => fs::remove_dir_all(&path).map_err(io_write)?,
            Ok(_) => fs::remove_file(&path).map_err(io_write)?,
            Err(_) => {}
        }
        touched_desktop |= file.kind == FileKind::DesktopEntry;
    }
    if installed.dir.exists() {
        fs::remove_dir_all(&installed.dir).map_err(io_write)?;
    }
    if touched_desktop {
        desktop::update_database(&installed.layout.applications_dir());
    }
    if installed.record.path_mode == Some(PathMode::Path) {
        let remaining: Vec<Record> = others
            .iter()
            .filter(|r| r.id != installed.record.id)
            .cloned()
            .collect();
        write_path_file(&installed.layout, &remaining)?;
        if drop_profile_line
            && !remaining
                .iter()
                .any(|r| r.path_mode == Some(PathMode::Path))
        {
            remove_profile_lines(&installed.layout)?;
        }
    }
    Ok(())
}

/// Rewrites what the record says should exist and does not: the `current`
/// link, the desktop entry, the icon, the command symlinks. Returns what
/// was done.
///
/// # Errors
///
/// The I/O error from a write.
pub fn repair(installed: &Installed) -> Result<Vec<String>, Error> {
    let record = &installed.record;
    let layout = &installed.layout;
    let mut done = Vec::new();
    let mut version_dir = installed.version_dir();
    let mut record = record.clone();
    if !version_dir.is_dir() {
        // The active version is gone: the previous one, if kept, becomes it.
        if let Some(previous) = record.previous_version.clone()
            && installed.dir.join(&previous).is_dir()
        {
            record.version = previous;
            record.previous_version = None;
            version_dir = installed.dir.join(&record.version);
            record.write(&installed.record_path())?;
            done.push(format!("switched to the kept version {}", record.version));
        } else {
            return Err(err(InstallError::Invalid(format!(
                "no version folder of {} remains; Forget removes the record",
                record.name
            ))));
        }
    }
    let current = installed.current();
    if fs::read_link(&current).ok().as_deref() != Some(Path::new(&record.version))
        || !current.exists()
    {
        flip_symlink(&current, Path::new(&record.version))?;
        done.push("current symlink".to_owned());
    }
    let mut wrote_desktop = false;
    for file in &record.external_files {
        let path = layout.expand(&file.path);
        match file.kind {
            FileKind::DesktopEntry if !path.is_file() => {
                let has_icon = record
                    .external_files
                    .iter()
                    .any(|f| f.kind == FileKind::Icon && layout.expand(&f.path).is_file());
                let entry = DesktopEntry::generate(
                    &record.name,
                    &desktop::exec_through(&current.join(&record.main_executable), None),
                    if has_icon { &record.id } else { "" },
                    "Utility",
                    record.startup_wm_class.as_deref(),
                );
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(io_write)?;
                }
                fs::write(&path, entry.text()).map_err(io_write)?;
                wrote_desktop = true;
                done.push("desktop entry".to_owned());
            }
            FileKind::Icon if !path.is_file() => {
                // The icon's source inside the version folder is found again
                // by the detection rules: a file named like the app.
                let wanted = path
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Some(source) = find_icon_in(&version_dir, &record.id, &wanted) {
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent).map_err(io_write)?;
                    }
                    fs::copy(&source, &path).map_err(io_write)?;
                    done.push("icon".to_owned());
                }
            }
            FileKind::Symlink if fs::read_link(&path).is_err() || !path.exists() => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let target = if version_dir.join("bin").join(&name).is_file() {
                    PathBuf::from("bin").join(&name)
                } else {
                    PathBuf::from(&record.main_executable)
                };
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(io_write)?;
                }
                flip_symlink(&path, &current.join(target))?;
                done.push(format!("symlink {name}"));
            }
            _ => {}
        }
    }
    if wrote_desktop {
        desktop::update_database(&layout.applications_dir());
    }
    Ok(done)
}

/// A PNG or SVG under `dir` named like the app, for a repair.
fn find_icon_in(dir: &Path, id: &str, ext: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    let mut best: Option<(u64, PathBuf)> = None;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let Some(stem) = name.strip_suffix(&format!(".{ext}")) else {
                continue;
            };
            let size = entry.metadata().map_or(0, |m| m.len());
            if detect::slug(stem) == id && best.as_ref().is_none_or(|(s, _)| size > *s) {
                best = Some((size, path));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Rewrites the launcher's name, icon, category and window class, and the
/// record's name and class.
///
/// # Errors
///
/// The I/O error from a write.
pub fn edit_launcher(
    installed: &Installed,
    name: &str,
    icon: Option<&Path>,
    category: &str,
    wm_class: &str,
) -> Result<(), Error> {
    let layout = &installed.layout;
    let mut record = installed.record.clone();
    let desktop_path = installed
        .desktop_entry()
        .unwrap_or_else(|| layout.desktop_path(&record.id));
    let mut entry = fs::read_to_string(&desktop_path).ok().map_or_else(
        || {
            DesktopEntry::generate(
                name,
                &desktop::exec_through(&installed.current().join(&record.main_executable), None),
                "",
                category,
                None,
            )
        },
        |text| DesktopEntry::parse(&text),
    );
    if !name.trim().is_empty() {
        entry.set("Name", name.trim());
        name.trim().clone_into(&mut record.name);
    }
    if !category.trim().is_empty() {
        let mut categories = category.trim().to_owned();
        if !categories.ends_with(';') {
            categories.push(';');
        }
        entry.set("Categories", &categories);
    }
    if wm_class.trim().is_empty() {
        entry.remove("StartupWMClass");
        record.startup_wm_class = None;
    } else {
        entry.set("StartupWMClass", wm_class.trim());
        record.startup_wm_class = Some(wm_class.trim().to_owned());
    }
    if let Some(icon) = icon
        && let Some((size, ext)) = icon_file_size(icon)
    {
        let target = layout.icon_path(&record.id, &size, &ext);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(io_write)?;
        }
        fs::copy(icon, &target).map_err(io_write)?;
        let tilde = layout.tilde(&target);
        if !record.external_files.iter().any(|f| f.path == tilde) {
            record.external_files.push(ExternalFile {
                path: tilde,
                kind: FileKind::Icon,
            });
        }
        entry.set("Icon", &record.id);
    }
    if !record
        .external_files
        .iter()
        .any(|f| f.kind == FileKind::DesktopEntry)
    {
        record.external_files.push(ExternalFile {
            path: layout.tilde(&desktop_path),
            kind: FileKind::DesktopEntry,
        });
    }
    if let Some(parent) = desktop_path.parent() {
        fs::create_dir_all(parent).map_err(io_write)?;
    }
    fs::write(&desktop_path, entry.text()).map_err(io_write)?;
    record.write(&installed.record_path())?;
    desktop::update_database(&layout.applications_dir());
    Ok(())
}

/// Launches a GUI app through its desktop entry with `gio launch`, so the
/// environment, startup notification and window matching are the app
/// menu's.
///
/// # Errors
///
/// `Error::Unsupported` without `gio` or a desktop entry; otherwise the
/// error from starting it.
pub fn launch(installed: &Installed) -> Result<(), Error> {
    let Some(desktop) = installed.desktop_entry().filter(|p| p.is_file()) else {
        return Err(Error::Unsupported(
            "the app has no desktop entry to launch through; Repair writes one",
        ));
    };
    let status = Command::new("gio")
        .arg("launch")
        .arg(&desktop)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(Error::Io(std::io::Error::other(format!(
            "gio launch ended with {status}"
        )))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::Unsupported(
            "gio is not installed; it launches apps through their desktop entries",
        )),
        Err(e) => Err(Error::from_io(e)),
    }
}

/// Deletes the record only, leaving the files: *Forget*.
///
/// # Errors
///
/// The I/O error from the deletion.
pub fn forget(installed: &Installed) -> Result<(), Error> {
    fs::remove_file(installed.record_path()).map_err(Error::from_io)
}

/// A problem's fix in words, for the Repair confirmation.
#[must_use]
pub fn repair_text(problems: &[Problem]) -> String {
    problems
        .iter()
        .map(|p| match p {
            Problem::VersionMissing => "switch to the kept version".to_owned(),
            Problem::CurrentDangling => "rewrite the current symlink".to_owned(),
            Problem::MainMissing => "the main executable is gone; a reinstall is needed".to_owned(),
            Problem::DesktopMissing => "rewrite the desktop entry".to_owned(),
            Problem::IconMissing => "copy the icon again".to_owned(),
            Problem::SymlinkBroken(name) => format!("rewrite the symlink {name}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
