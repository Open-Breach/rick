//! *"Show in folder" from other apps* (specs/system-integration.md §7.1):
//! the activation file of the `org.freedesktop.FileManager1` D-Bus
//! service in the scope's `share/dbus-1/services`, which starts Rick when
//! another application calls the service and no Rick is running. The
//! running instance owns the name; that half is not a file and not this
//! module's.

use std::fs;
use std::path::{Path, PathBuf};

use super::{LinuxEnv, ini_value, is_foreign};
use crate::integration::{Actual, Check, Receipt};

/// The well-known bus name of the freedesktop file-manager interface.
pub(super) const BUS_NAME: &str = "org.freedesktop.FileManager1";

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
}
