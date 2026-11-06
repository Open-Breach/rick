# Questions

Open questions raised while implementing SPEC.md. The agent appends one
numbered entry per ambiguity, with the assumption it made; the maintainer
answers in place. If an answer changes the spec, SPEC.md changes in the
same commit.

Entry format:

`12. (issue 3) Should Undo of a trash operation restore to the original path if a new entry now has that name? Assumed: no, restore with a numbered suffix.`

Once answered, an entry keeps its number and gains a `Decided (date):` line.

---

1. (issue 1) "Package layout" said `types` has "no dependencies beyond std",
   but `Attrs` is a `bitflags` type and library errors use `thiserror`.
   Assumed: `bitflags` and `thiserror` are allowed, nothing else.
   Decided (2026-11-06): `types` may use the small ecosystem crates it
   needs; it stays free of I/O, async runtimes and other Rick crates. Spec
   updated.

2. (issue 1) `Error::Extension` and `Entry::mime` used `ExtId` and `Mime`,
   which the "Value types" block did not define. Assumed: both live in
   `types`; `ExtId(pub &'static str)` mirrors `ProviderId`, `Mime` is an
   unvalidated `type/subtype` string.
   Decided (2026-11-06): as assumed; both added to the spec's value types.
   Reconsider the `mime` crate when a previewer needs parameter parsing.

3. (issue 1) `JobHandle<T>` was listed among the no-I/O value types of
   `types` but is made of tokio channel ends and handed out by `runtime`.
   Assumed: `JobHandle`, `JobCtx` and `ProgressSink` live in `runtime`,
   which re-exports `tokio::sync::{watch, oneshot}`.
   Decided (2026-11-06): as assumed; the spec now has a "Job types (crate
   `runtime`)" block with them and the `JobRunner` methods.

4. (issue 1) Must a job that never inspects its `CancelToken` still stop
   when cancelled? Assumed: yes; the runner drops the job's future and the
   handle resolves with `Error::Cancelled`.
   Decided (2026-11-06): as assumed. The "Every long operation is a Job"
   rule now states it, plus the duty of jobs with partial state to check
   the token or hold a `Drop` guard before irreversible steps, and of long
   compute work to check `is_cancelled()` between chunks.

5. (issue 1) "`fixture::generate(200_000, seed)` runs in under 30 s on CI"
   failed on the GitHub-hosted Windows runner (297 s with four writers on
   the system drive; NTFS and Defender real-time scanning).
   Decided (2026-11-06): the 30 s budget is an exception on Windows for
   now. The test keeps its assertion and runs there with
   `continue-on-error`, so it reports its time without failing the job.
   Since the writers moved to the blocking pool and the fixture to the
   runner's temp disk it passes on Windows too; remove the exception once
   `vfs-local` exercises the fixture regularly and the margin is known.
   Extended (2026-11-06, issue 2): the macOS runner sits at the limit
   (28 s on two runs, 31.5 s and 32.8 s on the next two), so the same
   exception now covers macOS; the budget is enforced on Linux only.

6. (issue 2) The spec says async trait methods use native `async fn` in
   traits, yet it also passes `Arc<dyn Provider>` around. On stable Rust a
   trait with `async fn` is not `dyn`-compatible (E0038).
   Assumed: the five traits use the `async_trait` attribute (boxed futures)
   so they work as `dyn` objects; to be replaced by native `async fn` when
   `dyn` dispatch for it stabilises.

7. (issue 2) "Package layout" lists `ext` after `ops`, but the Provider
   trait lives in `ext` and `vfs` and `vfs-local` implement it, so they must
   depend on `ext`.
   Assumed: the layer order is types → runtime → platform → ext → vfs →
   vfs-local → model → ops → app → ui-egui → rick; deny.toml encodes it.

8. (issue 2) The trait signatures name types the spec never defines:
   `ListOptions`, `WriteOptions`, `RemoveMode`, `LinkKind`, `CopyOptions`,
   `Badge`, `Color`, `ColumnDef`, `ColumnId`, `CellValue`, `PanelSection`,
   `DecorateCtx`, `PaneId`, `Invocation`, `Shortcut`, `UiHost`,
   `RenderRequest`, `TextLayer`, `TextBlock`, `TextureId`, `Transport`,
   `SearchScope`, `QueryKind`, `Filter`, `SearchHit`, `SettingsSchema`,
   `Version`.
   Assumed: the smallest shapes that make the traits usable, as defined in
   `ext`. `LinkKind` has only `Symbolic` and `Hard`; junctions and `.lnk`
   shortcuts are not in 0.1.

9. (issue 2) The build order mentions "vfs-local with the two-pass
   listing", which nothing else describes.
   Assumed: one pass is enough when the per-entry `statx` calls of a batch
   run in parallel on the compute pool; measured on the 200k fixture in
   release on four cores: first batch 1.7 ms, complete 213 ms, against the
   50 ms and 400 ms targets. A names-first pass can be added if a slower
   machine needs it.

10. (issue 2) `Provider::copy` is described as the same-provider fast path
    but not whether it walks directories.
    Assumed: it copies one regular file; `vfs::copy` walks the tree, creates
    directories and calls it once per file, so one place owns recursion,
    progress totals and cancellation.

11. (issue 2) Restoring from the trash is not specified beyond "Restore".
    Assumed: Windows restores by moving the `$R` payload out of the Recycle
    Bin and deleting its `$I` record, matched through the shell's
    `System.Recycle.DeletedFrom` property, rather than through the shell's
    "undelete" verb. macOS has no public reader for the original paths of
    trashed items, so `Trash::list` returns `Error::Unsupported` there and a
    restore needs the `TrashedItem` the trash call returned. Restoring into
    an original path that is taken again fails with `AlreadyExists`; `ops`
    decides later whether to suffix.

12. (issue 2) The platform traits are named but not shaped.
    Assumed: `OpenWith` has `open` (default application) and `reveal`
    (system file manager), the two operations 0.1 needs; `Properties::show`
    opens the native dialogue on Windows and macOS and returns label/value
    rows on Linux; `NativeDialogs` is async and implemented with `rfd`
    (XDG portal on Linux), whose D-Bus dependencies carry three RustSEC
    "unmaintained" notices that deny.toml ignores with the reason recorded.

13. (issue 3) The `Command` enum in "Commands and events" has no way to make
    a tab active, to focus a pane or to clear a finished job from the job
    table, yet tabs, panes and the operations area all need them.
    Assumed: three commands are added, `ActivateTab(TabId)`,
    `FocusPane(PaneId)` and `DismissJob(JobId)`, with the smallest meaning;
    nothing else beyond the spec's list.

14. (issue 3) The spec does not say who holds the copy/cut clipboard between
    Copy and Paste, nor whether it is the system clipboard.
    Assumed: the built-in `ops::Core` extension holds it in memory as
    entries plus a copy-or-move mode; Paste after Cut empties it; Paste after
    Copy keeps it. Only Copy path writes to the system clipboard. Sharing
    file lists with other applications is left for a later issue.

15. (issue 3) `Provider::remove(loc, RemoveMode::Trash)` returns `()`, so
    an undo of Move to trash has nothing to restore from, while
    `platform::Trash::trash` returns the `TrashedItem` that `restore` needs.
    Assumed: Move to trash on a local location goes through the platform
    directly and keeps the items for undo; on any other provider it goes
    through the provider and cannot be undone. Only local exists in 0.1.

16. (issue 3) "Every long operation is a Job" names directory listings, and
    the job table shows jobs. A listing per navigation would flood the
    operations area, and a change watch never ends.
    Assumed: listings run as jobs (cancel token, progress, result) but `app`
    keeps them out of the model's job table; their output is the
    `Event::Listed` stream. The watch on a pane's location is a cancellable
    runtime task tied to the listing's cancel token, not a job.

17. (issue 3) The spec relies on change notifications for the list to follow
    the filesystem, and offers Refresh "for providers without change
    notifications or when in doubt".
    Assumed: when a job that an action, an undo or a redo started finishes,
    `app` also relists the pane the command came from, swapping the result
    in without an empty intermediate state, so the model is right even where
    the watcher lags or is unsupported. Watch events still apply as they
    arrive.

18. (issue 3) "Settings in TOML" names no file, location or behaviour on a
    bad file.
    Assumed: `rick.toml` in the `ConfigDir` directory, which `app` creates
    if missing so it can be watched. Keys not present take their defaults; a
    file that does not parse is reported as a warning and the last good
    settings stay. Every `Command::Settings` writes the whole file.

19. (issue 3) Some entries of the 0.1 operations list need the UI rather
    than an `Action`: "Open containing folder" navigates the pane and selects
    an entry, which `ActionOutcome` cannot express; "back and forward", "go
    to root" and "refresh" are navigation; Rename and New file/folder ask
    for a name "in inline edit".
    Assumed: navigation is `Command`s (`Navigate`, `Back`, `Forward`,
    `Refresh`), Open containing folder is `Navigate` plus `Select` issued by
    the UI, and the name for Rename, New file and New folder comes through
    `UiHost::prompt`, which the UI may implement as inline editing. The
    shortcut table's navigation and selection rows are therefore bound by the
    UI's key map, not by registered actions.

20. (issue 3) The Paste conflict handling ("asks once with skip, overwrite,
    rename or apply-to-all") needs a dialogue the UI issue brings.
    Assumed: Paste, Duplicate and the undo of a move never overwrite; a name
    that exists fails that item's job with `AlreadyExists`, reported in the
    job table and as a notification. Copy path's "name only" variant and
    New folder's "move the selection into it" variant are not in 0.1.
