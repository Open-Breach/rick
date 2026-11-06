# vfs-local

`vfs-local` is the `Provider` for the local filesystem: listing in batches of
1 024 entries through the platform bulk reader, `stat`, reading and writing,
directories, rename, permanent removal and trash through `platform`, links,
native copy fast paths with progress, change notification through the
`notify` crate translated into `FsChange`, and the subtree search over the
`ignore` crate's parallel walker. It depends on `types`, `runtime`,
`platform`, `ext` and `vfs`; only `tests` and the application layers above may
depend on it.
