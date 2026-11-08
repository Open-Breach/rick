//! `rick-plugin-manifest <library> <plugins dir>`: places one plugin into a
//! plugins directory for `cargo xtask bundle`.
//!
//! Loads the library once, reads the manifest its entry point registers,
//! writes `<plugins dir>/<id>/plugin.toml` from it with this build's
//! toolchain (QUESTIONS.md 83), copies the library beside the file and
//! prints the id. A plugin whose manifest leaves this platform out (the
//! Spotlight connector on Linux) is not placed: the helper says so on its
//! standard error and exits with [`NOT_FOR_THIS_PLATFORM`], and the bundler
//! moves on (QUESTIONS.md 121). Built with the workspace, never shipped:
//! the bundler runs it so that it needs no kernel crate of its own
//! (QUESTIONS.md 81).

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use plugins_native::MANIFEST_FILE;

/// The exit code for a plugin built for other platforms than this one.
pub const NOT_FOR_THIS_PLATFORM: u8 = 3;

/// Why the plugin was not placed.
enum Skip {
    /// Its manifest names other platforms.
    Platform(String),
    /// Anything else.
    Failed(String),
}

fn main() -> ExitCode {
    match run() {
        Ok(id) => {
            // The command's output is its contract with the bundler.
            let _ = writeln!(std::io::stdout().lock(), "{id}");
            ExitCode::SUCCESS
        }
        Err(Skip::Platform(message)) => {
            let _ = writeln!(std::io::stderr().lock(), "rick-plugin-manifest: {message}");
            ExitCode::from(NOT_FOR_THIS_PLATFORM)
        }
        Err(Skip::Failed(message)) => {
            let _ = writeln!(std::io::stderr().lock(), "rick-plugin-manifest: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, Skip> {
    let mut args = std::env::args_os().skip(1);
    let (Some(library), Some(plugins_dir), None) = (args.next(), args.next(), args.next()) else {
        return Err(Skip::Failed(
            "usage: rick-plugin-manifest <library> <plugins dir>".to_owned(),
        ));
    };
    let library = PathBuf::from(library);
    let plugins_dir = PathBuf::from(plugins_dir);
    let toml = plugins_native::describe(&library)
        .map_err(|e| Skip::Failed(format!("{}: {e}", library.display())))?;
    if !toml.platforms.iter().any(|p| p == std::env::consts::OS) {
        return Err(Skip::Platform(format!(
            "{}: built for {}, not {}",
            toml.id,
            toml.platforms.join(", "),
            std::env::consts::OS
        )));
    }
    let dir = plugins_dir.join(&toml.id);
    toml.write(&dir.join(MANIFEST_FILE))
        .map_err(|e| Skip::Failed(e.to_string()))?;
    let file = toml
        .library
        .for_os(std::env::consts::OS)
        .ok_or_else(|| Skip::Failed(format!("no library name for {}", std::env::consts::OS)))?;
    let target = dir.join(file);
    std::fs::copy(&library, &target).map_err(|e| {
        Skip::Failed(format!(
            "copying {} to {}: {e}",
            library.display(),
            target.display()
        ))
    })?;
    Ok(toml.id)
}
