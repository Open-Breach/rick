# ext

`ext` is the one API an extension sees: the five extension-point traits
(`Provider`, `Decorator`, `Action`, `Previewer`, `SearchIndex`), the data they
exchange (`Capabilities`, `ListOptions`, `FsChange`, `Decoration`, `ActionCtx`,
`PreviewDoc`, `Frame`, `Content` with its `Listing`, `Grid`, `Document`
and `MissingLibrary`, `Transport`, `Query` and friends, the slots an
action fills from a job, `Compare` for a tab and `Hashes` for the Hash
dialogue, and `FormLater` with its `Continuation`, the form an action
opens when a job of its finishes and the rest of the action the
application runs with the values through `UiHost::form_later`; an
action's `app_menu` puts it in the burger menu under its extension), the
`Manifest` every extension declares (with `previews`, the
file extensions its previewers claim, so the host can say a disabled
plugin is available for a file without loading it), the `Extension` trait
that registers contributions, and the `Registry` that collects them and
refuses manifests of another API version. The registry is also the
extension context: the host builds it with `Registry::with_platform`, so
an extension takes the platform services (trash, config directory,
dialogues) from `Registry::platform` while it registers, reads and writes
the settings it declared through the `SettingsStore` of
`Registry::settings_store` (a previewer keeps the path of the third-party
library it found there), a provider serving a virtual location,
`rick://<name>`,
registers it with `Registry::view`, and a provider reached over a network
registers a `Connector` beside itself (`Registry::connector`): the fields of
its *Add connection…* dialogue and the calls behind *Connect*, *Disconnect*
and *Remove connection* on its rows in the side panel. `Provider` carries
`transfer()` for a move or copy the service does by itself under
`NATIVE_TRANSFER`, `open_read_at()` for a read from an offset and
`remote_preview_cap()` for the size above which a whole-file preview waits
for *Preview anyway*, `remote_preview_always()` for a source whose *Always
preview* switch is on, and `REMOTE` marks a remote-latency provider, whose
files a `Previewer` with `reads() == Reads::Prefix` sees a chunk at a time
with *Load more*. An extension working off the UI thread asks the user
through the `Prompts` the registry lends (`Registry::prompts`): a choice
or a secret, shown by the shell as a dialogue and answered by id. The
`terminal` module is the terminal emulator (`Screen` over `vte`) the
console pane draws, so a plugin can open a console on a stream of its own
(`View::Console`). Every string an extension shows goes through the `t!` lookup
(`i18n`): Fluent files the extension bundles and hands to
`Registry::translations`, the kernel's own from `locales/`, the language
the user chose or the system's, English as the fallback, and a file in the
configuration directory overriding any of it; a label the registry keeps
(a setting's, a connection field's, an info section's) is a `Text` key read
live, so a language change relabels it too. It contains traits and plain
data only, no implementation beyond the registry, the lookup, the emulator
and the `plugin!` macro that declares a plugin library's entry point, and
it re-exports `platform` and `runtime` so an extension depends on `ext`
and `types` alone. It is built as a Rust
dylib that the binary and every plugin library share, so the traits cross
the library boundary as themselves (QUESTIONS.md 81). It depends on
`types`, `runtime` and `platform`; `vfs`, `vfs-local`, `plugins-native` and
every `ext-*` crate depend on it, and the kernel never depends on an
extension.
