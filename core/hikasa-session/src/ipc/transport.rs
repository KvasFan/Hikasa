//! The `interprocess` local-socket glue shared by the server and the client (private).

use std::io;
use std::time::Duration;

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{
    ConnectOptions, GenericFilePath, GenericNamespaced, Name, Stream,
};

use super::endpoint::{Endpoint, EndpointAddress};
use super::error::IpcError;

/// The `interprocess` name of an endpoint: a file-system socket path, or a namespaced (named-pipe) name.
pub(crate) fn name_of(endpoint: &Endpoint) -> io::Result<Name<'_>> {
    match &endpoint.address {
        EndpointAddress::UnixSocket(path) => path.as_path().to_fs_name::<GenericFilePath>(),
        EndpointAddress::NamedPipe(name) => name.as_str().to_ns_name::<GenericNamespaced>(),
    }
}

/// Connects a blocking stream to `endpoint`. "Nobody listens" (no socket file, connection refused, no pipe)
/// is [`IpcError::NotRunning`]; a connect that cannot complete within `timeout` is [`IpcError::Timeout`].
pub(crate) fn connect_stream(endpoint: &Endpoint, timeout: Duration) -> Result<Stream, IpcError> {
    connect_sync(endpoint, timeout).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => IpcError::NotRunning,
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => IpcError::Timeout,
        _ => IpcError::Io(e),
    })
}

/// Unix: a plain blocking connect (connecting to a Unix socket does not wait for the server to accept).
#[cfg(not(windows))]
fn connect_sync(endpoint: &Endpoint, _timeout: Duration) -> io::Result<Stream> {
    ConnectOptions::new()
        .name(name_of(endpoint)?)
        .connect_sync()
}

/// Windows: a connect to a pipe whose every instance is busy waits for a free instance, and the local-socket
/// connect of `interprocess` waits without a limit (it ignores the connect wait mode). A stopped server cannot
/// close a named-pipe connection from another thread (§1.8.7), so while such a connection lingers its pipe
/// stays busy and a client would block forever. The pipe is therefore connected through the named-pipe API
/// with a bounded wait and then wrapped as a local-socket stream.
#[cfg(windows)]
fn connect_sync(endpoint: &Endpoint, timeout: Duration) -> io::Result<Stream> {
    use interprocess::ConnectWaitMode;
    use interprocess::os::windows::named_pipe::{DuplexPipeStream, local_socket, pipe_mode};
    use std::os::windows::io::OwnedHandle;

    let EndpointAddress::NamedPipe(name) = &endpoint.address else {
        return ConnectOptions::new()
            .name(name_of(endpoint)?)
            .connect_sync();
    };
    let path = format!(r"\\.\pipe\{name}");
    let pipe = DuplexPipeStream::<pipe_mode::Bytes>::connect_by_path_with_wait_mode(
        path.as_str(),
        ConnectWaitMode::Timeout(timeout.max(Duration::from_millis(1))),
    )?;
    let handle = OwnedHandle::try_from(pipe)
        .map_err(|_| io::Error::other("a freshly connected pipe stream is split"))?;
    Ok(Stream::from(local_socket::Stream::try_from(handle)?))
}
