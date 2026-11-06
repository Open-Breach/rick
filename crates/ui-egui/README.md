# ui-egui

`ui-egui` is the only crate that knows egui. It renders the window from the
`model` the `app` owns: the window frame with its shadow and resize grips
where the system draws none, the title row with the burger menu, the `egui_dock` tree of tabs and
split panes, each tab's toolbar, path bar, file list (details or list view,
only the visible rows laid out) and status line, the preview pane (toolbar,
info badges, content, status bar), the
operations area, the context menu and command palette built from the action
registry, the keyboard shortcut table, and the Settings and Keyboard
shortcuts screens. It never mutates the model: everything the user does
becomes a `Command` handed to `app`, and it re-renders from the model. It
depends on `types`, `runtime`, `platform`, `ext`, `model` and `app`; only
`rick` and `tests` may depend on it.
