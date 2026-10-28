//! [`ApiError`] and the exit-code mapping of every service result (§2.21, §5.5; D-01, D-02, D-22, D-23, D-35).

use hikasa_core::{Classify, ErrorKind, ErrorReport, ExitCode, Side, SpecError, io_error_kind};
use hikasa_folder::FolderError;
use hikasa_formats::FormatError;
use hikasa_report::ReportError;
use hikasa_text::TextError;
use hikasa_vfs::VfsError;

use super::ApiOp;
use crate::error::SessionError;
use crate::ipc::IpcError;

/// API errors. `exit_code(op)` implements §2.21.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiError {
    /// Bad argument combination or value (also unknown `gui open` switches and vocabulary words).
    #[error("{0}")]
    InvalidArgument(String),
    /// A named item does not exist (a settings key, a format id of a `format` write command). Added in M2 W1
    /// (kind `not_found`).
    #[error("{0}")]
    NotFound(String),
    /// The target already exists (`format new`, `format import` without `--replace`). Added in M2 W1 (kind
    /// `already_exists`).
    #[error("{0}")]
    AlreadyExists(String),
    /// Not supported in this version (a reserved P1 session type, a session type `session run` cannot run yet,
    /// a GUI that does not serve sessions, D-89). Added in M2 W2 (kind `unsupported`).
    #[error("{0}")]
    Unsupported(String),
    /// Invalid data in a file the user gave (a settings package entry, D-86). Added in M2 W2 (kind
    /// `invalid_data`).
    #[error("{0}")]
    InvalidData(String),
    /// Spec syntax.
    #[error(transparent)]
    Spec(#[from] SpecError),
    /// An input could not be loaded (stat/read/unsupported scheme/too large).
    #[error("{side} input {spec}: {source}")]
    LoadFailed {
        /// The side.
        side: Side,
        /// Canonical spec.
        spec: String,
        /// The VFS failure.
        #[source]
        source: VfsError,
    },
    /// Other VFS failure.
    #[error(transparent)]
    Vfs(#[from] VfsError),
    /// Format failure.
    #[error(transparent)]
    Format(#[from] FormatError),
    /// Text engine failure.
    #[error(transparent)]
    Text(#[from] TextError),
    /// Folder engine failure.
    #[error(transparent)]
    Folder(#[from] FolderError),
    /// Report failure.
    #[error(transparent)]
    Report(#[from] ReportError),
    /// Configuration failure.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// GUI connection failure.
    #[error(transparent)]
    Ipc(#[from] IpcError),
    /// The GUI executable was not found (exit 103).
    #[error("cannot find the hikasa-gui executable")]
    GuiNotFound,
    /// A GUI executable was found but cannot be started (it is not executable, its interpreter is missing, or
    /// spawning failed otherwise): there is no usable GUI executable, exit 103 like `GuiNotFound`. The launcher
    /// names the executable in the error's message. Added in the wave-5 review (§1.9.3).
    #[error("cannot start the GUI executable: {0}")]
    GuiNotStartable(#[source] std::io::Error),
    /// Cancelled.
    #[error("operation cancelled")]
    Cancelled,
    /// A panic or invariant violation (bug).
    #[error("internal error: {0}")]
    Internal(String),
    /// A rules file (`--rules-file`, `--text-rules-file`) that does not parse or validate: the error of the file,
    /// named by its canonical spec (JSON `error.spec`). Its kind and exit code are those of `source`. Added in
    /// the wave-4 review (§1.8.7).
    #[error("{spec}: {source}")]
    RulesFile {
        /// Canonical spec of the file.
        spec: String,
        /// What is wrong with it.
        #[source]
        source: Box<ApiError>,
    },
}

impl Classify for ApiError {
    fn kind(&self) -> ErrorKind {
        ApiError::kind(self)
    }
}

impl ApiError {
    /// Error kind (JSON `error.kind`).
    pub fn kind(&self) -> ErrorKind {
        match self {
            ApiError::InvalidArgument(_) => ErrorKind::InvalidInput,
            ApiError::NotFound(_) => ErrorKind::NotFound,
            ApiError::AlreadyExists(_) => ErrorKind::AlreadyExists,
            ApiError::Unsupported(_) => ErrorKind::Unsupported,
            ApiError::InvalidData(_) => ErrorKind::InvalidData,
            ApiError::Spec(e) => e.kind(),
            ApiError::LoadFailed { source, .. } | ApiError::Vfs(source) => source.kind(),
            ApiError::Format(e) => e.kind(),
            ApiError::Text(e) => e.kind(),
            ApiError::Folder(e) => e.kind(),
            ApiError::Report(e) => e.kind(),
            ApiError::Session(e) => e.kind(),
            ApiError::Ipc(e) => e.kind(),
            ApiError::GuiNotFound => ErrorKind::NotFound,
            ApiError::GuiNotStartable(e) => io_error_kind(e),
            ApiError::Cancelled => ErrorKind::Cancelled,
            ApiError::Internal(_) => ErrorKind::Internal,
            ApiError::RulesFile { source, .. } => source.kind(),
        }
    }

    /// Exit code for this error in the context of `op` (§2.21).
    ///
    /// | op | error | code |
    /// |---|---|---|
    /// | `FolderCompare` | `FolderError::BaseNotLoadable`/`BaseNotFolder`, `LoadFailed` (a base) | 107 |
    /// | `GuiOpen { wait }` | `Ipc(NotRunning)` | 102 when `wait`, else 100 |
    /// | `GuiOpen { .. }` | `Ipc(LaunchTimeout)`, `Ipc(ConnectionLost)` | 102 |
    /// | any | `GuiNotFound`, `GuiNotStartable` | 103 |
    /// | any | everything else (incl. cancellation, panics and invariant violations) | 100 |
    pub fn exit_code(&self, op: ApiOp) -> ExitCode {
        match self {
            ApiError::GuiNotFound | ApiError::GuiNotStartable(_) => ExitCode::GuiNotFound,
            ApiError::LoadFailed { .. } => match op {
                ApiOp::FolderCompare => ExitCode::LoadFailed,
                ApiOp::Quick
                | ApiOp::TextCompare
                | ApiOp::Format
                | ApiOp::Vfs
                | ApiOp::GuiOpen { .. }
                | ApiOp::GuiControl
                | ApiOp::Other => ExitCode::UnknownError,
            },
            ApiError::Folder(e) => match op {
                ApiOp::FolderCompare => match e {
                    FolderError::BaseNotLoadable { .. } | FolderError::BaseNotFolder { .. } => {
                        ExitCode::LoadFailed
                    }
                    // Another crate's #[non_exhaustive] enum: every other variant is 100 (§0.4 rule 9).
                    _ => ExitCode::UnknownError,
                },
                ApiOp::Quick
                | ApiOp::TextCompare
                | ApiOp::Format
                | ApiOp::Vfs
                | ApiOp::GuiOpen { .. }
                | ApiOp::GuiControl
                | ApiOp::Other => ExitCode::UnknownError,
            },
            ApiError::RulesFile { source, .. } => source.exit_code(op),
            ApiError::Ipc(e) => match op {
                ApiOp::GuiOpen { wait } => match e {
                    IpcError::NotRunning => {
                        if wait {
                            ExitCode::CannotWaitForGui
                        } else {
                            ExitCode::UnknownError
                        }
                    }
                    IpcError::LaunchTimeout | IpcError::ConnectionLost => {
                        ExitCode::CannotWaitForGui
                    }
                    IpcError::AlreadyRunning
                    | IpcError::InsecureEndpoint { .. }
                    | IpcError::FrameTooLarge { .. }
                    | IpcError::Protocol(_)
                    | IpcError::BadMessage { .. }
                    | IpcError::IncompatibleVersion { .. }
                    | IpcError::Remote { .. }
                    | IpcError::Timeout
                    | IpcError::Io(_) => ExitCode::UnknownError,
                },
                ApiOp::Quick
                | ApiOp::TextCompare
                | ApiOp::FolderCompare
                | ApiOp::Format
                | ApiOp::Vfs
                | ApiOp::GuiControl
                | ApiOp::Other => ExitCode::UnknownError,
            },
            ApiError::InvalidArgument(_)
            | ApiError::NotFound(_)
            | ApiError::AlreadyExists(_)
            | ApiError::Unsupported(_)
            | ApiError::InvalidData(_)
            | ApiError::Spec(_)
            | ApiError::Vfs(_)
            | ApiError::Format(_)
            | ApiError::Text(_)
            | ApiError::Report(_)
            | ApiError::Session(_)
            | ApiError::Cancelled
            | ApiError::Internal(_) => ExitCode::UnknownError,
        }
    }

    /// Serialisable report (JSON `error`, §4.2): kind, message, the canonical spec involved (a `LoadFailed` side's
    /// spec, a VFS error's spec), and the `source()` chain.
    pub fn report(&self) -> ErrorReport {
        let mut report = ErrorReport::capture(self, None);
        report.spec = match self {
            ApiError::LoadFailed { spec, .. } | ApiError::RulesFile { spec, .. } => {
                Some(spec.clone())
            }
            ApiError::Vfs(e) => e.spec().map(str::to_owned),
            ApiError::Folder(e) => match e {
                FolderError::BaseNotLoadable { spec, .. }
                | FolderError::BaseNotFolder { spec, .. } => Some(spec.clone()),
                // Another crate's #[non_exhaustive] enum: no spec for the other variants.
                _ => None,
            },
            ApiError::InvalidArgument(_)
            | ApiError::NotFound(_)
            | ApiError::AlreadyExists(_)
            | ApiError::Unsupported(_)
            | ApiError::InvalidData(_)
            | ApiError::Spec(_)
            | ApiError::Format(_)
            | ApiError::Text(_)
            | ApiError::Report(_)
            | ApiError::Session(_)
            | ApiError::Ipc(_)
            | ApiError::GuiNotFound
            | ApiError::GuiNotStartable(_)
            | ApiError::Cancelled
            | ApiError::Internal(_) => None,
        };
        report
    }

    /// Wraps the error of a rules file with the file's spec (`Cancelled` stays `Cancelled`).
    pub(crate) fn in_rules_file(spec: &hikasa_core::Spec, error: ApiError) -> ApiError {
        match error {
            ApiError::Cancelled => ApiError::Cancelled,
            source => ApiError::RulesFile {
                spec: spec.canonical(),
                source: Box::new(source),
            },
        }
    }

    /// True when the error is a broken pipe on the output (D-35: the CLI stops writing and keeps the computed
    /// exit code). Additive.
    pub fn is_broken_pipe(&self) -> bool {
        let mut source: Option<&(dyn std::error::Error + 'static)> = Some(self);
        while let Some(error) = source {
            if let Some(io) = error.downcast_ref::<std::io::Error>()
                && io.kind() == std::io::ErrorKind::BrokenPipe
            {
                return true;
            }
            source = error.source();
        }
        false
    }

    /// `Cancelled` for cancellation, `LoadFailed { side, spec }` for a VFS load failure, `Text` otherwise.
    pub(crate) fn from_text(error: TextError, side: Side, spec: &str) -> ApiError {
        match error {
            TextError::Load { source, .. } if !source.is_cancelled() => ApiError::LoadFailed {
                side,
                spec: spec.to_owned(),
                source,
            },
            e if e.is_cancelled() => ApiError::Cancelled,
            e => ApiError::Text(e),
        }
    }

    /// `Cancelled` for cancellation, else `Folder`.
    pub(crate) fn from_folder(error: FolderError) -> ApiError {
        if error.kind() == ErrorKind::Cancelled {
            ApiError::Cancelled
        } else {
            ApiError::Folder(error)
        }
    }

    /// `Cancelled` for cancellation, else `Vfs`.
    pub(crate) fn from_vfs(error: VfsError) -> ApiError {
        if error.is_cancelled() {
            ApiError::Cancelled
        } else {
            ApiError::Vfs(error)
        }
    }

    /// A failure writing to the caller's output: `Vfs(Io { spec: "<output>", op: Write })`, so that
    /// [`ApiError::is_broken_pipe`] sees a broken pipe.
    pub(crate) fn output(error: std::io::Error) -> ApiError {
        if io_error_kind(&error) == ErrorKind::Cancelled {
            return ApiError::Cancelled;
        }
        ApiError::Vfs(VfsError::Io {
            spec: "<output>".into(),
            op: hikasa_vfs::VfsOp::Write,
            source: error,
        })
    }
}
