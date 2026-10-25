//! Shared helpers of the CLI integration tests (contract §6.1, §6.3).
//!
//! Every test runs `hikasa` in its own temporary folder with its own `--config-dir` (`<tmp>/cfg`), its own IPC
//! endpoint (`HIKASA_IPC_ENDPOINT=<tmp>/s.sock`; the folder is private, mode 0700) and a `HIKASA_GUI_EXE` that
//! does not exist, so no test touches the real configuration, a real GUI or the user's endpoint.
//!
//! Goldens live in `tests/goldens/<suite>/<case>`; `HIKASA_BLESS=1` rewrites them. CLI goldens replace the
//! temporary folder with `<TMP>` and the version with `<VERSION>`, and drop the report's `Generated:` line
//! (§6.1); JSON goldens of file metadata also mask the values that depend on the machine (`<UID>`, `<GID>`,
//! `<CTIME>`).

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use hikasa_core::{ContentKind, ExitCode};
use hikasa_session::ipc::{
    CloseReason, Endpoint, EndpointAddress, IpcErrorCode, IpcHandler, IpcServer, Request, Response,
    ServerHandle, ServerInfo, ViewId, ViewInfo, ViewResult, ViewWaiter,
};
use parking_lot_free::Mutex;

/// `std::sync::Mutex` with a poison-free `lock` (the tests have no parking_lot dependency).
mod parking_lot_free {
    pub struct Mutex<T>(std::sync::Mutex<T>);

    impl<T> Mutex<T> {
        pub fn new(value: T) -> Mutex<T> {
            Mutex(std::sync::Mutex::new(value))
        }

        pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
    }
}

/// A fixed modification time for fixtures: 2026-10-19T13:12:00Z.
pub const MTIME: i64 = 1_792_415_520;

/// The path of the `hikasa` binary.
pub fn hikasa_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hikasa"))
}

/// One test's private world.
pub struct Sandbox {
    tmp: tempfile::TempDir,
}

impl Default for Sandbox {
    fn default() -> Sandbox {
        Sandbox::new()
    }
}

impl Sandbox {
    /// A new private folder (mode 0700).
    pub fn new() -> Sandbox {
        Sandbox {
            tmp: tempfile::tempdir().expect("temporary folder"),
        }
    }

    /// The folder.
    pub fn path(&self) -> &Path {
        self.tmp.path()
    }

    /// `<tmp>/<rel>`.
    pub fn join(&self, rel: &str) -> PathBuf {
        self.tmp.path().join(rel)
    }

    /// The configuration folder of the test (`<tmp>/cfg`; it need not exist).
    pub fn config(&self) -> PathBuf {
        self.join("cfg")
    }

    /// The IPC endpoint of the test (`<tmp>/s.sock`).
    pub fn socket(&self) -> PathBuf {
        self.join("s.sock")
    }

    /// `hikasa --config-dir <tmp>/cfg` in `<tmp>`, with the private endpoint, a missing GUI executable, a UTF-8
    /// locale and the UTC zone.
    pub fn cmd(&self) -> assert_cmd::Command {
        let mut cmd = assert_cmd::Command::new(hikasa_exe());
        cmd.current_dir(self.path())
            .env("HIKASA_IPC_ENDPOINT", self.socket())
            .env("HIKASA_GUI_EXE", self.join("no-such-gui"))
            .env("LC_ALL", "C.UTF-8")
            .env("TZ", "UTC")
            .env_remove("NO_COLOR")
            .env_remove("HIKASA_TEST_FAKE_GUI")
            .env_remove("HIKASA_FAKE_GUI_MODE")
            .env_remove("HIKASA_FAKE_GUI_CODE")
            .env_remove("HIKASA_FAKE_GUI_RECORD")
            .arg("--config-dir")
            .arg(self.config());
        cmd
    }

    /// Writes a file (creating its folders) with mode 0644 and the fixed [`MTIME`].
    pub fn file(&self, rel: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        self.file_at(rel, bytes, MTIME)
    }

    /// Writes a file (creating its folders) with mode 0644 and modification time `mtime` (Unix seconds).
    pub fn file_at(&self, rel: &str, bytes: impl AsRef<[u8]>, mtime: i64) -> PathBuf {
        let path = self.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("folders");
        }
        std::fs::write(&path, bytes).expect("write");
        set_mode(&path, 0o644);
        filetime::set_file_mtime(&path, filetime::FileTime::from_unix_time(mtime, 0))
            .expect("mtime");
        path
    }

    /// Creates a folder (mode 0755).
    pub fn dir(&self, rel: &str) -> PathBuf {
        let path = self.join(rel);
        std::fs::create_dir_all(&path).expect("folder");
        set_mode(&path, 0o755);
        path
    }

    /// Sets mode 0755 and the fixed [`MTIME`] on every folder below `rel` (after the files were written).
    pub fn settle(&self, rel: &str) {
        fn walk(dir: &Path) {
            for entry in std::fs::read_dir(dir).expect("list").flatten() {
                let path = entry.path();
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    walk(&path);
                }
            }
            set_mode(dir, 0o755);
            filetime::set_file_mtime(dir, filetime::FileTime::from_unix_time(MTIME, 0))
                .expect("mtime");
        }
        walk(&self.join(rel));
    }

    /// Writes `options.toml` into the configuration folder.
    pub fn options(&self, text: &str) {
        let config = self.config();
        std::fs::create_dir_all(&config).expect("config");
        std::fs::write(config.join("options.toml"), text).expect("options");
    }

    /// Normalises output for a golden (§6.1): `<TMP>`, `<VERSION>`, no `Generated:` line (in HTML the
    /// `Generated` row), the column padding after a replaced folder collapsed, and the machine's uid, gid and
    /// status-change times masked.
    pub fn normalise(&self, text: &str) -> String {
        self.normalise_with(text, false)
    }

    /// [`Sandbox::normalise`]; with `truncated`, lines that start with the temporary folder (the column header of a
    /// narrow side-by-side report, cut at a place that depends on the folder's length) are dropped.
    pub fn normalise_with(&self, text: &str, truncated: bool) -> String {
        let mut text = text.to_owned();
        for root in [
            std::fs::canonicalize(self.path()).ok(),
            Some(self.path().to_path_buf()),
        ]
        .into_iter()
        .flatten()
        {
            text = text.replace(&root.display().to_string(), "<TMP>");
        }
        text = text.replace(hikasa_session::HIKASA_VERSION, "<VERSION>");
        let (uid, gid) = ids(self.path());
        let root = self.path().display().to_string();
        let root_start: String = root.chars().take(5).collect();
        let mut out = String::new();
        for line in text.split_inclusive('\n') {
            // The report's generation time (text: `Generated: …`, HTML: the `Generated` header row).
            if line.contains("Generated:") || line.contains("<th>Generated</th>") {
                continue;
            }
            // A narrow column cuts the folder at a length-dependent place: drop such a header line.
            let first = line.trim_start();
            if truncated && (first.starts_with("<TMP") || first.starts_with(&root_start)) {
                continue;
            }
            let trimmed = line.trim_start();
            let indent = &line[..line.len() - trimmed.len()];
            let masked = if trimmed.starts_with("\"ctime\": \"") {
                let comma = if trimmed.trim_end().ends_with(',') {
                    ","
                } else {
                    ""
                };
                format!("{indent}\"ctime\": \"<CTIME>\"{comma}\n")
            } else if let Some(rest) = trimmed.strip_prefix(&format!("\"uid\": {uid}")) {
                format!("{indent}\"uid\": <UID>{rest}")
            } else if let Some(rest) = trimmed.strip_prefix(&format!("\"gid\": {gid}")) {
                format!("{indent}\"gid\": <GID>{rest}")
            } else {
                line.to_owned()
            };
            // Column padding after a replaced folder depends on its length (side-by-side headers).
            let masked = if masked.contains("<TMP") {
                collapse_padding(&masked)
            } else {
                masked
            };
            out.push_str(&masked);
        }
        out
    }
}

/// Collapses every run of two or more spaces after the indentation into two spaces.
fn collapse_padding(line: &str) -> String {
    let body = line.trim_start_matches(' ');
    let mut out = line[..line.len() - body.len()].to_owned();
    let mut spaces = 0;
    for c in body.chars() {
        if c == ' ' {
            spaces += 1;
            continue;
        }
        out.push_str(match spaces {
            0 => "",
            1 => " ",
            _ => "  ",
        });
        spaces = 0;
        out.push(c);
    }
    if spaces > 0 && !body.ends_with('\n') {
        out.push_str(if spaces == 1 { " " } else { "  " });
    }
    out
}

/// The uid and gid owning `path` (Unix; 0 elsewhere).
fn ids(path: &Path) -> (u32, u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path)
            .map(|m| (m.uid(), m.gid()))
            .unwrap_or((0, 0))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        (0, 0)
    }
}

/// Sets a Unix mode (no-op elsewhere).
pub fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}

/// Compares `actual` with `tests/goldens/<name>`; `HIKASA_BLESS=1` rewrites the golden.
pub fn assert_golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(name);
    if std::env::var_os("HIKASA_BLESS").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("golden folder");
        std::fs::write(&path, actual).expect("bless");
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("golden {} ({e}); run with HIKASA_BLESS=1", path.display()));
    pretty_assertions::assert_eq!(expected, actual, "golden {name}");
}

/// stdout of an assert as text.
pub fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// stderr of an assert as text.
pub fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Parses stdout as exactly one JSON document followed by a newline (CLI invariant 2).
pub fn json(output: &std::process::Output) -> serde_json::Value {
    let text = stdout(output);
    assert!(
        text.ends_with("}\n"),
        "one document and a newline: {text:?}"
    );
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("not one JSON document ({e}): {text}"))
}

/// The process exit code (−1 when killed by a signal).
pub fn code(output: &std::process::Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

// ---- an in-process scripted GUI ----------------------------------------------------------------------------

/// What a waited view does in [`FakeGui`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseMode {
    /// `view_closed` before the handler returns.
    Immediately,
    /// Kept until `CloseView` or `Shutdown`.
    OnRequest,
}

/// A scripted GUI server in the test process: opens, lists and closes views, answers pings, records requests.
pub struct FakeGui {
    pub mode: CloseMode,
    pub exit_code: ExitCode,
    next: AtomicU64,
    views: Mutex<Vec<ViewInfo>>,
    waiters: Mutex<Vec<(ViewId, ViewWaiter)>>,
    requests: Mutex<Vec<Request>>,
}

impl FakeGui {
    pub fn new(mode: CloseMode, exit_code: ExitCode) -> Arc<FakeGui> {
        Arc::new(FakeGui {
            mode,
            exit_code,
            next: AtomicU64::new(7),
            views: Mutex::new(Vec::new()),
            waiters: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
        })
    }

    /// The requests received so far.
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().clone()
    }

    fn result(reason: CloseReason) -> ViewResult {
        ViewResult::new(Some(ContentKind::UnimportantOnly), None, false, reason)
    }
}

impl IpcHandler for FakeGui {
    fn server_info(&self) -> ServerInfo {
        ServerInfo {
            server: "fake-gui test".into(),
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
                        CloseMode::Immediately => waiter.view_closed(
                            view,
                            Self::result(CloseReason::User),
                            self.exit_code,
                        ),
                        CloseMode::OnRequest => self.waiters.lock().push((view, waiter)),
                    }
                }
                Response::ViewOpened { view }
            }
            Request::ListViews => Response::Views {
                views: self.views.lock().clone(),
            },
            Request::CloseView { view } => {
                let known = {
                    let mut views = self.views.lock();
                    let before = views.len();
                    views.retain(|v| v.view != view);
                    views.len() != before
                };
                if !known {
                    return Response::error(IpcErrorCode::NotFound, format!("no view {}", view.0));
                }
                let mut waiters = self.waiters.lock();
                if let Some(pos) = waiters.iter().position(|(v, _)| *v == view) {
                    let (v, w) = waiters.remove(pos);
                    w.view_closed(v, Self::result(CloseReason::User), self.exit_code);
                }
                Response::Ok
            }
            Request::Ping => Response::Pong,
            Request::Shutdown => {
                for (v, w) in self.waiters.lock().drain(..) {
                    w.view_closed(v, Self::result(CloseReason::Shutdown), self.exit_code);
                }
                Response::Ok
            }
            _ => Response::error(IpcErrorCode::InvalidRequest, "not scripted"),
        }
    }
}

/// The endpoint `hikasa` computes from `HIKASA_IPC_ENDPOINT=<socket>` (§2.23 row 7).
pub fn endpoint(socket: &Path) -> Endpoint {
    #[cfg(unix)]
    {
        let mut lock = socket.as_os_str().to_os_string();
        lock.push(".lock");
        Endpoint {
            address: EndpointAddress::UnixSocket(socket.to_path_buf()),
            lock_path: PathBuf::from(lock),
            own_dir: None,
            uid: hikasa_session::capture_env().uid,
        }
    }
    #[cfg(not(unix))]
    {
        Endpoint {
            address: EndpointAddress::NamedPipe(socket.display().to_string()),
            lock_path: socket.with_extension("lock"),
            own_dir: None,
            uid: None,
        }
    }
}

/// Starts `gui` on the sandbox's endpoint.
pub fn serve(sandbox: &Sandbox, gui: Arc<FakeGui>) -> ServerHandle {
    IpcServer::bind(&endpoint(&sandbox.socket()))
        .expect("bind")
        .spawn(gui)
        .expect("spawn")
}
