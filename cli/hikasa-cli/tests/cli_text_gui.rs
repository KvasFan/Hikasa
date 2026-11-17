//! A11.21 (M2 W11b, D-314): `hikasa text compare --gui` sends the per-side formats, the anchors (`--align`,
//! `--isolate`) and the ignored lines (`--ignored`) as typed in its `OpenView` (a fake GUI server in this process
//! records it); a bad word or format name is refused before the GUI is contacted; `--format` with a side's own format
//! is refused too.

mod common;

use std::sync::Arc;

use common::{CloseMode, FakeGui, Sandbox, code, json, serve, stderr};
use hikasa_core::ExitCode;
use hikasa_session::ipc::{Request, ViewKind};

fn sandbox() -> Sandbox {
    let s = Sandbox::new();
    s.file("a.c", "int a;\nint b;\nint c;\n");
    s.file("b.c", "int a;\nint B;\nint c;\n");
    s
}

fn text(s: &Sandbox, args: &[&str]) -> std::process::Output {
    s.cmd()
        .args(["text", "compare", "a.c", "b.c", "--gui", "--no-launch"])
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn a11_21_the_words_reach_the_open_view() {
    let s = sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame);
    let server = serve(&s, Arc::clone(&gui));
    let out = text(
        &s,
        &[
            "--left-format",
            "c-cpp",
            "--right-format",
            "Everything Else",
            "--align",
            "2:2",
            "--isolate",
            "left:3",
            "--ignored",
            "right:1-2",
            "--json",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.gui_open.v1", "{doc}");
    let requests: Vec<Request> = gui
        .requests()
        .into_iter()
        .filter(|r| matches!(r, Request::OpenView { .. }))
        .collect();
    let [Request::OpenView { kind, options, .. }] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    assert_eq!(*kind, ViewKind::TextCompare);
    assert_eq!(
        options.formats,
        [Some("c-cpp".to_owned()), Some("Everything Else".to_owned())]
    );
    assert_eq!(options.anchors, ["2:2", "left:3"]);
    assert_eq!(options.ignored, ["right:1-2"]);
    server.shutdown().unwrap();
}

#[test]
fn a11_21_bad_words_are_refused_before_any_contact() {
    let s = sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame);
    let server = serve(&s, Arc::clone(&gui));
    for args in [
        &["--align", "x:2"][..],
        &["--isolate", "middle:3"],
        &["--ignored", "left:0"],
        &["--left-format", "no-such-format"],
        &["--format", "c-cpp", "--right-format", "c-cpp"],
    ] {
        let mut all = args.to_vec();
        all.push("--json");
        let out = text(&s, &all);
        assert_eq!(code(&out), 100, "{args:?}: {}", stderr(&out));
        let doc = json(&out);
        assert_eq!(doc["schema"], "hikasa.gui_open.v1", "{doc}");
        assert_eq!(doc["error"]["kind"], "invalid_input", "{args:?}: {doc}");
    }
    assert!(
        !gui.requests()
            .iter()
            .any(|r| matches!(r, Request::OpenView { .. })),
        "nothing was sent"
    );
    server.shutdown().unwrap();
}
