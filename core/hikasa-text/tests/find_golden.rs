//! A10.22 (D-283): find and replace (TC-064, §1.10.18). Goldens for plain text and regular expressions, case on and off
//! (Unicode simple folding), whole word with and without the format's word characters (`+=` as a whole word), the
//! sides and row order of a comparison, wrap, `all` with its limit, a fancy pattern (backreference), the backtrack
//! limit (`regex_runtime_error`, the search goes on), a line-break pattern refused, empty matches, CPL pieces;
//! replacements `$1`, `${1}`, `${name}`, `$$`, an unknown group refused, a replacement with a line break, Replace All
//! as one step with its count.

mod common;

use std::sync::Arc;

use common::{compare_texts, format};
use hikasa_core::error::Classify;
use hikasa_core::{Ctl, ErrorKind, Pair, Side, WarningCode};
use hikasa_text::edit::{EditOptions, EditRequest, Editability, EditableDocument, GroupHint};
use hikasa_text::find::{
    Dir, FindError, FindQuery, FindSides, Finder, Hit, Replacement, find_in_comparison,
    find_in_comparison_at, replace_all, replace_one,
};
use hikasa_text::{RegexLimits, RowIdx, TextDocument, TextPos, TextRules};

fn editable(text: &str) -> EditableDocument {
    editable_cpl(text, None)
}

fn editable_cpl(text: &str, cpl: Option<u32>) -> EditableDocument {
    EditableDocument::new(
        Arc::new(TextDocument::from_text(text)),
        EditOptions {
            chars_per_line_limit: cpl,
            ..EditOptions::default()
        },
        Editability::EDITABLE,
    )
}

fn query(text: &str, regex: bool, match_case: bool, whole_word: bool) -> FindQuery {
    FindQuery {
        text: text.to_owned(),
        regex,
        match_case,
        whole_word,
    }
}

fn finder(q: &FindQuery, word_chars: &str) -> Finder {
    Finder::new(q, word_chars, &RegexLimits::default()).unwrap()
}

/// Every hit as `(line, byte)..(line, byte)`.
fn all(f: &Finder, doc: &EditableDocument) -> Vec<((u32, u32), (u32, u32))> {
    f.all(doc, u64::MAX, &Ctl::detached())
        .unwrap()
        .into_iter()
        .map(|h| {
            (
                (h.range.start.line.0, h.range.start.byte),
                (h.range.end.line.0, h.range.end.byte),
            )
        })
        .collect()
}

fn pos(line: u32, byte: u32) -> TextPos {
    TextPos::new(line, byte)
}

#[test]
fn plain_text_with_and_without_case() {
    let doc = editable("Foo foo\nfOO bar foo\n\nfoofoo\n");
    let f = finder(&query("foo", false, true, false), "");
    assert_eq!(
        all(&f, &doc),
        [
            ((0, 4), (0, 7)),
            ((1, 8), (1, 11)),
            ((3, 0), (3, 3)),
            ((3, 3), (3, 6))
        ]
    );
    let f = finder(&query("foo", false, false, false), "");
    assert_eq!(all(&f, &doc).len(), 6);
    // Characters a regex would read are plain text here.
    let doc = editable("a.b axb (a.b)\n");
    let f = finder(&query("a.b", false, true, false), "");
    assert_eq!(all(&f, &doc), [((0, 0), (0, 3)), ((0, 9), (0, 12))]);
}

#[test]
fn case_folding_is_unicode() {
    let doc = editable("ÉCOLE école École\nΣΊΣΥΦΟΣ σίσυφος\n");
    let f = finder(&query("école", false, false, false), "");
    assert_eq!(all(&f, &doc).len(), 3);
    let f = finder(&query("école", false, true, false), "");
    assert_eq!(all(&f, &doc), [((0, 7), (0, 13))]);
    let f = finder(&query("σ", false, false, false), "");
    // Σ and σ fold together; ς is its own letter under simple folding of the regex crate's (?i) (it folds to σ).
    assert!(all(&f, &doc).len() >= 4);
}

#[test]
fn regular_expressions_see_the_line() {
    let doc = editable("abc_17 x\nfoo_17\nbar_170\n  x_17\n");
    let f = finder(&query(r"\b\w+_17\b", true, true, false), "");
    assert_eq!(
        all(&f, &doc),
        [((0, 0), (0, 6)), ((1, 0), (1, 6)), ((3, 2), (3, 6))]
    );
    // ^ and $ are the line's edges.
    let f = finder(&query(r"^\w+$", true, true, false), "");
    assert_eq!(all(&f, &doc), [((1, 0), (1, 6)), ((2, 0), (2, 7))]);
    let f = finder(&query(r"X_17$", true, false, false), "");
    assert_eq!(all(&f, &doc), [((3, 2), (3, 6))]);
}

#[test]
fn whole_word_by_the_format_s_word_characters() {
    let doc = editable("foo foobar bar-foo foo_x\n");
    let f = finder(&query("foo", false, true, true), "");
    assert_eq!(all(&f, &doc), [((0, 0), (0, 3)), ((0, 15), (0, 18))]);
    // With `-` a word character, bar-foo is one word.
    let f = finder(&query("foo", false, true, true), "-");
    assert_eq!(all(&f, &doc), [((0, 0), (0, 3))]);
    // A regex gets the same rule.
    let f = finder(&query("fo+", true, true, true), "-");
    assert_eq!(all(&f, &doc), [((0, 0), (0, 3))]);
    let f = finder(&query("fo+", true, true, true), "");
    assert_eq!(all(&f, &doc), [((0, 0), (0, 3)), ((0, 15), (0, 18))]);
    // A regex whose first alternative fails the boundary still finds the longer one.
    let doc = editable("abc ab\n");
    let f = finder(&query("ab|abc", true, true, true), "");
    assert_eq!(all(&f, &doc), [((0, 0), (0, 3)), ((0, 4), (0, 6))]);
}

#[test]
fn an_operator_is_a_whole_word_as_itself() {
    let doc = editable("a += b;a+=b; c +== d\n");
    let f = finder(&query("+=", false, true, true), "");
    // Its own characters are no word characters, so only word characters outside could bound it: none here.
    assert_eq!(
        all(&f, &doc),
        [((0, 2), (0, 4)), ((0, 8), (0, 10)), ((0, 15), (0, 17))]
    );
    let f = finder(&query(r"\+=", true, true, true), "");
    assert_eq!(all(&f, &doc).len(), 3);
}

#[test]
fn whole_word_checks_both_ends() {
    let doc = editable("cat cats bobcat cat\n");
    let f = finder(&query("cat", false, true, true), "");
    assert_eq!(all(&f, &doc), [((0, 0), (0, 3)), ((0, 16), (0, 19))]);
    let f = finder(&query("cat", true, true, true), "");
    assert_eq!(all(&f, &doc), [((0, 0), (0, 3)), ((0, 16), (0, 19))]);
}

#[test]
fn next_wraps_once_and_says_so() {
    let doc = editable("x foo\nbar\nfoo y\n");
    let f = finder(&query("foo", false, true, false), "");
    let next = |from, dir, wrap| f.next(&doc, from, dir, wrap).unwrap();
    let h = next(pos(0, 0), Dir::Forward, false).unwrap();
    assert_eq!(h.range, pos(0, 2)..pos(0, 5));
    assert!(!h.wrapped);
    // From the hit's end: the next one.
    let h = next(pos(0, 5), Dir::Forward, false).unwrap();
    assert_eq!(h.range, pos(2, 0)..pos(2, 3));
    assert_eq!(next(pos(2, 1), Dir::Forward, false), None);
    let h = next(pos(2, 1), Dir::Forward, true).unwrap();
    assert_eq!(h.range, pos(0, 2)..pos(0, 5));
    assert!(h.wrapped);
    // Backward: the last hit that starts before the position.
    let h = next(pos(2, 0), Dir::Backward, false).unwrap();
    assert_eq!(h.range, pos(0, 2)..pos(0, 5));
    assert_eq!(next(pos(0, 2), Dir::Backward, false), None);
    let h = next(pos(0, 2), Dir::Backward, true).unwrap();
    assert_eq!(h.range, pos(2, 0)..pos(2, 3));
    assert!(h.wrapped);
    let h = next(pos(2, 1), Dir::Backward, false).unwrap();
    assert_eq!(h.range, pos(2, 0)..pos(2, 3));
    // Nothing at all: None also with wrap.
    let none = finder(&query("zzz", false, true, false), "");
    assert_eq!(
        none.next(&doc, pos(1, 0), Dir::Forward, true).unwrap(),
        None
    );
}

#[test]
fn all_stops_at_its_limit() {
    let doc = editable(&"ab ab ab\n".repeat(10));
    let f = finder(&query("ab", false, true, false), "");
    assert_eq!(f.all(&doc, 7, &Ctl::detached()).unwrap().len(), 7);
    assert_eq!(f.all(&doc, 0, &Ctl::detached()).unwrap().len(), 0);
    assert_eq!(f.all(&doc, u64::MAX, &Ctl::detached()).unwrap().len(), 30);
}

#[test]
fn a_backreference_runs_on_the_fancy_engine() {
    let doc = editable("abab xyxy abba\n");
    let f = finder(&query(r"(\w\w)\1", true, true, false), "");
    assert_eq!(all(&f, &doc), [((0, 0), (0, 4)), ((0, 5), (0, 9))]);
    let f = finder(&query(r"(?<=x)y", true, true, false), "");
    assert_eq!(all(&f, &doc), [((0, 6), (0, 7)), ((0, 8), (0, 9))]);
}

#[test]
fn the_backtrack_limit_is_no_match_and_one_warning() {
    let long = "a".repeat(64);
    let text = format!("{long}\nab xx\n{long}\nxx\n");
    let doc = editable(&text);
    let limits = RegexLimits {
        backtrack_limit: 1_000,
        ..RegexLimits::default()
    };
    // Catastrophic on the long lines, quick elsewhere; a backreference forces the fancy engine.
    let f = Finder::new(
        &query(r"(?:(?=a)a*)*b|(x)\1", true, true, false),
        "",
        &limits,
    )
    .unwrap();
    let hits = f.all(&doc, u64::MAX, &Ctl::detached()).unwrap();
    // `ab` and the two `xx`: the lines around them hit the limit and count as no match.
    assert_eq!(hits.len(), 3, "{hits:?}");
    let warning = f.take_warning().unwrap();
    assert_eq!(warning.code, WarningCode::RegexRuntimeError);
    assert!(warning.message.contains("2 lines"), "{}", warning.message);
    // Once per search: taken, it is gone.
    assert_eq!(f.take_warning(), None);
}

#[test]
fn a_search_does_not_cross_line_ends() {
    for (text, regex) in [
        (r"\n", true),
        (r"a\r\nb", true),
        ("[\\r\\n]+", true),
        ("a\nb", false),
    ] {
        let e = Finder::new(
            &query(text, regex, true, false),
            "",
            &RegexLimits::default(),
        )
        .unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidInput, "{text}");
        assert!(e.to_string().contains("does not cross line ends"), "{e}");
    }
    let e = Finder::new(&query("(", true, true, false), "", &RegexLimits::default()).unwrap_err();
    assert!(matches!(e, FindError::InvalidInput(_)));
    let e = Finder::new(&query("", false, true, false), "", &RegexLimits::default()).unwrap_err();
    assert!(matches!(e, FindError::InvalidInput(_)));
    // \s may match a break but finds blanks in a line.
    let doc = editable("a b\n");
    let f = finder(&query(r"\s", true, true, false), "");
    assert_eq!(all(&f, &doc), [((0, 1), (0, 2))]);
}

#[test]
fn empty_matches_are_skipped() {
    let doc = editable("bxxb\n\nx\n");
    let f = finder(&query("x*", true, true, false), "");
    assert_eq!(all(&f, &doc), [((0, 1), (0, 3)), ((2, 0), (2, 1))]);
    let f = finder(&query("^", true, true, false), "");
    assert!(all(&f, &doc).is_empty());
}

#[test]
fn a_word_cut_by_the_characters_per_line_limit_is_found() {
    let doc = editable_cpl("0123456789abcdefghij\n", Some(8));
    assert!(doc.line_count() > 1);
    let f = finder(&query("6789ab", false, true, false), "");
    assert_eq!(all(&f, &doc), [((0, 6), (1, 4))]);
    let f = finder(&query("^0", true, true, false), "");
    assert_eq!(all(&f, &doc).len(), 1);
    // A hit that starts at a piece's start begins in that piece; one that ends at a piece's end ends there.
    let f = finder(&query("01234567", false, true, false), "");
    assert_eq!(all(&f, &doc), [((0, 0), (0, 8))]);
    let f = finder(&query("89", false, true, false), "");
    assert_eq!(all(&f, &doc), [((1, 0), (1, 2))]);
}

#[test]
fn crlf_and_cr_lines() {
    let doc = editable("foo\r\nbar foo\rfoo");
    let f = finder(&query("foo", false, false, false), "");
    assert_eq!(
        all(&f, &doc),
        [((0, 0), (0, 3)), ((1, 4), (1, 7)), ((2, 0), (2, 3))]
    );
    let f = finder(&query("o$", true, true, false), "");
    assert_eq!(all(&f, &doc).len(), 3);
}

#[test]
fn a_comparison_by_row_and_left_side_first() {
    let left = "a\nfoo\nb\nc\n";
    let right = "a\nfoo x\nb\nc\nfoo\n";
    let cmp = compare_texts(left, right, &format("ee"), &TextRules::default());
    let docs = Pair::new(editable(left), editable(right));
    let f = finder(&query("foo", false, true, false), "");
    let step = |from: (u32, Side, u32), sides, dir, wrap| {
        find_in_comparison_at(
            &cmp,
            Pair::new(&docs.left, &docs.right),
            &f,
            (RowIdx(from.0), from.1, from.2),
            sides,
            dir,
            wrap,
        )
        .unwrap()
        .map(|(side, hit)| (side, hit.range.start.line.0, hit.wrapped))
    };
    let row_of = |line| {
        cmp.row_for_line(Side::Right, hikasa_text::LineIdx(line))
            .unwrap()
            .0
    };
    let last = row_of(4);
    // Forward from the start: row 1 left, row 1 right, the last row right.
    assert_eq!(
        step((0, Side::Left, 0), FindSides::Both, Dir::Forward, false),
        Some((Side::Left, 1, false))
    );
    assert_eq!(
        step((1, Side::Left, 1), FindSides::Both, Dir::Forward, false),
        Some((Side::Right, 1, false))
    );
    assert_eq!(
        step((1, Side::Right, 1), FindSides::Both, Dir::Forward, false),
        Some((Side::Right, 4, false))
    );
    assert_eq!(
        step((last, Side::Right, 1), FindSides::Both, Dir::Forward, false),
        None
    );
    assert_eq!(
        step((last, Side::Right, 1), FindSides::Both, Dir::Forward, true),
        Some((Side::Left, 1, true))
    );
    // One side only.
    assert_eq!(
        step((0, Side::Left, 0), FindSides::Right, Dir::Forward, false),
        Some((Side::Right, 1, false))
    );
    assert_eq!(
        step((1, Side::Left, 1), FindSides::Left, Dir::Forward, false),
        None
    );
    // Backward: the right side of a row comes after its left side.
    assert_eq!(
        step(
            (last, Side::Right, 0),
            FindSides::Both,
            Dir::Backward,
            false
        ),
        Some((Side::Right, 1, false))
    );
    assert_eq!(
        step((1, Side::Right, 0), FindSides::Both, Dir::Backward, false),
        Some((Side::Left, 1, false))
    );
    assert_eq!(
        step((1, Side::Left, 0), FindSides::Both, Dir::Backward, false),
        None
    );
    assert_eq!(
        step((1, Side::Left, 0), FindSides::Both, Dir::Backward, true),
        Some((Side::Right, 4, true))
    );
    // The contract's form: from a cell's start.
    let first = find_in_comparison(
        &cmp,
        Pair::new(&docs.left, &docs.right),
        &f,
        (RowIdx(1), Side::Right),
        FindSides::Both,
        Dir::Forward,
        false,
    )
    .unwrap()
    .unwrap();
    assert_eq!((first.0, first.1.range.start), (Side::Right, pos(1, 0)));
}

#[test]
fn replacements_by_group() {
    let doc = editable("key=value\n");
    let f = finder(&query(r"(?<k>\w+)=(\w+)", true, true, false), "");
    let hit = f
        .next(&doc, TextPos::ZERO, Dir::Forward, false)
        .unwrap()
        .unwrap();
    let one = |template: &str| {
        let with = Replacement::new(template, &f).unwrap();
        match replace_one(&doc, &f, &hit, &with).unwrap() {
            EditRequest::Replace {
                text,
                range,
                keep_eols,
            } => {
                assert_eq!(range, hit.range);
                assert!(!keep_eols);
                text
            }
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(one("$2=$1"), "value=key");
    assert_eq!(one("${2}x"), "valuex");
    assert_eq!(one("${k}:$$"), "key:$");
    assert_eq!(one("$0!"), "key=value!");
    for bad in ["$3", "${nope}", "$", "$x"] {
        let e = Replacement::new(bad, &f).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidInput, "{bad}");
    }
    // Plain text takes the replacement literally.
    let plain = finder(&query("=", false, true, false), "");
    let with = Replacement::new("$1", &plain).unwrap();
    let hit = plain
        .next(&doc, TextPos::ZERO, Dir::Forward, false)
        .unwrap()
        .unwrap();
    match replace_one(&doc, &plain, &hit, &with).unwrap() {
        EditRequest::Replace { text, .. } => assert_eq!(text, "$1"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_replacement_with_a_line_break_takes_the_document_s_terminator() {
    let mut doc = editable("a, b\r\nc, d\r\n");
    let f = finder(&query(", ", false, true, false), "");
    let with = Replacement::new("\n", &f).unwrap();
    let (req, count) = replace_all(&doc, &f, &with, None, &Ctl::detached()).unwrap();
    assert_eq!(count, 2);
    doc.apply(&req.unwrap(), GroupHint::None).unwrap();
    assert_eq!(
        doc.text(TextPos::ZERO..TextPos::new(doc.line_count(), 0)),
        "a\r\nb\r\nc\r\nd\r\n"
    );
}

#[test]
fn replace_all_is_one_step_with_its_count() {
    let text = "foo bar foo\nbaz\nfoofoo foo\n";
    let mut doc = editable(text);
    let f = finder(&query("foo", false, true, true), "");
    let with = Replacement::new("qux", &f).unwrap();
    let (req, count) = replace_all(&doc, &f, &with, None, &Ctl::detached()).unwrap();
    assert_eq!(count, 3);
    let steps = doc.undo_steps();
    doc.apply(&req.unwrap(), GroupHint::None).unwrap();
    assert_eq!(doc.undo_steps(), steps + 1);
    let all_text = |d: &EditableDocument| d.text(TextPos::ZERO..TextPos::new(d.line_count(), 0));
    assert_eq!(all_text(&doc), "qux bar qux\nbaz\nfoofoo qux\n");
    doc.undo().unwrap();
    assert_eq!(all_text(&doc), text);
    // A scope: only the hits inside it.
    let (req, count) = replace_all(
        &doc,
        &f,
        &with,
        Some(pos(0, 1)..pos(2, 0)),
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!(count, 1);
    doc.apply(&req.unwrap(), GroupHint::None).unwrap();
    assert_eq!(all_text(&doc), "foo bar qux\nbaz\nfoofoo foo\n");
    // Nothing to replace: no request.
    let none = finder(&query("zzz", false, true, false), "");
    let with = Replacement::new("y", &none).unwrap();
    assert_eq!(
        replace_all(&doc, &none, &with, None, &Ctl::detached()).unwrap(),
        (None, 0)
    );
}

#[test]
fn a_stale_hit_is_refused() {
    let mut doc = editable("foo\n");
    let f = finder(&query("foo", false, true, false), "");
    let hit: Hit = f
        .next(&doc, TextPos::ZERO, Dir::Forward, false)
        .unwrap()
        .unwrap();
    doc.apply(
        &EditRequest::Replace {
            range: pos(0, 0)..pos(0, 1),
            text: "g".into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
    let with = Replacement::new("x", &f).unwrap();
    assert!(replace_one(&doc, &f, &hit, &with).is_err());
}

#[test]
fn a_cancelled_search_says_so() {
    let doc = editable(&"abc\n".repeat(1000));
    let cancel = hikasa_core::CancelToken::new();
    cancel.cancel();
    let progress = hikasa_core::NullProgress;
    let ctl = Ctl {
        cancel: &cancel,
        progress: &progress,
    };
    let f = finder(&query("abc", false, true, false), "");
    assert_eq!(f.all(&doc, 10, &ctl).unwrap_err(), FindError::Cancelled);
}

#[test]
fn the_current_word_starts_a_search() {
    let doc = editable("let alpha = beta;\n");
    let word = hikasa_text::edit::word_at(&doc, pos(0, 6));
    assert_eq!(doc.text(word), "alpha");
}

#[test]
fn windows_are_seamless() {
    // About 3.5 MiB: several 1 MiB windows, hits on every side of their edges, one physical line of 1.5 MiB cut by
    // CPL into many pieces, regex and plain, forward and backward.
    let mut rng = common::XorShift(0x5EED_F1ED);
    let mut text = String::new();
    while text.len() < 2 << 20 {
        let n = rng.below(120) as usize;
        for _ in 0..n {
            text.push(if rng.below(40) == 0 { 'n' } else { 'x' });
        }
        if rng.below(7) == 0 {
            text.push_str("needle");
        }
        text.push_str(["\n", "\r\n", "\r"][rng.below(3) as usize]);
    }
    for k in 0..300_000 {
        text.push(if k % 997 == 0 { 'N' } else { 'y' });
        if k % 4999 == 0 {
            text.push_str("needle");
        }
    }
    text.push('\n');
    text.push_str("needle at the end");
    for cpl in [None, Some(1000)] {
        let doc = editable_cpl(&text, cpl);
        let abs =
            |p: TextPos| doc.document().line_byte_range(p.line).unwrap().start + p.byte as usize;
        let expected: Vec<usize> = text.match_indices("needle").map(|(k, _)| k).collect();
        for regex in [false, true] {
            let f = finder(&query("needle", regex, true, false), "");
            let got: Vec<usize> = f
                .all(&doc, u64::MAX, &Ctl::detached())
                .unwrap()
                .iter()
                .map(|h| abs(h.range.start))
                .collect();
            assert_eq!(got, expected, "cpl {cpl:?} regex {regex}");
            let mut back = Vec::new();
            let mut at = TextPos::new(doc.line_count(), 0);
            while let Some(h) = f.next(&doc, at, Dir::Backward, false).unwrap() {
                at = h.range.start;
                back.push(abs(h.range.start));
            }
            back.reverse();
            assert_eq!(back, expected, "backward, cpl {cpl:?} regex {regex}");
        }
        let f = finder(&query("NEEDLE", false, false, false), "");
        assert_eq!(
            f.all(&doc, u64::MAX, &Ctl::detached()).unwrap().len(),
            expected.len()
        );
    }
}
