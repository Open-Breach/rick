# model

`model` is the single source of truth for everything the windows show, as
plain data: every window with its geometry, side panel and dock tree of
panes, each pane's tabs in strip order, and per tab the location and history,
the listing with its sort, group, filter and selection, the view mode with
the tree view's expanded folders and the scroll offset, the job table every
window shares, the side panel's data (recent entries, volumes, folder
tree), the name conflicts jobs wait on, and the settings snapshot, with the
pinned locations, the drag and the search settings among them. It turns
itself into the `Session` that `session.toml` and the saved layouts hold, and
is rebuilt from one. It knows how to apply a listing batch or a filesystem
change and how to derive the visible, sorted, grouped rows, but it performs
no I/O and knows nothing about the OS or the UI. It depends on `types`,
`runtime` (sorting on the compute pool), `platform` (the volume values),
`ext` and `serde`; `ops`, `app` and the crates above
them may depend on it.
