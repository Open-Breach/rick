//! *"Show in folder" from other apps* (specs/system-integration.md §7.1):
//! the activation file of the `org.freedesktop.FileManager1` D-Bus
//! service in the scope's `share/dbus-1/services`, which starts Rick when
//! another application calls the service and no Rick is running. The
//! running instance owns the name; that half is not a file and not this
//! module's.

use std::fs;
use std::path::{Path, PathBuf};

use types::Error;

use super::{LinuxEnv, ini_value, is_foreign, refuse_foreign, remove, run_tool, write_recorded};
use crate::file_manager::{BUS_NAME, SERVICE_ARGUMENT};
use crate::integration::{Actual, Check, Receipt, Request};

/// Where the activation file goes:
/// `<data home>/dbus-1/services/org.freedesktop.FileManager1.service`.
pub(super) fn service_path(env: &LinuxEnv) -> PathBuf {
    env.data_home
        .join("dbus-1/services")
        .join(format!("{BUS_NAME}.service"))
}

/// The service is set up when the activation file names the bus name and
/// starts `binary`. A file that starts something else is in the way
/// unless the receipt says Rick wrote it.
pub(super) fn check(env: &LinuxEnv, binary: &Path, receipt: &Receipt) -> Check {
    let file = service_path(env);
    let mut check = Check {
        actual: Actual::Missing,
        place: Some(file.clone()),
        ..Check::default()
    };
    let ours = fs::read_to_string(&file).is_ok_and(|text| {
        ini_value(&text, "D-BUS Service", "Name").as_deref() == Some(BUS_NAME)
            && ini_value(&text, "D-BUS Service", "Exec")
                .is_some_and(|exec| env.launches(&exec, binary))
    });
    if ours {
        check.actual = Actual::SetUp;
    } else if is_foreign(env, &file, receipt) {
        check.foreign.push(file);
    }
    check
}

/// A path as one word of a service file's `Exec`, which the bus splits
/// like a shell: bare when it is plain, otherwise in double quotes with
/// `"`, `\`, `$` and `` ` `` escaped.
fn exec_word(path: &Path) -> String {
    let text = path.to_string_lossy();
    let plain = text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | '+' | ':' | '@'));
    if plain && !text.is_empty() {
        return text.into_owned();
    }
    let mut out = String::from("\"");
    for c in text.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The activation file for `binary`.
pub(super) fn service_text(binary: &Path) -> String {
    format!(
        "[D-BUS Service]\nName={BUS_NAME}\nExec={} {SERVICE_ARGUMENT}\n",
        exec_word(binary)
    )
}

/// Asks the session bus to read its service files again, so a file in a
/// directory that was not there when the bus started is found without a
/// new login. Only with a bus and `dbus-send`; a failure is logged.
fn reload_bus(env: &LinuxEnv) {
    let Some(address) = &env.session_bus else {
        return;
    };
    let bus = format!("--bus={address}");
    run_tool(
        env,
        "dbus-send",
        &[
            bus.as_ref(),
            "--dest=org.freedesktop.DBus".as_ref(),
            "--type=method_call".as_ref(),
            "/org/freedesktop/DBus".as_ref(),
            "org.freedesktop.DBus.ReloadConfig".as_ref(),
        ],
    );
}

/// Writes the activation file; recorded when it was written.
pub(super) fn perform(env: &LinuxEnv, binary: &Path, request: &Request) -> Result<Receipt, Error> {
    refuse_foreign(&check(env, binary, &request.receipt), request)?;
    let mut receipt = request.receipt.clone();
    write_recorded(
        env,
        &service_path(env),
        service_text(binary).as_bytes(),
        &mut receipt,
    )?;
    reload_bus(env);
    Ok(receipt)
}

/// Removes the activation file the receipt lists.
pub(super) fn undo(env: &LinuxEnv, receipt: &Receipt) -> Result<(), Error> {
    for file in &receipt.files {
        remove(&env.expand(file))?;
    }
    reload_bus(env);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{Home, write};
    use super::*;
    use crate::integration::{Integration as _, Item, Request};

    #[test]
    fn the_activation_file_that_starts_this_rick_is_the_service() {
        let home = Home::new("GNOME");
        let integration = home.integration();
        let request = Request::default();
        let found = integration.check_integration(Item::ShowInFolder, &request);
        assert_eq!(found.actual, Actual::Missing);
        let file = service_path(&home.env);
        assert_eq!(found.place, Some(file.clone()));
        assert!(
            file.ends_with(".local/share/dbus-1/services/org.freedesktop.FileManager1.service")
        );

        write(
            &file,
            &format!(
                "[D-BUS Service]\nName=org.freedesktop.FileManager1\nExec={} --dbus-service\n",
                home.env.exe.display()
            ),
        );
        let found = integration.check_integration(Item::ShowInFolder, &request);
        assert_eq!(found.actual, Actual::SetUp);

        // Another file manager's file under the same name is in the way.
        write(
            &file,
            "[D-BUS Service]\nName=org.freedesktop.FileManager1\nExec=/usr/bin/nautilus --gapplication-service\n",
        );
        let found = integration.check_integration(Item::ShowInFolder, &request);
        assert_eq!(found.actual, Actual::Missing);
        assert_eq!(found.foreign, [file]);
    }

    #[test]
    fn performing_writes_the_activation_file_and_undo_removes_it() {
        let home = Home::new("GNOME");
        let integration = home.integration();
        let receipt = integration
            .perform_integration(Item::ShowInFolder, &Request::default())
            .unwrap_or_else(|e| panic!("{e}"));
        let file = service_path(&home.env);
        assert_eq!(
            fs::read_to_string(&file).unwrap_or_default(),
            format!(
                "[D-BUS Service]\nName=org.freedesktop.FileManager1\nExec={} --dbus-service\n",
                home.env.exe.display()
            )
        );
        assert_eq!(receipt.files, [home.env.tilde(&file)]);
        let request = Request {
            receipt,
            ..Request::default()
        };
        assert_eq!(
            integration
                .check_integration(Item::ShowInFolder, &request)
                .actual,
            Actual::SetUp
        );
        integration
            .undo_integration(Item::ShowInFolder, &request)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!file.exists());
    }

    #[test]
    fn another_file_managers_file_is_replaced_only_on_request() {
        let home = Home::new("GNOME");
        let integration = home.integration();
        let file = service_path(&home.env);
        let other = "[D-BUS Service]\nName=org.freedesktop.FileManager1\nExec=/usr/bin/nautilus --gapplication-service\n";
        write(&file, other);
        let refused = integration.perform_integration(Item::ShowInFolder, &Request::default());
        assert!(
            matches!(&refused, Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists),
            "{refused:?}"
        );
        assert_eq!(fs::read_to_string(&file).unwrap_or_default(), other);
        let replaced = integration
            .perform_integration(
                Item::ShowInFolder,
                &Request {
                    replace: true,
                    ..Request::default()
                },
            )
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(replaced.files, [home.env.tilde(&file)]);
    }

    #[test]
    fn a_path_with_spaces_is_one_quoted_word_the_check_reads_back() {
        assert_eq!(
            exec_word(Path::new("/opt/rick/current/rick")),
            "/opt/rick/current/rick"
        );
        assert_eq!(
            exec_word(Path::new("/home/a b/$x/rick")),
            "\"/home/a b/\\$x/rick\""
        );
        let home = Home::new("GNOME");
        let odd = home.dir.path().join("with space");
        fs::create_dir_all(&odd).unwrap_or_else(|e| panic!("{e}"));
        let binary = odd.join("rick");
        fs::write(&binary, b"x").unwrap_or_else(|e| panic!("{e}"));
        let exec = ini_value(&service_text(&binary), "D-BUS Service", "Exec")
            .unwrap_or_else(|| panic!("an Exec line"));
        assert!(home.env.launches(&exec, &binary), "{exec}");
    }
}
