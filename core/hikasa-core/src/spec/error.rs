//! [`SpecError`].

use crate::error::{Classify, ErrorKind};

/// Spec parsing errors (all carry the offending input where one exists; messages are stable, see §6.2.1).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SpecError {
    /// Empty or whitespace-only input.
    #[error("empty spec")]
    Empty,
    /// The input contains a NUL character.
    #[error("spec contains a NUL character")]
    Nul,
    /// Relative path without a known current directory.
    #[error("relative path {path:?} needs a current directory")]
    RelativeWithoutCwd {
        /// The relative path as typed.
        path: String,
    },
    /// `~` without a known home directory.
    #[error("'~' needs a home directory")]
    HomeUnknown,
    /// `~user/…` is not supported.
    #[error("'~user' expansion is not supported: {input:?}")]
    TildeUser {
        /// The input as typed.
        input: String,
    },
    /// `scheme://` with an unknown scheme (D-05).
    #[error("unknown URL scheme {scheme:?}")]
    UnknownScheme {
        /// The scheme as typed.
        scheme: String,
    },
    /// Malformed URL authority or port.
    #[error("malformed URL {input:?}: {reason}")]
    MalformedUrl {
        /// The input as typed, with a URL password replaced by `***` (secrets never appear in messages).
        input: String,
        /// Stable reason ("invalid port", "empty host", …).
        reason: &'static str,
    },
    /// `profile:` without a name.
    #[error("malformed profile spec {input:?}")]
    MalformedProfile {
        /// The input as typed.
        input: String,
    },
    /// Windows `C:foo` with a current directory on another drive (or none).
    #[error("drive-relative path {path:?} cannot be resolved")]
    DriveRelative {
        /// The path as typed.
        path: String,
    },
    /// Windows device namespace (`\\.\…`).
    #[error("device path {path:?} is not supported")]
    DevicePath {
        /// The path as typed.
        path: String,
    },
    /// UNC path without a share (`\\server`).
    #[error("UNC path {path:?} lacks a share name")]
    UncWithoutShare {
        /// The path as typed.
        path: String,
    },
    /// A component that `LocalPath::join_name` rejects.
    #[error("invalid file name component {name:?}")]
    InvalidComponent {
        /// The rejected component.
        name: String,
    },
    /// `LocalPath::new` on a relative or non-normalised path, or a current/home directory that is not an
    /// absolute path.
    #[error("not an absolute normalised path: {path:?}")]
    NotNormalized {
        /// The rejected path.
        path: String,
    },
}

impl Classify for SpecError {
    /// `DevicePath` and `TildeUser` are `unsupported`; every other variant is `invalid_input`.
    fn kind(&self) -> ErrorKind {
        match self {
            SpecError::DevicePath { .. } | SpecError::TildeUser { .. } => ErrorKind::Unsupported,
            SpecError::Empty
            | SpecError::Nul
            | SpecError::RelativeWithoutCwd { .. }
            | SpecError::HomeUnknown
            | SpecError::UnknownScheme { .. }
            | SpecError::MalformedUrl { .. }
            | SpecError::MalformedProfile { .. }
            | SpecError::DriveRelative { .. }
            | SpecError::UncWithoutShare { .. }
            | SpecError::InvalidComponent { .. }
            | SpecError::NotNormalized { .. } => ErrorKind::InvalidInput,
        }
    }
}
