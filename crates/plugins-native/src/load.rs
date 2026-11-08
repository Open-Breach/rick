//! Opening a plugin library and calling its entry point.

use std::fmt;
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::platform::Platform;
use ext::plugin::{EntryPoint, Status, entry_point};
use ext::{Extension, Manifest, Registry};
use tracing::{debug, warn};
use types::Error;

/// Why a library could not be loaded as a plugin.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The dynamic loader refused the file: missing, not a library, or
    /// built against another kernel build whose libraries it cannot find.
    #[error("the library could not be loaded: {0}")]
    Open(#[source] libloading::Error),
    /// The library is not a Rick plugin: it exports no entry point named
    /// after its file.
    #[error("the library exports no `{0}`: {1}")]
    NoEntryPoint(String, #[source] libloading::Error),
    /// The registry refused the manifest or the extension's `register`
    /// failed.
    #[error("{0}")]
    Failed(String),
    /// The plugin panicked while registering; caught, the library kept
    /// mapped and left alone.
    #[error("{0}")]
    Panicked(String),
    /// The entry point registered several extensions or none; a plugin is
    /// one extension.
    #[error("the entry point registered {0} extensions instead of one")]
    NotOne(usize),
}

/// A plugin library with its extension registered: what the application
/// loads like a built-in extension.
///
/// The library stays mapped for the rest of the process, whatever happens
/// to this value: every contribution points into its code, and code that
/// ran may have registered itself with the runtime (tracing callsites,
/// thread-local destructors), so unmapping it is never safe (QUESTIONS.md
/// 85).
pub struct Loaded {
    manifest: Manifest,
    /// The plugin's contributions, as the entry point registered them.
    /// Declared before `library` so they drop first, while the code their
    /// `Drop` implementations live in is still mapped.
    contributions: Registry,
    path: PathBuf,
    /// Never dropped: see the type's documentation.
    _library: ManuallyDrop<libloading::Library>,
}

impl Loaded {
    /// The plugin's manifest.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The contributions the plugin registered.
    #[must_use]
    pub fn contributions(&self) -> &Registry {
        &self.contributions
    }

    /// The library file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Extension for Loaded {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.absorb(&self.contributions);
        Ok(())
    }
}

impl fmt::Debug for Loaded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Loaded")
            .field("manifest", &self.manifest)
            .field("contributions", &self.contributions)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// The crate name a library file stands for: `libext_git.so` and
/// `ext_git.dll` give `ext_git`.
#[must_use]
pub fn crate_stem(path: &Path) -> String {
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

/// Opens the library at `path`, calls its entry point (named after the
/// library's file: `libext_git.so` exports `rick_plugin_register_ext_git`)
/// with a fresh registry lending `platform` and returns the plugin with its
/// contributions.
///
/// The caller has checked the plugin's manifest file first: the toolchain
/// and the API version are what make the library's entry point safe to
/// call. A library whose entry point ran stays mapped even when it is
/// refused, because its code may have registered itself with the runtime;
/// only a library that is no plugin at all is unmapped again.
///
/// # Errors
///
/// [`LoadError`] says what went wrong; the reason a plugin gave for failing
/// or the message of the panic it raised is in the error's text.
pub fn load_library(path: &Path, platform: Arc<dyn Platform>) -> Result<Loaded, LoadError> {
    debug!(path = %path.display(), "loading");
    // SAFETY: loading runs the library's initialisers. A Rust cdylib built
    // from this workspace has none beyond the runtime's own, and the caller
    // has checked the manifest that says it is one.
    let library = unsafe { libloading::Library::new(path) }.map_err(LoadError::Open)?;
    let symbol = entry_point(&crate_stem(path));
    let mut registry = Registry::with_platform(platform);
    let status = {
        // SAFETY: `EntryPoint` is the signature `ext::plugin!` declares for
        // this symbol; the manifest checks (API version, toolchain) make
        // the library one built against the same `ext`.
        let entry: libloading::Symbol<EntryPoint> = unsafe { library.get(symbol.as_bytes()) }
            .map_err(|error| LoadError::NoEntryPoint(symbol.clone(), error))?;
        // SAFETY: the entry point catches its own panics and only touches
        // the registry it is given, which outlives the call.
        unsafe { entry(&mut registry) }
    };
    // From here on the plugin's code has run: whatever it registered with
    // the process (tracing callsites, thread-local destructors) points into
    // the library, so it stays mapped for the rest of the process.
    let library = ManuallyDrop::new(library);
    let failure = || registry.failure().unwrap_or("no reason given").to_owned();
    let result = match status {
        Status::Registered => match registry.manifests() {
            [manifest] => Ok(manifest.clone()),
            other => Err(LoadError::NotOne(other.len())),
        },
        Status::Failed => Err(LoadError::Failed(failure())),
        Status::Panicked => Err(LoadError::Panicked(failure())),
    };
    match result {
        Ok(manifest) => Ok(Loaded {
            manifest,
            contributions: registry,
            path: path.to_owned(),
            _library: library,
        }),
        Err(error) => {
            warn!(path = %path.display(), %error, "plugin not loaded; its library stays mapped");
            // The contributions drop while the code they point into is
            // mapped, which it stays.
            drop(registry);
            Err(error)
        }
    }
}
