//! The error types of the crate: [`FormatError`], [`PatternError`] and [`ConvertError`] (§1.4.7, §5.5).

use hikasa_core::error::{Classify, ErrorKind, io_error_kind};
use hikasa_core::mask::MaskError;
use hikasa_core::schema::SchemaError;

/// Format errors (all carry the format id or file and a precise location).
///
/// `Clone` lets the registry hand out a cached compile error more than once (the per-entry `OnceLock`).
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum FormatError {
    /// TOML syntax or type error.
    #[error("{origin}: {message}")]
    Toml {
        /// File name or `built-in:<file>`.
        origin: String,
        /// The parser's message (with line and column).
        message: String,
    },
    /// SES-052: the file was written by a newer version.
    #[error("{origin}: {source}")]
    Schema {
        /// File name or `built-in:<file>`.
        origin: String,
        /// The version check that failed.
        #[source]
        source: SchemaError,
    },
    /// Invalid field value.
    #[error("format {id}: {field}: {reason}")]
    Invalid {
        /// Format id.
        id: String,
        /// Field path (`grammar[3].to_column`, `text.tab_size`, …).
        field: String,
        /// What is wrong.
        reason: String,
    },
    /// Pattern compile error.
    #[error("format {id}: {location}: {source}")]
    Pattern {
        /// Format id.
        id: String,
        /// Field path of the pattern (`grammar[2].text`, `line_weight[0].pattern`, …).
        location: String,
        /// Both engines' messages.
        #[source]
        source: PatternError,
    },
    /// Mask syntax error.
    #[error("format {id}: mask: {source}")]
    Mask {
        /// Format id.
        id: String,
        /// The mask engine's error.
        #[source]
        source: MaskError,
    },
    /// Two entries with the same id.
    #[error("duplicate format id {id}")]
    DuplicateId {
        /// The id.
        id: String,
    },
    /// Unknown encoding label.
    #[error("unknown encoding {label:?}")]
    UnknownEncoding {
        /// The label as given.
        label: String,
    },
    /// Unknown id or name (`--format`).
    #[error("unknown format {name:?}")]
    UnknownFormat {
        /// The id or name as given.
        name: String,
    },
}

impl Classify for FormatError {
    /// `invalid_data` for broken format definitions; `invalid_input` for an unknown format or encoding (§5.5);
    /// `unsupported` for a format file written by a newer version (as [`SchemaError`] itself).
    fn kind(&self) -> ErrorKind {
        match self {
            FormatError::Toml { .. }
            | FormatError::Invalid { .. }
            | FormatError::Pattern { .. }
            | FormatError::Mask { .. }
            | FormatError::DuplicateId { .. } => ErrorKind::InvalidData,
            FormatError::Schema { source, .. } => source.kind(),
            FormatError::UnknownEncoding { .. } | FormatError::UnknownFormat { .. } => {
                ErrorKind::InvalidInput
            }
        }
    }
}

/// Pattern compile/runtime error (both engines' messages kept).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("pattern {pattern:?}: {message}")]
pub struct PatternError {
    /// The pattern as written.
    pub pattern: String,
    /// `"<regex message>; <fancy-regex message>"` for compile errors, the runtime message otherwise.
    pub message: String,
    /// True for a run-time failure (fancy-regex backtracking limit), false for a compile error.
    pub runtime: bool,
}

impl Classify for PatternError {
    /// `invalid_input` for a pattern that does not compile; `invalid_data` for a run-time limit hit.
    fn kind(&self) -> ErrorKind {
        if self.runtime {
            ErrorKind::InvalidData
        } else {
            ErrorKind::InvalidInput
        }
    }
}

/// External conversion errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConvertError {
    /// Template syntax (unbalanced quote, unknown `%x`).
    #[error("command template: {0}")]
    Template(String),
    /// Program could not start.
    #[error("cannot start {program:?}: {source}")]
    Spawn {
        /// `argv[0]`.
        program: String,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// Non-zero exit status.
    #[error("{program:?} exited with status {status}")]
    Failed {
        /// `argv[0]`.
        program: String,
        /// Exit status (−1 when the program was ended by a signal).
        status: i32,
    },
    /// Exit 0 but the target is missing or empty.
    #[error("{program:?} produced no output")]
    EmptyOutput {
        /// `argv[0]`.
        program: String,
    },
    /// Killed after the timeout.
    #[error("{program:?} timed out after {secs} s")]
    Timeout {
        /// `argv[0]`.
        program: String,
        /// The timeout in seconds.
        secs: u32,
    },
    /// Temp-file I/O.
    #[error("temporary file: {0}")]
    Io(#[from] std::io::Error),
    /// Cancelled (child killed).
    #[error("operation cancelled")]
    Cancelled,
}

impl Classify for ConvertError {
    /// `invalid_input` for a bad template; the OS error's kind for spawn and temp-file failures; `io` for a
    /// failed or timed-out program; `invalid_data` for an empty output; `cancelled` for cancellation.
    fn kind(&self) -> ErrorKind {
        match self {
            ConvertError::Template(_) => ErrorKind::InvalidInput,
            ConvertError::Spawn { source, .. } | ConvertError::Io(source) => io_error_kind(source),
            ConvertError::Failed { .. } | ConvertError::Timeout { .. } => ErrorKind::Io,
            ConvertError::EmptyOutput { .. } => ErrorKind::InvalidData,
            ConvertError::Cancelled => ErrorKind::Cancelled,
        }
    }
}

impl From<hikasa_core::cancel::Cancelled> for ConvertError {
    fn from(_: hikasa_core::cancel::Cancelled) -> ConvertError {
        ConvertError::Cancelled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification() {
        let invalid = FormatError::Invalid {
            id: "x".into(),
            field: "text.tab_size".into(),
            reason: "must be 1..=32".into(),
        };
        assert_eq!(invalid.kind(), ErrorKind::InvalidData);
        assert_eq!(
            invalid.to_string(),
            "format x: text.tab_size: must be 1..=32"
        );
        assert_eq!(
            FormatError::UnknownFormat { name: "zz".into() }.kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            FormatError::UnknownEncoding {
                label: "cp437".into()
            }
            .kind(),
            ErrorKind::InvalidInput
        );
        let schema = FormatError::Schema {
            origin: "a.toml".into(),
            source: SchemaError {
                found: 9,
                supported: 1,
            },
        };
        assert_eq!(schema.kind(), ErrorKind::Unsupported);
        assert_eq!(
            schema.to_string(),
            "a.toml: schema_version 9 is newer than the supported version 1"
        );
        let pattern = PatternError {
            pattern: "(".into(),
            message: "a; b".into(),
            runtime: false,
        };
        assert_eq!(pattern.kind(), ErrorKind::InvalidInput);
        assert_eq!(pattern.to_string(), "pattern \"(\": a; b");
        assert_eq!(ConvertError::Cancelled.kind(), ErrorKind::Cancelled);
        assert_eq!(
            ConvertError::Template("x".into()).kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            ConvertError::Io(std::io::Error::from(std::io::ErrorKind::NotFound)).kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            ConvertError::from(hikasa_core::cancel::Cancelled).kind(),
            ErrorKind::Cancelled
        );
    }
}
