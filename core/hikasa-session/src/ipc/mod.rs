//! IPC between the CLI and the GUI (`hikasa_session::ipc`, ARCH-090…092; §1.8.6, §2.23, §4.4).
//!
//! - [`protocol`]: the messages of protocol v1 (§4.4.2) with forward-compatible decoding (D-46);
//! - [`framing`]: u32 little-endian length-prefixed JSON frames and the two-phase decode (§4.4.1, §4.4.3);
//! - [`endpoint`]: the per-user, per-configuration endpoint and the single-instance lock (§2.23, D-47);
//! - [`server`]: the blocking server (one accept thread, one thread per connection) and its [`IpcHandler`];
//! - [`client`]: the blocking client, the launch retry policy and [`connect_or_launch`].
//!
//! The transport is `interprocess` local sockets: Unix domain sockets in a private folder (socket mode 0600,
//! peer uid checked), named pipes on Windows. There is no async runtime in M1 (§0.7) and no sleeping or clock
//! reading: timeouts are socket receive timeouts and channel waits (T1).

pub mod client;
pub mod endpoint;
mod error;
pub mod framing;
pub mod protocol;
pub mod server;
mod transport;

use std::time::Duration;

pub use client::{IpcClient, MIN_PROTOCOL_VERSION, RetryPolicy, connect_or_launch};
pub use endpoint::{Endpoint, EndpointAddress, InstanceLock, MAX_SOCKET_PATH, fnv1a64};
pub use error::IpcError;
pub use framing::{read_frame, recv_request, recv_response, send, write_frame};
pub use protocol::{
    CloseReason, Envelope, IpcErrorCode, OpenOptions, ReadOnly, Request, Response, ServerInfo,
    ViewId, ViewInfo, ViewKind, ViewResult,
};
pub use server::{
    ABANDONED_EXIT_CODE, IpcHandler, IpcServer, ServerHandle, ViewWaiter, WRITE_QUEUE,
    WRITE_TIMEOUT, peer_uid_matches,
};

/// Protocol version spoken by this build.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum frame payload (bytes). Larger frames are a protocol error.
pub const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;

/// Maximum number of JSON values in one frame (an upper bound counted before decoding, §1.8.7): more is a
/// protocol error, which bounds the decoder's peak memory per frame. Real messages hold a few hundred values.
pub const MAX_FRAME_VALUES: usize = 1 << 18;

/// Timeout of an immediate response (§4.4.3 rule 7: 10 s); waiting for `ViewClosed` has none. Additive.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
