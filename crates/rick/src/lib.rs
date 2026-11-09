//! Rick's command line and single instance as a library, so the binary and
//! its integration tests share them: [`cli`] parses the command line
//! (specs/system-integration.md §4), [`instance`] hands it to the running
//! Rick or serves the endpoint for the next launch (§3). The binary wires
//! the kernel, the plugins and the egui shell around them.

pub mod cli;
pub mod instance;

/// The version and the git hash of this build, as `--version` prints them
/// and the About window shows them.
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("RICK_GIT_HASH"), ")");
