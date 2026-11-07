# plugins-native

`plugins-native` is the host for every `ext-*` crate that is not compiled
into the binary: it loads a plugin library, calls its one exported entry
point, `rick_plugin_register`, with a registry of its own, and hands the
plugin to the application as an extension it registers like a built-in. A
plugin shares the kernel crates `types`, `runtime` and `ext` with the binary
as Rust dylibs (QUESTIONS.md 81), so the extension traits cross the boundary
unchanged and a plugin's jobs, tracing and contributions land in the host's
one runtime, subscriber and registry; a plugin that fails or panics while
registering is reported and unloaded. This is the one crate besides
`platform` that uses `unsafe`, for the loader calls, each with a `// SAFETY:`
comment. It depends on `types` and `ext`; `rick` and `tests` depend on it.
