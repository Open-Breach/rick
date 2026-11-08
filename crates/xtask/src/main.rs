//! Rick's build tasks, run as `cargo xtask <task>` through the alias in
//! `.cargo/config.toml`.
//!
//! `bundle [--release] [--features <spec>]` builds the whole workspace and
//! lays out `target/<profile>/bundle/` (QUESTIONS.md 84): the `rick`
//! binary, the kernel dylibs and the `libstd` they need beside it, pdfium
//! when `RICK_PDFIUM_DIR` names it, the GStreamer runtime when
//! `RICK_GSTREAMER_DIR` names it (QUESTIONS.md 106: on macOS into
//! `gstreamer/lib/`, on Windows the DLLs beside the binary and the
//! elements into `gstreamer-1.0/`; Linux uses the system's),
//! `plugins/<id>/plugin.toml` with the library for every plugin crate (an
//! `ext-*` member built as a `cdylib`) whose manifest names this platform,
//! and the 7-Zip library (`7z.so` / `7z.dll`, LGPL) with its licence into
//! `plugins/vfs-archive/` beside the archive plugin when
//! `RICK_SEVENZIP_DIR` names the directory holding it (SPEC.md
//! "Third-party libraries"; `.github/scripts/sevenzip.sh` fills one);
//! a plugin built for other platforms only (the Spotlight connector on
//! Linux) is left out and said so (QUESTIONS.md 121). `--features` is
//! handed to the build as is (`--features ext-preview-media/gstreamer`
//! turns playback on). Running `rick` from there finds every plugin with
//! nothing copied by hand, and the release workflow archives the same
//! directory.
//!
//! The task builds with `--workspace --exclude xtask --lib --bins
//! --examples` (every library, binary and example of every member but this
//! one: the one build flavour the tests use too, QUESTIONS.md 81) and runs the workspace's own
//! `rick-plugin-manifest` to write the manifests, so that it depends on no
//! kernel crate itself: a build of a smaller package set would rewrite the
//! shared libraries for another feature set.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const KERNEL_DYLIBS: [&str; 3] = ["types", "runtime", "ext"];
const PDFIUM_FILES: [&str; 3] = ["libpdfium.so", "libpdfium.dylib", "pdfium.dll"];
/// The 7-Zip library's names, and the plugin it travels beside.
const SEVENZIP_FILES: [&str; 2] = ["7z.so", "7z.dll"];
const SEVENZIP_PLUGIN: &str = "vfs-archive";
/// The licence file beside the library in `RICK_SEVENZIP_DIR`, and its
/// name in the bundle.
const SEVENZIP_LICENSE: (&str, &str) = ("License.txt", "7-Zip-License.txt");
/// The manifest helper's exit code for a plugin built for other platforms.
const NOT_FOR_THIS_PLATFORM: i32 = 3;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args.first().map(String::as_str) {
        Some("bundle") => bundle(&args[1..]),
        _ => Err("usage: cargo xtask bundle [--release] [--features <spec>]".into()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // A build tool reports to the terminal.
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn bundle(args: &[String]) -> Result<()> {
    let mut release = false;
    let mut features: Option<String> = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--release" => release = true,
            "--features" => {
                features = Some(
                    args.next()
                        .ok_or("--features takes the features to enable")?
                        .clone(),
                );
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());

    // One build of every library, binary and example in the workspace,
    // with the dev-dependency features the tests build with: the one
    // flavour every other command of this repository builds. A target
    // selection builds only what it names, so the libraries and binaries
    // are named with the examples. xtask itself stays out: `cargo run -p
    // xtask` built it with its own feature set, and a rebuild would
    // replace the running binary, which Windows refuses.
    let mut build = Command::new(&cargo);
    build.args([
        "build",
        "--workspace",
        "--exclude",
        "xtask",
        "--lib",
        "--bins",
        "--examples",
    ]);
    if release {
        build.arg("--release");
    }
    if let Some(features) = &features {
        build.args(["--features", features]);
    }
    run(&mut build)?;

    let meta = metadata(&cargo)?;
    let profile_dir = meta
        .target_dir
        .join(if release { "release" } else { "debug" });
    let out = profile_dir.join("bundle");
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }
    let plugins_dir = out.join("plugins");
    std::fs::create_dir_all(&plugins_dir)?;

    copy(&profile_dir.join(exe("rick")), &out)?;
    // The privileged helper lives beside the binary, where Rick looks for
    // it (`platform::perms::helper_path`).
    copy(&profile_dir.join(exe("rick-perms")), &out)?;
    for name in KERNEL_DYLIBS {
        copy(&profile_dir.join(dylib(name)), &out)?;
    }
    for lib in libstd(&cargo)? {
        copy(&lib, &out)?;
    }
    if let Some(dir) = std::env::var_os("RICK_PDFIUM_DIR") {
        for file in PDFIUM_FILES {
            let path = Path::new(&dir).join(file);
            if path.is_file() {
                copy(&path, &out)?;
            }
        }
    }
    if let Some(dir) = std::env::var_os("RICK_GSTREAMER_DIR") {
        gstreamer(Path::new(&dir), &out)?;
    }

    let helper = profile_dir.join(exe("rick-plugin-manifest"));
    for crate_name in &meta.plugin_crates {
        let library = profile_dir.join(dylib(&crate_name.replace('-', "_")));
        let output = Command::new(&helper)
            .arg(&library)
            .arg(&plugins_dir)
            .output()?;
        if output.status.code() == Some(NOT_FOR_THIS_PLATFORM) {
            println!(
                "plugin from {crate_name}: {}; not bundled",
                String::from_utf8_lossy(&output.stderr)
                    .trim()
                    .trim_start_matches("rick-plugin-manifest: ")
            );
            continue;
        }
        if !output.status.success() {
            return Err(format!(
                "{} failed for {}: {}",
                helper.display(),
                library.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .into());
        }
        let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        println!(
            "plugin {id}: {} from {crate_name}",
            plugins_dir.join(&id).display()
        );
    }
    match std::env::var_os("RICK_SEVENZIP_DIR") {
        Some(dir) => sevenzip(Path::new(&dir), &plugins_dir)?,
        None => println!("7-zip: RICK_SEVENZIP_DIR not set; the library is not bundled"),
    }
    println!("bundle: {}", out.display());
    Ok(())
}

/// Copies the 7-Zip library and its licence from `dir` into the archive
/// plugin's folder, where the plugin looks for it first after its setting.
fn sevenzip(dir: &Path, plugins_dir: &Path) -> Result<()> {
    let target = plugins_dir.join(SEVENZIP_PLUGIN);
    std::fs::create_dir_all(&target)?;
    let library = SEVENZIP_FILES
        .iter()
        .map(|file| dir.join(file))
        .find(|path| path.is_file())
        .ok_or_else(|| format!("no {} in {}", SEVENZIP_FILES.join(" or "), dir.display()))?;
    copy(&library, &target)?;
    let (source, name) = SEVENZIP_LICENSE;
    let license = dir.join(source);
    if license.is_file() {
        std::fs::copy(&license, target.join(name))
            .map_err(|e| format!("copying {}: {e}", license.display()))?;
    } else {
        println!("7-zip: no {source} beside the library; the licence is not bundled");
    }
    println!("7-zip: {} into {}", library.display(), target.display());
    Ok(())
}

/// Copies the GStreamer runtime at `dir` into the bundle, in the layout the
/// media previewer looks in (QUESTIONS.md 106). macOS: the libraries into
/// `gstreamer/lib/`, the elements into `gstreamer/lib/gstreamer-1.0/`, the
/// plugin scanner into `gstreamer/libexec/gstreamer-1.0/`; the official
/// framework names its libraries through `@rpath`, which the binary and
/// the media plugin point there. Windows: the DLLs beside the binary,
/// where every DLL the process loads is searched for, the elements into
/// `gstreamer-1.0/`. Linux uses the system GStreamer and copies nothing.
fn gstreamer(dir: &Path, out: &Path) -> Result<()> {
    let (lib_dir, plugin_dir, scanner) = match std::env::consts::OS {
        "macos" => (
            Some((dir.join("lib"), out.join("gstreamer").join("lib"), "dylib")),
            (
                dir.join("lib").join("gstreamer-1.0"),
                out.join("gstreamer").join("lib").join("gstreamer-1.0"),
            ),
            Some((
                dir.join("libexec")
                    .join("gstreamer-1.0")
                    .join("gst-plugin-scanner"),
                out.join("gstreamer").join("libexec").join("gstreamer-1.0"),
            )),
        ),
        "windows" => (
            Some((dir.join("bin"), out.to_path_buf(), "dll")),
            (
                dir.join("lib").join("gstreamer-1.0"),
                out.join("gstreamer-1.0"),
            ),
            None,
        ),
        other => {
            println!("gstreamer: {other} uses the system runtime; nothing bundled");
            return Ok(());
        }
    };
    let mut count = 0;
    if let Some((from, to, suffix)) = lib_dir {
        count += copy_matching(&from, &to, |name| {
            Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(suffix))
        })?;
    }
    let (from, to) = plugin_dir;
    count += copy_matching(&from, &to, |name| {
        Path::new(name).extension().is_some_and(|e| {
            e.eq_ignore_ascii_case("dylib")
                || e.eq_ignore_ascii_case("so")
                || e.eq_ignore_ascii_case("dll")
        })
    })?;
    if let Some((from, to)) = scanner
        && from.is_file()
    {
        std::fs::create_dir_all(&to)?;
        copy(&from, &to)?;
        count += 1;
    }
    if count == 0 {
        return Err(format!("no GStreamer runtime files found under {}", dir.display()).into());
    }
    println!("gstreamer: {count} files from {}", dir.display());
    Ok(())
}

/// Copies the regular files of `from` whose name `keep` accepts into `to`
/// (created), skipping symbolic links (the framework's version-less
/// aliases, which would double the size); returns how many.
fn copy_matching(from: &Path, to: &Path, keep: impl Fn(&str) -> bool) -> Result<usize> {
    let entries = match std::fs::read_dir(from) {
        Ok(entries) => entries,
        Err(error) => {
            println!("gstreamer: {}: {error}; skipped", from.display());
            return Ok(0);
        }
    };
    std::fs::create_dir_all(to)?;
    let mut count = 0;
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() || !meta.is_file() || !keep(&name) {
            continue;
        }
        copy(&path, to)?;
        count += 1;
    }
    Ok(count)
}

/// What `cargo metadata` tells about the workspace.
struct Metadata {
    target_dir: PathBuf,
    /// The `ext-*` members built as a `cdylib`: the plugins.
    plugin_crates: Vec<String>,
}

fn metadata(cargo: &std::ffi::OsStr) -> Result<Metadata> {
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let target_dir = json["target_directory"]
        .as_str()
        .ok_or("cargo metadata: no target_directory")?;
    let mut plugin_crates = Vec::new();
    for package in json["packages"].as_array().into_iter().flatten() {
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        let is_cdylib = package["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|target| {
                target["crate_types"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|kind| kind == "cdylib")
            });
        if name.starts_with("ext-") && is_cdylib {
            plugin_crates.push(name.to_owned());
        }
    }
    plugin_crates.sort();
    Ok(Metadata {
        target_dir: PathBuf::from(target_dir),
        plugin_crates,
    })
}

/// The `libstd` the build links dynamically, from the toolchain cargo
/// belongs to.
fn libstd(cargo: &std::ffi::OsStr) -> Result<Vec<PathBuf>> {
    // `CARGO` is the real binary inside the toolchain; its sibling `rustc`
    // is the compiler the build used. Fall back to the one on the path.
    let rustc = Path::new(cargo)
        .parent()
        .map(|dir| dir.join(exe("rustc")))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(exe("rustc")));
    let output = Command::new(&rustc)
        .args(["--print", "target-libdir"])
        .output()?;
    if !output.status.success() {
        return Err(format!("{} --print target-libdir failed", rustc.display()).into());
    }
    let libdir = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let prefix = format!("{}std-", std::env::consts::DLL_PREFIX);
    let mut found: Vec<PathBuf> = std::fs::read_dir(&libdir)?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with(&prefix) && n.ends_with(std::env::consts::DLL_SUFFIX)
            })
        })
        .collect();
    found.sort();
    if found.is_empty() {
        return Err(format!(
            "no {prefix}*{} in {}",
            std::env::consts::DLL_SUFFIX,
            libdir.display()
        )
        .into());
    }
    Ok(found)
}

fn run(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{command:?} failed with {status}").into())
    }
}

fn copy(from: &Path, to_dir: &Path) -> Result<()> {
    let name = from
        .file_name()
        .ok_or_else(|| format!("no file name: {}", from.display()))?;
    std::fs::copy(from, to_dir.join(name))
        .map_err(|e| format!("copying {}: {e}", from.display()))?;
    Ok(())
}

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn dylib(name: &str) -> String {
    format!(
        "{}{name}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}
