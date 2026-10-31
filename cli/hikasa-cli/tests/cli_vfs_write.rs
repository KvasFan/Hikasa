//! `hikasa vfs cp|mv|rm|mkdir|touch` and `clipboard://` end to end (M2 W3, contract M2 §1.3.6–§1.3.7):
//! A3.8 (the Linux real-trash run in an isolated home), A3.9 (clipboard through `HIKASA_CLIPBOARD_FILE`), A3.10
//! (`gui open clipboard://` sends it verbatim), A3.11 (goldens, policies, the `resolve_targets` table, T11) and
//! A3.12 (reading commands, dry runs and a refused `rm` create nothing).

mod common;

use std::path::Path;
use std::sync::Arc;

#[cfg(unix)]
use common::assert_golden;
use common::{CloseMode, FakeGui, Sandbox, code, json, serve, stderr, stdout};
use hikasa_core::ExitCode;
use hikasa_session::ipc::Request;

/// `src/` with `a.txt` and `sub/b.txt`, plus `other.txt`.
fn sandbox() -> Sandbox {
    let s = Sandbox::new();
    s.file("src/a.txt", "alpha\n");
    s.file("src/sub/b.txt", "beta\n");
    s.file("other.txt", "other\n");
    s.settle("src");
    s
}

/// Every path below `root` (folders and files with their bytes), sorted.
fn tree(root: &Path) -> Vec<(String, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            let name = path.strip_prefix(root).unwrap().display().to_string();
            if meta.is_dir() {
                pending.push(path);
                out.push((name, None));
            } else {
                out.push((name, std::fs::read(&path).ok()));
            }
        }
    }
    out.sort();
    out
}

/// Runs `args` twice on fresh sandboxes (human, then `--json`) and checks both goldens and the exit code.
#[cfg(unix)]
fn goldens(name: &str, args: &[&str], want: i32, setup: impl Fn(&Sandbox)) -> serde_json::Value {
    let s = sandbox();
    setup(&s);
    let out = s.cmd().args(args).output().unwrap();
    assert_eq!(code(&out), want, "{args:?}: {}", stderr(&out));
    assert_golden(
        &format!("vfs_write/{name}.txt"),
        &s.normalise(&stdout(&out)),
    );
    let s = sandbox();
    setup(&s);
    let out = s.cmd().args(args).arg("--json").output().unwrap();
    assert_eq!(code(&out), want, "{args:?} --json: {}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["exit_code"], want);
    assert_golden(
        &format!("vfs_write/{name}.json"),
        &s.normalise(&stdout(&out)),
    );
    doc
}

#[cfg(unix)]
#[test]
fn a3_11_cp_goldens() {
    let doc = goldens("cp", &["vfs", "cp", "-r", "src", "dst"], 0, |_| {});
    assert_eq!(doc["schema"], "hikasa.vfs_op.v1");
    assert_eq!(doc["result"]["summary"]["done"], 4);
    goldens(
        "cp_dry_run",
        &["vfs", "cp", "-r", "src", "dst", "--dry-run"],
        0,
        |_| {},
    );
    // A folder without --recursive is an item error.
    goldens(
        "cp_folder_without_recursive",
        &["vfs", "cp", "src", "dst"],
        100,
        |_| {},
    );
}

#[cfg(unix)]
#[test]
fn a3_11_cp_partial_failure_keeps_ok_true() {
    let doc = goldens(
        "cp_partial_failure",
        &["vfs", "cp", "src/a.txt", "other.txt", "dst/"],
        100,
        |s| {
            s.file("dst/other.txt", "old\n");
        },
    );
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["error"], serde_json::Value::Null);
    let items = doc["result"]["items"].as_array().unwrap();
    assert_eq!(items[0]["status"], "done");
    assert_eq!(items[1]["status"], "failed");
    assert_eq!(items[1]["error"]["kind"], "already_exists");
}

#[cfg(unix)]
#[test]
fn a3_11_whole_command_failures() {
    let doc = goldens(
        "cp_several_into_a_file",
        &["vfs", "cp", "src/a.txt", "src/sub/b.txt", "other.txt"],
        100,
        |_| {},
    );
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["error"]["kind"], "invalid_input");
    let doc = goldens("cp_bad_spec", &["vfs", "cp", "ftp://", "x"], 100, |_| {});
    assert_eq!(doc["ok"], false);
}

#[cfg(unix)]
#[test]
fn a3_11_mv_rm_mkdir_touch_goldens() {
    goldens("mv", &["vfs", "mv", "src", "moved"], 0, |_| {});
    goldens(
        "mv_dry_run",
        &["vfs", "mv", "src", "moved", "--dry-run"],
        0,
        |_| {},
    );
    goldens("rm", &["vfs", "rm", "-r", "--yes", "src"], 0, |_| {});
    goldens(
        "rm_dry_run",
        &["vfs", "rm", "-r", "--dry-run", "src"],
        0,
        |_| {},
    );
    goldens("rm_not_empty", &["vfs", "rm", "--yes", "src"], 100, |_| {});
    goldens(
        "mkdir",
        &["vfs", "mkdir", "-p", "new/deep", "src"],
        0,
        |_| {},
    );
    goldens("mkdir_exists", &["vfs", "mkdir", "src"], 100, |_| {});
    goldens(
        "touch_time",
        &[
            "vfs",
            "touch",
            "--time",
            "2026-01-02T03:04:05Z",
            "other.txt",
        ],
        0,
        |_| {},
    );
    goldens(
        "touch_create",
        &[
            "vfs",
            "touch",
            "--create",
            "--time",
            "2026-01-02T03:04:05Z",
            "new.txt",
            "missing/x",
        ],
        100,
        |_| {},
    );
    goldens(
        "touch_dry_run",
        &[
            "vfs",
            "touch",
            "--dry-run",
            "--time",
            "2026-01-02T03:04:05Z",
            "other.txt",
        ],
        0,
        |_| {},
    );
}

#[test]
fn a3_11_touch_sets_the_time_and_reference_and_created() {
    let s = sandbox();
    let out = s
        .cmd()
        .args([
            "vfs",
            "touch",
            "--time",
            "2026-01-02T03:04:05Z",
            "other.txt",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let mtime = |p: &str| {
        filetime::FileTime::from_last_modification_time(&std::fs::metadata(s.join(p)).unwrap())
            .unix_seconds()
    };
    assert_eq!(mtime("other.txt"), 1_767_323_045);
    let out = s
        .cmd()
        .args(["vfs", "touch", "--reference", "other.txt", "src/a.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(mtime("src/a.txt"), 1_767_323_045);
    let conflict = s
        .cmd()
        .args([
            "vfs",
            "touch",
            "--time",
            "2026-01-02T03:04:05Z",
            "--reference",
            "x",
            "y",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&conflict), 100, "--time and --reference conflict");
    let out = s
        .cmd()
        .args([
            "vfs",
            "touch",
            "--created",
            "2026-01-02T03:04:05Z",
            "other.txt",
            "--json",
        ])
        .output()
        .unwrap();
    let doc = json(&out);
    if cfg!(any(windows, target_os = "macos")) {
        assert_eq!(code(&out), 0, "{}", stderr(&out));
    } else {
        // No creation times on Linux: the item fails `unsupported`.
        assert_eq!(code(&out), 100);
        assert_eq!(doc["result"]["items"][0]["error"]["kind"], "unsupported");
    }
    let bad = s
        .cmd()
        .args(["vfs", "touch", "--time", "yesterday", "other.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&bad), 100);
}

#[test]
fn a3_11_rm_without_yes_is_refused_and_changes_nothing() {
    let s = sandbox();
    let before = tree(s.path());
    let out = s
        .cmd()
        .args(["vfs", "rm", "-r", "src", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    let doc = json(&out);
    assert_eq!(doc["error"]["kind"], "invalid_input");
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .contains("pass --yes, or --dry-run"),
        "{doc}"
    );
    assert_eq!(tree(s.path()), before);
    let both = s
        .cmd()
        .args(["vfs", "rm", "--yes", "--dry-run", "src"])
        .output()
        .unwrap();
    assert_eq!(code(&both), 100, "--yes and --dry-run conflict");
}

#[test]
fn a3_11_each_overwrite_policy() {
    // `other.txt` is older than the source, `newer.txt` newer.
    for (policy, status, reason, content) in [
        ("fail", "failed", None, "old\n"),
        ("skip", "planned", Some("exists"), "old\n"),
        ("always", "done", None, "alpha\n"),
        ("newer", "done", None, "alpha\n"),
    ] {
        let s = sandbox();
        s.file_at("dst.txt", "old\n", common::MTIME - 100);
        let out = s
            .cmd()
            .args([
                "vfs",
                "cp",
                "src/a.txt",
                "dst.txt",
                "--overwrite",
                policy,
                "--json",
            ])
            .output()
            .unwrap();
        let doc = json(&out);
        let item = &doc["result"]["items"][0];
        let want_status = if policy == "skip" { "skipped" } else { status };
        assert_eq!(item["status"], want_status, "{policy}: {doc}");
        assert_eq!(item["reason"].as_str(), reason, "{policy}");
        assert_eq!(
            std::fs::read_to_string(s.join("dst.txt")).unwrap(),
            content,
            "{policy}"
        );
        assert_eq!(
            code(&out),
            if policy == "fail" { 100 } else { 0 },
            "{policy}"
        );
    }
    let s = sandbox();
    s.file_at("dst.txt", "newer\n", common::MTIME + 100);
    let out = s
        .cmd()
        .args([
            "vfs",
            "cp",
            "src/a.txt",
            "dst.txt",
            "--overwrite",
            "newer",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(json(&out)["result"]["items"][0]["reason"], "not_newer");
    let bad = s
        .cmd()
        .args([
            "vfs",
            "cp",
            "src/a.txt",
            "dst.txt",
            "--overwrite",
            "sometimes",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&bad), 100);
    assert!(stderr(&bad).contains("sometimes"), "{}", stderr(&bad));
}

#[cfg(unix)]
#[test]
fn a3_11_symlinks_skip_and_copy() {
    let s = sandbox();
    std::os::unix::fs::symlink("a.txt", s.join("src/link")).unwrap();
    let out = s
        .cmd()
        .args([
            "vfs",
            "cp",
            "-r",
            "--symlinks",
            "skip",
            "src",
            "dst",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let doc = json(&out);
    let link = doc["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["target"].as_str().unwrap().ends_with("/link"))
        .unwrap()
        .clone();
    assert_eq!(link["reason"], "symlink");
    assert!(std::fs::symlink_metadata(s.join("dst/link")).is_err());
    let out = s
        .cmd()
        .args(["vfs", "cp", "-r", "src", "dst2"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        std::fs::read_link(s.join("dst2/link")).unwrap(),
        Path::new("a.txt"),
        "copied as a link"
    );
}

#[test]
fn a3_11_resolve_targets_table() {
    let target_of = |s: &Sandbox, args: &[&str]| -> (i32, serde_json::Value) {
        let out = s.cmd().args(args).arg("--json").output().unwrap();
        (code(&out), json(&out))
    };
    let s = sandbox();
    s.dir("folder");
    // An existing folder receives the source under its name.
    let (c, doc) = target_of(&s, &["vfs", "cp", "other.txt", "folder"]);
    assert_eq!(c, 0);
    assert!(
        doc["result"]["items"][0]["target"]
            .as_str()
            .unwrap()
            .ends_with("other.txt")
    );
    // A missing target is the new name.
    let (c, _) = target_of(&s, &["vfs", "cp", "other.txt", "renamed.txt"]);
    assert_eq!(c, 0);
    assert!(s.join("renamed.txt").is_file());
    // A trailing separator: a folder; missing without --parents → not_found, with it → created.
    let (c, doc) = target_of(&s, &["vfs", "cp", "other.txt", "into/"]);
    assert_eq!((c, doc["error"]["kind"].as_str()), (100, Some("not_found")));
    let (c, _) = target_of(&s, &["vfs", "cp", "-p", "other.txt", "into/"]);
    assert_eq!(c, 0);
    assert!(s.join("into/other.txt").is_file());
    // Several sources need an existing folder.
    let (c, doc) = target_of(&s, &["vfs", "cp", "other.txt", "src/a.txt", "nowhere"]);
    assert_eq!(c, 100);
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .contains("is not an existing folder")
    );
    // A blob source into a folder needs a file name.
    std::fs::write(s.join("clip.txt"), "clip").unwrap();
    let out = s
        .cmd()
        .env("HIKASA_CLIPBOARD_FILE", s.join("clip.txt"))
        .args(["vfs", "cp", "clipboard://", "folder", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    assert!(
        json(&out)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("needs a file name")
    );
    // Target inside the source, target = source: item errors.
    let (c, doc) = target_of(&s, &["vfs", "cp", "-r", "src", "src/sub/inner"]);
    assert_eq!(c, 100);
    assert!(
        doc["result"]["items"][0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("is inside the source folder")
    );
    let (c, doc) = target_of(&s, &["vfs", "cp", "other.txt", "other.txt"]);
    assert_eq!(c, 100);
    assert!(
        doc["result"]["items"][0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("is the source itself")
    );
}

/// T11: the CLI's JSON result equals a direct `Service::vfs_run` on an identical tree.
#[test]
fn a3_11_cli_equals_the_service() {
    use hikasa_session::api::VfsOpRequest;
    use hikasa_vfs::ops::CopyOptions;
    let cli = sandbox();
    let out = cli
        .cmd()
        .args(["vfs", "cp", "-r", "src", "dst", "--json"])
        .output()
        .unwrap();
    let cli_result = cli.normalise(&json(&out)["result"].to_string());
    let direct = sandbox();
    let env = hikasa_session::EnvFacts {
        cwd: Some(direct.path().to_path_buf()),
        ..hikasa_session::EnvFacts::new(hikasa_session::Platform::HOST, direct.path().to_path_buf())
    };
    let init = hikasa_session::ServiceInit {
        config_dir: Some(direct.config()),
        ..hikasa_session::ServiceInit::new(
            env,
            Arc::new(hikasa_session::FixedClock(jiff::Zoned::now())),
            Arc::new(hikasa_formats::StdProcessRunner),
        )
    };
    let service = hikasa_session::Service::new(init).unwrap();
    let job = service.job(
        hikasa_core::JobKind::VfsWrite,
        Arc::new(hikasa_core::NullProgress),
    );
    let dto = service
        .vfs_run(
            &VfsOpRequest::Copy {
                sources: vec!["src".into()],
                target: "dst".into(),
                options: CopyOptions {
                    recursive: true,
                    ..CopyOptions::default()
                },
            },
            &job,
        )
        .unwrap();
    let direct_result = direct.normalise(&serde_json::to_value(&dto).unwrap().to_string());
    assert_eq!(cli_result, direct_result);
}

#[test]
fn a3_11_version_lists_35_schemas_and_help_resolves() {
    let s = Sandbox::new();
    let out = s.cmd().args(["version", "--json"]).output().unwrap();
    let doc = json(&out);
    let schemas = doc["result"]["schemas"].as_array().unwrap();
    assert_eq!(schemas.len(), 35);
    assert!(schemas.iter().any(|v| v == "hikasa.vfs_op.v1"));
    let out = s
        .cmd()
        .args(["help", "vfs", "cp", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(json(&out)["result"]["topic"]["id"], "vfs");
}

#[test]
fn a3_12_reading_dry_runs_and_refusals_create_nothing() {
    let s = sandbox();
    std::fs::write(s.join("clip.txt"), "clip\n").unwrap();
    let cache = s.join("cache");
    let before = tree(s.path());
    let runs: &[&[&str]] = &[
        &["vfs", "ls", "src"],
        &["vfs", "stat", "src/a.txt"],
        &["vfs", "cat", "src/a.txt"],
        &["vfs", "cat", "clipboard://"],
        &["vfs", "stat", "clipboard://"],
        &["vfs", "cp", "-r", "src", "dst", "--dry-run"],
        &["vfs", "mv", "src", "moved", "--dry-run"],
        &["vfs", "rm", "-r", "src", "--dry-run"],
        &["vfs", "rm", "-r", "src"],
        &["vfs", "mkdir", "-p", "a/b", "--dry-run"],
        &["vfs", "touch", "--create", "n.txt", "--dry-run"],
    ];
    for args in runs {
        let out = s
            .cmd()
            .env("HIKASA_CLIPBOARD_FILE", s.join("clip.txt"))
            .env("XDG_CACHE_HOME", &cache)
            .args(*args)
            .output()
            .unwrap();
        assert!(code(&out) == 0 || code(&out) == 100, "{args:?}");
    }
    assert_eq!(tree(s.path()), before, "nothing was created or changed");
}

// ---- A3.9 through HIKASA_CLIPBOARD_FILE ---------------------------------------------------------------------

#[test]
fn a3_9_clipboard_file_text_empty_and_missing() {
    let s = sandbox();
    std::fs::write(s.join("clip.txt"), "alpha\n").unwrap();
    std::fs::write(s.join("empty.txt"), "").unwrap();
    let run = |clip: &str, args: &[&str]| {
        s.cmd()
            .env("HIKASA_CLIPBOARD_FILE", s.join(clip))
            .args(args)
            .output()
            .unwrap()
    };
    let out = run("clip.txt", &["quick", "clipboard://", "src/a.txt"]);
    assert_eq!(
        code(&out),
        i32::from(ExitCode::BinarySame.code()),
        "{}",
        stderr(&out)
    );
    let out = run("clip.txt", &["vfs", "cat", "clipboard://"]);
    assert_eq!(stdout(&out), "alpha\n");
    let out = run("clip.txt", &["vfs", "cp", "clipboard://", "pasted.txt"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(s.join("pasted.txt")).unwrap(),
        "alpha\n"
    );
    for (clip, kind) in [("empty.txt", "not_found"), ("missing.txt", "unsupported")] {
        for args in [
            &["quick", "clipboard://", "src/a.txt", "--json"][..],
            &["text", "compare", "clipboard://", "src/a.txt", "--json"],
            &["vfs", "cat", "clipboard://", "--json"],
            &["vfs", "stat", "clipboard://", "--json"],
        ] {
            let out = run(clip, args);
            assert_eq!(code(&out), 100, "{clip} {args:?}");
            assert_eq!(json(&out)["error"]["kind"], kind, "{clip} {args:?}");
        }
    }
}

// ---- A3.10 ---------------------------------------------------------------------------------------------------

#[test]
fn a3_10_gui_open_sends_clipboard_verbatim() {
    let s = sandbox();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame);
    let server = serve(&s, Arc::clone(&gui));
    let out = s
        .cmd()
        .args(["gui", "open", "clipboard://", "src/a.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let specs = gui
        .requests()
        .into_iter()
        .find_map(|r| match r {
            Request::OpenView { specs, .. } => Some(specs),
            _ => None,
        })
        .unwrap();
    assert_eq!(specs[0], "clipboard://");
    server.shutdown().unwrap();
}

// ---- A3.8: the real recycle bin, isolated (Linux) ------------------------------------------------------------

#[cfg(target_os = "linux")]
#[test]
fn a3_8_rm_trash_uses_an_isolated_home() {
    let s = sandbox();
    let home = s.dir("home");
    let data = s.dir("home/data");
    let victim = s.file("home/victim.txt", "bye\n");
    let out = s
        .cmd()
        .env("HOME", &home)
        .env("XDG_DATA_HOME", &data)
        .args(["vfs", "rm", "--trash", "--yes"])
        .arg(&victim)
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!victim.exists());
    let trashed = data.join("Trash/files/victim.txt");
    assert_eq!(std::fs::read_to_string(&trashed).unwrap(), "bye\n");
    let info = std::fs::read_to_string(data.join("Trash/info/victim.txt.trashinfo")).unwrap();
    assert!(info.contains("Path="), "{info}");
    assert!(info.contains("DeletionDate="), "{info}");
}

#[cfg(not(target_os = "linux"))]
#[ignore = "manual: moves a file to the real recycle bin of this platform"]
#[test]
fn a3_8_rm_trash_uses_an_isolated_home() {}

// ---- W3 review fixes (§1.3.12) -----------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn fix_blob_targets_are_unsupported() {
    for (name, target) in [("cp_to_stdin", "-"), ("cp_to_clipboard", "clipboard://")] {
        let doc = goldens(name, &["vfs", "cp", "other.txt", target], 100, |_| {});
        assert_eq!(doc["ok"], false);
        assert_eq!(doc["error"]["kind"], "unsupported");
        let message = doc["error"]["message"].as_str().unwrap();
        assert!(!message.contains("in this version"), "{message}");
        assert!(
            message.ends_with("write is not supported here"),
            "{message}"
        );
    }
}

#[cfg(unix)]
#[test]
fn fix_dot_names_are_never_removed_or_moved() {
    for args in [
        &["vfs", "rm", "-r", "--yes", "."][..],
        &["vfs", "rm", "-r", "--yes", "./"],
        &["vfs", "rm", "-r", "--yes", ".."],
        &["vfs", "rm", "-r", "--yes", "src/sub/.."],
        &["vfs", "rm", "-r", "--yes", "src/."],
        &["vfs", "rm", "-r", "--trash", "--yes", "src/sub/.."],
        &["vfs", "rm", "-r", "--dry-run", "src/."],
        &["vfs", "mv", "src/.", "moved"],
        &["vfs", "mv", "src/sub/..", "moved"],
    ] {
        let s = sandbox();
        let before = tree(s.path());
        let out = s.cmd().args(args).arg("--json").output().unwrap();
        assert_eq!(code(&out), 100, "{args:?}: {}", stderr(&out));
        let doc = json(&out);
        assert_eq!(doc["ok"], false, "{args:?}");
        assert_eq!(doc["error"]["kind"], "invalid_input", "{args:?}");
        assert!(
            doc["error"]["message"]
                .as_str()
                .unwrap()
                .ends_with("needs a file name"),
            "{args:?}: {doc}"
        );
        assert_eq!(tree(s.path()), before, "{args:?}: nothing changed");
    }
    // `cp x .` still copies into the current folder.
    let s = sandbox();
    let out = s
        .cmd()
        .args(["vfs", "cp", "src/a.txt", "."])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(std::fs::read(s.join("a.txt")).unwrap(), b"alpha\n");
}
