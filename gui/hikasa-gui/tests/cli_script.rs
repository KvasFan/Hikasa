//! A8.18 (§1.8.13, D-228) through the real `hikasa-gui`, without a display: `hikasa-gui @s.txt --silent` runs the script
//! headless in the process and exits with its code; with a GUI running (a recorder in the test process holds the private
//! endpoint) `hikasa-gui @s.txt a b` is checked here and forwarded as `RunScript` without waiting. On every OS (a named
//! pipe on Windows); the paths are compared as the platform resolves them (`/private/var/…` on macOS).
//!
//! Ctrl-C of `hikasa-gui @s.txt --silent` (D-159, review F8) is Unix only, as in `cli/hikasa-cli/tests/cli_interrupt.rs`:
//! a console control event on Windows reaches every process of the console, the test runner too. There the handler's
//! state is covered by the unit test of `hikasa_session::interrupt` on every OS.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;

const GUI: &str = env!("CARGO_BIN_EXE_hikasa-gui");

/// A private world: files, a configuration folder and an endpoint.
struct World {
    dir: tempfile::TempDir,
}

impl World {
    fn new() -> World {
        let dir = tempfile::tempdir().expect("temp dir");
        let ipc = dir.path().join("ipc");
        std::fs::create_dir(&ipc).expect("mkdir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ipc, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        }
        World { dir }
    }

    fn path(&self, rel: &str) -> PathBuf {
        let mut path = self.dir.path().to_path_buf();
        path.extend(rel.split('/'));
        path
    }

    fn file(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, text).expect("write");
        path
    }

    /// `HIKASA_IPC_ENDPOINT`: a socket in the world on Unix, a pipe name of this world on Windows.
    fn endpoint_override(&self) -> std::ffi::OsString {
        if cfg!(unix) {
            self.path("ipc/s.sock").into_os_string()
        } else {
            let name = self
                .dir
                .path()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("hikasa-gui-script-test-{}-{name}", std::process::id()).into()
        }
    }

    /// The endpoint `hikasa-gui` computes from [`World::endpoint_override`].
    fn endpoint(&self) -> hikasa_session::ipc::Endpoint {
        let o = self.endpoint_override();
        if cfg!(unix) {
            let mut lock = o.clone();
            lock.push(".lock");
            hikasa_session::ipc::Endpoint {
                address: hikasa_session::ipc::EndpointAddress::UnixSocket(PathBuf::from(o)),
                lock_path: PathBuf::from(lock),
                own_dir: None,
                uid: hikasa_session::capture_env().uid,
            }
        } else {
            let name = o.to_string_lossy().into_owned();
            hikasa_session::ipc::Endpoint {
                lock_path: std::env::temp_dir().join(format!("{name}.lock")),
                address: hikasa_session::ipc::EndpointAddress::NamedPipe(name),
                own_dir: None,
                uid: None,
            }
        }
    }

    fn config(&self) -> PathBuf {
        self.path("config")
    }

    /// `hikasa-gui --config-dir <cfg> <args>` in `work/`, with the private endpoint and no display at all.
    fn gui(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(GUI);
        cmd.arg("--config-dir")
            .arg(self.config())
            .args(args)
            .current_dir(self.path("work"))
            .env("HIKASA_IPC_ENDPOINT", self.endpoint_override())
            .env("HIKASA_CLIPBOARD_FILE", self.path("no-clipboard"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        cmd
    }
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `path` resolved (`/private/var/…` on macOS), to compare the child's spelling with the test's.
fn canonical(path: impl AsRef<Path>) -> PathBuf {
    std::fs::canonicalize(path.as_ref())
        .unwrap_or_else(|e| panic!("{}: {e}", path.as_ref().display()))
}

/// A GUI stand-in in the test process for `hikasa-gui @script` (A8.18): it holds the private endpoint and records each
/// `RunScript`.
struct ScriptRecorder {
    seen: std::sync::Mutex<Vec<hikasa_session::ipc::Request>>,
}

impl hikasa_session::ipc::IpcHandler for ScriptRecorder {
    fn server_info(&self) -> hikasa_session::ipc::ServerInfo {
        hikasa_session::ipc::ServerInfo {
            server: "recorder".into(),
            version: 1,
        }
    }

    fn handle(
        &self,
        _request: hikasa_session::ipc::Request,
        _waiter: Option<hikasa_session::ipc::ViewWaiter>,
    ) -> hikasa_session::ipc::Response {
        hikasa_session::ipc::Response::Pong
    }

    fn handle_script(
        &self,
        request: hikasa_session::ipc::Request,
        _waiter: Option<hikasa_session::ipc::ScriptWaiter>,
    ) -> hikasa_session::ipc::Response {
        self.seen.lock().expect("lock").push(request);
        hikasa_session::ipc::Response::Ok
    }
}

/// A recorder serving the world's endpoint.
fn recorder(world: &World) -> (Arc<ScriptRecorder>, hikasa_session::ipc::ServerHandle) {
    let gui = Arc::new(ScriptRecorder {
        seen: std::sync::Mutex::new(Vec::new()),
    });
    let handler: Arc<dyn hikasa_session::ipc::IpcHandler> = gui.clone();
    let server = hikasa_session::ipc::IpcServer::bind(&world.endpoint())
        .expect("bind")
        .spawn(handler)
        .expect("spawn");
    (gui, server)
}

/// A8.18 (D-228): `hikasa-gui @s.txt --silent` never goes through a GUI and opens no window (no display at all): the
/// script runs headless in the process, its stream on stderr, and the process exits with the script's code; with a
/// GUI running, `hikasa-gui @s.txt a b` is checked here and forwarded as `RunScript` (its cwd, the referenced
/// variables) without waiting, exit 0. No display is needed for either.
#[test]
fn a8_18_hikasa_gui_at_script_runs_headless_or_is_forwarded() {
    let world = World::new();
    world.file("work/s.txt", "beep\nload %1 %2\n");
    world.file("work/l/x.txt", "x\n");
    world.file("work/r/x.txt", "x\n");
    let (gui, server) = recorder(&world);
    let silent = |args: &[&str]| {
        world
            .gui(args)
            .stdin(Stdio::null())
            .output()
            .expect("run hikasa-gui")
    };
    let out = silent(&["@s.txt", "nowhere", "r", "--silent"]);
    assert_eq!(code(&out), 107, "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("run L2 load nowhere r"),
        "{}",
        text(&out.stderr)
    );
    let out = silent(&["@s.txt", "l", "r", "--silent"]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert!(
        gui.seen.lock().expect("lock").is_empty(),
        "no RunScript: never through a GUI"
    );
    // A script that does not check: 106 here, nothing sent.
    world.file("work/bad.txt", "selct all\n");
    let out = silent(&["@bad.txt"]);
    assert_eq!(code(&out), 106, "{}", text(&out.stderr));
    // A GUI runs (the recorder holds the endpoint): forwarded, exit 0.
    let out = silent(&["@s.txt", "l", "r", "--closescript"]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let seen = gui.seen.lock().expect("lock").clone();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let hikasa_session::ipc::Request::RunScript {
        path,
        args,
        cwd,
        wait,
        close,
        ..
    } = seen[0].clone()
    else {
        panic!("{seen:?}")
    };
    // The child's spelling of the world (on macOS `/private/var/…` for the `/var/…` the test created).
    assert!(Path::new(&path).is_absolute(), "{path}");
    assert_eq!(canonical(&path), canonical(world.path("work/s.txt")));
    assert_eq!(args, ["l", "r"]);
    assert_eq!(
        cwd.as_deref().map(canonical),
        Some(canonical(world.path("work"))),
        "{cwd:?}"
    );
    assert!(!wait, "a second instance never waits");
    assert_eq!(close, Some(true));
    server.shutdown().expect("shutdown");
}

/// D-159 for `hikasa-gui @s.txt --silent` (review F8), as `hikasa script run --silent`: Ctrl-C during the script's copy
/// stops that copy as `folder op` stops (what was done stays whole, no temporary file), the later lines do not run, exit
/// 100, and the log ends with `Script stopped at line <n>`. The source is a hole, so it costs no disk; nothing is timed:
/// the signal goes once the copy's temporary file is seen.
#[cfg(unix)]
mod ctrl_c {
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use super::World;

    /// What a loaded machine may take to end after a cancel.
    const LIMIT: Duration = Duration::from_secs(60);

    /// The script: a copy of `a` and of a hole, then a beep that must not run.
    fn world() -> World {
        let world = World::new();
        world.file("work/l/a", "first\n");
        let hole = std::fs::File::create(world.path("work/l/b")).expect("create");
        hole.set_len(4 << 30).expect("a hole");
        drop(hole);
        std::fs::create_dir_all(world.path("work/r")).expect("mkdir");
        world.file(
            "work/s.txt",
            "log normal script.log\noption confirm:yes-to-all\nload l r\nselect all\ncopy left->right\nbeep\n",
        );
        world
    }

    fn names(world: &World) -> Vec<String> {
        std::fs::read_dir(world.path("work/r"))
            .expect("list")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect()
    }

    /// Waits until the copy of the hole has begun; the child must still run.
    fn wait_for_the_copy(world: &World, child: &mut Child) {
        let began = Instant::now();
        while !names(world).iter().any(|n| n.contains("b.hikasa-tmp")) {
            let ended = child.try_wait().expect("wait");
            assert!(
                ended.is_none(),
                "ended before the copy of the hole: {ended:?}"
            );
            if began.elapsed() > LIMIT {
                child.kill().expect("kill the child");
                child.wait().expect("wait");
                panic!("the copy of the hole never began");
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The child's exit code, within [`LIMIT`].
    fn wait(child: &mut Child) -> Option<i32> {
        let began = Instant::now();
        loop {
            if let Some(status) = child.try_wait().expect("wait") {
                return status.code();
            }
            if began.elapsed() > LIMIT {
                child.kill().expect("kill the child");
                child.wait().expect("wait");
                panic!("the process did not end within {LIMIT:?}");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// What Ctrl-C leaves: exit 100, `a` whole, no partial `b`, the line after the copy not run.
    fn stopped(world: &World, code: Option<i32>) {
        assert_eq!(code, Some(100));
        assert_eq!(names(world), ["a"], "nothing half-written");
        let log = std::fs::read_to_string(world.path("work/script.log")).expect("the log");
        assert!(
            log.contains("info - Script stopped at line 5 (exit 100)"),
            "{log}"
        );
        assert!(!log.contains("L6"), "the line after it did not run: {log}");
    }

    /// SIGINT aimed at the child by its process id.
    #[test]
    fn a8_14_ctrl_c_stops_a_silent_hikasa_gui_script_at_that_line() {
        let world = world();
        let mut child = world
            .gui(&["@s.txt", "--silent"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn hikasa-gui");
        wait_for_the_copy(&world, &mut child);
        let sent = Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .expect("run kill");
        assert!(sent.success(), "the child is gone");
        let code = wait(&mut child);
        stopped(&world, code);
    }

    /// Ctrl-C typed on a terminal: `script` gives the child a pty as its controlling terminal, so `\x03` is the press
    /// of a key. `exec` makes `hikasa-gui` the process of `script`'s line, so a shell that forks for the last command
    /// (dash, `/bin/sh` on Debian and Ubuntu) does not stay in the foreground group and die of the same Ctrl-C. Linux
    /// only: the flags of `script` differ elsewhere.
    #[cfg(target_os = "linux")]
    #[test]
    fn a8_14_ctrl_c_on_a_terminal_stops_a_silent_hikasa_gui_script() {
        use std::io::Write as _;
        let world = world();
        let quoted = |word: &str| format!("'{}'", word.replace('\'', r"'\''"));
        let line = [
            "exec".to_owned(),
            quoted(super::GUI),
            "--config-dir".to_owned(),
            quoted(world.config().to_str().expect("utf-8")),
            quoted("@s.txt"),
            "--silent".to_owned(),
        ]
        .join(" ");
        let mut child = Command::new("script")
            .args(["--quiet", "--return", "--command", &line, "/dev/null"])
            .current_dir(world.path("work"))
            .env("SHELL", "/bin/sh")
            .env("HIKASA_IPC_ENDPOINT", world.endpoint_override())
            .env("HIKASA_CLIPBOARD_FILE", world.path("no-clipboard"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn script: util-linux's `script` is needed here");
        wait_for_the_copy(&world, &mut child);
        let stdin = child.stdin.as_mut().expect("piped");
        stdin.write_all(b"\x03").expect("the terminal");
        stdin.flush().expect("the terminal");
        let code = wait(&mut child);
        stopped(&world, code);
    }
}
