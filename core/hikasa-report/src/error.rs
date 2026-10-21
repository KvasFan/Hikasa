//! [`ReportError`]: the one error enum of the crate (T3).

use hikasa_core::{Classify, ErrorKind, io_error_kind};

/// Report errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReportError {
    /// The layout cannot produce the requested format (e.g. HTML patch).
    #[error("layout {layout} does not support {format}")]
    Unsupported {
        /// The layout (`"patch"`, `"statistics"`, `"list"`, …).
        layout: &'static str,
        /// The output format (`"html"`, …).
        format: &'static str,
    },
    /// Writing failed (`BrokenPipe` included: callers decide the policy, D-35).
    #[error("write failed: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialisation failed (bug).
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// A word of the report vocabulary (SCR-001: display mode, wrap, patch style, columns, layout, output format)
    /// is not recognised. Additive (wave 4): the vocabulary parsers live here (T11).
    #[error("invalid {option} {value:?} (expected {expected})")]
    InvalidOption {
        /// The option (`"display mode"`, `"wrap"`, …).
        option: &'static str,
        /// The rejected text.
        value: String,
        /// The accepted words.
        expected: &'static str,
    },
}

impl ReportError {
    /// Maps a `serde_json` error: a failure of the underlying writer is [`ReportError::Io`] (so that callers see
    /// `BrokenPipe`), anything else [`ReportError::Json`].
    pub(crate) fn from_json(error: serde_json::Error) -> ReportError {
        if error.is_io() {
            ReportError::Io(std::io::Error::from(error))
        } else {
            ReportError::Json(error)
        }
    }

    /// An [`ReportError::InvalidOption`].
    pub(crate) fn invalid(
        option: &'static str,
        value: &str,
        expected: &'static str,
    ) -> ReportError {
        ReportError::InvalidOption {
            option,
            value: value.to_owned(),
            expected,
        }
    }
}

impl Classify for ReportError {
    /// `Unsupported` → `unsupported`; `Io` → [`io_error_kind`]; `Json` → `internal` (a bug); `InvalidOption` →
    /// `invalid_input`.
    fn kind(&self) -> ErrorKind {
        match self {
            ReportError::Unsupported { .. } => ErrorKind::Unsupported,
            ReportError::Io(e) => io_error_kind(e),
            ReportError::Json(_) => ErrorKind::Internal,
            ReportError::InvalidOption { .. } => ErrorKind::InvalidInput,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_and_kinds() {
        let e = ReportError::Unsupported {
            layout: "patch",
            format: "html",
        };
        assert_eq!(e.to_string(), "layout patch does not support html");
        assert_eq!(e.kind(), ErrorKind::Unsupported);
        let io = ReportError::from(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        assert!(io.to_string().starts_with("write failed: "));
        assert_eq!(
            io.kind(),
            io_error_kind(&std::io::ErrorKind::BrokenPipe.into())
        );
        let bad = ReportError::invalid("wrap", "zig", "none, character, word");
        assert_eq!(
            bad.to_string(),
            "invalid wrap \"zig\" (expected none, character, word)"
        );
        assert_eq!(bad.kind(), ErrorKind::InvalidInput);
        let json = serde_json::from_str::<u8>("x").unwrap_err();
        let json = ReportError::from_json(json);
        assert!(matches!(json, ReportError::Json(_)));
        assert_eq!(json.kind(), ErrorKind::Internal);
    }

    #[test]
    fn json_io_errors_stay_io() {
        struct Broken;
        impl std::io::Write for Broken {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let e = serde_json::to_writer(Broken, &[1, 2, 3]).unwrap_err();
        match ReportError::from_json(e) {
            ReportError::Io(io) => assert_eq!(io.kind(), std::io::ErrorKind::BrokenPipe),
            other => panic!("{other:?}"),
        }
    }
}
