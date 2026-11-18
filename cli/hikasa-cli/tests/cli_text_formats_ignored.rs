//! `--left-format`/`--right-format` and `--ignored` end to end (contract §6.10 A10.20, A10.21, §1.10.16, §1.10.17,
//! §1.10.24; D-281, D-282): JSON `right_format`, the conflict with `--format` (exit 100), `[specs] left_format`
//! through `session run`; ignored lines on `text compare` (a pair whose only differences are ignored exits as
//! rules same), `report text` and `session run`, a line beyond a document and malformed words (exit 100), and a
//! session whose marked lines changed dropping its marks with `ignored_lines_dropped`.

mod common;

use common::{Sandbox, code, json, stderr, stdout};

const C_TEXT: &str = "int a = 1; // one\nint b = 2;\n/* c */ int c = 3;\nreturn a;\n";

/// `a.c` and `b.txt`: the same C text, `b.txt` with one more line (`extra`, right line 5).
fn pair() -> Sandbox {
    let s = Sandbox::new();
    s.file("a.c", C_TEXT);
    s.file("b.txt", format!("{C_TEXT}extra\n"));
    s
}

fn run(s: &Sandbox, args: &[&str]) -> std::process::Output {
    s.cmd().args(args).output().unwrap()
}

#[test]
fn a_format_per_pane() {
    let s = pair();
    let base = [
        "text",
        "compare",
        "a.c",
        "b.txt",
        "--unimportant",
        "Comment",
        "--json",
    ];
    let one = run(&s, &base);
    assert_eq!(code(&one), 13, "{}", stderr(&one));
    let one = json(&one);
    assert!(one["result"].get("right_format").is_none());
    let plain = run(
        &s,
        &[&base[..], &["--right-format", "Everything Else"]].concat(),
    );
    assert_eq!(code(&plain), 13, "{}", stderr(&plain));
    let plain = json(&plain);
    let r = &plain["result"];
    assert_eq!(r["right_format"]["name"], "Everything Else");
    assert_eq!(r["right_format"]["reason"]["reason"], "fallback");
    assert_eq!(r["format"]["id"], "c-cpp");
    let differing = |doc: &serde_json::Value| {
        let st = &doc["result"]["stats"];
        st["rows"].as_u64().unwrap() - st["same_rows"].as_u64().unwrap()
    };
    assert!(differing(&plain) > differing(&one));
    // By id on the left: the right side keeps the pair's format (D-15: the left name's).
    let java = json(&run(&s, &[&base[..], &["--left-format", "java"]].concat()));
    assert_eq!(java["result"]["format"]["id"], "java");
    assert_eq!(java["result"]["right_format"]["id"], "c-cpp");
    // `--format` with either is exit 100.
    let both = run(
        &s,
        &[&base[..], &["--format", "c-cpp", "--right-format", "java"]].concat(),
    );
    assert_eq!(code(&both), 100, "{}", stdout(&both));
    assert_eq!(json(&both)["error"]["kind"], "invalid_input");
    let unknown = run(
        &s,
        &[&base[..], &["--right-format", "no-such-format"]].concat(),
    );
    assert_eq!(code(&unknown), 100);
}

#[test]
fn ignored_lines_on_text_compare_and_report_text() {
    let s = pair();
    let plain = run(&s, &["text", "compare", "a.c", "b.txt"]);
    assert_eq!(code(&plain), 13);
    // The extra line ignored: rules same (exit 2).
    let same = run(
        &s,
        &[
            "text",
            "compare",
            "a.c",
            "b.txt",
            "--ignored",
            "right:5",
            "--json",
        ],
    );
    assert_eq!(code(&same), 2, "{}", stdout(&same));
    let doc = json(&same);
    assert_eq!(doc["result"]["verdict"]["result"], "rules_same");
    assert_eq!(doc["result"]["stats"]["ignored_rows"], 1);
    assert_eq!(
        doc["result"]["ignored"],
        serde_json::json!({"left": [], "right": [[5, 6]]})
    );
    let marked: Vec<&serde_json::Value> = doc["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["ignored"] == true)
        .collect();
    // The default JSON display is `mismatches`: the ignored row counts as same and is not listed.
    assert!(marked.is_empty());
    let all = json(&run(
        &s,
        &[
            "text",
            "compare",
            "a.c",
            "b.txt",
            "--ignored",
            "right:5",
            "--display",
            "all",
            "--json",
        ],
    ));
    let marked = all["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["ignored"] == true)
        .count();
    assert_eq!(marked, 1);
    // `report text` takes the flag: the ignored hunk is left out of the patch.
    let report = run(
        &s,
        &[
            "report",
            "text",
            "a.c",
            "b.txt",
            "--ignored",
            "right:5",
            "--report",
            "patch",
        ],
    );
    assert_eq!(code(&report), 2, "{}", stderr(&report));
    assert!(!stdout(&report).contains("+extra"), "{}", stdout(&report));
    // A line beyond the file, malformed words: exit 100.
    for bad in ["right:6", "left:0", "middle:1", "right"] {
        let out = run(
            &s,
            &[
                "text",
                "compare",
                "a.c",
                "b.txt",
                "--ignored",
                bad,
                "--json",
            ],
        );
        assert_eq!(code(&out), 100, "{bad}: {}", stdout(&out));
        assert_eq!(json(&out)["error"]["kind"], "invalid_input", "{bad}");
    }
    // `--gui` does not take them.
    let gui = run(
        &s,
        &[
            "text",
            "compare",
            "a.c",
            "b.txt",
            "--gui",
            "--no-launch",
            "--ignored",
            "right:5",
            "--json",
        ],
    );
    assert_eq!(code(&gui), 100);
}

#[test]
fn sessions_keep_pane_formats_and_ignored_lines() {
    let s = pair();
    let new = run(
        &s,
        &[
            "session",
            "new",
            "t",
            "--type",
            "text-compare",
            "--left",
            "a.c",
            "--right",
            "b.txt",
        ],
    );
    assert_eq!(code(&new), 0, "{}", stderr(&new));
    let set = |key: &str, value: &str| {
        let out = run(
            &s,
            &["session", "edit", "t", "--set", &format!("{key}={value}")],
        );
        assert_eq!(code(&out), 0, "{key}: {}", stderr(&out));
    };
    set("specs.right_format", "\"Everything Else\"");
    set("specs.ignored_right", "[\"5\"]");
    let session = run(&s, &["session", "run", "t", "--json"]);
    assert_eq!(code(&session), 2, "{}", stdout(&session));
    let doc = json(&session);
    assert_eq!(doc["result"]["right_format"]["name"], "Everything Else");
    assert_eq!(doc["result"]["stats"]["ignored_rows"], 1);
    // `session run --ignored` replaces the session's marks of the side it names.
    let other = run(
        &s,
        &["session", "run", "t", "--ignored", "right:1", "--json"],
    );
    assert_eq!(code(&other), 13, "{}", stdout(&other));
    assert_eq!(
        json(&other)["result"]["ignored"]["right"],
        serde_json::json!([[1, 2]])
    );
    // A saved check that the marked lines no longer have: the marks are dropped with the warning.
    set("specs.ignored_check_right", "\"00000000\"");
    let dropped = run(&s, &["session", "run", "t", "--json"]);
    assert_eq!(code(&dropped), 13, "{}", stdout(&dropped));
    let doc = json(&dropped);
    let codes: Vec<&str> = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|w| w["code"].as_str())
        .collect();
    assert!(codes.contains(&"ignored_lines_dropped"), "{codes:?}");
    assert_eq!(doc["result"]["stats"]["ignored_rows"], 0);
    // `report text --session` takes the flags on top.
    let report = run(
        &s,
        &["report", "text", "--session", "t", "--ignored", "right:5"],
    );
    assert_eq!(code(&report), 2, "{}", stdout(&report));
    // A folder session refuses them.
    s.dir("l");
    s.dir("r");
    let folder = run(
        &s,
        &[
            "session",
            "new",
            "f",
            "--type",
            "folder-compare",
            "--left",
            "l",
            "--right",
            "r",
        ],
    );
    assert_eq!(code(&folder), 0, "{}", stderr(&folder));
    let refused = run(
        &s,
        &["session", "run", "f", "--ignored", "left:1", "--json"],
    );
    assert_eq!(code(&refused), 100);
}
