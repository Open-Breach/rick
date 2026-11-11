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
//! contract is reported instead of misbehaving. It is also the extension
//! context: the host builds it with [`Registry::with_platform`], and an
//! extension that needs the platform services (the trash, the config
//! directory, the dialogues) takes them from [`Registry::platform`] while it
//! registers. A provider can serve a virtual location, `rick://<name>`,
//! through [`Registry::view`]; the kernel's `rick` provider routes the
//! location's subtree to it.
//!
//! # Worked example
//!
//! One extension contributing to two points.
//!
//! ```
//! use ext::{
//!     API_VERSION, Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability,
//!     Capabilities, ColumnDef, DecorateCtx, Decoration, Decorator, Extension, Manifest,
//!     PanelSection, Platforms, Points, Registry, SettingField, SettingKind, SettingsSchema,
//!     Shortcut, Text, UiHost, Version, t,
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
//!     fn label(&self) -> String { t!("hello-say") }
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
//!                 label: Text("greeting"),
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
//!     description: "",
//!     version: Version::new(0, 1, 0),
//!     api_version,
//!     requires: Capabilities::empty(),
//!     points: Points::DECORATOR | Points::ACTION,
//!     platforms: Platforms::ALL,
//!     previews: &[],
//!     commands: &[],
//! };
//!
//! let mut registry = Registry::new();
//! registry.load(&Demo(manifest(API_VERSION))).unwrap();
//! assert_eq!(registry.decorators().len(), 1);
//! assert_eq!(registry.actions().len(), 1);
//! assert_eq!(registry.settings_schemas().len(), 1);
//! assert_eq!(registry.extensions(), &[ExtId("demo")]);
//! assert_eq!(registry.points(), Points::DECORATOR | Points::ACTION);
//!
//! // A manifest from another contract version is refused, and so is a
//! // second load of the same id.
//! assert!(matches!(registry.load(&Demo(manifest(API_VERSION + 1))), Err(Error::Unsupported(_))));
//! assert!(matches!(registry.load(&Demo(manifest(API_VERSION))), Err(Error::Unsupported(_))));
//! assert_eq!(registry.actions().len(), 1);
//! ```

use std::fmt;
use std::sync::Arc;

use platform::Platform;
use runtime::Prompts;
use semver::Version;
use types::{Error, ExtId};

use crate::action::Action;
use crate::cli::{CliCommand, CommandSpec};
use crate::connection::Connector;
use crate::decorator::Decorator;
use crate::preview::Previewer;
use crate::provider::{Capabilities, Provider};
use crate::search::SearchIndex;

/// Version of the extension contract: the traits in this crate and the
/// manifest format. Any breaking change bumps it.
///
/// History: 1 the first contract; 2 `PreviewDoc::info` became `sections`,
/// with `status`, `controls` and `set_control` added; 3 [`Manifest`] gained
/// `points` and `platforms`, and plugin libraries gained their entry point
/// ([`plugin`](mod@crate::plugin)) and their manifest file; 4
/// `PreviewDoc` gained `content` (file listings, grids and documents the
/// UI lays out), `Transport` gained volume, speed, frame step and loop,
/// and `Icon` gained `Rendered` (QUESTIONS.md 103); 5 `Filter` gained
/// `NoHidden` and `SkipNames`, what the subtree walk takes from the pane
/// and the settings, and `SearchScope` gained `covers` (QUESTIONS.md 119);
/// 6 `UiHost` gained `form`, `ActionOutcome` gained `Select` and `View`,
/// `ActionCtx` gained `other`, `LinkKind` gained `Junction` and `Shortcut`
/// with their capabilities, `JobCtx` gained `conflicts`, and settings gained
/// [`SettingValue`] (QUESTIONS.md 123–126); 7 `Provider` gained `transfer`
/// behind `NATIVE_TRANSFER`, `Capabilities` gained `REMOTE`, and the registry
/// takes a [`Connector`] per network provider (QUESTIONS.md 135–137); 8
/// `PreviewControl` gained `Pointer` (a drag, the wheel or a double-click
/// over the content handed to the document) and `ControlValue` gained
/// `Delta` (QUESTIONS.md 142); 9 `ActionOutcome` gained `Navigate` and
/// `platform` gained `Elevate` (QUESTIONS.md 149–150); 10 the registry
/// lends the [`Prompts`] queue for the questions an extension asks the user
/// while it works, `Previewer` gained `reads` and `Provider`
/// `remote_preview_cap` for the previews over a remote provider,
/// `Connector::schema` is computed per call and gained a `Choice` field,
/// and the terminal emulation moved into `ext::terminal` (QUESTIONS.md
/// 159–162); 11 every user-facing string goes through the
/// [`t!`](crate::t) lookup: `Action::label` returns a `String`,
/// `Availability::Disabled` carries one, the labels the registry keeps
/// (`SettingField`, `ConnectionField`, `ConnectionSchema`, `InfoSection`,
/// `ColumnDef`) are [`Text`](crate::Text) keys, and the registry takes an
/// extension's translation files (QUESTIONS.md 178–181); 12 the preview
/// pane became the only viewer (issue 50): [`Manifest`] gained `previews`,
/// the file extensions a plugin's previewers claim, the registry lends a
/// [`SettingsStore`] so a plugin reads and writes its own declared
/// settings, `Content` gained `MissingLibrary` for a third-party library a
/// previewer cannot find, and `Provider` gained `remote_preview_always`
/// (QUESTIONS.md 191–197); 13 `platform` gained `Mount` (which disk
/// images this OS mounts, and mounting one through it) and the `sevenzip`
/// binding of the 7-Zip library behind the disk images (QUESTIONS.md
/// 202–207); 14 an action opens its form when a job of its finishes:
/// `ActionOutcome` gained `Later` with the [`FormLater`](crate::FormLater)
/// a job fills, `UiHost` gained `form_later` and the
/// [`Continuation`](crate::Continuation) it runs with the values; and
/// `Action` gained `app_menu`, whether the action is listed in the
/// application menu under its extension (QUESTIONS.md 210–211); 15 the
/// command line gained plugin subcommands (specs/system-integration.md
/// §4): [`Manifest`] gained `commands`, the [`CommandSpec`]s
/// a plugin declares and `plugin.toml` carries as `[[commands]]`, and the
/// registry takes a [`CliCommand`] per declared name
/// through [`Registry::command`]; 16 the shell integration of the same
/// spec: `Console` gained `cwd`, the working directory the shell reports
/// through OSC 7, and `foreground_job`, whether a program other than the
/// shell has the terminal; 17 a name conflict carries what its dialogue
/// shows (issue 139): `Conflicts::ask` takes a `ConflictDetails` — both
/// sides of the conflict, the name *Keep both* would give and the
/// conflict's place among the job's — where it took `target_is_dir` and
/// `remaining` (QUESTIONS.md 350); 18 an extension describes itself
/// (issue 139): [`Manifest`] gained `description`, which `plugin.toml`
/// carries and the Extensions dialogue shows (QUESTIONS.md 344); 19 a
/// provider says what a location offers (issue 149):
/// [`Capabilities`] gained `CREATE_DIR`,
/// `CREATE_FILE`, `SEARCH_SUBTREE` and `DOWNLOAD`, and
/// [`Provider`] gained `unavailable_reason`, the tooltip
/// of a control a location rules out, and `name_problem`, a new folder's
/// name checked against the provider's own rules before any request.
pub const API_VERSION: u32 = 19;

bitflags::bitflags! {
    /// The extension points an extension fills, as its manifest declares
    /// them and as the plugin manifest file lists them.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Points: u8 {
        /// A [`Provider`].
        const PROVIDER = 1;
        /// A [`Decorator`].
        const DECORATOR = 2;
        /// One or more [`Action`]s.
        const ACTION = 4;
        /// A [`Previewer`].
        const PREVIEWER = 8;
        /// A [`SearchIndex`].
        const SEARCH_INDEX = 16;
    }
}

impl Points {
    const NAMED: [(Self, &'static str); 5] = [
        (Self::PROVIDER, "provider"),
        (Self::DECORATOR, "decorator"),
        (Self::ACTION, "action"),
        (Self::PREVIEWER, "previewer"),
        (Self::SEARCH_INDEX, "search-index"),
    ];

    /// The names of the points set, in the order of the five points:
    /// `"provider"`, `"decorator"`, `"action"`, `"previewer"`,
    /// `"search-index"`.
    #[must_use]
    pub fn names(self) -> Vec<&'static str> {
        Self::NAMED
            .iter()
            .filter(|(point, _)| self.contains(*point))
            .map(|(_, name)| *name)
            .collect()
    }

    /// The point called `name`, as [`names`](Self::names) spells it;
    /// bitflags' own `from_name` takes the constant names.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::NAMED
            .iter()
            .find(|(_, known)| *known == name)
            .map(|(point, _)| *point)
    }
}

impl fmt::Display for Points {
    /// The names, comma-separated; `none` for no point at all.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            f.write_str("none")
        } else {
            f.write_str(&self.names().join(", "))
        }
    }
}

bitflags::bitflags! {
    /// The operating systems an extension runs on.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Platforms: u8 {
        /// Linux.
        const LINUX = 1;
        /// macOS.
        const MACOS = 2;
        /// Windows.
        const WINDOWS = 4;
    }
}

impl Platforms {
    /// All three.
    pub const ALL: Self = Self::LINUX.union(Self::MACOS).union(Self::WINDOWS);

    const NAMED: [(Self, &'static str); 3] = [
        (Self::LINUX, "linux"),
        (Self::MACOS, "macos"),
        (Self::WINDOWS, "windows"),
    ];

    /// The platform this code runs on, from `std::env::consts::OS`; empty
    /// on any other operating system.
    #[must_use]
    pub fn current() -> Self {
        Self::named(std::env::consts::OS).unwrap_or_default()
    }

    /// The names of the platforms set: `"linux"`, `"macos"`, `"windows"`.
    #[must_use]
    pub fn names(self) -> Vec<&'static str> {
        Self::NAMED
            .iter()
            .filter(|(platform, _)| self.contains(*platform))
            .map(|(_, name)| *name)
            .collect()
    }

    /// The platform called `name`, as [`names`](Self::names) spells it.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::NAMED
            .iter()
            .find(|(_, known)| *known == name)
            .map(|(platform, _)| *platform)
    }
}

impl fmt::Display for Platforms {
    /// The names, comma-separated; `none` for no platform at all.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            f.write_str("none")
        } else {
            f.write_str(&self.names().join(", "))
        }
    }
}

/// What an extension declares about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Unique id, also the name of the extension's settings file.
    pub id: ExtId,
    /// Human-readable name.
    pub name: &'static str,
    /// What the extension does, in a sentence or two of plain English:
    /// the text under its name in the Extensions dialogue, and
    /// `description` in `plugin.toml`. Empty for one that says nothing.
    pub description: &'static str,
    /// The extension's own version.
    pub version: Version,
    /// The [`API_VERSION`] the extension was built against.
    pub api_version: u32,
    /// Provider capabilities the extension needs to be useful.
    pub requires: Capabilities,
    /// The extension points it fills. The registry refuses a contribution
    /// to a point not declared here, so the manifest stays honest.
    pub points: Points,
    /// The operating systems it runs on; the plugin host refuses the
    /// plugin elsewhere.
    pub platforms: Platforms,
    /// The file name extensions the extension's previewers claim, lower
    /// case without the dot (`"pdf"`, `"tar.gz"`), so the host can say
    /// *Plugin available* for a file of such a type from the manifest
    /// alone, without loading a disabled plugin. Empty for an extension
    /// without previewers, and for a previewer that claims no file by its
    /// name (the hex view, which takes what nothing else recognises).
    pub previews: &'static [&'static str],
    /// The subcommands of the `rick` command line the extension provides
    /// (`rick <name> <args…>`), each with its help line: what `rick
    /// --help` lists and `plugin.toml` carries as `[[commands]]`. The
    /// registry refuses a [`CliCommand`] whose name is
    /// not declared here. Empty for an extension without commands.
    pub commands: &'static [CommandSpec],
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

/// The stored value of a setting, as the extension's TOML file holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    /// A checkbox.
    Bool(bool),
    /// A text field.
    Text(String),
    /// A whole number.
    Integer(i64),
    /// The chosen option of a choice, by its text.
    Choice(String),
}

impl SettingKind {
    /// The value a setting has when its file does not name one.
    #[must_use]
    pub fn default_value(&self) -> SettingValue {
        match self {
            Self::Bool { default } => SettingValue::Bool(*default),
            Self::Text { default } => SettingValue::Text(default.clone()),
            Self::Integer { default, .. } => SettingValue::Integer(*default),
            Self::Choice { options, default } => SettingValue::Choice(
                options
                    .get(*default)
                    .or(options.first())
                    .map(|o| (*o).to_owned())
                    .unwrap_or_default(),
            ),
        }
    }
}

/// One setting of an extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingField {
    /// Key in the extension's TOML file.
    pub key: &'static str,
    /// Label in the Settings screen: a Fluent key, read live.
    pub label: crate::Text,
    /// How it is edited.
    pub kind: SettingKind,
}

/// The values of the settings extensions declare ([`SettingsSchema`]), as
/// the host keeps them in each extension's own TOML file: how an extension
/// reads and writes its own settings while it runs (issue 50: a previewer
/// stores the path of the third-party library it found). The registry
/// lends it like the platform services ([`Registry::settings_store`]); an
/// extension keeps the handle from `register` and asks lazily, since the
/// values are read after every extension has declared its schema.
pub trait SettingsStore: Send + Sync {
    /// The value of `key` of the extension `ext`: the file's, else the
    /// schema's default; `None` when no schema declares it.
    fn get(&self, ext: ExtId, key: &str) -> Option<SettingValue>;

    /// Stores `value` for `key` of `ext` and writes the extension's file.
    ///
    /// # Errors
    ///
    /// The I/O error of writing the file.
    fn set(&self, ext: ExtId, key: &str, value: SettingValue) -> Result<(), Error>;
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

/// A static whose address tells this copy of the kernel from any other: a
/// plugin that carries its own copy of `ext` sees another address than the
/// host's registry records (QUESTIONS.md 81).
static KERNEL: [u8; 1] = [1];

/// The address of this copy of the kernel.
#[inline(never)]
pub(crate) fn kernel_id() -> usize {
    std::ptr::from_ref(&KERNEL) as usize
}

/// Collects contributions from extensions, and carries the platform
/// services the host lends them.
pub struct Registry {
    /// The kernel the registry was created in, as [`kernel_id`] gives it.
    kernel: usize,
    /// The platform services of the host, when it lent them.
    platform: Option<Arc<dyn Platform>>,
    /// The question queue of the host, when it lent one.
    prompts: Option<Arc<Prompts>>,
    /// The settings values of the host, when it lent them.
    settings_store: Option<Arc<dyn SettingsStore>>,
    extensions: Vec<ExtId>,
    manifests: Vec<Manifest>,
    failure: Option<String>,
    providers: Vec<Arc<dyn Provider>>,
    /// Virtual locations, `rick://<name>`, by name.
    views: Vec<(String, Arc<dyn Provider>)>,
    /// The connectors of the network providers.
    connectors: Vec<Arc<dyn Connector>>,
    decorators: Vec<Arc<dyn Decorator>>,
    actions: Vec<Arc<dyn Action>>,
    previewers: Vec<Arc<dyn Previewer>>,
    search_indexes: Vec<Arc<dyn SearchIndex>>,
    settings: Vec<SettingsSchema>,
    /// The plugin subcommands of the command line.
    commands: Vec<Arc<dyn CliCommand>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            kernel: kernel_id(),
            platform: None,
            prompts: None,
            settings_store: None,
            extensions: Vec::new(),
            manifests: Vec::new(),
            failure: None,
            providers: Vec::new(),
            views: Vec::new(),
            connectors: Vec::new(),
            decorators: Vec::new(),
            actions: Vec::new(),
            previewers: Vec::new(),
            search_indexes: Vec::new(),
            settings: Vec::new(),
            commands: Vec::new(),
        }
    }
}

impl Registry {
    /// An empty registry without platform services: for tests and for
    /// extensions that need none.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty registry lending `platform` to the extensions it loads.
    #[must_use]
    pub fn with_platform(platform: Arc<dyn Platform>) -> Self {
        Self {
            platform: Some(platform),
            ..Self::default()
        }
    }

    /// The same registry, lending `prompts` as well: the queue an
    /// extension asks the user through while it works with no action in
    /// hand (a host key to trust, a passphrase); the host answers through
    /// its dialogues.
    #[must_use]
    pub fn with_prompts(mut self, prompts: Arc<Prompts>) -> Self {
        self.prompts = Some(prompts);
        self
    }

    /// The same registry, lending `store` as well: where an extension reads
    /// and writes the settings it declared, after registration.
    #[must_use]
    pub fn with_settings_store(mut self, store: Arc<dyn SettingsStore>) -> Self {
        self.settings_store = Some(store);
        self
    }

    /// The settings values the host lent, for an extension's `register` to
    /// keep: `None` in a registry built with [`new`](Self::new), when an
    /// extension works with its schema's defaults and remembers nothing.
    #[must_use]
    pub fn settings_store(&self) -> Option<&Arc<dyn SettingsStore>> {
        self.settings_store.as_ref()
    }

    /// The platform services the host lent, for an extension's `register`:
    /// the trash, open-with, the config directory, the native dialogues.
    /// `None` in a registry built with [`new`](Self::new); an extension
    /// that cannot do without them fails its registration with
    /// `Error::Unsupported` and says so.
    #[must_use]
    pub fn platform(&self) -> Option<&Arc<dyn Platform>> {
        self.platform.as_ref()
    }

    /// The question queue the host lent, for an extension's `register`.
    /// `None` when the host lent none; an extension then has no way to ask
    /// and must refuse what it cannot decide alone (an unknown host key).
    #[must_use]
    pub fn prompts(&self) -> Option<&Arc<Prompts>> {
        self.prompts.as_ref()
    }

    /// Whether the registry was created by this copy of the kernel: `false`
    /// when a plugin carrying its own copy of `ext` looks at a host's
    /// registry, or the other way round.
    #[must_use]
    pub fn shares_kernel(&self) -> bool {
        self.kernel == kernel_id()
    }

    /// A registry that claims another kernel, for the tests of the check.
    #[cfg(test)]
    pub(crate) fn from_another_kernel() -> Self {
        Self {
            kernel: kernel_id().wrapping_add(1),
            ..Self::default()
        }
    }

    /// Checks the manifest and lets the extension register.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when the manifest's `api_version` is not
    /// [`API_VERSION`], an extension with the same id is already loaded, or
    /// the extension registered a contribution to a point its manifest does
    /// not declare; otherwise whatever [`Extension::register`] returns. In
    /// every failing case nothing the extension registered is kept.
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
        let mut staging = Registry {
            platform: self.platform.clone(),
            prompts: self.prompts.clone(),
            settings_store: self.settings_store.clone(),
            ..Registry::default()
        };
        ext.register(&mut staging)?;
        if !manifest.points.contains(staging.points()) {
            return Err(Error::Unsupported(
                "extension registered a point its manifest does not declare",
            ));
        }
        if staging
            .commands
            .iter()
            .any(|c| !manifest.commands.iter().any(|spec| spec.name == c.name()))
        {
            return Err(Error::Unsupported(
                "extension registered a command its manifest does not declare",
            ));
        }
        self.extensions.push(manifest.id);
        self.manifests.push(manifest.clone());
        self.providers.append(&mut staging.providers);
        self.views.append(&mut staging.views);
        self.connectors.append(&mut staging.connectors);
        self.decorators.append(&mut staging.decorators);
        self.actions.append(&mut staging.actions);
        self.previewers.append(&mut staging.previewers);
        self.search_indexes.append(&mut staging.search_indexes);
        self.settings.append(&mut staging.settings);
        self.commands.append(&mut staging.commands);
        Ok(())
    }

    /// Registers a provider.
    pub fn provider(&mut self, p: Arc<dyn Provider>) {
        self.providers.push(p);
    }

    /// Registers a virtual location: `rick://<name>`, served by `p`. The
    /// kernel's `rick` provider hands `p` every location under
    /// `/<name>` with the full path, so the entries `p` lists carry
    /// paths that start with `/<name>` and the provider id
    /// `ProviderId::RICK`; `p`'s own `id` is not consulted. A view counts
    /// as a `Points::PROVIDER` contribution.
    pub fn view(&mut self, name: impl Into<String>, p: Arc<dyn Provider>) {
        self.views.push((name.into(), p));
    }

    /// Registers the connector of a network provider: the fields of its
    /// *Add connection…* dialogue and the calls behind *Connect*,
    /// *Disconnect* and *Remove* on its rows. A connector counts as a
    /// `Points::PROVIDER` contribution, like the provider it belongs to.
    pub fn connector(&mut self, c: Arc<dyn Connector>) {
        self.connectors.push(c);
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

    /// Registers a subcommand of the `rick` command line
    /// ([`crate::cli`]); its name must be one the manifest's `commands`
    /// declare, or [`load`](Self::load) refuses the extension.
    pub fn command(&mut self, c: Arc<dyn CliCommand>) {
        self.commands.push(c);
    }

    /// Hands the extension's translation files to the lookup
    /// ([`crate::i18n`]): one `(language tag, Fluent source)` pair per
    /// language, `include_str!`'d from the crate's `locales/` directory.
    /// Every key the extension uses in [`t!`](crate::t) comes from here, and
    /// English is the one language every key must have.
    pub fn translations(&mut self, files: &[(&str, &str)]) {
        for (language, source) in files {
            crate::i18n::add_bundled(language, source);
        }
    }

    /// The points filled so far: the kinds of contribution registered.
    #[must_use]
    pub fn points(&self) -> Points {
        let mut points = Points::empty();
        points.set(
            Points::PROVIDER,
            !self.providers.is_empty() || !self.views.is_empty() || !self.connectors.is_empty(),
        );
        points.set(Points::DECORATOR, !self.decorators.is_empty());
        points.set(Points::ACTION, !self.actions.is_empty());
        points.set(Points::PREVIEWER, !self.previewers.is_empty());
        points.set(Points::SEARCH_INDEX, !self.search_indexes.is_empty());
        points
    }

    /// Takes every contribution of `other` as well: the plugin host fills a
    /// registry of its own per plugin and the application absorbs it.
    pub fn absorb(&mut self, other: &Registry) {
        self.providers.extend(other.providers.iter().cloned());
        self.views.extend(other.views.iter().cloned());
        self.connectors.extend(other.connectors.iter().cloned());
        self.decorators.extend(other.decorators.iter().cloned());
        self.actions.extend(other.actions.iter().cloned());
        self.previewers.extend(other.previewers.iter().cloned());
        self.search_indexes
            .extend(other.search_indexes.iter().cloned());
        self.settings.extend(other.settings.iter().cloned());
        self.commands.extend(other.commands.iter().cloned());
    }

    /// Ids of the extensions loaded so far, in load order.
    #[must_use]
    pub fn extensions(&self) -> &[ExtId] {
        &self.extensions
    }

    /// Manifests of the extensions loaded so far, in load order.
    #[must_use]
    pub fn manifests(&self) -> &[Manifest] {
        &self.manifests
    }

    /// Why a plugin's registration through this registry failed, as
    /// [`plugin::register`](crate::plugin::register) recorded it; `None`
    /// when nothing failed.
    #[must_use]
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub(crate) fn set_failure(&mut self, reason: String) {
        self.failure = Some(reason);
    }

    /// Registered providers, in registration order.
    #[must_use]
    pub fn providers(&self) -> &[Arc<dyn Provider>] {
        &self.providers
    }

    /// Registered virtual locations, name and provider, in registration
    /// order.
    #[must_use]
    pub fn views(&self) -> &[(String, Arc<dyn Provider>)] {
        &self.views
    }

    /// Registered connectors, in registration order.
    #[must_use]
    pub fn connectors(&self) -> &[Arc<dyn Connector>] {
        &self.connectors
    }

    /// The connector of `provider`, if a network provider registered one.
    #[must_use]
    pub fn connector_of(&self, provider: types::ProviderId) -> Option<&Arc<dyn Connector>> {
        self.connectors.iter().find(|c| c.provider() == provider)
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

    /// The plugin subcommands registered so far ([`crate::cli`]).
    #[must_use]
    pub fn commands(&self) -> &[Arc<dyn CliCommand>] {
        &self.commands
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
            .field("manifests", &self.manifests.len())
            .field("platform", &self.platform.is_some())
            .field("prompts", &self.prompts.is_some())
            .field("providers", &self.providers.len())
            .field(
                "views",
                &self.views.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            )
            .field("connectors", &self.connectors.len())
            .field("decorators", &self.decorators.len())
            .field("actions", &self.actions.len())
            .field("previewers", &self.previewers.len())
            .field("search_indexes", &self.search_indexes.len())
            .field("settings", &self.settings.len())
            .field("failure", &self.failure)
            .finish_non_exhaustive()
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
            description: "",
            version: Version::new(0, 1, 0),
            api_version: API_VERSION,
            requires: Capabilities::empty(),
            points: Points::empty(),
            platforms: Platforms::ALL,
            previews: &[],
            commands: &[],
        }));
        assert!(matches!(result, Err(Error::Unsupported(_))));
        assert_eq!(registry.extensions(), [] as [ExtId; 0]);
        assert_eq!(registry.settings_schemas(), [] as [SettingsSchema; 0]);
        assert!(format!("{registry:?}").contains("extensions: []"));
    }

    /// A previewer that handles nothing.
    struct Nothing;

    #[async_trait::async_trait]
    impl Previewer for Nothing {
        fn id(&self) -> ExtId {
            ExtId("nothing")
        }
        fn score(&self, _entry: &types::Entry, _sniff: &[u8]) -> u8 {
            0
        }
        async fn open(
            &self,
            _loc: &types::Location,
            _provider: Arc<dyn Provider>,
            _cancel: types::CancelToken,
        ) -> Result<Box<dyn crate::preview::PreviewDoc>, Error> {
            Err(Error::Unsupported("nothing to preview"))
        }
    }

    struct Undeclared(Manifest);

    impl Extension for Undeclared {
        fn manifest(&self) -> &Manifest {
            &self.0
        }
        fn register(&self, reg: &mut Registry) -> Result<(), Error> {
            reg.settings(SettingsSchema {
                ext: self.0.id,
                fields: Vec::new(),
            });
            reg.previewer(Arc::new(Nothing));
            Ok(())
        }
    }

    #[test]
    fn a_point_the_manifest_does_not_declare_is_refused() {
        let mut registry = Registry::new();
        let manifest = Manifest {
            id: ExtId("undeclared"),
            name: "Undeclared",
            description: "",
            version: Version::new(0, 1, 0),
            api_version: API_VERSION,
            requires: Capabilities::empty(),
            points: Points::ACTION,
            platforms: Platforms::ALL,
            previews: &[],
            commands: &[],
        };
        assert!(matches!(
            registry.load(&Undeclared(manifest.clone())),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(registry.previewers().len(), 0);
        let declared = Manifest {
            points: Points::ACTION | Points::PREVIEWER,
            ..manifest
        };
        registry
            .load(&Undeclared(declared))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(registry.points(), Points::PREVIEWER);
    }

    /// A plugin whose command the manifest does not name.
    struct Chatty(Manifest);

    struct Say;

    impl crate::cli::CliCommand for Say {
        fn name(&self) -> &'static str {
            "say"
        }
        fn run(
            &self,
            ctx: crate::cli::CliCtx,
            _: &runtime::JobRunner,
            _: &dyn crate::action::UiHost,
        ) -> Result<crate::cli::CliOutcome, Error> {
            Ok(crate::cli::CliOutcome::Text(ctx.args.join(" ")))
        }
    }

    impl Extension for Chatty {
        fn manifest(&self) -> &Manifest {
            &self.0
        }
        fn register(&self, reg: &mut Registry) -> Result<(), Error> {
            reg.command(Arc::new(Say));
            Ok(())
        }
    }

    #[test]
    fn a_command_the_manifest_does_not_declare_is_refused() {
        let mut registry = Registry::new();
        let manifest = Manifest {
            id: ExtId("chatty"),
            name: "Chatty",
            description: "",
            version: Version::new(0, 1, 0),
            api_version: API_VERSION,
            requires: Capabilities::empty(),
            points: Points::empty(),
            platforms: Platforms::ALL,
            previews: &[],
            commands: &[],
        };
        assert!(matches!(
            registry.load(&Chatty(manifest.clone())),
            Err(Error::Unsupported(_))
        ));
        assert!(registry.commands().is_empty());
        let declared = Manifest {
            commands: &[CommandSpec {
                name: "say",
                help: "Say the arguments back",
            }],
            ..manifest
        };
        registry
            .load(&Chatty(declared))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(registry.commands().len(), 1);
        assert_eq!(registry.commands()[0].name(), "say");
        // The application's registry takes a plugin's commands with the
        // rest of its contributions.
        let mut other = Registry::new();
        other.absorb(&registry);
        assert_eq!(other.commands().len(), 1);
    }

    #[test]
    fn points_and_platforms_have_names() {
        assert_eq!(
            (Points::DECORATOR | Points::ACTION).names(),
            ["decorator", "action"]
        );
        assert_eq!(Points::named("search-index"), Some(Points::SEARCH_INDEX));
        assert_eq!(Points::named("searchindex"), None);
        assert_eq!(Points::empty().to_string(), "none");
        assert_eq!(Platforms::ALL.to_string(), "linux, macos, windows");
        assert_eq!(Platforms::named("macos"), Some(Platforms::MACOS));
        assert!(Platforms::ALL.contains(Platforms::current()));
        assert!(!Platforms::current().is_empty());
    }
}
