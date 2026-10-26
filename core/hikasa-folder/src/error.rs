//! [`FolderError`]: failures of a whole folder comparison (T3, §5.5).
//!
//! Per-item failures (a folder that cannot be listed, an entry that cannot be stat'ed, a content test that fails)
//! are never errors of the comparison: they become node states (`Error`, with [`crate::NodeError`]s) and
//! warnings (ARCH-101).

use hikasa_core::{Cancelled, Classify, ErrorKind, MaskError, SchemaError, Side};
use hikasa_vfs::VfsError;

/// Folder errors (only failures of the whole comparison; per-node failures are node states).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FolderError {
    /// A base folder cannot be stat'ed or listed (exit 107).
    #[error("{side} base folder {spec}: {source}")]
    BaseNotLoadable {
        /// The side of the base.
        side: Side,
        /// Canonical spec of the base.
        spec: String,
        /// The VFS failure.
        #[source]
        source: VfsError,
    },
    /// A base spec is not a folder (exit 107).
    #[error("{side} base {spec} is not a folder")]
    BaseNotFolder {
        /// The side of the base.
        side: Side,
        /// Canonical spec of the base.
        spec: String,
    },
    /// Invalid filters.
    #[error("filters: {0}")]
    Filter(#[from] MaskError),
    /// Invalid rules, filter expression or criteria word.
    #[error("rules: {0}")]
    InvalidRules(String),
    /// `ContentMethod::Rules` without a rules comparator (configuration bug).
    #[error("rules-based content comparison requested but no comparator was provided")]
    RulesComparatorMissing,
    /// `compare_into` was called on a model that was already scanned.
    #[error("this folder model was already scanned")]
    AlreadyScanned,
    /// Cancelled before the base folders were loaded.
    #[error("operation cancelled")]
    Cancelled,
    /// A rules file written by a newer version of Hikasa (`schema_version` newer than
    /// [`crate::FOLDER_RULES_SCHEMA_VERSION`], SES-052, D-31). Additive variant (see the crate documentation).
    #[error("folder rules: {0}")]
    Schema(#[from] SchemaError),
}

impl From<Cancelled> for FolderError {
    fn from(_: Cancelled) -> FolderError {
        FolderError::Cancelled
    }
}

impl Classify for FolderError {
    /// `BaseNotLoadable` → the VFS error's kind; `BaseNotFolder` → `not_a_directory`; `Filter`,
    /// `InvalidRules` → `invalid_input`; `RulesComparatorMissing`, `AlreadyScanned` → `internal` (wiring bugs);
    /// `Cancelled` → `cancelled`; `Schema` → `unsupported`.
    fn kind(&self) -> ErrorKind {
        match self {
            FolderError::BaseNotLoadable { source, .. } => source.kind(),
            FolderError::BaseNotFolder { .. } => ErrorKind::NotADirectory,
            FolderError::Filter(e) => e.kind(),
            FolderError::InvalidRules(_) => ErrorKind::InvalidInput,
            FolderError::RulesComparatorMissing | FolderError::AlreadyScanned => {
                ErrorKind::Internal
            }
            FolderError::Cancelled => ErrorKind::Cancelled,
            FolderError::Schema(e) => e.kind(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_messages() {
        let base = FolderError::BaseNotLoadable {
            side: Side::Left,
            spec: "/a".into(),
            source: VfsError::NotFound { spec: "/a".into() },
        };
        assert_eq!(base.kind(), ErrorKind::NotFound);
        assert_eq!(base.to_string(), "left base folder /a: /a: not found");
        let not_folder = FolderError::BaseNotFolder {
            side: Side::Right,
            spec: "/b".into(),
        };
        assert_eq!(not_folder.kind(), ErrorKind::NotADirectory);
        assert_eq!(not_folder.to_string(), "right base /b is not a folder");
        assert_eq!(
            FolderError::InvalidRules("x".into()).kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            FolderError::RulesComparatorMissing.kind(),
            ErrorKind::Internal
        );
        assert_eq!(FolderError::AlreadyScanned.kind(), ErrorKind::Internal);
        assert_eq!(FolderError::from(Cancelled).kind(), ErrorKind::Cancelled);
        let schema = FolderError::from(SchemaError {
            found: 9,
            supported: 1,
        });
        assert_eq!(schema.kind(), ErrorKind::Unsupported);
        let mask = hikasa_core::Mask::parse(
            "[a",
            hikasa_core::MaskOptions {
                case: hikasa_core::CaseSensitivity::Sensitive,
            },
        )
        .unwrap_err();
        assert_eq!(FolderError::from(mask).kind(), ErrorKind::InvalidInput);
    }
}
