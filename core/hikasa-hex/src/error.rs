//! [`HexError`], [`HexWarning`], [`HexWarningCode`] and [`Changed`] (§1.12.22, D-345).

use std::fmt;

use hikasa_core::{Cancelled, Classify, ErrorKind};
use hikasa_vfs::VfsError;
use serde::{Deserialize, Serialize};

use crate::vocab::VocabError;

/// Failure of a hex operation. Every variant classifies to an [`ErrorKind`]; the CLI exits 100 with the side's
/// spec (RPT-004).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HexError {
    /// The VFS could not stat, open or read an input.
    #[error("{source}")]
    Io {
        /// Canonical spec of the input.
        spec: String,
        /// The VFS error.
        #[source]
        source: VfsError,
    },
    /// An input ended before the length it had when it was opened (a truncation during the job): never
    /// zero-filled bytes. Kind `unexpected_eof`.
    #[error(
        "{spec}: the file is shorter than when it was opened (truncated during the comparison)"
    )]
    Truncated {
        /// Canonical spec of the input.
        spec: String,
    },
    /// A word, address, pattern or option that names nothing.
    #[error(transparent)]
    Vocab(#[from] VocabError),
    /// A load conversion failed (the Service's converter; D-336).
    #[error("{spec}: {message}")]
    Convert {
        /// Canonical spec of the input.
        spec: String,
        /// What the converter reported.
        message: String,
        /// Its kind.
        kind: ErrorKind,
    },
    /// The block alignment refused its input (more blocks than `hikasa-diff` accepts). Kind `too_large`.
    #[error("block alignment failed: {0}")]
    Align(String),
    /// Cancelled (never a partial result).
    #[error("cancelled")]
    Cancelled,
}

impl HexError {
    /// The error of a VFS failure on `spec`: a truncation becomes [`HexError::Truncated`], a cancellation
    /// [`HexError::Cancelled`].
    pub fn from_vfs(spec: &str, error: VfsError) -> HexError {
        match error {
            VfsError::Cancelled => HexError::Cancelled,
            VfsError::Truncated { spec, .. } => HexError::Truncated { spec },
            e if e.is_cancelled() => HexError::Cancelled,
            source => HexError::Io {
                spec: source.spec().unwrap_or(spec).to_owned(),
                source,
            },
        }
    }

    /// The spec of the input the error is about (`None` for vocabulary errors and cancellation).
    pub fn spec(&self) -> Option<&str> {
        match self {
            HexError::Io { spec, .. }
            | HexError::Truncated { spec }
            | HexError::Convert { spec, .. } => Some(spec),
            HexError::Vocab(_) | HexError::Align(_) | HexError::Cancelled => None,
        }
    }

    /// True for [`HexError::Cancelled`].
    pub fn is_cancelled(&self) -> bool {
        matches!(self, HexError::Cancelled)
    }
}

impl From<hikasa_diff::DiffError> for HexError {
    fn from(error: hikasa_diff::DiffError) -> HexError {
        match error {
            hikasa_diff::DiffError::Cancelled => HexError::Cancelled,
            other => HexError::Align(other.to_string()),
        }
    }
}

impl From<Cancelled> for HexError {
    fn from(_: Cancelled) -> HexError {
        HexError::Cancelled
    }
}

impl Classify for HexError {
    fn kind(&self) -> ErrorKind {
        match self {
            HexError::Io { source, .. } => source.kind(),
            HexError::Truncated { .. } => ErrorKind::UnexpectedEof,
            HexError::Vocab(_) => ErrorKind::InvalidInput,
            HexError::Convert { kind, .. } => *kind,
            HexError::Align(_) => ErrorKind::TooLarge,
            HexError::Cancelled => ErrorKind::Cancelled,
        }
    }
}

/// Codes of the hex warnings; W12c adds the same names to `hikasa_core::WarningCode` (§1.12.22). The snake_case
/// names are the stable codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HexWarningCode {
    /// Complete alignment ran out of its byte budget: some gaps were laid out as Fast lays out large gaps.
    AlignmentDegraded,
    /// More sections than `max_sections`: the list was cut, the statistics stay exact.
    SectionsTruncated,
    /// An input's (length, modification time) changed during the job: the result may be stale.
    InputChanged,
}

impl HexWarningCode {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            HexWarningCode::AlignmentDegraded => "alignment_degraded",
            HexWarningCode::SectionsTruncated => "sections_truncated",
            HexWarningCode::InputChanged => "input_changed",
        }
    }
}

impl fmt::Display for HexWarningCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A non-fatal note of a hex operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HexWarning {
    /// Stable code.
    pub code: HexWarningCode,
    /// Message (English, stable for tests).
    pub message: String,
    /// The input concerned, if any.
    pub spec: Option<String>,
}

/// An input whose stamp changed since it was opened ([`crate::HexDocument::check_unchanged`]).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{spec}: the file changed on disk since it was opened")]
pub struct Changed {
    /// Canonical spec of the input.
    pub spec: String,
}

impl Changed {
    /// The `input_changed` warning for this change.
    pub fn warning(&self) -> HexWarning {
        HexWarning {
            code: HexWarningCode::InputChanged,
            message: self.to_string(),
            spec: Some(self.spec.clone()),
        }
    }
}
