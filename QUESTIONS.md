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
