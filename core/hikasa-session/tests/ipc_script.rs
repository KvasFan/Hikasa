//! A8.17 (§1.8.13, D-228): `RunScript` over a local socket. `ScriptFinished` follows the `Ok` with the request's id,
//! only when the request waits, also when the handler finishes before it answered; a second script while one runs is
//! `busy`; a server that stops (or a GUI that drops the waiter) during a waited script sends `ScriptFinished{100}`; an
//! older GUI's `unsupported_request` is "the running GUI does not support scripts; restart it" (exit 100). And the
//! `RunScript` `Service::gui_script_request` builds: absolute path, cwd, the referenced variables, UTF-8 only.

mod common;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::{CloseMode, FakeGui};
use crossbeam_channel::{Receiver, Sender};
use hikasa_core::ExitCode;
use hikasa_session::api::{ApiOp, GuiScriptOptions};
use hikasa_session::ipc::{
    Envelope, IpcErrorCode, IpcHandler, IpcServer, Request, Response, RetryPolicy, ScriptSummary,
    ScriptWaiter, ServerHandle, ServerInfo, ViewWaiter, recv_response, write_frame,
};
use hikasa_session::{ApiError, Service};
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{ConnectOptions, GenericFilePath, GenericNamespaced, Stream};

const TIMEOUT: Duration = Duration::from_secs(10);

/// How the scripted GUI ends a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finish {
    /// `script_finished` before `handle_script` returns (the gate holds it until the `Ok` is out).
    Immediately,
    /// The waiter is kept (the run goes on) and handed to the test.
    Held,
}

/// A GUI that runs scripts: one at a time (`busy`), records each `RunScript`.
struct ScriptGui {
    finish: Finish,
    running: AtomicBool,
    seen: parking_lot::Mutex<Vec<Request>>,
    held: Sender<Option<ScriptWaiter>>,
}

impl ScriptGui {
    fn new(finish: Finish) -> (Arc<ScriptGui>, Receiver<Option<ScriptWaiter>>) {
        let (held, waiters) = crossbeam_channel::unbounded();
        let gui = ScriptGui {
            finish,
            running: AtomicBool::new(false),
            seen: parking_lot::Mutex::new(Vec::new()),
            held,
        };
        (Arc::new(gui), waiters)
    }
}

impl IpcHandler for ScriptGui {
    fn server_info(&self) -> ServerInfo {
        ServerInfo {
            server: "script-gui 0.0.0".into(),
            version: 1,
        }
    }

    fn handle(&self, request: Request, _waiter: Option<ViewWaiter>) -> Response {
        match request {
            Request::Ping => Response::Pong,
            _ => Response::error(IpcErrorCode::InvalidRequest, "not scripted"),
        }
    }

    fn handle_script(&self, request: Request, waiter: Option<ScriptWaiter>) -> Response {
        self.seen.lock().push(request);
        if self.running.swap(true, Ordering::SeqCst) {
            return Response::error(
                IpcErrorCode::Busy,
                "a script is running in the window; wait for it or stop it",
            );
        }
        match self.finish {
            Finish::Immediately => {
                if let Some(waiter) = waiter {
                    let summary = ScriptSummary {
                        commands: 3,
                        ok: 2,
                        failed: 1,
                        ..ScriptSummary::default()
                    };
                    waiter.script_finished(ExitCode::UnknownError, summary);
                }
                self.running.store(false, Ordering::SeqCst);
            }
            Finish::Held => {
                let _ = self.held.send(waiter);
            }
        }
        Response::Ok
    }
}

fn start(service: &Service, gui: Arc<dyn IpcHandler>) -> ServerHandle {
    IpcServer::bind(&service.endpoint().unwrap())
        .unwrap()
        .spawn(gui)
        .unwrap()
}

/// A raw stream to the service's endpoint, after the handshake.
fn raw(service: &Service) -> Stream {
    let endpoint = service.endpoint().unwrap();
    let name = match &endpoint.address {
        hikasa_session::ipc::EndpointAddress::UnixSocket(p) => {
            p.as_path().to_fs_name::<GenericFilePath>().unwrap()
        }
        hikasa_session::ipc::EndpointAddress::NamedPipe(n) => {
            n.as_str().to_ns_name::<GenericNamespaced>().unwrap()
        }
    };
    let stream = ConnectOptions::new().name(name).connect_sync().unwrap();
    if let Err(e) = stream.set_recv_timeout(Some(TIMEOUT)) {
        assert!(
            cfg!(windows) && e.kind() == std::io::ErrorKind::Unsupported,
            "{e}"
        );
    }
    send_json(
        &stream,
        r#"{"id":1,"type":"hello","client":"raw","version":1,"min_version":1}"#,
    );
    assert!(matches!(recv(&stream).body, Response::Hello { .. }));
    stream
}

fn send_json(stream: &Stream, json: &str) {
    let mut w: &Stream = stream;
    write_frame(&mut w, json.as_bytes()).unwrap();
}

fn recv(stream: &Stream) -> Envelope<Response> {
    let mut r: &Stream = stream;
    recv_response(&mut r).unwrap().unwrap()
}

/// A `RunScript` of `path` that waits or not.
fn run_script(path: &str, wait: bool) -> Request {
    Request::RunScript {
        path: path.into(),
        args: vec![],
        cwd: None,
        env: BTreeMap::new(),
        wait,
        stop_on_error: false,
        confirm: None,
        close: None,
    }
}

#[test]
fn a8_17_script_finished_follows_ok_only_when_waited() {
    let tmp = common::short_tempdir();
    let service = common::service(tmp.path());
    let (gui, _) = ScriptGui::new(Finish::Immediately);
    let server = start(&service, gui);
    let stream = raw(&service);
    // Waited: `Ok`, then one `ScriptFinished` with the request's id (the handler finished before it answered).
    send_json(
        &stream,
        r#"{"id":2,"type":"run_script","path":"/s.txt","wait":true}"#,
    );
    let first = recv(&stream);
    assert_eq!((first.id, first.body), (2, Response::Ok));
    let second = recv(&stream);
    assert_eq!(second.id, 2);
    assert_eq!(
        second.body,
        Response::ScriptFinished {
            exit_code: 100,
            summary: ScriptSummary {
                commands: 3,
                ok: 2,
                failed: 1,
                ..ScriptSummary::default()
            },
        }
    );
    // Not waited: `Ok` only; the next frame is the answer to the next request.
    send_json(&stream, r#"{"id":3,"type":"run_script","path":"/s.txt"}"#);
    let ok = recv(&stream);
    assert_eq!((ok.id, ok.body), (3, Response::Ok));
    send_json(&stream, r#"{"id":4,"type":"ping"}"#);
    let pong = recv(&stream);
    assert_eq!((pong.id, pong.body), (4, Response::Pong));
    drop(stream);
    server.shutdown().unwrap();
}

#[test]
fn a8_17_a_second_script_is_busy_and_a_finished_one_reaches_the_waiting_client() {
    let tmp = common::short_tempdir();
    let service = common::service(tmp.path());
    let (gui, waiters) = ScriptGui::new(Finish::Held);
    let server = start(&service, Arc::clone(&gui) as Arc<dyn IpcHandler>);
    let waiting = {
        let service = service.clone();
        std::thread::spawn(move || {
            service.gui_run_script(run_script("/a.txt", true), None, &RetryPolicy::default())
        })
    };
    let waiter = waiters
        .recv_timeout(TIMEOUT)
        .unwrap()
        .expect("a waited run has a waiter");
    // One script at a time.
    let busy = service
        .gui_run_script(run_script("/b.txt", false), None, &RetryPolicy::default())
        .unwrap_err();
    assert!(
        matches!(&busy, ApiError::Ipc(hikasa_session::ipc::IpcError::Remote { code, message })
            if code == "busy" && message == "a script is running in the window; wait for it or stop it"),
        "{busy:?}"
    );
    assert_eq!(
        busy.exit_code(ApiOp::GuiOpen { wait: false }),
        ExitCode::UnknownError
    );
    let summary = ScriptSummary {
        commands: 2,
        ok: 1,
        not_run: 1,
        stopped_at: Some(3),
        ..ScriptSummary::default()
    };
    waiter.script_finished(ExitCode::LoadFailed, summary);
    let report = waiting.join().unwrap().unwrap();
    assert_eq!(report.exit_code(), 107);
    assert_eq!(report.finished, Some((107, summary)));
    let dto = serde_json::to_value(report.dto()).unwrap();
    assert_eq!(dto["waited"], true);
    assert_eq!(dto["summary"]["stopped_at"], 3);
    server.shutdown().unwrap();
}

#[test]
fn a8_17_a_stopping_server_or_a_dropped_waiter_sends_100() {
    let tmp = common::short_tempdir();
    let service = common::service(tmp.path());
    let (gui, waiters) = ScriptGui::new(Finish::Held);
    let server = start(&service, gui);
    let waiting = {
        let service = service.clone();
        std::thread::spawn(move || {
            service.gui_run_script(run_script("/a.txt", true), None, &RetryPolicy::default())
        })
    };
    // The GUI keeps the run going; the server stops.
    let waiter = waiters.recv_timeout(TIMEOUT).unwrap();
    server.shutdown().unwrap();
    let report = waiting.join().unwrap().unwrap();
    assert_eq!(report.finished, Some((100, ScriptSummary::default())));
    drop(waiter);
    // A GUI that drops the waiter without finishing: 100 as well.
    let tmp = common::short_tempdir();
    let service = common::service(tmp.path());
    let (gui, waiters) = ScriptGui::new(Finish::Held);
    let server = start(&service, gui);
    let waiting = {
        let service = service.clone();
        std::thread::spawn(move || {
            service.gui_run_script(run_script("/a.txt", true), None, &RetryPolicy::default())
        })
    };
    drop(waiters.recv_timeout(TIMEOUT).unwrap());
    let report = waiting.join().unwrap().unwrap();
    assert_eq!(report.exit_code(), 100);
    server.shutdown().unwrap();
}

#[test]
fn a8_17_an_older_gui_does_not_support_scripts() {
    let tmp = common::short_tempdir();
    let service = common::service(tmp.path());
    let server = start(
        &service,
        FakeGui::new(CloseMode::OnRequest, ExitCode::Similar),
    );
    let err = service
        .gui_run_script(run_script("/a.txt", true), None, &RetryPolicy::default())
        .unwrap_err();
    assert!(matches!(&err, ApiError::Unsupported(_)), "{err:?}");
    assert_eq!(
        err.to_string(),
        "the running GUI does not support scripts; restart it"
    );
    assert_eq!(
        err.exit_code(ApiOp::GuiOpen { wait: true }),
        ExitCode::UnknownError
    );
    // No GUI and no launcher: as `gui open --no-launch` (100; 102 when waiting).
    server.shutdown().unwrap();
    let err = service
        .gui_run_script(run_script("/a.txt", false), None, &RetryPolicy::default())
        .unwrap_err();
    assert_eq!(
        err.exit_code(ApiOp::GuiOpen { wait: false }),
        ExitCode::UnknownError
    );
}

#[test]
fn a8_17_the_request_carries_the_path_cwd_and_the_referenced_variables() {
    let tmp = common::short_tempdir();
    let service = common::service(tmp.path());
    let file = common::write(tmp.path(), "work/s.txt", b"log %LOGS%/x.txt\nbeep\n");
    let source = service.script_locate(&OsString::from("s.txt")).unwrap();
    let text = service.script_load(source, None).unwrap();
    let references = service.script_references(&text);
    assert_eq!(references.env.iter().collect::<Vec<_>>(), ["LOGS"]);
    let env = BTreeMap::from([("LOGS".to_owned(), OsString::from("/var/log"))]);
    let options = GuiScriptOptions {
        wait: true,
        stop_on_error: true,
        confirm: Some("no-to-all".into()),
        closescript: true,
    };
    let args = [OsString::from("a b"), OsString::from("c")];
    let request = service
        .gui_script_request(&text, &args, &env, &options)
        .unwrap();
    assert_eq!(
        request,
        Request::RunScript {
            path: file.to_str().unwrap().to_owned(),
            args: vec!["a b".into(), "c".into()],
            cwd: Some(tmp.path().join("work").to_str().unwrap().to_owned()),
            env: BTreeMap::from([("LOGS".to_owned(), "/var/log".to_owned())]),
            wait: true,
            stop_on_error: true,
            confirm: Some("no-to-all".into()),
            close: Some(true),
        }
    );
    // Without --closescript the option decides (`None`).
    let plain = service
        .gui_script_request(&text, &[], &BTreeMap::new(), &GuiScriptOptions::default())
        .unwrap();
    assert!(matches!(
        plain,
        Request::RunScript {
            close: None,
            wait: false,
            ..
        }
    ));
    // Standard input cannot be sent.
    let stdin = service
        .script_load(
            hikasa_session::api::script::ScriptSource::Stdin,
            Some(&mut std::io::Cursor::new(b"beep\n".to_vec())),
        )
        .unwrap();
    let err = service
        .gui_script_request(&stdin, &[], &BTreeMap::new(), &options)
        .unwrap_err();
    assert!(err.to_string().contains("use hikasa script run"), "{err}");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let bad = OsString::from_vec(vec![b'a', 0xFF]);
        let err = service
            .gui_script_request(
                &text,
                std::slice::from_ref(&bad),
                &BTreeMap::new(),
                &options,
            )
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "argument %1 cannot be sent to the GUI: not valid UTF-8; use hikasa script run"
        );
        assert_eq!(
            err.exit_code(ApiOp::GuiOpen { wait: true }),
            ExitCode::UnknownError
        );
        let env = BTreeMap::from([("LOGS".to_owned(), bad)]);
        let err = service
            .gui_script_request(&text, &[], &env, &options)
            .unwrap_err();
        assert!(err.to_string().starts_with("the value of %LOGS%"), "{err}");
    }
}
