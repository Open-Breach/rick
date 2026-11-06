//! The kernel error type.

use crate::ExtId;

/// One error type for the whole kernel; extensions map their own into it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The entry or location does not exist.
    #[error("not found")]
    NotFound,
    /// The provider or the platform refused access.
    #[error("permission denied")]
    PermissionDenied,
    /// The job's [`CancelToken`](crate::CancelToken) was cancelled.
    #[error("cancelled")]
    Cancelled,
    /// The provider or platform cannot do this; the text names what.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    /// An I/O error that maps to no more specific variant.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// An error raised inside an extension.
    #[error("extension {ext}: {source}")]
    Extension {
        /// The extension that failed.
        ext: ExtId,
        /// The extension's own error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn io_errors_convert() {
        let io = std::io::Error::other("boom");
        let err: Error = io.into();
        assert!(matches!(err, Error::Io(_)));
        assert_eq!(err.to_string(), "I/O error: boom");
    }

    #[test]
    fn extension_errors_keep_their_source() {
        let err = Error::Extension {
            ext: ExtId("git"),
            source: "index locked".into(),
        };
        assert_eq!(err.to_string(), "extension git: index locked");
        assert_eq!(
            err.source().map(ToString::to_string).as_deref(),
            Some("index locked")
        );
    }

    #[test]
    fn plain_variants_display() {
        assert_eq!(Error::NotFound.to_string(), "not found");
        assert_eq!(Error::PermissionDenied.to_string(), "permission denied");
        assert_eq!(Error::Cancelled.to_string(), "cancelled");
        assert_eq!(
            Error::Unsupported("reflink").to_string(),
            "unsupported: reflink"
        );
    }
}
