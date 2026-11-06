//! Linux: the XDG trash specification, `xdg-open`, Rick's own properties
//! rows, `$XDG_CONFIG_HOME/rick`.

mod trash;

use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use types::Error;

use crate::Native;
use crate::time::iso8601;
use crate::traits::{
    Chrome, ConfigDir, ControlsSide, OpenWith, Properties, PropertiesOutcome, Trash, TrashedItem,
    WindowChrome,
};

impl Trash for Native {
    fn trash(&self, path: &Path) -> Result<TrashedItem, Error> {
        trash::trash(path)
    }

    fn restore(&self, item: &TrashedItem) -> Result<(), Error> {
        trash::restore(item)
    }

    fn list(&self) -> Result<Vec<TrashedItem>, Error> {
        trash::list()
    }
}

fn spawn_detached(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), Error> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(Error::from_io)
}

impl OpenWith for Native {
    fn open(&self, path: &Path) -> Result<(), Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        spawn_detached("xdg-open", &[path.as_os_str()])
    }

    fn reveal(&self, path: &Path) -> Result<(), Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        // The freedesktop file-manager interface selects the item; fall back
        // to opening the parent folder where no D-Bus session is reachable.
        let uri = format!(
            "array:string:file://{}",
            percent_encode(path.as_os_str().as_encoded_bytes())
        );
        let shown = Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &uri,
                "string:",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if shown {
            return Ok(());
        }
        let parent = path.parent().unwrap_or(&path);
        spawn_detached("xdg-open", &[parent.as_os_str()])
    }
}

/// Percent-encodes everything but the unreserved characters and `/`.
pub(crate) fn percent_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

impl Properties for Native {
    fn show(&self, path: &Path) -> Result<PropertiesOutcome, Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        let meta = fs::symlink_metadata(&path).map_err(Error::from_io)?;
        let kind = if meta.is_dir() {
            "Folder"
        } else if meta.file_type().is_symlink() {
            "Symbolic link"
        } else if meta.is_file() {
            "File"
        } else {
            "Special file"
        };
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let location = path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let mode = meta.permissions().mode();
        let mut rows = vec![
            ("Name".to_owned(), name),
            ("Type".to_owned(), kind.to_owned()),
            ("Location".to_owned(), location),
            ("Size".to_owned(), format!("{} bytes", meta.len())),
            (
                "On disk".to_owned(),
                format!("{} bytes", meta.blocks() * 512),
            ),
            (
                "Modified".to_owned(),
                meta.modified().map(iso8601).unwrap_or_default(),
            ),
            (
                "Accessed".to_owned(),
                meta.accessed().map(iso8601).unwrap_or_default(),
            ),
        ];
        if let Ok(created) = meta.created() {
            rows.push(("Created".to_owned(), iso8601(created)));
        }
        rows.push(("Owner".to_owned(), name_of(meta.uid(), "/etc/passwd")));
        rows.push(("Group".to_owned(), name_of(meta.gid(), "/etc/group")));
        rows.push((
            "Permissions".to_owned(),
            format!("{} ({:04o})", rwx(mode), mode & 0o7777),
        ));
        if meta.file_type().is_symlink()
            && let Ok(target) = fs::read_link(&path)
        {
            rows.push(("Target".to_owned(), target.display().to_string()));
        }
        Ok(PropertiesOutcome::Rows(rows))
    }
}

/// `uid`/`gid` as `name (id)` from a passwd-style file, or just the id.
fn name_of(id: u32, database: &str) -> String {
    fs::read_to_string(database)
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                let mut fields = line.split(':');
                let name = fields.next()?;
                let found: u32 = fields.nth(1)?.parse().ok()?;
                (found == id).then(|| format!("{name} ({id})"))
            })
        })
        .unwrap_or_else(|| id.to_string())
}

fn rwx(mode: u32) -> String {
    let mut out = String::with_capacity(9);
    for shift in [6u32, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        out.push(if bits & 4 != 0 { 'r' } else { '-' });
        out.push(if bits & 2 != 0 { 'w' } else { '-' });
        out.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    out
}

impl ConfigDir for Native {
    fn config_dir(&self) -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| PathBuf::from(".config"))
            .join("rick")
    }

    fn home_dir(&self) -> PathBuf {
        std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map_or_else(|| PathBuf::from("."), PathBuf::from)
    }
}

impl Chrome for Native {
    fn window_chrome(&self) -> WindowChrome {
        WindowChrome {
            controls: ControlsSide::Right,
            native_controls: false,
            native_controls_width: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_rows_describe_a_file() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let file = dir.path().join("x.txt");
        fs::write(&file, b"12345").unwrap_or_else(|e| panic!("{e}"));
        let rows = match Native.show(&file) {
            Ok(PropertiesOutcome::Rows(rows)) => rows,
            other => panic!("{other:?}"),
        };
        let get = |label: &str| {
            rows.iter()
                .find(|(l, _)| l == label)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("Name"), Some("x.txt".to_owned()));
        assert_eq!(get("Type"), Some("File".to_owned()));
        assert_eq!(get("Size"), Some("5 bytes".to_owned()));
        assert!(get("Permissions").is_some_and(|p| p.len() > 9));
        assert!(matches!(
            Native.show(&dir.path().join("missing")),
            Err(Error::NotFound)
        ));
    }

    #[test]
    fn config_dir_follows_xdg() {
        let dir = Native.config_dir();
        assert!(dir.ends_with("rick"), "{dir:?}");
    }

    #[test]
    fn percent_encoding_leaves_safe_bytes_alone() {
        assert_eq!(percent_encode(b"/tmp/a b%.txt"), "/tmp/a%20b%25.txt");
        assert_eq!(rwx(0o754), "rwxr-xr--");
    }
}
