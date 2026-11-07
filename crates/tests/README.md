# tests

`tests` holds Rick's integration tests and the `fixture` module that generates
the directory tree they and the benchmarks run against: N entries (200 000 by
default) with configurable depth and file-size distribution, written into a
temporary directory, identical for the same seed on every platform, and never
committed, the `bench` module with the acceptance benchmarks' targets per
operating system, which `BENCHMARKS.md` documents, the `media` module that
writes a photo with EXIF and a PDF, and the `mem` module, an in-memory
`Provider` under any id with a settable delay per copied file, the tests'
stand-in for a cloud provider. It is the top of the dependency graph and may
depend on every other crate in the workspace; nothing depends on it.
