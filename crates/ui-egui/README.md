# ui-egui

`ui-egui` is the only crate that knows egui. It renders every window from
the `model` the `app` owns, the first as eframe's root viewport and the
others as viewports of their own drawn in the same frame: the window frame
with its shadow and resize grips where the system draws none, the title row
with the burger menu (New window, the Layouts submenu), the `egui_dock` tree
of tabs and split panes, whose tab order and divider positions it reports
back for the session, a tab dragged outside every window opening a new one,
each tab's toolbar, path bar (breadcrumbs per provider: a `rick://` view's
title, the folders above an archive as local segments and the archive and
its contents on the archive provider), file list (details, tree, list or
icon view, only the visible rows laid out, names edited in place for
Rename, New file and New folder) and status line with the subtree-search
toggle, the Preview tab (toolbar, info badges, content, status bar; one per
window, following the focused explorer, docked and torn out like any tab),
the viewer tabs (one file full size with the folder behind it as a
slideshow: zoom, pages with a strip, find in the text layer, the info panel,
the actions menu, the transport bar, fullscreen), the side panel (the
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
too when asked), the context menu and command palette
built from the action registry, the keyboard shortcut table, the Settings
and Keyboard shortcuts screens, and drag and drop of entries: rows are drag
sources, folders, pane space, breadcrumb segments and tab headers are drop
zones with the ghost, the live indicator and spring-loading in between (a
pane at the Trash view trashes what lands in it); the side panel is never a
file-operation target, a dragged folder becomes a place at the insertion
line and files are refused with the not-allowed indicator. It never mutates
the model: everything the user does becomes a `Command` handed to `app`,
and it re-renders from the model. It depends on `types`, `runtime`,
`platform`, `ext`, `vfs-local` (the local provider's id, for dropped
paths), `vfs-rick` (the virtual locations' titles and ids), `model` and
`app`; only `rick` and
`tests` may depend on it.
