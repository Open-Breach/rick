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

86. (issue 20) The trash record of QUESTIONS.md 74 lived in `app`
    (`trash.toml` through `ops::TrashStore`); the trash provider plugin,
    which depends on `ext` and `types` only, cannot reach it.
    Assumed: the record moves into `platform`, behind `Trash::list`: on
    macOS the trash itself writes `trash.toml` in the config directory when
    it trashes, forgets on restore and removal, and `list` reads it back
    pruned of items that left the trash, so the local provider's trashing
    and the Trash view see one list and `list` never answers `Unsupported`
    any more. The "Items trashed elsewhere are not listed" note of the 0.1
    section is gone with the section (macOS users see Rick's own record as
    before), and the `Fake`'s unlistable knob with it; the record module
    carries its own tests.

87. (issue 20) The spec leaves the virtual-location scheme to the
    implementer and suggests `rick://`.
    Decided: `ProviderId::RICK` (`"rick"`), the view's name as the first
    path component: the location `rick:/trash` shows as `rick://trash` and
    its items live under `/trash/<name>`. One provider, `vfs::Virtual`,
    mounted from the start, routes each view's subtree to the provider an
    extension registered with `Registry::view(name, provider)`; the view
    sees full paths and hands back entries carrying `ProviderId::RICK`. The
    root, `rick://`, lists the views as folders through their own `stat`.
    Favorites, Recent and the three overviews register views the same way
    in the side-panel milestone. The path field parses `rick://…` back.

88. (issue 20) Plugins depend on `ext` and `types` only (Module rules 2),
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

89. (issue 20) "Entering an archive … navigates into it as a location
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

90. (issue 20) "Change notification by polling (watch returns
    Unsupported)" names no poller.
    Assumed: the kernel polls. For a pane whose provider's `watch` answers
    `Unsupported`, `app` calls `stat` on the location every 3 s and right
    after an operation finished, and lists again when the entry's size or
    times changed: the Trash view counts its items in `size` and takes the
    newest deletion as `modified`; an archive's root entry carries the
    archive file's size and time.

91. (issue 20) Two plugin libraries linked into one binary (the tests, or
    `rick` with two plugin features on) both exported
    `rick_plugin_register` and the link failed.
    Assumed: the exported symbol carries the crate name,
    `rick_plugin_register_ext_vfs_trash`, which `ext::plugin!` takes from
    `CARGO_CRATE_NAME`; the host and the bundler derive it from the
    library's file name, so `plugin.toml` needs no new field. The Rust
    function keeps its name, `rick_plugin_register`.

92. (issue 20) Restore, Restore trash items and Empty trash are in the
    spec's operations list without a context-menu group, and "live inside
    the view only".
    Assumed: they are the "Trash" action group
    (`ActionGroup::Extension("Trash")`), registered by `ext-vfs-trash` as
    `trash.restore`, `trash.restore_items` and `trash.empty`, Hidden unless
    the pane is at `rick://trash`, so the context menu offers them under
    Trash in the view and nowhere else, and the palette lists them there
    too. Delete permanently is the core action through the view's
    `remove(Permanent)`. Enter on a trashed item opens nothing, since the
    item is not a local file; Restore is in the menu and the palette. The
    panel row's "quiet hint of how much is in the trash" (spec: "may") is
    left out, since the panel no longer lists the trash.

93. (issue 20) Which core actions apply at a location another provider
    serves.
    Assumed: the actions follow the provider's capabilities, as the
    Provider contract says the kernel should: Paste, Duplicate, Rename, New
    file and New folder are Hidden without `WRITE`, Move to trash without
    `TRASH`, so a read-only archive, the Trash view and a RAR show only
    what works. The inline edits of New file and New folder start only
    where the provider takes writes.

94. (issue 20) How a write into an archive happens.
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

