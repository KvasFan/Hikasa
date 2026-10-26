//! `hikasa text compare` end to end (contract §6.3 `cli_text.rs`, §3.2, §4.3.2; D-01, D-03, D-15, D-50).

mod common;

use std::path::Path;
use std::sync::Arc;

use common::{CloseMode, FakeGui, Sandbox, assert_golden, code, json, serve, stderr, stdout};
use hikasa_core::ExitCode;
use hikasa_session::ipc::Request;

const LEFT: &str = "#include <stdio.h>\n\n/* The program prints one number; this comment is long enough to wrap in a column. */\nint main(void) {\n    int x = 1;\n    printf(\"%d\\n\", x);\n    return 0;\n}\n";
const RIGHT: &str = "#include <stdio.h>\n\n/* The program prints one number; this comment is long enough to wrap in a column. */\nint main(void) {\n    int x = 2;\n    int y = x * 2;\n    printf(\"%d\\n\", y);\n    return 0;\n}\n";

/// The C pair of the goldens: `a/x.c` and `b/x.c`.
fn pair() -> Sandbox {
    let s = Sandbox::new();
    s.file("a/x.c", LEFT);
    s.file("b/x.c", RIGHT);
    s
}

/// Runs `text compare a/x.c b/x.c <extra>` and checks the exit code 13.
fn compare(s: &Sandbox, extra: &[&str]) -> std::process::Output {
    let out = s
        .cmd()
        .args(["text", "compare", "a/x.c", "b/x.c"])
        .args(extra)
        .output()
        .unwrap();
    assert_eq!(code(&out), 13, "{extra:?}: {}", stderr(&out));
    out
}

#[test]
fn annotated_unified_is_the_default() {
    let s = pair();
    let out = compare(&s, &[]);
    let text = stdout(&out);
    assert!(text.contains("@@ -2,7 +2,8 @@ important"), "{text}");
    assert_golden("text/unified.txt", &s.normalise(&text));
}

#[test]
fn side_by_side_text_and_html() {
    let s = pair();
    let text = compare(
        &s,
        &[
            "--report",
            "side-by-side",
            "--width",
            "120",
            "--wrap",
            "word",
        ],
    );
    assert_golden(
        "text/side-by-side.txt",
        &s.normalise_with(&stdout(&text), true),
    );
    let narrow = compare(
        &s,
        &[
            "--report",
            "side-by-side",
            "--width",
            "40",
            "--wrap",
            "none",
            "--no-line-numbers",
        ],
    );
    assert_golden(
        "text/side-by-side-narrow.txt",
        &s.normalise_with(&stdout(&narrow), true),
    );
    let html = compare(
        &s,
        &["--report", "side-by-side", "--html", "--title", "Review"],
    );
    let page = stdout(&html);
    assert!(page.contains("<html") && page.contains("Review"), "{page}");
    assert!(!page.contains("<script"), "no scripts");
    assert_golden("text/side-by-side.html", &s.normalise(&page));
    let mono = compare(&s, &["--report", "side-by-side", "--html-mono"]);
    assert!(stdout(&mono).contains("<html"));
}

#[test]
fn patch_styles() {
    let s = pair();
    for style in ["normal", "context", "unified"] {
        let out = compare(&s, &["--report", "patch", "--patch-style", style]);
        assert_golden(
            &format!("text/patch-{style}.txt"),
            &s.normalise(&stdout(&out)),
        );
    }
    let wide = compare(&s, &["--report", "patch", "--context", "1"]);
    assert_golden("text/patch-unified-1.txt", &s.normalise(&stdout(&wide)));
}

#[test]
fn summary_and_statistics() {
    let s = pair();
    let stats = compare(&s, &["--report", "statistics"]);
    assert_golden("text/statistics.csv", &s.normalise(&stdout(&stats)));
    let summary = compare(&s, &["--report", "summary"]);
    assert_golden("text/summary.txt", &s.normalise(&stdout(&summary)));
}

#[test]
fn output_is_atomic_and_prints_nothing() {
    let s = pair();
    let out = compare(
        &s,
        &["--report", "side-by-side", "--html", "--output", "r.html"],
    );
    assert!(out.stdout.is_empty(), "--output prints nothing");
    let written = std::fs::read_to_string(s.join("r.html")).unwrap();
    assert!(written.contains("<html"));
    // On error the target is untouched.
    std::fs::write(s.join("r.html"), "old report").unwrap();
    let failed = s
        .cmd()
        .args([
            "text",
            "compare",
            "a/x.c",
            "missing.c",
            "--output",
            "r.html",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&failed), 100);
    assert_eq!(
        std::fs::read_to_string(s.join("r.html")).unwrap(),
        "old report"
    );
    // A target folder that does not exist fails without leaving anything behind.
    let nowhere = s
        .cmd()
        .args([
            "text",
            "compare",
            "a/x.c",
            "b/x.c",
            "--output",
            "no/such/dir/r.txt",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&nowhere), 100);
    assert!(!s.join("no").exists());
    // With --json the envelope still goes to stdout and the report to the file.
    let both = compare(&s, &["--json", "--output", "r.txt"]);
    assert_eq!(json(&both)["schema"], "hikasa.text_compare.v1");
    assert!(
        std::fs::read_to_string(s.join("r.txt"))
            .unwrap()
            .contains("@@")
    );
}

/// Regression (wave-5 review, maintainer decision 4): `--output` through a symbolic link replaces the file the
/// link points to and keeps the link; a FIFO or a device such as `/dev/null` is written to, never replaced.
#[cfg(unix)]
#[test]
fn output_through_links_fifos_and_devices() {
    let s = pair();
    s.file("real/r.html", "KEEP");
    std::os::unix::fs::symlink("real/r.html", s.join("rlink.html")).unwrap();
    let out = compare(
        &s,
        &[
            "--report",
            "side-by-side",
            "--html",
            "--output",
            "rlink.html",
        ],
    );
    assert!(out.stdout.is_empty());
    let link = std::fs::symlink_metadata(s.join("rlink.html")).unwrap();
    assert!(link.file_type().is_symlink(), "the link stays");
    let written = std::fs::read_to_string(s.join("real/r.html")).unwrap();
    assert!(written.contains("<html"), "the linked file is updated");
    // A FIFO receives the report and stays a FIFO.
    let fifo = s.join("myfifo");
    if std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .is_ok_and(|st| st.success())
    {
        let reader = {
            let fifo = fifo.clone();
            std::thread::spawn(move || std::fs::read_to_string(fifo).unwrap())
        };
        compare(&s, &["--output", "myfifo"]);
        assert!(reader.join().unwrap().contains("@@"));
        let meta = std::fs::symlink_metadata(&fifo).unwrap();
        assert!(!meta.is_file(), "still a FIFO");
    }
    // `/dev/null` works (the temporary file used to be refused in /dev).
    let null = compare(&s, &["--output", "/dev/null"]);
    assert!(
        null.stdout.is_empty() && null.stderr.is_empty(),
        "{}",
        stderr(&null)
    );
    assert!(!std::fs::metadata("/dev/null").unwrap().is_file());
}

#[test]
fn missing_input_is_100() {
    let s = pair();
    let out = s
        .cmd()
        .args(["text", "compare", "a/x.c", "nope.c", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.text_compare.v1");
    assert_eq!(doc["error"]["kind"], "not_found");
    assert!(doc["error"]["spec"].as_str().unwrap().ends_with("nope.c"));
}

#[test]
fn json_goldens() {
    let s = pair();
    let out = compare(&s, &["--json"]);
    let doc = json(&out);
    assert_eq!(
        doc["result"]["display"]["mode"], "mismatches",
        "JSON default display"
    );
    assert_golden("text/json.json", &s.normalise(&stdout(&out)));
    let context = compare(&s, &["--json", "--context", "2"]);
    let doc = json(&context);
    assert_eq!(doc["result"]["display"]["mode"], "context");
    assert_eq!(doc["result"]["display"]["lines"], 2);
    assert_golden("text/json-context-2.json", &s.normalise(&stdout(&context)));
    // Display filters never change the exit code or the statistics (D-03).
    let all = compare(&s, &["--json", "--display", "all"]);
    assert_eq!(json(&all)["result"]["stats"], doc["result"]["stats"]);
}

#[test]
fn every_algorithm() {
    let s = pair();
    for algorithm in [
        "auto",
        "unaligned",
        "standard",
        "myers",
        "patience",
        "histogram",
    ] {
        let out = compare(&s, &["--algorithm", algorithm, "--json"]);
        let doc = json(&out);
        let used = doc["result"]["algorithm"].as_str().unwrap().to_owned();
        if algorithm != "auto" {
            assert_eq!(used, algorithm);
        }
    }
    let bad = s
        .cmd()
        .args(["text", "compare", "a/x.c", "b/x.c", "--algorithm", "diff3"])
        .output()
        .unwrap();
    assert_eq!(code(&bad), 100);
    assert!(stderr(&bad).contains("histogram"), "lists the valid words");
}

#[test]
fn alignment_and_importance_flags_reach_the_rules() {
    let s = pair();
    let out = compare(
        &s,
        &[
            "--json",
            "--skew",
            "12",
            "--no-closeness",
            "--closeness-threshold",
            "700",
            "--never-align",
            "--compare-line-endings",
            "--orphans-unimportant",
            "--unimportant",
            "trailing-ws,case",
            "--unimportant-regex",
            "^#",
            "--unimportant-text",
            "TODO",
        ],
    );
    let rules = &json(&out)["result"]["rules"];
    assert_eq!(rules["alignment"]["skew_tolerance"], 12, "{rules}");
    assert_eq!(rules["alignment"]["closeness"], false);
    assert_eq!(rules["alignment"]["closeness_threshold_permille"], 700);
    assert_eq!(rules["alignment"]["never_align_differences"], true);
    assert_eq!(rules["importance"]["compare_line_endings"], true);
    assert_eq!(rules["importance"]["orphans_always_important"], false);
    assert_eq!(rules["importance"]["trailing_whitespace"], false);
    assert_eq!(rules["importance"]["everything_else_case_sensitive"], false);
    // The patterns keep their command-line order.
    let patterns = rules["importance"]["unimportant"].as_array().unwrap();
    assert_eq!(patterns.len(), 2, "{rules}");
    assert_eq!(patterns[0]["pattern"], "^#");
    assert_eq!(patterns[0]["regex"], true);
    assert_eq!(patterns[1]["pattern"], "TODO");
    assert_eq!(patterns[1]["regex"], false);
}

#[test]
fn rules_file() {
    let s = Sandbox::new();
    s.file("a.txt", "alpha beta\n");
    s.file("b.txt", "alpha    beta\n");
    s.file(
        "ws.toml",
        "schema_version = 1\n[importance]\nembedded_whitespace = false\n",
    );
    for flag in ["--rules-file", "--rules"] {
        let out = s
            .cmd()
            .args(["text", "compare", "a.txt", "b.txt", flag, "ws.toml"])
            .output()
            .unwrap();
        assert_eq!(code(&out), 12, "{flag}: {}", stderr(&out));
    }
    let plain = s
        .cmd()
        .args(["text", "compare", "a.txt", "b.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&plain), 13);
    let ignored = s
        .cmd()
        .args([
            "text", "compare", "a.txt", "b.txt", "--rules", "ws.toml", "--iu",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&ignored), 2);
}

#[test]
fn per_side_encodings() {
    let s = Sandbox::new();
    s.file("a.txt", b"caf\xe9\n");
    s.file("b.txt", "café\n");
    let out = s
        .cmd()
        .args([
            "text",
            "compare",
            "a.txt",
            "b.txt",
            "--left-encoding",
            "windows-1252",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 2, "decoded text is the same: {}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["result"]["left"]["encoding"], "windows-1252", "{doc}");
    assert_eq!(doc["result"]["right"]["encoding"], "UTF-8");
    let wrong = s
        .cmd()
        .args([
            "text",
            "compare",
            "a.txt",
            "b.txt",
            "--left-encoding",
            "koi8-r",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&wrong), 13);
    let both = s
        .cmd()
        .args([
            "text",
            "compare",
            "a.txt",
            "a.txt",
            "--encoding",
            "koi8-r",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&both), 1);
    assert_eq!(json(&both)["result"]["right"]["encoding"], "KOI8-R");
    let bad = s
        .cmd()
        .args([
            "text",
            "compare",
            "a.txt",
            "b.txt",
            "--encoding",
            "no-such-encoding",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&bad), 100);
}

#[test]
fn stdin_takes_the_format_of_the_named_side() {
    let s = Sandbox::new();
    s.file("x.c", "int a; // new\n");
    let out = s
        .cmd()
        .args(["text", "compare", "-", "x.c", "--json"])
        .write_stdin("int a; // old\n")
        .output()
        .unwrap();
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["result"]["format"]["id"], "c-cpp", "D-15: {doc}");
    assert_eq!(doc["result"]["left"]["spec"], "-");
    let unimportant = s
        .cmd()
        .args([
            "text",
            "compare",
            "-",
            "x.c",
            "--unimportant",
            "element:Comment",
        ])
        .write_stdin("int a; // old\n")
        .output()
        .unwrap();
    assert_eq!(code(&unimportant), 12, "{}", stderr(&unimportant));
}

#[test]
fn bad_words_fail_before_comparing() {
    let s = pair();
    for args in [
        ["--report", "sideways"],
        ["--patch-style", "fancy"],
        ["--wrap", "loose"],
        ["--display", "some"],
    ] {
        let out = s
            .cmd()
            .args(["text", "compare", "a/x.c", "b/x.c"])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(code(&out), 100, "{args:?}");
        assert!(out.stdout.is_empty());
    }
    let html_unified = s
        .cmd()
        .args(["text", "compare", "a/x.c", "b/x.c", "--html"])
        .output()
        .unwrap();
    assert_eq!(code(&html_unified), 100, "HTML has no unified layout");
}

#[test]
fn gui_sends_the_comparison_to_the_gui() {
    let s = pair();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::Similar);
    let server = serve(&s, Arc::clone(&gui));
    let out = s
        .cmd()
        .args([
            "text",
            "compare",
            "a/x.c",
            "b/x.c",
            "--gui",
            "--wait",
            "--display",
            "context",
            "--context",
            "5",
            "--iu",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 12, "the view's code: {}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.gui_open.v1");
    assert_eq!(doc["result"]["view_exit_code"], 12);
    let requests = gui.requests();
    let Some(Request::OpenView {
        kind,
        specs,
        options,
        wait,
    }) = requests
        .iter()
        .find(|r| matches!(r, Request::OpenView { .. }))
    else {
        panic!("no OpenView: {requests:?}");
    };
    assert_eq!(kind.as_str(), "text_compare");
    assert!(wait);
    assert!(
        Path::new(&specs[0]).ends_with("a/x.c") && Path::new(&specs[1]).ends_with("b/x.c"),
        "{specs:?}"
    );
    assert_eq!(options.display.as_deref(), Some("context"));
    assert_eq!(options.context_lines, Some(5));
    assert!(options.ignore_unimportant);
    // Without --wait the CLI returns 0 once the view is open.
    let opened = s
        .cmd()
        .args(["text", "compare", "a/x.c", "b/x.c", "--gui"])
        .output()
        .unwrap();
    assert_eq!(code(&opened), 0, "{}", stderr(&opened));
    assert!(opened.stdout.is_empty());
    // --gui never compares locally: a bad word is refused before the GUI is contacted.
    let before = gui.requests().len();
    let bad = s
        .cmd()
        .args([
            "text",
            "compare",
            "a/x.c",
            "b/x.c",
            "--gui",
            "--display",
            "bogus",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&bad), 100);
    assert_eq!(gui.requests().len(), before);
    // --output and the report flags make no sense with --gui.
    let output = s
        .cmd()
        .args([
            "text", "compare", "a/x.c", "b/x.c", "--gui", "--output", "r.txt",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 100);
    server.shutdown().unwrap();
}

#[test]
fn exit_codes() {
    let s = Sandbox::new();
    s.file("same1.txt", "one\ntwo\n");
    s.file("same2.txt", "one\ntwo\n");
    s.file("crlf.txt", "one\r\ntwo\r\n");
    s.file("ws.txt", "one\n two\n");
    s.file("bin1.dat", b"\x00\x01\x02 binary \x00");
    s.file("bin2.dat", b"\x00\x01\x03 binary \x00");
    let run = |args: &[&str]| {
        let out = s
            .cmd()
            .args(["text", "compare"])
            .args(args)
            .output()
            .unwrap();
        (code(&out), stderr(&out))
    };
    assert_eq!(run(&["same1.txt", "same2.txt"]).0, 1, "bytes identical");
    assert_eq!(run(&["same1.txt", "crlf.txt"]).0, 2, "rules-based same");
    assert_eq!(
        run(&["same1.txt", "ws.txt"]).0,
        13,
        "whitespace is important by default"
    );
    let unimportant = ["same1.txt", "ws.txt", "--unimportant", "leading-ws"];
    assert_eq!(run(&unimportant).0, 12);
    let ignored = [
        "same1.txt",
        "ws.txt",
        "--unimportant",
        "leading-ws",
        "--ignore-unimportant",
    ];
    assert_eq!(run(&ignored).0, 2);
    let (binary, err) = run(&["bin1.dat", "bin2.dat"]);
    assert_eq!(
        binary, 11,
        "binary content never gets a rules verdict (D-12): {err}"
    );
    assert_eq!(run(&["bin1.dat", "bin1.dat"]).0, 1);
}
