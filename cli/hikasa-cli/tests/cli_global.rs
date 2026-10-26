//! Global behaviour of `hikasa` (contract §6.3 `cli_global.rs`, §1.9.2 CLI invariants, §2.21, §4.1, §4.2;
//! D-35, D-40): usage errors, help and version, `--quiet`, broken pipes, colours, logging, one JSON document,
//! completions and version.

mod common;

use std::io::Read;
use std::process::{Command, Stdio};

use common::{Sandbox, assert_golden, code, json, stderr, stdout};

#[test]
fn usage_errors_are_100_not_2() {
    let s = Sandbox::new();
    for args in [
        &["bogus"][..],
        &[],
        &["text"],
        &["text", "compare", "only-one"],
        &["quick", "--no-such-flag", "a", "b"],
        &[
            "folder",
            "compare",
            "a",
            "b",
            "--size",
            "--html",
            "--html-mono",
        ],
        &["gui", "close", "not-a-number"],
        &["completions", "tcsh"],
        &["text", "compare", "a", "b", "--wait"],
    ] {
        let out = s.cmd().args(args).output().unwrap();
        assert_eq!(code(&out), 100, "D-35 {args:?}");
        assert!(
            out.stdout.is_empty(),
            "{args:?}: nothing on stdout without --json"
        );
        assert!(!stderr(&out).is_empty(), "{args:?}: the error is on stderr");
    }
    let out = s.cmd().args(["--json", "bogus"]).output().unwrap();
    assert_eq!(code(&out), 100);
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.usage_error.v1");
    assert_eq!(doc["error"]["kind"], "invalid_input");
    assert_golden("global/usage-error.json", &s.normalise(&stdout(&out)));
    let late = s
        .cmd()
        .args(["quick", "a", "--bogus", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&late), 100);
    assert_eq!(
        json(&late)["schema"],
        "hikasa.usage_error.v1",
        "--json anywhere"
    );
}

#[test]
fn help_and_version_are_0() {
    let s = Sandbox::new();
    for args in [
        &["--help"][..],
        &["-h"],
        &["--version"],
        &["-V"],
        &["text", "compare", "--help"],
        &["folder", "compare", "--help"],
        &["gui", "--help"],
        &["quick", "--version"],
    ] {
        let out = s.cmd().args(args).output().unwrap();
        assert_eq!(code(&out), 0, "{args:?}");
        assert!(!out.stdout.is_empty(), "{args:?}");
    }
    let version = s.cmd().arg("--version").output().unwrap();
    assert_eq!(
        stdout(&version),
        format!("hikasa {}\n", hikasa_session::HIKASA_VERSION)
    );
    // Regression (wave-5 review): after a subcommand too, `hikasa <version>` (not `hikasa-quick 0.0.0`).
    for args in [
        &["quick", "--version"][..],
        &["text", "compare", "--version"],
        &["gui", "--version"],
        &["format", "list", "-V"],
    ] {
        let out = s.cmd().args(args).output().unwrap();
        assert_eq!(code(&out), 0, "{args:?}");
        assert_eq!(
            stdout(&out),
            format!("hikasa {}\n", hikasa_session::HIKASA_VERSION),
            "{args:?}"
        );
    }
}

/// Regression (wave-5 review): with `--json`, `--version` prints the `hikasa version --json` document; help is
/// text (the documented exemption of CLI invariant 2, §1.9.3).
#[test]
fn json_with_help_and_version() {
    let s = Sandbox::new();
    let version = s.cmd().args(["version", "--json"]).output().unwrap();
    for args in [&["--json", "--version"][..], &["quick", "--json", "-V"]] {
        let out = s.cmd().args(args).output().unwrap();
        assert_eq!(code(&out), 0, "{args:?}");
        assert_eq!(json(&out)["schema"], "hikasa.version.v1");
        assert_eq!(stdout(&out), stdout(&version), "{args:?}");
    }
    for args in [&["--json", "--help"][..], &["quick", "--help", "--json"]] {
        let out = s.cmd().args(args).output().unwrap();
        assert_eq!(code(&out), 0, "{args:?}");
        assert!(stdout(&out).contains("Usage: hikasa"), "{args:?}");
    }
}

/// Regression (wave-5 review): the usage-error document keeps clap's whole message, and `--json` after `--` is
/// an operand (a file name), not the flag.
#[test]
fn usage_error_documents() {
    let s = Sandbox::new();
    s.file("a.txt", "x\n");
    s.file("--json", "x\n");
    let out = s.cmd().args(["--json", "quick", "a.txt"]).output().unwrap();
    assert_eq!(code(&out), 100);
    let message = json(&out)["error"]["message"].as_str().unwrap().to_owned();
    assert!(
        message.contains("the following required arguments were not provided")
            && message.contains("<RIGHT>"),
        "{message}"
    );
    // A file named `--json` after `--`: a normal comparison in human mode.
    let file = s
        .cmd()
        .args(["quick", "a.txt", "--", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&file), 1, "binary same: {}", stderr(&file));
    assert!(file.stdout.is_empty(), "no JSON: {}", stdout(&file));
    let usage = s.cmd().args(["quick", "--", "--json"]).output().unwrap();
    assert_eq!(code(&usage), 100);
    assert!(usage.stdout.is_empty(), "no JSON: {}", stdout(&usage));
}

/// Regression (wave-5 review, §1.8.7 item 34): a global value flag repeated at different levels with different
/// values is a usage error (clap kept the innermost silently); the same folder twice is fine.
#[test]
fn repeated_global_values() {
    let s = Sandbox::new();
    for args in [
        &["format", "list", "--config-dir", "other"][..],
        &["--json", "format", "--config-dir=other", "list"],
        &["--log", "a.log", "format", "list", "--log", "b.log"],
    ] {
        let out = s.cmd().args(args).output().unwrap();
        assert_eq!(code(&out), 100, "{args:?}");
        assert!(
            stderr(&out).contains("is given twice with different values"),
            "{args:?}: {}",
            stderr(&out)
        );
        if args.contains(&"--json") {
            assert_eq!(json(&out)["schema"], "hikasa.usage_error.v1");
        }
    }
    let same = s
        .cmd()
        .args(["format", "list", "--config-dir", "cfg"])
        .output()
        .unwrap();
    assert_eq!(code(&same), 0, "{}", stderr(&same));
}

/// Regression (wave-5 review): one failed write to stdout gives one error line (it gave two).
#[cfg(target_os = "linux")]
#[test]
fn a_full_stdout_is_one_error_line() {
    let s = Sandbox::new();
    let big: String = (0..20_000).map(|k| format!("line {k}\n")).collect();
    s.file("l.txt", &big);
    s.file("r.txt", big.replace("line 7\n", "changed\n"));
    for args in [
        &["text", "compare", "l.txt", "r.txt"][..],
        &["vfs", "cat", "l.txt"],
        &["format", "list", "--json"],
    ] {
        let Ok(full) = std::fs::OpenOptions::new().write(true).open("/dev/full") else {
            return;
        };
        let out = Command::new(common::hikasa_exe())
            .current_dir(s.path())
            .env("HIKASA_IPC_ENDPOINT", s.socket())
            .env("LC_ALL", "C.UTF-8")
            .arg("--config-dir")
            .arg(s.config())
            .args(args)
            .stdout(full)
            .output()
            .unwrap();
        assert_eq!(code(&out), 100, "{args:?}");
        let err = stderr(&out);
        assert_eq!(err.lines().count(), 1, "{args:?}: {err}");
        assert!(err.contains("error:"), "{err}");
    }
}

#[test]
fn version_goldens() {
    let s = Sandbox::new();
    let out = s.cmd().arg("version").output().unwrap();
    assert_eq!(code(&out), 0);
    assert_eq!(
        stdout(&out),
        format!("hikasa {}\n", hikasa_session::HIKASA_VERSION)
    );
    let doc = s.cmd().args(["version", "--json"]).output().unwrap();
    assert_eq!(
        json(&doc)["result"]["schemas"].as_array().unwrap().len(),
        18
    );
    assert_golden("global/version.json", &s.normalise(&stdout(&doc)));
}

#[test]
fn completions_for_every_shell() {
    let s = Sandbox::new();
    for shell in ["bash", "elvish", "fish", "powershell", "zsh"] {
        let out = s.cmd().args(["completions", shell]).output().unwrap();
        assert_eq!(code(&out), 0, "{shell}");
        let script = stdout(&out);
        assert!(script.len() > 1000, "{shell}: {} bytes", script.len());
        assert!(
            script.contains("hikasa") && script.contains("compare"),
            "{shell}"
        );
        let doc = s
            .cmd()
            .args(["completions", shell, "--json"])
            .output()
            .unwrap();
        let parsed = json(&doc);
        assert_eq!(parsed["schema"], "hikasa.completions.v1");
        assert_eq!(parsed["result"]["shell"], shell);
        assert_eq!(parsed["result"]["script"], script.as_str(), "{shell}");
    }
    // The shape of the document as emitted (field order included, §4.1 rule 2); the script itself is checked
    // against the human output above.
    let doc = s
        .cmd()
        .args(["completions", "zsh", "--json"])
        .output()
        .unwrap();
    let script = json(&doc)["result"]["script"].clone();
    let escaped = serde_json::to_string(&script).unwrap();
    let raw = stdout(&doc);
    assert_eq!(raw.matches(&escaped).count(), 1, "the script appears once");
    let text = raw.replace(&escaped, "\"<SCRIPT>\"");
    assert_golden("global/completions.json", &s.normalise(&text));
}

/// Runs `hikasa <args>` in `s` with each extra flag set and returns (exit code, stdout).
fn run_with(s: &Sandbox, args: &[&str], extra: &[&str]) -> (i32, Vec<u8>) {
    let out = s.cmd().args(args).args(extra).output().unwrap();
    (code(&out), out.stdout)
}

#[test]
fn quiet_never_changes_stdout_or_the_exit_code() {
    let s = Sandbox::new();
    s.file("a/x.c", "int x;\n");
    s.file("b/x.c", "int y;\n");
    s.file("a/only.txt", "o\n");
    s.options("[no_such_section]\nkey = 1\n");
    for args in [
        &["text", "compare", "a/x.c", "b/x.c"][..],
        &["text", "compare", "a/x.c", "b/x.c", "--json"],
        &["folder", "compare", "a", "b"],
        &["folder", "compare", "a", "b", "--json"],
        &["quick", "a/x.c", "b/x.c"],
        &["format", "list"],
        &["vfs", "cat", "a/x.c"],
        &["quick", "a/x.c", "missing"],
    ] {
        let loud = run_with(&s, args, &[]);
        let quiet = run_with(&s, args, &["--quiet"]);
        let normalise = |bytes: &[u8]| {
            String::from_utf8_lossy(bytes)
                .lines()
                .filter(|l| !l.contains("Generated:"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(loud.0, quiet.0, "{args:?}");
        assert_eq!(normalise(&loud.1), normalise(&quiet.1), "{args:?}");
    }
    // The startup warning (an unknown section) is printed once without --quiet, never with it.
    let loud = s.cmd().args(["format", "list"]).output().unwrap();
    assert_eq!(
        stderr(&loud).matches("no_such_section").count(),
        1,
        "{}",
        stderr(&loud)
    );
    let quiet = s.cmd().args(["format", "list", "-q"]).output().unwrap();
    assert!(quiet.stderr.is_empty(), "{}", stderr(&quiet));
    let failing = s
        .cmd()
        .args(["quick", "a/x.c", "missing"])
        .output()
        .unwrap();
    assert_eq!(
        stderr(&failing).matches("no_such_section").count(),
        1,
        "once, also when the command fails: {}",
        stderr(&failing)
    );
    // Errors are still reported with --quiet.
    let failed = s
        .cmd()
        .args(["quick", "a/x.c", "missing", "--quiet"])
        .output()
        .unwrap();
    assert!(stderr(&failed).contains("missing"), "{}", stderr(&failed));
    // With --json the startup warning is the first warning of the envelope.
    let doc = s.cmd().args(["format", "list", "--json"]).output().unwrap();
    assert_eq!(json(&doc)["warnings"][0]["code"], "unknown_config_key");
}

#[test]
fn a_broken_pipe_keeps_the_exit_code() {
    let s = Sandbox::new();
    let mut left = String::new();
    let mut right = String::new();
    for k in 0..40_000 {
        left.push_str(&format!("line number {k} of the left file\n"));
        if k == 20_000 {
            right.push_str("a changed line\n");
        } else {
            right.push_str(&format!("line number {k} of the left file\n"));
        }
    }
    s.file("l.txt", &left);
    s.file("r.txt", &right);
    for extra in [
        &["--report", "side-by-side"][..],
        &["--json", "--display", "all"],
    ] {
        let mut child = Command::new(common::hikasa_exe())
            .current_dir(s.path())
            .env("HIKASA_IPC_ENDPOINT", s.socket())
            .arg("--config-dir")
            .arg(s.config())
            .args(["text", "compare", "l.txt", "r.txt"])
            .args(extra)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Read a little, then close the pipe while the CLI is still writing.
        let mut stdout = child.stdout.take().unwrap();
        let mut head = [0u8; 16];
        stdout.read_exact(&mut head).unwrap();
        drop(stdout);
        let mut err = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut err)
            .unwrap();
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(13), "{extra:?}: {err}");
        assert!(!err.contains("panic"), "{err}");
    }
    // vfs cat: the computed code is 0.
    let mut child = Command::new(common::hikasa_exe())
        .current_dir(s.path())
        .env("HIKASA_IPC_ENDPOINT", s.socket())
        .arg("--config-dir")
        .arg(s.config())
        .args(["vfs", "cat", "l.txt"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

#[test]
fn no_color_means_no_ansi() {
    let s = Sandbox::new();
    s.file("a.c", "int x;\n");
    s.file("b.c", "int y;\n");
    let plain = s
        .cmd()
        .args(["text", "compare", "a.c", "b.c", "--no-color"])
        .output()
        .unwrap();
    assert_eq!(code(&plain), 13);
    let env = s
        .cmd()
        .args(["text", "compare", "a.c", "b.c", "--verbose"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let usage = s.cmd().args(["bogus", "--no-color"]).output().unwrap();
    for out in [&plain, &env, &usage] {
        assert!(!out.stdout.contains(&0x1b), "no ANSI on stdout");
        assert!(!out.stderr.contains(&0x1b), "no ANSI on stderr");
    }
}

#[test]
fn log_file_and_verbose() {
    let s = Sandbox::new();
    s.options("[text_editing]\nno_such_option = 1\n");
    let log = s.join("hikasa.log");
    let out = s
        .cmd()
        .args(["format", "list", "--log"])
        .arg(&log)
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(log.exists(), "--log creates the file");
    // --verbose logs the ignored option keys (D-31): to stderr without --log, to the file with it.
    let verbose = s
        .cmd()
        .args(["format", "list", "--verbose"])
        .output()
        .unwrap();
    assert!(
        stderr(&verbose).contains("no_such_option"),
        "{}",
        stderr(&verbose)
    );
    let quiet = s.cmd().args(["format", "list"]).output().unwrap();
    assert!(
        !stderr(&quiet).contains("no_such_option"),
        "ignored silently"
    );
    let both = s
        .cmd()
        .args(["format", "list", "--verbose", "--log"])
        .arg(&log)
        .output()
        .unwrap();
    assert!(
        !stderr(&both).contains("no_such_option"),
        "the log goes to the file"
    );
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains("no_such_option"), "{text}");
    // A log file that cannot be created fails the command, with the command's failure document.
    let bad = s
        .cmd()
        .args(["format", "list", "--json", "--log"])
        .arg(s.join("no/such/dir/x.log"))
        .output()
        .unwrap();
    assert_eq!(code(&bad), 100);
    let doc = json(&bad);
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["schema"], "hikasa.format_list.v1");
    assert_eq!(doc["error"]["kind"], "not_found", "{doc}");
    let folder = s
        .cmd()
        .args(["version", "--json", "--log"])
        .arg(s.path())
        .output()
        .unwrap();
    assert_eq!(code(&folder), 100);
    let doc = json(&folder);
    assert_eq!(doc["schema"], "hikasa.version.v1");
    assert_eq!(doc["exit_code"], 100);
    assert_eq!(doc["error"]["kind"], "is_a_directory", "{doc}");
    assert_eq!(stderr(&folder).lines().count(), 1, "{}", stderr(&folder));
}

#[test]
fn json_on_error_is_still_one_document() {
    let s = Sandbox::new();
    s.file("a.txt", "a\n");
    for args in [
        &["quick", "a.txt", "missing.txt"][..],
        &["text", "compare", "a.txt", "missing.txt"],
        &["folder", "compare", "missing", "."],
        &["format", "show", "no-such-format"],
        &["vfs", "stat", "missing"],
        &["gui", "ping"],
        &["text", "compare", "a.txt", "a.txt", "--display", "bogus"],
    ] {
        let out = s.cmd().args(args).arg("--json").output().unwrap();
        let doc = json(&out);
        assert_eq!(doc["ok"], false, "{args:?}");
        assert!(doc["result"].is_null());
        assert_eq!(doc["exit_code"].as_i64().unwrap(), i64::from(code(&out)));
        assert!(doc["error"]["message"].is_string());
        assert!(!stderr(&out).is_empty(), "one stderr line too");
    }
}

#[test]
fn configuration_errors() {
    let s = Sandbox::new();
    let empty = s
        .cmd()
        .args(["format", "list", "--config-dir", ""])
        .output()
        .unwrap();
    assert_eq!(code(&empty), 100, "an empty --config-dir is refused");
    s.options("schema_version = 99\n");
    let newer = s.cmd().args(["format", "list", "--json"]).output().unwrap();
    assert_eq!(code(&newer), 100, "a newer options.toml stops the program");
    assert_eq!(json(&newer)["schema"], "hikasa.format_list.v1");
    // version and completions read no configuration.
    let version = s.cmd().arg("version").output().unwrap();
    assert_eq!(code(&version), 0);
}
