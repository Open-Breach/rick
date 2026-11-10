//! Reading what `zfs list -H -p` prints, and the rules for snapshot names.
//!
//! `-H` prints one record per line with tab-separated fields and no
//! header; `-p` prints numbers as plain integers (bytes, seconds since the
//! epoch). A line that does not read is skipped and logged, never a panic.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use chrono::NaiveDateTime;

/// A mounted ZFS file system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSystem {
    /// The dataset's name, `tank/home`.
    pub name: String,
    /// Where it is mounted.
    pub mountpoint: PathBuf,
}

/// One snapshot of a dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The dataset's name, `tank/home`.
    pub dataset: String,
    /// The snapshot's own name, the part after `@`.
    pub name: String,
    /// When it was taken.
    pub created: SystemTime,
    /// The space only it holds, in bytes.
    pub used: u64,
}

impl Snapshot {
    /// `dataset@name`, as `zfs` takes it.
    #[must_use]
    pub fn full_name(&self) -> String {
        format!("{}@{}", self.dataset, self.name)
    }
}

/// The arguments that list the file systems.
#[must_use]
pub fn list_filesystems_args() -> Vec<String> {
    strings(&[
        "list",
        "-H",
        "-p",
        "-o",
        "name,mountpoint,mounted",
        "-t",
        "filesystem",
    ])
}

/// The arguments that list the snapshots of `dataset`, oldest first.
#[must_use]
pub fn list_snapshots_args(dataset: &str) -> Vec<String> {
    let mut args = strings(&[
        "list",
        "-H",
        "-p",
        "-t",
        "snapshot",
        "-o",
        "name,creation,used",
        "-s",
        "creation",
        "-d",
        "1",
    ]);
    args.push(dataset.to_owned());
    args
}

pub(crate) fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_owned()).collect()
}

/// The mounted file systems in `zfs list -o name,mountpoint,mounted`
/// output. Those mounted nowhere (`none`, `legacy`, a relative path) or not
/// mounted now are left out.
#[must_use]
pub fn filesystems(output: &str) -> Vec<FileSystem> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let (Some(name), Some(mountpoint), Some(mounted), None) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                tracing::debug!(line, "zfs list: malformed file-system line skipped");
                return None;
            };
            let mountpoint = PathBuf::from(mountpoint);
            (!name.is_empty() && mounted == "yes" && mountpoint.is_absolute()).then(|| FileSystem {
                name: name.to_owned(),
                mountpoint,
            })
        })
        .collect()
}

/// The snapshots in `zfs list -t snapshot -o name,creation,used -p`
/// output, in the order printed.
#[must_use]
pub fn snapshots(output: &str) -> Vec<Snapshot> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let snapshot = snapshot_line(line);
            if snapshot.is_none() {
                tracing::debug!(line, "zfs list: malformed snapshot line skipped");
            }
            snapshot
        })
        .collect()
}

fn snapshot_line(line: &str) -> Option<Snapshot> {
    let mut fields = line.split('\t');
    let (Some(full), Some(created), Some(used), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return None;
    };
    let (dataset, name) = full.split_once('@')?;
    if dataset.is_empty() || name.is_empty() {
        return None;
    }
    let created = SystemTime::UNIX_EPOCH + Duration::from_secs(created.parse().ok()?);
    Some(Snapshot {
        dataset: dataset.to_owned(),
        name: name.to_owned(),
        created,
        used: used.parse().ok()?,
    })
}

/// The longest `dataset@name` ZFS takes, in bytes.
pub const MAX_FULL_NAME: usize = 255;

/// Why a snapshot name cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameProblem {
    /// Nothing was entered.
    Empty,
    /// A character ZFS does not allow in snapshot names.
    Character(char),
    /// `dataset@name` is longer than [`MAX_FULL_NAME`] bytes.
    TooLong,
}

impl NameProblem {
    /// The reason, in the language in use.
    #[must_use]
    pub fn text(self) -> String {
        match self {
            Self::Empty => ext::t!("zfs-name-empty"),
            Self::Character(c) => ext::t!("zfs-name-character", character = c.to_string()),
            Self::TooLong => ext::t!("zfs-name-too-long"),
        }
    }
}

/// Checks a snapshot name for `dataset`: ASCII letters, digits, `_ - . :`
/// and space, and `dataset@name` at most [`MAX_FULL_NAME`] bytes.
///
/// # Errors
///
/// The first problem found.
pub fn check_name(dataset: &str, name: &str) -> Result<(), NameProblem> {
    if name.is_empty() {
        return Err(NameProblem::Empty);
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | ' ')))
    {
        return Err(NameProblem::Character(bad));
    }
    if dataset.len() + 1 + name.len() > MAX_FULL_NAME {
        return Err(NameProblem::TooLong);
    }
    Ok(())
}

/// The name a new snapshot gets unless the user types another:
/// `rick-YYYY-MM-DD-HHMMSS` of `at`, which is local time.
#[must_use]
pub fn default_name_at(at: NaiveDateTime) -> String {
    at.format("rick-%Y-%m-%d-%H%M%S").to_string()
}

/// [`default_name_at`] now, in local time.
#[must_use]
pub fn default_name() -> String {
    default_name_at(chrono::Local::now().naive_local())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `zfs list -H -p -o name,mountpoint,mounted -t filesystem` on the
    /// development machine (`OpenZFS` 2.4.1, root on ZFS), shortened, with a
    /// dataset whose mount point has a space added.
    const FILESYSTEMS: &str = "bpool\t/boot\tno
bpool/BOOT\tnone\tno
bpool/BOOT/ubuntu_62wyjh\t/boot\tyes
rpool\t/\tno
rpool/ROOT\tnone\tno
rpool/ROOT/ubuntu_62wyjh\t/\tyes
rpool/ROOT/ubuntu_62wyjh/usr\t/usr\tno
rpool/ROOT/ubuntu_62wyjh/usr/local\t/usr/local\tyes
rpool/ROOT/ubuntu_62wyjh/var/lib\t/var/lib\tyes
rpool/USERDATA\tnone\tno
rpool/USERDATA/home_wyt8xu\t/home\tyes
rpool/legacy\tlegacy\tyes
rpool/USERDATA/My Files\t/srv/my files\tyes
";

    /// `zfs list -H -p -t snapshot -o name,creation,used -s creation -d 1`
    /// on the development machine, with a second snapshot and a name with
    /// a space added.
    const SNAPSHOTS: &str =
        "rpool/ROOT/ubuntu_62wyjh/var/lib@pre-26.04-upgrade\t1790877375\t11363057664
rpool/ROOT/ubuntu_62wyjh/var/lib@before update\t1790963775\t81920
";

    #[test]
    fn mounted_filesystems_are_read_and_the_rest_skipped() {
        let read = filesystems(FILESYSTEMS);
        let names: Vec<&str> = read.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "bpool/BOOT/ubuntu_62wyjh",
                "rpool/ROOT/ubuntu_62wyjh",
                "rpool/ROOT/ubuntu_62wyjh/usr/local",
                "rpool/ROOT/ubuntu_62wyjh/var/lib",
                "rpool/USERDATA/home_wyt8xu",
                "rpool/USERDATA/My Files",
            ]
        );
        assert_eq!(read[5].mountpoint, PathBuf::from("/srv/my files"));
    }

    #[test]
    fn snapshots_are_read_with_their_date_and_space() {
        let read = snapshots(SNAPSHOTS);
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].dataset, "rpool/ROOT/ubuntu_62wyjh/var/lib");
        assert_eq!(read[0].name, "pre-26.04-upgrade");
        assert_eq!(
            read[0].created,
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_877_375)
        );
        assert_eq!(read[0].used, 11_363_057_664);
        assert_eq!(read[1].name, "before update");
        assert_eq!(
            read[1].full_name(),
            "rpool/ROOT/ubuntu_62wyjh/var/lib@before update"
        );
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let fs = filesystems("tank\t/tank\n\t/x\tyes\ntank/a\t/a\tyes\textra\nok\t/ok\tyes\n");
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].name, "ok");
        let snaps = snapshots(
            "tank\t1\t2\ntank@\t1\t2\n@a\t1\t2\ntank@a\tsoon\t2\ntank@b\t1\t-\ntank@c\t5\t6\n",
        );
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].name, "c");
        assert_eq!(snapshots(""), []);
        assert_eq!(filesystems("garbage without tabs"), []);
    }

    #[test]
    fn names_are_checked() {
        assert_eq!(check_name("tank", "rick-2026-11-09-143005"), Ok(()));
        assert_eq!(check_name("tank", "before upgrade_1.0:a"), Ok(()));
        assert_eq!(check_name("tank", ""), Err(NameProblem::Empty));
        assert_eq!(check_name("tank", "a/b"), Err(NameProblem::Character('/')));
        assert_eq!(check_name("tank", "a@b"), Err(NameProblem::Character('@')));
        assert_eq!(check_name("tank", "ä"), Err(NameProblem::Character('ä')));
        let long = "a".repeat(MAX_FULL_NAME - "tank@".len());
        assert_eq!(check_name("tank", &long), Ok(()));
        assert_eq!(
            check_name("tank", &format!("{long}a")),
            Err(NameProblem::TooLong)
        );
    }

    #[test]
    fn the_default_name_is_a_timestamp() {
        let at = chrono::NaiveDate::from_ymd_opt(2026, 11, 9)
            .and_then(|d| d.and_hms_opt(14, 30, 5))
            .unwrap_or_default();
        assert_eq!(default_name_at(at), "rick-2026-11-09-143005");
        let now = default_name();
        assert!(now.starts_with("rick-20"), "{now}");
        assert_eq!(now.len(), "rick-2026-11-09-143005".len());
        assert_eq!(check_name("tank", &now), Ok(()));
    }
}
