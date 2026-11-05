# tests

`tests` holds Rick's integration tests and the `fixture` module that generates
the directory tree they and the benchmarks run against: N entries (200 000 by
default) with configurable depth and file-size distribution, written into a
temporary directory, identical for the same seed on every platform, and never
committed. It is the top of the dependency graph and may depend on every other
crate in the workspace; nothing depends on it.
