//! The blocking IPC client (§1.8.6, §3.6, §4.4.3): [`IpcClient`], [`RetryPolicy`], [`connect_or_launch`].
//!
//! Before connecting, the client verifies the socket folder ([`Endpoint::verify_folder`]): a socket planted by
//! another user is never contacted. A reader thread (`hikasa-ipc-read`) decodes the server's frames into a
//! bounded channel; the caller waits on that channel with a deadline from `crossbeam_channel::after`, so the
//! timeout of an immediate response is one total budget per call, never per read, and no clock is read in this
//! crate (T1). Waiting for `ViewClosed` has no timeout. Messages for other requests are parked in a bounded
//! queue. The launch retry budget is the sum of the durations slept through `GuiLauncher::sleep`.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use interprocess::local_socket::Stream;
use interprocess::local_socket::prelude::*;

use super::PROTOCOL_VERSION;
use super::endpoint::Endpoint;
use super::error::IpcError;
use super::framing::{recv_response, send};
use super::protocol::{Envelope, IpcErrorCode, Request, Response, ServerInfo, ViewId, ViewResult};
use super::transport::connect_stream;
use crate::api::ApiError;
use crate::api::gui::{GuiLaunch, GuiLauncher, LaunchError};

/// Oldest protocol version this client accepts.
pub const MIN_PROTOCOL_VERSION: u32 = 1;

/// Messages for other requests a client keeps while it waits for a response; more is a protocol error.
pub const MAX_PARKED: usize = 16;

/// Decoded frames buffered between the reader thread and the caller.
const READ_QUEUE: usize = 16;

/// Retry policy for connecting after launching the GUI. The budget is the sum of the slept durations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// First delay (default 50 ms), doubled after each attempt up to `max_delay` (default 500 ms).
    pub initial_delay: Duration,
    /// Maximum delay.
    pub max_delay: Duration,
    /// Total budget of slept time (default 10 s); exceeded → `IpcError::LaunchTimeout`.
    pub total: Duration,
}

impl Default for RetryPolicy {
    /// 50 ms, 500 ms, 10 s.
    fn default() -> RetryPolicy {
        RetryPolicy {
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_millis(500),
            total: Duration::from_secs(10),
        }
    }
}

/// One item from the reader thread.
type Frame = Result<Envelope<Response>, IpcError>;

/// IPC client.
#[derive(Debug)]
pub struct IpcClient {
    stream: Arc<Stream>,
    frames: Receiver<Frame>,
    next_id: u64,
    server: ServerInfo,
    timeout: Duration,
    /// Messages that arrived while another response was awaited (a `ViewClosed` of an earlier wait).
    parked: VecDeque<Envelope<Response>>,
}

impl IpcClient {
    /// Verifies the socket folder ([`Endpoint::verify_folder`]: `InsecureEndpoint` for a folder another user
    /// controls; `NotRunning` when it does not exist), connects and performs the `Hello` handshake. Every
    /// immediate response must arrive within `timeout` after its request was sent (one budget per call,
    /// expiry → `IpcError::Timeout`); writes use `timeout` as the send timeout.
    pub fn connect(
        endpoint: &Endpoint,
        client: &str,
        timeout: Duration,
    ) -> Result<IpcClient, IpcError> {
        IpcClient::connect_with_versions(
            endpoint,
            client,
            timeout,
            PROTOCOL_VERSION,
            MIN_PROTOCOL_VERSION,
        )
    }

    /// [`IpcClient::connect`] announcing other protocol versions (tests of the negotiation; additive).
    #[doc(hidden)]
    pub fn connect_with_versions(
        endpoint: &Endpoint,
        client: &str,
        timeout: Duration,
        version: u32,
        min_version: u32,
    ) -> Result<IpcClient, IpcError> {
        endpoint.verify_folder()?;
        let stream = connect_stream(endpoint)?;
        let timeout = if timeout.is_zero() {
            Duration::from_millis(1)
        } else {
            timeout
        };
        // A platform without send timeouts still works; writes then block like the stream does.
        let _ = stream.set_send_timeout(Some(timeout));
        let stream = Arc::new(stream);
        let (tx, frames) = crossbeam_channel::bounded::<Frame>(READ_QUEUE);
        let reader_stream = Arc::clone(&stream);
        std::thread::Builder::new()
            .name("hikasa-ipc-read".into())
            .spawn(move || read_loop(&reader_stream, &tx))?;
        let mut this = IpcClient {
            stream,
            frames,
            next_id: 1,
            server: ServerInfo {
                server: String::new(),
                version: 0,
            },
            timeout,
            parked: VecDeque::new(),
        };
        let hello = Request::Hello {
            client: client.to_owned(),
            version,
            min_version,
        };
        match this.call(hello)?.1 {
            Response::Hello {
                server,
                version: negotiated,
            } if negotiated >= min_version && negotiated <= version => {
                this.server = ServerInfo {
                    server,
                    version: negotiated,
                };
                Ok(this)
            }
            Response::Hello {
                version: negotiated,
                ..
            } => Err(IpcError::IncompatibleVersion {
                client: version,
                server: negotiated,
            }),
            Response::Error { code, message }
                if code == IpcErrorCode::IncompatibleVersion.as_str() =>
            {
                Err(IpcError::IncompatibleVersion {
                    client: version,
                    server: server_version_in(&message),
                })
            }
            Response::Error { code, message } => Err(IpcError::Remote { code, message }),
            other => Err(unexpected(&other)),
        }
    }

    /// Negotiated server identity.
    pub fn server(&self) -> &ServerInfo {
        &self.server
    }

    /// Sends a request and returns its immediate response and the request id used.
    ///
    /// The whole call has one deadline (the connect timeout). A response of a type this build does not know is a
    /// protocol error (D-46). An `Error` response is returned as it is (see [`IpcClient::request`] for the
    /// variant that turns it into [`IpcError::Remote`]); a protocol error the server sends with id 0 (§4.4.1) is
    /// the failure of this call ([`IpcError::Remote`]). More than [`MAX_PARKED`] messages for other requests is a
    /// protocol error.
    pub fn call(&mut self, request: Request) -> Result<(u64, Response), IpcError> {
        let id = self.next_id;
        self.next_id += 1;
        let mut writer: &Stream = &self.stream;
        send(&mut writer, &Envelope { id, body: request })?;
        let deadline = crossbeam_channel::after(self.timeout);
        loop {
            let envelope = self.next_frame(Some(&deadline))?;
            if envelope.id == id {
                return match envelope.body {
                    Response::Unknown => Err(IpcError::Protocol(format!(
                        "the GUI answered request {id} with a message type this version does not know"
                    ))),
                    body => Ok((id, body)),
                };
            }
            self.park(envelope)?;
        }
    }

    /// [`IpcClient::call`] that turns `Error{code, message}` into [`IpcError::Remote`] (additive).
    pub fn request(&mut self, request: Request) -> Result<(u64, Response), IpcError> {
        match self.call(request)? {
            (_, Response::Error { code, message }) => Err(IpcError::Remote { code, message }),
            ok => Ok(ok),
        }
    }

    /// Blocks (no timeout) until `ViewClosed` for the `OpenView` request `request_id` arrives; EOF →
    /// `ConnectionLost`. Returns the raw exit code byte. An `Error` for the request (or with id 0) is
    /// [`IpcError::Remote`]; any other message for the request is a protocol error.
    pub fn wait_view_closed(
        &mut self,
        request_id: u64,
    ) -> Result<(ViewId, ViewResult, u8), IpcError> {
        if let Some(pos) = self
            .parked
            .iter()
            .position(|e| e.id == request_id && matches!(e.body, Response::ViewClosed { .. }))
            && let Some(Envelope {
                body:
                    Response::ViewClosed {
                        view,
                        result,
                        exit_code,
                    },
                ..
            }) = self.parked.remove(pos)
        {
            return Ok((view, result, exit_code));
        }
        loop {
            let envelope = self.next_frame(None)?;
            if envelope.id != request_id {
                self.park(envelope)?;
                continue;
            }
            return match envelope.body {
                Response::ViewClosed {
                    view,
                    result,
                    exit_code,
                } => Ok((view, result, exit_code)),
                Response::Error { code, message } => Err(IpcError::Remote { code, message }),
                other @ (Response::Hello { .. }
                | Response::ViewOpened { .. }
                | Response::Views { .. }
                | Response::Ok
                | Response::Pong
                | Response::Unknown) => Err(unexpected(&other)),
            };
        }
    }

    /// The next frame from the reader thread; `deadline` bounds the wait (`Timeout`). A server message with id 0
    /// is the failure of the pending request (§4.4.1: protocol errors carry id 0, then the server closes).
    fn next_frame(
        &mut self,
        deadline: Option<&Receiver<Instant>>,
    ) -> Result<Envelope<Response>, IpcError> {
        let received = match deadline {
            Some(deadline) => crossbeam_channel::select! {
                recv(self.frames) -> frame => frame,
                recv(deadline) -> _ => return Err(IpcError::Timeout),
            },
            None => self.frames.recv(),
        };
        let envelope = match received {
            Ok(Ok(envelope)) => envelope,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(IpcError::ConnectionLost),
        };
        if envelope.id == 0 {
            return Err(match envelope.body {
                Response::Error { code, message } => IpcError::Remote { code, message },
                other => IpcError::Protocol(format!(
                    "the GUI sent `{}` without a request id",
                    response_name(&other)
                )),
            });
        }
        Ok(envelope)
    }

    /// Keeps a message for another request (bounded).
    fn park(&mut self, envelope: Envelope<Response>) -> Result<(), IpcError> {
        if self.parked.len() >= MAX_PARKED {
            return Err(IpcError::Protocol(format!(
                "the GUI sent more than {MAX_PARKED} messages for other requests"
            )));
        }
        self.parked.push_back(envelope);
        Ok(())
    }
}

impl Drop for IpcClient {
    /// Closes the connection, which also ends the reader thread.
    fn drop(&mut self) {
        shutdown_stream(&self.stream);
    }
}

/// The reader thread: decodes frames until EOF, an error or a dropped client.
fn read_loop(stream: &Stream, frames: &Sender<Frame>) {
    loop {
        let mut reader: &Stream = stream;
        match recv_response(&mut reader) {
            Ok(Some(envelope)) => {
                if frames.send(Ok(envelope)).is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(e) => {
                let _ = frames.send(Err(e));
                break;
            }
        }
    }
}

/// Shuts a client connection down (Unix; on Windows the pipe closes when the last handle drops).
#[cfg(unix)]
fn shutdown_stream(stream: &Stream) {
    let Stream::UdSocket(socket) = stream;
    let _ = socket.inner().shutdown(std::net::Shutdown::Both);
}

/// See the Unix variant.
#[cfg(not(unix))]
fn shutdown_stream(_stream: &Stream) {}

/// The server version named in an `incompatible_version` message (`"… server speaks 1, …"`), else 0.
fn server_version_in(message: &str) -> u32 {
    message
        .split("server speaks ")
        .nth(1)
        .and_then(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .unwrap_or(0)
}

/// The protocol error of a response that does not fit the request.
pub(crate) fn unexpected(response: &Response) -> IpcError {
    IpcError::Protocol(format!("unexpected response `{}`", response_name(response)))
}

/// The wire name of a response's `type`.
fn response_name(response: &Response) -> &'static str {
    match response {
        Response::Hello { .. } => "hello",
        Response::ViewOpened { .. } => "view_opened",
        Response::ViewClosed { .. } => "view_closed",
        Response::Views { .. } => "views",
        Response::Ok => "ok",
        Response::Pong => "pong",
        Response::Error { .. } => "error",
        Response::Unknown => "unknown",
    }
}

/// Connects; if there is no GUI and `launcher` is given, launches it once with `launch` (the arguments and
/// environment that make the GUI serve this endpoint, `Service::gui_launch`) and retries per `policy`, sleeping
/// through `GuiLauncher::sleep`. Errors: no GUI and no launcher → `Ipc(NotRunning)`; launcher reports
/// `NotFound` → `GuiNotFound`; budget exhausted → `Ipc(LaunchTimeout)`; an insecure socket folder →
/// `Ipc(InsecureEndpoint)` without launching (exit 100).
///
/// The delays are `initial_delay`, doubled after each attempt up to `max_delay`; the last delay is shortened
/// so that the sum of the slept durations never exceeds `policy.total`. A spawn failure of the launcher is
/// `Ipc(Io)` (exit 100).
pub fn connect_or_launch(
    endpoint: &Endpoint,
    client: &str,
    launcher: Option<&dyn GuiLauncher>,
    launch: &GuiLaunch,
    policy: &RetryPolicy,
    timeout: Duration,
) -> Result<IpcClient, ApiError> {
    match IpcClient::connect(endpoint, client, timeout) {
        Err(IpcError::NotRunning) => {}
        other => return other.map_err(ApiError::from),
    }
    let Some(launcher) = launcher else {
        return Err(ApiError::Ipc(IpcError::NotRunning));
    };
    match launcher.launch(launch) {
        Ok(()) => {}
        Err(LaunchError::NotFound) => return Err(ApiError::GuiNotFound),
        Err(LaunchError::Spawn(e)) => return Err(ApiError::Ipc(IpcError::Io(e))),
    }
    let mut slept = Duration::ZERO;
    let mut delay = policy.initial_delay.max(Duration::from_millis(1));
    loop {
        let remaining = policy.total.saturating_sub(slept);
        if remaining.is_zero() {
            return Err(ApiError::Ipc(IpcError::LaunchTimeout));
        }
        let step = delay.min(remaining);
        launcher.sleep(step);
        slept += step;
        match IpcClient::connect(endpoint, client, timeout) {
            Ok(connected) => return Ok(connected),
            Err(IpcError::NotRunning) => {}
            Err(e) => return Err(ApiError::Ipc(e)),
        }
        delay = (delay * 2).min(policy.max_delay.max(Duration::from_millis(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_version_parsing() {
        assert_eq!(
            server_version_in("no common protocol version: server speaks 1, client requires 2..=3"),
            1
        );
        assert_eq!(server_version_in("whatever"), 0);
    }
}
