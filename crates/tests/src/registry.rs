//! A plugin registry of the tests' own (issue 251): the index a release
//! would publish, built from the plugins' manifests, for the server and
//! the key of `plugins_registry::fixtures`.
//!
//! No test asks the real registry. A test starts a [`Server`], publishes
//! an [`index`] on it under a [`Signer`] of its own, and writes
//! [`rick_toml`] into the config directory of the Rick it starts.

use plugins_registry::fixtures::{Server, Signer};
use serde_json::{Value, json};

/// The version of Rick the registry tests ask as, and the builds of their
/// indexes are for.
pub const RICK: &str = "0.6.0";

/// A plugin of an index, as the publisher writes it from a release's
/// package manifest: what `manifest` says the plugin is and is for, with
/// its `builds`.
#[must_use]
pub fn plugin(manifest: &ext::Manifest, builds: &[Value]) -> Value {
    let connections: Vec<Value> = manifest
        .connections
        .iter()
        .map(|spec| json!({ "section": spec.section.name(), "title": spec.title }))
        .collect();
    // The triggers hold the lists that are not empty.
    let mut triggers = serde_json::Map::new();
    if !manifest.previews.is_empty() {
        triggers.insert("previews".to_owned(), json!(manifest.previews));
    }
    if !manifest.markers.is_empty() {
        triggers.insert("markers".to_owned(), json!(manifest.markers));
    }
    if !connections.is_empty() {
        triggers.insert("connections".to_owned(), json!(connections));
    }
    json!({
        "id": manifest.id.0,
        "name": manifest.name,
        "description": manifest.description,
        "publisher": "Open-Breach",
        "meta": { "points": manifest.points.names(), "triggers": triggers },
        "builds": builds,
    })
}

/// A build for this platform and a Rick of version `rick`: the package at
/// `url`, of `size` bytes and with the SHA-256 `sha256`, holding the
/// plugin in version `version`.
#[must_use]
pub fn build(version: &str, rick: &str, url: &str, size: u64, sha256: &str) -> Value {
    json!({
        "version": version,
        "app_version": rick,
        "api_version": ext::API_VERSION,
        "platform": plugins_registry::platform(),
        "url": url,
        "size": size,
        "sha256": sha256,
    })
}

/// A build for this platform and [`RICK`] that is only listed: a package
/// of `size` bytes that no server has.
#[must_use]
pub fn listed(version: &str, size: u64) -> Value {
    build(
        version,
        RICK,
        "https://example.invalid/package.rkp",
        size,
        &"00".repeat(32),
    )
}

/// An index of `plugins`.
#[must_use]
pub fn index(plugins: &[Value]) -> Value {
    json!({
        "schema": 1,
        "generated": "2026-11-19T02:00:00Z",
        "apps": { "rick": { "plugins": plugins } },
    })
}

/// [`index`] that names `version` as the newest release of Rick, with
/// `url` as its release page (`apps.rick.latest`).
#[must_use]
pub fn index_with_latest(plugins: &[Value], version: &str, url: &str) -> Value {
    let mut index = index(plugins);
    index["apps"]["rick"]["latest"] = json!({ "version": version, "url": url });
    index
}

/// The background check of the plugin list, as the application's timer
/// brings it a minute after the start and whenever the list is a day old
/// (specs/plugin-distribution.md §6): a test does not wait the minute.
/// What comes of it reaches the window through the application's loop.
pub fn background_check(app: &mut app::App) {
    app.handle(
        app::Command::Registry(app::RegistryCommand::Daily),
        &mut Vec::new(),
    );
}

/// The `[registry]` table that points a Rick at `server` and the key of
/// `signer`, with *Look for plugins and updates online* as `check` has it.
#[must_use]
pub fn rick_toml(server: &Server, signer: &Signer, check: bool) -> String {
    format!(
        "[registry]\ncheck = {check}\nurl = \"{}\"\npublic_key = \"{}\"\n",
        server.index_url(),
        signer.public_key()
    )
}
