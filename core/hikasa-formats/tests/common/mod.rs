//! Shared helpers of the hikasa-formats integration tests (§6.1): the golden-file helper, fixture paths, the
//! proptest configuration, a line splitter and a token dump.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use hikasa_formats::{CompiledGrammar, LineState, Tokenizer, write_token_dump};
use proptest::test_runner::{Config, FileFailurePersistence};

/// Compares `actual` with the golden file `expected`; with `HIKASA_BLESS=1` the golden file is (re)written
/// instead. Text files are shown as a `pretty_assertions` diff.
pub fn assert_golden(expected: &Path, actual: &[u8]) {
    let bless = std::env::var_os("HIKASA_BLESS").is_some_and(|v| v == "1");
    if bless {
        if let Some(dir) = expected.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(expected, actual).unwrap();
        return;
    }
    let golden = std::fs::read(expected).unwrap_or_else(|e| {
        panic!(
            "missing golden file {} ({e}); run with HIKASA_BLESS=1",
            expected.display()
        )
    });
    if golden != actual {
        match (std::str::from_utf8(&golden), std::str::from_utf8(actual)) {
            (Ok(want), Ok(got)) => {
                pretty_assertions::assert_eq!(want, got, "golden {}", expected.display())
            }
            _ => panic!("golden {} differs (binary)", expected.display()),
        }
    }
}

/// `tests/fixtures/<suite>`.
pub fn fixtures(suite: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(suite)
}

/// Proptest configuration: 256 cases unless `PROPTEST_CASES` says otherwise (§6.1); failing seeds are
/// persisted in `tests/proptest-regressions/<suite>.txt` and committed.
pub fn config(regressions: &'static str) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256);
    Config {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(regressions))),
        ..Config::default()
    }
}

/// Splits text into lines at CR, LF and CRLF (content without terminators; a final terminator does not start
/// another line), like the text document model.
pub fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(&text[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                lines.push(&text[start..i]);
                i += if bytes.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// The `hikasa format tokens` text dump of a whole document.
pub fn token_dump(grammar: &CompiledGrammar, lines: &[&str]) -> String {
    let mut out = String::new();
    let mut tokenizer = Tokenizer::new(grammar);
    let mut tokens = Vec::new();
    let mut state = LineState::Normal;
    for (k, line) in lines.iter().enumerate() {
        let outcome = tokenizer.tokenize_line(line, k as u32, state, &mut tokens);
        write_token_dump(&mut out, k as u32 + 1, line, &tokens, grammar.elements());
        state = outcome.end_state;
    }
    out
}
