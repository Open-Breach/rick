//! The native plugin host: loads plugin libraries and hands their
//! extensions to the application.
//!
//! A plugin is one `cdylib` built from this workspace (QUESTIONS.md 81): it
//! shares the kernel crates `types`, `runtime` and `ext` with the binary as
//! Rust dylibs, so the extension traits cross the library boundary as
//! themselves and a job the plugin spawns runs on the one tokio runtime, its
//! tracing events reach the one subscriber, and its contributions land in
//! the one registry. [`load_library`] opens a library, calls its
//! [`ext::plugin::ENTRY_POINT`] with a registry of its own and wraps the
//! result as a [`Loaded`] extension the application registers like a
//! built-in; a plugin that fails or panics while registering is reported
//! and unloaded.
//!
//! This is the prototype of the host: discovery of the plugins directories,
//! the manifest file and its checks follow.

// Loading a native library and calling into it is unsafe by nature, and
// this crate is where Rick does it; every block carries a `// SAFETY:`
// comment (QUESTIONS.md 82).
#![allow(unsafe_code)]

mod load;

pub use load::{LoadError, Loaded, load_library};

/// The toolchain this host was built with, as `rustc -V` prints it. A
/// plugin's manifest must name the same one.
pub const HOST_TOOLCHAIN: &str = env!("RICK_TOOLCHAIN");
