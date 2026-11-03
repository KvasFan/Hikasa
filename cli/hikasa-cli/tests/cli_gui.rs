//! `hikasa gui …` end to end (contract §6.3 `cli_gui.rs`, §2.21, §3.6; D-22, D-23, D-49, D-51).
//!
//! Scripted GUI: an in-process server ([`common::FakeGui`] on the sandbox's `HIKASA_IPC_ENDPOINT`). The paths that
//! launch a GUI process are in `cli_gui_launch.rs` (a test binary that is also the fake GUI).

#![cfg(unix)]

mod common;

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

/// The `OpenView` requests a fake GUI received.
fn opened(gui: &FakeGui) -> Vec<Request> {
    gui.requests()
        .into_iter()
        .filter(|r| matches!(r, Request::OpenView { .. }))
        .collect()
}

#[test]
fn control_commands_without_a_gui_are_100() {
    let s = sandbox();
    for args in [
        &["gui", "ping"][..],
        &["gui", "list"],
        &["gui", "close", "3"],
        &["gui", "quit"],
    ] {
        let out = s.cmd().args(args).arg("--json").output().unwrap();
        assert_eq!(code(&out), 100, "D-22 {args:?}: {}", stderr(&out));
        let doc = json(&out);
        assert_eq!(doc["ok"], false);
        assert!(doc["schema"].as_str().unwrap().starts_with("hikasa.gui_"));
    }
}

#[test]
fn control_commands_against_a_gui() {
    let s = sandbox();
    let gui = FakeGui::new(CloseMode::OnRequest, ExitCode::RulesSame);
    let server = serve(&s, Arc::clone(&gui));
    let ping = s.cmd().args(["gui", "ping"]).output().unwrap();
    assert_eq!(code(&ping), 0, "{}", stderr(&ping));
    assert_eq!(stdout(&ping), "fake-gui test\t1\n");
    let ping = s.cmd().args(["gui", "ping", "--json"]).output().unwrap();
    assert_golden("gui/ping.json", &s.normalise(&stdout(&ping)));
    let open = s
        .cmd()
        .args(["gui", "open", "a.c", "b.c"])
        .output()
        .unwrap();
    assert_eq!(code(&open), 0, "{}", stderr(&open));
    let list = s.cmd().args(["gui", "list"]).output().unwrap();
    assert_eq!(code(&list), 0);
    assert_golden("gui/list.txt", &s.normalise(&stdout(&list)));
    let list = s.cmd().args(["gui", "list", "--json"]).output().unwrap();
    assert_golden("gui/list.json", &s.normalise(&stdout(&list)));
    let close = s
        .cmd()
        .args(["gui", "close", "7", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&close), 0, "{}", stderr(&close));
    assert_golden("gui/close.json", &s.normalise(&stdout(&close)));
    let unknown = s
        .cmd()
        .args(["gui", "close", "7", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&unknown), 100, "an unknown view");
    assert_eq!(json(&unknown)["error"]["kind"], "not_found");
    let quit = s.cmd().args(["gui", "quit", "--json"]).output().unwrap();
    assert_eq!(code(&quit), 0);
    assert_golden("gui/quit.json", &s.normalise(&stdout(&quit)));
    assert!(
        gui.requests()
            .iter()
            .any(|r| matches!(r, Request::Shutdown))
    );
    server.shutdown().unwrap();
}

#[test]
fn open_wait_returns_the_scripted_code() {
    let s = sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::Similar);
    let server = serve(&s, Arc::clone(&gui));
    let out = s
        .cmd()
        .args(["gui", "open", "a.c", "b.c", "--wait", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 12, "the view's code (D-23): {}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["exit_code"], 12);
    assert_eq!(doc["result"]["waited"], true);
    assert_golden("gui/open-wait.json", &s.normalise(&stdout(&out)));
    // Without --wait: 0 once the view is open.
    let out = s
        .cmd()
        .args(["gui", "open", "l", "r", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0);
    assert_golden("gui/open.json", &s.normalise(&stdout(&out)));
    let requests = opened(&gui);
    let Request::OpenView { kind, specs, .. } = &requests[1] else {
        unreachable!()
    };
    assert_eq!(
        kind.as_str(),
        "folder_compare",
        "two folders open a folder view"
    );
    assert!(
        specs.iter().all(|s| s.starts_with('/')),
        "absolute specs: {specs:?}"
    );
    server.shutdown().unwrap();
}

#[test]
fn no_launch() {
    let s = sandbox();
    let without = s
        .cmd()
        .args(["gui", "open", "a.c", "b.c", "--no-launch"])
        .output()
        .unwrap();
    assert_eq!(code(&without), 100, "{}", stderr(&without));
    let with_wait = s
        .cmd()
        .args(["gui", "open", "a.c", "b.c", "--no-launch", "--wait"])
        .output()
        .unwrap();
    assert_eq!(code(&with_wait), 102, "{}", stderr(&with_wait));
    let legacy = s
        .cmd()
        .args(["gui", "open", "-nolaunch", "-wait", "a.c", "b.c"])
        .output()
        .unwrap();
    assert_eq!(code(&legacy), 102);
    let compare = s
        .cmd()
        .args([
            "text",
            "compare",
            "a.c",
            "b.c",
            "--gui",
            "--wait",
            "--no-launch",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&compare), 102);
}

#[test]
fn a_missing_gui_executable_is_103() {
    let s = sandbox();
    for extra in [&[][..], &["--wait"]] {
        let out = s
            .cmd()
            .args(["gui", "open", "a.c", "b.c"])
            .args(extra)
            .arg("--json")
            .output()
            .unwrap();
        assert_eq!(code(&out), 103, "{extra:?}");
        assert_eq!(json(&out)["exit_code"], 103);
    }
    let compare = s
        .cmd()
        .args(["folder", "compare", "l", "r", "--gui"])
        .output()
        .unwrap();
    assert_eq!(code(&compare), 103);
}

/// Regression (wave-5 review): a GUI executable that exists but cannot be started — no execute permission, a
/// missing interpreter — is no usable GUI executable: 103 with a message naming it (it was 100 "ipc I/O").
#[test]
fn a_gui_executable_that_cannot_start_is_103() {
    let s = sandbox();
    let not_executable = s.file("notexec", "#!/bin/sh\nexit 0\n");
    let bad_interpreter = s.file("badinterp", "#!/nonexistent/interp\n");
    common::set_mode(&bad_interpreter, 0o755);
    for (exe, kind) in [
        (&not_executable, "permission_denied"),
        (&bad_interpreter, "not_found"),
    ] {
        for extra in [&[][..], &["--wait"]] {
            let out = s
                .cmd()
                .args(["gui", "open", "a.c", "b.c", "--json"])
                .args(extra)
                .env("HIKASA_GUI_EXE", exe)
                .output()
                .unwrap();
            assert_eq!(code(&out), 103, "{exe:?} {extra:?}: {}", stderr(&out));
            let doc = json(&out);
            assert_eq!(doc["exit_code"], 103);
            assert_eq!(doc["error"]["kind"], kind, "{doc}");
            let message = doc["error"]["message"].as_str().unwrap();
            assert!(
                message.contains("cannot start") && message.contains(&*exe.to_string_lossy()),
                "{message}"
            );
            assert!(stderr(&out).contains("cannot start"), "{}", stderr(&out));
        }
    }
    let compare = s
        .cmd()
        .args(["text", "compare", "a.c", "b.c", "--gui"])
        .env("HIKASA_GUI_EXE", &not_executable)
        .output()
        .unwrap();
    assert_eq!(code(&compare), 103, "{}", stderr(&compare));
}

/// Regression (wave-5 review, D-51): the arguments after `open` reach the shared parser unchanged, including a
/// leading `--`, so `hikasa gui open -- -x a.c` opens the files `-x` and `a.c` like `hikasa-gui -- -x a.c`.
#[test]
fn a_separator_right_after_open_is_kept() {
    let s = sandbox();
    s.file("-x", "int x;\n");
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame);
    let server = serve(&s, Arc::clone(&gui));
    for args in [
        &["gui", "open", "--", "-x", "a.c"][..],
        &["gui", "open", "--json", "--", "-x", "a.c"],
        &["gui", "open", "-q", "--", "-x", "a.c"],
    ] {
        let out = s.cmd().args(args).output().unwrap();
        assert_eq!(code(&out), 0, "{args:?}: {}", stderr(&out));
        if args.contains(&"--json") {
            assert_eq!(json(&out)["schema"], "hikasa.gui_open.v1");
        }
    }
    let requests = opened(&gui);
    assert_eq!(requests.len(), 3);
    for request in &requests {
        let Request::OpenView { specs, .. } = request else {
            unreachable!()
        };
        assert!(
            specs[0].ends_with("/-x") && specs[1].ends_with("/a.c"),
            "{specs:?}"
        );
    }
    // After the separator `--json` is a spec, not a flag: human mode, and the GUI receives the name.
    let out = s
        .cmd()
        .args(["gui", "open", "--", "--json", "a.c"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(out.stdout.is_empty(), "no JSON document: {}", stdout(&out));
    let requests = opened(&gui);
    let Some(Request::OpenView { specs, .. }) = requests.last() else {
        panic!("no OpenView");
    };
    assert!(specs[0].ends_with("/--json"), "{specs:?}");
    server.shutdown().unwrap();
}

/// Regression (wave-5 review, §1.8.7 item 34): a global value flag repeated with different values is a usage
/// error wherever the repeats stand (clap used to keep the innermost one silently); the same folder spelled
/// relative and absolute is one value.
#[test]
fn repeated_globals_must_agree_at_every_position() {
    let s = sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame);
    let server = serve(&s, Arc::clone(&gui));
    for args in [
        &["--json", "gui", "open", "--config-dir", "cB", "a.c", "b.c"][..],
        &["--json", "gui", "open", "a.c", "b.c", "--config-dir", "cB"],
        &["--json", "gui", "--config-dir", "cB", "open", "a.c", "b.c"],
    ] {
        // `s.cmd()` gives `--config-dir <tmp>/cfg` first.
        let out = s.cmd().args(args).output().unwrap();
        assert_eq!(code(&out), 100, "{args:?}");
        let doc = json(&out);
        assert_eq!(doc["schema"], "hikasa.usage_error.v1", "{args:?}");
        assert!(
            doc["error"]["message"]
                .as_str()
                .unwrap()
                .contains("`--config-dir` is given twice"),
            "{doc}"
        );
    }
    // The same folder, relative and absolute, both ways.
    for args in [
        &["gui", "open", "--config-dir", "cfg", "a.c", "b.c"][..],
        &["gui", "open", "a.c", "b.c", "--config-dir", "./cfg/"],
    ] {
        let out = s.cmd().args(args).output().unwrap();
        assert_eq!(code(&out), 0, "{args:?}: {}", stderr(&out));
    }
    assert!(opened(&gui).len() == 2);
    server.shutdown().unwrap();
}

/// Regression (wave-5 review): the legacy spelling `-json` asks for JSON also when the `gui open` arguments do
/// not parse; `--help` stays text with `--json`; `--version` is not a `gui open` switch wherever it stands
/// (the shared parser's rule, as for `hikasa-gui`).
#[test]
fn json_help_and_version_in_the_open_arguments() {
    let s = sandbox();
    let out = s
        .cmd()
        .args(["gui", "open", "a.c", "b.c", "-json", "--bogus"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.usage_error.v1");
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--bogus"),
        "{doc}"
    );
    let help = s
        .cmd()
        .args(["gui", "open", "--json", "--help"])
        .output()
        .unwrap();
    assert_eq!(code(&help), 0);
    assert!(stdout(&help).starts_with("Usage: hikasa gui open"));
    for args in [
        &["gui", "open", "--version"][..],
        &["gui", "open", "a.c", "b.c", "--version"],
    ] {
        let out = s.cmd().args(args).output().unwrap();
        assert_eq!(code(&out), 100, "{args:?}");
        assert!(
            stderr(&out).contains("unknown switch `--version`"),
            "{}",
            stderr(&out)
        );
    }
}

#[test]
fn legacy_spellings_and_forwarded_switches() {
    let s = sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame);
    let server = serve(&s, Arc::clone(&gui));
    let out = s
        .cmd()
        .args([
            "gui",
            "open",
            "-ro",
            "--lefttitle=x",
            "-title2",
            "y z",
            "--fv=Text Compare",
            "--automerge",
            "a.c",
            "b.c",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["warnings"][0]["code"], "switch_ignored", "{doc}");
    let requests = opened(&gui);
    let Some(Request::OpenView { kind, options, .. }) = requests.first() else {
        panic!("no OpenView");
    };
    assert_eq!(kind.as_str(), "text_compare");
    assert!(options.read_only.all);
    assert_eq!(options.titles[0].as_deref(), Some("x"));
    assert_eq!(options.titles[1].as_deref(), Some("y z"));
    assert!(options.extra.contains_key("automerge"));
    // The human mode prints the warning on stderr.
    let human = s
        .cmd()
        .args(["gui", "open", "--automerge", "a.c", "b.c"])
        .output()
        .unwrap();
    assert!(stderr(&human).contains("automerge"), "{}", stderr(&human));
    server.shutdown().unwrap();
}

#[test]
fn bad_open_arguments_are_100() {
    let s = sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame);
    let server = serve(&s, Arc::clone(&gui));
    for args in [
        &["--bogus", "a.c", "b.c"][..],
        &["a.c"],
        &["a.c", "l"],
        &["--fv=Spreadsheet", "a.c", "b.c"],
        &["-", "-"],
    ] {
        let out = s
            .cmd()
            .args(["gui", "open"])
            .args(args)
            .write_stdin("x")
            .output()
            .unwrap();
        assert_eq!(code(&out), 100, "{args:?}: {}", stderr(&out));
    }
    assert!(opened(&gui).is_empty(), "nothing reached the GUI");
    // `--gui` with a bad --criteria is refused without contacting the GUI.
    let out = s
        .cmd()
        .args([
            "folder",
            "compare",
            "l",
            "r",
            "--gui",
            "--criteria",
            "crc,binary",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    assert!(opened(&gui).is_empty());
    server.shutdown().unwrap();
}

#[test]
fn help_and_flag_merging() {
    let s = sandbox();
    let help = s.cmd().args(["gui", "open", "--help"]).output().unwrap();
    assert_eq!(code(&help), 0);
    assert!(
        stdout(&help).starts_with("Usage: hikasa gui open"),
        "{}",
        stdout(&help)
    );
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame);
    let server = serve(&s, Arc::clone(&gui));
    // A global flag means the same before and after the specs.
    let after = s
        .cmd()
        .args(["gui", "open", "a.c", "b.c", "--json", "-q"])
        .output()
        .unwrap();
    assert_eq!(code(&after), 0, "{}", stderr(&after));
    assert_eq!(json(&after)["schema"], "hikasa.gui_open.v1");
    // The same value flag given both ways with different values is a usage error.
    let twice = s
        .cmd()
        .args(["gui", "open", "a.c", "b.c", "--json", "--config-dir"])
        .arg(s.join("elsewhere"))
        .output()
        .unwrap();
    assert_eq!(code(&twice), 100);
    assert_eq!(json(&twice)["schema"], "hikasa.usage_error.v1");
    let same = s
        .cmd()
        .args(["gui", "open", "a.c", "b.c", "--config-dir"])
        .arg(s.config())
        .output()
        .unwrap();
    assert_eq!(
        code(&same),
        0,
        "the same value both ways is fine: {}",
        stderr(&same)
    );
    server.shutdown().unwrap();
}

#[test]
fn standard_input_is_spooled_and_removed_after_waiting() {
    let s = sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesDifferent);
    let server = serve(&s, Arc::clone(&gui));
    let out = s
        .cmd()
        .args([
            "gui",
            "open",
            "-",
            "b.c",
            "--stdin-name",
            "x.c",
            "--wait",
            "--json",
        ])
        .write_stdin("int x;\n")
        .output()
        .unwrap();
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    let doc = json(&out);
    let spool = doc["result"]["spooled_stdin"].as_str().unwrap().to_owned();
    assert!(spool.ends_with("x.c"), "D-49: {spool}");
    assert!(
        spool.starts_with(&s.config().join("cache").display().to_string()),
        "{spool}"
    );
    assert!(
        !std::path::Path::new(&spool).exists(),
        "deleted after the view closed"
    );
    let requests = opened(&gui);
    let Some(Request::OpenView { specs, .. }) = requests.first() else {
        panic!("no OpenView");
    };
    assert_eq!(specs[0], spool);
    server.shutdown().unwrap();
}
