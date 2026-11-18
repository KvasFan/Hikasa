//! `hikasa text compare --align/--isolate` end to end (contract §6.10 A10.7, §1.10.8, §1.10.24; D-266): the anchors
//! change the rows, `hikasa.text_compare.v1` lists them (1-based), a line beyond a document is exit 100, dropped
//! anchors are warnings, and `report text` and `session run` take the same flags.

mod common;

use common::{Sandbox, assert_golden, code, json, stderr, stdout};

/// 16 numbered words.
const WORDS: [&str; 16] = [
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
    "lambda", "mu", "nu", "xi", "omicron", "pi",
];

/// `a.txt`: the 16 words. `b.txt` (17 lines): two header lines, then the words with `kappa` (right line 12) in
/// capitals and `xi` gone.
fn pair() -> Sandbox {
    let s = Sandbox::new();
    let left: String = WORDS.iter().map(|w| format!("{w}\n")).collect();
    let right: String = ["header one", "header two"]
        .iter()
        .map(|h| format!("{h}\n"))
        .chain(WORDS.iter().filter(|w| **w != "xi").map(|w| {
            if *w == "kappa" {
                "KAPPA\n".to_owned()
            } else {
                format!("{w}\n")
            }
        }))
        .collect();
    s.file("a.txt", left);
    s.file("b.txt", right);
    s
}

/// Runs `text compare a.txt b.txt <extra>`.
fn run(s: &Sandbox, extra: &[&str]) -> std::process::Output {
    s.cmd()
        .args(["text", "compare", "a.txt", "b.txt", "--display", "all"])
        .args(extra)
        .output()
        .unwrap()
}

/// Runs a comparison that must find differences (exit 13).
fn differ(s: &Sandbox, extra: &[&str]) -> std::process::Output {
    let out = run(s, extra);
    assert_eq!(code(&out), 13, "{extra:?}: {}", stderr(&out));
    out
}

/// The `(left, right)` 1-based lines of every row of a JSON document (`null` as 0).
fn rows(doc: &serde_json::Value) -> Vec<(u64, u64)> {
    let line = |v: &serde_json::Value| v["line"].as_u64().unwrap_or(0);
    doc["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "row")
        .map(|item| (line(&item["left"]), line(&item["right"])))
        .collect()
}

#[test]
fn align_one_line_with_another() {
    let s = pair();
    let out = differ(
        &s,
        &[
            "--align",
            "10:12",
            "--report",
            "side-by-side",
            "--width",
            "60",
        ],
    );
    assert_golden(
        "text_align/align-10-12.txt",
        &s.normalise_with(&stdout(&out), true),
    );
    let doc = json(&differ(&s, &["--align", "10:12", "--json"]));
    assert_eq!(
        doc["result"]["anchors"],
        serde_json::json!([{"kind": "pair", "left": [10, 11], "right": [12, 13], "side": null, "lines": null}])
    );
    assert!(rows(&doc).contains(&(10, 12)), "{:?}", rows(&doc));
    // Without the anchor `kappa` and `KAPPA` are a deletion and an insertion.
    let plain = json(&differ(&s, &["--json"]));
    assert_eq!(plain["result"]["anchors"], serde_json::json!([]));
    let plain_rows = rows(&plain);
    assert!(
        plain_rows.contains(&(10, 0)) && plain_rows.contains(&(0, 12)),
        "{plain_rows:?}"
    );
}

#[test]
fn align_ranges() {
    let s = pair();
    let out = differ(
        &s,
        &[
            "--align",
            "10-14:12-13",
            "--report",
            "side-by-side",
            "--width",
            "60",
        ],
    );
    assert_golden(
        "text_align/align-10-14-12-13.txt",
        &s.normalise_with(&stdout(&out), true),
    );
    let out = differ(&s, &["--align", "10-14:12-13", "--json"]);
    assert_golden(
        "text_align/align-10-14-12-13.json",
        &s.normalise(&stdout(&out)),
    );
    let doc = json(&out);
    assert_eq!(
        doc["result"]["anchors"],
        serde_json::json!([{"kind": "pair", "left": [10, 15], "right": [12, 14], "side": null, "lines": null}])
    );
    // No row crosses the pair's boundaries: a row with a left line in 10..=14 has a right line in 12..=13 or
    // none, and the other way round.
    for (l, r) in rows(&doc) {
        if (10..=14).contains(&l) && r != 0 {
            assert!((12..=13).contains(&r), "row ({l}, {r}) crosses the pair");
        }
        if (12..=13).contains(&r) && l != 0 {
            assert!((10..=14).contains(&l), "row ({l}, {r}) crosses the pair");
        }
    }
}

#[test]
fn isolate_lines() {
    let s = pair();
    let out = differ(
        &s,
        &[
            "--isolate",
            "left:5-6",
            "--report",
            "side-by-side",
            "--width",
            "60",
        ],
    );
    assert_golden(
        "text_align/isolate-left-5-6.txt",
        &s.normalise_with(&stdout(&out), true),
    );
    let doc = json(&differ(&s, &["--isolate", "left:5-6", "--json"]));
    assert_eq!(
        doc["result"]["anchors"],
        serde_json::json!([{"kind": "isolate", "left": null, "right": null, "side": "left", "lines": [5, 7]}])
    );
    let rows = rows(&doc);
    // Left lines 5 and 6 align with nothing; the right's copies of them are inserted.
    assert!(rows.contains(&(5, 0)) && rows.contains(&(6, 0)), "{rows:?}");
    assert!(rows.contains(&(0, 7)) && rows.contains(&(0, 8)), "{rows:?}");
    // Both flags at once, each repeatable.
    let both = json(&differ(
        &s,
        &[
            "--align",
            "10:12",
            "--isolate",
            "left:5-6",
            "--isolate",
            "right:1",
            "--json",
        ],
    ));
    let kinds: Vec<&str> = both["result"]["anchors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["pair", "isolate", "isolate"]);
}

#[test]
fn a_line_beyond_the_file_is_exit_100() {
    let s = pair();
    for (flag, word) in [
        ("--align", "17:1"),
        ("--align", "1:18"),
        ("--align", "16-17:1"),
        ("--isolate", "left:17"),
        ("--isolate", "right:3-18"),
    ] {
        let out = run(&s, &[flag, word, "--json"]);
        assert_eq!(code(&out), 100, "{flag} {word}");
        let doc = json(&out);
        assert_eq!(doc["schema"], "hikasa.text_compare.v1");
        assert_eq!(doc["ok"], false);
        assert_eq!(doc["error"]["kind"], "invalid_input", "{flag} {word}");
        let message = doc["error"]["message"].as_str().unwrap();
        assert!(message.contains("beyond the"), "{message}");
        let human = run(&s, &[flag, word]);
        assert_eq!(code(&human), 100);
        assert!(stdout(&human).is_empty(), "nothing is compared");
    }
    let out = run(&s, &["--align", "17:1"]);
    assert_golden("text_align/beyond.txt", &s.normalise(&stderr(&out)));
}

#[test]
fn malformed_words_are_exit_100() {
    let s = pair();
    for (flag, word) in [
        ("--align", "10"),
        ("--align", "0:1"),
        ("--align", "left:3"),
        ("--isolate", "3:4"),
        ("--isolate", "up:3"),
        ("--isolate", "left:4-2"),
    ] {
        let out = run(&s, &[flag, word, "--json"]);
        assert_eq!(code(&out), 100, "{flag} {word}");
        assert_eq!(
            json(&out)["error"]["kind"],
            "invalid_input",
            "{flag} {word}"
        );
    }
}

#[test]
fn crossing_pairs_are_dropped_with_a_warning() {
    let s = pair();
    let doc = json(&differ(
        &s,
        &["--align", "10:12", "--align", "11:3", "--json"],
    ));
    assert_eq!(doc["result"]["anchors"].as_array().unwrap().len(), 1);
    let warnings: Vec<&str> = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].as_str().unwrap())
        .collect();
    assert_eq!(warnings, ["anchor_dropped"]);
    let human = differ(&s, &["--align", "10:12", "--align", "11:3"]);
    assert!(stderr(&human).contains("anchor"), "{}", stderr(&human));
}

#[test]
fn report_text_and_session_run_take_the_flags() {
    let s = pair();
    let report = s
        .cmd()
        .args(["report", "text", "a.txt", "b.txt", "--align", "10-14:12-13"])
        .output()
        .unwrap();
    assert_eq!(code(&report), 13, "{}", stderr(&report));
    let compare_default = s
        .cmd()
        .args([
            "text",
            "compare",
            "a.txt",
            "b.txt",
            "--report",
            "side-by-side",
            "--align",
            "10-14:12-13",
        ])
        .output()
        .unwrap();
    let report = s.normalise(&stdout(&report));
    assert_eq!(report, s.normalise(&stdout(&compare_default)));
    let plain = s
        .cmd()
        .args(["report", "text", "a.txt", "b.txt"])
        .output()
        .unwrap();
    assert_ne!(
        report,
        s.normalise(&stdout(&plain)),
        "the anchor changes the report"
    );
    let new = s
        .cmd()
        .args([
            "session",
            "new",
            "t",
            "--type",
            "text-compare",
            "--left",
            "a.txt",
            "--right",
            "b.txt",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&new), 0, "{}", stderr(&new));
    let session = s
        .cmd()
        .args(["session", "run", "t", "--align", "10-14:12-13", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&session), 13, "{}", stderr(&session));
    let direct = s
        .cmd()
        .args([
            "text",
            "compare",
            "a.txt",
            "b.txt",
            "--align",
            "10-14:12-13",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(stdout(&session), stdout(&direct));
    // A report of the session takes them too.
    let report_session = s
        .cmd()
        .args(["report", "text", "--session", "t", "--isolate", "right:1"])
        .output()
        .unwrap();
    assert_eq!(code(&report_session), 13, "{}", stderr(&report_session));
    // A folder session refuses them.
    s.dir("l");
    s.dir("r");
    let folder = s
        .cmd()
        .args([
            "session",
            "new",
            "f",
            "--type",
            "folder-compare",
            "--left",
            "l",
            "--right",
            "r",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&folder), 0, "{}", stderr(&folder));
    let refused = s
        .cmd()
        .args(["session", "run", "f", "--align", "1:1", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&refused), 100);
    assert_eq!(json(&refused)["error"]["kind"], "invalid_input");
}

/// M2 W11b (D-314): `--gui` takes the anchors (the window applies them); a word that is not an anchor is still refused
/// before any GUI is contacted.
#[test]
fn gui_takes_anchors_and_refuses_bad_words() {
    let s = pair();
    let run = |words: &[&str]| {
        s.cmd()
            .args(["text", "compare", "a.txt", "b.txt", "--gui", "--no-launch"])
            .args(words)
            .arg("--json")
            .output()
            .unwrap()
    };
    let out = run(&["--align", "x:1"]);
    assert_eq!(code(&out), 100, "{}", stdout(&out));
    let doc = json(&out);
    assert_eq!(doc["schema"], "hikasa.gui_open.v1");
    assert_eq!(doc["error"]["kind"], "invalid_input");
    // A good word gets as far as the window (none runs here, and `--no-launch` starts none).
    let out = run(&["--align", "1:1"]);
    assert_eq!(code(&out), 100, "{}", stdout(&out));
    let doc = json(&out);
    assert_ne!(doc["error"]["kind"], "invalid_input", "{doc}");
}
