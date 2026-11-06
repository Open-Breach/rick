//! Integration tests and the generated fixture tree for Rick.
//!
//! The crate exposes [`fixture`], which generates a deterministic directory
//! tree of N entries for tests and benchmarks. The integration tests
//! themselves live under `tests/`.

#![forbid(unsafe_code)]

pub mod fixture;
