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

/// [`golden`] for a verb whose words name the sandbox (a `--target`) or need a file there first.
fn golden_in(name: &str, prepare: &dyn Fn(&Sandbox), args: &dyn Fn(&Sandbox) -> Vec<String>) {
    let s = sandbox();
    prepare(&s);
    let words = args(&s);
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let out = op(&s, &words);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_golden(
        &format!("folder_op/{name}.txt"),
        &s.normalise(&stdout(&out)),
    );
    let s = sandbox();
    prepare(&s);
    let mut words = args(&s);
    words.push("--json".to_owned());
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let out = op(&s, &words);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(json(&out)["ok"], true);
    assert_golden(
        &format!("folder_op/{name}.json"),
        &s.normalise(&stdout(&out)),
    );
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|word| (*word).to_owned()).collect()
}

/// A6.25: the goldens of the verbs that are not plans of items, and of `copyto`, `moveto` in text.
#[test]
fn a6_25_goldens_of_the_other_verbs() {
    let nothing = |_: &Sandbox| {};
    let target = |s: &Sandbox| s.join("out").to_str().unwrap().to_owned();
    golden_in("copyto", &nothing, &|s| {
        let mut list = words(&["copyto", "--side", "left", "--path", "sub/inner.txt"]);
        list.extend(["--target".to_owned(), target(s), "--structure".into(), "base".into(), "--yes".into()]);
        list
    });
    golden_in("moveto", &nothing, &|s| {
        let mut list = words(&["moveto", "--side", "left", "--path", "sub"]);
        list.extend(["--target".to_owned(), target(s), "--yes".into()]);
        list
    });
    golden_in("compare", &nothing, &|_| {
        words(&["compare", "--method", "binary", "--select", "all.files"])
    });
    golden_in("crc", &nothing, &|_| {
        words(&["crc", "--side", "both", "--select", "all.files"])
    });
    golden_in("exclude", &nothing, &|_| {
        words(&["exclude", "--what", "items", "--select", "left.orphan", "--dry-run"])
    });
    golden_in("ignored", &nothing, &|_| {
        words(&["ignored", "--path", "only.txt", "--path", "sub", "--dry-run"])
    });
    let app = |s: &Sandbox| {
        std::fs::create_dir_all(s.config()).unwrap();
        std::fs::write(
            s.config().join("openwith.toml"),
            "schema_version = 1\n[[app]]\ndescription = \"view\"\ncommand = \"viewer --line %l %n\"\n",
        )
        .unwrap();
    };
    golden_in("open-with", &app, &|_| {
        words(&["open-with", "--path", "changed.txt", "--side", "both", "--app", "view", "--dry-run"])
    });
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

/// §1.6.11: a folder that replaces a file asks what a file that replaces it asks, and a text dry run says so.
#[test]
fn a6_16_a_folder_that_replaces_a_file_is_a_conflict() {
    let fresh = || {
        let s = Sandbox::new();
        s.file_at("l/n/in", "in", 1_000_000);
        s.settle("l");
        s.file_at("r/n", "my newer work", 4_000_000_000);
        s
    };
    for (verb, words) in [
        ("copy", "[overwrite_newer]"),
        ("move", "[overwrite_newer,replace_during_move]"),
    ] {
        let s = fresh();
        let before = state(&s);
        let out = op(&s, &[verb, "--to", "right", "--select", "all", "--dry-run"]);
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        let text = stdout(&out);
        assert!(text.contains("replace with folder"), "{text}");
        assert!(text.contains(words), "{text}");
        // `no-to-all` keeps it, and what lies below stays by the same answer.
        let out = op(
            &s,
            &[
                verb,
                "--to",
                "right",
                "--select",
                "all",
                "--confirm",
                "no-to-all",
            ],
        );
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        assert_eq!(state(&s), before);
    }
    let s = fresh();
    let out = op(&s, &["move", "--to", "right", "--select", "all", "--yes"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(std::fs::read(s.join("r/n/in")).unwrap(), b"in");
}

/// Rule 6 through the CLI (p02 of the review): filters win, and a folder that was never listed asks as a row does.
#[test]
fn a6_15_rule6_filters_and_questions_through_the_cli() {
    let filtered = |verb: &[&str]| {
        let s = Sandbox::new();
        s.file("l/d/x.txt", "x");
        s.file("l/d/hidden.log", "k");
        s.dir("r");
        let mut args = verb.to_vec();
        args.extend([
            "--path",
            "d",
            "--filters",
            "*.txt",
            "--no-scan-orphans",
            "--yes",
        ]);
        let out = op(&s, &args);
        assert_eq!(code(&out), 0, "{verb:?} {}", stderr(&out));
        s
    };
    let s = filtered(&["delete", "--side", "left"]);
    assert!(s.join("l/d/hidden.log").exists());
    assert!(!s.join("l/d/x.txt").exists());
    let s = filtered(&["move", "--to", "right"]);
    assert!(s.join("l/d/hidden.log").exists());
    assert!(!s.join("r/d/hidden.log").exists());
    assert!(s.join("r/d/x.txt").exists());
    let s = filtered(&["copy", "--to", "right"]);
    assert!(!s.join("r/d/hidden.log").exists());

    let s = Sandbox::new();
    s.file_at("l/p/d/a.txt", "new", 1_000_000);
    s.file_at("r/p/d/a.txt", "newer work", 2_000_000);
    common::set_mode(&s.join("r/p/d/a.txt"), 0o444);
    let flags = ["--to", "right", "--path", "p", "--no-auto-scan"];
    let mut dry = vec!["copy"];
    dry.extend(flags);
    dry.push("--dry-run");
    let out = op(&s, &dry);
    let text = stdout(&out);
    #[cfg(unix)]
    assert!(text.contains("[read_only,overwrite_newer]"), "{text}");
    #[cfg(windows)]
    assert!(text.contains("overwrite_newer"), "{text}");
    let mut keep = vec!["copy"];
    keep.extend(flags);
    keep.extend(["--confirm", "no-to-all"]);
    let out = op(&s, &keep);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(std::fs::read(s.join("r/p/d/a.txt")).unwrap(), b"newer work");
}

/// A folder that cannot go is no reason for what follows it: a plan runs what it showed (p04 of the review).
#[cfg(unix)]
#[test]
fn a6_15_a_skipped_folder_cuts_nothing_of_its_neighbours() {
    let s = Sandbox::new();
    s.file("real/f", "r");
    s.file("l/B/x", "bx");
    s.file("r/B/o", "o");
    std::os::unix::fs::symlink("../real", s.join("l/A")).unwrap();
    let flags = ["--to", "right", "--select", "all", "--follow-symlinks"];
    let mut args = vec!["move"];
    args.extend(flags);
    args.extend(["--yes", "--json"]);
    let out = op(&s, &args);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    let items = doc["result"]["items"].as_array().unwrap();
    assert!(items.iter().all(|item| item["status"] != "not_run"), "{doc}");
    assert_eq!(std::fs::read(s.join("r/B/x")).unwrap(), b"bx");
    assert!(!s.join("l/B").exists());
    assert!(s.join("l/A").exists());
}

/// A zip at `rel` with `members`.
fn zip_at(s: &Sandbox, rel: &str, members: &[&str]) {
    let archive = s.join(rel);
    let made = s.cmd().args(["vfs", "mkdir"]).arg(&archive).output().unwrap();
    assert_eq!(code(&made), 0, "{}", stderr(&made));
    for name in members {
        let file = s.file(&format!("zip-input/{name}"), *name);
        let copied = s
            .cmd()
            .args(["vfs", "cp"])
            .arg(file)
            .arg(format!("{}/{name}", archive.display()))
            .output()
            .unwrap();
        assert_eq!(code(&copied), 0, "{}", stderr(&copied));
    }
}

fn zip_members(s: &Sandbox, rel: &str) -> String {
    let listed = s.cmd().args(["vfs", "ls"]).arg(s.join(rel)).output().unwrap();
    assert_eq!(code(&listed), 0, "{}", stderr(&listed));
    stdout(&listed)
}

/// §1.6.11: a change of a member rewrites the file around it, and a read-only file says so.
#[cfg(unix)]
#[test]
fn a6_16_a_read_only_archive_is_a_conflict() {
    let s = Sandbox::new();
    s.dir("l");
    s.dir("r");
    zip_at(&s, "l/a.zip", &["m.txt", "n.txt"]);
    common::set_mode(&s.join("l/a.zip"), 0o444);
    let flags = [
        "--archives",
        "folders",
        "--side",
        "left",
        "--path",
        "a.zip/m.txt",
    ];
    let mut dry = vec!["delete"];
    dry.extend(flags);
    dry.push("--dry-run");
    let out = op(&s, &dry);
    assert!(stdout(&out).contains("[read_only]"), "{}", stdout(&out));
    let mut keep = vec!["delete"];
    keep.extend(flags);
    keep.extend(["--confirm", "no-to-all"]);
    let before = std::fs::read(s.join("l/a.zip")).unwrap();
    let out = op(&s, &keep);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(std::fs::read(s.join("l/a.zip")).unwrap(), before);
    let mut go = vec!["delete"];
    go.extend(flags);
    go.push("--yes");
    let out = op(&s, &go);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!zip_members(&s, "l/a.zip").contains("m.txt"));
}

/// An opened archive that goes whole is the file it is, onto its pair too: the plan does what it showed (p09).
#[test]
fn a6_15_a_move_of_an_archive_open_on_both_sides() {
    let s = Sandbox::new();
    s.dir("l");
    s.dir("r");
    zip_at(&s, "l/a.zip", &["left.txt"]);
    zip_at(&s, "r/a.zip", &["right.txt"]);
    let flags = ["--to", "right", "--select", "all", "--archives", "folders"];
    let mut dry = vec!["move"];
    dry.extend(flags);
    dry.extend(["--dry-run", "--json"]);
    let shown = op(&s, &dry);
    assert_eq!(code(&shown), 0, "{}", stderr(&shown));
    let items = json(&shown)["result"]["items"].clone();
    assert_eq!(items.as_array().unwrap().len(), 1, "{items}");
    assert_eq!(items[0]["action"], "move");
    assert_eq!(items[0]["replaces"], true);
    let conflicts = items[0]["conflicts"].to_string();
    assert!(conflicts.contains("replace_during_move"), "{conflicts}");
    let mut go = vec!["move"];
    go.extend(flags);
    go.push("--yes");
    let out = op(&s, &go);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!s.join("l/a.zip").exists());
    assert!(zip_members(&s, "r/a.zip").contains("left.txt"));
}

/// The same through `moveto`, and flat: nothing asks to remove an archive's root (p09).
#[test]
fn a6_19_moveto_of_an_opened_archive_goes_as_its_file() {
    for structure in ["relative", "none"] {
        let s = Sandbox::new();
        s.dir("l");
        s.dir("r");
        s.file("out/a.zip", "something else");
        zip_at(&s, "l/a.zip", &["m.txt"]);
        let target = s.join("out");
        let out = op(
            &s,
            &[
                "moveto",
                "--side",
                "left",
                "--select",
                "all",
                "--archives",
                "folders",
                "--target",
                target.to_str().unwrap(),
                "--structure",
                structure,
                "--overwrite",
                "always",
                "--yes",
            ],
        );
        assert_eq!(code(&out), 0, "{structure}: {}", stderr(&out));
        assert!(!s.join("l/a.zip").exists());
        assert!(zip_members(&s, "out/a.zip").contains("m.txt"));
    }
}

/// Flat rows have real folders: names of two folders do not collide, and a missing folder is planned (p08).
#[test]
fn a6_15_a_flat_comparison_uses_real_parents() {
    let s = Sandbox::new();
    s.file("l/a/x", "1");
    s.file("l/b/y", "2");
    s.dir("r/a");
    s.dir("r/b");
    let out = op(
        &s,
        &[
            "rename", "--side", "left", "--select", "all", "--flatten", "--mask", "q*", "--yes",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(s.join("l/a/q").exists() && s.join("l/b/q").exists());
    // A name taken in its own folder still is.
    let s = Sandbox::new();
    s.file("l/a/x", "1");
    s.file("l/a/q", "kept");
    s.dir("r");
    let out = op(
        &s,
        &[
            "rename", "--side", "left", "--select", "all", "--flatten", "--mask", "q*", "--yes",
            "--json",
        ],
    );
    assert_eq!(std::fs::read(s.join("l/a/q")).unwrap(), b"kept");
    assert_eq!(code(&out), 100, "{}", stdout(&out));

    let s = Sandbox::new();
    s.file("l/a/deep/x", "1");
    s.file("l/b/x", "2");
    s.dir("r");
    let out = op(
        &s,
        &[
            "copy", "--to", "right", "--select", "all", "--flatten", "--yes", "--json",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(std::fs::read(s.join("r/a/deep/x")).unwrap(), b"1");
    assert_eq!(std::fs::read(s.join("r/b/x")).unwrap(), b"2");
    let doc = json(&out);
    let made = doc["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["action"] == "create_dir")
        .count();
    assert_eq!(made, 3, "{doc}");
    // `copyto` keeps the folders of a flat row by its structure.
    let target = s.join("kept");
    let out = op(
        &s,
        &[
            "copyto",
            "--side",
            "left",
            "--select",
            "all",
            "--flatten",
            "--target",
            target.to_str().unwrap(),
            "--structure",
            "base",
            "--yes",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(std::fs::read(s.join("kept/a/deep/x")).unwrap(), b"1");
    assert_eq!(std::fs::read(s.join("kept/b/x")).unwrap(), b"2");
}

/// A move onto itself: nothing goes, folders no more than files, by any spelling (p12).
#[test]
fn a6_15_a_move_onto_itself_is_refused() {
    let s = Sandbox::new();
    s.file("l/d/x", "a");
    s.dir("l/e");
    s.dir("r");
    let before = tree(&s.join("l"));
    for right in [s.join("l"), s.join("r/../l")] {
        let out = s
            .cmd()
            .args(["folder", "op", "move"])
            .arg(s.join("l"))
            .arg(&right)
            .args(["--to", "right", "--select", "all", "--yes"])
            .output()
            .unwrap();
        assert_eq!(code(&out), 100, "{}", stdout(&out));
        assert!(stdout(&out).contains("is the source itself"), "{}", stdout(&out));
        assert_eq!(tree(&s.join("l")), before);
    }
}

/// §1.6.5: a file at the backup's name this plan did not make asks, a dry run shows it, and `--yes` replaces it (p13).
#[test]
fn a6_16_a_backup_over_a_file_of_nobody_asks() {
    let s = Sandbox::new();
    s.file_at("l/x.txt", "new", 2_000_000);
    s.file_at("r/x.txt", "old", 1_000_000);
    s.file("r/x.txt.bak", "my own notes");
    let flags = ["--to", "right", "--select", "all", "--backup"];
    let mut args = vec!["copy"];
    args.extend(flags);
    args.push("--dry-run");
    let out = op(&s, &args);
    assert!(stdout(&out).contains("[overwrite_backup]"), "{}", stdout(&out));
    let mut args = vec!["copy"];
    args.extend(flags);
    args.extend(["--confirm", "no-to-all"]);
    let out = op(&s, &args);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(std::fs::read(s.join("r/x.txt")).unwrap(), b"old");
    assert_eq!(std::fs::read(s.join("r/x.txt.bak")).unwrap(), b"my own notes");
    let mut args = vec!["copy"];
    args.extend(flags);
    args.push("--yes");
    let out = op(&s, &args);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(std::fs::read(s.join("r/x.txt")).unwrap(), b"new");
    assert_eq!(std::fs::read(s.join("r/x.txt.bak")).unwrap(), b"old");
    // A backup with nothing at its name asks nothing.
    let s = Sandbox::new();
    s.file_at("l/x.txt", "new", 2_000_000);
    s.file_at("r/x.txt", "old", 1_000_000);
    let mut args = vec!["copy"];
    args.extend(flags);
    args.extend(["--confirm", "no-to-all"]);
    let out = op(&s, &args);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(std::fs::read(s.join("r/x.txt.bak")).unwrap(), b"old");
}

/// A refused move leaves nothing behind (p15), and a folder on the way to nothing is not made.
#[test]
fn a6_19_a_refused_moveto_makes_no_folder() {
    let s = Sandbox::new();
    s.file("l/a/x", "1");
    s.dir("r");
    let before = tree(&s.join("l"));
    let target = s.join("l/a/inside");
    let out = op(
        &s,
        &[
            "moveto",
            "--side",
            "left",
            "--path",
            "a",
            "--target",
            target.to_str().unwrap(),
            "--yes",
        ],
    );
    assert_eq!(code(&out), 100, "{}", stdout(&out));
    assert_eq!(tree(&s.join("l")), before);
    // What is needed stays: a second row goes there.
    s.file("l/b", "2");
    let out = op(
        &s,
        &[
            "moveto",
            "--side",
            "left",
            "--select",
            "all",
            "--target",
            target.to_str().unwrap(),
            "--yes",
        ],
    );
    assert_eq!(code(&out), 100, "{}", stdout(&out));
    assert_eq!(std::fs::read(s.join("l/a/inside/b")).unwrap(), b"2");
}

/// The small refusals: a `rename` into itself (p17.1), an archive copied into itself (p17.2).
#[test]
fn a6_15_a_folder_into_itself_is_refused_at_planning() {
    let s = Sandbox::new();
    s.file("l/sub/a", "b");
    let out = s
        .cmd()
        .args(["folder", "op", "move"])
        .arg(s.join("l"))
        .arg(s.join("l/sub"))
        .args(["--to", "right", "--path", "sub", "--dry-run", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100, "{}", stdout(&out));
    let doc = json(&out);
    assert_eq!(doc["result"]["items"][0]["status"], "failed", "{doc}");
}

#[test]
fn a6_19_an_archive_into_itself_is_an_item_error() {
    let s = Sandbox::new();
    s.dir("l");
    s.dir("r");
    zip_at(&s, "l/a.zip", &["m.txt"]);
    let before = std::fs::read(s.join("l/a.zip")).unwrap();
    let target = format!("{}/", s.join("l/a.zip").display());
    let out = op(
        &s,
        &[
            "copyto", "--side", "left", "--path", "a.zip", "--target", &target, "--yes",
        ],
    );
    assert_eq!(code(&out), 100, "{}", stdout(&out));
    assert_eq!(std::fs::read(s.join("l/a.zip")).unwrap(), before);
}

/// `--select all` over a folder that failed to list says so: never 0 (p17.3).
#[cfg(unix)]
#[test]
fn a6_25_select_all_over_a_failed_folder_is_not_a_success() {
    let s = Sandbox::new();
    s.file("l/A/a", "a");
    s.file("r/A/a", "a");
    s.file("l/B/x", "x");
    s.dir("r/B");
    common::set_mode(&s.join("l/A"), 0o000);
    // Only a process the mode stops can see this.
    if std::fs::read_dir(s.join("l/A")).is_err() {
        let out = op(&s, &["copy", "--to", "right", "--select", "all", "--dry-run"]);
        assert_eq!(code(&out), 100, "{}", stdout(&out));
        assert!(stdout(&out).contains("compare_error"), "{}", stdout(&out));
    }
    common::set_mode(&s.join("l/A"), 0o755);
}

/// `x.zip/` is a folder, whatever its name looks like; `x.zip` is an archive (D-150).
#[test]
fn a6_19_a_trailing_separator_makes_a_folder() {
    let s = sandbox();
    let target = format!("{}/", s.join("out.zip").display());
    let flags = ["--side", "left", "--path", "only.txt", "--yes"];
    let mut args = vec!["copyto"];
    args.extend(flags);
    args.extend(["--target", &target]);
    let out = op(&s, &args);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(s.join("out.zip").is_dir());
    assert_eq!(
        std::fs::read(s.join("out.zip/only.txt")).unwrap(),
        b"only\n"
    );
    let s = sandbox();
    let target = s.join("out.zip");
    let mut args = vec!["copyto"];
    args.extend(flags);
    args.extend(["--target", target.to_str().unwrap()]);
    let out = op(&s, &args);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(target.is_file());
    assert!(zip_members(&s, "out.zip").contains("only.txt"));
}

/// Open With: no name reaches a program as an option (p14), a row missing on a side starts nothing, and it takes a target.
#[test]
fn a6_23_open_with_refusals() {
    let s = sandbox();
    s.file("l/-rf", "x");
    std::fs::create_dir_all(s.config()).unwrap();
    std::fs::write(
        s.config().join("openwith.toml"),
        "schema_version = 1\n[[app]]\ndescription = \"view\"\ncommand = \"viewer %n %F %P\"\n",
    )
    .unwrap();
    let flags = ["--path=-rf", "--app", "view", "--dry-run", "--json"];
    let mut args = vec!["open-with"];
    args.extend(flags);
    let out = op(&s, &args);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    for word in doc["result"]["commands"][0]["args"].as_array().unwrap() {
        assert_eq!(word, "./-rf");
    }
    let mut args = vec!["open-with"];
    args.extend(flags);
    args.extend(["--side", "right"]);
    let out = op(&s, &args);
    assert_eq!(code(&out), 100);
    assert_eq!(json(&out)["error"]["kind"], "not_found");
    // Neither `--app` nor `--associated`: a usage error, before anything is compared.
    let out = op(&s, &["open-with", "--path", "only.txt", "--json"]);
    assert_eq!(code(&out), 100);
    assert!(stdout(&out).contains("hikasa.usage_error.v1"), "{}", stdout(&out));
}

/// `mkdir --name 'x\\'` is no `x`: off Windows the name is refused, and nothing is made.
#[cfg(not(windows))]
#[test]
fn a6_19_a_trailing_backslash_is_refused() {
    let s = sandbox();
    let before = state(&s);
    let out = op(&s, &["mkdir", "--side", "left", "--name", "x\\", "--yes", "--json"]);
    assert_eq!(code(&out), 100, "{}", stdout(&out));
    assert_eq!(json(&out)["error"]["kind"], "invalid_input");
    assert_eq!(state(&s), before);
}

/// D-184: Disable editing follows where a `--target` leads, for `copyto` and `moveto`, on the left and on the right.
#[test]
fn a6_21_a_target_on_a_read_only_side_is_refused() {
    let s = sandbox();
    s.dir("r/sub");
    let before = state(&s);
    let spellings = [
        s.join("r"),
        s.join("r/sub"),
        s.join("r/new/below"),
        s.join("r/../r/sub"),
        s.join("l/../r/sub"),
    ];
    for verb in ["copyto", "moveto"] {
        for target in &spellings {
            let target = target.to_str().unwrap();
            let out = op(
                &s,
                &[
                    verb,
                    "--side",
                    "left",
                    "--select",
                    "all",
                    "--target",
                    target,
                    "--read-only",
                    "right",
                    "--overwrite",
                    "always",
                    "--yes",
                    "--json",
                ],
            );
            assert_eq!(code(&out), 100, "{verb} {target}");
            assert_eq!(json(&out)["error"]["kind"], "permission_denied");
        }
    }
    assert_eq!(state(&s), before);
    // The other side stays open: nothing in it is read-only.
    let elsewhere = s.join("elsewhere");
    let out = op(
        &s,
        &[
            "copyto",
            "--side",
            "left",
            "--select",
            "all",
            "--target",
            elsewhere.to_str().unwrap(),
            "--read-only",
            "right",
            "--yes",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
}

/// A link at the base and a link on the way say where a target leads, and a session says as `--read-only` does.
#[cfg(unix)]
#[test]
fn a6_21_a_read_only_target_seen_through_links_and_a_session() {
    use std::os::unix::fs::symlink;
    let s = sandbox();
    s.dir("r/sub");
    symlink(s.join("r"), s.join("base")).unwrap();
    symlink(s.join("r/sub"), s.join("way")).unwrap();
    let before = state(&s);
    let verb = |left: &Path, right: &Path, target: &Path| {
        let mut command = s.cmd();
        command.args(["folder", "op", "moveto"]).args([left, right]);
        command.args(["--side", "left", "--select", "all", "--read-only", "right"]);
        command.arg("--target").arg(target).args(["--yes", "--json"]);
        command.output().unwrap()
    };
    let (l, r) = (s.join("l"), s.join("r"));
    for (right, target) in [
        (&r, s.join("way")),
        (&r, s.join("way/new")),
        (&r, s.join("base/sub")),
        (&s.join("base"), s.join("r/sub")),
    ] {
        let out = verb(&l, right, &target);
        assert_eq!(code(&out), 100, "{}", target.display());
        assert_eq!(json(&out)["error"]["kind"], "permission_denied");
    }
    assert_eq!(state(&s), before);
    // A session with the right side read-only.
    let mut new = s.cmd();
    new.args(["session", "new", "s", "--type", "folder-compare", "--left"]);
    new.arg(&l).arg("--right").arg(&r);
    assert_eq!(code(&new.output().unwrap()), 0);
    let mut edit = s.cmd();
    edit.args(["session", "edit", "s", "--set", "specs.read_only_right=true"]);
    assert_eq!(code(&edit.output().unwrap()), 0);
    for target in [s.join("r/sub"), s.join("way")] {
        let mut command = s.cmd();
        command.args(["folder", "op", "copyto", "--session", "s", "--side", "left"]);
        command.args(["--select", "all", "--yes", "--json", "--target"]);
        command.arg(&target);
        let out = command.output().unwrap();
        assert_eq!(code(&out), 100, "{}", target.display());
        assert_eq!(json(&out)["error"]["kind"], "permission_denied");
    }
    assert_eq!(state(&s), before);
}

/// An archive that is the base, and one below it: a write to a member rewrites what is read-only.
#[test]
fn a6_21_a_member_of_a_read_only_side_is_refused() {
    let s = sandbox();
    let empty = s.join("empty.zip");
    let made = s
        .cmd()
        .args(["vfs", "mkdir"])
        .arg(&empty)
        .output()
        .unwrap();
    assert_eq!(code(&made), 0, "{}", stderr(&made));
    let moved = s.join("r/a.zip");
    std::fs::rename(&empty, &moved).unwrap();
    let target = format!("{}/", moved.display());
    let out = op(
        &s,
        &[
            "copyto",
            "--side",
            "left",
            "--path",
            "only.txt",
            "--target",
            &target,
            "--read-only",
            "right",
            "--yes",
            "--json",
        ],
    );
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert_eq!(json(&out)["error"]["kind"], "permission_denied");
}

/// D-185: `--path` names one row. No spelling of an empty shell variable reaches the base, on any entry that takes it.
#[test]
fn a6_25_no_spelling_of_path_reaches_the_base() {
    let s = sandbox();
    let before = state(&s);
    for word in [
        "", ".", "/", "//", "./", "..", "../", "left:", "right:", "left:.", "right:/", "left:./",
    ] {
        for verb in [
            &["delete", "--side", "both", "--yes"][..],
            &["copy", "--to", "right", "--yes"][..],
            &["touch", "--side", "left"][..],
            &["crc", "--side", "left"][..],
            &["exclude", "--dry-run"][..],
            &["ignored", "--dry-run"][..],
            &["open-with", "--associated", "--dry-run"][..],
        ] {
            let mut args = verb.to_vec();
            args.extend(["--path", word, "--json"]);
            let out = op(&s, &args);
            assert_eq!(code(&out), 100, "{verb:?} --path {word:?}");
            assert_eq!(json(&out)["error"]["kind"], "invalid_input", "{word:?}");
        }
    }
    assert_eq!(state(&s), before);
    // A name of dots is a name, and a trailing separator still names its row.
    s.file_at("l/.../x", "x", 1_000_000);
    let out = op(&s, &["copy", "--to", "right", "--path", ".../", "--yes"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(s.join("r/.../x").exists());
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
