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
