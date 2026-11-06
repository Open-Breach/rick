# model

`model` is the single source of truth for everything a window shows, as plain
data: the dock tree of panes, each pane's tabs, and per tab the location and
history, the listing with its sort, group, filter and selection, the view
mode, the job table and the settings snapshot, with the pinned locations and
the drag settings among them. It knows how to apply a
listing batch or a filesystem change and how to derive the visible, sorted,
grouped rows, but it performs no I/O and knows nothing about the OS or the
UI. It depends on `types`, `runtime` (sorting on the compute pool), `ext` and
`serde`; `ops`, `app` and the crates above them may depend on it.
