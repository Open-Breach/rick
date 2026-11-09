//! Records the git hash of the build, which `rick --version` prints after the
//! version and the About window shows, gives the binary a run path so it
//! finds the kernel dylibs and `libstd` beside itself in the bundle, and on
//! Windows embeds the icon and the version resource into `rick.exe`.
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

    // The kernel dylibs (`types`, `runtime`, `ext`) and `libstd` sit beside
    // the binary in a bundle (`cargo xtask bundle`, QUESTIONS.md 84). Linux
    // and macOS look there through a run path; Windows searches the
    // binary's directory on its own. This is build configuration, not a
    // `cfg(target_os)` in code.
    // On macOS the bundle may also carry the GStreamer runtime under
    // `gstreamer/lib/` (QUESTIONS.md 106); its libraries name each other
    // through `@rpath`, and the elements GStreamer loads at run time
    // resolve those names through the executable's run paths.
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("linux") => println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN"),
        Ok("macos") => {
            println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path");
            println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path/gstreamer/lib");
        }
        // The icon and version resource (specs/app-icons.md): Explorer and
        // the Properties dialog show the Portal burst icon, "Rick" and the
        // version of Cargo.toml. Windows targets only, and `rick.exe` alone:
        // the helper `rick-perms` is no application. Build configuration
        // again, decided by the target.
        Ok("windows") => windows_resource(manifest),
        _ => {}
    }
}

/// Compiles `assets/icons/windows/rick.ico` and the version information
/// into a resource linked into `rick.exe`. winresource takes `FileVersion`
/// and `ProductVersion` from the package version cargo hands the build
/// script (the numbers from major, minor and patch, the strings whole, a
/// pre-release suffix included). A resource that does not compile fails
/// the build: an executable without its icon is not the product.
fn windows_resource(manifest: &Path) {
    let icon = manifest.join("../../assets/icons/windows/rick.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(&icon.to_string_lossy())
        .set("ProductName", "Rick")
        .set("FileDescription", "Rick file manager")
        .set("OriginalFilename", "rick.exe")
        .set("InternalName", "rick")
        .set(
            "LegalCopyright",
            "Copyright © 2026 Open-Breach and the Rick contributors. \
             Licensed under the Apache License, Version 2.0.",
        );
    if let Err(error) = resource.compile_for("rick") {
        println!(
            "cargo:warning=the icon and version resource of rick.exe did not compile: {error}"
        );
        std::process::exit(1);
    }
}
