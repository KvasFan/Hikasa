//! Golden comparisons (§6.2.5): `tests/fixtures/compare/<case>/{left.<ext>, right.<ext>, rules.toml?,
//! expected.json}`. The effective format is detected from the left file name (D-15), unless `format.txt` names
//! a built-in format (forced) or `format.toml` holds a whole format file. `expected.json` is the canonical
//! serialisation of the runs, the rows (run-length compressed), sections, statistics, verdicts, warnings and
//! the classified inline ranges. `HIKASA_BLESS=1` rewrites the expected files.
//!
//! Every case is also checked for: the same result on a 4-thread pool (T2), the consistency theorem
//! (`quick_compare_bytes == compare().verdict(false)`, TC-016) and the invariants.

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use common::{assert_golden, canonical_json, fixtures, registry};
use hikasa_core::Ctl;
use hikasa_formats::{CompiledFormat, EnabledOverrides, RegexLimits, parse_format};
use hikasa_text::{
    CompareInput, DocOrigin, LoadOptions, TextDocument, TextRules, compare, quick_compare_bytes,
};

/// The file of `dir` whose name starts with `stem.`.
fn side_file(dir: &Path, stem: &str) -> std::path::PathBuf {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&format!("{stem}.")))
        })
        .unwrap_or_else(|| panic!("{}: no {stem}.* file", dir.display()))
}

fn effective_format(dir: &Path, left_name: &str) -> Arc<CompiledFormat> {
    let registry = registry();
    if let Ok(id) = fs::read_to_string(dir.join("format.txt")) {
        return registry.force(id.trim()).unwrap().format;
    }
    if let Ok(text) = fs::read_to_string(dir.join("format.toml")) {
        let (format, warnings) = parse_format(&text, "format.toml").unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        return Arc::new(
            CompiledFormat::compile(Arc::new(format), &RegexLimits::default()).unwrap(),
        );
    }
    registry
        .detect(left_name, &EnabledOverrides::default())
        .unwrap()
        .format
}

fn run_case(dir: &Path) {
    let left_path = side_file(dir, "left");
    let right_path = side_file(dir, "right");
    let left_bytes = fs::read(&left_path).unwrap();
    let right_bytes = fs::read(&right_path).unwrap();
    let left_name = left_path.file_name().unwrap().to_str().unwrap().to_owned();
    let right_name = right_path.file_name().unwrap().to_str().unwrap().to_owned();
    let format = effective_format(dir, &left_name);
    let rules = match fs::read_to_string(dir.join("rules.toml")) {
        Ok(text) => {
            let (rules, warnings) = TextRules::from_toml(&text).unwrap();
            assert!(warnings.is_empty(), "{}: {warnings:?}", dir.display());
            rules
        }
        Err(_) => TextRules::default(),
    };
    rules.validate(&RegexLimits::default()).unwrap();
    let load = LoadOptions::default();
    let doc = |bytes: &[u8]| {
        Arc::new(
            TextDocument::from_bytes(
                bytes,
                &format,
                &load,
                DocOrigin {
                    spec: None,
                    mtime: None,
                    size: bytes.len() as u64,
                },
            )
            .unwrap(),
        )
    };
    let input = CompareInput {
        left: doc(&left_bytes),
        right: doc(&right_bytes),
        format: Arc::clone(&format),
        rules: rules.clone(),
        bytes_identical: left_bytes == right_bytes,
        force_tokenize: false,
    };
    let cmp = compare(input.clone(), None, &Ctl::detached(), None).unwrap();
    cmp.check_invariants().unwrap();
    let json = canonical_json(&cmp);
    assert_golden(&dir.join("expected.json"), json.as_bytes());

    // T2: the same result on a pool.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let parallel = compare(input, Some(&pool), &Ctl::detached(), None).unwrap();
    assert_eq!(canonical_json(&parallel), json, "{}: pool", dir.display());

    // TC-016: the quick compare agrees with the verdict.
    let quick = quick_compare_bytes(
        (&left_bytes, &left_name),
        (&right_bytes, &right_name),
        &format,
        &rules,
        &load,
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!(quick.result, cmp.verdict(false), "{}: quick", dir.display());
}

#[test]
fn golden_comparisons() {
    let root = fixtures("compare");
    let mut dirs: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(dirs.len() >= 25, "only {} golden cases", dirs.len());
    for dir in &dirs {
        run_case(dir);
    }
    // Every algorithm is covered by at least one case.
    let mut algorithms: Vec<String> = dirs
        .iter()
        .map(|d| {
            let json: serde_json::Value =
                serde_json::from_slice(&fs::read(d.join("expected.json")).unwrap()).unwrap();
            json["algorithm"].as_str().unwrap().to_owned()
        })
        .collect();
    algorithms.sort();
    algorithms.dedup();
    assert_eq!(
        algorithms,
        ["histogram", "myers", "patience", "standard", "unaligned"]
    );
}
