# Questions

Open questions raised while implementing SPEC.md. The agent appends one
numbered entry per ambiguity, with the assumption it made; the maintainer
answers in place. If an answer changes the spec, SPEC.md changes in the
same commit.

Entry format:

`12. (issue 3) Should Undo of a trash operation restore to the original path if a new entry now has that name? Assumed: no, restore with a numbered suffix.`

---

1. (issue 1) "Package layout" says `types` has "no dependencies beyond std",
   while the "Value types" block makes `Attrs` a `bitflags` type and the
   engineering guidelines require `thiserror` in library crates. Assumed:
   `types` may depend on `bitflags` and `thiserror` (macro-only crates, no
   I/O) and on nothing else; `CancelToken` is implemented on `std` alone so
   `types` stays free of `tokio`.
2. (issue 1) `Error::Extension { ext: ExtId, .. }` and `Entry { mime:
   Option<Mime>, .. }` use `ExtId` and `Mime`, which the "Value types" block
   does not define. Assumed: both live in `types`; `ExtId(pub &'static str)`
   mirrors `ProviderId`, and `Mime` is an unvalidated `type/subtype` string
   with `type_()` / `subtype()` accessors, to be replaced only through a spec
   change.
3. (issue 1) `JobHandle<T>` is listed under "Value types (crate `types`, no
   I/O)" but is documented as "what `runtime` hands back" and is made of
   `tokio` channel ends. Assumed: `JobHandle`, `JobCtx` and `ProgressSink`
   live in `runtime`, which re-exports `tokio::sync::{watch, oneshot}` so
   callers need not depend on `tokio`.
4. (issue 1) The spec does not say whether a job that never inspects its
   `CancelToken` must still stop when cancelled. Assumed: yes; `JobRunner`
   races the job's future against the token and drops the future, so every
   cancelled job ends with `Error::Cancelled` at its next await point.
5. (issue 1) The acceptance criterion "`fixture::generate(200_000, seed)`
   runs in under 30 s on CI" is met on the Linux and macOS runners but not
   on the GitHub-hosted Windows runner, where file creation is bound by NTFS
   and Defender's real-time scanning (297 s with four writers on the system
   drive). Answered by the maintainer (2026-11-06): the 30 s budget is an
   exception on Windows for now. The test keeps its 30 s assertion and still
   runs on Windows in CI, but with `continue-on-error`, so it reports its
   time without failing the job. To revisit when the Windows runner can be
   made faster (Defender exclusion for the temp disk) or the target is
   confirmed as Linux-only.
