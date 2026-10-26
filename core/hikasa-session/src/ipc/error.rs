//! [`IpcError`] (§1.8.6, §5.5).

use hikasa_core::{Classify, ErrorKind, io_error_kind};

/// IPC errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IpcError {
    /// No server listening.
    #[error("the Hikasa GUI is not running")]
    NotRunning,
    /// Another instance holds the endpoint lock.
    #[error("another Hikasa GUI instance is running")]
    AlreadyRunning,
    /// The socket folder has the wrong owner or permissions.
    #[error("insecure IPC endpoint {path}: {reason}")]
    InsecureEndpoint {
        /// The folder or socket.
        path: String,
        /// What is wrong.
        reason: &'static str,
    },
    /// Frame larger than the limit.
    #[error("frame of {len} bytes exceeds the limit of {max}")]
    FrameTooLarge {
        /// The announced length.
        len: u32,
        /// The limit.
        max: u32,
    },
    /// Malformed frame or sequence (no usable request id).
    #[error("protocol error: {0}")]
    Protocol(String),
    /// A frame with a valid `id` whose body does not decode.
    #[error("bad message {id}: {message}")]
    BadMessage {
        /// The id the frame carried.
        id: u64,
        /// The decoder's message.
        message: String,
    },
    /// No common protocol version.
    #[error("incompatible protocol versions (client {client}, server {server})")]
    IncompatibleVersion {
        /// The client's version.
        client: u32,
        /// The server's version (0 when the server did not say).
        server: u32,
    },
    /// The peer answered with an error (`code` is the wire string, possibly unknown to this build).
    #[error("{code}: {message}")]
    Remote {
        /// The `IpcErrorCode` wire name.
        code: String,
        /// The peer's message.
        message: String,
    },
    /// EOF while waiting for a response or `ViewClosed`.
    #[error("connection to the GUI was lost")]
    ConnectionLost,
    /// The launched GUI did not become reachable within the retry budget.
    #[error("the Hikasa GUI did not start listening in time")]
    LaunchTimeout,
    /// An immediate response did not arrive within the timeout.
    #[error("the Hikasa GUI did not answer in time")]
    Timeout,
    /// Transport I/O.
    #[error("ipc I/O: {0}")]
    Io(#[from] std::io::Error),
}

impl Classify for IpcError {
    /// `NotRunning` → `not_found`; `AlreadyRunning` → `already_exists`; `InsecureEndpoint` →
    /// `permission_denied`; `FrameTooLarge`, `Protocol`, `BadMessage`, `IncompatibleVersion` → `protocol` (§5.5);
    /// `Remote` → by its code (`not_found`, `unsupported_*` → `unsupported`, `invalid_request` →
    /// `invalid_input`, `internal`, `protocol`/`incompatible_version` → `protocol`, unknown codes → `io`);
    /// `ConnectionLost`, `LaunchTimeout`, `Timeout` → `io`; `Io` → [`io_error_kind`].
    fn kind(&self) -> ErrorKind {
        match self {
            IpcError::NotRunning => ErrorKind::NotFound,
            IpcError::AlreadyRunning => ErrorKind::AlreadyExists,
            IpcError::InsecureEndpoint { .. } => ErrorKind::PermissionDenied,
            IpcError::FrameTooLarge { .. }
            | IpcError::Protocol(_)
            | IpcError::BadMessage { .. }
            | IpcError::IncompatibleVersion { .. } => ErrorKind::Protocol,
            IpcError::Remote { code, .. } => match code.as_str() {
                "not_found" => ErrorKind::NotFound,
                "unsupported_request" | "unsupported_view" => ErrorKind::Unsupported,
                "invalid_request" => ErrorKind::InvalidInput,
                "internal" => ErrorKind::Internal,
                "protocol" | "incompatible_version" => ErrorKind::Protocol,
                _ => ErrorKind::Io,
            },
            IpcError::ConnectionLost | IpcError::LaunchTimeout | IpcError::Timeout => ErrorKind::Io,
            IpcError::Io(e) => io_error_kind(e),
        }
    }
}

impl IpcError {
    /// The error of a failed transport read or write: a timeout is [`IpcError::Timeout`]; a reset, aborted or
    /// broken connection and an unexpected EOF are [`IpcError::ConnectionLost`]; anything else is
    /// [`IpcError::Io`].
    pub(crate) fn from_transport(e: std::io::Error) -> IpcError {
        use std::io::ErrorKind as K;
        match e.kind() {
            K::WouldBlock | K::TimedOut => IpcError::Timeout,
            K::UnexpectedEof | K::ConnectionReset | K::ConnectionAborted | K::BrokenPipe => {
                IpcError::ConnectionLost
            }
            _ => IpcError::Io(e),
        }
    }
}
