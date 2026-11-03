//! M2 W7b acceptance through the binary: A7.15 (the clipboard file, the Linux helper's hidden entry), A7.17 (identical
//! output: `report`, the compare shortcuts, stdout, a file, the clipboard, `Service::report_execute` into a `Vec`, and
//! the printer and browser files on a recording launcher), A7.18 (`hikasa report`), A7.19 (remembered choices change
//! nothing on the command line).
//!
//! The printer and browser targets open the associated application: the binary is never run with them except with
//! `--dry-run`; their files are compared in-process on a `RecordingLauncher` (never the real desktop).

mod common;

use std::path::Path;
use std::sync::Arc;

use common::{MTIME, Sandbox, assert_golden, code, json, stderr, stdout};
use hikasa_core::JobKind;
use hikasa_folder::NullObserver;
use hikasa_report::{ReportKind, ReportWords};
use hikasa_session::api::{
    FolderCompareRequest, ReportFlags, ReportSource, ReportTarget, TextCompareRequest,
};
use hikasa_session::launcher::{Recorded, RecordingLauncher};
use hikasa_session::{EnvFacts, Platform, Service, ServiceInit};

/// The report time of every test here (2023-11-14T22:13:20Z).
const EPOCH: &str = "1700000000";

/// The sandbox's command with the fixed report time.
fn cmd(s: &Sandbox) -> assert_cmd::Command {
    let mut cmd = s.cmd();
    cmd.env("SOURCE_DATE_EPOCH", EPOCH);
    cmd
}

/// Runs the binary with `args`.
fn run(s: &Sandbox, args: &[&str]) -> std::process::Output {
    cmd(s).args(args).output().unwrap()
}

/// The two text files and the two folders of the tests.
fn inputs(s: &Sandbox) -> (String, String, String, String) {
    let l = s.file("t/left.txt", "one\ntwo\nthree\nfour\n");
    let r = s.file("t/right.txt", "one\nTWO\nthree\nfive\nsix\n");
    s.file("l/a.txt", "alpha\n");
    s.file("r/a.txt", "alpha beta\n");
    s.file("l/same.txt", "same\n");
    s.file("r/same.txt", "same\n");
    s.file("l/sub/c.txt", "left only\n");
    // A folder's time too, last: NTFS moves it when a file inside gets its time, so two reports could differ there.
    filetime::set_file_mtime(
        s.join("l/sub"),
        filetime::FileTime::from_unix_time(MTIME, 0),
    )
    .expect("folder mtime");
    let (dl, dr) = (s.join("l"), s.join("r"));
    let p = |p: &Path| p.display().to_string();
    (p(&l), p(&r), p(&dl), p(&dr))
}

/// An in-process service like the binary's: the sandbox's folders, the fixed report time, a recording launcher.
fn service(s: &Sandbox, launcher: Arc<RecordingLauncher>) -> Service {
    let env = EnvFacts {
        cwd: Some(s.path().to_path_buf()),
        home: Some(s.join("home")),
        source_date_epoch: Some(EPOCH.into()),
        ..EnvFacts::new(Platform::HOST, s.join("tmp"))
    };
    std::fs::create_dir_all(s.join("tmp")).unwrap();
    let init = ServiceInit {
        config_dir: Some(s.config()),
        launcher,
        ..ServiceInit::new(
            env,
            Arc::new(hikasa_session::SystemClock),
            Arc::new(hikasa_formats::StdProcessRunner),
        )
    };
    Service::new(init).unwrap()
}

/// The layouts of a kind with the palettes each takes.
fn layouts(kind: ReportKind) -> Vec<(&'static str, &'static [&'static str])> {
    const ALL: &[&str] = &["", "html-color", "html-mono"];
    const TEXT: &[&str] = &[""];
    match kind {
        ReportKind::Text => vec![
            ("unified", TEXT),
            ("side-by-side", ALL),
            ("summary", ALL),
            ("interleaved", ALL),
            ("patch", TEXT),
            ("statistics", TEXT),
            ("xml", TEXT),
        ],
        ReportKind::Folder => vec![
            ("list", TEXT),
            ("side-by-side", ALL),
            ("summary", ALL),
            ("json", TEXT),
            ("xml", TEXT),
        ],
    }
}

/// A7.17: for every layout × palette × kind, the bytes of `hikasa report` to a file, to stdout and to the clipboard
/// file, of the compare subcommand's `--report` shortcut and of `Service::report_execute` into a `Vec` are the same;
/// the printer and browser files equal a file target's with the words they imply.
#[test]
fn a7_17_one_request_gives_the_same_bytes_everywhere() {
    let s = Sandbox::new();
    let (l, r, dl, dr) = inputs(&s);
    let launcher = Arc::new(RecordingLauncher::default());
    let service = service(&s, Arc::clone(&launcher));
    let text = service
        .text_compare(
            &TextCompareRequest {
                left: l.clone().into(),
                right: r.clone().into(),
                ..TextCompareRequest::default()
            },
            &service.job(JobKind::TextCompare, Arc::new(hikasa_core::NullProgress)),
            None,
        )
        .unwrap();
    let folder_request = FolderCompareRequest {
        left: dl.clone().into(),
        right: dr.clone().into(),
        ..FolderCompareRequest::default()
    };
    let control = service.job(JobKind::FolderCompare, Arc::new(hikasa_core::NullProgress));
    let prepared = service
        .folder_prepare_in(&folder_request, &control)
        .unwrap();
    let folder = service
        .folder_run(&prepared, &control, &NullObserver)
        .unwrap();
    for kind in [ReportKind::Text, ReportKind::Folder] {
        let (word, a, b, compare) = match kind {
            ReportKind::Text => ("text", &l, &r, ["text", "compare"]),
            ReportKind::Folder => ("folder", &dl, &dr, ["folder", "compare"]),
        };
        let source = match kind {
            ReportKind::Text => ReportSource::Text(&text),
            ReportKind::Folder => ReportSource::Folder {
                request: &folder_request,
                prepared: &prepared,
                response: &folder,
            },
        };
        for (layout, palettes) in layouts(kind) {
            for palette in palettes {
                let what = format!("{word} {layout} {palette}");
                let mut words: Vec<&str> = vec!["report", word, a, b, "--layout", layout];
                if !palette.is_empty() {
                    words.extend(["--output-options", palette]);
                }
                // A file.
                let file = s.join(&format!("out/{word}-{layout}-{palette}.out"));
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                let mut to_file = words.clone();
                let file_arg = file.display().to_string();
                to_file.extend(["--output-to", &file_arg]);
                let out = run(&s, &to_file);
                assert!(
                    stdout(&out).is_empty(),
                    "{what}: a file target prints nothing"
                );
                let bytes = std::fs::read(&file)
                    .unwrap_or_else(|e| panic!("{what}: {e}; {}", stderr(&out)));
                // Stdout.
                let out = run(&s, &words);
                assert_eq!(out.stdout, bytes, "{what}: stdout");
                // The shortcut of the compare subcommand.
                let mut shortcut: Vec<&str> =
                    vec![compare[0], compare[1], a, b, "--report", layout];
                if !palette.is_empty() {
                    shortcut.extend(["--output-options", palette]);
                }
                let out = run(&s, &shortcut);
                assert_eq!(out.stdout, bytes, "{what}: the compare shortcut");
                // The clipboard file: the HTML of an HTML report, else the text.
                let clip = s.join(&format!("clip/{word}-{layout}-{palette}"));
                std::fs::create_dir_all(clip.parent().unwrap()).unwrap();
                let mut to_clip = words.clone();
                to_clip.extend(["--output-to", "clipboard"]);
                let out = cmd(&s)
                    .env("HIKASA_CLIPBOARD_FILE", &clip)
                    .args(&to_clip)
                    .output()
                    .unwrap();
                assert_eq!(
                    code(&out),
                    code(&run(&s, &words)),
                    "{what}: {}",
                    stderr(&out)
                );
                assert_eq!(
                    std::fs::read(&clip).unwrap(),
                    bytes,
                    "{what}: the clipboard"
                );
                // The Service into a `Vec`.
                let flags = ReportFlags {
                    words: ReportWords {
                        layout: Some(layout.to_owned()),
                        output_options: if palette.is_empty() {
                            vec![]
                        } else {
                            vec![(*palette).to_owned()]
                        },
                        ..ReportWords::default()
                    },
                    ..ReportFlags::default()
                };
                let job = service.job(JobKind::Report, Arc::new(hikasa_core::NullProgress));
                let mut req = service
                    .report_request(kind, "side-by-side", &flags, &ReportTarget::Stdout)
                    .unwrap();
                let plan = service
                    .report_plan(source, &mut req, &ReportTarget::Stdout, None, &job)
                    .unwrap();
                let mut vec = Vec::new();
                service
                    .report_execute(source, &req, plan, Some(&mut vec), &job)
                    .unwrap();
                assert_eq!(vec, bytes, "{what}: Service::report_execute");
                // The printer and the browser: the words they imply.
                if !palette.is_empty() {
                    let print = if *palette == "html-mono" {
                        "print-mono"
                    } else {
                        "print-color"
                    };
                    for (target, implied) in [
                        (
                            ReportTarget::Printer,
                            vec![*palette, print, "print-portrait"],
                        ),
                        (ReportTarget::Browser, vec![*palette, print]),
                    ] {
                        let flags = ReportFlags {
                            words: ReportWords {
                                layout: Some(layout.to_owned()),
                                output_options: vec![print.to_owned()],
                                ..ReportWords::default()
                            },
                            ..ReportFlags::default()
                        };
                        let mut req = service
                            .report_request(kind, "side-by-side", &flags, &target)
                            .unwrap();
                        let plan = service
                            .report_plan(source, &mut req, &target, None, &job)
                            .unwrap();
                        let outcome = service
                            .report_execute(source, &req, plan, None, &job)
                            .unwrap();
                        let opened = outcome.opened.unwrap();
                        assert_eq!(
                            launcher.calls().last(),
                            Some(&Recorded::Associated(opened.clone()))
                        );
                        let printed = std::fs::read(&opened).unwrap();
                        let implied_file =
                            s.join(&format!("out/{word}-{layout}-{palette}-{target:?}.html"));
                        let implied_arg = implied_file.display().to_string();
                        let joined = implied.join(",");
                        let out = run(
                            &s,
                            &[
                                "report",
                                word,
                                a,
                                b,
                                "--layout",
                                layout,
                                "--output-options",
                                &joined,
                                "--output-to",
                                &implied_arg,
                            ],
                        );
                        assert!(
                            implied_file.is_file(),
                            "{what} {target:?}: {}",
                            stderr(&out)
                        );
                        assert_eq!(
                            printed,
                            std::fs::read(&implied_file).unwrap(),
                            "{what}: {target:?} with {joined}"
                        );
                    }
                }
            }
        }
    }
}

/// A7.17: `SOURCE_DATE_EPOCH` through the environment: a value gives the `Generated:` line in UTC; one that does not
/// parse gives the warning and the clock.
#[test]
fn a7_17_source_date_epoch_from_the_environment() {
    let s = Sandbox::new();
    let (l, r, _, _) = inputs(&s);
    for (value, line) in [
        ("0", Some("Generated: 1970-01-01T00:00:00+00:00 by hikasa ")),
        (
            "2208988800",
            Some("Generated: 2040-01-01T00:00:00+00:00 by hikasa "),
        ),
        ("", None),
        ("-1", None),
        ("abc", None),
        ("99999999999999999", None),
    ] {
        let out = s
            .cmd()
            .env("SOURCE_DATE_EPOCH", value)
            .args(["report", "text", &l, &r])
            .output()
            .unwrap();
        assert_eq!(code(&out), 13, "{value:?}: {}", stderr(&out));
        let text = stdout(&out);
        match line {
            Some(line) => {
                assert!(text.contains(line), "{value:?}: {text}");
                assert!(!stderr(&out).contains("SOURCE_DATE_EPOCH"), "{value:?}");
            }
            None => {
                assert!(text.contains("Generated: "), "{value:?}");
                assert!(
                    stderr(&out).contains("SOURCE_DATE_EPOCH"),
                    "{value:?}: {}",
                    stderr(&out)
                );
            }
        }
    }
    // Only reports read it: a JSON comparison has no time and no warning.
    let out = s
        .cmd()
        .env("SOURCE_DATE_EPOCH", "abc")
        .args(["--json", "text", "compare", &l, &r])
        .output()
        .unwrap();
    assert!(
        json(&out)["warnings"].as_array().unwrap().is_empty(),
        "{}",
        stdout(&out)
    );
}

/// A7.15: the clipboard file gets exactly a file target's bytes; a clipboard that cannot be written is exit 100.
#[test]
fn a7_15_the_clipboard_file_and_its_failure() {
    let s = Sandbox::new();
    let (l, r, _, _) = inputs(&s);
    let clip = s.join("clip.txt");
    let out = cmd(&s)
        .env("HIKASA_CLIPBOARD_FILE", &clip)
        .args([
            "report",
            "text",
            &l,
            &r,
            "--html",
            "--output-to",
            "clipboard://",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    assert!(stdout(&out).is_empty());
    let file = s.join("same.html");
    run(
        &s,
        &[
            "report",
            "text",
            &l,
            &r,
            "--html",
            "--output-to",
            &file.display().to_string(),
        ],
    );
    assert_eq!(std::fs::read(&clip).unwrap(), std::fs::read(&file).unwrap());
    // A clipboard file in a folder that does not exist cannot be written: `io`, exit 100.
    let out = cmd(&s)
        .env("HIKASA_CLIPBOARD_FILE", s.join("no/such/folder/clip"))
        .args([
            "--json",
            "report",
            "text",
            &l,
            &r,
            "--output-to",
            "clipboard",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100, "{}", stdout(&out));
    assert_eq!(json(&out)["error"]["kind"], "io");
    // Links to the clipboard: refused before anything is compared.
    let out = run(
        &s,
        &[
            "report",
            "folder",
            "l",
            "r",
            "--html",
            "--options",
            "include-file-links",
            "--output-to",
            "clipboard",
        ],
    );
    assert_eq!(code(&out), 100);
    assert!(
        stderr(&out).contains("file reports need a folder on disk"),
        "{}",
        stderr(&out)
    );
}

/// A7.15 (review): on Linux the system clipboard goes through the helper; a helper that cannot place the report is a
/// failed *write* of the clipboard (`io`, "cannot write the clipboard"), exit 100. The helper's fake placer
/// (`HIKASA_CLIPBOARD_HELPER_FILE`, in a folder that does not exist) stands in for the clipboard: no display is used.
#[cfg(target_os = "linux")]
#[test]
fn a7_15_a_failed_helper_is_a_write_of_the_clipboard() {
    let s = Sandbox::new();
    let (l, r, _, _) = inputs(&s);
    let out = cmd(&s)
        .env_remove("HIKASA_CLIPBOARD_FILE")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env(
            "HIKASA_CLIPBOARD_HELPER_FILE",
            s.join("no/such/folder/clip"),
        )
        .args([
            "--json",
            "report",
            "text",
            &l,
            &r,
            "--output-to",
            "clipboard",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100, "{}", stdout(&out));
    let doc = json(&out);
    assert_eq!(doc["error"]["kind"], "io", "{doc}");
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(message.contains("cannot write the clipboard"), "{message}");
    assert!(!message.contains("cannot read"), "{message}");
    // Placed through the same helper: the file holds a file target's bytes.
    let placed = s.join("placed.txt");
    let out = cmd(&s)
        .env_remove("HIKASA_CLIPBOARD_FILE")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env("HIKASA_CLIPBOARD_HELPER_FILE", &placed)
        .args(["report", "text", &l, &r, "--output-to", "clipboard"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    let file = s.join("same.txt");
    run(
        &s,
        &[
            "report",
            "text",
            &l,
            &r,
            "--output-to",
            &file.display().to_string(),
        ],
    );
    assert_eq!(
        std::fs::read(&placed).unwrap(),
        std::fs::read(&file).unwrap()
    );
}

/// A7.15: the Linux helper's hidden entry reads a payload from stdin and reports it placed (a fake placer, never the
/// real clipboard); the entry is in no help, no reference and no topic.
#[test]
fn a7_15_the_helper_entry() {
    let s = Sandbox::new();
    let placed = s.join("placed.html");
    let mut payload = Vec::new();
    let (text, html) = ("plain", "<p>html</p>");
    payload.extend_from_slice(&(text.len() as u64).to_le_bytes());
    payload.extend_from_slice(&(html.len() as u64).to_le_bytes());
    payload.extend_from_slice(text.as_bytes());
    payload.extend_from_slice(html.as_bytes());
    let out = assert_cmd::Command::new(common::hikasa_exe())
        .arg("__clipboard-helper")
        .env("HIKASA_CLIPBOARD_HELPER_FILE", &placed)
        .write_stdin(payload)
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(stdout(&out), "placed\n");
    assert_eq!(std::fs::read_to_string(&placed).unwrap(), html);
    // A short payload: one failure line.
    let out = assert_cmd::Command::new(common::hikasa_exe())
        .arg("__clipboard-helper")
        .env("HIKASA_CLIPBOARD_HELPER_FILE", &placed)
        .write_stdin(vec![1u8, 2, 3])
        .output()
        .unwrap();
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).starts_with("failed: "), "{}", stdout(&out));
    // Nowhere to be seen.
    let help = stdout(&run(&s, &["--help"]));
    assert!(!help.contains("clipboard-helper"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let reference = std::fs::read_to_string(root.join("docs/reference/commands.md")).unwrap();
    assert!(!reference.contains("clipboard-helper"));
    for entry in std::fs::read_dir(root.join("help")).unwrap() {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        assert!(!text.contains("__clipboard-helper"));
    }
}

/// A7.18: `report text` and `report folder` with specs and with `--session` (a text-compare, a folder-compare and a
/// folder-sync session), `-` and `clipboard://` as sources.
#[test]
fn a7_18_specs_sessions_and_sources() {
    let s = Sandbox::new();
    let (l, r, dl, dr) = inputs(&s);
    let direct = run(&s, &["report", "text", &l, &r]);
    assert_eq!(code(&direct), 13, "{}", stderr(&direct));
    for (name, kind, left, right) in [
        ("t", "text-compare", &l, &r),
        ("f", "folder-compare", &dl, &dr),
        ("y", "folder-sync", &dl, &dr),
    ] {
        let out = run(
            &s,
            &[
                "session", "new", name, "--type", kind, "--left", left, "--right", right,
            ],
        );
        assert_eq!(code(&out), 0, "{}", stderr(&out));
    }
    let via = run(&s, &["report", "text", "--session", "t"]);
    assert_eq!(
        via.stdout, direct.stdout,
        "a text session reports its files"
    );
    let folder = run(&s, &["report", "folder", &dl, &dr]);
    for name in ["f", "y"] {
        let via = run(&s, &["report", "folder", "--session", name]);
        assert_eq!(code(&via), code(&folder), "{name}: {}", stderr(&via));
        assert_eq!(via.stdout, folder.stdout, "{name}");
    }
    let wrong = run(&s, &["report", "folder", "--session", "t"]);
    assert_eq!(code(&wrong), 100);
    let wrong = run(&s, &["report", "text", "--session", "f"]);
    assert_eq!(code(&wrong), 100);
    // The flags apply on top of a session.
    let iu = run(
        &s,
        &[
            "report",
            "text",
            "--session",
            "t",
            "--layout",
            "summary",
            "--unimportant",
            "case",
        ],
    );
    assert!(
        stdout(&iu).contains("Summary") || code(&iu) != 100,
        "{}",
        stderr(&iu)
    );
    // `-` and `clipboard://` as sources.
    let left_bytes = std::fs::read(&l).unwrap();
    let out = cmd(&s)
        .args(["report", "text", "-", &r])
        .write_stdin(left_bytes.clone())
        .output()
        .unwrap();
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    assert!(stdout(&out).contains("TWO"));
    let clip = s.file("clip.txt", left_bytes);
    let out = cmd(&s)
        .env("HIKASA_CLIPBOARD_FILE", &clip)
        .args(["report", "text", "clipboard://", &r])
        .output()
        .unwrap();
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    assert!(stdout(&out).contains("TWO"));
}

/// A7.18: the flags of REPORT, word lists, conflicts, `--output` with `--output-to`, a file named like a keyword, the
/// scopes, `--json` with stdout.
#[test]
fn a7_18_the_flags_of_report() {
    let s = Sandbox::new();
    let (l, r, dl, dr) = inputs(&s);
    let ok = |args: &[&str]| {
        let out = run(&s, args);
        assert!(code(&out) < 100, "{args:?}: {}", stderr(&out));
        out
    };
    let refused = |args: &[&str], needle: &str| {
        let out = run(&s, args);
        assert_eq!(code(&out), 100, "{args:?}: {}", stdout(&out));
        assert!(stderr(&out).contains(needle), "{args:?}: {}", stderr(&out));
    };
    // Every flag of REPORT, once.
    ok(&[
        "report",
        "text",
        &l,
        &r,
        "--layout",
        "interleaved",
        "--options",
        "display-context,strikeout-left-diffs",
        "--output-options",
        "wrap-word",
        "--title",
        "T",
        "--context",
        "1",
        "--width",
        "100",
        "--no-line-numbers",
    ]);
    ok(&[
        "report",
        "text",
        &l,
        &r,
        "--report",
        "side-by-side",
        "--display",
        "mismatches",
        "--line-numbers",
        "--wrap",
        "none",
        "--html-mono",
    ]);
    ok(&[
        "report",
        "text",
        &l,
        &r,
        "--layout",
        "patch",
        "--patch-style",
        "context",
    ]);
    ok(&[
        "report",
        "folder",
        &dl,
        &dr,
        "--display",
        "all",
        "--folders",
        "always",
        "--suppress-filters",
        "--columns",
        "size,crc",
        "--html",
    ]);
    let out = ok(&[
        "report",
        "text",
        &l,
        &r,
        "--options",
        "display-all",
        "--options",
        "ignore-unimportant",
        "--output-options",
        "html-color,wrap-word",
    ]);
    assert!(stdout(&out).starts_with("<!DOCTYPE html>"));
    // Conflicts, also with a flag-made word; the same word twice is one.
    refused(
        &[
            "report",
            "text",
            &l,
            &r,
            "--html-mono",
            "--output-options",
            "html-color",
        ],
        "conflicting report options",
    );
    refused(
        &[
            "report",
            "text",
            &l,
            &r,
            "--options",
            "display-all,display-context",
        ],
        "conflicting report options",
    );
    ok(&[
        "report",
        "text",
        &l,
        &r,
        "--options",
        "display-all,display-all",
    ]);
    // Unused words: one warning.
    let out = ok(&[
        "report",
        "text",
        &l,
        &r,
        "--options",
        "strikeout-left-diffs",
    ]);
    assert_eq!(
        stderr(&out)
            .matches("side-by-side does not use: strikeout-left-diffs")
            .count(),
        1,
        "{}",
        stderr(&out)
    );
    let out = ok(&[
        "--json",
        "report",
        "text",
        &l,
        &r,
        "--options",
        "strikeout-left-diffs",
        "--output-to",
        "unused.txt",
    ]);
    let warnings = json(&out)["warnings"].as_array().unwrap().clone();
    assert_eq!(
        warnings
            .iter()
            .filter(|w| w["code"] == "report_option_unused")
            .count(),
        1,
        "{warnings:?}"
    );
    // A flag of the other kind.
    refused(
        &["report", "folder", &dl, &dr, "--wrap", "word"],
        "--wrap applies to text reports",
    );
    refused(
        &["report", "text", &l, &r, "--folders", "always"],
        "--folders applies to folder reports",
    );
    // `--output` with `--output-to`: a usage error.
    let out = run(
        &s,
        &[
            "report",
            "text",
            &l,
            &r,
            "--output",
            "a",
            "--output-to",
            "b",
        ],
    );
    assert_eq!(code(&out), 100);
    // A file named like a keyword.
    std::fs::create_dir_all(s.join("k")).unwrap();
    let out = cmd(&s)
        .current_dir(s.join("k"))
        .args(["report", "text", &l, &r, "--output-to", "./printer"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    assert!(s.join("k/printer").is_file());
    // Scopes: a range of lines, rows by mask and by path; a scope with a layout without rows is refused.
    let out = ok(&["report", "text", &l, &r, "--lines", "2-3"]);
    assert!(
        stdout(&out).contains("scope: lines 2\u{2013}3 of the left"),
        "{}",
        stdout(&out)
    );
    let out = ok(&[
        "report",
        "text",
        &l,
        &r,
        "--lines",
        "right:4-5",
        "--layout",
        "interleaved",
    ]);
    assert!(stdout(&out).contains("of the right"));
    refused(
        &[
            "report", "text", &l, &r, "--lines", "2-3", "--layout", "summary",
        ],
        "a scope needs",
    );
    let out = ok(&[
        "report",
        "folder",
        &dl,
        &dr,
        "--select",
        "all.diff.files",
        "--layout",
        "list",
    ]);
    assert!(
        stdout(&out).contains("a.txt") && !stdout(&out).contains("c.txt"),
        "{}",
        stdout(&out)
    );
    let out = ok(&[
        "report", "folder", &dl, &dr, "--path", "a.txt", "--layout", "xml",
    ]);
    assert!(stdout(&out).contains("scope=\""), "{}", stdout(&out));
    // `--json` with the stdout target.
    let out = run(&s, &["--json", "report", "text", &l, &r]);
    assert_eq!(code(&out), 100);
    assert!(stderr(&out).contains("cannot share stdout: pass --output-to"));
    assert_eq!(json(&out)["schema"], "hikasa.report.v1");
}

/// A JSON document with the sizes of a written report as `"<BYTES>"`: they hold the temporary folder's path, whose
/// length differs between machines (a dry run's `0` stays).
fn sizes(doc: &str) -> String {
    let mut out = String::new();
    for line in doc.lines() {
        let trimmed = line.trim_start();
        let key = ["\"bytes\": ", "\"text_bytes\": ", "\"html_bytes\": "]
            .into_iter()
            .find(|k| trimmed.starts_with(k));
        match key {
            Some(key)
                if !trimmed[key.len()..].starts_with('0')
                    && !trimmed[key.len()..].starts_with("null") =>
            {
                let indent = &line[..line.len() - trimmed.len()];
                let comma = if trimmed.ends_with(',') { "," } else { "" };
                out.push_str(&format!("{indent}{key}\"<BYTES>\"{comma}"));
            }
            _ => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// A7.18: the dry run's lines and JSON for each target and links state, as goldens.
#[test]
fn a7_18_dry_runs_and_json_goldens() {
    let s = Sandbox::new();
    let (l, r, dl, dr) = inputs(&s);
    std::fs::create_dir_all(s.join("out")).unwrap();
    let main = s.join("out/report.html").display().to_string();
    let links = [
        "report",
        "folder",
        &dl,
        &dr,
        "--html",
        "--options",
        "include-file-links",
        "--output-to",
        &main,
    ];
    let golden = |name: &str, out: &std::process::Output| {
        assert!(code(out) < 100, "{name}: {}", stderr(out));
        assert_golden(
            &format!("report/{name}"),
            &sizes(&s.normalise(&stdout(out))),
        );
    };
    let mut dry = links.to_vec();
    dry.push("--dry-run");
    golden("links-new-dry-run.txt", &run(&s, &dry));
    let mut dry_json = vec!["--json"];
    dry_json.extend(&dry);
    golden("links-new-dry-run.json", &run(&s, &dry_json));
    assert!(
        !s.join("out/report.html").exists(),
        "a dry run writes nothing"
    );
    let mut json_run = vec!["--json"];
    json_run.extend(&links);
    golden("links-new.json", &run(&s, &json_run));
    golden("links-replace-ours-dry-run.txt", &run(&s, &dry));
    golden("links-replace-ours.json", &run(&s, &json_run));
    // A folder that is not this report's.
    std::fs::remove_dir_all(s.join("out")).unwrap();
    s.file("out/report_files/theirs.txt", "keep");
    golden("links-renamed-dry-run.txt", &run(&s, &dry));
    golden("links-renamed.json", &run(&s, &json_run));
    assert_eq!(
        std::fs::read_to_string(s.join("out/report_files/theirs.txt")).unwrap(),
        "keep"
    );
    // Each target.
    let file = s.join("out/t.txt").display().to_string();
    golden(
        "text-file.json",
        &run(
            &s,
            &["--json", "report", "text", &l, &r, "--output-to", &file],
        ),
    );
    let clip = s.join("clip");
    let out = cmd(&s)
        .env("HIKASA_CLIPBOARD_FILE", &clip)
        .args([
            "--json",
            "report",
            "text",
            &l,
            &r,
            "--html",
            "--output-to",
            "clipboard",
        ])
        .output()
        .unwrap();
    golden("text-clipboard.json", &out);
    golden(
        "text-printer-dry-run.json",
        &run(
            &s,
            &[
                "--json",
                "report",
                "text",
                &l,
                &r,
                "--output-to",
                "printer",
                "--dry-run",
            ],
        ),
    );
    golden(
        "text-browser-dry-run.txt",
        &run(
            &s,
            &[
                "report",
                "text",
                &l,
                &r,
                "--output-to",
                "browser",
                "--dry-run",
            ],
        ),
    );
    golden(
        "text-file-dry-run.txt",
        &run(
            &s,
            &["report", "text", &l, &r, "--output-to", &file, "--dry-run"],
        ),
    );
}

/// A7.18: the exit codes of `report` are the compare subcommand's on the same pairs.
#[test]
fn a7_18_exit_codes_are_the_comparisons() {
    let s = Sandbox::new();
    let (l, r, dl, dr) = inputs(&s);
    let same = s.file("t/same.txt", "one\ntwo\nthree\nfour\n");
    let case = s.file("t/case.txt", "ONE\ntwo\nthree\nfour\n");
    let bin_l = s.file("t/a.bin", b"\x00\x01binary".as_slice());
    let bin_r = s.file("t/b.bin", b"\x00\x02binary".as_slice());
    let missing = s.join("t/missing.txt").display().to_string();
    let p = |p: &Path| p.display().to_string();
    let pairs: Vec<(String, String, Vec<&str>)> = vec![
        (l.clone(), p(&same), vec![]),
        (l.clone(), r.clone(), vec![]),
        (l.clone(), p(&case), vec!["--unimportant", "case"]),
        (
            l.clone(),
            p(&case),
            vec!["--unimportant", "case", "--ignore-unimportant"],
        ),
        (p(&bin_l), p(&bin_r), vec![]),
        (l.clone(), missing.clone(), vec![]),
    ];
    let mut codes = std::collections::BTreeSet::new();
    for (a, b, extra) in &pairs {
        let mut report = vec!["report", "text", a.as_str(), b.as_str()];
        report.extend(extra);
        let mut compare = vec!["text", "compare", a.as_str(), b.as_str()];
        compare.extend(extra);
        let (x, y) = (code(&run(&s, &report)), code(&run(&s, &compare)));
        assert_eq!(x, y, "{a} {b} {extra:?}");
        codes.insert(x);
    }
    for (a, b) in [
        (dl.clone(), dr.clone()),
        (dl.clone(), dl.clone()),
        (dl.clone(), missing.clone()),
    ] {
        let (x, y) = (
            code(&run(&s, &["report", "folder", &a, &b])),
            code(&run(&s, &["folder", "compare", &a, &b])),
        );
        assert_eq!(x, y, "{a} {b}");
        codes.insert(x);
    }
    assert!(
        codes.contains(&107) && codes.contains(&100) && codes.len() >= 5,
        "{codes:?}"
    );
}

/// A7.19: choices remembered in `state.toml` change nothing on the command line.
#[test]
fn a7_19_remembered_choices_do_not_reach_the_command_line() {
    let s = Sandbox::new();
    let (l, r, dl, dr) = inputs(&s);
    let before_text = run(&s, &["report", "text", &l, &r]);
    let before_folder = run(&s, &["report", "folder", &dl, &dr]);
    let service = service(&s, Arc::new(RecordingLauncher::default()));
    let every_slot = hikasa_session::api::ReportChoices {
        layout: Some("xml".into()),
        options: vec![
            "display-none".into(),
            "ignore-unimportant".into(),
            "no-line-numbers".into(),
            "strikeout-left-diffs".into(),
        ],
        output_options: vec![
            "html-mono".into(),
            "print-landscape".into(),
            "wrap-none".into(),
        ],
        output_to: Some("clipboard".into()),
        file: Some("/elsewhere/report.html".into()),
    };
    service
        .remember_report_choices(ReportKind::Text, &every_slot)
        .unwrap();
    let folder_slots = hikasa_session::api::ReportChoices {
        layout: Some("json".into()),
        options: vec!["display-none".into(), "column-crc".into()],
        ..every_slot.clone()
    };
    service
        .remember_report_choices(ReportKind::Folder, &folder_slots)
        .unwrap();
    assert_eq!(
        service.report_choices(ReportKind::Text).layout.as_deref(),
        Some("xml")
    );
    let after_text = run(&s, &["report", "text", &l, &r]);
    let after_folder = run(&s, &["report", "folder", &dl, &dr]);
    assert_eq!(after_text.stdout, before_text.stdout);
    assert_eq!(after_folder.stdout, before_folder.stdout);
    assert_eq!(code(&after_text), code(&before_text));
}

/// A7.20: `hikasa help report` and `hikasa help report folder` resolve to the report topic.
#[test]
fn a7_20_help_report_resolves() {
    let s = Sandbox::new();
    for words in [
        &["help", "report"][..],
        &["help", "report", "folder"],
        &["help", "report", "text"],
    ] {
        let out = run(&s, words);
        assert_eq!(code(&out), 0, "{words:?}");
        assert!(
            stdout(&out).starts_with("hikasa report\n"),
            "{words:?}: {}",
            stdout(&out)
        );
    }
    let out = run(&s, &["--json", "help", "report", "folder"]);
    assert_eq!(json(&out)["result"]["topic"]["id"], "report");
}

/// A7.12, A7.14, A7.18 (review): a folder target is refused alike by the dry run and the run (exit 100, "is a
/// folder"); links into an archive are refused before anything is compared (exit 100 where a comparison of a missing
/// folder would be 107); M1's flags stay silent on a layout that does not use them, the words do not.
#[test]
fn review_targets_refused_before_and_m1_flags_silent() {
    let s = Sandbox::new();
    let (l, r, dl, dr) = inputs(&s);
    let folder = s.join("out");
    std::fs::create_dir_all(&folder).unwrap();
    let to = folder.display().to_string();
    let mut messages = Vec::new();
    for dry in [true, false] {
        let mut args = vec![
            "report",
            "text",
            l.as_str(),
            r.as_str(),
            "--output-to",
            to.as_str(),
        ];
        if dry {
            args.push("--dry-run");
        }
        let out = run(&s, &args);
        assert_eq!(code(&out), 100, "dry {dry}: {}", stderr(&out));
        assert!(stdout(&out).is_empty(), "dry {dry}: {}", stdout(&out));
        assert!(stderr(&out).contains("is a folder"), "{}", stderr(&out));
        messages.push(stderr(&out));
    }
    assert_eq!(
        messages[0], messages[1],
        "the dry run says what the run does"
    );
    // Links into an archive: nothing compared (the right folder does not exist).
    std::fs::write(
        s.join("b.zip"),
        b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    )
    .unwrap();
    let member = s.join("b.zip").join("r.html").display().to_string();
    let missing = s.join("missing").display().to_string();
    let out = run(
        &s,
        &[
            "report",
            "folder",
            &dl,
            &missing,
            "--html",
            "--options",
            "include-file-links",
            "--output-to",
            &member,
        ],
    );
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert!(
        stderr(&out).contains("file reports need a folder on disk"),
        "{}",
        stderr(&out)
    );
    // M1's flags: no warning.
    for args in [
        vec![
            "text",
            "compare",
            &l,
            &r,
            "--wrap",
            "word",
            "--patch-style",
            "context",
            "--no-line-numbers",
        ],
        vec![
            "folder",
            "compare",
            &dl,
            &dr,
            "--report",
            "summary",
            "--columns",
            "size,crc",
        ],
    ] {
        let out = run(&s, &args);
        assert!(
            !stderr(&out).contains("does not use"),
            "{args:?}: {}",
            stderr(&out)
        );
    }
    // The words: the warning.
    let out = run(
        &s,
        &["text", "compare", &l, &r, "--output-options", "wrap-word"],
    );
    assert!(
        stderr(&out).contains("does not use: wrap-word"),
        "{}",
        stderr(&out)
    );
}
