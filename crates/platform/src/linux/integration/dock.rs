//! The dock or launcher favourite (specs/system-integration.md §7.1),
//! *Best effort*: GNOME's `org.gnome.shell favorite-apps` and Cinnamon's
//! `org.cinnamon favorite-apps` through `gsettings`, the launcher list of
//! Plasma's task manager in `plasma-org.kde.plasma.desktop-appletsrc`,
//! and the favourites of COSMIC's dock under `~/.config/cosmic`. Every
//! other desktop is unsupported.
//!
//! Setting the favourite up appends Rick to the desktop's list and puts
//! on record where, with the list as it was; the undo takes Rick out of
//! the list as it is by then, so favourites the user added or removed
//! since stay as they are.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use types::Error;

use super::{APP_ID, DESKTOP_ID, LinuxEnv, run_tool, write_file};
use crate::integration::{Actual, Check, Desktop, Reason, Receipt, Request};

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
            let managers =
                fs::read_to_string(&file).map_or_default(|text| plasma_task_managers(&text));
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

/// A string list as `gsettings set` takes it: `['a', 'b']`, and
/// `@as []` for none, which needs its type said.
fn gvariant_list(items: &[String]) -> String {
    if items.is_empty() {
        return "@as []".to_owned();
    }
    let quoted: Vec<String> = items
        .iter()
        .map(|item| {
            let mut out = String::from("'");
            for c in item.chars() {
                if matches!(c, '\'' | '\\') {
                    out.push('\\');
                }
                out.push(c);
            }
            out.push('\'');
            out
        })
        .collect();
    format!("[{}]", quoted.join(", "))
}

/// A string list as COSMIC's configuration writes one: RON, an item a
/// line, each with its comma.
fn ron_list(items: &[String]) -> String {
    if items.is_empty() {
        return "[]".to_owned();
    }
    let mut out = String::from("[\n");
    for item in items {
        out.push_str("    \"");
        for c in item.chars() {
            if matches!(c, '"' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push_str("\",\n");
    }
    out.push(']');
    out
}

/// Rick as a launcher of Plasma's task manager.
const PLASMA_LAUNCHER: &str = "applications:rick.desktop";
/// The launchers a Plasma task manager shows while its configuration
/// names none: the default of the applet's own configuration schema.
/// Written out before Rick is added, since a list in the file replaces
/// the default instead of adding to it.
const PLASMA_DEFAULT: &str = "applications:systemsettings.desktop,applications:org.kde.discover.desktop,preferred://filemanager,preferred://browser";

/// The header of the group a task manager keeps its launchers in.
fn plasma_general(manager: &str) -> String {
    format!("{manager}[Configuration][General]")
}

/// Plasma's panel configuration with the launcher list of the task
/// manager `manager` set to `value`, or without the key for `None`.
/// Every other line stays as it is; the group is added at the end when
/// it is not there, and goes again when its last key does.
fn plasma_with_launchers(text: &str, manager: &str, value: Option<&str>) -> String {
    let general = plasma_general(manager);
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
    let mut start = None;
    let mut end = lines.len();
    for (index, line) in lines.iter().enumerate() {
        let line = line.trim();
        if !line.starts_with('[') {
            continue;
        }
        if start.is_some() {
            end = index;
            break;
        }
        if line == general {
            start = Some(index);
        }
    }
    let Some(start) = start else {
        let Some(value) = value else {
            return text.to_owned();
        };
        let mut out = text.to_owned();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&general);
        out.push_str("\nlaunchers=");
        out.push_str(value);
        out.push('\n');
        return out;
    };
    let key = (start + 1..end).find(|index| {
        lines[*index]
            .split_once('=')
            .is_some_and(|(key, _)| key.trim() == "launchers")
    });
    match (key, value) {
        (Some(index), Some(value)) => lines[index] = format!("launchers={value}\n"),
        (None, Some(value)) => {
            // After the group's last line that says something.
            let at = (start + 1..end)
                .rev()
                .find(|index| !lines[*index].trim().is_empty())
                .map_or(start + 1, |index| index + 1);
            if at > 0 && !lines[at - 1].ends_with('\n') {
                lines[at - 1].push('\n');
            }
            lines.insert(at, format!("launchers={value}\n"));
        }
        (Some(index), None) => {
            lines.remove(index);
            let end = end - 1;
            if lines[start + 1..end]
                .iter()
                .all(|line| line.trim().is_empty())
            {
                // The group says nothing any more: it goes, with the
                // empty line that set it apart at the end of the file.
                let mut from = start;
                if end == lines.len() && from > 0 && lines[from - 1].trim().is_empty() {
                    from -= 1;
                }
                lines.drain(from..end);
            }
        }
        (None, None) => {}
    }
    lines.concat()
}

/// Asks the running Plasma shell to start over from its configuration,
/// which it otherwise reads at login only. Only with a session bus and
/// `dbus-send`; a failure is logged.
fn reload_plasma(env: &LinuxEnv) {
    let Some(address) = &env.session_bus else {
        return;
    };
    let bus = format!("--bus={address}");
    run_tool(
        env,
        "dbus-send",
        &[
            bus.as_ref(),
            "--dest=org.kde.plasmashell".as_ref(),
            "--type=method_call".as_ref(),
            "/PlasmaShell".as_ref(),
            "org.kde.PlasmaShell.refreshCurrentShell".as_ref(),
        ],
    );
}

/// What a desktop outside the four answers.
const NO_DOCK: &str = "this desktop has no dock Rick knows how to pin to";

/// Adds Rick to the desktop's favourites and records where, with the
/// list as it was. Nothing is written when Rick is among them already.
///
/// The favourite names the launcher entry, so there is nothing to pin
/// while that is not installed.
pub(super) fn perform(
    env: &LinuxEnv,
    gsettings: &dyn Gsettings,
    request: &Request,
) -> Result<Receipt, Error> {
    let Some(desktop) = Desktop::from_current_desktop(&env.current_desktop) else {
        return Err(Error::Unsupported(NO_DOCK));
    };
    if !super::mime::entry_installed(env) {
        return Err(Error::Unsupported(
            "the dock favourite needs the app launcher entry, which is not set up",
        ));
    }
    let mut receipt = request.receipt.clone();
    match desktop {
        Desktop::Gnome | Desktop::Cinnamon => {
            let Some((schema, key)) = gsettings_key(desktop) else {
                return Err(Error::Unsupported(NO_DOCK));
            };
            let value = gsettings.get(schema, key)?;
            let mut ids = strings_in(&value);
            if !ids.iter().any(|id| id == DESKTOP_ID) {
                ids.push(DESKTOP_ID.to_owned());
                gsettings.set(schema, key, &gvariant_list(&ids))?;
                receipt.previous.insert(format!("{schema} {key}"), value);
            }
        }
        Desktop::Plasma => {
            let file = plasma_file(env);
            let text = fs::read_to_string(&file).map_err(Error::from_io)?;
            let managers = plasma_task_managers(&text);
            if managers
                .iter()
                .any(|(_, launchers)| launchers.as_deref().is_some_and(plasma_has_rick))
            {
                return Ok(receipt);
            }
            let Some((manager, launchers)) = managers.into_iter().next() else {
                return Err(Error::Unsupported("Plasma's panels have no task manager"));
            };
            let list = match launchers.as_deref() {
                None => format!("{PLASMA_DEFAULT},{PLASMA_LAUNCHER}"),
                Some("") => PLASMA_LAUNCHER.to_owned(),
                Some(list) => format!("{list},{PLASMA_LAUNCHER}"),
            };
            let out = plasma_with_launchers(&text, &manager, Some(&list));
            write_file(&file, out.as_bytes())?;
            receipt.previous.insert(
                format!("{} {manager}", env.tilde(&file)),
                launchers.unwrap_or_default(),
            );
            reload_plasma(env);
        }
        Desktop::Cosmic => {
            let file = cosmic_file(env);
            let text = fs::read_to_string(&file).map_err(Error::from_io)?;
            let mut ids = strings_in(&text);
            if !ids.iter().any(|id| id == APP_ID) {
                ids.push(APP_ID.to_owned());
                let mut out = ron_list(&ids);
                if text.ends_with('\n') {
                    out.push('\n');
                }
                write_file(&file, out.as_bytes())?;
                receipt.previous.insert(env.tilde(&file), text);
            }
        }
    }
    Ok(receipt)
}

/// Takes Rick out of the favourites the receipt names, wherever the
/// desktop keeps them, leaving the rest of each list as it is by now. A
/// list that is no longer there is skipped.
pub(super) fn undo(
    env: &LinuxEnv,
    gsettings: &dyn Gsettings,
    receipt: &Receipt,
) -> Result<(), Error> {
    for desktop in [Desktop::Gnome, Desktop::Cinnamon] {
        let Some((schema, key)) = gsettings_key(desktop) else {
            continue;
        };
        if !receipt.previous.contains_key(&format!("{schema} {key}")) {
            continue;
        }
        // No gsettings or no such key any more: nothing to take Rick
        // out of.
        let Ok(value) = gsettings.get(schema, key) else {
            continue;
        };
        let ids = strings_in(&value);
        let kept: Vec<String> = ids.iter().filter(|id| *id != DESKTOP_ID).cloned().collect();
        if kept.len() != ids.len() {
            gsettings.set(schema, key, &gvariant_list(&kept))?;
        }
    }

    let file = plasma_file(env);
    let prefix = format!("{} ", env.tilde(&file));
    let managers: Vec<(&str, &str)> = receipt
        .previous
        .iter()
        .filter_map(|(key, previous)| Some((key.strip_prefix(&prefix)?, previous.as_str())))
        .collect();
    if !managers.is_empty()
        && let Ok(text) = fs::read_to_string(&file)
    {
        let mut out = text.clone();
        for (manager, previous) in managers {
            let Some((_, Some(launchers))) = plasma_task_managers(&out)
                .into_iter()
                .find(|(group, _)| group == manager)
            else {
                continue;
            };
            if !plasma_has_rick(&launchers) {
                continue;
            }
            let kept: Vec<&str> = launchers
                .split(',')
                .filter(|launcher| !plasma_has_rick(launcher))
                .collect();
            let kept = kept.join(",");
            // The default Rick wrote out for a task manager that named
            // no launchers goes back to being unsaid.
            let unsaid = previous.is_empty() && kept == PLASMA_DEFAULT;
            let value = (!unsaid).then_some(kept.as_str());
            out = plasma_with_launchers(&out, manager, value);
        }
        if out != text {
            write_file(&file, out.as_bytes())?;
            reload_plasma(env);
        }
    }

    let file = cosmic_file(env);
    if receipt.previous.contains_key(&env.tilde(&file))
        && let Ok(text) = fs::read_to_string(&file)
    {
        let ids = strings_in(&text);
        let kept: Vec<String> = ids.iter().filter(|id| *id != APP_ID).cloned().collect();
        if kept.len() != ids.len() {
            let mut out = ron_list(&kept);
            if text.ends_with('\n') {
                out.push('\n');
            }
            write_file(&file, out.as_bytes())?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, PoisonError};

    use super::super::LinuxIntegration;
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

    /// A home whose launcher entry is installed: what a favourite names.
    fn with_entry(desktop: &str) -> Home {
        let home = Home::new(desktop);
        write(
            &home.env.data_home.join("applications").join(DESKTOP_ID),
            "[Desktop Entry]\nName=Rick\n",
        );
        home
    }

    fn with_receipt(receipt: &Receipt) -> Request {
        Request {
            receipt: receipt.clone(),
            ..Request::default()
        }
    }

    #[test]
    fn gnome_and_cinnamon_get_rick_appended_and_the_undo_takes_only_rick_out() {
        for (desktop, schema) in [
            ("ubuntu:GNOME", "org.gnome.shell"),
            ("X-Cinnamon", "org.cinnamon"),
        ] {
            let home = with_entry(desktop);
            let before = "['org.gnome.Nautilus.desktop', 'firefox.desktop']";
            let settings = Arc::new(Settings::with(schema, "favorite-apps", before));
            let favourites = || settings.get(schema, "favorite-apps").unwrap_or_default();
            let integration = LinuxIntegration::new(home.env.clone(), settings.clone());
            let receipt = integration
                .perform_integration(Item::DockFavourite, &Request::default())
                .unwrap_or_else(|e| panic!("{desktop}: {e}"));
            assert_eq!(
                favourites(),
                "['org.gnome.Nautilus.desktop', 'firefox.desktop', 'rick.desktop']",
                "{desktop}"
            );
            assert_eq!(
                receipt
                    .previous
                    .get(&format!("{schema} favorite-apps"))
                    .map(String::as_str),
                Some(before)
            );
            assert_eq!(receipt.files, [] as [String; 0]);
            let request = with_receipt(&receipt);
            assert_eq!(
                integration
                    .check_integration(Item::DockFavourite, &request)
                    .actual,
                Actual::SetUp
            );
            // Once more: Rick is there, nothing is written or recorded.
            assert_eq!(
                integration
                    .perform_integration(Item::DockFavourite, &request)
                    .ok(),
                Some(receipt)
            );
            assert_eq!(
                favourites(),
                "['org.gnome.Nautilus.desktop', 'firefox.desktop', 'rick.desktop']"
            );

            // The user rearranged the favourites since: the undo takes
            // Rick out and leaves the rest as it is by now.
            settings
                .set(
                    schema,
                    "favorite-apps",
                    "['rick.desktop', 'org.gnome.Terminal.desktop', 'firefox.desktop']",
                )
                .unwrap_or_else(|e| panic!("{e}"));
            integration
                .undo_integration(Item::DockFavourite, &request)
                .unwrap_or_else(|e| panic!("{desktop}: {e}"));
            assert_eq!(
                favourites(),
                "['org.gnome.Terminal.desktop', 'firefox.desktop']"
            );
            // Without a record there is nothing of Rick's to take out.
            settings
                .set(schema, "favorite-apps", "['rick.desktop']")
                .unwrap_or_else(|e| panic!("{e}"));
            integration
                .undo_integration(Item::DockFavourite, &Request::default())
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(favourites(), "['rick.desktop']");
            // The last favourite gone leaves the empty list, typed.
            integration
                .undo_integration(Item::DockFavourite, &request)
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(favourites(), "@as []");
            integration
                .perform_integration(Item::DockFavourite, &Request::default())
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(favourites(), "['rick.desktop']");
        }
    }

    #[test]
    fn nothing_is_pinned_without_the_launcher_entry_or_the_means() {
        // No launcher entry: the favourite would name nothing.
        let home = Home::new("GNOME");
        let settings = Arc::new(Settings::with(
            "org.gnome.shell",
            "favorite-apps",
            "['firefox.desktop']",
        ));
        let integration = LinuxIntegration::new(home.env.clone(), settings.clone());
        assert!(matches!(
            integration.perform_integration(Item::DockFavourite, &Request::default()),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(
            settings.get("org.gnome.shell", "favorite-apps").ok(),
            Some("['firefox.desktop']".to_owned())
        );

        // No gsettings, and a desktop Rick knows no dock of.
        let home = with_entry("GNOME");
        let integration = LinuxIntegration::new(
            home.env.clone(),
            Arc::new(Settings {
                missing: true,
                ..Settings::default()
            }),
        );
        assert!(matches!(
            integration.perform_integration(Item::DockFavourite, &Request::default()),
            Err(Error::NotFound)
        ));
        let home = with_entry("XFCE");
        assert!(matches!(
            home.integration()
                .perform_integration(Item::DockFavourite, &Request::default()),
            Err(Error::Unsupported(_))
        ));

        // Plasma without a panel configuration, COSMIC without its list.
        for desktop in ["KDE", "COSMIC"] {
            let home = with_entry(desktop);
            let before = snapshot(home.dir.path());
            assert!(
                home.integration()
                    .perform_integration(Item::DockFavourite, &Request::default())
                    .is_err(),
                "{desktop}"
            );
            assert_eq!(snapshot(home.dir.path()), before, "{desktop}");
        }
    }

    #[test]
    fn plasma_gets_rick_at_the_end_of_the_first_task_managers_launchers() {
        let home = with_entry("KDE");
        let file = plasma_file(&home.env);
        let launchers = "applications:systemsettings.desktop,preferred://filemanager";
        let original = plasma_fixture(Some(launchers));
        write(&file, &original);
        let integration = home.integration();
        let receipt = integration
            .perform_integration(Item::DockFavourite, &Request::default())
            .unwrap_or_else(|e| panic!("{e}"));
        let text = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            text.contains(&format!(
                "launchers={launchers},applications:rick.desktop\n"
            )),
            "{text}"
        );
        assert_eq!(
            text.replace(",applications:rick.desktop", ""),
            original,
            "every other line is as it was"
        );
        assert_eq!(
            receipt.previous,
            BTreeMap::from([(
                "~/.config/plasma-org.kde.plasma.desktop-appletsrc [Containments][2][Applets][5]"
                    .to_owned(),
                launchers.to_owned()
            )])
        );
        let request = with_receipt(&receipt);
        assert_eq!(
            integration
                .check_integration(Item::DockFavourite, &request)
                .actual,
            Actual::SetUp
        );
        assert_eq!(
            integration
                .perform_integration(Item::DockFavourite, &request)
                .ok(),
            Some(receipt)
        );
        assert_eq!(fs::read_to_string(&file).ok(), Some(text));

        // The user pinned something after Rick: it stays.
        write(
            &file,
            &plasma_fixture(Some(&format!(
                "{launchers},applications:rick.desktop,applications:org.kde.konsole.desktop"
            ))),
        );
        integration
            .undo_integration(Item::DockFavourite, &request)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            fs::read_to_string(&file).ok(),
            Some(plasma_fixture(Some(&format!(
                "{launchers},applications:org.kde.konsole.desktop"
            ))))
        );
    }

    #[test]
    fn a_plasma_task_manager_that_names_no_launchers_keeps_its_defaults() {
        let home = with_entry("KDE");
        let file = plasma_file(&home.env);
        let general = "[Containments][2][Applets][5][Configuration][General]";
        let integration = home.integration();

        // No group at all: it is added at the end with the defaults and
        // Rick, and goes again with the undo.
        let original = plasma_fixture(None);
        write(&file, &original);
        let receipt = integration
            .perform_integration(Item::DockFavourite, &Request::default())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            fs::read_to_string(&file).ok(),
            Some(format!(
                "{original}\n{general}\nlaunchers={PLASMA_DEFAULT},applications:rick.desktop\n"
            ))
        );
        assert_eq!(
            receipt
                .previous
                .values()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [""]
        );
        integration
            .undo_integration(Item::DockFavourite, &with_receipt(&receipt))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fs::read_to_string(&file).ok(), Some(original.clone()));

        // The group with other keys: the list joins them, and only it
        // goes again.
        let with_group = format!("{original}\n{general}\nshowOnlyCurrentDesktop=false\n");
        write(&file, &with_group);
        let receipt = integration
            .perform_integration(Item::DockFavourite, &Request::default())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            fs::read_to_string(&file).ok(),
            Some(format!(
                "{with_group}launchers={PLASMA_DEFAULT},applications:rick.desktop\n"
            ))
        );
        integration
            .undo_integration(Item::DockFavourite, &with_receipt(&receipt))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fs::read_to_string(&file).ok(), Some(with_group));

        // A list the user emptied gets Rick alone and is empty again.
        let emptied = plasma_fixture(Some(""));
        write(&file, &emptied);
        let receipt = integration
            .perform_integration(Item::DockFavourite, &Request::default())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            fs::read_to_string(&file).ok(),
            Some(plasma_fixture(Some("applications:rick.desktop")))
        );
        integration
            .undo_integration(Item::DockFavourite, &with_receipt(&receipt))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fs::read_to_string(&file).ok(), Some(emptied));
    }

    #[test]
    fn cosmic_gets_rick_at_the_end_of_its_favourites() {
        let home = with_entry("COSMIC");
        let file = cosmic_file(&home.env);
        let original = "[\n    \"firefox\",\n    \"com.system76.CosmicFiles\",\n]";
        write(&file, original);
        let integration = home.integration();
        let receipt = integration
            .perform_integration(Item::DockFavourite, &Request::default())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            fs::read_to_string(&file).ok().as_deref(),
            Some("[\n    \"firefox\",\n    \"com.system76.CosmicFiles\",\n    \"rick\",\n]")
        );
        assert_eq!(
            receipt.previous,
            BTreeMap::from([(
                "~/.config/cosmic/com.system76.CosmicAppList/v1/favorites".to_owned(),
                original.to_owned()
            )])
        );
        let request = with_receipt(&receipt);
        assert_eq!(
            integration
                .check_integration(Item::DockFavourite, &request)
                .actual,
            Actual::SetUp
        );
        integration
            .undo_integration(Item::DockFavourite, &request)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fs::read_to_string(&file).ok().as_deref(), Some(original));

        // The only favourite, in a file that ends with a line break.
        write(&file, "[]\n");
        let receipt = integration
            .perform_integration(Item::DockFavourite, &Request::default())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            fs::read_to_string(&file).ok().as_deref(),
            Some("[\n    \"rick\",\n]\n")
        );
        integration
            .undo_integration(Item::DockFavourite, &with_receipt(&receipt))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fs::read_to_string(&file).ok().as_deref(), Some("[]\n"));
    }

    #[test]
    fn lists_are_written_as_each_desktop_reads_them() {
        let items = ["a.desktop".to_owned(), "it's.desktop".to_owned()];
        assert_eq!(gvariant_list(&items), "['a.desktop', 'it\\'s.desktop']");
        assert_eq!(strings_in(&gvariant_list(&items)), items);
        assert_eq!(gvariant_list(&[]), "@as []");
        let items = ["a".to_owned(), "say \"hi\"".to_owned()];
        assert_eq!(strings_in(&ron_list(&items)), items);
        assert_eq!(ron_list(&[]), "[]");
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
