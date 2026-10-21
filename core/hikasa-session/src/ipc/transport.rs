//! The `interprocess` local-socket glue shared by the server and the client (private).

use std::io;

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
/// is [`IpcError::NotRunning`].
pub(crate) fn connect_stream(endpoint: &Endpoint) -> Result<Stream, IpcError> {
    let name = name_of(endpoint)?;
    ConnectOptions::new()
        .name(name)
        .connect_sync()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => IpcError::NotRunning,
            _ => IpcError::Io(e),
        })
}
