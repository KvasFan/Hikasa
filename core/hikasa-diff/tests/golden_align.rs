//! Golden alignments (§6.2.3): `tests/fixtures/align/<case>/{left.txt, right.txt, case.toml,
//! expected.<algorithm>.json}`. Each line is one token; trivial tokens are blank lines; closeness uses the bigram
//! similarity of the lines. Every case is aligned with all five algorithms. `HIKASA_BLESS=1` rewrites the
//! expected files.
//!
//! `case.toml` holds flat `key = value` lines (integers, booleans, quoted strings; `#` comments). The crate has
//! no `toml` dev-dependency (§0.5), so this file reads that subset itself. Keys: `description`, `skew`,
//! `closeness` (default true), `threshold`, `max_cells_per_gap`, `max_cells_total`, `never_align`,
//! `work_budget`, `weight_prefix` and `weight` (lines starting with the prefix get the weight on both sides).
//! Any other key is an error, so a misspelt key can never bless a wrong golden.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use common::{assert_golden, fixtures};
use hikasa_core::CancelToken;
use hikasa_diff::{
    Algorithm, AlignInput, AlignOptions, Alignment, BigramProfile, Closeness, PairScorer,
    TrivialSet, align, bigram_similarity, intern_pair,
};

/// A value of the `case.toml` subset.
#[derive(Debug, Clone)]
enum Value {
    Int(u64),
    Bool(bool),
    Str(String),
}

/// The keys `case.toml` may use.
const KEYS: [&str; 10] = [
    "description",
    "skew",
    "closeness",
    "threshold",
    "max_cells_per_gap",
    "max_cells_total",
    "never_align",
    "work_budget",
    "weight_prefix",
    "weight",
];

/// Parses `text` (from the file `source`, named in every panic message).
fn parse_case(source: &str, text: &str) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("{source} line {}: expected key = value", number + 1));
        let value = value.trim();
        let value = if let Some(s) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            Value::Str(s.replace("\\\"", "\"").replace("\\\\", "\\"))
        } else if value == "true" || value == "false" {
            Value::Bool(value == "true")
        } else {
            Value::Int(
                value
                    .replace('_', "")
                    .parse()
                    .unwrap_or_else(|_| panic!("{source} line {}: bad value {value}", number + 1)),
            )
        };
        let key = key.trim();
        assert!(
            KEYS.contains(&key),
            "{source} line {}: unknown key {key:?} (known: {KEYS:?})",
            number + 1
        );
        assert!(
            out.insert(key.to_owned(), value).is_none(),
            "{source} line {}: duplicate key {key:?}",
            number + 1
        );
    }
    out
}

struct Settings(BTreeMap<String, Value>);

impl Settings {
    fn int(&self, key: &str) -> Option<u64> {
        match self.0.get(key) {
            Some(Value::Int(v)) => Some(*v),
            None => None,
            Some(other) => panic!("{key}: expected an integer, got {other:?}"),
        }
    }

    fn bool(&self, key: &str, default: bool) -> bool {
        match self.0.get(key) {
            Some(Value::Bool(v)) => *v,
            None => default,
            Some(other) => panic!("{key}: expected a boolean, got {other:?}"),
        }
    }

    fn str(&self, key: &str) -> Option<&str> {
        match self.0.get(key) {
            Some(Value::Str(v)) => Some(v),
            None => None,
            Some(other) => panic!("{key}: expected a string, got {other:?}"),
        }
    }

    fn options(&self, algorithm: Algorithm) -> AlignOptions {
        let closeness = self.bool("closeness", true).then(|| Closeness {
            threshold_permille: self.int("threshold").map_or(500, |v| v as u16),
            max_cells_per_gap: self.int("max_cells_per_gap").unwrap_or(4_000_000),
            max_cells_total: self.int("max_cells_total"),
        });
        AlignOptions {
            algorithm,
            skew_tolerance: self.int("skew").map_or(300, |v| v as u32),
            closeness,
            never_align_differences: self.bool("never_align", false),
            work_budget: self.int("work_budget"),
        }
    }
}

/// Bigram similarity of the case's lines.
struct LineScorer {
    left: Vec<BigramProfile>,
    right: Vec<BigramProfile>,
}

impl PairScorer for LineScorer {
    fn similarity(&self, l: u32, r: u32) -> u16 {
        bigram_similarity(&self.left[l as usize], &self.right[r as usize])
    }
}

/// The canonical golden form: one run per line, compact.
fn render(alignment: &Alignment) -> String {
    let mut out = String::new();
    writeln!(out, "{{").unwrap();
    writeln!(out, "  \"algorithm\": \"{}\",", alignment.algorithm()).unwrap();
    writeln!(out, "  \"rows\": {},", alignment.row_count()).unwrap();
    let warnings: Vec<String> = alignment
        .warnings()
        .iter()
        .map(|w| format!("\"{}\"", w.code))
        .collect();
    writeln!(out, "  \"warnings\": [{}],", warnings.join(", ")).unwrap();
    writeln!(out, "  \"runs\": [").unwrap();
    let runs = alignment.runs();
    for (k, run) in runs.iter().enumerate() {
        let comma = if k + 1 < runs.len() { "," } else { "" };
        writeln!(
            out,
            "    {{\"kind\": \"{}\", \"left\": [{}, {}], \"right\": [{}, {}]}}{comma}",
            run.kind.as_str(),
            run.left.start,
            run.left.end,
            run.right.start,
            run.right.end
        )
        .unwrap();
    }
    writeln!(out, "  ]").unwrap();
    writeln!(out, "}}").unwrap();
    out
}

fn run_case(dir: &Path) -> usize {
    let read = |name: &str| {
        std::fs::read_to_string(dir.join(name))
            .unwrap_or_else(|e| panic!("{}: {e}", dir.join(name).display()))
    };
    let (left_text, right_text) = (read("left.txt"), read("right.txt"));
    let settings = Settings(parse_case(
        &dir.join("case.toml").display().to_string(),
        &read("case.toml"),
    ));
    assert!(
        settings.str("description").is_some(),
        "{}: case.toml needs a description",
        dir.display()
    );
    let left: Vec<&str> = left_text.lines().collect();
    let right: Vec<&str> = right_text.lines().collect();
    let keys = intern_pair(
        left.iter().map(|l| l.as_bytes()),
        right.iter().map(|l| l.as_bytes()),
    )
    .unwrap();
    let mut trivial = TrivialSet::with_capacity(keys.distinct);
    for (line, id) in left
        .iter()
        .zip(&keys.left)
        .chain(right.iter().zip(&keys.right))
    {
        if line.trim().is_empty() {
            trivial.insert(*id);
        }
    }
    let weight_of = |line: &str| match settings.str("weight_prefix") {
        Some(prefix) if line.starts_with(prefix) => settings.int("weight").map_or(4, |w| w as u8),
        _ => 0,
    };
    let (left_weights, right_weights): (Vec<u8>, Vec<u8>) =
        if settings.str("weight_prefix").is_some() {
            (
                left.iter().map(|l| weight_of(l)).collect(),
                right.iter().map(|l| weight_of(l)).collect(),
            )
        } else {
            (Vec::new(), Vec::new())
        };
    let input = AlignInput {
        left: &keys.left,
        right: &keys.right,
        left_weights: &left_weights,
        right_weights: &right_weights,
        trivial: &trivial,
        distinct: keys.distinct,
    };
    let scorer = LineScorer {
        left: left.iter().map(|l| BigramProfile::new(l)).collect(),
        right: right.iter().map(|l| BigramProfile::new(l)).collect(),
    };
    for algorithm in Algorithm::ALL {
        let opts = settings.options(algorithm);
        let alignment = align(&input, &opts, Some(&scorer), None, &CancelToken::never()).unwrap();
        alignment
            .check_invariants(&input, &opts)
            .unwrap_or_else(|v| panic!("{} {algorithm}: {v}", dir.display()));
        assert_eq!(alignment.apply(&keys.left, &keys.right), keys.right);
        assert_golden(
            &dir.join(format!("expected.{algorithm}.json")),
            render(&alignment).as_bytes(),
        );
    }
    Algorithm::ALL.len()
}

#[test]
fn golden_alignments() {
    let root = fixtures("align");
    let mut cases: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    cases.sort();
    let mut goldens = 0;
    for case in &cases {
        goldens += run_case(case);
    }
    assert!(
        cases.len() >= 14,
        "§6.2.3 needs at least 14 cases, found {}",
        cases.len()
    );
    assert_eq!(goldens, cases.len() * 5);
}

#[test]
fn the_case_reader_understands_its_subset() {
    let parsed = parse_case(
        "test",
        "# comment\ndescription = \"a \\\"b\\\"\"\nskew = 1_000\nnever_align = true\n",
    );
    let settings = Settings(parsed);
    assert_eq!(settings.str("description"), Some("a \"b\""));
    assert_eq!(settings.int("skew"), Some(1000));
    assert!(settings.bool("never_align", false));
    assert!(settings.bool("closeness", true));
}

/// Regression (golden reader): a misspelt key is rejected instead of being ignored.
#[test]
#[should_panic(expected = "unknown key \"never_aling\"")]
fn the_case_reader_rejects_unknown_keys() {
    parse_case("test", "description = \"x\"\nnever_aling = true\n");
}

#[test]
#[should_panic(expected = "duplicate key \"skew\"")]
fn the_case_reader_rejects_duplicate_keys() {
    parse_case("test", "skew = 1\nskew = 2\n");
}
