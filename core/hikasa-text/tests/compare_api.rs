//! The `TextComparison` API on real comparisons: navigation (TC-050), Go To line, spans and their lazy start
//! states (identity path with a multi-line grammar), inline availability, thumbnail and views, progress and
//! phases, cancellation, and every warning the engine emits (binary content, lossy decode, regex runtime
//! limit, closeness ignored, alignment degraded).

mod common;

use std::sync::Arc;

use common::{ProgressLog, XorShift, compare_docs, compare_texts, doc, format, rules_with};
use hikasa_core::{CancelToken, ContentResult, Ctl, Phase, Side, Unit, WarningCode};
use hikasa_text::{
    AlgorithmChoice, CompareInput, Direction, Importance, LineIdx, RowIdx, RowState, SectionId,
    SectionKind, TextDisplayFilter, TextDocument, TextError, TextPattern, TextRules, ThumbCell,
    ViewOptions, compare,
};

const LEFT: &str = "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\n";
const RIGHT: &str = "alpha\nBeta\ngamma\ndelta\nx-ray\nyankee\nepsilon\nzeta\ntheta\n";

#[test]
fn navigation_and_go_to_line() {
    let cmp = compare_texts(LEFT, RIGHT, &format("ee"), &TextRules::default());
    // Rows: a, b/B, c, d, +x, +y, e, f, -g, h.
    let states: Vec<RowState> = cmp.rows(0..100).map(|r| r.state).collect();
    assert_eq!(
        states,
        [
            RowState::Same,
            RowState::Changed,
            RowState::Same,
            RowState::Same,
            RowState::RightOnly,
            RowState::RightOnly,
            RowState::Same,
            RowState::Same,
            RowState::LeftOnly,
            RowState::Same,
        ]
    );
    let kinds: Vec<SectionKind> = cmp.sections().iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [SectionKind::Change, SectionKind::Add, SectionKind::Delete]
    );
    assert_eq!(cmp.section_at(RowIdx(1)), Some(SectionId(0)));
    assert_eq!(cmp.section_at(RowIdx(5)), Some(SectionId(1)));
    assert_eq!(cmp.section_at(RowIdx(6)), None);
    assert_eq!(cmp.section_at(RowIdx(99)), None);
    assert_eq!(
        cmp.next_section(RowIdx(0), Direction::Forward, false),
        Some(SectionId(0))
    );
    assert_eq!(
        cmp.next_section(RowIdx(1), Direction::Forward, false),
        Some(SectionId(1))
    );
    assert_eq!(cmp.next_section(RowIdx(8), Direction::Forward, false), None);
    assert_eq!(
        cmp.next_section(RowIdx(9), Direction::Backward, false),
        Some(SectionId(2))
    );
    assert_eq!(
        cmp.next_section(RowIdx(5), Direction::Backward, false),
        Some(SectionId(1))
    );
    assert_eq!(
        cmp.next_section(RowIdx(1), Direction::Backward, false),
        None
    );
    assert_eq!(cmp.row_for_line(Side::Left, LineIdx(6)), Some(RowIdx(8)));
    assert_eq!(cmp.row_for_line(Side::Right, LineIdx(5)), Some(RowIdx(5)));
    assert_eq!(cmp.row_for_line(Side::Right, LineIdx(8)), Some(RowIdx(9)));
    assert_eq!(cmp.row_for_line(Side::Right, LineIdx(9)), None);
    assert_eq!(cmp.row_for_line(Side::Center, LineIdx(0)), Some(RowIdx(0)));
    assert!(Arc::ptr_eq(
        cmp.document(Side::Output),
        cmp.document(Side::Left)
    ));
    // Inline: Changed rows only.
    assert!(cmp.inline(RowIdx(1)).is_some());
    assert!(cmp.inline(RowIdx(0)).is_none());
    assert!(cmp.inline(RowIdx(4)).is_none());
    assert!(cmp.inline(RowIdx(99)).is_none());
    assert_eq!(cmp.row(RowIdx(10)), None);
    assert_eq!(cmp.rows(8..100).count(), 2);
}

#[test]
fn ignore_unimportant_navigation_thumbnail_and_view() {
    let mut rules = TextRules::default();
    rules.importance.everything_else_case_sensitive = false;
    let cmp = compare_texts(LEFT, RIGHT, &format("ee"), &rules);
    // b/B is now an unimportant change.
    assert_eq!(
        cmp.row(RowIdx(1)).unwrap().importance,
        Importance::Unimportant
    );
    assert_eq!(cmp.sections()[0].importance, Importance::Unimportant);
    assert_eq!(
        cmp.next_section(RowIdx(0), Direction::Forward, true),
        Some(SectionId(1))
    );
    assert_eq!(cmp.verdict(false), ContentResult::ImportantDifferences);
    let cells = cmp.thumbnail(10, false);
    assert_eq!(cells[1], ThumbCell::UNIMPORTANT);
    assert_eq!(cells[4], ThumbCell::IMPORTANT | ThumbCell::RIGHT_ONLY);
    assert_eq!(cells[8], ThumbCell::IMPORTANT | ThumbCell::LEFT_ONLY);
    assert_eq!(cmp.thumbnail(10, true)[1], ThumbCell::SAME);
    let view = cmp.view(&ViewOptions {
        filter: TextDisplayFilter::Differences,
        ignore_unimportant: true,
        expanded: vec![],
    });
    assert_eq!(view.hidden_rows(), 7);
    assert_eq!(view.len(), 6);
}

#[test]
fn spans_on_the_identity_path_compute_start_states_lazily() {
    let rust = format("rust");
    assert!(TextRules::default().is_identity(&rust));
    let left = "/* open\nstill comment */ fn x() {}\nlet y = 1;\n";
    let right = "/* open\nstill comment */ fn x() {}\nlet y = 2;\n";
    let cmp = compare_texts(left, right, &rust, &TextRules::default());
    assert!(cmp.used_identity_path());
    // A cancelled preparation caches nothing; the next call computes the states.
    let cancelled = CancelToken::new();
    cancelled.cancel();
    assert!(cmp.prepare_spans(Side::Left, &cancelled).is_err());
    let spans = cmp.spans(Side::Left, LineIdx(1));
    let elements = cmp.format().grammar().elements();
    assert_eq!(elements.name(spans[0].element), "Comment");
    assert_eq!(spans[0].range, 0..16);
    assert!(
        cmp.prepare_spans(Side::Right, &CancelToken::never())
            .is_ok()
    );
    assert!(cmp.spans(Side::Right, LineIdx(9)).is_empty());
    // The general path stores the states while tokenising.
    let general = compare_docs(
        doc(left),
        doc(right),
        &rust,
        &TextRules::default(),
        true,
        None,
    );
    assert!(!general.used_identity_path());
    assert_eq!(general.spans(Side::Left, LineIdx(1)), spans);
}

#[test]
fn phases_progress_and_cancellation() {
    let log = ProgressLog::default();
    let cancel = CancelToken::new();
    let ctl = Ctl {
        cancel: &cancel,
        progress: &log,
    };
    let input = CompareInput {
        left: doc(LEFT),
        right: doc(RIGHT),
        format: format("c-cpp"),
        rules: TextRules::default(),
        bytes_identical: false,
        force_tokenize: false,
    };
    compare(input.clone(), None, &ctl, None).unwrap();
    let reports = log.0.lock().unwrap().clone();
    for phase in [Phase::Tokenizing, Phase::Aligning, Phase::Classifying] {
        let done: Vec<u64> = reports
            .iter()
            .filter(|p| p.phase == phase)
            .map(|p| p.done)
            .collect();
        assert!(done.len() >= 2, "{phase:?}");
        assert!(done.windows(2).all(|w| w[0] <= w[1]), "{phase:?}: {done:?}");
        let last = reports.iter().rev().find(|p| p.phase == phase).unwrap();
        assert_eq!(Some(last.done), last.total, "{phase:?} completes");
    }
    assert!(
        reports
            .iter()
            .any(|p| p.phase == Phase::Tokenizing && p.unit == Unit::Lines)
    );
    cancel.cancel();
    let err = compare(input, None, &ctl, None).unwrap_err();
    assert!(matches!(err, TextError::Cancelled));
}

#[test]
fn warnings_binary_and_lossy() {
    let ee = format("ee");
    let load = hikasa_text::LoadOptions::default();
    let bin = |b: &[u8]| {
        Arc::new(
            TextDocument::from_bytes(b, &ee, &load, hikasa_text::DocOrigin::default()).unwrap(),
        )
    };
    let cmp = compare_docs(
        bin(b"\x89PNG\r\n\x1a\nabc\x00y\n"),
        bin(b"\x89PNG\r\n\x1a\nabc\x00z\n"),
        &ee,
        &TextRules::default(),
        false,
        None,
    );
    assert_eq!(cmp.verdict(false), ContentResult::BinaryDifferent);
    assert_eq!(cmp.verdict(true), ContentResult::BinaryDifferent);
    let codes: Vec<WarningCode> = cmp.warnings().iter().map(|w| w.code).collect();
    assert_eq!(
        codes,
        [WarningCode::BinaryContent, WarningCode::BinaryContent]
    );
    let mut lossy = "héllo wörld ".repeat(10).into_bytes();
    lossy.push(0xFF);
    let cmp = compare_docs(
        bin(&lossy),
        bin(b"plain\n"),
        &ee,
        &TextRules::default(),
        false,
        None,
    );
    assert_eq!(cmp.warnings()[0].code, WarningCode::LossyDecode);
    assert!(cmp.warnings()[0].message.contains("1 malformed"));
}

#[test]
fn warnings_closeness_ignored_and_regex_limit() {
    let cmp = compare_texts(
        "a\nb\nc\n",
        "a\nx\nc\n",
        &format("ee"),
        &rules_with(AlgorithmChoice::Myers),
    );
    assert!(
        cmp.warnings()
            .iter()
            .any(|w| w.code == WarningCode::ClosenessIgnored)
    );
    // A backtracking pattern that hits the fancy-regex step limit: counted, reported, treated as no match.
    let mut rules = TextRules::default();
    rules
        .importance
        .unimportant
        .push(TextPattern::regex(r"(a*)*\1b"));
    let long = format!("{}\n", "a".repeat(30));
    let cmp = compare_texts(&long, "z\n", &format("ee"), &rules);
    assert!(
        cmp.warnings()
            .iter()
            .any(|w| w.code == WarningCode::RegexRuntimeError),
        "{:?}",
        cmp.warnings()
    );
}

#[test]
fn warnings_alignment_degraded_is_mapped_to_the_core_code() {
    // Random lines over 50 symbols exhaust Standard's work budget and then its fallback budget (D-16).
    let mut rng = XorShift(0x5EED_7E47_0000_0001);
    let mut side = || {
        (0..20_000)
            .map(|_| format!("s{}\n", rng.below(50)))
            .collect::<String>()
    };
    let (left, right) = (side(), side());
    let cmp = compare_texts(
        &left,
        &right,
        &format("ee"),
        &rules_with(AlgorithmChoice::Standard),
    );
    let codes: Vec<WarningCode> = cmp.warnings().iter().map(|w| w.code).collect();
    assert!(
        codes.contains(&WarningCode::WorkBudgetExhausted),
        "{codes:?}"
    );
    let degraded = cmp
        .warnings()
        .iter()
        .find(|w| w.code == WarningCode::AlignmentDegraded)
        .expect("alignment_degraded");
    assert!(degraded.spec.is_none());
    assert!(!degraded.message.is_empty());
}

#[test]
fn regex_error_counts_do_not_depend_on_the_pool() {
    // A block comment open across the first chunk boundary (8,192 lines) makes the parallel tokenizer re-run the
    // start of the second chunk; a line there hits the backtracking limit of the unimportant pattern.
    let mut lines: Vec<String> = (0..20_000).map(|k| format!("int v{k} = {k};")).collect();
    lines[8190] = "/* open".into();
    lines[8195] = "close */".into();
    lines[8193] = "a".repeat(30);
    lines[100] = "a".repeat(30);
    let left = lines.join("\n") + "\n";
    lines[5000] = "int changed = 1;".into();
    let right = lines.join("\n") + "\n";
    let mut rules = TextRules::default();
    rules
        .importance
        .unimportant
        .push(TextPattern::regex(r"(a*)*\1b"));
    let c = format("c-cpp");
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let sequential = compare_docs(doc(&left), doc(&right), &c, &rules, false, None);
    let parallel = compare_docs(doc(&left), doc(&right), &c, &rules, false, Some(&pool));
    let regex = |cmp: &hikasa_text::TextComparison| {
        cmp.warnings()
            .iter()
            .find(|w| w.code == WarningCode::RegexRuntimeError)
            .map(|w| w.message.clone())
    };
    assert!(regex(&sequential).is_some());
    assert_eq!(regex(&sequential), regex(&parallel));
    assert_eq!(sequential.warnings(), parallel.warnings());
}
