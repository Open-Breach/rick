# Core types and extension traits

Related: specs/architecture.md, specs/gui-preview.md

First cut of the contract between the kernel and the extensions. Names and shapes are binding; signatures may gain parameters as the implementation learns, but only through a spec change. Everything here is `Send + Sync`; async methods are written with the `async_trait` attribute so the traits stay usable as `dyn` objects (`Arc<dyn Provider>`), to be replaced by native `async fn` once `dyn`-compatible async traits are stable; streamed results use `BoxStream` from `futures`.

The supporting types the signatures name without defining them here (`ListOptions`, `WriteOptions`, `RemoveMode`, `LinkKind`, `CopyOptions`, `Badge`, `Color`, `ColumnDef`, `ColumnId`, `CellValue`, `PanelSection`, `DecorateCtx`, `PaneId`, `Invocation`, `Shortcut`, `UiHost`, `RenderRequest`, `TextLayer`, `TextBlock`, `TextureId`, `Transport`, `SearchScope`, `QueryKind`, `Filter`, `SearchHit`, `SettingsSchema`, `Version`) are defined and documented in crate `ext`, with the smallest shape that makes the trait usable. Their shapes are binding as implemented; a change to one goes through a numbered question like a change to a signature.

**Value types** (crate `types`, no I/O):

```rust
/// Which extension provides a tree. "local", "archive", "ssh", "s3", "trash".
pub struct ProviderId(pub &'static str);

/// Identity of an extension, as declared in its manifest. "git", "preview-pdf".
pub struct ExtId(pub &'static str);

/// Stable identity of an entry inside one provider. Local uses (device, inode);
/// other providers choose their own 128-bit key. Never a path string.
pub struct EntryId { pub provider: ProviderId, pub key: u128 }

/// Provider-neutral path. Keeps raw bytes, offers a lossy display form.
pub struct VPath(Vec<u8>);

/// A place a pane can show.
pub struct Location { pub provider: ProviderId, pub path: VPath }

pub enum EntryKind { File, Dir, Symlink { target: VPath, broken: bool }, Other }

/// Media type as `type/subtype` without parameters, not validated. Filled
/// lazily by sniffing, so most entries carry `None` until something asks.
pub struct Mime(Box<str>);

pub struct Entry {
    pub id: EntryId,
    pub name: String,            // display name; raw bytes live in `path`
    pub path: VPath,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub attrs: Attrs,            // bitflags: HIDDEN, SYSTEM, READONLY, EXECUTABLE
    pub mime: Option<Mime>,      // filled lazily by sniffing
}

/// Cooperative cancellation. Cloneable; `cancelled().await` for async code,
/// `is_cancelled()` for loops.
pub struct CancelToken;

pub enum Progress {
    Items { done: u64, total: Option<u64> },
    Bytes { done: u64, total: Option<u64> },
    Message(String),
}

/// One error type for the whole kernel; extensions map their own into it.
pub enum Error { NotFound, PermissionDenied, Cancelled, Unsupported(&'static str), Io(std::io::Error), Extension { ext: ExtId, source: Box<dyn std::error::Error + Send + Sync> } }
```

**Job types** (crate `runtime`, which owns the tokio runtime and re-exports `tokio::sync::{watch, oneshot}` so callers need no `tokio` dependency):

```rust
/// What `runtime` hands back for every long operation.
pub struct JobHandle<T> {
    pub id: JobId,
    pub cancel: CancelToken,
    pub progress: watch::Receiver<Progress>,
    pub result: oneshot::Receiver<Result<T, Error>>,
}

/// What a job receives when it starts. Cancelling drops the job's future, so
/// a job that never looks at the token still stops at its next await point
/// and its handle resolves with `Error::Cancelled`.
pub struct JobCtx { pub id: JobId, pub cancel: CancelToken, pub progress: ProgressSink, pub pause: PauseGate }

/// Pausable jobs await the gate before each item, directory, file and streamed
/// buffer; `JobRunner::pause` closes it, `resume` opens it. Copy and move jobs
/// are pausable; listings and decorations are not.
pub struct PauseGate;

/// Where a job publishes progress; `report` replaces the previous value and never fails.
pub struct ProgressSink(watch::Sender<Progress>);

impl JobRunner {
    pub fn spawn<T, F, Fut>(&self, job: F) -> JobHandle<T>
        where F: FnOnce(JobCtx) -> Fut, Fut: Future<Output = Result<T, Error>> + Send + 'static;
    pub fn spawn_pausable<T, F, Fut>(&self, job: F) -> JobHandle<T>
        where F: FnOnce(JobCtx) -> Fut, Fut: Future<Output = Result<T, Error>> + Send + 'static;
    pub fn running(&self) -> Vec<JobId>;
    pub fn cancel(&self, id: JobId) -> bool;
    pub fn pause(&self, id: JobId) -> bool;
    pub fn resume(&self, id: JobId) -> bool;
}
```

**Provider** — a tree of entries (crate `ext`):

```rust
bitflags! { pub struct Capabilities: u32 {
    const WRITE = 1; const TRASH = 2; const RENAME = 4; const SYMLINK = 8;
    const REFLINK = 16; const WATCH = 32; const SEEK = 64; const PERMISSIONS = 128;
    /// Copy and move between two locations of this provider instance happen
    /// on the provider's side (server-side copy, remote rename); nothing
    /// streams through the machine and the job only tracks progress.
    const NATIVE_TRANSFER = 256;
} }

pub trait Provider: Send + Sync + 'static {
    fn id(&self) -> ProviderId;
    fn capabilities(&self, loc: &Location) -> Capabilities;

    /// Streams entries in batches. The first batch must arrive within the
    /// first-screen budget; `opts` carries hidden/system flags and depth (1 or
    /// unlimited for flattened views). Cancelling the token ends the stream.
    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken)
        -> BoxStream<'static, Result<Vec<Entry>, Error>>;

    async fn stat(&self, loc: &Location) -> Result<Entry, Error>;
    async fn open_read(&self, loc: &Location) -> Result<Box<dyn AsyncRead + Send + Unpin>, Error>;
    async fn open_write(&self, loc: &Location, opts: WriteOptions) -> Result<Box<dyn AsyncWrite + Send + Unpin>, Error>;
    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error>;
    async fn rename(&self, from: &Location, to: &Location) -> Result<Entry, Error>;
    async fn remove(&self, loc: &Location, mode: RemoveMode) -> Result<(), Error>;   // Trash | Permanent
    async fn link(&self, target: &Location, at: &Location, kind: LinkKind) -> Result<Entry, Error>;

    /// Copies one regular file with the native fast path (copy_file_range,
    /// reflink, server-side copy). `vfs::copy` composes directory trees from
    /// it and cross-provider copies from `open_read` and `open_write`; it
    /// owns recursion, progress totals and cancellation.
    async fn copy(&self, from: &Location, to: &Location, opts: CopyOptions,
                  progress: ProgressSink, cancel: CancelToken) -> Result<(), Error>;

    /// Moves or copies a whole selection within this provider instance on the
    /// provider's side, if NATIVE_TRANSFER is set: the object store's copy
    /// API across buckets of one account, `ext-ssh`'s remote `cp` and `mv`. The provider
    /// submits the operation, reports progress as the service exposes it and
    /// honours `cancel` where the service allows. `ops` calls this first when
    /// source and target share a provider instance and falls back to
    /// `rename`, `copy` and the streamed path otherwise.
    async fn transfer(&self, items: &[Location], to: &Location, mode: TransferMode, // Move | Copy
                      progress: ProgressSink, cancel: CancelToken) -> Result<(), Error>;

    /// Change notifications for one location, if WATCH is set.
    fn watch(&self, loc: &Location) -> Result<BoxStream<'static, FsChange>, Error>;
}

pub enum FsChange { Created(Entry), Removed(EntryId), Modified(Entry), Renamed { from: EntryId, to: Entry }, Rescan }
```

**Decorator** — adds badges, columns and panel sections to entries:

```rust
pub trait Decorator: Send + Sync + 'static {
    fn id(&self) -> ExtId;
    /// Cheap check per location, e.g. "inside a Git work tree".
    fn applies_to(&self, loc: &Location) -> bool;
    /// Extra columns this decorator can fill (e.g. Git status).
    fn columns(&self) -> &[ColumnDef];
    /// Called for each visible batch. Must return fast; slow lookups return
    /// `Pending` and complete later through `ctx.publish`.
    fn decorate(&self, entries: &[Entry], ctx: &DecorateCtx) -> Vec<(EntryId, Decoration)>;
    /// Optional section for the info panel.
    fn panel(&self, entry: &Entry) -> Option<PanelSection>;
    /// Locations whose changes stale this decorator's cache for `loc` (the
    /// `.git` directory of a work tree); the kernel watches them for the pane.
    fn invalidates_on(&self, loc: &Location) -> Vec<Location>;
    /// One of those locations changed: drop the cache; the kernel decorates
    /// the listing again.
    fn invalidate(&self, loc: &Location);
}

pub struct Decoration {
    pub badge: Option<Badge>,              // small glyph in the icon corner
    pub tint: Option<Color>,               // name colour
    pub cells: SmallVec<[(ColumnId, CellValue); 2]>,
}
```

**Action** — a command the user can invoke:

```rust
pub struct ActionId(pub &'static str);     // "core.copy", "git.commit"

pub enum ActionGroup { Core, Extension(&'static str) }   // Extension("Git") → context-menu submenu

pub enum Availability { Hidden, Disabled(&'static str), Enabled }

pub struct ActionCtx {
    pub pane: PaneId,
    pub location: Location,
    pub selection: Vec<Entry>,
    pub invoked_from: Invocation,          // ContextMenu | Palette | Shortcut | Menu
}

pub enum ActionOutcome { Done, Started(JobId), Undoable(Box<dyn UndoCommand>) }

pub trait Action: Send + Sync + 'static {
    fn id(&self) -> ActionId;
    fn label(&self) -> &str;
    fn group(&self) -> ActionGroup;
    fn default_shortcut(&self) -> Option<Shortcut>;
    fn availability(&self, ctx: &ActionCtx) -> Availability;
    /// Runs on the app thread; anything long is spawned as a Job through `jobs`.
    fn run(&self, ctx: ActionCtx, jobs: &JobRunner, ui: &dyn UiHost) -> Result<ActionOutcome, Error>;
}

pub trait UndoCommand: Send { fn undo(&self, jobs: &JobRunner) -> Result<(), Error>; fn redo(&self, jobs: &JobRunner) -> Result<(), Error>; fn label(&self) -> String; }
```

**Previewer** — turns an entry into something the preview pane can show:

```rust
pub trait Previewer: Send + Sync + 'static {
    fn id(&self) -> ExtId;
    /// 0 = cannot handle; the highest score wins. `sniff` is the first 4 KiB.
    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8;
    async fn open(&self, loc: &Location, provider: Arc<dyn Provider>, cancel: CancelToken)
        -> Result<Box<dyn PreviewDoc>, Error>;
}

pub trait PreviewDoc: Send {
    fn pages(&self) -> Option<usize>;                       // None = single surface
    /// Produces a frame for one page at the requested size; the UI uploads it
    /// to a texture. Text documents return `Frame::Text` and let the UI lay out.
    fn render(&mut self, page: usize, req: RenderRequest) -> Result<Frame, Error>;
    fn text_layer(&self) -> Option<&TextLayer>;             // for find
    /// Metadata, grouped in sections for the pane's File info card. The first section is
    /// "File info"; others (EXIF, stream details) appear only when they apply.
    fn sections(&self) -> Vec<InfoSection>;
    /// One-line facts for the pane's status bar, in display order.
    fn status(&self) -> Vec<String>;
    /// The previewer's own toolbar controls (e.g. pretty print); the standard
    /// ones (line numbers, wrap, pager, fit/1:1) come from the kind of document.
    fn controls(&self) -> Vec<PreviewControl>;
    /// A toolbar control was used; the next `render` reflects it.
    fn set_control(&mut self, id: ControlId, value: ControlValue) -> Result<(), Error>;
    fn transport(&mut self) -> Option<&mut dyn Transport>;  // media only: play/pause/seek
}

pub struct InfoSection { pub id: &'static str, pub title: &'static str, pub icon: Icon, pub rows: Vec<(String, String)> }

pub enum PreviewControl {
    Toggle { id: ControlId, icon: Icon, label: &'static str, on: bool },
    Button { id: ControlId, icon: Icon, label: &'static str, enabled: bool },
}

pub enum Frame { Rgba { width: u32, height: u32, data: Arc<[u8]> }, Text(TextBlock), External(TextureId) }

/// Text the UI lays out itself. `spans` are coloured byte ranges the previewer
/// computed (syntax highlighting); `RenderRequest.dark` tells it which theme.
pub struct TextBlock { pub text: String, pub spans: Vec<(Range<usize>, Color)> }
```

The pane's toolbar is built from these plus the document kind, and the pane itself never asks a previewer to draw: a Decorator's `panel()` and a previewer's `sections()` both feed the same File info card.

**SearchIndex** — a query backend:

```rust
pub trait SearchIndex: Send + Sync + 'static {
    fn id(&self) -> ExtId;
    fn availability(&self) -> Availability;                 // platform backend present?
    fn scope(&self) -> SearchScope;                         // Everywhere | Under(Location)
    fn query(&self, q: &Query, cancel: CancelToken) -> BoxStream<'static, Result<Vec<SearchHit>, Error>>;
}

pub struct Query { pub text: String, pub root: Option<Location>, pub kind: QueryKind /* Name | Content | Both */, pub filters: Vec<Filter> }
```

**Registration** — how a crate like `ext-git` contributes to several points at once:

```rust
pub struct Manifest { pub id: ExtId, pub name: &'static str, pub version: Version, pub api_version: u32, pub requires: Capabilities }

pub trait Extension: Send + Sync + 'static {
    fn manifest(&self) -> &Manifest;
    fn register(&self, reg: &mut Registry) -> Result<(), Error>;
}

impl Registry {
    pub fn provider(&mut self, p: Arc<dyn Provider>);
    pub fn decorator(&mut self, d: Arc<dyn Decorator>);
    pub fn action(&mut self, a: Arc<dyn Action>);
    pub fn previewer(&mut self, p: Arc<dyn Previewer>);
    pub fn search_index(&mut self, s: Arc<dyn SearchIndex>);
    pub fn settings(&mut self, schema: SettingsSchema);   // renders in the Settings screen, stored in the extension's TOML
}
```

**Commands and events** — the only two things that cross the UI boundary:

```rust
pub enum Command {
    Navigate { pane: PaneId, to: Location }, Back(PaneId), Forward(PaneId), Refresh(PaneId),
    Select { pane: PaneId, change: SelectionChange },
    SetView { pane: PaneId, mode: ViewMode }, Sort { pane: PaneId, by: Column, dir: SortDir }, GroupBy { pane: PaneId, by: Option<Column> },
    Filter { pane: PaneId, text: String }, ToggleHidden(PaneId), Flatten { pane: PaneId, on: bool },
    Run { action: ActionId, ctx: ActionCtx },
    NewTab(Location), CloseTab(TabId), ActivateTab(TabId), FocusPane(PaneId),
    Split { pane: PaneId, dir: SplitDir }, MoveTab { tab: TabId, to: DockTarget },
    CancelJob(JobId), DismissJob(JobId), Undo, Redo,
    Settings(SettingsChange),
    /// Drag-and-drop (and later Move to / Copy to): move or copy `items` into `to`.
    /// `Ask` opens the move/copy prompt first. Runs as one job; a move within a
    /// provider is a rename, a copy uses the provider's fast path.
    Transfer { items: Vec<EntryId>, from: PaneId, to: Location, mode: TransferMode /* Move | Copy | Ask */ },
    /// Spring-loading step while a drag is in progress: navigates like `Navigate`
    /// but records history only on the first step of a drag (`first: true`).
    SpringLoad { pane: PaneId, to: Location, first: bool },
    /// Pin and unpin, undoable; `at` is the position in Favorites, None = end.
    Pin { items: Vec<Location>, at: Option<usize> }, Unpin(Location),
    /// Items dropped from outside the process (file URIs, no EntryId): the same
    /// job as Transfer with the outside rule (copy unless Shift) already applied.
    Receive { items: Vec<Location>, to: Location, mode: TransferMode },
    /// Thumbnail for the icon view, rendered by the best-scoring previewer as an
    /// internal job; answered by Event::Thumbnail.
    Thumbnail { pane: PaneId, id: EntryId, size: u32 },
    /// Session, layouts and windows (General behaviour).
    TabOrder { pane: PaneId, tabs: Vec<TabId>, active: TabId }, Layout(DockLayout), Scroll { pane: PaneId, offset: f32 },
    WindowGeometry { window: WindowId, rect: Rect, maximised: bool }, SidePanel { window: WindowId, state: SidePanelState },
    FocusWindow(WindowId), NewWindow, CloseWindow(WindowId),
    SaveLayout(String), SwitchLayout(String), DeleteLayout(String), RenameLayout { from: String, to: String },
}

pub enum Event {
    Listed { pane: PaneId, batch: Vec<Entry>, done: bool },
    Changed { pane: PaneId, change: FsChange },
    Decorated { pane: PaneId, items: Vec<(EntryId, Decoration)> },
    JobProgress { id: JobId, progress: Progress }, JobDone { id: JobId, result: Result<(), Error> },
    SelectionChanged { pane: PaneId, ids: Vec<EntryId> },
    PreviewReady { pane: PaneId, doc: Box<dyn PreviewDoc> },
    PreviewFailed { pane: PaneId, error: Error },   // no previewer claims the file, or open failed
    Thumbnail { pane: PaneId, id: EntryId, size: u32, frame: Frame },
    Notify { level: Level, text: String },
}
```
