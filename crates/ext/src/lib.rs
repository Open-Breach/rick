//! The extension API: everything an extension sees, and nothing else.
//!
//! Rick is a small kernel with five extension points. Each is a trait in this
//! crate, small enough for one screen, that returns data or jobs and never
//! draws:
//!
//! | Point | Trait | Answers |
//! | --- | --- | --- |
//! | Provider | [`Provider`] | where files come from |
//! | Decorator | [`Decorator`] | status on entries or a directory |
//! | Action | [`Action`] | a function for a selection or location |
//! | Previewer | [`Previewer`] | rendering a file type |
//! | `SearchIndex` | [`SearchIndex`] | querying a file search index |
//!
//! A packaged feature implements [`Extension`]: a [`Manifest`] and a
//! `register` method that contributes to any of the five points through the
//! [`Registry`]. Built-ins register exactly like plugins. The contract is
//! versioned by [`API_VERSION`]; the registry refuses a manifest of another
//! version.
//!
//! Each module's documentation carries a worked example of its extension
//! point; [`registry`] shows how one extension contributes to two of them.
//!
//! Async trait methods use [`macro@async_trait`] so the traits stay usable as
//! `dyn` objects (`Arc<dyn Provider>`), which native `async fn` in traits
//! does not allow yet; streamed results are [`BoxStream`]s.

#![forbid(unsafe_code)]
// The spec fixes these names (`ActionCtx`, `SearchHit`, `PreviewDoc`, …) and
// the module split mirrors the five extension points.
#![allow(clippy::module_name_repetitions)]

pub mod action;
pub mod decorator;
pub mod preview;
pub mod provider;
pub mod registry;
pub mod search;

pub use action::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Invocation, Key, Level,
    Modifiers, PaneId, Shortcut, ShortcutParseError, UiHost, UndoCommand,
};
pub use async_trait::async_trait;
pub use decorator::{
    Badge, CellValue, Color, ColumnDef, ColumnId, DecorateCtx, Decoration, Decorator, PanelSection,
};
pub use futures::stream::BoxStream;
pub use preview::{
    Frame, PreviewDoc, Previewer, RenderRequest, TextBlock, TextLayer, TextureId, Transport,
};
pub use provider::{
    Capabilities, ChangeStream, CopyOptions, Depth, EntryStream, FsChange, LinkKind, ListOptions,
    Provider, ReadHandle, RemoveMode, WriteHandle, WriteOptions,
};
pub use registry::{
    API_VERSION, Extension, Manifest, Registry, SettingField, SettingKind, SettingsSchema,
};
pub use search::{Filter, Query, QueryKind, SearchHit, SearchIndex, SearchScope};
pub use semver::Version;
