# ui-egui

`ui-egui` is the only crate that knows egui. Every text it shows goes
through the `t!` lookup with a key of `locales/en.ftl`, which
`.github/scripts/check-strings.py` enforces in CI. Every colour and size it
draws comes from the scheme and density tokens of `theme` (the Figma
collections *Rick* and *Density*, one to one; Light and Dark, Comfortable
and Compact), from which egui's visuals and spacing are derived; its type
is the system's UI and monospace fonts (`fonts`, through the platform's
`SystemFonts`), egui's bundled fonts behind them, and the Phosphor icons. It renders every window from
the `model` the `app` owns, the first as eframe's root viewport and the
others as viewports of their own drawn in the same frame: the window frame
with its shadow and resize grips where the system draws none, the title row
with the burger menu (New window, the Layouts submenu), the workspace tabs
(one per workspace, titled after the folder in its focused pane, reordered
by drag, a tab dragged outside every window opening a new one) and the
search box, the `egui_dock` tree of the shown workspace's split panes, whose
divider positions it reports back for the session,
each pane's toolbar (Back and Forward, the parent chip and breadcrumb, the view options menu with the per-type filter, the grab handle that docks the pane on another's dock tiles), path bar (breadcrumbs per provider: a `rick://` view's
title, the folders above an archive as local segments and the archive and
its contents on the archive provider), file list (details, tree, list or
icon view, only the visible rows laid out, names edited in place for
Rename, New file and New folder) and status line with the subtree-search
toggle, the Preview tab (toolbar with the media transport and *Copy*, info
badges, content, status bar; one per window, following the focused
explorer, docked and torn out like any tab; Rick's only viewer, laying out
every kind of content a previewer returns itself in `content.rs`: a file
list in the explorer's columns, a grid, a rendered document, and the
message for a missing third-party library with *Browse…* and *Retry*; the
placeholder for a file no previewer claims), the side panel (the
editable Places list with its drag reorder, Edit… dialogue
and keyboard, Favorites, Recent and the Trash view among the places; the
This Machine, Network and Cloud sections with their utilisation bars, Eject
and Add… rows, folding without an indent guide; the section overviews drawn
as drive tables in the pane) with the operations area at its foot (a header
with the running count and an overall bar, one row per job with its progress
bar, bytes or items done of total, throughput and remaining time, the
*Waiting* state, pause, resume and cancel, a height that grows with the jobs
up to a cap), the conflict dialogue, the question dialogue an extension asks through, the
Properties dialogue (the general rows and, for a local entry, its owner,
group, mode bits and ACL edited in place, applied to the enclosed entries
too when asked), the Hash dialogue (a block per selected file with the
MD5, SHA-1, SHA-256 and BLAKE3 rows, each with a read-only monospace value
and Copy, then Copy all; byte progress and Cancel while the job runs; the
format toggle and the Verify field, compared live against every row of
the pasted length), the context menu and command palette
built from the action registry, the keyboard shortcut table, the Settings
(the language row among the rows) and Keyboard shortcuts screens, and drag and drop of entries: rows are drag
sources, folders, pane space, breadcrumb segments and tab headers are drop
zones with the ghost, the live indicator and spring-loading in between (a
pane at the Trash view trashes what lands in it); the side panel is never a
file-operation target, a dragged folder becomes a place at the insertion
line and files are refused with the not-allowed indicator; a drag that
leaves the window is handed to the platform's drag source one frame later,
once the ghost is gone, and followed until the platform says it ended; a
drop from outside lands where the platform says the cursor is, or in the
focused pane's folder where it cannot say. It never mutates
the model: everything the user does becomes a `Command` handed to `app`,
and it re-renders from the model. It depends on `types`, `runtime`,
`platform`, `ext`, `vfs-local` (the local provider's id, for dropped
paths), `vfs-rick` (the virtual locations' titles and ids), `model` and
`app`; only `rick` and
`tests` may depend on it.
