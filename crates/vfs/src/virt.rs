//! The `rick` provider: Rick's own virtual locations under one scheme.
//!
//! `rick://trash`, `rick://favorites`, `rick://recent` and the overviews
//! are [`Location`]s of the provider [`ProviderId::RICK`] whose path's first
//! component names a *view*. Each view is a [`Provider`] an extension
//! registered through `ext::Registry::view`; [`Virtual`] routes every call
//! for a location under `/<name>` to that provider, unchanged, so the view
//! works with full paths and hands back entries that carry them. The root,
//! `rick://`, lists the views as folders through their own `stat`.
//!
//! The scheme and its display form are the implementer's call (SPEC.md
//! "Virtual locations", QUESTIONS.md 87): the location `rick:/trash` shows
//! as `rick://trash`.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use ext::{
    Capabilities, ChangeStream, CopyOptions, EntryStream, LinkKind, ListOptions, Provider,
    ReadHandle, RemoveMode, WriteHandle, WriteOptions, async_trait,
};
use futures::{StreamExt, stream};
use runtime::ProgressSink;
use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, Location, ProviderId, VPath};

/// The scheme virtual locations show under: `rick://trash`.
pub const SCHEME: &str = "rick://";

/// The name of the view a `rick` location's path is under: `"trash"` for
/// `/trash` and `/trash/x`; `None` for the root and for a path that is not
/// UTF-8 up to its first separator.
#[must_use]
pub fn view_name(path: &VPath) -> Option<&str> {
    let bytes = path.as_bytes();
    let bytes = bytes
        .strip_prefix(b"/")
        .or_else(|| bytes.strip_prefix(b"\\"))?;
    let end = bytes
        .iter()
        .position(|b| matches!(b, b'/' | b'\\'))
        .unwrap_or(bytes.len());
    let name = std::str::from_utf8(&bytes[..end]).ok()?;
    (!name.is_empty()).then_some(name)
}

/// The location of the view `name`: `rick://<name>`.
#[must_use]
pub fn view_location(name: &str) -> Location {
    Location::new(ProviderId::RICK, format!("/{name}"))
}

/// The display form of a `rick` location: `rick://trash`, `rick://trash/x`,
/// `rick://` for the root.
#[must_use]
pub fn display(path: &VPath) -> String {
    let text = path.to_string_lossy();
    let rest = text
        .strip_prefix('/')
        .or_else(|| text.strip_prefix('\\'))
        .unwrap_or(&text);
    format!("{SCHEME}{rest}")
}

/// Parses the display form back: `rick://trash/x` is `rick:/trash/x`.
#[must_use]
pub fn parse(text: &str) -> Option<Location> {
    let rest = text.strip_prefix(SCHEME)?;
    let rest = rest.trim_start_matches('/');
    Some(Location::new(ProviderId::RICK, format!("/{rest}")))
}

/// A stable id for a view's root entry: the hash of its name, so it is the
/// same across listings and processes.
#[must_use]
pub fn view_id(name: &str) -> EntryId {
    // FNV-1a over the name, folded into the upper half so a view's own
    // keys, which start at zero, cannot collide with it.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    EntryId::new(ProviderId::RICK, u128::from(hash) << 64)
}

/// The `rick` provider: routes each view's subtree to its provider.
#[derive(Default)]
pub struct Virtual {
    views: RwLock<BTreeMap<String, Arc<dyn Provider>>>,
}

impl Virtual {
    /// A provider with no views yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn table(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<String, Arc<dyn Provider>>> {
        // Plain handles; a panic elsewhere cannot corrupt them.
        self.views
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Adds a view under `name`, replacing one with the same name.
    pub fn add(&self, name: &str, provider: Arc<dyn Provider>) {
        self.views
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(name.to_owned(), provider);
    }

    /// The provider serving the view `name`, if any.
    #[must_use]
    pub fn view(&self, name: &str) -> Option<Arc<dyn Provider>> {
        self.table().get(name).cloned()
    }

    /// The names of the views, sorted.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.table().keys().cloned().collect()
    }

    /// The provider that serves `loc`.
    fn route(&self, loc: &Location) -> Result<Arc<dyn Provider>, Error> {
        if loc.provider != ProviderId::RICK {
            return Err(Error::Unsupported("location belongs to another provider"));
        }
        let name = view_name(&loc.path).ok_or(Error::NotFound)?;
        self.view(name).ok_or(Error::NotFound)
    }

    fn is_root(loc: &Location) -> bool {
        loc.provider == ProviderId::RICK && view_name(&loc.path).is_none()
    }

    /// The root entry: `rick://` itself.
    fn root_entry() -> Entry {
        Entry {
            id: view_id(""),
            name: "Rick".to_owned(),
            path: VPath::from("/"),
            kind: EntryKind::Dir,
            size: None,
            created: None,
            modified: None,
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        }
    }
}

impl std::fmt::Debug for Virtual {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Virtual")
            .field("views", &self.names())
            .finish()
    }
}

#[async_trait]
impl Provider for Virtual {
    fn id(&self) -> ProviderId {
        ProviderId::RICK
    }

    fn capabilities(&self, loc: &Location) -> Capabilities {
        self.route(loc)
            .map(|view| view.capabilities(loc))
            .unwrap_or_default()
    }

    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken) -> EntryStream {
        if Self::is_root(loc) {
            // The views as folders, each described by its own provider.
            let views: Vec<(String, Arc<dyn Provider>)> = self
                .table()
                .iter()
                .map(|(name, provider)| (name.clone(), Arc::clone(provider)))
                .collect();
            return stream::once(async move {
                let mut entries = Vec::with_capacity(views.len());
                for (name, view) in views {
                    if cancel.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    match view.stat(&view_location(&name)).await {
                        Ok(entry) => entries.push(entry),
                        Err(error) => tracing::debug!(%name, %error, "view not described"),
                    }
                }
                Ok(entries)
            })
            .boxed();
        }
        match self.route(loc) {
            Ok(view) => view.list(loc, opts, cancel),
            Err(error) => stream::once(async move { Err(error) }).boxed(),
        }
    }

    async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
        if Self::is_root(loc) {
            return Ok(Self::root_entry());
        }
        self.route(loc)?.stat(loc).await
    }

    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
        self.route(loc)?.open_read(loc).await
    }

    async fn open_write(&self, loc: &Location, opts: WriteOptions) -> Result<WriteHandle, Error> {
        self.route(loc)?.open_write(loc, opts).await
    }

    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error> {
        self.route(loc)?.create_dir(loc).await
    }

    async fn rename(&self, from: &Location, to: &Location) -> Result<Entry, Error> {
        let view = self.route(from)?;
        if view_name(&from.path) != view_name(&to.path) {
            return Err(Error::Unsupported("cannot rename across views"));
        }
        view.rename(from, to).await
    }

    async fn remove(&self, loc: &Location, mode: RemoveMode) -> Result<(), Error> {
        self.route(loc)?.remove(loc, mode).await
    }

    async fn link(&self, target: &Location, at: &Location, kind: LinkKind) -> Result<Entry, Error> {
        self.route(at)?.link(target, at, kind).await
    }

    async fn copy(
        &self,
        from: &Location,
        to: &Location,
        opts: CopyOptions,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        let view = self.route(from)?;
        if view_name(&from.path) != view_name(&to.path) {
            return Err(Error::Unsupported("cannot copy across views"));
        }
        view.copy(from, to, opts, progress, cancel).await
    }

    fn watch(&self, loc: &Location) -> Result<ChangeStream, Error> {
        if Self::is_root(loc) {
            return Err(Error::Unsupported("the list of views does not change"));
        }
        self.route(loc)?.watch(loc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_names_and_display_forms() {
        assert_eq!(view_name(&VPath::from("/trash")), Some("trash"));
        assert_eq!(view_name(&VPath::from("/trash/a/b")), Some("trash"));
        assert_eq!(view_name(&VPath::from("\\trash\\a")), Some("trash"));
        assert_eq!(view_name(&VPath::from("/")), None);
        assert_eq!(view_name(&VPath::from("")), None);
        assert_eq!(view_name(&VPath::from_bytes(vec![b'/', 0xFF])), None);
        assert_eq!(view_location("trash").to_string(), "rick:/trash");
        assert_eq!(display(&VPath::from("/trash")), "rick://trash");
        assert_eq!(display(&VPath::from("/trash/x")), "rick://trash/x");
        assert_eq!(display(&VPath::from("/")), "rick://");
        assert_eq!(parse("rick://trash/x"), Some(view_location("trash/x")));
        assert_eq!(parse("rick://"), Some(Location::new(ProviderId::RICK, "/")));
        assert_eq!(parse("/home"), None);
        assert_eq!(view_id("trash"), view_id("trash"));
        assert_ne!(view_id("trash"), view_id("recent"));
        assert_eq!(view_id("trash").key & u128::from(u64::MAX), 0);
    }
}
