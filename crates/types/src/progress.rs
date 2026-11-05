//! Progress reports from long operations.

/// What a job has done so far. Published through the job's `watch` channel;
/// the receiver only ever sees the latest value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// Counted in items (entries listed, files copied).
    Items {
        /// Items finished so far.
        done: u64,
        /// Items expected in total, once known.
        total: Option<u64>,
    },
    /// Counted in bytes.
    Bytes {
        /// Bytes finished so far.
        done: u64,
        /// Bytes expected in total, once known.
        total: Option<u64>,
    },
    /// A free-text status line.
    Message(String),
}
