//! Records the build's details for `rick --version` and the About dialog
//! (specs/about-dialog.md), gives the binary a run path so it finds the
//! kernel dylibs and `libstd` beside itself in the bundle, and on Windows
//! embeds the icon and the version resource into `rick.exe`.
//!
//! The details, each a `cargo:rustc-env` the binary reads with `env!`:
//! `RICK_GIT_HASH`, the short commit, from `RICK_GIT_HASH` when set (a
//! build outside a checkout can pass it), else from `git rev-parse`, else
//! `unknown`; `RICK_BUILD_DATE`, the UTC date of the build as `YYYY-MM-DD`
//! (`SOURCE_DATE_EPOCH` when set, for a reproducible build); `RICK_TARGET`,
//! the target triple; and `RICK_CONTRIBUTORS`, the path of a file in
//! `OUT_DIR` with the names of `git shortlog -sn --no-merges`, one per line,
//! empty outside a checkout (cargo passes one line per `rustc-env`, so the
//! names travel in a file the binary embeds with `include_str!`;
//! QUESTIONS.md 258), beside `RICK_CONTRIBUTOR_ROLES`, the path of a copy of
//! the repository's `CONTRIBUTORS.md`, empty when there is none. Nothing is
//! read from git at run time.

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
    println!("cargo:rustc-env=RICK_BUILD_DATE={}", build_date());
    println!(
        "cargo:rustc-env=RICK_TARGET={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned())
    );
    contributors(manifest);

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

/// The UTC date of the build, `YYYY-MM-DD`: from `SOURCE_DATE_EPOCH` when
/// the environment sets it (a reproducible build), else the clock.
fn build_date() -> String {
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let seconds = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
        });
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    format!("{year:04}-{month:02}-{day:02}")
}

/// The proleptic Gregorian date of a day count since 1970-01-01 (Howard
/// Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    // `doy` and `mp` are small and non-negative: the casts are exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Writes the contributors' names (`git shortlog -sn --no-merges`, in its
/// order, one per line; empty outside a checkout or without git) and a copy
/// of `CONTRIBUTORS.md` (empty when the repository has none) into `OUT_DIR`,
/// and names the two files in `RICK_CONTRIBUTORS` and
/// `RICK_CONTRIBUTOR_ROLES` for the binary's `include_str!`. The git hash's
/// `rerun-if-changed` on `.git/HEAD` and the branch's ref file reruns this
/// when a commit lands.
fn contributors(manifest: &Path) {
    let out_dir = std::env::var_os("OUT_DIR").map(std::path::PathBuf::from);
    let Some(out_dir) = out_dir else {
        println!("cargo:warning=OUT_DIR is not set; the contributors are not recorded");
        return;
    };
    let names: String = Command::new("git")
        .args(["shortlog", "-sn", "--no-merges", "HEAD"])
        .current_dir(manifest)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map_or_default(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| line.split_once('\t').map(|(_, name)| name.trim()))
                .filter(|name| !name.is_empty())
                .fold(String::new(), |mut names, name| {
                    names.push_str(name);
                    names.push('\n');
                    names
                })
        });
    let roles_file = manifest.join("../../CONTRIBUTORS.md");
    // A file that is not there cannot be watched; the next commit reruns
    // the script and picks a new one up.
    if roles_file.is_file() {
        println!("cargo:rerun-if-changed={}", roles_file.display());
    }
    let roles = std::fs::read_to_string(&roles_file).unwrap_or_default();
    for (name, env, text) in [
        ("contributors.txt", "RICK_CONTRIBUTORS", names),
        ("contributors.md", "RICK_CONTRIBUTOR_ROLES", roles),
    ] {
        let path = out_dir.join(name);
        if let Err(error) = std::fs::write(&path, text) {
            println!(
                "cargo:warning={} could not be written: {error}",
                path.display()
            );
            std::process::exit(1);
        }
        println!("cargo:rustc-env={env}={}", path.display());
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
