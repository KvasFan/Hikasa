//! Goldens of the script tests: `HIKASA_BLESS=1` rewrites them.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// The fixtures folder.
pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Compares `actual` with the golden `path`, or writes it with `HIKASA_BLESS=1`.
pub fn golden(path: &Path, actual: &str) {
    if std::env::var_os("HIKASA_BLESS").is_some_and(|v| v == "1") {
        std::fs::write(path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e} (bless with HIKASA_BLESS=1)", path.display()));
    pretty_assertions::assert_eq!(expected, actual, "{}", path.display());
}

/// Every `*.txt` of a fixtures subfolder, sorted.
pub fn scripts(dir: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(fixtures().join(dir))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    out.sort();
    out
}
