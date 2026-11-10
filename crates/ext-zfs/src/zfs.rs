//! The seam to ZFS: the [`Zfs`] runner that executes the `zfs` command
//! line, its system implementation [`SystemZfs`], the scripted
//! [`ScriptedZfs`] the tests use, and the classes a failure falls into.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock, PoisonError};

use types::Error;

/// What one run of `zfs` produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// The exit code, `None` when the process was killed by a signal.
    pub code: Option<i32>,
    /// Standard output, lossily decoded.
    pub stdout: String,
    /// Standard error, lossily decoded.
    pub stderr: String,
}

impl Output {
    /// A successful run that printed `stdout`.
    #[must_use]
    pub fn ok(stdout: impl Into<String>) -> Self {
        Self {
            code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    /// A failed run with exit code `code` that printed `stderr`.
    #[must_use]
    pub fn failed(code: i32, stderr: impl Into<String>) -> Self {
        Self {
            code: Some(code),
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    /// Whether `zfs` exited with code 0.
    #[must_use]
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Runs the `zfs` command line.
///
/// Every call ZFS answers goes through one of these, so the tests replace
/// the system's `zfs` with [`ScriptedZfs`].
pub trait Zfs: Send + Sync + fmt::Debug {
    /// Runs `zfs args…` and waits for it.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when there is no `zfs` command; the I/O error
    /// when it cannot be started. A run that exits non-zero is an `Ok`
    /// [`Output`].
    fn run(&self, args: &[String]) -> Result<Output, Error>;

    /// The absolute path of the `zfs` command, what an elevated run starts;
    /// `None` when there is none.
    fn program(&self) -> Option<PathBuf>;
}

/// Where a desktop session's `PATH` may not reach but distributions put
/// `zfs`.
const SBIN: [&str; 2] = ["/usr/sbin", "/sbin"];

/// The system's `zfs`, found on `PATH` (then in `/usr/sbin` and `/sbin`)
/// once and remembered.
#[derive(Debug, Default)]
pub struct SystemZfs {
    path: OnceLock<Option<PathBuf>>,
}

impl SystemZfs {
    /// The runner; nothing is looked up before the first call.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn find() -> Option<PathBuf> {
        let path = std::env::var_os("PATH").unwrap_or_default();
        std::env::split_paths(&path)
            .chain(SBIN.iter().map(PathBuf::from))
            .map(|dir| dir.join("zfs"))
            .find(|candidate| candidate.is_absolute() && candidate.is_file())
    }
}

impl Zfs for SystemZfs {
    fn run(&self, args: &[String]) -> Result<Output, Error> {
        let program = self
            .program()
            .ok_or(Error::Unsupported("the zfs command is not installed"))?;
        // `LC_ALL=C`: the error texts `classify` reads are the English
        // ones.
        let output = Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .output()
            .map_err(Error::from_io)?;
        Ok(Output {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn program(&self) -> Option<PathBuf> {
        self.path.get_or_init(Self::find).clone()
    }
}

/// One scripted answer: the arguments it answers and what it answers.
type Reply = (Vec<String>, Result<Output, &'static str>);

/// A `zfs` that replays recorded answers, for tests.
///
/// An answer added with [`ScriptedZfs::on`] is given every time its exact
/// arguments come; one added with [`ScriptedZfs::once`] is given once, ahead
/// of the lasting ones, and then forgotten. Arguments nobody scripted are
/// answered as `zfs` answers an unknown dataset. Every call is recorded.
#[derive(Debug, Default)]
pub struct ScriptedZfs {
    missing: bool,
    lasting: Mutex<Vec<Reply>>,
    once: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<Vec<String>>>,
}

fn args_of(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}

impl ScriptedZfs {
    /// A runner with nothing scripted yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A machine without the `zfs` command: every call is
    /// `Error::Unsupported`.
    #[must_use]
    pub fn missing() -> Self {
        Self {
            missing: true,
            ..Self::default()
        }
    }

    /// Answers `zfs args…` with `output` every time it comes.
    pub fn on(&self, args: &[&str], output: Output) -> &Self {
        lock(&self.lasting).push((args_of(args), Ok(output)));
        self
    }

    /// Answers the next `zfs args…` with `output`, once.
    pub fn once(&self, args: &[&str], output: Output) -> &Self {
        lock(&self.once).push_back((args_of(args), Ok(output)));
        self
    }

    /// Answers `zfs args…` with an I/O error every time it comes.
    pub fn fail_to_start(&self, args: &[&str]) -> &Self {
        lock(&self.lasting).push((args_of(args), Err("zfs could not be started")));
        self
    }

    /// Every call so far, oldest first.
    #[must_use]
    pub fn calls(&self) -> Vec<Vec<String>> {
        lock(&self.calls).clone()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Zfs for ScriptedZfs {
    fn run(&self, args: &[String]) -> Result<Output, Error> {
        lock(&self.calls).push(args.to_vec());
        if self.missing {
            return Err(Error::Unsupported("the zfs command is not installed"));
        }
        let reply = {
            let mut once = lock(&self.once);
            once.iter()
                .position(|(want, _)| want == args)
                .and_then(|at| once.remove(at))
        }
        .or_else(|| {
            lock(&self.lasting)
                .iter()
                .rev()
                .find(|(want, _)| want == args)
                .cloned()
        });
        match reply {
            Some((_, Ok(output))) => Ok(output),
            Some((_, Err(text))) => Err(Error::from_io(std::io::Error::other(text))),
            None => Ok(Output::failed(
                1,
                format!("cannot open '{}': dataset does not exist\n", args.join(" ")),
            )),
        }
    }

    fn program(&self) -> Option<PathBuf> {
        (!self.missing).then(|| PathBuf::from("/usr/sbin/zfs"))
    }
}

/// Why a `zfs` command failed, by its exit code and its (`LC_ALL=C`)
/// standard error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ZfsError {
    /// ZFS refused for lack of permission; elevation may help.
    #[error("{0}")]
    PermissionDenied(String),
    /// The snapshot (or dataset) to create exists already.
    #[error("{0}")]
    AlreadyExists(String),
    /// A destroy ZFS refused because the snapshot is held or has dependent
    /// clones.
    #[error("{0}")]
    Busy(String),
    /// Any other failure, with ZFS's message verbatim.
    #[error("{0}")]
    Other(String),
}

impl ZfsError {
    /// ZFS's message.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::PermissionDenied(m) | Self::AlreadyExists(m) | Self::Busy(m) | Self::Other(m) => {
                m
            }
        }
    }
}

/// The class of a finished run: `Ok` with standard output when it exited
/// with 0, the failure's class otherwise.
///
/// # Errors
///
/// The [`ZfsError`] the run's exit code and standard error describe.
pub fn classify(output: Output) -> Result<String, ZfsError> {
    if output.success() {
        return Ok(output.stdout);
    }
    let message = match output.stderr.trim() {
        "" => format!(
            "zfs exited with {}",
            output
                .code
                .map_or_else(|| "a signal".to_owned(), |c| format!("code {c}"))
        ),
        text => text.to_owned(),
    };
    let lower = message.to_ascii_lowercase();
    Err(
        if lower.contains("permission denied") || lower.contains("insufficient privileges") {
            ZfsError::PermissionDenied(message)
        } else if lower.contains("already exists") {
            ZfsError::AlreadyExists(message)
        } else if lower.contains("dataset is busy") || lower.contains("has dependent clones") {
            ZfsError::Busy(message)
        } else {
            ZfsError::Other(message)
        },
    )
}

/// The kernel's error for a ZFS failure: an extension error carrying it.
#[must_use]
pub fn to_error(error: ZfsError) -> Error {
    Error::Extension {
        ext: crate::ID,
        source: Box::new(error),
    }
}

/// The arguments as the `OsString`s an elevated run takes.
#[must_use]
pub fn os_args(args: &[String]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args_of(args)
    }

    #[test]
    fn the_fake_replays_recorded_output_and_records_calls() {
        let zfs = ScriptedZfs::new();
        zfs.on(&["list", "-H"], Output::ok("tank\t/tank\tyes\n"));
        zfs.once(&["list", "-H"], Output::failed(1, "first\n"));
        let first = zfs.run(&strings(&["list", "-H"])).unwrap_or_default();
        assert_eq!(first, Output::failed(1, "first\n"));
        for _ in 0..2 {
            let again = zfs.run(&strings(&["list", "-H"])).unwrap_or_default();
            assert_eq!(again.stdout, "tank\t/tank\tyes\n");
        }
        let unknown = zfs.run(&strings(&["get", "x"])).unwrap_or_default();
        assert!(!unknown.success());
        assert_eq!(zfs.calls().len(), 4);
        assert_eq!(zfs.program(), Some(PathBuf::from("/usr/sbin/zfs")));
    }

    #[test]
    fn a_missing_zfs_is_unsupported_and_has_no_program() {
        let zfs = ScriptedZfs::missing();
        assert!(matches!(
            zfs.run(&strings(&["list"])),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(zfs.program(), None);
        let broken = ScriptedZfs::new();
        broken.fail_to_start(&["list"]);
        assert!(matches!(broken.run(&strings(&["list"])), Err(Error::Io(_))));
    }

    #[test]
    fn failures_fall_into_their_classes() {
        let cases = [
            (
                "cannot create snapshot 'tank/home@a': permission denied\n",
                "permission",
            ),
            ("insufficient privileges\n", "permission"),
            (
                "cannot create snapshot 'tank/home@a': dataset already exists\n",
                "exists",
            ),
            (
                "cannot destroy snapshot tank/home@a: dataset is busy\n",
                "busy",
            ),
            (
                "cannot destroy 'tank/home@a': snapshot has dependent clones\nuse '-R' to destroy the following datasets:\ntank/clone\n",
                "busy",
            ),
            (
                "cannot open 'tank/nope': dataset does not exist\n",
                "other",
            ),
        ];
        for (stderr, class) in cases {
            let error = classify(Output::failed(1, stderr)).err();
            let got = match error {
                Some(ZfsError::PermissionDenied(_)) => "permission",
                Some(ZfsError::AlreadyExists(_)) => "exists",
                Some(ZfsError::Busy(_)) => "busy",
                Some(ZfsError::Other(_)) => "other",
                None => "ok",
            };
            assert_eq!(got, class, "{stderr}");
        }
        assert_eq!(classify(Output::ok("out")), Ok("out".to_owned()));
        let silent = classify(Output::failed(2, "")).err();
        assert_eq!(
            silent.as_ref().map(ZfsError::message),
            Some("zfs exited with code 2")
        );
        let other = classify(Output::failed(1, "  something odd  \n")).err();
        assert_eq!(other, Some(ZfsError::Other("something odd".to_owned())));
    }
}
