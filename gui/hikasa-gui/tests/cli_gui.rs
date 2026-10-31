//! The real `hikasa` CLI against the real `hikasa-gui` (acceptance (c) of contract §6.5 wave 6; §3.6, D-23,
//! ARCH-090): `gui open … --wait` launches the GUI and returns the view's exit code, `gui ping/list/close/quit`
//! work, and a second `hikasa-gui a b` forwards its request to the running instance and exits 0.
//!
//! The tests open a window, so they need a display and are ignored by default. Run them on a virtual X display
//! with
//!
//! ```text
//! cargo build -p hikasa-cli && env -u WAYLAND_DISPLAY xvfb-run -a cargo test -p hikasa-gui --test cli_gui -- --ignored
//! ```
//!
//! `xvfb-run` only sets `DISPLAY`: in a Wayland session the inherited `WAYLAND_DISPLAY` would make winit open
//! the window on the real screen, so it is removed (`env -u WAYLAND_DISPLAY`, docs/17 §3.5). The tests also
//! remove it themselves from every process they start whenever `DISPLAY` is set ([`use_x11`]); without
//! `DISPLAY` they run on the Wayland display the caller names.
//!
//! Every process gets a private configuration folder (`--config-dir`) and a private endpoint
//! (`HIKASA_IPC_ENDPOINT`) inside the test's temporary folder; the real configuration and endpoint are never
//! touched. The CLI binary is `$HIKASA_CLI_EXE` or the `hikasa` next to the GUI binary.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const GUI: &str = env!("CARGO_BIN_EXE_hikasa-gui");

fn cli_exe() -> PathBuf {
    if let Some(exe) = std::env::var_os("HIKASA_CLI_EXE") {
        return PathBuf::from(exe);
    }
    Path::new(GUI).with_file_name("hikasa")
}

/// A private world: files, a configuration folder and an endpoint.
struct World {
    dir: tempfile::TempDir,
}

impl World {
    fn new() -> World {
        let dir = tempfile::tempdir().expect("temp dir");
        let ipc = dir.path().join("ipc");
        std::fs::create_dir(&ipc).expect("mkdir");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ipc, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        World { dir }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn file(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, text).expect("write");
        path
    }

    fn endpoint(&self) -> PathBuf {
        self.path("ipc/s.sock")
    }

    fn config(&self) -> PathBuf {
        self.path("config")
    }

    /// `hikasa --config-dir <cfg> <args>` with the private endpoint and this GUI binary.
    fn cli(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(cli_exe());
        cmd.arg("--config-dir")
            .arg(self.config())
            .args(args)
            .env("HIKASA_IPC_ENDPOINT", self.endpoint())
            .env("HIKASA_GUI_EXE", GUI)
            .stdin(Stdio::null());
        // The CLI passes its environment to the GUI it launches.
        use_x11(&mut cmd);
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cli(args).output().expect("run hikasa")
    }

    /// `hikasa-gui --config-dir <cfg> <args>` with the private endpoint.
    fn gui(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(GUI);
        cmd.arg("--config-dir")
            .arg(self.config())
            .args(args)
            .env("HIKASA_IPC_ENDPOINT", self.endpoint())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        use_x11(&mut cmd);
        cmd
    }

    fn views(&self) -> Vec<Value> {
        let out = self.run(&["gui", "list", "--json"]);
        assert_eq!(code(&out), 0, "gui list: {}", text(&out.stderr));
        let doc: Value = serde_json::from_slice(&out.stdout).expect("json");
        doc["result"]["views"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn wait_for_views(&self, n: usize) -> Vec<Value> {
        let began = Instant::now();
        loop {
            let views = self.views();
            if views.len() >= n {
                return views;
            }
            assert!(
                began.elapsed() < Duration::from_secs(30),
                "{n} views did not appear"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn wait_running(&self, running: bool) {
        let began = Instant::now();
        loop {
            let out = self.run(&["gui", "ping"]);
            if (code(&out) == 0) == running {
                return;
            }
            assert!(
                began.elapsed() < Duration::from_secs(30),
                "the GUI did not {}",
                if running { "start" } else { "stop" }
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Quits the GUI when a test ends (also on failure), so no window stays open.
struct QuitOnDrop<'a>(&'a World);

impl Drop for QuitOnDrop<'_> {
    fn drop(&mut self) {
        let _ = self.0.run(&["gui", "quit"]);
    }
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn wait_child(child: &mut Child, limit: Duration) -> i32 {
    let began = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("wait") {
            return status.code().unwrap_or(-1);
        }
        assert!(began.elapsed() < limit, "the process did not end");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// With `DISPLAY` set (`xvfb-run` sets it), the GUI must use that X display: winit prefers Wayland whenever
/// `WAYLAND_DISPLAY` is set, which would put the window on the caller's real screen instead of the virtual one.
fn use_x11(cmd: &mut Command) -> &mut Command {
    if std::env::var_os("DISPLAY").is_some() {
        cmd.env_remove("WAYLAND_DISPLAY");
    }
    cmd
}

fn has_display() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn ready() -> bool {
    if !has_display() {
        eprintln!("skipped: no display");
        return false;
    }
    assert!(
        cli_exe().is_file(),
        "{} is missing: build it with `cargo build -p hikasa-cli` (or set HIKASA_CLI_EXE)",
        cli_exe().display()
    );
    true
}

#[test]
#[ignore = "opens a window: needs a display and the hikasa binary (cargo build -p hikasa-cli)"]
fn gui_open_wait_control_commands_and_second_instance() {
    if !ready() {
        return;
    }
    let w = World::new();
    let a = w.file("a.txt", "one\ntwo\nthree\n");
    let b = w.file("b.txt", "one\nTWO\nthree\n");
    let c = w.file("c.txt", "one\ntwo\nthree\nfour\n");
    w.file("l/x.txt", "x\n");
    w.file("l/only-left.txt", "l\n");
    w.file("r/x.txt", "x\n");
    let (l, r) = (w.path("l"), w.path("r"));
    let s = |p: &Path| p.display().to_string();

    // The codes the views must report are the CLI's own verdicts (D-23).
    let text_code = code(&w.run(&["text", "compare", &s(&a), &s(&b)]));
    let folder_code = code(&w.run(&["folder", "compare", &s(&l), &s(&r)]));
    assert_eq!(text_code, 13, "important differences");
    assert_eq!(folder_code, 13, "an orphan is a difference");

    // No GUI yet.
    assert_eq!(
        code(&w.run(&["gui", "ping"])),
        100,
        "D-22: ping never launches"
    );

    // `gui open --wait` launches the GUI (HIKASA_GUI_EXE, the private endpoint and --config-dir travel with it).
    let mut waiting = w
        .cli(&["gui", "open", &s(&a), &s(&b), "--wait"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("gui open");
    w.wait_running(true);
    let _quit = QuitOnDrop(&w);
    let ping = w.run(&["gui", "ping"]);
    assert_eq!(code(&ping), 0);
    assert!(
        text(&ping.stdout).starts_with("hikasa-gui "),
        "{}",
        text(&ping.stdout)
    );
    // The first-run format files (D-21) are written after the window is up, not before the endpoint.
    let order = w.config().join("formats").join("order.toml");
    let began = Instant::now();
    while !order.is_file() {
        assert!(
            began.elapsed() < Duration::from_secs(30),
            "the GUI did not write the format files"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let views = w.wait_for_views(1);
    assert_eq!(views[0]["kind"], "text_compare");
    assert_eq!(views[0]["specs"][0], s(&a));
    let view = views[0]["view"].as_u64().expect("view id");
    let human = w.run(&["gui", "list"]);
    assert!(
        text(&human.stdout).contains("a.txt"),
        "{}",
        text(&human.stdout)
    );

    // Let the small comparison finish, then close the view: the waiting CLI exits with the view's code.
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(code(&w.run(&["gui", "close", &view.to_string()])), 0);
    assert_eq!(wait_child(&mut waiting, Duration::from_secs(30)), text_code);
    assert_eq!(
        code(&w.run(&["gui", "close", "9999"])),
        100,
        "unknown view: not_found"
    );

    // A second `hikasa-gui a c` forwards to the running instance and exits 0.
    let mut second = w.gui(&[&s(&a), &s(&c)]).spawn().expect("second hikasa-gui");
    let status = wait_child(&mut second, Duration::from_secs(30));
    assert_eq!(status, 0, "forwarded");
    let views = w.wait_for_views(1);
    assert!(
        views.iter().any(|v| v["specs"][1] == s(&c)),
        "the forwarded view is open: {views:?}"
    );
    // A second instance without specs only checks that the first one answers.
    let mut third = w.gui(&[]).spawn().expect("third hikasa-gui");
    assert_eq!(wait_child(&mut third, Duration::from_secs(30)), 0);

    // A waited folder view still open when the GUI quits reports its real verdict (reason `shutdown`).
    let mut waiting = w
        .cli(&["gui", "open", &s(&l), &s(&r), "--wait", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("gui open folders");
    let views = w.wait_for_views(2);
    assert!(views.iter().any(|v| v["kind"] == "folder_compare"));
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(code(&w.run(&["gui", "quit"])), 0);
    let status = wait_child(&mut waiting, Duration::from_secs(30));
    let out = waiting.wait_with_output().expect("output");
    assert_eq!(status, folder_code, "{}", text(&out.stderr));
    let doc: Value = serde_json::from_slice(&out.stdout).expect("gui open --json");
    assert_eq!(doc["exit_code"], folder_code);
    assert_eq!(doc["result"]["result"]["reason"], "shutdown");
    assert_eq!(doc["result"]["result"]["folder_verdict"], "differences");
    w.wait_running(false);
    assert_eq!(code(&w.run(&["gui", "ping"])), 100);
}

/// A4.15 (D-127): `hikasa-gui <session>` and `hikasa-gui <workspace>` open them; with a GUI running a second
/// process forwards them and exits 0.
#[test]
#[ignore = "opens a window: needs a display and the hikasa binary (cargo build -p hikasa-cli)"]
fn the_gui_command_line_opens_sessions_and_workspaces() {
    if !ready() {
        return;
    }
    let world = World::new();
    let left = world.file("a.txt", "one\n");
    let right = world.file("b.txt", "two\n");
    let (l, r) = (left.display().to_string(), right.display().to_string());
    let out = world.run(&[
        "session",
        "new",
        "work/api",
        "--type",
        "text-compare",
        "--left",
        &l,
        "--right",
        &r,
    ]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    world.file(
        "config/workspaces/w.toml",
        "schema_version = 1\n[[window]]\n[[window.tab]]\nsession = \"work/api\"\n",
    );
    let mut gui = world.gui(&["work/api"]).spawn().expect("spawn");
    world.wait_running(true);
    let views = world.wait_for_views(1);
    assert_eq!(views[0]["title"], "work/api");
    // A second process forwards the workspace and exits 0.
    let mut second = world.gui(&["w"]).spawn().expect("spawn");
    assert_eq!(wait_child(&mut second, Duration::from_secs(30)), 0);
    assert_eq!(code(&world.run(&["gui", "quit"])), 0);
    assert_eq!(wait_child(&mut gui, Duration::from_secs(30)), 0);
}

/// A4.21 (D-127): `hikasa gui open x.hikasa-pkg` exits 0 with a running GUI, a second `hikasa-gui x.hikasa-pkg`
/// forwards it, `hikasa-gui x.hikasa-pkg` starts with the wizard, and `--silent` imports with no window.
#[test]
#[ignore = "opens a window: needs a display and the hikasa binary (cargo build -p hikasa-cli)"]
fn settings_packages_reach_the_import_wizard() {
    if !ready() {
        return;
    }
    let world = World::new();
    let pkg = world.path("all.hikasa-pkg");
    let pkg_text = pkg.display().to_string();
    let out = world.run(&["settings", "export", "-o", &pkg_text]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    // The GUI starts on the package, with the wizard, and keeps running.
    let mut gui = world.gui(&[&pkg_text]).spawn().expect("spawn");
    world.wait_running(true);
    let out = world.run(&["gui", "open", &pkg_text, "--no-launch"]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let mut second = world.gui(&[&pkg_text]).spawn().expect("spawn");
    assert_eq!(wait_child(&mut second, Duration::from_secs(30)), 0);
    assert_eq!(code(&world.run(&["gui", "quit"])), 0);
    assert_eq!(wait_child(&mut gui, Duration::from_secs(30)), 0);
    // `--silent`: no window, no IPC.
    let mut silent = world.gui(&[&pkg_text, "--silent"]).spawn().expect("spawn");
    assert_eq!(wait_child(&mut silent, Duration::from_secs(30)), 0);
}
