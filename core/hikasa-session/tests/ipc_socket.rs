//! Local-socket integration of the IPC server and client (§1.8.6, §2.23, §4.4.3; §6.2.8 "IPC"): handshake,
//! version negotiation, `OpenView{wait}` → `ViewClosed`, forward-compatible answers with the request's own id,
//! dropped connections, the instance lock, the 0700 folder check, the peer-uid check, `ServerHandle::shutdown`
//! through the self-connect, and `connect_or_launch` with a fake launcher. Every endpoint lives in a per-test
//! temporary folder.

mod common;

use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use common::{CloseMode, FakeGui, FakeLauncher, short_tempdir, test_endpoint};
use hikasa_core::ExitCode;
use hikasa_session::ApiError;
use hikasa_session::api::GuiLaunch;
use hikasa_session::ipc::{
    Endpoint, EndpointAddress, Envelope, InstanceLock, IpcClient, IpcError, IpcServer,
    MAX_FRAME_LEN, Request, Response, RetryPolicy, ViewId, ViewKind, connect_or_launch, read_frame,
    recv_response, send, write_frame,
};
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{ConnectOptions, GenericFilePath, GenericNamespaced, Stream};

const TIMEOUT: Duration = Duration::from_secs(10);

fn start(endpoint: &Endpoint, gui: Arc<FakeGui>) -> hikasa_session::ipc::ServerHandle {
    IpcServer::bind(endpoint).unwrap().spawn(gui).unwrap()
}

/// A raw stream to `endpoint` (no handshake), for protocol-level tests.
fn raw(endpoint: &Endpoint) -> Stream {
    let name = match &endpoint.address {
        EndpointAddress::UnixSocket(p) => p.as_path().to_fs_name::<GenericFilePath>().unwrap(),
        EndpointAddress::NamedPipe(n) => n.as_str().to_ns_name::<GenericNamespaced>().unwrap(),
    };
    let stream = ConnectOptions::new().name(name).connect_sync().unwrap();
    // Windows named pipes have no I/O timeouts (`Unsupported`); there a lost answer blocks the test.
    if let Err(e) = stream.set_recv_timeout(Some(TIMEOUT)) {
        assert!(
            cfg!(windows) && e.kind() == std::io::ErrorKind::Unsupported,
            "{e}"
        );
    }
    stream
}

fn send_json(stream: &Stream, json: &str) {
    let mut w: &Stream = stream;
    write_frame(&mut w, json.as_bytes()).unwrap();
}

fn recv(stream: &Stream) -> Option<Envelope<Response>> {
    let mut r: &Stream = stream;
    recv_response(&mut r).unwrap()
}

fn hello(stream: &Stream) {
    send_json(
        stream,
        r#"{"id":1,"type":"hello","client":"raw","version":1,"min_version":1}"#,
    );
    let reply = recv(stream).unwrap();
    assert_eq!(reply.id, 1);
    assert!(
        matches!(reply.body, Response::Hello { version: 1, .. }),
        "{reply:?}"
    );
}

#[test]
fn handshake_requests_and_views() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let gui = FakeGui::new(CloseMode::OnRequest, ExitCode::Similar);
    let server = start(&endpoint, Arc::clone(&gui));
    let mut client = IpcClient::connect(&endpoint, "test 1", TIMEOUT).unwrap();
    assert_eq!(client.server().server, "fake-gui 0.0.0");
    assert_eq!(client.server().version, 1);
    assert_eq!(client.call(Request::Ping).unwrap().1, Response::Pong);
    let (id, opened) = client
        .call(Request::OpenView {
            kind: ViewKind::TextCompare,
            specs: vec!["/a.c".into(), "/b.c".into()],
            options: Default::default(),
            wait: false,
        })
        .unwrap();
    assert!(id > 1, "ids strictly increase");
    let Response::ViewOpened { view } = opened else {
        panic!("{opened:?}")
    };
    let (_, views) = client.call(Request::ListViews).unwrap();
    assert!(
        matches!(&views, Response::Views { views } if views.len() == 1 && views[0].view == view)
    );
    assert_eq!(
        client.call(Request::CloseView { view }).unwrap().1,
        Response::Ok
    );
    let err = client
        .request(Request::CloseView {
            view: ViewId(12345),
        })
        .unwrap_err();
    assert!(
        matches!(&err, IpcError::Remote { code, .. } if code == "not_found"),
        "{err:?}"
    );
    // A response type this build does not know is a protocol error for the request (D-46).
    let unknown = client
        .call(Request::CloseView { view: ViewId(999) })
        .unwrap_err();
    assert!(matches!(unknown, IpcError::Protocol(_)), "{unknown:?}");
    // `RunScript` is answered by the server itself (rule 5); `LoadSession`/`LoadWorkspace` reach the handler
    // (D-89), and this fake answers like an M1 GUI.
    for request in [
        Request::LoadSession {
            name: "s".into(),
            wait: false,
        },
        Request::LoadWorkspace { name: "w".into() },
        Request::RunScript {
            path: "/x".into(),
            args: vec![],
        },
    ] {
        let err = client.request(request).unwrap_err();
        assert!(
            matches!(&err, IpcError::Remote { code, .. } if code == "unsupported_request"),
            "{err:?}"
        );
    }
    for kind in [ViewKind::TextMerge, ViewKind::Unknown] {
        let err = client
            .request(Request::OpenView {
                kind,
                specs: vec![],
                options: Default::default(),
                wait: true,
            })
            .unwrap_err();
        assert!(
            matches!(&err, IpcError::Remote { code, .. } if code == "unsupported_view"),
            "{err:?}"
        );
    }
    let duplicate_hello = client
        .request(Request::Hello {
            client: "again".into(),
            version: 1,
            min_version: 1,
        })
        .unwrap_err();
    assert!(matches!(&duplicate_hello, IpcError::Remote { code, .. } if code == "protocol"));
    assert_eq!(client.call(Request::Shutdown).unwrap().1, Response::Ok);
    drop(client);
    server.shutdown().unwrap();
    // Unknown requests, `RunScript` and a second `Hello` never reached the handler; `LoadSession` did (D-89).
    let seen = gui.requests.lock();
    assert!(seen.iter().all(|r| !matches!(
        r,
        Request::RunScript { .. } | Request::Unknown | Request::Hello { .. }
    )));
    assert!(
        seen.iter()
            .any(|r| matches!(r, Request::LoadSession { .. }))
    );
}

#[test]
fn version_negotiation() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let server = start(
        &endpoint,
        FakeGui::new(CloseMode::Immediately, ExitCode::Similar),
    );
    // client 2 / min 1 vs server 1 → 1.
    let client = IpcClient::connect_with_versions(&endpoint, "newer", TIMEOUT, 2, 1).unwrap();
    assert_eq!(client.server().version, 1);
    // client min 2 → incompatible.
    let err = IpcClient::connect_with_versions(&endpoint, "newest", TIMEOUT, 3, 2).unwrap_err();
    assert!(
        matches!(
            err,
            IpcError::IncompatibleVersion {
                client: 3,
                server: 1
            }
        ),
        "{err:?}"
    );
    drop(client);
    server.shutdown().unwrap();
}

#[test]
fn open_view_wait_gets_view_closed_after_view_opened() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    // The handler closes the view before returning: ViewClosed still comes after ViewOpened.
    let server = start(
        &endpoint,
        FakeGui::new(CloseMode::Immediately, ExitCode::Similar),
    );
    let mut client = IpcClient::connect(&endpoint, "t", TIMEOUT).unwrap();
    let (id, opened) = client
        .call(Request::OpenView {
            kind: ViewKind::TextCompare,
            specs: vec!["/a".into(), "/b".into()],
            options: Default::default(),
            wait: true,
        })
        .unwrap();
    let Response::ViewOpened { view } = opened else {
        panic!("{opened:?}")
    };
    let (closed, result, code) = client.wait_view_closed(id).unwrap();
    assert_eq!(closed, view);
    assert_eq!(code, 12);
    assert_eq!(result.verdict.as_deref(), Some("unimportant_only"));
    drop(client);
    server.shutdown().unwrap();

    // The view closes later (CloseView on a second connection); the waiting client receives it.
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let gui = FakeGui::new(CloseMode::OnRequest, ExitCode::RulesDifferent);
    let server = start(&endpoint, Arc::clone(&gui));
    let mut waiting = IpcClient::connect(&endpoint, "waiter", TIMEOUT).unwrap();
    let (id, opened) = waiting
        .call(Request::OpenView {
            kind: ViewKind::FolderCompare,
            specs: vec!["/a".into(), "/b".into()],
            options: Default::default(),
            wait: true,
        })
        .unwrap();
    let Response::ViewOpened { view } = opened else {
        panic!("{opened:?}")
    };
    let closer_endpoint = endpoint.clone();
    let closer = std::thread::spawn(move || {
        let mut other = IpcClient::connect(&closer_endpoint, "closer", TIMEOUT).unwrap();
        other.call(Request::CloseView { view }).unwrap().1
    });
    let (closed, _, code) = waiting.wait_view_closed(id).unwrap();
    assert_eq!((closed, code), (view, 13));
    assert_eq!(closer.join().unwrap(), Response::Ok);
    // After the wait the immediate-response timeout applies again and requests still work.
    assert_eq!(waiting.call(Request::Ping).unwrap().1, Response::Pong);
    drop(waiting);
    server.shutdown().unwrap();
}

#[test]
fn raw_protocol_errors_use_the_request_id() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let server = start(
        &endpoint,
        FakeGui::new(CloseMode::OnRequest, ExitCode::Similar),
    );

    let stream = raw(&endpoint);
    hello(&stream);
    // Unknown type → unsupported_request with the same id (the §4.4.3 wire example).
    send_json(&stream, r#"{"id":4,"type":"open_session_v9","name":"x"}"#);
    let reply = recv(&stream).unwrap();
    assert_eq!(reply.id, 4);
    assert_eq!(
        reply.body,
        Response::Error {
            code: "unsupported_request".into(),
            message: "unknown request type `open_session_v9`".into()
        }
    );
    // A known type with bad fields → invalid_request with the same id; the connection stays usable.
    send_json(&stream, r#"{"id":5,"type":"close_view","view":"seven"}"#);
    let reply = recv(&stream).unwrap();
    assert_eq!(reply.id, 5);
    assert!(matches!(&reply.body, Response::Error { code, .. } if code == "invalid_request"));
    // Unknown fields are ignored.
    send_json(&stream, r#"{"id":6,"type":"ping","extra":[1,2,3]}"#);
    assert_eq!(recv(&stream).unwrap().body, Response::Pong);
    // A frame without an id → protocol error with id 0, then the server closes.
    send_json(&stream, r#"{"type":"ping"}"#);
    let reply = recv(&stream).unwrap();
    assert_eq!(reply.id, 0);
    assert!(matches!(&reply.body, Response::Error { code, .. } if code == "protocol"));
    assert!(recv(&stream).is_none(), "closed after a protocol error");

    // The first message must be Hello.
    let stream = raw(&endpoint);
    send_json(&stream, r#"{"id":1,"type":"ping"}"#);
    let reply = recv(&stream).unwrap();
    assert!(matches!(&reply.body, Response::Error { code, .. } if code == "protocol"));
    assert!(recv(&stream).is_none());

    // An oversized frame header → protocol error, close.
    let stream = raw(&endpoint);
    hello(&stream);
    {
        use std::io::Write;
        let mut w: &Stream = &stream;
        w.write_all(&(MAX_FRAME_LEN + 1).to_le_bytes()).unwrap();
    }
    let reply = recv(&stream).unwrap();
    assert_eq!(reply.id, 0);
    assert!(matches!(&reply.body, Response::Error { code, .. } if code == "protocol"));
    drop(stream);
    server.shutdown().unwrap();
}

/// A scripted raw server: handshake, `ViewOpened`, then the connection drops while the client waits.
#[test]
fn dropped_connection_while_waiting_is_connection_lost() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let name = match &endpoint.address {
        EndpointAddress::UnixSocket(p) => p.as_path().to_fs_name::<GenericFilePath>().unwrap(),
        EndpointAddress::NamedPipe(n) => n.as_str().to_ns_name::<GenericNamespaced>().unwrap(),
    };
    let listener = interprocess::local_socket::ListenerOptions::new()
        .name(name)
        .create_sync()
        .unwrap();
    let server = std::thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let mut r: &Stream = &stream;
        let hello = hikasa_session::ipc::recv_request(&mut r).unwrap().unwrap();
        let mut w: &Stream = &stream;
        send(
            &mut w,
            &Envelope {
                id: hello.id,
                body: Response::Hello {
                    server: "scripted".into(),
                    version: 1,
                },
            },
        )
        .unwrap();
        let open = hikasa_session::ipc::recv_request(&mut r).unwrap().unwrap();
        send(
            &mut w,
            &Envelope {
                id: open.id,
                body: Response::ViewOpened { view: ViewId(1) },
            },
        )
        .unwrap();
        drop(stream);
    });
    let mut client = IpcClient::connect(&endpoint, "t", TIMEOUT).unwrap();
    let (id, _) = client
        .call(Request::OpenView {
            kind: ViewKind::TextCompare,
            specs: vec![],
            options: Default::default(),
            wait: true,
        })
        .unwrap();
    let err = client.wait_view_closed(id).unwrap_err();
    assert!(matches!(err, IpcError::ConnectionLost), "{err:?}");
    server.join().unwrap();
}

#[test]
fn not_running_and_the_single_instance_lock() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    assert!(matches!(
        IpcClient::connect(&endpoint, "t", TIMEOUT),
        Err(IpcError::NotRunning)
    ));
    let first = InstanceLock::try_acquire(&endpoint).unwrap();
    assert!(first.is_some());
    assert!(
        InstanceLock::try_acquire(&endpoint).unwrap().is_none(),
        "exclusive"
    );
    assert!(matches!(
        IpcServer::bind(&endpoint),
        Err(IpcError::AlreadyRunning)
    ));
    drop(first);
    let again = InstanceLock::try_acquire(&endpoint).unwrap();
    assert!(again.is_some(), "released on drop");
    drop(again);
    // Two configurations hold two locks.
    let other_dir = short_tempdir();
    let other = test_endpoint(other_dir.path());
    let a = InstanceLock::try_acquire(&endpoint).unwrap();
    let b = InstanceLock::try_acquire(&other).unwrap();
    assert!(a.is_some() && b.is_some());
    drop((a, b));
    // A running server holds the lock; a second server on the same endpoint is refused.
    let server = start(
        &endpoint,
        FakeGui::new(CloseMode::Immediately, ExitCode::Similar),
    );
    assert!(matches!(
        IpcServer::bind(&endpoint),
        Err(IpcError::AlreadyRunning)
    ));
    server.shutdown().unwrap();
    // After shutdown the endpoint is free again (socket removed, lock released).
    let server = start(
        &endpoint,
        FakeGui::new(CloseMode::Immediately, ExitCode::Similar),
    );
    server.shutdown().unwrap();
}

/// Regression (Windows CI port): a shutdown right after the server started must still wake the accept thread. A
/// pipe client that connected and closed before the listener waited for it is discarded by the listener, so the
/// self-connect stays open until the accept thread has ended.
#[test]
fn shutdown_right_after_the_start_wakes_the_accept_thread() {
    let dir = short_tempdir();
    for _ in 0..20 {
        let endpoint = test_endpoint(dir.path());
        let server = start(
            &endpoint,
            FakeGui::new(CloseMode::Immediately, ExitCode::Similar),
        );
        let started = Instant::now();
        server.shutdown().unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
    }
}

#[test]
fn shutdown_returns_quickly_through_the_self_connect() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let server = start(
        &endpoint,
        FakeGui::new(CloseMode::Immediately, ExitCode::Similar),
    );
    // A connected, idle client does not delay the shutdown.
    let idle = IpcClient::connect(&endpoint, "idle", TIMEOUT).unwrap();
    let started = Instant::now();
    server.shutdown().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    drop(idle);
    #[cfg(unix)]
    let endpoint = {
        assert!(matches!(
            IpcClient::connect(&endpoint, "t", TIMEOUT),
            Err(IpcError::NotRunning)
        ));
        if let EndpointAddress::UnixSocket(path) = &endpoint.address {
            assert!(!path.exists(), "the socket file is removed");
        }
        endpoint
    };
    // Windows: a stopped server cannot close a named-pipe connection, and the dropped client's reader thread
    // keeps its end open (§1.8.7), so the idle connection lingers and keeps the pipe busy. A new connect neither
    // reaches the stopped server nor waits forever: its wait is bounded by the timeout. The next server
    // needs another pipe name.
    #[cfg(windows)]
    let endpoint = {
        let started = Instant::now();
        let result = IpcClient::connect(&endpoint, "t", Duration::from_millis(300));
        assert!(
            matches!(result, Err(IpcError::Timeout | IpcError::NotRunning)),
            "{:?}",
            result.map(|_| ())
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        test_endpoint(dir.path())
    };
    // Dropping a handle shuts the server down too.
    let server = start(
        &endpoint,
        FakeGui::new(CloseMode::Immediately, ExitCode::Similar),
    );
    drop(server);
    assert!(matches!(
        IpcClient::connect(&endpoint, "t", TIMEOUT),
        Err(IpcError::NotRunning)
    ));
}

#[cfg(unix)]
#[test]
fn own_folder_is_created_0700_and_verified() {
    use std::os::unix::fs::PermissionsExt;
    let dir = short_tempdir();
    let own = dir.path().join("hikasa-1000");
    let endpoint = Endpoint {
        address: EndpointAddress::UnixSocket(own.join("hikasa.sock")),
        lock_path: own.join("hikasa.sock.lock"),
        own_dir: Some(own.clone()),
        uid: common::my_uid(),
    };
    let server = start(
        &endpoint,
        FakeGui::new(CloseMode::Immediately, ExitCode::Similar),
    );
    let mode = std::fs::metadata(&own).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    let socket_mode = std::fs::metadata(own.join("hikasa.sock"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(socket_mode & 0o777, 0o600);
    let mut client = IpcClient::connect(&endpoint, "t", TIMEOUT).unwrap();
    assert_eq!(client.call(Request::Ping).unwrap().1, Response::Pong);
    drop(client);
    server.shutdown().unwrap();
    // A folder other users can enter is refused.
    std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = IpcServer::bind(&endpoint).unwrap_err();
    assert!(matches!(err, IpcError::InsecureEndpoint { .. }), "{err:?}");
    assert!(matches!(
        InstanceLock::try_acquire(&endpoint),
        Err(IpcError::InsecureEndpoint { .. })
    ));
    // A symbolic link in place of the folder is refused.
    std::fs::remove_dir_all(&own).unwrap();
    let target = dir.path().join("elsewhere");
    std::fs::create_dir(&target).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&target, &own).unwrap();
    assert!(matches!(
        IpcServer::bind(&endpoint),
        Err(IpcError::InsecureEndpoint { .. })
    ));
}

#[cfg(unix)]
#[test]
fn peers_of_another_uid_are_rejected() {
    use hikasa_session::ipc::peer_uid_matches;
    use std::os::unix::fs::MetadataExt;
    assert!(peer_uid_matches(Some(1000), 1000));
    assert!(!peer_uid_matches(Some(1001), 1000));
    assert!(peer_uid_matches(None, 1000));
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let mut server = IpcServer::bind(&endpoint).unwrap();
    let me = std::fs::metadata(dir.path()).unwrap().uid();
    // Pretend the socket belongs to another user: our own connection is then a foreign peer.
    server.set_peer_uid(me.wrapping_add(1));
    let handle = server
        .spawn(FakeGui::new(CloseMode::Immediately, ExitCode::Similar))
        .unwrap();
    let err = IpcClient::connect(&endpoint, "intruder", TIMEOUT).unwrap_err();
    assert!(
        matches!(err, IpcError::ConnectionLost | IpcError::Io(_)),
        "{err:?}"
    );
    handle.shutdown().unwrap();
    // With the real owner uid the same client is accepted.
    let mut server = IpcServer::bind(&endpoint).unwrap();
    server.set_peer_uid(me);
    let handle = server
        .spawn(FakeGui::new(CloseMode::Immediately, ExitCode::Similar))
        .unwrap();
    assert!(IpcClient::connect(&endpoint, "owner", TIMEOUT).is_ok());
    handle.shutdown().unwrap();
}

#[test]
fn connect_or_launch_with_a_fake_launcher() {
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let policy = RetryPolicy {
        initial_delay: Duration::from_millis(50),
        max_delay: Duration::from_millis(500),
        total: Duration::from_millis(1700),
    };
    // No GUI, no launcher (`--no-launch`).
    let err = connect_or_launch(
        &endpoint,
        "t",
        None,
        &GuiLaunch::default(),
        &policy,
        TIMEOUT,
    )
    .unwrap_err();
    assert!(matches!(err, ApiError::Ipc(IpcError::NotRunning)));
    // No executable.
    let missing = FakeLauncher {
        not_found: true,
        ..FakeLauncher::default()
    };
    let err = connect_or_launch(
        &endpoint,
        "t",
        Some(&missing),
        &GuiLaunch::default(),
        &policy,
        TIMEOUT,
    )
    .unwrap_err();
    assert!(matches!(err, ApiError::GuiNotFound));
    assert!(missing.sleeps.lock().is_empty());
    // Regression (wave-5 review): an executable that cannot be started is no usable GUI executable (103), not
    // an IPC I/O failure (100).
    struct Unstartable;
    impl hikasa_session::api::GuiLauncher for Unstartable {
        fn launch(&self, _: &GuiLaunch) -> Result<(), hikasa_session::api::LaunchError> {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "/x/hikasa-gui: Permission denied",
            )
            .into())
        }
        fn sleep(&self, _: Duration) {
            unreachable!("nothing to wait for");
        }
    }
    let err = connect_or_launch(
        &endpoint,
        "t",
        Some(&Unstartable),
        &GuiLaunch::default(),
        &policy,
        TIMEOUT,
    )
    .unwrap_err();
    assert!(matches!(err, ApiError::GuiNotStartable(_)), "{err:?}");
    assert_eq!(
        err.exit_code(hikasa_session::ApiOp::GuiOpen { wait: true }),
        ExitCode::GuiNotFound
    );
    assert!(err.to_string().contains("/x/hikasa-gui"), "{err}");
    // A GUI that never comes up: the budget is exhausted, the sleeps sum to exactly the total.
    let never = FakeLauncher::default();
    let err = connect_or_launch(
        &endpoint,
        "t",
        Some(&never),
        &GuiLaunch::default(),
        &policy,
        TIMEOUT,
    )
    .unwrap_err();
    assert!(
        matches!(err, ApiError::Ipc(IpcError::LaunchTimeout)),
        "{err:?}"
    );
    let sleeps = never.sleeps.lock().clone();
    assert_eq!(never.launches.load(Ordering::SeqCst), 1);
    assert_eq!(sleeps.iter().sum::<Duration>(), policy.total);
    assert_eq!(
        sleeps,
        vec![
            Duration::from_millis(50),
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(400),
            Duration::from_millis(500),
            Duration::from_millis(450),
        ]
    );
    // The GUI comes up during the first sleep: connected after one retry.
    let server_endpoint = endpoint.clone();
    let holder = Arc::new(parking_lot::Mutex::new(None));
    let slot = Arc::clone(&holder);
    let starts = FakeLauncher {
        on_sleep: parking_lot::Mutex::new(Some(Box::new(move || {
            *slot.lock() = Some(start(
                &server_endpoint,
                FakeGui::new(CloseMode::Immediately, ExitCode::Similar),
            ));
        }))),
        ..FakeLauncher::default()
    };
    let client = connect_or_launch(
        &endpoint,
        "t",
        Some(&starts),
        &GuiLaunch::default(),
        &policy,
        TIMEOUT,
    )
    .unwrap();
    assert_eq!(client.server().server, "fake-gui 0.0.0");
    assert_eq!(starts.sleeps.lock().len(), 1);
    drop(client);
    if let Some(server) = holder.lock().take() {
        server.shutdown().unwrap();
    }
    // Windows: the dropped client's connection lingers in this process (§1.8.7 item 13) and keeps a server
    // instance of the pipe name, so a new server cannot create the name's first instance ("access denied"):
    // the next server needs another pipe name. (A GUI process that exits frees its pipes.)
    #[cfg(windows)]
    let endpoint = test_endpoint(dir.path());
    // A running GUI: no launch at all.
    let server = start(
        &endpoint,
        FakeGui::new(CloseMode::Immediately, ExitCode::Similar),
    );
    let unused = FakeLauncher::default();
    connect_or_launch(
        &endpoint,
        "t",
        Some(&unused),
        &GuiLaunch::default(),
        &policy,
        TIMEOUT,
    )
    .unwrap();
    assert_eq!(unused.launches.load(Ordering::SeqCst), 0);
    server.shutdown().unwrap();
}

#[test]
fn immediate_responses_time_out() {
    // A server that accepts and never answers: the client's Hello read times out (§4.4.3 rule 7).
    let dir = short_tempdir();
    let endpoint = test_endpoint(dir.path());
    let name = match &endpoint.address {
        EndpointAddress::UnixSocket(p) => p.as_path().to_fs_name::<GenericFilePath>().unwrap(),
        EndpointAddress::NamedPipe(n) => n.as_str().to_ns_name::<GenericNamespaced>().unwrap(),
    };
    let listener = interprocess::local_socket::ListenerOptions::new()
        .name(name)
        .create_sync()
        .unwrap();
    let (done_tx, done_rx) = crossbeam_channel::bounded::<()>(1);
    let server = std::thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let _ = done_rx.recv();
        drop(stream);
    });
    let started = Instant::now();
    let err = IpcClient::connect(&endpoint, "t", Duration::from_millis(200)).unwrap_err();
    assert!(matches!(err, IpcError::Timeout), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    done_tx.send(()).unwrap();
    server.join().unwrap();
    // Frames never exceed the limit (INV-IPC-3): a reader with a small limit refuses a bigger frame.
    let mut buf = Vec::new();
    write_frame(&mut buf, &[b'x'; 10]).unwrap();
    assert!(read_frame(&mut Cursor::new(buf), 5).is_err());
}
