//! `hikasa gui open` launching a GUI process (contract §6.3 `cli_gui.rs`, §2.21, §3.6; D-23, D-47, D-51).
//!
//! A test target without libtest (`harness = false`): `main` acts as the scripted GUI ([`fake_gui`]) when
//! `HIKASA_TEST_FAKE_GUI=1` is set, and otherwise runs the cases below. The cases launch this very binary
//! through `HIKASA_GUI_EXE=<current_exe>` with `HIKASA_TEST_FAKE_GUI=1`, which the CLI passes on to the process
//! it starts, so the fake GUI is never a product binary. The in-process fake GUI tests stay in `cli_gui.rs`.
//!
//! Arguments (as cargo passes them): words are name filters (substring; `--exact` for whole names), `--list`
//! lists the cases, `--skip <word>` leaves cases out, `--ignored` runs nothing (no case is ignored); other
//! libtest options are accepted and ignored.

mod common;
#[path = "cli_gui_launch/fake_gui.rs"]
mod fake_gui;

use std::process::ExitCode;

/// One case: its name and its body (a panic is a failure).
type Case = (&'static str, fn());

#[cfg(unix)]
const CASES: &[Case] = &[
    (
        "control_commands_never_launch",
        cases::control_commands_never_launch,
    ),
    (
        "a_gui_that_exits_while_waiting_is_102",
        cases::a_gui_that_exits_while_waiting_is_102,
    ),
    (
        "launch_through_hikasa_gui_exe",
        cases::launch_through_hikasa_gui_exe,
    ),
    (
        "a_relative_gui_executable_is_resolved_against_the_current_folder",
        cases::a_relative_gui_executable_is_resolved_against_the_current_folder,
    ),
    (
        "launch_without_an_endpoint_override",
        cases::launch_without_an_endpoint_override,
    ),
    (
        "a_launched_gui_that_never_listens_is_102",
        cases::a_launched_gui_that_never_listens_is_102,
    ),
    (
        "an_insecure_socket_folder_is_100_and_launches_nothing",
        cases::an_insecure_socket_folder_is_100_and_launches_nothing,
    ),
];

/// The launch paths need Unix sockets and executable scripts.
#[cfg(not(unix))]
const CASES: &[Case] = &[];

fn main() -> ExitCode {
    if std::env::var_os(fake_gui::ENV).is_some_and(|v| v == "1") {
        return fake_gui::run();
    }
    run_cases(CASES)
}

/// A minimal stand-in for libtest: selects the cases per the arguments, runs them one after the other (each
/// in its own sandbox) and reports in libtest's format.
fn run_cases(cases: &[Case]) -> ExitCode {
    let mut filters: Vec<String> = Vec::new();
    let mut skips: Vec<String> = Vec::new();
    let (mut list, mut exact, mut ignored_only) = (false, false, false);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--list" => list = true,
            "--exact" => exact = true,
            "--ignored" => ignored_only = true,
            "--skip" => skips.extend(args.next()),
            // libtest options with a separate value.
            "--test-threads" | "--format" | "--color" | "--logfile" | "-Z" => {
                args.next();
            }
            other if other.starts_with('-') => {}
            other => filters.push(other.to_owned()),
        }
    }
    let matches = |name: &str, word: &String| {
        if exact {
            name == word
        } else {
            name.contains(word.as_str())
        }
    };
    let selected: Vec<&Case> = cases
        .iter()
        .filter(|_| !ignored_only)
        .filter(|(name, _)| filters.is_empty() || filters.iter().any(|f| matches(name, f)))
        .filter(|(name, _)| !skips.iter().any(|s| matches(name, s)))
        .collect();
    if list {
        for (name, _) in &selected {
            println!("{name}: test");
        }
        return ExitCode::SUCCESS;
    }
    println!("\nrunning {} tests", selected.len());
    let started = std::time::Instant::now();
    let mut failed: Vec<&str> = Vec::new();
    for (name, body) in &selected {
        let outcome = std::panic::catch_unwind(body);
        println!(
            "test {name} ... {}",
            if outcome.is_ok() { "ok" } else { "FAILED" }
        );
        if outcome.is_err() {
            failed.push(name);
        }
    }
    if !failed.is_empty() {
        println!("\nfailures:");
        for name in &failed {
            println!("    {name}");
        }
    }
    println!(
        "\ntest result: {}. {} passed; {} failed; 0 ignored; 0 measured; {} filtered out; finished in {:.2}s\n",
        if failed.is_empty() { "ok" } else { "FAILED" },
        selected.len() - failed.len(),
        failed.len(),
        cases.len() - selected.len(),
        started.elapsed().as_secs_f64()
    );
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(unix)]
mod cases {
    use std::path::PathBuf;

    use crate::common::{self, Sandbox, code, stderr};
    use crate::fake_gui;

    /// Two text files and two folders.
    fn sandbox() -> Sandbox {
        let s = Sandbox::new();
        s.file("a.c", "int a;\n");
        s.file("b.c", "int b;\n");
        s.file("l/x.txt", "x\n");
        s.file("r/x.txt", "x\n");
        s
    }

    /// This test binary, which acts as the fake GUI when launched with `HIKASA_TEST_FAKE_GUI=1`.
    fn fake_gui_exe() -> PathBuf {
        std::env::current_exe().expect("the test binary")
    }

    /// `cmd` launches the fake GUI.
    fn with_fake_gui(cmd: &mut assert_cmd::Command) -> &mut assert_cmd::Command {
        cmd.env("HIKASA_GUI_EXE", fake_gui_exe())
            .env(fake_gui::ENV, "1")
    }

    pub fn control_commands_never_launch() {
        let s = sandbox();
        // A launch would leave a record.
        let out = with_fake_gui(&mut s.cmd())
            .args(["gui", "ping"])
            .env("HIKASA_FAKE_GUI_RECORD", s.join("record.txt"))
            .output()
            .unwrap();
        assert_eq!(code(&out), 100, "D-22");
        assert!(!s.join("record.txt").exists());
    }

    pub fn a_gui_that_exits_while_waiting_is_102() {
        let s = sandbox();
        let out = with_fake_gui(&mut s.cmd())
            .args(["gui", "open", "a.c", "b.c", "--wait"])
            .env("HIKASA_FAKE_GUI_MODE", "exit")
            .output()
            .unwrap();
        assert_eq!(code(&out), 102, "connection lost: {}", stderr(&out));
    }

    pub fn launch_through_hikasa_gui_exe() {
        let s = sandbox();
        let record = s.join("record.txt");
        let out = with_fake_gui(&mut s.cmd())
            .args(["gui", "open", "a.c", "b.c", "--wait"])
            .env("HIKASA_FAKE_GUI_CODE", "11")
            .env("HIKASA_FAKE_GUI_RECORD", &record)
            .output()
            .unwrap();
        assert_eq!(code(&out), 11, "{}", stderr(&out));
        let log = std::fs::read_to_string(&record).unwrap();
        // An explicit configuration is passed to the GUI, which listens where the CLI waits (§1.8.7 item 18).
        let config = s.config().display().to_string();
        assert!(
            log.contains(&format!("arg: --config-dir\narg: {config}\n")),
            "{log}"
        );
        assert!(
            log.contains(&format!("endpoint: {}\n", s.socket().display())),
            "{log}"
        );
        assert!(log.contains("request: open_view"), "{log}");
        // Without --wait the CLI returns 0 as soon as the view is open.
        let other = Sandbox::new();
        other.file("a.c", "x\n");
        other.file("b.c", "y\n");
        let out = with_fake_gui(&mut other.cmd())
            .args(["gui", "open", "a.c", "b.c"])
            .output()
            .unwrap();
        assert_eq!(code(&out), 0, "{}", stderr(&out));
    }

    /// Regression (wave-5 review): a relative `HIKASA_GUI_EXE` is resolved against the current folder, the way
    /// it is checked (it used to be searched on `PATH` at launch and fail with exit 100).
    pub fn a_relative_gui_executable_is_resolved_against_the_current_folder() {
        let s = sandbox();
        std::os::unix::fs::symlink(fake_gui_exe(), s.join("fakegui")).unwrap();
        let out = s
            .cmd()
            .args(["gui", "open", "a.c", "b.c", "--wait"])
            .env("HIKASA_GUI_EXE", "fakegui")
            .env(fake_gui::ENV, "1")
            .env("HIKASA_FAKE_GUI_CODE", "12")
            .output()
            .unwrap();
        assert_eq!(code(&out), 12, "{}", stderr(&out));
    }

    pub fn launch_without_an_endpoint_override() {
        let s = sandbox();
        let runtime = s.dir("xdg");
        common::set_mode(&runtime, 0o700);
        let config = s.join("portable-config");
        let record = s.join("record.txt");
        let out = assert_cmd::Command::new(common::hikasa_exe())
            .current_dir(s.path())
            .env_remove("HIKASA_IPC_ENDPOINT")
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("HIKASA_GUI_EXE", fake_gui_exe())
            .env(fake_gui::ENV, "1")
            .env("HIKASA_FAKE_GUI_CODE", "2")
            .env("HIKASA_FAKE_GUI_RECORD", &record)
            .env("LC_ALL", "C.UTF-8")
            .args(["gui", "open", "a.c", "b.c", "--wait", "--config-dir"])
            .arg(&config)
            .output()
            .unwrap();
        assert_eq!(
            code(&out),
            2,
            "the launched GUI was reached: {}",
            stderr(&out)
        );
        let log = std::fs::read_to_string(&record).unwrap();
        assert!(
            log.contains(&format!("arg: --config-dir\narg: {}\n", config.display())),
            "--config-dir reached the GUI: {log}"
        );
        let endpoint = format!("endpoint: {}/hikasa-", runtime.display());
        assert!(log.contains(&endpoint), "{log}");
    }

    pub fn a_launched_gui_that_never_listens_is_102() {
        // Both cases wait out the 10 s retry budget, so they run side by side (separate endpoints).
        let cases: Vec<(bool, Sandbox, std::process::Child)> = [false, true]
            .into_iter()
            .map(|wait| {
                let s = sandbox();
                let mut cmd = std::process::Command::new(common::hikasa_exe());
                cmd.current_dir(s.path())
                    .env("HIKASA_IPC_ENDPOINT", s.socket())
                    .env("HIKASA_GUI_EXE", fake_gui_exe())
                    .env(fake_gui::ENV, "1")
                    .env("HIKASA_FAKE_GUI_MODE", "deaf")
                    .arg("--config-dir")
                    .arg(s.config())
                    .args(["gui", "open", "a.c", "b.c"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                if wait {
                    cmd.arg("--wait");
                }
                let child = cmd.spawn().unwrap();
                (wait, s, child)
            })
            .collect();
        for (wait, _sandbox, mut child) in cases {
            let status = child.wait().unwrap();
            assert_eq!(status.code(), Some(102), "retry budget, wait = {wait}");
        }
    }

    pub fn an_insecure_socket_folder_is_100_and_launches_nothing() {
        let s = sandbox();
        let open = s.dir("open");
        common::set_mode(&open, 0o777);
        let record = s.join("record.txt");
        let out = with_fake_gui(&mut s.cmd())
            .args(["gui", "open", "a.c", "b.c", "--wait"])
            .env("HIKASA_IPC_ENDPOINT", open.join("s.sock"))
            .env("HIKASA_FAKE_GUI_RECORD", &record)
            .output()
            .unwrap();
        assert_eq!(code(&out), 100, "{}", stderr(&out));
        assert!(!record.exists(), "nothing was launched");
        let ping = s
            .cmd()
            .args(["gui", "ping"])
            .env("HIKASA_IPC_ENDPOINT", open.join("s.sock"))
            .output()
            .unwrap();
        assert_eq!(code(&ping), 100);
    }
}
