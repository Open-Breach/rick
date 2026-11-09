# vfs-rick

`vfs-rick` is the `Provider` behind Rick's virtual locations, the `rick://`
scheme: `rick://favorites`, `rick://recent` and `rick://trash`, the views the
side panel's Places open, and `rick://machine`, `rick://network` and
`rick://cloud`, the overviews its three titled sections open. Favorites,
Recent and the overviews are read-only listings built from what the kernel
already knows: the pinned locations the application hands it through
`Favorites`, the system's recently-used list and the mounted volumes from
`platform`, and what the network and cloud providers mounted in `vfs` list
at their roots: their connections and accounts, one row each. A view an extension registered (`Registry::view`, handed on through
`add_view`) is served by that extension's provider, the location and
everything below it: the Trash view is the `ext-vfs-trash` plugin's, and
lists nothing without it. Because they are provider-backed paths they sit
in a tab, the breadcrumb, the history and the session like any folder;
`changed` lets the application tell open views to list again. It depends on
`types`, `runtime`, `platform`, `ext`, `vfs` and `vfs-local`; `model`,
`ops`, `app`, `ui-egui`, `rick` and `tests` may depend on it.
