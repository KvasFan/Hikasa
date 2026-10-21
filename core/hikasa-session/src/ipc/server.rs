//! The blocking IPC server (§1.8.6, §4.4.3, §5.1): [`IpcServer`], [`IpcHandler`], [`ViewWaiter`],
//! [`ServerHandle`].
//!
//! One accept thread plus one thread per connection (no async runtime is approved, §0.7). The server owns the
//! protocol rules of §4.4.3: the `Hello` handshake and version negotiation, the two-phase decode with
//! `invalid_request`/`unsupported_request` answers carrying the request's own id (D-46), `Error{protocol}` with
//! id 0 and a close for frames without a usable id, the M1 request support of rule 5 (`unsupported_request` for
//! sessions, workspaces and scripts; `unsupported_view` for every view kind but `folder_compare` and
//! `text_compare`), and the ordering "immediate response first, then at most one `ViewClosed`" (INV-IPC-1/2).
//! Everything else goes to the [`IpcHandler`] (the GUI).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use hikasa_core::ExitCode;
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{Listener, ListenerOptions, Stream};
use parking_lot::Mutex;

use super::PROTOCOL_VERSION;
use super::endpoint::{Endpoint, InstanceLock, prepare_own_dir};
use super::error::IpcError;
use super::framing::{recv_typed, write_frame};
use super::protocol::{Envelope, IpcErrorCode, Request, Response, ServerInfo, ViewId, ViewResult};
use super::transport::{connect_stream, name_of};
use crate::jobs::panic_message;

/// How long `ServerHandle::shutdown` waits for the accept thread after waking it.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

/// Pause after a failed `accept` (a wait on the shutdown channel, never a sleep: T1).
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);

/// Server-side request handler (implemented by hikasa-gui).
pub trait IpcHandler: Send + Sync + 'static {
    /// Identity for the `Hello` reply.
    fn server_info(&self) -> ServerInfo;
    /// Handles one request after the handshake (`Request::Unknown` never reaches it: the server answers
    /// `unsupported_request` itself). For `OpenView { wait: true }`, `waiter` is `Some`; the handler stores it and
    /// later calls `ViewWaiter::view_closed`. The return value is the immediate response. Must not block: the
    /// GUI posts to its event loop and returns.
    ///
    /// In M1 the server also answers `Hello` after the handshake (protocol error), `LoadSession`,
    /// `LoadWorkspace` and `RunScript` (`unsupported_request`) and `OpenView` of a kind other than
    /// `folder_compare`/`text_compare` (`unsupported_view`) itself (§4.4.3 rule 5). A handler panic is answered
    /// with `Error{internal}`.
    fn handle(&self, request: Request, waiter: Option<ViewWaiter>) -> Response;
}

/// One accepted connection: the stream (read by its thread, written by it and by waiters).
struct Conn {
    stream: Stream,
    /// Serialises frame writes (the connection thread's responses and the waiters' `ViewClosed`).
    write: Mutex<()>,
}

impl Conn {
    fn write_payload(&self, payload: &[u8]) -> Result<(), IpcError> {
        let _guard = self.write.lock();
        let mut writer: &Stream = &self.stream;
        write_frame(&mut writer, payload)
    }

    fn send(&self, id: u64, response: &Response) -> Result<(), IpcError> {
        let payload = serde_json::to_vec(&Envelope { id, body: response })
            .map_err(|e| IpcError::Protocol(format!("cannot serialise a response: {e}")))?;
        self.write_payload(&payload)
    }
}

/// Orders a waiter's `ViewClosed` after the immediate response of its `OpenView`.
enum GateState {
    /// The immediate response is not written yet; a `ViewClosed` sent meanwhile waits here.
    Pending(Option<Vec<u8>>),
    /// The view was opened: `ViewClosed` goes out at once.
    Open,
    /// The `OpenView` failed: a `ViewClosed` is dropped (there is no view).
    Closed,
}

struct Gate(Mutex<GateState>);

impl Gate {
    /// Called after the immediate response was written: flushes a pending `ViewClosed` when the view opened.
    fn release(&self, conn: &Conn, opened: bool) {
        let mut state = self.0.lock();
        let pending = match &mut *state {
            GateState::Pending(slot) => slot.take(),
            GateState::Open | GateState::Closed => None,
        };
        *state = if opened {
            GateState::Open
        } else {
            GateState::Closed
        };
        if opened && let Some(payload) = pending {
            let _ = conn.write_payload(&payload);
        }
    }
}

/// Sends the deferred `ViewClosed` on the originating connection (no-op when the client is gone).
pub struct ViewWaiter {
    conn: Arc<Conn>,
    id: u64,
    gate: Arc<Gate>,
}

impl ViewWaiter {
    /// Sends `ViewClosed` (exactly once: consumes self).
    ///
    /// When called before the server wrote the `OpenView`'s immediate response (a handler that closes the view at
    /// once), the message is held back and sent right after it; when the immediate response was not
    /// `ViewOpened`, it is dropped.
    pub fn view_closed(self, view: ViewId, result: ViewResult, exit_code: ExitCode) {
        let envelope = Envelope {
            id: self.id,
            body: Response::ViewClosed {
                view,
                result,
                exit_code: exit_code.code(),
            },
        };
        let Ok(payload) = serde_json::to_vec(&envelope) else {
            return;
        };
        let mut state = self.gate.0.lock();
        match &mut *state {
            GateState::Pending(slot) => *slot = Some(payload),
            GateState::Open => {
                if let Err(e) = self.conn.write_payload(&payload) {
                    tracing::debug!(target: "hikasa::ipc", "view_closed for request {}: {e}", self.id);
                }
            }
            GateState::Closed => {}
        }
    }

    /// The id of the `OpenView` request (additive).
    pub fn request_id(&self) -> u64 {
        self.id
    }
}

impl std::fmt::Debug for ViewWaiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewWaiter")
            .field("request_id", &self.id)
            .finish_non_exhaustive()
    }
}

/// A bound IPC server.
#[derive(Debug)]
pub struct IpcServer {
    listener: Listener,
    endpoint: Endpoint,
    lock: InstanceLock,
    /// The uid peers must have (Unix: the socket file's owner); `None` = no check.
    peer_uid: Option<u32>,
}

impl IpcServer {
    /// Acquires the instance lock (`AlreadyRunning` when held). Unix: creates the socket folder with mode 0700
    /// when it is Hikasa's own fallback folder and verifies its owner and mode (`InsecureEndpoint` otherwise),
    /// removes a stale socket file, binds, and sets the socket mode to 0600.
    pub fn bind(endpoint: &Endpoint) -> Result<IpcServer, IpcError> {
        prepare_own_dir(endpoint)?;
        let lock = InstanceLock::try_acquire(endpoint)?.ok_or(IpcError::AlreadyRunning)?;
        remove_stale_socket(endpoint)?;
        let listener = ListenerOptions::new()
            .name(name_of(endpoint)?)
            .reclaim_name(true)
            .create_sync()?;
        let peer_uid = secure_socket(endpoint)?;
        tracing::debug!(target: "hikasa::ipc", "listening on {}", endpoint.display());
        Ok(IpcServer {
            listener,
            endpoint: endpoint.clone(),
            lock,
            peer_uid,
        })
    }

    /// The endpoint (additive).
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Replaces the uid that connecting peers must have (tests of the peer check; `None` disables it).
    #[doc(hidden)]
    pub fn set_peer_uid(&mut self, uid: Option<u32>) {
        self.peer_uid = uid;
    }

    /// Starts the accept thread (one thread per connection) and returns its handle. Unix: rejects peers whose
    /// `peer_creds().euid()` differs from the owner uid of the socket file.
    pub fn spawn(self, handler: Arc<dyn IpcHandler>) -> Result<ServerHandle, IpcError> {
        let IpcServer {
            listener,
            endpoint,
            lock,
            peer_uid,
        } = self;
        let stop = Arc::new(AtomicBool::new(false));
        let (done_tx, done_rx) = crossbeam_channel::bounded::<()>(1);
        let (wake_tx, wake_rx) = crossbeam_channel::bounded::<()>(1);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("hikasa-ipc-accept".into())
            .spawn(move || {
                accept_loop(&listener, &thread_stop, &handler, peer_uid, &wake_rx);
                drop(listener);
                drop(lock);
                let _ = done_tx.send(());
            })?;
        Ok(ServerHandle {
            stop,
            endpoint,
            thread: Some(thread),
            done: done_rx,
            wake: wake_tx,
        })
    }
}

/// A running server.
#[derive(Debug)]
pub struct ServerHandle {
    stop: Arc<AtomicBool>,
    endpoint: Endpoint,
    thread: Option<JoinHandle<()>>,
    done: Receiver<()>,
    wake: Sender<()>,
}

impl ServerHandle {
    /// Stops accepting: sets the stop flag, wakes the blocked `accept` by connecting to its own endpoint once
    /// (self-connect; no sleeping or polling), and joins the accept thread. Connection threads end when their
    /// clients disconnect or the process exits; waiters are notified by the GUI before it calls this.
    ///
    /// The listener (and with it the socket file) and the instance lock are released when the accept thread
    /// ends. Dropping the handle does the same.
    pub fn shutdown(mut self) -> Result<(), IpcError> {
        self.stop_server()
    }

    /// The endpoint (additive).
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    fn stop_server(&mut self) -> Result<(), IpcError> {
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        self.stop.store(true, Ordering::Release);
        let _ = self.wake.try_send(());
        drop(connect_stream(&self.endpoint));
        match self.done.recv_timeout(SHUTDOWN_WAIT) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                Ok(())
            }
            Err(RecvTimeoutError::Timeout) => Err(IpcError::Timeout),
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        let _ = self.stop_server();
    }
}

/// Removes a socket file left by a dead server (the instance lock proves no live server owns it).
#[cfg(unix)]
fn remove_stale_socket(endpoint: &Endpoint) -> Result<(), IpcError> {
    if let super::EndpointAddress::UnixSocket(path) = &endpoint.address {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(IpcError::Io(e)),
        }
    }
    Ok(())
}

/// Named pipes leave no files behind.
#[cfg(not(unix))]
fn remove_stale_socket(_endpoint: &Endpoint) -> Result<(), IpcError> {
    Ok(())
}

/// Unix: sets the socket mode to 0600, checks that the socket folder belongs to the socket's owner (our euid),
/// and returns that uid for the per-connection peer check.
#[cfg(unix)]
fn secure_socket(endpoint: &Endpoint) -> Result<Option<u32>, IpcError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let super::EndpointAddress::UnixSocket(path) = &endpoint.address else {
        return Ok(None);
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    let owner = std::fs::metadata(path)?.uid();
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => std::path::Path::new("."),
    };
    if std::fs::metadata(dir)?.uid() != owner {
        return Err(IpcError::InsecureEndpoint {
            path: dir.display().to_string(),
            reason: "the socket folder belongs to another user",
        });
    }
    Ok(Some(owner))
}

/// Windows: per-user pipe names (an owner-only DACL is a P1 item, Appendix C).
#[cfg(not(unix))]
fn secure_socket(_endpoint: &Endpoint) -> Result<Option<u32>, IpcError> {
    Ok(None)
}

/// True when the peer may talk to this server: its effective uid equals `expected` (Unix). A platform that
/// cannot report the peer's uid is allowed (the socket folder is private anyway) and logged.
#[cfg(unix)]
fn peer_allowed(stream: &Stream, expected: Option<u32>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    match stream.peer_creds() {
        Ok(creds) => match creds.euid() {
            Some(euid) => peer_uid_matches(Some(euid), expected),
            None => {
                tracing::warn!(target: "hikasa::ipc", "the peer's uid is unknown; connection accepted");
                true
            }
        },
        Err(e) => {
            tracing::warn!(target: "hikasa::ipc", "cannot read the peer's credentials ({e}); connection accepted");
            true
        }
    }
}

/// Windows: the pipe name is per user (no peer uid).
#[cfg(not(unix))]
fn peer_allowed(_stream: &Stream, _expected: Option<u32>) -> bool {
    true
}

/// The peer-uid rule of §2.23: a known uid must equal the socket owner's; an unknown one is allowed.
pub fn peer_uid_matches(peer: Option<u32>, owner: u32) -> bool {
    peer.is_none_or(|uid| uid == owner)
}

fn accept_loop(
    listener: &Listener,
    stop: &AtomicBool,
    handler: &Arc<dyn IpcHandler>,
    peer_uid: Option<u32>,
    wake: &Receiver<()>,
) {
    let mut connections: u64 = 0;
    loop {
        match listener.accept() {
            Ok(stream) => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if !peer_allowed(&stream, peer_uid) {
                    tracing::warn!(target: "hikasa::ipc", "rejected a connection from another user");
                    continue;
                }
                connections += 1;
                let handler = Arc::clone(handler);
                let spawned = thread::Builder::new()
                    .name(format!("hikasa-ipc-conn-{connections}"))
                    .spawn(move || serve(stream, &handler));
                if let Err(e) = spawned {
                    tracing::warn!(target: "hikasa::ipc", "cannot start a connection thread: {e}");
                }
            }
            Err(e) => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                tracing::warn!(target: "hikasa::ipc", "accept failed: {e}");
                match wake.recv_timeout(ACCEPT_BACKOFF) {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
        }
    }
}

/// Serves one connection until the client disconnects or breaks the protocol.
fn serve(stream: Stream, handler: &Arc<dyn IpcHandler>) {
    let conn = Arc::new(Conn {
        stream,
        write: Mutex::new(()),
    });
    if !handshake(&conn, handler) {
        return;
    }
    loop {
        let mut reader: &Stream = &conn.stream;
        match recv_typed::<Request>(&mut reader) {
            Ok(None) => break,
            Ok(Some((envelope, type_name))) => {
                if dispatch(&conn, handler, envelope, &type_name).is_err() {
                    break;
                }
            }
            Err(IpcError::BadMessage { id, message }) => {
                let reply = Response::error(IpcErrorCode::InvalidRequest, message);
                if conn.send(id, &reply).is_err() {
                    break;
                }
            }
            Err(e @ (IpcError::Protocol(_) | IpcError::FrameTooLarge { .. })) => {
                let _ = conn.send(0, &Response::error(IpcErrorCode::Protocol, e.to_string()));
                break;
            }
            Err(_) => break,
        }
    }
}

/// §4.4.3 rule 1. Returns false when the connection must close.
fn handshake(conn: &Conn, handler: &Arc<dyn IpcHandler>) -> bool {
    let mut reader: &Stream = &conn.stream;
    match recv_typed::<Request>(&mut reader) {
        Ok(Some((envelope, _))) => match envelope.body {
            Request::Hello {
                version,
                min_version,
                ..
            } => {
                let negotiated = version.min(PROTOCOL_VERSION);
                if negotiated == 0 || negotiated < min_version {
                    let message = format!(
                        "no common protocol version: server speaks {PROTOCOL_VERSION}, client requires \
                         {min_version}..={version}"
                    );
                    let reply = Response::error(IpcErrorCode::IncompatibleVersion, message);
                    let _ = conn.send(envelope.id, &reply);
                    return false;
                }
                let info = match catch_unwind(AssertUnwindSafe(|| handler.server_info())) {
                    Ok(info) => info,
                    Err(payload) => {
                        let message =
                            format!("the server failed: {}", panic_message(payload.as_ref()));
                        let _ = conn.send(
                            envelope.id,
                            &Response::error(IpcErrorCode::Internal, message),
                        );
                        return false;
                    }
                };
                let reply = Response::Hello {
                    server: info.server,
                    version: negotiated,
                };
                conn.send(envelope.id, &reply).is_ok()
            }
            other => {
                let message = format!(
                    "the first message must be `hello`, not `{}`",
                    other.type_name()
                );
                let _ = conn.send(
                    envelope.id,
                    &Response::error(IpcErrorCode::Protocol, message),
                );
                false
            }
        },
        Ok(None) => false,
        Err(IpcError::BadMessage { id, message }) => {
            let _ = conn.send(id, &Response::error(IpcErrorCode::Protocol, message));
            false
        }
        Err(e @ (IpcError::Protocol(_) | IpcError::FrameTooLarge { .. })) => {
            let _ = conn.send(0, &Response::error(IpcErrorCode::Protocol, e.to_string()));
            false
        }
        Err(_) => false,
    }
}

/// Answers one request (rules 2 and 5 of §4.4.3).
fn dispatch(
    conn: &Arc<Conn>,
    handler: &Arc<dyn IpcHandler>,
    envelope: Envelope<Request>,
    type_name: &str,
) -> Result<(), IpcError> {
    let id = envelope.id;
    let mut gate = None;
    let response = match envelope.body {
        Request::Unknown => Response::error(
            IpcErrorCode::UnsupportedRequest,
            format!("unknown request type `{type_name}`"),
        ),
        Request::Hello { .. } => Response::error(
            IpcErrorCode::Protocol,
            "the handshake was already done on this connection",
        ),
        request @ (Request::LoadSession { .. }
        | Request::LoadWorkspace { .. }
        | Request::RunScript { .. }) => Response::error(
            IpcErrorCode::UnsupportedRequest,
            format!("`{}` is not supported in this version", request.type_name()),
        ),
        Request::OpenView { kind, .. } if !kind.is_served_in_m1() => Response::error(
            IpcErrorCode::UnsupportedView,
            format!(
                "view kind `{}` is not supported in this version",
                kind.as_str()
            ),
        ),
        request @ Request::OpenView { wait: true, .. } => {
            let shared = Arc::new(Gate(Mutex::new(GateState::Pending(None))));
            let waiter = ViewWaiter {
                conn: Arc::clone(conn),
                id,
                gate: Arc::clone(&shared),
            };
            gate = Some(shared);
            call_handler(handler, request, Some(waiter))
        }
        request => call_handler(handler, request, None),
    };
    let sent = conn.send(id, &response);
    if let Some(gate) = gate {
        gate.release(
            conn,
            sent.is_ok() && matches!(response, Response::ViewOpened { .. }),
        );
    }
    sent
}

/// Calls the handler behind a panic boundary (a panic is answered with `Error{internal}`).
fn call_handler(
    handler: &Arc<dyn IpcHandler>,
    request: Request,
    waiter: Option<ViewWaiter>,
) -> Response {
    match catch_unwind(AssertUnwindSafe(|| handler.handle(request, waiter))) {
        Ok(response) => response,
        Err(payload) => Response::error(
            IpcErrorCode::Internal,
            format!("the request failed: {}", panic_message(payload.as_ref())),
        ),
    }
}
