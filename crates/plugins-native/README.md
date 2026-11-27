# plugins-native

`plugins-native` is the host for every `ext-*` crate that is not compiled
into the binary. At start it scans `<binary dir>/plugins/` and
`<config dir>/plugins/` (per-user wins on an id clash), reads each plugin's
`plugin.toml`, refuses a plugin whose `rick`, `api_version`, `toolchain` or
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
                description: "Says hello.",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::ACTION,
                platforms: Platforms::ALL,
                previews: &[],
                markers: &[],
                connections: &[],
                commands: &[],
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
description = "Says hello."
version = "0.5.0"
api_version = 25
points = ["action"]
platforms = ["linux", "macos", "windows"]
previews = []
markers = []
rick = "0.5.0"
toolchain = "rustc 1.99.1 (58cf8f9c3 2026-10-13)"

[library]
linux = "libext_hello.so"
macos = "libext_hello.dylib"
windows = "ext_hello.dll"
```

`rick` is the version of Rick the plugin was built with, and the host
looks at it first. A plugin shares the kernel libraries with its Rick, so
a build loads in that release and in no other: a plugin of another one is
refused before its library is opened, with *Built for Rick 0.5.0*, and a
file without the key as *Built for an unknown Rick*
(specs/plugin-distribution.md §3). After that the host refuses the plugin
when `api_version` is not its own, when `toolchain` is not the exact
`rustc -V` line it was built with, when `platforms` leaves out the current
one, or when the library's manifest disagrees with the file; `cargo xtask
bundle` leaves out a plugin whose `platforms` name other operating systems
only, so a bundle carries the plugins of its own platform (QUESTIONS.md
121). Add the crate to `deny.toml` (`wrappers = ["rick", "tests"]`) like
`ext-git`.

**What the plugin is for.** Three lists of the manifest are the plugin's
triggers: what Rick can know about a plugin without having it
(specs/plugin-distribution.md §5). `previews` holds the file name
extensions its previewers claim. `markers` holds the names of an entry
directly inside a folder that make the folder the plugin's business, and
`connections` what the plugin adds to *Add…* in the side panel, the
section (`ConnectionSection::Network` or `Cloud`) and the title:

```rust
// ext-git
markers: &[".git"],

// ext-vfs-s3
connections: &[ConnectionSpec {
    section: ConnectionSection::Cloud,
    title: "S3",
}],
```

```toml
markers = [".git"]
```

```toml
[[connections]]
section = "cloud"
title = "S3"
```

A plugin that has none leaves the lists empty. They land in `plugin.toml`
and in the manifest of the plugin's package. The host hands all three on
with every plugin it found, loaded or not (`Plugin::previews`, `markers`
and `connections`), and Rick suggests a plugin by them: one of the
registry that is not installed, and one that is installed and disabled
from this very file (specs/plugin-distribution.md §8).

**A subcommand of the command line.** A plugin can add `rick <command>
<args…>` (specs/system-integration.md §4): the manifest declares the
command and its help line, `register` adds an `ext::CliCommand` of that
name, and the host dispatches the command to it — in the running Rick
when there is one, so the command works on the window the user sees. The
command runs on the application thread like an action: it answers with
text for the caller's terminal, or with what an action would return (a
job, a form, a view), which the application applies to the focused pane
while the caller gets exit status zero. `ext-installer`'s `rick install
<archive>` opens the install form this way. The manifest's `[[commands]]`
land in `plugin.toml`, so `rick --help` lists them without loading the
plugin:

```rust
use ext::{CliCommand, CliCtx, CliOutcome, CommandSpec, JobRunner, UiHost};

// In the manifest:
//     commands: &[CommandSpec { name: "greet", help: "Say hello to someone" }],

struct Greet;

impl CliCommand for Greet {
    fn name(&self) -> &'static str {
        "greet"
    }
    fn run(&self, ctx: CliCtx, _jobs: &JobRunner, _ui: &dyn UiHost) -> Result<CliOutcome, Error> {
        match ctx.args.as_slice() {
            [name] => Ok(CliOutcome::Text(format!("hello, {name}"))),
            _ => Err(Error::Unsupported("usage: rick greet <name>")),
        }
    }
}

// In `register`:
//     reg.command(Arc::new(Greet));
```

```toml
[[commands]]
name = "greet"
help = "Say hello to someone"
```

A relative path among the arguments resolves against `ctx.cwd`, the
caller's working directory, never the instance's.

**Where it goes.** `cargo xtask bundle` puts the plugin under
`target/<profile>/bundle/plugins/<id>/`, every plugin of the platform, so a
development build has them all. A release splits them
(specs/plugin-distribution.md §4):

- The **default set** is in the archive's `plugins/` directory beside the
  binary. A plugin crate that says nothing is in it.
- A **separate plugin** is published as a package of its own,
  `<id>-<version>-<platform>.rkp`, and is not in the archive. Its crate
  says so in `Cargo.toml`:

  ```toml
  [package.metadata.rick]
  separate = true
  # or, separate on some platforms only:
  # separate = ["macos", "windows"]
  ```

  `cargo xtask package` makes the packages from the bundle
  (`crates/xtask/README.md`).

A library the plugin needs travels in the plugin's folder, so that its
package holds it: the 7-Zip library of the archive plugin, the GStreamer
runtime of the media previewer on macOS and Windows. A library the plugin
loads itself is looked for there by the plugin, and opened by its full
path. One it links has to be found by the system's loader: on macOS the
plugin's library carries a run path relative to itself (`build.rs` of
`ext-preview-media`). Windows never looks in the folder of the library
that asks, so there the host names the folder to the loader
(`platform::loader`, `AddDllDirectory`): when a plugin's library does not
load and its folder holds libraries besides its own, the folder is named
and the library loaded once more, and the folder stays named for the
libraries the plugin's own open later. A plugin that loads the first time
is left alone, the archive plugin among them. The host never changes
Rick's environment for a plugin: nothing goes on the `PATH`, which every
program Rick starts would inherit. From the start of the host on, Windows
looks for libraries beside the binary, in the named folders and in the
system's folder only (`SetDefaultDllDirectories`), no longer in the
current directory or on the `PATH`; a library a plugin opens by name has
to lie in one of those.

A user drops a plugin into the `plugins/` folder beside the binary or into
the per-user one in the config directory (`~/.config/rick/plugins/` on
Linux); the per-user copy wins when both hold the same id. Switching a
plugin off in the Extensions dialogue
writes its id to `[plugins] disabled` in `rick.toml`. The table has two
more keys, for the suggestions of specs/plugin-distribution.md §8:

```toml
[plugins]
disabled = ["svn"]      # found and listed, not loaded
suggest = true          # Suggest plugins, on the Behaviour page
no_suggest = ["git"]    # Don't suggest again; Suggest again takes it back
```

A plugin named in `disabled` is suggested from its own `plugin.toml`,
with *Enable*, where what the user is looking at is what it is for. The
dialogue's
*Install from file…* unpacks a plugin package into the per-user folder: an
`.rkp` file, which is a zip archive of the plugin's folder — the folder
itself, or its contents, in which case the folder takes the package's
name. Nothing is checked at that point; the host judges what it finds at
the next start, as for a folder put there by hand.

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
