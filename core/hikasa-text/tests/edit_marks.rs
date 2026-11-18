//! A10.4 (D-263): Original / Edited / SavedEdit through edit, save, edit, undo past the save (fresh generation, no
//! file line), redo; `edited_ranges` both ways; bookmarks: toggle, move, removal of their line, undo restores,
//! `clear_bookmarks`.

use std::sync::Arc;

use hikasa_text::edit::{
    Dir, EditOptions, EditRequest, Editability, EditableDocument, GroupHint, LineMark, TextPos,
};
use hikasa_text::{LineIdx, TextDocument};

fn doc(text: &str) -> EditableDocument {
    EditableDocument::new(
        Arc::new(TextDocument::from_text(text)),
        EditOptions::default(),
        Editability::EDITABLE,
    )
}

fn put(d: &mut EditableDocument, a: (u32, u32), b: (u32, u32), text: &str) {
    d.apply(
        &EditRequest::Replace {
            range: TextPos::new(a.0, a.1)..TextPos::new(b.0, b.1),
            text: text.into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
}

fn marks(d: &EditableDocument) -> Vec<LineMark> {
    (0..d.line_count()).map(|l| d.mark(LineIdx(l))).collect()
}

fn origs(d: &EditableDocument) -> Vec<Option<u32>> {
    (0..d.line_count())
        .map(|l| d.marks(LineIdx(l)).orig.map(|o| o.0))
        .collect()
}

use LineMark::{Edited as E, Original as O, SavedEdit as S};

#[test]
fn marks_through_edit_save_edit_undo_redo() {
    let mut d = doc("a\nb\nc\nd\n");
    assert_eq!(marks(&d), [O, O, O, O]);
    put(&mut d, (1, 1), (1, 1), "1");
    assert_eq!(marks(&d), [O, E, O, O]);
    assert_eq!(origs(&d), [Some(0), None, Some(2), Some(3)]);
    // An inserted line is Edited; the lines after keep their file lines.
    put(&mut d, (3, 0), (3, 0), "new\n");
    assert_eq!(marks(&d), [O, E, O, E, O]);
    assert_eq!(origs(&d), [Some(0), None, Some(2), None, Some(3)]);
    // Save: Edited become SavedEdit, every line's file line is its index.
    d.saved(hikasa_text::LoadStamp {
        size: 0,
        mtime: None,
        crc32: 0,
    });
    assert!(!d.is_modified());
    assert_eq!(marks(&d), [O, S, O, S, O]);
    assert_eq!(origs(&d), [Some(0), Some(1), Some(2), Some(3), Some(4)]);
    put(&mut d, (0, 1), (0, 1), "2");
    assert_eq!(marks(&d), [E, S, O, S, O]);
    // Undo to the save point: marks as saved.
    d.undo();
    assert_eq!(marks(&d), [O, S, O, S, O]);
    assert_eq!(origs(&d), [Some(0), Some(1), Some(2), Some(3), Some(4)]);
    assert!(!d.is_modified());
    // Undo past the save: the restored lines differ from the file: a fresh generation and no file line.
    d.undo();
    assert_eq!(marks(&d), [O, S, O, O]);
    assert!(d.is_modified());
    // The line brought back by the undo of the insertion is gone; the lines left keep their saved file lines,
    // and the removal itself joined nothing.
    assert_eq!(origs(&d), [Some(0), Some(1), Some(2), Some(4)]);
    d.undo();
    assert_eq!(marks(&d), [O, E, O, O]);
    assert_eq!(origs(&d), [Some(0), None, Some(2), Some(4)]);
    assert_eq!(d.document().rope().to_string(), "a\nb\nc\nd\n");
    // Redo forward again: back at the save point.
    d.redo();
    d.redo();
    assert_eq!(marks(&d), [O, S, O, S, O]);
    assert!(!d.is_modified());
}

#[test]
fn edited_ranges_both_ways() {
    let text: String = (0..20).map(|k| format!("{k}\n")).collect();
    let mut d = doc(&text);
    for line in [3, 4, 5, 10, 15, 16] {
        put(&mut d, (line, 0), (line, 0), "x");
    }
    assert_eq!(d.edited_ranges(LineIdx(0), Dir::Forward), Some(3..6));
    assert_eq!(d.edited_ranges(LineIdx(4), Dir::Forward), Some(10..11));
    assert_eq!(d.edited_ranges(LineIdx(10), Dir::Forward), Some(15..17));
    assert_eq!(d.edited_ranges(LineIdx(15), Dir::Forward), None);
    assert_eq!(d.edited_ranges(LineIdx(19), Dir::Backward), Some(15..17));
    assert_eq!(d.edited_ranges(LineIdx(16), Dir::Backward), Some(10..11));
    assert_eq!(d.edited_ranges(LineIdx(3), Dir::Backward), None);
    // SavedEdit lines count too.
    d.saved(hikasa_text::LoadStamp {
        size: 0,
        mtime: None,
        crc32: 0,
    });
    assert_eq!(d.edited_ranges(LineIdx(0), Dir::Forward), Some(3..6));
}

#[test]
fn bookmarks_follow_their_lines_and_undo_restores_them() {
    let mut d = doc("a\nb\nc\nd\ne\n");
    d.toggle_bookmark(1, LineIdx(1));
    d.toggle_bookmark(2, LineIdx(3));
    assert_eq!(d.bookmark(1), Some(LineIdx(1)));
    // Toggling on its own line removes it; elsewhere moves it.
    d.toggle_bookmark(1, LineIdx(1));
    assert_eq!(d.bookmark(1), None);
    d.toggle_bookmark(1, LineIdx(2));
    d.toggle_bookmark(1, LineIdx(1));
    assert_eq!(d.bookmark(1), Some(LineIdx(1)));
    // Lines inserted above move them.
    put(&mut d, (0, 0), (0, 0), "x\ny\n");
    assert_eq!(
        (d.bookmark(1), d.bookmark(2)),
        (Some(LineIdx(3)), Some(LineIdx(5)))
    );
    // A removed line's bookmark moves to the start of the removal (the joined line).
    put(&mut d, (2, 1), (5, 0), "");
    assert_eq!(d.bookmark(1), Some(LineIdx(2)));
    assert_eq!(d.bookmark(2), Some(LineIdx(2)));
    // Undo puts them back.
    d.undo();
    assert_eq!(
        (d.bookmark(1), d.bookmark(2)),
        (Some(LineIdx(3)), Some(LineIdx(5)))
    );
    d.undo();
    assert_eq!(
        (d.bookmark(1), d.bookmark(2)),
        (Some(LineIdx(1)), Some(LineIdx(3)))
    );
    let all = d.bookmarks();
    assert_eq!(all.iter().filter(|b| b.is_some()).count(), 2);
    d.clear_bookmarks();
    assert!(d.bookmarks().iter().all(Option::is_none));
    // A line split at its start keeps the bookmark on the moved text.
    d.toggle_bookmark(0, LineIdx(2));
    put(&mut d, (2, 0), (2, 0), "\n");
    assert_eq!(d.bookmark(0), Some(LineIdx(3)));
}

/// The naive runs of edited lines (one line at a time).
fn naive_ranges(d: &EditableDocument, from: u32, dir: Dir) -> Option<std::ops::Range<u32>> {
    let n = d.line_count();
    let marked = |l: u32| d.mark(LineIdx(l)) != O;
    match dir {
        Dir::Forward => {
            let mut l = from + 1;
            if from < n && marked(from) {
                while l < n && marked(l) {
                    l += 1;
                }
            }
            let start = (l..n).find(|&k| marked(k))?;
            let mut end = start + 1;
            while end < n && marked(end) {
                end += 1;
            }
            Some(start..end)
        }
        Dir::Backward => {
            let mut l = from.min(n);
            if from < n && marked(from) {
                while l > 0 && marked(l - 1) {
                    l -= 1;
                }
            }
            let end = (0..l).rev().find(|&k| marked(k))? + 1;
            let mut start = end - 1;
            while start > 0 && marked(start - 1) {
                start -= 1;
            }
            Some(start..end)
        }
    }
}

/// M2 W11c: a run that starts or ends exactly at a block's edge, beside an unmarked block (the block-wise walk stops
/// there, neither a line early nor late).
#[test]
fn runs_at_block_edges() {
    let n = 2_048u32;
    let text: String = (0..n).map(|k| format!("{k}\n")).collect();
    let mut d = doc(&text);
    // The first block (512 lines) stays unmarked; lines 512..600 edited in place.
    let lines: String = (512..600).map(|k| format!("e{k}\n")).collect();
    put(&mut d, (512, 0), (600, 0), &lines);
    for (from, dir) in [
        (700, Dir::Backward),
        (300, Dir::Forward),
        (550, Dir::Backward),
        (2_000, Dir::Backward),
    ] {
        assert_eq!(
            d.edited_ranges(LineIdx(from), dir),
            naive_ranges(&d, from, dir),
            "{from} {dir:?}"
        );
    }
    assert_eq!(d.edited_ranges(LineIdx(700), Dir::Backward), Some(512..600));
    assert_eq!(d.edited_run_end(LineIdx(512)), 600);
}

mod runs {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// M2 W11c: the block-wise runs (whole edited blocks skipped) equal the line-by-line ones, with runs that span
        /// several blocks, start or end at a block's edge, and cover the whole document.
        #[test]
        fn edited_ranges_equal_the_line_by_line_scan(
            runs in prop::collection::vec((0u32..3_000, 1u32..1_500), 0..6),
            all in any::<bool>(),
            probes in prop::collection::vec(0u32..3_200, 8),
        ) {
            let n = 3_000u32;
            let text: String = (0..n).map(|k| format!("{k}\n")).collect();
            let mut d = doc(&text);
            let mut runs = runs;
            if all {
                runs = vec![(0, n)];
            }
            for (start, len) in runs {
                let end = (start + len).min(n);
                if start < end {
                    // Every line of the run edited, in one replace that keeps the line count.
                    let lines: String = (start..end).map(|k| format!("e{k}\n")).collect();
                    put(&mut d, (start, 0), (end, 0), &lines);
                }
            }
            for from in probes.into_iter().chain([0, 1, n - 1, n]) {
                for dir in [Dir::Forward, Dir::Backward] {
                    prop_assert_eq!(d.edited_ranges(LineIdx(from), dir), naive_ranges(&d, from, dir), "{} {:?}", from, dir);
                }
                if from < n {
                    let mut end = from;
                    while end < n && d.mark(LineIdx(end)) != O {
                        end += 1;
                    }
                    prop_assert_eq!(d.edited_run_end(LineIdx(from)), end);
                }
            }
        }
    }
}
