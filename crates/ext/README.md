# ext

`ext` is the one API an extension sees: the five extension-point traits
(`Provider`, `Decorator`, `Action`, `Previewer`, `SearchIndex`), the data they
exchange (`Capabilities`, `ListOptions`, `FsChange`, `Decoration`, `ActionCtx`,
`PreviewDoc`, `Frame`, `Query` and friends), the `Manifest` every extension
declares, the `Extension` trait that registers contributions, and the
`Registry` that collects them and refuses manifests of another API version. It
contains traits and plain data only, no implementation beyond the registry. It
depends on `types` and `runtime`; `vfs`, `vfs-local` and every `ext-*` crate
depend on it, and the kernel never depends on an extension.
