//! A8.3 (pure part): one golden per `ScriptErrorCode` with its `<script>:<line>:<column>: error: <message>` line, the flow
//! check, suggestions, the cap of 100 errors, and a host's refusal at its column.

mod common;

use std::collections::BTreeMap;
use std::ffi::OsString;

use hikasa_script::{
    AnyWord, CommandName, RefCheck, ScriptErrorCode, Slot, WordChecker, parse, parse_checked,
};

fn errors_of(source: &[u8], checker: &dyn WordChecker) -> Vec<String> {
    let env = BTreeMap::new();
    let args: Vec<OsString> = vec!["one".into()];
    let refs = RefCheck {
        args: Some(&args),
        env: &env,
        env_case_insensitive: false,
    };
    match parse_checked(source, checker, Some(&refs)) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.lines("s.txt"),
    }
}

#[test]
fn one_golden_per_code() {
    let scripts = common::scripts("check");
    for code in ScriptErrorCode::ALL.into_iter().filter(|c| !c.is_load()) {
        let script = common::fixtures().join(format!("check/{}.txt", code.as_str()));
        assert!(
            scripts.contains(&script),
            "no fixture for {}",
            code.as_str()
        );
    }
    for script in scripts {
        let source = std::fs::read(&script).unwrap();
        let env = BTreeMap::new();
        let args: Vec<OsString> = vec!["one".into()];
        let refs = RefCheck {
            args: Some(&args),
            env: &env,
            env_case_insensitive: false,
        };
        let errors = parse_checked(&source, &AnyWord, Some(&refs)).unwrap_err();
        let code = script.file_stem().unwrap().to_str().unwrap();
        assert_eq!(errors.list[0].code.as_str(), code);
        let mut text = errors.lines("s.txt").join("\n");
        text.push('\n');
        common::golden(&script.with_extension("err"), &text);
    }
}

#[test]
fn suggestions() {
    let lines = errors_of(b"selct all\n", &AnyWord);
    assert!(lines[0].contains("did you mean `select`"), "{lines:?}");
    let lines = errors_of(b"syncc\n", &AnyWord);
    assert!(lines[0].contains("did you mean `sync`"), "{lines:?}");
}

#[test]
fn every_command_that_needs_a_comparison() {
    let forms = [
        "expand all",
        "collapse all",
        "select all",
        "compare",
        "copy left->right",
        "move left->right",
        "copyto /t",
        "moveto /t",
        "delete left",
        "rename x",
        "touch left:now",
        "attrib +r",
        "sync update:all",
        "folder-report output-to:x",
    ];
    for form in forms {
        let name = CommandName::parse(form.split(' ').next().unwrap()).unwrap();
        assert!(name.needs_comparison(), "{form}");
        let lines = errors_of(form.as_bytes(), &AnyWord);
        assert_eq!(
            lines,
            vec![format!(
                "s.txt:1:1: error: {} needs a loaded comparison: no load before line 1",
                name.as_str()
            )]
        );
        let after = format!("load a b\nload <default>\n{form}\n");
        let lines = errors_of(after.as_bytes(), &AnyWord);
        assert_eq!(
            lines,
            vec![format!(
                "s.txt:3:1: error: {} needs a loaded comparison: load <default> at line 2 dropped it",
                name.as_str()
            )]
        );
        assert!(errors_of(format!("load a b\n{form}\n").as_bytes(), &AnyWord).is_empty());
    }
    let needing = CommandName::ALL
        .into_iter()
        .filter(|n| n.needs_comparison())
        .count();
    assert_eq!(needing, forms.len());
}

#[test]
fn mirror_all_is_refused() {
    let lines = errors_of(b"load a b\nsync mirror:all\n", &AnyWord);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("s.txt:2:6: error: mirror:all"),
        "{lines:?}"
    );
}

#[test]
fn a_hundred_and_fifty_errors_give_a_hundred_lines() {
    let source = "nope\n".repeat(150);
    let errors = parse(source.as_bytes(), &AnyWord).unwrap_err();
    let lines = errors.lines("s.txt");
    assert_eq!(lines.len(), 101);
    assert_eq!(lines[100], "… and 50 more");
    assert_eq!(errors.total(), 150);
}

/// Refuses the word `bad` in every slot, and `lt` (it must arrive as `left`).
struct Picky;

impl WordChecker for Picky {
    fn check(&self, slot: Slot, word: &str) -> Result<(), String> {
        if word == "bad" || word.contains("lt") || word.contains("rt") {
            return Err(format!("`{word}` refused in {slot:?}"));
        }
        Ok(())
    }
}

#[test]
fn a_refused_word_is_reported_at_its_column_unless_it_expands() {
    let lines = errors_of(b"load a b\nselect all bad\n", &Picky);
    assert_eq!(
        lines,
        vec!["s.txt:2:12: error: `bad` refused in SelectMask"]
    );
    let lines = errors_of(b"criteria   bad\n", &Picky);
    assert_eq!(lines, vec!["s.txt:1:12: error: `bad` refused in Criteria"]);
    // With `%` the word is judged when it ran.
    assert!(errors_of(b"criteria %1\n", &Picky).is_empty());
    // `lt`, `rt` reach the host as `left`, `right` in side positions.
    assert!(
        errors_of(
            b"load a b\ncopy lt->rt\ndelete RT\nselect Lt.diff.files\n",
            &Picky
        )
        .is_empty()
    );
    assert!(errors_of(b"criteria timezone:lt+2\n", &Picky).is_empty());
}

#[test]
fn missing_arguments_only_with_arguments_and_unset_always() {
    let env = BTreeMap::new();
    let none = RefCheck {
        args: None,
        env: &env,
        env_case_insensitive: false,
    };
    assert!(parse_checked(b"load %5 %6\n", &AnyWord, Some(&none)).is_ok());
    let errors = parse_checked(b"load %UNSET_Q% b\n", &AnyWord, Some(&none)).unwrap_err();
    assert_eq!(errors.list[0].code, ScriptErrorCode::UnsetVariable);
    let mut set = BTreeMap::new();
    set.insert("EMPTY".to_owned(), OsString::new());
    let refs = RefCheck {
        args: None,
        env: &set,
        env_case_insensitive: false,
    };
    assert!(parse_checked(b"load %EMPTY%a b\n", &AnyWord, Some(&refs)).is_ok());
}

#[test]
fn a_lexical_error_hides_no_other_error() {
    // An unknown command before the broken line, a `no_comparison` after it: every error, in line order.
    let lines = errors_of(b"foo\nlog \"x\nselect all\n", &AnyWord);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[0].starts_with("s.txt:1:1: error: "), "{lines:?}");
    assert!(lines[0].contains("unknown command"), "{lines:?}");
    assert!(lines[1].starts_with("s.txt:2:5: error: "), "{lines:?}");
    assert!(lines[1].contains("quote"), "{lines:?}");
    assert!(lines[2].starts_with("s.txt:3:1: error: "), "{lines:?}");
    assert!(lines[2].contains("needs a loaded comparison"), "{lines:?}");
    // A broken `load` still counts as a load: the `select` after it gives no second error.
    let lines = errors_of(b"load \"l r\nselect all\n", &AnyWord);
    assert_eq!(lines.len(), 1, "{lines:?}");
    // A continuation at the end, then more errors.
    let lines = errors_of(b"beep &\n\nfoo\n", &AnyWord);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("s.txt:1:6: error: "), "{lines:?}");
    assert!(lines[1].starts_with("s.txt:3:1: error: "), "{lines:?}");
}

#[test]
fn lexical_errors_count_toward_the_cap_in_order() {
    let mut source = String::new();
    for _ in 0..80 {
        source.push_str("foo\nlog \"x\n");
    }
    let errors = parse(source.as_bytes(), &AnyWord).unwrap_err();
    assert_eq!(errors.list.len(), 100);
    assert_eq!(errors.more, 60);
    let positions: Vec<u32> = errors.list.iter().map(|e| e.pos.line).collect();
    let mut sorted = positions.clone();
    sorted.sort_unstable();
    assert_eq!(positions, sorted);
    assert_eq!(errors.list[1].code, ScriptErrorCode::UnterminatedQuote);
}
