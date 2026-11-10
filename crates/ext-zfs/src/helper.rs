//! `rick-zfs`, the privileged helper: the one program the plugin runs as
//! the administrator, through the platform's prompt, when ZFS refuses a
//! snapshot change for lack of permission. Rick itself never runs as root.
//!
//! The helper takes the `zfs` arguments of exactly one change and checks
//! them against the three forms the plugin sends before it runs anything:
//!
//! - `snapshot [-r] <dataset>@<name>`
//! - `rename <dataset>@<old> <dataset>@<new>`, both of one dataset
//! - `destroy <dataset>@<name>`
//! - `set <property>=<value>… <dataset>` and `inherit <property> <dataset>`
//!   for the properties of the editor ([`crate::edit::EDITABLE`]), each
//!   value checked as the editor checks it
//!
//! Every snapshot command needs a snapshot's full name (`@` present), so
//! the helper can never destroy or rename a dataset; the property commands
//! take a dataset without `@`. Nothing may start with `-`, so no further
//! option reaches `zfs`. It runs `zfs` from the system's own
//! directories, never from the caller's `PATH`, in the C locale, with
//! its output passed through and its exit code returned.
//!
//! Under `pkexec`, the polkit action [`POLKIT_ACTION`] applies when the
//! policy file `org.openbreach.rick.zfs.policy` is installed system-wide
//! and names the helper's path; otherwise `pkexec` asks under its generic
//! action, as for `rick-perms` (QUESTIONS.md 366).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::parse::check_name;

/// The helper's file name.
pub const HELPER_NAME: &str = "rick-zfs";

/// The environment variable that names the helper, for packagers who keep
/// it elsewhere than beside the binary.
pub const HELPER_ENV: &str = "RICK_ZFS_HELPER";

/// The polkit action the helper runs under where its policy is installed.
pub const POLKIT_ACTION: &str = "org.openbreach.rick.zfs";

/// Where `zfs` may be, in this order. The caller's `PATH` is not used.
const ZFS_PATHS: [&str; 4] = ["/usr/sbin/zfs", "/sbin/zfs", "/usr/bin/zfs", "/bin/zfs"];

/// Exit code for arguments the helper refuses, as `sysexits.h`'s
/// `EX_USAGE`.
pub const EXIT_REFUSED: i32 = 64;

/// Exit code when `zfs` is not found or does not start.
pub const EXIT_NO_ZFS: i32 = 69;

/// Why the helper refused its arguments.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// Not one of `snapshot`, `rename` and `destroy`, or the wrong number
    /// of arguments for it.
    #[error(
        "rick-zfs runs only: snapshot [-r] DATASET@NAME, rename DATASET@OLD DATASET@NEW, destroy DATASET@NAME, set PROPERTY=VALUE… DATASET, inherit PROPERTY DATASET"
    )]
    Usage,
    /// An argument that is not a snapshot's full name.
    #[error("not a snapshot name: {0}")]
    NotSnapshot(String),
    /// A rename between two datasets.
    #[error("a snapshot is renamed within its dataset only")]
    OtherDataset,
    /// Not a dataset's name.
    #[error("not a dataset name: {0}")]
    NotDataset(String),
    /// A property the editor does not offer, or a value it would not send.
    #[error("not an editable property or value: {0}")]
    Property(String),
}

/// Checks `args` against the three forms the helper runs.
///
/// # Errors
///
/// The [`Refusal`] that names what is wrong.
pub fn check(args: &[String]) -> Result<(), Refusal> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["snapshot", "-r", full] | ["snapshot" | "destroy", full] => snapshot(full).map(|_| ()),
        ["inherit", property, dataset] => {
            self::dataset(dataset)?;
            if crate::edit::EDITABLE.contains(property) {
                Ok(())
            } else {
                Err(Refusal::Property((*property).to_owned()))
            }
        }
        ["set", assignments @ .., dataset] if !assignments.is_empty() => {
            self::dataset(dataset)?;
            for assignment in assignments {
                let ok = assignment.split_once('=').is_some_and(|(p, v)| {
                    crate::edit::EDITABLE.contains(&p) && crate::edit::value_ok(p, v)
                });
                if !ok {
                    return Err(Refusal::Property((*assignment).to_owned()));
                }
            }
            Ok(())
        }
        ["rename", old, new] => {
            let (from, _) = snapshot(old)?;
            let (to, _) = snapshot(new)?;
            if from == to {
                Ok(())
            } else {
                Err(Refusal::OtherDataset)
            }
        }
        _ => Err(Refusal::Usage),
    }
}

/// Splits a snapshot's full name into dataset and name, checking both.
fn snapshot(full: &str) -> Result<(&str, &str), Refusal> {
    let bad = || Refusal::NotSnapshot(full.to_owned());
    let (dataset, name) = full.split_once('@').ok_or_else(bad)?;
    if !dataset_ok(dataset) || name.contains('@') || check_name(dataset, name).is_err() {
        return Err(bad());
    }
    Ok((dataset, name))
}

/// Checks a dataset's name for the property commands.
fn dataset(name: &str) -> Result<(), Refusal> {
    if dataset_ok(name) {
        Ok(())
    } else {
        Err(Refusal::NotDataset(name.to_owned()))
    }
}

/// Whether `dataset` is a dataset name: components of letters, digits and
/// `_ - . :`, separated by `/`, the first not starting with `-`.
fn dataset_ok(dataset: &str) -> bool {
    !dataset.starts_with('-')
        && dataset.split('/').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
        })
}

/// The first `zfs` of the system's directories.
#[must_use]
pub fn system_zfs() -> Option<PathBuf> {
    ZFS_PATHS.iter().map(PathBuf::from).find(|p| p.is_file())
}

/// The helper's entry point: checks `args`, runs `zfs` with them and
/// returns the exit code for the process. Refused arguments and a missing
/// `zfs` are reported on standard error.
#[must_use]
pub fn main(args: &[String]) -> i32 {
    if let Err(refusal) = check(args) {
        say(&refusal.to_string());
        return EXIT_REFUSED;
    }
    let Some(zfs) = system_zfs() else {
        say("zfs is not installed in /usr/sbin, /sbin, /usr/bin or /bin");
        return EXIT_NO_ZFS;
    };
    run(&zfs, args)
}

/// Runs `zfs` with `args`, its output passed through.
fn run(zfs: &Path, args: &[String]) -> i32 {
    match Command::new(zfs)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            say(&format!("{}: {error}", zfs.display()));
            EXIT_NO_ZFS
        }
    }
}

/// Writes one line to standard error, where `zfs` writes its messages.
fn say(text: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "rick-zfs: {text}");
}

/// Where the helper is: [`HELPER_ENV`] when set, else `rick-zfs` beside
/// the running binary, or one directory up from it, where a test binary
/// in `target/<profile>/deps` finds the one `cargo` built. `None` when
/// there is none, and then a refused change is not offered elevated.
#[must_use]
pub fn path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(HELPER_ENV).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path)).filter(|p| p.is_file());
    }
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let beside = dir.join(HELPER_NAME);
    if beside.is_file() {
        return Some(beside);
    }
    dir.parent()
        .map(|up| up.join(HELPER_NAME))
        .filter(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn the_three_forms_pass() {
        for ok in [
            &["snapshot", "tank/home@rick-2026-11-10-120000"][..],
            &["snapshot", "-r", "tank/home@weekly"],
            &["rename", "tank/home@a", "tank/home@b c"],
            &["destroy", "rpool/rick-test@eins"],
            &["destroy", "tank@x"],
        ] {
            assert_eq!(check(&args(ok)), Ok(()), "{ok:?}");
        }
    }

    #[test]
    fn a_dataset_without_a_snapshot_is_refused() {
        for bad in [
            &["destroy", "tank/home"][..],
            &["destroy", "tank/home@"],
            &["destroy", "@x"],
            &["snapshot", "tank/home"],
            &["rename", "tank/home@a", "tank/other"],
            &["destroy", "tank/home@a@b"],
        ] {
            assert!(
                matches!(check(&args(bad)), Err(Refusal::NotSnapshot(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn options_and_other_commands_are_refused() {
        for bad in [
            &[][..],
            &["destroy"],
            &["destroy", "-r", "tank/home@a"],
            &["destroy", "-R", "tank/home@a"],
            &["destroy", "tank/home@a", "tank/home@b"],
            &["snapshot", "-o", "x=y", "tank/home@a"],
            &["rollback", "tank/home@a"],
            &["list"],
        ] {
            assert_eq!(check(&args(bad)), Err(Refusal::Usage), "{bad:?}");
        }
        for bad in [
            &["destroy", "-tank/home@a"][..],
            &["destroy", "tank//home@a"],
            &["destroy", "tank/home/@a"],
            &["destroy", "tank/ho;me@a"],
            &["snapshot", "tank/home@a/b"],
        ] {
            assert!(
                matches!(check(&args(bad)), Err(Refusal::NotSnapshot(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_property_commands_pass_for_the_editable_properties() {
        for ok in [
            &["set", "compression=zstd", "tank/home"][..],
            &[
                "set",
                "compression=zstd-3",
                "quota=53687091200",
                "atime=off",
                "tank/home",
            ],
            &["set", "quota=none", "refquota=50G", "tank/home"],
            &["set", "recordsize=1M", "copies=2", "tank"],
            &["inherit", "atime", "tank/home"],
        ] {
            assert_eq!(check(&args(ok)), Ok(()), "{ok:?}");
        }
    }

    #[test]
    fn other_properties_and_values_are_refused() {
        for bad in [
            &["set", "mountpoint=/x", "tank/home"][..],
            &["set", "readonly=on", "tank/home"],
            &["set", "compression=zstd", "dedup=on", "tank/home"],
            &["set", "copies=4", "tank/home"],
            &["set", "atime=maybe", "tank/home"],
            &["set", "quota=fifty", "tank/home"],
            &["set", "compression=-o", "tank/home"],
            &["set", "compression", "tank/home"],
            &["inherit", "mountpoint", "tank/home"],
        ] {
            assert!(
                matches!(check(&args(bad)), Err(Refusal::Property(_))),
                "{bad:?}"
            );
        }
        for bad in [
            &["set", "compression=zstd", "tank/home@a"][..],
            &["inherit", "atime", "-r"],
            &["set", "atime=on", "tank//home"],
        ] {
            assert!(
                matches!(check(&args(bad)), Err(Refusal::NotDataset(_))),
                "{bad:?}"
            );
        }
        assert_eq!(check(&args(&["set", "tank/home"])), Err(Refusal::Usage));
        assert_eq!(
            check(&args(&["inherit", "-r", "atime", "tank/home"])),
            Err(Refusal::Usage)
        );
    }

    #[test]
    fn a_rename_stays_in_its_dataset() {
        assert_eq!(
            check(&args(&["rename", "tank/home@a", "tank/other@a"])),
            Err(Refusal::OtherDataset)
        );
    }

    #[test]
    fn refused_arguments_never_reach_zfs() {
        assert_eq!(main(&args(&["destroy", "tank/home"])), EXIT_REFUSED);
        assert_eq!(main(&args(&["rollback", "tank/home@a"])), EXIT_REFUSED);
    }
}
