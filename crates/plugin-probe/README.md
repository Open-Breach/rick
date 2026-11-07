# plugin-probe

`plugin-probe` is the probe plugin: the fixture that proves Rick's plugin
boundary (QUESTIONS.md 81) and stands in for a real plugin in the host's
tests. Built as a `cdylib` beside the test binaries and never shipped, it
registers a Previewer for `*.probe` files whose `open` spawns a job that
reads the file through the host's provider and reports the thread it ran
on, and the action `probe.wait`, a job that runs until it is cancelled from
the job table. It depends on `ext` and `types` only, like every plugin;
only the tests of `plugins-native` and `tests` use it, as a library file.
