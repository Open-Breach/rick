# runtime

`runtime` is the only crate that spawns threads. It owns the single `tokio`
runtime of the process (`runtime::handle`, `runtime::block_on`), with its
time and I/O drivers on, so the network providers' sockets run on it too; the
`JobRunner` that turns a future into a cancellable, progress-reporting `Job`
and hands back a `JobHandle<T>` (cancel token, `watch` progress receiver,
`oneshot` result), lists the running jobs and cancels them by `JobId`, the two question queues
between a job and the application (`Conflicts` for an existing name a copy
would land on, `Prompts` for a question an extension asks the user while it
works, such as an unknown host key or a passphrase; the application shows
the dialogue and answers by id), and the
`rayon` pool for CPU-bound work exposed as `runtime::compute`. It depends on
`types` and nothing else in the workspace; every crate above it in the layer
order may depend on it. It is built as a Rust dylib that the binary and every
plugin library share, which is what makes the runtime, the compute pool and
the tracing subscriber inside it the only ones in the process
(QUESTIONS.md 81).
