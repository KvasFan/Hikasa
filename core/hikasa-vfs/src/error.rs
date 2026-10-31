//! [`VfsError`] and [`VfsOp`]: the error model of the VFS (T3, §5.5).
//!
//! `Classify` maps every variant to one [`ErrorKind`]; [`VfsError::Io`] uses [`io_error_kind`] on its source.
//! [`VfsError::from_io`] turns a `std::io::Error` into the most specific variant, and `From<VfsError> for
//! std::io::Error` goes the other way without losing the kind (for [`hikasa_core::ContentSource::open`], whose
//! signature is `std::io::Result`).

use std::io;

use hikasa_core::{Cancelled, Classify, ErrorKind, io_error_kind};
use serde::{Deserialize, Serialize};

/// VFS operation names (error context).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VfsOp {
    /// Reading metadata.
    Stat,
    /// Listing a folder.
    List,
    /// Opening a file for reading.
    Open,
    /// Reading content.
    Read,
    /// Opening or writing an output file.
    Write,
    /// Committing an atomic write.
    Commit,
    /// Memory-mapping a file.
    Map,
    /// Creating a folder (M2 W3).
    Mkdir,
    /// Removing an item (M2 W3).
    Remove,
    /// Renaming an item (M2 W3).
    Rename,
    /// Setting times (M2 W3).
    SetTimes,
    /// Setting attributes (M2 W3).
    SetAttrs,
    /// Creating a symbolic link (M2 W3).
    Symlink,
    /// Moving an item to the recycle bin (M2 W3).
    Trash,
}

impl VfsOp {
    /// Every operation, in declaration order.
    pub const ALL: [VfsOp; 14] = [
        VfsOp::Stat,
        VfsOp::List,
        VfsOp::Open,
        VfsOp::Read,
        VfsOp::Write,
        VfsOp::Commit,
        VfsOp::Map,
        VfsOp::Mkdir,
        VfsOp::Remove,
        VfsOp::Rename,
        VfsOp::SetTimes,
        VfsOp::SetAttrs,
        VfsOp::Symlink,
        VfsOp::Trash,
    ];

    /// Stable snake_case name, identical to the serde form (`"stat"`, …).
    pub const fn as_str(self) -> &'static str {
        match self {
            VfsOp::Stat => "stat",
            VfsOp::List => "list",
            VfsOp::Open => "open",
            VfsOp::Read => "read",
            VfsOp::Write => "write",
            VfsOp::Commit => "commit",
            VfsOp::Map => "map",
            VfsOp::Mkdir => "mkdir",
            VfsOp::Remove => "remove",
            VfsOp::Rename => "rename",
            VfsOp::SetTimes => "set_times",
            VfsOp::SetAttrs => "set_attrs",
            VfsOp::Symlink => "symlink",
            VfsOp::Trash => "trash",
        }
    }
}

/// VFS errors. `Classify` maps them to `ErrorKind` one-to-one by variant (`Io` via `io_error_kind`).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VfsError {
    /// Missing.
    #[error("{spec}: not found")]
    NotFound {
        /// Canonical spec.
        spec: String,
    },
    /// Permission denied.
    #[error("{spec}: permission denied")]
    PermissionDenied {
        /// Canonical spec.
        spec: String,
    },
    /// Expected a folder.
    #[error("{spec}: not a folder")]
    NotADirectory {
        /// Canonical spec.
        spec: String,
    },
    /// Expected a file.
    #[error("{spec}: is a folder")]
    IsADirectory {
        /// Canonical spec.
        spec: String,
    },
    /// Write target exists and `overwrite` was false.
    #[error("{spec}: already exists")]
    AlreadyExists {
        /// Canonical spec.
        spec: String,
    },
    /// Spec kind or operation unsupported in this version (D-05).
    #[error("{spec}: {what} is not supported in this version")]
    Unsupported {
        /// Canonical spec (never contains a password).
        spec: String,
        /// What is unsupported (stable text, e.g. `"ftp specs"`).
        what: &'static str,
    },
    /// Content larger than a caller-imposed limit.
    #[error("{spec}: larger than the limit of {limit} bytes")]
    TooLarge {
        /// Canonical spec.
        spec: String,
        /// The limit in bytes.
        limit: u64,
    },
    /// Invalid file name.
    #[error("invalid file name {name:?}")]
    InvalidName {
        /// The rejected name.
        name: String,
    },
    /// Other I/O failure.
    #[error("{spec}: {op:?} failed: {source}")]
    Io {
        /// Canonical spec.
        spec: String,
        /// The failed operation.
        op: VfsOp,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
    /// Cancelled during a VFS call.
    #[error("operation cancelled")]
    Cancelled,
    /// DOS attribute letters that are not in `A C D E H I L M N O P R S T U Z` (`DosAttrs::parse_letters`;
    /// additive variant, see "Additions to the M1 contract" in the crate documentation).
    #[error("invalid DOS attribute letters {input:?}")]
    InvalidAttributes {
        /// The rejected text.
        input: String,
    },
    /// A folder that must be empty is not (M2 W3, D-93).
    #[error("{spec}: folder is not empty")]
    NotEmpty {
        /// Canonical spec.
        spec: String,
    },
    /// A rename between two file systems (M2 W3, D-93); callers fall back to copy + remove (D-97).
    #[error("{from} -> {to}: cannot rename across file systems")]
    CrossDevice {
        /// Canonical spec of the source.
        from: String,
        /// Canonical spec of the target.
        to: String,
    },
    /// A mutating primitive this back-end (or this item) does not support (M2 W3, D-93).
    #[error("{spec}: {} is not supported here", op.as_str())]
    OperationUnsupported {
        /// Canonical spec.
        spec: String,
        /// The primitive.
        op: VfsOp,
    },
    /// A target an operation refuses (M2 W3, D-93). Reasons (stable text): `"is the source itself"`, `"is
    /// inside the source folder"`, `"is not an existing folder"`, `"needs a file name"`, `"is a root folder"`,
    /// `"changed since the plan was made"`.
    #[error("{spec}: {reason}")]
    InvalidTarget {
        /// Canonical spec.
        spec: String,
        /// Stable reason text.
        reason: &'static str,
    },
    /// A source that exists in principle but cannot be reached now, such as "no clipboard" (M2 W3, D-93).
    #[error("{spec}: {reason}")]
    Unavailable {
        /// Canonical spec.
        spec: String,
        /// Why.
        reason: String,
    },
}

impl VfsError {
    /// The most specific variant for an I/O failure of `op` on `spec`: not found, permission denied, not a
    /// folder, is a folder and already exists get their own variants; an error that wraps [`Cancelled`]
    /// becomes [`VfsError::Cancelled`]; an error that wraps a `VfsError` (see `From<VfsError> for
    /// std::io::Error`) is unwrapped; everything else is [`VfsError::Io`].
    pub fn from_io(spec: impl Into<String>, op: VfsOp, error: io::Error) -> VfsError {
        let error = match error.downcast::<VfsError>() {
            Ok(vfs) => return vfs,
            Err(error) => error,
        };
        if error.get_ref().is_some_and(|inner| inner.is::<Cancelled>()) {
            return VfsError::Cancelled;
        }
        let spec = spec.into();
        match error.kind() {
            io::ErrorKind::NotFound => VfsError::NotFound { spec },
            io::ErrorKind::PermissionDenied => VfsError::PermissionDenied { spec },
            io::ErrorKind::NotADirectory => VfsError::NotADirectory { spec },
            io::ErrorKind::IsADirectory => VfsError::IsADirectory { spec },
            io::ErrorKind::AlreadyExists => VfsError::AlreadyExists { spec },
            io::ErrorKind::DirectoryNotEmpty => VfsError::NotEmpty { spec },
            io::ErrorKind::CrossesDevices => VfsError::CrossDevice {
                from: spec.clone(),
                to: spec,
            },
            _ => VfsError::Io {
                spec,
                op,
                source: error,
            },
        }
    }

    /// The spec the error is about (`None` for `InvalidName`, `InvalidAttributes` and `Cancelled`; `from` for
    /// `CrossDevice`).
    pub fn spec(&self) -> Option<&str> {
        match self {
            VfsError::NotFound { spec }
            | VfsError::PermissionDenied { spec }
            | VfsError::NotADirectory { spec }
            | VfsError::IsADirectory { spec }
            | VfsError::AlreadyExists { spec }
            | VfsError::Unsupported { spec, .. }
            | VfsError::TooLarge { spec, .. }
            | VfsError::Io { spec, .. }
            | VfsError::NotEmpty { spec }
            | VfsError::OperationUnsupported { spec, .. }
            | VfsError::InvalidTarget { spec, .. }
            | VfsError::Unavailable { spec, .. } => Some(spec),
            VfsError::CrossDevice { from, .. } => Some(from),
            VfsError::InvalidName { .. }
            | VfsError::InvalidAttributes { .. }
            | VfsError::Cancelled => None,
        }
    }

    /// True when the error means cancellation (`Cancelled`, or an `Io` error that wraps [`Cancelled`]).
    pub fn is_cancelled(&self) -> bool {
        self.kind() == ErrorKind::Cancelled
    }
}

impl From<Cancelled> for VfsError {
    fn from(_: Cancelled) -> VfsError {
        VfsError::Cancelled
    }
}

impl Classify for VfsError {
    /// One kind per variant; `Io` → `io_error_kind(source)`.
    fn kind(&self) -> ErrorKind {
        match self {
            VfsError::NotFound { .. } => ErrorKind::NotFound,
            VfsError::PermissionDenied { .. } => ErrorKind::PermissionDenied,
            VfsError::NotADirectory { .. } => ErrorKind::NotADirectory,
            VfsError::IsADirectory { .. } => ErrorKind::IsADirectory,
            VfsError::AlreadyExists { .. } => ErrorKind::AlreadyExists,
            VfsError::Unsupported { .. } => ErrorKind::Unsupported,
            VfsError::TooLarge { .. } => ErrorKind::TooLarge,
            VfsError::InvalidName { .. } | VfsError::InvalidAttributes { .. } => {
                ErrorKind::InvalidInput
            }
            VfsError::Io { source, .. } => io_error_kind(source),
            VfsError::Cancelled => ErrorKind::Cancelled,
            VfsError::NotEmpty { .. } => ErrorKind::NotEmpty,
            VfsError::CrossDevice { .. }
            | VfsError::OperationUnsupported { .. }
            | VfsError::Unavailable { .. } => ErrorKind::Unsupported,
            VfsError::InvalidTarget { .. } => ErrorKind::InvalidInput,
        }
    }
}

impl From<VfsError> for io::Error {
    /// Wraps the error so that `io_error_kind` of the result equals `Classify::kind` of `error`, and
    /// [`VfsError::from_io`] recovers the original. `Cancelled` becomes an error that wraps [`Cancelled`].
    fn from(error: VfsError) -> io::Error {
        let kind = match &error {
            VfsError::Cancelled => return io::Error::other(Cancelled),
            VfsError::NotFound { .. } => io::ErrorKind::NotFound,
            VfsError::PermissionDenied { .. } => io::ErrorKind::PermissionDenied,
            VfsError::NotADirectory { .. } => io::ErrorKind::NotADirectory,
            VfsError::IsADirectory { .. } => io::ErrorKind::IsADirectory,
            VfsError::AlreadyExists { .. } => io::ErrorKind::AlreadyExists,
            VfsError::Unsupported { .. } => io::ErrorKind::Unsupported,
            VfsError::TooLarge { .. } => io::ErrorKind::FileTooLarge,
            VfsError::InvalidName { .. }
            | VfsError::InvalidAttributes { .. }
            | VfsError::InvalidTarget { .. } => io::ErrorKind::InvalidInput,
            VfsError::NotEmpty { .. } => io::ErrorKind::DirectoryNotEmpty,
            VfsError::CrossDevice { .. } => io::ErrorKind::CrossesDevices,
            VfsError::OperationUnsupported { .. } | VfsError::Unavailable { .. } => {
                io::ErrorKind::Unsupported
            }
            VfsError::Io { source, .. } => {
                if source
                    .get_ref()
                    .is_some_and(|inner| inner.is::<Cancelled>())
                {
                    return io::Error::other(Cancelled);
                }
                source.kind()
            }
        };
        io::Error::new(kind, error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_names_round_trip() {
        for op in VfsOp::ALL {
            assert!(!op.as_str().is_empty());
        }
        assert_eq!(VfsOp::Commit.as_str(), "commit");
    }

    #[test]
    fn from_io_picks_specific_variants() {
        let cases = [
            (io::ErrorKind::NotFound, ErrorKind::NotFound),
            (io::ErrorKind::PermissionDenied, ErrorKind::PermissionDenied),
            (io::ErrorKind::NotADirectory, ErrorKind::NotADirectory),
            (io::ErrorKind::IsADirectory, ErrorKind::IsADirectory),
            (io::ErrorKind::AlreadyExists, ErrorKind::AlreadyExists),
            (io::ErrorKind::InvalidData, ErrorKind::InvalidData),
            (io::ErrorKind::FileTooLarge, ErrorKind::TooLarge),
            (io::ErrorKind::Other, ErrorKind::Io),
        ];
        for (io_kind, kind) in cases {
            let error = VfsError::from_io("/x", VfsOp::Read, io::Error::new(io_kind, "boom"));
            assert_eq!(error.kind(), kind, "{io_kind:?}");
        }
        let cancelled = VfsError::from_io("/x", VfsOp::Read, io::Error::other(Cancelled));
        assert!(matches!(cancelled, VfsError::Cancelled));
        assert!(cancelled.is_cancelled());
    }

    #[test]
    fn io_round_trip_keeps_the_error() {
        let original = VfsError::Unsupported {
            spec: "ftp://h/p".into(),
            what: "ftp specs",
        };
        let io_error = io::Error::from(original);
        assert_eq!(io_error_kind(&io_error), ErrorKind::Unsupported);
        let back = VfsError::from_io("ignored", VfsOp::Open, io_error);
        assert_eq!(
            back.to_string(),
            "ftp://h/p: ftp specs is not supported in this version"
        );
        let cancelled = io::Error::from(VfsError::Cancelled);
        assert_eq!(io_error_kind(&cancelled), ErrorKind::Cancelled);
        let too_large = io::Error::from(VfsError::TooLarge {
            spec: "/x".into(),
            limit: 1,
        });
        assert_eq!(io_error_kind(&too_large), ErrorKind::TooLarge);
    }

    #[test]
    fn messages_are_stable() {
        let io = VfsError::Io {
            spec: "/x".into(),
            op: VfsOp::Read,
            source: io::Error::other("boom"),
        };
        assert_eq!(io.to_string(), "/x: Read failed: boom");
        assert_eq!(
            VfsError::NotFound { spec: "/x".into() }.to_string(),
            "/x: not found"
        );
        assert_eq!(
            VfsError::InvalidName { name: "a/b".into() }.to_string(),
            "invalid file name \"a/b\""
        );
    }
}
