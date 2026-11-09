//! The app launcher entry (specs/system-integration.md §7.1): the desktop
//! entry `packaging/linux/rick.desktop` in the scope's
//! `share/applications` and the hicolor icons of `assets/icons/linux` in
//! its `share/icons/hicolor`. Both travel inside the binary, so the item
//! does the same from an unpacked archive, an installed copy and a
//! development build.

use std::fs;
use std::path::{Path, PathBuf};

use super::{DESKTOP_ID, LinuxEnv, ini_value, is_foreign};
use crate::integration::{Actual, Check, Receipt};

/// One icon of the hicolor theme: its path under `icons/hicolor` and its
/// bytes.
pub(super) struct Icon {
    pub rel: &'static str,
    pub bytes: &'static [u8],
}

macro_rules! icon {
    ($rel:literal) => {
        Icon {
            rel: $rel,
            bytes: include_bytes!(concat!("../../../../../assets/icons/linux/hicolor/", $rel)),
        }
    };
}

/// The icons the release archive ships under `share/icons/hicolor`
/// (specs/app-icons.md).
pub(super) const ICONS: [Icon; 10] = [
    icon!("16x16/apps/rick.png"),
    icon!("22x22/apps/rick.png"),
    icon!("24x24/apps/rick.png"),
    icon!("32x32/apps/rick.png"),
    icon!("48x48/apps/rick.png"),
    icon!("64x64/apps/rick.png"),
    icon!("128x128/apps/rick.png"),
    icon!("256x256/apps/rick.png"),
    icon!("512x512/apps/rick.png"),
    icon!("scalable/apps/rick.svg"),
];

/// Where the desktop entry goes: `<data home>/applications/rick.desktop`.
pub(super) fn entry_path(env: &LinuxEnv) -> PathBuf {
    env.data_home.join("applications").join(DESKTOP_ID)
}

/// Where an icon goes.
pub(super) fn icon_path(env: &LinuxEnv, icon: &Icon) -> PathBuf {
    env.data_home.join("icons/hicolor").join(icon.rel)
}

/// The launcher entry is set up when the desktop entry is there and
/// launches `binary`, and every icon is there. An entry that launches
/// something else and an icon with other contents are in the way unless
/// the receipt says Rick wrote them.
pub(super) fn check(env: &LinuxEnv, binary: &Path, receipt: &Receipt) -> Check {
    let mut check = Check {
        actual: Actual::Missing,
        place: Some(env.data_home.clone()),
        ..Check::default()
    };
    let entry = entry_path(env);
    let mut whole = fs::read_to_string(&entry).is_ok_and(|text| {
        ini_value(&text, "Desktop Entry", "Exec").is_some_and(|exec| env.launches(&exec, binary))
    });
    // An entry that launches something else, or something that is no
    // readable file: in the way either way.
    if !whole && is_foreign(env, &entry, receipt) {
        check.foreign.push(entry);
    }
    for icon in &ICONS {
        let path = icon_path(env, icon);
        match fs::read(&path) {
            Ok(bytes) => {
                if bytes != icon.bytes && is_foreign(env, &path, receipt) {
                    check.foreign.push(path);
                }
            }
            Err(_) => whole = false,
        }
    }
    if whole {
        check.actual = Actual::SetUp;
    }
    check
}

#[cfg(test)]
mod tests {
    use super::super::tests::{Home, write};
    use super::*;
    use crate::integration::{Integration as _, Item, Request};

    fn install_icons(env: &LinuxEnv) {
        for icon in &ICONS {
            let path = icon_path(env, icon);
            fs::create_dir_all(path.parent().unwrap_or(&path)).unwrap_or_else(|e| panic!("{e}"));
            fs::write(&path, icon.bytes).unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn the_entry_and_every_icon_make_the_launcher_entry() {
        let home = Home::new("GNOME");
        let integration = home.integration();
        let check = |receipt: Receipt| {
            integration.check_integration(
                Item::LauncherEntry,
                &Request {
                    receipt,
                    ..Request::default()
                },
            )
        };
        let found = check(Receipt::default());
        assert_eq!(found.actual, Actual::Missing);
        assert_eq!(found.place, Some(home.env.data_home.clone()));
        assert!(found.foreign.is_empty());

        // The entry alone is not the item.
        let entry = entry_path(&home.env);
        write(
            &entry,
            &format!(
                "[Desktop Entry]\nType=Application\nName=Rick\nExec=\"{}\" %U\nIcon=rick\n",
                home.env.exe.display()
            ),
        );
        assert_eq!(check(Receipt::default()).actual, Actual::Missing);
        install_icons(&home.env);
        let found = check(Receipt::default());
        assert_eq!(found.actual, Actual::SetUp);
        assert!(found.foreign.is_empty());

        // Removed by hand: missing again.
        fs::remove_file(&entry).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(check(Receipt::default()).actual, Actual::Missing);
    }

    #[test]
    fn an_entry_of_another_origin_is_in_the_way_unless_rick_wrote_it() {
        let home = Home::new("GNOME");
        let integration = home.integration();
        let entry = entry_path(&home.env);
        write(
            &entry,
            "[Desktop Entry]\nType=Application\nName=Rick\nExec=/opt/other/rick %U\n",
        );
        install_icons(&home.env);
        let request = Request::default();
        let found = integration.check_integration(Item::LauncherEntry, &request);
        assert_eq!(found.actual, Actual::Missing);
        assert_eq!(found.foreign, std::slice::from_ref(&entry));
        // The same file, listed in Rick's receipt: stale, and Rick's own.
        let request = Request {
            receipt: Receipt {
                files: vec![home.env.tilde(&entry)],
                ..Receipt::default()
            },
            ..Request::default()
        };
        let found = integration.check_integration(Item::LauncherEntry, &request);
        assert_eq!(found.actual, Actual::Missing);
        assert!(found.foreign.is_empty());
        // An icon with other contents is in the way too.
        let icon = icon_path(&home.env, &ICONS[0]);
        fs::write(&icon, b"not rick's icon").unwrap_or_else(|e| panic!("{e}"));
        let found = integration.check_integration(Item::LauncherEntry, &request);
        assert_eq!(found.foreign, [icon]);
    }

    #[test]
    fn the_icons_are_the_archives() {
        assert_eq!(ICONS.len(), 10);
        for icon in &ICONS {
            assert!(!icon.bytes.is_empty(), "{}", icon.rel);
            let png = icon.bytes.starts_with(b"\x89PNG");
            let named_png = Path::new(icon.rel)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"));
            assert_eq!(png, named_png, "{}", icon.rel);
        }
    }
}
