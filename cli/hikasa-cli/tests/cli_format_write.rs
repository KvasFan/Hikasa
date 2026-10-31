//! The `format` write commands end to end (M2 W1, contract §6.1 A1.10, A1.11; D-75): `new` (clone and empty),
//! `edit` (file and `--set`), `delete`, `import`, `export`, `enable`/`disable`, `move` (all six forms) and
//! `restore`, in text and JSON. After each, `format list --json` shows the expected registry and only the
//! touched files exist.

mod common;

use common::{Sandbox, assert_golden, code, json, stderr, stdout};
use serde_json::Value;

/// Runs `hikasa <args>`, expecting `expected`; returns stdout.
fn ok(s: &Sandbox, args: &[&str]) -> String {
    let out = s.cmd().args(args).output().unwrap();
    assert_eq!(code(&out), 0, "{args:?}: {}", stderr(&out));
    stdout(&out)
}

/// `hikasa <args>` must exit 100 with error kind `kind`.
fn fails(s: &Sandbox, args: &[&str], kind: &str) {
    let mut args = args.to_vec();
    args.push("--json");
    let out = s.cmd().args(&args).output().unwrap();
    assert_eq!(code(&out), 100, "{args:?}");
    assert_eq!(json(&out)["error"]["kind"], kind, "{args:?}");
}

/// The registry: (id, enabled, origin) in detection order.
fn registry(s: &Sandbox) -> Vec<(String, bool, String)> {
    let out = s.cmd().args(["format", "list", "--json"]).output().unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    json(&out)["result"]["formats"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["id"].as_str().unwrap().to_owned(),
                f["enabled"].as_bool().unwrap(),
                f["origin"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// The ids in detection order.
fn ids(s: &Sandbox) -> Vec<String> {
    registry(s).into_iter().map(|(id, _, _)| id).collect()
}

/// The files of the formats folder.
fn files(s: &Sandbox) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(s.config().join("formats")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn new_edit_delete_import_export() {
    let s = Sandbox::new();
    let builtin = ids(&s);
    // An empty format.
    let text = ok(
        &s,
        &[
            "format", "new", "notes", "--name", "Notes", "--mask", "*.notes",
        ],
    );
    assert_golden("format-write/new.txt", &s.normalise(&text));
    assert_eq!(files(&s), vec!["notes.toml", "order.toml"]);
    let mut expected = builtin.clone();
    expected.push("notes".into());
    assert_eq!(ids(&s), expected);
    assert!(
        registry(&s)
            .iter()
            .all(|(_, _, o)| o == "built_in" || o == "user")
    );
    // A clone, in JSON.
    let text = ok(&s, &["format", "new", "c2", "--from", "c-cpp", "--json"]);
    assert_golden("format-write/new-clone.json", &s.normalise(&text));
    assert_eq!(files(&s), vec!["c2.toml", "notes.toml", "order.toml"]);
    let shown = json(
        &s.cmd()
            .args(["format", "show", "c2", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        shown["result"]["format"]["name"],
        "C, C++, C#, ObjC Source (copy)"
    );
    fails(&s, &["format", "new", "c2"], "already_exists");
    fails(&s, &["format", "new", "python"], "already_exists");
    fails(&s, &["format", "new", "Bad Id"], "invalid_input");
    fails(&s, &["format", "new", "x1", "--from", "nope"], "not_found");
    fails(
        &s,
        &["format", "new", "x1", "--kind", "nope"],
        "invalid_input",
    );
    fails(
        &s,
        &["format", "new", "x1", "--mask", "a/b"],
        "invalid_input",
    );
    assert_eq!(files(&s), vec!["c2.toml", "notes.toml", "order.toml"]);

    // Edit with --set, then from a file; a built-in gets its override file.
    let text = ok(
        &s,
        &[
            "format",
            "edit",
            "notes",
            "--set",
            "mask=*.nt;*.notes",
            "--set",
            "text.tab_size=8",
        ],
    );
    assert_golden("format-write/edit.txt", &s.normalise(&text));
    let shown = json(
        &s.cmd()
            .args(["format", "show", "notes", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(shown["result"]["format"]["mask"], "*.nt;*.notes");
    assert_eq!(shown["result"]["format"]["text"]["tab_size"], 8);
    fails(
        &s,
        &["format", "edit", "notes", "--set", "bogus=1"],
        "invalid_input",
    );
    fails(
        &s,
        &["format", "edit", "notes", "--set", "text.tab_size=99"],
        "invalid_input",
    );
    fails(
        &s,
        &["format", "edit", "notes", "--set", "id=other"],
        "invalid_input",
    );
    fails(
        &s,
        &["format", "edit", "nope", "--set", "mask=*.x"],
        "not_found",
    );
    let other = s.file(
        "other.toml",
        "schema_version = 1\nid = \"notes\"\nname = \"From a file\"\nkind = \"text\"\n",
    );
    let text = ok(
        &s,
        &[
            "format",
            "edit",
            "notes",
            "--from-file",
            other.to_str().unwrap(),
            "--json",
        ],
    );
    assert_golden("format-write/edit-file.json", &s.normalise(&text));
    let wrong = s.file(
        "wrong.toml",
        "schema_version = 1\nid = \"wrong\"\nname = \"W\"\nkind = \"text\"\n",
    );
    fails(
        &s,
        &[
            "format",
            "edit",
            "notes",
            "--from-file",
            wrong.to_str().unwrap(),
        ],
        "invalid_input",
    );
    ok(&s, &["format", "edit", "python", "--set", "name=Snake"]);
    assert_eq!(
        files(&s),
        vec!["c2.toml", "notes.toml", "order.toml", "python.toml"]
    );
    assert!(registry(&s).contains(&("python".into(), true, "user".into())));

    // Export to stdout and to a file.
    let text = ok(&s, &["format", "export", "notes"]);
    assert!(
        text.starts_with("schema_version = 1\nid = \"notes\"\nname = \"From a file\"\n"),
        "{text}"
    );
    let out = s.join("exported.toml");
    let text = ok(
        &s,
        &[
            "format",
            "export",
            "notes",
            "-o",
            out.to_str().unwrap(),
            "--json",
        ],
    );
    assert_golden("format-write/export.json", &s.normalise(&text));
    let text = ok(
        &s,
        &["format", "export", "notes", "-o", out.to_str().unwrap()],
    );
    assert_golden("format-write/export.txt", &s.normalise(&text));
    assert!(
        std::fs::read_to_string(&out)
            .unwrap()
            .contains("From a file")
    );

    // Delete a user format; a built-in is refused.
    let text = ok(&s, &["format", "delete", "notes"]);
    assert_golden("format-write/delete.txt", &s.normalise(&text));
    assert_eq!(files(&s), vec!["c2.toml", "order.toml", "python.toml"]);
    assert!(!ids(&s).contains(&"notes".to_owned()));
    let text = ok(&s, &["format", "delete", "c2", "--json"]);
    assert_golden("format-write/delete.json", &s.normalise(&text));
    assert_eq!(files(&s), vec!["order.toml", "python.toml"]);
    fails(&s, &["format", "delete", "python"], "invalid_input");
    fails(&s, &["format", "delete", "c-cpp"], "invalid_input");
    fails(&s, &["format", "delete", "notes"], "not_found");

    // Import, then import again only with --replace.
    let text = ok(&s, &["format", "import", out.to_str().unwrap(), "--json"]);
    assert_golden("format-write/import.json", &s.normalise(&text));
    ok(&s, &["format", "delete", "notes"]);
    let text = ok(&s, &["format", "import", out.to_str().unwrap()]);
    assert_golden("format-write/import.txt", &s.normalise(&text));
    assert!(ids(&s).contains(&"notes".to_owned()));
    fails(
        &s,
        &["format", "import", out.to_str().unwrap()],
        "already_exists",
    );
    ok(
        &s,
        &["format", "import", out.to_str().unwrap(), "--replace"],
    );
    assert_eq!(
        std::fs::read(&out).unwrap(),
        std::fs::read(s.config().join("formats/notes.toml")).unwrap(),
        "an import copies the file"
    );
    fails(
        &s,
        &[
            "format",
            "import",
            wrong.with_extension("missing").to_str().unwrap(),
        ],
        "not_found",
    );
}

#[test]
fn enable_disable_move_restore() {
    let s = Sandbox::new();
    let builtin = ids(&s);
    let text = ok(&s, &["format", "disable", "python"]);
    assert_golden("format-write/disable.txt", &s.normalise(&text));
    assert_eq!(files(&s), vec!["order.toml"], "only order.toml is written");
    assert!(registry(&s).contains(&("python".into(), false, "built_in".into())));
    assert_eq!(ids(&s), builtin, "the order is unchanged");
    let text = ok(&s, &["format", "enable", "python", "--json"]);
    assert_golden("format-write/enable.json", &s.normalise(&text));
    assert!(registry(&s).contains(&("python".into(), true, "built_in".into())));
    fails(&s, &["format", "enable", "nope"], "not_found");

    let position = |s: &Sandbox, id: &str| ids(s).iter().position(|i| i == id).unwrap();
    let last = builtin.len() - 1;
    ok(&s, &["format", "move", "python", "--top"]);
    assert_eq!(position(&s, "python"), 0);
    ok(&s, &["format", "move", "python", "--down"]);
    assert_eq!(position(&s, "python"), 1);
    ok(&s, &["format", "move", "python", "--up"]);
    assert_eq!(position(&s, "python"), 0);
    ok(&s, &["format", "move", "python", "--up"]);
    assert_eq!(position(&s, "python"), 0, "the top stays the top");
    ok(&s, &["format", "move", "python", "--bottom"]);
    assert_eq!(position(&s, "python"), last);
    let text = ok(
        &s,
        &["format", "move", "python", "--before", "c-cpp", "--json"],
    );
    assert_golden("format-write/move.json", &s.normalise(&text));
    let text = ok(&s, &["format", "move", "python", "--before", "c-cpp"]);
    assert_golden("format-write/move.txt", &s.normalise(&text));
    assert_eq!(position(&s, "python") + 1, position(&s, "c-cpp"));
    ok(&s, &["format", "move", "python", "--after", "sql"]);
    assert_eq!(position(&s, "python"), position(&s, "sql") + 1);
    fails(
        &s,
        &["format", "move", "python", "--after", "python"],
        "invalid_input",
    );
    fails(
        &s,
        &["format", "move", "python", "--before", "nope"],
        "not_found",
    );
    let usage = s.cmd().args(["format", "move", "python"]).output().unwrap();
    assert_eq!(code(&usage), 100, "one of the six forms is required");
    assert_eq!(files(&s), vec!["order.toml"]);

    // Restore: one id (plan, then for real), and --all.
    ok(&s, &["format", "edit", "python", "--set", "name=Snake"]);
    ok(&s, &["format", "new", "notes"]);
    fails(&s, &["format", "restore", "python"], "invalid_input");
    let text = ok(&s, &["format", "restore", "python", "c-cpp", "--dry-run"]);
    assert_golden("format-write/restore-dry-run.txt", &s.normalise(&text));
    assert_eq!(files(&s), vec!["notes.toml", "order.toml", "python.toml"]);
    let text = ok(&s, &["format", "restore", "python", "--yes", "--json"]);
    assert_golden("format-write/restore.json", &s.normalise(&text));
    assert_eq!(files(&s), vec!["notes.toml", "order.toml"]);
    assert!(registry(&s).contains(&("python".into(), true, "built_in".into())));
    fails(
        &s,
        &["format", "restore", "notes", "--yes"],
        "invalid_input",
    );
    let doc: Value = json(
        &s.cmd()
            .args(["format", "restore", "--all", "--yes", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(doc["result"]["ids"], serde_json::json!(["notes"]));
    assert_eq!(files(&s), Vec::<String>::new());
    assert_eq!(ids(&s), builtin);
}

#[test]
fn the_order_id_is_reserved_and_skipped_files_are_not_replaced() {
    let s = Sandbox::new();
    ok(&s, &["format", "disable", "python"]);
    let order = std::fs::read(s.config().join("formats/order.toml")).unwrap();
    for id in ["order", "Order", "ORDER"] {
        fails(&s, &["format", "new", id], "invalid_input");
        fails(
            &s,
            &["format", "edit", id, "--set", "name=X"],
            "invalid_input",
        );
    }
    let named_order = s.file(
        "named-order.toml",
        "schema_version = 1\nid = \"order\"\nname = \"O\"\nkind = \"text\"\n",
    );
    for replace in [false, true] {
        let mut args = vec!["format", "import", named_order.to_str().unwrap()];
        if replace {
            args.push("--replace");
        }
        fails(&s, &args, "invalid_input");
    }
    assert_eq!(
        std::fs::read(s.config().join("formats/order.toml")).unwrap(),
        order,
        "order.toml is untouched"
    );
    assert_eq!(files(&s), vec!["order.toml"]);

    // A user file the load skipped (it does not parse) is not overwritten by `new` or `import`.
    let broken = s.file("cfg/formats/mine.toml", "this is not toml [");
    let before = std::fs::read(&broken).unwrap();
    fails(&s, &["format", "new", "mine"], "already_exists");
    let mine = s.file(
        "mine.toml",
        "schema_version = 1\nid = \"mine\"\nname = \"Mine\"\nkind = \"text\"\n",
    );
    fails(
        &s,
        &["format", "import", mine.to_str().unwrap()],
        "already_exists",
    );
    assert_eq!(std::fs::read(&broken).unwrap(), before);
    assert_eq!(
        std::fs::read(s.config().join("formats/order.toml")).unwrap(),
        order
    );
    ok(
        &s,
        &["format", "import", mine.to_str().unwrap(), "--replace"],
    );
    assert!(ids(&s).contains(&"mine".to_owned()));
}
