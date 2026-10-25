//! The scripted stand-in for `hikasa-gui` (contract §6.3, `cli_gui.rs`): the `cli_gui_launch` test binary runs
//! this instead of its cases when [`ENV`] is `1`. The tests launch that binary through `HIKASA_GUI_EXE` (with
//! `HIKASA_TEST_FAKE_GUI=1`, which the CLI and the launched process inherit), so no fake GUI is built as a
//! product binary. Like the real GUI it reads its command line with `parse_open_args` (D-51), resolves the
//! configuration (`--config-dir`, §2.22) and listens on the endpoint of that configuration, or on
//! `HIKASA_IPC_ENDPOINT` (§2.23).
//!
//! Environment:
//! - `HIKASA_FAKE_GUI_MODE`: `close` (default; a waited view closes at once with `HIKASA_FAKE_GUI_CODE`,
//!   default 13), `exit` (the process exits right after answering `ViewOpened`, so a waiting CLI sees the
//!   connection drop), or `deaf` (exits without listening: the launched GUI never becomes reachable);
//! - `HIKASA_FAKE_GUI_RECORD`: a file that receives the arguments (one per line, `arg: …`) and one line per
//!   request (`request: <type>`).
//!
//! The process ends shortly after its first view was handled, on `Shutdown`, or after 30 seconds.

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use hikasa_core::{ContentKind, ExitCode};
use hikasa_session::api::parse_open_args;
use hikasa_session::ipc::{
    CloseReason, Endpoint, IpcErrorCode, IpcHandler, IpcServer, Request, Response, ServerInfo,
    ViewId, ViewResult, ViewWaiter,
};
use hikasa_session::{Platform, capture_env, resolve_config_dir};

/// The variable that turns the test binary into the fake GUI.
pub const ENV: &str = "HIKASA_TEST_FAKE_GUI";

/// Longest life of the process.
const LIFETIME: Duration = Duration::from_secs(30);

/// Time between the last answer and the end of the process (the server's writer sends the frames first).
const LINGER: Duration = Duration::from_millis(400);

/// What the fake GUI does with a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Close,
    Exit,
    Deaf,
}

/// Appends one line to the record file, if any.
fn record(path: Option<&PathBuf>, line: &str) {
    if let Some(path) = path
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

struct Handler {
    mode: Mode,
    code: ExitCode,
    record: Option<PathBuf>,
    next: AtomicU64,
    done: Mutex<mpsc::Sender<()>>,
}

impl Handler {
    fn finish(&self) {
        if let Ok(done) = self.done.lock() {
            let _ = done.send(());
        }
    }
}

impl IpcHandler for Handler {
    fn server_info(&self) -> ServerInfo {
        ServerInfo {
            server: "fake-gui 1.0".into(),
            version: 1,
        }
    }

    fn handle(&self, request: Request, waiter: Option<ViewWaiter>) -> Response {
        record(
            self.record.as_ref(),
            &format!("request: {}", request.type_name()),
        );
        match request {
            Request::OpenView { specs, .. } => {
                let view = ViewId(self.next.fetch_add(1, Ordering::SeqCst));
                record(
                    self.record.as_ref(),
                    &format!("specs: {}", specs.join(" | ")),
                );
                match self.mode {
                    Mode::Close | Mode::Deaf => {
                        if let Some(waiter) = waiter {
                            let result = ViewResult::new(
                                Some(ContentKind::ImportantDifferences),
                                None,
                                false,
                                CloseReason::User,
                            );
                            waiter.view_closed(view, result, self.code);
                        }
                        self.finish();
                    }
                    Mode::Exit => {
                        // Keep the waiter alive (no `abandoned` message) and end the process once the answer is
                        // out: the CLI then sees the connection drop while it waits.
                        std::mem::forget(waiter);
                        let _ = std::thread::Builder::new().spawn(|| {
                            std::thread::sleep(LINGER);
                            std::process::exit(0);
                        });
                    }
                }
                Response::ViewOpened { view }
            }
            Request::Ping => Response::Pong,
            Request::ListViews => Response::Views { views: Vec::new() },
            Request::Shutdown => {
                self.finish();
                Response::Ok
            }
            _ => Response::error(IpcErrorCode::InvalidRequest, "not scripted"),
        }
    }
}

/// Runs the fake GUI with the process's arguments (the ones the launcher passed).
pub fn run() -> std::process::ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let record_path = std::env::var_os("HIKASA_FAKE_GUI_RECORD").map(PathBuf::from);
    for arg in &args {
        record(
            record_path.as_ref(),
            &format!("arg: {}", arg.to_string_lossy()),
        );
    }
    let mode = match std::env::var("HIKASA_FAKE_GUI_MODE").as_deref() {
        Ok("exit") => Mode::Exit,
        Ok("deaf") => Mode::Deaf,
        _ => Mode::Close,
    };
    if mode == Mode::Deaf {
        return std::process::ExitCode::SUCCESS;
    }
    let code = std::env::var("HIKASA_FAKE_GUI_CODE")
        .ok()
        .and_then(|c| c.parse::<u8>().ok())
        .and_then(ExitCode::from_code)
        .unwrap_or(ExitCode::RulesDifferent);
    let open = match parse_open_args(&args, Platform::HOST) {
        Ok(open) => open,
        Err(e) => {
            record(record_path.as_ref(), &format!("error: {e}"));
            return std::process::ExitCode::FAILURE;
        }
    };
    let facts = capture_env();
    let endpoint = resolve_config_dir(&facts, open.config_dir.as_deref(), &|p| p.is_file())
        .map_err(|e| e.to_string())
        .and_then(|config| Endpoint::for_config(&facts, &config).map_err(|e| e.to_string()));
    let endpoint = match endpoint {
        Ok(endpoint) => endpoint,
        Err(e) => {
            record(record_path.as_ref(), &format!("error: {e}"));
            return std::process::ExitCode::FAILURE;
        }
    };
    record(
        record_path.as_ref(),
        &format!("endpoint: {}", endpoint.display()),
    );
    let (done_tx, done_rx) = mpsc::channel();
    let handler = Handler {
        mode,
        code,
        record: record_path.clone(),
        next: AtomicU64::new(1),
        done: Mutex::new(done_tx),
    };
    let server = match IpcServer::bind(&endpoint).and_then(|s| s.spawn(Arc::new(handler))) {
        Ok(server) => server,
        Err(e) => {
            record(record_path.as_ref(), &format!("error: {e}"));
            return std::process::ExitCode::FAILURE;
        }
    };
    let _ = done_rx.recv_timeout(LIFETIME);
    std::thread::sleep(LINGER);
    let _ = server.shutdown();
    std::process::ExitCode::SUCCESS
}
