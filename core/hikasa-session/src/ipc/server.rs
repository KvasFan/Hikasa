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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError};
use hikasa_core::ExitCode;
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{Listener, ListenerOptions, Stream};
use parking_lot::Mutex;

use super::PROTOCOL_VERSION;
use super::endpoint::{Endpoint, InstanceLock, prepare_own_dir};
use super::error::IpcError;
use super::framing::{recv_typed, write_frame};
use super::protocol::{
    CloseReason, Envelope, IpcErrorCode, Request, Response, ServerInfo, ViewId, ViewResult,
};
use super::transport::{connect_stream, name_of};
use crate::jobs::panic_message;

/// How long `ServerHandle::shutdown` waits for the accept thread after waking it.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

/// How long `ServerHandle::shutdown` waits, in total, for the connections' writers to flush the last
/// `ViewClosed` messages before it returns.
const SHUTDOWN_FLUSH: Duration = Duration::from_secs(2);

/// Pause after a failed `accept` (a wait on the shutdown channel, never a sleep: T1).
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);

/// Send timeout of a server connection: a client that does not read for this long is dropped (§1.8.7).
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Frames queued per connection before the client counts as stuck and is dropped (§1.8.7).
pub const WRITE_QUEUE: usize = 64;

/// Exit code of a `ViewClosed` the server sends on its own: a waiter dropped without `view_closed` (reason
/// `abandoned`) or a view still open when the server shuts down (reason `shutdown`). The view's real code is
/// unknown, so the CLI reports 100 (§1.8.7, D-23 amendment).
pub const ABANDONED_EXIT_CODE: ExitCode = ExitCode::UnknownError;

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

/// What a connection's writer thread receives.
enum Out {
    /// One frame payload.
    Frame(Vec<u8>),
    /// Write what came before, then close the connection.
    Close,
}

/// One accepted connection. Every frame goes through a bounded queue to the connection's writer thread, which
/// writes with [`WRITE_TIMEOUT`]; nobody else ever blocks on the socket (§5.2 amendment).
struct Conn {
    stream: Arc<Stream>,
    out: Sender<Out>,
    /// Set once the connection is closing: later frames are dropped.
    closed: AtomicBool,
    /// The `OpenView { wait: true }` requests of this connection (for `ServerHandle::shutdown`).
    gates: Mutex<Vec<Weak<Gate>>>,
    /// Disconnected when the writer thread has ended.
    writer_done: Receiver<()>,
}

impl Conn {
    /// Wraps an accepted stream and starts its writer thread.
    fn start(stream: Stream, n: u64) -> Result<Arc<Conn>, IpcError> {
        // A platform that cannot set a send timeout still works: the queue bound drops a stuck client.
        let _ = stream.set_send_timeout(Some(WRITE_TIMEOUT));
        let stream = Arc::new(stream);
        let (out_tx, out_rx) = crossbeam_channel::bounded::<Out>(WRITE_QUEUE);
        let (done_tx, done_rx) = crossbeam_channel::bounded::<()>(1);
        let writer_stream = Arc::clone(&stream);
        thread::Builder::new()
            .name(format!("hikasa-ipc-write-{n}"))
            .spawn(move || writer(&writer_stream, &out_rx, done_tx))?;
        Ok(Arc::new(Conn {
            stream,
            out: out_tx,
            closed: AtomicBool::new(false),
            gates: Mutex::new(Vec::new()),
            writer_done: done_rx,
        }))
    }

    /// Queues one frame (never blocks). A full queue means the client stopped reading: the connection is
    /// dropped. Returns false when the frame will not be written.
    fn enqueue(&self, payload: Vec<u8>) -> bool {
        if self.closed.load(Ordering::Acquire) {
            return false;
        }
        match self.out.try_send(Out::Frame(payload)) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                tracing::warn!(
                    target: "hikasa::ipc",
                    "a client stopped reading ({WRITE_QUEUE} frames queued); connection dropped"
                );
                self.abort();
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    /// Queues one response.
    fn send(&self, id: u64, response: &Response) -> Result<(), IpcError> {
        let payload = serde_json::to_vec(&Envelope { id, body: response })
            .map_err(|e| IpcError::Protocol(format!("cannot serialise a response: {e}")))?;
        if self.enqueue(payload) {
            Ok(())
        } else {
            Err(IpcError::ConnectionLost)
        }
    }

    /// Closes the connection after the queued frames are written.
    fn close_after_flush(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) && self.out.try_send(Out::Close).is_err() {
            shutdown_stream(&self.stream);
        }
    }

    /// Closes the connection at once (queued frames are lost).
    fn abort(&self) {
        self.closed.store(true, Ordering::Release);
        shutdown_stream(&self.stream);
    }

    /// Registers the gate of an `OpenView { wait: true }`.
    fn track(&self, gate: &Arc<Gate>) {
        let mut gates = self.gates.lock();
        gates.retain(|g| g.strong_count() > 0);
        gates.push(Arc::downgrade(gate));
    }

    /// `ServerHandle::shutdown`: every view still open gets its `ViewClosed` (reason `shutdown`, exit 100),
    /// then the connection closes.
    fn shut_down(&self) {
        let gates: Vec<Arc<Gate>> = self.gates.lock().iter().filter_map(Weak::upgrade).collect();
        for gate in gates {
            gate.finish(self, Closing::Abandon(CloseReason::Shutdown));
        }
        self.close_after_flush();
    }
}

/// The writer thread of a connection: writes queued frames in order (each with the stream's send timeout) and
/// closes the connection on `Out::Close`, on a write failure or when every sender is gone.
fn writer(stream: &Stream, queue: &Receiver<Out>, done: Sender<()>) {
    for out in queue.iter() {
        match out {
            Out::Frame(payload) => {
                let mut w: &Stream = stream;
                if let Err(e) = write_frame(&mut w, &payload) {
                    tracing::debug!(target: "hikasa::ipc", "dropping a connection: {e}");
                    break;
                }
            }
            Out::Close => break,
        }
    }
    shutdown_stream(stream);
    drop(done);
}

/// Shuts a connection down in both directions, so that its reader sees EOF and the peer sees the close
/// (Unix). Windows named pipes cannot be closed from another thread without `unsafe`: there the connection
/// ends when the client disconnects (§1.8.7).
#[cfg(unix)]
fn shutdown_stream(stream: &Stream) {
    let Stream::UdSocket(socket) = stream;
    let _ = socket.inner().shutdown(std::net::Shutdown::Both);
}

/// See the Unix variant.
#[cfg(not(unix))]
fn shutdown_stream(_stream: &Stream) {}

/// How a waiter's view ends.
enum Closing {
    /// The handler's `ViewClosed` payload.
    Message(Vec<u8>),
    /// Closed by the server itself (exit 100).
    Abandon(CloseReason),
}

/// Orders a waiter's `ViewClosed` after the immediate response of its `OpenView`, and makes it exactly one.
enum GateState {
    /// The immediate response is not queued yet; a close requested meanwhile waits here.
    Pending(Option<Closing>),
    /// The view is open: `ViewClosed` goes out at once.
    Open(ViewId),
    /// `ViewClosed` was queued, or the `OpenView` failed (there is no view).
    Done,
}

/// The `ViewClosed` slot of one `OpenView { wait: true }` request.
struct Gate {
    id: u64,
    state: Mutex<GateState>,
}

impl Gate {
    /// Called after the immediate response was queued: flushes a pending close when the view opened.
    fn release(&self, conn: &Conn, opened: Option<ViewId>) {
        let mut state = self.state.lock();
        let pending = match std::mem::replace(&mut *state, GateState::Done) {
            GateState::Pending(pending) => pending,
            GateState::Open(_) | GateState::Done => None,
        };
        let Some(view) = opened else {
            return;
        };
        match pending {
            Some(closing) => {
                drop(state);
                conn.enqueue(self.payload(view, closing));
            }
            None => *state = GateState::Open(view),
        }
    }

    /// Ends the view once: the first close wins, later ones are dropped (§4.4.3 rule 2).
    fn finish(&self, conn: &Conn, closing: Closing) {
        let mut state = self.state.lock();
        match &mut *state {
            GateState::Pending(slot) => {
                if slot.is_none() {
                    *slot = Some(closing);
                }
            }
            GateState::Open(view) => {
                let view = *view;
                *state = GateState::Done;
                drop(state);
                conn.enqueue(self.payload(view, closing));
            }
            GateState::Done => {}
        }
    }

    /// The `ViewClosed` frame for `closing`.
    fn payload(&self, view: ViewId, closing: Closing) -> Vec<u8> {
        match closing {
            Closing::Message(payload) => payload,
            Closing::Abandon(reason) => view_closed_payload(
                self.id,
                view,
                ViewResult::new(None, None, false, reason),
                ABANDONED_EXIT_CODE.code(),
            ),
        }
    }
}

/// The serialised `ViewClosed` envelope.
fn view_closed_payload(id: u64, view: ViewId, result: ViewResult, exit_code: u8) -> Vec<u8> {
    let envelope = Envelope {
        id,
        body: Response::ViewClosed {
            view,
            result,
            exit_code,
        },
    };
    serde_json::to_vec(&envelope).unwrap_or_default()
}

/// Sends the deferred `ViewClosed` on the originating connection (no-op when the client is gone).
///
/// Protocol rule 2 is enforced here (§1.8.7): a waiter dropped without [`ViewWaiter::view_closed`] sends
/// `ViewClosed` with reason `abandoned`, no verdict and exit code 100 ([`ABANDONED_EXIT_CODE`]), so a
/// `gui open --wait` never hangs on a GUI error path. Neither path blocks: the frame is queued to the
/// connection's writer thread.
pub struct ViewWaiter {
    inner: Option<WaiterInner>,
}

/// The parts of a live waiter.
struct WaiterInner {
    conn: Arc<Conn>,
    gate: Arc<Gate>,
}

impl ViewWaiter {
    /// Sends `ViewClosed` (exactly once: consumes self). Never blocks.
    ///
    /// When called before the server queued the `OpenView`'s immediate response (a handler that closes the view
    /// at once), the message is held back and sent right after it; when the immediate response was not
    /// `ViewOpened`, it is dropped.
    pub fn view_closed(mut self, view: ViewId, result: ViewResult, exit_code: ExitCode) {
        if let Some(inner) = self.inner.take() {
            let payload = view_closed_payload(inner.gate.id, view, result, exit_code.code());
            inner.gate.finish(&inner.conn, Closing::Message(payload));
        }
    }

    /// The id of the `OpenView` request (additive).
    pub fn request_id(&self) -> u64 {
        self.inner.as_ref().map_or(0, |inner| inner.gate.id)
    }
}

impl Drop for ViewWaiter {
    /// A waiter dropped without `view_closed`: `ViewClosed` with reason `abandoned` and exit code 100.
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            tracing::warn!(
                target: "hikasa::ipc",
                "the waiter of request {} was dropped without view_closed; the view is reported as abandoned",
                inner.gate.id
            );
            inner
                .gate
                .finish(&inner.conn, Closing::Abandon(CloseReason::Abandoned));
        }
    }
}

impl std::fmt::Debug for ViewWaiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewWaiter")
            .field("request_id", &self.request_id())
            .finish_non_exhaustive()
    }
}

/// The live connections of a server (for `ServerHandle::shutdown`).
#[derive(Default)]
struct Registry {
    shut: bool,
    conns: Vec<Weak<Conn>>,
}

/// A bound IPC server.
#[derive(Debug)]
pub struct IpcServer {
    listener: Listener,
    endpoint: Endpoint,
    lock: InstanceLock,
    /// The uid peers must have (Unix: the socket file's owner); `None` on Windows.
    peer_uid: Option<u32>,
}

impl IpcServer {
    /// Acquires the instance lock (`AlreadyRunning` when held). Unix: creates the socket folder with mode 0700
    /// when it is Hikasa's own fallback folder (create-new; a folder another user created is refused), verifies
    /// the socket folder ([`Endpoint::verify_folder`]: owned by this user, not writable by others;
    /// `InsecureEndpoint` otherwise), removes a stale socket (only a socket: any other file at the socket path
    /// is an error and is never deleted), binds, and sets the socket mode to 0600.
    pub fn bind(endpoint: &Endpoint) -> Result<IpcServer, IpcError> {
        prepare_own_dir(endpoint)?;
        endpoint.verify_folder().map_err(|e| match e {
            IpcError::NotRunning => IpcError::InsecureEndpoint {
                path: endpoint.display(),
                reason: "the socket folder does not exist",
            },
            other => other,
        })?;
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

    /// Replaces the uid that connecting peers must have (tests of the peer check). The check cannot be switched
    /// off (test-only hook, §1.8.7).
    #[doc(hidden)]
    pub fn set_peer_uid(&mut self, uid: u32) {
        self.peer_uid = Some(uid);
    }

    /// Starts the accept thread (one thread per connection, plus one writer thread per connection) and returns
    /// its handle. Unix: rejects peers whose `peer_creds().euid()` differs from the owner uid of the socket file,
    /// and peers whose credentials cannot be read.
    pub fn spawn(self, handler: Arc<dyn IpcHandler>) -> Result<ServerHandle, IpcError> {
        let IpcServer {
            listener,
            endpoint,
            lock,
            peer_uid,
        } = self;
        let stop = Arc::new(AtomicBool::new(false));
        let registry = Arc::new(Mutex::new(Registry::default()));
        let (done_tx, done_rx) = crossbeam_channel::bounded::<()>(1);
        let (wake_tx, wake_rx) = crossbeam_channel::bounded::<()>(1);
        let thread_stop = Arc::clone(&stop);
        let thread_registry = Arc::clone(&registry);
        let thread = thread::Builder::new()
            .name("hikasa-ipc-accept".into())
            .spawn(move || {
                accept_loop(
                    &listener,
                    &thread_stop,
                    &handler,
                    peer_uid,
                    &wake_rx,
                    &thread_registry,
                );
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
            registry,
        })
    }
}

/// A running server.
pub struct ServerHandle {
    stop: Arc<AtomicBool>,
    endpoint: Endpoint,
    thread: Option<JoinHandle<()>>,
    done: Receiver<()>,
    wake: Sender<()>,
    registry: Arc<Mutex<Registry>>,
}

impl std::fmt::Debug for ServerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerHandle")
            .field("endpoint", &self.endpoint)
            .field("running", &self.thread.is_some())
            .finish_non_exhaustive()
    }
}

impl ServerHandle {
    /// Stops the server: sets the stop flag, wakes the blocked `accept` by connecting to its own endpoint once
    /// (self-connect; no sleeping or polling) and joins the accept thread; then every waiter still outstanding
    /// gets `ViewClosed` (reason `shutdown`, exit code 100: a GUI should notify its waiters with the views' real
    /// codes before) and every connection is closed after its last frame, so no client can block on a stopped
    /// server. Waits up to 2 s in total for the connections' writers to flush (§1.8.7).
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
        let accept = match self.done.recv_timeout(SHUTDOWN_WAIT) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                Ok(())
            }
            Err(RecvTimeoutError::Timeout) => Err(IpcError::Timeout),
        };
        self.close_connections();
        accept
    }

    /// Notifies the outstanding waiters, closes every connection and waits (bounded) for the writers.
    fn close_connections(&self) {
        let conns: Vec<Arc<Conn>> = {
            let mut registry = self.registry.lock();
            registry.shut = true;
            registry
                .conns
                .drain(..)
                .filter_map(|c| c.upgrade())
                .collect()
        };
        for conn in &conns {
            conn.shut_down();
        }
        let deadline = crossbeam_channel::after(SHUTDOWN_FLUSH);
        for conn in &conns {
            crossbeam_channel::select! {
                recv(conn.writer_done) -> _ => {}
                recv(deadline) -> _ => break,
            }
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        let _ = self.stop_server();
    }
}

/// Removes a socket left by a dead server (the instance lock proves no live server owns it). Only a socket is
/// removed: any other file at the socket path is reported and left alone (00 §4 safety).
#[cfg(unix)]
fn remove_stale_socket(endpoint: &Endpoint) -> Result<(), IpcError> {
    use std::os::unix::fs::FileTypeExt;
    if let super::EndpointAddress::UnixSocket(path) = &endpoint.address {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_socket() => match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(IpcError::Io(e)),
            },
            Ok(_) => {
                return Err(IpcError::InsecureEndpoint {
                    path: path.display().to_string(),
                    reason: "a file that is not a socket exists at the socket path",
                });
            }
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

/// Unix: sets the socket mode to 0600, checks that the socket file belongs to [`Endpoint::uid`], and returns
/// that uid for the per-connection peer check.
#[cfg(unix)]
fn secure_socket(endpoint: &Endpoint) -> Result<Option<u32>, IpcError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let super::EndpointAddress::UnixSocket(path) = &endpoint.address else {
        return Ok(None);
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    let owner = std::fs::symlink_metadata(path)?.uid();
    if endpoint.uid != Some(owner) {
        return Err(IpcError::InsecureEndpoint {
            path: path.display().to_string(),
            reason: "the socket belongs to another user",
        });
    }
    Ok(Some(owner))
}

/// Windows: per-user pipe names (an owner-only DACL is a P1 item, Appendix C).
#[cfg(not(unix))]
fn secure_socket(_endpoint: &Endpoint) -> Result<Option<u32>, IpcError> {
    Ok(None)
}

/// True when the peer may talk to this server: its effective uid equals `expected` (Unix). Credentials that
/// cannot be read close the connection (fail closed); a platform whose credentials carry no uid is allowed and
/// logged (a documented platform limitation: the private socket folder still protects the endpoint, §1.8.7).
#[cfg(unix)]
fn peer_allowed(stream: &Stream, expected: Option<u32>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    match stream.peer_creds() {
        Ok(creds) => match creds.euid() {
            Some(euid) => peer_uid_matches(Some(euid), expected),
            None => {
                tracing::warn!(target: "hikasa::ipc", "the platform reports no peer uid; connection accepted");
                true
            }
        },
        Err(e) => {
            tracing::warn!(target: "hikasa::ipc", "cannot read the peer's credentials ({e}); connection closed");
            false
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
    registry: &Arc<Mutex<Registry>>,
) {
    let connections = AtomicU64::new(0);
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
                let n = connections.fetch_add(1, Ordering::Relaxed) + 1;
                let handler = Arc::clone(handler);
                let registry = Arc::clone(registry);
                let spawned = thread::Builder::new()
                    .name(format!("hikasa-ipc-conn-{n}"))
                    .spawn(move || serve(stream, n, &handler, &registry));
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

/// Serves one connection until the client disconnects or breaks the protocol, then closes it (after the last
/// queued frame, e.g. `Error{protocol}`).
fn serve(stream: Stream, n: u64, handler: &Arc<dyn IpcHandler>, registry: &Arc<Mutex<Registry>>) {
    let conn = match Conn::start(stream, n) {
        Ok(conn) => conn,
        Err(e) => {
            tracing::warn!(target: "hikasa::ipc", "cannot start a connection writer: {e}");
            return;
        }
    };
    {
        let mut registry = registry.lock();
        if registry.shut {
            drop(registry);
            conn.close_after_flush();
            return;
        }
        registry.conns.retain(|c| c.strong_count() > 0);
        registry.conns.push(Arc::downgrade(&conn));
    }
    if handshake(&conn, handler) {
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
    conn.close_after_flush();
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
            let shared = Arc::new(Gate {
                id,
                state: Mutex::new(GateState::Pending(None)),
            });
            conn.track(&shared);
            let waiter = ViewWaiter {
                inner: Some(WaiterInner {
                    conn: Arc::clone(conn),
                    gate: Arc::clone(&shared),
                }),
            };
            gate = Some(shared);
            call_handler(handler, request, Some(waiter))
        }
        request @ (Request::OpenView { .. }
        | Request::ListViews
        | Request::CloseView { .. }
        | Request::Ping
        | Request::Shutdown) => call_handler(handler, request, None),
    };
    let sent = conn.send(id, &response);
    if let Some(gate) = gate {
        let opened = match (&sent, &response) {
            (Ok(()), Response::ViewOpened { view }) => Some(*view),
            _ => None,
        };
        gate.release(conn, opened);
    }
    sent
}

/// Calls the handler behind a panic boundary (a panic is answered with `Error{internal}`; a waiter the panic
/// dropped reports its view as abandoned).
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
