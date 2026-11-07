//! The native plugin host: finds plugin libraries, checks their manifests,
//! loads them and hands their extensions to the application.
//!
//! A plugin is one `cdylib` built from this workspace (QUESTIONS.md 81): it
//! shares the kernel crates `types`, `runtime` and `ext` with the binary as
//! Rust dylibs, so the extension traits cross the library boundary as
//! themselves and a job the plugin spawns runs on the one tokio runtime, its
//! tracing events reach the one subscriber, and its contributions land in
//! the one registry.
//!
//! At start [`Host::load`] scans `<binary dir>/plugins/` and
//! `<config dir>/plugins/` (per-user wins on an id clash), reads every
//! [`plugin.toml`](PluginToml), refuses a plugin whose `api_version`,
//! `toolchain` or `platforms` do not fit this host, skips the ids
//! `rick.toml` disables, and loads the rest with [`load_library`]: the
//! library's entry point ([`ext::plugin::entry_point`], named after the
//! crate) is called with a registry of its
//! own and the result wrapped as a [`Loaded`] extension that the application
//! registers like a built-in. A plugin that fails or panics while
//! registering is reported in its row and left alone, its library kept
//! mapped (QUESTIONS.md 85); it never takes the host down. One tracing span
//! per plugin carries the outcome.
//!
//! [`describe`] is the other direction: `cargo xtask bundle` loads a freshly
//! built library through it to write the plugin's manifest file
//! (QUESTIONS.md 83).

// Loading a native library and calling into it is unsafe by nature, and
// this crate is where Rick does it; every block carries a `// SAFETY:`
// comment (QUESTIONS.md 82).
#![allow(unsafe_code)]

mod host;
mod load;
mod manifest;

use std::path::Path;

pub use host::{Host, Options, PLUGINS_DIR, Plugin, PluginStatus, Source};
pub use load::{LoadError, Loaded, load_library};
pub use manifest::{Library, MANIFEST_FILE, ManifestError, PluginToml};

/// The toolchain this host was built with, as `rustc -V` prints it. A
/// plugin's manifest must name the same one.
pub const HOST_TOOLCHAIN: &str = env!("RICK_TOOLCHAIN");

/// Loads the library at `path` once, reads the manifest its entry point
/// registers and returns the `plugin.toml` for it, with this build's
/// toolchain and the library names cargo gives the crate; the library
/// stays mapped. For the bundler, not the running application.
///
/// # Errors
///
/// [`LoadError`] when the library is not a plugin of this build.
pub fn describe(path: &Path) -> Result<PluginToml, LoadError> {
    let loaded = load_library(path)?;
    Ok(PluginToml::describe(
        loaded.manifest(),
        &stem_of(path),
        HOST_TOOLCHAIN,
    ))
}

/// The crate name a library file stands for: its file name without the
/// platform's prefix and suffix (`libext_git.so` gives `ext_git`), which
/// names the entry point it exports.
#[must_use]
pub fn stem_of(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.strip_prefix(std::env::consts::DLL_PREFIX)
        .unwrap_or(&name)
        .strip_suffix(std::env::consts::DLL_SUFFIX)
        .unwrap_or(&name)
        .to_owned()
}
