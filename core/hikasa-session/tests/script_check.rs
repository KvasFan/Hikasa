//! A8.3 with the Service's `WordChecker` (D-222): a word each CLI parser refuses is 106 at its column; the same word with
//! `%` passes the check (it is judged when it runs); nothing runs. And A8.15's smoke bound: a 16 MiB script checks within
//! G14's parse bound scaled by 4 in a debug build.

mod common;
#[path = "script_common/mod.rs"]
mod script_common;

use std::collections::BTreeMap;
use std::ffi::OsString;

use hikasa_core::ExitCode;
use script_common::World;

fn errors(w: &World, source: &str) -> Vec<String> {
    let text = w.text(source);
    let check = w.service.script_check(&text, None, &BTreeMap::new());
    check.errors.lines("s.txt")
}

#[test]
fn a8_3_each_vocabulary_refuses_its_word_at_its_column() {
    let w = World::new();
    let rows: &[(&str, &str)] = &[
        ("load l r\nselect left.bogus\n", "s.txt:2:8: error: "),
        ("criteria crc nonsense\n", "s.txt:1:14: error: "),
        ("filter cutoff:<yesterday\n", "s.txt:1:8: error: "),
        ("filter size:>lots\n", "s.txt:1:8: error: "),
        ("filter attrib:+q\n", "s.txt:1:8: error: "),
        ("filter unixtype:+z\n", "s.txt:1:8: error: "),
        ("option confirm:maybe\n", "s.txt:1:8: error: "),
        (
            "text-report layout:sideways output-to:x a b\n",
            "s.txt:1:13: error: ",
        ),
        (
            "text-report options:display-bogus output-to:x a b\n",
            "s.txt:1:13: error: ",
        ),
        (
            "load l r\nfolder-report layout:patch output-to:x\n",
            "s.txt:2:15: error: ",
        ),
        ("load l r\ntouch left:yesterday\n", "s.txt:2:12: error: "),
        ("load l r\ncompare sha1\n", "s.txt:2:9: error: "),
        ("load l r\ncopyto path:sideways /t\n", "s.txt:2:8: error: "),
        ("load l r\nattrib +q\n", "s.txt:2:8: error: "),
    ];
    for (source, at) in rows {
        let lines = errors(&w, source);
        assert_eq!(lines.len(), 1, "{source}: {lines:?}");
        assert!(lines[0].starts_with(at), "{source}: {lines:?}");
    }
    // The same words through `%` pass the check.
    let text = w.text("load l r\nselect %1\ncriteria %2\noption confirm:%3\ntouch left:%4\n");
    let args: Vec<OsString> = ["left.bogus", "nonsense", "maybe", "yesterday"]
        .map(OsString::from)
        .to_vec();
    let check = w.service.script_check(&text, Some(&args), &BTreeMap::new());
    assert!(check.errors.is_empty(), "{:?}", check.errors.lines("s.txt"));
    // `lt` reaches the criteria parser as `left`; the CLI's own `--criteria` stays strict (A8.5).
    assert!(errors(&w, "criteria timezone:lt+2\n").is_empty());
    let text = w.text("beep\n");
    let _ = text;
}

#[test]
fn a8_3_nothing_runs_when_a_check_fails() {
    let w = World::new();
    w.file("l/a.txt", "a", 1_000_000);
    w.dir("r");
    let text = w.text("load l r\nselect all\ncopy left->right\nselct\n");
    let check = w.service.script_check(&text, Some(&[]), &BTreeMap::new());
    assert_eq!(check.exit_code(), ExitCode::ScriptSyntaxError);
    assert!(check.script.is_none());
    assert!(w.dump("r").is_empty());
}

#[test]
fn a8_15_a_16_mib_script_checks_within_the_scaled_bound() {
    let w = World::new();
    let line = "load \"/a b/c\" &\n  # comment\n  d\nselect %1 left.diff.files\ncopyto left path:base \"x%date%.zip\"\n";
    let mut source = String::new();
    while source.len() + line.len() <= 16 * 1024 * 1024 {
        source.push_str(line);
    }
    let text = w.text(&source);
    let args = vec![OsString::from("all")];
    let start = std::time::Instant::now();
    let check = w.service.script_check(&text, Some(&args), &BTreeMap::new());
    let took = start.elapsed();
    assert!(
        check.errors.is_empty(),
        "{:?}",
        check.errors.lines("s.txt").first()
    );
    // G14: 0.5 s for 8 MB in release; 16 MiB is twice that, and a debug build gets 4 times the time.
    eprintln!("16 MiB check: {took:?}");
    assert!(took.as_secs_f64() <= 0.5 * 2.0 * 4.0, "{took:?}");
}

/// A8.21: docs/12's command table of §3.1 and the parser's agree (no command missing, none extra), every keyword of the
/// parser is in that table, and every check code and new warning code is in the help.
#[test]
fn a8_21_docs_and_help_name_the_language() {
    use hikasa_session::api::script::{CommandName, SCRIPT_KEYWORDS, ScriptErrorCode};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let docs = std::fs::read_to_string(root.join("docs/12-cli-and-scripting.md")).unwrap();
    let start = docs.find("### 3.1 Commands").unwrap();
    let block = &docs[start..];
    let block = &block[block.find("```").unwrap() + 3..];
    let block = &block[..block.find("```").unwrap()];
    let mut documented = std::collections::BTreeSet::new();
    for line in block.lines() {
        if line.starts_with(' ') || line.trim().is_empty() {
            continue;
        }
        let first = line.split_whitespace().next().unwrap();
        documented.insert(first.to_owned());
    }
    let parsed: std::collections::BTreeSet<String> = CommandName::ALL
        .iter()
        .map(|n| n.as_str().to_owned())
        .collect();
    assert_eq!(documented, parsed, "docs/12 §3.1 against the parser");
    assert!(block.contains("alias data-report"));
    for keyword in SCRIPT_KEYWORDS {
        // The grammar writes `(update|mirror):`, `recyclebin=`: the stem of each keyword is there.
        let stem = keyword.trim_end_matches([':', '=']);
        assert!(block.contains(stem), "{keyword} is not in docs/12 §3.1");
    }
    let script = std::fs::read_to_string(root.join("help/script.md")).unwrap();
    let json = std::fs::read_to_string(root.join("help/json-output.md")).unwrap();
    for code in ScriptErrorCode::ALL {
        assert!(
            script.contains(&format!("`{}`", code.as_str())),
            "{}",
            code.as_str()
        );
    }
    for code in ["nothing_selected", "attrib_unsupported", "log_not_written"] {
        assert!(json.contains(code) || script.contains(code), "{code}");
    }
}
