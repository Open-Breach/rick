//! The app launcher entry (specs/system-integration.md §7.1): the desktop
//! entry `packaging/linux/rick.desktop` in the scope's
//! `share/applications` and the hicolor icons of `assets/icons/linux` in
//! its `share/icons/hicolor`. Both travel inside the binary, so the item
//! does the same from an unpacked archive, an installed copy and a
//! development build.

use std::fs;
use std::path::{Path, PathBuf};

use types::Error;

use super::{
    DESKTOP_ID, LinuxEnv, ini_value, is_foreign, refuse_foreign, remove, run_tool, write_recorded,
};
use crate::integration::{Actual, Check, Receipt, Request};

/// The desktop entry as the repository packages it.
const DESKTOP_ENTRY: &str = include_str!("../../../../../packaging/linux/rick.desktop");

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

/// A path as one quoted argument of a desktop `Exec` line: double quotes,
/// with `"`, `` ` ``, `$` and `\` escaped as the desktop entry
/// specification asks, and `%` doubled so it is no field code.
fn quote(path: &Path) -> String {
    let mut out = String::from("\"");
    for c in path.to_string_lossy().chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The packaged desktop entry with its `Exec` starting `binary` by its
/// full path, the arguments (`%U`) kept: the entry launches this Rick
/// wherever the `PATH` of the session points.
pub(super) fn entry_text(binary: &Path) -> String {
    let mut out = String::with_capacity(DESKTOP_ENTRY.len() + 64);
    for line in DESKTOP_ENTRY.lines() {
        match line.strip_prefix("Exec=") {
            Some(exec) => {
                out.push_str("Exec=");
                out.push_str(&quote(binary));
                if let Some((_, arguments)) = exec.trim().split_once(char::is_whitespace) {
                    out.push(' ');
                    out.push_str(arguments.trim());
                }
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// Tells the desktop about the change, where its tools are installed:
/// `update-desktop-database` for the entry's MIME types and
/// `gtk-update-icon-cache` for the icons.
fn refresh_caches(env: &LinuxEnv) {
    let applications = env.data_home.join("applications");
    run_tool(
        env,
        "update-desktop-database",
        &["-q".as_ref(), applications.as_os_str()],
    );
    let icons = env.data_home.join("icons/hicolor");
    run_tool(
        env,
        "gtk-update-icon-cache",
        &[
            "-q".as_ref(),
            "-t".as_ref(),
            "-f".as_ref(),
            icons.as_os_str(),
        ],
    );
}

/// Writes the desktop entry and the icons; what was written is added to
/// the receipt, and a file that was there with the same contents is left
/// out of it.
pub(super) fn perform(env: &LinuxEnv, binary: &Path, request: &Request) -> Result<Receipt, Error> {
    refuse_foreign(&check(env, binary, &request.receipt), request)?;
    let mut receipt = request.receipt.clone();
    write_recorded(
        env,
        &entry_path(env),
        entry_text(binary).as_bytes(),
        &mut receipt,
    )?;
    for icon in &ICONS {
        write_recorded(env, &icon_path(env, icon), icon.bytes, &mut receipt)?;
    }
    refresh_caches(env);
    Ok(receipt)
}

/// Removes the entry and the icons the receipt lists.
pub(super) fn undo(env: &LinuxEnv, receipt: &Receipt) -> Result<(), Error> {
    for file in &receipt.files {
        remove(&env.expand(file))?;
    }
    refresh_caches(env);
    Ok(())
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
        assert_eq!(found.foreign, [] as [PathBuf; 0]);

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
        assert_eq!(found.foreign, [] as [PathBuf; 0]);

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
        assert_eq!(found.foreign, [] as [PathBuf; 0]);
        // An icon with other contents is in the way too.
        let icon = icon_path(&home.env, &ICONS[0]);
        fs::write(&icon, b"not rick's icon").unwrap_or_else(|e| panic!("{e}"));
        let found = integration.check_integration(Item::LauncherEntry, &request);
        assert_eq!(found.foreign, [icon]);
    }

    #[test]
    fn performing_writes_the_entry_and_the_icons_and_undo_removes_them() {
        let home = Home::new("GNOME");
        let integration = home.integration();
        let receipt = integration
            .perform_integration(Item::LauncherEntry, &Request::default())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(receipt.files.len(), 11, "the entry and ten icons");
        assert!(
            receipt
                .files
                .contains(&"~/.local/share/applications/rick.desktop".to_owned())
        );
        let entry = fs::read_to_string(entry_path(&home.env)).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            entry.contains(&format!("Exec=\"{}\" %U\n", home.env.exe.display())),
            "{entry}"
        );
        assert!(entry.contains("MimeType=inode/directory;x-scheme-handler/rick;"));
        assert!(entry.contains("StartupWMClass=rick"));
        let request = Request {
            receipt: receipt.clone(),
            ..Request::default()
        };
        assert_eq!(
            integration
                .check_integration(Item::LauncherEntry, &request)
                .actual,
            Actual::SetUp
        );
        // Again: nothing to write, nothing new on record.
        let again = integration
            .perform_integration(Item::LauncherEntry, &request)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again, receipt);

        integration
            .undo_integration(Item::LauncherEntry, &request)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!entry_path(&home.env).exists());
        for icon in &ICONS {
            assert!(!icon_path(&home.env, icon).exists(), "{}", icon.rel);
        }
        // Undoing twice is fine: what is gone is skipped.
        integration
            .undo_integration(Item::LauncherEntry, &request)
            .unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn a_foreign_entry_is_replaced_only_on_request_and_identical_icons_stay_foreign() {
        let home = Home::new("GNOME");
        let integration = home.integration();
        let entry = entry_path(&home.env);
        let foreign = "[Desktop Entry]\nType=Application\nName=Rick\nExec=/opt/other/rick %U\n";
        write(&entry, foreign);
        // A manual install copied one icon already, byte for byte.
        let copied = icon_path(&home.env, &ICONS[4]);
        fs::create_dir_all(copied.parent().unwrap_or(&copied)).unwrap_or_else(|e| panic!("{e}"));
        fs::write(&copied, ICONS[4].bytes).unwrap_or_else(|e| panic!("{e}"));

        let refused = integration.perform_integration(Item::LauncherEntry, &Request::default());
        assert!(
            matches!(&refused, Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists),
            "{refused:?}"
        );
        assert_eq!(
            fs::read_to_string(&entry).unwrap_or_default(),
            foreign,
            "nothing was overwritten"
        );
        assert!(
            !icon_path(&home.env, &ICONS[0]).exists(),
            "nothing was written"
        );

        let receipt = integration
            .perform_integration(
                Item::LauncherEntry,
                &Request {
                    replace: true,
                    ..Request::default()
                },
            )
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(receipt.files.contains(&home.env.tilde(&entry)));
        assert!(
            !receipt.files.contains(&home.env.tilde(&copied)),
            "the icon that was there is not Rick's"
        );
        assert_eq!(receipt.files.len(), 10);
        let request = Request {
            receipt,
            ..Request::default()
        };
        integration
            .undo_integration(Item::LauncherEntry, &request)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!entry.exists());
        assert!(copied.exists(), "what Rick did not write stays");
    }

    #[test]
    fn a_path_with_spaces_and_signs_is_one_quoted_argument() {
        let text = entry_text(Path::new("/home/a b/50% $HOME/rick"));
        assert!(
            text.contains("Exec=\"/home/a b/50%% \\$HOME/rick\" %U\n"),
            "{text}"
        );
        let home = Home::new("GNOME");
        let exec = ini_value(&entry_text(&home.env.exe), "Desktop Entry", "Exec")
            .unwrap_or_else(|| panic!("an Exec line"));
        assert!(home.env.launches(&exec, &home.env.exe), "{exec}");
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
