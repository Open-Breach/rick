//! The default file manager (specs/system-integration.md §7.1):
//! `inode/directory` and `x-scheme-handler/rick` set to `rick.desktop` in
//! the scope's `mimeapps.list`, the file `xdg-mime default` writes.

use std::fs;
use std::path::PathBuf;

use super::super::apps::{ids_in, parse_desktop};
use super::{DESKTOP_ID, LinuxEnv};
use crate::integration::{Actual, Check};

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
}
