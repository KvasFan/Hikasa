//! Shared helpers of the hikasa-text integration tests (§6.1): the golden-file helper, the proptest
//! configuration, document and comparison builders, the canonical serialisation of a comparison (goldens and
//! "pool None equals pool 8") and a recording observer.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hikasa_core::{ContentResult, Ctl, Phase, Progress, Warning};
use hikasa_formats::{FormatId, FormatRegistry};
use hikasa_text::{
    AlgorithmChoice, CompareInput, CompareObserver, CompiledFormat, Importance, PartialRows,
    RowIdx, RowState, Run, Section, TextComparison, TextDocument, TextRules, TextStats, compare,
};
use proptest::test_runner::{Config, FileFailurePersistence};
use serde::Serialize;

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

/// Proptest configuration: 256 cases unless `PROPTEST_CASES` says otherwise (§6.1); failing seeds are persisted
/// in `tests/proptest-regressions/<suite>.txt` and committed.
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

/// The built-in registry.
pub fn registry() -> FormatRegistry {
    FormatRegistry::builtin().unwrap()
}

/// A built-in format by id (`"ee"` = Everything Else).
pub fn format(id: &str) -> Arc<CompiledFormat> {
    if id == "ee" {
        return Arc::new(CompiledFormat::everything_else());
    }
    registry().compiled(&FormatId::new(id).unwrap()).unwrap()
}

/// A document from UTF-8 text.
pub fn doc(text: &str) -> Arc<TextDocument> {
    Arc::new(TextDocument::from_text(text))
}

/// Rules with an algorithm.
pub fn rules_with(algorithm: AlgorithmChoice) -> TextRules {
    let mut rules = TextRules::default();
    rules.alignment.algorithm = algorithm;
    rules
}

/// Compares two texts.
pub fn compare_texts(
    left: &str,
    right: &str,
    format: &Arc<CompiledFormat>,
    rules: &TextRules,
) -> TextComparison {
    compare_docs(doc(left), doc(right), format, rules, false, None)
}

/// Compares two documents (bytes identical when their texts are).
pub fn compare_docs(
    left: Arc<TextDocument>,
    right: Arc<TextDocument>,
    format: &Arc<CompiledFormat>,
    rules: &TextRules,
    force_tokenize: bool,
    pool: Option<&rayon::ThreadPool>,
) -> TextComparison {
    let input = CompareInput {
        bytes_identical: left.rope() == right.rope()
            && left.eol() == right.eol()
            && left.line_count() == right.line_count(),
        left,
        right,
        format: Arc::clone(format),
        rules: rules.clone(),
        force_tokenize,
    };
    let cmp = compare(input, pool, &Ctl::detached(), None).unwrap();
    cmp.check_invariants().unwrap();
    cmp
}

/// `[start, end)` as a two-element array (§4.1 rule 7).
fn pair(r: &std::ops::Range<u32>) -> [u32; 2] {
    [r.start, r.end]
}

#[derive(Serialize)]
struct RunDto {
    kind: &'static str,
    left: [u32; 2],
    right: [u32; 2],
}

#[derive(Serialize)]
struct RowsDto {
    rows: [u32; 2],
    state: RowState,
    importance: Importance,
}

#[derive(Serialize)]
struct InlineDto {
    row: u32,
    left: Vec<(u32, u32, bool)>,
    right: Vec<(u32, u32, bool)>,
    whole_line: bool,
}

#[derive(Serialize)]
struct SectionDto {
    id: u32,
    kind: &'static str,
    importance: Importance,
    rows: [u32; 2],
    left_lines: [u32; 2],
    right_lines: [u32; 2],
}

/// The canonical serialisation of a comparison (§6.2.5): algorithm, runs, rows (run-length compressed by
/// state and importance), sections, statistics, verdicts, warnings and the classified inline ranges of the
/// first 50 changed rows.
#[derive(Serialize)]
pub struct Canonical {
    format: String,
    algorithm: String,
    runs: Vec<RunDto>,
    rows: Vec<RowsDto>,
    sections: Vec<SectionDto>,
    stats: TextStats,
    verdict: ContentResult,
    verdict_ignore_unimportant: ContentResult,
    warnings: Vec<Warning>,
    inline: Vec<InlineDto>,
}

/// Builds the canonical form.
pub fn canonical(cmp: &TextComparison) -> Canonical {
    let mut rows: Vec<RowsDto> = Vec::new();
    for (k, row) in cmp.rows(0..cmp.row_count()).enumerate() {
        let k = k as u32;
        match rows.last_mut() {
            Some(last)
                if last.rows[1] == k
                    && last.state == row.state
                    && last.importance == row.importance =>
            {
                last.rows[1] = k + 1;
            }
            _ => rows.push(RowsDto {
                rows: [k, k + 1],
                state: row.state,
                importance: row.importance,
            }),
        }
    }
    let inline = (0..cmp.row_count())
        .filter_map(|k| {
            let c = cmp.inline(RowIdx(k))?;
            let conv = |v: &[hikasa_text::InlineRange]| {
                v.iter()
                    .map(|r| (r.range.start, r.range.end, r.important))
                    .collect()
            };
            Some(InlineDto {
                row: k,
                left: conv(&c.left),
                right: conv(&c.right),
                whole_line: c.whole_line,
            })
        })
        .take(50)
        .collect();
    Canonical {
        format: cmp.format().name().to_owned(),
        algorithm: cmp.algorithm().as_str().to_owned(),
        runs: cmp
            .runs()
            .iter()
            .map(|r: &Run| RunDto {
                kind: r.kind.as_str(),
                left: pair(&r.left),
                right: pair(&r.right),
            })
            .collect(),
        rows,
        sections: cmp
            .sections()
            .iter()
            .map(|s: &Section| SectionDto {
                id: s.id.0,
                kind: s.kind.as_str(),
                importance: s.importance,
                rows: pair(&s.rows),
                left_lines: pair(&s.left_lines),
                right_lines: pair(&s.right_lines),
            })
            .collect(),
        stats: *cmp.stats(),
        verdict: cmp.verdict(false),
        verdict_ignore_unimportant: cmp.verdict(true),
        warnings: cmp.warnings().to_vec(),
        inline,
    }
}

/// The canonical form as pretty JSON (2 spaces, trailing newline; §4.1 rule 1).
pub fn canonical_json(cmp: &TextComparison) -> String {
    let mut text = serde_json::to_string_pretty(&canonical(cmp)).unwrap();
    text.push('\n');
    text
}

/// The canonical form without warnings and inline ranges (P-TC-7 compares the identity and the general path,
/// whose regex warnings may differ).
pub fn canonical_core(cmp: &TextComparison) -> String {
    let mut c = canonical(cmp);
    c.warnings.clear();
    serde_json::to_string(&c).unwrap()
}

/// What an observer saw.
#[derive(Debug, Default, Clone)]
pub struct Seen {
    pub phases: Vec<Phase>,
    pub partial: Vec<Run>,
    pub done: Vec<(u32, u32)>,
    pub loaded: usize,
}

/// A recording observer.
#[derive(Debug, Default)]
pub struct Recorder(pub Seen);

impl CompareObserver for Recorder {
    fn loaded(&mut self, _left: &Arc<TextDocument>, _right: &Arc<TextDocument>) {
        self.0.loaded += 1;
    }

    fn phase(&mut self, phase: Phase) {
        self.0.phases.push(phase);
    }

    fn partial(&mut self, rows: &PartialRows<'_>) {
        self.0.partial.extend_from_slice(rows.runs);
        self.0.done.push((rows.left_done, rows.right_done));
    }
}

/// A progress sink that records every report.
#[derive(Debug, Default)]
pub struct ProgressLog(pub Mutex<Vec<Progress>>);

impl hikasa_core::ProgressSink for ProgressLog {
    fn report(&self, progress: Progress) {
        self.0.lock().unwrap().push(progress);
    }
}

/// Seeded xorshift64* (§6.1: no `rand` crate).
pub struct XorShift(pub u64);

impl XorShift {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}
