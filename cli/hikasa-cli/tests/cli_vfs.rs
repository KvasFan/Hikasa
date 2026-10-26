//! `hikasa vfs ls|stat|cat` end to end (contract §6.3 `cli_vfs.rs`, §4.3.4; D-32).

mod common;

use common::{Sandbox, assert_golden, code, json, stderr, stdout};

/// `d/` with a text file, a binary file, a name holding a tab and (on Unix) a symbolic link to the text file.
fn sandbox() -> Sandbox {
    let s = Sandbox::new();
    s.file("d/a.txt", "alpha\n");
    s.file("d/b.bin", b"\x00\x01\x02\xff");
    s.file("d/tab\there.txt", "t\n");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("a.txt", s.join("d/link")).unwrap();
        let time = filetime::FileTime::from_unix_time(common::MTIME, 0);
        filetime::set_symlink_file_times(s.join("d/link"), time, time).unwrap();
    }
    s.settle("d");
    s
}

#[cfg(unix)]
#[test]
fn ls_goldens() {
    let s = sandbox();
    let out = s.cmd().args(["vfs", "ls", "d"]).output().unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("tab\\there.txt"), "names are escaped: {text}");
    assert!(text.contains(" link -> a.txt\n"), "{text}");
    assert_golden("vfs/ls.txt", &s.normalise(&text));
    let doc = s.cmd().args(["vfs", "ls", "d", "--json"]).output().unwrap();
    assert_eq!(json(&doc)["schema"], "hikasa.vfs_list.v1");
    assert_golden("vfs/ls.json", &s.normalise(&stdout(&doc)));
    let followed = s
        .cmd()
        .args(["vfs", "ls", "d", "--follow-symlinks", "--json"])
        .output()
        .unwrap();
    let doc = json(&followed);
    let link = doc["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "link")
        .unwrap()
        .clone();
    assert_eq!(link["followed"], true, "{link}");
    assert_eq!(link["kind"], "file");
}

#[cfg(unix)]
#[test]
fn stat_goldens() {
    let s = sandbox();
    let out = s.cmd().args(["vfs", "stat", "d/a.txt"]).output().unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_golden("vfs/stat.txt", &s.normalise(&stdout(&out)));
    let doc = s
        .cmd()
        .args(["vfs", "stat", "d/a.txt", "--json"])
        .output()
        .unwrap();
    assert_golden("vfs/stat.json", &s.normalise(&stdout(&doc)));
    let link = s.cmd().args(["vfs", "stat", "d/link"]).output().unwrap();
    assert!(stdout(&link).starts_with('l'), "{}", stdout(&link));
    let target = s
        .cmd()
        .args(["vfs", "stat", "d/link", "--follow-symlinks"])
        .output()
        .unwrap();
    assert!(stdout(&target).starts_with('-'), "{}", stdout(&target));
}

#[test]
fn cat_and_cat_json() {
    let s = sandbox();
    let out = s.cmd().args(["vfs", "cat", "d/b.bin"]).output().unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(out.stdout, b"\x00\x01\x02\xff", "raw bytes");
    let doc = s
        .cmd()
        .args(["vfs", "cat", "d/b.bin", "--json"])
        .output()
        .unwrap();
    let parsed = json(&doc);
    assert_eq!(parsed["result"]["data_base64"], "AAEC/w==", "D-32");
    assert_eq!(parsed["result"]["size"], 4);
    assert_golden("vfs/cat.json", &s.normalise(&stdout(&doc)));
    let text = s.cmd().args(["vfs", "cat", "d/a.txt"]).output().unwrap();
    assert_eq!(stdout(&text), "alpha\n");
    // Standard input is a spec too.
    let stdin = s
        .cmd()
        .args(["vfs", "cat", "-"])
        .write_stdin("from stdin")
        .output()
        .unwrap();
    assert_eq!(stdout(&stdin), "from stdin");
}

#[test]
fn errors_are_100() {
    let s = sandbox();
    for args in [
        &["vfs", "ls", "missing"][..],
        &["vfs", "stat", "missing"],
        &["vfs", "cat", "missing"],
        &["vfs", "cat", "d"],
        &["vfs", "ls", "d/a.txt"],
        &["vfs", "ls", "ftp://example.org/x"],
    ] {
        let out = s.cmd().args(args).arg("--json").output().unwrap();
        assert_eq!(code(&out), 100, "{args:?}");
        let doc = json(&out);
        assert_eq!(doc["ok"], false);
        assert!(doc["schema"].as_str().unwrap().starts_with("hikasa.vfs_"));
    }
}
