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
    Decided (2026-11-08): `ext-ssh` replaces it; `ext-vfs-sftp` removed in
    this PR.

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

189. (issue 54) The Compute hash paragraph puts the format toggle and the
    Verify field under the rows of a dialogue and says a selection of
    several files shows one block per file, without saying whether each
    block has its own toggle and field or the dialogue has one of each.
    Assumed: one format toggle and one Verify field for the whole dialogue,
    under the blocks; the format applies to every block, and the pasted
    value is compared against every block's rows of its length, since a
    download page's hash belongs to one of the files and the dialogue shows
    which. Each block keeps its own *Copy all*, which copies that file's
    four lines.
190. (issue 54) The paragraph hides Compute hash on folders and says what
    it hashes are files; it does not say what a symbolic link in the
    selection is. Assumed: the entry shows only when every selected entry
    is a plain file (`EntryKind::File`), so a symlink, whatever it points
    at, hides it like a folder does; following links to their targets can
    come with a spec change.

191. (issue 50) "The manifest gains the list of file extensions a plugin's
    previewers claim so the host answers from manifests without loading the
    plugin"; the spec does not say how a multi-part suffix (`tar.gz`) or a
    name without an extension is matched.
    Assumed: `Manifest::previews` is a list of lower-case extensions
    without the dot, multi-part ones allowed (`"tar.gz"`), written to
    `plugin.toml` as `previews` (absent in a manifest written before API
    12, read as empty); the host says *Plugin available* when any suffix
    after a dot of the file's name equals one of them, case-insensitively,
    for a plugin whose status is *disabled* only. Every previewer plugin of
    the workspace lists the extensions its `score` claims by name; the hex
    view lists none. The extension API is version 12.

192. (issue 50) The spec's four content kinds for the pinned info section
    are `pages`, `text`, `image` and `other`; a file list, a grid and a
    rendered document are none of these by name.
    Assumed: they count as `text`, since each of their documents carries a
    text layer (the listing's dump, the raw CSV, the source), so a pin
    made on a Markdown file holds for a CSV and a text file alike. The
    layout lives in `ui-egui/content.rs`: the listing is one table in the
    explorer's columns plus *Compressed* when the format records it, a
    collapsed directory hides the rows deeper than it until its depth comes
    back, the grid lays out at most 200 columns and says how many more
    there are, a document's pictures are decoded by the `image` crate in
    the UI crate (the first dependency on it there) and drawn no wider than
    the pane and never above their own size, and a link opens with the
    system when it is an `http`, `https` or `mailto` address.

193. (issue 50) "If a plugin has no way to read or write its own declared
    settings today, add the smallest accessor on the extension context".
    Assumed: the extension context is the registry, which now lends a
    `SettingsStore` (`get` and `set` of a `SettingValue` by extension id and
    key) the way it lends the platform services and the question queue;
    the application's `ExtSettings` is that store, shared behind an `Arc`
    and created by the binary before the plugin host loads, so a plugin
    loaded there and an extension compiled in read and write the same
    values and the Settings screen shows what a plugin wrote back. The
    values are read once every extension has declared its schema, which
    is why a plugin keeps the handle from `register` and asks lazily. The
    `Options` of the plugin host carry the store; `App::with_services`
    takes it. Part of API 12.

194. (issue 50) "Development and CI write the setting instead" of
    `RICK_PDFIUM_DIR`, while the tests run every `App` over a temporary
    config directory of the fake platform, where no setting file is.
    Assumed: the CI setup action and the release workflow write
    `library = "<path>"` into `preview-pdf.toml` of the runner's config
    directory (`$XDG_CONFIG_HOME/rick` or `~/.config/rick`,
    `~/Library/Application Support/Rick`, `%APPDATA%\Rick`), which the
    `rick` binary and `cargo xtask bundle` read; and the setup action also
    copies the library beside the test binaries (`target/<profile>/deps/`
    and `target/<profile>/`), which is the spec's second place to look, so
    the tests find it the way a user without the setting would. The
    bundler copies pdfium from the setting (a file, or the directory
    holding it) and says so when the setting is unset; the release smoke
    test keeps running the check binary beside the unpacked `rick` with no
    setting at all. `RICK_REQUIRE_PDFIUM` stays, as the issue allows.

195. (issue 50) "*Browse…* (a file dialogue through `UiHost`, the choice
    stored in the setting)", while `UiHost` has prompt, confirm, show and
    form, every one a synchronous answer-queue dialogue, and the platform
    already has the asynchronous file dialogues (`NativeDialogs`).
    Assumed: *Browse…* opens `platform::NativeDialogs::pick_files` the way
    the side panel's folder *Browse…* does, with the filters the previewer
    names in `MissingLibrary`; the shell stores the chosen file into the
    previewer's setting through `Command::ExtSetting` and opens the preview
    again (`Command::PreviewRetry`). No `UiHost` method was added.

196. (issue 50) The Office previewer renders through its own pdfium
    binding (QUESTIONS.md 104), which also needs the library's path.
    Assumed: one setting for one library: the Office plugin reads the PDF
    previewer's `library` setting through the `SettingsStore` and looks in
    the same places, but never writes it; its message for a missing pdfium
    points *Browse…* at the PDF previewer's setting. The Office plugin's
    own setting is `soffice` (the program, or the directory it is in),
    with the `PATH` and the platforms' install directories after the
    setting, beside the binary and the plugins directory.

197. (issue 50) The issue writes `preview_always = true` "for that SSH host
    (`[[ssh.hosts]]`)", while Rick's hosts are `[[connections]]` tables
    (QUESTIONS.md 163) and a host of `~/.ssh/config` has no table at all,
    being read-only in Rick.
    Assumed: the switch is the connection field `preview_always` of the
    `[[connections]]` table (`provider = "ssh"`), a checkbox in the host
    editor; for a configuration host the application writes a
    `[[connections]]` entry that carries the switch alone (and a cap
    override, when one is set), which `Hosts` merges onto the
    configuration host of that alias instead of hiding it, so *Always
    preview on <host>* and the row's menu work for every host in Network.
    `Provider::remote_preview_always` reports the switch; the kernel skips
    the cap when it is on. Cloud accounts have no editor yet, so their
    rows get the menu entry only once their schema has the field.

198. (issue 50) *Copy* "puts the whole file's text on the clipboard"; the
    spec says nothing about a file too large to hold as text, nor about
    the encoding.
    Assumed: the file is read again as a job up to 256 MiB (`COPY_LIMIT`),
    decoded by its byte-order mark (UTF-8, UTF-16) and as UTF-8 otherwise
    with invalid sequences replaced, and put on the clipboard through
    `platform::Clipboard`, with a notification of the character count; a
    larger file is refused with a notification. The button shows for a
    preview with a text layer and no pages: text, code, Markdown, HTML and
    CSV, not a file list (whose text layer is the listing's dump) and not a
    paged document.

199. (issue 50) *No previewer* when a previewer claims the file but fails
    to open it.
    Assumed: the placeholder is for a file no previewer claims, which the
    application now reports as `Event::PreviewNone` (with the disabled
    plugin that would claim it, from the plugin rows); a previewer that
    claims a file and fails keeps `Event::PreviewFailed` and the error
    line, since the type is known and the failure is the news.

200. (issue 50) How a test stands on a machine without pdfium or
    LibreOffice when CI has both beside the binary.
    Assumed: `PdfPreview::with_search(Search::SettingOnly)` and the Office
    plugin's `Lookup::with_search` restrict the search to the setting, the
    one place a test controls; the acceptance test of the in-pane message
    uses it and then browses to the real library where the machine has
    one.

201. (issue 50) Space "while the preview has focus" plays or pauses, while
    Space in a list toggles the focused entry.
    Assumed: the Preview pane table of the shortcut screen has one row,
    *Play / pause media* (`preview.play`, Space), live while the Preview
    tab has the focus and tried before the other tables there; it is not
    bound in an explorer. The transport in the toolbar is play or pause, a
    seek bar with elapsed and total time and a volume slider; speed, frame
    step and loop of `Transport` have no control in the pane.

202. (issue 51) The issue builds on #50, "which is merged into `dev`"; when
    this task started, `dev` carried #50's spec change and none of its
    code, and #50 landed on `dev` (#56) while the work was under way.
    Assumed: the branch was brought level with `dev` before it merged, and
    the four items use #50's API where it exists: the markup plugin's
    manifest lists its extensions in `previews`; the Rendered toggle is the
    `PreviewControl::Toggle` with `Icon::Rendered` the pane shows for any
    previewer, on for every newly shown file, and the raw source is the
    text layer the pane's *Copy* reads; a disk image enters on Enter
    through the archive provider's path, so `archives.on_open` governs it
    like any archive; the 7-Zip library's setting is read and written
    through the registry's `SettingsStore` (question 193), with no store,
    as in the compress plugin's extraction, nothing is remembered; and the
    missing library is `Content::MissingLibrary` carrying
    `ext::MissingLibrary` (the library's name, the install command for this
    platform, the setting `vfs-archive`/`sevenzip_library` and where Rick
    looked), which the pane lays out with its *Browse…* and *Retry*, while
    the provider, which has no pane, raises the same facts as an
    `Error::Extension` carrying `ext_vfs_archive::MissingLibrary`.
203. (issue 51) "The `unsafe` needed to call the library stays in
    `platform` or behind the smallest wrapper the Architecture rules
    allow — record the choice".
    Assumed: in `platform`, as `platform::sevenzip`: the `libloading` load,
    the COM-style virtual tables the library is called through
    (`IInArchive`, `IInArchiveGetStream`) and the three objects it calls
    back into (an `IInStream` over a file, the open callback, the extract
    callback with its `ISequentialOutStream`), each `unsafe` block with its
    `SAFETY:` comment, behind a safe API (`SevenZip::load`, `open_file`,
    `open_stream`, `Archive::items`, `sub_stream`, `extract`). The plugins
    reach it through `ext::platform` and hold no `unsafe` of their own.
    `BSTR`s are freed with the C `free` the library's Unix build allocates
    them with (a 4-byte length prefix, `wchar_t` text) and `SysFreeString`
    on Windows.

204. (issue 51) Two plugins read disk images (`ext-vfs-archive` lists and
    reads them, `ext-preview-archive` lists them in the pane) and the spec
    puts a library's path in "the plugin's own TOML settings file".
    Assumed: one setting, `sevenzip_library` in `vfs-archive.toml`, owned
    by the archive provider, which the library travels beside; the
    previewer reads disk images through `ext_vfs_archive::diskimage` with
    the same `Resolver`, so it reads and writes that one setting, and
    `ext-preview-archive` depends on `ext-vfs-archive` as the compress
    plugin does (question 127; deny.toml names it). Both resolve lazily on
    the first image shown, in the spec's order: the setting, then beside
    the binary and in the plugins directories (`plugins/vfs-archive/` and
    `plugins/` beside the binary and under the config directory), then
    the platform's usual installs (`/usr/lib/7zip/7z.so` and the p7zip
    paths on Linux, Homebrew and MacPorts on macOS, `C:\Program
    Files\7-Zip\7z.dll`); the first hit is written back.

205. (issue 51) How a disk image becomes a file tree: the library's
    container handler (VHD, VHDX, VMDK, VDI, QCOW, Dmg) lists the disk as
    one item, not the files.
    Assumed: the walk 7-Zip's own browser does, nested through
    `IInArchiveGetStream`: the container's main item (`kpidMainSubfile`)
    as a stream, opened with the partition table handlers (GPT, MBR) and,
    failing those, the file system handlers (NTFS, FAT, ext, HFS+, APFS,
    UDF, ISO); each partition as a stream opened with the file system
    handlers. The tree lists one folder per partition, named as 7-Zip
    names it (`0.Basic data partition.ntfs`), with the partition's files
    inside; a partition no handler reads is a file of its size; a raw
    image is the disk or the file system itself, so a FAT floppy image
    lists its files at the top. Raw images are `.img`, `.ima` and `.raw`;
    the containers are told by their signatures where they have one
    (VHDX, QCOW2, VMDK, a dynamic VHD, VDI) and by name otherwise (a fixed
    VHD, DMG, raw). The spec's "no mature Rust crate" stands: nothing is
    parsed natively.

206. (issue 51) The Mount action's outcome and its undo.
    Assumed: `platform::Mount::mount_image` returns the `Volume` the image
    became, as `Volumes::volumes` lists it, and the action runs it as one
    job for the selection, so the mount shows in the operations area; the
    side panel's live volume watch (every two seconds) shows the row, and
    nothing navigates to it. On Linux the image is set up as a loop device
    (`-r` for an ISO), the first partition is mounted when the kernel
    announces one, the device itself otherwise, and Eject on such a row
    unmounts and deletes the loop device (`udisksctl loop-delete`) instead
    of powering a drive off. On macOS `hdiutil attach -plist` names the
    mount points and the first is the volume; on Windows the drive letter
    the image's volume gets. A mounted VHD or VHDX shows on Windows as a
    fixed disk, so its row has no Eject there; a mounted ISO is a virtual
    optical drive and ejects. The formats per platform are the spec's
    table, as `platform::NATIVE_MOUNTABLE`, and the `Fake` lets a test set
    another platform's list, so the action's availability is tried for all
    three on every runner.

207. (issue 51) "The source one toggle away" for Markdown and HTML is the
    code previewer's highlighted view (SPEC.md "Preview support").
    Assumed: `ext-preview-markup` depends on `ext-preview-code` and builds
    its document over a `CodeDoc`, so the source behind the toggle is the
    same highlighted text the code previewer shows when the plugin is
    disabled, with the pretty-print toggle offered while the source is
    shown; the dependency is the kind question 127 allows. The CSV grid
    keeps question 107's behaviour: the delimiter sniffed over the first
    twenty records, the first row a header unless all its cells read as
    numbers, up to 256 MiB indexed, the raw text up to 1 MiB as the source
    and the text layer (a file over 1 MiB is cut there; Copy's "whole
    file" for such a file is #50's pane to resolve).

208. (issue 51) Where the bundled 7-Zip library comes from on each
    platform, and its licence.
    Assumed: `cargo xtask bundle` copies `7z.so` or `7z.dll` and
    `License.txt` from `RICK_SEVENZIP_DIR` into `plugins/vfs-archive/`
    (the licence as `7-Zip-License.txt`), the way pdfium comes from
    `RICK_PDFIUM_DIR`; `.github/scripts/sevenzip.sh` fills that directory:
    for the release archives from the pinned 7-Zip 25.01 sources on Linux
    and macOS (`make -f ../../cmpl_gcc.mak` and `cmpl_mac_arm64.mak` in
    `CPP/7zip/Bundles/Format7zF`, as 7-Zip's DOC/readme.txt says) and from
    the pinned installer's `7z.dll` on Windows, every download checked
    against its SHA-256; for the tests, Ubuntu's `7zip` package on Linux,
    the runner's 7-Zip on Windows and the same source build, cached, on
    macOS. The release archive carries the licence at its root beside
    THIRD-PARTY-LICENSES.md as well. `RICK_SEVENZIP_DIR` is a build and
    test variable; the plugin reads no environment variable. The macOS
    source build could not be run in this task's environment and is
    verified by tier 2.

209. (issue 51) "Integration test on Linux x64 where udisks is available
    in CI, otherwise a fake Mount".
    Assumed: `crates/tests/tests/mount_linux.rs` mounts a generated ISO
    through `platform::Native`, finds it among the volumes, reads a file
    from it and ejects it, and runs where `udisksctl status` answers;
    `.github/actions/setup` installs udisks2 with a polkit rule allowing
    the runner user the udisks2 actions and sets `RICK_REQUIRE_UDISKS` when
    the daemon answers, so the test fails rather than skips there. The
    action itself, its availability per platform and the volume's
    appearance and Eject in This Machine are tested through the `Fake`.

210. (installer follow-up, 2026-11-08) specs/ext-installer.md lists the
    Installed Apps view "in the burger menu under *Installer*", and
    SPEC.md's burger menu holds the kernel's global entries only, with no
    hook for an extension's action (question 150 left the menu without
    extension groups).
    Assumed: `Action::app_menu(&self) -> bool`, false unless overridden,
    joins the extension API (version 14): the burger menu lists, after
    the Layouts submenu and a separator, every action whose `app_menu` is
    true and whose availability is Enabled for the focused pane's context
    (`Invocation::Menu`), the core ones directly and the rest under one
    submenu per `ActionGroup::Extension` name, each running through the
    same `RunAction` request as the context menu. The installer's
    *Installed Apps…* is the one action that opts in; the per-row actions
    of the view and *Install as application…* need a selection and stay
    out. SPEC.md's burger menu section is not edited; the hook is
    recorded here for the next spec pass.

211. (installer follow-up, 2026-11-08) specs/ext-installer.md "Detection
    is a job" wants the install form to open when the job reading the
    archive finishes, while a form could only be opened from inside
    `Action::run`, on the app thread, and SPEC.md's rule is that the UI
    emits commands, the application handles them and publishes events.
    Assumed: the smallest hook is on the outcome side. `ActionOutcome`
    gains `Later(FormLater)`, the receiving end of a channel the job
    fills with the `Form` and a `Continuation` (the rest of the action,
    `FnOnce(Option<Values>) -> Result<ActionOutcome, Error>`) when it is
    done, and nothing when it fails or is cancelled; the application
    keeps the receivers, looks at them on every poll and hands a form
    that arrived to `UiHost::form_later(pane, form, then)`, which a
    blocking host (the headless tests) answers at once, returning the
    continuation's outcome, and the egui host queues, showing the form
    as its one dialogue and sending `Command::Continue { pane, then,
    values }` (`None` when dismissed), which the application runs like an
    action's outcome. The continuation runs once and its clones share
    that run, so `Command` stays `Clone`. No `Event` carries the form: a
    form travels through the `UiHost` today too, and the application,
    not the UI, runs the rest. Extension API 14.

212. (installer follow-up, 2026-11-08) specs/ext-installer.md says
    "switching the kind recomputes the main executable, icon and command
    defaults", and `ext::Form` is data the UI lays out once: a field's
    value cannot change the other fields, only the preview is recomputed
    from the values as they are typed.
    Assumed: the defaults are derived in `assemble`, the preview and the
    plan: the chosen kind decides the commands (a GUI application links
    one command named after the app id at the chosen main executable, a
    toolchain its `bin/` contents) and whether a desktop entry and an
    icon are written, and the summary rows show the result. The command
    checkboxes are laid out for the detected kind; when the chosen kind
    differs they no longer describe the offer and the chosen kind's
    defaults are taken whole. The main executable and the icon stay the
    user's choice in their lists, prefilled by detection for the detected
    kind. A form whose fields react to another field is a later API
    question.

213. (0.2.0 release, 2026-11-08) CHANGELOG.md's release heading carries
    "the date of the tag", and the tag is made only after the dev → main
    pull request is reviewed and tier 2 has passed, which may be a later
    day than the one the release branch was prepared on.
    Assumed: `## [0.2.0] - 2026-11-08`, the day of preparation, stays
    unless Serge asks for the tag's day before the batch merges; the
    release body is this section either way.

214. (0.2.0 release, 2026-11-08) The 0.1.0 section opens with a
    one-paragraph summary before "### Added"; the instructions for the
    0.2.0 section name the heading, the new Unreleased heading and the
    link references.
    Assumed: the 0.2.0 section gets the same one-paragraph summary, since
    the section is the release body. The entries stay as they were except
    for two corrections as a record: the API-8 entry no longer names the
    viewer #56 withdrew, and the #18 entry lists the three previewers that
    ship, its CSV grid and rendered Markdown and HTML having been
    superseded by `ext-preview-markup` (#51).

215. (0.2.0 release, 2026-11-08) specs/ext-installer.md names its target
    version twice: in the overview list ("Target version: 0.3.") and under
    "Settled questions" ("Target version: 0.3 (system integration)").
    Assumed: both now say it ships with 0.2 (Linux only); the `rick
    install <archive>` command line of "Other ways in" stays a 0.3 item.

216. (0.2.0 release, 2026-11-08) After the version bump, `0.2.0-dev` is
    still named in SPEC.md ("During 0.2 the workspace version is
    0.2.0-dev"), in the example manifest of crates/plugins-native/README.md
    and in the changelog's #16 entry, beside the doc comment and the test
    fixture in crates/plugins-native/src/manifest.rs.
    Assumed: all are records or examples and stay; SPEC.md's sentence is
    not one of the two authorised edits.

217. (0.2.0 release, 2026-11-08) The README's "Release archives" paragraph
    was checked against the plugins `cargo xtask bundle` lays out (the
    `ext-*` cdylib crates, as .github/scripts/check-plugins.sh lists
    them): the SSH, S3, Azure Blob and Google Cloud Storage providers and
    the hex and 3D previewers were missing and are added. The extension
    point table's Provider row still says "SFTP", which `ext-ssh`
    replaced.
    Assumed: the table row is not among the listed edits and stays for a
    follow-up.

218. (0.2.0 release, 2026-11-08) Tier 2's first runs on the 0.2 content
    failed in two tests on timing, not on the platform: the SSH host test
    read rick.toml right after `settle()` while the settings saver, a task
    of the runtime that `settle` does not wait for, had not written it
    (Linux arm64), and the cloud upload test expected the copy's final
    byte count as the job's last progress report, while the transfer job
    reports its item count right after the copy and reports closer
    together than `runtime::PROGRESS_EVERY` go out as the last one (Linux
    x64; the "upload ends short" of issue 21's tier 1 runs).
    Assumed: both are assumptions of the tests, not defects: the tests
    read the settings file until it holds the expected text
    (`tests::read_until`) and accept a complete item count as the job's
    final report; the coalescing stays as the side panel specifies it,
    twenty reports a second per job.

219. (0.2.0 release, 2026-11-08) Building the 7-Zip library from the
    pinned sources on macOS fails under Apple clang 17: the mac makefile
    compiles with -Weverything -Werror, and every new compiler brings new
    warnings (-Wc++-keyword on the arm64 assembly macros,
    -Wimplicit-void-ptr-cast in LzmaEnc.c).
    Assumed: on macOS the library builds with -Wall -Wextra and without
    -Werror, through the makefile's own variables; the Linux build keeps
    the gcc flags as the makefile has them, and the pinned release and
    its checksums are unchanged.

220. (0.2.0 release, 2026-11-08) The in-process SSH server of the plugin's
    live tests and of the host's `ssh.rs` runs its commands through this
    machine's `sh`, and the fixture home carries a name with a newline;
    on the Windows runner no Unix shell answers and the name is invalid,
    so twelve tests failed there in the first Windows run of the 0.2
    content, while specs/ext-ssh.md serves Unix-like hosts only.
    Assumed: the tests against the in-process server say so and stop on
    Windows (the plugin's other tests run there); a Windows host for the
    SSH fixture is not a 0.2 concern. The macOS runner refuses a non-UTF-8
    name, reports a `/var/…` home as `/private/var/…` and answers the
    profile probe with BSD; the fixture and the expectations follow.

221. (0.2.0 release, 2026-11-08) The Windows Search acceptance writes a
    file into the fixture directory and expects the index to find it; on
    the Windows runner the service starts in CI's setup and then crawls
    the whole profile, the cargo registry the build fills included, and a
    file written at test time was not indexed within five minutes, with
    the indexer's backoff disabled or not. A probe on an idle runner found
    such a file within twenty seconds through the connector's own script
    and SQL, in both scope spellings.
    Assumed: the connector works; the Windows job requires the service
    (RICK_REQUIRE_WINDOWS_SEARCH) but names no fixture, so the test there
    checks availability and a broad query; the fixture check stays on
    macOS and Linux, and the test keeps its minute and a half.

222. (0.2.0 release, 2026-11-08) `vfs-local`'s watch handed every event to
    its stream through a bounded channel whose sender blocks while the
    channel is full, and dropped the watcher before the receiver; FSEvents
    joins the callback thread when the watcher drops, so a relist over a
    burst of more than 1024 events (a bulk rename of a thousand files)
    waited for ever and the macOS job ran into its timeout.
    Assumed: the watch state closes its receiver first, so the blocked
    send fails and the thread ends; the channel's depth and the blocking
    send, which keep the watcher from dropping events, stay as they are.
    Found through a stack sample of the hung test process on a throwaway
    branch (`ci/macos-hang`, which this session could not delete).

223. (0.2.0 release, 2026-11-09) The close-out's order has the 0.3 kickoff
    (workspace version 0.3.0-dev on dev) after the release is published;
    the tag could not be pushed from the session that ran the close-out,
    the kickoff was merged while the tag's release workflow still ran, and
    that workflow then failed on three platforms, so the fixes must reach
    main through dev while release.yml requires main's workspace version
    to be the tag's.
    Assumed: the kickoff is reverted on dev together with the release
    fixes, the batch dev → main carries 0.2.0 again, the tag is deleted
    and made anew on the new merge commit, and the kickoff is reapplied
    once the release is out. The revert and the reapplication are plain
    commits; nothing is rewritten.

224. (design refresh, issue 60, 2026-11-09) The Figma note *Spacing &
    density* gives the title bar 46 px (Comfortable) and 36 px (Compact),
    while the *Density* variable collection, and the frames themselves,
    say 40 and 34.
    Assumed: the variables, since the token set matches the collection one
    to one; the note is out of date.

225. (design refresh, 2026-11-09) The row-density setting had three levels
    (compact, normal, comfortable); the design has two (Comfortable, the
    default, and Compact).
    Assumed: `density = "normal"` in an existing `rick.toml` reads as
    Comfortable, `compact` and `comfortable` stay as they are, and the
    file is written with the new value the next time settings change.

226. (design refresh, 2026-11-09) SPEC.md's Theming said "accent colour can
    be chosen"; the handoff says every colour comes from the token set,
    whose pairs were checked for WCAG contrast, and that more colour comes
    as further schemes.
    Assumed: the accent-colour setting is withdrawn; an `accent` key in an
    existing `rick.toml` is ignored and dropped on the next write.

227. (design refresh, 2026-11-09) SPEC.md's Icons marked an active toggle
    with Phosphor's fill weight; the Figma note says toolbar buttons are
    ghost buttons and only active toggles keep a tint.
    Assumed: the tint (`bg/selected-soft` behind an `accent/focus` icon)
    and the regular weight everywhere; the fill-weight font stays loaded
    for a later use.

228. (design refresh, 2026-11-09) The Figma context menus, labelled draft,
    show fewer entries than SPEC.md's groups: on a file no *Open in new
    tab*, *Open in terminal here*, *Duplicate*, *Paste as link*, *Compress
    to archive* or *Extract here*; on empty space no *Paste as link*, *Open
    in system file manager*, *Refresh* or the view, sort and group entries.
    The note on the file menu says the *Copy as* entries are placeholders
    to align with the three copy actions.
    Assumed: the frames fix the groups, their order and their look, and
    the Action registry keeps deciding what is listed, so every registered
    action that applies stays in its group (dropping actions would remove
    functions, which is outside a design refresh). *Copy as* holds *Name*,
    *Path* and *Path and name*, not Figma's *Full path*, *File name* and
    *Path as URI*. The view entries leave the empty-space menu, since the
    view options menu holds them; sort stays on the column header and
    group in the command palette.

229. (design refresh, 2026-11-09) With tabs in the title row and no
    per-pane strips, SPEC.md's tab operations (new tab, close tab, open in
    new tab, duplicate tab, next and previous tab) need a meaning.
    Assumed: a tab is a workspace. *New tab* and *Open in new tab* open a
    new workspace with one pane at the location; Mod+W closes the
    workspace tab; next and previous cycle the row; *Duplicate tab* copies
    the active pane into a new workspace. A pane leaves a workspace by
    being dragged out (edge, centre, tab row or outside) or by F6, so
    there is no *Close pane* command and a workspace never ends up empty.
    *Split right* and *Split down* keep their meaning.

230. (design refresh, 2026-11-09) The operations drawer shows a round
    cancel button per job and nothing for pause and resume, which SPEC.md
    and the job table have; it also shows no finished or failed job, which
    SPEC.md keeps listed briefly or until dismissed. The summary's
    percentage is a literal `#2e85e0` in Figma, not a token.
    Assumed: a pausable running job shows a pause or resume ghost button
    before the cancel button; finished and failed jobs are compact rows
    like the waiting ones, the failed one with its error in `status/danger`
    and a dismiss button; the percentage is `accent/focus`.

231. (design refresh, 2026-11-09) SPEC.md left open which layout toggles
    sit in the title row. The Figma title bar has two icons, sidebar-left
    and sidebar-right, before the window controls.
    Assumed: the side panel toggle and the preview pane toggle, in that
    order; the open question is closed.

232. (design refresh, 2026-11-09) The footer frames show the folder and
    file counts of an unfiltered list; SPEC.md's status line read "12 of
    340 files" while a filter narrows the list.
    Assumed: the counts read "12 of 340" while a filter is active, in the
    footer's form; nothing else changes.

233. (design refresh, 2026-11-09) The hash dialogue is drawn for one file;
    SPEC.md hashes several selected files in one job and showed a block
    per file.
    Assumed: with several files the body repeats the hash card per file,
    each under a row naming the file, Verify on top compares against every
    row, and the header names the count and the total size.

234. (design refresh, 2026-11-09) Only the View page of the settings window
    is drawn; the category list names General, View, Appearance and the
    plugins.
    Assumed: View holds the list settings (Files, Sorting, Side panel);
    Appearance holds Theme, Density and Font size; General holds Language,
    Start with, Opening an archive, After a drag, the spring-loading delay
    and the subtree-search ignore list; a plugin page renders the plugin's
    settings schema as cards; Keyboard shortcuts and Extensions stay their
    own screens, opened from the burger menu, in the dialogue frame.

235. (design refresh, 2026-11-09) The Figma workspace tabs carry a folder
    icon, a code icon and a download icon for three folders.
    Assumed: a tab's icon is the side panel's icon when the folder is a
    place (Downloads, Home, Trash and so on), the provider's icon for an
    archive, a remote host or a cloud account, and `FOLDER` otherwise; the
    code icon on *flowcpt* has no rule behind it that the file states.

236. (design refresh, 2026-11-09) The Compact frame's toolbar is 42 px and
    its preview toolbar 37 px, where the Density tokens give 28 + 2 × 4 =
    36 for both.
    Assumed: the formula from the tokens (button size plus twice the
    toolbar padding) in both densities; the Compact frame was not re-laid
    to its own tokens.

237. (design refresh, 2026-11-09) The install dialogue labels the scope
    *Install for: Just me / All users*; specs/ext-installer.md says *Scope:
    user space (recommended) / system-wide* and lists the fields in another
    order, with Category and the commands among them.
    Assumed: the GUI chapter uses the Figma labels and order, with Category
    and the commands under Advanced options; specs/ext-installer.md is not
    among the files this task edits and keeps its wording, the GUI chapter
    governing the look.

238. (design refresh, 2026-11-09) The view options menu lists Details, List
    and three icon sizes; SPEC.md also has the Tree view and an extra-large
    icon size.
    Assumed: both stay reachable through the command palette and their
    shortcuts, and the menu lists what the design shows.

239. (design refresh, 2026-11-09) The *Components* frame's FileRow uses
    `bg/selected` (`#90d1ff`) for its selected state, while every row in
    the main frames and the *States* frame uses `bg/selected-soft`.
    Assumed: `bg/selected-soft` for selected rows in the focused pane, as
    the frames show; `bg/selected` stays in the token set, reserved, and
    `bg/active` marks a pressed tool button.

240. (design refresh, 2026-11-09) Saved layouts were "an arrangement of
    panels and tabs"; with workspaces the unit needs stating.
    Assumed: a saved layout is one window's arrangement — its workspace
    tabs and their panes — and the start layout restores all of them.

241. (design refresh, 2026-11-09) The Portal spinner's component note says
    reduce motion shows "a static ring or a plain bar"; the yellow note
    says "static portal, no flying files".
    Assumed: a still portal — the rings drawn at rest, no glyphs — in both
    the spinner and the burst.

242. (design refresh, 2026-11-09) The handoff says every text and control
    pair of the "Rick" collection meets WCAG contrast. Computed from the
    variables' values, three pairs do not: `text/secondary` on
    `bg/control` and on `bg/selected-inactive` (4.19:1 in Light, 4.35:1
    in Dark, under the 4.5:1 for text — the Size, Type and Modified
    columns of a selected row in a pane without focus), `border/strong` on
    `bg/pane` in Light (2.94:1, under the 3:1 for controls — a checkbox on
    a pane) and `status/warning-border` on `bg/pane` and `dialog/bg` in
    Light (2.61:1 — the border of the installer's version field).
    Assumed: the tokens stay as the Figma file has them, the unit test
    that holds the pairs to the rule leaves these three out and names
    them, and the values are for the design to revisit; nothing is
    invented in their place.

243. (design refresh, 2026-11-09) The shortcut table says Mod+W closes the
    workspace tab; the spec says nothing about the last tab of the last
    window, and *Windows* says closing the last window asks to exit.
    Assumed: Mod+W on the last workspace tab of the last window does
    nothing, as the model keeps the last pane; closing the window is
    Mod+Shift+W's job (the window's close button).

244. (design refresh, 2026-11-09) The shortcut table keeps *Duplicate tab*
    (Mod+Shift+D); a workspace tab is a whole layout, and whether a
    duplicate copies the layout or only the folder is not said.
    Assumed: a new workspace with one pane at the focused pane's folder,
    the same as Mod+T, until the layout copy is designed.

245. (design refresh, 2026-11-09) *Workspace tabs* makes the tab row a
    target for a dragged pane; files and folders dragged from a list over
    a workspace tab are not mentioned.
    Assumed: a workspace tab takes a drop into the folder of its focused
    pane, and resting on it while dragging shows the workspace, as the
    pane tabs of 0.2 did.

246. (design refresh, 2026-11-09) Until the pane handle of the panes PR
    exists, a pane's one tab in its bar stays its drag handle, and such a
    tab can be released on another window's title row, which the spec
    covers for workspace tabs only.
    Assumed: the pane becomes a new workspace tab of that window, as it
    does on its own window's row.

247. (design refresh, 2026-11-09) *Controls* gives menu entries 30 px; the
    *View options menu* frame draws its rows 32 px tall, with 18 px
    switches and a 28 px segmented control in them.
    Assumed: the view options menu takes the frame's 32 px rows; plain
    menus keep 30 px.

248. (design refresh, 2026-11-09) *Panes* says "F6 (Dual pane) splits the
    focused pane to the right … and back again"; the shortcut table, the
    single source of defaults, has no F6 and no Dual pane row.
    Assumed: no Dual pane command until the table names it; Mod+Shift+R
    (Split right) stays the way to a second pane.

249. (design refresh, 2026-11-09) The *Empty folder* frame's hint reads
    "Drop files here, or press Ctrl+Shift+N for a new folder."; Rick spells
    every shown shortcut in the table's text form, `Mod+Shift+N`, in menus
    and tooltips alike. Assumed: the hint takes the key map's spelling like
    the rest of the UI; a platform spelling (Ctrl, ⌘) for shown shortcuts
    would be one change for all of them, not this hint's.

250. (design refresh, 2026-11-09) *Operations drawer* gives a running job
    "its icon (the Portal spinner for a copy or move; `ARCHIVE` for an
    archive job, and so on)", but a job in the job table has a name and a
    state, no kind. Assumed: the spinner for every running job except one
    whose name starts with *Extract*, *Compress*, *Archive*, *Zip* or
    *Unzip*, which shows `ARCHIVE`; a job kind on the job table would let
    every plugin name its glyph.

251. (design refresh, 2026-11-09) *Trash pill*: "the size and the share
    come from the trash provider's statistics", but no trash API reports a
    size — the platform lists the items, the trash plugin shows them.
    Assumed: the app sums the trashed items' sizes when it lists the trash
    (a folder is walked once, links are not followed) and takes the share
    against the volume whose mount point contains them; no pill while the
    trash is empty, the low tint while the disk's size is unknown.

252. (design refresh, 2026-11-09) *Hash dialogue*: a match gives the Verify field a
    `status/success-bg` pill; no match "a `status/danger` border and a *No
    match* pill", whose colours neither the tokens nor the Figma file (only
    the matching state is drawn) give.
    Assumed: the *No match* pill is `status/danger` text on `status/danger`
    at 15 % over the field; a `status/danger-bg` token would replace it. The
    clipboard is read on the dialogue's first frame, hex of either case
    before Base64, and anything that is not a digest's length leaves the
    field empty.

253. (design refresh, 2026-11-09) *Settings window*: "a window of their
    own, 880 × 640", and the controls "a switch, a dropdown, a segmented
    control, a text field or a button" — the 0.2 screen had sliders for the
    spring-loading delay and the font size, and the test harness has no
    viewports.
    Assumed: the window is an egui viewport where the backend has them and
    an area over the main window, in the dialogue frame, where viewports are
    embedded (the tests and the screenshots); the spring-loading delay is a
    dropdown of 200, 500, 750, 1000, 1500, 2000 and 3000 ms (plus the saved
    value when it is none of these) and the font size a dropdown of 9 to
    24 pt; the General page's groups are Language, Session, Archives, Drag
    and drop and Search, the Appearance page's Theme and Type and spacing;
    a plugin's page is one card named after the plugin, its Integer fields
    as text fields that take a number within the schema's bounds; the
    category glyphs are chosen from the plugin's id (pdf, office, 3d,
    console, ssh, archive) with a puzzle piece otherwise, manifests having
    no icon.

254. (design refresh fix round 1, 2026-11-09) *Footer*: "one row at the
    foot of every pane, as tall as the toolbar (`button/size` plus twice
    `space/toolbar-y`)"; the three *Rick — refined* frames draw the status
    bar 39 px tall in Comfortable and 35 in Compact — the 26 px filter
    field with `space/toolbar-y` above and below it under the hairline —
    and the hidden-files toggle 26 px like the field, not `button/size`;
    issue 79 item 4 asks for the Figma height.
    Assumed: the footer takes the frames' height, the hairline, the field
    and the toolbar padding (`Metrics::footer_height`), the toggle is the
    field's 26 px, and the preview pane's footer follows; the spec's "as
    tall as the toolbar" is for the author to reword.

255. (preview formats, 2026-11-09) *Vector images* gives an SVG "the
    raster images' Fit and 1:1 plus the *Rendered* toggle", makes its
    source the text layer, and shows transparent regions "over the
    checkerboard raster images with alpha use". The pane offers Fit and
    1:1 — and so sends the actual-size request — only to a document with
    neither pages nor a text layer, and draws no checkerboard behind any
    frame; the GUI chapter is the design refresh's.
    Assumed: the document answers both kinds of request as the spec says
    (a fitted request rasterises to fit the pane, a `MAX_ACTUAL` request
    gives the intrinsic size at the display scale, both proven by its
    tests) and keeps the text layer so *Copy* and find work, while the
    pane shows an SVG as text for the toolbar's purposes: Fit, with
    *Rendered* and *Copy*, no 1:1 button until the pane's rule becomes
    "a raster is any document whose frames are pixels", one change in the
    pane for the design refresh, with the checkerboard.

256. (preview formats, 2026-11-09) *Vector images*: "*Copy* copies the raw
    SVG as text — decompressed for SVGZ". *Copy* is the app's: it reads the
    whole file itself (the whole file, not the window the pane loaded) and
    decodes the bytes as text, so it never asks the previewer what the text
    of a file is, and the gzip of an SVGZ would land on the clipboard.
    Assumed: the app inflates a gzip-compressed file before decoding it,
    which serves SVGZ and any gzip-compressed text alike and keeps the
    previewers out of *Copy*; a hook on the Previewer trait for the text of
    a whole file would be a change to the versioned contract.

257. (preview formats, 2026-11-09) *Icon containers*: the variants of an
    ICO, CUR or ICNS are pages, "the pane's standard pager steps through
    them", and "1:1 shows actual pixels … both are the previewer's own
    work on the frame it returns at the size the pane asked for, so the
    pane needs nothing new"; XPM is text with *Copy*. As in 255, the pane
    sends the actual-size request only for a document with neither pages
    nor a text layer, so a container with two or more variants, and an
    XPM, get the pager or *Copy* but no 1:1 button; a single-variant
    container gets Fit and 1:1. The toolbar's `Icon` set has no crosshair
    for the hotspot toggle either.
    Assumed: the document answers both kinds of request (a fitted request
    magnifies by the largest whole factor up to 8, a `MAX_ACTUAL` request
    gives the variant's own pixels), proven by its tests; the hotspot
    toggle shows the `Tag` glyph until the set has a crosshair; both are
    the pane's to change under the design refresh.

258. (issue 101, 2026-11-09) *About dialog*, Generation: "`RICK_CONTRIBUTORS`
    (newline-separated names) from `crates/rick/build.rs`". A build script
    hands cargo one `cargo:rustc-env` directive per line, so a value with a
    newline in it cannot travel through the environment.
    Assumed: `build.rs` writes the names, one per line, into
    `OUT_DIR/contributors.txt` and `RICK_CONTRIBUTORS` names that file,
    which the binary embeds with `include_str!`; `RICK_CONTRIBUTOR_ROLES`
    names a copy of the repository's `CONTRIBUTORS.md` the same way (empty
    when there is none), for the roles of the credits card. The release
    build checks the repository out with its whole history, since a shallow
    clone's shortlog names one person.

259. (issue 101, 2026-11-09) *About dialog*, Third-party licences page:
    the bundled runtimes are "listed by hand in `about/bundled.toml` with
    name, version, …". `release.yml` pins the GStreamer and 7-Zip releases
    but downloads the latest pdfium build of bblanchon/pdfium-binaries, so
    pdfium's version in the file can drift from the archive's.
    Assumed: `bundled.toml` records the pdfium build current when the file
    was written (chromium/8138, 2026-11-04) and says so in a comment; the
    fonts are egui's bundled four (Ubuntu Light, Hack, Noto Emoji, the
    emoji icon font) and Phosphor Icons, with the versions from the font
    files' name tables. Pinning pdfium in `release.yml`, or reading the
    version from the downloaded release, would close the gap.

260. (issue 101, 2026-11-09) *About dialog*, Generation: "The bundle puts
    `third-party.json` beside the binary (macOS: `Contents/Resources/`).
    The dialog reads it once at startup."
    Assumed: the release's package step copies the file beside `rick` on
    Linux and Windows and into `Rick.app/Contents/Resources/` on macOS; the
    binary looks beside itself first and in `../Resources/` second, so a
    bundle and a flat directory both work, and `cargo xtask bundle` (a
    developer's layout) carries no file: a build without it shows "not
    available in this build", as the spec says for `cargo run`.

261. (issue 101, 2026-11-09) *About dialog*, Generation: "The release
    workflow fails when the file is missing or when the set of crates in it
    differs from `Cargo.lock`." `Cargo.lock` holds crates of every platform
    (wasm, Android), the build and dev dependencies and the workspace's own
    crates, none of which the release links, so the two sets never match
    as they are; and cargo-about lists the workspace crates themselves
    (they do not say `publish = false`, which is what its `private.ignore`
    reads) plus a few crates of target expressions it cannot evaluate
    (`windows_x86_64_gnu`, `schannel`).
    Assumed: `.github/scripts/check-third-party.py` fails the `licenses`
    job when the file is missing or malformed, when a crate `cargo tree`
    resolves for any release target with the release build's features is
    not in it, or when an entry names a crate `Cargo.lock` lacks; the extra
    entries are reported and allowed (over-attribution harms no one), and
    the dialog leaves out the entries whose repository is Rick's own. The
    `copyright` of an entry is the crate's authors from Cargo's metadata,
    an array of strings, Cargo having no copyright field; the dialog shows
    the first "Copyright" line of the licence text when there is one.
    Marking the workspace crates `publish = false` would take them out of
    the generated file at the source.

262. (issue 101, 2026-11-09) *About dialog*, Credits card: "fixed rows first
    — Serge …; Claude (Anthropic) … — then the GitHub contributors appended
    at build time (`git shortlog -sn --no-merges` …), each with the role
    from `CONTRIBUTORS.md` when a line matches the name". The shortlog of
    this repository names "Claude", "Serge" and "Serge Berwert", the two
    people of the fixed rows, and the spec does not say how a line of
    `CONTRIBUTORS.md` (which does not exist yet) matches a name.
    Assumed: a contributor whose first word is a fixed row's first word
    (case aside) is that person and is not repeated; a line matches when,
    after a list marker, it starts with the name, and what follows a dash,
    a colon or a comma is the role. Two more small calls of this item: the
    OS and desktop line needs the operating system's version, so a
    `platform::os_description()` was added to the platform crate rather
    than a `cfg(target_os)` elsewhere; and the "medium" weight of the
    frames is drawn in the system font's one weight, egui having no
    medium face.


263. (issue 101, 2026-11-09) *About dialog*, Third-party licences page: the
    footer reads "Generated at build time from Cargo.lock by cargo-about ·
    N components, M licence types", and a build without the generated file
    "shows the Third-party licences row as 'not available in this build'
    and no count; the Bundled runtimes section still shows" — but not what
    the page's footer says then, nor whether the plugins' entries count
    among the N components.
    Assumed: without the file the footer says the crates' list is not
    available in this build and gives the counts of what is shown; the
    plugins' entries count as components and their licence ids as types,
    on the About page's row too; *Export…* writes every section shown,
    the plugins' included, whatever the search and the filter show.

264. (system integration, 2026-11-09) §4: "`rick [<path>…]` (no argument:
    the current directory …)". The desktop entry runs `rick %F`, so a
    launcher click without a selection is also a launch with no argument,
    with the launcher's working directory (usually the home) — before the
    single instance it restored the session as it was, now the spec's
    reading adds a tab at that directory to the restored session, and a
    second click while Rick runs opens a home tab in the running window.
    Assumed: the spec's reading, literally, for every entry point, since
    the binary cannot tell a terminal from a launcher and the spec names
    no flag that would; if the launcher should only raise the window, the
    desktop entry needs an argument form of its own, Serge's call.

265. (system integration, 2026-11-09) §4: "`rick://<path>` URL scheme …
    `rick:///home/serge/Apps` opens the folder … anything that is not an
    existing local path shows an error toast and opens nothing." Rick's
    own virtual locations are spelled `rick://apps`, `rick://trash` (the
    `rick` provider of `vfs-rick`), so a link `rick://apps` reads as the
    Installed Apps view to anyone who knows them, while the spec's grammar
    has an empty host and a local path only.
    Assumed: the spec's grammar, literally — a `rick://` URL with a host
    names no path on this computer, so it shows the error toast and opens
    nothing; mapping `rick://<view>` to the virtual provider's view would
    be one more arm in the binary's `apply` if wanted.

266. (system integration, 2026-11-09) §4: "the console reports its working
    directory through OSC 7". No shell reports it to a terminal it does
    not know: bash and zsh do so through the distributions' `vte.sh` and
    fish by itself, and each checks the terminal by `VTE_VERSION`.
    Assumed: the console's session sets `VTE_VERSION` (to a recent VTE's,
    7800) in the shell's environment, so the three shells report the
    directory where that script or that check is in place, and a shell
    without either reports nothing and the tab stays where it was; the
    emulator ignores the other sequences the variable may invite (OSC 777
    notifications).

267. (system integration, 2026-11-09) §4: "Rick emits `cd` on tab
    navigation only while the console has no foreground job". Linux and
    macOS say whether a job has the terminal (`/proc/<pid>/stat`,
    `proc_pidinfo`); a Windows pseudo console has no foreground process
    group to ask about.
    Assumed: where the system cannot say, Rick does not type into the
    console, so on Windows the tab follows the console (OSC 7, where the
    shell sends it) but the console does not follow the tab, rather than
    risk typing into a running program; a Windows answer (the console
    host's process list) would lift that.

268. (issue 80, 2026-11-09) Rule 4 has a workspace tab spring-load after
    "~500 ms"; *Spring-loading* says "about a second (settable)", and the
    one setting behind it (`drag.spring_delay_ms`, Settings → General)
    defaults to 1000 ms.
    Assumed: one delay for every kind of drag — entries, a pane, a tab —
    and it is the existing setting; its default stays 1000 ms until this
    is answered. If 500 ms is meant, is it the new default for folders
    too?

269. (issue 80, 2026-11-09) Rule 1: "neighbouring tabs slide apart
    (animated)" names no duration, and nothing says what the slide does
    under the system's reduce-motion setting.
    Assumed: 150 ms, eased out, one constant; with reduce motion the tabs
    take their places at once, as the portal animations stand still.

270. (issue 80, 2026-11-09) Rules 2 and 3 turn on whether a tab holds
    "exactly one pane"; the GUI chapter calls the preview "a pane like any
    other", and the default layout is one explorer pane with the preview
    beside it.
    Assumed: every pane counts — the preview, a console and a comparison
    included — so it is the model's count of the workspace's panes, and
    the default tab with its preview is a multi-pane tab: it reorders and
    tears off, and its explorer pane moves by its handle.

271. (issue 80, 2026-11-09) Rule 2: a single-pane tab "can be dragged down
    into the current workspace". Today a tab is activated by a click, not
    by the press that starts a drag.
    Assumed: it stays so. Dragging a tab never activates it, the window
    goes on showing the workspace it showed, and an inactive single-pane
    tab docks into that workspace directly. The dragged tab's own pane
    shows no targets, so the shown tab, when it holds one pane, docks
    elsewhere only through spring-loading (rule 4) or in another window
    (rule 6).

272. (issue 80, 2026-11-09) Rule 4 lets "a single-pane tab" spring-load by
    "hovering another workspace tab"; rule 1 keeps a dragged tab in the row
    under the cursor, so the cursor is never seen on another tab, and with
    Chrome-style sliding a neighbour swaps away once the tab is halfway
    over it.
    Assumed: a dragged tab rests on a neighbour while it overlaps it by a
    quarter of a tab's width or more, short of the half where the two
    swap; held there for the delay it spring-loads that neighbour's
    workspace, and held squarely in its gap it spring-loads nothing, so a
    pause while reordering does not switch the window. A dragged pane
    rests on a tab while the cursor is on the tab. Written into *Workspace
    tabs*, Spring-loaded tabs.

273. (issue 80, 2026-11-09) Rule 7 gives the pane's ghost its scale and
    opacity and says of a tab only "the tab itself, lifted with a shadow";
    rule 1 has the tab "follow the cursor along the row", rule 7 anchors
    every ghost 8 px down and right of the cursor.
    Assumed: the tab's ghost is full size and opaque, with the pane
    ghost's shadow. While the cursor is in a title row — its own window's
    or another's — the tab rides that row, on its line, the grab point
    under the cursor with no offset; once the cursor leaves the title row
    the tab follows it freely, the grab point 8 px down and right.

274. (issue 80, 2026-11-09) Rule 3: a multi-pane tab "can only be
    reordered in the row or torn off into its own window". Today a tab
    released anywhere inside its window — over the panes, over the side
    panel — is reordered to the gap it last showed.
    Assumed: only a release on a tab row reorders and only one outside
    every window tears off; a tab of either kind released anywhere else
    inside a window — a single-pane one beside the dock targets too —
    moves nothing and returns to its place. Rule 6 adds another window's
    tab row to what a multi-pane tab can do.

275. (issue 80, 2026-11-09) Rule 6: a tab or pane "dragged onto the tab bar
    of another Rick window joins that window (as a tab, or via
    spring-loading onto a dock target)". Whether the panes the other
    window already shows take the drop directly is not said.
    Assumed: they do. The dock targets appear on the pane under the cursor
    in whichever window of this Rick the cursor is over, for a pane and
    for a single-pane tab; the tab row is one way in, not the only one.
    Written into *Windows*.

276. (issue 80, 2026-11-09) Rule 9: Escape "restores the original layout";
    *Spring-loading* leaves a pane where it drilled to when a drag "ends or
    is cancelled", by the *After a drag* setting, and question 65 decided
    that its "return" also switches a spring-loaded tab back.
    Assumed: Escape returns every window to the workspace it showed when
    the drag began, for a drag of entries too and whatever *After a drag*
    says; the setting still decides the folders the panes drilled into,
    and the workspace only when a drag of entries ends in a drop. A pane
    or tab drag knows no *After a drag*: dropped, it leaves each window on
    the workspace it shows, the one docked into; released where nothing
    moves, it ends as Escape does.

277. (issue 80, 2026-11-09) Rule 8: "open hand on hovering a grip or a
    tab", "no arrow during a drag". A tab carries a close button, the row
    a new-tab button, and a drag of entries has its own ghost, indicator
    and the not-allowed cursor over the side panel.
    Assumed: the open hand shows over a tab's body; its close button and
    the new-tab button keep the cursor they have. The closed hand stays
    for the whole of a pane or tab drag, also where a release would move
    nothing. Rule 8 does not touch a drag of entries.

278. (system integration, issue 120, 2026-11-09) §6.1: "Rick compares the
    wanted state with the actual one, item by item" and "*Install to the
    standard location*, which moves Rick to `~/.local/opt/rick` … through
    `ext-installer`". The check and the dialog are the core's, the
    installer is a plugin that may be switched off or removed, and a core
    crate may not depend on a plugin.
    Assumed (as the handoff of issue 120 sets out): the core owns the item
    list, the check and the dialog. `platform::Integration` lists and
    checks every item, the install location among them (set up when the
    running binary is inside a version folder of `~/.local/opt/rick`);
    performing the install location is the plugin's registered command,
    the platform answers *unsupported* for it, and without the plugin the
    row reads *Unsupported* with the reason while every other row works
    against the folder Rick runs from.

279. (system integration, issue 120, 2026-11-09) §6.1: "The *Install for*
    segmented control (Just me / All users) at the top of the dialog sets
    the scope for every row", with `/opt/rick` and `pkexec` "and the
    installer's helper" for All users. That helper does not exist
    (specs/ext-installer.md: "its own milestone"; `install()` refuses the
    system scope).
    Assumed (handoff of issue 120): this issue delivers *Just me*
    completely. For *All users* the Linux items answer *unsupported* (not
    available yet) and the dialog shows the segment disabled with the
    tooltip *Not available yet*; the helper is not built here.

280. (system integration, issue 120, 2026-11-09) §6.1: "The check reads
    files and registry keys only" — but not what *set up* is for each
    item of §7.1.
    Assumed, for *Just me*: the **install location** when the running
    binary is inside a version folder of `~/.local/opt/rick`; the
    **launcher entry** when `$XDG_DATA_HOME/applications/rick.desktop` is
    there with an `Exec` that starts this Rick and every hicolor icon
    file is there; the **default file manager** when `rick.desktop` is
    the first id of `inode/directory` and of `x-scheme-handler/rick`
    under `[Default Applications]` of `$XDG_CONFIG_HOME/mimeapps.list`,
    the file `xdg-mime default` writes; **"Show in folder"** when
    `$XDG_DATA_HOME/dbus-1/services/org.freedesktop.FileManager1.service`
    names that bus name and starts this Rick; the **`rick` command** when
    `~/.local/bin/rick` is a link to this Rick and `path.sh` defines the
    `rick` function for it; the **dock favourite** when the desktop's
    favourites name `rick.desktop` (`rick` on COSMIC). "This Rick" is the
    running binary, named through the `current` link when it runs from
    the install location, so an update that flips the link takes every
    item with it.

281. (system integration, issue 120, 2026-11-09) §7.1: "`packaging/linux/
    rick.desktop` and the icons installed under `share/applications` and
    `share/icons/hicolor` of the chosen scope" — from where: the release
    archive has `share/` beside the binary, a development build has not,
    and an AppImage or a package will not have it beside the binary
    either.
    Assumed: the binary carries both, the desktop entry and the ten
    hicolor files compiled into `platform`, so the item does the same
    from every build; the archive's `share/` stays what a manual install
    copies (specs/app-icons.md).

282. (system integration, issue 120, 2026-11-09) §7.1, dock favourite:
    "Other desktops: *Unsupported*" — but not what the item is on one of
    the four desktops whose means are not there: no `gsettings` program,
    a `gsettings` without the desktop's schema, no Plasma panel
    configuration or no task manager in it, no favourites file of
    COSMIC's dock.
    Assumed: *Unsupported* with the reason, not *missing*, which would
    list a row in the dialog whose Apply could only fail. COSMIC's
    favourites are read from `~/.config/cosmic/
    com.system76.CosmicAppList/v1/favorites`, the list its dock keeps.

283. (system integration, issue 120, 2026-11-09) §6.1: "one record per
    integration item (§7): `state = "set-up" | "declined" | "unsupported"
    | "missing"` and the date of the last check". Three things the record
    does not settle: where what Rick wrote for an item is kept, so it can
    be undone (§7.2 keeps the previous values of *Make Rick the default*
    "in `rick.toml`"); how *drifted* is remembered between starts, there
    being no fifth state; and how fine the date is — a time would rewrite
    `rick.toml` at every start.
    Assumed: the record is `[integration.items.<item>]` with `state`,
    `checked`, and, when Rick wrote something, `files` (the files and
    links created, `~` for the home) and `previous` (the values replaced,
    as they were). A drifted item's record keeps `state = "set-up"`:
    drift is a set-up record meeting an item the check finds missing, at
    this start and every later one until it is restored or switched off.
    `checked` is the calendar day in UTC, so checks change the file at
    most once a day; the time on Settings' status line is the running
    session's.

284. (system integration, issue 120, 2026-11-09) §6.1: "the preference
    switches (shell integration, *Check integration at startup*)" and
    "`[integration] ask = false`" — the two switches have no key, and the
    spec does not say what each leaves running.
    Assumed: `[integration] check_at_startup` and `shell_integration`,
    both `true` by default, beside `ask`. With `ask = false` the startup
    check still runs, so Settings shows drift, and only the dialog stays
    closed; with `check_at_startup = false` nothing is checked until
    *Check again* (or Settings → Integration is opened). With
    `shell_integration = false` the embedded console and its tab stop
    following each other, both ways, and switching it on again catches up
    on nothing.

285. (system integration, issue 120, 2026-11-09) §6.1: "**Reset choices**,
    which clears every declined mark so the dialog asks once more", while
    the Figma notes say of the per-row switch and of the footer checkbox
    "Both are reversible in Settings → Integration (“Reset choices”)".
    Assumed: *Reset choices* turns every declined record into *missing*
    and sets `ask = true` as well, so the dialog does ask at the next
    start whichever way it was silenced.

286. (system integration, issue 120, 2026-11-09) §6.1: "**Apply N items**
    performs the switched-on rows"; only the dock is said to be one whose
    failure "never blocks the rest". What a failing row of any other item
    does to the rows after it, and what its record becomes, is not said.
    Assumed: every row is performed for itself, in the dialog's order; a
    row that fails is reported (a warning for a *Best effort* item, an
    error otherwise), the rows after it are still performed, and its
    record stays as it was — *missing*, so the next start asks again, or
    *set-up* for a drifted one. Nothing is retried by itself.

287. (system integration, issue 120, 2026-11-09) §6.1 has "every item with
    its own switch" and the installer's spec has "Rick never overwrites
    silently" for its own collisions, but the integration spec does not
    say what a row does when a `rick.desktop`, a `rick` link or a
    `org.freedesktop.FileManager1.service` is already there from
    somewhere else.
    Assumed (handoff of issue 120: "never overwritten silently: the row
    says so and asks"): Rick tells its own files by the item's record —
    a file the record lists is Rick's and is rewritten freely; a file
    that is there, is not listed and does not already do what Rick's
    would (the entry or the service file starts another program, the
    link points elsewhere, an icon has other contents) is *in the way*.
    The check reports it, the row says so, and performing the item
    refuses with *exists and was not written by Rick* unless the user
    agreed to replace it; once replaced it is on record as Rick's. A
    file that already is what Rick would write is left as it is and
    stays off the record, so an undo never removes what Rick did not
    make.

288. (system integration, issue 120, 2026-11-09) §7.1: "**Default file
    manager** — `xdg-mime default rick.desktop inode/directory` and
    `x-scheme-handler/rick` for the scope's `mimeapps.list`". The spec
    names the command; a test may not run it against the real system,
    and an undo needs what the two types named before.
    Assumed: Rick edits `$XDG_CONFIG_HOME/mimeapps.list` itself, to what
    `xdg-mime default` leaves — each of the two keys under `[Default
    Applications]` set to `rick.desktop`, every other line untouched,
    the group or the file created when missing — and records each key's
    previous value. The undo puts a previous value back (or removes the
    key) only where the type still names Rick, and removes a file Rick
    created once it names nothing. The item refuses while no
    `rick.desktop` is where the desktop looks for it: a default that
    names no application would open folders with nothing.

289. (system integration, issue 120, 2026-11-09) §7.1: "**`rick` command**
    — a symlink in `~/.local/bin` …, plus the shell function of §4 in
    Rick's `path.sh` (the one the installer plugin sources from the
    profile)". The installer regenerates `path.sh` from its install
    records and writes the function only when Rick is among them; a Rick
    that runs from where it was unpacked has no record, the plugin may
    not be loaded, and a regeneration would drop a line it does not
    know.
    Assumed: the item writes the function itself, as a block between two
    marker lines (`# >>> rick command >>>` … `# <<< rick command <<<`) in
    `path.sh` and, where `fish` is configured, in its drop-in, and adds
    the installer's own profile line (same text, same marker) where it is
    missing. The installer carries the block over when it regenerates
    either file, in place of its own function, and keeps a file alive
    that only the block still needs. A `path.sh` that already defines the
    function for this Rick gets no second one. The undo removes the
    block, the files once they say nothing else, and the profile line
    Rick added once `path.sh` is gone.

290. (system integration, issue 120, 2026-11-09) §7.1: "`update-desktop-
    database` and `gtk-update-icon-cache` run when present" — what a
    failure of either means for the launcher entry is not said.
    Assumed: nothing. Each is looked up on the `PATH` and run after the
    entry and the icons are written or removed; a tool that is missing
    or fails (a user's hicolor folder has no theme index) is logged at
    debug level and the item is still set up, since the desktops read
    the entry and the icons without the caches.

291. (issue 80, rule 7, 2026-11-09) *Drag ghost*: "a real snapshot of the
    grabbed pane … drawn at 70 % opacity". A picture of the frame would
    give one even 70 %, but the backend delivers a screenshot for its
    root viewport only (eframe 0.36 paints an immediate viewport with no
    screenshot request), and every Rick window beyond the first is such a
    viewport.
    Assumed: the snapshot is the pane's own shapes, kept from the frame it
    was grabbed in and painted again at a quarter, each shape at 70 %. It
    is the same in every window and needs no frame of delay; where shapes
    lie over one another — a name on a selected row — a little less than
    30 % of what is under the ghost shows through, and the snapshot's
    corners are cut square under the rounded border. A paint callback in
    the pane is left out of its ghost.

292. (issue 80, rule 7, 2026-11-09) *Drag ghost*: the ghost "moves rigidly
    with the cursor". A window paints inside itself only: once the cursor
    leaves the window a pane or a tab was dragged from, nothing of Rick
    is under it to paint the ghost on, unless it is over another Rick
    window.
    Assumed for now: the ghost shows while the cursor is inside the
    window the drag began in, and the closed hand alone follows the
    cursor outside it; another window painting the ghost, and what shows
    over the desktop, are part of rule 6 and are asked there.

293. (system integration, issue 120, 2026-11-09) §6.1 and §7.1: the
    install location goes "through `ext-installer`", whose `install()`
    takes an archive; a started Rick is an unpacked folder and the
    archive may be gone.
    Assumed (handoff of issue 120): the installer gains a source, an
    unpacked folder, read as one more archive format (`Format::Folder`)
    so the index, the safety rules, the staging folder renamed into
    place, the record and the `current` link are the archive path's own;
    the folder is never packed first. The tree is the folder that holds
    the running binary, with its libraries, `plugins/` and `share/`. The
    install is `~/.local/opt/rick/<version>` with the version the binary
    reports (`0.3.0-dev` in a development build), the app id `rick`, and
    nothing written outside the app folder: the launcher entry, the
    icons and the `rick` command are items with switches of their own.

294. (system integration, issue 120, 2026-11-09) §4 has plugins register
    subcommands and §6.1 has the core perform the install location
    through the plugin, but no command for it is named.
    Assumed: the installer declares `install-rick <folder> <version>`
    beside `install <archive>`: no form, one job in the operations area,
    an update when a Rick is installed already, one previous version
    kept. The core runs it with the folder the check reported and its
    own version, waits for the job, and only then performs the other
    rows, on the installed copy; a failed install stops them, since they
    would otherwise point at the folder Rick was meant to leave. The
    command is listed by `rick --help` like any other.

295. (system integration, issue 120, 2026-11-09) §6.1: "which moves Rick
    to `~/.local/opt/rick` … and relaunches from there"; the Figma row
    reads "Move Rick from ~/Downloads/rick-0.3.0 to ~/.local/opt/rick".
    The process that performs the row runs from the folder it would
    move, and the installer never deletes what it installs from.
    Assumed: Rick is copied, and the folder it was started from is left
    for the user to remove; the row's line says *Copy*. Starting the old
    folder again offers the row again, as an update of the installed
    copy.

296. (system integration, issue 120, 2026-11-09) §6.1: "relaunches from
    there", and §3: one Rick per user session, a launch that finds an
    instance hands over and exits. How the running Rick gives way to the
    installed one is not said.
    Assumed (handoff of issue 120: "the old process exits after the new
    one answers"): the session is saved; the old process stops serving
    and unbinds the endpoint, starts `current/rick` with its own
    `--config-dir` if it had one, and asks the endpoint for `--help`
    every 100 ms until the new instance answers, then exits. If nothing
    answers within 20 seconds it binds the endpoint again, stays, and
    shows *Rick could not be started from …*. Until the relaunch the
    installed copy stands for the running Rick in the checks, so the
    rows read *Set up*.

297. (system integration, issue 120, 2026-11-09) §7.1: "The running
    instance owns the name; activation starts Rick when it is not
    running." Whether a Rick whose user switched the row off also takes
    `org.freedesktop.FileManager1`, and with it "Show in folder" from the
    desktop's own file manager while it runs, is not said.
    Assumed: no. Rick asks for the name when the item is on record as
    *set up* at its start, when the session bus itself started it, and
    from the moment a later launch hands over the activation argument.
    A row set up in a running Rick therefore takes effect at the first
    call — the bus starts `rick --dbus-service`, that launch hands over
    to the running instance, which takes the name and answers — or at
    the next start. The check looks at the activation file only, never
    at who owns the name.

298. (system integration, issue 120, 2026-11-09) §7.1: "a service file
    for D-Bus activation"; §4 lists the core command line and has
    nothing the activation could run without opening the folder the bus
    happens to start it in.
    Assumed: one more flag, `rick --dbus-service`, which is what the
    file's `Exec` runs. Started with it, Rick restores its session,
    opens no tab for the working directory and takes the name; handed to
    a running instance it opens nothing and makes that instance take the
    name. `--help` lists it. After the file is written or removed Rick
    asks the session bus to read its services again (`dbus-send …
    ReloadConfig`, when there is a bus and the tool); a failure is
    logged and the item is set up by the file all the same.

299. (system integration, issue 120, 2026-11-09) §7.1: "The running
    instance owns the name" — another file manager may own it already
    when Rick starts.
    Assumed: Rick does not take the name away and does not give up: it
    waits in the bus's queue and has the name when the other lets go;
    once it has the name it keeps it while it runs.

300. (system integration, issue 120, 2026-11-09) §4: "`ShowItemProperties`
    to open plus the Properties dialog". The call takes a list of
    items; the Properties dialog shows one entry.
    Assumed: every item is revealed and selected as for `ShowItems`, and
    Properties opens on the first folder's selection once it is listed,
    as the Properties command does for a selection of several. For all
    three calls a URI that is not a `file://` path of this computer is
    left out, and the caller's startup id is the activation token the
    window is raised with.

301. (system integration, issue 120, 2026-11-09) §7.1, dock favourite:
    the spec names where each desktop keeps its favourites, not what
    Rick writes there or what the undo puts back.
    Assumed: Rick is appended at the end of the list and nothing else in
    it changes; a list that has Rick already is left alone. The record
    names the list and keeps it as it was, and the undo takes Rick out
    of the list as it is by then rather than writing the old list back,
    so favourites pinned or removed since stay. A favourite names the
    launcher entry, so the row fails with a warning while that entry is
    not installed, as the default file manager does.

302. (system integration, issue 120, 2026-11-09) §7.1: "Plasma: the
    task-manager launcher list in
    `plasma-org.kde.plasma.desktop-appletsrc`". A panel may hold several
    task managers; one whose configuration names no launchers shows
    Plasma's defaults, which a list in the file replaces; and the
    running shell reads the file at its start only.
    Assumed: the first task manager of the file gets Rick. Where it
    names no launchers, Plasma's own default list (System Settings,
    Discover, the preferred file manager and browser) is written out
    with Rick after it, and the undo removes the key again when only
    that default is left. After the write Rick asks the running shell to
    start over from its configuration (`org.kde.PlasmaShell.
    refreshCurrentShell` through `dbus-send`, when there is a session
    bus and the tool): the panel restarts once. Without that call the
    favourite appears at the next login. Say so if Rick should not
    restart the shell.

303. (system integration, issue 120, 2026-11-09) §7.1: "COSMIC: the
    dock's configuration under `~/.config/cosmic`" (question 282 has the
    file). The file is a RON list that COSMIC's own configuration
    library writes and watches.
    Assumed: `rick` is appended and the list written back the way that
    library writes it, an id a line; the dock picks the change up by
    itself. A COSMIC that has not written the file yet stays
    *Unsupported*, as in question 282: Rick does not create it, since a
    list of its own would replace the dock's defaults.

304. (system integration, issue 120, 2026-11-09) §6.1: "each row with its
    own switch (on by default for missing, on for drifted)"; the Figma
    frame *Integration dialog — first run* draws the *Dock favourite*
    switch off.
    Assumed: the spec wins. Every listed row starts switched on, the
    dock favourite included; the frame's "Apply 5 items" is that one
    switched off by hand.

305. (system integration, issue 120, 2026-11-09) Neither the spec nor
    the frame shows a row whose place holds a file Rick did not write
    (question 287).
    Assumed (handoff of issue 120: "the row says so and asks"): under
    its line the row names the file in `status/warning` — *~/.local/bin/
    rick is already there and was not written by Rick* — with a
    *Replace* checkbox. Ticked, the row is set up and the file replaced.
    Not ticked, a row that is on is neither set up nor declined: it is
    left out of *Apply N items* and asked about again at the next start.
    Switched off it is declined like any other.

306. (system integration, issue 120, 2026-11-09) §6.1: "**Not now**
    closes without changes and asks again at the next start; the footer
    checkbox **Don't ask again** silences the whole dialog". Which
    button the checkbox goes with, and what *Apply* does with every row
    off, is not said.
    Assumed: the checkbox is stored with whichever button closes the
    dialog, so *Don't ask again* with *Not now* stores `ask = false` and
    nothing else. Escape and a click beside the dialog are *Not now*.
    With every row off *Apply* reads *Apply 0 items*, stores the rows as
    declined and closes. While the rows are set up the footer shows
    *Setting up <row> — n of N* in place of the checkbox and the
    controls rest; the dialog closes with the last row, and a row that
    failed is a toast.

307. (system integration, issue 120, 2026-11-09) The frame's lines are
    worded for one machine: "so Rick appears in the Activities
    overview", "instead of Files", "Pin Rick to the GNOME dash", "Move
    Rick from …".
    Assumed: the lines are worded from what the check found. The
    launcher entry says *Activities overview* on GNOME and *application
    launcher* elsewhere; the default file manager names the application
    that opens folders now, or leaves "instead of …" out when there is
    none; the `rick` command names the folder its link goes to; the dock
    row names the desktop's own place (GNOME dash, Plasma task manager,
    COSMIC dock, the favourites of the Cinnamon menu); the install row
    says *Copy* (question 295). *All users* is drawn and cannot be
    chosen, with the tooltip *Not available yet*; the frame's line about
    passwords is kept as it is.

308. (issue 80, rule 4, 2026-11-09) *Spring-loaded tabs*: a dragged tab
    "rests on a neighbour while it overlaps it by a quarter of a tab or
    more, short of the half where the two would swap". A tab taken past a
    neighbour overlaps it by a quarter or more for half the way — before
    the swap from one side, after it from the other — so at an unhurried
    pace an ordinary reorder would outlast the delay and switch the
    window.
    Assumed: "rests" is taken at its word. A dragged tab spring-loads the
    neighbour it leans on only while it is held still, the cursor within
    6 px (`ui_egui::drag::REST_SLOP`) of where the rest began; moving on
    starts the clock again. The overlap counts on either side of the
    swap. A dragged pane needs no such rule: its cursor is on the tab or
    it is not.

309. (issue 80, rule 9, 2026-11-09) *No-ops and cancel*: Escape leaves
    "every window showing the workspace it showed". A drag of entries that
    spring-loaded through two tabs records each switch, and *After a
    drag: return* undoes them in the order they happened, which ends on
    the second-to-last workspace, not the first.
    Assumed: a cancelled drag undoes the switches last first, so the
    window ends on the workspace the drag began in. The order for a
    dropped drag with *return* is left as it is: it is not part of issue
    80, and is noted for the maintainer.

310. (issue 80, rule 6, 2026-11-09) *Windows*: "the dock targets appear on
    the pane under the cursor in whichever window that is". While a
    button is down the pointer belongs to the window the drag began in;
    another window hears nothing of the drag, so the first has to say
    where the cursor is, which takes each window's place on the screen. A
    Wayland compositor does not tell a window its place.
    Assumed: the window the drag began in publishes the drag with the
    cursor's place on the screen, and the window under the cursor shows
    the gap, the slot, the targets and the ghost and says where a drop
    lands. Where windows overlap, the one the drag began in counts as in
    front, having been clicked last. Where the system gives no positions
    (Wayland), a drag knows its own window alone: nothing shows over
    another window, and a release outside the window does nothing, as
    before this issue. Reaching another window there takes the
    compositor's own drag and drop with a payload of Rick's; not built,
    noted for the maintainer.

311. (issue 80, rules 6 and 7, 2026-11-09) *Drag ghost*: the ghost "moves
    rigidly with" the cursor; question 292 left open what paints it
    outside the window the drag began in.
    Assumed: the window the cursor is over paints the ghost, a tab from
    the tab's own label, a pane from the snapshot its window took.
    Between windows, over the desktop, no window can paint: the closed
    hand alone shows there. A borderless, transparent window of its own
    that follows the cursor would carry the ghost everywhere; not built.

312. (issue 80, rule 6, 2026-11-09) *Panes*: "the centre, or the tab row,
    makes the dragged pane a new workspace tab", and "another window
    takes the pane on its tab row or on its dock targets". Where in the
    other window's row a pane dropped on a centre target becomes a tab is
    not said.
    Assumed: at the end of that window's row, shown; dropped on the row
    itself, at the slot the row shows.

313. (system integration, issue 120, 2026-11-09) The Figma frame
    *Settings — Integration* has the line "Fedora Linux 43 · GNOME 49 ·
    checked at start, 14:02" and, under *Terminal and links*, a row
    "rick:// links" with a *Set up* pill.
    Assumed: the line is the system and the desktop as the platform
    names them, without the desktop's version, then *checked at start,
    HH:MM* in local time, or *checked HH:MM* after **Check again**. The
    `rick://` row is left out: §7.1 has no such item, the scheme is
    registered by the desktop entry and made Rick's by the default file
    manager item (`x-scheme-handler/rick`), whose line says so.

314. (system integration, issue 120, 2026-11-09) The frame's lines
    "Reset by a system update on 8 Nov" and "Skipped at first run" say
    more than Rick knows: the check finds an item it set up missing, not
    who changed it or when, and a row can be switched off at any start.
    Assumed: a reset item reads *Was set up by Rick and has been changed
    or removed since*; a skipped one *Skipped in the setup dialog on
    <day>*, the day being the one its record was last touched, which
    for a declined item is the day it was switched off. A set-up item
    says where it is; an unsupported one says why.

315. (system integration, issue 120, 2026-11-09) §6.1 names four pills.
    An item can also be neither set up nor declined: after *Not now*,
    with the dialog silenced, or after *Reset choices*.
    Assumed: such a row has a grey *Not set up* pill, the line the
    dialog shows for it, and **Add**. A row with a file of another
    origin in its way names the file and its button reads *Replace*,
    which is the user's word to replace it (question 305). The buttons
    rest while a check or an apply runs. *Reset choices* is there to
    press when a row is declined or the dialog is silenced, and undoes
    both. Opening the page before any check ran asks for one.

316. (system integration, tier 2 on Windows and macOS, 2026-11-10) §4 says
    paths "are resolved against the caller's working directory before they
    are sent" and nothing about their form. `cli::resolve` makes a path
    that exists canonical, so a launcher's relative path and a symlink
    reach the thing itself; a `rick://` URL's path went through as spelled.
    On macOS the temporary directory is a symlink (`/var` → `/private/var`)
    and on Windows `canonicalize` returns the verbatim `\\?\C:\…` form, so
    the two forms differ there and the tests that compared them failed.
    Assumed: a `rick://` path is resolved exactly like a command-line path
    (canonical where it names something), and the tests expect the
    canonical form. Open: whether a tab's location should carry the
    `\\?\` prefix on Windows at all, or be simplified to `C:\…` where
    that is the same path; the path bar shows the location as stored.

317. (preview pane, tier 2 on macOS, 2026-11-10) A start with a path
    (`rick <path>`, or a test's `start` location) restores the last session
    and opens the path as a workspace of its own, which is the one shown
    (SPEC.md *Workspace tabs*). A Preview tab the session had stays in the
    restored workspace, out of sight, until the toggle, which "closes
    there and opens here" (`Model::open_preview`). The acceptance tests
    took "the window has a Preview tab" for "the Preview tab is shown"
    and, when the first window's session had been saved in time (only on
    a loaded runner), never opened it: the exif badges test's "a.jpg: no
    frame" on every platform. Assumed: that is the intended behaviour, and
    the test accessor `inspector_open` means shown; whether a start with a
    path should instead carry the Preview tab over into the new workspace
    is open.

318. (0.3.0 release, 2026-11-10) SPEC.md's *Version 0.3* says the two
    tracks "ship in one release", and the integration track has ten
    milestones (specs/system-integration.md §12). Milestones 1 and 3 are
    merged; 2 and 4 to 10 are not: clipboard interoperability, the Windows
    and macOS integration items, notifications and progress, signing and
    plugin security, and the three distribution channels. Serge asked for
    the release on 2026-11-10 with `main` as it stood, and issues 139 and
    149 start "once 0.3.0 is tagged".
    Assumed: 0.3.0 ships what is merged and the remaining milestones move
    to a later release. SPEC.md is not edited: its scope chapter, §12 of
    the integration spec and a "Released" paragraph like 0.1's are Serge's
    to write. The README says what is not in the release yet.

319. (0.3.0 release, 2026-11-10) The date of CHANGELOG.md's release
    heading, as question 213 for 0.2.0.
    Assumed: `## [0.3.0] - 2026-11-10`, the day of preparation, unless the
    tag is made on a later day and Serge asks for that day.

320. (0.3.0 release, 2026-11-10) The Unreleased section had no line for
    the About dialog (issue 101, #105, #107, #109) and none for the fixes
    the first tier 2 runs of the batch brought (#144, #145, #146, #148),
    and the section is the release body and the text of *What's new*.
    Assumed: the lines are added with the release, written from the pull
    requests' own descriptions, and the section gets the one-paragraph
    summary of question 214. The pull requests that changed tests, CI or
    specs alone (#143, #147, #150, #151 among them) get no line.

321. (0.3.0 release, 2026-11-10) After the version bump `0.3.0-dev` is
    still named in the fixtures of `ui_egui::about` and
    `ui_egui::licences`, of `crates/tests/tests/about.rs` and
    `screenshots.rs`, in the changelog's own line and in questions 223 and
    the About dialog's.
    Assumed: all are example data or records and stay, as question 216;
    the unit test that reads the repository's changelog asks for the
    section of a `-dev` version, which is still Unreleased.

322. (0.3.0 release, 2026-11-10) The README said the archives are "not
    signed or notarised yet (planned for 0.3)" and unpacked the macOS
    archive with `tar` into a folder with a `rick` binary; BENCHMARKS.md
    describes the benchmarks of 0.2.
    Assumed: the parenthesis goes, since signing is milestones 7, 9 and
    10 and no version is promised in its place; the macOS lines unpack
    the zip with `ditto` and open `Rick.app`, as the release workflow's
    smoke test does; BENCHMARKS.md stays, the `bench` workflow measuring
    again on the tag.

323. (GUI fixes, issue 139, 2026-11-10) The issue starts "once 0.3.0 is
    tagged and `dev` is level with `main` again", and the handoff asks for
    one chore pull request that sets the workspace version to `0.4.0-dev`
    first. `v0.3.0` is `main`'s head (`025bae1`); `dev` is `main` plus
    #154, which is that version bump, merged on 2026-11-10.
    Assumed: the preconditions hold and the bump is not made a second
    time.

324. (GUI fixes, issue 139, 2026-11-10) CLAUDE.md says "Do not change
    SPEC.md yourself"; item 1 of the issue asks for the rules of items 2
    to 11 to be written into SPEC.md, as a pull request that waits for
    Serge's review.
    Assumed: the review is what makes the change Serge's. The agent
    opens the pull request and does not merge it, and builds nothing on
    it before it is merged. `specs/ext-ssh.md` is edited in the same
    pull request where it named the old cap (4 MiB, *Preview anyway*).

325. (GUI fixes, issue 139, 2026-11-10) The Figma section also holds
    *List view mode* (`163:5119`) with its notes; the issue lists the
    layout of the List view as out of scope.
    Assumed: read, and neither specified nor built here; it belongs to
    issue 149.

326. (GUI fixes, app menu, 2026-11-10) The notes say "same pattern as the
    View options and right-click menus: card, 10 px corners, soft shadow,
    32 px rows". SPEC.md's *Look and feel* fixed 30 px menu entries and
    8 px corners on cards and menus. In the Figma file the context menus
    have 30 px entries and the View options menu 32 px, all with 10 px
    corners; the code of 0.3 follows Figma for the heights and SPEC.md for
    the corners. The new frames draw their cards (the two file cards, the
    File info card, the attention card) with 10 px corners too.
    Assumed: the app menu has 32 px entries, as the View options menu
    has, and *Look and feel* now says which menu has which height. The
    8 px corners stay, for the app menu and for the cards of the new
    dialogues, which reuse the menu and card components of the design
    refresh; 10 px, if it is wanted, is one constant for every menu and
    card and a step of its own.

327. (GUI fixes, app menu, 2026-11-10) The frame draws seven entries.
    Nothing says where a second extension's submenu goes, which icon it
    has, whether the entries of the submenus get icons, or whether
    *Rename…* and *Save layout as…* in the Layouts submenu lose their
    "…".
    Assumed: every extension with application-menu actions has its
    submenu in the first group after Layouts, with the icon its group
    has in the context menu and `PUZZLE_PIECE` without one. The submenus
    take the same card and keep their entries and labels: the rule on
    "…" names Settings, Keyboard shortcuts and Extensions. The version
    beside About Rick is the build's, as the About dialog shows it.

328. (GUI fixes, paste conflict, 2026-11-10) "(3 of 12)" and "the
    remaining 9 conflicts" count conflicts, but a job learns of a
    conflict only when it reaches the item; today it reports the items
    left, not the conflicts.
    Assumed: the job compares the top-level names with the destination
    before it brings anything, so the number is known at the first
    question. Names inside a merged folder are neither counted nor asked
    about, as before.

329. (GUI fixes, paste conflict, 2026-11-10) Folder conflicts are "not
    drawn": the notes give *Merge* in place of *Replace* and no *Keep
    both*, and nothing else. A file meeting a folder of the same name is
    not mentioned.
    Assumed: *Merge folder?* with the captions *Existing folder* and *New
    folder*; *Merge* keeps the caution style, since it replaces the
    same-named files inside; Enter is *Skip*, the choice that loses
    nothing, as *Keep both* is for files; a folder's card shows its
    modified time and no size. A file and a folder of one name are asked
    about as two files are, the title and captions naming what each is,
    and *Replace* removes the existing item first as SPEC.md already
    said. A standing *Keep both* does not answer for two folders. The
    close button is *Cancel*.

330. (GUI fixes, properties, 2026-11-10) The frames draw a folder and a
    symbolic link. No frame draws a regular file; none shows the media
    type, which SPEC.md's *Inspect* names; none shows *System
    properties…*, which Windows and macOS have today.
    Assumed: a file's Size group has *Size* and *On disk*; the media type
    is a row of the Location group for a regular file; *System
    properties…* sits at the left of the General tab's footer, where
    *Open target folder* sits for a link.

331. (GUI fixes, properties, 2026-11-10) The folder frame has an Options
    group with a *Hidden* switch. Nothing in Rick sets the hidden state
    today, and on Linux it is the leading dot of the name.
    Assumed: the switch shows the state and is not changed here. Setting
    it is a function of its own — an attribute on Windows and macOS, a
    rename on Linux — and the issue holds visual and interaction changes
    only. The group is not shown for a symbolic link, as drawn.

332. (GUI fixes, properties, 2026-11-10) The Access group draws the
    octal mode as text and "setuid · setgid · sticky: off" beside it;
    0.3 has an octal field that can be edited and three checkboxes.
    Assumed: the octal value stays a field, the special bits are set
    through its first digit, and the text at the right names the bits
    that are on, or reads as drawn when none is. The checkboxes go.

333. (GUI fixes, properties, 2026-11-10) The notes say "Owner/group/
    others ACL rows mirror the Access grid above"; the frame draws
    *Owning group* without Execute while the grid's *Group* row has it.
    Assumed: the frame, which is what POSIX does: with named entries the
    mode's group bits are the mask, so the grid's *Group* row moves with
    the *Mask* row and the owning group has its own. The other kinds of
    ACL (NFSv4, macOS, NTFS) open in the same place with the rows they
    had in 0.3.

334. (GUI fixes, properties, 2026-11-10) *Apply to enclosed items* is
    one switch, "Set the same permissions on everything inside". SPEC.md
    and 0.3 had a separate choice for files and for folders.
    Assumed: the switch applies to both and the separate choice is
    withdrawn; SPEC.md's *Permission management* says so now.

335. (GUI fixes, preview pane, 2026-11-10) One info button and one card
    replace the badges, one per section (File info, EXIF, tags, stream
    details, a decorator's panel). The notes do not say what becomes of
    the sections, of the search field of a long section, or of the pin
    remembered per content kind.
    Assumed: the card lists every section's rows in order, the further
    sections under their titles once it is unfolded; the search field
    goes; the pin is one switch per previewer id, and a pin stored per
    content kind is not carried over. A click on the info button pins
    and unpins as the pin button does, because a popover on hover alone
    cannot be reached from the keyboard.

336. (GUI fixes, preview pane, 2026-11-10) "Rows are the first 4–5
    fields of the previewer": the popover draws five, the pinned card
    four. Both draw *Modified*, which no previewer reports.
    Assumed: five as a popover, four when pinned, as drawn; the pane
    adds the entry's *Modified* after the previewer's *Size* row.

337. (GUI fixes, preview pane, 2026-11-10) *Open as hex*: the hex
    previewer claims no file, and its claiming `.bin` is on the issue's
    out-of-scope list. SPEC.md had *Plugin available* for a type a
    disabled plugin claims. The card for a previewer that failed is not
    drawn.
    Assumed: the button shows this one file with `ext-preview-hex` until
    the selection moves on, and is absent while that plugin is not
    loaded; *plugin available* takes the place of "no previewer for
    .bin" in the head; an error is the card with one row, *Preview*, and
    *Retry*.

338. (GUI fixes, preview pane, 2026-11-10) *Download pdfium*: the
    plugins name either a download address (pdfium) or an install command
    (LibreOffice, the 7-Zip library on Linux), and nothing in Rick
    downloads or unpacks a library.
    Assumed: *Download* opens the address in the browser and Rick fetches
    and installs nothing; where the plugin names a command, the card has
    it as an *Install* row with a copy button and no *Download* button.
    GStreamer, which reports through the media previewer's *Playback*
    row and not as a missing library, is unchanged.
    Decided (2026-11-10): yes, browser only, and the card also shows the
    destination folder with an open-folder button (Serge). SPEC.md's
    *Third-party libraries* has the *Destination* row now. Assumed with
    it: the destination is the per-user plugins folder, which the
    library search already includes and which never needs elevation, as
    the folder beside the binary can; the button creates it when it is
    missing. A library installed with a command keeps its *Install* row
    and has no *Destination*.

339. (GUI fixes, preview tables, 2026-11-10) The notes call the header —
    a 30 px `bg/panel` band with one hairline below and medium titles —
    "the same header as the main file list". The file list's header is
    34 px in `bg/subtle` between two hairlines with regular titles, in
    SPEC.md and in Figma (`116:3485`). The frame draws 30 px rows while
    the notes take the row height from the Density tokens, and in the
    *After* frame the cells do not sit under their resized header cells.
    Assumed: the tables in previews get the header as drawn and the file
    list is not changed; rows are `row/height`; every cell sits under its
    header; both tables are striped whatever the list's zebra setting,
    as both are drawn; widths are kept in `rick.toml` per previewer, the
    CSV grid's by position because its columns differ from file to file.

340. (GUI fixes, remote size cap, 2026-11-10) The notes set the limit at
    10 MiB and put it, and the undoing of *Always*, on *Settings →
    Previews*. SPEC.md and `specs/ext-ssh.md` had 4 MiB, a per-host
    override and the host editor; `ext-ssh` also has a plugin-wide
    `preview_size_cap_mib`; no Previews page is drawn.
    Assumed: 10 MiB, one core setting for every remote provider, on a
    Previews page that also lists the sources that are always previewed;
    the per-host override and the switches in the host or account editor
    and the row's context menu stay; the plugin-wide key of `ext-ssh` is
    no longer read. The progress line shows for every whole-file download
    from a remote source, not only after a button, and Escape cancels the
    download before it clears the filter or the selection.
    Decided (2026-11-10): *Always preview on <host>* must actually
    stick; the limit stays 4 MiB; no settings page and no host editor
    this round (Serge). SPEC.md and `specs/ext-ssh.md` are back at
    4 MiB, the Previews page is taken out again, and the plugin-wide key
    of `ext-ssh` is read as before. What item 7 does: the card, the
    download with its progress line and Escape, and the switch honoured
    by every remote provider and kept across restarts. Assumed with it:
    the switch is undone where SPEC.md already had it, the source row's
    context menu, which therefore has to show it for a cloud account
    too — without a settings page or an editor it is the only way back
    short of editing `rick.toml`.

341. (GUI fixes, keyboard shortcuts, 2026-11-10) The drawn categories
    are All, Navigation, File operations, View, Selection, Panes & tabs,
    Search, then the plugins. 0.3 has *Window* and *Preview pane* and no
    *Search*. The frame files *Compress…* and *Bulk rename…* under File
    operations, though both come from plugins and the issue says plugin
    actions are never filed there. The category icons are stand-ins, two
    of them the same.
    Assumed: the window commands join *Panes & tabs*; *Search* holds the
    search box in both modes, the filter and the search in
    subdirectories; *Preview pane* stays a category, because its chords
    apply only while the preview has the focus and may repeat the list's.
    The rule wins over the frame: *Compress and extract*, *Bulk rename*,
    *Compare* and *Console* are categories of their own. Icons:
    `LIST_CHECKS` for Selection, `COLUMNS` for Panes & tabs, `IMAGE` for
    Preview pane.

342. (GUI fixes, keyboard shortcuts, 2026-11-10) The issue calls the
    duplicate *Commit* row "a registration bug in `ext-git`". `ext-git`
    registers `git.commit` once; the second row is `svn.commit` of
    `ext-svn`, and both sit under "File operations" because 0.3 files
    every registered action there.
    Assumed: the fix is the filing — each under its plugin — and nothing
    changes in what `ext-git` registers. Under *All* the two rows remain,
    one under *GIT · PLUGIN* and one under *SVN · PLUGIN*.
    Decided (2026-11-10): as found, and the two get names of their own —
    Serge: "then it should say git commit and svn commit". The actions
    are *Git commit* and *SVN commit*; SPEC.md's Keyboard shortcuts
    dialogue and the two plugins' lines in the package layout say so.
    Assumed with it: the name is the action's one label, so the command
    palette and the *Git* and *SVN* submenus of the context menu read
    the same as the shortcuts list.

343. (GUI fixes, keyboard shortcuts, 2026-11-10) Four details the frame
    leaves open. The conflict chip is drawn with `text/primary` on
    `status/warning`, 2.5:1. Nothing says which action a chord in
    conflict runs. Several actions have more than one default chord
    (Back: Backspace, Alt+Left). The notes write changes to "the keymap
    TOML".
    Assumed: the chip is `trash/mid-bg` with `trash/mid-text`, the amber
    pair the scheme already holds to 4.5:1. A chord in conflict runs the
    built-in action before a plugin's, otherwise the first in the list.
    All of an action's chords are shown, one after the other; recording
    replaces them with the one pressed. Backspace unbinds while
    recording, so it cannot be recorded in the dialogue, only written in
    the file. The keymap is the `[shortcuts]` table of `rick.toml`. Mod
    is written *Ctrl*, and *Cmd* on macOS.

344. (GUI fixes, extensions, 2026-11-10) The details show things the
    host does not have: a description (the manifest has none), "2
    accounts connected" in Status, an attention text about what Azure
    cannot do yet, and "deprecated" as a reason. The frame shows *Remove*
    and a per-user path on a plugin with the *Built-in plugin* chip.
    Assumed: the manifest gains a description and the API version goes
    up. Status is the load state alone. A plugin needs attention when the
    host refused or failed to load it, or when it reported a missing
    library in this session; "deprecated" and notes on known limits wait
    for a field that carries them. Built-in means beside the binary;
    the notes win over the frame, so *Remove* is hidden for a built-in.
    The extensions compiled into the binary are not listed, as before. A
    plugin that fills only the decorator point would need a fifth
    heading; none ships.

345. (GUI fixes, extensions, 2026-11-10) *Install from file…* "picks a
    .rkp", and *Remove* deletes a plugin. No `.rkp` format is defined
    anywhere, Rick has no code that installs or removes a plugin, the
    issue holds "visual and interaction changes only", and the trust on
    first use of `specs/system-integration.md` §9 is not built.
    Assumed: an `.rkp` is a zip archive of the plugin's folder as it sits
    in the plugins directory. Install unpacks it into the per-user
    plugins folder, refuses a file without a plugin manifest and asks
    before it replaces a plugin of the same id; *Remove* asks and deletes
    the folder; both take effect at the next start. No trust check is
    added beyond what the loader does today: §9 arrives with its
    milestone. If the two buttons should wait for that milestone
    instead, they are left out of item 9.
    Decided (2026-11-10): build both buttons, zip-based, with no checks
    and no confirmation; the trust check comes later (Serge). So an
    `.rkp` is a zip of the plugin's folder; install unpacks it and
    replaces what is there without asking, and does not look for a
    manifest — the loader judges the result at the next start; *Remove*
    deletes the folder without a question. SPEC.md's Extensions dialogue
    is changed to say so. One thing stays, which is not a check of the
    plugin: nothing in an archive is written outside the plugins folder.

346. (GUI fixes, title bar colour, 2026-11-10) The notes say "contrast
    of text/secondary on every light preset is ≥ 4.5:1; verify after
    implementation", and the issue's acceptance asks for 4.5:1. Computed:
    `text/secondary` (`#6b6b6b`) on Dusty rose 4.12, Warm grey 4.13,
    Slate teal 4.10, Sage 4.17, Plum 3.99. In Dark it holds, 4.81 to
    5.47, and `icon/default` holds everywhere.
    Assumed: the presets stay as given and the inactive titles change
    where they fall short: `#2b2b2b` at 78 % on a light band (5.8 to
    6.0:1), the mirror of the white at 78 % the dark band draws. The
    frames show `text/secondary`; the difference is a slightly darker
    grey in Light.
    Decided (2026-11-10): as assumed, for now — Serge: "go with it for
    now". SPEC.md already says so; nothing changes there.

347. (GUI fixes, title bar colour, 2026-11-10) Three cases the frames do
    not cover. On the Dark band in Light, a hovered inactive tab at
    "`bg/pane` at 50 %" is a mid grey on which white at 78 % has 2.4:1
    and `icon/default` 3.0:1. A light *Custom* colour in Dark is not
    mentioned. The search box without its border has 1.2:1 against a
    light band.
    Assumed: on a dark band in Light the hovered tab is white at 12 %,
    as the frame draws the hovered menu button; a light band in Dark is
    the mirror image, with `#2b2b2b`; the search box is told by its icon
    and placeholder, which are held to 3:1 and 4.5:1, and its fill
    against the band is not a checked pair, as a tab's is not. *Custom*
    is two `#rrggbb` fields with a swatch each; no colour picker is
    drawn. The setting is `titlebar_colour`, with
    `titlebar_custom_light` and `titlebar_custom_dark`.

348. (GUI fixes, small items, 2026-11-10) "Window controls spaced wider
    apart" has no number and no frame of its own. In 0.3 the glyphs are
    about 10 px wide and 9.5 px apart, a 19 px step; the title row
    frames of this issue draw 16 px icons 12 px apart, a 28 px step.
    Assumed: the frames' spacing, for the whole window group, toggles
    included, each control answering over its whole step. If wider than
    the frames is meant, the number is needed.

349. (GUI fixes, small items, 2026-11-10) Three small items leave a
    choice. The settings' second close button: where the settings cannot
    open as a window of their own they are drawn inside the main window
    and the round button is the only one. A console's title: the shell
    can set one. An operation's error "also appears where the user
    acted": the brief says "a toast or an inline card"; 0.3 already
    shows a toast for every failed job, for five seconds.
    Assumed: the round button stays in the embedded case only; the tab
    reads *Console* whatever the shell sets; the error is a toast in the
    window the user acted in, with the whole text wrapped, that stays
    until it is dismissed, and the drawer's row expands on a click.

350. (GUI fixes, paste conflict, issue 139 item 3, 2026-11-10) The
    dialogue needs both entries' size and date and the conflict's place
    among the job's, and `Conflicts::ask` is what plugins call too: the
    extraction of `ext-action-compress` asks through it.
    Assumed: `ask` takes a `ConflictDetails` in place of `target_is_dir`
    and `remaining`, and the extension API version goes to 17. A copy or
    move counts its conflicts before it begins; an extraction does not, so
    its dialogue has no "(n of m)" and its checkbox reads *Do the same
    for all remaining conflicts*, without a number.

351. (GUI fixes, paste conflict, issue 139 item 3, 2026-11-10) Between
    two folders Enter is *Skip* (question 329), and nothing says which
    button is the filled one there.
    Assumed: *Skip* is filled, as the button Enter chooses is everywhere
    else, and *Merge* keeps the amber of *Replace*.

352. (GUI fixes, Properties, issue 139 item 4, 2026-11-10) The header
    names "the type" — *Folder*, *Symbolic link*, *Executable file*, and
    for the File info card *JPEG image*, *PDF document* — and nothing
    says how a type's name is found; the file list's Type column has
    `JPG`.
    Assumed: one helper for both places. A file is named after the
    family of its extension — image, video, audio, archive, document —
    with the format in capitals before it (*JPEG image*, *MKV video*,
    *TAR archive*, *PDF document*); a file in no family is an
    *Executable file* when it is executable, else its extension and
    *file* (*BIN file*), or *File* without one.

353. (GUI fixes, Properties, issue 139 item 4, 2026-11-10) *Contents*
    reads "12 items · 3.4 GiB" and does not say whether the items are
    the folder's own entries or everything below it; the size can only
    be everything below it.
    Assumed: one walk gives both — every file and folder inside, at any
    depth, links counted as themselves and never followed, entries that
    cannot be read left out. *On disk* is what all of it occupies, the
    folder's own blocks included. Only a folder on this machine is
    counted; a remote folder has no Size group.

354. (GUI fixes, Properties, issue 139 item 4, 2026-11-10) *Open target
    folder* "shows the folder that holds the target", and a link's text
    can be relative or go through other links.
    Assumed: the folder the link's text names, a relative text taken
    from the link's folder, `.` and `..` read as text; where no entry
    has that name (a `..` after another link), the real folder of the
    target. *Resolves to* describes the entry at the end of the chain.
    A link on a remote provider shows its *Target* only: nothing follows
    it from here, so it has no *Resolves to* unless it is broken, and
    its button is off.

355. (GUI fixes, Properties, issue 139 item 4, 2026-11-10) "For a
    regular file also *Media type*": the row shows what the provider
    reports for the entry, and the local provider reports none, as in
    0.3.
    Assumed: the row is there when the entry has a media type and left
    out when it has none; finding the type of a local file (the shared
    MIME database is only read on Linux, for *Open with*) is not part of
    this item.

356. (GUI fixes, Properties, issue 139 item 4, 2026-11-10) Three
    behaviours of the Permissions tab follow from the frames and are not
    written out. 0.3 chose files and folders separately under *Apply to
    enclosed items*; the frame has one switch. 0.3 gave a new list entry
    a mask of `rwx`; the two grids now move together, so the mask shows
    as the Group row. An entry on a provider without permissions has
    nothing to show on the tab.
    Assumed: the switch covers files and folders alike. A new entry gets
    read access and the mask becomes the union of the owning group's and
    the named entries' rights, as `setfacl` computes it, and goes with
    the last named entry. Without permissions the *Permissions* segment
    is washed out and says why in its tooltip. The dialogue is a modal
    of the shared shell now: Escape and a click beside it close it,
    unapplied changes included, and the keymap is quiet while it is
    open.

357. (GUI fixes, preview pane, issue 139 item 5, 2026-11-10) The frame
    of *No previewer* names `random-4k.bin` a *Binary file*; question 352
    names a file by the family of its extension and `.bin` is in none.
    Assumed: in that card the file is known to be no text — no previewer
    took it for one — so a file in no family and not executable is a
    *Binary file* there; everywhere else it stays *BIN file*.

358. (GUI fixes, preview pane, issue 139 item 5, 2026-11-10) *Open as
    hex* shows "this file only, until the selection moves on", and the
    hex previewer claims no file by itself.
    Assumed: a command of its own, `PreviewWith`, opens the pane's file
    through one previewer whatever the scores say; the choice is dropped
    when the pane's selection changes, so coming back to the file shows
    the card again. The button is left out for a folder, and while the
    hex plugin is not loaded.

359. (GUI fixes, preview pane, issue 139 item 5, 2026-11-10) Four
    things about the card's places are not written out. With the card
    centred in the content the frames draw no file icon and no controls
    in the toolbar. The pin "is remembered per previewer". A missing
    library's *Where Rick looked* "unfolds the places that were searched
    and the loader's error". *Destination*'s button "shows the folder …
    creating it first".
    Assumed: the file icon stays at the toolbar's left, as *Toolbar*
    says for every entry, and the previewer's controls go, since there
    is no document for them to act on; the info button is then inert,
    the card being on show. The pin is `<previewer id> = true` in
    `[preview.pinned]`, the id being the extension's (`preview-image`).
    The line unfolds the text the plugin reports, as it reports it. The
    button makes the per-user plugins folder and opens it with the
    system's handler for a folder.

360. (GUI fixes, preview pane, issue 139 item 5, 2026-11-10) "Only the
    tags Rick knows by name and that have a value": a GPS block without
    a position still holds named tags with values (the version, a
    reference letter), and a maker note is a named tag whose value is
    bytes.
    Assumed: no GPS row at all unless latitude and longitude are both
    numbers; every other named tag with a value is listed, bytes
    included, cut at 200 characters as before.

361. (GUI fixes, preview tables, issue 139 item 6, 2026-11-10) Widths
    are remembered "per previewer … the archive tree's by column, the CSV
    grid's by position", and nothing says under which key or what a
    table wider than the pane does.
    Assumed: `[preview.columns]` in `rick.toml`, one array per
    previewer id holding every column's width but the last, by position
    — the archive tree's columns have fixed places, so by position is by
    column there. They are written when a drag ends or a double-click
    has fitted a column. A table whose columns are wider than the pane
    scrolls sideways, its header with it. The tree's titles follow the
    rule: *Modified* and *Packed*, where 0.3 had *Date modified* and
    *Compressed*.
