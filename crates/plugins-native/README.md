# plugins-native

`plugins-native` is the host for every `ext-*` crate that is not compiled
into the binary. At start it scans `<binary dir>/plugins/` and
`<config dir>/plugins/` (per-user wins on an id clash), reads each plugin's
`plugin.toml`, refuses a plugin whose `api_version`, `toolchain` or
`platforms` do not fit this host, skips the ids `rick.toml` disables, loads
the rest with `libloading`, calls the one exported entry point, named
after the crate (`rick_plugin_register_ext_git`), with a registry of its
own that lends the platform services, and hands each plugin to the
application as an extension it registers like a built-in. A plugin that
fails or panics while registering is reported in the plugins table and left
alone, its library kept mapped since code that ran may have registered
itself with the runtime (QUESTIONS.md 85); it never takes the host down. A plugin shares the kernel crates
`types`, `runtime` and `ext` with the binary as Rust dylibs (QUESTIONS.md
81), so its jobs, tracing and contributions land in the host's one runtime,
subscriber and registry. This is the one crate besides `platform` that uses
`unsafe`, for the loader calls, each with a `// SAFETY:` comment
(QUESTIONS.md 82). It depends on `types` and `ext`; `rick`, `tests` and the
bundler's helper `rick-plugin-manifest` (a binary of this crate) use it.

## How to make a plugin

A plugin is one crate in this workspace whose name starts with `ext-` and
that builds as a library file. `ext-git` is the reference plugin; the pieces
are:

**Cargo.toml.** Depend on `ext` and `types` only (SPEC.md "Module rules");
build both the plugin library and a plain library, the latter for tests and
for compiling the plugin into the binary behind a feature:

```toml
[package]
name = "ext-hello"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
types = { workspace = true }
ext = { workspace = true }
tracing = { workspace = true }

[lints]
workspace = true
```

**The extension and its entry point.** Implement `ext::Extension` as any
built-in does: a `Manifest` with the id, the name, the crate's version, the
`API_VERSION`, the points it fills and the platforms it runs on, and
`register`, which contributes to the registry it is given. A provider that
serves a virtual location registers it with `reg.view("name", provider)`
(`rick://name`); an extension that needs the platform services (the trash,
the config directory, the dialogues) takes `reg.platform()`, which the host
lends and the tests fill with a `Fake`. Then one line declares the entry
point; the macro carries the `#[export_name]` allowance, so the crate keeps
`#![deny(unsafe_code)]` (QUESTIONS.md 82, 99):

```rust
#![deny(unsafe_code)]

use std::sync::Arc;

use ext::{API_VERSION, Capabilities, Extension, Manifest, Platforms, Points, Registry, Version};
use types::{Error, ExtId};

pub struct Hello {
    manifest: Manifest,
}

impl Hello {
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ExtId("hello"),
                name: "Hello",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::ACTION,
                platforms: Platforms::ALL,
            },
        }
    }
}

impl Extension for Hello {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.action(Arc::new(/* an ext::Action */ todo!()));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(Hello::new()) as Arc<dyn Extension>);
```

Take `tracing`, `tokio` (through `ext::JobRunner` and `runtime`) and the
compute pool from the kernel crates, never from a copy of your own: the
kernel dylibs hold the one runtime and the one subscriber of the process
(QUESTIONS.md 81). Spawn long work through the `JobRunner` an action
receives, and it shows in the operations area and cancels from there.

**The manifest file.** Never written by hand. `cargo xtask bundle` loads the
freshly built library, reads the manifest the entry point registers and
writes `plugins/<id>/plugin.toml` beside a copy of the library:

```toml
id = "hello"
name = "Hello"
version = "0.2.0-dev"
api_version = 4
points = ["action"]
platforms = ["linux", "macos", "windows"]
toolchain = "rustc 1.97.0 (2d8144b78 2026-07-07)"

[library]
linux = "libext_hello.so"
macos = "libext_hello.dylib"
windows = "ext_hello.dll"
```

The host refuses the plugin when `api_version` is not its own, when
`toolchain` is not the exact `rustc -V` line it was built with, when
`platforms` leaves out the current one, or when the library's manifest
disagrees with the file. Add the crate to `deny.toml` (`wrappers = ["rick",
"tests"]`) like `ext-git`.

**Where it goes.** `cargo xtask bundle` puts the plugin under
`target/<profile>/bundle/plugins/<id>/`; the release archives carry the same
`plugins/` directory beside the binary. A user drops a plugin into the
`plugins/` folder beside the binary or into the per-user one in the config
directory (`~/.config/rick/plugins/` on Linux); the per-user copy wins when
both hold the same id. Disabling a plugin in the Extensions screen writes
its id to `[plugins] disabled` in `rick.toml`.

**Testing it.** `cargo xtask bundle` then
`target/debug/bundle/rick --list-plugins` prints the table the Extensions
screen shows: id, name, version, points, status (loaded, disabled, or
refused with the reason) and the directory it was loaded from. For a test
that loads the library in-process, see `crates/tests/tests/plugins.rs`,
which installs the fixture plugins into a temporary plugins directory with
`plugins_native::describe` and `Host::load`.

**Build the workspace as a whole.** The kernel dylibs are built once per
feature set and carry no hash in their file name, so build and test with
`cargo build --workspace --all-targets`, `cargo test --workspace` and
`cargo xtask bundle`; a `cargo test -p <crate>` of a crate below `ext`
rewrites them for its own feature set, and the next whole-workspace build
recompiles them and everything above (QUESTIONS.md 81).
