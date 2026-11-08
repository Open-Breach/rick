# Questions

Open questions raised while implementing SPEC.md. The agent appends one
numbered entry per ambiguity, with the assumption it made; the maintainer
answers in place. If an answer changes the spec, SPEC.md changes in the
same commit.

Entry format:

`12. (issue 3) Should Undo of a trash operation restore to the original path if a new entry now has that name? Assumed: no, restore with a numbered suffix.`

Once answered, an entry keeps its number and gains a dated decision line.

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
   Decided (2026-11-06): as assumed; the spec's extension-traits preamble
   now says so.

7. (issue 2) "Package layout" lists `ext` after `ops`, but the Provider
   trait lives in `ext` and `vfs` and `vfs-local` implement it, so they must
   depend on `ext`.
   Assumed: the layer order is types → runtime → platform → ext → vfs →
   vfs-local → model → ops → app → ui-egui → rick; deny.toml encodes it.
   Decided (2026-11-06): as assumed; the spec's Kernel list is now in
   that order and says cargo-deny enforces it.

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
   Decided (2026-11-06): as assumed; the spec now points at `ext` for the
   supporting types and notes that junctions and `.lnk` shortcuts are not
   in 0.1.

9. (issue 2) The build order mentions "vfs-local with the two-pass
   listing", which nothing else describes.
   Assumed: one pass is enough when the per-entry `statx` calls of a batch
   run in parallel on the compute pool; measured on the 200k fixture in
   release on four cores: first batch 1.7 ms, complete 213 ms, against the
   50 ms and 400 ms targets. A names-first pass can be added if a slower
   machine needs it.
   Decided (2026-11-06): one pass; the build order now says "streamed
   listing". A names-first pass stays an option for a provider whose
   per-entry metadata is slow, such as a network share, once measured.

10. (issue 2) `Provider::copy` is described as the same-provider fast path
    but not whether it walks directories.
    Assumed: it copies one regular file; `vfs::copy` walks the tree, creates
    directories and calls it once per file, so one place owns recursion,
    progress totals and cancellation.
    Decided (2026-11-06): as assumed; the spec's doc comment on `copy` now
    says so and names `vfs::copy`, not `ops`, as the composer.

11. (issue 2) Restoring from the trash is not specified beyond "Restore".
    Assumed: Windows restores by moving the `$R` payload out of the Recycle
    Bin and deleting its `$I` record, matched through the shell's
    `System.Recycle.DeletedFrom` property, rather than through the shell's
    "undelete" verb. macOS has no public reader for the original paths of
    trashed items, so `Trash::list` returns `Error::Unsupported` there and a
    restore needs the `TrashedItem` the trash call returned. Restoring into
    an original path that is taken again fails with `AlreadyExists`; `ops`
    decides later whether to suffix.
    Decided (2026-11-06), to revisit: as assumed for 0.1, noted in the
    spec's Restore entry. Open for the Trash location issue: a way to read
    original paths on macOS (private Finder store, `.DS_Store` of the
    Trash, or Rick's own record of what it trashed), and whether a taken
    original path gets a numbered suffix or a prompt.

12. (issue 2) The platform traits are named but not shaped.
    Assumed: `OpenWith` has `open` (default application) and `reveal`
    (system file manager), the two operations 0.1 needs; `Properties::show`
    opens the native dialogue on Windows and macOS and returns label/value
    rows on Linux; `NativeDialogs` is async and implemented with `rfd`
    (XDG portal on Linux), whose D-Bus dependencies carry three RustSEC
    "unmaintained" notices that deny.toml ignores with the reason recorded.
    Decided (2026-11-06): as assumed; the spec's `platform` bullet now
    lists the 0.1 trait set and the accepted advisories.

13. (issue 3) The `Command` enum in "Commands and events" has no way to make
    a tab active, to focus a pane or to clear a finished job from the job
    table, yet tabs, panes and the operations area all need them.
    Assumed: three commands are added, `ActivateTab(TabId)`,
    `FocusPane(PaneId)` and `DismissJob(JobId)`, with the smallest meaning;
    nothing else beyond the spec's list.
    Decided (2026-11-06): as assumed; the three are in the spec's
    `Command` enum.

14. (issue 3) The spec does not say who holds the copy/cut clipboard between
    Copy and Paste, nor whether it is the system clipboard.
    Assumed: the built-in `ops::Core` extension holds it in memory as
    entries plus a copy-or-move mode; Paste after Cut empties it; Paste after
    Copy keeps it. Only Copy path writes to the system clipboard. Sharing
    file lists with other applications is left for a later issue.
    Decided (2026-11-06): as assumed; spec updated in 1309213.

15. (issue 3) `Provider::remove(loc, RemoveMode::Trash)` returns `()`, so
    an undo of Move to trash has nothing to restore from, while
    `platform::Trash::trash` returns the `TrashedItem` that `restore` needs.
    Assumed: Move to trash on a local location goes through the platform
    directly and keeps the items for undo; on any other provider it goes
    through the provider and cannot be undone. Only local exists in 0.1.
    Decided (2026-11-06): as assumed; spec unchanged.

16. (issue 3) "Every long operation is a Job" names directory listings, and
    the job table shows jobs. A listing per navigation would flood the
    operations area, and a change watch never ends.
    Assumed: listings run as jobs (cancel token, progress, result) but `app`
    keeps them out of the model's job table; their output is the
    `Event::Listed` stream. The watch on a pane's location is a cancellable
    runtime task tied to the listing's cancel token, not a job.
    Decided (2026-11-06): as assumed; the spec's Job rule now says which
    jobs the operations area lists.

17. (issue 3) The spec relies on change notifications for the list to follow
    the filesystem, and offers Refresh "for providers without change
    notifications or when in doubt".
    Assumed: when a job that an action, an undo or a redo started finishes,
    `app` also relists the pane the command came from, swapping the result
    in without an empty intermediate state, so the model is right even where
    the watcher lags or is unsupported. Watch events still apply as they
    arrive.
    Decided (2026-11-06): as assumed; spec unchanged.

18. (issue 3) "Settings in TOML" names no file, location or behaviour on a
    bad file.
    Assumed: `rick.toml` in the `ConfigDir` directory, which `app` creates
    if missing so it can be watched. Keys not present take their defaults; a
    file that does not parse is reported as a warning and the last good
    settings stay. Every `Command::Settings` writes the whole file.
    Decided (2026-11-06): as assumed; spec unchanged.

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
    Decided (2026-11-06): as assumed; spec unchanged.

20. (issue 3) The Paste conflict handling ("asks once with skip, overwrite,
    rename or apply-to-all") needs a dialogue the UI issue brings.
    Assumed: Paste, Duplicate and the undo of a move never overwrite; a name
    that exists fails that item's job with `AlreadyExists`, reported in the
    job table and as a notification. Copy path's "name only" variant and
    New folder's "move the selection into it" variant are not in 0.1.
    Decided (2026-11-06): superseded: the conflict dialogue is specified
    (Drag and drop, Execution) and built in close-out issue A.

21. (issue 4) The spec has two shortcut tables that disagree: the first
    ("Keyboard shortcuts" under Functions) binds Backspace to Parent
    directory, Space to Toggle preview, Ctrl+N to New window and
    Ctrl+Shift+F to Search subdirectories; the second ("Keyboard
    shortcuts" under Architecture, "the source for defaults") binds
    Backspace to Back, Space to Toggle selection, Mod+N to New file and
    Mod+Shift+F to Toggle flatten subfolders.
    Assumed: the second table is the source of defaults, as it says. Rows
    of the first table that do not conflict are bound too: F11 Full screen,
    Mod+Q Quit, Mod+B Toggle side panel, Mod+, Settings, Alt+Enter
    Properties. New window and Search subdirectories are not bound.
    Decided (2026-11-06): as assumed; spec updated in 1309213.

22. (issue 4) The GUI section says type-to-filter "replaces the classic
    jump to first entry starting with the letter behaviour", while the
    Selection table lists "Type-ahead: jump to the first entry starting
    with the typed letters".
    Assumed: type-to-filter. Typing while the list has the keyboard appends
    to the tab's filter, shown in the bar at the foot of the list; Backspace
    edits it, Escape clears it and Enter opens the first match.
    Decided (2026-11-06): type-to-filter stands; the Selection table row is
    corrected in 0aaaf1a.

23. (issue 4) Several rows of the shortcut tables name operations outside
    the 0.1 scope: Quick open, Select by pattern, Paste as link, Bulk
    rename, Open with…, Open console, Tree and Icons views, icon size, the
    side panel. The acceptance test wants every row to trigger its action.
    Assumed: every row is bound and dispatches a named intent; the intents
    of operations outside 0.1 show a notification saying so, so the binding
    is live and visible and the implementation slots in later. The Viewer
    table is not bound: the viewer is an explicit 0.1 non-goal.
    Decided (2026-11-06): as assumed; spec unchanged.

24. (issue 4) "60 fps on the CI runner's software renderer" names a
    renderer the headless test harness does not have: `egui_kittest` runs
    the UI without a GPU and only renders pixels with a wgpu adapter, which
    the runners lack.
    Assumed: the frame time measured is egui's whole frame on the CPU
    (input, layout of the visible rows, tessellation), which is everything
    the UI code influences; the GPU upload of a few thousand triangles is
    not part of it. The test asserts the 95th percentile under 16.6 ms over
    240 scrolling frames of the 200k fixture, in release, on every runner.
    Decided (2026-11-06): as assumed; spec unchanged.

25. (issue 4) The GUI section says context-menu entries "that do not apply
    to the selection are hidden rather than greyed", while
    `Availability::Disabled` carries a reason meant for a tooltip.
    Assumed: the context menu hides both `Hidden` and `Disabled` entries,
    as the GUI section says; the command palette shows `Disabled` entries
    greyed with the reason as tooltip, which is what the variant is for.
    Decided (2026-11-06): as assumed; spec updated in 1309213.

26. (issue 4) Rename, New file and New folder are specified with "inline
    edit", and Delete permanently with a confirmation; actions ask through
    `UiHost::prompt` and `UiHost::confirm`, which block, and an immediate
    mode UI cannot block inside a frame.
    Assumed: the egui host answers in two passes. An action that asks
    without an answer queued gets `None` or `false` and its question is
    recorded; the shell shows a modal dialogue and, when the user answers,
    queues the answer and runs the action again. Inline editing in the list
    can replace the dialogue later without touching the actions.
    Decided (2026-11-06): superseded for Rename, New file and New folder,
    which edit inline (close-out issue A); `UiHost::prompt` remains for
    other actions.

27. (issue 4) `egui_dock` 0.21 builds against egui 0.36 while eframe,
    `egui_extras` and `egui_kittest` are at 0.37.
    Assumed: the whole egui stack is pinned to 0.36 until `egui_dock`
    moves; the dock is the spec's choice and must share egui's types.
    Decided (2026-11-06): as assumed; spec unchanged.

28. (issue 4) The model's `Pane` keeps its tabs in a `Vec`, and the spec's
    `Command` set has no way to reorder tabs within a pane, yet dragging a
    tab along the strip and "Move tab left / right" do exactly that.
    Assumed: tab order within a pane is display state the UI keeps in the
    `egui_dock` tree; the model's order is only the initial placement. Drag
    and drop between panes and into splits is translated into `MoveTab`,
    `CloseTab`, `ActivateTab` and `FocusPane` commands after each frame.
    Decided (2026-11-06): as assumed; spec unchanged.

29. (issue 4) Several GUI items are specified but not in this issue's task
    list or the 0.1 definition of done: the side panel with its sections,
    the Preview tab, the type filter with per-type counts, session restore,
    saved layouts, new windows and tab tear-out, column reorder and hide,
    dual pane (F6), the Extensions screen.
    Assumed: left for later issues. The inspector pane stands in for the
    Preview tab with a placeholder, as the issue asks; tabs cannot be torn
    out into windows yet.
    Decided (2026-11-06): superseded by the 0.1 close-out list and issues A,
    B, C.

30. (issue 5) "A 50 MB image … shows its first frame within 200 ms" names a
    budget no CPU decoder meets for a 50 MB JPEG or PNG: entropy decoding
    alone reads every byte, and a 200 ms frame needs the whole file in that
    time.
    Assumed: the previewer opens with the header (format, dimensions, EXIF)
    in a few milliseconds so the inspector shows them at once, and the first
    pixel frame comes as fast as the format allows: an uncompressed BMP is
    sampled straight from the file, a JPEG is decoded at the smallest DCT
    scale that covers the pane, every other format is decoded whole and
    downscaled. The acceptance fixture is a 48 MB BMP, which shows pixels
    well within 200 ms; a 50 MB JPEG shows its dimensions within 200 ms and
    its pixels after the decode, which never blocks the UI thread.
    Decided (2026-11-06): as assumed; spec unchanged.

31. (issue 5) `Frame::Text(TextBlock)` lets the UI lay text out, but the
    spec puts syntax highlighting in `ext-preview-code` with syntect, and a
    plain string cannot carry colours.
    Assumed: `TextBlock` gains `spans`, coloured byte ranges the previewer
    computed, and `RenderRequest` gains `dark` so the previewer can pick a
    theme that follows the UI (base16-ocean.dark, InspiredGitHub). Both
    types were shaped under question 8; this extends them.
    Decided (2026-11-06): as assumed; spec updated in 1309213.

32. (issue 5) The spec's Event list ends at `PreviewReady`; a file no
    previewer claims, or one whose previewer fails to open it, has no event.
    Assumed: `Event::PreviewFailed { pane, error }`, so the inspector can say
    why instead of showing a stale or empty preview. The alternative, a
    notification per unsupported selection, would be noise.
    Decided (2026-11-06): as assumed; spec updated in 1309213.

33. (issue 5) `PreviewDoc` must be `Send`, but pdfium's documents cannot be
    sent between threads and pdfium itself is single-threaded.
    Assumed: the PDF document keeps the file bytes and re-parses them on
    every render, which pdfium does lazily in a few milliseconds, and
    `pdfium-render`'s `thread_safe` feature serialises all calls. Each
    page's text is extracted when that page is first rendered, so the text
    layer fills as the user reads instead of delaying `open` by 300 pages
    of extraction.
    Decided (2026-11-06): as assumed; spec unchanged.

34. (issue 5) pdfium is a dynamic library the crate does not bundle.
    Assumed: the previewer binds it from `RICK_PDFIUM_DIR`, then from the
    executable's directory, then from the system, and says which to set
    when none works; CI downloads the release build for each runner. How
    the packaged application ships pdfium is a later decision.
    Decided (2026-11-06): pdfium ships beside the binary in every build and
    release (spec 0aaaf1a); `RICK_PDFIUM_DIR` stays as the development
    override.

35. (issue 5) The spec's code list includes TypeScript, F# and Visual
    Basic, and TOML is common, but syntect's bundled grammars have none of
    them.
    Assumed: TypeScript is highlighted with the JavaScript grammar; F#,
    Visual Basic and TOML render as plain text until grammars are added.
    Decided (2026-11-06): as assumed; spec unchanged.

36. (issue 5) The spec puts word wrap and line numbers on the text preview
    as persisted toggles, but names no setting for them.
    Assumed: both are toggles on the preview, kept for the session, not
    written to `rick.toml`; CSV grids, Markdown and HTML rendering and the
    pretty-print toggle are not in this issue.
    Decided (2026-11-06): superseded by the preview pane round: line numbers
    and wrap are persisted defaults, pretty print is never remembered (spec
    9e31f31, built in #10).

37. (issue 6) The issue asks for status, branch and ahead/behind "via gix"
    and for actions that gix does not implement (stash, push with
    credentials, pull with merge, blame).
    Assumed: the decorator and the info panel read through gix; the nine
    actions run the `git` command line as jobs through `ext::blocking`,
    with `GIT_TERMINAL_PROMPT=0` so a missing credential fails instead of
    hanging. Show log and Blame run synchronously and open their text
    through `UiHost::show`, a method added for them (a read-only text
    window; the spec's `UiHost` had notify, confirm and prompt only).
    Decided (2026-11-06): as assumed; spec unchanged.

38. (issue 6) The spec's `Decorator` has no way to say what stales its
    cache or to be told when it did, yet the issue asks for a cache
    "invalidated by watch on .git".
    Assumed: `Decorator` gains `invalidates_on(&Location) -> Vec<Location>`
    (locations the kernel watches for the pane; `.git` of the work tree
    here) and `invalidate(&Location)`. The app calls `invalidate` and
    decorates the whole listing again, debounced by 150 ms, when a watched
    location or the listed directory changes, and on an explicit refresh.
    Navigating within the same work tree keeps the cache.
    Decided (2026-11-06): as assumed; spec updated in 1309213.

39. (issue 6) The spec says every long operation is a `Job`, and the
    decorator's status read is one, but the listing jobs are deliberately
    kept out of the job table.
    Assumed: the `DecorateCtx` runner is a separate `JobRunner` without an
    event sink, so status reads do not flash in the operations area; they
    are cancelled with the pane's listing through the context's token.
    The Git actions themselves run on the app's runner and are listed.
    Decided (2026-11-06): as assumed; spec unchanged.

40. (issue 6) `Decoration` has one badge and one tint, but several
    decorators may decorate the same entry.
    Assumed: `Model::decorate` merges: a later decoration replaces the
    badge, the tint and the cells of the columns it names and keeps the
    other cells; a decoration that is still `Pending` only adds what was
    missing, so badges do not blink while a decorator looks again. Which
    badge wins between two decorators is the later one's; the spec would
    need a priority to do better.
    Decided (2026-11-06): as assumed; spec unchanged.

41. (issue 6) The context menu's extension groups show only `Enabled`
    actions, and the issue says the group is "available only when
    ActionCtx.location is inside a work tree".
    Assumed: every Git action is `Hidden` outside a work tree and `Enabled`
    inside; Discard changes and Blame are `Disabled` with a reason inside
    when the selection does not fit (nothing selected; not exactly one
    file), so they are absent from the context menu but explained in the
    palette.
    Decided (2026-11-06): as assumed; spec unchanged.

42. (issue 6) The local provider's watch is not recursive, so a watch on
    `.git` sees the index, HEAD, ORIG_HEAD and packed-refs change but not
    a loose ref under `.git/refs/`.
    Assumed: `.git` alone is watched, which covers commits, stashes,
    checkouts and staging done outside Rick; a fetch or push from outside
    that only writes a loose remote ref is picked up on the next change
    or refresh. Rick's own Git actions drop the cache themselves.
    Decided (2026-11-06): as assumed; spec unchanged.

43. (side panel) The GUI section specifies a side panel with Home, Recent,
    Network, Trash and Pinned, but the 0.1 definition of done does not list
    it and names "pin to side panel" a non-goal.
    Assumed: a first cut ships now: Home, Pinned and the operations area,
    with Recent, Network and Trash as sections that say what they will hold
    until the change feed, the network providers and the trash location
    exist. Pinning is done from a button in the Pinned section's header for
    the focused pane's folder, and unpinning from the entry; the context
    menu action stays out.
    Decided (2026-11-06): superseded by close-out A (#15): the sections, the
    conflict dialogue with pause, and mounted volumes as drop targets are
    built.

44. (side panel) The spec says the folded state and width persist across
    sessions, and session restore is a later issue.
    Assumed: visibility, side, width and the folded sections, and the pinned
    locations, are settings in `rick.toml` (`[side_panel]` and `pinned`),
    written like every other setting, so they survive a restart today and
    can move into the session file when it exists.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

45. (side panel) The spec puts the operations area at the bottom of the
    side panel; it does not say where it goes when the panel is hidden.
    Assumed: at the bottom of the window, as before, so a running job is
    never invisible.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

46. (side panel) The spec lets the panel "collapse to a narrow icon strip or
    hide entirely".
    Assumed: hide entirely (Mod+B, the Panel toggle in the top bar, the
    Settings screen); the icon strip comes with the Recent and Network
    sections, which would give it more than two icons.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

47. (GUI chrome) The spec's title row has no undo and redo buttons, which
    the old top bar had, and the burger menu lists New window, Settings,
    Extensions, Keyboard shortcuts and About only.
    Assumed: undo and redo stay reachable through their shortcuts and the
    search box's command mode; no button or menu entry is added until the
    spec names one.
    Decided (2026-11-06): as assumed; spec unchanged.

48. (GUI chrome) The spec says Rick draws its own chrome and on macOS "the
    traffic lights keep their place".
    Assumed: on Linux and Windows the window has no decorations and Rick
    draws minimise, maximise and close; on macOS the title bar is hidden
    with the content view at full size, so the system's buttons stay, and
    the title row leaves 78 points free for them. Which platform does what
    comes from `platform::Chrome`, not from `cfg(target_os)` in the UI.
    Decided (2026-11-06): as assumed; spec unchanged.

49. (GUI chrome) The search box's name search is specified as "the current
    location first, the platform index when a SearchIndex is available".
    Assumed: the current location only, as the issue scopes it; a hit is
    selected and scrolled into view on Enter. The index path comes with the
    first search-index extension.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

50. (GUI chrome) Thumbnails are specified "at the cell size, off the UI
    thread, cached per entry and size".
    Assumed: the UI asks the app with `Command::Thumbnail`, the app renders
    through the previewer that scores the entry highest as an internal job
    (not in the operations area, like listings) and answers with
    `Event::Thumbnail`; the UI keeps the textures per entry id and pixel
    size, up to 2048, and starts over past that. Folders never ask for one.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

51. (GUI chrome) The right-click on empty space is specified to open the
    folder's menu "with no selection", while a right-click on a row outside
    the selection selects that row first.
    Assumed: a right-click on empty space clears the selection as well, so
    the menu and the list agree about what the actions would act on.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

52. (GUI chrome) The spec turns the platform's title bar off and has Rick draw
    the chrome, but says nothing about the window's border, shadow or how it
    is resized once the system's decorations are gone with the title bar.
    Assumed, as asked for after the first cut: on platforms without native
    controls the window is transparent and Rick draws a 10-point margin
    holding a light shadow, a one-point border in the theme's window stroke
    colour, and invisible grips along the edges and corners that resize the
    window through the backend; a maximised or fullscreen window has no
    frame, and macOS keeps the system's border and resize handles.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

53. (Preview pane) The spec remembers the pinned info section "per content
    kind (image, text, PDF, …)" without naming the kinds.
    Assumed: three kinds derived from the document itself — `pages` when
    `pages()` is `Some` (PDF, animated GIF), `text` when it has a text layer
    and no pages, `image` otherwise — plus `other` for a file no previewer
    opens but a decorator describes. Stored as
    `[preview.pinned] text = "file"` in `rick.toml`.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

54. (Preview pane) The spec's Pretty print covers "JSON, XML, YAML and
    minified HTML, CSS or JavaScript".
    Assumed: JSON (through `serde_json`, key order kept) and XML and HTML
    (a tag-per-line indenter) get the toggle; YAML, CSS and JavaScript do
    not yet, since each needs a formatter of its own, so their files show
    no Pretty print control rather than one that does nothing.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

55. (Preview pane) The spec names `kamadak-exif` and `rexiv2` for "every
    EXIF, IPTC and XMP tag".
    Assumed: `rexiv2` binds the system's gexiv2 library and was not a
    dependency, so the EXIF section lists what `kamadak-exif` reads (every
    EXIF tag, thumbnail IFD included); IPTC and XMP wait for a reader.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

56. (Preview pane) The spec says the pane needs no heading because "the
    dock tab's label is enough", but the pane is still a side panel, not a
    dock tab (the preview-as-tab is a later issue).
    Assumed: the heading goes now; the panel has no label at all until it
    becomes a tab. The title row's toggle reads "Toggle preview pane".
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

57. (Preview pane) The spec says wrapped lines "break at the pane width"
    and asks for only the visible lines to be laid out.
    Assumed: lines break at any character, not at word boundaries, so a
    monospace line's row count follows from its length and the pane can
    place 200 000 lines without laying them out; a line whose real row
    count differs (tabs, wide glyphs) corrects the index on the next frame.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

58. (drag and drop) The spec's `Transfer` carries `EntryId`s and the source
    pane; items dropped from another application or another Rick process
    have neither, since they are in no pane's listing.
    Assumed: a `Command::Receive { items: Vec<Location>, to, mode }` next to
    `Transfer`, the same job behind it, with the outside rule (copy unless
    Shift) already applied by the UI. No other shape of the spec's enum is
    touched.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

59. (drag and drop) `TransferMode::Ask` "opens the move/copy prompt first",
    but `UiHost` offers yes/no and a text line only, and the prompt sits at
    the drop point, which only the UI knows.
    Assumed: the UI shows the prompt and sends `Move` or `Copy` in its
    place; an `Ask` that reaches the app is answered with a warning and
    does nothing. The prompt is Move, Copy and Cancel at the cursor;
    Escape cancels it.
    Decided (2026-11-06): as assumed; spec unchanged.

60. (drag and drop) The spec says a conflict on an existing name "asks once
    with skip, overwrite, rename or apply-to-all", and the operations area
    has "pause". Neither exists today: Paste fails on the first existing
    name and the job runner has no pause.
    Assumed: a drop behaves as Paste does (the job fails on the first
    conflict with the error shown), and the operations area offers cancel
    only, as before. The conflict prompt and pause are their own issue; the
    transfer job is written so a conflict policy drops into the pair loop.
    Decided (2026-11-06): superseded by close-out A (#15): the sections, the
    conflict dialogue with pause, and mounted volumes as drop targets are
    built.

61. (drag and drop) "Same volume moves, across volumes copies" needs the
    volume of the target folder; a `Location` has none.
    Assumed: the device half of the local `EntryId`s decides: the dragged
    items' device against the device of the target folder's entry (a
    folder row) or of the entries listed in the target pane (its space, a
    breadcrumb segment, a tab header). An empty target listing, the Home
    entry and a folder row that is a mount point read as the same volume,
    so the drag moves; the rename then falls back to a copy and remove
    inside the job when the device differs, so the outcome is the same and
    only the undo entry is missing.
    Decided (2026-11-06): as assumed; spec unchanged.

62. (drag and drop) `egui` 0.36 over `winit` 0.30 receives hovered and
    dropped files on Linux (Wayland and X11), macOS and Windows but
    reports no cursor position with them, and offers no way to start a
    drag: a Wayland source needs the `wl_data_device` and the press serial,
    X11 the XDND handshake on the window, macOS the `NSView` and the
    `NSEvent` of the press, Windows `DoDragDrop` on the window's message
    loop, none of which `eframe` hands out.
    Assumed: drop reception goes through egui's files on all three
    platforms; where the backend gives no position the drop lands in the
    focused pane's folder. The drag source is the `platform::DragSource`
    hook, called with the items' `file://` URIs when the cursor leaves the
    window; the three native implementations return `Unsupported` with the
    reason until the backend exposes what they need (winit's
    `DragEntered`/`DragMoved` events and a drag-source API are on its main
    branch), and the drag then stays inside the window. The fake platform
    records the URIs, which is what the acceptance test checks.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

63. (drag and drop) A pinned entry is a `Location`; the spec wants a pinned
    folder to open in the pane and a pinned file to open its folder with
    the file selected, and the UI has no way to tell the two apart without
    looking at the file system.
    Assumed: the side panel asks the file system for the kind of a local
    pinned path when it is clicked (one metadata call) and takes a pinned
    location of any other provider for a folder. Pinned locations are
    written as `provider:path` strings; a bare path in an older file is
    read as local.
    Decided (2026-11-06): as assumed; spec unchanged.

64. (drag and drop) The spec's targets include "a mounted volume under
    Network", and the Network section holds nothing yet.
    Assumed: Home is the one side-panel folder target today; volumes join
    it when the Network section is filled.
    Decided (2026-11-06): superseded by close-out A (#15): the sections, the
    conflict dialogue with pause, and mounted volumes as drop targets are
    built.

65. (drag and drop) The spec has `Pin to side panel` in the context menu's
    Open group "of every entry"; actions carry a fixed label, and the flip
    to `Unpin` depends on the selection.
    Assumed: two actions, `core.pin` and `core.unpin`, each hidden when the
    other applies (Pin when everything selected is pinned, Unpin otherwise),
    so menu and palette show one of them; the pinned list stays in the
    settings and the actions reach it through a `PinStore` the app provides.
    Spring-loading into a tab header switches the pane to that tab and
    "return" switches back; a drop on a header never switches.
    Decided (2026-11-06): as assumed; spec unchanged.

66. (session) The spec's shortcut table binds Mod+N to New file, the
    Windows paragraph of General behaviour opens a new window with Ctrl+N,
    and the issue asks for Mod+N to open a window.
    Assumed: Mod+N opens a new window, as the issue and the Windows
    paragraph say; `core.new_file` has no default shortcut until the spec
    names one (it stays in the menu and the palette and is rebindable in
    Settings → Keyboard shortcuts).
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

67. (session) The issue wants "egui_dock's serialised tree" in the session,
    and `model` may not depend on `egui_dock` (only `ui-egui` knows egui).
    Assumed: the model keeps a mirror of what that tree holds that matters
    across a start, the splits with their fractions (`Dock::Split` gained
    `fraction`) and each pane's tabs in strip order with the active one, and
    the UI reports the tree's changes to it after every frame
    (`Command::TabOrder`, `Command::Layout`). Question 28 is thereby
    narrowed: tab order is still the dock's display state, but it is
    reported to the model and is part of the session. Leaf rectangles and
    scroll positions of the strips are not kept.
    Decided (2026-11-06): as assumed; spec unchanged.

68. (session) The issue names `show_viewport_deferred` as the mechanism for
    further windows. A deferred viewport's callback must be `Send + Sync +
    'static`, so the whole UI state would live behind a lock, which the
    test harness's `&App` accessors cannot give.
    Assumed: `ViewportBuilder` per window as specified, shown with
    `show_viewport_immediate`, so every window draws from the one `RickUi`
    in the parent's frame; the windows share one app, one job table and
    one undo log either way. eframe's root viewport cannot close while its
    children stay, so closing the first window with others open moves the
    root viewport to the next window (position, size and state follow);
    the window that closes is the model's.
    Decided (2026-11-06): as assumed; spec unchanged.

69. (session) "Continue where I left off" names the last session, and the
    session file is overwritten continuously, so after starting with a
    layout the file no longer holds where the user left off.
    Assumed: the session file as read at start is kept in memory for the
    run and is what the entry switches to; at the next start it is the
    file. New window "opens with the start layout": with a named start
    layout it is that layout's focused window; with `start = "last"` it is
    one tab at the focused tab's location, since the last session is the
    whole set of windows already on screen. A path on the command line
    opens in a new tab of the restored session's focused pane rather than
    replacing the session. The issue's "window-qualified `DockTarget`" is
    `DockTarget::Into(pane)` as before, since pane ids are unique across
    windows, plus `DockTarget::NewWindow` for a tear-out.
    Decided (2026-11-06): as assumed; spec unchanged.

70. (session) The spec's `Command` list has nothing for sessions, layouts
    and windows, and question 44 put the side panel's state in `rick.toml`.
    Assumed, as the issue asks: the side panel's side, width, collapsed
    state and folded sections move from the settings to each window's
    session state (`Command::SidePanel`; an old `[side_panel]` table in
    `rick.toml` is ignored), the Settings screen's side panel row changes
    the window it is open in, and the commands `TabOrder`, `Layout`,
    `Scroll`, `WindowGeometry`, `SidePanel`, `FocusWindow`, `NewWindow`,
    `CloseWindow`, `SaveLayout`, `SwitchLayout`, `DeleteLayout` and
    `RenameLayout` join the list, to be added to the spec. Settings gained
    `[session] start = "last" | "<layout name>"` as the issue names it; a
    layout cannot be called `last`.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

71. (session) The issue says a tab whose location no longer exists "opens
    at the nearest existing ancestor and says so in the status line", and
    the spec's status line holds counts only.
    Assumed: `Tab` carries a `note` the status line appends after the
    counts, cleared by the next navigation; the check at start is a
    synchronous `std::fs` look at the local path and its ancestors (the
    only provider in 0.1), locations of a provider that is not mounted open
    at Home with a note, and locations of any other mounted provider are
    taken as they are.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

72. (close-out A) The spec's View shortcut table gives Mod+Shift+F to
    "Toggle flatten subfolders", while the Main area and the issue give
    Mod+Shift+F (Ctrl+Shift+F) to Search in subdirectories.
    Assumed: the issue wins. Mod+Shift+F toggles the subtree search and
    focuses the filter; Flatten keeps its intent and palette entry with no
    default binding, rebindable in Settings → Keyboard shortcuts.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

73. (close-out A) The spec names the side panel's Tree view "as a section
    in the side panel and as its own dockable tab", but the side panel table
    lists five sections and gives the Tree no place.
    Assumed: `Section::Tree` is the sixth section, after Pinned, folding and
    persisting like the others; it is rooted at the top of the focused
    pane's volume (`/`, the drive root), lists folders lazily as they are
    opened, and follows the focused pane unless its link toggle is off. The
    dockable Tree tab is not in issue A.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

74. (close-out A) The `Trash` platform trait is specified as "trash, list,
    restore, empty", and the Trash section's Delete permanently needs to
    remove one trashed item without restoring it.
    Assumed: `Trash::remove(&TrashedItem)` joins the set, implemented beside
    `empty` on every platform and the `Fake`. Trash items are not entries of
    any provider in 0.1, so the trash actions (`core.restore`,
    `core.restore_trash`, `core.purge_trash`, `core.empty_trash`) take an
    `ActionCtx` whose location is the `trash` pseudo-provider and whose
    selection holds one synthesised entry per item (`ops::trash_entry`);
    they are `Hidden` outside that context. Restore is undoable as a trash
    of the restored entries; a restore into a taken path is reported, not
    resolved (QUESTIONS.md 11 stands). On macOS, and wherever `Trash::list`
    answers `Unsupported`, the section shows Rick's own record of what it
    trashed, persisted in `trash.toml` in the config directory, and says
    that items trashed elsewhere are not listed. Supersedes 15's "cannot
    be undone" for the restore direction.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

75. (close-out A) The spec feeds Recent "by file-system events, not only by
    what Rick touched", and the local provider's watch is not recursive
    (QUESTIONS.md 42).
    Assumed: Recent is fed by the non-recursive watches on its roots (the
    home directory and the pinned local folders), by every pane's own watch,
    and by a bounded scan at start (the newest 200 of up to 50 000 entries
    per root, 5 s at most). A change deep under Home that no pane shows is
    picked up by the next scan. The list persists in `recent.toml`, a path
    per row, re-examined on start so the rows carry current ids.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

76. (close-out A) "udisks2 on Linux" for the volume list would need a D-Bus
    client; `rfd` already pulls `zbus` in, but only as its portal backend.
    Assumed: Linux reads `/proc/mounts` and the kernel's removable flag from
    `/sys/block/<disk>/removable` (what udisks2 itself reads), filtering
    pseudo file systems, the system disk and its bind mounts; network mounts
    are told by type. The list is polled every 3 s, so media that come and
    go show without a user action. udisks2 proper comes with mount and
    unmount in 0.2.
    Decided (2026-11-06): as assumed; spec unchanged.

77. (close-out A) The conflict dialogue replaces 20's "never overwrite":
    Paste, Duplicate's siblings (Transfer, drag and drop, undo of a move)
    ask once per conflict with Skip, Overwrite, Rename, Apply to all and
    Cancel; the job pauses at the first conflict. Assumed details: Overwrite
    of a folder onto a folder merges (files replaced, others kept), of a
    file replaces it, and of a folder onto a file (or the reverse) removes
    the target first; Rename proposes `name (2).ext` and a Rename applied to
    all numbers each remaining name itself with the first free number; the
    undo of a move that overwrote moves the item back but cannot bring the
    overwritten target back. Conflicts are values in `types`
    (`Conflict`, `ConflictAnswer`) since they cross from `ops` to the UI.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

78. (close-out A) Pause in the operations area needs a job-side gate the
    spec's `JobCtx` does not name.
    Assumed: `JobCtx` gains `pause: PauseGate`, `JobRunner` gains `pause`,
    `resume` and `spawn_pausable`, and `JobEvent::Started` says whether a
    job is pausable; copy and move jobs are pausable and wait at the gate
    before each item, directory, file and streamed buffer. Pause buttons
    show only for pausable jobs, as the issue asks. `Command` gains
    `PauseJob`, `ResumeJob`, `Subtree`, `TreeExpand`, `TreeCollapse`,
    `FolderTreeToggle`, `FolderTreeLink`, `ResolveConflict`, `RefreshTrash`
    and `RefreshVolumes`, and `Event` gains `Conflict`, all in the spirit of
    the spec's enum; the spec block should list them.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

79. (close-out A) The spec's "slow second click" has no delay. Assumed: a
    plain second click on the one selected row at least 0.5 s after the
    first (past egui's double-click window) starts the rename; a faster
    second click opens the entry as a double-click does.
    Decided (2026-11-06): as assumed; spec updated in 12f3b25.

80. (release) Two macOS flakes on the release pull request's CI: the tree
    view's first screen measured 80.8 ms once against an 80 ms target set
    at median × 1.5 (median 42 ms, worst 50.5 ms in five runs), and the
    search box test counted a third label with a file's name because the
    side panel's Recent section, fed by every open pane's watch since
    close-out A, listed the test's freshly written files before the
    assertion ran, which the FSEvents timing on macOS allows.
    Assumed: on macOS and Windows a timed test repeats its measured part
    three times and asserts the median against the target, whose rule
    (median × 1.5, rounded up, never below Linux) is unchanged; Linux keeps
    the spec's numbers on a single run. Label counts in the integration
    tests are scoped to the widgets they are about (the file list and the
    search dropdown), never to the whole window, since the side panel may
    list the same names.
    Decided (2026-11-06): as assumed; spec unchanged. BENCHMARKS.md says so
    in its Targets paragraph.

81. (issue 16) The spec leaves the plugin ABI to the implementer: "dynamic
    linking of the kernel crates, or a C-ABI vtable", recorded here.
    Decided (2026-11-07): **Rust dylibs.** `types`, `runtime` and `ext` are
    `crate-type = ["dylib"]`, the whole workspace builds with
    `-C prefer-dynamic` (`.cargo/config.toml`; cargo gives a workspace
    member's dylib a static std otherwise, which no consumer can link next
    to the dynamic one), and macOS adds `-C rpath` so the dylibs get an
    `@rpath/…` install name. The binary carries a run path to its own
    directory (`rick`'s build script, reading `CARGO_CFG_TARGET_OS`, which
    is build configuration and not a `cfg(target_os)` in code). A plugin is
    a `cdylib` + `rlib` crate that exports one `extern "C"` function,
    `rick_plugin_register(&mut Registry) -> Status`, through `ext::plugin!`;
    the five traits, `JobRunner`, `Arc<dyn Provider>`, `Box<dyn
    PreviewDoc>` and the error type cross the boundary as themselves, since
    both sides run the same code from the same shared libraries, and the
    plugin's jobs run on the host's tokio runtime, its tracing events reach
    the host's subscriber and its contributions land in the host's registry
    with nothing duplicated. A panic during registration is caught on the
    plugin's side of the `extern "C"` boundary and reported as a status.
    Constraints: (a) the same toolchain, the exact `rustc -V` line, which
    the manifest records and the host checks before loading; (b) the same
    workspace build, since cargo's feature unification and crate metadata
    fix the symbol names — a plugin built elsewhere fails at `dlopen` with
    an unresolved symbol, which the host reports as a refusal; (c) the
    bundle ships `libstd-<hash>`, `libtypes`, `libruntime` and `libext`
    beside the binary (cargo names workspace dylibs and cdylibs without a
    hash, only `libstd` keeps one); (d) `RUSTFLAGS` must stay unset, as it
    replaces the config's flags and the build then fails with "cannot
    satisfy dependencies so `std` only shows up once"; (e) a crate that
    both a plugin and the host use but no kernel dylib contains is
    duplicated, which is harmless unless it holds process-wide state — the
    kernel dylibs hold tokio, rayon, tracing, futures, thiserror, smallvec,
    semver and bitflags, so the state that matters is shared, and a plugin
    must take `tracing`, `tokio` and `rayon` through `ext` and `runtime`
    rather than depend on a copy of its own; (f) every build is a build of
    the whole workspace with its dev targets (`cargo test --workspace`,
    `cargo build --workspace --all-targets`, `cargo xtask bundle`), because
    cargo names a workspace dylib without the metadata hash and so one file
    serves every feature set — a `-p` build of a crate below `ext` rewrites
    the shared libraries for its own, smaller feature set (measured: the
    `plugins-native` tests alone give another `libruntime` and `libext`),
    and the next whole-workspace build recompiles them and everything above
    them; `cargo clean -p types -p runtime -p ext` when the two disagree.
    For the same reason `xtask` depends on no kernel crate: it drives
    `cargo build --workspace --exclude xtask --lib --bins --examples` (the
    libraries, binaries and examples of every member but itself, with the
    dev-dependency features the tests build with; a rebuild of the running
    `xtask` binary is what Windows refuses) and a helper binary from that
    build, never a kernel dylib of its own.
    Rejected: a C-ABI vtable layer (`abi_stable` or hand-written). The five
    traits are async (boxed futures, `BoxStream`, `AsyncRead` handles), the
    host hands the plugin `Arc<dyn Provider>`, `&JobRunner`, `&DecorateCtx`
    and `&dyn UiHost`, and the plugin hands back `Box<dyn PreviewDoc>` and
    `Arc<dyn Decorator>`: every one of these would need an FFI-safe mirror
    with adapters in both directions, the plugin would still carry its own
    tokio, tracing and rayon whose statics the host cannot reach, and
    "one runtime, one subscriber" would mean a forwarding shim for each —
    the opposite of the issue's "without duplicated statics". A C ABI stays
    the right tool for plugins built outside the workspace or by another
    compiler, which 0.2 does not promise.
    Proven by `crates/plugin-probe` (a Previewer whose `open` spawns a job,
    plus an Action whose job waits for cancellation) and `crates/plugin-panic`,
    loaded by `crates/plugins-native/tests/probe.rs` on Linux, macOS and
    Windows in CI.

82. (issue 16) The spec allows `unsafe` only in `platform`, and the plugin
    host it asks for must call `libloading` and a symbol it resolved, which
    is unsafe by nature; a plugin crate must declare its entry point with
    `#[unsafe(no_mangle)]`, which the `unsafe_code` lint also flags.
    Assumed: `plugins-native` is the second crate allowed `unsafe`, for the
    loader calls only, each block with a `// SAFETY:` comment like
    `platform`'s; the `ext::plugin!` macro carries the `#[allow(unsafe_code)]`
    for the one attribute, so a plugin crate writes no `unsafe` of its own.
    A plugin crate uses `#![deny(unsafe_code)]` rather than `forbid`, since a
    `forbid` would refuse the macro's allowance.

83. (issue 16) The issue lists the fields of `plugin.toml` — id, name,
    version, `api_version`, points, platforms, toolchain, library — and says
    `ext::Manifest` "gains the fields it lacks", with the TOML "generated at
    build time from the crate's Cargo.toml and the manifest constant".
    Assumed: `Manifest` gains `points` and `platforms`, which an extension
    declares about itself (the registry refuses a contribution to a point the
    manifest does not declare, so the declaration stays honest); `toolchain`
    and `library` describe the compiled file, not the extension, so they
    live in the file only (`plugins_native::PluginToml`). The file is
    written by `cargo xtask bundle`, the build step that lays the bundle
    out: it loads the freshly built library through the workspace's
    `rick-plugin-manifest` helper, reads the manifest the entry point
    registers (its `version` comes from the crate's `Cargo.toml` through
    `CARGO_PKG_VERSION`), adds the toolchain of the build and the library's
    file names, and writes `plugins/<id>/plugin.toml` beside a copy of the
    library. At start the host checks the file against the library it loads
    (id, version, points) and refuses a stale one. The extension API is
    version 3 for the manifest change.

84. (issue 16) The issue places the bundle in `target/<profile>/rick/`, the
    path of the `rick` binary itself on Linux and macOS, where cargo writes
    it; a directory cannot take its place.
    Assumed: `cargo xtask bundle` lays the bundle out in
    `target/<profile>/bundle/` (the binary, the kernel dylibs and `libstd`,
    pdfium when `RICK_PDFIUM_DIR` names it, `plugins/<id>/` per plugin), and
    the release archives are that directory's contents.

85. (issue 16) The issue says a plugin that panics during registration is
    "caught, unloaded and reported". Unloading proved unsafe: the first CI
    run of the pull request loaded the probe plugin, dropped it, loaded it
    again at the same address, and `tracing` panicked on a callsite it had
    already registered — the first load's `info!` had put a pointer to the
    library's static into tracing-core's process-wide callsite list, which
    `dlclose` left dangling. Thread-local destructors would do the same.
    Assumed: a library whose entry point ran is never unmapped, whatever
    became of it; a refused or panicking plugin is caught and reported, its
    contributions dropped, and its code stays mapped and idle until the
    process ends (`Loaded` holds the library in a `ManuallyDrop`). Only a
    file that is no plugin at all (no entry point) is unmapped again. The
    cost is the memory of a plugin that could not be used.

86. (side panel) The spec leaves the scheme of the virtual locations to the
    implementer and suggests `rick://favorites`, `rick://recent`,
    `rick://trash`, `rick://machine`, `rick://network` and `rick://cloud`.
    Assumed: those six names, served by one built-in provider in the new
    crate `vfs-rick` under the provider id `rick`, each location's path
    being the bare name, so the session and places files write them in the
    `provider:path` form every location uses (`rick:trash`); the path bar
    accepts the `rick://trash` form typed anywhere. The 0.1 `trash`
    pseudo-provider of the trash actions is folded into it.

87. (side panel) The Cloud section needs to know which providers are cloud
    ones and the Network section which are host-addressed connections, and
    the `Provider` contract has no kind: the manifest of the plugin host
    (#27) declares the points an extension fills and the platforms it runs
    on, not what kind of provider it mounts.
    Assumed: a mounted provider counts as cloud when its id is `s3`, `azure`
    or `gcs` and as network when it is `sftp`, the ids the spec's package
    layout fixes; the Cloud rows are what each cloud provider lists at its
    root (its accounts or containers), fetched at start and whenever the
    Cloud overview lists; the *Add…* row of an empty Network or Cloud
    section says that a provider plugin creates connections and accounts,
    since none exists yet. To be replaced by a registry contribution once a
    provider can declare its kind. The integration tests prove the section with an in-memory
    provider under the `s3` id.

88. (side panel) The standard folders and the system's recently-used list
    are platform services the `platform` trait set did not have.
    Assumed: `StandardFolders` (XDG user-dirs on Linux, the known folders
    on Windows, the fixed directories under the home on macOS, `Video`
    being `Movies` there) and `RecentlyUsed` (`recently-used.xbel` on
    Linux; the Recent folder's shortcuts resolved with `IShellLinkW` on
    Windows; Spotlight's `kMDItemLastUsedDate` over the home for the last
    thirty days through `mdfind` on macOS, which keeps its Recent Items in a
    private store) join the set, implemented on the fake with three folders
    under its home and a settable list. The Recent view sorts by date
    accessed descending, the use time being the entry's `accessed`. This
    Machine lists the system disk too, as `System`, since `/` has no last
    component to name it by, with the file system name added to `Volume`.

89. (operations) "Jobs are scheduled per volume" needs a volume key and a
    place in the job's life to wait.
    Assumed: the key is the device half of the local entry id (of the
    nearest existing ancestor for a target that does not exist yet) and the
    provider id for every other provider; copy, move and its undo, duplicate
    and a drag's transfer acquire their keys first thing through an
    `ops::Scheduler` the application owns, queued first in first out, and a
    cancelled job releases them by dropping its lease; trash, restore,
    rename, new file and new folder, which end in milliseconds, do not
    queue. The waiting job's row shows `JobState::Waiting(name)` with the
    name of the first running job holding one of its keys. Progress events
    are coalesced in the runtime to one per 50 ms per job, a message (a
    conflict, a wait) going out at once, so a job over hundreds of thousands
    of entries never redraws the area per entry.

90. (side panel) Editing Places: the spec names Remove, Edit… (title and a
    Phosphor icon, `FOLDER` by default) and drag reorder, without saying
    what a dropped folder is called, how the icon is picked from 1 500
    glyphs, or whether edits are undoable.
    Assumed: a dropped folder's title is its name and its icon `FOLDER`;
    Edit… offers a curated set of forty Phosphor icons by name (the names are
    what `places.toml` stores, an unknown name shows the folder); place edits
    are not undoable, the spec listing none among the undoable operations;
    removing the place of the folder a tab shows leaves the tab where it is.
    Clicking the place of the folder the tab already shows lists it again
    instead of emptying it, which 0.1's `Navigate` to the current location
    did (a latent bug the Places rows would have exposed).

91. (side panel) Eject and "the list follows mounts live".
    Assumed: Eject runs as a job, so a refusal lands in the operations
    area: `udisksctl unmount` then `power-off` on Linux (`umount` where
    udisks2 is absent), `diskutil eject` on macOS, lock, dismount and
    `IOCTL_STORAGE_EJECT_MEDIA` through `DeviceIoControl` on Windows; the
    fake records the mount point and forgets the volume. "Live" is the
    platform's volume list polled every two seconds (76 continued, which
    polled every three); udisks2 and the Windows device notifications come
    with milestone 5's mount and unmount.

92. (side panel) The Trash view's operations and the other views' entries.
    Assumed: the trash actions (`core.restore`, `core.restore_trash`,
    `core.purge_trash`, `core.empty_trash`) move into the context menu of
    the Trash view as a group of their own and are hidden elsewhere; in the
    view Delete deletes permanently and Enter or a double-click restores;
    the file actions are hidden there, and the ones that write to the
    location (Paste, Rename, New file, New folder) in every virtual view,
    while Copy, Open, Copy path, Pin and Move to trash work on the real
    entries the Favorites and Recent views list. Those entries carry their
    own provider and path (`Tab::location_of` follows the entry's provider),
    so Open, Open in new tab and Unpin work; a pin whose path is gone stays
    listed with a hashed id so it can be unpinned. Nothing can be dropped
    into a virtual view.

93. (side panel) Keyboard: "with the panel focused" is not defined.
    Assumed: a click on a panel row gives the panel the keyboard, a click
    anywhere else gives it back; Up and Down move over every row including
    the section headers and *Add…*, Enter opens, Mod+Enter in a new tab,
    Escape releases. The Edit… dialogue and the reorder drag are the
    panel's own state, not the model's.

94. (issue 20) The trash record of QUESTIONS.md 74 lived in `app`
    (`trash.toml` through `ops::TrashStore`, which the `rick://trash` view
    of 86 fell back to as `TrashRecord`); the trash provider plugin, which
    depends on `ext` and `types` only, cannot reach it.
    Assumed: the record moves into `platform`, behind `Trash::list`: on
    macOS the trash itself writes `trash.toml` in the config directory when
    it trashes, forgets on restore and removal, and `list` reads it back
    pruned of items that left the trash, so the local provider's trashing
    and the Trash view see one list and `list` never answers `Unsupported`
    any more. `TrashStore`, `TrashRecord` and the `Fake`'s unlistable knob
    go; the record module carries its own tests.

95. (issue 20) Milestone 2 (86) made `vfs-rick` the one provider of the six
    virtual locations, built in; the issue wants the Trash view served by
    the plugin `ext-vfs-trash`, and Favorites, Recent and the overviews are
    to follow the same road later.
    Assumed: an extension serves a virtual location by registering a view,
    `Registry::view(name, provider)`, which the application hands to
    `RickProvider::add_view`; the rick provider then routes that location
    and everything below it (`rick://trash`, `rick://trash/<item>/…`) to the
    view's provider for `list`, `stat`, `open_read`, `remove` and the rest,
    including `capabilities`, keeps `watch` itself so `changed` still
    re-lists the view, and lists a location no extension serves as empty.
    The view's provider sees the full path (`trash/<name>`), answers under
    `ProviderId::RICK` and lives in a plugin that knows nothing of
    `vfs-rick`. The path bar takes `rick://trash/<name>` as typed.

96. (issue 20) Plugins depend on `ext` and `types` only (Module rules 2),
    but the trash provider needs `platform::Trash` and the archive provider
    the runtime's blocking pool.
    Assumed: `ext` re-exports `platform` and `runtime`, and the `Registry`
    is the extension context the Package layout names: the host builds it
    with `Registry::with_platform` and an extension takes
    `Registry::platform()` while it registers (`None` in a registry built
    with `Registry::new()`, which the trash extension refuses). The plugin
    host and the binary pass the platform on; the tests pass the `Fake`, so
    a plugin's trash is the test's. `platform` becomes part of the `ext`
    dylib; the layer order is unchanged (platform sits below ext).

97. (issue 20) "Entering an archive … navigates into it as a location
    (archive: provider); the breadcrumb shows the archive as a segment"
    does not say what the location's path is.
    Assumed: the archive's OS path with the path inside appended,
    `archive:/home/me/a.zip/docs/x`. Which prefix is the archive is found
    by asking the filesystem for the shortest prefix that is a file
    (`ext_vfs_archive::split`); the UI does the same once per location for
    its breadcrumbs, whose segments above the archive navigate the local
    provider, and Up from the archive root goes to its folder. Open
    (Enter, double-click) hands a local file to the archive provider first,
    whose `stat` says whether it browses the file; anything else opens with
    the system default. An archive inside an archive is a file, not a
    location: the archive previewer of #18 is the way to look inside
    without extracting. Entry ids hash the archive path and the inner path,
    so they are stable across listings and survive a rewrite.

98. (issue 20) "Change notification by polling (watch returns
    Unsupported)" names no poller.
    Assumed: the kernel polls. For a pane whose provider's `watch` answers
    `Unsupported` (an archive, a trashed folder browsed below the view),
    `app` calls `stat` on the location every 3 s and right after an
    operation finished, and lists again when the entry's size or times
    changed; an archive's root entry carries the archive file's size and
    time. The Trash view itself is re-listed the milestone-2 way: `app`
    lists the platform trash after every operation for the Places row's
    count and rings `RickProvider::changed` when the list changed.

99. (issue 20) Two plugin libraries linked into one binary (the tests, or
    `rick` with two plugin features on) both exported
    `rick_plugin_register` and the link failed.
    Assumed: the exported symbol carries the crate name,
    `rick_plugin_register_ext_vfs_trash`, which `ext::plugin!` takes from
    `CARGO_CRATE_NAME`; the host and the bundler derive it from the
    library's file name, so `plugin.toml` needs no new field. The Rust
    function keeps its name, `rick_plugin_register`.

100. (issue 20) Restore, Restore trash items and Empty trash are in the
    spec's operations list without a context-menu group, and "live inside
    the view only"; 92 had them as core actions over the application's
    record.
    Assumed: they are the plugin's, in the "Trash" action group
    (`ActionGroup::Extension("Trash")`): `trash.restore`,
    `trash.restore_items` and `trash.empty`, Hidden unless the pane is at
    `rick://trash`, so the context menu offers them under Trash in the view
    and nowhere else, and the palette lists them there too. Delete
    permanently in the view is the core action through the view's
    `remove(Permanent)`; Move to trash hides there, as 92 says, and the
    other file actions with it. Enter and a double-click still restore
    (the shell runs `trash.restore_items`). Without the plugin the view
    lists nothing. The Places row keeps its count hint (92).

101. (issue 20) Which core actions apply at a location another provider
    serves; 92 settled the virtual locations only.
    Assumed: the actions follow the provider's capabilities, as the
    Provider contract says the kernel should: Paste, Duplicate, Rename, New
    file and New folder are Hidden without `WRITE`, Move to trash without
    `TRASH` (except in Favorites and Recent, whose entries are real files
    elsewhere), so a read-only archive, the Trash view and a RAR show only
    what works. The inline edits of New file and New folder start only
    where the provider takes writes.

102. (issue 20) How a write into an archive happens.
    Assumed: zip, the tar family and 7z declare `WRITE | RENAME`; every
    `open_write`, `create_dir`, `rename`, `remove` and same-archive `copy`
    rewrites the whole archive into a temporary file beside it and renames
    it over the original, under a lock per archive, inside the operation's
    job (the rewrite reports no progress of its own). Zip entries keep
    their compressed bytes, a compressed tar is recompressed with its
    codec, 7z entries are re-encoded with LZMA2, new zip entries use
    DEFLATE. RAR, ISO 9660 and UDF images and single-file codecs are
    read-only. A `.iso` is tried as ISO 9660 first, then as UDF; a codec
    file whose stream holds a tar is a tar.

103. (issue 18) The spec's `PreviewDoc` returns `Frame::{Rgba, Text,
     External}`, and the archive, CSV and rendered Markdown/HTML previews
     need the pane to lay out a file list, a table and a document; the
     issue's media transport asks for volume, speed, frame step and loop,
     which the spec's `Transport` does not have. Changing `Frame` would
     have touched `ui-egui`, which the viewer (issue 17) is reworking in
     parallel.
     Assumed: `PreviewDoc` gains `content(&self) -> Option<Content>` with a
     default of `None`; `Content` is `Listing(Arc<Listing>)` (entries in
     explorer order with depth, kind, size, compressed size, date and an
     encrypted flag, plus a stats line), `Grid(Arc<dyn Grid>)` (header, row
     count, and the cells of any window of rows, parsed on demand) or
     `Document(Arc<Document>)` (headings, paragraphs, styled inlines, links,
     lists, quotes, code, tables, images, rules). `render` keeps answering
     for such a document with a textual fallback (the listing as text, the
     raw CSV, the highlighted source), so the 0.1 pane shows something
     sensible until the viewer lays the content out. `Transport` gains
     `volume`/`set_volume`, `speed`/`set_speed`, `step` and
     `looping`/`set_looping` with defaults that do nothing, so the viewer's
     fake transport keeps compiling. `Icon` gains `Rendered`. The extension
     API is version 4.

104. (issue 18) The office previewer: the issue says LibreOffice is found
     "through platform", but a plugin may depend on `ext` and `types` only
     (SPEC.md "Module rules"); and the spec names no place for the cache of
     converted PDFs, while plugins have no access to the config directory.
     Assumed: the plugin looks for `soffice` itself: `RICK_SOFFICE` (a
     development override like `RICK_PDFIUM_DIR`), then the `PATH`, then the
     install directories of all three platforms checked as plain paths (a
     path that does not exist on the current OS is simply absent), so no
     `cfg(target_os)` is needed; a hit is remembered, a miss is looked up
     again next time. The cache lives under the temporary directory,
     `<temp>/rick/office/<key>.pdf` with `<key>` a hash of the file's path,
     size and modification time (a changed file misses the cache by
     itself), trimmed to 512 MiB and 200 files, oldest out; LibreOffice runs
     with its own user profile under the same directory so that an open
     LibreOffice does not swallow the command. A conversion runs inside the
     preview job on the blocking pool, cancellable (the process is killed)
     and limited to 60 s; it is not a separate row in the operations area,
     since the preview job already is the job. The office pages render
     through the plugin's own pdfium binding (a mirror of the PDF
     previewer's; a plugin cannot call `ext-preview-pdf`), which means two
     copies of `pdfium-render`'s call serialisation exist in the process; a
     PDF and an office page rendering at the same instant are not
     serialised against each other. To revisit if pdfium shows it.

105. (issue 18) The spec and the issue say nothing about encrypted entries
     in an archive listing.
     Assumed: an entry whose content is encrypted (zip and RAR with a
     password, 7z without header encryption) is listed with its name and
     size and marked `encrypted`, and the stats line counts them; an archive
     whose directory itself is encrypted (7z `-mhe`, RAR `-hp`) lists
     nothing, its stats line says the list of files needs a password, and
     the status bar says "encrypted"; no password is ever asked for in the
     preview. Also assumed: a listing stops after 200 000 entries or, for a
     compressed tar (which has no directory and must be streamed), after
     10 s, and says so; symbolic links in zips show without their target
     (it is the entry's content). Archives are case-sensitive: `Alpha/` and
     `alpha/` are two folders, ordered without regard to case.

106. (issue 18) GStreamer: the spec bundles the runtime on macOS and
     Windows and the issue asks for "a clear runtime missing state"; the
     GStreamer Rust bindings link the libraries at build time, so a plugin
     built with them needs the development files to build and cannot load
     where the core library is absent.
     Assumed: playback sits behind the media plugin's `gstreamer` Cargo
     feature, off by default so every workspace build still builds; Linux
     CI and the release builds turn it on (`--features
     ext-preview-media/gstreamer`, which `cargo xtask bundle` passes
     through), with the system packages on Linux and the official
     GStreamer packages (1.28.8) installed on the macOS and Windows runners.
     The bundle carries the runtime where `RICK_GSTREAMER_DIR` names it:
     macOS `gstreamer/lib/` (the libraries name each other through
     `@rpath`, which the media plugin's run path `@loader_path/../../
     gstreamer/lib` and the binary's `@executable_path/gstreamer/lib`
     resolve) and the elements in `gstreamer/lib/gstreamer-1.0/`, thinned
     to arm64 by the release workflow; Windows the DLLs beside the binary,
     where every DLL the process loads is searched for, and the elements in
     `gstreamer-1.0/`; the plugin scans those directories at start, and
     Linux uses the system's. The missing-runtime state has three shapes:
     built without the feature, or GStreamer failing to initialise, the
     document shows the file's metadata (read natively from the headers and
     tags) and says so; a core library absent on Linux, where the plugin
     cannot be loaded at all, the Extensions screen and `--list-plugins`
     show it refused with the loader's message; a decoder or demuxer
     missing for one file (the usual case: codec plug-ins not installed),
     the document shows the metadata and names the missing plug-in and the
     package family to install. A plugin written against GStreamer's C API
     with `libloading`, which would have given the first shape in every
     case, would have put hundreds of `unsafe` calls into a plugin; a helper
     process would have doubled the code. Hardware decoding is GStreamer's
     own choice of the highest-ranked decoder. On a machine without an audio
     device (CI) playback runs with a `fakesink` and says so.

107. (issue 18) The CSV grid's header and window: the spec says "a
     virtualised grid with header row" and leaves the heuristics open.
     Assumed: the first record is the header unless every cell of it reads
     as a number, in which case the columns are numbered; the delimiter is
     the one of comma, semicolon, tab and pipe that appears the same number
     of times on each of the first twenty records, the most frequent on the
     first record otherwise, a tab for `.tsv` and a comma for `.csv` when
     none appears; up to 256 MiB of the file is read and indexed (a cut
     file says so), the raw text up to 1 MiB is the fallback and the text
     layer for find. Rendered Markdown and HTML: the Rendered toggle is on
     for every newly opened file (unlike pretty print), images are read
     from the document's own folder or below only (no parent steps, no
     absolute paths, no network), 16 MiB each and 64 MiB in all, and a link
     is kept only with a web, mail or relative address; raw HTML inside
     Markdown is shown as text.

108. (issue 17) The spec says the preview "follows focus: it always shows
    the selection of whichever explorer tab was last focused", and as a
    dock tab the preview sits in a pane of its own that a click would
    focus, which would leave it with no selection to show.
    Assumed: the model keeps the explorer tab the preview follows
    (`Model::preview_follows`): the focused pane's active tab whenever that
    is an explorer, else the one followed so far while it exists. A pane
    whose active tab is the Preview tab never takes the model's focus from
    the dock, so clicking in the preview leaves the keyboard with the
    explorer; the Preview tab is the one tab the Action registry has no
    context for (`App::action_ctx` is `None` there). A Preview tab dragged
    into an explorer pane can still be that pane's active tab, in which
    case the keys about files go to the followed explorer and only the
    tab-strip keys (close, next, previous, move) stay with the pane. The
    tab is one per window, opens in a new pane right of the whole dock with
    the width the 0.1 panel had (the UI works the split fraction out), and
    its kind is part of the session (`TabSession::kind`), so it comes back
    where it was.

109. (issue 17) The spec's viewer "steps to the previous and next file of
    the same folder in the list's current sort and filter", and nothing
    says what the viewer holds once the list it came from has moved on or
    closed.
    Assumed: a viewer tab is a copy of the explorer tab it was opened from
    (`TabKind::Viewer`: location, listing, sort, filter, hidden flag), with
    the file shown as its selection, so it steps through its own rows,
    the app lists and watches it like any tab (a rename or trash of the
    file reaches it, and the explorer, through their watches), and the
    Action registry works on its selection through the same `ActionCtx`.
    The explorer's listing is cloned at open, so the position ("12 of 240")
    is known at once and the relist reconciles behind it. Enter or a
    double-click on another file of the same folder reuses the pane's
    viewer tab for that folder rather than opening a second one; a file of
    another folder opens another viewer tab. When the file shown goes
    (trashed, renamed out of the filter) the viewer shows the file that
    took its place in the order, the last one when it was last. The
    viewer's inline-rename, new-file and new-folder interceptions of the
    list do not apply: those actions use their own prompts there. The
    viewer tab persists in the session like an explorer tab.

110. (issue 17) "Enter … previewable file: viewer; other: system default",
    where the hex fallback claims every file, and the issue's acceptance
    test opens an unknown binary in hex with Enter.
    Assumed: a file is previewable when any registered previewer scores
    it above 0, the hex plugin included, so with it loaded Enter opens every
    file in the viewer; without it (disabled in the Extensions screen, or
    absent) Enter hands a file no previewer claims to the system's default
    application. The decision reads the file's first 4 KiB as an internal
    job (`Command::Open`), like the preview's scoring; the per-type flip the
    spec names is `[viewer] external`, a list of extensions in `rick.toml`
    with a row in the Settings screen, and Mod+Enter (`core.open_default`)
    forces the application whatever the type. When the viewer lands on a
    file nothing claims (stepping through a folder without the hex plugin,
    or a previewer failing to open it) it says "No preview for this file".

111. (issue 17) "The zoom level is remembered per type" does not say for
    how long, and "1:1" has no meaning for paged content, since
    `PreviewDoc` does not say how large a page is in points.
    Assumed: the zoom is remembered per lower-case file extension for the
    session, in the UI, not in `rick.toml`: a setting the spec does not
    name, and a wheel zoom would rewrite the file on every step. For a
    raster Fit never enlarges past 100%, Fit width and the free factors
    may, and 1:1 is one pixel per pixel; for paged content 1:1 shows the
    page as wide as the content, the same as Fit width, until a page size
    reaches the trait (a `PreviewDoc` method, a spec change, when the first
    previewer that knows its page sizes needs it). The free zoom is a
    factor on the fitted size, shown as a percentage of it.

112. (issue 17) The shortcut table binds Left/Right and PageUp/PageDown to
    "Previous / next item or page" in one row, while the Navigation bullet
    gives both pairs to files.
    Assumed: Left and Right always step files; PageUp and PageDown step
    pages while a paged document has a page that way and files otherwise.
    The Viewer table is live only while a viewer tab is the pane's active
    tab, where its rows shadow the list's and the view's of the same keys
    (Mod+F finds, Mod+1 is actual size, Space plays, Home and End are the
    first and last file), the list's own rows (cursor movement, Enter,
    type-to-filter) are not bound at all, and every other row (tabs, panes,
    windows, actions) keeps working; F11 fills the screen with the viewer
    alone, Escape leaves fullscreen and then closes the tab.

113. (issue 17) Find "searches the text layer … with match count, next and
    previous, and highlights in place", and `TextLayer` holds page texts
    without positions.
    Assumed: matches are highlighted in place for text content, which the
    UI lays out itself; for paged content find counts the matches, and
    next and previous go to the match's page, without a box on the page.
    The PDF previewer fills a page's text when the page is rendered, so
    find in a PDF covers the pages seen so far. Matching ignores case where
    lower-casing keeps the byte offsets, which it does for ASCII and most
    scripts, and is exact otherwise.

114. (issue 17) The spec's viewer media transport names "seek bar, volume,
    playback speed, frame step, loop", and `Transport` has play, pause,
    seek, position, duration and playing.
    Assumed: the viewer's transport bar is built against the trait as it
    is (play/pause, a seek bar with elapsed and total time; Space, J, K and
    L), tested with a fake transport; volume, speed, frame step and loop
    wait for the media previewer of milestone 4, which will need them added
    to the trait through a question of its own.

115. (issue 17) The spec's Preview section says "Space on a file or folder
    opens the preview tab if it is closed and shows the entry; Space again
    closes it", while the Selection table binds Space to "Toggle selection
    of the focused entry" (question 21 settled the tables in the second
    one's favour).
    Assumed: Space keeps toggling the selection; the Preview tab opens and
    closes with Mod+Shift+I and the title row's toggle, as in 0.1.

116. (issue 17) The issue asks for "a page strip on the side"; the spec
    says the same without saying what a tile is.
    Assumed: one tile per page with the page's number and a thumbnail the
    pane's slot renders as the tile comes into view, the current page
    marked and kept in view; a click goes to the page. Pages open one at a
    time; a toolbar toggle stacks them in one continuous scroll, where the
    page under the upper third of the view is the current one. Both are
    kept per tab for the session.

117. (issue 35) SPEC.md asks for the performance targets "met on the
    200k-entry fixture, on all three platforms" and for the 0.2 milestones
    to "merge to `main` behind CI on all three OSes as in 0.1"; until now
    every push ran the acceptance benchmarks once per OS and asserted the
    targets. Issue 35 moves the benchmarks to tier 3, which measures
    (`RICK_BENCH_RECORD`) and never blocks a pull request.
    Assumed: the targets are no longer asserted automatically. The nightly
    run on Linux x64 and the runs on release tags record the medians next
    to the targets of `bench.rs`, and a release is checked by reading the
    tag's bench summary; the three-OS gate before `main` is kept by tier 2,
    which runs the tests, the bundle and the plugin loading on the full
    matrix, without the timings.

118. (issue 22) The spec's Indexed search says a search box "appears on the
    toolbar" when the platform has an index and is "absent, not greyed
    out" without one, while the Window chrome puts one search box in the
    title row that is also the command palette and, in 0.1, searches the
    current location by name. It also says index results show "in the
    list with their paths", while the issue has the title-row box "show
    results with their paths" and the acceptance has the box list the
    hits. And `SearchIndex::scope` is `Everywhere | Under(Location)` with
    no argument, while the issue asks the walk plugin to have "scope =
    Under(location)".
    Assumed: the title-row box stays, whatever the platform has; what an
    index adds is a second group of rows in its dropdown, after the
    current location's matches: every available index is queried with the
    text (root `None`, names only) once the text has stood for 150 ms, and
    each hit lists once with its folder as the hint, errors and a
    "searching" row below. Picking a hit does what Open containing folder
    does: the pane goes to the hit's folder and selects it there; it does
    not open the file. Two kinds of index share the trait: a platform
    index has scope `Everywhere` and feeds the box; the walk has scope
    `Under` the local provider's root (a `Location` with an empty path,
    which `SearchScope::covers` reads as the provider's whole tree) and
    answers only a query with a root, which the filter bar's *Search in
    subdirectories* sends with the pane's location. The filter bar shows
    its toggle only while an `Under` index is registered, and a subtree
    search without one ends with a notice. The pane's listing is not
    touched by the box.

119. (issue 22) The walk of 0.1 took the pane's hidden toggle and the
    `[search] ignore` list of `rick.toml`, and `Query` carries neither.
    Assumed: `Filter` gained `NoHidden` (leave hidden entries out and
    never descend into a hidden directory) and `SkipNames` (never descend
    into or report these names), both narrowing a query as the other
    filters do, and `SearchScope` gained `covers`; `API_VERSION` is 5. The
    ignore list stays in `rick.toml` and reaches the walk through the
    query, since no plugin reads its own settings file yet. A platform
    connector applies both, and the size, extension and date filters, to
    the hits its service returns, below the query's root when it has one.

120. (issue 22) The issue names the services' interfaces (Windows Search
    "via ISearchQueryHelper/OLE DB", Tracker "SPARQL over D-Bus", Baloo
    "baloosearch or D-Bus") and asks for a fake service beside the real
    one; a plugin may not use `unsafe`, which the COM interfaces need, and
    the kernel has no D-Bus library.
    Assumed: every connector drives a command line through one `Service`
    trait (`output` for the probe, `stream` for a query's lines), the
    real one a child process read on the blocking pool and killed on
    cancel, the tests' one canned output. Windows Search runs its SQL
    through a PowerShell script over an ADODB connection to the
    `Search.CollatorDSO` OLE DB provider; Tracker sends its SPARQL to the
    miner's D-Bus endpoint through `tracker3 sparql --dbus-service`;
    Spotlight runs `mdfind`; Baloo runs `baloosearch` (the `6`-suffixed
    tools first). "Detected at start and on change" is a probe at
    construction and every ten seconds after (`sc.exe query WSearch`,
    `mdutil -s /`, the bus name's owner through `gdbus` or `busctl`,
    `balooctl status`); an index whose service is installed but not
    running is not available, so Tracker is never started by a query.
    Every hit is looked up on disk for its entry and id, hits that are
    gone are dropped, and a query stops at 500 hits. The real-service
    tests run where the runner has the service: Windows Search on
    Windows (the setup action starts the service), Spotlight on macOS,
    Tracker and Baloo on Linux inside a private session bus with the
    indexers over a fixture directory (`.github/scripts/with-indexers.sh`,
    both installed by apt in under half a minute); elsewhere they print a
    note and pass, and `RICK_REQUIRE_<SERVICE>` makes them fail instead.

121. (issue 22) The bundler put every `ext-*` library into `plugins/`,
    and the host refused one whose manifest left the platform out; the
    first platform-specific plugins would have shipped three refused rows
    on every platform.
    Assumed: `rick-plugin-manifest` exits with code 3 for a plugin whose
    manifest leaves this platform out and `cargo xtask bundle` leaves it
    out of the bundle, saying so; the release archives carry only the
    connector of their platform, and `.github/scripts/check-plugins.sh`
    checks the per-platform set.

122. (issue 22) The spec's `ext-svn` row names "Update, Commit, Revert,
    Log actions" and the issue adds Add; the "revision in the info panel"
    does not say which revision a mixed-revision working copy shows.
    Assumed: the group is Update, Commit, Revert, Log and Add, as the
    issue says. The panel shows the working copy root's revision as `svn
    info` reports it, with the relative URL, the repository root and the
    last commit the root knows of; a commit made in the working copy
    shows after an update, as in Subversion itself. Commit adds the
    selected files that are not versioned yet before committing the
    selection, Revert takes `svn revert -R` on the selection after a
    confirmation and leaves unversioned files alone, Update and Log act
    on the selection when there is one and the whole working copy
    otherwise, and the badges use Subversion's own status letters
    (`C` conflict, `!` missing, `?` unversioned). The tests generate a
    repository with `svnadmin`; where the command line is missing they
    skip with a note unless `RICK_REQUIRE_SVN` is set, which the setup
    action does after installing Subversion.

123. (issue 23) The action plugins need dialogues richer than `UiHost`'s
    `prompt` and `confirm`: a format and a name for Compress to…, a
    pattern with a live preview for Bulk rename, an application list for
    Open with…, a pattern kind and mode for Select by pattern.
    Assumed: `UiHost` gains `form(Form) -> Option<Values>`: a titled list
    of fields (text, checkbox, choice, folder with a Browse… button) and an
    optional preview closure the UI calls with the values as they are
    typed, each row a from → to pair with an optional problem; a problem
    keeps the form from being confirmed. The egui host answers it the way
    it answers `prompt`: the first run records the form, the shell shows
    it, and the action runs again with the values. The extension API is
    version 6 for this and the changes of 124 to 128.

124. (issue 23) An action that opens a console or a comparison, or that
    selects entries, has no outcome to say so: `ActionOutcome` had `Done`,
    `Started` and `Undoable`, and the UI never touches the model.
    Assumed: `ActionOutcome::View(View)` hands the application data the
    UI draws — `View::Compare(Arc<Compare>)`, filled by the action's job,
    and `View::Console(Arc<dyn Console>)`, live — and the application opens
    a tab of the new kinds `TabKind::Compare` and `TabKind::Console` next to
    the pane's active one and passes the view on with `Event::ViewOpened`;
    `ActionOutcome::Select(Selector)` hands it a matcher it applies to the
    pane's rows, replacing or adding to the selection. A console or compare
    tab lists nothing, offers no action context (like the Preview tab) and
    is left out of the session, since a shell or a comparison cannot come
    back; closing the tab drops the view, which ends the shell.

125. (issue 23) Compare "the folders of the two panes": the action's
    context carries one pane's location and selection.
    Assumed: `ActionCtx` gains `other: Option<Location>`, the location of
    the first other pane of the window whose active tab is an explorer, in
    dock order. Compare uses it with nothing selected; with two files or
    two folders selected it compares those and ignores it.

126. (issue 23) An extraction "with the conflict dialogue": the `Conflicts`
    queue lived in `ops`, which a plugin cannot depend on.
    Assumed: the queue moves to `runtime` and every `JobCtx` carries its
    runner's queue (`JobCtx::conflicts`), so a plugin's job asks the same
    dialogue as a copy; `ops` keeps the landing policy and re-exports the
    queue, and the application takes it from its `JobRunner`. The compress
    plugin's extraction asks for a file over an existing entry and merges a
    folder over a folder without asking, as the copy rules say; a job
    whose dialogue was never answered ends when it is cancelled.

127. (issue 23) The issue says Compress writes "through ext-vfs-archive's
    writers", while Module rule 2 says extensions depend on `ext` and
    `types` only.
    Assumed: `ext-action-compress` links `ext-vfs-archive` as a library,
    the one exception, so the zip, tar and 7z code exists once; the
    archive crate gains `create` (a new archive from files on disk) and
    `extract` (a whole archive in one pass, asking where each entry goes)
    for it, and `deny.toml` allows the edge. The archive plugin's library
    is still loaded by the host on its own; the compress plugin carries its
    own copy of that code, which holds no process-wide state.

128. (issue 23) "Create link / Paste as link (symlink; junction and .lnk
    on Windows with a choice)": `LinkKind` had `Symbolic` and `Hard`, and
    nothing told an action which kinds a provider offers.
    Assumed: `LinkKind` gains `Junction` and `Shortcut`, `Capabilities`
    gains `JUNCTION` and `SHORTCUT` (and the spec's `NATIVE_TRANSFER`,
    which was missing), the local provider offers them where the platform
    has them (Windows, through the `junction` crate and the shell link
    object), and Paste as link and Create link… ask which kind when more
    than one is offered. Create link… asks for the folder in a form; Paste
    as link links to the clipboard's items, whatever the clipboard's mode,
    in the current folder, under a numbered name when the name is taken; a
    shortcut is named `name.lnk`. Both are undoable: undo removes the links.

129. (issue 23) Open with… "the platform's application list for the type
    plus any executable, remember-for-type".
    Assumed: `platform::OpenWith` gains `applications` (the desktop files
    the MIME databases register for the type on Linux, with `xdg-open` as
    "System default" where nothing is registered; Launch Services' list on
    macOS; the shell's recommended handlers for the extension on Windows,
    File Explorer for a folder) and `open_with` (a listed application, or
    a program the user typed, run with the path as its argument). The
    choice is remembered per lower-case extension (`folder`, `none`) in
    `core.toml` under `[open_with]` and listed first the next time; Enter
    keeps opening with the viewer or the system default, as the spec's
    navigation row says.

130. (issue 23) "The system terminal when the setting says so": the spec
    has `Registry::settings` render "in the Settings screen, stored in the
    extension's TOML", which nothing did yet, and names the `Terminal`
    platform trait without a shape.
    Assumed: the Settings window renders every declared schema under the
    extension's name and the application keeps the values in
    `<config dir>/<extension id>.toml` (`Command::ExtSetting`); an
    extension reads its own file. The console plugin declares `console`
    with the options `embedded` and `system`. `platform::Terminal` is
    `open_terminal(dir, command: Option<&[String]>)`: `$TERMINAL` or the
    first of x-terminal-emulator, gnome-terminal, konsole, xfce4-terminal,
    kitty, alacritty, foot, wezterm and xterm on Linux, Terminal.app on
    macOS (`osascript` for a command), Windows Terminal or `cmd` in a new
    window on Windows. The spec said `ext-ssh` introduces the trait; it
    arrives here and `ext-ssh` reuses it.

131. (issue 23) The embedded console: "a dock tab through portable-pty
    with a small VT renderer" leaves the emulator's extent open.
    Assumed: the user's shell (`$SHELL`, `ComSpec`) with `TERM` set to
    `xterm-256color`; a screen of cells handling the VT100 to xterm
    sequences a shell, an editor and a pager use (cursor movement, erasing,
    insert and delete, a scroll region, the alternate screen, tab stops,
    SGR with the 16 named colours, the 256-colour cube and true colour, the
    cursor and application cursor keys modes, the title, the cursor
    position and device attributes answers), a scrollback of 10 000 lines
    shown with the wheel and Shift+PageUp/PageDown, and every character one
    cell wide (double-width characters misalign until a width table is
    added). While the console has the keyboard every key goes to the
    shell, Tab, Escape and the arrows included, so the shell's shortcuts
    rest until the mouse moves on; Ctrl+V pastes (egui turns it into a
    paste) and Ctrl+C is the interrupt. The pane's title is the folder, or
    what the shell sets. The reader runs on the runtime's blocking pool,
    the one place a plugin may block.

132. (issue 23) Bulk rename's placeholders and order are not defined, nor
    what "Rename with several selected" does.
    Assumed: `{name}` is the name without its extension, `{ext}` the
    extension with its dot (empty when there is none, a folder has none),
    `{n}` the counter from the form's start padded to the width of the last
    number (`{n:4}` for a fixed width), `{date}` the modification date as
    `YYYY-MM-DD` in local time; find and replace runs on the original name
    first (a regular expression with `$1` groups when the box is ticked),
    the pattern fills from the result, the case change applies to the
    whole result. Clashes — two entries on one name, a name another entry
    of the folder has, an empty name, a separator — are marked in the
    preview and refuse the rename. The renames are one undoable job
    through temporary names (so swaps and case-only renames work on
    case-insensitive file systems), on local folders only. The shell runs
    the bulk rename for Rename (F2) with several entries selected when the
    plugin is loaded; the core action keeps asking for one entry otherwise.
    The spec's operations list gives Select by pattern Ctrl+Shift+A, its
    table Mod+Shift+S (Mod+Shift+A inverts); the table wins, as for 21.

133. (issue 23) Compare's limits.
    Assumed: local files and folders only; two files are compared as text
    when both look like text and are at most 64 MiB, byte for byte
    otherwise (reported as identical or different); folders are compared
    one level deep by name, size and modification time (two times within a
    second are the same), a subfolder present on both sides counting as
    the same; the copy buttons of the folder diff run the kernel's copy
    (`Command::Receive`), so the conflict dialogue and the operations area
    apply. The comparison is a job on the blocking pool that fills the tab;
    the tab's title is `left ↔ right`.

134. (issue 23) The package layout lists "open in system file manager"
    under `ext-action-terminal`, the issue says it "stays".
    Assumed: it stays `core.reveal` in the built-in core actions, so it
    works without the plugin; the terminal plugin holds Open console only.

135. (issue 21) SPEC.md's milestone 5 and package layout replaced
    `ext-vfs-sftp` with `ext-ssh` (`specs/ext-ssh.md`, shell commands over
    `exec` channels, no SFTP); the issue, written against db17a20, asks for
    `ext-vfs-sftp` through russh and russh-sftp, and so does the task.
    Assumed: `ext-vfs-sftp` as the issue says, on the pieces the ssh spec
    introduces for both (`platform::Secrets`, `Capabilities::REMOTE`, the
    Network section's one row per connection), so `ext-ssh` lands on the
    same foundations when its turn comes. Its connection dialogue asks for
    host, port, user, key file, key passphrase, password and the remote
    folder the connection opens at (the login folder when empty). Host keys
    follow `~/.ssh/known_hosts` with OpenSSH semantics through russh: an
    unknown host is learnt and recorded with a log line, a changed key is
    refused with its fingerprint; the ssh spec's two dialogues, agent
    authentication, jump hosts and `~/.ssh/config` are `ext-ssh`'s. The
    listing streams packet by packet through a raw SFTP session of its own
    beside the one the files use; `copy` within a host streams through the
    machine (the protocol has no copy) and `transfer` answers `Unsupported`
    for a copy, so the kernel's path takes it; a move is a remote rename.

136. (issue 21) The issue wants connections in `rick.toml` with secrets in
    the keychain and connect, disconnect and remove on the row's menu, and
    a plugin depends on `ext` and `types` only (Module rules 2): it can
    neither write `rick.toml` nor draw a dialogue, and the `Provider`
    contract has no connections.
    Assumed: `ext` gains the connection contract (API version 7): a
    `Connector` a network provider registers beside its provider
    (`Registry::connector`, part of the provider point) with a
    `ConnectionSchema` (the dialogue's fields: text, integer, bool, path,
    secret), `update` (the connections as the application keeps them, at
    start and after every change), `connect`, `disconnect` and
    `is_connected`. The application owns the list: the non-secret fields
    are `[[connections]]` tables of `rick.toml` with `provider`, `name`
    and the fields inline, a `Secret` field goes to the keychain under
    `<provider>/<connection>/<field>` as a job (a keychain may ask to be
    unlocked) before the table is written, and the connector is handed the
    list; `Remove` deletes the table and the keys. The side panel builds
    the *Add connection…* dialogue from the schema (the name, then the
    fields, secrets masked), offered in the section header's context menu
    per plugin and by the *Add…* row of an empty section; a connection
    row's menu offers *Connect* or *Disconnect* and *Remove connection*.
    The Network section lists what the network providers list at their
    roots, as Cloud does (87); `vfs-rick` keeps the id lists of 87 for
    which section a provider belongs to.

137. (issue 21) `transfer()` joins the `Provider` trait as the spec writes
    it, and the trait's other implementers (local, archive, trash, the
    tests' providers) have no native transfer.
    Assumed: a default body answering `Unsupported`, so every provider
    keeps compiling; `ops` lands each pair through the conflict policy as
    before and hands the provider, in one call per target folder, the
    items whose source and target resolve to the same provider instance
    with `NATIVE_TRANSFER` at both ends, land under their own name and
    overwrite nothing; a provider that answers `Unsupported` (SFTP for a
    copy) gets the kernel's path for those items, any other error fails
    the job. Overwrites and renamed landings keep the kernel's path. The
    undo of a move goes back the same way.

138. (issue 21) Folders in an object store, and the spec's "no rename".
    Assumed: a folder is a zero-byte object whose key ends in `/` (the
    consoles' convention), listed through the service's delimiter as a
    common prefix and never shown as a file; a folder's `stat` is a
    one-key listing under its prefix; deleting a folder deletes every
    object under it, deleting a bucket or container empties it first; the
    bucket level offers `WRITE` (a new folder there is a new bucket) and
    nothing moves into it natively. `RENAME` is not set, so F2 hides, and
    a move within the account is the service's copy per object then a
    delete, across buckets or containers of the account too; across two
    connections the kernel streams. S3's `CopyObject` stops at 5 GB per
    object, the service's limit; a larger object needs `UploadPartCopy`,
    left for when someone has one. Uploads stream in 8 MiB parts, blocks
    or chunks through a pipe to an upload task; dropping the writer before
    `shutdown` aborts the S3 multipart upload, leaves Azure's blocks
    uncommitted and abandons the GCS session, so a cancelled copy leaves
    no object. The GCS connection's secret is the service account's key
    file, its JSON or its path, exchanged for a bearer token through a
    JWT signed with ring; no key means unauthenticated requests, which an
    emulator takes. Downloads are streamed `GET`s; readers do not seek.

139. (issue 21) The kernel's tokio runtime was built with its time driver
    only; russh and reqwest need the I/O driver.
    Assumed: `runtime` enables it (`enable_io`), the one runtime of the
    process driving the network providers' sockets too (81).

140. (issue 21) "Remote-latency behaviour (no eager stat, no thumbnails by
    default)" for the network providers.
    Assumed: `Capabilities::REMOTE` as the ssh spec names it; the kernel
    answers `Thumbnail` with nothing for an entry of a provider with it,
    and the connection rows of the side panel are synthesised without
    connecting (a connection's `stat` and the root listing never touch
    the network), so a panel full of connections opens nothing until one
    is clicked or *Connect* is picked. The preview size caps of the ssh
    spec are not built here.

141. (issue 21) The acceptance names containers for Linux CI and "where
    Docker is available" for the other runners.
    Assumed: tier 1 and the Linux x64 job of tier 2 start MinIO (Bitnami's
    frozen `bitnamilegacy/minio:2025.7.23-debian-12-r5`, since MinIO's
    community edition is source only and Docker Hub's `minio/minio` is
    gone), Azurite, fake-gcs-server and atmoz/sftp with `docker run`
    (`.github/scripts/containers.sh`), name them to the tests through
    `RICK_S3_ENDPOINT` and friends and set `RICK_REQUIRE_CONTAINERS`, so a
    round trip that would skip for a missing variable fails there; the
    plugins' round trips live in their crates' unit tests, the kernel
    pipeline (a 100 MB upload with progress, cancelled half-way leaving no
    object) in `tests`; macOS, Windows and Linux arm64 skip them with a
    note. The sshd container's user is chrooted, so the connection's
    folder is its writable `upload`. The in-account byte counter runs on
    every OS through the in-memory provider's native mode, and the
    plugins' own round trips assert the moves land without the machine
    reading the objects.

142. (issue 19) The spec's 3D previewer is orbited by drag, zoomed by the
    wheel and reset by a double-click, and the issue asks for them to
    arrive through `set_control` so that the viewer and the preview pane
    need no 3D code; `PreviewControl` (toggle, button) and `ControlValue`
    (bool, pressed) cannot carry a pointer gesture.
    Assumed: API version 5 adds `PreviewControl::Pointer { id, gesture,
    label }` with `Gesture::{Drag, Wheel, DoubleClick}` and
    `ControlValue::Delta { x, y }` in points. A pane forwards a gesture
    the document declares to `set_control` instead of its own pan, scroll
    or 1:1 (the modifier wheel stays the viewer's zoom), draws nothing in
    the toolbar for it, and keeps the frame rendered before a control
    change on screen until the next render lands, so an orbit never drops
    to a spinner; before, every control change cleared the frames.

143. (issue 19) "Models beyond a configurable triangle budget" (the issue:
    "setting, default 2 M"): how a plugin's setting reaches it.
    Assumed: the plugin declares the schema (`triangle_budget`, an integer
    from 1 000 to 20 000 000) and, as milestone 6's extension settings have
    it (the Settings screen writes `<id>.toml`, an extension reads its own
    file), reads `preview-3d.toml` in the config directory through the
    platform the registry lends it, at every open, so a change takes effect
    for the next file; a value outside the bounds is clamped as the
    application clamps every integer setting. A budget above what the
    device's largest buffer holds is capped there.

144. (issue 19) The status bar shows "bounding box in millimetres", while
    STL and OBJ carry no unit, and the spec says nothing about which way
    is up.
    Assumed: STL and OBJ coordinates are millimetres, the convention of
    slicers; a 3MF's declared unit (micron, millimeter, centimeter, inch,
    foot, meter) converts and is shown as declared in File info; Z is up
    for all three formats and the ground grid lies under the model's
    lowest point.

145. (issue 19) "decimate for the preview and say so" names no method.
    Assumed: vertex clustering on a uniform grid over the bounding box,
    the finest grid resolution (bisection, up to 4096 cells a side) whose
    result fits the budget, every corner snapped to its cell's centre and
    collapsed triangles dropped; one pass per step and no allocation. A
    3 000 000-triangle binary STL opened, parsed and decimated in 2.3 s in
    release on the development machine, most of it the parse. The
    triangle count shown stays the file's; the status bar adds "decimated
    to N triangles for the preview" and File info the budget.

146. (issue 44) The issue puts the install locations at
    `~/.local/share/rick/apps/<id>/<version>/` and `/opt/rick/apps/…` and
    asks for "the spec's exact paths and rules"; specs/ext-installer.md
    puts them at `~/.local/opt/<app-id>/<version>/` and
    `/opt/<app-id>/<version>/`, with `~/.local/bin`, the applications and
    hicolor directories beside them.
    Assumed: the spec's paths.

147. (issue 44) The issue says an update "keeps previous versions until the
    user removes them"; the spec keeps exactly one previous version, not
    configurable (its settled questions).
    Assumed: one previous version is kept, anything older is deleted when
    an update lands, and Revert flips to the one kept.

148. (issue 44) The issue says "uninstall is undoable while the previous
    version's folder still exists"; the spec's uninstall deletes the app
    folder with every version folder and the record, and nothing outside
    the record's list, so nothing of the app remains to undo from.
    Assumed: Uninstall asks first, listing exactly what goes and that the
    app's settings stay, and is not on the undo stack; Revert in the view
    is the way back while the app is installed.

149. (issue 44) "Elevation for system-wide installs goes through
    `platform::Elevate` from #24": the repository has no pull request 24
    and `platform` had no such trait; the spec makes the privileged helper
    its own milestone and shows the Scope choice as unavailable until it
    exists.
    Assumed: `platform::Elevate` is added now as the hook the issue names,
    one method that runs a program as the administrator and waits for it,
    through `pkexec` and the polkit prompt on Linux, `Unsupported` on
    macOS and Windows, recorded by the Fake; nothing calls it yet. The
    form offers "System-wide (unavailable: the privileged helper is not
    part of this version)", and choosing it is a problem row; the helper,
    its plan file and the elevated path are the next milestone.

150. (issue 44) The spec opens the Installed Apps view "from the command
    palette and the app menu", the issue puts `rick://apps` "in the Places
    list", and no kernel path let an action take a pane to a location.
    Assumed: `ActionOutcome::Navigate(Location)` joins the extension API
    (API version 9, together with `Elevate`): the application dispatches
    it as a `Navigate` command, and *Installed apps* in the Installer group
    of the palette and the context menu returns it, as does *Open install
    folder*. The Places list is the user's own file, seeded from the
    kernel's defaults, which cannot name a plugin; a `rick:apps` place can
    be added in places.toml, and a side-panel entry for views waits for a
    kernel hook. The burger menu lists no extension groups today.

151. (issue 44) "Dropping an archive onto the Installed Apps view opens the
    same dialog", while the spec says the plugin needs only two generic
    hooks, an action opening a dialogue and a job with progress; a drop
    onto a virtual location is a no-op in the UI, and a provider has no
    way to open a dialogue.
    Assumed: the drop target is left out; the dialog opens from the
    context menu and the palette on the archive; `rick install <archive>`
    is 0.3's command line.

152. (issue 44) Off Linux the issue wants the plugin to "report
    'unsupported on this platform' for its actions" and its actions
    "hidden".
    Assumed: both. The actions' availability is Hidden, a run by id
    answers `Unsupported("unsupported on this platform")`, and the view's
    listing answers the same error.

153. (issue 44) The issue cites the spec file's "Acceptance section"; the
    spec has the "Test fixtures" table instead.
    Assumed: the table's ten fixtures plus the issue's own list are the
    acceptance tests: the crate's tests cover every fixture row and the
    round trip, `crates/tests/tests/installer.rs` the round trip through
    the application with the filesystem diff and the plugin loading
    through the host.

154. (issue 44) The UI host answers one dialogue per action run and runs
    the action again once the user confirmed; detection must have happened
    before the form is built.
    Assumed: detection runs on the app thread when the action runs (a
    compressed tar's index means one pass over the stream) and is cached
    per archive by size and modification time, so the re-run with the
    answer is instant; collisions and a downgrade are problem rows of the
    one form, cleared by the `Overwrite` and `Allow downgrade` checkboxes,
    instead of a second question; *Update from archive…* asks for the
    archive's path with `prompt` and remembers it until the form is
    answered, so its re-run goes straight to the form.

155. (issue 44) An update leaves the desktop entry, icon and symlinks
    alone "unless the main executable's relative path changed, in which
    case the symlinks are updated".
    Assumed: the desktop entry's `Exec` is rewritten in that case too: it
    names the main executable through `current`, and a stale path is the
    vanishing-launcher problem the spec avoids.

156. (issue 44) The view's row shows "icon, name, version…", and the icon
    field of the dialog has "a file picker"; a Form has text, check,
    choice and folder fields, and a row's glyph is the kernel's.
    Assumed: the row shows the folder glyph, the version, kind, scope,
    state and commands in columns and the record's facts in the info
    panel; the form's icon is a choice of the archive's icon files plus a
    text field for a path of one's own, the file picker being a later
    sibling of the folder field.

157. (issue 44) Archive safety says hostile entries are "rejected" and
    links and special entries "skipped", and the hostile fixture expects
    "entries rejected or skipped" within one install.
    Assumed: an entry whose path is absolute, carries `..` or a NUL, a
    duplicate name, an outside-pointing link and a special entry are each
    left out and listed in the install summary, and the install goes on
    with the rest; only a source tree, a wrong architecture, a checksum
    mismatch and a missing main executable refuse the whole install.

158. (issue 44) The spec has the plugin declare "in its manifest that it
    writes outside the browsed location (`~/.local/opt`, `~/.local/share`,
    `~/.local/bin`) and may invoke `pkexec`; a good first real test of the
    declarative permission model"; the extension manifest has no such
    field, and the permission model is among the spec's open questions.
    Assumed: the manifest declares nothing of the kind yet; the crate's
    README and docs name the paths it writes, and the field comes with
    the extension security model.

159. (issue 34) The host-key dialogues, the passphrase and the password
    prompt are asked from inside the plugin while a listing or a copy runs,
    but the API's prompts (`UiHost::confirm`, `prompt`, `form`) are
    synchronous calls meant for an action on the UI thread; nothing lets a
    provider's asynchronous operation ask the user and wait for the answer.
    Assumed: a `Prompts` queue in `runtime` beside `Conflicts`, lent to
    extensions through `Registry::prompts`: an extension asks with
    `Prompts::ask`, `choose` or `secret` and awaits; the kernel publishes
    the question as `Event::Question`, the UI shows one modal dialogue at a
    time, the oldest first, and answers through `Command::AnswerQuestion`,
    a dismissed dialogue answering *Cancelled*. The extension API is at
    version 10 for this and the three points below.

160. (issue 34) *Open console here* attaches "an externally supplied PTY
    stream" to the console pane that `ext-action-terminal` owns, but a
    plugin cannot depend on another plugin, and the terminal emulator was
    that plugin's private module.
    Assumed: the emulator (`Screen` and the `vte` parser) moved into the
    kernel as `ext::terminal`, where both plugins use it; the SSH console
    is an `ActionOutcome::View(View::Console)` over a PTY `shell` channel
    of the pooled connection, so no second login happens, and
    `ext-action-terminal` is unchanged in behaviour.

161. (issue 34) "Previewers declare whether they need the whole file or
    can work on a prefix", the prefix served by the provider and the cap
    "applied by the preview frame when the provider is REMOTE", with "a
    temporary file that is removed when the tab closes".
    Assumed: `Previewer::reads()` answers `Reads::Whole` (the default) or
    `Reads::Prefix` (text, code, CSV and hex; Markdown has no previewer of
    its own); for a `REMOTE` provider the kernel hands a prefix previewer a
    provider whose reads stop at 256 KiB (`app::REMOTE_PREFIX_CHUNK`),
    shows *Load more* under the preview with the bytes served, and each
    *Load more* adds another chunk; a whole-file previewer's file above
    the cap shows the badges and *Preview anyway* instead. The cap is the
    provider's `remote_preview_cap(loc)` (the host's
    `preview_size_cap_mib`, then the `ssh.toml` setting), the kernel's
    4 MiB (`app::REMOTE_PREVIEW_CAP`) when the provider names none.
    `Provider::open_read_at(loc, offset)` reads from an offset (`tail -c
    +N` here; the default reads and discards). Nothing is spooled to a
    temporary file by the kernel: the previewers read through the
    provider, and the ones that need a path (Office, media) keep their
    own temporary copies as they do for archives.

162. (issue 34) The host editor offers "jump host (optional, picked from
    the host list)" and Rick's hosts "are created and edited from the
    Network section", but `Connector::schema` returned a borrowed, static
    schema of text fields and the side panel had *Add connection…* only.
    Assumed: `Connector::schema(&self) -> ConnectionSchema` is owned and
    computed per call, so the jump field lists the hosts known now;
    `ConnectionFieldKind::Choice { options, default }` draws a combo box;
    `Command::EditConnection` and the row item *Edit…* open the same
    dialogue filled in, for Rick's own connections only (a host of
    `~/.ssh/config` is read-only in Rick, so its row has neither *Edit…*
    nor *Remove connection*). The other provider plugins return a clone
    of their schema.

163. (issue 34) The configuration section keeps Rick's hosts as
    `[[ssh.hosts]]` and the three settings under `[ssh]` in `rick.toml`,
    while issue 21 made the connections of every network provider
    `[[connections]]` tables of `rick.toml` and issue 23 put an
    extension's settings in its own TOML file.
    Assumed: Rick's hosts are `[[connections]]` tables with
    `provider = "ssh"`, the alias as `name` and the spec's fields (`host`,
    `port`, `user`, `default_dir`, `identity_file`, `jump`,
    `remember_password`, `preview_size_cap_mib`), so the host editor is
    the connection dialogue every network provider has, with *Edit…*; the
    three settings (`read_openssh_config`, `idle_timeout_minutes`,
    `preview_size_cap_mib`) live in `ssh.toml` and appear in Settings like
    every extension's. The keys are the spec's; the tables moved.

164. (issue 34) What `ssh:/<alias>` lists and how a remote path is kept in
    a location.
    Assumed: `ssh:/<alias>` is the host's default directory, the remote
    home when the host names none, resolved once per connection with
    `pwd`; every location below carries the remote path as absolute bytes,
    so a folder outside the default directory is reached by typing its
    path and names that are no UTF-8 survive. The rows of the provider's
    root are synthesised from the configuration without connecting.

165. (issue 34) "Secrets live only in the OS keychain", without the keys.
    Assumed: `ssh/<alias>/password` and `ssh/<alias>/passphrase` through
    `ext::secret_key`, the scheme of the other network providers; the
    editor's *Remember password in the OS keychain* stores the password
    typed there, the passphrase dialogue's *Remember in the keychain* box
    the passphrase, and a secret typed when nothing is remembered serves
    the one connection; *Remove connection* removes both keys.

166. (issue 34) "disk usage: `df -P` for the status bar", but the status
    bar shows no disk usage for any provider yet.
    Assumed: `SshProvider::disk_usage` runs `df -P` and parses it, and no
    kernel slot asks for it until the status bar grows one for every
    provider.

167. (issue 34) "Connection errors surface in the tab as a banner with
    Retry, never as a modal dialogue": a listing that failed was a toast.
    Assumed: the tab gained `failure`, set by the kernel for any provider's
    listing error and cleared by the next listing that works, drawn as a
    banner over the empty list with *Retry* (`Command::Refresh`); the toast
    is gone for listing errors. A failed connection is a listing error.

168. (issue 34) The `busybox` profile lists with "`ls -lan` where
    `find -printf` is missing", but `ls -lan` has no stable layout (the
    date changes with the file's age) and splits a name on its spaces and
    newlines, which the acceptance requires to survive.
    Assumed: `busybox` and `bsd` list through a shell loop over the
    directory's names (`for f in * .[!.]* ..?*`) that runs `stat -c` or
    `stat -f` on each and prints NUL-separated records: slower than one
    `find`, byte-exact. `uname`, `stat`, `find -printf`, `head -c`,
    `tail -c` and `busybox` itself are probed once per host.

169. (issue 34) The acceptance names Docker containers for Linux CI and
    the macOS runner's own sshd for `bsd`; the sandbox the plugin was built
    in has neither Docker nor an sshd.
    Assumed: an SSH server in the test process (`ext_ssh::fake_sshd`, the
    server side of `russh`, behind the `fake-sshd` feature; a deterministic
    host key, password, public-key and keyboard-interactive login,
    `MaxSessions`, `direct-tcpip` for a bastion) runs the plugin's own
    tests and the acceptance tests in `tests` on every OS; tier 1 and the
    Linux x64 job of tier 2 also build the `gnu` (Debian) and `busybox`
    (Alpine) sshd images and the bastion network from
    `.github/scripts/containers.sh` and name them through
    `RICK_SSH_GNU_PORT` and friends, with `RICK_REQUIRE_CONTAINERS` making
    a skip a failure. The macOS runner's sshd is not enabled (Remote Login
    is an interactive setting), so the `bsd` profile is proven by the
    command tests with BSD `stat -f` output.

170. (issue 24) The spec's *Inspect* opens "the Windows Properties dialogue,
    the macOS Get Info window, and on Linux Rick's own properties
    dialogue", while the issue has Properties edit ownership, mode bits and
    ACLs, an NTFS ACL entry added and removed on Windows among them, which
    the system dialogues cannot be driven to.
    Assumed: Properties is Rick's own dialogue on every platform, the
    general rows above the Permissions section, and on Windows and macOS a
    *System properties…* button opens the platform's own dialogue as
    before. The dialogue follows the entry: a change sends the new state
    back while it is open.

171. (issue 24) "ACLs where the file system has them (POSIX ACLs, NFSv4
    ACLs, NTFS ACLs on Windows with a trustee list)"; macOS has ACLs of its
    own the issue does not name.
    Assumed: one `Acl` type with a flavour per file system, read and
    written by `platform::Permissions`: POSIX ACLs through the
    `system.posix_acl_access` and `system.posix_acl_default` attributes
    (the minimal list derived from the mode when none is stored, and
    removed again when the list is reduced to its three base entries),
    NFSv4 ACLs through `system.nfs4_acl` (shown and reduced in the
    dialogue, new entries written with the server's tools since the `who`
    forms and masks are the server's), macOS ACLs through `acl_to_text`
    and `acl_from_text` (allow or deny, user or group, the permission
    words `chmod +a` takes), NTFS discretionary ACLs through the security
    API (allow or deny per trustee, the standard rights as checkboxes, the
    raw mask shown, inherited entries greyed out and left to the parent,
    inheritance blockable). No ACL library is linked.

172. (issue 24) "Changes that need elevation ask through a new
    `platform::Elevate` trait (polkit on Linux, Authorization Services on
    macOS, UAC via a helper on Windows) and run only the privileged step
    elevated"; the installer (issue 44) had added `Elevate` for Linux.
    Assumed: the privileged step is `rick-perms`, a second binary beside
    `rick` that reads a request file (the entries and their changes),
    records what each had and applies them with the same platform code,
    and prints the reply; a job applies every entry unprivileged first and
    hands the ones refused to the helper in one call, so the user sees one
    prompt per job. `Elevate::run_elevated` reaches `pkexec` on Linux,
    `osascript … with administrator privileges` on macOS (Authorization
    Services' own prompt) and `ShellExecuteEx` with `runas` on Windows,
    where an elevated process shares no pipes, so the arguments get
    `--output <file>` and the file comes back as the output; a program run
    elevated honours that flag. `RICK_PERMS_HELPER` names the helper for
    packagers who keep it elsewhere; a test binary finds it one directory
    up from `deps/`.

173. (issue 24) "Apply recursively with separate choices for files and
    folders".
    Assumed: the folder's dialogue has *Apply to enclosed items* with
    *Files* and *Folders* boxes: which kinds the one change applies to
    inside, the target folder itself always included; folders are walked
    without following links, and every folder is changed after what is
    inside it, the target last, so a mode without the execute bit still
    reaches the contents (undo restores in the reverse order for the same
    reason). Locking folders out is otherwise the user's choice, as
    `chmod -R` leaves it.

174. (issue 24) "Every change a job with undo where the previous state was
    captured".
    Assumed: the job records each entry's owner, group, mode and ACL before
    it changes it, the helper reports the same for the entries it changed,
    and the whole job is one step on the undo stack; undo and redo run as
    jobs of their own and may need the helper again, which prompts again.
    An entry that fails is reported in a notification and the rest goes
    on; nothing elevated runs when no entry was refused.

175. (issue 24) "An elevation-needed change on a root-owned fixture prompts
    (Linux CI with a polkit fake) and succeeds".
    Assumed: the test job of tier 1 installs `.github/scripts/fake-pkexec.sh`
    as `/usr/local/bin/pkexec`, which runs the program through the
    runner's passwordless `sudo`, and sets `RICK_ELEVATE_FIXTURE`, under
    which `crates/rick/tests/perms.rs` makes a file root's with `sudo`,
    changes its mode through the job (refused, then the helper as root)
    and undoes it; elsewhere that test prints a note and passes. The fake
    platform runs the helper as the current user when a test asks
    (`Fake::set_elevate_runs`), for the protocol round trip.

176. (issue 24) Where the owner and group lists come from.
    Assumed: `/etc/passwd` and `/etc/group` on Linux (a name typed resolves
    through the C library, so LDAP and the like work by name), Open
    Directory through `getpwent` and `getgrent` on macOS (the `_` service
    accounts left out), the Net API's local users and groups plus the
    well-known trustees (Everyone, Authenticated Users, SYSTEM,
    Administrators, Users) on Windows; a name typed into the dialogue is
    resolved when the change is applied.

177. (issue 24) `Capabilities::PERMISSIONS` exists and the local provider
    sets it, but `Provider` has no permission methods and the ssh spec says
    the flag stays off there.
    Assumed: permissions are the platform's business for local entries;
    the Permissions section appears for a local entry only, every other
    entry's Properties shows the general rows. A provider-level API waits
    for a provider that wants it.

178. (issue 24) "User-facing strings still go through one `t!` lookup
    function from the start" (SPEC.md "Language"), and the issue has the
    lookup read Fluent files; the spec names no crate for it, and every
    plugin's strings must go through the same one.
    Assumed: the lookup is `ext::i18n` with the `ext::t!` macro, in the
    dylib the binary and every plugin share, so one bundle per language
    serves the kernel and the plugins alike; `ext` gained `fluent-bundle`
    for it. The strings of the crates below `ext` (`platform`'s error texts,
    `types`' error display) stay English, as do the texts the issue does
    not name: error messages and the reasons in `Error::Unsupported`, job
    and undo labels ("Copy 3 items"), the bodies of confirmations that
    carry data, the status bar's facts and the rows' keys of the info
    sections; a later pass can move them the same way.

179. (issue 24) A label an extension hands the registry once (a setting's,
    a connection field's, a connection schema's title, an info section's,
    a decorator column's) was a `&'static str`; a language applied live
    cannot relabel it.
    Assumed: those labels are `ext::Text`, a Fluent key read every time it
    is shown, so they follow a language change too; `Action::label` and
    `Availability::Disabled` return a `String` built with `t!` on each call.
    The extension API is at version 11 for it.

180. (issue 24) Where a plugin's translations live and how they reach the
    lookup.
    Assumed: `crates/<plugin>/locales/<lang>.ftl`, bundled with
    `include_str!` and handed to `Registry::translations` while the plugin
    registers; the keys carry the plugin's prefix (`git-commit`,
    `ssh-open-console-here`) so one bundle holds every plugin's messages;
    English is the one language every key must have. The names of the
    extension groups (Git, SVN, SSH, Trash, Installer) and the option texts
    of a `Choice` setting are values, not translated.

181. (issue 24) "A CI check fails on a user-facing string literal outside
    `t!` (a lint or a grep over ui-egui)".
    Assumed: `.github/scripts/check-strings.py`, run by the `fmt` job and
    by a test, reads the non-test code of `crates/ui-egui/src` and reports
    a string literal with letters handed to a widget or a window title
    (`ui.label("…")`, `egui::Button::new("…")`, `.on_hover_text("…")`, the
    shell's toast); `t!("…")` on the line is the key, and `// i18n: ok`
    exempts a glyph, a unit or a format of numbers. Text a helper function
    returns was moved into `t!` by hand, since no grep sees it.

182. (issue 24) The German proof.
    Assumed: `locales/de.ftl` translates the Settings screen in full, the
    burger and context menus with every core action and the reasons a core
    action is greyed out, the Properties dialogue's permission words, and
    the plugins with actions in the context menus translate those actions
    in their own `de.ftl` (Git, SVN, SSH, the trash, compress, compare,
    rename, the console, the installer); everything else falls back to
    English, which is what the issue asks of the rest.

183. (issue 24) "A language setting (system default, or a listed language)".
    Assumed: `language` in `rick.toml`, `None` for the system's; the
    Settings row lists *System default* and every language that has
    messages under the name its own file gives it (`language-name`). The
    system's language is `sys-locale`'s list, negotiated against the bundled
    languages by whole tag, then by language alone (`de-CH` finds `de`),
    English when nothing matches; `fluent-langneg` was not used, since it
    speaks ICU's locale type. A file `locales/<lang>.ftl` in the
    configuration directory overrides the bundled messages of that language
    key by key, read at start.

184. (issue 24) Fluent wraps placeables in Unicode isolation marks.
    Assumed: off (`set_use_isolating(false)`), since egui draws the marks as
    boxes; a key nobody wrote comes back as itself, so a missing translation
    is a visible text or key, never a panic.

185. (issue 25) Question 62 assumed that `egui` 0.36 over `winit` 0.30
    "receives hovered and dropped files on Linux (Wayland and X11), macOS
    and Windows". It does not on Wayland: `winit` 0.30.13 implements the
    drop target for X11 (XDND), macOS and Windows only, so a Wayland Rick
    received nothing from outside in 0.1.
    Assumed: on Wayland the platform receives drags from outside itself,
    with a `wl_data_device` of its own on `winit`'s display (joined through
    the raw display handle), reads the `text/uri-list` and hands the files
    and the cursor position to the UI through `DropTarget::incoming_files`
    and `DropTarget::drop_position`; on the other three backends `winit`'s
    hovered and dropped files are kept and the platform supplies only the
    position (`GetCursorPos` against the window, `NSEvent.mouseLocation`
    converted into the view, `XQueryPointer` on the window). The spec's
    "a drop arriving without a cursor position … lands in the focused
    pane's folder" stays as the fallback wherever the position cannot be
    had.

186. (issue 25) The issue asks for the drag source "via winit's raw window
    handle". `eframe` hands out the raw window and display handles of the
    root viewport (its `Frame`); Rick's further windows are egui viewports
    drawn in the same frame, with no handle of their own.
    Assumed: a drag out of a secondary window stays inside it (nothing to
    hand the platform), and a drop into one lands in the focused pane's
    folder, the fallback above; the root window gets both. Neither the
    Wayland nor the X11 source shows a drag image outside the window: a
    Wayland icon surface would need a buffer of Rick's own and XDND has no
    image at all; macOS shows the first file's icon, Windows the system's
    drag cursor.

187. (issue 25) Once the desktop runs a drag, the windowing backend never
    sees the button come up (`AppKit`, OLE and the compositor consume it),
    so egui would keep the row dragged and the button down until the next
    click.
    Assumed: the UI forgets egui's pointer state and the dragged widget when
    the handover succeeds and again when the platform reports the drag
    ended (`DragSource::drag_in_progress`, polled every frame meanwhile);
    the next press starts afresh. The handover itself happens one frame
    after the cursor left the window, so the frame without the ghost is on
    the screen when the desktop takes over, as the issue asks; on Windows
    `DoDragDrop` runs its own loop and the frame that starts it ends when
    the drag does, `winit` buffering the window's messages meanwhile.

188. (issue 25) The issue asks to "re-measure the four macOS benchmarks that
    run against measured targets" without naming them; `BENCHMARKS.md` had
    five macOS targets above the spec's numbers (fixture, listing-first,
    listing-complete, pdf-first, tree-first) and three with no spec number
    (startup, cancel, session-start).
    Assumed: the four are the listing pair, `pdf-first` and `tree-first`,
    the ones the Released paragraph was written about; `fixture` was
    re-measured with them. Every macOS and Windows target follows the 0.1
    rule from the five-run bench of this branch (median × 1.5, rounded up,
    never below the Linux target, and past the worst run where that sits
    higher), and where that lands at or under the spec's number the spec's
    number is the target. The 0.2 tables never measured by the bench
    workflow (viewer-frame, plugin-load, archive-zip-first, office-convert,
    waveform, zip-first, csv-window, previewers-frame) are filled from the
    same run, with the provisional macOS and Windows targets of
    `plugin-load` and `archive-zip-first` set by the same rule. Two targets
    had to loosen by that rule rather than tighten, since the runner did
    not meet them: cancelling the sparse copy on macOS (80 ms, was 50 ms)
    and the fixture on Windows (45 s, was the spec's 30 s, QUESTIONS.md 5's
    Defender case); `BENCHMARKS.md` says why beside each table.
