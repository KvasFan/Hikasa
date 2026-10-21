//! The blocking IPC client (§1.8.6, §3.6, §4.4.3): [`IpcClient`], [`RetryPolicy`], [`connect_or_launch`].
//!
//! Timeouts are client-side and use the socket's receive timeout (`set_recv_timeout`), never a clock: an
//! immediate response must arrive within the connect timeout (10 s in the service); waiting for `ViewClosed`
//! has no timeout. The launch retry budget is the sum of the durations slept through
//! `GuiLauncher::sleep` (T1).

use std::collections::VecDeque;
use std::time::Duration;

use interprocess::local_socket::Stream;
use interprocess::local_socket::prelude::*;

use super::PROTOCOL_VERSION;
use super::endpoint::Endpoint;
use super::error::IpcError;
use super::framing::{recv_response, send};
use super::protocol::{Envelope, IpcErrorCode, Request, Response, ServerInfo, ViewId, ViewResult};
use super::transport::connect_stream;
use crate::api::ApiError;
use crate::api::gui::{GuiLauncher, LaunchError};

/// Oldest protocol version this client accepts.
pub const MIN_PROTOCOL_VERSION: u32 = 1;

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

/// IPC client.
#[derive(Debug)]
pub struct IpcClient {
    stream: Stream,
    next_id: u64,
    server: ServerInfo,
    timeout: Duration,
    /// Messages that arrived while another response was awaited (a `ViewClosed` of an earlier wait).
    parked: VecDeque<Envelope<Response>>,
}

impl IpcClient {
    /// Connects and performs the `Hello` handshake. Every read of an immediate response is bounded by
    /// `timeout` through `interprocess`'s `Stream::set_recv_timeout` (expiry → `IpcError::Timeout`).
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
        let stream = connect_stream(endpoint)?;
        let timeout = if timeout.is_zero() {
            Duration::from_millis(1)
        } else {
            timeout
        };
        stream.set_recv_timeout(Some(timeout))?;
        stream.set_send_timeout(Some(timeout))?;
        let mut this = IpcClient {
            stream,
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
    /// A response of a type this build does not know is a protocol error (D-46). An `Error` response is returned
    /// as it is (see [`IpcClient::request`] for the variant that turns it into [`IpcError::Remote`]).
    pub fn call(&mut self, request: Request) -> Result<(u64, Response), IpcError> {
        let id = self.next_id;
        self.next_id += 1;
        let mut writer: &Stream = &self.stream;
        send(&mut writer, &Envelope { id, body: request })?;
        loop {
            let mut reader: &Stream = &self.stream;
            let Some(envelope) = recv_response(&mut reader)? else {
                return Err(IpcError::ConnectionLost);
            };
            if envelope.id != id {
                self.parked.push_back(envelope);
                continue;
            }
            return match envelope.body {
                Response::Unknown => Err(IpcError::Protocol(format!(
                    "the GUI answered request {id} with a message type this version does not know"
                ))),
                body => Ok((id, body)),
            };
        }
    }

    /// [`IpcClient::call`] that turns `Error{code, message}` into [`IpcError::Remote`] (additive).
    pub fn request(&mut self, request: Request) -> Result<(u64, Response), IpcError> {
        match self.call(request)? {
            (_, Response::Error { code, message }) => Err(IpcError::Remote { code, message }),
            ok => Ok(ok),
        }
    }

    /// Blocks (receive timeout disabled) until `ViewClosed` for the `OpenView` request `request_id` arrives;
    /// EOF → `ConnectionLost`. Returns the raw exit code byte.
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
        self.stream.set_recv_timeout(None)?;
        let outcome = loop {
            let mut reader: &Stream = &self.stream;
            let envelope = match recv_response(&mut reader) {
                Ok(Some(envelope)) => envelope,
                Ok(None) => break Err(IpcError::ConnectionLost),
                Err(e) => break Err(e),
            };
            if envelope.id != request_id {
                self.parked.push_back(envelope);
                continue;
            }
            match envelope.body {
                Response::ViewClosed {
                    view,
                    result,
                    exit_code,
                } => break Ok((view, result, exit_code)),
                Response::Error { code, message } => break Err(IpcError::Remote { code, message }),
                _ => continue,
            }
        };
        let _ = self.stream.set_recv_timeout(Some(self.timeout));
        outcome
    }
}

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
    let name = match response {
        Response::Hello { .. } => "hello",
        Response::ViewOpened { .. } => "view_opened",
        Response::ViewClosed { .. } => "view_closed",
        Response::Views { .. } => "views",
        Response::Ok => "ok",
        Response::Pong => "pong",
        Response::Error { .. } => "error",
        Response::Unknown => "unknown",
    };
    IpcError::Protocol(format!("unexpected response `{name}`"))
}

/// Connects; if there is no GUI and `launcher` is given, launches it once and retries per `policy`, sleeping
/// through `GuiLauncher::sleep`. Errors: no GUI and no launcher → `Ipc(NotRunning)`; launcher reports
/// `NotFound` → `GuiNotFound`; budget exhausted → `Ipc(LaunchTimeout)`.
///
/// The delays are `initial_delay`, doubled after each attempt up to `max_delay`; the last delay is shortened
/// so that the sum of the slept durations never exceeds `policy.total`. A spawn failure of the launcher is
/// `Ipc(Io)` (exit 100).
pub fn connect_or_launch(
    endpoint: &Endpoint,
    client: &str,
    launcher: Option<&dyn GuiLauncher>,
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
    match launcher.launch() {
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
