//! `hikasa format list|show|detect|tokens` end to end (contract §6.3 `cli_format.rs`, §4.3.4; D-15, D-21).
//!
//! The configuration folder of every case holds a user format `sql.toml` that replaces the built-in SQL format
//! (same id, other name and masks, §2.12.4).

mod common;

use common::{Sandbox, assert_golden, code, json, stderr, stdout};

/// The user format that overrides the built-in `sql`.
const USER_SQL: &str = r#"schema_version = 1
id = "sql"
name = "My SQL"
kind = "text"
mask = "*.sql;*.mysql"

[[grammar]]
element = "Comment"
kind = "delimited"
from = "--"
stop_at_eol = true

[[grammar]]
element = "String"
kind = "delimited"
from = "'"
to = "'"
stop_at_eol = true
"#;

/// A sandbox whose configuration holds the user format.
fn sandbox() -> Sandbox {
    let s = Sandbox::new();
    s.file("cfg/formats/sql.toml", USER_SQL);
    s
}

#[test]
fn list_goldens() {
    let s = sandbox();
    let out = s.cmd().args(["format", "list"]).output().unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("\tsql\tvisible\tMy SQL\t*.sql;*.mysql\n"),
        "{text}"
    );
    assert_golden("format/list.txt", &s.normalise(&text));
    let doc = s.cmd().args(["format", "list", "--json"]).output().unwrap();
    let parsed = json(&doc);
    let sql = parsed["result"]["formats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "sql")
        .unwrap()
        .clone();
    assert_eq!(sql["origin"], "user", "{sql}");
    assert_golden("format/list.json", &s.normalise(&stdout(&doc)));
    // Read-only commands never write the built-ins to the configuration folder (D-21).
    let entries: Vec<_> = std::fs::read_dir(s.join("cfg/formats")).unwrap().collect();
    assert_eq!(entries.len(), 1);
}

#[test]
fn show_goldens() {
    let s = sandbox();
    let out = s.cmd().args(["format", "show", "sql"]).output().unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_golden("format/show-sql.toml", &s.normalise(&stdout(&out)));
    let by_name = s.cmd().args(["format", "show", "my sql"]).output().unwrap();
    assert_eq!(stdout(&by_name), stdout(&out), "by case-insensitive name");
    let doc = s
        .cmd()
        .args(["format", "show", "sql", "--json"])
        .output()
        .unwrap();
    assert_eq!(json(&doc)["result"]["origin"], "user");
    assert_golden("format/show-sql.json", &s.normalise(&stdout(&doc)));
    let builtin = s.cmd().args(["format", "show", "python"]).output().unwrap();
    assert_eq!(code(&builtin), 0);
    assert_golden("format/show-python.toml", &s.normalise(&stdout(&builtin)));
    let unknown = s
        .cmd()
        .args(["format", "show", "cobol-85", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&unknown), 100);
    assert_eq!(json(&unknown)["schema"], "hikasa.format_show.v1");
}

#[test]
fn detect_goldens() {
    let s = sandbox();
    let out = s
        .cmd()
        .args(["format", "detect", "q.mysql"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(stdout(&out), "sql\tMy SQL\n", "the user format's mask");
    let plain = s
        .cmd()
        .args(["format", "detect", "notes"])
        .output()
        .unwrap();
    assert_eq!(stdout(&plain), "-\tEverything Else\n");
    let doc = s
        .cmd()
        .args(["format", "detect", "q.mysql", "--json"])
        .output()
        .unwrap();
    assert_golden("format/detect.json", &s.normalise(&stdout(&doc)));
    // `-` uses the --stdin-name hint and never reads standard input.
    let stdin = s
        .cmd()
        .args(["format", "detect", "-", "--stdin-name", "a.py", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&stdin), 0, "{}", stderr(&stdin));
    let doc = json(&stdin);
    assert_eq!(doc["result"]["format"]["id"], "python", "{doc}");
    assert_eq!(doc["result"]["file"], "-");
}

#[test]
fn tokens_goldens() {
    let s = sandbox();
    s.file("q.sql", "SELECT 'a' -- one\nFROM t;\n-- two\nSELECT 2;\n");
    let out = s
        .cmd()
        .args(["format", "tokens", "q.sql"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.starts_with("L1:"), "{text}");
    assert_golden("format/tokens.txt", &s.normalise(&text));
    let lines = s
        .cmd()
        .args(["format", "tokens", "q.sql", "--lines", "3-4", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&lines), 0);
    let doc = json(&lines);
    let numbers: Vec<u64> = doc["result"]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["line"].as_u64().unwrap())
        .collect();
    assert_eq!(numbers, [3, 4]);
    assert_golden("format/tokens-3-4.json", &s.normalise(&stdout(&lines)));
    let forced = s
        .cmd()
        .args([
            "format", "tokens", "q.sql", "--format", "c-cpp", "--lines", "1",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&forced), 0);
    assert_ne!(stdout(&forced), text.lines().next().unwrap_or_default());
    // Standard input with a name hint.
    let stdin = s
        .cmd()
        .args(["format", "tokens", "-", "--stdin-name", "x.py"])
        .write_stdin("# hi\nx = 'y'\n")
        .output()
        .unwrap();
    assert_eq!(code(&stdin), 0, "{}", stderr(&stdin));
    assert!(
        stdout(&stdin).contains("\tComment\t# hi"),
        "{}",
        stdout(&stdin)
    );
    for range in ["0-1", "5-", "9-2", "x"] {
        let bad = s
            .cmd()
            .args(["format", "tokens", "q.sql", "--lines", range])
            .output()
            .unwrap();
        assert_eq!(code(&bad), 100, "{range}");
    }
    let missing = s
        .cmd()
        .args(["format", "tokens", "nope.sql"])
        .output()
        .unwrap();
    assert_eq!(code(&missing), 100);
}

#[test]
fn a_broken_user_format_is_a_startup_warning() {
    let s = sandbox();
    s.file("cfg/formats/broken.toml", "this is [not toml");
    let out = s.cmd().args(["format", "list"]).output().unwrap();
    assert_eq!(code(&out), 0);
    assert!(stderr(&out).contains("broken.toml"), "{}", stderr(&out));
    let quiet = s
        .cmd()
        .args(["format", "list", "--quiet"])
        .output()
        .unwrap();
    assert!(quiet.stderr.is_empty(), "--quiet: no warnings on stderr");
    assert_eq!(quiet.stdout, out.stdout, "--quiet never changes stdout");
    let doc = s.cmd().args(["format", "list", "--json"]).output().unwrap();
    let warnings = json(&doc)["warnings"].clone();
    assert_eq!(warnings[0]["code"], "user_format_skipped", "{warnings}");
}
