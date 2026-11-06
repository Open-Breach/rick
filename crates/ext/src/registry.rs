//! Manifests, the `Extension` trait and the `Registry`.
//!
//! A packaged feature implements [`Extension`]: a [`Manifest`] (id, name,
//! version, the API version it was built against, and the provider
//! capabilities it needs) and `register`, which contributes to any of the five
//! extension points. Grouping is by domain, not by point: a Git extension
//! registers one decorator, several actions and a panel from one manifest.
//!
//! The [`Registry`] collects the contributions and refuses a manifest whose
//! `api_version` is not [`API_VERSION`], so a plugin built against another
//! contract is reported instead of misbehaving.
//!
//! # Worked example
//!
//! One extension contributing to two points.
//!
//! ```
//! use ext::{
//!     API_VERSION, Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability,
//!     Capabilities, ColumnDef, DecorateCtx, Decoration, Decorator, Extension, Manifest,
//!     PanelSection, Registry, SettingField, SettingKind, SettingsSchema, Shortcut, UiHost,
//!     Version,
//! };
//! use runtime::JobRunner;
//! use std::sync::Arc;
//! use types::{Entry, EntryId, Error, ExtId, Location};
//!
//! struct Badges;
//! impl Decorator for Badges {
//!     fn id(&self) -> ExtId { ExtId("demo") }
//!     fn applies_to(&self, _loc: &Location) -> bool { true }
//!     fn columns(&self) -> &[ColumnDef] { &[] }
//!     fn decorate(&self, _entries: &[Entry], _ctx: &DecorateCtx) -> Vec<(EntryId, Decoration)> { Vec::new() }
//!     fn panel(&self, _entry: &Entry) -> Option<PanelSection> { None }
//!     fn invalidates_on(&self, _loc: &Location) -> Vec<Location> { Vec::new() }
//!     fn invalidate(&self, _loc: &Location) {}
//! }
//!
//! struct Hello;
//! impl Action for Hello {
//!     fn id(&self) -> ActionId { ActionId("demo.hello") }
//!     fn label(&self) -> &str { "Say hello" }
//!     fn group(&self) -> ActionGroup { ActionGroup::Extension("Demo") }
//!     fn default_shortcut(&self) -> Option<Shortcut> { None }
//!     fn availability(&self, _ctx: &ActionCtx) -> Availability { Availability::Enabled }
//!     fn run(&self, _ctx: ActionCtx, _jobs: &JobRunner, _ui: &dyn UiHost) -> Result<ActionOutcome, Error> {
//!         Ok(ActionOutcome::Done)
//!     }
//! }
//!
//! struct Demo(Manifest);
//!
//! impl Extension for Demo {
//!     fn manifest(&self) -> &Manifest {
//!         &self.0
//!     }
//!
//!     fn register(&self, reg: &mut Registry) -> Result<(), Error> {
//!         reg.decorator(Arc::new(Badges));
//!         reg.action(Arc::new(Hello));
//!         reg.settings(SettingsSchema {
//!             ext: self.0.id,
//!             fields: vec![SettingField {
//!                 key: "greeting",
//!                 label: "Greeting",
//!                 kind: SettingKind::Text { default: "hello".into() },
//!             }],
//!         });
//!         Ok(())
//!     }
//! }
//!
//! let manifest = |api_version| Manifest {
//!     id: ExtId("demo"),
//!     name: "Demo",
//!     version: Version::new(0, 1, 0),
//!     api_version,
//!     requires: Capabilities::empty(),
//! };
//!
//! let mut registry = Registry::new();
//! registry.load(&Demo(manifest(API_VERSION))).unwrap();
//! assert_eq!(registry.decorators().len(), 1);
//! assert_eq!(registry.actions().len(), 1);
//! assert_eq!(registry.settings_schemas().len(), 1);
//! assert_eq!(registry.extensions(), &[ExtId("demo")]);
//!
//! // A manifest from another contract version is refused, and so is a
//! // second load of the same id.
//! assert!(matches!(registry.load(&Demo(manifest(API_VERSION + 1))), Err(Error::Unsupported(_))));
//! assert!(matches!(registry.load(&Demo(manifest(API_VERSION))), Err(Error::Unsupported(_))));
//! assert_eq!(registry.actions().len(), 1);
//! ```

use std::sync::Arc;

use semver::Version;
use types::{Error, ExtId};

use crate::action::Action;
use crate::decorator::Decorator;
use crate::preview::Previewer;
use crate::provider::{Capabilities, Provider};
use crate::search::SearchIndex;

/// Version of the extension contract: the traits in this crate and the
/// manifest format. Any breaking change bumps it.
///
/// History: 1 the first contract; 2 `PreviewDoc::info` became `sections`,
/// with `status`, `controls` and `set_control` added.
pub const API_VERSION: u32 = 2;

/// What an extension declares about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Unique id, also the name of the extension's settings file.
    pub id: ExtId,
    /// Human-readable name.
    pub name: &'static str,
    /// The extension's own version.
    pub version: Version,
    /// The [`API_VERSION`] the extension was built against.
    pub api_version: u32,
    /// Provider capabilities the extension needs to be useful.
    pub requires: Capabilities,
}

/// How a setting is edited and stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingKind {
    /// A checkbox.
    Bool {
        /// Value when unset.
        default: bool,
    },
    /// A text field.
    Text {
        /// Value when unset.
        default: String,
    },
    /// A whole number within bounds.
    Integer {
        /// Value when unset.
        default: i64,
        /// Smallest accepted value.
        min: i64,
        /// Largest accepted value.
        max: i64,
    },
    /// One of a fixed list.
    Choice {
        /// The options, shown in order.
        options: Vec<&'static str>,
        /// Index into `options` when unset.
        default: usize,
    },
}

/// One setting of an extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingField {
    /// Key in the extension's TOML file.
    pub key: &'static str,
    /// Label in the Settings screen.
    pub label: &'static str,
    /// How it is edited.
    pub kind: SettingKind,
}

/// The settings an extension exposes; rendered in the Settings screen and
/// stored in the extension's own TOML file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsSchema {
    /// The extension the settings belong to.
    pub ext: ExtId,
    /// The settings, in display order.
    pub fields: Vec<SettingField>,
}

/// A packaged feature: a manifest and what it contributes.
pub trait Extension: Send + Sync + 'static {
    /// The manifest.
    fn manifest(&self) -> &Manifest;

    /// Contributes to any of the five extension points.
    ///
    /// # Errors
    ///
    /// Fails when the extension cannot set itself up, for instance when a
    /// resource it needs is missing; nothing it registered before the error
    /// is kept.
    fn register(&self, reg: &mut Registry) -> Result<(), Error>;
}

/// Collects contributions from extensions.
#[derive(Default)]
pub struct Registry {
    extensions: Vec<ExtId>,
    providers: Vec<Arc<dyn Provider>>,
    decorators: Vec<Arc<dyn Decorator>>,
    actions: Vec<Arc<dyn Action>>,
    previewers: Vec<Arc<dyn Previewer>>,
    search_indexes: Vec<Arc<dyn SearchIndex>>,
    settings: Vec<SettingsSchema>,
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Checks the manifest and lets the extension register.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when the manifest's `api_version` is not
    /// [`API_VERSION`] or an extension with the same id is already loaded;
    /// otherwise whatever [`Extension::register`] returns, in which case
    /// nothing the extension registered is kept.
    pub fn load(&mut self, ext: &dyn Extension) -> Result<(), Error> {
        let manifest = ext.manifest();
        if manifest.api_version != API_VERSION {
            return Err(Error::Unsupported(
                "extension built against another extension API version",
            ));
        }
        if self.extensions.contains(&manifest.id) {
            return Err(Error::Unsupported("extension id already loaded"));
        }
        let mut staging = Registry::default();
        ext.register(&mut staging)?;
        self.extensions.push(manifest.id);
        self.providers.append(&mut staging.providers);
        self.decorators.append(&mut staging.decorators);
        self.actions.append(&mut staging.actions);
        self.previewers.append(&mut staging.previewers);
        self.search_indexes.append(&mut staging.search_indexes);
        self.settings.append(&mut staging.settings);
        Ok(())
    }

    /// Registers a provider.
    pub fn provider(&mut self, p: Arc<dyn Provider>) {
        self.providers.push(p);
    }

    /// Registers a decorator.
    pub fn decorator(&mut self, d: Arc<dyn Decorator>) {
        self.decorators.push(d);
    }

    /// Registers an action.
    pub fn action(&mut self, a: Arc<dyn Action>) {
        self.actions.push(a);
    }

    /// Registers a previewer.
    pub fn previewer(&mut self, p: Arc<dyn Previewer>) {
        self.previewers.push(p);
    }

    /// Registers a search index.
    pub fn search_index(&mut self, s: Arc<dyn SearchIndex>) {
        self.search_indexes.push(s);
    }

    /// Declares settings; they render in the Settings screen and are stored
    /// in the extension's TOML file.
    pub fn settings(&mut self, schema: SettingsSchema) {
        self.settings.push(schema);
    }

    /// Ids of the extensions loaded so far, in load order.
    #[must_use]
    pub fn extensions(&self) -> &[ExtId] {
        &self.extensions
    }

    /// Registered providers, in registration order.
    #[must_use]
    pub fn providers(&self) -> &[Arc<dyn Provider>] {
        &self.providers
    }

    /// Registered decorators, in registration order.
    #[must_use]
    pub fn decorators(&self) -> &[Arc<dyn Decorator>] {
        &self.decorators
    }

    /// Registered actions, in registration order.
    #[must_use]
    pub fn actions(&self) -> &[Arc<dyn Action>] {
        &self.actions
    }

    /// Registered previewers, in registration order.
    #[must_use]
    pub fn previewers(&self) -> &[Arc<dyn Previewer>] {
        &self.previewers
    }

    /// Registered search indexes, in registration order.
    #[must_use]
    pub fn search_indexes(&self) -> &[Arc<dyn SearchIndex>] {
        &self.search_indexes
    }

    /// Declared settings schemas, in registration order.
    #[must_use]
    pub fn settings_schemas(&self) -> &[SettingsSchema] {
        &self.settings
    }
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("extensions", &self.extensions)
            .field("providers", &self.providers.len())
            .field("decorators", &self.decorators.len())
            .field("actions", &self.actions.len())
            .field("previewers", &self.previewers.len())
            .field("search_indexes", &self.search_indexes.len())
            .field("settings", &self.settings.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Broken(Manifest);

    impl Extension for Broken {
        fn manifest(&self) -> &Manifest {
            &self.0
        }
        fn register(&self, reg: &mut Registry) -> Result<(), Error> {
            reg.settings(SettingsSchema {
                ext: self.0.id,
                fields: Vec::new(),
            });
            Err(Error::Unsupported("needs a resource that is missing"))
        }
    }

    #[test]
    fn a_failing_register_leaves_nothing_behind() {
        let mut registry = Registry::new();
        let result = registry.load(&Broken(Manifest {
            id: ExtId("broken"),
            name: "Broken",
            version: Version::new(0, 1, 0),
            api_version: API_VERSION,
            requires: Capabilities::empty(),
        }));
        assert!(matches!(result, Err(Error::Unsupported(_))));
        assert_eq!(registry.extensions(), [] as [ExtId; 0]);
        assert_eq!(registry.settings_schemas(), [] as [SettingsSchema; 0]);
        assert!(format!("{registry:?}").contains("extensions: []"));
    }
}
