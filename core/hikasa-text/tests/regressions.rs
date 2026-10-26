//! Regression tests of the wave-3 review (adversarial and conformance findings): each test names the finding it
//! pins. Timing-dependent checks (G4 on adversarial inputs) live in `gates.rs`.

mod common;

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::{canonical_core, compare_docs, compare_texts, doc, format};
use hikasa_core::{
    CancelToken, CaseSensitivity, Classify, ContentComparator, ContentResult, ContentSource, Ctl,
    ErrorKind, LocalPath, ReadError, Spec, WarningCode,
};
use hikasa_formats::{
    Conversion, ConvertError, ExternalConversion, FileFormat, FilenameEncoding, FormatId,
    FormatKind, FormatRegistry, Origin, ProcessRunner, RegexLimits, RegistryEntry,
};
use hikasa_text::{
    Algorithm, AlgorithmChoice, CompareInput, CompiledFormat, DocOrigin, Importance, LineIdx,
    LoadOptions, QuickOptions, RowIdx, RowState, RulesComparator, SPAN_LIMIT_BYTES, TextDocument,
    TextPattern, TextRules, UnimportantItem, compare, compare_with_limits, load_document,
    quick_compare_bytes, quick_compare_bytes_with,
};
use hikasa_vfs::{MemMeta, MemoryVfs, StreamOptions};

fn rows(cmp: &hikasa_text::TextComparison) -> Vec<(RowState, Importance)> {
    cmp.rows(0..cmp.row_count())
        .map(|r| (r.state, r.importance))
        .collect()
}

fn quick(left: &str, right: &str, format: &CompiledFormat, rules: &TextRules) -> ContentResult {
    quick_compare_bytes(
        (left.as_bytes(), "l"),
        (right.as_bytes(), "r"),
        format,
        rules,
        &LoadOptions::default(),
        &Ctl::detached(),
    )
    .unwrap()
    .result
}

/// Decision 2 (D-53/INV-KEY-3): with a multi-line grammar the identity path, the general path and a
/// never-matching unimportant pattern give identical results; equal raw lines inside and outside a block
/// comment are Same.
#[test]
fn d53_identity_equals_general_for_multiline_grammars() {
    let rust = format("rust");
    let left = "fn a() {}\nlet x = 1;\nlet y = 2;\nfn b() {}\n";
    let right = "fn a() {}\n/*\nlet x = 1;\nlet y = 2;\n*/\nfn b() {}\n";
    let rules = TextRules::default();
    assert!(rules.is_identity(&rust));
    let fast = compare_docs(doc(left), doc(right), &rust, &rules, false, None);
    let general = compare_docs(doc(left), doc(right), &rust, &rules, true, None);
    let mut never = TextRules::default();
    never
        .importance
        .unimportant
        .push(TextPattern::plain("NEVER_MATCHES"));
    let with_pattern = compare_docs(doc(left), doc(right), &rust, &never, false, None);
    assert!(fast.used_identity_path() && !general.used_identity_path());
    assert!(!with_pattern.used_identity_path());
    let same = (RowState::Same, Importance::None);
    let added = (RowState::RightOnly, Importance::Important);
    let expected = [same, added, same, same, added, same];
    assert_eq!(rows(&fast), expected);
    assert_eq!(canonical_core(&fast), canonical_core(&general));
    assert_eq!(canonical_core(&fast), canonical_core(&with_pattern));
    // The same holds for C under rules that drop nothing (its keys equal the raw lines).
    let c = format("c-cpp");
    let (cl, cr) = (
        "int a;\nint x = 1;\nint b;\n",
        "int a;\n/*\nint x = 1;\n*/\nint b;\n",
    );
    let c_rows = rows(&compare_texts(cl, cr, &c, &rules));
    assert_eq!(c_rows, [same, added, same, added, same]);
}

/// Decision 2: byte-identical lines never show as important, even when their keys differ through the
/// tokenizer state (here a hex number, case-insensitive in C, folded in code but not inside a comment).
#[test]
fn byte_identical_lines_are_never_important() {
    let c = format("c-cpp");
    let left = "a;\n/*\nX = 0xFF;\n";
    let right = "a;\n//\nX = 0xFF;\n";
    let cmp = compare_texts(left, right, &c, &TextRules::default());
    let row = cmp.row(RowIdx(2)).unwrap();
    assert_eq!(
        (row.left, row.right, row.state, row.importance),
        (
            Some(LineIdx(2)),
            Some(LineIdx(2)),
            RowState::Changed,
            Importance::Unimportant
        )
    );
    let inline = cmp.inline(RowIdx(2)).unwrap();
    assert!(inline.left.is_empty() && inline.right.is_empty());
    for k in 0..cmp.row_count() {
        let r = cmp.row(RowIdx(k)).unwrap();
        if let (Some(l), Some(x)) = (r.left, r.right)
            && cmp.document(hikasa_core::Side::Left).line(l)
                == cmp.document(hikasa_core::Side::Right).line(x)
        {
            assert_ne!(r.importance, Importance::Important, "row {k}");
        }
    }
}

/// Decision 2 and TC-016: the quick compare's step-5 shortcut is not taken when raw-identical lines can have
/// different keys (equal line counts, keys that depend on the tokenizer state), so quick still equals
/// `compare().verdict(false)` when every difference is such a pair.
#[test]
fn quick_step_5_guard_keeps_raw_identical_pairs_consistent() {
    let c = format("c-cpp");
    let mut rules = TextRules::default();
    rules
        .importance
        .apply_unimportant(&[UnimportantItem::Element("Comment".into())]);
    rules.alignment.closeness = false;
    let (left, right) = ("/* a\nX = 0xFF; */\n", "// a\nX = 0xFF; */\n");
    for algorithm in AlgorithmChoice::ALL {
        rules.alignment.algorithm = algorithm;
        let cmp = compare_texts(left, right, &c, &rules);
        assert_eq!(
            quick(left, right, &c, &rules),
            cmp.verdict(false),
            "{algorithm}"
        );
    }
    rules.alignment.algorithm = AlgorithmChoice::Unaligned;
    assert_eq!(
        compare_texts(left, right, &c, &rules).verdict(false),
        ContentResult::UnimportantOnly
    );
}

/// A format with a characters-per-line limit of 3.
fn wrapped() -> Arc<CompiledFormat> {
    let mut f = FileFormat::new(FormatId::new("t").unwrap(), "T", FormatKind::Text);
    f.text.chars_per_line_limit = Some(3);
    Arc::new(CompiledFormat::compile(Arc::new(f), &RegexLimits::default()).unwrap())
}

fn wrapped_doc(bytes: &[u8], format: &CompiledFormat) -> Arc<TextDocument> {
    Arc::new(
        TextDocument::from_bytes(bytes, format, &LoadOptions::default(), DocOrigin::default())
            .unwrap(),
    )
}

/// Decision 5 (TC-003): an inserted real line break never compares equal to an artificial wrap, on both key
/// paths and in the quick compare.
#[test]
fn an_inserted_line_break_differs_from_an_artificial_wrap() {
    let f = wrapped();
    for (left, right) in [
        (&b"abcdef\n"[..], &b"abc\ndef\n"[..]),
        (b"abcdef", b"abc\r\ndef"),
    ] {
        for force_tokenize in [false, true] {
            for compare_eol in [false, true] {
                let mut rules = TextRules::default();
                rules.importance.compare_line_endings = compare_eol;
                let cmp = compare_docs(
                    wrapped_doc(left, &f),
                    wrapped_doc(right, &f),
                    &f,
                    &rules,
                    force_tokenize,
                    None,
                );
                assert_eq!(cmp.verdict(false), ContentResult::ImportantDifferences);
                assert_eq!(cmp.row(RowIdx(0)).unwrap().state, RowState::Changed);
                let q = quick_compare_bytes(
                    (left, "l"),
                    (right, "r"),
                    &f,
                    &rules,
                    &LoadOptions::default(),
                    &Ctl::detached(),
                )
                .unwrap();
                assert_eq!(q.result, ContentResult::ImportantDifferences);
            }
        }
    }
    // Wrapped the same way on both sides: Same.
    let cmp = compare_docs(
        wrapped_doc(b"abcdef\n", &f),
        wrapped_doc(b"abcdef\r\n", &f),
        &f,
        &TextRules::default(),
        false,
        None,
    );
    assert_eq!(cmp.verdict(false), ContentResult::RulesSame);
}

/// Decision 6 (TC-014): every inline range of a Changed/Unimportant row is unimportant (blue).
#[test]
fn inline_ranges_of_unimportant_rows_are_blue() {
    let mut rules = TextRules::default();
    rules
        .importance
        .apply_unimportant(&[UnimportantItem::Whitespace, UnimportantItem::Case]);
    for (left, right) in [
        ("Hello World\n", "helloworld\n"),
        ("a B\n", "Ab\n"),
        ("x = Foo(1)\n", "x=foo( 1 )\n"),
        ("ab cd\n", "AbCd\n"),
    ] {
        for f in ["ee", "c-cpp"] {
            let cmp = compare_texts(left, right, &format(f), &rules);
            let row = cmp.row(RowIdx(0)).unwrap();
            if row.importance != Importance::Unimportant {
                continue;
            }
            let inline = cmp.inline(RowIdx(0)).unwrap();
            assert!(!inline.left.is_empty() || !inline.right.is_empty());
            assert!(
                inline
                    .left
                    .iter()
                    .chain(&inline.right)
                    .all(|r| !r.important),
                "{left:?} vs {right:?} ({f}): {inline:?}"
            );
        }
    }
    let cmp = compare_texts("Hello World\n", "helloworld\n", &format("ee"), &rules);
    assert_eq!(
        cmp.row(RowIdx(0)).unwrap().importance,
        Importance::Unimportant
    );
}

/// Repetitive C statements (the adversarial input of D-36): `lines` per side, each drawn from 50 statements.
fn repetitive_c(lines: usize, seed: u64) -> String {
    let mut rng = common::XorShift(seed);
    (0..lines)
        .map(|_| {
            let s = rng.below(50);
            format!("    value_{s} = compute(value_{s}, {s});\n")
        })
        .collect()
}

/// Decision 4 (D-36 as amended): `Auto` uses Histogram for source code only while the stripped middle fits the
/// fallback budget; beyond it, the bounded Standard. Explicit Histogram stays Histogram.
#[test]
fn auto_uses_histogram_only_within_the_fallback_budget() {
    let c = format("c-cpp");
    let rules = TextRules::default();
    let small = compare_texts("int a;\nint b;\n", "int a;\nint c;\n", &c, &rules);
    assert_eq!(small.algorithm(), Algorithm::Histogram);
    // 6,000 lines per side, different from the first to the last line: 36,000,000 cells exceed
    // 128·12,000 + 32,000,000.
    let (left, right) = (repetitive_c(6_000, 1), repetitive_c(6_000, 2));
    let big = compare_texts(&left, &right, &c, &rules);
    assert_eq!(big.algorithm(), Algorithm::Standard);
    let explicit = compare_texts(
        &left[..left.len() / 8],
        &right[..right.len() / 8],
        &c,
        &common::rules_with(AlgorithmChoice::Histogram),
    );
    assert_eq!(explicit.algorithm(), Algorithm::Histogram);
    // Everything Else keeps Standard.
    assert_eq!(
        compare_texts("a\n", "b\n", &format("ee"), &rules).algorithm(),
        Algorithm::Standard
    );
}

/// Decision 3 (closeness): the scorer's work is budgeted in units proportional to the compared text, so two
/// unrelated files with long lines finish with warning `closeness_budget_exhausted`, deterministically.
#[test]
fn closeness_work_is_bounded_by_the_scorer_budget() {
    let mut rng = common::XorShift(42);
    let mut make = || -> String {
        (0..400)
            .map(|_| {
                let mut line: String = (0..600)
                    .map(|_| (b'a' + rng.below(26) as u8) as char)
                    .collect();
                line.push('\n');
                line
            })
            .collect()
    };
    let (left, right) = (make(), make());
    let ee = format("ee");
    let rules = TextRules::default();
    let a = compare_texts(&left, &right, &ee, &rules);
    assert!(
        a.warnings()
            .iter()
            .any(|w| w.code == WarningCode::ClosenessBudgetExhausted),
        "{:?}",
        a.warnings()
    );
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let b = compare_docs(doc(&left), doc(&right), &ee, &rules, false, Some(&pool));
    assert_eq!(common::canonical_json(&a), common::canonical_json(&b));
}

/// Upper-cases `%s` into `%t`.
struct Upper;

impl ProcessRunner for Upper {
    fn run(
        &self,
        argv: &[OsString],
        _timeout: Option<Duration>,
        cancel: &CancelToken,
    ) -> Result<i32, ConvertError> {
        cancel.check()?;
        let source = std::fs::read(&argv[1])?;
        std::fs::write(&argv[2], source.to_ascii_uppercase())?;
        Ok(0)
    }
}

fn upper_format() -> FileFormat {
    let mut f = FileFormat::new(FormatId::new("upper").unwrap(), "Upper", FormatKind::Text);
    f.mask = "*.up".into();
    f.conversion = Conversion::External(ExternalConversion {
        load: "upper %s %t".into(),
        save: None,
        filename_encoding: FilenameEncoding::Unicode,
        timeout_secs: 5,
    });
    f
}

/// An in-memory content source.
struct Src(Vec<u8>, &'static str);

impl ContentSource for Src {
    fn name(&self) -> &str {
        self.1
    }
    fn display(&self) -> String {
        self.1.to_owned()
    }
    fn len(&self) -> Option<u64> {
        Some(self.0.len() as u64)
    }
    fn open(&self) -> std::io::Result<Box<dyn std::io::Read + Send + '_>> {
        Ok(Box::new(std::io::Cursor::new(self.0.clone())))
    }
    fn read_all(&self, _limit: u64, _cancel: &CancelToken) -> Result<Vec<u8>, ReadError> {
        Ok(self.0.clone())
    }
}

/// Decision 7 (TC-016): External conversions run on every path with the same runner, so text compare, the
/// quick compare and `RulesComparator` agree; without a runner every path fails with the same error.
#[test]
fn external_conversions_agree_on_every_path() {
    let temp = tempfile::tempdir().unwrap();
    let fmt = Arc::new(
        CompiledFormat::compile(Arc::new(upper_format()), &RegexLimits::default()).unwrap(),
    );
    let registry = Arc::new(
        FormatRegistry::from_entries(
            vec![RegistryEntry::new(
                Arc::new(upper_format()),
                true,
                Origin::BuiltIn,
            )],
            RegexLimits::default(),
        )
        .unwrap(),
    );
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let spec = |p: &str| Spec::Local(LocalPath::new(PathBuf::from(common::host(p))).unwrap());
    let rules = TextRules::default();
    let runner: Arc<dyn ProcessRunner> = Arc::new(Upper);
    let external: Option<(&std::path::Path, &dyn ProcessRunner)> =
        Some((temp.path(), runner.as_ref()));
    let cases: [(&[u8], &[u8], ContentResult); 3] = [
        (b"hello world\n", b"HELLO world\n", ContentResult::RulesSame),
        (b"same\n", b"same\n", ContentResult::BinarySame),
        (b"a\n", b"b\n", ContentResult::ImportantDifferences),
    ];
    for (k, (a, b, expected)) in cases.into_iter().enumerate() {
        let (pa, pb) = (format!("/w/a{k}.up"), format!("/w/b{k}.up"));
        vfs.insert_file(&common::host(&pa), a.to_vec(), MemMeta::default())
            .unwrap();
        vfs.insert_file(&common::host(&pb), b.to_vec(), MemMeta::default())
            .unwrap();
        let load = |p: &str| {
            load_document(
                &vfs,
                &spec(p),
                &fmt,
                &LoadOptions::default(),
                external,
                &Ctl::detached(),
            )
            .unwrap()
        };
        let ((l, lraw), (r, rraw)) = (load(&pa), load(&pb));
        let text = compare(
            CompareInput {
                left: Arc::new(l),
                right: Arc::new(r),
                format: Arc::clone(&fmt),
                rules: rules.clone(),
                bytes_identical: lraw == rraw,
                force_tokenize: false,
            },
            None,
            &Ctl::detached(),
            None,
        )
        .unwrap()
        .verdict(false);
        let quick = quick_compare_bytes_with(
            (a, "a.up"),
            (b, "b.up"),
            &fmt,
            &rules,
            &LoadOptions::default(),
            &QuickOptions {
                external,
                ..QuickOptions::default()
            },
            &Ctl::detached(),
        )
        .unwrap();
        let folder =
            RulesComparator::new(Arc::clone(&registry), rules.clone(), LoadOptions::default())
                .unwrap()
                .with_external(temp.path().to_path_buf(), Arc::clone(&runner))
                .compare(
                    &Src(a.to_vec(), "a.up"),
                    &Src(b.to_vec(), "b.up"),
                    &Ctl::detached(),
                )
                .unwrap();
        assert_eq!(text, expected, "case {k}");
        assert_eq!(quick.result, expected, "case {k}");
        assert_eq!(folder.result, expected, "case {k}");
    }
    // Without a runner: the error of load_document, on every path.
    let err = load_document(
        &vfs,
        &spec("/w/a0.up"),
        &fmt,
        &LoadOptions::default(),
        None,
        &Ctl::detached(),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    let q = quick_compare_bytes(
        (b"x\n", "a.up"),
        (b"x\n", "b.up"),
        &fmt,
        &rules,
        &LoadOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    assert!(
        matches!(&q.result, ContentResult::Error { error } if error.kind == ErrorKind::Unsupported),
        "{q:?}"
    );
    let folder = RulesComparator::new(Arc::clone(&registry), rules, LoadOptions::default())
        .unwrap()
        .compare(
            &Src(b"x\n".to_vec(), "a.up"),
            &Src(b"y\n".to_vec(), "b.up"),
            &Ctl::detached(),
        )
        .unwrap();
    assert!(
        matches!(&folder.result, ContentResult::Error { error } if error.kind == ErrorKind::Unsupported),
        "{folder:?}"
    );
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}

/// Decision 8 (TC-016): the regex limits of the unimportant patterns are the registry's on every path.
#[test]
fn regex_limits_agree_between_rules_comparator_and_compare() {
    let limits = RegexLimits {
        backtrack_limit: 100,
        ..RegexLimits::default()
    };
    let registry = Arc::new(FormatRegistry::builtin_with_limits(limits).unwrap());
    let mut rules = TextRules::default();
    // A backreference needs fancy-regex (§2.11), the engine with a runtime limit.
    rules
        .importance
        .unimportant
        .push(TextPattern::regex(r"(Q+)\1"));
    let (a, b) = (format!("a{}\n", "Q".repeat(200)), "a\n".to_owned());
    let folder = RulesComparator::new(Arc::clone(&registry), rules.clone(), LoadOptions::default())
        .unwrap()
        .compare(
            &Src(a.clone().into_bytes(), "a.txt"),
            &Src(b.clone().into_bytes(), "b.txt"),
            &Ctl::detached(),
        )
        .unwrap();
    let ee = Arc::clone(registry.everything_else());
    let text = compare_with_limits(
        CompareInput {
            left: doc(&a),
            right: doc(&b),
            format: Arc::clone(&ee),
            rules: rules.clone(),
            bytes_identical: false,
            force_tokenize: false,
        },
        registry.limits(),
        None,
        &Ctl::detached(),
        None,
    )
    .unwrap();
    let quick = quick_compare_bytes_with(
        (a.as_bytes(), "a.txt"),
        (b.as_bytes(), "b.txt"),
        &ee,
        &rules,
        &LoadOptions::default(),
        &QuickOptions {
            limits,
            external: None,
        },
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!(folder.result, text.verdict(false));
    assert_eq!(quick.result, text.verdict(false));
    assert!(
        text.warnings()
            .iter()
            .any(|w| w.code == WarningCode::RegexRuntimeError)
    );
}

/// Conformance finding: `lossy_decode` is reported once per side when the quick compare runs step 6.
#[test]
fn lossy_decode_is_reported_once_per_side() {
    let mut rules = TextRules::default();
    rules.importance.orphans_always_important = false;
    // Mostly valid UTF-8 with one stray byte (D-42: decoded as lossy UTF-8).
    let mut left = "é\n".repeat(10).into_bytes();
    left.extend_from_slice(b"x\xFF");
    let right = "é\n".repeat(9).into_bytes();
    let q = quick_compare_bytes(
        (&left, "l.txt"),
        (&right, "r.txt"),
        &format("ee"),
        &rules,
        &LoadOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    let lossy: Vec<&str> = q
        .warnings
        .iter()
        .filter(|w| w.code == WarningCode::LossyDecode)
        .map(|w| w.message.as_str())
        .collect();
    assert_eq!(lossy.len(), 1, "{lossy:?}");
    assert!(lossy[0].starts_with("l.txt:"), "{lossy:?}");
}

/// Conformance finding (D-15): a right-side format that fails to compile does not fail the pair; the left
/// format is used and a warning says why the right side was not detected.
#[test]
fn a_right_format_that_fails_to_compile_does_not_fail_the_pair() {
    let toml = "schema_version = 1\nid = \"bad\"\nname = \"Bad\"\nkind = \"text\"\nmask = \"*.bad\"\n\n\
                [[grammar]]\nelement = \"Word\"\nkind = \"basic\"\ntext = 'a{1,100}b{1,100}c{1,100}'\nregex = true\n";
    let (bad, _) = hikasa_formats::parse_format(toml, "bad.toml").unwrap();
    let tiny = RegexLimits {
        size_limit: 64,
        ..RegexLimits::default()
    };
    let registry = Arc::new(
        FormatRegistry::from_entries(
            vec![RegistryEntry::new(Arc::new(bad), true, Origin::BuiltIn)],
            tiny,
        )
        .unwrap(),
    );
    assert!(
        registry
            .detect("b.bad", &hikasa_formats::EnabledOverrides::default())
            .is_err()
    );
    let out = RulesComparator::new(registry, TextRules::default(), LoadOptions::default())
        .unwrap()
        .compare(
            &Src(b"x\n".to_vec(), "a.txt"),
            &Src(b"y\n".to_vec(), "b.bad"),
            &Ctl::detached(),
        )
        .unwrap();
    assert_eq!(out.result, ContentResult::ImportantDifferences);
    assert!(
        out.warnings
            .iter()
            .any(|w| w.code == WarningCode::FormatMismatch && w.message.contains("b.bad")),
        "{:?}",
        out.warnings
    );
}

/// Conformance finding: the byte comparisons of `RulesComparator` take the session's stream options.
#[test]
fn rules_comparator_byte_comparisons_take_stream_options() {
    let rc = RulesComparator::new(
        Arc::new(FormatRegistry::builtin().unwrap()),
        TextRules::default(),
        LoadOptions::default(),
    )
    .unwrap()
    .with_max_bytes(4)
    .with_stream_options(StreamOptions {
        buffer_size: 4096,
        threads: 2,
        ..StreamOptions::default()
    });
    let out = rc
        .compare(
            &Src(b"0123456789".to_vec(), "a.txt"),
            &Src(b"0123456789".to_vec(), "b.txt"),
            &Ctl::detached(),
        )
        .unwrap();
    assert_eq!(out.result, ContentResult::BinarySame);
    assert!(
        out.warnings
            .iter()
            .any(|w| w.code == WarningCode::RulesSizeLimit)
    );
}

/// Decision 8: `inline()` and `spans()` stay bounded on very long lines; the whole-line ranges equal
/// `inline_diff`'s.
#[test]
fn long_lines_inline_and_spans_are_bounded() {
    // Just over the byte threshold of the direct whole-line scan: identical to inline_diff.
    for (a, b) in [
        (
            format!("{}x{}", "é".repeat(10_001), "z".repeat(20_000)),
            format!("{}y{}", "é".repeat(10_001), "z".repeat(20_000)),
        ),
        ("a".repeat(40_001), "a".repeat(40_002)),
        (
            format!("{}é", "q".repeat(40_001)),
            format!("{}è", "q".repeat(40_001)),
        ),
        ("ab".repeat(20_001), "ba".repeat(20_001)),
    ] {
        let cmp = compare_texts(
            &format!("{a}\n"),
            &format!("{b}\n"),
            &format("ee"),
            &common::rules_with(AlgorithmChoice::Unaligned),
        );
        let inline = cmp.inline(RowIdx(0)).unwrap();
        let reference = hikasa_diff::inline_diff(&a, &b, &hikasa_diff::InlineOptions::DEFAULT);
        assert!(inline.whole_line && reference.whole_line);
        let ranges = |v: &[hikasa_text::InlineRange]| -> Vec<std::ops::Range<u32>> {
            v.iter().map(|r| r.range.clone()).collect()
        };
        assert_eq!(ranges(&inline.left), reference.left);
        assert_eq!(ranges(&inline.right), reference.right);
    }
    // A 2 MiB line with an unimportant pattern every other char: the spans cover the first SPAN_LIMIT_BYTES,
    // then one tail span.
    let mut rules = TextRules::default();
    rules.importance.unimportant.push(TextPattern::plain("x"));
    let line: String = "yx".repeat(1 << 20);
    let other: String = format!("{}z", &line[..line.len() - 1]);
    let cmp = compare_texts(
        &format!("{line}\n"),
        &format!("{other}\n"),
        &format("ee"),
        &rules,
    );
    let spans = cmp.spans(hikasa_core::Side::Left, LineIdx(0));
    assert!(spans.len() <= SPAN_LIMIT_BYTES + 1, "{}", spans.len());
    let tail = spans.last().unwrap();
    assert_eq!(tail.range, SPAN_LIMIT_BYTES as u32..line.len() as u32);
    assert!(tail.important);
    let inline = cmp.inline(RowIdx(0)).unwrap();
    assert!(inline.whole_line);
    assert_eq!(inline.left.len(), 1);
}
