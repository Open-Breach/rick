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
    let output = Command::new(pkexec)
        .arg(&program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(Error::from_io)?;
    match output.status.code() {
        // pkexec's own codes: 126 when the prompt was dismissed, 127 when
        // the user is not authorised or the program could not be started.
        Some(126 | 127) => Err(Error::PermissionDenied),
        code => Ok(ElevatedOutput {
            code,
            stdout: output.stdout,
            stderr: output.stderr,
        }),
    }
}
