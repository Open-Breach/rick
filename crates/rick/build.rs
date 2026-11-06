//! Records the git hash of the build, which `rick --version` prints after the
//! version and the About window shows.
//!
//! The hash comes from `RICK_GIT_HASH` when set (a build outside a checkout
//! can pass it), else from `git rev-parse`, else it is `unknown`.

use std::path::Path;
use std::process::Command;

fn main() {
    // The build script's only channel to cargo is stdout.
    println!("cargo:rerun-if-env-changed=RICK_GIT_HASH");
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let git_dir = manifest.join("../../.git");
    let head = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head.display());
    if let Ok(text) = std::fs::read_to_string(&head)
        && let Some(reference) = text.trim().strip_prefix("ref: ")
    {
        println!(
            "cargo:rerun-if-changed={}",
            git_dir.join(reference).display()
        );
    }

    let hash = std::env::var("RICK_GIT_HASH")
        .ok()
        .map(|h| h.trim().to_owned())
        .filter(|h| !h.is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short=9", "HEAD"])
                .current_dir(manifest)
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
                .filter(|h| !h.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=RICK_GIT_HASH={hash}");
}
