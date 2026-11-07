# ui-egui

`ui-egui` is the only crate that knows egui. It renders every window from
the `model` the `app` owns, the first as eframe's root viewport and the
others as viewports of their own drawn in the same frame: the window frame
with its shadow and resize grips where the system draws none, the title row
with the burger menu (New window, the Layouts submenu), the `egui_dock` tree
of tabs and split panes, whose tab order and divider positions it reports
back for the session, a tab dragged outside every window opening a new one,
each tab's toolbar, path bar, file list (details, tree, list or icon view,
only the visible rows laid out, names edited in place for Rename, New file
and New folder) and status line with the subtree-search toggle, the Preview
tab (toolbar, info badges, content, status bar; one per window, following
the focused explorer, docked and torn out like any tab), the viewer tabs
(one file full size with the folder behind it as a slideshow: zoom, pages
with a strip, find in the text layer, the info panel, the actions menu, the
transport bar, fullscreen), the side panel (Home,
Recent, Network, Trash, Pinned, Tree) with the operations area at its foot
(progress, throughput, remaining time, pause, resume, cancel), the conflict
dialogue, the context menu and command palette built from the action
registry, the keyboard shortcut table, the Settings and Keyboard shortcuts
screens, and drag and drop of entries: rows are drag sources, folders, pane
space, breadcrumb segments, tab headers and the side panel's Home, Trash,
volume and Pinned entries are drop zones, with the ghost, the live indicator
and spring-loading in between. It never mutates the model: everything the user does
becomes a `Command` handed to `app`, and it re-renders from the model. It
depends on `types`, `runtime`, `platform`, `ext`, `vfs-local` (the local
provider's id, for pinned and dropped paths), `model`, `ops` (the trash
context the Trash section hands the actions) and `app`; only `rick` and
`tests` may depend on it.
