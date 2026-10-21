//! The error type of the crate: [`TextError`] (§1.5.6, §5.5).

use hikasa_core::cancel::Cancelled;
use hikasa_core::error::{Classify, ErrorKind};
use hikasa_core::schema::SchemaError;
use hikasa_core::side::Side;
use hikasa_diff::DiffError;
use hikasa_formats::{ConvertError, FormatError};
use hikasa_vfs::VfsError;

/// Text errors.
///
/// `Classify` (§5.5 rule 4): `Load` → the VFS error's kind; `TooLarge` and `TooManyLines` → `too_large`;
/// `Format` and `Convert` → their own kinds; `Rules` → `invalid_input`; `Schema` → `unsupported` (a rules file
/// written by a newer version, as [`SchemaError`] itself); `Diff` → its kind; `Cancelled` → `cancelled`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TextError {
    /// The input could not be read (`Side` implements `Display`, §1.1.2).
    #[error("{side} input: {source}")]
    Load {
        /// The side whose input failed. [`crate::load_document`] does not know the side and reports
        /// `Side::Left`; a caller loading another side applies [`TextError::with_side`].
        side: Side,
        /// The VFS failure.
        #[source]
        source: VfsError,
    },
    /// Document over `max_bytes` (or decoded text over 4 GiB − 1 bytes, D-43).
    #[error("{spec}: larger than {limit} bytes")]
    TooLarge {
        /// Canonical spec (or `<memory>` for in-memory documents).
        spec: String,
        /// The limit in bytes.
        limit: u64,
    },
    /// Document with more than `MAX_LINES` lines (D-43).
    #[error("{spec}: {lines} lines exceed the limit of {limit}")]
    TooManyLines {
        /// Canonical spec (or `<memory>` for in-memory documents).
        spec: String,
        /// Lines found.
        lines: u64,
        /// The limit ([`crate::MAX_LINES`]).
        limit: u64,
    },
    /// Format problem (compile, unknown encoding).
    #[error(transparent)]
    Format(#[from] FormatError),
    /// External conversion failed.
    #[error(transparent)]
    Convert(#[from] ConvertError),
    /// Invalid rules (range, pattern, vocabulary word, TOML syntax).
    #[error("rules: {0}")]
    Rules(String),
    /// A rules file written by a newer version (SES-052). Additive variant (wave 3): `TextRules::from_toml`
    /// reports a newer `schema_version` with it, so that it classifies as `unsupported` like every other
    /// newer-schema file.
    #[error("rules: {0}")]
    Schema(#[source] SchemaError),
    /// Alignment error.
    #[error(transparent)]
    Diff(DiffError),
    /// Cancelled.
    #[error("operation cancelled")]
    Cancelled,
}

impl TextError {
    /// Sets the side of a [`TextError::Load`] error (other variants are returned unchanged). Used by callers of
    /// [`crate::load_document`], which does not know which side it loads.
    pub fn with_side(self, side: Side) -> TextError {
        match self {
            TextError::Load { source, .. } => TextError::Load { side, source },
            other => other,
        }
    }

    /// True when the error is a cancellation.
    pub fn is_cancelled(&self) -> bool {
        self.kind() == ErrorKind::Cancelled
    }
}

impl From<Cancelled> for TextError {
    fn from(_: Cancelled) -> TextError {
        TextError::Cancelled
    }
}

impl From<DiffError> for TextError {
    /// `DiffError::Cancelled` becomes [`TextError::Cancelled`] (cancellation is never an error state, T3);
    /// every other diff error is wrapped.
    fn from(error: DiffError) -> TextError {
        match error {
            DiffError::Cancelled => TextError::Cancelled,
            other => TextError::Diff(other),
        }
    }
}

impl Classify for TextError {
    fn kind(&self) -> ErrorKind {
        match self {
            TextError::Load { source, .. } => source.kind(),
            TextError::TooLarge { .. } | TextError::TooManyLines { .. } => ErrorKind::TooLarge,
            TextError::Format(e) => e.kind(),
            TextError::Convert(e) => e.kind(),
            TextError::Rules(_) => ErrorKind::InvalidInput,
            TextError::Schema(e) => e.kind(),
            TextError::Diff(e) => e.kind(),
            TextError::Cancelled => ErrorKind::Cancelled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        let not_found = TextError::Load {
            side: Side::Left,
            source: VfsError::NotFound { spec: "/x".into() },
        };
        assert_eq!(not_found.kind(), ErrorKind::NotFound);
        assert_eq!(not_found.to_string(), "left input: /x: not found");
        let right = not_found.with_side(Side::Right);
        assert_eq!(right.to_string(), "right input: /x: not found");
        assert_eq!(
            TextError::TooLarge {
                spec: "a".into(),
                limit: 1
            }
            .kind(),
            ErrorKind::TooLarge
        );
        assert_eq!(
            TextError::TooManyLines {
                spec: "a".into(),
                lines: 3,
                limit: 2
            }
            .kind(),
            ErrorKind::TooLarge
        );
        assert_eq!(TextError::Rules("x".into()).kind(), ErrorKind::InvalidInput);
        assert_eq!(
            TextError::Schema(SchemaError {
                found: 2,
                supported: 1
            })
            .kind(),
            ErrorKind::Unsupported
        );
        assert_eq!(
            TextError::from(DiffError::Cancelled).kind(),
            ErrorKind::Cancelled
        );
        assert!(TextError::from(DiffError::Cancelled).is_cancelled());
        assert_eq!(
            TextError::from(DiffError::InputTooLarge { tokens: 9 }).kind(),
            ErrorKind::TooLarge
        );
        assert_eq!(TextError::from(Cancelled).kind(), ErrorKind::Cancelled);
        assert_eq!(
            TextError::from(FormatError::UnknownEncoding { label: "x".into() }).kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            TextError::from(ConvertError::Cancelled).kind(),
            ErrorKind::Cancelled
        );
    }
}
