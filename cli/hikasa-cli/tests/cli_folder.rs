//! `hikasa folder compare` end to end (contract §6.3 `cli_folder.rs`, §3.3, §4.3.3; D-02, D-29, D-44, D-50,
//! D-57).

mod common;

use std::path::Path;
use std::sync::Arc;

use common::{
    CloseMode, FakeGui, MTIME, Sandbox, assert_golden, code, json, serve, stderr, stdout,
};
use hikasa_core::ExitCode;
use hikasa_session::ipc::Request;

/// The tree of the goldens:
///
/// | path | left | right |
/// |---|---|---|
/// | `same.txt` | same | same |
/// | `src/main.c` | `return 0` | `return 1` (same size and time: equal by the quick tests) |
/// | `src/util.c` | older | newer, longer |
/// | `src/lib.h` | 10 bytes | 11 bytes (a trailing space), same time (different) |
/// | `docs/old.md` | yes | — |
/// | `docs/new.md` | — | yes |
/// | `img/logo.bin` | binary | binary, one byte changed (same size and time) |
fn tree() -> Sandbox {
    let s = Sandbox::new();
    for side in ["a", "b"] {
        s.file(&format!("{side}/same.txt"), "same\n");
    }
    s.file("a/src/main.c", "int main(void) { return 0; }\n");
    s.file("b/src/main.c", "int main(void) { return 1; }\n");
    s.file_at("a/src/util.c", "int util;\n", MTIME);
    s.file_at("b/src/util.c", "int util = 1;\n", MTIME + 3600);
    s.file("a/src/lib.h", "int lib;\n\n");
    s.file("b/src/lib.h", "int lib; \n\n");
    s.file("a/docs/old.md", "# Old\n");
    s.file("b/docs/new.md", "# New\n");
    s.file("a/img/logo.bin", b"\x00PNG\x01\x02\x03");
    s.file("b/img/logo.bin", b"\x00PNG\x01\x02\x04");
    s.settle("a");
    s.settle("b");
    s
}

/// Runs `folder compare a b <extra>`.
fn compare(s: &Sandbox, extra: &[&str]) -> std::process::Output {
    s.cmd()
        .args(["folder", "compare", "a", "b"])
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn list_golden() {
    let s = tree();
    let out = compare(&s, &[]);
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.ends_with("unknown, 0 errors\n"), "{text}");
    assert_golden("folder/list.txt", &s.normalise(&text));
    let all = compare(&s, &["--display", "all"]);
    assert_golden("folder/list-all.txt", &s.normalise(&stdout(&all)));
}

#[test]
fn json_golden() {
    let s = tree();
    let out = compare(&s, &["--json"]);
    assert_eq!(code(&out), 13);
    let doc = json(&out);
    assert_eq!(doc["result"]["verdict"], "differences");
    assert_eq!(doc["result"]["display"]["filter"], "differences");
    assert_eq!(doc["result"]["display"]["folders"], "compare_structure");
    common::assert_platform_metadata(&stdout(&out));
    assert_golden("folder/json.json", &s.normalise(&stdout(&out)));
}

#[test]
fn exit_codes() {
    // 2: no differences.
    let same = Sandbox::new();
    same.file("a/x.txt", "x\n");
    same.file("b/x.txt", "x\n");
    let out = same
        .cmd()
        .args(["folder", "compare", "a", "b"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    // 12: only unimportant content differences.
    let similar = Sandbox::new();
    similar.file("a/x.txt", "a b\n");
    similar.file("b/x.txt", "a   b\n");
    let args = [
        "folder",
        "compare",
        "a",
        "b",
        "--criteria",
        "rules-based",
        "--unimportant",
        "whitespace",
    ];
    let out = similar.cmd().args(args).output().unwrap();
    assert_eq!(code(&out), 12, "{}", stderr(&out));
    let ignored = similar.cmd().args(args).arg("--iu").output().unwrap();
    assert_eq!(code(&ignored), 2);
    // 13: differences.
    let s = tree();
    assert_eq!(code(&compare(&s, &[])), 13);
    // 107: a base that cannot be loaded.
    let missing = s
        .cmd()
        .args(["folder", "compare", "a", "nowhere", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&missing), 107);
    let doc = json(&missing);
    assert_eq!(doc["exit_code"], 107);
    assert!(doc["error"]["spec"].as_str().unwrap().ends_with("nowhere"));
    let file_base = s
        .cmd()
        .args(["folder", "compare", "a", "a/same.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&file_base), 107, "a base that is not a folder");
    // 100: any other error.
    for criteria in ["crc,binary", "version", "bogus"] {
        let out = compare(&s, &["--criteria", criteria]);
        assert_eq!(code(&out), 100, "{criteria}");
    }
}

#[cfg(unix)]
#[test]
fn an_unreadable_folder_makes_the_result_incomplete() {
    let s = Sandbox::new();
    s.file("a/x.txt", "x\n");
    s.file("b/x.txt", "x\n");
    s.file("a/locked/y.txt", "y\n");
    s.file("b/locked/y.txt", "y\n");
    common::set_mode(&s.join("a/locked"), 0o000);
    let readable = std::fs::read_dir(s.join("a/locked")).is_ok();
    let out = s
        .cmd()
        .args(["folder", "compare", "a", "b", "--json"])
        .output()
        .unwrap();
    common::set_mode(&s.join("a/locked"), 0o755);
    if readable {
        // Running with privileges that ignore permissions: nothing to test.
        return;
    }
    assert_eq!(code(&out), 100, "D-02: an error node cannot claim sameness");
    assert_eq!(json(&out)["result"]["verdict"], "incomplete");
}

#[test]
fn filters_and_suppress_filters() {
    let s = tree();
    let out = compare(&s, &["--filters", "*.c", "--display", "all"]);
    let text = stdout(&out);
    assert!(
        text.contains("src/main.c") && !text.contains("same.txt"),
        "{text}"
    );
    let suppressed = compare(
        &s,
        &["--filters", "*.c", "--display", "all", "--suppress-filters"],
    );
    let text = stdout(&suppressed);
    assert!(
        text.contains(".  same.txt"),
        "filtered items in teal: {text}"
    );
    assert_golden("folder/suppress-filters.txt", &s.normalise(&text));
    let excluded = compare(&s, &["--filters", "-docs\\", "--display", "all"]);
    assert!(
        !stdout(&excluded).contains("docs/"),
        "{}",
        stdout(&excluded)
    );
}

#[test]
fn criteria_rows() {
    let s = tree();
    let rules = |criteria: &str| -> serde_json::Value {
        let out = compare(&s, &["--criteria", criteria, "--json"]);
        assert!(matches!(code(&out), 2 | 12 | 13 | 100), "{criteria}");
        json(&out)["result"].clone()
    };
    // Row 1: size only.
    let r = rules("size");
    assert_eq!(r["rules"]["criteria"]["size"], true);
    assert_eq!(r["rules"]["criteria"]["timestamps"]["enabled"], false);
    assert_eq!(r["rules"]["criteria"]["content"]["method"], "none");
    // Row 2: size and timestamps with a tolerance and DST.
    let r = rules("size,timestamp:3sec;IgnoreDST");
    assert_eq!(r["rules"]["criteria"]["timestamps"]["tolerance_secs"], 3);
    assert_eq!(r["rules"]["criteria"]["timestamps"]["ignore_dst"], true);
    // Row 3: an offset and CRC content.
    let r = rules("timestamp timezone:left+2 crc");
    assert_eq!(r["rules"]["criteria"]["timestamps"]["offset"]["hours"], 2);
    assert_eq!(r["rules"]["criteria"]["content"]["method"], "crc");
    // Row 4: rules content, and the display ignores unimportant differences.
    let r = rules("rules-based ignore-unimportant");
    assert_eq!(r["rules"]["criteria"]["content"]["method"], "rules");
    assert_eq!(r["display"]["ignore_unimportant"], true);
    // Row 5: attributes and owner.
    let r = rules("attrib:HRS,owner");
    assert_eq!(r["rules"]["criteria"]["dos_attributes"], "HRS");
    assert_eq!(r["rules"]["criteria"]["unix"]["owner"], true);
    // Rows 6 and 7 are errors (exit_codes). Individual flags win over --criteria (§3.5).
    let r = rules("size");
    assert_eq!(r["rules"]["criteria"]["size"], true);
    let out = compare(
        &s,
        &[
            "--criteria",
            "size",
            "--no-size",
            "--timestamps",
            "--tolerance",
            "5",
            "--json",
        ],
    );
    let r = &json(&out)["result"]["rules"]["criteria"];
    assert_eq!(r["size"], false);
    assert_eq!(r["timestamps"]["enabled"], true);
    assert_eq!(r["timestamps"]["tolerance_secs"], 5);
    // The unix flags set one test each (§1.8.7 item 19).
    let out = compare(&s, &["--criteria", "owner", "--group", "--json"]);
    let unix = &json(&out)["result"]["rules"]["criteria"]["unix"];
    assert_eq!(unix["owner"], true);
    assert_eq!(unix["group"], true);
    assert_eq!(unix["permissions"], false);
}

#[test]
fn rule_flags_reach_the_rules() {
    let s = tree();
    let out = compare(
        &s,
        &[
            "--json",
            "--ignore-dst",
            "--ignore-timezone",
            "--filename-case",
            "--attributes",
            "RH",
            "--permissions",
            "--content",
            "binary",
            "--skip-if-quick-same",
            "--no-override-quick",
            "--case-insensitive",
            "--align-extensions",
            "--align-normalization",
            "--cutoff",
            ">2000-01-01",
            "--size-filter",
            "<1MB",
            "--include-protected",
            "--follow-symlinks",
            "--no-scan-orphans",
        ],
    );
    assert!(matches!(code(&out), 2 | 13), "{}", stderr(&out));
    let rules = &json(&out)["result"]["rules"];
    let c = &rules["criteria"];
    assert_eq!(c["timestamps"]["ignore_dst"], true, "{rules}");
    assert_eq!(c["timestamps"]["ignore_timezone"], true);
    assert_eq!(c["filename_case"], true);
    assert_eq!(c["dos_attributes"], "HR");
    assert_eq!(c["unix"]["permissions"], true);
    assert_eq!(c["content"]["method"], "binary");
    assert_eq!(c["content"]["skip_if_quick_same"], true);
    assert_eq!(c["content"]["override_quick"], false);
    assert_eq!(rules["alignment"]["case"], "insensitive");
    assert_eq!(rules["alignment"]["align_extensions"], true);
    assert_eq!(rules["alignment"]["align_normalization"], true);
    assert!(!rules["filters"]["other"]["date"].is_null());
    assert!(!rules["filters"]["other"]["size"].is_null());
    assert_eq!(rules["filters"]["other"]["exclude_protected"], false);
    assert_eq!(rules["handling"]["follow_symlinks"], true);
    assert_eq!(rules["handling"]["scan_orphan_folders"], false);
    for (flag, value) in [
        ("--cutoff", "yesterday"),
        ("--size-filter", "big"),
        ("--attributes", "Q"),
        ("--content", "md5"),
        ("--folders", "tree"),
        ("--columns", "colour"),
        ("--report", "tree"),
    ] {
        let bad = compare(&s, &[flag, value]);
        assert_eq!(code(&bad), 100, "{flag} {value}");
        assert!(bad.stdout.is_empty());
    }
}

#[test]
fn every_display_preset() {
    let s = tree();
    let mut seen = std::collections::BTreeSet::new();
    for preset in [
        "all",
        "differences",
        "same",
        "no-orphans",
        "differences-no-orphans",
        "orphans",
        "left-newer",
        "right-newer",
        "left-newer-orphans",
        "right-newer-orphans",
        "left-orphans",
        "right-orphans",
        "none",
        "display-mismatches",
    ] {
        let out = compare(&s, &["--display", preset]);
        assert_eq!(
            code(&out),
            13,
            "display filters never change the exit code (D-03): {preset}"
        );
        let text = stdout(&out);
        let rows: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
        match preset {
            "same" => assert!(
                rows.iter().all(|r| r.starts_with('=') || r.ends_with('/')),
                "{text}"
            ),
            "left-orphans" => assert!(rows.contains(&"-  docs/old.md"), "{text}"),
            "right-orphans" => assert!(rows.contains(&"+  docs/new.md"), "{text}"),
            "right-newer" => assert!(rows.contains(&"<  src/util.c"), "{text}"),
            "none" => assert!(rows.is_empty(), "{text}"),
            _ => {}
        }
        seen.insert(rows.join("\n"));
    }
    assert!(seen.len() >= 6, "the presets show different rows");
    let bogus = compare(&s, &["--display", "everything"]);
    assert_eq!(code(&bogus), 100);
    let always = compare(&s, &["--display", "same", "--folders", "always"]);
    assert!(stdout(&always).contains("docs/"), "{}", stdout(&always));
    // A folder on one side only, without files: shown with the structure, hidden with files only.
    let lone = Sandbox::new();
    lone.file("a/x.txt", "x\n");
    lone.file("b/x.txt", "x\n");
    lone.dir("a/only");
    let run = |mode: &str| {
        let out = lone
            .cmd()
            .args(["folder", "compare", "a", "b", "--folders", mode])
            .output()
            .unwrap();
        assert_eq!(code(&out), 13, "{mode}");
        stdout(&out)
    };
    assert!(run("structure").contains("-  only/"));
    assert!(!run("files-only").contains("only/"));
}

#[test]
fn flatten() {
    let s = tree();
    let out = compare(&s, &["--flatten", "--display", "all"]);
    assert_eq!(code(&out), 13);
    let text = stdout(&out);
    assert!(
        !text.lines().any(|l| l.ends_with('/')),
        "no folder rows: {text}"
    );
    assert_golden("folder/flatten.txt", &s.normalise(&text));
}

#[test]
fn content_methods() {
    let s = tree();
    // img/logo.bin and src/main.c are equal by size and time: only a content test sees them differ.
    let quick = compare(&s, &["--display", "all"]);
    assert!(
        stdout(&quick).contains("=  img/logo.bin"),
        "{}",
        stdout(&quick)
    );
    for method in ["crc", "binary", "rules"] {
        let out = compare(&s, &["--content", method, "--display", "all", "--json"]);
        assert_eq!(code(&out), 13, "{method}");
        let doc = json(&out);
        let items = doc["result"]["items"].as_array().unwrap();
        let logo = items.iter().find(|i| i["path"] == "img/logo.bin").unwrap();
        assert_eq!(logo["state"], "different", "{method}: {logo}");
        assert_eq!(logo["content"]["method"], method);
    }
    let none = compare(&s, &["--content", "none", "--criteria", "size"]);
    assert_eq!(code(&none), 13);
    // Rules-based content with unimportant whitespace: lib.h differs in whitespace only.
    let out = compare(
        &s,
        &[
            "--content",
            "rules",
            "--unimportant",
            "whitespace",
            "--display",
            "all",
            "--json",
        ],
    );
    let doc = json(&out);
    let items = doc["result"]["items"].as_array().unwrap();
    let lib = items.iter().find(|i| i["path"] == "src/lib.h").unwrap();
    assert_eq!(
        lib["content"]["result"]["result"], "unimportant_only",
        "{lib}"
    );
}

#[test]
fn side_by_side_html_output() {
    let s = tree();
    let out = compare(
        &s,
        &[
            "--report",
            "side-by-side",
            "--html",
            "--output",
            "r.html",
            "--title",
            "Tree",
        ],
    );
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    assert!(out.stdout.is_empty(), "--output prints nothing");
    let page = std::fs::read_to_string(s.join("r.html")).unwrap();
    assert!(page.contains("<html") && page.contains("Tree") && page.contains("util.c"));
    let text = compare(
        &s,
        &[
            "--report",
            "side-by-side",
            "--columns",
            "size,modified,status",
        ],
    );
    assert_eq!(code(&text), 13);
    assert_golden(
        "folder/side-by-side.txt",
        &s.normalise_with(&stdout(&text), true),
    );
    let summary = compare(&s, &["--report", "summary"]);
    assert_golden("folder/summary.txt", &s.normalise(&stdout(&summary)));
    let report_json = compare(&s, &["--report", "json"]);
    let doc: serde_json::Value = serde_json::from_slice(&report_json.stdout).unwrap();
    assert_eq!(doc["schema"], "hikasa.folder_compare.v1");
}

#[test]
fn gui_sends_the_comparison_to_the_gui() {
    let s = tree();
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::RulesDifferent);
    let server = serve(&s, Arc::clone(&gui));
    let out = compare(
        &s,
        &[
            "--gui",
            "--wait",
            "--filters",
            "*.c",
            "--criteria",
            "size crc",
            "--display",
            "orphans",
            "--folders",
            "always",
            "--expand-all",
        ],
    );
    assert_eq!(code(&out), 13, "the view's code: {}", stderr(&out));
    assert!(out.stdout.is_empty());
    let requests = gui.requests();
    let Some(Request::OpenView {
        kind,
        specs,
        options,
        ..
    }) = requests
        .iter()
        .find(|r| matches!(r, Request::OpenView { .. }))
    else {
        panic!("no OpenView: {requests:?}");
    };
    assert_eq!(kind.as_str(), "folder_compare");
    assert!(
        Path::new(&specs[0]).ends_with("a") && Path::new(&specs[1]).ends_with("b"),
        "{specs:?}"
    );
    assert_eq!(options.criteria.as_deref(), Some("size crc"));
    assert_eq!(options.display.as_deref(), Some("orphans"));
    assert_eq!(options.folders.as_deref(), Some("always"));
    assert!(options.expand_all);
    let patch = options.folder_rules.as_ref().unwrap();
    assert_eq!(patch.name_filter.as_deref(), Some("*.c"));
    // A bad --criteria is refused locally: the GUI is never contacted.
    let before = gui.requests().len();
    let bad = compare(&s, &["--gui", "--criteria", "version", "--json"]);
    assert_eq!(code(&bad), 100);
    assert_eq!(json(&bad)["schema"], "hikasa.gui_open.v1");
    assert_eq!(gui.requests().len(), before);
    server.shutdown().unwrap();
}
