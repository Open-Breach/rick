//! The dock or launcher favourite (specs/system-integration.md §7.1),
//! *Best effort*: GNOME's `org.gnome.shell favorite-apps` and Cinnamon's
//! `org.cinnamon favorite-apps` through `gsettings`, the launcher list of
//! Plasma's task manager in `plasma-org.kde.plasma.desktop-appletsrc`,
//! and the favourites of COSMIC's dock under `~/.config/cosmic`. Every
//! other desktop is unsupported.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use types::Error;

use super::{APP_ID, DESKTOP_ID, LinuxEnv};
use crate::integration::{Actual, Check, Desktop, Reason};

/// The `gsettings` program, as far as the favourites need it. Behind a
/// trait so no test reads or writes the settings of the machine it runs
/// on.
pub trait Gsettings: Send + Sync {
    /// `gsettings get <schema> <key>`: the value as text, a `GVariant`
    /// string list such as `['a.desktop', 'b.desktop']`.
    ///
    /// # Errors
    ///
    /// `Error::NotFound` when `gsettings` is not installed;
    /// `Error::Unsupported` when the schema or the key is not there;
    /// otherwise the I/O error of running it.
    fn get(&self, schema: &str, key: &str) -> Result<String, Error>;

    /// `gsettings set <schema> <key> <value>`.
    ///
    /// # Errors
    ///
    /// As [`get`](Self::get).
    fn set(&self, schema: &str, key: &str, value: &str) -> Result<(), Error>;
}

/// The real `gsettings`, found on a `PATH`.
#[derive(Debug, Clone)]
pub(super) struct Program(Option<PathBuf>);

impl Program {
    /// `gsettings` in the first directory of `path` that has it.
    pub(super) fn on(path: &[PathBuf]) -> Self {
        Self(
            path.iter()
                .map(|dir| dir.join("gsettings"))
                .find(|candidate| candidate.is_file()),
        )
    }

    fn run(&self, args: &[&str]) -> Result<String, Error> {
        let program = self.0.as_ref().ok_or(Error::NotFound)?;
        let output = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(Error::from_io)?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        } else {
            // No such schema, no such key: the desktop this is for is not
            // the one that runs.
            Err(Error::Unsupported("gsettings does not have this key"))
        }
    }
}

impl Gsettings for Program {
    fn get(&self, schema: &str, key: &str) -> Result<String, Error> {
        self.run(&["get", schema, key])
    }

    fn set(&self, schema: &str, key: &str, value: &str) -> Result<(), Error> {
        self.run(&["set", schema, key, value]).map(drop)
    }
}

/// The schema and key of a desktop that keeps its favourites in
/// `gsettings`.
pub(super) fn gsettings_key(desktop: Desktop) -> Option<(&'static str, &'static str)> {
    match desktop {
        Desktop::Gnome => Some(("org.gnome.shell", "favorite-apps")),
        Desktop::Cinnamon => Some(("org.cinnamon", "favorite-apps")),
        Desktop::Plasma | Desktop::Cosmic => None,
    }
}

/// The strings of a list written as text: a `GVariant` string array
/// (`['a', 'b']`, `@as []`) or a RON list (`["a", "b"]`). Quotes of
/// either kind, backslash escapes inside them.
pub(super) fn strings_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\'' && c != '"' {
            continue;
        }
        let quote = c;
        let mut word = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => word.extend(chars.next()),
                c if c == quote => break,
                c => word.push(c),
            }
        }
        out.push(word);
    }
    out
}

/// Plasma's panel configuration.
pub(super) fn plasma_file(env: &LinuxEnv) -> PathBuf {
    env.config_home
        .join("plasma-org.kde.plasma.desktop-appletsrc")
}

/// The favourites of COSMIC's dock.
pub(super) fn cosmic_file(env: &LinuxEnv) -> PathBuf {
    env.config_home
        .join("cosmic/com.system76.CosmicAppList/v1/favorites")
}

/// The groups of Plasma's panel configuration that are task managers,
/// each as its header (`[Containments][2][Applets][5]`), with the
/// launcher list its `[Configuration][General]` group gives, when it has
/// one.
pub(super) fn plasma_task_managers(text: &str) -> Vec<(String, Option<String>)> {
    let mut group = String::new();
    let mut managers: Vec<(String, Option<String>)> = Vec::new();
    let mut launchers: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            line.clone_into(&mut group);
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "plugin"
                if matches!(
                    value.trim(),
                    "org.kde.plasma.icontasks" | "org.kde.plasma.taskmanager"
                ) =>
            {
                managers.push((group.clone(), None));
            }
            "launchers" => launchers.push((group.clone(), value.trim().to_owned())),
            _ => {}
        }
    }
    for (manager, list) in &mut managers {
        let general = format!("{manager}[Configuration][General]");
        *list = launchers
            .iter()
            .find(|(group, _)| *group == general)
            .map(|(_, value)| value.clone());
    }
    managers
}

/// Whether a Plasma launcher list has Rick: `applications:rick.desktop`,
/// or a `file://` URL of the desktop file.
pub(super) fn plasma_has_rick(launchers: &str) -> bool {
    launchers.split(',').any(|launcher| {
        let launcher = launcher.trim();
        launcher == format!("applications:{DESKTOP_ID}")
            || (launcher.starts_with("file://") && launcher.ends_with(&format!("/{DESKTOP_ID}")))
    })
}

fn unsupported(reason: Reason) -> Check {
    Check::unsupported(reason)
}

fn found(set_up: bool, place: Option<&Path>) -> Check {
    Check {
        actual: if set_up {
            Actual::SetUp
        } else {
            Actual::Missing
        },
        place: place.map(Path::to_path_buf),
        ..Check::default()
    }
}

/// The dock favourite: set up when the desktop's favourites name Rick;
/// unsupported on a desktop that is not one of the four, without
/// `gsettings` where the desktop needs it, and where the desktop's
/// configuration is not there to read.
pub(super) fn check(env: &LinuxEnv, gsettings: &dyn Gsettings) -> Check {
    let Some(desktop) = Desktop::from_current_desktop(&env.current_desktop) else {
        return unsupported(Reason::Desktop);
    };
    match desktop {
        Desktop::Gnome | Desktop::Cinnamon => {
            let Some((schema, key)) = gsettings_key(desktop) else {
                return unsupported(Reason::Desktop);
            };
            match gsettings.get(schema, key) {
                Ok(value) => found(strings_in(&value).iter().any(|id| id == DESKTOP_ID), None),
                Err(Error::NotFound) => unsupported(Reason::Program("gsettings".to_owned())),
                Err(_) => unsupported(Reason::Config(PathBuf::from(format!("{schema} {key}")))),
            }
        }
        Desktop::Plasma => {
            let file = plasma_file(env);
            let managers = fs::read_to_string(&file)
                .map(|text| plasma_task_managers(&text))
                .unwrap_or_default();
            if managers.is_empty() {
                return unsupported(Reason::Config(file));
            }
            let pinned = managers
                .iter()
                .any(|(_, launchers)| launchers.as_deref().is_some_and(plasma_has_rick));
            found(pinned, Some(&file))
        }
        Desktop::Cosmic => {
            let file = cosmic_file(env);
            match fs::read_to_string(&file) {
                Ok(text) => found(strings_in(&text).iter().any(|id| id == APP_ID), Some(&file)),
                Err(_) => unsupported(Reason::Config(file)),
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Mutex, PoisonError};

    use super::super::tests::{Home, snapshot, write};
    use super::*;
    use crate::integration::{Integration as _, Item, Request};

    /// A `gsettings` in memory: the keys the test set, every write
    /// recorded; a key that was never set is no key of this desktop.
    #[derive(Debug, Default)]
    pub(crate) struct Settings {
        pub values: Mutex<BTreeMap<String, String>>,
        pub missing: bool,
    }

    impl Settings {
        pub(crate) fn with(schema: &str, key: &str, value: &str) -> Self {
            let settings = Self::default();
            settings
                .values
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(format!("{schema} {key}"), value.to_owned());
            settings
        }
    }

    impl Gsettings for Settings {
        fn get(&self, schema: &str, key: &str) -> Result<String, Error> {
            if self.missing {
                return Err(Error::NotFound);
            }
            self.values
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&format!("{schema} {key}"))
                .cloned()
                .ok_or(Error::Unsupported("no such key"))
        }

        fn set(&self, schema: &str, key: &str, value: &str) -> Result<(), Error> {
            if self.missing {
                return Err(Error::NotFound);
            }
            self.values
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(format!("{schema} {key}"), value.to_owned());
            Ok(())
        }
    }

    fn dock(home: &Home, settings: Settings) -> Check {
        super::super::LinuxIntegration::new(home.env.clone(), std::sync::Arc::new(settings))
            .check_integration(Item::DockFavourite, &Request::default())
    }

    #[test]
    fn a_desktop_outside_the_four_has_no_dock_rick_knows() {
        for desktop in ["XFCE", "sway", "", "MATE:Unity"] {
            let home = Home::new(desktop);
            assert_eq!(
                dock(&home, Settings::default()).actual,
                Actual::Unsupported(Reason::Desktop),
                "{desktop}"
            );
        }
    }

    #[test]
    fn gnome_and_cinnamon_keep_their_favourites_in_gsettings() {
        for (desktop, schema) in [("GNOME", "org.gnome.shell"), ("X-Cinnamon", "org.cinnamon")] {
            let home = Home::new(desktop);
            let without = "['org.gnome.Nautilus.desktop', 'firefox.desktop']";
            assert_eq!(
                dock(&home, Settings::with(schema, "favorite-apps", without)).actual,
                Actual::Missing,
                "{desktop}"
            );
            let with = "['firefox.desktop', 'rick.desktop']";
            assert_eq!(
                dock(&home, Settings::with(schema, "favorite-apps", with)).actual,
                Actual::SetUp,
                "{desktop}"
            );
            assert_eq!(
                dock(&home, Settings::with(schema, "favorite-apps", "@as []")).actual,
                Actual::Missing
            );
            // No gsettings, or a gsettings without the desktop's schema.
            assert_eq!(
                dock(
                    &home,
                    Settings {
                        missing: true,
                        ..Settings::default()
                    }
                )
                .actual,
                Actual::Unsupported(Reason::Program("gsettings".to_owned()))
            );
            assert_eq!(
                dock(&home, Settings::default()).actual,
                Actual::Unsupported(Reason::Config(PathBuf::from(format!(
                    "{schema} favorite-apps"
                ))))
            );
        }
    }

    /// A Plasma panel with a task manager, as Plasma 6 writes it.
    pub(crate) fn plasma_fixture(launchers: Option<&str>) -> String {
        let mut text = String::from(
            "[ActionPlugins][0]\nRightButton;NoModifier=org.kde.contextmenu\n\n\
             [Containments][2]\nformfactor=2\nlocation=4\nplugin=org.kde.panel\n\n\
             [Containments][2][Applets][3]\nplugin=org.kde.plasma.kickoff\n\n\
             [Containments][2][Applets][5]\nimmutability=1\nplugin=org.kde.plasma.icontasks\n\n",
        );
        if let Some(launchers) = launchers {
            text.push_str("[Containments][2][Applets][5][Configuration][General]\nlaunchers=");
            text.push_str(launchers);
            text.push_str("\n\n");
        }
        text.push_str("[Containments][2][Applets][6]\nplugin=org.kde.plasma.systemtray\n");
        text
    }

    #[test]
    fn plasma_keeps_its_launchers_in_the_task_managers_configuration() {
        let home = Home::new("KDE");
        let file = plasma_file(&home.env);
        assert_eq!(
            dock(&home, Settings::default()).actual,
            Actual::Unsupported(Reason::Config(file.clone())),
            "no panel configuration"
        );
        write(&file, "[Containments][1]\nplugin=org.kde.plasma.folder\n");
        assert_eq!(
            dock(&home, Settings::default()).actual,
            Actual::Unsupported(Reason::Config(file.clone())),
            "no task manager in it"
        );
        write(&file, &plasma_fixture(None));
        assert_eq!(dock(&home, Settings::default()).actual, Actual::Missing);
        write(
            &file,
            &plasma_fixture(Some(
                "applications:systemsettings.desktop,preferred://filemanager",
            )),
        );
        let found = dock(&home, Settings::default());
        assert_eq!(found.actual, Actual::Missing);
        assert_eq!(found.place, Some(file.clone()));
        write(
            &file,
            &plasma_fixture(Some(
                "applications:systemsettings.desktop,applications:rick.desktop",
            )),
        );
        assert_eq!(dock(&home, Settings::default()).actual, Actual::SetUp);
        assert!(plasma_has_rick(
            "file:///home/u/.local/share/applications/rick.desktop"
        ));
        assert!(!plasma_has_rick("applications:patrick.desktop"));
    }

    #[test]
    fn cosmic_keeps_its_favourites_in_a_list_file() {
        let home = Home::new("COSMIC");
        let file = cosmic_file(&home.env);
        assert_eq!(
            dock(&home, Settings::default()).actual,
            Actual::Unsupported(Reason::Config(file.clone()))
        );
        write(
            &file,
            "[\n    \"firefox\",\n    \"com.system76.CosmicFiles\",\n]",
        );
        assert_eq!(dock(&home, Settings::default()).actual, Actual::Missing);
        write(&file, "[\n    \"firefox\",\n    \"rick\",\n]");
        assert_eq!(dock(&home, Settings::default()).actual, Actual::SetUp);
        // Reading the favourites wrote nothing.
        let before = snapshot(home.dir.path());
        let _ = dock(&home, Settings::default());
        assert_eq!(snapshot(home.dir.path()), before);
    }

    #[test]
    fn lists_are_read_in_both_spellings() {
        assert_eq!(
            strings_in("['a.desktop', 'b c.desktop']"),
            ["a.desktop", "b c.desktop"]
        );
        assert_eq!(strings_in("@as []"), Vec::<String>::new());
        assert_eq!(
            strings_in("[\"it's\", 'say \\'hi\\'']"),
            ["it's", "say 'hi'"]
        );
    }

    #[test]
    fn without_gsettings_on_the_path_the_program_says_so() {
        let program = Program::on(&[PathBuf::from("/nonexistent")]);
        assert!(matches!(
            program.get("org.gnome.shell", "favorite-apps"),
            Err(Error::NotFound)
        ));
    }
}
