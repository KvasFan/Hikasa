//! A10.25 (D-287): details data (§1.10.20). Hex Details for UTF-8, UTF-16BE with a BOM, Shift_JIS, an unmappable
//! character, the cap; Alignment Details' classes equal `inline()`'s on the same row; the ruler.

mod common;

use std::sync::Arc;

use common::{compare_texts, format};
use hikasa_core::Side;
use hikasa_text::details::{CellClass, char_alignment, char_alignment_of_row, hex_details, ruler};
use hikasa_text::edit::{EditOptions, EditRequest, Editability, EditableDocument, GroupHint};
use hikasa_text::{LineIdx, RowIdx, TextDocument, TextEncoding, TextPos, TextRules};

fn editable(text: &str) -> EditableDocument {
    EditableDocument::new(
        Arc::new(TextDocument::from_text(text)),
        EditOptions::default(),
        Editability::EDITABLE,
    )
}

fn with_encoding(text: &str, encoding: TextEncoding, bom: bool) -> EditableDocument {
    let mut doc = editable(text);
    doc.apply(&EditRequest::SetEncoding { encoding, bom }, GroupHint::None)
        .unwrap();
    doc
}

#[test]
fn hex_details_of_utf8() {
    let doc = editable("aé\r\nz");
    let h = hex_details(&doc, LineIdx(0), 4096);
    assert_eq!(h.bytes, b"a\xc3\xa9");
    assert_eq!(h.eol, b"\r\n");
    assert!(!h.truncated && h.unmappable.is_empty());
    let last = hex_details(&doc, LineIdx(1), 4096);
    assert_eq!(
        (last.bytes.as_slice(), last.eol.as_slice()),
        (&b"z"[..], &b""[..])
    );
}

#[test]
fn hex_details_of_utf16be_with_a_bom() {
    let doc = with_encoding("aé\n", TextEncoding::for_label("UTF-16BE").unwrap(), true);
    let h = hex_details(&doc, LineIdx(0), 4096);
    // The line's bytes: no BOM (that belongs to the file's start).
    assert_eq!(h.bytes, [0x00, 0x61, 0x00, 0xE9]);
    assert_eq!(h.eol, [0x00, 0x0A]);
}

#[test]
fn hex_details_of_shift_jis_and_an_unmappable_character() {
    let doc = with_encoding(
        "あa€\n",
        TextEncoding::for_label("Shift_JIS").unwrap(),
        false,
    );
    let h = hex_details(&doc, LineIdx(0), 4096);
    assert_eq!(&h.bytes[..3], &[0x82, 0xA0, b'a']);
    // The euro sign has no Shift_JIS bytes: written as `?` and listed.
    assert_eq!(h.bytes[3], b'?');
    assert_eq!(h.unmappable, vec![3u32..4u32; 1]);
    let doc = with_encoding(
        "€x\n",
        TextEncoding::for_label("windows-1252").unwrap(),
        false,
    );
    let h = hex_details(&doc, LineIdx(0), 4096);
    assert_eq!(h.bytes, [0x80, b'x']);
    assert!(h.unmappable.is_empty());
}

#[test]
fn hex_details_are_capped() {
    let doc = editable(&format!("{}\n", "ab".repeat(3000)));
    let h = hex_details(&doc, LineIdx(0), 4096);
    assert_eq!(h.bytes.len(), 4096);
    assert!(h.truncated);
}

#[test]
fn alignment_details_follow_the_row_s_classes() {
    let left = "x\nint value = 10;\ny\n";
    let right = "x\nint  valve = 12;\ny\n";
    let mut rules = TextRules::default();
    rules.importance.embedded_whitespace = false;
    let cmp = compare_texts(left, right, &format("ee"), &rules);
    let row = RowIdx(1);
    let cells = char_alignment_of_row(&cmp, row).unwrap();
    let inline = cmp.inline(row).unwrap();
    // Left characters, in order, are the left line.
    let l: String = cells.iter().filter_map(|c| c.left).collect();
    let r: String = cells.iter().filter_map(|c| c.right).collect();
    assert_eq!(l, "int value = 10;");
    assert_eq!(r, "int  valve = 12;");
    // Each left character's class matches the inline range it is in (important: Changed or one-sided; unimportant:
    // Unimportant or one-sided; none: Same).
    let mut at = 0u32;
    for c in cells.iter().filter(|c| c.left.is_some()) {
        let ch = c.left.unwrap();
        let range = inline
            .left
            .iter()
            .find(|x| x.range.start <= at && at < x.range.end);
        match range {
            None => assert_eq!(c.class, CellClass::Same, "{ch} at {at}"),
            Some(x) if x.important => {
                assert!(
                    matches!(c.class, CellClass::Changed | CellClass::LeftOnly),
                    "{ch}"
                )
            }
            Some(_) => assert!(
                matches!(c.class, CellClass::Unimportant | CellClass::LeftOnly),
                "{ch}"
            ),
        }
        at += ch.len_utf8() as u32;
    }
    assert!(cells.iter().any(|c| c.class == CellClass::Changed));
    assert!(
        cells
            .iter()
            .any(|c| c.class == CellClass::RightOnly || c.class == CellClass::Unimportant)
    );
    // A same row and a one-sided row.
    assert!(
        char_alignment_of_row(&cmp, RowIdx(0))
            .unwrap()
            .iter()
            .all(|c| c.class == CellClass::Same)
    );
    let one = compare_texts("a\n", "a\nnew\n", &format("ee"), &TextRules::default());
    let cells = char_alignment_of_row(&one, RowIdx(1)).unwrap();
    assert!(
        cells
            .iter()
            .all(|c| c.class == CellClass::RightOnly && c.left.is_none())
    );
    let _ = Side::Left;
    let _ = TextPos::ZERO;
}

#[test]
fn alignment_details_of_two_texts() {
    let cells = char_alignment("abcd", "abXd", &hikasa_diff_default());
    let classes: Vec<CellClass> = cells.iter().map(|c| c.class).collect();
    assert_eq!(
        classes,
        [
            CellClass::Same,
            CellClass::Same,
            CellClass::Changed,
            CellClass::Same
        ]
    );
    let cells = char_alignment("ab", "aXYb", &hikasa_diff_default());
    assert_eq!(
        cells
            .iter()
            .filter(|c| c.class == CellClass::RightOnly)
            .count(),
        2
    );
}

fn hikasa_diff_default() -> hikasa_text::InlineOptions {
    hikasa_text::InlineOptions::DEFAULT
}

#[test]
fn the_ruler() {
    let marks = ruler(4, 1, 21);
    assert_eq!(marks.len(), 21);
    assert_eq!(marks[9].label, Some(10));
    assert_eq!(marks[0].label, None);
    let stops: Vec<u32> = marks
        .iter()
        .filter(|m| m.tab_stop)
        .map(|m| m.column)
        .collect();
    assert_eq!(stops, [1, 5, 9, 13, 17, 21]);
}
