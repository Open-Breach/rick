//! The plugin boundary: how a plugin library hands its [`Extension`] to the
//! host.
//!
//! A plugin is a `cdylib` crate built from this workspace. It shares the
//! kernel crates (`types`, `runtime` and this crate) with the host as Rust
//! dylibs, so the five traits, the job runner and the tracing subscriber
//! are the same code in the same memory on both sides and cross the
//! boundary unchanged (QUESTIONS.md 81). The one thing a plugin exports is
//! the `extern "C"` function [`ENTRY_POINT`], `rick_plugin_register`,
//! declared with [`plugin!`](crate::plugin!): the host loads the library, calls it with a
//! [`Registry`] and reads a [`Status`] back. Inside, [`register`] builds
//! the extension, lets the registry load it and catches any panic, so a
//! plugin that fails to register is reported and never takes the host
//! down.
//!
//! # Worked example
//!
//! The whole of a plugin crate beyond its extension is one line, and the
//! host's side after loading the library is one call.
//!
//! ```
//! use std::sync::Arc;
//!
//! use ext::plugin::Status;
//! use ext::{API_VERSION, Capabilities, Extension, Manifest, Registry, Version};
//! use types::{Error, ExtId};
//!
//! struct Hello(Manifest);
//!
//! impl Hello {
//!     fn new() -> Self {
//!         Hello(Manifest {
//!             id: ExtId("hello"),
//!             name: "Hello",
//!             version: Version::new(0, 1, 0),
//!             api_version: API_VERSION,
//!             requires: Capabilities::empty(),
//!         })
//!     }
//! }
//!
//! impl Extension for Hello {
//!     fn manifest(&self) -> &Manifest {
//!         &self.0
//!     }
//!     fn register(&self, _reg: &mut Registry) -> Result<(), Error> {
//!         Ok(())
//!     }
//! }
//!
//! // The plugin crate (`crate-type = ["cdylib", "rlib"]`) declares its
//! // entry point; in the library this is the one exported symbol.
//! ext::plugin!(|| Arc::new(Hello::new()) as Arc<dyn Extension>);
//!
//! // The host, having loaded the library and resolved `ENTRY_POINT` as an
//! // `EntryPoint`, calls it with a registry of its own.
//! let mut registry = Registry::new();
//! assert_eq!(rick_plugin_register(&mut registry), Status::Registered);
//! assert_eq!(registry.extensions(), &[ExtId("hello")]);
//! assert_eq!(registry.failure(), None);
//! ```
//!
//! [`Status::Registered`] means the registry now holds the plugin's manifest
//! and contributions; anything else leaves the reason in
//! [`Registry::failure`].

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use crate::registry::{Extension, Registry};

/// The name of the function every plugin library exports.
pub const ENTRY_POINT: &str = "rick_plugin_register";

/// The entry point's signature, as the host resolves it from the library.
///
/// Calling it is unsafe for the host only because the symbol comes from a
/// library it did not compile: the manifest checks (`api_version`,
/// toolchain) are what make the signature trustworthy.
pub type EntryPoint = unsafe extern "C" fn(&mut Registry) -> Status;

/// What the entry point reports back to the host.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The extension registered; the registry holds its manifest and its
    /// contributions.
    Registered = 0,
    /// The registry refused the manifest or `register` returned an error;
    /// [`Registry::failure`] says which.
    Failed = 1,
    /// Building or registering the extension panicked; the panic was caught
    /// inside the plugin and [`Registry::failure`] carries its message.
    Panicked = 2,
}

/// Registers the extension `ctor` builds into `reg`, catching panics.
///
/// This is what the function [`plugin!`](crate::plugin!) declares calls; a plugin does not
/// call it itself. A registry from another copy of the kernel, which a
/// plugin built without the workspace's dynamic linking would see, is
/// refused before anything runs. A panic in `ctor` or in `Extension::register` is caught
/// here, on the plugin's side of the `extern "C"` boundary, where unwinding
/// is still allowed, and turned into [`Status::Panicked`] with the message
/// stored in the registry. The panic payload is dropped before returning,
/// so nothing of the plugin's survives in the host once it unloads the
/// library.
pub fn register(reg: &mut Registry, ctor: impl FnOnce() -> Arc<dyn Extension>) -> Status {
    if !reg.shares_kernel() {
        reg.set_failure(
            "the plugin carries its own copy of the kernel crates, so its jobs and tracing \
             would not be the host's; build it in the workspace, with its `.cargo/config.toml`"
                .to_owned(),
        );
        return Status::Failed;
    }
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let extension = ctor();
        reg.load(&*extension)
    }));
    match outcome {
        Ok(Ok(())) => Status::Registered,
        Ok(Err(error)) => {
            reg.set_failure(error.to_string());
            Status::Failed
        }
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "no message".to_owned());
            drop(payload);
            reg.set_failure(format!("panicked during registration: {message}"));
            Status::Panicked
        }
    }
}

/// Declares the plugin entry point.
///
/// Takes an expression that builds the extension, `|| Arc::new(…) as
/// Arc<dyn Extension>`, and defines the exported `rick_plugin_register`
/// function around [`register`]. Use it once, at the top level of the
/// plugin crate. `#[no_mangle]` is the one unsafe attribute a plugin needs
/// and the macro carries the allowance for it.
#[macro_export]
macro_rules! plugin {
    ($ctor:expr) => {
        /// The plugin's entry point; the host calls it once with a registry.
        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "C" fn rick_plugin_register(
            registry: &mut $crate::Registry,
        ) -> $crate::plugin::Status {
            $crate::plugin::register(registry, $ctor)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Capabilities;
    use crate::registry::{API_VERSION, Manifest};
    use semver::Version;
    use types::{Error, ExtId};

    struct Fine(Manifest);

    impl Extension for Fine {
        fn manifest(&self) -> &Manifest {
            &self.0
        }
        fn register(&self, _reg: &mut Registry) -> Result<(), Error> {
            Ok(())
        }
    }

    fn manifest(api_version: u32) -> Manifest {
        Manifest {
            id: ExtId("fine"),
            name: "Fine",
            version: Version::new(0, 1, 0),
            api_version,
            requires: Capabilities::empty(),
        }
    }

    #[test]
    fn a_registering_extension_is_registered() {
        let mut reg = Registry::new();
        let status = register(&mut reg, || Arc::new(Fine(manifest(API_VERSION))));
        assert_eq!(status, Status::Registered);
        assert_eq!(reg.extensions(), &[ExtId("fine")]);
        assert_eq!(reg.failure(), None);
    }

    #[test]
    fn a_refused_manifest_fails_with_the_reason() {
        let mut reg = Registry::new();
        let status = register(&mut reg, || Arc::new(Fine(manifest(API_VERSION + 1))));
        assert_eq!(status, Status::Failed);
        assert!(reg.extensions().is_empty());
        assert!(
            reg.failure().is_some_and(|r| r.contains("API version")),
            "{reg:?}"
        );
    }

    #[test]
    fn a_registry_of_another_kernel_is_refused() {
        let mut reg = Registry::from_another_kernel();
        let status = register(&mut reg, || Arc::new(Fine(manifest(API_VERSION))));
        assert_eq!(status, Status::Failed);
        assert!(reg.extensions().is_empty());
        assert!(
            reg.failure()
                .is_some_and(|r| r.contains("own copy of the kernel")),
            "{reg:?}"
        );
        assert!(Registry::new().shares_kernel());
    }

    #[test]
    fn a_panic_is_caught_and_reported() {
        let mut reg = Registry::new();
        let status = register(&mut reg, || -> Arc<dyn Extension> {
            panic!("no registry for me")
        });
        assert_eq!(status, Status::Panicked);
        assert_eq!(
            reg.failure(),
            Some("panicked during registration: no registry for me")
        );
        let status = register(&mut reg, || -> Arc<dyn Extension> {
            panic!("formatted {}", 42)
        });
        assert_eq!(status, Status::Panicked);
        assert_eq!(
            reg.failure(),
            Some("panicked during registration: formatted 42")
        );
    }
}
