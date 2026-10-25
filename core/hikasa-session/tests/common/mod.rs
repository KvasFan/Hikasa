//! Shared helpers of the hikasa-session integration tests: the golden helper (§6.1, `HIKASA_BLESS=1` rewrites),
//! literal host facts, a service in a temporary configuration, fake conversion runners and GUI launchers.
#![allow(dead_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use hikasa_core::{CancelToken, JobKind, NullProgress};
use hikasa_formats::{ConvertError, ProcessRunner};
use hikasa_session::api::StdinSource;
use hikasa_session::api::gui::{GuiLaunch, GuiLauncher, LaunchError};
use hikasa_session::{EnvFacts, FixedClock, JobControl, Platform, Service, ServiceInit};
use parking_lot::Mutex;

/// Compares `actual` with the golden file; `HIKASA_BLESS=1` rewrites it instead (§6.1).
pub fn assert_golden(expected: &Path, actual: &[u8]) {
    if std::env::var_os("HIKASA_BLESS").is_some_and(|v| v == "1") {
        if let Some(dir) = expected.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(expected, actual).unwrap();
        return;
    }
    let wanted = std::fs::read(expected).unwrap_or_else(|e| {
        panic!(
            "golden file {} is missing ({e}); run with HIKASA_BLESS=1",
            expected.display()
        )
    });
    if wanted != actual {
        match (std::str::from_utf8(&wanted), std::str::from_utf8(actual)) {
            (Ok(w), Ok(a)) => pretty_assertions::assert_eq!(w, a, "golden {}", expected.display()),
            _ => panic!("golden {} differs (binary)", expected.display()),
        }
    }
}

/// The fixtures folder of a suite.
pub fn fixtures(suite: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(suite)
}

/// A temporary folder with a short path (Unix socket paths are limited to ~100 bytes).
pub fn short_tempdir() -> tempfile::TempDir {
    #[cfg(unix)]
    {
        tempfile::Builder::new()
            .prefix("hks")
            .tempdir_in("/tmp")
            .unwrap()
    }
    #[cfg(not(unix))]
    {
        tempfile::tempdir().unwrap()
    }
}

/// The effective uid of the test process (the socket and spool folders are verified against it; tests may read
/// the environment, §6.1).
pub fn my_uid() -> Option<u32> {
    hikasa_session::capture_env().uid
}

/// Host facts for a test rooted at `root`: cwd `root/work`, home `root/home`, temp `root/tmp`, no XDG runtime
/// dir, an explicit IPC endpoint override at `root/s.sock` (never the real runtime dir), the real uid.
pub fn facts(root: &Path) -> EnvFacts {
    for sub in ["work", "home", "tmp"] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    EnvFacts {
        cwd: Some(root.join("work")),
        home: Some(root.join("home")),
        uid: my_uid(),
        user: Some("tester".into()),
        ipc_endpoint_override: Some(endpoint_override(root)),
        ..EnvFacts::new(Platform::HOST, root.join("tmp"))
    }
}

/// A per-test IPC endpoint: a socket in `root` (Unix), a uniquely named pipe (Windows).
fn endpoint_override(root: &Path) -> OsString {
    #[cfg(unix)]
    {
        root.join("s.sock").into_os_string()
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        OsString::from(format!(
            "hikasa-svc-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ))
    }
}

/// 2026-10-21T12:00:00Z in UTC.
pub fn fixed_clock() -> Arc<FixedClock> {
    let ts = jiff::Timestamp::from_second(1_792_584_000).unwrap();
    Arc::new(FixedClock(ts.to_zoned(jiff::tz::TimeZone::UTC)))
}

/// A service with a configuration folder `root/config` (explicit), 4 threads, the fake runner.
pub fn service(root: &Path) -> Service {
    service_with(root, None, Arc::new(UpperRunner::default()))
}

/// A service with an optional stdin source and a given runner.
pub fn service_with(
    root: &Path,
    stdin: Option<StdinSource>,
    runner: Arc<dyn ProcessRunner>,
) -> Service {
    let init = ServiceInit {
        config_dir: Some(root.join("config")),
        threads: std::num::NonZeroUsize::new(4),
        stdin,
        ..ServiceInit::new(facts(root), fixed_clock(), runner)
    };
    Service::new(init).unwrap()
}

/// A stdin source with known content.
pub fn blob(bytes: &[u8], name: Option<&str>) -> StdinSource {
    StdinSource::from_bytes(bytes.to_vec(), name.map(str::to_owned))
}

/// A job control for direct calls.
pub fn job(service: &Service, kind: JobKind) -> JobControl {
    service.job(kind, Arc::new(NullProgress))
}

/// Writes `content` to `root/rel`, creating folders.
pub fn write(root: &Path, rel: &str, content: &[u8]) -> PathBuf {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

/// An `OsString` of a path.
pub fn os(path: &Path) -> OsString {
    path.as_os_str().to_owned()
}

/// A conversion runner that writes the upper-cased source to the target (`argv = [prog, %s, %t]`) and records
/// the threads it ran on.
#[derive(Default)]
pub struct UpperRunner {
    pub calls: AtomicUsize,
    pub threads: Mutex<Vec<String>>,
}

impl ProcessRunner for UpperRunner {
    fn run(
        &self,
        argv: &[OsString],
        _timeout: Option<Duration>,
        cancel: &CancelToken,
    ) -> Result<i32, ConvertError> {
        cancel.check()?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.threads.lock().push(
            std::thread::current()
                .name()
                .unwrap_or("<unnamed>")
                .to_owned(),
        );
        let source = std::fs::read(&argv[1])?;
        std::fs::write(&argv[2], source.to_ascii_uppercase())?;
        Ok(0)
    }
}

/// A conversion runner that panics (the panic boundary tests).
pub struct PanicRunner;

impl ProcessRunner for PanicRunner {
    fn run(
        &self,
        _argv: &[OsString],
        _timeout: Option<Duration>,
        _cancel: &CancelToken,
    ) -> Result<i32, ConvertError> {
        panic!("the fake converter exploded");
    }
}

/// A GUI launcher that records its calls and never starts anything.
#[derive(Default)]
pub struct FakeLauncher {
    pub launches: AtomicUsize,
    /// The launch requests received.
    pub requests: Mutex<Vec<GuiLaunch>>,
    pub sleeps: Mutex<Vec<Duration>>,
    pub not_found: bool,
    /// Called on the first sleep (e.g. to start a server, simulating the GUI coming up).
    pub on_sleep: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl GuiLauncher for FakeLauncher {
    fn launch(&self, launch: &GuiLaunch) -> Result<(), LaunchError> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().push(launch.clone());
        if self.not_found {
            Err(LaunchError::NotFound)
        } else {
            Ok(())
        }
    }

    fn sleep(&self, d: Duration) {
        self.sleeps.lock().push(d);
        if let Some(f) = self.on_sleep.lock().take() {
            f();
        }
    }
}

/// A user format with an External conversion (`conv %s %t`) for `*.up` files.
pub const EXTERNAL_FORMAT: &str = r#"schema_version = 1
id = "upper"
name = "Upper"
kind = "text"
mask = "*.up"

[conversion]
method = "external"
load = "conv %s %t"
"#;

// ---- IPC helpers ---------------------------------------------------------------------------------------------

use hikasa_core::{ContentKind, ExitCode};
use hikasa_session::ipc::{
    CloseReason, Endpoint, EndpointAddress, IpcHandler, Request, Response, ServerInfo, ViewId,
    ViewInfo, ViewResult, ViewWaiter,
};

/// A private endpoint inside `dir` (a Unix socket; a uniquely named pipe on Windows).
pub fn test_endpoint(dir: &Path) -> Endpoint {
    #[cfg(unix)]
    {
        Endpoint {
            address: EndpointAddress::UnixSocket(dir.join("t.sock")),
            lock_path: dir.join("t.sock.lock"),
            own_dir: None,
            uid: my_uid(),
        }
    }
    #[cfg(not(unix))]
    {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let name = format!(
            "hikasa-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        );
        Endpoint {
            lock_path: dir.join(format!("{name}.lock")),
            address: EndpointAddress::NamedPipe(name),
            own_dir: None,
            uid: None,
        }
    }
}

/// What a view does when it is opened with `wait`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseMode {
    /// The handler calls `view_closed` before returning (tests the ordering gate).
    Immediately,
    /// The waiter is kept until `CloseView` or `Shutdown`.
    OnRequest,
}

/// A scripted GUI: opens views, lists and closes them, answers pings; `CloseView { view: 999 }` returns
/// `Response::Unknown` (a message type the client does not know).
pub struct FakeGui {
    pub mode: CloseMode,
    pub exit_code: ExitCode,
    pub next: std::sync::atomic::AtomicU64,
    pub views: Mutex<Vec<ViewInfo>>,
    pub waiters: Mutex<Vec<(ViewId, ViewWaiter)>>,
    pub requests: Mutex<Vec<Request>>,
}

impl FakeGui {
    pub fn new(mode: CloseMode, exit_code: ExitCode) -> Arc<FakeGui> {
        Arc::new(FakeGui {
            mode,
            exit_code,
            next: std::sync::atomic::AtomicU64::new(7),
            views: Mutex::new(Vec::new()),
            waiters: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn result(&self, reason: CloseReason) -> ViewResult {
        ViewResult::new(Some(ContentKind::UnimportantOnly), None, false, reason)
    }
}

impl IpcHandler for FakeGui {
    fn server_info(&self) -> ServerInfo {
        ServerInfo {
            server: "fake-gui 0.0.0".into(),
            version: 1,
        }
    }

    fn handle(&self, request: Request, waiter: Option<ViewWaiter>) -> Response {
        self.requests.lock().push(request.clone());
        match request {
            Request::OpenView { kind, specs, .. } => {
                let view = ViewId(self.next.fetch_add(1, Ordering::SeqCst));
                self.views.lock().push(ViewInfo {
                    view,
                    kind,
                    title: specs.join(" | "),
                    specs,
                });
                if let Some(waiter) = waiter {
                    match self.mode {
                        CloseMode::Immediately => {
                            waiter.view_closed(view, self.result(CloseReason::User), self.exit_code)
                        }
                        CloseMode::OnRequest => self.waiters.lock().push((view, waiter)),
                    }
                }
                Response::ViewOpened { view }
            }
            Request::ListViews => Response::Views {
                views: self.views.lock().clone(),
            },
            Request::CloseView { view } if view.0 == 999 => Response::Unknown,
            Request::CloseView { view } => {
                let known = {
                    let mut views = self.views.lock();
                    let before = views.len();
                    views.retain(|v| v.view != view);
                    views.len() != before
                };
                if !known {
                    return Response::error(
                        hikasa_session::ipc::IpcErrorCode::NotFound,
                        format!("no view {}", view.0),
                    );
                }
                let mut waiters = self.waiters.lock();
                if let Some(pos) = waiters.iter().position(|(v, _)| *v == view) {
                    let (v, w) = waiters.remove(pos);
                    w.view_closed(v, self.result(CloseReason::User), self.exit_code);
                }
                Response::Ok
            }
            Request::Ping => Response::Pong,
            Request::Shutdown => {
                for (v, w) in self.waiters.lock().drain(..) {
                    w.view_closed(v, self.result(CloseReason::Shutdown), self.exit_code);
                }
                Response::Ok
            }
            _ => Response::error(hikasa_session::ipc::IpcErrorCode::InvalidRequest, "?"),
        }
    }
}
