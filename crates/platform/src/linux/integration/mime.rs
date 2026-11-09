//! The default file manager (specs/system-integration.md §7.1):
//! `inode/directory` and `x-scheme-handler/rick` set to `rick.desktop` in
//! the scope's `mimeapps.list`, the file `xdg-mime default` writes.

use std::fs;
use std::path::PathBuf;

use types::Error;

use super::super::apps::{ids_in, parse_desktop};
use super::{DESKTOP_ID, LinuxEnv, record, remove, write_file};
use crate::integration::{Actual, Check, Receipt, Request};

/// The two types Rick is made the default of.
pub(super) const TYPES: [&str; 2] = ["inode/directory", "x-scheme-handler/rick"];
/// The group of `mimeapps.list` that names the defaults.
pub(super) const DEFAULTS: &str = "Default Applications";

/// The user's `mimeapps.list`: `<config home>/mimeapps.list`.
pub(super) fn list_path(env: &LinuxEnv) -> PathBuf {
    env.config_home.join("mimeapps.list")
}

/// The desktop id that is the default for `mime` in a `mimeapps.list`
/// text: the first of the key's list.
pub(super) fn default_in(text: &str, mime: &str) -> Option<String> {
    ids_in(text, DEFAULTS, mime).into_iter().next()
}

/// The desktop id that opens folders now: the user's default, else the
/// first default the data directories name, else the first application
/// their caches register for folders.
fn present_default(env: &LinuxEnv, user: &str) -> Option<String> {
    let mime = TYPES[0];
    if let Some(id) = default_in(user, mime) {
        return Some(id);
    }
    let dirs: Vec<PathBuf> = std::iter::once(&env.data_home)
        .chain(&env.data_dirs)
        .map(|dir| dir.join("applications"))
        .collect();
    for dir in &dirs {
        for (file, group) in [("mimeapps.list", DEFAULTS), ("defaults.list", DEFAULTS)] {
            if let Some(id) = fs::read_to_string(dir.join(file))
                .ok()
                .and_then(|text| ids_in(&text, group, mime).into_iter().next())
            {
                return Some(id);
            }
        }
    }
    dirs.iter().find_map(|dir| {
        let text = fs::read_to_string(dir.join("mimeinfo.cache")).ok()?;
        ids_in(&text, "MIME Cache", mime).into_iter().next()
    })
}

/// The name the desktop file `id` gives its application.
fn name_of(env: &LinuxEnv, id: &str) -> Option<String> {
    std::iter::once(&env.data_home)
        .chain(&env.data_dirs)
        .find_map(|dir| fs::read_to_string(dir.join("applications").join(id)).ok())
        .and_then(|text| parse_desktop(&text))
        .map(|entry| entry.name)
}

/// Rick is the default file manager when both types name `rick.desktop`
/// first in the user's `mimeapps.list`; otherwise the check names the
/// application that opens folders now.
pub(super) fn check(env: &LinuxEnv) -> Check {
    let file = list_path(env);
    let text = fs::read_to_string(&file).unwrap_or_default();
    let mut check = Check {
        actual: Actual::Missing,
        place: Some(file),
        ..Check::default()
    };
    if TYPES
        .iter()
        .all(|mime| default_in(&text, mime).as_deref() == Some(DESKTOP_ID))
    {
        check.actual = Actual::SetUp;
        return check;
    }
    check.name = present_default(env, &text)
        .filter(|id| id != DESKTOP_ID)
        .and_then(|id| name_of(env, &id));
    check
}

/// The whole value of `mime` under `[Default Applications]`, as written.
fn raw_default(text: &str, mime: &str) -> Option<String> {
    super::ini_value(text, DEFAULTS, mime)
}

/// `text` with `mime` under `[Default Applications]` set to `value`, or
/// without the key for `None`; every other line stays as it is. The
/// group is added at the end when the text has none.
fn with_default(text: &str, mime: &str, value: Option<&str>) -> String {
    let header = format!("[{DEFAULTS}]");
    let mut lines: Vec<String> = Vec::new();
    let mut inside = false;
    let mut seen_group = false;
    let mut written = false;
    // Where a new key goes: after the group's last line that is not blank.
    let mut insert_at: Option<usize> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == header;
            seen_group |= inside;
            lines.push(line.to_owned());
            if inside {
                insert_at = Some(lines.len());
            }
            continue;
        }
        let is_key = inside
            && trimmed
                .split_once('=')
                .is_some_and(|(key, _)| key.trim() == mime);
        if is_key {
            // The first occurrence carries the new value; a repeated key
            // goes, as does the key itself when there is no value.
            if let Some(value) = value.filter(|_| !written) {
                lines.push(format!("{mime}={value}"));
                insert_at = Some(lines.len());
            }
            written = true;
            continue;
        }
        lines.push(line.to_owned());
        if inside && !trimmed.is_empty() {
            insert_at = Some(lines.len());
        }
    }
    if let Some(value) = value.filter(|_| !written) {
        let line = format!("{mime}={value}");
        if let Some(at) = insert_at {
            lines.insert(at, line);
        } else {
            if lines.last().is_some_and(|last| !last.trim().is_empty()) {
                lines.push(String::new());
            }
            if !seen_group {
                lines.push(header);
            }
            lines.push(line);
        }
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Whether a `mimeapps.list` text names anything at all: a key under any
/// group.
fn has_keys(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        !line.starts_with('[') && !line.starts_with('#') && line.contains('=')
    })
}

/// Whether `rick.desktop` is where the desktop looks for it; a default
/// that names a desktop file nobody finds would open folders with
/// nothing.
fn entry_installed(env: &LinuxEnv) -> bool {
    std::iter::once(&env.data_home)
        .chain(&env.data_dirs)
        .any(|dir| dir.join("applications").join(DESKTOP_ID).is_file())
}

/// Makes `rick.desktop` the default of both types, as `xdg-mime default`
/// would, and records what each type named before; the file itself is
/// recorded when it did not exist.
pub(super) fn perform(env: &LinuxEnv, request: &Request) -> Result<Receipt, Error> {
    if !entry_installed(env) {
        return Err(Error::Unsupported(
            "the default file manager needs the app launcher entry, which is not set up",
        ));
    }
    let file = list_path(env);
    let existing = match fs::read_to_string(&file) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(Error::from_io(error)),
    };
    let text = existing.clone().unwrap_or_default();
    let mut receipt = request.receipt.clone();
    let mut out = text.clone();
    for mime in TYPES {
        if default_in(&out, mime).as_deref() == Some(DESKTOP_ID) {
            // Rick already: what an earlier perform replaced stays on
            // record, and without one there was nothing before.
            receipt.previous.entry(mime.to_owned()).or_default();
            continue;
        }
        receipt
            .previous
            .insert(mime.to_owned(), raw_default(&out, mime).unwrap_or_default());
        out = with_default(&out, mime, Some(DESKTOP_ID));
    }
    if out != text {
        write_file(&file, out.as_bytes())?;
    }
    if existing.is_none() {
        record(env, &mut receipt, &file);
    }
    Ok(receipt)
}

/// Puts back what each type named before, where it still names Rick; a
/// type the user gave to another application since is left alone. A
/// `mimeapps.list` Rick created and that names nothing afterwards goes.
pub(super) fn undo(env: &LinuxEnv, receipt: &Receipt) -> Result<(), Error> {
    let file = list_path(env);
    let Ok(text) = fs::read_to_string(&file) else {
        return Ok(());
    };
    let mut out = text.clone();
    for mime in TYPES {
        let Some(previous) = receipt.previous.get(mime) else {
            continue;
        };
        if default_in(&out, mime).as_deref() == Some(DESKTOP_ID) {
            out = with_default(
                &out,
                mime,
                Some(previous.as_str()).filter(|p| !p.is_empty()),
            );
        }
    }
    if receipt.files.contains(&env.tilde(&file)) && !has_keys(&out) {
        return remove(&file);
    }
    if out != text {
        write_file(&file, out.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{Home, write};
    use super::*;
    use crate::integration::{Integration as _, Item, Request};

    #[test]
    fn both_types_default_to_rick_or_the_item_is_missing() {
        let mut home = Home::new("GNOME");
        let system = home.dir.path().join("usr/share");
        home.env.data_dirs = vec![system.clone()];
        write(
            &system.join("applications/org.gnome.Nautilus.desktop"),
            "[Desktop Entry]\nType=Application\nName=Files\nExec=nautilus %U\n",
        );
        write(
            &system.join("applications/mimeinfo.cache"),
            "[MIME Cache]\ninode/directory=org.gnome.Nautilus.desktop;\n",
        );
        let integration = home.integration();
        let request = Request::default();
        let found = integration.check_integration(Item::DefaultFileManager, &request);
        assert_eq!(found.actual, Actual::Missing);
        assert_eq!(found.name.as_deref(), Some("Files"));
        assert_eq!(found.place, Some(list_path(&home.env)));

        // One of the two is not the item.
        write(
            &list_path(&home.env),
            "[Default Applications]\ninode/directory=rick.desktop;\ntext/plain=org.gnome.TextEditor.desktop\n",
        );
        let found = integration.check_integration(Item::DefaultFileManager, &request);
        assert_eq!(found.actual, Actual::Missing);
        assert_eq!(found.name, None, "Rick opens folders already");

        write(
            &list_path(&home.env),
            "[Added Associations]\ninode/directory=org.gnome.Nautilus.desktop;\n\n[Default Applications]\ninode/directory=rick.desktop\nx-scheme-handler/rick=rick.desktop;\n",
        );
        let found = integration.check_integration(Item::DefaultFileManager, &request);
        assert_eq!(found.actual, Actual::SetUp);

        // The system put its own file manager back in front.
        write(
            &list_path(&home.env),
            "[Default Applications]\ninode/directory=org.gnome.Nautilus.desktop;rick.desktop;\nx-scheme-handler/rick=rick.desktop\n",
        );
        let found = integration.check_integration(Item::DefaultFileManager, &request);
        assert_eq!(found.actual, Actual::Missing);
        assert_eq!(found.name.as_deref(), Some("Files"));
    }

    /// A home whose launcher entry is in place, as the item before this
    /// one leaves it.
    fn with_entry() -> Home {
        let home = Home::new("GNOME");
        write(
            &home.env.data_home.join("applications/rick.desktop"),
            "[Desktop Entry]\nType=Application\nName=Rick\nExec=rick %U\n",
        );
        home
    }

    #[test]
    fn performing_sets_both_defaults_keeps_the_rest_and_undo_puts_back() {
        let home = with_entry();
        let integration = home.integration();
        let file = list_path(&home.env);
        let before = "# mine\n[Added Associations]\ntext/plain=gedit.desktop;\n\n[Default Applications]\ntext/plain=gedit.desktop\ninode/directory=org.gnome.Nautilus.desktop;thunar.desktop;\n\n[Removed Associations]\nimage/png=gimp.desktop;\n";
        write(&file, before);
        let receipt = integration
            .perform_integration(Item::DefaultFileManager, &Request::default())
            .unwrap_or_else(|e| panic!("{e}"));
        let after = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            after,
            "# mine\n[Added Associations]\ntext/plain=gedit.desktop;\n\n[Default Applications]\ntext/plain=gedit.desktop\ninode/directory=rick.desktop\nx-scheme-handler/rick=rick.desktop\n\n[Removed Associations]\nimage/png=gimp.desktop;\n"
        );
        assert_eq!(
            receipt.previous.get("inode/directory").map(String::as_str),
            Some("org.gnome.Nautilus.desktop;thunar.desktop;")
        );
        assert_eq!(
            receipt
                .previous
                .get("x-scheme-handler/rick")
                .map(String::as_str),
            Some("")
        );
        assert!(receipt.files.is_empty(), "the file was there before");
        let request = Request {
            receipt: receipt.clone(),
            ..Request::default()
        };
        assert_eq!(
            integration
                .check_integration(Item::DefaultFileManager, &request)
                .actual,
            Actual::SetUp
        );
        // A second perform keeps what the first one replaced.
        let again = integration
            .perform_integration(Item::DefaultFileManager, &request)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again, receipt);

        integration
            .undo_integration(Item::DefaultFileManager, &request)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fs::read_to_string(&file).unwrap_or_default(), before);
    }

    #[test]
    fn a_list_rick_created_goes_again_and_a_default_changed_since_stays() {
        let home = with_entry();
        let integration = home.integration();
        let file = list_path(&home.env);
        let receipt = integration
            .perform_integration(Item::DefaultFileManager, &Request::default())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            fs::read_to_string(&file).unwrap_or_default(),
            "[Default Applications]\ninode/directory=rick.desktop\nx-scheme-handler/rick=rick.desktop\n"
        );
        assert_eq!(receipt.files, [home.env.tilde(&file)]);
        let request = Request {
            receipt,
            ..Request::default()
        };
        // The user gives folders to another application: not Rick's to undo.
        write(
            &file,
            "[Default Applications]\ninode/directory=org.kde.dolphin.desktop\nx-scheme-handler/rick=rick.desktop\n",
        );
        integration
            .undo_integration(Item::DefaultFileManager, &request)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            fs::read_to_string(&file).unwrap_or_default(),
            "[Default Applications]\ninode/directory=org.kde.dolphin.desktop\n"
        );
        // With nothing else in it, the file Rick created goes with the undo.
        write(
            &file,
            "[Default Applications]\ninode/directory=rick.desktop\nx-scheme-handler/rick=rick.desktop\n",
        );
        integration
            .undo_integration(Item::DefaultFileManager, &request)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!file.exists());
    }

    #[test]
    fn without_the_launcher_entry_there_is_nothing_to_make_the_default() {
        let home = Home::new("GNOME");
        let result = home
            .integration()
            .perform_integration(Item::DefaultFileManager, &Request::default());
        assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
        assert!(!list_path(&home.env).exists());
    }

    #[test]
    fn a_key_is_set_replaced_and_removed_in_its_group_only() {
        assert_eq!(
            with_default("", "a/b", Some("x.desktop")),
            "[Default Applications]\na/b=x.desktop\n"
        );
        assert_eq!(
            with_default("[Other]\na/b=y.desktop\n", "a/b", Some("x.desktop")),
            "[Other]\na/b=y.desktop\n\n[Default Applications]\na/b=x.desktop\n"
        );
        assert_eq!(
            with_default(
                "[Default Applications]\na/b=y.desktop\na/b=z.desktop\n\n[Other]\na/b=y.desktop\n",
                "a/b",
                Some("x.desktop")
            ),
            "[Default Applications]\na/b=x.desktop\n\n[Other]\na/b=y.desktop\n"
        );
        assert_eq!(
            with_default(
                "[Default Applications]\na/b=y.desktop\nc/d=z.desktop\n",
                "a/b",
                None
            ),
            "[Default Applications]\nc/d=z.desktop\n"
        );
        assert!(!has_keys("[Default Applications]\n\n# note\n"));
        assert!(has_keys("[Default Applications]\nc/d=z.desktop\n"));
    }
}
