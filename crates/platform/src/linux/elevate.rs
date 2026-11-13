//! Running one command as root through `pkexec`, so the user sees the
//! desktop's polkit prompt and Rick itself never runs elevated.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

use types::Error;

use super::apps::on_path;
use crate::traits::ElevatedOutput;

/// Runs `program` with `args` under `pkexec` and waits for it.
pub(crate) fn run(program: &Path, args: &[OsString]) -> Result<ElevatedOutput, Error> {
    let Some(pkexec) = on_path("pkexec") else {
        return Err(Error::Unsupported(
            "pkexec is not installed; running a command as the administrator needs polkit",
        ));
    };
    let program = std::path::absolute(program).map_err(Error::from_io)?;
    let output = command(&pkexec, &program, args)
        .output()
        .map_err(Error::from_io)?;
    let code = output.status.code();
    if let Some(error) = refusal(code, &String::from_utf8_lossy(&output.stderr)) {
        return Err(error);
    }
    Ok(ElevatedOutput {
        code,
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

/// The `pkexec` call for `program` with `args`. Without `SHELL`: `pkexec`
/// refuses to start when it names a shell missing from `/etc/shells`, as
/// one from Nix or Home Manager is, and an unset `SHELL` is not checked.
fn command(pkexec: &Path, program: &Path, args: &[OsString]) -> Command {
    let mut command = Command::new(pkexec);
    command
        .arg(program)
        .args(args)
        .env_remove("SHELL")
        .stdin(Stdio::null());
    command
}

/// What `pkexec`'s own exit codes mean, `None` for the program's: 126 is
/// the prompt dismissed, 127 the user not authorised or `pkexec` unable to
/// start the program, which it says on stderr and the user is told.
fn refusal(code: Option<i32>, stderr: &str) -> Option<Error> {
    match code {
        Some(126) => Some(Error::PermissionDenied),
        Some(127) if stderr.trim().is_empty() || stderr.contains("Not authorized") => {
            Some(Error::PermissionDenied)
        }
        Some(127) => Some(Error::Io(std::io::Error::other(format!(
            "pkexec: {}",
            stderr.trim()
        )))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `SHELL` outside `/etc/shells` made `pkexec` exit 127 before any
    /// prompt, which read as "permission denied".
    #[test]
    fn pkexec_runs_without_the_callers_shell() {
        let command = command(
            Path::new("/usr/bin/pkexec"),
            Path::new("/opt/rick/rick-zfs"),
            &[OsString::from("snapshot")],
        );
        let shell = command
            .get_envs()
            .find(|(name, _)| *name == "SHELL")
            .map(|(_, value)| value);
        assert_eq!(shell, Some(None), "SHELL removed");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["/opt/rick/rick-zfs", "snapshot"]);
    }

    #[test]
    fn pkexecs_own_failures_are_told_apart() {
        assert!(matches!(
            refusal(Some(126), ""),
            Some(Error::PermissionDenied)
        ));
        assert!(matches!(
            refusal(
                Some(127),
                "Error executing command as another user: Not authorized\n"
            ),
            Some(Error::PermissionDenied)
        ));
        let shell = refusal(
            Some(127),
            "The value for the SHELL variable was not found in the /etc/shells file\n",
        );
        assert!(
            shell
                .as_ref()
                .is_some_and(|e| e.to_string().contains("/etc/shells")),
            "{shell:?}"
        );
        assert!(refusal(Some(0), "").is_none());
        assert!(refusal(Some(1), "cannot create snapshots").is_none());
    }
}
