//! Properties of the comparison (§6.2.5): P-TC-1..8, over random documents with whitespace, case, comment and
//! line-ending variants, under random rules, for every algorithm.

mod common;

use std::sync::{Arc, OnceLock};

use common::{Recorder, canonical_core, canonical_json, compare_docs, config, doc, format};
use hikasa_core::{ContentResult, Ctl};
use hikasa_text::{
    AlgorithmChoice, CompareInput, CompiledFormat, Importance, LoadOptions, RowIdx, RowState,
    TextDisplayFilter, TextDocument, TextPattern, TextRules, TextToggles, ThumbCell, ViewItem,
    ViewOptions, compare, quick_compare_bytes,
};
use proptest::prelude::*;

/// Line fragments: identifiers, keywords, case variants, numbers, comments (one-line and block pieces),
/// strings, an Id keyword pattern and blanks.
const FRAGMENTS: &[&str] = &[
    "int", "x", "X", "y", "=", "1", "0xFF", "0xff", "// c", "// C", "/* a", "b */", "\"s\"",
    "SELECT", "select", "TODO", "$Id: 1 $", "foo", "Foo", "é", "e\u{301}", "#", "", "<!-- x",
    "-->",
];
const SEPARATORS: &[&str] = &["", " ", "  ", "\t"];
const EOLS: &[&str] = &["\n", "\n", "\n", "\r\n", "\r"];

/// A random text: up to `max_lines` lines of fragments with random separators, indentation and endings (the
/// last line may lack one).
fn text(max_lines: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(
        (
            0usize..3,
            prop::collection::vec((0..FRAGMENTS.len(), 0..SEPARATORS.len()), 0..5),
            0..EOLS.len(),
            0usize..2,
        ),
        0..max_lines,
    )
    .prop_flat_map(|lines| (Just(lines), any::<bool>()))
    .prop_map(|(lines, final_eol)| {
        let mut out = String::new();
        let n = lines.len();
        for (k, (indent, parts, eol, trailing)) in lines.into_iter().enumerate() {
            out.push_str(&" ".repeat(indent * 2));
            for (f, s) in parts {
                out.push_str(FRAGMENTS[f]);
                out.push_str(SEPARATORS[s]);
            }
            out.push_str(&" ".repeat(trailing));
            if k + 1 < n || final_eol {
                out.push_str(EOLS[eol]);
            }
        }
        out
    })
}

/// A pair of related texts: the right side is the left side with random line edits.
fn text_pair() -> impl Strategy<Value = (String, String)> {
    (text(24), text(6), 0usize..4, 0usize..24).prop_map(|(left, extra, mode, at)| {
        let lines: Vec<&str> = left.split_inclusive('\n').collect();
        let at = at.min(lines.len());
        let right = match mode {
            0 => left.clone(),
            1 => {
                let mut v: Vec<&str> = lines.clone();
                v.insert(at, &extra);
                v.concat()
            }
            2 => {
                let mut v: Vec<&str> = lines.clone();
                if at < v.len() {
                    v.remove(at);
                }
                v.concat()
            }
            _ => extra.clone() + &left,
        };
        (left, right)
    })
}

/// A grammar without multi-line items (keywords, numbers, a line comment, strings that stop at the end of
/// the line), for P-TC-7.
fn single_line_grammar() -> Arc<CompiledFormat> {
    static FORMAT: OnceLock<Arc<CompiledFormat>> = OnceLock::new();
    Arc::clone(FORMAT.get_or_init(|| {
        let text = r#"
schema_version = 1
id = "single-line"
name = "Single-line grammar"
kind = "text"

[[grammar]]
element = "Comment"
kind = "delimited"
from = "//"

[[grammar]]
element = "String"
kind = "delimited"
from = '"'
to = '"'
stop_at_eol = true

[[grammar]]
element = "Keyword"
kind = "list"
tokens = ["int", "SELECT", "select"]

[[grammar]]
element = "Number"
kind = "basic"
text = '0x[0-9A-Fa-f]+|\d+'
regex = true
"#;
        let (f, warnings) = hikasa_formats::parse_format(text, "test").unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let compiled =
            CompiledFormat::compile(Arc::new(f), &hikasa_formats::RegexLimits::default()).unwrap();
        assert!(!compiled.grammar().has_multiline());
        Arc::new(compiled)
    }))
}

fn formats() -> &'static [Arc<CompiledFormat>] {
    static FORMATS: OnceLock<Vec<Arc<CompiledFormat>>> = OnceLock::new();
    FORMATS.get_or_init(|| {
        ["ee", "c-cpp", "python", "sql", "rust"]
            .iter()
            .map(|id| format(id))
            .collect()
    })
}

/// Random rules (every field), with valid values.
fn rules() -> impl Strategy<Value = TextRules> {
    (
        prop::array::uniform6(any::<bool>()),
        prop::array::uniform4(any::<bool>()),
        0..AlgorithmChoice::ALL.len(),
        0u16..=1000,
        0u32..6,
        0usize..4,
    )
        .prop_map(|(i, a, algorithm, threshold, skew, patterns)| {
            let mut r = TextRules::default();
            r.importance.leading_whitespace = i[0];
            r.importance.embedded_whitespace = i[1];
            r.importance.trailing_whitespace = i[2];
            r.importance.everything_else = i[3];
            r.importance.everything_else_case_sensitive = i[4];
            if i[5] {
                r.importance.elements.insert("Comment".into(), false);
            }
            r.importance.orphans_always_important = a[0];
            r.importance.compare_line_endings = a[1];
            r.alignment.closeness = a[2];
            r.alignment.never_align_differences = a[3];
            r.alignment.algorithm = AlgorithmChoice::ALL[algorithm];
            r.alignment.closeness_threshold_permille = threshold;
            r.alignment.skew_tolerance = skew;
            match patterns {
                1 => r.importance.unimportant.push(TextPattern::plain("TODO")),
                2 => r
                    .importance
                    .unimportant
                    .push(TextPattern::regex(r"\$Id:[^$]*\$")),
                3 => r.importance.unimportant.push(TextPattern {
                    pattern: "foo".into(),
                    regex: false,
                    match_case: false,
                }),
                _ => {}
            }
            r
        })
}

fn input(
    left: &str,
    right: &str,
    format: &Arc<CompiledFormat>,
    rules: &TextRules,
    force_tokenize: bool,
) -> CompareInput {
    CompareInput {
        left: doc(left),
        right: doc(right),
        format: Arc::clone(format),
        rules: rules.clone(),
        bytes_identical: left == right,
        force_tokenize,
    }
}

/// The per-row visibility of §2.18 (brute force).
fn brute_visible(cmp: &hikasa_text::TextComparison, opts: &ViewOptions) -> Vec<bool> {
    let n = cmp.row_count() as usize;
    let cat = |k: usize| {
        let row = cmp.row(RowIdx(k as u32)).unwrap();
        if row.state == RowState::Same
            || (opts.ignore_unimportant && row.importance == Importance::Unimportant)
        {
            TextToggles::SAME
        } else {
            match row.state {
                RowState::LeftOnly => TextToggles::LEFT_ORPHANS,
                RowState::RightOnly => TextToggles::RIGHT_ORPHANS,
                _ => TextToggles::DIFFERENCES,
            }
        }
    };
    (0..n)
        .map(|k| {
            let base = match opts.filter {
                TextDisplayFilter::All => true,
                TextDisplayFilter::None => false,
                TextDisplayFilter::Same => cat(k) == TextToggles::SAME,
                TextDisplayFilter::Differences => cat(k) != TextToggles::SAME,
                TextDisplayFilter::Context { lines } => (0..n).any(|d| {
                    cat(d) != TextToggles::SAME
                        && (k as i64 - d as i64).unsigned_abs() <= u64::from(lines)
                }),
                TextDisplayFilter::Toggles { bits } => {
                    TextToggles::from_bits_truncate(bits).contains(cat(k))
                }
            };
            base || opts
                .expanded
                .iter()
                .any(|r| (r.start as usize) <= k && k < r.end as usize)
        })
        .collect()
}

fn filters() -> impl Strategy<Value = ViewOptions> {
    (
        0usize..6,
        0u32..4,
        0u8..16,
        any::<bool>(),
        prop::collection::vec((0u32..40, 0u32..8), 0..3),
    )
        .prop_map(|(f, lines, bits, iu, expanded)| ViewOptions {
            filter: match f {
                0 => TextDisplayFilter::All,
                1 => TextDisplayFilter::Differences,
                2 => TextDisplayFilter::Same,
                3 => TextDisplayFilter::None,
                4 => TextDisplayFilter::Context { lines },
                _ => TextDisplayFilter::Toggles { bits },
            },
            ignore_unimportant: iu,
            expanded: expanded.into_iter().map(|(s, l)| s..s + l).collect(),
        })
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_compare.txt"))]

    /// P-TC-1: the invariants hold on random documents under random rules (and P-TC-5: the statistics
    /// identities; P-TC-6: ignore-unimportant only makes the verdict "more same").
    #[test]
    fn p_tc_1_5_6_invariants_statistics_and_monotonicity(
        (left, right) in text_pair(),
        f in 0usize..5,
        rules in rules(),
    ) {
        let cmp = compare_docs(doc(&left), doc(&right), &formats()[f], &rules, false, None);
        cmp.check_invariants().unwrap();
        let s = cmp.stats();
        prop_assert!(s.identities_hold(), "{s:?}");
        prop_assert_eq!(s.rows, u64::from(cmp.row_count()));
        prop_assert_eq!(s.sections, cmp.sections().len() as u64);
        prop_assert!(cmp.verdict(true).kind() <= cmp.verdict(false).kind());
        if cmp.verdict(false) == ContentResult::UnimportantOnly {
            prop_assert_eq!(cmp.verdict(true), ContentResult::RulesSame);
        }
        // Spans cover every line exactly; inline ranges are sorted, disjoint and on char boundaries.
        for side in [hikasa_core::Side::Left, hikasa_core::Side::Right] {
            let d = cmp.document(side);
            for line in 0..d.line_count() {
                let text = d.line(hikasa_text::LineIdx(line));
                let spans = cmp.spans(side, hikasa_text::LineIdx(line));
                let mut at = 0;
                for s in &spans {
                    prop_assert_eq!(s.range.start, at);
                    prop_assert!(s.range.end > s.range.start);
                    at = s.range.end;
                }
                prop_assert_eq!(at as usize, text.len());
            }
        }
        for k in 0..cmp.row_count() {
            if let Some(inline) = cmp.inline(RowIdx(k)) {
                let row = cmp.row(RowIdx(k)).unwrap();
                for (ranges, side, line) in [
                    (&inline.left, hikasa_core::Side::Left, row.left.unwrap()),
                    (&inline.right, hikasa_core::Side::Right, row.right.unwrap()),
                ] {
                    let text = cmp.document(side).line(line);
                    let mut end = 0;
                    for r in ranges {
                        prop_assert!(r.range.start >= end && r.range.start < r.range.end);
                        prop_assert!(text.is_char_boundary(r.range.start as usize));
                        prop_assert!(text.is_char_boundary(r.range.end as usize));
                        end = r.range.end;
                    }
                }
            }
        }
    }

    /// P-TC-2: the consistency theorem, `quick_compare_bytes == compare().verdict(false)`, for every algorithm
    /// and both values of `orphans_always_important` (the latter covers the step-5 shortcut).
    #[test]
    fn p_tc_2_quick_compare_is_consistent(
        (left, right) in text_pair(),
        f in 0usize..5,
        base in rules(),
    ) {
        let format = &formats()[f];
        for algorithm in AlgorithmChoice::ALL {
            for orphans in [false, true] {
                let mut rules = base.clone();
                rules.alignment.algorithm = algorithm;
                rules.importance.orphans_always_important = orphans;
                let cmp = compare(input(&left, &right, format, &rules, false), None, &Ctl::detached(), None)
                    .unwrap();
                let quick = quick_compare_bytes(
                    (left.as_bytes(), "l"),
                    (right.as_bytes(), "r"),
                    format,
                    &rules,
                    &LoadOptions::default(),
                    &Ctl::detached(),
                )
                .unwrap();
                prop_assert_eq!(&quick.result, &cmp.verdict(false), "{} orphans={}", algorithm, orphans);
            }
        }
    }

    /// P-TC-3: pool None gives the same result as a pool of 8 threads (and of 1).
    #[test]
    fn p_tc_3_parallel_equals_sequential(
        (left, right) in text_pair(),
        f in 0usize..5,
        rules in rules(),
    ) {
        static POOLS: OnceLock<(rayon::ThreadPool, rayon::ThreadPool)> = OnceLock::new();
        let (one, eight) = POOLS.get_or_init(|| {
            (
                rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap(),
                rayon::ThreadPoolBuilder::new().num_threads(8).build().unwrap(),
            )
        });
        let format = &formats()[f];
        let seq = canonical_json(&compare_docs(doc(&left), doc(&right), format, &rules, false, None));
        for pool in [one, eight] {
            let par = canonical_json(&compare_docs(doc(&left), doc(&right), format, &rules, false, Some(pool)));
            prop_assert_eq!(&par, &seq);
        }
    }

    /// P-TC-4: view invariants, and every filter's visibility (Context included) equals the brute-force
    /// definition of §2.18.
    #[test]
    fn p_tc_4_views(
        (left, right) in text_pair(),
        rules in rules(),
        opts in filters(),
    ) {
        let cmp = compare_docs(doc(&left), doc(&right), &formats()[0], &rules, false, None);
        let view = cmp.view(&opts);
        view.check_invariants().unwrap();
        let brute = brute_visible(&cmp, &opts);
        let mut visible = vec![false; brute.len()];
        let mut previous_gap = false;
        for (pos, item) in view.iter().enumerate() {
            prop_assert_eq!(view.get(pos as u32), Some(item.clone()));
            match item {
                ViewItem::Row { row } => {
                    visible[row.0 as usize] = true;
                    prop_assert_eq!(view.position_of(row), Ok(pos as u32));
                    previous_gap = false;
                }
                ViewItem::Gap { rows } => {
                    prop_assert!(!previous_gap, "adjacent gaps");
                    previous_gap = true;
                    for r in rows {
                        prop_assert_eq!(view.position_of(RowIdx(r)), Err(pos as u32));
                    }
                }
            }
        }
        prop_assert_eq!(&visible, &brute);
        prop_assert_eq!(
            view.hidden_rows(),
            brute.iter().filter(|v| !**v).count() as u64
        );
        // Thumbnail cells agree with a per-row recount (the classes of §2.18 `cat`, plus the importance flag of
        // orphans; buckets that cover no row are empty).
        let rows = cmp.row_count();
        for buckets in [1u32, 3, 7, 50] {
            let cells = cmp.thumbnail(buckets, opts.ignore_unimportant);
            prop_assert_eq!(cells.len(), buckets as usize);
            for (b, cell) in cells.iter().enumerate() {
                let s = (b as u64 * u64::from(rows) / u64::from(buckets)) as u32;
                let e = ((b as u64 + 1) * u64::from(rows) / u64::from(buckets)) as u32;
                let mut expected = ThumbCell::empty();
                for k in s..e {
                    let row = cmp.row(RowIdx(k)).unwrap();
                    expected |= if row.state == RowState::Same
                        || (opts.ignore_unimportant && row.importance == Importance::Unimportant)
                    {
                        ThumbCell::SAME
                    } else {
                        let class = if row.importance == Importance::Important {
                            ThumbCell::IMPORTANT
                        } else {
                            ThumbCell::UNIMPORTANT
                        };
                        class
                            | match row.state {
                                RowState::LeftOnly => ThumbCell::LEFT_ONLY,
                                RowState::RightOnly => ThumbCell::RIGHT_ONLY,
                                _ => ThumbCell::empty(),
                            }
                    };
                }
                prop_assert_eq!(*cell, expected, "bucket {} of {}", b, buckets);
            }
        }
    }

    /// P-TC-7: the identity fast path equals the general path (`force_tokenize`) under rules for which
    /// `is_identity` holds (D-53), for grammars with and without multi-line items (Rust and XML block comments),
    /// and so does the general path forced by a never-matching unimportant pattern (INV-KEY-3).
    #[test]
    fn p_tc_7_identity_equals_general(
        (left, right) in text_pair(),
        compare_eol in any::<bool>(),
        algorithm in 0..AlgorithmChoice::ALL.len(),
        closeness in any::<bool>(),
        grammar in 0usize..4,
    ) {
        let mut rules = TextRules::default();
        rules.importance.compare_line_endings = compare_eol;
        rules.alignment.algorithm = AlgorithmChoice::ALL[algorithm];
        rules.alignment.closeness = closeness;
        let format = &match grammar {
            0 => Arc::clone(&formats()[0]),
            1 => single_line_grammar(),
            2 => format("rust"),
            _ => format("xml"),
        };
        prop_assert!(rules.is_identity(format));
        let fast = compare_docs(doc(&left), doc(&right), format, &rules, false, None);
        let general = compare_docs(doc(&left), doc(&right), format, &rules, true, None);
        prop_assert!(fast.used_identity_path());
        prop_assert!(!general.used_identity_path());
        prop_assert_eq!(canonical_core(&fast), canonical_core(&general));
        let mut never = rules.clone();
        never.importance.unimportant.push(TextPattern::plain("§never§"));
        let patterned = compare_docs(doc(&left), doc(&right), format, &never, false, None);
        prop_assert!(!patterned.used_identity_path());
        prop_assert_eq!(canonical_core(&fast), canonical_core(&patterned));
    }

    /// P-TC-8: the runs passed to `CompareObserver::partial`, concatenated, form a prefix of (here: equal) the
    /// final runs, the confirmed line counts never decrease, the phases come in order, and the observer never
    /// changes the result.
    #[test]
    fn p_tc_8_observer(
        (left, right) in text_pair(),
        f in 0usize..5,
        rules in rules(),
    ) {
        let format = &formats()[f];
        let mut recorder = Recorder::default();
        let with = compare(input(&left, &right, format, &rules, false), None, &Ctl::detached(), Some(&mut recorder))
            .unwrap();
        let without = compare(input(&left, &right, format, &rules, false), None, &Ctl::detached(), None).unwrap();
        prop_assert_eq!(canonical_json(&with), canonical_json(&without));
        let seen = recorder.0;
        prop_assert!(seen.partial.len() <= with.runs().len());
        prop_assert_eq!(&seen.partial[..], &with.runs()[..seen.partial.len()]);
        prop_assert_eq!(seen.partial.len(), with.runs().len());
        prop_assert!(seen.done.windows(2).all(|w| w[0].0 <= w[1].0 && w[0].1 <= w[1].1));
        prop_assert_eq!(
            seen.phases,
            vec![hikasa_core::Phase::Tokenizing, hikasa_core::Phase::Aligning, hikasa_core::Phase::Classifying]
        );
    }
}

/// Regression guard for the generators: documents with every kind of ending load and compare.
#[test]
fn generator_smoke() {
    let left = "a\r\nb\rc\n  d  ";
    let right = "a\nb\nc\nd";
    let cmp = compare_docs(
        Arc::new(TextDocument::from_text(left)),
        doc(right),
        &formats()[1],
        &TextRules::default(),
        false,
        None,
    );
    assert!(cmp.row_count() >= 4);
    cmp.check_invariants().unwrap();
}
