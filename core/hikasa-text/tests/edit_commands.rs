//! A10.5 (D-264): one row per command of §1.10.6 (tab and space formats, auto indent on and off, backspace
//! unindent on and off, overwrite at a line end and over a combining mark, word commands with the format's word
//! characters, smart Home, `select_section`); `copy_rows` makes the rows equal (a fresh compare shows them same)
//! for change, insert and delete sections and a single line, as one step.

use std::sync::Arc;

use hikasa_core::{Ctl, Side};
use hikasa_text::edit::{
    Dir, EditCommand, EditOptions, Editability, EditableDocument, GroupHint, Selection, TextPos,
    copy_rows, line_home, plan_command, select_section, word_at, word_boundary,
};
use hikasa_text::{
    CompareInput, CompiledFormat, LineIdx, RowIdx, RowState, TextComparison, TextDocument,
    TextRules, compare,
};

fn doc_with(text: &str, opts: EditOptions) -> EditableDocument {
    EditableDocument::new(
        Arc::new(TextDocument::from_text(text)),
        opts,
        Editability::EDITABLE,
    )
}

fn spaces() -> EditOptions {
    EditOptions {
        tab_as_spaces: true,
        tab_size: 4,
        ..EditOptions::default()
    }
}

/// Runs `cmd` at `sel` and returns the text and the planned caret.
fn run(text: &str, opts: EditOptions, sel: Selection, cmd: EditCommand) -> (String, Selection) {
    let mut d = doc_with(text, opts);
    let Some((req, after)) = plan_command(&d, sel, &cmd) else {
        return (text.to_owned(), sel);
    };
    let out = d.apply(&req, GroupHint::None).unwrap();
    // One step.
    assert_eq!(d.undo_steps(), 1);
    let _ = out;
    (d.document().rope().to_string(), after)
}

fn at(line: u32, byte: u32) -> Selection {
    Selection::caret(TextPos::new(line, byte))
}

fn span(a: (u32, u32), b: (u32, u32)) -> Selection {
    Selection {
        anchor: TextPos::new(a.0, a.1),
        caret: TextPos::new(b.0, b.1),
    }
}

#[test]
fn command_table() {
    let tabs = EditOptions::default;
    let no_auto = EditOptions {
        auto_indent: false,
        backspace_unindents: false,
        ..EditOptions::default()
    };
    type Row = (
        &'static str,
        EditOptions,
        Selection,
        EditCommand,
        &'static str,
        Selection,
    );
    let t = |s: &str| EditCommand::Type {
        text: s.into(),
        overwrite: false,
    };
    let rows: Vec<Row> = vec![
        ("ab\n", tabs(), at(0, 1), t("X"), "aXb\n", at(0, 2)),
        (
            "abc\n",
            tabs(),
            span((0, 0), (0, 2)),
            t("Z"),
            "Zc\n",
            at(0, 1),
        ),
        // Overwrite replaces as many grapheme clusters as typed, never the terminator.
        (
            "ab\n",
            tabs(),
            at(0, 1),
            EditCommand::Type {
                text: "XY".into(),
                overwrite: true,
            },
            "aXY\n",
            at(0, 3),
        ),
        (
            "e\u{301}x\n",
            tabs(),
            at(0, 0),
            EditCommand::Type {
                text: "Q".into(),
                overwrite: true,
            },
            "Qx\n",
            at(0, 1),
        ),
        (
            "ab\n",
            tabs(),
            at(0, 2),
            EditCommand::Type {
                text: "Q".into(),
                overwrite: true,
            },
            "abQ\n",
            at(0, 3),
        ),
        (
            "ab\n",
            tabs(),
            at(0, 1),
            EditCommand::Paste {
                text: "1\r\n2".into(),
            },
            "a1\n2b\n",
            at(1, 1),
        ),
        // New line with and without auto indent; the indent is taken only up to the caret.
        (
            "\t  x\n",
            tabs(),
            at(0, 4),
            EditCommand::NewLine,
            "\t  x\n\t  \n",
            at(1, 3),
        ),
        (
            "\t  x\n",
            tabs(),
            at(0, 2),
            EditCommand::NewLine,
            "\t \n\t  x\n",
            at(1, 2),
        ),
        (
            "\t  x\n",
            no_auto.clone(),
            at(0, 4),
            EditCommand::NewLine,
            "\t  x\n\n",
            at(1, 0),
        ),
        // Tab: a tab, or spaces to the next tab stop of the display column.
        (
            "ab\n",
            tabs(),
            at(0, 1),
            EditCommand::Tab,
            "a\tb\n",
            at(0, 2),
        ),
        (
            "ab\n",
            spaces(),
            at(0, 1),
            EditCommand::Tab,
            "a   b\n",
            at(0, 4),
        ),
        (
            "\tab\n",
            spaces(),
            at(0, 2),
            EditCommand::Tab,
            "\ta   b\n",
            at(0, 5),
        ),
        // Tab over several lines indents them (blank lines excepted).
        (
            "a\n\nb\n",
            tabs(),
            span((0, 0), (2, 1)),
            EditCommand::Tab,
            "\ta\n\n\tb\n",
            span((0, 0), (2, 2)),
        ),
        (
            "a\nb\n",
            spaces(),
            span((0, 1), (1, 0)),
            EditCommand::IncreaseIndent,
            "    a\nb\n",
            span((0, 5), (1, 0)),
        ),
        // Decrease: one tab, or up to tab_size spaces, or the spaces before a tab.
        (
            "\t\ta\n      b\n  \tc\n",
            tabs(),
            span((0, 0), (2, 1)),
            EditCommand::DecreaseIndent,
            "\ta\n  b\n\tc\n",
            span((0, 0), (2, 0)),
        ),
        (
            "  a\n",
            spaces(),
            at(0, 3),
            EditCommand::BackTab,
            "a\n",
            at(0, 1),
        ),
        // Backspace: unindent inside leading blanks, else one grapheme, else the terminator.
        (
            "        x\n",
            spaces(),
            at(0, 6),
            EditCommand::Backspace,
            "      x\n",
            at(0, 4),
        ),
        (
            "        x\n",
            no_auto.clone(),
            at(0, 6),
            EditCommand::Backspace,
            "       x\n",
            at(0, 5),
        ),
        (
            "ae\u{301}\n",
            tabs(),
            at(0, 4),
            EditCommand::Backspace,
            "a\n",
            at(0, 1),
        ),
        (
            "a\nb\n",
            tabs(),
            at(1, 0),
            EditCommand::Backspace,
            "ab\n",
            at(0, 1),
        ),
        (
            "a\n",
            tabs(),
            at(0, 0),
            EditCommand::Backspace,
            "a\n",
            at(0, 0),
        ),
        // Delete: one grapheme, or the terminator at a line end.
        (
            "e\u{301}x\n",
            tabs(),
            at(0, 0),
            EditCommand::Delete,
            "x\n",
            at(0, 0),
        ),
        (
            "a\r\nb\n",
            tabs(),
            at(0, 1),
            EditCommand::Delete,
            "ab\n",
            at(0, 1),
        ),
        // Lines of the selection with their terminators; the last line takes the terminator before it.
        (
            "a\nb\nc\n",
            tabs(),
            at(1, 1),
            EditCommand::DeleteLine,
            "a\nc\n",
            at(1, 0),
        ),
        (
            "a\nb\nc",
            tabs(),
            at(2, 1),
            EditCommand::DeleteLine,
            "a\nb",
            at(1, 0),
        ),
        (
            "a\nb\nc\n",
            tabs(),
            span((0, 1), (2, 0)),
            EditCommand::DeleteLine,
            "c\n",
            at(0, 0),
        ),
        (
            "abcd\n",
            tabs(),
            at(0, 2),
            EditCommand::DeleteToLineStart,
            "cd\n",
            at(0, 0),
        ),
        (
            "abcd\n",
            tabs(),
            at(0, 2),
            EditCommand::DeleteToLineEnd,
            "ab\n",
            at(0, 2),
        ),
        // Words: letters, digits, `_` and the format's word characters; between words, the blanks.
        (
            "foo_1 bar\n",
            tabs(),
            at(0, 2),
            EditCommand::DeleteWord,
            " bar\n",
            at(0, 0),
        ),
        (
            "foo   bar\n",
            tabs(),
            at(0, 4),
            EditCommand::DeleteWord,
            "foobar\n",
            at(0, 3),
        ),
        (
            "a-b c\n",
            EditOptions {
                word_chars: "-".into(),
                ..tabs()
            },
            at(0, 1),
            EditCommand::DeleteWord,
            " c\n",
            at(0, 0),
        ),
        (
            "foo bar\n",
            tabs(),
            at(0, 6),
            EditCommand::DeleteToWordStart,
            "foo r\n",
            at(0, 4),
        ),
        (
            "foo bar\n",
            tabs(),
            at(0, 1),
            EditCommand::DeleteToWordEnd,
            "fbar\n",
            at(0, 1),
        ),
        // Empty lines before and after, with auto indent.
        (
            "  x\n",
            tabs(),
            at(0, 3),
            EditCommand::InsertLineBefore,
            "  \n  x\n",
            at(0, 2),
        ),
        (
            "  x\n",
            tabs(),
            at(0, 1),
            EditCommand::InsertLineAfter,
            "  x\n  \n",
            at(1, 2),
        ),
        (
            "  x\n",
            no_auto,
            at(0, 1),
            EditCommand::InsertLineAfter,
            "  x\n\n",
            at(1, 0),
        ),
    ];
    for (k, (text, opts, sel, cmd, want, want_sel)) in rows.into_iter().enumerate() {
        let (got, got_sel) = run(text, opts, sel, cmd.clone());
        assert_eq!(got, want, "row {k}: {cmd:?}");
        assert_eq!(got_sel, want_sel, "row {k}: {cmd:?} selection");
    }
}

#[test]
fn word_positions_and_smart_home() {
    let d = doc_with("  foo_bar+=x  y\n", EditOptions::default());
    assert_eq!(
        word_at(&d, TextPos::new(0, 4)),
        TextPos::new(0, 2)..TextPos::new(0, 9)
    );
    assert_eq!(
        word_at(&d, TextPos::new(0, 9)),
        TextPos::new(0, 2)..TextPos::new(0, 9)
    );
    assert_eq!(
        word_at(&d, TextPos::new(0, 10)),
        TextPos::new(0, 10)..TextPos::new(0, 11)
    );
    let plus = doc_with(
        "  foo_bar+=x  y\n",
        EditOptions {
            word_chars: "+=".into(),
            ..EditOptions::default()
        },
    );
    assert_eq!(
        word_at(&plus, TextPos::new(0, 10)),
        TextPos::new(0, 2)..TextPos::new(0, 12)
    );
    assert_eq!(
        word_boundary(&d, TextPos::new(0, 2), Dir::Forward),
        TextPos::new(0, 9)
    );
    assert_eq!(
        word_boundary(&d, TextPos::new(0, 11), Dir::Forward),
        TextPos::new(0, 14)
    );
    assert_eq!(
        word_boundary(&d, TextPos::new(0, 15), Dir::Forward),
        TextPos::new(1, 0)
    );
    assert_eq!(
        word_boundary(&d, TextPos::new(0, 14), Dir::Backward),
        TextPos::new(0, 11)
    );
    assert_eq!(
        word_boundary(&d, TextPos::new(0, 9), Dir::Backward),
        TextPos::new(0, 2)
    );
    assert_eq!(line_home(&d, TextPos::new(0, 7)), TextPos::new(0, 2));
    assert_eq!(line_home(&d, TextPos::new(0, 2)), TextPos::new(0, 0));
    assert_eq!(line_home(&d, TextPos::new(0, 0)), TextPos::new(0, 2));
}

fn cmp_of(left: &str, right: &str) -> TextComparison {
    compare(
        CompareInput {
            left: Arc::new(TextDocument::from_text(left)),
            right: Arc::new(TextDocument::from_text(right)),
            format: Arc::new(CompiledFormat::everything_else()),
            rules: TextRules::default(),
            bytes_identical: false,
            force_tokenize: false,
        },
        None,
        &Ctl::detached(),
        None,
    )
    .unwrap()
}

#[test]
fn select_section_takes_the_side_lines() {
    let cmp = cmp_of("a\nb\nc\nd\n", "a\nB\nC\nd\n");
    assert_eq!(
        select_section(&cmp, Side::Left, LineIdx(2)),
        Some(TextPos::new(1, 0)..TextPos::new(3, 0))
    );
    assert_eq!(
        select_section(&cmp, Side::Right, LineIdx(1)),
        Some(TextPos::new(1, 0)..TextPos::new(3, 0))
    );
    assert_eq!(select_section(&cmp, Side::Left, LineIdx(0)), None);
    let open = cmp_of("a\nb", "a\nc");
    assert_eq!(
        select_section(&open, Side::Left, LineIdx(1)),
        Some(TextPos::new(1, 0)..TextPos::new(1, 1))
    );
}

/// Copies the rows of the section holding row `row` (or the single row when `single`) from `from` and checks
/// that a fresh comparison shows every row same.
fn copy_and_check(left: &str, right: &str, row: u32, single: bool, from: Side) {
    let cmp = cmp_of(left, right);
    let rows = if single {
        row..row + 1
    } else {
        let id = cmp.section_at(RowIdx(row)).unwrap();
        cmp.sections()[id.0 as usize].rows.clone()
    };
    let edit = copy_rows(&cmp, rows.clone(), from).unwrap();
    let target_text = if from == Side::Left { right } else { left };
    let mut target = doc_with(target_text, EditOptions::default());
    let out = target.apply(&edit.request, GroupHint::None).unwrap();
    assert_eq!(target.undo_steps(), 1);
    let _ = out;
    let new = target.document().rope().to_string();
    let after = if from == Side::Left {
        cmp_of(left, &new)
    } else {
        cmp_of(&new, right)
    };
    for r in 0..after.row_count() {
        let row = after.row(RowIdx(r)).unwrap();
        assert_eq!(
            row.state,
            RowState::Same,
            "{left:?} {right:?} rows {rows:?}: row {r} of {new:?}"
        );
    }
}

#[test]
fn copy_rows_makes_them_equal() {
    // Change, insert (rows without a target line) and delete (rows without a source line) sections.
    copy_and_check("a\nb\nc\nd\n", "a\nB\nC\nd\n", 1, false, Side::Left);
    copy_and_check(
        "a\nbeta1\ngamma\nd\n",
        "a\nbeta2\ngamma2\nd\n",
        1,
        false,
        Side::Left,
    );
    copy_and_check("a\nx\ny\nd\n", "a\nd\n", 1, false, Side::Left);
    copy_and_check("a\nd\n", "a\nx\ny\nd\n", 1, false, Side::Left);
    copy_and_check("x\na\n", "a\n", 0, false, Side::Left);
    copy_and_check("a\nb\nc\n", "a\nB\nc\n", 1, false, Side::Right);
    // A single line (Copy Line), and open last lines on either side.
    copy_and_check("a\nline one\nc\n", "a\nline two\nc\n", 1, true, Side::Left);
    copy_and_check("a\nb", "a\nB", 1, false, Side::Left);
    copy_and_check("a\nb\nz", "a\nb", 2, false, Side::Left);
    copy_and_check("a\nb", "a\nb\nz", 2, false, Side::Left);
    copy_and_check("q\na", "a", 0, false, Side::Left);
    copy_and_check("a\nb\n", "a\nb\nz", 2, false, Side::Left);
}
