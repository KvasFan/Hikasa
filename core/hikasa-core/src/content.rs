//! The injected content-comparison seam (FC-025, FC-029).
//!
//! `hikasa-folder` runs content tests through [`ContentComparator`] trait objects and reads content through
//! [`ContentSource`]; `hikasa-vfs` implements the CRC and binary comparators and `hikasa-text` the rules-based
//! one, so the folder engine never depends on the text engine (ARCH-010/011).

use std::fmt;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::cancel::{CancelToken, Cancelled};
use crate::error::{Classify, ErrorKind, ErrorReport, io_error_kind};
use crate::progress::Ctl;
use crate::warning::Warning;

/// Content test method (FC-025).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentMethod {
    /// No content test.
    None,
    /// CRC32 (archive-provided values are used as they are).
    Crc,
    /// Byte-by-byte.
    Binary,
    /// Through the file format engine (ignores unimportant differences, line endings, encoding).
    Rules,
}

impl ContentMethod {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            ContentMethod::None => "none",
            ContentMethod::Crc => "crc",
            ContentMethod::Binary => "binary",
            ContentMethod::Rules => "rules",
        }
    }
}

/// Result of a content comparison (FC-032 icon set). Ordered from "most same" to "most different".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ContentResult {
    /// Byte-identical (or CRC-equal for the CRC method).
    BinarySame,
    /// Bytes differ, but nothing differs once rules apply (encoding, line endings, …).
    RulesSame,
    /// Only unimportant differences ("similar").
    UnimportantOnly,
    /// At least one important difference.
    ImportantDifferences,
    /// Bytes differ (binary or CRC method, or binary fallback).
    BinaryDifferent,
    /// The comparison could not complete.
    Error {
        /// What failed.
        error: ErrorReport,
    },
}

impl ContentResult {
    /// True for `BinarySame`/`RulesSame`, and for `UnimportantOnly` when `ignore_unimportant`.
    pub fn is_same(&self, ignore_unimportant: bool) -> bool {
        match self {
            ContentResult::BinarySame | ContentResult::RulesSame => true,
            ContentResult::UnimportantOnly => ignore_unimportant,
            ContentResult::ImportantDifferences
            | ContentResult::BinaryDifferent
            | ContentResult::Error { .. } => false,
        }
    }

    /// The payload-free kind (O(1); used by compact models and the IPC `ViewResult`).
    pub fn kind(&self) -> ContentKind {
        match self {
            ContentResult::BinarySame => ContentKind::BinarySame,
            ContentResult::RulesSame => ContentKind::RulesSame,
            ContentResult::UnimportantOnly => ContentKind::UnimportantOnly,
            ContentResult::ImportantDifferences => ContentKind::ImportantDifferences,
            ContentResult::BinaryDifferent => ContentKind::BinaryDifferent,
            ContentResult::Error { .. } => ContentKind::Error,
        }
    }
}

/// `ContentResult` without the error payload. `Copy`; same variant names and serde names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    /// See [`ContentResult::BinarySame`].
    BinarySame,
    /// See [`ContentResult::RulesSame`].
    RulesSame,
    /// See [`ContentResult::UnimportantOnly`].
    UnimportantOnly,
    /// See [`ContentResult::ImportantDifferences`].
    ImportantDifferences,
    /// See [`ContentResult::BinaryDifferent`].
    BinaryDifferent,
    /// See [`ContentResult::Error`].
    Error,
}

impl ContentKind {
    /// Every kind, from "most same" to "most different".
    pub const ALL: [ContentKind; 6] = [
        ContentKind::BinarySame,
        ContentKind::RulesSame,
        ContentKind::UnimportantOnly,
        ContentKind::ImportantDifferences,
        ContentKind::BinaryDifferent,
        ContentKind::Error,
    ];

    /// Stable snake_case name (`"unimportant_only"`, …), identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            ContentKind::BinarySame => "binary_same",
            ContentKind::RulesSame => "rules_same",
            ContentKind::UnimportantOnly => "unimportant_only",
            ContentKind::ImportantDifferences => "important_differences",
            ContentKind::BinaryDifferent => "binary_different",
            ContentKind::Error => "error",
        }
    }

    /// Inverse of `as_str` (`None` for unknown names, e.g. from a newer peer).
    pub fn parse(s: &str) -> Option<ContentKind> {
        ContentKind::ALL.into_iter().find(|kind| kind.as_str() == s)
    }
}

impl fmt::Display for ContentKind {
    /// Writes [`ContentKind::as_str`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A content result plus the method that produced it (CRC-equal is not proof of identity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentOutcome {
    /// Method requested.
    pub method: ContentMethod,
    /// Result.
    pub result: ContentResult,
    /// Non-fatal notes (e.g. `fell_back_to_binary`).
    pub warnings: Vec<Warning>,
}

/// Read access to one file's content, independent of any VFS (implemented by hikasa-vfs, and by tests).
// `len` is "the length when known"; an `is_empty` companion would have no meaning here.
#[allow(clippy::len_without_is_empty)]
pub trait ContentSource: Send + Sync {
    /// File name used for format detection (no directories).
    fn name(&self) -> &str;
    /// Canonical spec string for messages.
    fn display(&self) -> String;
    /// Length in bytes when known.
    fn len(&self) -> Option<u64>;
    /// CRC32 known without reading (archives, snapshots).
    fn known_crc32(&self) -> Option<u32> {
        None
    }
    /// Local path when the content is a local regular file (enables mmap in hikasa-vfs).
    fn local_path(&self) -> Option<&Path> {
        None
    }
    /// Opens a fresh sequential reader at offset 0.
    fn open(&self) -> std::io::Result<Box<dyn Read + Send + '_>>;
    /// Reads the whole content into an exact-size buffer. Checks `cancel` at least every 1 MiB read.
    fn read_all(&self, limit: u64, cancel: &CancelToken) -> Result<Vec<u8>, ReadError>;
}

/// Failure of `ContentSource::read_all`. `Classify`: TooLarge → `too_large`, Io → `io_error_kind`,
/// Cancelled → `cancelled`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReadError {
    /// The content is larger than the caller's limit.
    #[error("content larger than {limit} bytes")]
    TooLarge {
        /// The caller's limit in bytes.
        limit: u64,
    },
    /// I/O failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// `cancel` fired.
    #[error("operation cancelled")]
    Cancelled,
}

impl From<Cancelled> for ReadError {
    fn from(_: Cancelled) -> ReadError {
        ReadError::Cancelled
    }
}

impl Classify for ReadError {
    fn kind(&self) -> ErrorKind {
        match self {
            ReadError::TooLarge { .. } => ErrorKind::TooLarge,
            ReadError::Io(e) => io_error_kind(e),
            ReadError::Cancelled => ErrorKind::Cancelled,
        }
    }
}

/// A content test. Implementations: hikasa-vfs `CrcComparator`, `BinaryComparator`;
/// hikasa-text `RulesComparator` (injected into hikasa-folder, which never depends on hikasa-text).
pub trait ContentComparator: Send + Sync {
    /// The method implemented.
    fn method(&self) -> ContentMethod;
    /// Largest file (bytes) this comparator loads completely; larger files are handled by a documented
    /// fallback (`RulesComparator`: byte comparison, D-25). `None` = streams, no limit.
    fn size_limit(&self) -> Option<u64> {
        None
    }
    /// Compares two sources. Cancellation is not a result: it returns `Err(Cancelled)`.
    fn compare(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        ctl: &Ctl<'_>,
    ) -> Result<ContentOutcome, Cancelled>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    fn error_result() -> ContentResult {
        ContentResult::Error {
            error: ErrorReport {
                kind: ErrorKind::NotFound,
                message: "gone".into(),
                spec: None,
                causes: vec![],
            },
        }
    }

    #[test]
    fn sameness() {
        assert!(ContentResult::BinarySame.is_same(false));
        assert!(ContentResult::RulesSame.is_same(false));
        assert!(!ContentResult::UnimportantOnly.is_same(false));
        assert!(ContentResult::UnimportantOnly.is_same(true));
        assert!(!ContentResult::ImportantDifferences.is_same(true));
        assert!(!ContentResult::BinaryDifferent.is_same(true));
        assert!(!error_result().is_same(true));
    }

    #[test]
    fn kinds_and_names() {
        let results = [
            ContentResult::BinarySame,
            ContentResult::RulesSame,
            ContentResult::UnimportantOnly,
            ContentResult::ImportantDifferences,
            ContentResult::BinaryDifferent,
            error_result(),
        ];
        for (result, kind) in results.iter().zip(ContentKind::ALL) {
            assert_eq!(result.kind(), kind);
            assert_eq!(ContentKind::parse(kind.as_str()), Some(kind));
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{}\"", kind.as_str())
            );
            let tagged = serde_json::to_value(result).unwrap();
            assert_eq!(tagged["result"], kind.as_str());
        }
        assert_eq!(ContentKind::parse("similar"), None);
        assert!(
            ContentKind::BinarySame < ContentKind::Error,
            "ordered from most same to most different"
        );
    }

    #[test]
    fn read_error_kinds() {
        assert_eq!(ReadError::TooLarge { limit: 1 }.kind(), ErrorKind::TooLarge);
        assert_eq!(ReadError::Cancelled.kind(), ErrorKind::Cancelled);
        assert_eq!(ReadError::from(Cancelled).kind(), ErrorKind::Cancelled);
        let io = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(ReadError::from(io).kind(), ErrorKind::PermissionDenied);
        assert_eq!(ContentMethod::Rules.as_str(), "rules");
    }
}
