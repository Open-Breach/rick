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

impl Error {
    /// Maps an I/O error to the kernel error, lifting the two kinds that have
    /// their own variant and wrapping the rest in [`Error::Io`].
    #[must_use]
    pub fn from_io(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound,
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            _ => Self::Io(error),
        }
    }

    /// A second copy of this error for another receiver. `Error` cannot be
    /// `Clone` because `std::io::Error` is not; the copy keeps the variant,
    /// the I/O error kind and the message, and drops any wrapped payload.
    #[must_use]
    pub fn replicate(&self) -> Self {
        match self {
            Self::NotFound => Self::NotFound,
            Self::PermissionDenied => Self::PermissionDenied,
            Self::Cancelled => Self::Cancelled,
            Self::Unsupported(what) => Self::Unsupported(what),
            Self::Io(io) => Self::Io(std::io::Error::new(io.kind(), io.to_string())),
            Self::Extension { ext, source } => Self::Extension {
                ext: *ext,
                source: source.to_string().into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn from_io_lifts_the_kinds_with_their_own_variant() {
        let not_found = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(matches!(Error::from_io(not_found), Error::NotFound));
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(matches!(Error::from_io(denied), Error::PermissionDenied));
        let other = std::io::Error::from(std::io::ErrorKind::AlreadyExists);
        assert!(matches!(Error::from_io(other), Error::Io(_)));
    }

    #[test]
    fn io_errors_convert() {
        let io = std::io::Error::other("boom");
        let err: Error = io.into();
        assert!(matches!(err, Error::Io(_)));
        assert_eq!(err.to_string(), "I/O error: boom");
    }

    #[test]
    fn replicate_keeps_variant_kind_and_message() {
        let io = Error::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "taken",
        ));
        let copy = io.replicate();
        assert!(matches!(&copy, Error::Io(e) if e.kind() == std::io::ErrorKind::AlreadyExists));
        assert_eq!(copy.to_string(), io.to_string());
        assert!(matches!(
            Error::Unsupported("x").replicate(),
            Error::Unsupported("x")
        ));
        let ext = Error::Extension {
            ext: ExtId("git"),
            source: "locked".into(),
        };
        assert_eq!(ext.replicate().to_string(), ext.to_string());
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
