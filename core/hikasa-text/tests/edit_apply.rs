//! A10.2 (D-261, D-290): §1.10.3's rule table — joins keep the later terminator, pasted breaks become the insert
//! terminator, `keep_eols`, Enter at the end of an open last line, edits across artificial breaks re-wrap,
//! `TooLarge` and `BadRange` change nothing, `insert_eol` from the dominant kind and from `default_eol`.

use std::sync::Arc;

use hikasa_text::edit::{
    EditError, EditOptions, EditRequest, Editability, EditableDocument, GroupHint, LineDelta,
    Selection, TextPos,
};
use hikasa_text::{LineEnding, LineIdx, TextDocument};

fn doc(text: &str) -> EditableDocument {
    doc_with(text, EditOptions::default())
}

fn doc_with(text: &str, opts: EditOptions) -> EditableDocument {
    EditableDocument::new(
        Arc::new(TextDocument::from_text(text)),
        opts,
        Editability::EDITABLE,
    )
}

fn replace(
    d: &mut EditableDocument,
    a: (u32, u32),
    b: (u32, u32),
    text: &str,
    keep: bool,
) -> Result<hikasa_text::EditOutcome, EditError> {
    d.apply(
        &EditRequest::Replace {
            range: TextPos::new(a.0, a.1)..TextPos::new(b.0, b.1),
            text: text.into(),
            keep_eols: keep,
        },
        GroupHint::None,
    )
}

fn text(d: &EditableDocument) -> String {
    d.document().rope().to_string()
}

/// Lines as (content, terminator).
fn lines(d: &EditableDocument) -> Vec<(String, LineEnding)> {
    (0..d.line_count())
        .map(|l| (d.line(LineIdx(l)).into_owned(), d.line_ending(LineIdx(l))))
        .collect()
}

#[test]
fn a_join_keeps_the_later_terminator() {
    let mut d = doc("a\r\nb\nc\r");
    let out = replace(&mut d, (0, 1), (1, 0), "", false).unwrap();
    assert_eq!(text(&d), "ab\nc\r");
    assert_eq!(lines(&d)[0], ("ab".into(), LineEnding::Lf));
    assert_eq!(
        out.delta,
        LineDelta {
            at: 0,
            removed: 2,
            inserted: 1
        }
    );
    assert_eq!(out.selection, Selection::caret(TextPos::new(0, 1)));
    // Across three lines: the last line's terminator stays.
    let mut d = doc("x1\r\nx2\nx3\r");
    replace(&mut d, (0, 1), (2, 1), "-", false).unwrap();
    assert_eq!(text(&d), "x-3\r");
}

#[test]
fn pasted_breaks_take_the_insert_terminator_unless_kept() {
    let mut d = doc("one\ntwo\n");
    assert_eq!(d.insert_eol(), LineEnding::Lf);
    replace(&mut d, (1, 0), (1, 0), "a\r\nb\rc\n", false).unwrap();
    assert_eq!(text(&d), "one\na\nb\nc\ntwo\n");
    let mut d = doc("one\ntwo\n");
    let out = replace(&mut d, (1, 0), (1, 0), "a\r\nb\rc\n", true).unwrap();
    assert_eq!(text(&d), "one\na\r\nb\rc\ntwo\n");
    assert_eq!(
        out.delta,
        LineDelta {
            at: 1,
            removed: 0,
            inserted: 3
        }
    );
    // A document of LF lines stays LF after a pasted CRLF block (insert_eol does not change).
    assert_eq!(d.insert_eol(), LineEnding::Lf);
    // Splitting a line: the head takes insert_eol, the tail keeps the line's terminator.
    let mut d = doc("abcd\r\nz\n");
    replace(&mut d, (0, 2), (0, 2), "x\ny", false).unwrap();
    assert_eq!(
        lines(&d)[..2],
        [
            ("abx".to_owned(), LineEnding::Lf),
            ("ycd".to_owned(), LineEnding::CrLf)
        ]
    );
}

#[test]
fn enter_at_the_end_of_an_open_last_line() {
    let mut d = doc("x");
    assert_eq!(d.position_lines(), 1);
    let out = replace(&mut d, (0, 1), (0, 1), "\n", false).unwrap();
    assert_eq!(text(&d), "x\n");
    assert_eq!(d.line_count(), 1);
    assert_eq!(d.position_lines(), 2);
    assert_eq!(out.selection.caret, TextPos::new(1, 0));
    // Typing on the empty last line makes it a real (open) line.
    replace(&mut d, (1, 0), (1, 0), "y", false).unwrap();
    assert_eq!(text(&d), "x\ny");
    assert_eq!(lines(&d)[1], ("y".into(), LineEnding::None));
    // An empty document has one empty line for positions.
    let mut e = doc("");
    assert_eq!((e.line_count(), e.position_lines()), (0, 1));
    replace(&mut e, (0, 0), (0, 0), "hi", false).unwrap();
    assert_eq!(text(&e), "hi");
}

#[test]
fn edits_across_artificial_breaks_rewrap_the_physical_line() {
    let opts = EditOptions {
        chars_per_line_limit: Some(3),
        ..EditOptions::default()
    };
    let mut d = doc_with("", opts);
    replace(&mut d, (0, 0), (0, 0), "abcdefg\nxy\n", false).unwrap();
    let wrapped = |d: &EditableDocument| -> Vec<(String, LineEnding)> { lines(d) };
    assert_eq!(
        wrapped(&d),
        [
            ("abc".to_owned(), LineEnding::Artificial),
            ("def".to_owned(), LineEnding::Artificial),
            ("g".to_owned(), LineEnding::Lf),
            ("xy".to_owned(), LineEnding::Lf),
        ]
    );
    // An insert in the second piece re-wraps the whole physical line.
    replace(&mut d, (1, 0), (1, 0), "X", false).unwrap();
    assert_eq!(
        wrapped(&d)[..3],
        [
            ("abc".to_owned(), LineEnding::Artificial),
            ("Xde".to_owned(), LineEnding::Artificial),
            ("fg".to_owned(), LineEnding::Lf),
        ]
    );
    // A range ending at an artificial break is the one ending at the next piece's start.
    let mut a = d.clone();
    replace(&mut a, (0, 1), (0, 3), "", false).unwrap();
    let mut b = d.clone();
    replace(&mut b, (0, 1), (1, 0), "", false).unwrap();
    assert_eq!(text(&a), text(&b));
    assert_eq!(text(&a), "aXdefg\nxy\n");
    d.document().check_line_table(Some(3)).unwrap();
    assert_eq!(d.document().physical_line(LineIdx(3)), 2);
}

#[test]
fn too_large_and_bad_ranges_change_nothing() {
    let mut d = doc("abc\nd\n").with_limits(10, 3);
    let before = text(&d);
    assert_eq!(
        replace(&mut d, (0, 0), (0, 0), "123456", false).unwrap_err(),
        EditError::TooLarge
    );
    assert_eq!(
        replace(&mut d, (0, 0), (0, 0), "\n\n", false).unwrap_err(),
        EditError::TooLarge
    );
    assert!(replace(&mut d, (0, 0), (0, 0), "1234", false).is_ok());
    d.undo();
    assert_eq!(text(&d), before);
    // Past the content, inside a char, beyond the last position line.
    let mut d = doc("é\nx");
    for (a, b) in [
        ((0, 3), (0, 3)),
        ((0, 1), (0, 1)),
        ((2, 0), (2, 0)),
        ((1, 0), (5, 0)),
    ] {
        assert_eq!(
            replace(&mut d, a, b, "z", false).unwrap_err(),
            EditError::BadRange,
            "{a:?} {b:?}"
        );
    }
    assert_eq!(text(&d), "é\nx");
    assert!(!d.is_modified());
    assert_eq!(d.generation(), 0);
}

#[test]
fn the_insert_terminator() {
    // The dominant kind of the loaded document (ties Lf > CrLf > Cr).
    assert_eq!(doc("a\r\nb\r\nc\n").insert_eol(), LineEnding::CrLf);
    assert_eq!(doc("a\rb\r\nc\n").insert_eol(), LineEnding::Lf);
    assert_eq!(doc("a\rb\rc\r\n").insert_eol(), LineEnding::Cr);
    // No terminated line: the configured default (D-290).
    let opts = EditOptions {
        default_eol: LineEnding::CrLf,
        ..EditOptions::default()
    };
    let mut d = doc_with("open", opts.clone());
    assert_eq!(d.insert_eol(), LineEnding::CrLf);
    replace(&mut d, (0, 4), (0, 4), "\n", false).unwrap();
    assert_eq!(text(&d), "open\r\n");
    assert_eq!(doc_with("", opts).insert_eol(), LineEnding::CrLf);
    // A conversion of every line changes it.
    let mut d = doc("a\nb\n");
    d.apply(
        &EditRequest::SetEols {
            lines: 0..2,
            eol: LineEnding::Cr,
        },
        GroupHint::None,
    )
    .unwrap();
    assert_eq!(d.insert_eol(), LineEnding::Cr);
    assert_eq!(text(&d), "a\rb\r");
    d.undo();
    assert_eq!(d.insert_eol(), LineEnding::Lf);
}

#[test]
fn other_separators_and_controls_are_characters() {
    let mut d = doc("a\n");
    replace(
        &mut d,
        (0, 1),
        (0, 1),
        "\u{0B}\u{0C}\u{85}\u{2028}\u{2029}\u{0}",
        false,
    )
    .unwrap();
    assert_eq!(d.line_count(), 1);
    assert_eq!(
        d.line(LineIdx(0)),
        "a\u{0B}\u{0C}\u{85}\u{2028}\u{2029}\u{0}"
    );
}

#[test]
fn a_lone_cr_meeting_an_lf_joins_as_a_load_would() {
    // "a\r" + "b\n": deleting "b" leaves "a\r\n", which a fresh load reads as one CRLF line.
    let mut d = doc("a\rb\n");
    replace(&mut d, (1, 0), (1, 1), "", false).unwrap();
    assert_eq!(text(&d), "a\r\n");
    assert_eq!(lines(&d), [("a".to_owned(), LineEnding::CrLf)]);
    d.document().check_line_table(None).unwrap();
    d.undo();
    assert_eq!(
        lines(&d),
        [
            ("a".to_owned(), LineEnding::Cr),
            ("b".to_owned(), LineEnding::Lf)
        ]
    );
}

#[test]
fn set_encoding_is_an_undoable_property() {
    let mut d = doc("x\n");
    assert_eq!(d.encoding(), (hikasa_text::TextEncoding::UTF_8, false));
    let out = d
        .apply(
            &EditRequest::SetEncoding {
                encoding: hikasa_text::TextEncoding::UTF_16BE,
                bom: true,
            },
            GroupHint::None,
        )
        .unwrap();
    assert_eq!(out.delta, LineDelta::NONE);
    assert!(d.is_modified());
    assert_eq!(d.encoding(), (hikasa_text::TextEncoding::UTF_16BE, true));
    d.undo();
    assert_eq!(d.encoding(), (hikasa_text::TextEncoding::UTF_8, false));
    assert!(!d.is_modified());
}

#[test]
fn a_batch_is_one_step() {
    let mut d = doc("aa bb aa\naa\n");
    let out = d
        .apply(
            &EditRequest::Batch {
                edits: vec![
                    (TextPos::new(0, 0)..TextPos::new(0, 2), "X".into()),
                    (TextPos::new(0, 6)..TextPos::new(0, 8), "Y\nZ".into()),
                    (TextPos::new(1, 0)..TextPos::new(1, 2), "W".into()),
                ],
                keep_eols: false,
            },
            GroupHint::None,
        )
        .unwrap();
    assert_eq!(text(&d), "X bb Y\nZ\nW\n");
    assert_eq!(out.selection.caret, TextPos::new(2, 1));
    assert_eq!(
        out.delta,
        LineDelta {
            at: 0,
            removed: 2,
            inserted: 3
        }
    );
    d.undo();
    assert_eq!(text(&d), "aa bb aa\naa\n");
    // Overlapping ranges are refused.
    let err = d
        .apply(
            &EditRequest::Batch {
                edits: vec![
                    (TextPos::new(0, 0)..TextPos::new(0, 4), "X".into()),
                    (TextPos::new(0, 2)..TextPos::new(0, 5), "Y".into()),
                ],
                keep_eols: false,
            },
            GroupHint::None,
        )
        .unwrap_err();
    assert_eq!(err, EditError::BadRange);
}
