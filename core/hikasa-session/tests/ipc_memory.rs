//! Peak memory per frame is bounded (§1.8.7, decision 2 of the wave-4 review): a 16 MiB frame of tiny JSON
//! values is refused before the two-phase decode builds a value tree (before the fix one such frame raised the
//! peak RSS by about 0.5 GiB). Its own test binary, so that the process-wide peak (`VmHWM`) measures only it.
#![cfg(target_os = "linux")]

mod common;

use std::time::Duration;

use common::{CloseMode, FakeGui, short_tempdir, test_endpoint};
use hikasa_core::ExitCode;
use hikasa_session::ipc::{
    EndpointAddress, IpcServer, MAX_FRAME_LEN, MAX_FRAME_VALUES, Response, recv_response,
    write_frame,
};
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{ConnectOptions, GenericFilePath, Stream};

/// The peak resident set of this process (KiB).
fn vm_hwm_kib() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .unwrap()
}

#[test]
fn a_frame_of_millions_of_values_is_refused_without_amplification() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let server = IpcServer::bind(&endpoint)
        .unwrap()
        .spawn(FakeGui::new(CloseMode::Immediately, ExitCode::Similar))
        .unwrap();
    let EndpointAddress::UnixSocket(path) = &endpoint.address else {
        unreachable!()
    };
    let stream = ConnectOptions::new()
        .name(path.as_path().to_fs_name::<GenericFilePath>().unwrap())
        .connect_sync()
        .unwrap();
    stream
        .set_recv_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut w: &Stream = &stream;
    write_frame(
        &mut w,
        br#"{"id":1,"type":"hello","client":"raw","version":1,"min_version":1}"#,
    )
    .unwrap();
    let mut r: &Stream = &stream;
    assert!(matches!(
        recv_response(&mut r).unwrap().unwrap().body,
        Response::Hello { .. }
    ));
    let mut payload = String::from(r#"{"id":2,"type":"ping","x":["#);
    while payload.len() + 3 < MAX_FRAME_LEN as usize {
        payload.push_str("0,");
    }
    payload.push_str("0]}");
    assert!(payload.len() / 2 > MAX_FRAME_VALUES);
    let before = vm_hwm_kib();
    write_frame(&mut w, payload.as_bytes()).unwrap();
    let reply = recv_response(&mut r).unwrap().unwrap();
    assert_eq!(reply.id, 0);
    assert!(
        matches!(&reply.body, Response::Error { code, message } if code == "protocol" && message.contains("JSON values")),
        "{reply:?}"
    );
    let grown = vm_hwm_kib().saturating_sub(before);
    assert!(
        grown < 64 * 1024,
        "one 16 MiB frame raised the peak RSS by {grown} KiB"
    );
    // A normal message still works on a new connection.
    let mut client =
        hikasa_session::ipc::IpcClient::connect(&endpoint, "t", Duration::from_secs(10)).unwrap();
    assert_eq!(
        client.call(hikasa_session::ipc::Request::Ping).unwrap().1,
        Response::Pong
    );
    drop(client);
    server.shutdown().unwrap();
}
