//! The part of Rick's build tasks that is a library: what the `xtask`
//! binary runs and the workspace's tests run too.
//!
//! [`package`] makes the plugin packages of a bundle, the files a release
//! publishes for its separate plugins (specs/plugin-distribution.md §4).
//! The tasks themselves, `cargo xtask bundle`, `package` and `readme`, are
//! the binary of this crate.

pub mod package;

/// What a task returns; its error is a message for the terminal.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
