//! `hikasa folder sync` end to end (M2 W6c, contract §1.6.17): A6.31 and the CLI's half of A6.30. Each preset flag, `--custom`,
//! `--orphans`, `--create-empty`, `--recycle`, `--within-archives`, `--override`, `--just-selection`, `--visible`,
//! `--all-rows`, goldens, exit codes, dry runs equal to runs, `--gui` up to the IPC answer, and folder-sync sessions with
//! `session run`.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::{CloseMode, FakeGui, Sandbox, assert_golden, code, json, serve, stderr, stdout};
use hikasa_core::ExitCode;

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

/// `l/` and `r/`: the left newer, the right newer, an orphan on each side, a pair that differs at one time, one the same,
/// an empty folder on the left and a folder with a file on the right only.
fn sandbox() -> Sandbox {
    let s = Sandbox::new();
    s.file_at("l/newer-left.txt", "left wins\n", 2_000_000);
    s.file_at("r/newer-left.txt", "old\n", 1_000_000);
    s.file_at("l/newer-right.txt", "old\n", 1_000_000);
    s.file_at("r/newer-right.txt", "right wins\n", 2_000_000);
    s.file_at("l/left-only.txt", "left only\n", 1_000_000);
    s.file_at("r/right-only.txt", "right only\n", 1_000_000);
    s.file_at("l/different.txt", "left side\n", 1_000_000);
    s.file_at("r/different.txt", "the right side\n", 1_000_000);
    s.file_at("l/same.txt", "same\n", 1_000_000);
    s.file_at("r/same.txt", "same\n", 1_000_000);
    s.dir("l/empty");
    s.file_at("r/gone/inner.txt", "inner\n", 1_000_000);
    s.settle("l");
    s.settle("r");
    s
}

fn sync(s: &Sandbox, args: &[&str]) -> std::process::Output {
    let mut command = s.cmd();
    command.args(["folder", "sync"]);
    command.arg(s.join("l"));
    command.arg(s.join("r"));
    command.args(args);
    command.output().unwrap()
}

/// A tree: each path with its bytes (`None` for a folder).
type Tree = Vec<(String, Option<Vec<u8>>)>;

fn state(s: &Sandbox) -> (Tree, Tree) {
    (tree(&s.join("l")), tree(&s.join("r")))
}

fn read(s: &Sandbox, rel: &str) -> String {
    std::fs::read_to_string(s.join(rel)).unwrap()
}

/// A text golden and a JSON golden of a sync on fresh sandboxes.
fn golden(name: &str, args: &[&str]) {
    let s = sandbox();
    let out = sync(&s, args);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_golden(
        &format!("folder_sync/{name}.txt"),
        &s.normalise(&stdout(&out)),
    );
    let s = sandbox();
    let mut with_json = args.to_vec();
    with_json.push("--json");
    let out = sync(&s, &with_json);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.folder_sync.v1");
    assert_eq!(doc["ok"], true);
    assert_golden(
        &format!("folder_sync/{name}.json"),
        &s.normalise(&stdout(&out)),
    );
}

#[test]
fn a6_31_goldens_of_the_presets() {
    golden("mirror_right", &["--mirror-right", "--yes"]);
    golden("update_both", &["--update-both", "--yes"]);
    golden("mirror_left_dry", &["--mirror-left", "--dry-run"]);
}

#[test]
fn a6_31_each_preset_flag_does_its_table() {
    // Update Right: newer and new on the left go right; nothing is deleted; the right's newer file stays.
    let s = sandbox();
    let out = sync(&s, &["--update-right", "--yes"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&s, "r/newer-left.txt"), "left wins\n");
    assert_eq!(read(&s, "r/left-only.txt"), "left only\n");
    assert_eq!(read(&s, "r/newer-right.txt"), "right wins\n");
    assert_eq!(read(&s, "r/different.txt"), "the right side\n");
    assert!(s.join("r/right-only.txt").exists() && s.join("r/gone/inner.txt").exists());
    assert!(
        stdout(&out).contains("<!> leave different.txt (matrix)"),
        "{}",
        stdout(&out)
    );
    // Update Left: the other way.
    let s = sandbox();
    assert_eq!(code(&sync(&s, &["--update-left", "--yes"])), 0);
    assert_eq!(read(&s, "l/newer-right.txt"), "right wins\n");
    assert_eq!(read(&s, "l/right-only.txt"), "right only\n");
    assert_eq!(read(&s, "l/newer-left.txt"), "left wins\n");
    assert!(s.join("l/left-only.txt").exists());
    // Update Both.
    let s = sandbox();
    assert_eq!(code(&sync(&s, &["--update-both", "--yes"])), 0);
    assert_eq!(read(&s, "r/newer-left.txt"), "left wins\n");
    assert_eq!(read(&s, "l/newer-right.txt"), "right wins\n");
    assert!(s.join("l/right-only.txt").exists() && s.join("r/left-only.txt").exists());
    // Mirror to Right: the right is the left, empty folders aside.
    let s = sandbox();
    assert_eq!(code(&sync(&s, &["--mirror-right", "--yes"])), 0);
    let (l, r) = state(&s);
    let files = |list: &[(String, Option<Vec<u8>>)]| -> Vec<(String, Option<Vec<u8>>)> {
        list.iter()
            .filter(|(_, it)| it.is_some())
            .cloned()
            .collect()
    };
    assert_eq!(files(&l), files(&r));
    assert!(
        !s.join("r/gone").exists(),
        "a folder all of whose rows went goes too"
    );
    // Mirror to Left.
    let s = sandbox();
    assert_eq!(code(&sync(&s, &["--mirror-left", "--yes"])), 0);
    let (l, r) = state(&s);
    assert_eq!(files(&l), files(&r));
    assert!(!s.join("l/left-only.txt").exists());
}

#[test]
fn a6_31_custom_orphans_create_empty_and_override() {
    // `--custom`: a preset as the base, one cell changed.
    let s = sandbox();
    let out = sync(
        &s,
        &[
            "--custom",
            "mirror-right,different=leave",
            "--yes",
            "--json",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["result"]["preset"], "custom");
    assert_eq!(doc["result"]["matrix"]["different"], "leave");
    assert_eq!(doc["result"]["matrix"]["added_right"], "delete_right");
    assert_eq!(read(&s, "r/different.txt"), "the right side\n");
    assert!(!s.join("r/right-only.txt").exists());
    // `--orphans deleted`: a right-only file is gone from the left, so Mirror to Left... and Update Left leaves it.
    let s = sandbox();
    let out = sync(
        &s,
        &["--update-left", "--orphans", "deleted", "--yes", "--json"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["result"]["orphans"], "deleted");
    assert!(!s.join("l/right-only.txt").exists());
    // `--create-empty`: the empty folder is made.
    let s = sandbox();
    assert_eq!(code(&sync(&s, &["--mirror-right", "--yes"])), 0);
    assert!(!s.join("r/empty").exists());
    let s = sandbox();
    assert_eq!(
        code(&sync(&s, &["--mirror-right", "--create-empty", "--yes"])),
        0
    );
    assert!(s.join("r/empty").is_dir());
    // `--override`: a path, a folder over its subtree, the longest path wins.
    let s = sandbox();
    let out = sync(
        &s,
        &[
            "--mirror-right",
            "--override",
            "gone=leave",
            "--override",
            "newer-right.txt=leave",
            "--yes",
            "--json",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&s, "r/gone/inner.txt"), "inner\n");
    assert_eq!(read(&s, "r/newer-right.txt"), "right wins\n");
    let rows = json(&out)["result"]["rows"].as_array().unwrap().clone();
    let inner = rows.iter().find(|r| r["path"] == "gone/inner.txt").unwrap();
    assert_eq!(inner["overridden"], true);
    assert_eq!(inner["action"], "leave");
}

#[test]
fn a6_31_bad_words_are_100_and_write_nothing() {
    for args in [
        vec!["--custom", "added-left=copy-left", "--yes"],
        vec![
            "--custom",
            "newer-left=copy-right,newer-left=leave",
            "--yes",
        ],
        vec!["--custom", "sideways", "--yes"],
        vec!["--mirror-right", "--orphans", "gone", "--yes"],
        vec!["--mirror-right", "--override", "=leave", "--yes"],
        vec!["--mirror-right", "--override", "../x=leave", "--yes"],
        vec!["--mirror-right", "--override", "a.txt=move", "--yes"],
        // No preset at all.
        vec!["--yes"],
        // A sync ignores the selection: without `--just-selection` it is refused.
        vec!["--mirror-right", "--select", "all", "--yes"],
    ] {
        let s = sandbox();
        let before = state(&s);
        let mut with_json = args.clone();
        with_json.push("--json");
        let out = sync(&s, &with_json);
        assert_eq!(code(&out), 100, "{args:?}: {}", stderr(&out));
        let doc = json(&out);
        assert_eq!(doc["schema"], "hikasa.folder_sync.v1", "{args:?}");
        assert_eq!(doc["ok"], false, "{args:?}");
        assert_eq!(state(&s), before, "{args:?}");
    }
    // Two presets: a usage error of the parser.
    let s = sandbox();
    let out = sync(&s, &["--mirror-right", "--update-left", "--yes"]);
    assert_ne!(code(&out), 0);
}

#[test]
fn a6_31_just_selection_visible_and_all_rows() {
    let s = sandbox();
    let out = sync(
        &s,
        &[
            "--mirror-right",
            "--just-selection",
            "--path",
            "left-only.txt",
            "--yes",
            "--json",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(s.join("r/left-only.txt").exists());
    assert_eq!(read(&s, "r/newer-left.txt"), "old\n");
    assert!(s.join("r/right-only.txt").exists());
    let rows = json(&out)["result"]["rows"].as_array().unwrap().clone();
    assert!(rows.iter().any(|r| r["reason"] == "out_of_scope"));
    // `--just-selection` with nothing selected acts on nothing: 100.
    let s = sandbox();
    let before = state(&s);
    let out = sync(
        &s,
        &[
            "--mirror-right",
            "--just-selection",
            "--select",
            "left.newer.files",
            "--display",
            "same",
            "--yes",
        ],
    );
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert_eq!(state(&s), before);
    // `--visible`: what `--display` shows.
    let s = sandbox();
    let out = sync(
        &s,
        &[
            "--mirror-right",
            "--visible",
            "--display",
            "orphans",
            "--yes",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(s.join("r/left-only.txt").exists());
    assert!(!s.join("r/right-only.txt").exists());
    assert_eq!(read(&s, "r/newer-left.txt"), "old\n");
    // `--all-rows`: `same` rows and folders too.
    let s = sandbox();
    let some = json(&sync(&s, &["--update-right", "--dry-run", "--json"]));
    let all = json(&sync(
        &s,
        &["--update-right", "--dry-run", "--all-rows", "--json"],
    ));
    let paths = |doc: &serde_json::Value| -> Vec<String> {
        doc["result"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["path"].as_str().unwrap().to_owned())
            .collect()
    };
    assert!(!paths(&some).contains(&"same.txt".to_owned()));
    assert!(paths(&all).contains(&"same.txt".to_owned()));
    assert!(paths(&all).contains(&"gone".to_owned()));
}

#[test]
fn a6_31_recycle_plans_trash_and_a_dry_run_equals_the_run() {
    // The real recycle bin is never used by a test: `--recycle` is checked in a dry run.
    let s = sandbox();
    let out = sync(&s, &["--mirror-right", "--recycle", "--dry-run", "--json"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let items = json(&out)["result"]["items"].as_array().unwrap().clone();
    let trashed: Vec<&str> = items
        .iter()
        .filter(|i| i["action"] == "trash")
        .map(|i| i["path"].as_str().unwrap())
        .collect();
    assert_eq!(trashed, ["gone", "right-only.txt"], "{items:?}");
    // A dry run shows the items a run does, in order, and the run does each one.
    let s = sandbox();
    let dry = json(&sync(
        &s,
        &["--mirror-right", "--create-empty", "--dry-run", "--json"],
    ));
    let s = sandbox();
    let run = json(&sync(
        &s,
        &["--mirror-right", "--create-empty", "--yes", "--json"],
    ));
    let key = |doc: &serde_json::Value| -> Vec<(String, String)> {
        doc["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                (
                    i["action"].as_str().unwrap().to_owned(),
                    i["path"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    assert_eq!(key(&dry), key(&run));
    assert!(
        dry["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["status"] == "planned")
    );
    assert!(
        run["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["status"] == "done")
    );
    assert_eq!(dry["result"]["counts"], run["result"]["counts"]);
    assert_eq!(dry["result"]["dry_run"], true);
}

#[test]
fn a6_31_within_archives() {
    fn tar_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, data) in entries {
            let mut header = [0u8; 512];
            header[..name.len()].copy_from_slice(name.as_bytes());
            header[100..107].copy_from_slice(b"0000644");
            header[108..115].copy_from_slice(b"0000000");
            header[116..123].copy_from_slice(b"0000000");
            header[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
            header[136..147].copy_from_slice(b"15000000000");
            header[156] = b'0';
            header[257..263].copy_from_slice(b"ustar\0");
            header[263..265].copy_from_slice(b"00");
            header[148..156].copy_from_slice(b"        ");
            let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
            header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
            out.extend_from_slice(&header);
            out.extend_from_slice(data);
            out.resize(out.len().div_ceil(512) * 512, 0);
        }
        out.resize(out.len() + 1024, 0);
        out
    }
    let setup = || {
        let s = Sandbox::new();
        s.file_at(
            "l/a.tar",
            tar_of(&[("x", b"new x"), ("y", b"same")]),
            2_000_000,
        );
        s.file_at(
            "r/a.tar",
            tar_of(&[("x", b"old"), ("y", b"same"), ("z", b"stale")]),
            1_000_000,
        );
        s.settle("l");
        s.settle("r");
        s
    };
    // Without: the archive is a file, copied whole.
    let s = setup();
    let out = sync(&s, &["--mirror-right", "--yes", "--json"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        std::fs::read(s.join("r/a.tar")).unwrap(),
        std::fs::read(s.join("l/a.tar")).unwrap()
    );
    // Within: member by member.
    let s = setup();
    let out = sync(
        &s,
        &["--mirror-right", "--within-archives", "--yes", "--json"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    let rows = doc["result"]["rows"].as_array().unwrap();
    let paths: Vec<&str> = rows.iter().map(|r| r["path"].as_str().unwrap()).collect();
    assert!(
        paths.contains(&"a.tar/x") && paths.contains(&"a.tar/z"),
        "{paths:?}"
    );
    let mut list = s.cmd();
    list.args(["vfs", "ls", "--json"]).arg(s.join("r/a.tar"));
    let listed = json(&list.output().unwrap());
    let members: Vec<String> = listed["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(members, ["x", "y"]);
}

#[test]
fn a6_31_exit_codes_and_safety() {
    // A base that is not there: 107.
    let s = sandbox();
    let mut command = s.cmd();
    command.args(["folder", "sync", "--mirror-right", "--yes", "--json"]);
    command.arg(s.join("l")).arg(s.join("missing"));
    let out = command.output().unwrap();
    assert_eq!(code(&out), 107, "{}", stderr(&out));
    // An empty source: refused, nothing written; `--yes` does not answer it, `--allow-empty-source` does.
    let s = Sandbox::new();
    s.dir("l");
    s.file("r/a.txt", "a\n");
    s.file("r/d/b.txt", "b\n");
    let out = sync(&s, &["--mirror-right", "--yes", "--json"]);
    assert_eq!(code(&out), 100);
    assert_eq!(json(&out)["error"]["kind"], "invalid_input");
    assert!(
        json(&out)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("empty")
    );
    assert!(s.join("r/a.txt").exists() && s.join("r/d/b.txt").exists());
    let out = sync(&s, &["--mirror-right", "--yes", "--allow-empty-source"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!s.join("r/a.txt").exists() && !s.join("r/d").exists());
    // Update from an empty base is fine: it deletes nothing.
    let s = Sandbox::new();
    s.dir("l");
    s.file("r/a.txt", "a\n");
    assert_eq!(code(&sync(&s, &["--update-right", "--yes"])), 0);
    assert!(s.join("r/a.txt").exists());
    // Without a terminal and without `--yes`: 100, nothing touched.
    let s = sandbox();
    let before = state(&s);
    let out = sync(&s, &["--mirror-right"]);
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert!(stderr(&out).contains("need an answer"), "{}", stderr(&out));
    assert_eq!(state(&s), before);
    // A read-only side: its rows are left, the sync goes on and says so.
    let s = sandbox();
    let out = sync(
        &s,
        &["--update-both", "--read-only", "right", "--yes", "--json"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!s.join("r/left-only.txt").exists());
    assert!(s.join("l/right-only.txt").exists());
    let doc = json(&out);
    assert!(
        doc["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "read_only_side"),
        "{doc}"
    );
    // A row the comparison could not tell: 100 also in a dry run, and nothing below it is touched.
    #[cfg(unix)]
    {
        let s = sandbox();
        s.file_at("r/locked/keep.txt", "keep\n", 1_000_000);
        s.dir("l/locked");
        common::set_mode(&s.join("l/locked"), 0o000);
        let readable = std::fs::read_dir(s.join("l/locked")).is_ok();
        if !readable {
            let out = sync(&s, &["--mirror-right", "--dry-run", "--json"]);
            assert_eq!(code(&out), 100, "{}", stderr(&out));
            let doc = json(&out);
            assert_eq!(doc["ok"], true);
            assert!(doc["result"]["counts"]["unresolved"].as_u64().unwrap() >= 1);
            assert!(
                doc["warnings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|w| w["code"] == "sync_incomplete")
            );
            let out = sync(&s, &["--mirror-right", "--yes"]);
            assert_eq!(code(&out), 100);
            assert_eq!(read(&s, "r/locked/keep.txt"), "keep\n");
        }
        common::set_mode(&s.join("l/locked"), 0o755);
    }
}

#[test]
fn a6_31_gui_sends_the_sync_and_the_answer_is_unsupported() {
    let s = sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::Success);
    let server = serve(&s, Arc::clone(&gui));
    let out = sync(
        &s,
        &["--mirror-right", "--orphans", "deleted", "--gui", "--json"],
    );
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.gui_open.v1");
    assert_eq!(doc["error"]["kind"], "unsupported");
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot show a Folder Sync view yet"),
        "{doc}"
    );
    // What the GUI received on the wire: one `OpenView { kind: folder_sync }` with the two sides and the sync in words.
    let opened: Vec<hikasa_session::ipc::Request> = gui
        .requests()
        .into_iter()
        .filter(|r| matches!(r, hikasa_session::ipc::Request::OpenView { .. }))
        .collect();
    assert_eq!(opened.len(), 1, "{opened:?}");
    let hikasa_session::ipc::Request::OpenView {
        kind,
        specs,
        options,
        wait,
    } = &opened[0]
    else {
        unreachable!()
    };
    assert_eq!(*kind, hikasa_session::ipc::ViewKind::FolderSync);
    assert!(!wait);
    assert_eq!(specs.len(), 2, "{specs:?}");
    assert!(
        specs[0].ends_with("l") && specs[1].ends_with("r"),
        "{specs:?}"
    );
    let words = options.sync.as_ref().expect("the sync in words");
    assert_eq!(words.preset.as_deref(), Some("mirror_right"), "{words:?}");
    assert_eq!(words.orphans.as_deref(), Some("deleted"), "{words:?}");
    assert!(
        words.custom.is_none() && !words.recycle && words.overrides.is_empty(),
        "{words:?}"
    );
    // The words are checked before any contact: a bad word is 100 with its own message, and the GUI got nothing more.
    let before = gui.requests().len();
    let out = sync(&s, &["--custom", "same=sideways", "--gui", "--json"]);
    assert_eq!(code(&out), 100);
    let message = json(&out)["error"]["message"].as_str().unwrap().to_owned();
    assert!(message.contains("sideways"), "{message}");
    assert_eq!(gui.requests().len(), before, "{:?}", gui.requests());
    server.shutdown().unwrap();
    assert!(state(&s).1.iter().any(|(n, _)| n == "right-only.txt"));
}

#[test]
fn a6_31_help_topic() {
    let s = Sandbox::new();
    let mut command = s.cmd();
    command.args(["help", "folder", "sync"]);
    let out = command.output().unwrap();
    assert_eq!(code(&out), 0);
    assert!(
        stdout(&out).contains("hikasa folder sync"),
        "{}",
        stdout(&out)
    );
    assert!(
        stdout(&out).contains("--allow-empty-source"),
        "{}",
        stdout(&out)
    );
}

// ---- sessions (A6.30) ------------------------------------------------------------------------------------------

fn session(s: &Sandbox, args: &[&str]) -> std::process::Output {
    let mut command = s.cmd();
    command.arg("session");
    command.args(args);
    command.output().unwrap()
}

#[test]
fn a6_30_a_folder_sync_session_end_to_end() {
    let s = sandbox();
    let mut new = s.cmd();
    new.args([
        "session",
        "new",
        "nightly",
        "--type",
        "folder-sync",
        "--left",
    ]);
    new.arg(s.join("l")).arg("--right").arg(s.join("r"));
    assert_eq!(code(&new.output().unwrap()), 0);
    // The defaults: Update Right, and a sparse file.
    let show = session(&s, &["show", "nightly", "--json"]);
    assert_eq!(code(&show), 0, "{}", stderr(&show));
    let doc = json(&show);
    assert_eq!(doc["result"]["session"]["type"], "folder-sync", "{doc}");
    let file = std::fs::read_to_string(s.config().join("sessions/nightly.toml")).unwrap();
    assert!(!file.contains("[rules.sync]"), "sparse: {file}");
    // `--set` a preset and a cell; a bad cell is refused.
    let out = session(
        &s,
        &["edit", "nightly", "--set", "rules.sync.preset=mirror_right"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let bad = session(
        &s,
        &[
            "edit",
            "nightly",
            "--set",
            "rules.sync.matrix.added_left=copy_left",
        ],
    );
    assert_eq!(code(&bad), 100, "{}", stderr(&bad));
    let file = std::fs::read_to_string(s.config().join("sessions/nightly.toml")).unwrap();
    assert!(file.contains("preset = \"mirror_right\""), "{file}");
    assert!(!file.contains("added_left"), "{file}");
    // `session run`: a dry run writes nothing; without `--yes` and a terminal it is 100; with `--yes` it runs.
    let before = state(&s);
    let dry = session(&s, &["run", "nightly", "--dry-run", "--json"]);
    assert_eq!(code(&dry), 0, "{}", stderr(&dry));
    assert_eq!(json(&dry)["schema"], "hikasa.folder_sync.v1");
    assert_eq!(json(&dry)["result"]["preset"], "mirror_right");
    assert_eq!(state(&s), before);
    let ask = session(&s, &["run", "nightly"]);
    assert_eq!(code(&ask), 100, "{}", stderr(&ask));
    assert_eq!(state(&s), before);
    let run = session(&s, &["run", "nightly", "--yes"]);
    assert_eq!(code(&run), 0, "{}", stderr(&run));
    assert!(!s.join("r/right-only.txt").exists());
    assert_eq!(read(&s, "r/newer-left.txt"), "left wins\n");
    // `read_only_right` from the session: a mirror onto it writes nothing.
    let s = sandbox();
    let mut new = s.cmd();
    new.args(["session", "new", "ro", "--type", "folder-sync", "--left"]);
    new.arg(s.join("l")).arg("--right").arg(s.join("r"));
    assert_eq!(code(&new.output().unwrap()), 0);
    let out = session(
        &s,
        &[
            "edit",
            "ro",
            "--set",
            "rules.sync.preset=mirror_right",
            "--set",
            "specs.read_only_right=true",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let before = state(&s);
    let run = session(&s, &["run", "ro", "--yes", "--json"]);
    assert_eq!(code(&run), 0, "{}", stderr(&run));
    assert_eq!(state(&s).1, before.1);
    // `folder sync --session` takes the session's preset, flags on top.
    let out = {
        let mut command = s.cmd();
        command.args(["folder", "sync", "--session", "ro", "--dry-run", "--json"]);
        command.output().unwrap()
    };
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(json(&out)["result"]["preset"], "mirror_right");
    // The sync flags of `session run` mean nothing to a comparison: refused.
    let mut compare = s.cmd();
    compare.args([
        "session",
        "new",
        "cmp",
        "--type",
        "folder-compare",
        "--left",
    ]);
    compare.arg(s.join("l")).arg("--right").arg(s.join("r"));
    assert_eq!(code(&compare.output().unwrap()), 0);
    assert_eq!(code(&session(&s, &["run", "cmp", "--yes"])), 100);
}

#[test]
fn a6_30_an_old_folder_sync_file_reads_as_it_is() {
    let s = sandbox();
    let sessions = s.config().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    // A file of M1/W2: `type = "folder-sync"` with an empty `[rules]`.
    let l = s.join("l").display().to_string().replace('\\', "\\\\");
    let r = s.join("r").display().to_string().replace('\\', "\\\\");
    std::fs::write(
        sessions.join("old.toml"),
        format!(
            "schema_version = 1\ntype = \"folder-sync\"\ndescription = \"before W6\"\n\n[specs]\nleft = \"{l}\"\nright = \
             \"{r}\"\n\n[rules]\n"
        ),
    )
    .unwrap();
    let show = session(&s, &["show", "old", "--json"]);
    assert_eq!(code(&show), 0, "{}", stderr(&show));
    let doc = json(&show);
    let read = &doc["result"]["session"];
    assert_eq!(read["description"], "before W6");
    assert_eq!(read["type"], "folder-sync");
    let canonical = |rel: &str| {
        std::fs::canonicalize(s.join(rel))
            .unwrap()
            .display()
            .to_string()
    };
    for (side, rel) in [("left", "l"), ("right", "r")] {
        let got = read["specs"][side].as_str().unwrap().to_owned();
        assert!(
            got == s.join(rel).display().to_string() || got == canonical(rel),
            "{side}: {got}"
        );
    }
    // An empty `[rules]` is the defaults: Update Right, orphans added, nothing else.
    let dry = session(&s, &["run", "old", "--dry-run", "--json"]);
    assert_eq!(code(&dry), 0, "{}", stderr(&dry));
    let result = &json(&dry)["result"];
    assert_eq!(result["preset"], "update_right");
    assert_eq!(result["orphans"], "added");
    assert_eq!(result["matrix"]["newer_left"], "copy_right");
    assert_eq!(result["matrix"]["added_right"], "leave");
    let items: Vec<(String, String)> = result["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["action"].as_str().unwrap().to_owned(),
                i["path"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let paths: Vec<&str> = items.iter().map(|(_, p)| p.as_str()).collect();
    assert!(
        paths.contains(&"left-only.txt") && paths.contains(&"newer-left.txt"),
        "{items:?}"
    );
    assert!(
        items
            .iter()
            .all(|(action, _)| action == "copy" || action == "replace"),
        "Update Right only copies: {items:?}"
    );
    assert_eq!(result["counts"]["delete_right"], 0);
    // The file is not rewritten by reading it.
    let after = std::fs::read_to_string(sessions.join("old.toml")).unwrap();
    assert!(after.contains("description = \"before W6\""), "{after}");
    assert!(!after.contains("[rules.sync]"), "{after}");
}

// ---- after the review of W6c -------------------------------------------------------------------------------------

/// `[file_operations] include_hidden_by_default` is a folder operation's preference: it never reaches a sync, whose rule 3
/// leaves a side a filter hides unless the sync's own `--include-hidden` says otherwise.
#[test]
fn a6_31_the_include_hidden_preference_does_not_reach_a_sync() {
    let fresh = || {
        let s = Sandbox::new();
        s.file_at("l/f", "tiny\n", 2_000_000);
        s.file_at(
            "r/f",
            "the right side is large enough to pass the filter\n",
            1_000_000,
        );
        s.settle("l");
        s.settle("r");
        s.options("[file_operations]\ninclude_hidden_by_default = true\n");
        s
    };
    let s = fresh();
    let out = sync(
        &s,
        &["--mirror-right", "--size-filter=>10", "--yes", "--json"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        read(&s, "r/f"),
        "the right side is large enough to pass the filter\n"
    );
    let rows = json(&out)["result"]["rows"].as_array().unwrap().clone();
    assert!(
        rows.iter()
            .any(|r| r["path"] == "f" && r["reason"] == "filtered"),
        "{rows:?}"
    );
    // `--just-selection --select all` does not bring it either.
    let s = fresh();
    let out = sync(
        &s,
        &[
            "--mirror-right",
            "--size-filter=>10",
            "--just-selection",
            "--select",
            "all",
            "--yes",
        ],
    );
    assert_eq!(
        read(&s, "r/f"),
        "the right side is large enough to pass the filter\n",
        "{}",
        stdout(&out)
    );
    // The sync's own flag does.
    let s = fresh();
    let out = sync(
        &s,
        &[
            "--mirror-right",
            "--size-filter=>10",
            "--include-hidden",
            "--yes",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&s, "r/f"), "tiny\n");
}

/// A folder-compare session holds no sync: `folder sync --session` of one needs a preset flag, as the two sides do; it is
/// never Update Right by default. `--gui` refuses it too, before any contact.
#[test]
fn a6_31_a_folder_compare_session_needs_a_preset() {
    let s = sandbox();
    let mut compare = s.cmd();
    compare.args([
        "session",
        "new",
        "cmp",
        "--type",
        "folder-compare",
        "--left",
    ]);
    compare.arg(s.join("l")).arg("--right").arg(s.join("r"));
    assert_eq!(code(&compare.output().unwrap()), 0);
    let before = state(&s);
    for extra in [
        &["--yes", "--json"][..],
        &["--gui", "--no-launch", "--json"],
    ] {
        let mut command = s.cmd();
        command.args(["folder", "sync", "--session", "cmp"]);
        command.args(extra);
        let out = command.output().unwrap();
        assert_eq!(code(&out), 100, "{extra:?} {}", stdout(&out));
        let doc = json(&out);
        assert_eq!(doc["ok"], false);
        assert_eq!(doc["error"]["kind"], "invalid_input", "{doc}");
        assert!(
            doc["error"]["message"]
                .as_str()
                .unwrap()
                .contains("a sync needs its preset"),
            "{doc}"
        );
        assert_eq!(state(&s), before);
    }
    // With a preset flag it runs, from the session's sides.
    let mut command = s.cmd();
    command.args([
        "folder",
        "sync",
        "--session",
        "cmp",
        "--update-right",
        "--yes",
    ]);
    let out = command.output().unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(read(&s, "r/left-only.txt"), "left only\n");
    // A folder-sync session holds one (its default is Update Right).
    let mut new = s.cmd();
    new.args(["session", "new", "fs", "--type", "folder-sync", "--left"]);
    new.arg(s.join("l")).arg("--right").arg(s.join("r"));
    assert_eq!(code(&new.output().unwrap()), 0);
    let mut command = s.cmd();
    command.args(["folder", "sync", "--session", "fs", "--dry-run", "--json"]);
    let out = command.output().unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(json(&out)["result"]["preset"], "update_right");
}

/// `--path` naming a row that is not on its side selects nothing: that is `not_found`, exit 100, and nothing runs; a
/// `--select` that matches nothing is refused as nothing selected. A row on its side runs.
#[test]
fn a6_31_a_path_not_on_its_side_is_not_found() {
    let s = sandbox();
    let before = state(&s);
    for path in ["left:right-only.txt", "right:left-only.txt", "left:gone"] {
        let out = sync(
            &s,
            &[
                "--mirror-right",
                "--just-selection",
                "--path",
                path,
                "--yes",
                "--json",
            ],
        );
        assert_eq!(code(&out), 100, "{path}: {}", stdout(&out));
        let doc = json(&out);
        assert_eq!(doc["error"]["kind"], "not_found", "{path}: {doc}");
        assert_eq!(state(&s), before, "{path}");
    }
    let out = sync(
        &s,
        &[
            "--mirror-right",
            "--just-selection",
            "--select",
            "all.exact.folders",
            "--yes",
            "--json",
        ],
    );
    assert_eq!(code(&out), 100, "{}", stdout(&out));
    assert!(
        json(&out)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nothing selected"),
        "{}",
        stdout(&out)
    );
    assert_eq!(state(&s), before);
    // `folder op` too.
    let mut op = s.cmd();
    op.args(["folder", "op", "delete", "--side", "left", "--path"])
        .arg("right:left-only.txt")
        .arg(s.join("l"))
        .arg(s.join("r"))
        .args(["--yes", "--json"]);
    let out = op.output().unwrap();
    assert_eq!(code(&out), 100, "{}", stdout(&out));
    assert_eq!(json(&out)["error"]["kind"], "not_found");
    assert_eq!(state(&s), before);
    // The row on its side.
    let out = sync(
        &s,
        &[
            "--mirror-right",
            "--just-selection",
            "--path",
            "right:right-only.txt",
            "--yes",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!s.join("r/right-only.txt").exists());
    assert!(s.join("r/gone/inner.txt").exists());
}

/// A link to `target` at `at`; false where this system does not let the test make one (Windows without the privilege).
fn make_link(target: &str, at: &Path) -> bool {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, at).is_ok();
    #[cfg(windows)]
    return std::os::windows::fs::symlink_file(target, at).is_ok();
}

/// A copied link keeps its time, so a sync settles: a second mirror plans nothing, and Update Both does not copy a link
/// back and forth.
#[test]
fn a6_31_a_copied_link_keeps_its_time_and_a_sync_settles() {
    let s = Sandbox::new();
    s.dir("l");
    s.dir("r");
    // A target in the system's own spelling.
    let target = if cfg!(windows) { r"some\where" } else { "some/where" };
    if !make_link(target, &s.join("l/lnk")) {
        eprintln!("skipped: this system does not let the test make a link");
        return;
    }
    let old = filetime::FileTime::from_unix_time(1_000_000, 0);
    filetime::set_symlink_file_times(s.join("l/lnk"), old, old).unwrap();
    let out = sync(&s, &["--mirror-right", "--yes"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let copied = std::fs::symlink_metadata(s.join("r/lnk")).unwrap();
    assert!(copied.file_type().is_symlink());
    assert_eq!(
        filetime::FileTime::from_last_modification_time(&copied),
        old
    );
    let counts = |out: &std::process::Output| json(out)["result"]["counts"].clone();
    let again = sync(&s, &["--mirror-right", "--dry-run", "--json"]);
    assert_eq!(code(&again), 0, "{}", stderr(&again));
    assert_eq!(json(&again)["result"]["items"], serde_json::json!([]));
    for _ in 0..2 {
        let both = sync(&s, &["--update-both", "--yes", "--json"]);
        assert_eq!(code(&both), 0, "{}", stderr(&both));
        assert_eq!(json(&both)["result"]["items"], serde_json::json!([]));
        assert_eq!(counts(&both)["copy_left"], 0);
        assert_eq!(counts(&both)["copy_right"], 0);
    }
}

/// `--case-insensitive --filename-case`: a mirror gives the target the source's spelling, so the second run plans nothing.
/// Two source names of one key (where the file system keeps both): the one without a counterpart is left at planning as
/// `ambiguous_name`, and the summary says so; no item, and the next run the same.
#[test]
fn a6_31_case_insensitive_alignment_settles() {
    let s = Sandbox::new();
    s.file_at("l/a.txt", "left content\n", 2_000_000);
    s.file_at("r/A.txt", "right\n", 1_000_000);
    let flags = [
        "--mirror-right",
        "--case-insensitive",
        "--filename-case",
        "--yes",
        "--json",
    ];
    let out = sync(&s, &flags);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let names: Vec<String> = std::fs::read_dir(s.join("r"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["a.txt"]);
    assert_eq!(read(&s, "r/a.txt"), "left content\n");
    let again = sync(&s, &flags);
    assert_eq!(code(&again), 0, "{}", stderr(&again));
    assert_eq!(json(&again)["result"]["items"], serde_json::json!([]));
    // Two names of one key on the left, where this file system keeps them apart.
    let s = Sandbox::new();
    s.file_at("l/a.txt", "lower\n", 2_000_000);
    s.file_at("l/A.TXT", "UPPER\n", 2_000_000);
    s.file_at("r/a.txt", "r\n", 1_000_000);
    if read(&s, "l/a.txt") != "lower\n" {
        eprintln!("skipped: this file system folds names");
        return;
    }
    for _ in 0..2 {
        let out = sync(&s, &["--mirror-right", "--case-insensitive", "--yes"]);
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        let text = stdout(&out);
        assert!(text.contains("leave A.TXT (ambiguous_name)"), "{text}");
        assert!(
            text.contains("1 left as ambiguous names (two names align to one)"),
            "{text}"
        );
        assert!(!text.contains("skip ("), "{text}");
        assert!(!s.join("r/A.TXT").exists());
        assert_eq!(read(&s, "r/a.txt"), "lower\n");
    }
    let out = sync(
        &s,
        &[
            "--mirror-right",
            "--case-insensitive",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(json(&out)["result"]["counts"]["ambiguous"], 1);
}

/// An empty source is one with no file once the filters are applied: only an empty folder (`lost+found` of a mount point
/// that failed) or only a file a filter hides is refused without `--allow-empty-source`, and nothing is written. A source
/// folder with no file while the sync deletes the files of its counterpart is not refused: it is shown as *emptied
/// entirely* in the dry run, before the question and in the JSON.
#[test]
fn a6_31_an_empty_source_after_filters_and_folders_emptied_entirely() {
    let fresh = || {
        let s = Sandbox::new();
        s.dir("l");
        s.file_at("r/r.txt", "right\n", 1_000_000);
        s.file_at("r/d/f.txt", "inside\n", 1_000_000);
        s
    };
    let cases: [(&str, &[&str]); 2] = [("l/lost+found", &[]), ("l/.junk", &["--filters=-.junk"])];
    for (left, extra) in cases {
        let s = fresh();
        if left.ends_with("found") {
            s.dir(left);
        } else {
            s.file_at(left, "junk\n", 1_000_000);
        }
        let before = state(&s);
        let mut args = vec!["--mirror-right", "--yes", "--json"];
        args.extend_from_slice(extra);
        let out = sync(&s, &args);
        assert_eq!(code(&out), 100, "{left}: {}", stdout(&out));
        let doc = json(&out);
        assert_eq!(doc["error"]["kind"], "invalid_input", "{doc}");
        assert!(
            doc["error"]["message"]
                .as_str()
                .unwrap()
                .contains("the left side is empty"),
            "{doc}"
        );
        assert_eq!(state(&s), before, "{left}");
        // Allowed: it runs.
        args.push("--allow-empty-source");
        let out = sync(&s, &args);
        assert_eq!(code(&out), 0, "{left}: {}", stderr(&out));
        assert!(!s.join("r/r.txt").exists());
    }
    // A nested folder empty on the left: shown, not refused.
    let s = fresh();
    s.file_at("l/r.txt", "right\n", 1_000_000);
    s.dir("l/d");
    let dry = sync(&s, &["--mirror-right", "--dry-run"]);
    assert_eq!(code(&dry), 0, "{}", stderr(&dry));
    let line = "# emptied entirely: d on the right (the left holds no file there)";
    assert!(stdout(&dry).contains(line), "{}", stdout(&dry));
    let doc = json(&sync(&s, &["--mirror-right", "--dry-run", "--json"]));
    assert_eq!(
        doc["result"]["emptied"],
        serde_json::json!([{"path": "d", "empty": "left", "emptied": "right"}])
    );
    // The question comes after it: without a terminal the questions are refused, and the line was shown first.
    let before = state(&s);
    let ask = sync(&s, &["--mirror-right"]);
    assert_eq!(code(&ask), 100, "{}", stdout(&ask));
    assert!(stderr(&ask).contains(line), "{}", stderr(&ask));
    assert_eq!(state(&s), before);
    let run = sync(&s, &["--mirror-right", "--yes"]);
    assert_eq!(code(&run), 0, "{}", stderr(&run));
    assert!(!s.join("r/d/f.txt").exists());
}

/// A session file as text, in `<config>/sessions/<name>.toml`, with the sandbox's sides.
fn session_file(s: &Sandbox, name: &str, kind: &str, extra_specs: &str, rules: &str) {
    let sessions = s.config().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let l = s.join("l").display().to_string().replace('\\', "\\\\");
    let r = s.join("r").display().to_string().replace('\\', "\\\\");
    std::fs::write(
        sessions.join(format!("{name}.toml")),
        format!(
            "schema_version = 1\ntype = \"{kind}\"\n\n[specs]\nleft = \"{l}\"\nright = \"{r}\"\n{extra_specs}\n{rules}"
        ),
    )
    .unwrap();
}

/// Unknown keys of a session: a folder-sync session whose `[specs]` or rules hold a key this version does not know does not
/// run (it may be what protects a side, or a newer version's key that changes what is deleted), naming the key and the
/// nearest known one; nothing is written. Other keys, and other session types, run with a warning, always.
#[test]
fn a6_30_unknown_keys_of_a_sync_session_are_refused_and_others_warned() {
    let cases = [
        (
            "ro",
            "read_only = true",
            "[rules.sync]\npreset = \"mirror_right\"\n",
            "specs.read_only",
            "specs.read_only_",
        ),
        (
            "typo",
            "",
            "[rules.sync]\npreset = \"mirror_right\"\npresett = \"update_right\"\n",
            "rules.sync.presett",
            "rules.sync.preset`",
        ),
        (
            "rule",
            "",
            "[rules]\nfutur_filter = 1\n[rules.sync]\npreset = \"mirror_right\"\n",
            "rules.futur_filter",
            "nearest known key",
        ),
    ];
    for (name, specs, rules, key, near) in cases {
        let s = sandbox();
        session_file(&s, name, "folder-sync", specs, rules);
        let before = state(&s);
        for command in [
            vec!["session", "run", name, "--yes", "--json"],
            vec!["folder", "sync", "--session", name, "--yes", "--json"],
        ] {
            let out = s.cmd().args(&command).output().unwrap();
            assert_eq!(code(&out), 100, "{command:?}: {}", stdout(&out));
            let doc = json(&out);
            let message = doc["error"]["message"].as_str().unwrap().to_owned();
            assert!(message.contains(&format!("`{key}`")), "{message}");
            assert!(message.contains(near), "{message}");
            assert_eq!(state(&s), before, "{command:?}");
        }
    }
    // An unknown section of a sync session, and an unknown key of a folder-compare session: a warning, always.
    let s = sandbox();
    session_file(
        &s,
        "later",
        "folder-sync",
        "",
        "[rules.sync]\npreset = \"update_right\"\n\n[gui_layout]\nzoom = 2\n",
    );
    let out = s
        .cmd()
        .args(["session", "run", "later", "--dry-run", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let warnings = json(&out)["warnings"].clone();
    assert!(
        warnings
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "unknown_config_key"
                && w["message"].as_str().unwrap().contains("gui_layout.zoom")),
        "{warnings}"
    );
    let s = sandbox();
    session_file(&s, "cmp", "folder-compare", "read_only = true", "[rules]\n");
    let out = s.cmd().args(["session", "run", "cmp"]).output().unwrap();
    assert!(code(&out) < 100, "{}", stderr(&out));
    assert!(
        stderr(&out).contains("unknown key `specs.read_only`"),
        "{}",
        stderr(&out)
    );
}

/// `session edit --set` sets a per-row override of a folder-sync session (FS-004): a new path is a new entry, a bad action
/// or a path that names the base is refused, and the run honours it.
#[test]
fn a6_30_session_edit_sets_a_per_row_override() {
    let s = sandbox();
    let mut new = s.cmd();
    new.args(["session", "new", "o", "--type", "folder-sync", "--left"]);
    new.arg(s.join("l")).arg("--right").arg(s.join("r"));
    assert_eq!(code(&new.output().unwrap()), 0);
    let set = |pairs: &[&str]| {
        let mut args = vec!["edit", "o"];
        for pair in pairs {
            args.extend(["--set", pair]);
        }
        session(&s, &args)
    };
    let out = set(&[
        "rules.sync.preset=mirror_right",
        "rules.sync.overrides.\"right-only.txt\"=leave",
        "rules.sync.overrides.gone=leave",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let file = std::fs::read_to_string(s.config().join("sessions/o.toml")).unwrap();
    assert!(file.contains("\"right-only.txt\" = \"leave\""), "{file}");
    for bad in [
        "rules.sync.overrides.\"newer-left.txt\"=sideways",
        "rules.sync.overrides.\"..\"=leave",
    ] {
        let out = set(&[bad]);
        assert_eq!(code(&out), 100, "{bad}: {}", stdout(&out));
    }
    let before = std::fs::read_to_string(s.config().join("sessions/o.toml")).unwrap();
    assert_eq!(before, file, "a refused edit writes nothing");
    let run = session(&s, &["run", "o", "--yes"]);
    assert_eq!(code(&run), 0, "{}", stderr(&run));
    assert!(s.join("r/right-only.txt").exists());
    assert!(s.join("r/gone/inner.txt").exists());
    assert_eq!(read(&s, "r/newer-left.txt"), "left wins\n");
}
