# vfs-rick

`vfs-rick` is the `Provider` behind Rick's virtual locations, the `rick://`
scheme: `rick://favorites`, `rick://recent` and `rick://trash`, the views the
side panel's Places open, and `rick://machine`, `rick://network` and
`rick://cloud`, the overviews its three titled sections open. Each is a
read-only listing built from what the kernel already knows: the pinned
locations the application hands it through `Favorites`, the system's
recently-used list and the mounted volumes from `platform`, the platform
trash (or Rick's own record where the platform cannot list it, through
`TrashRecord`), and the roots of the network and cloud providers mounted in
`vfs`. Because they are provider-backed paths they sit in a tab, the
breadcrumb, the history and the session like any folder; `changed` lets the
application tell open views to list again. It depends on `types`, `runtime`,
`platform`, `ext`, `vfs` and `vfs-local`; `model`, `ops`, `app`, `ui-egui`,
`rick` and `tests` may depend on it.
