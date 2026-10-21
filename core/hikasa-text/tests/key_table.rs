//! The normative key table of §2.13.3 (every row), checked twice: with Unaligned (the run kind of the row's
//! pair is `Equal` exactly when the keys are equal) and with the default algorithm (the row's state and
//! importance). Plus the inline classification examples of §6.2.5.

mod common;

use common::{compare_texts, format, rules_with};
use hikasa_core::Side;
use hikasa_text::{
    AlgorithmChoice, Importance, LineIdx, RowIdx, RowState, RunKind, TextPattern, TextRules,
    UnimportantItem,
};

/// Rules of a table row.
#[derive(Clone, Copy)]
enum R {
    Default,
    WsOff,
    Unimportant(&'static str),
    CompareEol,
    IdPattern,
    OrphansNotImportant,
}

fn rules(r: R, algorithm: AlgorithmChoice) -> TextRules {
    let mut rules = rules_with(algorithm);
    match r {
        R::Default => {}
        R::WsOff => rules
            .importance
            .apply_unimportant(&[UnimportantItem::Whitespace]),
        R::Unimportant(items) => rules
            .importance
            .apply_unimportant(&UnimportantItem::parse_list(items).unwrap()),
        R::CompareEol => rules.importance.compare_line_endings = true,
        R::IdPattern => rules
            .importance
            .unimportant
            .push(TextPattern::regex(r"\$Id:[^$]*\$")),
        R::OrphansNotImportant => rules.importance.orphans_always_important = false,
    }
    rules
}

/// One table row: format, left text, right text, rules, the row checked, keys equal (None: an orphan row has no
/// pair), expected state and importance.
struct Case {
    n: u32,
    format: &'static str,
    left: String,
    right: String,
    rules: R,
    row: u32,
    keys_equal: Option<bool>,
    state: RowState,
    importance: Importance,
}

fn cobol_line(id: &str) -> String {
    let text = "       MOVE A TO B.";
    format!("{text:<72}{id}\n")
}

fn table() -> Vec<Case> {
    use Importance::{Important, Unimportant};
    use RowState::*;
    let c = |n, format, left: &str, right: &str, rules, keys, state, importance| Case {
        n,
        format,
        left: left.to_owned(),
        right: right.to_owned(),
        rules,
        row: 0,
        keys_equal: keys,
        state,
        importance,
    };
    let mut rows = vec![
        c(
            1,
            "c-cpp",
            "int  x = 1;\n",
            "int x = 1;\n",
            R::Default,
            Some(false),
            Changed,
            Important,
        ),
        c(
            2,
            "c-cpp",
            "int  x = 1;\n",
            "int x = 1;\n",
            R::WsOff,
            Some(true),
            Changed,
            Unimportant,
        ),
        c(
            3,
            "c-cpp",
            "int x;\n",
            "intx;\n",
            R::WsOff,
            Some(false),
            Changed,
            Important,
        ),
        c(
            4,
            "ee",
            "a b\n",
            "ab\n",
            R::WsOff,
            Some(true),
            Changed,
            Unimportant,
        ),
        c(
            5,
            "c-cpp",
            "x = 1; // old\n",
            "x = 1; // new\n",
            R::Unimportant("Comment"),
            Some(true),
            Changed,
            Unimportant,
        ),
        c(
            6,
            "c-cpp",
            "x = 1; // old\n",
            "x = 1; // new\n",
            R::Default,
            Some(false),
            Changed,
            Important,
        ),
        c(
            7,
            "sql",
            "SELECT a\n",
            "select a\n",
            R::Default,
            Some(true),
            Changed,
            Unimportant,
        ),
        c(
            8,
            "ee",
            "Hello\n",
            "hello\n",
            R::Default,
            Some(false),
            Changed,
            Important,
        ),
        c(
            9,
            "ee",
            "Hello\n",
            "hello\n",
            R::Unimportant("case"),
            Some(true),
            Changed,
            Unimportant,
        ),
        c(
            10,
            "c-cpp",
            "x\n",
            "x\r\n",
            R::Default,
            Some(true),
            Same,
            Importance::None,
        ),
        c(
            11,
            "c-cpp",
            "x\n",
            "x\r\n",
            R::CompareEol,
            Some(false),
            Changed,
            Important,
        ),
        c(
            12,
            "python",
            "    return x\n",
            "  return x\n",
            R::WsOff,
            Some(false),
            Changed,
            Important,
        ),
        c(
            13,
            "python",
            "x = 1   \n",
            "x = 1\n",
            R::Default,
            Some(true),
            Changed,
            Unimportant,
        ),
        c(
            14,
            "c-cpp",
            "x /* a\n   b */\n",
            "x /* a\nb */\n",
            R::WsOff,
            Some(true),
            Changed,
            Unimportant,
        ),
        c(
            15,
            "ee",
            "x  \n",
            "x\n",
            R::Default,
            Some(false),
            Changed,
            Important,
        ),
        c(
            16,
            "cobol",
            &cobol_line("AB000010"),
            &cobol_line("AB000020"),
            R::Default,
            Some(true),
            Changed,
            Unimportant,
        ),
        c(
            17,
            "ee",
            "$Id: 12 $\n",
            "$Id: 13 $\n",
            R::IdPattern,
            Some(true),
            Changed,
            Unimportant,
        ),
        c(
            18,
            "ee",
            "a\n\nb\n",
            "a\nb\n",
            R::Default,
            None,
            LeftOnly,
            Important,
        ),
        c(
            19,
            "ee",
            "a\n\nb\n",
            "a\nb\n",
            R::OrphansNotImportant,
            None,
            LeftOnly,
            Unimportant,
        ),
    ];
    rows[13].row = 1;
    rows[17].row = 1;
    rows[18].row = 1;
    rows
}

#[test]
fn every_row_of_the_key_table() {
    let rows = table();
    assert_eq!(rows.len(), 19);
    for case in rows {
        let fmt = format(case.format);
        // Unaligned pairs line k with line k, the pairing the table describes: the run is Equal exactly when
        // the keys are equal, and the row has the table's state and importance.
        let unaligned = compare_texts(
            &case.left,
            &case.right,
            &fmt,
            &rules(case.rules, AlgorithmChoice::Unaligned),
        );
        let row = unaligned.row(RowIdx(case.row)).unwrap();
        if let Some(equal) = case.keys_equal {
            assert_eq!(
                (row.state, row.importance),
                (case.state, case.importance),
                "row {} (unaligned)",
                case.n
            );
            let (l, r) = (row.left.unwrap().0, row.right.unwrap().0);
            let run = unaligned
                .runs()
                .iter()
                .find(|run| run.left.contains(&l) && run.right.contains(&r))
                .unwrap();
            assert_eq!(
                run.kind == RunKind::Equal,
                equal,
                "row {}: keys equal (run {:?})",
                case.n,
                run.kind
            );
        }
        // The default and the other engines: the row showing the left line has the table's state and importance;
        // equal keys always pair (an Equal run), and differing keys pair through closeness (row 15: `x  ` and
        // `x` are equal after removing whitespace, similarity 999; §2.15 as amended in wave 3).
        for algorithm in [
            AlgorithmChoice::Auto,
            AlgorithmChoice::Standard,
            AlgorithmChoice::Histogram,
        ] {
            let cmp = compare_texts(&case.left, &case.right, &fmt, &rules(case.rules, algorithm));
            // Orphan rows (18, 19): the blank left line `row` itself.
            let left_line = match case.keys_equal {
                Some(_) => unaligned.row(RowIdx(case.row)).unwrap().left.unwrap(),
                None => LineIdx(case.row),
            };
            let at = cmp.row_for_line(Side::Left, left_line).unwrap();
            let row = cmp.row(at).unwrap();
            assert_eq!(
                row.importance, case.importance,
                "row {} with {algorithm}",
                case.n
            );
            assert_eq!(row.state, case.state, "row {} with {algorithm}", case.n);
        }
    }
}

/// Decision 1 of the wave-3 review: short lines (one token, a brace) and whitespace-only changes pair as
/// Changed rows under the default rules, with every engine that uses closeness; unrelated one-char lines do
/// not pair.
#[test]
fn short_lines_and_braces_pair_as_changed_rows() {
    let pairs: [(&str, &str, &str, bool); 9] = [
        ("ee", "x  \n", "x\n", true),
        ("ee", "\tx\n", "x\n", true),
        ("c-cpp", "}\n", "} \n", true),
        ("c-cpp", "}\n", "};\n", true),
        ("ee", "ab\n", "aB\n", true),
        ("rust", "    }\n", "}\n", true),
        ("ee", "x\n", "y\n", false),
        ("c-cpp", "{\n", "} else {\n", false),
        ("python", "pass\n", "return\n", false),
    ];
    for (fmt, left, right, paired) in pairs {
        for algorithm in [
            AlgorithmChoice::Auto,
            AlgorithmChoice::Standard,
            AlgorithmChoice::Histogram,
            AlgorithmChoice::Patience,
        ] {
            let cmp = compare_texts(left, right, &format(fmt), &rules(R::Default, algorithm));
            let row = cmp.row(RowIdx(0)).unwrap();
            if paired {
                assert_eq!(
                    (row.state, row.importance, cmp.row_count()),
                    (RowState::Changed, Importance::Important, 1),
                    "{left:?} vs {right:?} ({fmt}, {algorithm})"
                );
            } else {
                assert_eq!(
                    (row.state, cmp.row_count()),
                    (RowState::LeftOnly, 2),
                    "{left:?} vs {right:?} ({fmt}, {algorithm})"
                );
            }
        }
    }
    // Inside a larger change the brace still finds its counterpart: the rows stay aligned.
    let cmp = compare_texts(
        "fn a() {\n    x();\n}\n",
        "fn a() {\n    y();\n};\n",
        &format("rust"),
        &TextRules::default(),
    );
    let states: Vec<RowState> = cmp.rows(0..cmp.row_count()).map(|r| r.state).collect();
    assert_eq!(
        states,
        [RowState::Same, RowState::Changed, RowState::Changed]
    );
}

#[test]
fn inline_case_only_sql_keyword_is_unimportant() {
    let cmp = compare_texts(
        "SELECT a\n",
        "select a\n",
        &format("sql"),
        &TextRules::default(),
    );
    let inline = cmp.inline(RowIdx(0)).unwrap();
    assert!(!inline.left.is_empty() && !inline.right.is_empty());
    assert!(
        inline
            .left
            .iter()
            .chain(&inline.right)
            .all(|r| !r.important),
        "{inline:?}"
    );
    // A case change of an identifier (case-sensitive) stays important.
    let cmp = compare_texts(
        "SELECT Abc\n",
        "SELECT abc\n",
        &format("sql"),
        &TextRules::default(),
    );
    let inline = cmp.inline(RowIdx(0)).unwrap();
    assert!(inline.left.iter().all(|r| r.important), "{inline:?}");
}

#[test]
fn inline_comment_change_is_blue_when_comments_are_unimportant() {
    let mut rules = TextRules::default();
    rules
        .importance
        .apply_unimportant(&[UnimportantItem::Element("Comment".into())]);
    let cmp = compare_texts(
        "x = 1; // old\n",
        "x = 2; // new\n",
        &format("c-cpp"),
        &rules,
    );
    let row = cmp.row(RowIdx(0)).unwrap();
    assert_eq!(
        (row.state, row.importance),
        (RowState::Changed, Importance::Important)
    );
    let inline = cmp.inline(RowIdx(0)).unwrap();
    let left: Vec<(u32, u32, bool)> = inline
        .left
        .iter()
        .map(|r| (r.range.start, r.range.end, r.important))
        .collect();
    assert_eq!(left.first(), Some(&(4, 5, true)), "{left:?}");
    assert!(
        left[1..].iter().all(|&(s, _, imp)| s >= 7 && !imp),
        "{left:?}"
    );
    // The spans tell the GUI which element each part is.
    let spans = cmp.spans(Side::Left, LineIdx(0));
    let comment = spans.last().unwrap();
    assert_eq!(
        cmp.format().grammar().elements().name(comment.element),
        "Comment"
    );
    assert!(!comment.important);
}
