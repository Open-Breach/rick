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
//!
//! Every name must be a snapshot (`@` present), so the helper can never
//! destroy or rename a dataset, and nothing may start with `-`, so no
//! further option reaches `zfs`. It runs `zfs` from the system's own
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
        "rick-zfs runs only: snapshot [-r] DATASET@NAME, rename DATASET@OLD DATASET@NEW, destroy DATASET@NAME"
    )]
    Usage,
    /// An argument that is not a snapshot's full name.
    #[error("not a snapshot name: {0}")]
    NotSnapshot(String),
    /// A rename between two datasets.
    #[error("a snapshot is renamed within its dataset only")]
    OtherDataset,
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
            &["set", "compression=zstd", "tank/home"],
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
