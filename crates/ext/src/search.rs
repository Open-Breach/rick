//! The `SearchIndex` extension point: querying a file search index.
//!
//! Rick builds no index of its own. A search index connects to one the
//! platform has (Windows Search, Spotlight, Tracker, Baloo) and streams hits
//! for a query as a cancellable job. `availability` is checked at start and
//! when the service changes; when no index is available the title-row
//! search box searches the current location alone, as it did in 0.1.
//!
//! Two kinds of index share the trait (QUESTIONS.md 118). A platform index
//! has [`SearchScope::Everywhere`]: it answers for the whole machine, with
//! or without a `root`, and the title-row search box queries every
//! available one. A walk such as `ext-search-walk` has
//! [`SearchScope::Under`] a provider's root (an empty path stands for the
//! provider's whole tree): it needs the query's `root` and streams the
//! entries it finds below it, and the filter bar's *Search in
//! subdirectories* goes through the one whose scope covers the pane's
//! location.
//!
//! # Worked example
//!
//! An index over a fixed list of entries that matches on the name.
//!
//! ```
//! use ext::{Availability, Query, QueryKind, SearchHit, SearchIndex, SearchScope};
//! use futures::{StreamExt, stream};
//! use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, ExtId, ProviderId};
//!
//! struct Names(Vec<Entry>);
//!
//! impl SearchIndex for Names {
//!     fn id(&self) -> ExtId {
//!         ExtId("search-names")
//!     }
//!
//!     fn availability(&self) -> Availability {
//!         Availability::Enabled
//!     }
//!
//!     fn scope(&self) -> SearchScope {
//!         SearchScope::Everywhere
//!     }
//!
//!     fn query(&self, q: &Query, _cancel: CancelToken) -> ext::BoxStream<'static, Result<Vec<SearchHit>, Error>> {
//!         let needle = q.text.to_lowercase();
//!         let hits: Vec<SearchHit> = self
//!             .0
//!             .iter()
//!             .filter(|e| e.name.to_lowercase().contains(&needle))
//!             .map(|e| SearchHit { entry: e.clone(), snippet: None })
//!             .collect();
//!         stream::iter(vec![Ok(hits)]).boxed()
//!     }
//! }
//!
//! let entry = |id: u128, name: &str| Entry {
//!     id: EntryId::new(ProviderId("local"), id),
//!     name: name.into(),
//!     path: format!("/{name}").into(),
//!     kind: EntryKind::File,
//!     size: None,
//!     created: None,
//!     modified: None,
//!     accessed: None,
//!     attrs: Attrs::empty(),
//!     mime: None,
//! };
//! let index = Names(vec![entry(1, "Budget.xlsx"), entry(2, "notes.txt")]);
//! let query = Query { text: "budget".into(), root: None, kind: QueryKind::Name, filters: Vec::new() };
//!
//! let hits = runtime::block_on(async {
//!     index.query(&query, CancelToken::new()).next().await.transpose().unwrap().unwrap()
//! });
//! assert_eq!(hits.len(), 1);
//! assert_eq!(hits[0].entry.name, "Budget.xlsx");
//! ```

use std::time::SystemTime;

use futures::stream::BoxStream;
use types::{CancelToken, Entry, Error, ExtId, Location};

use crate::action::Availability;

/// Where an index can search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchScope {
    /// The whole machine.
    Everywhere,
    /// Only below one location, which every query's `root` must lie in; a
    /// location with an empty path is the provider's whole tree.
    Under(Location),
}

impl SearchScope {
    /// Whether a query rooted at `root` lies within this scope: always for
    /// [`Everywhere`](Self::Everywhere); for [`Under`](Self::Under) when
    /// the providers match and `root` is the scope's location or below it.
    #[must_use]
    pub fn covers(&self, root: &Location) -> bool {
        match self {
            Self::Everywhere => true,
            Self::Under(under) => {
                under.provider == root.provider
                    && (under.path.is_empty() || root.path.is_inside(&under.path))
            }
        }
    }
}

/// What a query matches against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryKind {
    /// Entry names only.
    Name,
    /// File contents only.
    Content,
    /// Either.
    Both,
}

/// Narrows a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    /// Name ends with this extension, without the dot.
    Extension(String),
    /// At least this many bytes.
    MinSize(u64),
    /// At most this many bytes.
    MaxSize(u64),
    /// Modified after this time.
    ModifiedAfter(SystemTime),
    /// Modified before this time.
    ModifiedBefore(SystemTime),
    /// Leave hidden entries out, and never descend into a hidden directory:
    /// the pane's hidden-files toggle, off.
    NoHidden,
    /// Never descend into or report an entry with one of these names: the
    /// `[search] ignore` list of `rick.toml` (`.git`, `node_modules`,
    /// `target`). An index that has already descended drops the hits whose
    /// path has such a component.
    SkipNames(Vec<String>),
}

/// A search request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// What to look for.
    pub text: String,
    /// Restrict to this subtree; `None` is the index's whole scope.
    pub root: Option<Location>,
    /// Names, contents or both.
    pub kind: QueryKind,
    /// Further restrictions, all of which must hold.
    pub filters: Vec<Filter>,
}

/// One result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// The matching entry.
    pub entry: Entry,
    /// A content excerpt around the match, for content queries.
    pub snippet: Option<String>,
}

/// A query backend.
pub trait SearchIndex: Send + Sync + 'static {
    /// The extension this index belongs to.
    fn id(&self) -> ExtId;

    /// Whether the platform backend is present.
    fn availability(&self) -> Availability;

    /// What the index covers.
    fn scope(&self) -> SearchScope;

    /// Streams hits in batches until the query is answered or cancelled.
    fn query(
        &self,
        q: &Query,
        cancel: CancelToken,
    ) -> BoxStream<'static, Result<Vec<SearchHit>, Error>>;
}
