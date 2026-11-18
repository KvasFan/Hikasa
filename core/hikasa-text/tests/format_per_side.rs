//! A10.20 (D-281, FF-003): a format per pane. Without a per-side format every golden comparison is unchanged
//! (and a right format that is a renamed copy of the left one gives the same result through the per-side code);
//! a C file against its plain-text copy is *rules same* under the default rules and differs on every comment
//! line with comments unimportant; an element the other side's format lacks does not exist there; *Auto* with one
//! grammar; the identity fast path decided per side; each side's spans in the inline classification. JSON
//! `right_format`, the flags and `[specs] left_format` are tested in hikasa-session and hikasa-cli.

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use common::{canonical_core, canonical_json, fixtures, registry};
use hikasa_core::{ContentResult, Ctl};
use hikasa_formats::{CompiledFormat, EnabledOverrides, RegexLimits, parse_format};
use hikasa_text::{
    Algorithm, AlgorithmChoice, CompareInput, DocOrigin, Importance, LoadOptions, RowIdx, RowState,
    TextComparison, TextDocument, TextRules, compare,
};

/// A copy of `format` under another id and name: a different format with the same grammar.
fn renamed(format: &CompiledFormat) -> Arc<CompiledFormat> {
    let Some(source) = format.format() else {
        // Everything Else has no source: a second instance has no id, so it is another format.
        return Arc::new(CompiledFormat::everything_else());
    };
    let mut copy = source.clone();
    copy.id = hikasa_formats::FormatId::new("copy-of-left").unwrap();
    copy.name = format!("{} (copy)", source.name);
    Arc::new(CompiledFormat::compile(Arc::new(copy), &RegexLimits::default()).unwrap())
}

fn compare_with(
    left: &str,
    right: &str,
    format: &Arc<CompiledFormat>,
    right_format: Option<Arc<CompiledFormat>>,
    rules: &TextRules,
    force_tokenize: bool,
) -> TextComparison {
    let cmp = compare(
        CompareInput {
            left: common::doc(left),
            right: common::doc(right),
            format: format.clone(),
            rules: rules.clone(),
            bytes_identical: false,
            force_tokenize,
            anchors: Vec::new(),
            right_format,
            ignored: Default::default(),
        },
        None,
        &Ctl::detached(),
        None,
    )
    .unwrap();
    cmp.check_invariants().unwrap();
    cmp
}

/// The canonical JSON without the `right_format` key.
fn without_right_format(json: &str) -> String {
    let mut v: serde_json::Value = serde_json::from_str(json).unwrap();
    v.as_object_mut().unwrap().remove("right_format");
    serde_json::to_string_pretty(&v).unwrap()
}

/// The golden fixtures with the right side compared through a renamed copy of the effective format: every result
/// equals the single-format one (the per-side keys, weights, spans and identity decisions agree), and the
/// comparison names the right format.
#[test]
fn a_renamed_copy_of_the_format_changes_no_golden_comparison() {
    let root = fixtures("compare");
    let mut dirs: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(dirs.len() >= 25);
    for dir in &dirs {
        let side = |stem: &str| {
            fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(&format!("{stem}.")))
                })
                .unwrap()
        };
        let (lp, rp) = (side("left"), side("right"));
        let format = effective_format(dir, lp.file_name().unwrap().to_str().unwrap());
        let rules = fs::read_to_string(dir.join("rules.toml"))
            .map(|t| TextRules::from_toml(&t).unwrap().0)
            .unwrap_or_default();
        let (lb, rb) = (fs::read(&lp).unwrap(), fs::read(&rp).unwrap());
        let doc = |bytes: &[u8]| {
            Arc::new(
                TextDocument::from_bytes(
                    bytes,
                    &format,
                    &LoadOptions::default(),
                    DocOrigin {
                        spec: None,
                        mtime: None,
                        size: bytes.len() as u64,
                    },
                )
                .unwrap(),
            )
        };
        let input = |right_format| CompareInput {
            left: doc(&lb),
            right: doc(&rb),
            format: format.clone(),
            rules: rules.clone(),
            bytes_identical: lb == rb,
            force_tokenize: false,
            anchors: Vec::new(),
            right_format,
            ignored: Default::default(),
        };
        let one = compare(input(None), None, &Ctl::detached(), None).unwrap();
        let golden = fs::read_to_string(dir.join("expected.json")).unwrap();
        assert_eq!(canonical_json(&one), golden, "{}", dir.display());
        // The same format given twice is one format.
        let same = compare(input(Some(format.clone())), None, &Ctl::detached(), None).unwrap();
        assert!(same.right_format().is_none(), "{}", dir.display());
        assert_eq!(canonical_json(&same), golden, "{}", dir.display());
        let copy = renamed(&format);
        let two = compare(input(Some(copy.clone())), None, &Ctl::detached(), None).unwrap();
        two.check_invariants().unwrap();
        assert_eq!(two.right_format().map(|f| f.name()), Some(copy.name()));
        assert_eq!(
            without_right_format(&canonical_json(&two)),
            without_right_format(&golden),
            "{}",
            dir.display()
        );
    }
}

fn effective_format(dir: &Path, left_name: &str) -> Arc<CompiledFormat> {
    let registry = registry();
    if let Ok(id) = fs::read_to_string(dir.join("format.txt")) {
        return registry.force(id.trim()).unwrap().format;
    }
    if let Ok(text) = fs::read_to_string(dir.join("format.toml")) {
        let (format, _) = parse_format(&text, "format.toml").unwrap();
        return Arc::new(
            CompiledFormat::compile(Arc::new(format), &RegexLimits::default()).unwrap(),
        );
    }
    registry
        .detect(left_name, &EnabledOverrides::default())
        .unwrap()
        .format
}

const C_SOURCE: &str =
    "int a = 1; // first\n/* block\n   comment */\nint b = 2;\nint c = 3; /* tail */\nreturn a;\n";

/// D-281's example: a C file against an identical plain-text copy is *rules same* under the default rules
/// (everything important); with comments unimportant every line holding a comment differs, as the plain side
/// has no comment to remove.
#[test]
fn a_c_file_against_its_plain_text_copy() {
    let c = common::format("c-cpp");
    let plain = common::format("ee");
    let rules = TextRules::default();
    let cmp = compare_with(C_SOURCE, C_SOURCE, &c, Some(plain.clone()), &rules, false);
    assert_eq!(cmp.verdict(false), ContentResult::RulesSame);
    assert_eq!(cmp.right_format().map(|f| f.name()), Some(plain.name()));
    assert_eq!(cmp.format_of(hikasa_core::Side::Right).name(), plain.name());

    let mut rules = TextRules::default();
    rules
        .importance
        .elements
        .insert("Comment".to_owned(), false);
    let cmp = compare_with(C_SOURCE, C_SOURCE, &c, Some(plain.clone()), &rules, false);
    assert_eq!(cmp.verdict(false), ContentResult::ImportantDifferences);
    let differing: Vec<u32> = cmp
        .rows(0..cmp.row_count())
        .filter(|r| r.state != RowState::Same)
        .filter_map(|r| r.left.map(|l| l.0))
        .collect();
    // Lines 0, 1, 2 and 4 hold comments; 3 and 5 do not.
    assert_eq!(differing, [0, 1, 2, 4]);
    // Swapped: the same rows.
    let cmp = compare_with(C_SOURCE, C_SOURCE, &plain, Some(c), &rules, false);
    let swapped: Vec<u32> = cmp
        .rows(0..cmp.row_count())
        .filter(|r| r.state != RowState::Same)
        .filter_map(|r| r.left.map(|l| l.0))
        .collect();
    assert_eq!(swapped, differing);
}

/// An element a side's format lacks does not exist there: `Annotation` (Java) unimportant removes the annotation
/// from the Java side's key only, so the C side's identical line differs; important, the lines are the same.
#[test]
fn an_element_the_other_format_lacks() {
    let java = common::format("java");
    let c = common::format("c-cpp");
    let text = "@Deprecated\nint x;\n";
    let mut rules = TextRules::default();
    rules
        .importance
        .elements
        .insert("Annotation".to_owned(), false);
    let cmp = compare_with(text, text, &java, Some(c.clone()), &rules, false);
    let state = |side, line| {
        let row = cmp.row_for_line(side, hikasa_text::LineIdx(line)).unwrap();
        cmp.row(row).unwrap().state
    };
    assert_ne!(state(hikasa_core::Side::Right, 0), RowState::Same);
    assert_eq!(state(hikasa_core::Side::Right, 1), RowState::Same);
    assert_eq!(cmp.verdict(false), ContentResult::ImportantDifferences);
    // Both Java: the annotation is unimportant on both sides, the pair is the same.
    let both = compare_with(text, text, &java, None, &rules, false);
    assert_eq!(both.verdict(false), ContentResult::RulesSame);
    // Important (the default): the per-side pair is the same too.
    let cmp = compare_with(text, text, &java, Some(c), &TextRules::default(), false);
    assert_eq!(cmp.verdict(false), ContentResult::RulesSame);
}

/// *Auto* (D-36): Histogram when either side's format has a grammar (and the cost fits), else Standard.
#[test]
fn auto_takes_histogram_when_one_side_has_a_grammar() {
    let rules = common::rules_with(AlgorithmChoice::Auto);
    let (l, r) = ("a\nb\nc\nd\n", "a\nx\nc\nd\n");
    let (ee, c) = (common::format("ee"), common::format("c-cpp"));
    assert_eq!(
        compare_with(l, r, &ee, None, &rules, false).algorithm(),
        Algorithm::Standard
    );
    assert_eq!(
        compare_with(l, r, &ee, Some(c.clone()), &rules, false).algorithm(),
        Algorithm::Histogram
    );
    assert_eq!(
        compare_with(l, r, &c, Some(ee.clone()), &rules, false).algorithm(),
        Algorithm::Histogram
    );
    // An explicit choice stays.
    let rules = common::rules_with(AlgorithmChoice::Patience);
    assert_eq!(
        compare_with(l, r, &ee, Some(c), &rules, false).algorithm(),
        Algorithm::Patience
    );
}

/// The identity fast path (D-53) is decided per side: plain text keeps it while the C side with comments
/// unimportant tokenizes; the result equals the fully tokenized one, and the comparison reports the identity path
/// only when both sides took it.
#[test]
fn identity_paths_are_decided_per_side() {
    let (ee, c) = (common::format("ee"), common::format("c-cpp"));
    let right = "int a = 1; // other\n/* block\n   comment */\nint b = 2;\nint c = 4;\nreturn a;\n";
    let mut rules = TextRules::default();
    rules
        .importance
        .elements
        .insert("Comment".to_owned(), false);
    for (lf, rf) in [(&ee, &c), (&c, &ee)] {
        let fast = compare_with(C_SOURCE, right, lf, Some(rf.clone()), &rules, false);
        let slow = compare_with(C_SOURCE, right, lf, Some(rf.clone()), &rules, true);
        assert!(!fast.used_identity_path());
        assert_eq!(canonical_core(&fast), canonical_core(&slow));
    }
    // Default rules: plain text and its renamed copy both take the identity path, and it equals the tokenized
    // result; against C (whose default rules are not the identity's) the plain side alone takes it.
    let rules = TextRules::default();
    let fast = compare_with(C_SOURCE, right, &ee, Some(renamed(&ee)), &rules, false);
    let slow = compare_with(C_SOURCE, right, &ee, Some(renamed(&ee)), &rules, true);
    assert!(fast.right_format().is_some());
    assert!(fast.used_identity_path());
    assert!(!slow.used_identity_path());
    assert_eq!(canonical_core(&fast), canonical_core(&slow));
    let fast = compare_with(C_SOURCE, right, &ee, Some(c.clone()), &rules, false);
    let slow = compare_with(C_SOURCE, right, &ee, Some(c), &rules, true);
    assert_eq!(canonical_core(&fast), canonical_core(&slow));
}

/// Inline classification uses each side's spans: in a pair of a C line and the same line as plain text with
/// comments unimportant, the C side's difference inside the comment is unimportant while the plain side's is
/// important.
#[test]
fn inline_differences_use_each_sides_spans() {
    let (ee, c) = (common::format("ee"), common::format("c-cpp"));
    let mut rules = TextRules::default();
    rules
        .importance
        .elements
        .insert("Comment".to_owned(), false);
    let cmp = compare_with(
        "x = 1; // alpha\n",
        "x = 1; // omega\n",
        &c,
        Some(ee),
        &rules,
        false,
    );
    let row = cmp.row(RowIdx(0)).unwrap();
    assert_eq!(
        (row.state, row.importance),
        (RowState::Changed, Importance::Important)
    );
    let inline = cmp.inline(RowIdx(0)).unwrap();
    assert!(!inline.left.is_empty() && !inline.right.is_empty());
    assert!(inline.left.iter().all(|r| !r.important), "{inline:?}");
    assert!(inline.right.iter().all(|r| r.important), "{inline:?}");
    // Each side's spans come from its own grammar.
    let spans = cmp.spans(hikasa_core::Side::Left, hikasa_text::LineIdx(0));
    assert!(spans.iter().any(|s| !s.important));
    let spans = cmp.spans(hikasa_core::Side::Right, hikasa_text::LineIdx(0));
    assert!(spans.iter().all(|s| s.important));
}

/// Line weights come from each side's own format: Pascal (weighted) against its renamed copy aligns as Pascal
/// against Pascal; against plain text the plain side's lines weigh nothing.
#[test]
fn line_weights_come_from_each_side() {
    let pascal = common::format("pascal");
    assert!(pascal.has_line_weights());
    let rules = common::rules_with(AlgorithmChoice::Standard);
    let left = "procedure A;\nbegin\n  x := 1;\nend;\nprocedure B;\nbegin\n  y := 2;\nend;\n";
    let right = "procedure B;\nbegin\n  y := 2;\nend;\nprocedure A;\nbegin\n  x := 1;\nend;\n";
    let one = compare_with(left, right, &pascal, None, &rules, false);
    let two = compare_with(left, right, &pascal, Some(renamed(&pascal)), &rules, false);
    let value = |c: &TextComparison| {
        let mut v: serde_json::Value = serde_json::from_str(&canonical_core(c)).unwrap();
        v.as_object_mut().unwrap().remove("right_format");
        v
    };
    assert_eq!(value(&one), value(&two));
    let plain = compare_with(
        left,
        right,
        &pascal,
        Some(common::format("ee")),
        &rules,
        false,
    );
    plain.check_invariants().unwrap();
}
