# tests

`tests` holds Rick's integration tests and the `fixture` module that generates
the directory tree they and the benchmarks run against: N entries (200 000 by
default) with configurable depth and file-size distribution, written into a
temporary directory, identical for the same seed on every platform, and never
committed, and the `bench` module with the acceptance benchmarks' targets per
operating system, which `BENCHMARKS.md` documents. The previewer plugins
are dependencies so that their fixture generators are at hand and their
library files are built beside the test binaries, from where the tests
install them into a temporary plugins directory and load them through the
host; the `gstreamer` feature turns the media previewer's playback on for
the media tests. It is the top of the dependency graph and may depend on
every other crate in the workspace; nothing depends on it.
