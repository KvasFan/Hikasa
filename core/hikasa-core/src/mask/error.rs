//! [`MaskError`].

use crate::error::{Classify, ErrorKind};

/// Mask syntax errors. `mask` is the item after trimming and NFC normalisation (which looks the same as the item
/// as typed), and the position `at` is a char index into it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MaskError {
    /// `[` without a matching `]`.
    #[error("mask {mask:?}: unclosed '[' at {at}")]
    UnclosedBracket {
        /// The item.
        mask: String,
        /// Char index of the `[`.
        at: usize,
    },
    /// `[z-a]`.
    #[error("mask {mask:?}: invalid range {lo:?}-{hi:?} at {at}")]
    InvalidRange {
        /// The item.
        mask: String,
        /// Char index of the range's first char.
        at: usize,
        /// Lower bound as typed.
        lo: char,
        /// Upper bound as typed.
        hi: char,
    },
    /// Empty path component (`a\\b`), an empty item, or a mask that is only `-`.
    #[error("mask {mask:?}: empty component")]
    EmptyComponent {
        /// The item.
        mask: String,
    },
    /// `.` used anywhere except as the first component; `..` anywhere.
    #[error("mask {mask:?}: '.' or '..' component is only allowed as a leading '.'")]
    DotComponent {
        /// The item.
        mask: String,
    },
}

impl Classify for MaskError {
    /// Always `invalid_input`.
    fn kind(&self) -> ErrorKind {
        match self {
            MaskError::UnclosedBracket { .. }
            | MaskError::InvalidRange { .. }
            | MaskError::EmptyComponent { .. }
            | MaskError::DotComponent { .. } => ErrorKind::InvalidInput,
        }
    }
}
