//! `hikasa session …`, `hikasa workspace …`, `settings export|import|restore-defaults --category`, `gui session`,
//! `gui workspace` and the D-88 forms of `gui open` end to end (M2 W2, contract §6.2 A2.7, A2.8): goldens in text
//! and JSON in a temporary `--config-dir`, `session run` = `folder compare` / `text compare` (T11), CLI-002
//! resolution order, read-only commands create nothing (D-21).

mod common;

use common::{Sandbox, assert_golden, code, json, stderr, stdout};

/// Runs `hikasa <args>` and returns (code, stdout, stderr).
fn run(s: &Sandbox, args: &[&str]) -> (i32, String, String) {
    let out = s.cmd().args(args).output().unwrap();
    (code(&out), stdout(&out), stderr(&out))
}

/// Runs and checks exit 0.
fn ok(s: &Sandbox, args: &[&str]) -> String {
    let (c, out, err) = run(s, args);
    assert_eq!(c, 0, "{args:?}: {err}");
    s.normalise(&out)
}

/// Two text files and two folders.
fn sandbox() -> Sandbox {
    let s = Sandbox::new();
    s.file("a.c", "int a;\n  int x;\n");
    s.file("b.c", "int b;\nint x;\n");
    s.file("l/x.txt", "x\n");
    s.file("l/only.txt", "l\n");
    s.file("r/x.txt", "y\n");
    s
}

#[test]
fn read_only_commands_create_nothing() {
    let s = sandbox();
    for args in [
        &["session", "list"][..],
        &["session", "list", "--autosaved", "--defaults", "--json"],
        &["session", "show", "--defaults", "text-compare"],
        &["session", "show", "--defaults", "folder-compare", "--json"],
        &["workspace", "list", "--json"],
        &["settings", "get", "options.appearance.theme_mode"],
    ] {
        ok(&s, args);
    }
    // Failing reads create nothing either.
    for args in [
        &["session", "show", "nope"][..],
        &["session", "show", "nope", "--json"],
        &["session", "list", "--folder", "nope"],
        &["session", "run", "nope"],
        &["workspace", "show", "nope", "--json"],
        &["settings", "get", "options.nope"],
    ] {
        let (c, _, _) = run(&s, args);
        assert_eq!(c, 100, "{args:?}");
    }
    // A dry-run import of a package made elsewhere.
    let other = sandbox();
    ok(
        &other,
        &[
            "session",
            "new",
            "t",
            "--type",
            "text-compare",
            "--left",
            "a.c",
            "--right",
            "b.c",
        ],
    );
    let pkg = other.join("p.hikasa-pkg");
    ok(&other, &["settings", "export", "-o", pkg.to_str().unwrap()]);
    ok(
        &s,
        &[
            "settings",
            "import",
            pkg.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    );
    assert!(!s.config().exists(), "D-21");
}

/// D-90: `session edit --unset` removes a rule key, so the session follows the defaults again.
#[test]
fn session_edit_unset() {
    let s = sandbox();
    ok(
        &s,
        &[
            "session",
            "new",
            "f",
            "--type",
            "folder-compare",
            "--left",
            "l",
            "--right",
            "r",
        ],
    );
    ok(
        &s,
        &[
            "session",
            "edit",
            "f",
            "--set",
            "rules.handling.flatten=true",
            "--set",
            "rules.criteria.size=false",
        ],
    );
    assert_golden(
        "session/edit-unset.txt",
        &ok(
            &s,
            &["session", "edit", "f", "--unset", "rules.handling.flatten"],
        ),
    );
    assert_golden(
        "session/edit-unset.json",
        &ok(
            &s,
            &[
                "session",
                "edit",
                "f",
                "--unset",
                "rules.criteria.size",
                "--json",
            ],
        ),
    );
    let file = std::fs::read_to_string(s.join("cfg/sessions/f.toml")).unwrap();
    assert!(!file.contains("[rules"), "{file}");
    let (c, out, _) = run(
        &s,
        &["session", "edit", "f", "--unset", "rules.nope", "--json"],
    );
    assert_eq!(
        (c, json_text(&out)["error"]["kind"].as_str()),
        (100, Some("not_found"))
    );
}

/// SES-004: a locked session is deleted or renamed only with `--force`.
#[test]
fn locked_sessions_need_force() {
    let s = sandbox();
    for name in ["a", "b"] {
        ok(
            &s,
            &[
                "session",
                "new",
                name,
                "--type",
                "text-compare",
                "--left",
                "a.c",
            ],
        );
        ok(&s, &["session", "lock", name]);
    }
    for args in [
        &["session", "delete", "a", "--json"][..],
        &["session", "rename", "b", "c", "--json"],
    ] {
        let (c, out, err) = run(&s, args);
        assert_eq!(
            (c, json_text(&out)["error"]["kind"].as_str()),
            (100, Some("invalid_input")),
            "{args:?}"
        );
        assert!(err.contains("locked"), "{err}");
    }
    assert!(s.join("cfg/sessions/a.toml").is_file());
    assert!(s.join("cfg/sessions/b.toml").is_file());
    ok(&s, &["session", "delete", "a", "--force"]);
    ok(&s, &["session", "rename", "b", "c", "--force"]);
    assert!(!s.join("cfg/sessions/a.toml").exists());
    assert!(!s.join("cfg/sessions/b.toml").exists());
    let moved = std::fs::read_to_string(s.join("cfg/sessions/c.toml")).unwrap();
    assert!(moved.contains("locked = true"), "{moved}");
}

/// D-80 / D-88: session references resolve case-insensitively in `session show|run` and `gui open`.
#[test]
fn references_resolve_case_insensitively() {
    let s = sandbox();
    ok(
        &s,
        &[
            "session",
            "new",
            "work/t",
            "--type",
            "text-compare",
            "--left",
            "a.c",
            "--right",
            "b.c",
        ],
    );
    let shown = json_text(&ok(&s, &["session", "show", "WORK/T", "--json"]));
    assert_eq!(shown["result"]["ref"], "work/t");
    assert_eq!(
        compare(&s, &["session", "run", "WORK/T", "--json"]),
        compare(&s, &["session", "run", "work/t", "--json"])
    );
    // `gui open` resolves the session, then finds no GUI (not the one-spec error of an unknown name).
    let (c, _, err) = run(&s, &["gui", "open", "work/T", "--no-launch"]);
    assert_eq!(c, 100);
    assert!(err.contains("not running"), "{err}");
    let (c, _, err) = run(&s, &["gui", "open", "work/zz", "--no-launch"]);
    assert_eq!(c, 100);
    assert!(err.contains("two specs"), "{err}");
}

/// SES-004 / D-91: an import and a restore keep locked sessions (`skip_locked`, warning `session_locked`)
/// unless `--force`.
#[test]
fn import_and_restore_keep_locked_sessions() {
    let s = sandbox();
    ok(
        &s,
        &[
            "session",
            "new",
            "t",
            "--type",
            "text-compare",
            "--description",
            "from the package",
        ],
    );
    let pkg = s.join("p.hikasa-pkg");
    let pkg_arg = pkg.to_str().unwrap();
    ok(
        &s,
        &[
            "settings",
            "export",
            "-o",
            pkg_arg,
            "--category",
            "sessions",
        ],
    );
    let t = Sandbox::new();
    ok(
        &t,
        &[
            "session",
            "new",
            "t",
            "--type",
            "text-compare",
            "--description",
            "mine",
        ],
    );
    ok(&t, &["session", "lock", "t"]);
    let target = t.join("cfg/sessions/t.toml");
    let before = std::fs::read(&target).unwrap();
    let plan = t.normalise(&ok(
        &t,
        &["settings", "import", pkg_arg, "--dry-run", "--json"],
    ));
    assert_golden("session/import-locked.json", &plan);
    let doc = json_text(&plan);
    assert_eq!(doc["result"]["plan"]["items"][0]["action"], "skip_locked");
    assert_eq!(doc["warnings"][0]["code"], "session_locked");
    let (c, _, err) = run(&t, &["settings", "import", pkg_arg, "--yes"]);
    assert_eq!(c, 0, "{err}");
    assert!(err.contains("locked"), "{err}");
    assert_eq!(std::fs::read(&target).unwrap(), before);
    ok(&t, &["settings", "import", pkg_arg, "--yes", "--force"]);
    let text = std::fs::read_to_string(&target).unwrap();
    assert!(text.contains("from the package"), "{text}");
    // Restore: --delete-sessions keeps the locked session unless forced.
    ok(&t, &["session", "lock", "t"]);
    ok(&t, &["session", "new", "u", "--type", "text-compare"]);
    let plan = t.normalise(&ok(
        &t,
        &[
            "settings",
            "restore-defaults",
            "--category",
            "sessions",
            "--delete-sessions",
            "--dry-run",
            "--json",
        ],
    ));
    assert_golden("session/restore-locked.json", &plan);
    let (c, _, err) = run(
        &t,
        &[
            "settings",
            "restore-defaults",
            "--category",
            "sessions",
            "--delete-sessions",
            "--yes",
        ],
    );
    assert_eq!(c, 0, "{err}");
    assert!(err.contains("restore with --force"), "{err}");
    assert!(target.is_file());
    assert!(!t.join("cfg/sessions/u.toml").exists());
    ok(
        &t,
        &[
            "settings",
            "restore-defaults",
            "--category",
            "sessions",
            "--delete-sessions",
            "--yes",
            "--force",
        ],
    );
    assert!(!target.exists());
    let (c, _, err) = run(
        &t,
        &[
            "settings",
            "restore-defaults",
            "--category",
            "sessions",
            "--force",
            "--yes",
        ],
    );
    assert_eq!(c, 100, "--force requires --delete-sessions");
    assert!(err.contains("--delete-sessions"), "{err}");
}

#[test]
fn session_command_goldens() {
    let s = sandbox();
    assert_golden(
        "session/mkdir.json",
        &ok(&s, &["session", "mkdir", "work", "--json"]),
    );
    assert_golden(
        "session/new.txt",
        &ok(
            &s,
            &[
                "session",
                "new",
                "work/api",
                "--type",
                "folder-compare",
                "--left",
                "l",
                "--right",
                "r",
                "--description",
                "The API",
            ],
        ),
    );
    assert_golden(
        "session/new.json",
        &ok(
            &s,
            &[
                "session",
                "new",
                "t",
                "--type",
                "text-compare",
                "--left",
                "a.c",
                "--right",
                "b.c",
                "--json",
            ],
        ),
    );
    // stdin is never stored.
    let (c, out, err) = run(
        &s,
        &[
            "session",
            "new",
            "piped",
            "--type",
            "text-edit",
            "--left",
            "-",
            "--json",
        ],
    );
    assert_eq!(c, 0, "{err}");
    assert_eq!(json_text(&out)["warnings"][0]["code"], "spec_not_saved");
    assert_golden("session/list.txt", &ok(&s, &["session", "list"]));
    assert_golden("session/list.json", &ok(&s, &["session", "list", "--json"]));
    assert_golden(
        "session/list-folder.txt",
        &ok(&s, &["session", "list", "--folder", "WORK"]),
    );
    assert_golden(
        "session/show.txt",
        &ok(&s, &["session", "show", "work/api"]),
    );
    assert_golden(
        "session/show.json",
        &ok(&s, &["session", "show", "Work/API", "--json"]),
    );
    assert_golden(
        "session/edit.json",
        &ok(
            &s,
            &[
                "session",
                "edit",
                "work/api",
                "--set",
                "rules.criteria.size=false",
                "--set",
                "view.split=0.25",
                "--description",
                "Changed",
                "--json",
            ],
        ),
    );
    let shown = ok(&s, &["session", "show", "work/api"]);
    assert!(
        shown.contains("size = false") && shown.contains("split = 0.25"),
        "{shown}"
    );
    // An unknown key: not_found with a suggestion; a bad value: invalid_input.
    let (c, out, _) = run(
        &s,
        &[
            "session",
            "edit",
            "work/api",
            "--set",
            "rules.criteria.sise=false",
            "--json",
        ],
    );
    assert_eq!(c, 100);
    let doc = json_text(&out);
    assert_eq!(doc["error"]["kind"], "not_found");
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .contains("rules.criteria.size"),
        "{doc}"
    );
    let (c, out, _) = run(
        &s,
        &[
            "session",
            "edit",
            "work/api",
            "--set",
            "rules.criteria.size=7",
            "--json",
        ],
    );
    assert_eq!(
        (c, json_text(&out)["error"]["kind"].as_str()),
        (100, Some("invalid_input"))
    );
    let (c, _, _) = run(
        &s,
        &["session", "edit", "work/api", "--set", "type=text-compare"],
    );
    assert_eq!(c, 100);
    // Lock: edits are refused unless forced.
    assert_golden(
        "session/lock.txt",
        &ok(&s, &["session", "lock", "work/api"]),
    );
    let (c, _, err) = run(&s, &["session", "edit", "work/api", "--description", "x"]);
    assert_eq!(c, 100);
    assert!(err.contains("locked"), "{err}");
    ok(
        &s,
        &[
            "session",
            "edit",
            "work/api",
            "--description",
            "x",
            "--force",
        ],
    );
    ok(&s, &["session", "unlock", "work/api"]);
    // Defaults.
    assert_golden(
        "session/edit-defaults.txt",
        &ok(
            &s,
            &[
                "session",
                "edit",
                "--defaults",
                "text-compare",
                "--set",
                "rules.alignment.skew_tolerance=4",
            ],
        ),
    );
    assert_golden(
        "session/show-defaults.txt",
        &ok(&s, &["session", "show", "--defaults", "text-compare"]),
    );
    let (c, _, _) = run(
        &s,
        &[
            "session",
            "edit",
            "--defaults",
            "text-compare",
            "--set",
            "specs.left=/x",
        ],
    );
    assert_eq!(c, 100, "the defaults hold no specs");
    // Rename, collisions, delete, folders.
    assert_golden(
        "session/rename.json",
        &ok(
            &s,
            &["session", "rename", "work/api", "archive/api", "--json"],
        ),
    );
    let (c, out, _) = run(
        &s,
        &["session", "new", "T", "--type", "text-compare", "--json"],
    );
    assert_eq!(
        (c, json_text(&out)["error"]["kind"].as_str()),
        (100, Some("already_exists"))
    );
    let (c, out, _) = run(&s, &["session", "rmdir", "archive", "--json"]);
    assert_eq!(
        (c, json_text(&out)["error"]["kind"].as_str()),
        (100, Some("invalid_input"))
    );
    // D-91: a lock inside stops --recursive until --force.
    ok(&s, &["session", "lock", "archive/api"]);
    let (c, out, _) = run(
        &s,
        &["session", "rmdir", "archive", "--recursive", "--json"],
    );
    let doc = json_text(&out);
    assert_eq!(
        (c, doc["error"]["kind"].as_str()),
        (100, Some("invalid_input"))
    );
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .contains("1 locked")
    );
    ok(&s, &["session", "unlock", "archive/api"]);
    assert_golden(
        "session/rmdir.txt",
        &ok(&s, &["session", "rmdir", "archive", "--recursive"]),
    );
    assert_golden(
        "session/delete.json",
        &ok(&s, &["session", "delete", "t", "--json"]),
    );
    let (c, out, _) = run(&s, &["session", "delete", "t", "--json"]);
    assert_eq!(
        (c, json_text(&out)["error"]["kind"].as_str()),
        (100, Some("not_found"))
    );
    // P1 types and bad names.
    let (c, out, _) = run(
        &s,
        &["session", "new", "m", "--type", "text-merge", "--json"],
    );
    assert_eq!(
        (c, json_text(&out)["error"]["kind"].as_str()),
        (100, Some("unsupported"))
    );
    let (c, out, _) = run(
        &s,
        &["session", "new", "_x", "--type", "text-compare", "--json"],
    );
    assert_eq!(
        (c, json_text(&out)["error"]["kind"].as_str()),
        (100, Some("invalid_input"))
    );
}

fn json_text(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

/// The JSON of a compare command without the volatile parts (none here: the inputs have fixed times).
fn compare(s: &Sandbox, args: &[&str]) -> (i32, String) {
    let (c, out, err) = run(s, args);
    assert!(c != 100, "{args:?}: {err}");
    (c, s.normalise(&out))
}

/// T11: `session run` gives the exit code and output of `text compare` / `folder compare` on the same inputs and
/// rules.
#[test]
fn session_run_equals_the_compare_commands() {
    let s = sandbox();
    ok(
        &s,
        &[
            "session",
            "new",
            "t",
            "--type",
            "text-compare",
            "--left",
            "a.c",
            "--right",
            "b.c",
        ],
    );
    ok(
        &s,
        &[
            "session",
            "new",
            "f",
            "--type",
            "folder-compare",
            "--left",
            "l",
            "--right",
            "r",
        ],
    );
    assert_eq!(
        compare(&s, &["session", "run", "t", "--json"]),
        compare(&s, &["text", "compare", "a.c", "b.c", "--json"])
    );
    assert_eq!(
        compare(&s, &["session", "run", "t"]),
        compare(&s, &["text", "compare", "a.c", "b.c"])
    );
    assert_eq!(
        compare(&s, &["session", "run", "f", "--json"]),
        compare(&s, &["folder", "compare", "l", "r", "--json"])
    );
    assert_eq!(
        compare(&s, &["session", "run", "f", "--report", "summary"]),
        compare(&s, &["folder", "compare", "l", "r", "--report", "summary"])
    );
    // With rules: the session's rules equal the flags.
    ok(
        &s,
        &[
            "session",
            "edit",
            "t",
            "--set",
            "rules.importance.leading_whitespace=false",
        ],
    );
    ok(
        &s,
        &["session", "edit", "f", "--set", "rules.criteria.size=false"],
    );
    let session = compare(&s, &["session", "run", "t", "--json"]);
    let flags = compare(
        &s,
        &[
            "text",
            "compare",
            "a.c",
            "b.c",
            "--unimportant",
            "leading-ws",
            "--json",
        ],
    );
    assert_eq!(session, flags);
    assert_eq!(
        compare(
            &s,
            &["session", "run", "f", "--json", "--ignore-unimportant"]
        ),
        compare(
            &s,
            &[
                "folder",
                "compare",
                "l",
                "r",
                "--no-size",
                "--json",
                "--ignore-unimportant"
            ]
        )
    );
    // A missing base: the same 107.
    std::fs::remove_dir_all(s.join("r")).unwrap();
    let (a, _, _) = run(&s, &["session", "run", "f"]);
    let (b, _, _) = run(&s, &["folder", "compare", "l", "r"]);
    assert_eq!((a, b), (107, 107));
    // Other types: unsupported.
    ok(
        &s,
        &[
            "session",
            "new",
            "h",
            "--type",
            "hex-compare",
            "--left",
            "a.c",
            "--right",
            "b.c",
        ],
    );
    let (c, out, _) = run(&s, &["session", "run", "h", "--json"]);
    assert_eq!(
        (c, json_text(&out)["error"]["kind"].as_str()),
        (100, Some("unsupported"))
    );
    let (c, _, _) = run(&s, &["session", "run", "nope"]);
    assert_eq!(c, 100);
}

#[test]
fn workspace_goldens() {
    let s = sandbox();
    ok(
        &s,
        &[
            "session",
            "new",
            "t",
            "--type",
            "text-compare",
            "--left",
            "a.c",
            "--right",
            "b.c",
        ],
    );
    s.file(
        "cfg/workspaces/last.toml",
        "schema_version = 1\nshortcut = \"F9\"\n\n[[window]]\nactive_tab = 1\n\n[[window.tab]]\nsession = \"t\"\n\n[[window.tab]]\nsession = \"gone\"\n",
    );
    assert_golden(
        "session/workspace-list.txt",
        &ok(&s, &["workspace", "list"]),
    );
    assert_golden(
        "session/workspace-list.json",
        &ok(&s, &["workspace", "list", "--json"]),
    );
    assert_golden(
        "session/workspace-show.txt",
        &ok(&s, &["workspace", "show", "last"]),
    );
    let shown = ok(&s, &["workspace", "show", "LAST", "--json"]);
    assert_golden("session/workspace-show.json", &shown);
    assert_eq!(json_text(&shown)["warnings"][0]["code"], "session_missing");
    assert_golden(
        "session/workspace-rename.json",
        &ok(&s, &["workspace", "rename", "last", "previous", "--json"]),
    );
    assert_golden(
        "session/workspace-delete.txt",
        &ok(&s, &["workspace", "delete", "previous"]),
    );
    let (c, out, _) = run(&s, &["workspace", "show", "previous", "--json"]);
    assert_eq!(
        (c, json_text(&out)["error"]["kind"].as_str()),
        (100, Some("not_found"))
    );
}

#[test]
fn package_goldens() {
    let s = sandbox();
    ok(
        &s,
        &["settings", "set", "options.appearance.theme_mode", "dark"],
    );
    ok(
        &s,
        &[
            "session",
            "new",
            "work/t",
            "--type",
            "text-compare",
            "--left",
            "a.c",
            "--right",
            "b.c",
        ],
    );
    ok(
        &s,
        &[
            "session",
            "edit",
            "--defaults",
            "folder-compare",
            "--set",
            "rules.handling.flatten=true",
        ],
    );
    let pkg = s.join("all.hikasa-pkg");
    let pkg_arg = pkg.to_str().unwrap();
    assert_golden(
        "session/export.txt",
        &ok(&s, &["settings", "export", "-o", pkg_arg]),
    );
    assert_golden(
        "session/export.json",
        &ok(
            &s,
            &[
                "settings",
                "export",
                "-o",
                pkg_arg,
                "--item",
                "sessions:work/t",
                "--json",
            ],
        ),
    );
    ok(&s, &["settings", "export", "-o", pkg_arg]);
    // Into another configuration.
    let t = Sandbox::new();
    t.file("cfg/sessions/mine.toml", "type = \"text-edit\"\n");
    t.file(
        "cfg/options.toml",
        "schema_version = 1\n[appearance]\ntheme_mode = \"light\"\n",
    );
    let (c, _, err) = run(&t, &["settings", "import", pkg_arg]);
    assert_eq!(c, 100, "one of --dry-run, --yes, --silent: {err}");
    let plan = ok(
        &t,
        &[
            "settings",
            "import",
            pkg_arg,
            "--delete-existing",
            "sessions",
            "--dry-run",
        ],
    );
    assert_golden("session/import-dry-run.txt", &t.normalise(&plan));
    assert!(
        t.join("cfg/sessions/mine.toml").exists(),
        "a dry run writes nothing"
    );
    let done = ok(
        &t,
        &[
            "settings",
            "import",
            pkg_arg,
            "--category",
            "options",
            "--yes",
            "--json",
        ],
    );
    assert_golden("session/import.json", &t.normalise(&done));
    let options = std::fs::read_to_string(t.join("cfg/options.toml")).unwrap();
    assert!(options.contains("dark"), "{options}");
    assert!(!t.join("cfg/sessions/work/t.toml").exists());
    ok(&t, &["settings", "import", pkg_arg, "--silent"]);
    assert!(t.join("cfg/sessions/work/t.toml").exists());
    assert!(
        t.join("cfg/sessions/mine.toml").exists(),
        "--silent deletes nothing"
    );
    let (c, _, _) = run(
        &t,
        &[
            "settings",
            "import",
            pkg_arg,
            "--delete-existing",
            "options",
            "--yes",
        ],
    );
    assert_eq!(c, 100);
    // Restore factory defaults by category.
    assert_golden(
        "session/restore-category.txt",
        &t.normalise(&ok(
            &t,
            &[
                "settings",
                "restore-defaults",
                "--category",
                "options",
                "--category",
                "sessions",
                "--dry-run",
            ],
        )),
    );
    let doc = ok(
        &t,
        &[
            "settings",
            "restore-defaults",
            "--category",
            "sessions",
            "--delete-sessions",
            "--yes",
            "--json",
        ],
    );
    assert_golden("session/restore-category.json", &t.normalise(&doc));
    assert!(!t.join("cfg/sessions/mine.toml").exists());
    assert!(t.join("cfg/options.toml").exists());
}

#[cfg(unix)]
mod gui {
    use std::sync::Arc;

    use common::{CloseMode, FakeGui, serve};
    use hikasa_core::ExitCode;
    use hikasa_session::ipc::Request;

    use super::*;

    #[test]
    fn gui_session_and_workspace() {
        let s = sandbox();
        ok(
            &s,
            &[
                "session",
                "new",
                "t",
                "--type",
                "text-compare",
                "--left",
                "a.c",
                "--right",
                "b.c",
            ],
        );
        ok(
            &s,
            &[
                "session",
                "new",
                "h",
                "--type",
                "hex-compare",
                "--left",
                "a.c",
                "--right",
                "b.c",
            ],
        );
        s.file(
            "cfg/workspaces/w.toml",
            "[[window]]\n[[window.tab]]\nsession = \"t\"\n",
        );
        let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame);
        let server = serve(&s, Arc::clone(&gui));
        let doc = ok(&s, &["gui", "session", "T", "--json"]);
        assert_golden("session/gui-session.json", &doc);
        let doc = ok(&s, &["gui", "workspace", "w", "--json"]);
        assert_golden("session/gui-workspace.json", &doc);
        // --wait: the view's code.
        let (c, _, err) = run(&s, &["gui", "session", "t", "--wait"]);
        assert_eq!(c, i32::from(ExitCode::RulesSame.code()), "{err}");
        // Checked locally: unknown names and types the GUI cannot show never reach it.
        let before = gui.requests().len();
        let (c, out, _) = run(&s, &["gui", "session", "nope", "--json"]);
        assert_eq!(
            (c, json_text(&out)["error"]["kind"].as_str()),
            (100, Some("not_found"))
        );
        let (c, out, _) = run(&s, &["gui", "session", "h", "--json"]);
        assert_eq!(
            (c, json_text(&out)["error"]["kind"].as_str()),
            (100, Some("unsupported"))
        );
        assert_eq!(gui.requests().len(), before);
        let names: Vec<String> = gui
            .requests()
            .into_iter()
            .filter_map(|r| match r {
                Request::LoadSession { name, .. } => Some(name),
                Request::LoadWorkspace { name } => Some(format!("ws:{name}")),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["t", "ws:w", "t"]);
        server.shutdown().unwrap();
    }

    /// CLI-002 / D-88: existing path > session > workspace > error; packages with and without `--silent`.
    #[test]
    fn single_argument_resolution_order() {
        let s = sandbox();
        ok(
            &s,
            &[
                "session",
                "new",
                "t",
                "--type",
                "text-compare",
                "--left",
                "a.c",
                "--right",
                "b.c",
            ],
        );
        ok(
            &s,
            &[
                "session",
                "new",
                "both",
                "--type",
                "text-compare",
                "--left",
                "a.c",
                "--right",
                "b.c",
            ],
        );
        s.file("cfg/workspaces/w.toml", "[[window]]\n");
        s.file("cfg/workspaces/both.toml", "[[window]]\n");
        let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame);
        let server = serve(&s, Arc::clone(&gui));
        ok(&s, &["gui", "open", "t"]);
        ok(&s, &["gui", "open", "w"]);
        ok(&s, &["gui", "open", "both"]);
        // An existing path wins over a name: the M1 error (one spec).
        s.file("t", "a file named like the session\n");
        let (c, _, err) = run(&s, &["gui", "open", "t"]);
        assert_eq!(c, 100, "{err}");
        let (c, _, _) = run(&s, &["gui", "open", "nothing-at-all"]);
        assert_eq!(c, 100);
        let loads: Vec<String> = gui
            .requests()
            .into_iter()
            .filter_map(|r| match r {
                Request::LoadSession { name, .. } => Some(format!("s:{name}")),
                Request::LoadWorkspace { name } => Some(format!("w:{name}")),
                Request::OpenView { .. } => Some("open".into()),
                _ => None,
            })
            .collect();
        assert_eq!(loads, ["s:t", "w:w", "s:both"]);
        // A package: imported headless with --silent; without it the GUI shows the wizard (D-127).
        let pkg = s.join("p.hikasa-pkg");
        ok(
            &s,
            &[
                "settings",
                "export",
                "-o",
                pkg.to_str().unwrap(),
                "--category",
                "sessions",
            ],
        );
        let (c, out, err) = run(&s, &["gui", "open", pkg.to_str().unwrap(), "--json"]);
        assert_eq!(c, 0, "{err}");
        assert_eq!(json_text(&out)["schema"], "hikasa.gui_open.v1");
        assert!(
            gui.requests().iter().any(|r| matches!(
                r,
                Request::ImportPackage { path } if *path == pkg.to_str().unwrap()
            )),
            "{:?}",
            gui.requests()
        );
        let t = Sandbox::new();
        let (c, _, _) = run(&t, &["gui", "open", pkg.to_str().unwrap(), "--no-launch"]);
        assert_eq!(c, 100, "no GUI, --no-launch");
        let (c, out, err) = run(
            &t,
            &["gui", "open", pkg.to_str().unwrap(), "--silent", "--json"],
        );
        assert_eq!(c, 0, "{err}");
        assert_eq!(json_text(&out)["schema"], "hikasa.settings_import.v1");
        assert!(t.join("cfg/sessions/t.toml").exists());
        server.shutdown().unwrap();
    }

    /// D-89: an M1 GUI answers `unsupported_request`; the CLI says to restart it (exit 100).
    #[test]
    fn an_m1_gui_is_reported() {
        let s = sandbox();
        ok(
            &s,
            &[
                "session",
                "new",
                "t",
                "--type",
                "text-compare",
                "--left",
                "a.c",
                "--right",
                "b.c",
            ],
        );
        s.file("cfg/workspaces/w.toml", "[[window]]\n");
        let pkg = s.join("x.hikasa-pkg");
        ok(&s, &["settings", "export", "-o", pkg.to_str().unwrap()]);
        let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesSame).without_sessions();
        let server = serve(&s, Arc::clone(&gui));
        for args in [
            &["gui", "session", "t", "--json"][..],
            &["gui", "workspace", "w", "--json"],
            &["gui", "open", pkg.to_str().unwrap(), "--json"],
        ] {
            let (c, out, err) = run(&s, args);
            assert_eq!(c, 100, "{args:?}");
            let doc = json_text(&out);
            assert_eq!(doc["error"]["kind"], "unsupported");
            assert!(err.contains("restart it"), "{err}");
        }
        server.shutdown().unwrap();
    }
}

#[test]
fn help_topics_resolve() {
    let s = Sandbox::new();
    for (words, title) in [
        (&["help", "session"][..], "hikasa session"),
        (&["help", "workspace", "list"], "hikasa workspace"),
        (&["help", "settings", "export"], "hikasa settings"),
    ] {
        let out = s.cmd().args(words).output().unwrap();
        assert_eq!(code(&out), 0);
        assert!(stdout(&out).starts_with(title), "{}", stdout(&out));
    }
    let _ = json;
}
