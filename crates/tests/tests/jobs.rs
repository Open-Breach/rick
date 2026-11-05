//! Acceptance: 1 000 jobs, half cancelled, every outcome as promised.

use runtime::{JobRunner, watch};
use types::Error;

#[test]
fn cancelled_jobs_end_with_cancelled_and_the_rest_complete() {
    const JOBS: usize = 1_000;
    let runner = JobRunner::new();

    runtime::block_on(async {
        // A gate every job waits behind, so no job can finish before the
        // cancellations have been issued.
        let (gate_tx, gate_rx) = watch::channel(false);

        let handles: Vec<_> = (0..JOBS)
            .map(|n| {
                let mut gate = gate_rx.clone();
                runner.spawn(move |_ctx| async move {
                    // The gate sender lives until the end of the test.
                    let _ = gate.wait_for(|open| *open).await;
                    Ok::<usize, Error>(n)
                })
            })
            .collect();
        assert_eq!(runner.running().len(), JOBS);

        // Cancel every other job, alternating between the two ways to do it.
        for (n, handle) in handles.iter().enumerate() {
            if n % 2 == 0 {
                continue;
            }
            if n % 4 == 1 {
                assert!(runner.cancel(handle.id));
            } else {
                handle.cancel.cancel();
            }
        }
        let _ = gate_tx.send(true);

        for (n, handle) in handles.into_iter().enumerate() {
            let result = handle
                .result
                .await
                .unwrap_or_else(|_| panic!("job {n} lost its result"));
            if n % 2 == 1 {
                assert!(
                    matches!(result, Err(Error::Cancelled)),
                    "job {n}: {result:?}"
                );
            } else {
                assert!(matches!(result, Ok(v) if v == n), "job {n}: {result:?}");
            }
        }
        assert!(runner.running().is_empty());
    });
}
