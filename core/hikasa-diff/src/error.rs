//! The crate error type ([`DiffError`]) and its [`Classify`] mapping (T3, §5.5).

use hikasa_core::{Cancelled, Classify, ErrorKind};

/// Diff errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DiffError {
    /// A sequence longer than [`MAX_SEQUENCE_LEN`](crate::MAX_SEQUENCE_LEN), or more than `i32::MAX - 1`
    /// distinct tokens.
    #[error("input too large: {tokens} tokens")]
    InputTooLarge {
        /// The offending number of tokens (sequence length or distinct-token count).
        tokens: u64,
    },
    /// Weights slice length mismatch (caller bug).
    #[error("weights length {got} does not match sequence length {expected}")]
    WeightsLength {
        /// Length of the token sequence.
        expected: usize,
        /// Length of the weights slice.
        got: usize,
    },
    /// Cancelled.
    #[error("operation cancelled")]
    Cancelled,
}

impl From<Cancelled> for DiffError {
    /// [`DiffError::Cancelled`].
    fn from(_: Cancelled) -> DiffError {
        DiffError::Cancelled
    }
}

impl Classify for DiffError {
    /// `InputTooLarge` → `too_large`; `WeightsLength` (a caller bug) → `internal`; `Cancelled` → `cancelled`.
    fn kind(&self) -> ErrorKind {
        match self {
            DiffError::InputTooLarge { .. } => ErrorKind::TooLarge,
            DiffError::WeightsLength { .. } => ErrorKind::Internal,
            DiffError::Cancelled => ErrorKind::Cancelled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_classifies() {
        assert_eq!(
            DiffError::InputTooLarge { tokens: 7 }.kind(),
            ErrorKind::TooLarge
        );
        assert_eq!(
            DiffError::WeightsLength {
                expected: 2,
                got: 1
            }
            .kind(),
            ErrorKind::Internal
        );
        assert_eq!(DiffError::Cancelled.kind(), ErrorKind::Cancelled);
        assert_eq!(DiffError::from(Cancelled), DiffError::Cancelled);
        assert_eq!(
            DiffError::InputTooLarge { tokens: 7 }.to_string(),
            "input too large: 7 tokens"
        );
        assert_eq!(
            DiffError::WeightsLength {
                expected: 2,
                got: 1
            }
            .to_string(),
            "weights length 1 does not match sequence length 2"
        );
        assert_eq!(DiffError::Cancelled.to_string(), "operation cancelled");
    }
}
