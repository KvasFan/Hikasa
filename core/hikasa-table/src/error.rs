//! [`TableError`], [`TableWarning`] and [`TableWarningCode`] (contract T-9).

use std::fmt;

use hikasa_core::{Cancelled, Classify, ErrorKind, Side};
use hikasa_vfs::VfsError;
use serde::{Deserialize, Serialize};

/// Failure of a load or a comparison (T-9). Every variant classifies to an [`ErrorKind`]; the CLI exits 100.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TableError {
    /// The VFS could not stat or read an input.
    #[error("{source}")]
    Load {
        /// Side of the comparison (`Left` when the caller does not know it, see [`TableError::with_side`]).
        side: Side,
        /// The VFS error.
        #[source]
        source: VfsError,
    },
    /// The workbook is password-protected (an encrypted OOXML package or a BIFF `FILEPASS` record).
    #[error("{name}: the workbook is password-protected; Hikasa does not decrypt workbooks")]
    Encrypted {
        /// Input name.
        name: String,
    },
    /// The input is not a readable table or workbook (damaged, truncated, an unknown format).
    #[error("{name}: {message}")]
    Corrupt {
        /// Input name.
        name: String,
        /// What the reader reported.
        message: String,
    },
    /// The input exceeds a documented limit (`max_bytes`, `max_cells`, `u32` offsets).
    #[error("{name}: {what} exceeds the limit of {limit}")]
    TooLarge {
        /// Input name.
        name: String,
        /// What is too large ("bytes", "cells", …).
        what: &'static str,
        /// The limit.
        limit: u64,
    },
    /// Invalid options or a vocabulary word that names nothing (a column, a sheet).
    #[error("{0}")]
    InvalidInput(String),
    /// The alignment engine failed (input too large for `hikasa-diff`).
    #[error("row alignment failed: {0}")]
    Align(String),
    /// Cancelled (never a partial result).
    #[error("cancelled")]
    Cancelled,
}

impl TableError {
    /// Sets the side of a [`TableError::Load`] (other variants are returned unchanged).
    pub fn with_side(self, side: Side) -> TableError {
        match self {
            TableError::Load { source, .. } => TableError::Load { side, source },
            other => other,
        }
    }

    /// True for [`TableError::Cancelled`].
    pub fn is_cancelled(&self) -> bool {
        matches!(self, TableError::Cancelled)
    }
}

impl From<Cancelled> for TableError {
    fn from(_: Cancelled) -> TableError {
        TableError::Cancelled
    }
}

impl Classify for TableError {
    fn kind(&self) -> ErrorKind {
        match self {
            TableError::Load { source, .. } => source.kind(),
            TableError::Encrypted { .. } => ErrorKind::Unsupported,
            TableError::Corrupt { .. } => ErrorKind::InvalidData,
            TableError::TooLarge { .. } => ErrorKind::TooLarge,
            TableError::InvalidInput(_) => ErrorKind::InvalidInput,
            TableError::Align(_) => ErrorKind::TooLarge,
            TableError::Cancelled => ErrorKind::Cancelled,
        }
    }
}

/// Codes of the table warnings. Phase 2 adds the same names to `hikasa_core::WarningCode` (contract §6.2); the
/// snake_case names are the stable codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TableWarningCode {
    /// Decoding replaced malformed byte sequences with U+FFFD.
    LossyDecode,
    /// A qualified field was not closed before the end of the data.
    UnterminatedQualifier,
    /// Characters followed a closing qualifier before the next delimiter (kept as data).
    StrayAfterQualifier,
    /// A sheet of a workbook was skipped (a chart or macro sheet, or one the reader cannot read).
    SheetSkipped,
    /// Keys occur more than once on a side; duplicates paired in their order (T-8).
    DuplicateKeys,
    /// Standard alignment ran out of its fallback budget (from `hikasa-diff`).
    AlignmentDegraded,
    /// Standard alignment ran out of its work budget.
    WorkBudgetExhausted,
    /// The closeness budget ran out.
    ClosenessBudgetExhausted,
    /// Closeness does not apply to the chosen algorithm.
    ClosenessIgnored,
    /// A cell's formula has no cached value (compared as empty).
    FormulaWithoutValue,
}

impl TableWarningCode {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            TableWarningCode::LossyDecode => "lossy_decode",
            TableWarningCode::UnterminatedQualifier => "unterminated_qualifier",
            TableWarningCode::StrayAfterQualifier => "stray_after_qualifier",
            TableWarningCode::SheetSkipped => "sheet_skipped",
            TableWarningCode::DuplicateKeys => "duplicate_keys",
            TableWarningCode::AlignmentDegraded => "alignment_degraded",
            TableWarningCode::WorkBudgetExhausted => "work_budget_exhausted",
            TableWarningCode::ClosenessBudgetExhausted => "closeness_budget_exhausted",
            TableWarningCode::ClosenessIgnored => "closeness_ignored",
            TableWarningCode::FormulaWithoutValue => "formula_without_value",
        }
    }
}

impl fmt::Display for TableWarningCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A non-fatal observation of a load or a comparison (T8: core never logs).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableWarning {
    /// Stable code.
    pub code: TableWarningCode,
    /// Message.
    pub message: String,
    /// Side concerned, when one.
    pub side: Option<Side>,
    /// Sheet concerned, when one.
    pub sheet: Option<String>,
}

impl TableWarning {
    /// A warning without side or sheet.
    pub fn new(code: TableWarningCode, message: impl Into<String>) -> TableWarning {
        TableWarning {
            code,
            message: message.into(),
            side: None,
            sheet: None,
        }
    }

    /// Sets the sheet.
    pub fn in_sheet(mut self, sheet: &str) -> TableWarning {
        self.sheet = Some(sheet.to_owned());
        self
    }
}
