//! `hikasa text convert` end to end (contract §6.10 A10.19, §1.10.15, §1.10.24; D-280): standard output, `--output`,
//! `--in-place` of three files (one plan, backups), the `--dry-run` JSON golden (`hikasa.text_convert.v1`), `no_change`
//! files not rewritten (their time kept), `--encoding UTF-16LE --bom`, `--unmappable`, a Sorted format refused, the
//! exit codes 0 and 100, the questions without a terminal (exit 100 with their counts, nothing written), and the
//! exactness rule: the bytes equal those of a view's `transform` and save of the same file (A10.19's last clause).
//! A10.32: a file whose name is not UTF-8 (Linux).

mod common;

use std::path::Path;
use std::sync::Arc;

use common::{MTIME, Sandbox, assert_golden, code, json, stderr, stdout};

/// Runs `text convert <args>` in the sandbox (stdin is not a terminal).
fn convert(s: &Sandbox, args: &[&str]) -> std::process::Output {
    s.cmd()
        .args(["text", "convert"])
        .args(args)
        .output()
        .unwrap()
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

/// The modification time of `path` in Unix seconds.
fn mtime(path: &Path) -> i64 {
    filetime::FileTime::from_last_modification_time(&std::fs::metadata(path).unwrap())
        .unix_seconds()
}

#[test]
fn a10_19_standard_output_gets_the_converted_bytes() {
    let s = Sandbox::new();
    let a = s.file("a.txt", b"a  \r\n\tb\r\nc\n");
    let out = convert(&s, &["a.txt", "--trim-trailing", "--line-endings", "lf"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(out.stdout, b"a\n\tb\nc\n");
    assert!(
        stderr(&out).contains("converted 2 lines: "),
        "{}",
        stderr(&out)
    );
    // The file is untouched; no backup.
    assert_eq!(read(&a), b"a  \r\n\tb\r\nc\n");
    assert!(!s.join("a.txt.bak").exists());
    // Standard input to standard output.
    let out = s
        .cmd()
        .args([
            "text",
            "convert",
            "-",
            "--tabs-to-spaces",
            "--tab-size",
            "2",
        ])
        .write_stdin("\tx\n")
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(out.stdout, b"  x\n");
    // --json needs a file target or a dry run.
    let out = s
        .cmd()
        .args(["--json", "text", "convert", "a.txt", "--trim-trailing"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    let doc = json(&out);
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["schema"], "hikasa.text_convert.v1");
}

#[test]
fn a10_19_output_writes_one_file_and_asks_before_replacing_one() {
    let s = Sandbox::new();
    s.file("a.txt", b"x \ny\n");
    let out = convert(&s, &["a.txt", "--trim-trailing", "--output", "o.txt"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&s.join("o.txt")), b"x\ny\n");
    assert_eq!(read(&s.join("a.txt")), b"x \ny\n");
    assert!(
        stdout(&out).contains("converted 1 lines: "),
        "{}",
        stdout(&out)
    );
    // An existing output: without a terminal the question refuses the run, nothing written.
    s.file("o.txt", b"old\n");
    let out = convert(&s, &["a.txt", "--trim-trailing", "--output", "o.txt"]);
    assert_eq!(code(&out), 100);
    let err = stderr(&out);
    assert!(
        err.contains("1 questions need an answer: pass --yes"),
        "{err}"
    );
    assert!(err.contains("replace_existing 1"), "{err}");
    assert_eq!(read(&s.join("o.txt")), b"old\n");
    // --yes: replaced, the replaced file backed up.
    let out = convert(
        &s,
        &["a.txt", "--trim-trailing", "--output", "o.txt", "--yes"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&s.join("o.txt")), b"x\ny\n");
    assert_eq!(read(&s.join("o.txt.bak")), b"old\n");
    // --output takes one file.
    s.file("b.txt", b"b\n");
    let out = convert(
        &s,
        &["a.txt", "b.txt", "--trim-trailing", "--output", "o.txt"],
    );
    assert_eq!(code(&out), 100);
    assert!(stderr(&out).contains("--output takes one input file"));
}

#[test]
fn a10_19_in_place_rewrites_three_files_in_one_plan_with_backups() {
    let s = Sandbox::new();
    let files = [
        s.file("one.txt", b"1  \n"),
        s.file("two.txt", b"2\t\r\n"),
        s.file("three.txt", b"3 \n\n"),
    ];
    let out = s
        .cmd()
        .args([
            "--json",
            "text",
            "convert",
            "one.txt",
            "two.txt",
            "three.txt",
        ])
        .args(["--trim-trailing", "--in-place"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    let result = &doc["result"];
    assert_eq!(result["target"], "in_place");
    assert_eq!(result["plan"].as_array().unwrap().len(), 3);
    for (i, item) in result["plan"].as_array().unwrap().iter().enumerate() {
        assert_eq!(item["action"], "write");
        assert_eq!(item["status"], "done");
        assert!(
            item["backup"].as_str().unwrap().ends_with(".txt.bak"),
            "{i}: {item}"
        );
    }
    assert_eq!(result["summary"]["done"], 3);
    assert_eq!(result["summary"]["converted_lines"], 3);
    assert_eq!(read(&files[0]), b"1\n");
    assert_eq!(read(&files[1]), b"2\r\n");
    assert_eq!(read(&files[2]), b"3\n\n");
    assert_eq!(read(&s.join("one.txt.bak")), b"1  \n");
    assert_eq!(read(&s.join("two.txt.bak")), b"2\t\r\n");
    assert_eq!(read(&s.join("three.txt.bak")), b"3 \n\n");
    // --no-backup: none.
    s.file("four.txt", b"4 \n");
    let out = convert(
        &s,
        &["four.txt", "--trim-trailing", "--in-place", "--no-backup"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!s.join("four.txt.bak").exists());
}

#[test]
fn a10_19_the_dry_run_json_is_golden() {
    let s = Sandbox::new();
    s.file("a.txt", b"one  \r\n\ttwo\r\nthree\r\n");
    s.file("b.txt", b"clean\n");
    let out = s
        .cmd()
        .args(["--json", "text", "convert", "a.txt", "b.txt", "missing.txt"])
        .args([
            "--trim-trailing",
            "--tabs-to-spaces",
            "--line-endings",
            "lf",
        ])
        .args(["--in-place", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert_golden("text_convert/dry-run.json", &s.normalise(&stdout(&out)));
    // Nothing written.
    assert_eq!(read(&s.join("a.txt")), b"one  \r\n\ttwo\r\nthree\r\n");
    assert!(!s.join("a.txt.bak").exists());
    // The human dry run, with the plan's lines under --verbose.
    let out = s
        .cmd()
        .args(["--verbose", "text", "convert", "a.txt", "b.txt"])
        .args(["--trim-trailing", "--in-place", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_golden("text_convert/dry-run.txt", &s.normalise(&stdout(&out)));
}

#[test]
fn a10_19_a_file_with_nothing_to_change_is_not_rewritten() {
    let s = Sandbox::new();
    let clean = s.file("clean.txt", b"a\nb\n");
    let dirty = s.file("dirty.txt", b"a \n");
    let out = s
        .cmd()
        .args(["--json", "text", "convert", "clean.txt", "dirty.txt"])
        .args(["--trim-trailing", "--line-endings", "lf", "--in-place"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    let files = doc["result"]["files"].as_array().unwrap();
    assert_eq!(files[0]["status"], "skipped");
    assert_eq!(files[0]["skip"], "no_change");
    assert_eq!(files[0]["converted_lines"], 0);
    assert_eq!(doc["result"]["plan"][0]["action"], "skip");
    assert_eq!(doc["result"]["plan"][0]["reason"], "no_change");
    assert_eq!(files[1]["status"], "done");
    // The file's time and bytes are as they were, and no backup was made.
    assert_eq!(mtime(&clean), MTIME);
    assert_eq!(read(&clean), b"a\nb\n");
    assert!(!s.join("clean.txt.bak").exists());
    assert_ne!(mtime(&dirty), MTIME);
    // The human line.
    let out = convert(&s, &["clean.txt", "--trim-trailing", "--in-place"]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).starts_with("no change: "), "{}", stdout(&out));
}

/// W10b note 54: `no_change` skips only `--in-place`. Standard output gets the file's bytes as they are (malformed
/// bytes, BOM and terminators included; from a pipe too, and with `--lines` past the end), and `--output` gets a copy,
/// asking before it replaces a file and creating a missing one.
#[test]
fn a10_19_nothing_to_change_still_goes_to_stdout_and_output() {
    let s = Sandbox::new();
    let bytes: &[u8] = b"\xEF\xBB\xBFa\nb\xFF\r\nc";
    let a = s.file("a.txt", bytes);
    let out = convert(&s, &["a.txt", "--line-endings", "lf", "--lines", "1-1"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(out.stdout, bytes);
    let out = convert(&s, &["a.txt", "--trim-trailing"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(out.stdout, bytes);
    // `--lines` past the end: nothing to convert, the bytes still come.
    let out = convert(&s, &["a.txt", "--trim-trailing", "--lines", "5-9"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(out.stdout, bytes);
    // A pipe: its bytes as read.
    let out = s
        .cmd()
        .args(["text", "convert", "-", "--trim-trailing"])
        .write_stdin(bytes)
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(out.stdout, bytes);
    // `--output`: an existing file is asked about (refused without a terminal), then replaced with `--yes`.
    s.file("o.txt", b"stale\n");
    let out = convert(&s, &["a.txt", "--trim-trailing", "--output", "o.txt"]);
    assert_eq!(code(&out), 100);
    assert!(
        stderr(&out).contains("replace_existing 1"),
        "{}",
        stderr(&out)
    );
    assert_eq!(read(&s.join("o.txt")), b"stale\n");
    let out = convert(
        &s,
        &["a.txt", "--trim-trailing", "--output", "o.txt", "--yes"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&s.join("o.txt")), bytes);
    // A missing one is created.
    let out = convert(&s, &["a.txt", "--trim-trailing", "--output", "new.txt"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&s.join("new.txt")), bytes);
    // The source is never touched.
    assert_eq!(read(&a), bytes);
    assert_eq!(mtime(&a), MTIME);
}

/// The BOM stays when only a content conversion runs, and a BOM-only change (`--no-bom`, `--bom`) is a change that is
/// written in place (W10b note 66's mutation table).
#[test]
fn a10_19_the_bom_stays_unless_asked_and_its_change_is_written() {
    let s = Sandbox::new();
    let a = s.file("a.txt", b"\xEF\xBB\xBFx \ny\n");
    let out = convert(&s, &["a.txt", "--trim-trailing"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(out.stdout, b"\xEF\xBB\xBFx\ny\n");
    let out = convert(&s, &["a.txt", "--no-bom", "--in-place", "--no-backup"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stdout(&out).starts_with("converted 0 lines: "),
        "{}",
        stdout(&out)
    );
    assert_eq!(read(&a), b"x \ny\n");
    let out = convert(&s, &["a.txt", "--bom", "--in-place", "--no-backup"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&a), b"\xEF\xBB\xBFx \ny\n");
}

/// W10b note 65: `--in-place` refuses what is not a regular file before reading it, so a FIFO without a writer never
/// blocks the command (Unix).
#[cfg(unix)]
#[test]
fn a10_19_in_place_refuses_a_fifo_without_reading_it() {
    let s = Sandbox::new();
    let fifo = s.join("pipe");
    if !std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .is_ok_and(|status| status.success())
    {
        eprintln!("skipped: mkfifo is not available");
        return;
    }
    let out = s
        .cmd()
        .args(["text", "convert", "pipe", "--trim-trailing", "--in-place"])
        .timeout(std::time::Duration::from_secs(60))
        .output()
        .unwrap();
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("is not a regular file"),
        "{}",
        stdout(&out)
    );
    assert!(!s.join("pipe.bak").exists());
}

#[test]
fn a10_19_utf16le_with_a_bom() {
    let s = Sandbox::new();
    s.file("a.txt", "é\nb\n".as_bytes());
    let out = convert(
        &s,
        &[
            "a.txt",
            "--encoding",
            "UTF-16LE",
            "--bom",
            "--output",
            "u.txt",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let mut expected = vec![0xFF, 0xFE];
    for unit in "é\nb\n".encode_utf16() {
        expected.extend_from_slice(&unit.to_le_bytes());
    }
    assert_eq!(read(&s.join("u.txt")), expected);
    // Back to UTF-8 without a BOM: the original bytes.
    let out = convert(
        &s,
        &[
            "u.txt",
            "--encoding",
            "UTF-8",
            "--no-bom",
            "--output",
            "back.txt",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&s.join("back.txt")), "é\nb\n".as_bytes());
    // --bom for an encoding without one fails the file.
    let out = convert(
        &s,
        &[
            "a.txt",
            "--encoding",
            "windows-1252",
            "--bom",
            "--output",
            "w.txt",
        ],
    );
    assert_eq!(code(&out), 100);
    assert!(
        stdout(&out).contains("has no byte-order mark"),
        "{}",
        stdout(&out)
    );
    assert!(!s.join("w.txt").exists());
}

#[test]
fn a10_19_unmappable_characters_refuse_by_default_and_are_replaced_on_request() {
    let s = Sandbox::new();
    s.file("a.txt", "caf\u{e9} \u{65e5}\nok\n".as_bytes());
    // Without a terminal the default is refuse: the file fails.
    let out = convert(
        &s,
        &["a.txt", "--encoding", "windows-1252", "--output", "w.txt"],
    );
    assert_eq!(code(&out), 100);
    assert!(stdout(&out).contains("U+65E5"), "{}", stdout(&out));
    assert!(!s.join("w.txt").exists());
    // --unmappable replace: '?' and the warning; never a numeric reference.
    let out = convert(
        &s,
        &[
            "a.txt",
            "--encoding",
            "windows-1252",
            "--unmappable",
            "replace",
            "--output",
            "w.txt",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&s.join("w.txt")), b"caf\xe9 ?\nok\n");
    assert!(stderr(&out).contains("written as '?'"), "{}", stderr(&out));
    // Standard output refuses as well.
    let out = convert(&s, &["a.txt", "--encoding", "windows-1252"]);
    assert_eq!(code(&out), 100);
    assert!(out.stdout.is_empty());
    let out = convert(
        &s,
        &[
            "a.txt",
            "--encoding",
            "windows-1252",
            "--unmappable",
            "refuse",
        ],
    );
    assert_eq!(code(&out), 100);
    let out = convert(
        &s,
        &[
            "a.txt",
            "--encoding",
            "windows-1252",
            "--unmappable",
            "bogus",
        ],
    );
    assert_eq!(code(&out), 100);
    assert!(stderr(&out).contains("--unmappable"), "{}", stderr(&out));
}

#[test]
fn a10_19_a_sorted_format_and_binary_content_are_refused() {
    let s = Sandbox::new();
    s.file("a.txt", b"b\na\n");
    s.file("bin.dat", b"\x00\x01\x02\x03bin\n");
    let out = s
        .cmd()
        .args(["--json", "text", "convert", "a.txt", "--format", "sorted"])
        .args(["--trim-trailing", "--in-place"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    let doc = json(&out);
    assert_eq!(doc["ok"], true);
    let file = &doc["result"]["files"][0];
    assert_eq!(file["status"], "failed");
    assert_eq!(file["error"]["kind"], "permission_denied");
    assert_eq!(read(&s.join("a.txt")), b"b\na\n");
    let out = convert(&s, &["bin.dat", "--trim-trailing", "--in-place"]);
    assert_eq!(code(&out), 100);
    assert!(stdout(&out).contains("binary"), "{}", stdout(&out));
}

#[test]
fn a10_19_exit_codes() {
    let s = Sandbox::new();
    s.file("a.txt", b"a \n");
    s.file("b.txt", b"b \n");
    // Nothing to convert: a refused request, ok false.
    let out = s
        .cmd()
        .args(["--json", "text", "convert", "a.txt", "--in-place"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    let doc = json(&out);
    assert_eq!(doc["ok"], false);
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("nothing to convert")
    );
    // Several files to standard output.
    let out = convert(&s, &["a.txt", "b.txt", "--trim-trailing"]);
    assert_eq!(code(&out), 100);
    assert!(stderr(&out).contains("standard output"), "{}", stderr(&out));
    // A missing file fails its item; the others are written: ok true, exit 100.
    let out = s
        .cmd()
        .args([
            "--json",
            "text",
            "convert",
            "a.txt",
            "nope.txt",
            "--trim-trailing",
        ])
        .args(["--in-place"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    let doc = json(&out);
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["exit_code"], 100);
    assert_eq!(doc["result"]["files"][0]["status"], "done");
    assert_eq!(doc["result"]["files"][1]["status"], "failed");
    assert_eq!(doc["result"]["summary"]["failed"], 1);
    assert_eq!(read(&s.join("a.txt")), b"a\n");
    // Bad values.
    for args in [
        &["b.txt", "--tab-size", "40", "--tabs-to-spaces"][..],
        &["b.txt", "--line-endings", "dos"],
        &["b.txt", "--encoding", "no-such-encoding"],
        &["b.txt", "--trim-trailing", "--format", "no-such-format"],
        &["b.txt", "--trim-trailing", "--lines", "5-2"],
    ] {
        let out = convert(&s, args);
        assert_eq!(code(&out), 100, "{args:?}");
    }
    assert_eq!(read(&s.join("b.txt")), b"b \n");
    // Every file done or skipped: 0.
    let out = convert(&s, &["a.txt", "b.txt", "--trim-trailing", "--in-place"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
}

/// A read-only file without `--yes`: without a terminal the question refuses the run (exit 100 with its count),
/// nothing written; `--yes` writes it and it stays read-only; `--confirm no-to-all` skips it (exit 0).
#[test]
fn a10_15_a_read_only_file_needs_yes() {
    let s = Sandbox::new();
    let ro = s.file("ro.txt", b"r \n");
    let mut perms = std::fs::metadata(&ro).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&ro, perms).unwrap();
    if std::fs::OpenOptions::new().append(true).open(&ro).is_ok() && !cfg!(windows) {
        eprintln!("skipped: this process writes read-only files (root)");
        return;
    }
    let out = convert(&s, &["ro.txt", "--trim-trailing", "--in-place"]);
    assert_eq!(code(&out), 100);
    let err = stderr(&out);
    assert!(err.contains("read_only_target 1"), "{err}");
    assert!(err.contains("--yes"), "{err}");
    assert_eq!(read(&ro), b"r \n");
    let out = convert(
        &s,
        &[
            "ro.txt",
            "--trim-trailing",
            "--in-place",
            "--confirm",
            "no-to-all",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("skipped (declined)"),
        "{}",
        stdout(&out)
    );
    assert_eq!(read(&ro), b"r \n");
    let out = convert(
        &s,
        &[
            "ro.txt",
            "--trim-trailing",
            "--in-place",
            "--yes",
            "--no-backup",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&ro), b"r\n");
    // Unix writes through the folder and the mode stays; Windows clears the `R` attribute (§1.10.14).
    if cfg!(unix) {
        assert!(std::fs::metadata(&ro).unwrap().permissions().readonly());
    }
}

#[test]
fn a10_19_lines_limit_the_content_conversions() {
    let s = Sandbox::new();
    s.file("a.txt", b"1 \n2 \n3 \n4 \n");
    let out = convert(&s, &["a.txt", "--trim-trailing", "--lines", "2-3"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(out.stdout, b"1 \n2\n3\n4 \n");
}

/// The conversions run in the fixed order trim, tabs, line endings, encoding, whatever the order of the flags: a
/// line of blanks is trimmed before its tab could become spaces, and the counts say so.
#[test]
fn a10_19_conversions_run_in_the_fixed_order() {
    let s = Sandbox::new();
    s.file("a.txt", b"\t \nx\t\ty\r\n");
    let out = s
        .cmd()
        .args(["--json", "text", "convert", "a.txt", "--dry-run"])
        .args([
            "--encoding",
            "UTF-16LE",
            "--line-endings",
            "lf",
            "--tabs-to-spaces",
        ])
        .args(["--tab-size", "2", "--trim-trailing"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    let conversions: Vec<(String, u64)> = doc["result"]["files"][0]["conversions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["kind"].as_str().unwrap().to_owned(),
                c["lines"].as_u64().unwrap(),
            )
        })
        .collect();
    let expected = [
        ("trim_trailing", 1),
        ("tabs_to_spaces", 1),
        ("line_endings", 1),
        ("encoding", 2),
    ];
    assert_eq!(
        conversions,
        expected
            .iter()
            .map(|(k, n)| ((*k).to_owned(), *n))
            .collect::<Vec<_>>()
    );
    let out = convert(
        &s,
        &[
            "a.txt",
            "--line-endings",
            "lf",
            "--tabs-to-spaces",
            "--tab-size",
            "2",
            "--trim-trailing",
        ],
    );
    assert_eq!(out.stdout, b"\nx   y\n");
}

#[test]
fn a10_19_the_bytes_equal_a_views_conversion_and_save() {
    use hikasa_core::{JobKind, NullProgress, Pair, Side};
    use hikasa_formats::StdProcessRunner;
    use hikasa_session::api::{LiveOptions, TextCompareRequest};
    use hikasa_session::{EnvFacts, FixedClock, Platform, Service, ServiceInit};
    use hikasa_text::save::{SaveRequest, UnmappablePolicy};
    use hikasa_text::{GroupHint, LineEnding, Transform, transform};

    // windows-1252, a BOM-less file with mixed terminators, trailing blanks, leading spaces and a line that does not
    // change.
    let content: &[u8] = b"    caf\xe9  \r\nkeep \xe9 as is\n        two\t \rlast    ";
    let s = Sandbox::new();
    s.file("cli.txt", content);
    s.file("view.txt", content);
    s.file("other.txt", b"x\n");
    let out = convert(
        &s,
        &[
            "cli.txt",
            "--encoding-in",
            "windows-1252",
            "--trim-trailing",
            "--spaces-to-tabs",
            "--tab-size",
            "4",
            "--line-endings",
            "crlf",
            "--in-place",
            "--no-backup",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));

    // The same file in a view: the conversions in the same order, then a save.
    let root = s.path();
    let facts = EnvFacts {
        cwd: Some(root.to_path_buf()),
        ..EnvFacts::new(Platform::HOST, root.to_path_buf())
    };
    let clock = Arc::new(FixedClock(
        "2026-11-02T00:00:00+00:00[UTC]".parse().unwrap(),
    ));
    let init = ServiceInit {
        config_dir: Some(s.config()),
        ..ServiceInit::new(facts, clock, Arc::new(StdProcessRunner))
    };
    let service = Service::new(init).unwrap();
    let job = service.job(JobKind::TextCompare, Arc::new(NullProgress));
    let req = TextCompareRequest {
        left: root.join("other.txt").into_os_string(),
        right: root.join("view.txt").into_os_string(),
        encodings: Pair::new(None, Some("windows-1252".to_owned())),
        ..TextCompareRequest::default()
    };
    let mut view = service
        .text_compare_live(&req, LiveOptions::default(), &job, None)
        .unwrap();
    let doc = &mut view.docs.right;
    for step in [
        Transform::TrimTrailing,
        Transform::LeadingSpacesToTabs,
        Transform::LineEndings(LineEnding::CrLf),
    ] {
        if let Some(request) = transform(doc, step, None, 4).request {
            doc.apply(&request, GroupHint::None).unwrap();
        }
    }
    let save = SaveRequest::in_place(None, UnmappablePolicy::Ask);
    let plan = service
        .text_save_plan(&[(Some(Side::Right), &*doc, &save)], &job)
        .unwrap();
    let report = service.text_save_execute(plan, &[&*doc], &job).unwrap();
    assert!(report.ok());
    let by_view = read(&root.join("view.txt"));
    assert_eq!(read(&root.join("cli.txt")), by_view);
    // The unchanged line is the file's bytes.
    assert!(
        by_view
            .windows(b"keep \xe9 as is\r\n".len())
            .any(|w| w == b"keep \xe9 as is\r\n")
    );
    assert_eq!(by_view, b"\tcaf\xe9\r\nkeep \xe9 as is\r\n\t\ttwo\r\nlast");
}

/// A10.32: a file whose name is not valid UTF-8 converts in place and keeps its name (Linux).
#[cfg(target_os = "linux")]
#[test]
fn a10_32_a_non_utf8_file_name_converts_in_place() {
    use std::os::unix::ffi::OsStrExt;
    let s = Sandbox::new();
    let name = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
    let path = s.path().join(name);
    std::fs::write(&path, b"a \n").unwrap();
    let out = s
        .cmd()
        .args(["text", "convert", "--trim-trailing", "--in-place"])
        .arg(name)
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&path), b"a\n");
    let mut backup = name.to_owned();
    backup.push(".bak");
    assert_eq!(read(&s.path().join(backup)), b"a \n");
}
