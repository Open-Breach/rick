# tests

`tests` holds Rick's integration tests, one test binary (`tests/main.rs`)
with each file under `tests/` as a module of it, and the `fixture` module that generates
the directory tree they and the benchmarks run against: N entries (200 000 by
default) with configurable depth and file-size distribution, written into a
temporary directory, identical for the same seed on every platform, and never
committed, the `bench` module with the acceptance benchmarks' targets per
operating system, which `BENCHMARKS.md` documents, the `media` module that
writes a photo with EXIF and a PDF, and the `mem` module, an in-memory
`Provider` under any id with a settable delay per copied file, the tests'
stand-in for a cloud provider, with a native-transfer mode that counts the
bytes read through `open_read`, so a test proves an in-account move
streamed nothing. The `readme` module holds the parts of `README.md`
that are regenerated with every release (specs/readme.md): it loads every
extension a release carries into a registry and writes the Features list
and the Install section from what registered, and the test `readme` holds
the page against it and against the spec, or writes it when
`RICK_README_WRITE` is set, which is what `cargo xtask readme` runs. The provider plugins of issue 21 are dependencies too,
for the acceptance tests that load them through the host and drive them
against the containers CI starts (`RICK_S3_ENDPOINT` and friends), and
`ext-ssh` with its `fake-sshd` feature, the SSH server in the test process
that the SSH acceptance tests run against on every OS. The previewer plugins are dependencies so
that their fixture generators are at hand and their library files are
built beside the test binaries, from where the tests install them into a
temporary plugins directory and load them through the host; the
`gstreamer` feature turns the media previewer's playback on for the media
tests. It is the top of the dependency graph and may depend on every other
crate in the workspace; nothing depends on it.
