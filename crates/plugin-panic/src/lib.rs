//! A plugin whose entry point panics: the fixture for the host's promise
//! that a plugin panicking during registration is caught, unloaded and
//! reported, and never takes the host down.

#![deny(unsafe_code)]

use std::sync::Arc;

use ext::Extension;

/// The message the fixture panics with.
pub const MESSAGE: &str = "the panic fixture refuses to register";

ext::plugin!(|| -> Arc<dyn Extension> { panic!("{MESSAGE}") });
