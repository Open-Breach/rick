//! `rick-plugin-manifest <library> <plugins dir>`: places one plugin into a
//! plugins directory for `cargo xtask bundle`.
//!
//! Loads the library once, reads the manifest its entry point registers,
//! writes `<plugins dir>/<id>/plugin.toml` from it with this build's
//! toolchain (QUESTIONS.md 83), copies the library beside the file and
//! prints the id. Built with the workspace, never shipped: the bundler
//! runs it so that it needs no kernel crate of its own (QUESTIONS.md 81).

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use plugins_native::MANIFEST_FILE;

fn main() -> ExitCode {
    match run() {
        Ok(id) => {
            // The command's output is its contract with the bundler.
            let _ = writeln!(std::io::stdout().lock(), "{id}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            let _ = writeln!(std::io::stderr().lock(), "rick-plugin-manifest: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, String> {
    let mut args = std::env::args_os().skip(1);
    let (Some(library), Some(plugins_dir), None) = (args.next(), args.next(), args.next()) else {
        return Err("usage: rick-plugin-manifest <library> <plugins dir>".to_owned());
    };
    let library = PathBuf::from(library);
    let plugins_dir = PathBuf::from(plugins_dir);
    let toml =
        plugins_native::describe(&library).map_err(|e| format!("{}: {e}", library.display()))?;
    let dir = plugins_dir.join(&toml.id);
    toml.write(&dir.join(MANIFEST_FILE))
        .map_err(|e| e.to_string())?;
    let file = toml
        .library
        .for_os(std::env::consts::OS)
        .ok_or_else(|| format!("no library name for {}", std::env::consts::OS))?;
    let target = dir.join(file);
    std::fs::copy(&library, &target)
        .map_err(|e| format!("copying {} to {}: {e}", library.display(), target.display()))?;
    Ok(toml.id)
}
