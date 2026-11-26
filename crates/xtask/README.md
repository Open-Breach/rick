# xtask

`xtask` is the build tasks binary behind `cargo xtask` (an alias in
`.cargo/config.toml`), with the packaging as its library half.
`cargo xtask bundle [--release]` builds the whole
workspace with its examples and lays out `target/<profile>/bundle/`
(QUESTIONS.md 84): the
`rick` binary, the kernel dylibs `types`, `runtime` and `ext` and the
`libstd` they need beside it, pdfium when the PDF previewer's `library`
setting (`preview-pdf.toml` in the config directory) names it,
`licenses/` with the licence texts those two bring with them (the texts
a pdfium build carries beside its library into `licenses/pdfium/`, the
standard library's notices from the toolchain into `licenses/rust/`), and
`plugins/<id>/plugin.toml` with the library for every plugin crate (an
`ext-*` member built as a `cdylib`), the GStreamer runtime into the media
plugin's folder `plugins/preview-media/` on macOS and Windows when
`RICK_GSTREAMER_DIR` names it, and the 7-Zip library with its licence
into `plugins/vfs-archive/` beside the archive plugin when
`RICK_SEVENZIP_DIR` names the directory holding it
(`.github/scripts/sevenzip.sh` fills one), and on Windows the Visual C++
runtime's libraries beside the binary when `RICK_VCRUNTIME_DIR` names the
toolset's redistributable folder (the release workflow does), so that running `rick` from
there finds every plugin with nothing copied by hand.
`cargo xtask package [--release]` makes the packages of the separate
plugins from that bundle (specs/plugin-distribution.md §4): into
`target/<profile>/packages/` it writes one
`<id>-<version>-<platform>.rkp` per plugin of the bundle whose crate is
marked `separate` under `[package.metadata.rick]`, a zip archive of the
folder `<id>/` as the bundle has it, and `plugins-<platform>.json`, the
manifest the plugin registry's publisher reads, with each file's size and
SHA-256 and the plugin's points and triggers. The platform is the one the
task was built for, named as the release names its archives (`linux-x64`),
and the same bundle gives the same bytes: sorted entries, one timestamp,
one mode. The release workflow archives the bundle without the separate
plugins and attaches their packages. `cargo xtask readme` brings `README.md` up to
the release the workspace's version names (specs/readme.md): it runs the
`tests` crate's `readme` test with `RICK_README_WRITE`, which writes the
version, the Features list and the Install section, renders the README's
two screenshots through the screenshot harness into `docs/screenshots/`,
and runs the test again as a check. It depends on `serde_json`, `toml`,
`zip`, `flate2` and `sha2` and on no kernel crate: it
drives `cargo build --workspace --exclude xtask --lib --bins --examples`
and the workspace's own
`rick-plugin-manifest` helper, since a build of a smaller package set would
rewrite the shared libraries for another feature set (QUESTIONS.md 81).
`tests` may depend on its library, `xtask::package`, to run the packaging
on real plugins; nothing else does. It
is a developer tool, not part of Rick, and prints to the terminal.
