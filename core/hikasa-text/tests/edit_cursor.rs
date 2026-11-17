//! A11.2 (core part, D-295) and A11.10 (`set_options`, D-304): `next_pos`/`prev_pos` against a grapheme scan of the
//! text, `doc_end`, `lines_text` and `delete_lines`; `EditableDocument::set_options` with another characters-per-line
//! limit re-wraps, maps positions, keeps marks and bookmarks, adds no undo step, and P-EDIT-1 holds after it.

mod common;

use std::sync::Arc;

use common::config;
use hikasa_text::edit::{
    EditOptions, EditRequest, Editability, EditableDocument, GroupHint, LineSet, TextPos,
    char_width, delete_lines, doc_end, lines_text, next_pos, prev_pos,
};
use hikasa_text::{LineEnding, LineIdx, LineMark, TextDocument};
use proptest::prelude::*;
use unicode_segmentation::UnicodeSegmentation;

fn editable(text: &str, opts: EditOptions) -> EditableDocument {
    EditableDocument::new(
        Arc::new(TextDocument::from_text(text)),
        opts,
        Editability::default(),
    )
}

/// Every caret position of the document in order, by a scan of each line's grapheme clusters (the definition).
fn positions(doc: &EditableDocument) -> Vec<TextPos> {
    let mut out = Vec::new();
    for l in 0..doc.position_lines() {
        let line = doc.line(LineIdx(l));
        let artificial_before = l > 0 && doc.line_ending(LineIdx(l - 1)) == LineEnding::Artificial;
        for (k, _) in line.grapheme_indices(true) {
            // The start of a piece after an artificial break is the end of the piece before (one place).
            if !(k == 0 && artificial_before) {
                out.push(TextPos::new(l, k as u32));
            }
        }
        if !(line.is_empty() && artificial_before) {
            out.push(TextPos::new(l, line.len() as u32));
        }
    }
    out.dedup();
    out
}

const PIECES: [&str; 12] = [
    "a",
    "é",
    "e\u{301}",
    "👩\u{200d}💻",
    "🇮🇹",
    "\t",
    " ",
    "中",
    "\r\n",
    "\n",
    "\r",
    "x\u{20dd}",
];

fn text_of(picks: &[usize]) -> String {
    picks.iter().map(|&k| PIECES[k % PIECES.len()]).collect()
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/edit_cursor.txt"))]

    /// `next_pos` walks the positions of the scan one by one, `prev_pos` walks them back (with and without a
    /// characters-per-line limit).
    #[test]
    fn steps_follow_the_grapheme_scan(picks in prop::collection::vec(0usize..64, 0..40), cpl in prop::option::of(2u32..6)) {
        let opts = EditOptions { chars_per_line_limit: cpl, ..EditOptions::default() };
        let doc = editable(&text_of(&picks), opts);
        let all = positions(&doc);
        prop_assert_eq!(*all.last().unwrap(), doc_end(&doc));
        for w in all.windows(2) {
            prop_assert_eq!(next_pos(&doc, w[0]), w[1], "next of {:?}", w[0]);
            prop_assert_eq!(prev_pos(&doc, w[1]), w[0], "prev of {:?}", w[1]);
        }
        prop_assert_eq!(next_pos(&doc, doc_end(&doc)), doc_end(&doc));
        prop_assert_eq!(prev_pos(&doc, TextPos::ZERO), TextPos::ZERO);
    }

    /// `lines_text` is the text of the lines' ranges; `delete_lines` removes exactly them, as one step that undoes.
    #[test]
    fn line_sets_match_their_ranges(picks in prop::collection::vec(0usize..64, 1..40), a in 0u32..12, len in 1u32..5, b in 0u32..12) {
        let mut doc = editable(&text_of(&picks), EditOptions::default());
        let before = doc.document().rope().to_string();
        let set = LineSet::from_ranges(vec![a..a + len, b..b + 1]);
        let n = doc.line_count();
        let kept: Vec<String> = (0..n)
            .filter(|l| !set.0.contains(*l))
            .map(|l| format!("{}{}", doc.line(LineIdx(l)), doc.line_ending(LineIdx(l)).as_text()))
            .collect();
        let doc_before_eol: Vec<&str> = (0..n).map(|l| doc.line_ending(LineIdx(l)).as_text()).collect();
        let taken: String = (0..n)
            .filter(|l| set.0.contains(*l))
            .map(|l| format!("{}{}", doc.line(LineIdx(l)), doc.line_ending(LineIdx(l)).as_text()))
            .collect();
        prop_assert_eq!(lines_text(&doc, &set), taken);
        match delete_lines(&doc, &set) {
            None => prop_assert!(kept.len() as u32 == n),
            Some(req) => {
                doc.apply(&req, GroupHint::None).unwrap();
                let mut want: String = kept.concat();
                // A run through the last line took the terminator of the kept line before it.
                if set.0.contains(n - 1) {
                    let last_kept = (0..n).rev().find(|l| !set.0.contains(*l));
                    if let Some(l) = last_kept {
                        let eol = doc_before_eol[l as usize];
                        want.truncate(want.len() - eol.len());
                    }
                }
                prop_assert_eq!(doc.document().rope().to_string(), want);
                prop_assert_eq!(doc.undo_steps(), 1);
                doc.undo();
                prop_assert_eq!(doc.document().rope().to_string(), before);
            }
        }
    }
}

#[test]
fn set_options_rewraps_and_keeps_marks_and_bookmarks() {
    let text = "abcdefghij\nshort\n0123456789AB\n";
    let mut doc = editable(text, EditOptions::default());
    doc.toggle_bookmark(3, LineIdx(2));
    // Edit the second line: it is `Edited`.
    doc.apply(
        &EditRequest::Replace {
            range: TextPos::new(1, 5)..TextPos::new(1, 5),
            text: "!".into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
    let steps = doc.undo_steps();
    assert!(steps > 0);
    let generation = doc.generation();
    let map = doc.set_options(EditOptions {
        chars_per_line_limit: Some(4),
        ..EditOptions::default()
    });
    assert!(!map.is_identity());
    assert_eq!(
        doc.document().rope().to_string(),
        "abcdefghij\nshort!\n0123456789AB\n"
    );
    // 10 → 4+4+2, 6 → 4+2, 12 → 4+4+4.
    assert_eq!(doc.line_count(), 8);
    assert_eq!(
        doc.generation(),
        generation,
        "no undo step and no generation"
    );
    assert_eq!(
        doc.undo_steps(),
        0,
        "the log of the old wrapping is cleared"
    );
    assert!(doc.is_modified());
    // Positions keep their byte in the text.
    assert_eq!(map.pos(&doc, TextPos::new(0, 9)), TextPos::new(2, 1));
    assert_eq!(map.pos(&doc, TextPos::new(1, 6)), TextPos::new(4, 2));
    assert_eq!(map.line(&doc, 2), 5);
    assert_eq!(map.lines(&doc, 1..3), 3..8);
    // The edited physical line's pieces are edited; the others are not.
    assert_eq!(doc.mark(LineIdx(3)), LineMark::Edited);
    assert_eq!(doc.mark(LineIdx(4)), LineMark::Edited);
    assert_eq!(doc.mark(LineIdx(0)), LineMark::Original);
    assert_eq!(doc.bookmark(3), Some(LineIdx(5)));
    // P-EDIT-1 after it: an edit and its undo leave the document as it was.
    let before = doc.document().rope().to_string();
    let out = doc
        .apply(
            &EditRequest::Replace {
                range: TextPos::new(5, 1)..TextPos::new(6, 2),
                text: "Z\nY".into(),
                keep_eols: false,
            },
            GroupHint::None,
        )
        .unwrap();
    assert!(out.changed.end > out.changed.start);
    doc.undo();
    assert_eq!(doc.document().rope().to_string(), before);
    assert_eq!(doc.line_count(), 8);
    // The same limit again changes nothing.
    assert!(
        doc.set_options(EditOptions {
            chars_per_line_limit: Some(4),
            tab_size: 8,
            ..EditOptions::default()
        })
        .is_identity()
    );
    assert_eq!(doc.options().tab_size, 8);
}

#[test]
fn an_unmodified_document_takes_the_new_wrapping_as_its_files() {
    let mut doc = editable("abcdefgh\nxy\n", EditOptions::default());
    let map = doc.set_options(EditOptions {
        chars_per_line_limit: Some(3),
        ..EditOptions::default()
    });
    assert!(!map.is_identity());
    assert!(!doc.is_modified());
    assert!(
        !doc.spliceable(),
        "a text without a file has no splice source"
    );
    assert_eq!(doc.line_count(), 4);
    for l in 0..4 {
        assert_eq!(doc.mark(LineIdx(l)), LineMark::Original);
    }
    // Back to no limit.
    let back = doc.set_options(EditOptions::default());
    assert!(!back.is_identity());
    assert_eq!(doc.line_count(), 2);
    assert_eq!(back.pos(&doc, TextPos::new(2, 1)), TextPos::new(0, 7));
}

/// `char_width` (the status bar's display column, the test metrics): a tab counts 1 here, wide characters 2,
/// combining marks 0 (W11a review C4).
#[test]
fn char_widths() {
    assert_eq!(char_width('\t'), 1);
    assert_eq!(char_width('a'), 1);
    assert_eq!(char_width('中'), 2);
    assert_eq!(char_width('\u{301}'), 0);
}

/// Line sets over the pieces of a characters-per-line limit take whole physical lines (W11a review C6).
#[test]
fn line_sets_over_pieces_take_whole_physical_lines() {
    let doc = editable(
        "abcdefgh\nxy\n",
        EditOptions {
            chars_per_line_limit: Some(3),
            ..EditOptions::default()
        },
    );
    assert_eq!(doc.line_count(), 4, "abc def gh xy");
    let second_piece = LineSet::from_ranges(vec![1..2]);
    assert_eq!(lines_text(&doc, &second_piece), "abcdefgh\n");
    let mut d = doc.clone();
    let req = delete_lines(&d, &second_piece).expect("a deletion");
    d.apply(&req, GroupHint::None).expect("apply");
    assert_eq!(d.text(TextPos::ZERO..doc_end(&d)), "xy\n");
    let last = LineSet::from_ranges(vec![3..4]);
    assert_eq!(lines_text(&doc, &last), "xy\n");
}
