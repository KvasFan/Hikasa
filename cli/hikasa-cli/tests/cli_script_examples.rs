//! A8.11: the four examples of docs/12 §3.2, each file as docs/12 writes it (`tests/fixtures/script/example<n>.txt`) except
//! its absolute paths: the test writes a generated copy where `/var/log`, `/srv/www`, `/data/project` and `/backups` are
//! folders of the sandbox. `script check` of all four, verbatim, exits 0.

mod common;

use std::path::Path;

use common::{Sandbox, code, json, stderr};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/script")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

/// The tree below `root`: relative path and bytes (`None` for a folder), sorted.
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

/// The sandbox path of `rel` as a script writes it (forward slashes are fine on every platform).
fn at(s: &Sandbox, rel: &str) -> String {
    s.join(rel).display().to_string()
}

#[test]
fn a8_11_every_example_checks_verbatim() {
    let s = Sandbox::new();
    for n in 1..=4 {
        let name = format!("example{n}.txt");
        s.file(&name, fixture(&name));
        let out = s.cmd().args(["script", "check", &name]).output().unwrap();
        assert_eq!(code(&out), 0, "{name}: {}", stderr(&out));
    }
}

fn laptop() -> Sandbox {
    let s = Sandbox::new();
    s.file_at("l/a.txt", "new\n", 2_000_000);
    s.file_at("r/a.txt", "old, longer\n", 1_000_000);
    s.file_at("l/n/b.txt", "b\n", 1_000_000);
    s.file_at("r/keep.txt", "keep\n", 1_000_000);
    s.settle("l");
    s.settle("r");
    let mut new = s.cmd();
    new.args([
        "session",
        "new",
        "Sync Laptop",
        "--type",
        "folder-sync",
        "--left",
    ]);
    new.arg(s.join("l")).arg("--right").arg(s.join("r"));
    assert_eq!(code(&new.output().unwrap()), 0);
    s.file("ex1.txt", fixture("example1.txt"));
    s
}

/// (1) A folder-sync session: with `--confirm yes-to-all`, with a scripted terminal answering, and silent.
#[test]
fn a8_11_example_1_syncs_the_session_as_folder_sync_does() {
    // What `folder sync --session "Sync Laptop" --update-right --yes` leaves.
    let direct = laptop();
    let out = direct
        .cmd()
        .args([
            "folder",
            "sync",
            "--session",
            "Sync Laptop",
            "--update-right",
            "--yes",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let expected = tree(&direct.join("r"));
    let s = laptop();
    let out = s
        .cmd()
        .args(["script", "run", "ex1.txt", "--confirm", "yes-to-all"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(tree(&s.join("r")), expected);
    // A person at a terminal (debug builds read the scripted answers) who answers `a` (yes to all).
    let s = laptop();
    let out = s
        .cmd()
        .env("HIKASA_TEST_ANSWERS", "a;a;a;a")
        .args(["script", "run", "ex1.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(tree(&s.join("r")), expected);
    // Silent: the question declines the sync, nothing written.
    let s = laptop();
    let before = tree(&s.join("r"));
    let out = s
        .cmd()
        .args(["script", "run", "ex1.txt", "--silent"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    assert!(
        stderr(&out).contains("error L2 Attempted to open a dialog when running silently: "),
        "{}",
        stderr(&out)
    );
    assert_eq!(tree(&s.join("r")), before);
}

/// (2) A remote right side: 107 at line 4, the log holds lines 1–4, the sync is not run, the left side untouched.
#[test]
fn a8_11_example_2_ends_with_107_at_its_remote_load() {
    let s = Sandbox::new();
    s.file_at("srv/www/index.html", "<p>\n", 1_000_000);
    s.dir("var/log");
    let text = fixture("example2.txt")
        .replace("/var/log", &at(&s, "var/log"))
        .replace("\"/srv/www\"", &format!("\"{}\"", at(&s, "srv/www")));
    s.file("ex2.txt", &text);
    let before = tree(&s.join("srv"));
    let out = s
        .cmd()
        .args(["script", "run", "ex2.txt", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 107, "{}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["result"]["stopped_at"]["line"], 4);
    assert_eq!(doc["result"]["stopped_at"]["reason"], "load_failed");
    assert_eq!(doc["result"]["commands"][4]["status"], "not_run");
    assert!(
        stderr(&out).contains(
            "error L4 sftp://deploy@host/var/www: remote folders are not available in this version"
        ),
        "{}",
        stderr(&out)
    );
    let log = std::fs::read_to_string(s.join("var/log/hikasa-sync.txt")).unwrap();
    let kinds: Vec<&str> = log
        .lines()
        .map(|l| l.split(' ').nth(3).unwrap_or(""))
        .collect();
    assert!(
        kinds.contains(&"L1") && kinds.contains(&"L4") && !kinds.contains(&"L5"),
        "{log}"
    );
    assert!(
        log.contains(" error L4 sftp://deploy@host/var/www: remote folders"),
        "{log}"
    );
    assert_eq!(tree(&s.join("srv")), before);
}

/// (3) A text report from arguments: byte-identical to `report text` with the same words (`SOURCE_DATE_EPOCH`).
#[test]
fn a8_11_example_3_writes_the_report_report_text_writes() {
    let s = Sandbox::new();
    s.file("l.c", "int a;\nint b;\n  int c;\n");
    s.file("r.c", "int a;\nint B;\nint c;\n");
    s.file("ex3.txt", fixture("example3.txt"));
    let out = s
        .cmd()
        .env("SOURCE_DATE_EPOCH", "1792584000")
        .args(["script", "run", "ex3.txt", "l.c", "r.c", "out.html"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let out = s
        .cmd()
        .env("SOURCE_DATE_EPOCH", "1792584000")
        .args([
            "report",
            "text",
            "l.c",
            "r.c",
            "--layout",
            "side-by-side",
            "--options",
            "ignore-unimportant,display-context",
            "--output-options",
            "html-color",
            "--output-to",
            "direct.html",
        ])
        .output()
        .unwrap();
    // `report text` exits with the comparison's verdict (13: differences); the script's code is never a verdict.
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    let script = std::fs::read(s.join("out.html")).unwrap();
    assert!(!script.is_empty());
    assert_eq!(script, std::fs::read(s.join("direct.html")).unwrap());
}

/// (4) Verbatim: 107 at line 3 for the snapshot base, nothing written. A variant whose right side is a folder copy: the
/// zip holds exactly the left-only and differing files, and line 7 is refused (exit 100).
#[test]
fn a8_11_example_4_and_its_folder_variant() {
    let s = Sandbox::new();
    s.file_at("data/project/same.txt", "same\n", 1_000_000);
    s.file_at("data/project/diff.txt", "left version\n", 1_000_000);
    s.file_at("data/project/sub/new.txt", "new\n", 1_000_000);
    s.file_at("backups/project/same.txt", "same\n", 1_000_000);
    s.file_at("backups/project/diff.txt", "right version\n", 1_000_000);
    s.file_at("backups/project/gone.txt", "gone\n", 1_000_000);
    let text = fixture("example4.txt")
        .replace(
            "\"/data/project\"",
            &format!("\"{}\"", at(&s, "data/project")),
        )
        .replace("\"/backups/", &format!("\"{}/", at(&s, "backups")));
    s.file("ex4.txt", &text);
    let before = tree(&s.join("backups"));
    let out = s.cmd().args(["script", "run", "ex4.txt"]).output().unwrap();
    assert_eq!(code(&out), 107, "{}", stderr(&out));
    assert!(
        stderr(&out).contains("project.hksnap: snapshots are not available in this version"),
        "{}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("error L3 "), "{}", stderr(&out));
    assert_eq!(tree(&s.join("backups")), before);
    // The variant: the right side is a folder copy of the project.
    let variant = text.replace(
        &format!("\"{}/project.hksnap\"\nexpand", at(&s, "backups")),
        &format!("\"{}/project\"\nexpand", at(&s, "backups")),
    );
    assert!(variant.contains("/project\"\nexpand"), "{variant}");
    s.file("ex4b.txt", &variant);
    let day = |_: ()| {
        jiff::Zoned::now()
            .with_time_zone(jiff::tz::TimeZone::UTC)
            .strftime("%Y-%m-%d")
            .to_string()
    };
    let before_day = day(());
    let out = s
        .cmd()
        .args(["script", "run", "ex4b.txt"])
        .output()
        .unwrap();
    let after_day = day(());
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert!(
        stderr(&out).contains("error L7 snapshot is not available in this version"),
        "{}",
        stderr(&out)
    );
    let zip = [before_day, after_day]
        .into_iter()
        .map(|d| s.join(&format!("backups/changes-{d}.zip")))
        .find(|p| p.exists())
        .expect("the zip of today");
    // The zip against a folder of exactly the left-only and differing files, under their relative paths.
    s.file_at("expected/diff.txt", "left version\n", 1_000_000);
    s.file_at("expected/sub/new.txt", "new\n", 1_000_000);
    let out = s
        .cmd()
        .args(["folder", "compare", "--criteria", "size", "--json"])
        .arg(&zip)
        .arg(s.join("expected"))
        .output()
        .unwrap();
    let doc = json(&out);
    assert_eq!(doc["result"]["verdict"], "no_differences", "{doc}");
}
