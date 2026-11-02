//! `hikasa folder op` end to end (M2 W6b, contract §1.6.17): A6.25, the CLI's half of A6.16, A6.17 and A6.22. Each verb in text
//! and `--json`, the selection flags, nothing selected, dry runs equal to runs, exit codes, sessions, and that a plan through
//! the CLI and through the Service give one tree (T11).

mod common;

use std::path::Path;

use common::{Sandbox, assert_golden, code, json, stderr, stdout};

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
            if std::fs::symlink_metadata(&path).unwrap().is_dir() {
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

/// `l/` and `r/`: a pair that differs, an orphan, an orphan folder, and a pair that is the same.
fn sandbox() -> Sandbox {
    let s = Sandbox::new();
    s.file_at("l/changed.txt", "new text\n", 2_000_000);
    s.file_at("r/changed.txt", "old\n", 1_000_000);
    s.file_at("l/same.txt", "same\n", 1_000_000);
    s.file_at("r/same.txt", "same\n", 1_000_000);
    s.file_at("l/only.txt", "only\n", 1_000_000);
    s.file_at("l/sub/inner.txt", "inner\n", 1_000_000);
    s.file_at("r/ronly.txt", "right\n", 1_000_000);
    s.settle("l");
    s.settle("r");
    s
}

fn op(s: &Sandbox, args: &[&str]) -> std::process::Output {
    let mut command = s.cmd();
    command.args(["folder", "op"]);
    command.arg(args[0]);
    command.arg(s.join("l"));
    command.arg(s.join("r"));
    command.args(&args[1..]);
    command.output().unwrap()
}

/// The folder `s` with its bytes.
fn state(s: &Sandbox) -> Vec<(String, Option<Vec<u8>>)> {
    let mut all = tree(&s.join("l"));
    all.extend(tree(&s.join("r")));
    all
}

/// One text golden and one JSON golden of a verb on fresh sandboxes.
fn golden(name: &str, args: &[&str]) {
    let s = sandbox();
    let out = op(&s, args);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_golden(
        &format!("folder_op/{name}.txt"),
        &s.normalise(&stdout(&out)),
    );
    let s = sandbox();
    let mut with_json = args.to_vec();
    with_json.push("--json");
    let out = op(&s, &with_json);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.folder_op.v1");
    assert_eq!(doc["ok"], true);
    assert_golden(
        &format!("folder_op/{name}.json"),
        &s.normalise(&stdout(&out)),
    );
}

#[test]
fn a6_25_copy() {
    golden(
        "copy",
        &[
            "copy",
            "--to",
            "right",
            "--select",
            "left.diff.files",
            "--select",
            "left.orphan",
            "--yes",
        ],
    );
}

#[test]
fn a6_25_move() {
    golden(
        "move",
        &["move", "--to", "right", "--path", "only.txt", "--yes"],
    );
}

#[test]
fn a6_25_exchange() {
    golden("exchange", &["exchange", "--path", "changed.txt", "--yes"]);
}

#[test]
fn a6_25_delete() {
    golden(
        "delete",
        &["delete", "--side", "left", "--path", "only.txt", "--yes"],
    );
}

#[test]
fn a6_25_rename() {
    golden(
        "rename",
        &[
            "rename", "--side", "left", "--path", "only.txt", "--mask", "*_old.*",
        ],
    );
}

#[test]
fn a6_25_touch() {
    golden(
        "touch",
        &[
            "touch",
            "--side",
            "right",
            "--path",
            "same.txt",
            "--time",
            "2026-01-02T03:04:05Z",
        ],
    );
}

#[test]
fn a6_25_mkdir() {
    golden("mkdir", &["mkdir", "--side", "both", "--name", "new"]);
}

#[test]
fn a6_25_copyto() {
    let s = sandbox();
    let target = s.join("out");
    let out = op(
        &s,
        &[
            "copyto",
            "--side",
            "left",
            "--path",
            "sub/inner.txt",
            "--target",
            target.to_str().unwrap(),
            "--structure",
            "base",
            "--yes",
            "--json",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_golden("folder_op/copyto.json", &s.normalise(&stdout(&out)));
    assert_eq!(
        std::fs::read(target.join("sub/inner.txt")).unwrap(),
        b"inner\n"
    );
    assert!(s.join("l/sub/inner.txt").exists());
}

#[test]
fn a6_25_moveto() {
    let s = sandbox();
    let target = s.join("out");
    let out = op(
        &s,
        &[
            "moveto",
            "--side",
            "left",
            "--path",
            "sub",
            "--target",
            target.to_str().unwrap(),
            "--yes",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        std::fs::read(target.join("sub/inner.txt")).unwrap(),
        b"inner\n"
    );
    assert!(!s.join("l/sub").exists());
}

#[test]
fn a6_25_compare_and_crc_schemas() {
    let s = sandbox();
    let out = op(
        &s,
        &[
            "compare",
            "--method",
            "binary",
            "--select",
            "all.files",
            "--json",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.folder_contents.v1");
    assert_eq!(doc["result"]["op"], "compare");
    assert_eq!(doc["result"]["method"], "binary");
    assert_eq!(doc["result"]["counts"]["binary_same"], 1);
    assert_eq!(doc["result"]["counts"]["binary_different"], 1);
    let out = op(
        &s,
        &["crc", "--side", "left", "--path", "same.txt", "--json"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.folder_contents.v1");
    assert_eq!(doc["result"]["items"][0]["side"], "left");
    assert_eq!(
        doc["result"]["items"][0]["crc32"].as_str().unwrap().len(),
        8
    );
    let text = op(&s, &["compare", "--method", "binary", "--path", "same.txt"]);
    assert!(
        stdout(&text).contains("binary_same same.txt"),
        "{}",
        stdout(&text)
    );
}

#[test]
fn a6_25_nothing_selected_is_100_and_touches_nothing() {
    let s = sandbox();
    let before = state(&s);
    let out = op(&s, &["copy", "--to", "right", "--yes"]);
    assert_eq!(code(&out), 100);
    assert!(
        stderr(&out).contains("nothing selected"),
        "{}",
        stderr(&out)
    );
    assert_eq!(state(&s), before);
    let out = op(&s, &["copy", "--to", "right", "--yes", "--json"]);
    let doc = json(&out);
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["error"]["kind"], "invalid_input");
}

#[test]
fn a6_25_a_bad_word_is_100_with_nothing_written() {
    let s = sandbox();
    let before = state(&s);
    for args in [
        &["copy", "--to", "up", "--select", "all", "--yes"][..],
        &["copy", "--to", "right", "--select", "bogus", "--yes"][..],
        &["delete", "--side", "nowhere", "--select", "all", "--yes"][..],
        &[
            "copy",
            "--to",
            "right",
            "--overwrite",
            "maybe",
            "--select",
            "all",
            "--yes",
        ][..],
    ] {
        assert_eq!(code(&op(&s, args)), 100, "{args:?}");
    }
    assert_eq!(state(&s), before);
}

#[test]
fn a6_25_an_unknown_path_is_100_not_found() {
    let s = sandbox();
    let out = op(
        &s,
        &[
            "delete", "--side", "left", "--path", "ghost", "--yes", "--json",
        ],
    );
    assert_eq!(code(&out), 100);
    assert_eq!(json(&out)["error"]["kind"], "not_found");
}

#[test]
fn a6_25_path_prefixes_name_a_side() {
    let s = sandbox();
    let out = op(
        &s,
        &[
            "delete",
            "--side",
            "both",
            "--path",
            "left:same.txt",
            "--yes",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!s.join("l/same.txt").exists());
    assert!(s.join("r/same.txt").exists());
    let s = sandbox();
    let out = op(
        &s,
        &["delete", "--side", "both", "--path", "same.txt", "--yes"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!s.join("l/same.txt").exists());
    assert!(!s.join("r/same.txt").exists());
}

#[test]
fn a6_25_no_expand_sees_folders_as_one_row_and_expand_opens_them() {
    let s = sandbox();
    let list = |out: &std::process::Output| {
        json(out)["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["path"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let out = op(
        &s,
        &[
            "copy",
            "--to",
            "right",
            "--select",
            "left.orphan.files",
            "--no-expand",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(list(&out), ["only.txt"]);
    let out = op(
        &s,
        &[
            "copy",
            "--to",
            "right",
            "--select",
            "left.orphan.files",
            "--expand",
            "sub",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(list(&out), ["sub", "sub/inner.txt", "only.txt"]);
}

#[test]
fn a6_25_display_narrows_what_select_sees() {
    let s = sandbox();
    let out = op(
        &s,
        &[
            "copy",
            "--to",
            "right",
            "--select",
            "files",
            "--display",
            "left-newer",
            "--dry-run",
            "--json",
        ],
    );
    let doc = json(&out);
    let items = doc["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["path"], "changed.txt");
}

#[test]
fn a6_25_include_hidden_brings_what_filters_hide() {
    let s = sandbox();
    s.file_at("l/sub/hidden.bak", "hide\n", 1_000_000);
    s.settle("l");
    let paths = |extra: &[&str]| {
        let mut args = vec![
            "copy",
            "--to",
            "right",
            "--path",
            "sub",
            "--filters=-*.bak",
            "--dry-run",
            "--json",
        ];
        args.extend_from_slice(extra);
        let out = op(&s, &args);
        json(&out)["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["path"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(paths(&[]), ["sub", "sub/inner.txt"]);
    assert_eq!(
        paths(&["--include-hidden"]),
        ["sub", "sub/hidden.bak", "sub/inner.txt"]
    );
}

#[test]
fn a6_25_a_dry_run_writes_nothing_and_its_items_equal_a_run() {
    let s = sandbox();
    let before = state(&s);
    let args = [
        "copy",
        "--to",
        "right",
        "--select",
        "all",
        "--dry-run",
        "--json",
    ];
    let dry = json(&op(&s, &args));
    assert_eq!(state(&s), before);
    assert_eq!(dry["result"]["dry_run"], true);
    let run = json(&op(
        &s,
        &[
            "copy", "--to", "right", "--select", "all", "--yes", "--json",
        ],
    ));
    assert_eq!(run["result"]["dry_run"], false);
    let strip = |doc: &serde_json::Value| {
        doc["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                let mut item = item.clone();
                item.as_object_mut().unwrap().remove("status");
                item
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(strip(&dry), strip(&run));
    assert!(dry["result"]["items"][0]["status"] == "planned");
    assert!(run["result"]["items"][0]["status"] == "done");
}

#[test]
fn a6_25_a_dry_run_prints_the_conflicts() {
    let s = sandbox();
    s.file_at("r/changed.txt", "newer\n", 3_000_000);
    s.settle("r");
    let out = op(
        &s,
        &[
            "copy",
            "--to",
            "right",
            "--path",
            "changed.txt",
            "--dry-run",
        ],
    );
    assert!(
        stdout(&out).contains("[overwrite_newer]"),
        "{}",
        stdout(&out)
    );
    assert!(
        stdout(&out).contains("would copy 1 items"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn a6_25_yes_and_dry_run_are_refused_together() {
    let s = sandbox();
    let out = op(
        &s,
        &[
            "copy",
            "--to",
            "right",
            "--select",
            "all",
            "--yes",
            "--dry-run",
        ],
    );
    assert_ne!(code(&out), 0);
    assert!(
        stderr(&out).contains("cannot be used with"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a6_25_a_base_that_cannot_be_loaded_is_107() {
    let s = Sandbox::new();
    s.dir("l");
    let mut command = s.cmd();
    let out = command
        .args(["folder", "op", "copy"])
        .arg(s.join("l"))
        .arg(s.join("ghost"))
        .args(["--to", "right", "--select", "all", "--yes"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 107, "{}", stderr(&out));
}

#[test]
fn a6_25_item_failure_keeps_ok_true_and_exits_100() {
    let s = sandbox();
    // A folder stands where a row writes a file: the item fails at planning.
    std::fs::create_dir_all(s.join("r/only.txt/x")).unwrap();
    let out = op(
        &s,
        &[
            "copy",
            "--to",
            "right",
            "--path",
            "only.txt",
            "--overwrite",
            "fail",
            "--yes",
            "--json",
        ],
    );
    assert_eq!(code(&out), 100);
    let doc = json(&out);
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["error"], serde_json::Value::Null);
    assert_eq!(doc["result"]["summary"]["failed"], 1);
    assert_eq!(doc["result"]["items"][0]["status"], "failed");
}

#[test]
fn a6_16_no_terminal_no_yes_is_100_and_names_the_questions() {
    let s = sandbox();
    let before = state(&s);
    let mut command = s.cmd();
    let out = command
        .args(["folder", "op", "delete"])
        .arg(s.join("l"))
        .arg(s.join("r"))
        .args(["--side", "left", "--path", "only.txt"])
        .write_stdin("")
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    let text = stderr(&out);
    assert!(text.contains("1 questions need an answer"), "{text}");
    assert!(text.contains("--yes or --confirm no-to-all"), "{text}");
    assert!(text.contains("operation 1"), "{text}");
    assert_eq!(state(&s), before);
}

#[test]
fn a6_16_delete_asks_with_confirm_delete_false() {
    let s = sandbox();
    s.options("[file_operations]\nconfirm_delete = false\n");
    let before = state(&s);
    let mut command = s.cmd();
    let out = command
        .args(["folder", "op", "delete"])
        .arg(s.join("l"))
        .arg(s.join("r"))
        .args(["--side", "left", "--path", "only.txt"])
        .write_stdin("")
        .output()
        .unwrap();
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert_eq!(state(&s), before);
}

#[test]
fn a6_16_no_to_all_lets_the_items_without_a_conflict_go() {
    let s = sandbox();
    s.file_at("r/changed.txt", "newer\n", 3_000_000);
    s.settle("r");
    let out = op(
        &s,
        &[
            "copy",
            "--to",
            "right",
            "--select",
            "left.diff.files",
            "--select",
            "left.orphan.files",
            "--confirm",
            "no-to-all",
            "--json",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    let items = doc["result"]["items"].as_array().unwrap();
    let changed = items.iter().find(|i| i["path"] == "changed.txt").unwrap();
    assert_eq!(changed["action"], "skip");
    assert_eq!(changed["reason"], "declined");
    assert_eq!(changed["answer"], false);
    assert_eq!(std::fs::read(s.join("r/changed.txt")).unwrap(), b"newer\n");
    assert_eq!(std::fs::read(s.join("r/only.txt")).unwrap(), b"only\n");
}

#[test]
fn a6_17_recycle_on_an_archive_with_yes() {
    let s = Sandbox::new();
    s.dir("l");
    s.dir("r");
    let archive = s.join("l/a.zip");
    let made = s
        .cmd()
        .args(["vfs", "mkdir"])
        .arg(&archive)
        .output()
        .unwrap();
    assert_eq!(code(&made), 0, "{}", stderr(&made));
    for name in ["a.txt", "b.txt"] {
        let file = s.file(name, "data");
        let copied = s
            .cmd()
            .args(["vfs", "cp"])
            .arg(file)
            .arg(format!("{}/{name}", archive.display()))
            .output()
            .unwrap();
        assert_eq!(code(&copied), 0, "{}", stderr(&copied));
    }
    let mut command = s.cmd();
    let out = command
        .args(["folder", "op", "delete"])
        .arg(s.join("l"))
        .arg(s.join("r"))
        .args([
            "--archives",
            "folders",
            "--side",
            "left",
            "--path",
            "a.zip/a.txt",
            "--recycle",
            "--yes",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["result"]["items"][0]["action"], "remove");
    assert_eq!(doc["result"]["items"][0]["conflicts"][0], "no_recycle_bin");
    assert_eq!(doc["result"]["items"][0]["answer"], true);
    let listed = s
        .cmd()
        .args(["vfs", "ls"])
        .arg(s.join("l/a.zip"))
        .output()
        .unwrap();
    assert!(stdout(&listed).contains("b.txt"));
    assert!(!stdout(&listed).contains("a.txt"), "{}", stdout(&listed));
}

fn session(s: &Sandbox, name: &str, extra: &[&str]) {
    let out = s
        .cmd()
        .args(["session", "new", name, "--type", "folder-compare", "--left"])
        .arg(s.join("l"))
        .arg("--right")
        .arg(s.join("r"))
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    for setting in extra {
        let out = s
            .cmd()
            .args(["session", "edit", name, "--set", setting])
            .output()
            .unwrap();
        assert_eq!(code(&out), 0, "{}", stderr(&out));
    }
}

#[test]
fn a6_25_session_gives_the_sides_and_flags_go_on_top() {
    let s = sandbox();
    session(&s, "work", &[]);
    let mut command = s.cmd();
    let out = command
        .args(["folder", "op", "copy", "--session", "work"])
        .args([
            "--to",
            "right",
            "--select",
            "left.orphan.files",
            "--filters=-sub\\",
            "--yes",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(s.join("r/only.txt").exists());
    assert!(!s.join("r/sub").exists());
}

#[test]
fn a6_21_a_sessions_read_only_and_the_flag_are_refused_with_100() {
    let s = sandbox();
    session(&s, "ro", &["specs.read_only_right=true"]);
    let before = state(&s);
    let mut command = s.cmd();
    let out = command
        .args(["folder", "op", "copy", "--session", "ro"])
        .args(["--to", "right", "--select", "all", "--yes", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    assert_eq!(json(&out)["error"]["kind"], "permission_denied");
    assert_eq!(state(&s), before);
    let out = op(
        &s,
        &[
            "delete",
            "--side",
            "left",
            "--select",
            "all",
            "--read-only",
            "both",
            "--yes",
        ],
    );
    assert_eq!(code(&out), 100);
    assert_eq!(state(&s), before);
    // CopyTo out of a read-only side is allowed.
    let target = s.join("out");
    let out = op(
        &s,
        &[
            "copyto",
            "--side",
            "left",
            "--path",
            "only.txt",
            "--target",
            target.to_str().unwrap(),
            "--read-only",
            "left",
            "--yes",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
}

#[test]
fn a6_22_exclude_through_the_cli() {
    let s = sandbox();
    session(&s, "ex", &[]);
    let mut command = s.cmd();
    let out = command
        .args(["folder", "op", "exclude", "--session", "ex"])
        .args(["--path", "only.txt", "--dry-run", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.session_write.v1");
    assert_eq!(doc["result"]["masks"]["files"][0], ".\\only.txt");
    assert_eq!(doc["result"]["written"].as_array().unwrap().len(), 0);
    let mut command = s.cmd();
    let out = command
        .args(["folder", "op", "exclude", "--session", "ex"])
        .args(["--path", "only.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    // A re-comparison hides the item.
    let show = s
        .cmd()
        .args(["session", "show", "ex", "--json"])
        .output()
        .unwrap();
    assert!(stdout(&show).contains("only.txt"), "{}", stdout(&show));
    let mut command = s.cmd();
    let out = command
        .args(["folder", "op", "copy", "--session", "ex"])
        .args([
            "--to",
            "right",
            "--select",
            "left.orphan.files",
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    let doc = json(&out);
    let names: Vec<&str> = doc["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["path"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["sub", "sub/inner.txt"]);
    // Refuses a locked session without --force (D-91).
    let lock = s.cmd().args(["session", "lock", "ex"]).output().unwrap();
    assert_eq!(code(&lock), 0);
    let mut command = s.cmd();
    let out = command
        .args([
            "folder",
            "op",
            "exclude",
            "--session",
            "ex",
            "--what",
            "type",
        ])
        .args(["--path", "only.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    assert!(stderr(&out).contains("locked"), "{}", stderr(&out));
    let mut command = s.cmd();
    let out = command
        .args([
            "folder",
            "op",
            "exclude",
            "--session",
            "ex",
            "--what",
            "type",
            "--force",
        ])
        .args(["--path", "only.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
}

#[test]
fn a6_25_exclude_and_ignored_need_a_session_unless_dry_run() {
    let s = sandbox();
    let out = op(&s, &["exclude", "--path", "only.txt"]);
    assert_eq!(code(&out), 100);
    let out = op(&s, &["ignored", "--path", "only.txt"]);
    assert_eq!(code(&out), 100);
    let out = op(
        &s,
        &["ignored", "--path", "only.txt", "--dry-run", "--json"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(json(&out)["result"]["ignored"][0], "only.txt");
}

#[test]
fn a6_23_open_with_dry_run_prints_the_commands() {
    let s = sandbox();
    std::fs::create_dir_all(s.config()).unwrap();
    std::fs::write(
        s.config().join("openwith.toml"),
        "schema_version = 1\n[[app]]\ndescription = \"view\"\ncommand = \"viewer %n\"\n",
    )
    .unwrap();
    let out = op(
        &s,
        &[
            "open-with",
            "--path",
            "only.txt",
            "--app",
            "view",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["result"]["commands"][0]["program"], "viewer");
    assert_eq!(doc["result"]["commands"][0]["args"][0], "only.txt");
    let out = op(
        &s,
        &[
            "open-with",
            "--path",
            "only.txt",
            "--associated",
            "--dry-run",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("only.txt"), "{}", stdout(&out));
    let out = op(
        &s,
        &[
            "open-with",
            "--path",
            "only.txt",
            "--app",
            "ghost",
            "--dry-run",
        ],
    );
    assert_eq!(code(&out), 100);
}

#[test]
fn a6_25_help_and_the_flags_of_the_rules() {
    let s = Sandbox::new();
    let out = s.cmd().args(["help", "folder", "op"]).output().unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("folder op"), "{}", stdout(&out));
}

#[test]
fn a6_25_t11_the_cli_and_the_service_give_one_tree() {
    use std::sync::Arc;

    use hikasa_core::{JobKind, NullProgress};
    use hikasa_folder::NullObserver;
    use hikasa_folder::ops::{ConfirmMode, FolderOp, ReadOnlySides};
    use hikasa_formats::StdProcessRunner;
    use hikasa_session::api::{
        FolderCompareRequest, FolderOpRequest, OpOverrides, OpRequest, SelectRequest,
    };
    use hikasa_session::{EnvFacts, FixedClock, Platform, Service, ServiceInit};
    use hikasa_vfs::ops::OverwritePolicy;

    let by_cli = sandbox();
    let args = ["copy", "--to", "right", "--select", "all", "--yes"];
    assert_eq!(code(&op(&by_cli, &args)), 0);
    let by_service = sandbox();
    let root = by_service.path();
    let facts = EnvFacts {
        cwd: Some(root.to_path_buf()),
        ..EnvFacts::new(Platform::HOST, root.to_path_buf())
    };
    let clock = Arc::new(FixedClock(
        "2026-11-02T00:00:00+00:00[UTC]".parse().unwrap(),
    ));
    let init = ServiceInit {
        config_dir: Some(by_service.config()),
        ..ServiceInit::new(facts, clock, Arc::new(StdProcessRunner))
    };
    let service = Service::new(init).unwrap();
    let job = service.job(JobKind::FolderCompare, Arc::new(NullProgress));
    let request = FolderCompareRequest {
        left: root.join("l").into_os_string(),
        right: root.join("r").into_os_string(),
        ..FolderCompareRequest::default()
    };
    let prepared = service.folder_prepare_in(&request, &job).unwrap();
    service.folder_run(&prepared, &job, &NullObserver).unwrap();
    let plan = service
        .folder_op_plan(
            prepared.model(),
            &FolderOpRequest {
                op: OpRequest::Op(FolderOp::Copy {
                    to: hikasa_core::Side::Right,
                    overwrite: OverwritePolicy::Always,
                }),
                select: SelectRequest {
                    masks: vec!["all".to_owned()],
                    ..SelectRequest::default()
                },
                read_only: ReadOnlySides::default(),
                overrides: OpOverrides::default(),
            },
            &job,
        )
        .unwrap();
    let plan = service
        .folder_confirmation(&plan, Some(ConfirmMode::YesToAll))
        .finish(plan)
        .unwrap();
    service.folder_op_execute(plan, &job).unwrap();
    assert_eq!(state(&by_cli), state(&by_service));
}
