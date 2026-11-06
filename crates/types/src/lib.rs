//! Value types shared by every Rick crate.
//!
//! This crate is the bottom of the dependency graph: it performs no I/O,
//! spawns no threads and contains no platform-specific code. Everything in it
//! is `Send + Sync` so values can cross job and thread boundaries freely.
//!
//! The shapes defined here are the binding contract from SPEC.md, section
//! "Core types and extension traits", block "Value types":
//!
//! - identities: [`ProviderId`], [`ExtId`], [`EntryId`], [`JobId`]
//! - places: [`VPath`], [`Location`]
//! - directory entries: [`Entry`], [`EntryKind`], [`Attrs`], [`Mime`]
//! - long operations: [`CancelToken`], [`Progress`]
//! - failure: [`Error`]

#![forbid(unsafe_code)]

mod cancel;
mod entry;
mod error;
mod id;
mod path;
mod progress;

pub use cancel::{CancelToken, Cancelled};
pub use entry::{Attrs, Entry, EntryKind, Mime};
pub use error::Error;
pub use id::{EntryId, ExtId, JobId, ProviderId};
pub use path::{Location, VPath};
pub use progress::Progress;

#[cfg(test)]
mod assertions {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn every_value_type_is_send_and_sync() {
        assert_send_sync::<ProviderId>();
        assert_send_sync::<ExtId>();
        assert_send_sync::<EntryId>();
        assert_send_sync::<JobId>();
        assert_send_sync::<VPath>();
        assert_send_sync::<Location>();
        assert_send_sync::<Entry>();
        assert_send_sync::<EntryKind>();
        assert_send_sync::<Attrs>();
        assert_send_sync::<Mime>();
        assert_send_sync::<CancelToken>();
        assert_send_sync::<Cancelled>();
        assert_send_sync::<Progress>();
        assert_send_sync::<Error>();
    }
}
