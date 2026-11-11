//! The subdirectory search as a plugin: the `ignore` crate's parallel
//! walker, streaming the entries under a location whose name contains the
//! text, behind the [`SearchIndex`] extension point.
//!
//! The index's scope is [`SearchScope::Under`] the local provider's root
//! (an empty path, the whole tree), so it answers only a [`Query`] with a
//! `root` and the filter bar's *Search in subdirectories* goes through it
//! with the pane's location as the root; the title-row search box queries
//! the platform indexes, whose scope is `Everywhere`, and leaves it alone
//! (QUESTIONS.md 118). The pane's hidden-files toggle and the `[search]
//! ignore` list of `rick.toml` reach the walk as the query's
//! [`Filter::NoHidden`](ext::Filter::NoHidden) and
//! [`Filter::SkipNames`](ext::Filter::SkipNames) (QUESTIONS.md 119); the
//! other filters narrow what the walk reports.
//!
//! Matches arrive in batches as the walk proceeds, so the first screen
//! shows before the walk finishes; cancelling the token ends the walk
//! within one directory per thread. In 0.1 the same walk lived in
//! `vfs-local`.

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

pub mod walk;

use std::sync::Arc;

use ext::{
    API_VERSION, Availability, BoxStream, Capabilities, Extension, Manifest, Platforms, Points,
    Query, Registry, SearchHit, SearchIndex, SearchScope, Version,
};
use types::{CancelToken, Error, ExtId, Location, ProviderId, VPath};

/// The extension id.
pub const ID: ExtId = ExtId("search-walk");

/// The provider the walk searches: the local filesystem.
pub const PROVIDER_ID: ProviderId = ProviderId("local");

/// The extension: registers the one index.
#[derive(Debug)]
pub struct WalkExtension {
    manifest: Manifest,
}

impl Default for WalkExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl WalkExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Subdirectory search",
                description: "Searches the folder and its subfolders by walking them: the search in subdirectories of the filter bar.",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::SEARCH_INDEX,
                platforms: Platforms::ALL,
                previews: &[],
                commands: &[],
            },
        }
    }
}

impl Extension for WalkExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.search_index(Arc::new(WalkIndex));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(WalkExtension::new()) as Arc<dyn Extension>);

/// The walk as a search index: always available, scoped under the local
/// root, answering a query by walking its root.
#[derive(Debug, Clone, Copy, Default)]
pub struct WalkIndex;

impl SearchIndex for WalkIndex {
    fn id(&self) -> ExtId {
        ID
    }

    fn availability(&self) -> Availability {
        Availability::Enabled
    }

    fn scope(&self) -> SearchScope {
        SearchScope::Under(Location::new(PROVIDER_ID, VPath::new()))
    }

    fn query(
        &self,
        q: &Query,
        cancel: CancelToken,
    ) -> BoxStream<'static, Result<Vec<SearchHit>, Error>> {
        walk::search(q, cancel)
    }
}

#[cfg(test)]
mod tests;
