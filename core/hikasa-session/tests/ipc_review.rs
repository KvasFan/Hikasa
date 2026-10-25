//! Regression tests of the wave-4 IPC review (§1.8.7; decisions 1 and 2): the client verifies the socket folder
//! before it talks to anything; the server refuses folders it does not own and never deletes a file that is not
//! a socket; a dropped `ViewWaiter` and `ServerHandle::shutdown` always answer a waiting client; `view_closed`
//! never blocks on a stuck client; a protocol error closes the connection; request timeouts are one budget per
//! call; the parked queue is bounded; a protocol error with id 0 fails the pending request.
#![cfg(unix)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::{CloseMode, FakeGui, FakeLauncher, my_uid, short_tempdir, test_endpoint};
use hikasa_core::ExitCode;
use hikasa_session::api::{GuiLaunch, GuiOpenRequest};
use hikasa_session::ipc::{
    ABANDONED_EXIT_CODE, CloseReason, Endpoint, EndpointAddress, Envelope, IpcClient, IpcError,
    IpcHandler, IpcServer, Request, Response, RetryPolicy, ServerInfo, ViewId, ViewKind,
    ViewWaiter, connect_or_launch, recv_response, write_frame,
};
use hikasa_session::{ApiError, ApiOp};
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{ConnectOptions, GenericFilePath, ListenerOptions, Stream};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Runs `f` on a thread and waits at most `limit` for it.
fn within<T: Send + 'static>(limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(limit).ok()
}

/// A raw client stream (no handshake).
fn raw(endpoint: &Endpoint) -> Stream {
    let EndpointAddress::UnixSocket(path) = &endpoint.address else {
        unreachable!("Unix-only suite");
    };
    let stream = ConnectOptions::new()
        .name(path.as_path().to_fs_name::<GenericFilePath>().unwrap())
        .connect_sync()
        .unwrap();
    stream.set_recv_timeout(Some(TIMEOUT)).unwrap();
    stream
}

fn send_json(stream: &Stream, json: &str) {
    let mut w: &Stream = stream;
    write_frame(&mut w, json.as_bytes()).unwrap();
}

fn recv(stream: &Stream) -> Result<Option<Envelope<Response>>, IpcError> {
    let mut r: &Stream = stream;
    recv_response(&mut r)
}

fn hello(stream: &Stream) {
    send_json(
        stream,
        r#"{"id":1,"type":"hello","client":"raw","version":1,"min_version":1}"#,
    );
    assert!(matches!(
        recv(stream).unwrap().unwrap().body,
        Response::Hello { .. }
    ));
}

fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// A squatter: a plain listener at the endpoint's socket path that counts connections and answers `hello`
/// as "squatter".
fn squatter(endpoint: &Endpoint) -> Arc<AtomicUsize> {
    let EndpointAddress::UnixSocket(path) = &endpoint.address else {
        unreachable!("Unix-only suite");
    };
    let listener = ListenerOptions::new()
        .name(path.as_path().to_fs_name::<GenericFilePath>().unwrap())
        .create_sync()
        .unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            count.fetch_add(1, Ordering::SeqCst);
            let mut r: &Stream = &stream;
            if let Ok(Some(_)) = hikasa_session::ipc::recv_request(&mut r) {
                let mut w: &Stream = &stream;
                let _ = write_frame(
                    &mut w,
                    br#"{"id":1,"type":"hello","server":"squatter","version":1}"#,
                );
            }
        }
    });
    accepted
}

#[cfg(unix)]
#[test]
fn client_refuses_a_socket_folder_other_users_control() {
    let dir = short_tempdir();
    // Hikasa's own fallback folder, pre-created world-writable by "another user".
    let own = dir.path().join("hikasa-own");
    std::fs::create_dir(&own).unwrap();
    set_mode(&own, 0o777);
    let endpoint = Endpoint {
        address: EndpointAddress::UnixSocket(own.join("hikasa.sock")),
        lock_path: own.join("hikasa.sock.lock"),
        own_dir: Some(own.clone()),
        uid: my_uid(),
    };
    let accepted = squatter(&endpoint);
    let err = IpcClient::connect(&endpoint, "victim", TIMEOUT).unwrap_err();
    assert!(matches!(err, IpcError::InsecureEndpoint { .. }), "{err:?}");
    // The real GUI refuses the folder as well.
    assert!(matches!(
        IpcServer::bind(&endpoint),
        Err(IpcError::InsecureEndpoint { .. })
    ));
    // Group- or world-writable folders are refused for any endpoint (decision 1), private ones need 0700.
    set_mode(&own, 0o755);
    assert!(matches!(
        IpcClient::connect(&endpoint, "victim", TIMEOUT),
        Err(IpcError::InsecureEndpoint { .. })
    ));
    let plain = Endpoint {
        own_dir: None,
        ..endpoint.clone()
    };
    set_mode(&own, 0o775);
    assert!(matches!(
        IpcClient::connect(&plain, "victim", TIMEOUT),
        Err(IpcError::InsecureEndpoint { .. })
    ));
    // A folder owned by somebody else (simulated: the endpoint expects another uid).
    set_mode(&own, 0o700);
    let foreign = Endpoint {
        uid: my_uid().map(|u| u.wrapping_add(1)),
        ..endpoint.clone()
    };
    assert!(matches!(
        IpcClient::connect(&foreign, "victim", TIMEOUT),
        Err(IpcError::InsecureEndpoint { reason, .. }) if reason.contains("another user")
    ));
    // An unknown uid fails closed.
    let unknown = Endpoint {
        uid: None,
        ..endpoint.clone()
    };
    assert!(matches!(
        IpcClient::connect(&unknown, "victim", TIMEOUT),
        Err(IpcError::InsecureEndpoint { .. })
    ));
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        0,
        "the squatter never saw a connection"
    );
    // connect_or_launch never launches into an insecure folder, and the error is 100 for every gui command.
    set_mode(&own, 0o777);
    let launcher = FakeLauncher::default();
    let err = connect_or_launch(
        &endpoint,
        "victim",
        Some(&launcher),
        &GuiLaunch::default(),
        &RetryPolicy::default(),
        TIMEOUT,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ApiError::Ipc(IpcError::InsecureEndpoint { .. })
    ));
    assert_eq!(launcher.launches.load(Ordering::SeqCst), 0);
    for op in [
        ApiOp::GuiOpen { wait: true },
        ApiOp::GuiOpen { wait: false },
        ApiOp::GuiControl,
    ] {
        assert_eq!(err.exit_code(op), ExitCode::UnknownError);
    }
    // A missing folder simply means "not running".
    let missing = test_endpoint(&dir.path().join("nope"));
    assert!(matches!(
        IpcClient::connect(&missing, "c", TIMEOUT),
        Err(IpcError::NotRunning)
    ));
}

#[cfg(unix)]
#[test]
fn client_refuses_a_symlinked_own_folder_and_the_service_reports_100() {
    let tmp = short_tempdir();
    let target = tmp.path().join("elsewhere");
    std::fs::create_dir(&target).unwrap();
    set_mode(&target, 0o700);
    // The service's endpoint without $XDG_RUNTIME_DIR: <temp>/hikasa-<uid>/hikasa-<h8>.sock; a squatter put a
    // symbolic link there.
    let mut facts = common::facts(tmp.path());
    facts.ipc_endpoint_override = None;
    facts.xdg_runtime_dir = None;
    let init = hikasa_session::ServiceInit {
        config_dir: Some(tmp.path().join("cfg")),
        ..hikasa_session::ServiceInit::new(
            facts,
            common::fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    let service = hikasa_session::Service::new(init).unwrap();
    let endpoint = service.endpoint().unwrap();
    let own = endpoint.own_dir.clone().expect("own fallback folder");
    std::os::unix::fs::symlink(&target, &own).unwrap();
    let err = service.gui_ping().unwrap_err();
    assert!(
        matches!(err, ApiError::Ipc(IpcError::InsecureEndpoint { .. })),
        "{err:?}"
    );
    assert_eq!(err.exit_code(ApiOp::GuiControl), ExitCode::UnknownError);
    assert!(matches!(
        IpcServer::bind(&endpoint),
        Err(IpcError::InsecureEndpoint { .. })
    ));
}

#[cfg(unix)]
#[test]
fn server_creates_its_own_folder_private_and_refuses_foreign_ones() {
    use std::os::unix::fs::PermissionsExt;
    let dir = short_tempdir();
    let own = dir.path().join("hikasa-own");
    let endpoint = Endpoint {
        address: EndpointAddress::UnixSocket(own.join("hikasa.sock")),
        lock_path: own.join("hikasa.sock.lock"),
        own_dir: Some(own.clone()),
        uid: my_uid(),
    };
    let server = IpcServer::bind(&endpoint).unwrap();
    assert_eq!(
        std::fs::metadata(&own).unwrap().permissions().mode() & 0o777,
        0o700
    );
    drop(server);
    // The same folder with an owner the server does not expect is refused, never reused.
    let foreign = Endpoint {
        uid: my_uid().map(|u| u.wrapping_add(1)),
        ..endpoint.clone()
    };
    assert!(matches!(
        IpcServer::bind(&foreign),
        Err(IpcError::InsecureEndpoint { .. })
    ));
}

#[cfg(unix)]
#[test]
fn bind_never_deletes_a_file_that_is_not_a_socket() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let EndpointAddress::UnixSocket(path) = &endpoint.address else {
        unreachable!()
    };
    std::fs::write(path, b"precious user data").unwrap();
    let err = IpcServer::bind(&endpoint).unwrap_err();
    assert!(matches!(err, IpcError::InsecureEndpoint { .. }), "{err:?}");
    assert_eq!(std::fs::read(path).unwrap(), b"precious user data");
    // A stale socket is still replaced.
    std::fs::remove_file(path).unwrap();
    let first = IpcServer::bind(&endpoint).unwrap();
    drop(first);
    let second = IpcServer::bind(&endpoint).unwrap();
    drop(second);
}

/// A GUI whose views drop their waiters (an error path that forgets `view_closed`).
struct Dropper;

impl IpcHandler for Dropper {
    fn server_info(&self) -> ServerInfo {
        ServerInfo {
            server: "dropper".into(),
            version: 1,
        }
    }

    fn handle(&self, request: Request, waiter: Option<ViewWaiter>) -> Response {
        match request {
            Request::OpenView { .. } => {
                drop(waiter);
                Response::ViewOpened { view: ViewId(5) }
            }
            Request::Ping => Response::Pong,
            _ => Response::Ok,
        }
    }
}

#[test]
fn a_dropped_waiter_closes_the_view_as_abandoned() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let server = IpcServer::bind(&endpoint)
        .unwrap()
        .spawn(Arc::new(Dropper))
        .unwrap();
    let ep = endpoint.clone();
    let outcome = within(Duration::from_secs(5), move || {
        let mut client = IpcClient::connect(&ep, "t", TIMEOUT).unwrap();
        let (id, response) = client
            .call(Request::OpenView {
                kind: ViewKind::TextCompare,
                specs: vec!["/a".into(), "/b".into()],
                options: Default::default(),
                wait: true,
            })
            .unwrap();
        assert_eq!(response, Response::ViewOpened { view: ViewId(5) });
        client.wait_view_closed(id)
    })
    .expect("gui open --wait must never hang on a dropped waiter");
    let (view, result, code) = outcome.unwrap();
    assert_eq!(view, ViewId(5));
    assert_eq!(result.close_reason(), Some(CloseReason::Abandoned));
    assert_eq!(code, ABANDONED_EXIT_CODE.code());
    assert_eq!(code, 100);
    server.shutdown().unwrap();
}

#[test]
fn shutdown_notifies_waiters_and_closes_every_connection() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let gui = FakeGui::new(CloseMode::OnRequest, ExitCode::Similar);
    let server = IpcServer::bind(&endpoint)
        .unwrap()
        .spawn(Arc::clone(&gui) as Arc<dyn IpcHandler>)
        .unwrap();
    let ep = endpoint.clone();
    let (opened_tx, opened_rx) = crossbeam_channel::bounded(1);
    let waiting = std::thread::spawn(move || {
        let mut client = IpcClient::connect(&ep, "t", TIMEOUT).unwrap();
        let (id, _) = client
            .call(Request::OpenView {
                kind: ViewKind::TextCompare,
                specs: vec!["/a".into(), "/b".into()],
                options: Default::default(),
                wait: true,
            })
            .unwrap();
        opened_tx.send(()).unwrap();
        client.wait_view_closed(id)
    });
    opened_rx.recv_timeout(TIMEOUT).unwrap();
    let mut idle = IpcClient::connect(&endpoint, "idle", TIMEOUT).unwrap();
    let started = Instant::now();
    server.shutdown().unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    let (_, result, code) = waiting.join().unwrap().unwrap();
    assert_eq!(result.close_reason(), Some(CloseReason::Shutdown));
    assert_eq!(code, 100);
    // The idle connection was closed too: the next request fails at once instead of hanging.
    let err = idle.call(Request::Ping).unwrap_err();
    assert!(
        matches!(err, IpcError::ConnectionLost | IpcError::Io(_)),
        "{err:?}"
    );
    // A waiter the GUI still holds after the shutdown is a no-op (exactly one ViewClosed).
    let (view, waiter) = gui.waiters.lock().pop().unwrap();
    waiter.view_closed(
        view,
        hikasa_session::ipc::ViewResult::default(),
        ExitCode::Similar,
    );
}

#[cfg(unix)]
#[test]
fn view_closed_never_blocks_on_a_client_that_stopped_reading() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let gui = FakeGui::new(CloseMode::OnRequest, ExitCode::Similar);
    let server = IpcServer::bind(&endpoint)
        .unwrap()
        .spawn(Arc::clone(&gui) as Arc<dyn IpcHandler>)
        .unwrap();
    let stream = raw(&endpoint);
    hello(&stream);
    send_json(
        &stream,
        r#"{"id":2,"type":"open_view","kind":"text_compare","specs":["/a","/b"],"wait":true}"#,
    );
    assert!(matches!(
        recv(&stream).unwrap().unwrap().body,
        Response::ViewOpened { .. }
    ));
    // Flood pings and never read the pongs, until the server stops reading (its queue is full and the client
    // is dropped) or our send buffer is full.
    stream
        .set_send_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    for id in 3..200_000u64 {
        let mut w: &Stream = &stream;
        let ping = format!(r#"{{"id":{id},"type":"ping"}}"#);
        if write_frame(&mut w, ping.as_bytes()).is_err() {
            break;
        }
    }
    let (view, waiter) = gui.waiters.lock().pop().unwrap();
    let done = within(Duration::from_secs(2), move || {
        waiter.view_closed(
            view,
            hikasa_session::ipc::ViewResult::default(),
            ExitCode::Similar,
        );
    });
    assert!(done.is_some(), "view_closed blocked on a stalled client");
    server.shutdown().unwrap();
}

#[cfg(unix)]
#[test]
fn a_protocol_error_closes_a_connection_that_has_a_live_waiter() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let gui = FakeGui::new(CloseMode::OnRequest, ExitCode::Similar);
    let server = IpcServer::bind(&endpoint)
        .unwrap()
        .spawn(Arc::clone(&gui) as Arc<dyn IpcHandler>)
        .unwrap();
    let stream = raw(&endpoint);
    hello(&stream);
    send_json(
        &stream,
        r#"{"id":2,"type":"open_view","kind":"text_compare","specs":["/a","/b"],"wait":true}"#,
    );
    assert!(matches!(
        recv(&stream).unwrap().unwrap().body,
        Response::ViewOpened { .. }
    ));
    let mut w: &Stream = &stream;
    std::io::Write::write_all(&mut w, &[3, 0, 0, 0, b'{', b'x', b'}']).unwrap();
    let reply = recv(&stream).unwrap().unwrap();
    assert_eq!(reply.id, 0);
    assert!(matches!(reply.body, Response::Error { ref code, .. } if code == "protocol"));
    assert!(
        matches!(recv(&stream), Ok(None) | Err(IpcError::ConnectionLost)),
        "the connection is closed after Error{{protocol}}"
    );
    drop(gui.waiters.lock().pop());
    server.shutdown().unwrap();
}

/// A fake server that answers `hello` and then runs `script` on the connection.
fn scripted_server(
    endpoint: &Endpoint,
    script: impl FnOnce(&Stream) + Send + 'static,
) -> std::thread::JoinHandle<()> {
    let EndpointAddress::UnixSocket(path) = &endpoint.address else {
        unreachable!("Unix-only suite");
    };
    let listener = ListenerOptions::new()
        .name(path.as_path().to_fs_name::<GenericFilePath>().unwrap())
        .create_sync()
        .unwrap();
    std::thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let mut r: &Stream = &stream;
        let _ = hikasa_session::ipc::recv_request(&mut r);
        let mut w: &Stream = &stream;
        write_frame(
            &mut w,
            br#"{"id":1,"type":"hello","server":"scripted","version":1}"#,
        )
        .unwrap();
        script(&stream);
    })
}

#[cfg(unix)]
#[test]
fn a_chatty_server_cannot_defeat_the_request_timeout() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let (stop_tx, stop_rx) = crossbeam_channel::bounded::<()>(1);
    let server = scripted_server(&endpoint, move |stream| {
        // A frame for another id every 100 ms, never the answer.
        for _ in 0..100 {
            if stop_rx.recv_timeout(Duration::from_millis(100)).is_ok() {
                return;
            }
            let mut w: &Stream = stream;
            if write_frame(&mut w, br#"{"id":999,"type":"pong"}"#).is_err() {
                return;
            }
        }
    });
    let ep = endpoint.clone();
    let outcome = within(Duration::from_secs(4), move || {
        let mut client = IpcClient::connect(&ep, "t", Duration::from_millis(500)).unwrap();
        client.call(Request::Ping).map(|_| ())
    });
    let _ = stop_tx.send(());
    server.join().unwrap();
    assert!(
        matches!(outcome, Some(Err(IpcError::Timeout))),
        "one deadline per call: {outcome:?}"
    );
}

#[cfg(unix)]
#[test]
fn the_parked_queue_is_bounded() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let server = scripted_server(&endpoint, |stream| {
        let mut r: &Stream = stream;
        let _ = hikasa_session::ipc::recv_request(&mut r);
        for id in 100..200 {
            let mut w: &Stream = stream;
            let frame = format!(r#"{{"id":{id},"type":"pong"}}"#);
            if write_frame(&mut w, frame.as_bytes()).is_err() {
                return;
            }
        }
        // Keep the connection open until the client closes it.
        let mut r: &Stream = stream;
        while let Ok(Some(_)) = hikasa_session::ipc::recv_request(&mut r) {}
    });
    let mut client = IpcClient::connect(&endpoint, "t", TIMEOUT).unwrap();
    let err = client.call(Request::Ping).unwrap_err();
    assert!(matches!(err, IpcError::Protocol(_)), "{err:?}");
    drop(client);
    server.join().unwrap();
}

#[cfg(unix)]
#[test]
fn a_protocol_error_with_id_zero_fails_the_pending_request() {
    let tmp = short_tempdir();
    let service = common::service(tmp.path());
    let endpoint = service.endpoint().unwrap();
    let server = scripted_server(&endpoint, |stream| {
        let mut r: &Stream = stream;
        let _ = hikasa_session::ipc::recv_request(&mut r);
        let mut w: &Stream = stream;
        let _ = write_frame(
            &mut w,
            br#"{"id":0,"type":"error","code":"protocol","message":"the frame is not valid JSON"}"#,
        );
    });
    let w = tmp.path().join("work");
    let a = common::write(&w, "a.txt", b"a\n");
    let b = common::write(&w, "b.txt", b"b\n");
    let err = service
        .gui_open(
            &GuiOpenRequest {
                specs: vec![common::os(&a), common::os(&b)],
                wait: true,
                ..GuiOpenRequest::default()
            },
            None,
        )
        .unwrap_err();
    server.join().unwrap();
    assert!(
        matches!(err, ApiError::Ipc(IpcError::Remote { .. })),
        "{err:?}"
    );
    assert_eq!(
        err.exit_code(ApiOp::GuiOpen { wait: true }),
        ExitCode::UnknownError
    );
}
