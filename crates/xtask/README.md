# xtask

`xtask` is the build tasks binary behind `cargo xtask` (an alias in
`.cargo/config.toml`). `cargo xtask bundle [--release]` builds the whole
workspace with its examples and lays out `target/<profile>/bundle/`
(QUESTIONS.md 84): the
`rick` binary, the kernel dylibs `types`, `runtime` and `ext` and the
`libstd` they need beside it, pdfium when the PDF previewer's `library`
setting (`preview-pdf.toml` in the config directory) names it, and
`plugins/<id>/plugin.toml` with the library for every plugin crate (an
`ext-*` member built as a `cdylib`), and the 7-Zip library with its licence
into `plugins/vfs-archive/` beside the archive plugin when
`RICK_SEVENZIP_DIR` names the directory holding it
(`.github/scripts/sevenzip.sh` fills one), and on Windows the Visual C++
runtime's libraries beside the binary when `RICK_VCRUNTIME_DIR` names the
toolset's redistributable folder (the release workflow does), so that running `rick` from
there finds every plugin with nothing copied by hand; the release workflow
archives the same directory. `cargo xtask readme` brings `README.md` up to
the release the workspace's version names (specs/readme.md): it runs the
`tests` crate's `readme` test with `RICK_README_WRITE`, which writes the
version, the Features list and the Install section, renders the README's
two screenshots through the screenshot harness into `docs/screenshots/`,
and runs the test again as a check. It depends on `serde_json` only and on no kernel crate: it
drives `cargo build --workspace --exclude xtask --lib --bins --examples`
and the workspace's own
`rick-plugin-manifest` helper, since a build of a smaller package set would
rewrite the shared libraries for another feature set (QUESTIONS.md 81). It
is a developer tool, not part of Rick, and prints to the terminal.
