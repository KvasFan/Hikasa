//! The `schema_version` check of every TOML loader (SES-052, D-31).

use crate::error::{Classify, ErrorKind};

/// Outcome of a schema check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaCompat {
    /// Same version.
    Current,
    /// Older file; migrate in memory (SES-052).
    Older {
        /// The version found in the file.
        found: u32,
    },
}

/// Newer-than-supported file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("schema_version {found} is newer than the supported version {supported}")]
pub struct SchemaError {
    /// The version found in the file.
    pub found: u32,
    /// The newest version this build reads.
    pub supported: u32,
}

impl Classify for SchemaError {
    /// Always `unsupported`: the file is from a newer version of Hikasa.
    fn kind(&self) -> ErrorKind {
        ErrorKind::Unsupported
    }
}

/// SES-052 check used by every TOML loader: equal → `Current`, older → `Older`, newer → `SchemaError`.
pub const fn check_schema_version(found: u32, supported: u32) -> Result<SchemaCompat, SchemaError> {
    if found == supported {
        Ok(SchemaCompat::Current)
    } else if found < supported {
        Ok(SchemaCompat::Older { found })
    } else {
        Err(SchemaError { found, supported })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_checks() {
        assert_eq!(check_schema_version(1, 1), Ok(SchemaCompat::Current));
        assert_eq!(
            check_schema_version(0, 2),
            Ok(SchemaCompat::Older { found: 0 })
        );
        let err = check_schema_version(3, 2).unwrap_err();
        assert_eq!(
            err,
            SchemaError {
                found: 3,
                supported: 2
            }
        );
        assert_eq!(
            err.to_string(),
            "schema_version 3 is newer than the supported version 2"
        );
        assert_eq!(err.kind(), ErrorKind::Unsupported);
    }
}
