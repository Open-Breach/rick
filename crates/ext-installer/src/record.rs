//! The install record, `rick-install.toml` in the app's folder, the paths
//! of the two scopes, and the scan of `~/.local/opt/*/` and `/opt/*/` that
//! the Installed Apps view and the update matching read.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ext::platform::ConfigDir;
use serde::{Deserialize, Serialize};
use types::Error;

use crate::error::{InstallError, err, io_write};

/// The record's file name inside the app folder.
pub const RECORD_FILE: &str = "rick-install.toml";

/// The prefix of a staging folder beside the version folders.
pub const STAGING_PREFIX: &str = ".staging-";

/// The marker a finished extraction leaves in its staging folder, so an
/// update stopped by a running app resumes without extracting again.
pub const STAGED_MARKER: &str = ".rick-staged";

/// GUI application or command-line tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A desktop application with a launcher entry.
    Gui,
    /// A command-line tool or toolchain.
    Cli,
}

impl Kind {
    /// `GUI application` or `command-line tool`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Gui => "GUI application",
            Self::Cli => "command-line tool",
        }
    }
}

/// Where an app is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Under the user's home.
    User,
    /// Under `/opt`, through the privileged helper.
    System,
}

impl Scope {
    /// `user space` or `system-wide`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "user space",
            Self::System => "system-wide",
        }
    }
}

/// How a toolchain's commands reach the PATH.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathMode {
    /// One symlink per command in the bin folder.
    Symlinks,
    /// The toolchain's bin folder on the PATH through Rick's managed file.
    Path,
}

/// What kind of file Rick created outside the app folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileKind {
    /// The launcher entry.
    DesktopEntry,
    /// The icon in the hicolor theme.
    Icon,
    /// A command symlink.
    Symlink,
}

/// One file Rick created outside the app folder; uninstall removes exactly
/// these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalFile {
    /// The path, with `~` for the home directory.
    pub path: String,
    /// What it is.
    pub kind: FileKind,
}

/// The record: the single source of truth about one installed app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The app id, a lowercase hyphenated slug.
    pub id: String,
    /// The display name.
    pub name: String,
    /// GUI or CLI.
    pub kind: Kind,
    /// User or system.
    pub scope: Scope,
    /// The active version.
    pub version: String,
    /// The one previous version kept, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<String>,
    /// When the active version was installed.
    pub installed_at: toml::value::Datetime,
    /// The file name of the archive the active version came from.
    pub source_archive: String,
    /// The archive file name with the version digits masked, for update
    /// matching.
    pub source_pattern: String,
    /// The main executable, relative to the version folder.
    pub main_executable: String,
    /// The window class the launcher entry carries, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_wm_class: Option<String>,
    /// Symlinks or PATH mode, for a toolchain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_mode: Option<PathMode>,
    /// The bin folder on the PATH in PATH mode, relative to `current`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin_dir: Option<String>,
    /// An environment variable pointing at `current` in PATH mode
    /// (`JAVA_HOME`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_var: Option<String>,
    /// Every file created outside the app folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_files: Vec<ExternalFile>,
}

impl Record {
    /// Parses a record's text.
    ///
    /// # Errors
    ///
    /// The installer's `Corrupt` error with toml's message.
    pub fn parse(text: &str) -> Result<Self, Error> {
        toml::from_str(text).map_err(|e| err(InstallError::Corrupt(format!("{RECORD_FILE}: {e}"))))
    }

    /// The record as TOML.
    #[must_use]
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// Reads the record at `path`.
    ///
    /// # Errors
    ///
    /// `Error::NotFound`, the I/O error, or `Corrupt` for text that does
    /// not parse.
    pub fn read(path: &Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path).map_err(Error::from_io)?;
        Self::parse(&text)
    }

    /// Writes the record at `path` through a temporary file and a rename.
    ///
    /// # Errors
    ///
    /// The I/O error from writing.
    pub fn write(&self, path: &Path) -> Result<(), Error> {
        let tmp = path.with_extension(format!("toml.tmp-{}", std::process::id()));
        std::fs::write(&tmp, self.to_toml()).map_err(io_write)?;
        std::fs::rename(&tmp, path).map_err(io_write)
    }

    /// The current time as a TOML datetime in UTC, to the second.
    #[must_use]
    pub fn now() -> toml::value::Datetime {
        datetime_of(SystemTime::now())
    }

    /// `installed_at` as a system time.
    #[must_use]
    pub fn installed_at_time(&self) -> Option<SystemTime> {
        let text = self.installed_at.to_string();
        chrono::DateTime::parse_from_rfc3339(&text)
            .map(SystemTime::from)
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M:%S")
                    .map(|naive| SystemTime::from(naive.and_utc()))
            })
            .ok()
    }

    /// The names of the command symlinks the record lists.
    #[must_use]
    pub fn commands(&self) -> Vec<String> {
        self.external_files
            .iter()
            .filter(|f| f.kind == FileKind::Symlink)
            .filter_map(|f| f.path.rsplit('/').next().map(str::to_owned))
            .collect()
    }
}

/// A system time as a TOML datetime in UTC, to the second.
#[must_use]
pub fn datetime_of(time: SystemTime) -> toml::value::Datetime {
    let text = chrono::DateTime::<chrono::Utc>::from(time)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    text.parse().unwrap_or_else(|_| {
        // RFC 3339 is TOML's own datetime grammar; the epoch is the
        // fallback a parse failure would need, and it does parse.
        "1970-01-01T00:00:00Z"
            .parse()
            .unwrap_or_else(|_| unreachable!("the epoch is a TOML datetime"))
    })
}

/// The directories of one scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// User or system.
    pub scope: Scope,
    /// The user's home, for the user scope's paths and the `~` form.
    pub home: PathBuf,
    /// Rick's config directory, for the PATH-mode file.
    pub config: PathBuf,
}

impl Layout {
    /// The user-space layout under the platform's home directory.
    #[must_use]
    pub fn user(dirs: &dyn ConfigDir) -> Self {
        Self {
            scope: Scope::User,
            home: dirs.home_dir(),
            config: dirs.config_dir(),
        }
    }

    /// The system-wide layout.
    #[must_use]
    pub fn system(dirs: &dyn ConfigDir) -> Self {
        Self {
            scope: Scope::System,
            home: dirs.home_dir(),
            config: dirs.config_dir(),
        }
    }

    /// `~/.local/opt` or `/opt`.
    #[must_use]
    pub fn apps_root(&self) -> PathBuf {
        match self.scope {
            Scope::User => self.home.join(".local/opt"),
            Scope::System => PathBuf::from("/opt"),
        }
    }

    /// The app's folder.
    #[must_use]
    pub fn app_dir(&self, id: &str) -> PathBuf {
        self.apps_root().join(id)
    }

    /// A version folder.
    #[must_use]
    pub fn version_dir(&self, id: &str, version: &str) -> PathBuf {
        self.app_dir(id).join(version)
    }

    /// The `current` symlink.
    #[must_use]
    pub fn current(&self, id: &str) -> PathBuf {
        self.app_dir(id).join("current")
    }

    /// The staging folder of a version.
    #[must_use]
    pub fn staging(&self, id: &str, version: &str) -> PathBuf {
        self.app_dir(id).join(format!("{STAGING_PREFIX}{version}"))
    }

    /// The record's path.
    #[must_use]
    pub fn record_path(&self, id: &str) -> PathBuf {
        self.app_dir(id).join(RECORD_FILE)
    }

    /// `~/.local/bin` or `/usr/local/bin`.
    #[must_use]
    pub fn bin_dir(&self) -> PathBuf {
        match self.scope {
            Scope::User => self.home.join(".local/bin"),
            Scope::System => PathBuf::from("/usr/local/bin"),
        }
    }

    /// `~/.local/share/applications` or `/usr/share/applications`.
    #[must_use]
    pub fn applications_dir(&self) -> PathBuf {
        match self.scope {
            Scope::User => self.home.join(".local/share/applications"),
            Scope::System => PathBuf::from("/usr/share/applications"),
        }
    }

    /// The desktop entry of an app.
    #[must_use]
    pub fn desktop_path(&self, id: &str) -> PathBuf {
        self.applications_dir().join(format!("{id}.desktop"))
    }

    /// The hicolor theme's root.
    #[must_use]
    pub fn icons_root(&self) -> PathBuf {
        match self.scope {
            Scope::User => self.home.join(".local/share/icons/hicolor"),
            Scope::System => PathBuf::from("/usr/share/icons/hicolor"),
        }
    }

    /// The icon of an app at `size` (`512x512`, `scalable`) with `ext`.
    #[must_use]
    pub fn icon_path(&self, id: &str, size: &str, ext: &str) -> PathBuf {
        self.icons_root()
            .join(size)
            .join("apps")
            .join(format!("{id}.{ext}"))
    }

    /// Rick's managed PATH file, `~/.config/rick/path.sh`.
    #[must_use]
    pub fn path_file(&self) -> PathBuf {
        self.config.join("path.sh")
    }

    /// A path with `~` for the home directory, as the record writes it.
    #[must_use]
    pub fn tilde(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) => format!("~/{}", rest.to_string_lossy()),
            Err(_) => path.to_string_lossy().into_owned(),
        }
    }

    /// The OS path of a recorded `~`-form path.
    #[must_use]
    pub fn expand(&self, text: &str) -> PathBuf {
        text.strip_prefix("~/")
            .map_or_else(|| PathBuf::from(text), |rest| self.home.join(rest))
    }
}

/// A messy state the Installed Apps view shows instead of erroring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The active version's folder is gone.
    VersionMissing,
    /// The `current` link is missing or points at nothing.
    CurrentDangling,
    /// The main executable is not in the version folder.
    MainMissing,
    /// The launcher entry the record lists is gone.
    DesktopMissing,
    /// The icon the record lists is gone.
    IconMissing,
    /// A command symlink is missing or points at nothing.
    SymlinkBroken(String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VersionMissing => f.write_str("version folder missing"),
            Self::CurrentDangling => f.write_str("current symlink dangling"),
            Self::MainMissing => f.write_str("main executable missing"),
            Self::DesktopMissing => f.write_str("desktop entry missing"),
            Self::IconMissing => f.write_str("icon missing"),
            Self::SymlinkBroken(name) => write!(f, "symlink {name} broken"),
        }
    }
}

/// One installed app as the scan found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The record.
    pub record: Record,
    /// The scope's layout.
    pub layout: Layout,
    /// The app folder.
    pub dir: PathBuf,
    /// What is wrong, if anything; empty for a healthy install.
    pub problems: Vec<Problem>,
}

impl Installed {
    /// The record's path.
    #[must_use]
    pub fn record_path(&self) -> PathBuf {
        self.dir.join(RECORD_FILE)
    }

    /// The active version's folder.
    #[must_use]
    pub fn version_dir(&self) -> PathBuf {
        self.dir.join(&self.record.version)
    }

    /// The `current` link.
    #[must_use]
    pub fn current(&self) -> PathBuf {
        self.dir.join("current")
    }

    /// The desktop entry the record lists, if any.
    #[must_use]
    pub fn desktop_entry(&self) -> Option<PathBuf> {
        self.record
            .external_files
            .iter()
            .find(|f| f.kind == FileKind::DesktopEntry)
            .map(|f| self.layout.expand(&f.path))
    }

    /// The state text the view shows: the problems, or `ok`.
    #[must_use]
    pub fn state(&self) -> String {
        if self.problems.is_empty() {
            "ok".to_owned()
        } else {
            self.problems
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        }
    }
}

/// A staging folder nobody is extracting into any more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    /// The staging folder.
    pub dir: PathBuf,
    /// The app id of the folder it sits in.
    pub app_id: String,
    /// The version the folder was staged for.
    pub version: String,
}

/// What a scan of the two roots found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scan {
    /// The installed apps, by record, sorted by name.
    pub apps: Vec<Installed>,
    /// Staging folders left behind.
    pub orphans: Vec<Orphan>,
}

impl Scan {
    /// The app with this id, in either scope (user first).
    #[must_use]
    pub fn by_id(&self, id: &str) -> Option<&Installed> {
        self.apps.iter().find(|a| a.record.id == id)
    }

    /// Whether any record lists `path` among its external files.
    #[must_use]
    pub fn owns(&self, path: &Path) -> bool {
        self.apps.iter().any(|a| {
            a.record
                .external_files
                .iter()
                .any(|f| a.layout.expand(&f.path) == path)
        })
    }
}

/// The problems of a record against the disk.
#[must_use]
pub fn problems(record: &Record, layout: &Layout) -> Vec<Problem> {
    let mut out = Vec::new();
    let version_dir = layout.version_dir(&record.id, &record.version);
    if !version_dir.is_dir() {
        out.push(Problem::VersionMissing);
    } else if !version_dir.join(&record.main_executable).is_file() {
        out.push(Problem::MainMissing);
    }
    let current = layout.current(&record.id);
    if std::fs::read_link(&current).is_err() || !current.exists() {
        out.push(Problem::CurrentDangling);
    }
    for file in &record.external_files {
        let path = layout.expand(&file.path);
        match file.kind {
            FileKind::DesktopEntry if !path.is_file() => out.push(Problem::DesktopMissing),
            FileKind::Icon if !path.is_file() => out.push(Problem::IconMissing),
            FileKind::Symlink if std::fs::read_link(&path).is_err() || !path.exists() => {
                out.push(Problem::SymlinkBroken(
                    path.file_name()
                        .map_or_default(|n| n.to_string_lossy().into_owned()),
                ));
            }
            _ => {}
        }
    }
    out
}

/// Scans the roots of `layouts` for records and orphaned staging folders;
/// a root that does not exist holds nothing. `busy` names the staging
/// folders a running job is extracting into, which are not orphans.
#[must_use]
pub fn scan(layouts: &[Layout], busy: &[PathBuf]) -> Scan {
    let mut out = Scan::default();
    for layout in layouts {
        let Ok(entries) = std::fs::read_dir(layout.apps_root()) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let app_id = entry.file_name().to_string_lossy().into_owned();
            let record_path = dir.join(RECORD_FILE);
            if record_path.is_file() {
                match Record::read(&record_path) {
                    Ok(record) => {
                        let problems = problems(&record, layout);
                        out.apps.push(Installed {
                            record,
                            layout: layout.clone(),
                            dir: dir.clone(),
                            problems,
                        });
                    }
                    Err(error) => {
                        tracing::warn!(%error, path = %record_path.display(), "record not read");
                    }
                }
            }
            if let Ok(children) = std::fs::read_dir(&dir) {
                for child in children.flatten() {
                    let name = child.file_name().to_string_lossy().into_owned();
                    if let Some(version) = name.strip_prefix(STAGING_PREFIX)
                        && child.path().is_dir()
                        && !busy.contains(&child.path())
                    {
                        out.orphans.push(Orphan {
                            dir: child.path(),
                            app_id: app_id.clone(),
                            version: version.to_owned(),
                        });
                    }
                }
            }
        }
    }
    out.apps.sort_by_key(|a| a.record.name.to_lowercase());
    out.orphans.sort_by(|a, b| a.dir.cmp(&b.dir));
    out
}

/// Whether `name` matches `pattern`, a file name with `*` for any run of
/// characters, case-insensitively.
#[must_use]
pub fn glob_matches(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|i| go(&p[1..], &n[i..])),
            Some(c) => n.first().is_some_and(|d| d == c) && go(&p[1..], &n[1..]),
        }
    }
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    go(&p, &n)
}
