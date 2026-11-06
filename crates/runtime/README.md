# runtime

`runtime` is the only crate that spawns threads. It owns the single `tokio`
runtime of the process (`runtime::handle`, `runtime::block_on`), the
`JobRunner` that turns a future into a cancellable, progress-reporting `Job`
and hands back a `JobHandle<T>` (cancel token, `watch` progress receiver,
`oneshot` result), lists the running jobs and cancels them by `JobId`, and the
`rayon` pool for CPU-bound work exposed as `runtime::compute`. It depends on
`types` and nothing else in the workspace; every crate above it in the layer
order may depend on it.
