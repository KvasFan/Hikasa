//! Shared helpers of the hikasa-core integration tests: the golden-file helper (§6.1), the proptest
//! configuration and proptest strategies.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use proptest::prelude::*;
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

/// `tests/fixtures/<suite>/<case>`.
pub fn fixture(suite: &str, case: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(suite)
        .join(case)
}

/// Proptest configuration of the A1.3 properties: 4096 cases unless `PROPTEST_CASES` says otherwise; failing
/// seeds are persisted in `regressions` (`tests/proptest-regressions/<suite>.txt`, relative to the package, which
/// is the working directory of `cargo test`) and committed (§6.1).
pub fn config(regressions: &'static str) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4096);
    Config {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(regressions))),
        ..Config::default()
    }
}

/// Mask items over the alphabet of P-MASK-1: `a b . * ? [ ] ! -`.
pub fn simple_mask_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop::sample::select(vec!['a', 'b', '.', '*', '?', '[', ']', '!', '-']),
        1..8,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

/// Names for P-MASK-1 (ASCII: the regex oracle's case folding equals the simple 1:1 folding there).
pub fn ascii_name() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop::sample::select(vec![
            'a', 'b', 'A', 'B', '.', '-', '[', ']', '!', '*', '?', 'c',
        ]),
        0..8,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

/// A component usable in a `RelPath` (non-empty, not `.`/`..`, no `/`). Mapped, not filtered, so that runs
/// with millions of cases never hit proptest's rejection limit.
pub fn rel_component() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop::sample::select(vec!['a', 'b', 'A', '.', '-', 'x']),
        1..5,
    )
    .prop_map(|chars| valid_component(chars.into_iter().collect()))
}

/// `c` itself, or `c` + "x" when `c` is `.` or `..`.
pub fn valid_component(c: String) -> String {
    if c == "." || c == ".." { c + "x" } else { c }
}

/// Mask items with separators, `.`, `...`, leading `-` and trailing separators.
pub fn path_mask_text() -> impl Strategy<Value = String> {
    let component = prop_oneof![
        6 => proptest::collection::vec(prop::sample::select(vec!['a', 'b', 'x', '.', '*', '?', '[', ']', '!', '-']), 1..5)
            .prop_map(|chars| chars.into_iter().collect::<String>()),
        1 => Just("...".to_owned()),
        1 => Just(".".to_owned()),
    ];
    (
        any::<bool>(),
        prop::sample::select(vec!["", "\\", "/", ".\\", "./"]),
        proptest::collection::vec((component, prop::sample::select(vec!["\\", "/"])), 1..4),
        prop::sample::select(vec!["", "\\", "/"]),
    )
        .prop_map(|(exclude, lead, parts, trail)| {
            let mut text = String::new();
            if exclude {
                text.push('-');
            }
            text.push_str(lead);
            for (k, (component, sep)) in parts.iter().enumerate() {
                if k > 0 {
                    text.push_str(sep);
                }
                text.push_str(component);
            }
            text.push_str(trail);
            text
        })
}
