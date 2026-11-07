# plugin-panic

`plugin-panic` is a plugin whose entry point panics instead of registering:
the fixture for the host's promise that a plugin panicking during
registration is caught and reported with its message, and never takes the
host down. Built as a `cdylib` beside the test binaries and never
shipped. It depends on `ext` only; only the tests of `plugins-native` and
`tests` use it, as a library file.
