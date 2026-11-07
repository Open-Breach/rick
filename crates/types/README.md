# types

`types` holds the plain values that cross crate boundaries in Rick: provider
and entry identities (`ProviderId`, `EntryId`, `ExtId`), the provider-neutral
byte path `VPath` and `Location`, the directory `Entry` with its `EntryKind`,
`Attrs` and `Mime`, the cooperative `CancelToken`, job `Progress` and `JobId`,
the name `Conflict` a job asks about with its `ConflictAnswer`, and the
single kernel `Error`. It performs no I/O, spawns no threads and knows
nothing about any operating system; it depends on no other Rick crate and only
on `bitflags` and `thiserror` from the ecosystem, so every other crate in the
workspace may depend on it. It is built as a Rust dylib that the binary and
every plugin library share, so one `EntryId`, one `Error` and one
`CancelToken` exist in the process (QUESTIONS.md 81).
