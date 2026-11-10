//! `hikasa table compare --gui` (Table Compare phase 3, T-44): the words reach the window's `OpenView` of kind
//! `table_compare` (a fake GUI server in this process records it), a bad word is refused before the GUI is contacted,
//! and the exit codes are those of `gui open`: 0 without `--wait`, the view's own code with it, 100 without a GUI and
//! `--no-launch`.

mod common;

use std::sync::Arc;

use common::{CloseMode, FakeGui, Sandbox, code, json, serve, stderr};
use hikasa_core::ExitCode;
use hikasa_session::ipc::{Request, ViewKind};

fn sandbox() -> Sandbox {
    let s = Sandbox::new();
    s.file("a.csv", "id;v\n1;1,5\n");
    s.file("b.csv", "id;v\n1;2\n");
    s
}

fn table(s: &Sandbox, args: &[&str]) -> std::process::Output {
    s.cmd()
        .args(["table", "compare", "a.csv", "b.csv", "--gui", "--no-launch"])
        .args(args)
        .output()
        .unwrap()
}

/// T-44: the parser and rule words, the per-side formats, `--display` and `--ignore-unimportant` travel as typed, with
/// the two absolute specs; without `--wait` the command exits 0 with `hikasa.gui_open.v1`.
#[test]
fn t44_the_words_reach_the_open_view() {
    let s = sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame);
    let server = serve(&s, Arc::clone(&gui));
    let out = table(
        &s,
        &[
            "--delimiter",
            ";",
            "--decimal",
            ",",
            "--key",
            "id",
            "--tolerance",
            "v=0.5",
            "--sheets",
            "position",
            "--right-format",
            "Comma Separated Values",
            "--display",
            "differences",
            "--iu",
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
    let [
        Request::OpenView {
            kind,
            specs,
            options,
            wait,
        },
    ] = requests.as_slice()
    else {
        panic!("{requests:?}");
    };
    assert_eq!(*kind, ViewKind::TableCompare);
    assert!(!wait);
    assert_eq!(
        specs,
        &[
            s.join("a.csv").to_string_lossy().into_owned(),
            s.join("b.csv").to_string_lossy().into_owned()
        ]
    );
    assert_eq!(options.display.as_deref(), Some("differences"));
    assert!(options.ignore_unimportant);
    let words = options.table.as_ref().expect("the table words");
    assert_eq!(words.parse.delimiters, [";"]);
    assert_eq!(words.parse.decimal.as_deref(), Some(","));
    assert_eq!(words.rules.rules.keys, ["id"]);
    assert_eq!(words.rules.rules.tolerance, ["v=0.5"]);
    assert_eq!(words.rules.sheets.as_deref(), Some("position"));
    assert_eq!(
        words.formats,
        [None, Some("Comma Separated Values".to_owned())]
    );
    server.shutdown().unwrap();
}

/// With `--wait` the command exits with the view's code (the fake view closes with 2 at once).
#[test]
fn t44_wait_exits_with_the_view_code() {
    let s = sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame);
    let server = serve(&s, Arc::clone(&gui));
    let out = table(&s, &["--wait", "--json"]);
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    assert_eq!(json(&out)["result"]["waited"], true);
    server.shutdown().unwrap();
}

/// A bad word, a format of another kind or a report flag is exit 100 before the GUI is contacted; no GUI and
/// `--no-launch` is exit 100 too.
#[test]
fn t44_refusals_before_any_contact() {
    let s = sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame);
    let server = serve(&s, Arc::clone(&gui));
    for (args, kind) in [
        (&["--sheets", "sideways"][..], "invalid_input"),
        (&["--format", "c-cpp"], "invalid_input"),
        (&["--display", "some"], "invalid_input"),
        (&["--cells", "all"], "invalid_input"),
        (&["--html"], "invalid_input"),
    ] {
        let mut all = args.to_vec();
        all.push("--json");
        let out = table(&s, &all);
        assert_eq!(code(&out), 100, "{args:?}");
        let doc = json(&out);
        assert_eq!(doc["schema"], "hikasa.gui_open.v1", "{doc}");
        assert_eq!(doc["error"]["kind"], kind, "{args:?}: {doc}");
    }
    assert!(
        !gui.requests()
            .iter()
            .any(|r| matches!(r, Request::OpenView { .. })),
        "nothing was sent"
    );
    server.shutdown().unwrap();
    let out = table(&s, &["--json"]);
    assert_eq!(code(&out), 100, "{}", stderr(&out));
}
