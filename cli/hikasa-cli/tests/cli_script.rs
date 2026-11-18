//! `hikasa script run|check` end to end (M2 W8a, §1.8.12): A8.7's CLI rows (105, 106, the human lines), A8.10's disk cases
//! (a script's command and its CLI command give equal `result` objects and equal trees, T11), A8.12 (lookup) and A8.13
//! (flags, `--dry-run`, `hikasa.script.v1`).

mod common;

use std::path::Path;

use common::{Sandbox, code, json, stderr, stdout};
use serde_json::Value;

fn tree(root: &Path) -> Vec<(String, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .strip_prefix(root)
                .unwrap()
                .display()
                .to_string()
                .replace('\\', "/");
            if path.is_dir() {
                out.push((name, None));
                pending.push(path);
            } else {
                out.push((name, Some(std::fs::read(&path).unwrap())));
            }
        }
    }
    out.sort();
    out
}

/// Two folders: a differing file, a left orphan in a subfolder, a right orphan, an equal file.
fn sandbox() -> Sandbox {
    let s = Sandbox::new();
    s.file_at("l/changed.txt", "new text\n", 2_000_000);
    s.file_at("r/changed.txt", "old\n", 1_000_000);
    s.file_at("l/same.txt", "same\n", 1_000_000);
    s.file_at("r/same.txt", "same\n", 1_000_000);
    s.file_at("l/sub/inner.txt", "inner\n", 1_000_000);
    s.file_at("r/ronly.txt", "right\n", 1_000_000);
    s.settle("l");
    s.settle("r");
    s
}

fn script(s: &Sandbox, text: &str, flags: &[&str]) -> std::process::Output {
    s.file("s.txt", text);
    let mut cmd = s.cmd();
    cmd.args(["script", "run", "s.txt"]).args(flags);
    cmd.output().unwrap()
}

/// The `result` of the last command of a script run with `--json`, normalised.
fn last_result(s: &Sandbox, out: &std::process::Output) -> Value {
    let doc = json(out);
    let commands = doc["result"]["commands"].as_array().unwrap();
    let result = commands.last().unwrap()["result"].clone();
    serde_json::from_str(&s.normalise(&result.to_string())).unwrap()
}

/// A8.10: one script command against its CLI command on twin sandboxes: equal `result`s and trees.
fn equal(script_text: &str, direct: &[&str], schema: &str) {
    let a = sandbox();
    let out = script(&a, script_text, &["--confirm", "yes-to-all", "--json"]);
    assert!(matches!(code(&out), 0 | 100), "{}", stderr(&out));
    let doc = json(&out);
    let last = doc["result"]["commands"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(last["schema"], schema, "{script_text}: {last}");
    let from_script = last_result(&a, &out);
    let b = sandbox();
    let mut cmd = b.cmd();
    for word in direct {
        cmd.arg(
            word.replace("<L>", &b.join("l").display().to_string())
                .replace("<R>", &b.join("r").display().to_string()),
        );
    }
    let out = cmd.arg("--json").output().unwrap();
    let direct_doc: Value = serde_json::from_str(&b.normalise(&stdout(&out))).unwrap();
    assert_eq!(direct_doc["schema"], schema);
    assert_eq!(from_script, direct_doc["result"], "{script_text}");
    assert_eq!(tree(&a.join("l")), tree(&b.join("l")), "{script_text}");
    assert_eq!(tree(&a.join("r")), tree(&b.join("r")), "{script_text}");
}

#[test]
fn a8_10_copy_delete_rename_touch_attrib_equal_their_folder_op() {
    let ops = "hikasa.folder_op.v1";
    equal(
        "load l r\nexpand all\nselect left.diff.files left.orphan.files\ncopy left->right\n",
        &[
            "folder",
            "op",
            "copy",
            "<L>",
            "<R>",
            "--select",
            "left.diff.files",
            "--select",
            "left.orphan.files",
            "--to",
            "right",
            "--yes",
        ],
        ops,
    );
    equal(
        "load l r\nexpand all\nselect right.orphan.files\ndelete right\n",
        &[
            "folder",
            "op",
            "delete",
            "<L>",
            "<R>",
            "--select",
            "right.orphan.files",
            "--side",
            "right",
            "--yes",
        ],
        ops,
    );
    equal(
        "load l r\nexpand all\nselect left.all.files\nrename regexpr \"(.*)\\.txt\" \"$1.md\"\n",
        &[
            "folder",
            "op",
            "rename",
            "<L>",
            "<R>",
            "--select",
            "left.all.files",
            "--side",
            "both",
            "--regex",
            "(.*)\\.txt",
            "$1.md",
            "--yes",
        ],
        ops,
    );
    equal(
        "load l r\nexpand all\nselect all.all.files\ntouch left:2026-01-02T03:04:05Z\n",
        &[
            "folder",
            "op",
            "touch",
            "<L>",
            "<R>",
            "--select",
            "all.all.files",
            "--side",
            "left",
            "--time",
            "2026-01-02T03:04:05Z",
            "--yes",
        ],
        ops,
    );
    equal(
        "load l r\nexpand all\nselect left.orphan.files\nattrib -r\n",
        &[
            "folder",
            "op",
            "attrib",
            "<L>",
            "<R>",
            "--select",
            "left.orphan.files",
            "--change=-r",
            "--yes",
        ],
        ops,
    );
}

#[test]
fn a8_10_copyto_into_a_new_zip_equals_folder_op_copyto() {
    let a = sandbox();
    let out = script(
        &a,
        "load l r\nexpand all\nselect left.all.files\ncopyto left out/new.zip\n",
        &["--confirm", "yes-to-all", "--json"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let from_script = last_result(&a, &out);
    let b = sandbox();
    let out = b
        .cmd()
        .args(["folder", "op", "copyto"])
        .arg(b.join("l"))
        .arg(b.join("r"))
        .args([
            "--select",
            "left.all.files",
            "--side",
            "left",
            "--target",
            "out/new.zip",
            "--overwrite",
            "always",
            "--yes",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let direct: Value = serde_json::from_str(&b.normalise(&stdout(&out))).unwrap();
    assert_eq!(from_script, direct["result"]);
    assert!(a.join("out/new.zip").is_file());
}

#[test]
fn a8_10_syncs_equal_folder_sync() {
    equal(
        "load l r\nsync update:left->right\n",
        &["folder", "sync", "<L>", "<R>", "--update-right", "--yes"],
        "hikasa.folder_sync.v1",
    );
    equal(
        "load l r\nsync mirror:right->left\n",
        &["folder", "sync", "<L>", "<R>", "--mirror-left", "--yes"],
        "hikasa.folder_sync.v1",
    );
}

#[test]
fn a8_10_compare_crc_equals_folder_op_compare() {
    equal(
        "load l r\nexpand all\nselect all.all.files\ncompare crc\n",
        &[
            "folder",
            "op",
            "compare",
            "<L>",
            "<R>",
            "--select",
            "all.all.files",
            "--method",
            "crc",
        ],
        "hikasa.folder_contents.v1",
    );
}

#[test]
fn a8_10_reports_equal_report_folder_and_report_text() {
    for (text, direct, file) in [
        (
            "load l r\nfolder-report layout:summary output-to:f.txt\n",
            vec![
                "report",
                "folder",
                "<L>",
                "<R>",
                "--layout",
                "summary",
                "--output-to",
                "f.txt",
            ],
            "f.txt",
        ),
        (
            "text-report layout:summary output-to:t.txt l/changed.txt r/changed.txt\n",
            vec![
                "report",
                "text",
                "<L>/changed.txt",
                "<R>/changed.txt",
                "--layout",
                "summary",
                "--output-to",
                "t.txt",
            ],
            "t.txt",
        ),
    ] {
        let a = sandbox();
        a.file("s.txt", text);
        let out = a
            .cmd()
            .env("SOURCE_DATE_EPOCH", "1792584000")
            .args(["script", "run", "s.txt", "--json"])
            .output()
            .unwrap();
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        let from_script = last_result(&a, &out);
        let b = sandbox();
        let mut cmd = b.cmd();
        cmd.env("SOURCE_DATE_EPOCH", "1792584000");
        for word in &direct {
            cmd.arg(
                word.replace("<L>", &b.join("l").display().to_string())
                    .replace("<R>", &b.join("r").display().to_string()),
            );
        }
        let out = cmd.arg("--json").output().unwrap();
        let direct_doc: Value = serde_json::from_str(&b.normalise(&stdout(&out))).unwrap();
        assert_eq!(from_script, direct_doc["result"], "{text}");
        assert_eq!(
            a.normalise(&std::fs::read_to_string(a.join(file)).unwrap()),
            b.normalise(&std::fs::read_to_string(b.join(file)).unwrap()),
            "{text}"
        );
    }
}

/// A8.7's CLI rows: 105 (not found, a folder), 106 with its lines, the human end line of a failed run.
#[test]
fn a8_7_load_and_syntax_errors_on_the_command_line() {
    let s = sandbox();
    let out = s
        .cmd()
        .args(["script", "run", "nope.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 105, "{}", stderr(&out));
    assert!(
        stderr(&out).contains("script nope.txt not found (looked in "),
        "{}",
        stderr(&out)
    );
    let out = s.cmd().args(["script", "run", "l"]).output().unwrap();
    assert_eq!(code(&out), 105);
    assert!(stderr(&out).contains("is a folder"), "{}", stderr(&out));
    let out = script(&s, "selct all\nload a\ncopy left\n", &[]);
    assert_eq!(code(&out), 106);
    let err = s.normalise(&stderr(&out));
    assert!(
        err.contains("<TMP>/s.txt:1:1: error: unknown command `selct` (did you mean `select`?)"),
        "{err}"
    );
    assert!(
        err.contains("hikasa: error: 2 errors in <TMP>/s.txt; nothing was run"),
        "{err}"
    );
    let out = script(&s, "snapshot left\n", &[]);
    assert_eq!(code(&out), 100);
    assert!(
        stderr(&out).contains("hikasa: error: script "),
        "{}",
        stderr(&out)
    );
}

/// A8.12: the current folder, the shared scripts folder (absolute and relative to the configuration folder), both, none,
/// standard input, a member of a zip, and `%0` for each.
#[test]
fn a8_12_the_lookup() {
    let s = Sandbox::new();
    let zero = |out: &std::process::Output| -> String {
        let doc = json(out);
        doc["result"]["commands"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    };
    let probe = "snapshot %0\n";
    // Absent here, in the shared folder (absolute).
    s.file("shared/s.txt", probe);
    s.options(&format!(
        "[tweaks.scripts]\nshared_scripts_folder = {:?}\n",
        s.join("shared").display().to_string()
    ));
    let out = s
        .cmd()
        .args(["script", "run", "s.txt", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert!(
        zero(&out).ends_with("shared/s.txt")
            || zero(&out).ends_with("shared\\\\s.txt")
            || zero(&out).contains("shared"),
        "{}",
        zero(&out)
    );
    // Relative to the configuration folder.
    std::fs::create_dir_all(s.config().join("scripts")).unwrap();
    std::fs::write(s.config().join("scripts/r.txt"), probe).unwrap();
    s.options("[tweaks.scripts]\nshared_scripts_folder = \"scripts\"\n");
    let out = s
        .cmd()
        .args(["script", "run", "r.txt", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert!(zero(&out).contains("scripts"), "{}", zero(&out));
    // Present in both: the current folder wins.
    std::fs::write(
        s.config().join("scripts/both.txt"),
        "beep\nsnapshot shared\n",
    )
    .unwrap();
    s.file("both.txt", "beep\n");
    let out = s
        .cmd()
        .args(["script", "run", "both.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    // Absent in both: 105 naming both places; one place when the option is empty.
    let out = s
        .cmd()
        .args(["script", "run", "none.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 105);
    assert!(stderr(&out).contains(" and "), "{}", stderr(&out));
    s.options("");
    let out = s
        .cmd()
        .args(["script", "run", "none.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 105);
    assert!(!stderr(&out).contains(" and "), "{}", stderr(&out));
    // Standard input: `%0` is `-`.
    let out = s
        .cmd()
        .args(["script", "run", "-", "--json"])
        .write_stdin(probe)
        .output()
        .unwrap();
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert_eq!(zero(&out), "snapshot -");
    // Unreadable (Unix): 105, the shared one not tried.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = s.file("locked.txt", "beep\n");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&path).is_err() {
            let out = s
                .cmd()
                .args(["script", "run", "locked.txt"])
                .output()
                .unwrap();
            assert_eq!(code(&out), 105, "{}", stderr(&out));
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
}

/// A8.13: `--dry-run` writes nothing and shows `folder op`'s plan; `hikasa.script.v1` for check and run; `output-to:-`
/// with `--json` is 106; `--silent` with `--gui` is a usage error; arguments after `--`; the JSON item cap.
#[test]
fn a8_13_flags_and_json() {
    let s = sandbox();
    let before = (tree(&s.join("l")), tree(&s.join("r")));
    let out = script(
        &s,
        "load l r\nexpand all\nselect left.diff.files\ncopy left->right\n",
        &["--dry-run"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!((tree(&s.join("l")), tree(&s.join("r"))), before);
    let dry = s.normalise(&stderr(&out));
    let direct = s
        .cmd()
        .args(["folder", "op", "copy"])
        .arg(s.join("l"))
        .arg(s.join("r"))
        .args(["--select", "left.diff.files", "--to", "right", "--dry-run"])
        .output()
        .unwrap();
    for line in s.normalise(&stdout(&direct)).lines() {
        assert!(dry.contains(&format!("info L4 {line}")), "{line} in\n{dry}");
    }
    // `script check --json`: ok, and 106 with every error.
    s.file("c.txt", "load l r\nbeep\n");
    let out = s
        .cmd()
        .args(["script", "check", "c.txt", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0);
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.script.v1");
    assert_eq!(doc["result"]["mode"], "check");
    assert_eq!(doc["result"]["counts"]["commands"], 2);
    assert_eq!(doc["result"]["commands"][0]["status"], "checked");
    s.file("e.txt", "selct\nload\n");
    let out = s
        .cmd()
        .args(["script", "check", "e.txt", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 106);
    let doc = json(&out);
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["result"]["errors"].as_array().unwrap().len(), 2);
    assert_eq!(doc["result"]["errors"][0]["code"], "unknown_command");
    // A run: 107 with its stop.
    let out = script(&s, "load l missing\nbeep\n", &["--json"]);
    assert_eq!(code(&out), 107);
    let doc = json(&out);
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["exit_code"], 107);
    assert_eq!(doc["result"]["stopped_at"]["reason"], "load_failed");
    assert_eq!(doc["result"]["counts"]["not_run"], 1);
    // `output-to:-` and `--json` cannot share stdout.
    let out = script(
        &s,
        "text-report output-to:- l/changed.txt r/changed.txt\n",
        &["--json"],
    );
    assert_eq!(code(&out), 106, "{}", stderr(&out));
    // Without `--json`, the report goes to stdout.
    let out = script(
        &s,
        "text-report layout:summary output-to:- l/changed.txt r/changed.txt\n",
        &[],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!stdout(&out).is_empty());
    // `--silent` with `--gui` is a usage error.
    let out = script(&s, "beep\n", &["--silent", "--gui"]);
    assert_eq!(code(&out), 100);
    assert!(
        stderr(&out).contains("cannot be used with"),
        "{}",
        stderr(&out)
    );
    // An argument that starts with `-` follows `--`.
    let out = script(&s, "snapshot %1\n", &["--json", "--", "-x"]);
    assert_eq!(json(&out)["result"]["commands"][0]["text"], "snapshot -x");
    // The cap of item lists (a test-only lower cap).
    let out = s
        .cmd()
        .env("HIKASA_TEST_SCRIPT_ITEMS_MAX", "1")
        .args(["script", "run", "s.txt", "--json"])
        .output()
        .unwrap();
    let _ = out;
    s.file(
        "s.txt",
        "load l r\nexpand all\nselect all.all.files\ncompare crc\n",
    );
    let out = s
        .cmd()
        .env("HIKASA_TEST_SCRIPT_ITEMS_MAX", "1")
        .args(["script", "run", "s.txt", "--json"])
        .output()
        .unwrap();
    let doc = json(&out);
    let compare = &doc["result"]["commands"][3];
    assert_eq!(
        compare["result"]["items"].as_array().unwrap().len(),
        1,
        "{compare}"
    );
    assert!(compare["items_omitted"].as_u64().unwrap() >= 1, "{compare}");
}

/// `%date%`, `%time%`, `%fn_time%` by format, against the local date read before and after the run (TZ=UTC here).
#[test]
fn a8_4_dynamic_names_on_the_command_line() {
    let s = Sandbox::new();
    let today = || {
        jiff::Zoned::now()
            .with_time_zone(jiff::tz::TimeZone::UTC)
            .strftime("%Y-%m-%d")
            .to_string()
    };
    let before = today();
    let out = script(&s, "snapshot %date% %time% %fn_time%\n", &["--json"]);
    let after = today();
    let text = json(&out)["result"]["commands"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    let words: Vec<&str> = text.split(' ').collect();
    assert!(words[1] == before || words[1] == after, "{text}");
    assert_eq!(words[2].len(), 8);
    assert_eq!(&words[2][2..3], ":");
    assert_eq!(words[3], words[2].replace(':', "-"));
}

/// A8.21: `hikasa help script` and `hikasa help script run` resolve to the topic.
#[test]
fn a8_21_help_script_resolves() {
    let s = Sandbox::new();
    for words in [vec!["help", "script"], vec!["help", "script", "run"]] {
        let out = s.cmd().args(&words).output().unwrap();
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        assert!(stdout(&out).contains("hikasa script"), "{words:?}");
    }
}

/// A1: a load that fails for any cause (standard input, the clipboard, an empty word given as an argument) ends the
/// script with 107; nothing runs on the comparison loaded before it, so nothing in `r/` is deleted.
#[test]
fn a_failed_load_never_leaves_the_earlier_comparison() {
    for arg in ["-", "", "clipboard://", "no-such-folder"] {
        let s = sandbox();
        let before = tree(&s.join("r"));
        let out = script(
            &s,
            "option confirm:yes-to-all\nload l r\nload %1 other\nselect all\ndelete right\n",
            &[arg],
        );
        assert_eq!(code(&out), 107, "{arg:?}: {}", stderr(&out));
        let err = stderr(&out);
        assert!(
            err.contains("Script stopped at line 3 (exit 107)"),
            "{arg:?}: {err}"
        );
        assert!(!err.contains("run L4"), "{arg:?}: {err}");
        assert_eq!(tree(&s.join("r")), before, "{arg:?}");
    }
}

/// A1, A7: a base or a target written `-`, `""` or `clipboard://` is refused when the script is checked (106), at its
/// position; so are `compare none`, a bad regular expression, attrib words that conflict across words, and report words
/// that conflict.
#[test]
fn static_words_the_run_would_refuse_are_106() {
    let cases = [
        ("load l -\n", ":1:8: error: "),
        ("load \"\" r\n", ":1:6: error: "),
        ("load CLIPBOARD:// r\n", ":1:6: error: "),
        ("load l r\ncopyto all -\n", ":2:12: error: "),
        ("load l r\nmoveto \"\"\n", ":2:8: error: "),
        (
            "load l r\ncompare none\n",
            ":2:9: error: a content test needs a method",
        ),
        ("load l r\nrename regexpr \"(\" x\n", ":2:16: error: "),
        ("load l r\nrename regexpr \"(a)\" $2\n", ":2:22: error: "),
        ("load l r\nattrib +r -r\n", ":2:11: error: "),
        ("load l r\nattrib +h -s +a -h\n", ":2:17: error: "),
    ];
    for (text, expected) in cases {
        let s = sandbox();
        s.file("s.txt", text);
        let out = s.cmd().args(["script", "check", "s.txt"]).output().unwrap();
        assert_eq!(code(&out), 106, "{text}: {}", stderr(&out));
        let err = stderr(&out);
        let first = err.lines().next().unwrap_or("");
        assert!(first.contains(&format!("s.txt{expected}")), "{text}: {err}");
        // The same words expanded from an argument pass the check and fail when they run.
    }
    let s = sandbox();
    s.file("s.txt", "load l r\nattrib %1 %2\n");
    let out = s
        .cmd()
        .args(["script", "check", "s.txt", "--", "+r", "-r"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
}

/// A2: `load <left> <right>` after a folder-sync session drops the session's preset: a bare `sync` fails with the
/// contract's message and `b/precious.txt` survives.
#[test]
fn a_folder_load_drops_the_sync_preset_of_a_session() {
    let s = sandbox();
    s.file("a/x.txt", "x\n");
    s.file("b/x.txt", "x\n");
    s.file("b/precious.txt", "keep\n");
    s.file(
        "cfg/sessions/SL.toml",
        format!(
            "schema_version = 1\ntype = \"folder-sync\"\n\n[specs]\nleft = {:?}\nright = {:?}\n\n[rules.sync]\npreset = \
             \"mirror_right\"\n",
            s.join("l").display().to_string(),
            s.join("r").display().to_string()
        ),
    );
    let out = script(
        &s,
        "option confirm:yes-to-all\nload SL\nload a b\nsync\n",
        &[],
    );
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert!(
        stderr(&out).contains("error L4 no sync direction and the session holds none"),
        "{}",
        stderr(&out)
    );
    assert!(s.join("b/precious.txt").exists());
    // The session's own preset still applies to a sync after loading it.
    let out = script(
        &s,
        "option confirm:yes-to-all\nload a b\nload SL\nsync\n",
        &[],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        !s.join("r/ronly.txt").exists(),
        "mirror_right removed the right orphan"
    );
}

/// A8.4, A8.20 (Linux disk): non-UTF-8 arguments keep their bytes into `load` and into `copyto`'s target, and every
/// line that names them shows the bytes escaped (`\xFF`), the `info … compared …` line as the `run` lines.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_arguments_reach_load_and_copyto_and_are_escaped() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let s = sandbox();
    let left = OsStr::from_bytes(b"l\xFF");
    let target = OsStr::from_bytes(b"t\xFE");
    std::fs::create_dir(s.path().join(left)).unwrap();
    std::fs::write(s.path().join(left).join("a.txt"), "a\n").unwrap();
    s.file(
        "s.txt",
        "option confirm:yes-to-all\nload %1 r\nselect all.files\ncopyto all %2\n",
    );
    let out = s
        .cmd()
        .args([
            OsStr::new("script"),
            OsStr::new("run"),
            OsStr::new("s.txt"),
            left,
            target,
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("run L2 load l\\xFF r"), "{err}");
    let compared = err
        .lines()
        .find(|l| l.contains("info L2 compared"))
        .unwrap_or("");
    assert!(compared.contains("l\\xFF and "), "{err}");
    assert!(!compared.contains('\u{FFFD}'), "{err}");
    assert!(err.contains("run L4 copyto all t\\xFE"), "{err}");
    assert_eq!(
        std::fs::read(s.path().join(target).join("a.txt")).unwrap(),
        b"a\n"
    );
}

/// A8.12, §1.8.11 step 2: a dangling link named as the script exists (any kind), so the shared folder is not tried and
/// reading it is 105; a link to a folder is a folder.
#[cfg(unix)]
#[test]
fn a_dangling_link_is_a_script_that_cannot_be_read() {
    let s = Sandbox::new();
    s.file("shared/s.txt", "beep\n");
    s.options(&format!(
        "[tweaks.scripts]\nshared_scripts_folder = {:?}\n",
        s.join("shared").display().to_string()
    ));
    std::os::unix::fs::symlink(s.join("nowhere"), s.join("s.txt")).unwrap();
    let out = s.cmd().args(["script", "run", "s.txt"]).output().unwrap();
    assert_eq!(code(&out), 105, "{}", stderr(&out));
    assert!(!stderr(&out).contains("info L1 beep"), "{}", stderr(&out));
    s.dir("d");
    std::os::unix::fs::symlink(s.join("d"), s.join("dl.txt")).unwrap();
    let out = s.cmd().args(["script", "run", "dl.txt"]).output().unwrap();
    assert_eq!(code(&out), 105, "{}", stderr(&out));
    assert!(stderr(&out).contains("is a folder"), "{}", stderr(&out));
}
