//! `hikasa quick` end to end (contract §6.3 `cli_quick.rs`, §2.21, §3.1; D-01, D-12, D-15).

mod common;

use common::{Sandbox, assert_golden, code, json, stderr, stdout};

/// A C pair whose second line differs in a number (an important difference).
fn c_pair(s: &Sandbox) {
    s.file("a/x.c", "int x = 1;\nint y = 2;\n");
    s.file("b/x.c", "int x = 1;\nint y = 3;\n");
}

#[test]
fn identical_files_are_binary_same() {
    let s = Sandbox::new();
    s.file("a.txt", "same\n");
    s.file("b.txt", "same\n");
    let out = s.cmd().args(["quick", "a.txt", "b.txt"]).output().unwrap();
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    assert!(out.stdout.is_empty(), "quick prints nothing on stdout");
}

#[test]
fn line_endings_only_is_rules_same() {
    let s = Sandbox::new();
    s.file("a.txt", "one\r\ntwo\r\n");
    s.file("b.txt", "one\ntwo\n");
    let out = s.cmd().args(["quick", "a.txt", "b.txt"]).output().unwrap();
    assert_eq!(code(&out), 2, "{}", stderr(&out));
}

#[test]
fn different_binary_content_is_11() {
    let s = Sandbox::new();
    s.file("a.bin", b"\x00\x01\x02\x03binary\x00\xff");
    s.file("b.bin", b"\x00\x01\x02\x04binary\x00\xff");
    let out = s.cmd().args(["quick", "a.bin", "b.bin"]).output().unwrap();
    assert_eq!(code(&out), 11, "{}", stderr(&out));
    let same = s.cmd().args(["quick", "a.bin", "a.bin"]).output().unwrap();
    assert_eq!(code(&same), 1);
}

#[test]
fn important_and_unimportant_differences() {
    let s = Sandbox::new();
    c_pair(&s);
    let out = s.cmd().args(["quick", "a/x.c", "b/x.c"]).output().unwrap();
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    // A whitespace-only change: important by default (D-04), unimportant with `--unimportant whitespace`.
    s.file("a/w.c", "int x = 1;\n");
    s.file("b/w.c", "int  x =  1;\n");
    let important = s.cmd().args(["quick", "a/w.c", "b/w.c"]).output().unwrap();
    assert_eq!(code(&important), 13);
    let unimportant = s
        .cmd()
        .args(["quick", "a/w.c", "b/w.c", "--unimportant", "whitespace"])
        .output()
        .unwrap();
    assert_eq!(code(&unimportant), 12, "{}", stderr(&unimportant));
    // --ignore-unimportant (alias --iu) turns 12 into 2.
    for flag in ["--ignore-unimportant", "--iu"] {
        let ignored = s
            .cmd()
            .args([
                "quick",
                "a/w.c",
                "b/w.c",
                "--unimportant",
                "whitespace",
                flag,
            ])
            .output()
            .unwrap();
        assert_eq!(code(&ignored), 2, "{flag}");
    }
}

#[test]
fn unimportant_patterns_and_rules_files() {
    let s = Sandbox::new();
    s.file("a.txt", "id 41 ok\n");
    s.file("b.txt", "id 42 ok\n");
    let plain = s
        .cmd()
        .args(["quick", "a.txt", "b.txt", "--unimportant-regex", "[0-9]+"])
        .output()
        .unwrap();
    assert_eq!(code(&plain), 12, "{}", stderr(&plain));
    let text = s
        .cmd()
        .args([
            "quick",
            "a.txt",
            "b.txt",
            "--unimportant-text",
            "41",
            "--unimportant-text",
            "42",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&text), 12, "{}", stderr(&text));
    s.file(
        "ws.toml",
        "schema_version = 1\n[importance]\nembedded_whitespace = false\n",
    );
    s.file("c.txt", "a b\n");
    s.file("d.txt", "a   b\n");
    let rules = s
        .cmd()
        .args(["quick", "c.txt", "d.txt", "--rules-file", "ws.toml"])
        .output()
        .unwrap();
    assert_eq!(code(&rules), 12, "{}", stderr(&rules));
    s.file("bad.toml", "schema_version = 1\n[importance\n");
    let bad = s
        .cmd()
        .args([
            "quick",
            "c.txt",
            "d.txt",
            "--rules-file",
            "bad.toml",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&bad), 100);
    let doc = json(&bad);
    assert!(
        doc["error"]["spec"].as_str().unwrap().ends_with("bad.toml"),
        "{doc}"
    );
}

#[test]
fn size_crc_and_binary_methods() {
    let s = Sandbox::new();
    s.file("a.txt", "abcd\n");
    s.file("b.txt", "abce\n");
    s.file("c.txt", "abcd\n");
    s.file("d.txt", "abcdef\n");
    let run = |args: &[&str]| code(&s.cmd().arg("quick").args(args).output().unwrap());
    assert_eq!(run(&["--size", "a.txt", "b.txt"]), 1, "same size");
    assert_eq!(run(&["--size", "a.txt", "d.txt"]), 11);
    for method in ["--crc", "--binary"] {
        assert_eq!(run(&[method, "a.txt", "c.txt"]), 1, "{method}");
        assert_eq!(run(&[method, "a.txt", "b.txt"]), 11, "{method}");
    }
    assert_eq!(run(&["--rules", "a.txt", "c.txt"]), 1);
    // The methods exclude each other (a usage error is 100, never 2).
    assert_eq!(run(&["--size", "--crc", "a.txt", "b.txt"]), 100);
}

#[test]
fn errors_are_100() {
    let s = Sandbox::new();
    s.file("a.txt", "x\n");
    let missing = s
        .cmd()
        .args(["quick", "a.txt", "missing.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&missing), 100);
    assert!(
        stderr(&missing).contains("missing.txt"),
        "{}",
        stderr(&missing)
    );
    let unsupported = s
        .cmd()
        .args(["quick", "a.txt", "sftp://host/x.txt", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&unsupported), 100);
    let doc = json(&unsupported);
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["exit_code"], 100);
    assert_eq!(doc["error"]["kind"], "unsupported", "{doc}");
    let folder = s.cmd().args(["quick", ".", "a.txt"]).output().unwrap();
    assert_eq!(code(&folder), 100);
}

#[test]
fn stdin_uses_the_name_hint() {
    let s = Sandbox::new();
    s.file("b.c", "int x; /* two */\n");
    // A comment-only change is unimportant only when the C grammar tokenises the comment (D-15).
    let out = s
        .cmd()
        .args([
            "quick",
            "-",
            "b.c",
            "--stdin-name",
            "x.c",
            "--unimportant",
            "element:Comment",
            "--json",
        ])
        .write_stdin("int x; /* one */\n")
        .output()
        .unwrap();
    assert_eq!(code(&out), 12, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["result"]["format"]["id"], "c-cpp", "{doc}");
    assert_eq!(doc["result"]["left"], "-");
}

#[test]
fn json_golden() {
    let s = Sandbox::new();
    c_pair(&s);
    let out = s
        .cmd()
        .args(["quick", "a/x.c", "b/x.c", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 13);
    let doc = json(&out);
    assert_eq!(doc["exit_code"], 13);
    assert_golden("quick/json.json", &s.normalise(&stdout(&out)));
}

/// Regression (wave-5 review, maintainer decision 2): `--crc`/`--binary` on a file that cannot be read report the
/// error on stderr (also with `--quiet`; the command used to exit 100 silently), and the JSON document says
/// `ok: false` with the `error`, next to the result that carries it (§4.2, §4.3.1).
#[cfg(unix)]
#[test]
fn an_unreadable_file_is_an_error_on_stderr() {
    let s = Sandbox::new();
    s.file("a.c", "int a;\n");
    let unread = s.file("unread.c", "int b;\n");
    common::set_mode(&unread, 0o000);
    if std::fs::read(&unread).is_ok() {
        eprintln!("skipped: the file stays readable (root)");
        return;
    }
    for method in ["--crc", "--binary"] {
        for extra in [&[][..], &["--quiet"]] {
            let out = s
                .cmd()
                .args(["quick", "a.c", "unread.c", method])
                .args(extra)
                .output()
                .unwrap();
            assert_eq!(code(&out), 100, "{method} {extra:?}");
            assert!(out.stdout.is_empty());
            let err = stderr(&out);
            assert!(
                err.contains("error:") && err.contains("unread.c"),
                "{method} {extra:?}: {err:?}"
            );
            assert_eq!(err.lines().count(), 1, "{err}");
        }
        let out = s
            .cmd()
            .args(["quick", "a.c", "unread.c", method, "--json"])
            .output()
            .unwrap();
        assert_eq!(code(&out), 100);
        let doc = json(&out);
        assert_eq!(doc["ok"], false, "{doc}");
        assert_eq!(doc["exit_code"], 100);
        assert_eq!(doc["error"]["kind"], "permission_denied", "{doc}");
        assert_eq!(doc["result"]["result"]["result"], "error");
        assert_eq!(doc["result"]["result"]["error"], doc["error"]);
        assert!(stderr(&out).contains("unread.c"));
    }
    common::set_mode(&unread, 0o644);
}
