# ext

`ext` is the one API an extension sees: the five extension-point traits
(`Provider`, `Decorator`, `Action`, `Previewer`, `SearchIndex`), the data they
exchange (`Capabilities`, `ListOptions`, `FsChange`, `Decoration`, `ActionCtx`,
`PreviewDoc`, `Frame`, `Query` and friends), the `Manifest` every extension
declares, the `Extension` trait that registers contributions, and the
`Registry` that collects them and refuses manifests of another API version.
The registry is also the extension context: the host builds it with
`Registry::with_platform`, so an extension takes the platform services
(trash, config directory, dialogues) from `Registry::platform` while it
registers, and a provider serving a virtual location, `rick://<name>`,
registers it with `Registry::view`. It contains traits and plain data only,
no implementation beyond the registry and the `plugin!` macro that declares
a plugin library's entry point, and it re-exports `platform` and `runtime`
so an extension depends on `ext` and `types` alone. It is built as a Rust
dylib that the binary and every plugin library share, so the traits cross
the library boundary as themselves (QUESTIONS.md 81). It depends on
`types`, `runtime` and `platform`; `vfs`, `vfs-local`, `plugins-native` and
every `ext-*` crate depend on it, and the kernel never depends on an
extension.
