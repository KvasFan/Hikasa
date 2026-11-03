//! A8.18 (§1.8.13, D-228): `hikasa gui script`, `script run --gui` and `gui open @script` against an in-process fake
//! GUI ([`common::FakeGui`] on the sandbox's `HIKASA_IPC_ENDPOINT`). On every OS (a named pipe on Windows): the path and
//! the current folder are compared as the platform resolves them (`/private/var/…` on macOS, `\` on Windows).

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::{CloseMode, FakeGui, Sandbox, assert_golden, code, json, serve, stderr, stdout};
use hikasa_core::ExitCode;
use hikasa_session::ipc::Request;

/// Two text files and two folders.
fn sandbox() -> Sandbox {
    let s = Sandbox::new();
    s.file("a.c", "int a;\n");
    s.file("b.c", "int b;\n");
    s.file("l/x.txt", "x\n");
    s.file("r/x.txt", "x\n");
    s
}

/// `path` resolved (`/private/var/…` on macOS), to compare the child's spelling with the test's.
fn canonical(path: impl AsRef<Path>) -> PathBuf {
    std::fs::canonicalize(path.as_ref())
        .unwrap_or_else(|e| panic!("{}: {e}", path.as_ref().display()))
}

/// The `RunScript` requests a fake GUI received.
fn scripts(gui: &FakeGui) -> Vec<Request> {
    gui.requests()
        .into_iter()
        .filter(|r| matches!(r, Request::RunScript { .. }))
        .collect()
}

/// `hikasa` in the sandbox with the variable the script references.
fn script_cmd(s: &Sandbox) -> assert_cmd::Command {
    let mut cmd = s.cmd();
    cmd.env("HKS_W8B_DIR", "/var/logs");
    cmd
}

/// A script that references one variable (`HKS_W8B_DIR`) and an argument.
fn script_sandbox() -> Sandbox {
    let s = sandbox();
    s.file("s.txt", "log %HKS_W8B_DIR%/run.log\nload %1 r\nbeep\n");
    s
}

#[test]
fn a8_18_gui_script_sends_the_checked_script_with_its_cwd_and_variables() {
    let s = script_sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::LoadFailed);
    let server = serve(&s, Arc::clone(&gui));
    let out = script_cmd(&s)
        .args(["gui", "script", "s.txt", "l", "--wait", "--stop-on-error"])
        .args(["--confirm", "yes-to-all", "--closescript", "--json"])
        .env("HKS_W8B_OTHER", "never sent")
        .output()
        .unwrap();
    assert_eq!(
        code(&out),
        107,
        "the script's code from ScriptFinished: {}",
        stderr(&out)
    );
    assert_golden("gui/script-wait.json", &s.normalise(&stdout(&out)));
    let sent = scripts(&gui);
    let [
        Request::RunScript {
            path,
            args,
            cwd,
            env,
            wait,
            stop_on_error,
            confirm,
            close,
            dry_run,
        },
    ] = sent.as_slice()
    else {
        panic!("{sent:?}")
    };
    // The child's spelling of the folder (on macOS `/private/var/…` for the `/var/…` the test created).
    assert!(Path::new(path).is_absolute(), "{path}");
    assert_eq!(canonical(path), canonical(&s.join("s.txt")), "absolute");
    assert_eq!(args, &["l"]);
    assert_eq!(
        cwd.as_deref().map(canonical),
        Some(canonical(s.path())),
        "{cwd:?}"
    );
    assert!(!dry_run);
    assert_eq!(
        env.iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>(),
        [("HKS_W8B_DIR", "/var/logs")],
        "only the referenced variables"
    );
    assert!(*wait && *stop_on_error);
    assert_eq!(confirm.as_deref(), Some("yes-to-all"));
    assert_eq!(*close, Some(true));
    // Without --wait: 0 once started, no summary; `script run --gui` is the same command.
    let plain = script_cmd(&s)
        .args(["gui", "script", "s.txt", "l", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&plain), 0, "{}", stderr(&plain));
    assert_golden("gui/script.json", &s.normalise(&stdout(&plain)));
    let again = script_cmd(&s)
        .args(["script", "run", "s.txt", "l", "--gui", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&again), 0, "{}", stderr(&again));
    assert_eq!(stdout(&again), stdout(&plain));
    let sent = scripts(&gui);
    assert_eq!(
        sent[1], sent[2],
        "script run --gui sends what gui script sends"
    );
    assert!(matches!(
        &sent[1],
        Request::RunScript {
            wait: false,
            close: None,
            ..
        }
    ));
    // `gui open @s.txt l --wait --closescript` is `gui script s.txt l --wait --closescript`.
    let legacy = script_cmd(&s)
        .args([
            "gui",
            "open",
            "@s.txt",
            "l",
            "--wait",
            "--closescript",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&legacy), 107, "{}", stderr(&legacy));
    let sent = scripts(&gui);
    assert!(
        matches!(&sent[3], Request::RunScript { args, wait: true, close: Some(true), .. } if args == &["l"]),
        "{:?}",
        sent[3]
    );
    assert_eq!(json(&legacy)["schema"], "hikasa.gui_script.v1");
    server.shutdown().unwrap();
}

/// The script named as the platform spells a path (`sub dir\s.txt` on Windows), an argument with a space: the request
/// holds the absolute path of that file and the argument as one word.
#[test]
fn a8_18_the_script_path_and_arguments_as_the_platform_spells_them() {
    let s = script_sandbox();
    s.file("sub dir/s.txt", "load %1 r\nbeep\n");
    s.file("a b/x.txt", "x\n");
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::Success);
    let server = serve(&s, Arc::clone(&gui));
    let named = format!("sub dir{}s.txt", std::path::MAIN_SEPARATOR);
    let out = script_cmd(&s)
        .args(["gui", "script", &named, "a b"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let sent = scripts(&gui);
    let [Request::RunScript { path, args, .. }] = sent.as_slice() else {
        panic!("{sent:?}")
    };
    assert!(Path::new(path).is_absolute(), "{path}");
    assert_eq!(canonical(path), canonical(s.join("sub dir").join("s.txt")));
    assert_eq!(args, &["a b"]);
    server.shutdown().unwrap();
}

#[test]
fn a8_18_errors_of_the_script_come_before_any_connection() {
    let s = script_sandbox();
    s.file("bad.txt", "load l r\nselct all\n");
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::Success);
    let server = serve(&s, Arc::clone(&gui));
    let missing = script_cmd(&s)
        .args(["gui", "script", "nope.txt", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&missing), 105, "{}", stderr(&missing));
    assert_eq!(json(&missing)["schema"], "hikasa.gui_script.v1");
    let bad = script_cmd(&s)
        .args(["gui", "script", "bad.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&bad), 106);
    assert!(
        stderr(&bad).contains("bad.txt:2:1: error:"),
        "the error with its position: {}",
        stderr(&bad)
    );
    // The GUI has no standard output: `output-to:-` is refused here already.
    s.file(
        "out.txt",
        "load l r\nfolder-report layout:summary output-to:-\n",
    );
    let out = script_cmd(&s)
        .args(["gui", "script", "out.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 106, "{}", stderr(&out));
    // `%1` without an argument.
    let args = script_cmd(&s)
        .args(["gui", "open", "@s.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&args), 106, "{}", stderr(&args));
    // A bad confirm mode, and a non-UTF-8 argument: 100.
    let confirm = script_cmd(&s)
        .args(["gui", "script", "s.txt", "l", "--confirm", "maybe"])
        .output()
        .unwrap();
    assert_eq!(code(&confirm), 100);
    {
        #[cfg(unix)]
        let raw = {
            use std::os::unix::ffi::OsStrExt;
            std::ffi::OsStr::from_bytes(b"l\xFF").to_owned()
        };
        // A lone surrogate: what Windows has that is not UTF-8.
        #[cfg(windows)]
        let raw = {
            use std::os::windows::ffi::OsStringExt;
            std::ffi::OsString::from_wide(&[u16::from(b'l'), 0xD800])
        };
        let out = script_cmd(&s)
            .args(["gui", "script", "s.txt"])
            .arg(raw)
            .output()
            .unwrap();
        assert_eq!(code(&out), 100, "{}", stderr(&out));
        assert!(
            stderr(&out).contains(
                "argument %1 cannot be sent to the GUI: not valid UTF-8; use hikasa script run"
            ),
            "{}",
            stderr(&out)
        );
    }
    assert!(scripts(&gui).is_empty(), "the GUI was never asked");
    server.shutdown().unwrap();
    // Without a GUI: still 105 first; a good script with --no-launch is 100.
    let missing = script_cmd(&s)
        .args(["gui", "script", "nope.txt", "--no-launch"])
        .output()
        .unwrap();
    assert_eq!(code(&missing), 105);
    let none = script_cmd(&s)
        .args(["gui", "script", "s.txt", "l", "--no-launch"])
        .output()
        .unwrap();
    assert_eq!(code(&none), 100, "{}", stderr(&none));
}

#[test]
fn a8_18_silent_never_goes_through_a_gui() {
    let s = script_sandbox();
    s.file(
        "fail.txt",
        "load l r\nselect all\ncompare crc\nload nowhere r\nbeep\n",
    );
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::Success);
    let server = serve(&s, Arc::clone(&gui));
    for args in [
        &["gui", "script", "fail.txt", "--silent"][..],
        &["gui", "open", "@fail.txt", "--silent"],
    ] {
        let out = script_cmd(&s).args(args).output().unwrap();
        assert_eq!(
            code(&out),
            107,
            "{args:?}: the script's own code: {}",
            stderr(&out)
        );
        assert!(
            stderr(&out).contains("run L4 load nowhere r"),
            "the stream on stderr: {}",
            stderr(&out)
        );
    }
    assert!(scripts(&gui).is_empty(), "no RunScript");
    // `--silent` with `--gui` is a usage error.
    let out = script_cmd(&s)
        .args(["script", "run", "fail.txt", "--gui", "--silent"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100, "a usage error: {}", stderr(&out));
    server.shutdown().unwrap();
}

/// Review F2: `script run --gui --dry-run` and `gui script --dry-run` ask the GUI for a dry run (`dry_run: true`);
/// a GUI that does not list the feature in its `Hello` would run the script for real, so nothing is sent (exit 100).
#[test]
fn a8_18_a_dry_run_in_the_gui_is_asked_for_and_never_sent_to_a_gui_that_cannot() {
    let s = script_sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::Success);
    let server = serve(&s, Arc::clone(&gui));
    for args in [
        &["script", "run", "s.txt", "l", "--gui", "--dry-run"][..],
        &["gui", "script", "s.txt", "l", "--dry-run"],
    ] {
        let out = script_cmd(&s).args(args).output().unwrap();
        assert_eq!(code(&out), 0, "{args:?}: {}", stderr(&out));
    }
    let sent = scripts(&gui);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(
        sent.iter()
            .all(|r| matches!(r, Request::RunScript { dry_run: true, .. })),
        "{sent:?}"
    );
    server.shutdown().unwrap();
    let older = FakeGui::new(CloseMode::Immediately, ExitCode::Success).without_dry_runs();
    let server = serve(&s, Arc::clone(&older));
    let out = script_cmd(&s)
        .args(["script", "run", "s.txt", "l", "--gui", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert!(
        stderr(&out).contains("cannot run a script as a dry run"),
        "{}",
        stderr(&out)
    );
    assert!(scripts(&older).is_empty(), "nothing was sent");
    // Without --dry-run that GUI runs the script.
    let out = script_cmd(&s)
        .args(["gui", "script", "s.txt", "l"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    server.shutdown().unwrap();
}

#[test]
fn a8_18_an_older_gui_does_not_support_scripts() {
    let s = script_sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::Success).without_sessions();
    let server = serve(&s, Arc::clone(&gui));
    let out = script_cmd(&s)
        .args(["gui", "script", "s.txt", "l", "--wait"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    assert!(
        stderr(&out).contains("the running GUI does not support scripts; restart it"),
        "{}",
        stderr(&out)
    );
    server.shutdown().unwrap();
}
