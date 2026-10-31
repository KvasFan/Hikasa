//! The error model (T3, §5.5): [`ErrorKind`], [`Classify`], [`ErrorReport`], [`InvariantViolation`].
//!
//! Every crate error implements [`Classify`]; an [`ErrorKind`] combined with an operation context maps to one
//! exit code (§2.21). [`ErrorReport`] is the serialisable snapshot used by JSON output, IPC and per-node error
//! states.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::cancel::Cancelled;
use crate::spec::Spec;

/// Coarse classification of every error in Hikasa (stable JSON `error.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorKind {
    /// The named item does not exist.
    NotFound,
    /// Access was refused.
    PermissionDenied,
    /// The target already exists.
    AlreadyExists,
    /// A folder was expected.
    NotADirectory,
    /// A file was expected.
    IsADirectory,
    /// Invalid user input (spec, mask, option, argument).
    InvalidInput,
    /// Invalid data in a file or message.
    InvalidData,
    /// Not supported in this version or on this back-end.
    Unsupported,
    /// The operation was cancelled.
    Cancelled,
    /// Other I/O failure.
    Io,
    /// Larger than a documented limit.
    TooLarge,
    /// IPC protocol violation.
    Protocol,
    /// A bug: invariant violation or panic.
    Internal,
    /// A folder that must be empty is not (M2 W3, D-93; additive).
    NotEmpty,
}

impl ErrorKind {
    /// Stable snake_case name, identical to the serde form (`"not_found"`, …).
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorKind::NotFound => "not_found",
            ErrorKind::PermissionDenied => "permission_denied",
            ErrorKind::AlreadyExists => "already_exists",
            ErrorKind::NotADirectory => "not_a_directory",
            ErrorKind::IsADirectory => "is_a_directory",
            ErrorKind::InvalidInput => "invalid_input",
            ErrorKind::InvalidData => "invalid_data",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::Cancelled => "cancelled",
            ErrorKind::Io => "io",
            ErrorKind::TooLarge => "too_large",
            ErrorKind::Protocol => "protocol",
            ErrorKind::Internal => "internal",
            ErrorKind::NotEmpty => "not_empty",
        }
    }
}

impl fmt::Display for ErrorKind {
    /// Writes [`ErrorKind::as_str`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Implemented by every crate error.
pub trait Classify {
    /// The error's kind.
    fn kind(&self) -> ErrorKind;
}

/// Maps `std::io::ErrorKind` to `ErrorKind` (total; unknown kinds → `Io`). An I/O error that wraps
/// [`Cancelled`] (e.g. from a reader that observed cancellation) maps to `Cancelled`.
pub fn io_error_kind(e: &std::io::Error) -> ErrorKind {
    use std::io::ErrorKind as Io;
    if e.get_ref().is_some_and(|inner| inner.is::<Cancelled>()) {
        return ErrorKind::Cancelled;
    }
    match e.kind() {
        Io::NotFound => ErrorKind::NotFound,
        Io::PermissionDenied => ErrorKind::PermissionDenied,
        Io::AlreadyExists => ErrorKind::AlreadyExists,
        Io::NotADirectory => ErrorKind::NotADirectory,
        Io::IsADirectory => ErrorKind::IsADirectory,
        Io::InvalidInput | Io::InvalidFilename => ErrorKind::InvalidInput,
        Io::InvalidData => ErrorKind::InvalidData,
        Io::Unsupported => ErrorKind::Unsupported,
        Io::FileTooLarge => ErrorKind::TooLarge,
        Io::DirectoryNotEmpty => ErrorKind::NotEmpty,
        Io::CrossesDevices => ErrorKind::Unsupported,
        _ => ErrorKind::Io,
    }
}

impl Classify for std::io::Error {
    /// [`io_error_kind`].
    fn kind(&self) -> ErrorKind {
        io_error_kind(self)
    }
}

impl Classify for Cancelled {
    /// Always `cancelled`.
    fn kind(&self) -> ErrorKind {
        ErrorKind::Cancelled
    }
}

/// Upper bound on the recorded `source()` chain (defence against pathological chains).
const MAX_CAUSES: usize = 64;

/// Serialisable snapshot of an error (JSON output, IPC, per-node error states).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorReport {
    /// Kind.
    pub kind: ErrorKind,
    /// Top-level message (`Display` of the error).
    pub message: String,
    /// Canonical spec involved, if any.
    pub spec: Option<String>,
    /// `source()` chain messages, outermost first.
    pub causes: Vec<String>,
}

impl ErrorReport {
    /// Captures `err` (walks `source()`). The spec is recorded in canonical form, which never contains a
    /// password.
    pub fn capture<E: std::error::Error + Classify + ?Sized>(
        err: &E,
        spec: Option<&Spec>,
    ) -> ErrorReport {
        let mut causes = Vec::new();
        let mut source = err.source();
        while let Some(cause) = source {
            if causes.len() == MAX_CAUSES {
                break;
            }
            causes.push(cause.to_string());
            source = cause.source();
        }
        ErrorReport {
            kind: err.kind(),
            message: err.to_string(),
            spec: spec.map(Spec::canonical),
            causes,
        }
    }
}

impl fmt::Display for ErrorReport {
    /// Writes the top-level message.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Classify for ErrorReport {
    /// The captured kind.
    fn kind(&self) -> ErrorKind {
        self.kind
    }
}

/// An invariant check failed (always a bug; reported as `ErrorKind::Internal`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invariant {id} violated: {detail}")]
pub struct InvariantViolation {
    /// Invariant id from Appendix B (e.g. "INV-ALIGN-1").
    pub id: &'static str,
    /// Human-readable detail.
    pub detail: String,
}

impl Classify for InvariantViolation {
    /// Always `internal`.
    fn kind(&self) -> ErrorKind {
        ErrorKind::Internal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] Middle);

    #[derive(Debug, thiserror::Error)]
    #[error("middle")]
    struct Middle(#[source] io::Error);

    impl Classify for Outer {
        fn kind(&self) -> ErrorKind {
            ErrorKind::Io
        }
    }

    #[test]
    fn io_kinds() {
        let cases = [
            (io::ErrorKind::NotFound, ErrorKind::NotFound),
            (io::ErrorKind::PermissionDenied, ErrorKind::PermissionDenied),
            (io::ErrorKind::AlreadyExists, ErrorKind::AlreadyExists),
            (io::ErrorKind::NotADirectory, ErrorKind::NotADirectory),
            (io::ErrorKind::IsADirectory, ErrorKind::IsADirectory),
            (io::ErrorKind::InvalidInput, ErrorKind::InvalidInput),
            (io::ErrorKind::InvalidData, ErrorKind::InvalidData),
            (io::ErrorKind::Unsupported, ErrorKind::Unsupported),
            (io::ErrorKind::FileTooLarge, ErrorKind::TooLarge),
            (io::ErrorKind::UnexpectedEof, ErrorKind::Io),
            (io::ErrorKind::Other, ErrorKind::Io),
            (io::ErrorKind::BrokenPipe, ErrorKind::Io),
        ];
        for (io_kind, kind) in cases {
            assert_eq!(
                io_error_kind(&io::Error::from(io_kind)),
                kind,
                "{io_kind:?}"
            );
        }
        assert_eq!(
            io_error_kind(&io::Error::other(Cancelled)),
            ErrorKind::Cancelled
        );
    }

    #[test]
    fn capture_walks_the_source_chain() {
        let err = Outer(Middle(io::Error::new(io::ErrorKind::NotFound, "gone")));
        let report = ErrorReport::capture(&err, None);
        assert_eq!(report.kind, ErrorKind::Io);
        assert_eq!(report.message, "outer");
        assert_eq!(report.causes, ["middle", "gone"]);
        assert_eq!(report.spec, None);
        assert_eq!(report.to_string(), "outer");
        assert_eq!(report.kind(), ErrorKind::Io);
    }

    #[test]
    fn kind_names_match_serde() {
        let all = [
            ErrorKind::NotFound,
            ErrorKind::PermissionDenied,
            ErrorKind::AlreadyExists,
            ErrorKind::NotADirectory,
            ErrorKind::IsADirectory,
            ErrorKind::InvalidInput,
            ErrorKind::InvalidData,
            ErrorKind::Unsupported,
            ErrorKind::Cancelled,
            ErrorKind::Io,
            ErrorKind::TooLarge,
            ErrorKind::Protocol,
            ErrorKind::Internal,
        ];
        for kind in all {
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{}\"", kind.as_str())
            );
            assert_eq!(kind.to_string(), kind.as_str());
        }
        let violation = InvariantViolation {
            id: "INV-X-1",
            detail: "boom".into(),
        };
        assert_eq!(violation.kind(), ErrorKind::Internal);
        assert_eq!(violation.to_string(), "invariant INV-X-1 violated: boom");
    }
}
