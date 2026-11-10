//! `hikasa text find` end to end (contract §6.10 A10.24, §1.10.19, §1.10.24; D-285): the human lines (golden),
//! `--files-with-matches`, `--count`, `--verbose`'s skipped files, the JSON golden (`hikasa.text_find.v1`), exit 0
//! with and without hits, 100 for a bad pattern and a missing root; `--replace` with `--dry-run` (the plan golden, human
//! and JSON), `--yes` (files rewritten, unchanged lines byte-exact, backups), no terminal without `--yes` (exit 100,
//! nothing written), `--confirm no-to-all`, `--no-backup`; the exactness rule: the JSON hits equal the Service's and the
//! human lines are the JSON's. A file changed between the plan and the write failing its item is A10.24's Service row
//! (`core/hikasa-session/tests/text_find_files.rs`: a process cannot be paused between the two here). A10.32: a file
//! whose name is not UTF-8 (Linux).

mod common;

use std::path::Path;

use common::{Sandbox, assert_golden, code, json, stderr, stdout};

/// Runs `text find <args>` in the sandbox (stdin is not a terminal).
fn find(s: &Sandbox, args: &[&str]) -> std::process::Output {
    s.cmd().args(["text", "find"]).args(args).output().unwrap()
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

/// The tree of the goldens.
fn tree(s: &Sandbox) {
    s.file(
        "t/a.c",
        b"int old_a = 1;\r\nint b = old_a;\r\nint old_ab;\r\n",
    );
    s.file("t/b.txt", b"Old news\n\tsee old_x here\n");
    s.file("t/bin.dat", b"old\0\0\x01binary");
    s.file("t/.hidden.txt", b"old hidden\n");
    s.file("t/sub/c.txt", b"nothing\n");
    s.file("t/sub/d.txt", b"an old line, an old word\n");
    s.settle("t");
}

#[test]
fn a10_24_human_lines_files_and_counts() {
    let s = Sandbox::new();
    tree(&s);
    let out = find(&s, &["old", "t", "--recursive"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_golden("text_find/search.txt", &s.normalise(&stdout(&out)));
    // Not recursive: the root's files only; the skipped binary file named on stderr with --verbose.
    let out = s
        .cmd()
        .args(["--verbose", "text", "find", "old", "t", "--case"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0);
    let lines: Vec<String> = s
        .normalise(&stdout(&out))
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        lines,
        [
            "<TMP>/t/a.c:1:5: int old_a = 1;",
            "<TMP>/t/a.c:2:9: int b = old_a;",
            "<TMP>/t/a.c:3:5: int old_ab;",
            "<TMP>/t/b.txt:2:6: \tsee old_x here",
        ]
    );
    assert!(
        s.normalise(&stderr(&out))
            .contains("skipped (binary): <TMP>/t/bin.dat"),
        "{}",
        stderr(&out)
    );
    // -l and -c: each file with a hit once; whole words.
    let out = find(&s, &["old", "t", "-r", "-l", "--include-hidden"]);
    assert_eq!(
        s.normalise(&stdout(&out)),
        "<TMP>/t/.hidden.txt\n<TMP>/t/a.c\n<TMP>/t/b.txt\n<TMP>/t/sub/d.txt\n"
    );
    let out = find(&s, &["old", "t", "-r", "-c", "--word"]);
    assert_eq!(
        s.normalise(&stdout(&out)),
        "<TMP>/t/b.txt:1\n<TMP>/t/sub/d.txt:2\n"
    );
    // A mask with an excluded folder.
    let out = find(&s, &["old", "t", "-r", "-l", "--mask", "*.txt;-sub/"]);
    assert_eq!(s.normalise(&stdout(&out)), "<TMP>/t/b.txt\n");
}

#[test]
fn a10_24_exit_codes() {
    let s = Sandbox::new();
    tree(&s);
    // No hit is exit 0 too.
    let out = find(&s, &["no such words", "t", "-r"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).is_empty());
    // A bad pattern, a missing root, a line-break pattern: 100, nothing on stdout.
    for args in [
        &["(", "t", "--regex"][..],
        &["old", "t", "missing"],
        &[r"a\n", "t", "--regex"],
        &["old", "t", "--max-matches", "0"],
        &["old", "t", "--archives", "sometimes"],
    ] {
        let out = find(&s, args);
        assert_eq!(code(&out), 100, "{args:?}");
        assert!(stdout(&out).is_empty(), "{args:?}");
        assert!(
            stderr(&out).contains("error:"),
            "{args:?}: {}",
            stderr(&out)
        );
    }
    let out = s
        .cmd()
        .args(["--json", "text", "find", "old", "missing"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    let doc = json(&out);
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["schema"], "hikasa.text_find.v1");
    assert_eq!(doc["error"]["kind"], "not_found");
}

#[test]
fn a10_24_the_json_is_golden_and_the_human_lines_are_its_hits() {
    let s = Sandbox::new();
    tree(&s);
    s.file("t/z.zip", b"PK\x03\x04 not really a zip");
    s.settle("t");
    let out = s
        .cmd()
        .args([
            "--json",
            "text",
            "find",
            "old",
            "t",
            "-r",
            "--max-matches",
            "6",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_golden("text_find/search.json", &s.normalise(&stdout(&out)));
    let doc = json(&out);
    assert_eq!(doc["result"]["limit_reached"], true);
    assert_eq!(doc["warnings"][0]["code"], "find_limit_reached");
    // The human stream prints the hits the document holds, in its order.
    let human = find(&s, &["old", "t", "-r", "--max-matches", "6"]);
    let mut expected = String::new();
    for file in doc["result"]["files"].as_array().unwrap() {
        for hit in file["hits"].as_array().unwrap() {
            expected.push_str(&format!(
                "{}:{}:{}: {}\n",
                file["spec"].as_str().unwrap(),
                hit["line"],
                hit["column"],
                hit["preview"].as_str().unwrap()
            ));
        }
    }
    assert_eq!(stdout(&human), expected);
    assert!(stderr(&human).contains("warning: the search stopped at 6 hits"));
}

#[test]
fn a10_24_replace_dry_run_is_golden_and_writes_nothing() {
    let s = Sandbox::new();
    tree(&s);
    let before = read(&s.join("t/a.c"));
    let out = find(
        &s,
        &[
            r"old_(\w)\b",
            "t",
            "-r",
            "--regex",
            "--case",
            "--replace",
            "new_${1}",
            "--dry-run",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_golden("text_find/replace-dry-run.txt", &s.normalise(&stdout(&out)));
    let out = s
        .cmd()
        .args([
            "--json",
            "text",
            "find",
            r"old_(\w)\b",
            "t",
            "-r",
            "--regex",
            "--case",
        ])
        .args(["--replace", "new_${1}", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_golden(
        "text_find/replace-dry-run.json",
        &s.normalise(&stdout(&out)),
    );
    assert_eq!(read(&s.join("t/a.c")), before);
    assert!(!s.join("t/a.c.bak").exists());
}

#[test]
fn a10_24_replace_asks_without_a_terminal_and_writes_with_yes() {
    let s = Sandbox::new();
    tree(&s);
    let a = s.join("t/a.c");
    let b = s.join("t/b.txt");
    let before = (read(&a), read(&b));
    // No terminal, no --yes: the operation question refuses the run; nothing written.
    let args = [
        r"old_(\w)\b",
        "t",
        "-r",
        "--regex",
        "--case",
        "--replace",
        "new_${1}",
    ];
    let out = find(&s, &args);
    assert_eq!(code(&out), 100);
    let err = stderr(&out);
    assert!(
        err.contains("1 questions need an answer: pass --yes"),
        "{err}"
    );
    assert!(err.contains("replace 1"), "{err}");
    assert_eq!((read(&a), read(&b)), before);
    // --yes: one plan, unchanged lines and terminators byte for byte, backups (back_up_before_save).
    let out = s
        .cmd()
        .args(["text", "find"])
        .args(args)
        .arg("--yes")
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        read(&a),
        b"int new_a = 1;\r\nint b = new_a;\r\nint old_ab;\r\n"
    );
    assert_eq!(read(&b), b"Old news\n\tsee new_x here\n");
    assert_eq!(read(&s.join("t/a.c.bak")), before.0);
    assert_eq!(read(&s.join("t/b.txt.bak")), before.1);
    let text = s.normalise(&stdout(&out));
    assert!(
        text.contains("replaced 2 occurrences: <TMP>/t/a.c\n"),
        "{text}"
    );
    assert!(
        text.contains("replaced 1 occurrences: <TMP>/t/b.txt\n"),
        "{text}"
    );
    // --confirm no-to-all answers the operation; --no-backup.
    let out = find(
        &s,
        &[
            "new_",
            "t",
            "-r",
            "--case",
            "--replace",
            "NEW_",
            "--confirm",
            "no-to-all",
            "--no-backup",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&b), b"Old news\n\tsee NEW_x here\n");
    assert_eq!(read(&s.join("t/b.txt.bak")), before.1);
    // The JSON of a run: the plan as executed and each file's result.
    let out = s
        .cmd()
        .args([
            "--json",
            "text",
            "find",
            "NEW",
            "t/b.txt",
            "--case",
            "--replace",
            "x",
            "--yes",
            "--no-backup",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    let replace = &doc["result"]["replace"];
    assert_eq!(replace["dry_run"], false);
    assert_eq!(replace["results"][0]["status"], "done");
    assert_eq!(replace["results"][0]["replaced"], 1);
    assert_eq!(replace["plan"][0]["action"], "write");
    assert_eq!(replace["plan"][0]["status"], "done");
    assert_eq!(replace["summary"]["done"], 1);
    assert_eq!(read(&b), b"Old news\n\tsee x_x here\n");
}

#[test]
fn a10_24_a_failed_file_is_exit_100_and_the_others_are_written() {
    let s = Sandbox::new();
    s.file("t/a.txt", b"foo\n");
    // A file the replacement cannot be written to in its encoding (windows-1252, no terminal: refused).
    s.file("t/w.txt", b"caf\xe9 foo\n");
    s.settle("t");
    let out = s
        .cmd()
        .args([
            "--json",
            "text",
            "find",
            "foo",
            "t",
            "--replace",
            "\u{3a9}",
            "--yes",
            "--no-backup",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["result"]["replace"]["results"][0]["status"], "done");
    assert_eq!(doc["result"]["replace"]["results"][1]["status"], "failed");
    assert_eq!(read(&s.join("t/a.txt")), "\u{3a9}\n".as_bytes());
    assert_eq!(read(&s.join("t/w.txt")), b"caf\xe9 foo\n");
    // --unmappable replace writes `?`.
    let out = find(
        &s,
        &[
            "foo",
            "t/w.txt",
            "--replace",
            "\u{3a9}",
            "--yes",
            "--no-backup",
            "--unmappable",
            "replace",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&s.join("t/w.txt")), b"caf\xe9 ?\n");
}

/// A10.32: a file whose name is not UTF-8 is found and rewritten (Linux).
#[cfg(target_os = "linux")]
#[test]
fn a10_32_a_non_utf8_file_name() {
    use std::os::unix::ffi::OsStrExt;
    let s = Sandbox::new();
    let dir = s.dir("t");
    let odd = dir.join(std::ffi::OsStr::from_bytes(b"n\xffme.txt"));
    std::fs::write(&odd, b"a foo\n").unwrap();
    let out = find(&s, &["foo", "t", "-l"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(stdout(&out).lines().count(), 1);
    let out = find(
        &s,
        &["foo", "t", "--replace", "bar", "--yes", "--no-backup"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&odd), b"a bar\n");
}

/// A line's control characters are escaped (a file cannot move the cursor or recolour the terminal); tabs stay.
#[test]
fn a10_24_previews_are_escaped() {
    let s = Sandbox::new();
    s.file("t/a.txt", b"\x1b[31mred foo\tx\x07\n");
    let out = find(&s, &["foo", "t"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        !text.contains('\u{1b}') && !text.contains('\u{7}'),
        "{text:?}"
    );
    assert!(text.contains("red foo\tx"), "{text:?}");
}

/// A reader that goes away stops the search: the command ends, exit 0 and no error (D-35).
#[test]
fn a10_24_a_reader_that_goes_away_ends_the_search_with_exit_0() {
    use std::io::{BufRead, BufReader};
    let s = Sandbox::new();
    let text: String = (0..200_000).map(|k| format!("{k} foo\n")).collect();
    s.file("t/a.txt", text.as_bytes());
    s.file("t/b.txt", text.as_bytes());
    let mut child = std::process::Command::new(common::hikasa_exe())
        .current_dir(s.path())
        .env("HIKASA_IPC_ENDPOINT", s.socket())
        .env("HIKASA_CLIPBOARD_FILE", s.join("no-clipboard"))
        .args(["--config-dir"])
        .arg(s.config())
        .args([
            "text",
            "find",
            "foo",
            "t",
            "--max-matches-per-file",
            "1000000",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut first = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    assert!(first.ends_with(":1:3: 0 foo\n"), "{first:?}");
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains("error"));
}
