//! [`SessionError`]: failures of configuration loading (§1.8.2, §5.5).

use std::path::PathBuf;

use hikasa_core::{Classify, ErrorKind, SchemaError, io_error_kind};
use hikasa_formats::FormatError;

/// Session-level errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// No configuration folder can be determined.
    #[error("cannot determine the configuration folder (no home directory)")]
    NoConfigDir,
    /// `--config-dir` is relative and the current directory is unknown.
    #[error("--config-dir {0:?} must be absolute")]
    RelativeConfigDir(PathBuf),
    /// `--config-dir ""` (an unset shell variable): never the current directory. Added in the wave-4 review
    /// (kind `invalid_input`).
    #[error("--config-dir must not be empty")]
    EmptyConfigDir,
    /// Config file problem.
    #[error("{path}: {message}")]
    Config {
        /// The file.
        path: String,
        /// What is wrong (TOML syntax, a value of the wrong type or range).
        message: String,
    },
    /// Format problem.
    #[error(transparent)]
    Format(#[from] FormatError),
    /// I/O.
    #[error("{path}: {source}")]
    Io {
        /// The file or folder.
        path: String,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// A configuration file written by a newer version (`schema_version` newer than supported; D-31, SES-052).
    /// Additive variant (wave 4): it classifies as `unsupported`, like `SchemaError` itself.
    #[error("{path}: {source}")]
    Schema {
        /// The file.
        path: String,
        /// The version check that failed.
        #[source]
        source: SchemaError,
    },
}

impl Classify for SessionError {
    /// `NoConfigDir` → `not_found`; `RelativeConfigDir`, `EmptyConfigDir` → `invalid_input`; `Config` → `invalid_data`; `Format` →
    /// its kind; `Io` → [`io_error_kind`]; `Schema` → `unsupported`.
    fn kind(&self) -> ErrorKind {
        match self {
            SessionError::NoConfigDir => ErrorKind::NotFound,
            SessionError::RelativeConfigDir(_) | SessionError::EmptyConfigDir => {
                ErrorKind::InvalidInput
            }
            SessionError::Config { .. } => ErrorKind::InvalidData,
            SessionError::Format(e) => e.kind(),
            SessionError::Io { source, .. } => io_error_kind(source),
            SessionError::Schema { source, .. } => source.kind(),
        }
    }
}

impl SessionError {
    /// An [`SessionError::Io`] for `path`.
    pub(crate) fn io(path: &std::path::Path, source: std::io::Error) -> SessionError {
        SessionError::Io {
            path: path.display().to_string(),
            source,
        }
    }
}
