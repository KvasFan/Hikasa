//! A11.13 (D-306): the details panel and the ruler. Text Details edits the pane's own document (one undo log); Hex
//! Details of UTF-8, UTF-16BE with a BOM, Shift_JIS and an unmappable character equal core's `hex_details`; Alignment
//! Details' cells equal `char_alignment_of_row` and a typed key at a cell edits that side at its character; *Lock
//! Visible Whitespace*; the ruler's marks and the caret's column are core's.

use hikasa_core::Side;
use hikasa_text::LineIdx;
use hikasa_text::details::{char_alignment_of_row, hex_details, ruler};
use hikasa_text::edit::TextPos;

use crate::commands::CommandId;
use crate::tests::World;
use crate::tests_text::Fixture;
use crate::text::details;
use crate::text::layout::Details;

#[test]
fn a11_13_text_details_edit_the_panes_document() {
    let mut f = Fixture::new("alpha\nbeta\n", "alpha\nbetA\n");
    f.run(CommandId::ShowTextDetails);
    assert_eq!(f.m().display().details, Details::Text);
    f.place(Side::Left, 1, 0);
    let d = details::data(f.m());
    assert_eq!(d.lines[0].as_ref().map(|l| l.1.as_str()), Some("beta"));
    assert_eq!(d.lines[1].as_ref().map(|l| l.1.as_str()), Some("betA"));
    // A click in the panel at the right line's byte 2 puts that pane's caret there: the panel and the pane are one
    // editor, so a typed key edits the right document.
    details::text_press(f.m(), Side::Right, 2);
    assert_eq!(f.m().focus(), Side::Right);
    assert_eq!(f.caret(Side::Right), TextPos::new(1, 2));
    f.type_text("Z");
    assert_eq!(f.text(Side::Right), "alpha\nbeZtA\n");
    assert_eq!(
        details::data(f.m()).lines[1].as_ref().map(|l| l.1.clone()),
        Some("beZtA".to_owned()),
        "computed again after the edit"
    );
    // One undo log: the pane's Undo takes it back.
    f.run(CommandId::Undo);
    assert_eq!(f.text(Side::Right), "alpha\nbetA\n");
    // Computed when the row or the documents change, not per call.
    let _ = details::data(f.m());
    let builds = f.m().details_builds();
    let _ = details::data(f.m());
    let _ = details::data(f.m());
    assert_eq!(f.m().details_builds(), builds);
}

#[test]
fn a11_13_hex_details_equal_core() {
    let world = World::new();
    // UTF-8; UTF-16BE with a BOM; Shift_JIS; windows-1252 with a character it cannot write (typed in).
    let utf8 = world.write("work/u8.txt", "héllo\n");
    let utf16 = world.path("work/u16.txt");
    let mut bytes = vec![0xFE, 0xFF];
    for u in "Aé\n".encode_utf16() {
        bytes.extend(u.to_be_bytes());
    }
    std::fs::write(&utf16, bytes).expect("write");
    let sjis = world.path("work/sjis.txt");
    std::fs::write(&sjis, [0x82, 0xA0, 0x82, 0xA2, b'\n']).expect("write");
    let mut f = Fixture::open_paths(world, utf8, utf16);
    f.run(CommandId::ShowHexDetails);
    for side in [Side::Left, Side::Right] {
        f.place(side, 0, 0);
        let k = usize::from(side == Side::Right);
        let got = details::data(f.m()).hex[k].clone().expect("hex");
        let doc = f.doc(side);
        assert_eq!(
            got,
            hex_details(&doc, LineIdx(0), details::HEX_MAX),
            "{side:?}"
        );
    }
    let right = details::data(f.m()).hex[1].clone().expect("hex");
    assert_eq!(&right.bytes[..2], &[0x00, 0x41], "UTF-16BE");
    assert_eq!(right.eol, vec![0x00, 0x0A]);
    // The rows: 16 bytes each, the bytes as characters with `.` for the others.
    let rows = details::hex_rows(&right);
    assert_eq!(rows[0].hex_text(), "00 41 00 E9");
    assert_eq!(rows[0].chars_text(), ".A..");
    // Shift_JIS, and an unmappable character marked.
    let world = World::new();
    let a = world.path("work/a.txt");
    std::fs::write(&a, b"caf\xe9\n").expect("write");
    let mut f = Fixture::open_paths(world, sjis_path(&f), a);
    f.run(CommandId::ShowHexDetails);
    f.place(Side::Left, 0, 0);
    let doc = f.doc(Side::Left);
    assert_eq!(
        details::data(f.m()).hex[0].clone().expect("hex"),
        hex_details(&doc, LineIdx(0), details::HEX_MAX)
    );
    f.place(Side::Right, 0, 4);
    f.type_text("€\u{4e00}");
    let doc = f.doc(Side::Right);
    let hex = details::data(f.m()).hex[1].clone().expect("hex");
    assert_eq!(hex, hex_details(&doc, LineIdx(0), details::HEX_MAX));
    assert!(!hex.unmappable.is_empty(), "{hex:?}");
    let rows = details::hex_rows(&hex);
    assert!(rows[0].unmappable.iter().any(|u| *u));
    let _ = sjis;
}

/// The Shift_JIS file of the first fixture's world.
fn sjis_path(f: &Fixture) -> std::path::PathBuf {
    f.world.path("work/sjis.txt")
}

#[test]
fn a11_13_alignment_details_cells_and_their_edits() {
    let mut f = Fixture::new("one two\nkeep\n", "one tWo\nkeep\n");
    f.run(CommandId::ShowAlignDetails);
    f.place(Side::Left, 0, 0);
    let d = details::data(f.m());
    let row = f.m().cursor().expect("row");
    let cmp = std::sync::Arc::clone(f.m().comparison().expect("cmp"));
    assert_eq!(d.cells, char_alignment_of_row(&cmp, row));
    let cells = d.cells.expect("cells");
    // A typed key at cell 4 of the left ('t' of "two") inserts there, in the left document.
    let at = details::cell_pos(&cells, Side::Left, 4, 0);
    assert_eq!(at, TextPos::new(0, 4));
    assert!(details::align_type(f.m(), Side::Left, 4, "X"));
    assert_eq!(f.text(Side::Left), "one Xtwo\nkeep\n");
    // The cells are computed again from the row (a pending row: from its live lines; then the installed row's).
    let pending = details::data(f.m()).cells.expect("cells");
    assert!(pending.iter().any(|c| c.left == Some('X')));
    f.settle();
    let row = f.m().cursor().expect("row");
    let cmp = std::sync::Arc::clone(f.m().comparison().expect("cmp"));
    let again = details::data(f.m()).cells.expect("cells");
    assert_eq!(Some(again.clone()), char_alignment_of_row(&cmp, row));
    assert!(again.iter().any(|c| c.left == Some('X')));
    // Backspace and Delete at a cell.
    assert!(details::align_delete(f.m(), Side::Left, 5, true));
    assert_eq!(f.text(Side::Left), "one two\nkeep\n");
    f.settle();
    assert!(details::align_delete(f.m(), Side::Right, 0, false));
    assert_eq!(f.text(Side::Right), "ne tWo\nkeep\n");
    // A gap cell of the left: the next character's position on that side.
    let mut g = Fixture::new("abcd\n", "abXYcd\n");
    g.run(CommandId::ShowAlignDetails);
    g.place(Side::Left, 0, 0);
    let cells = details::data(g.m()).cells.expect("cells");
    let gap = cells
        .iter()
        .position(|c| c.left.is_none())
        .expect("a gap cell");
    assert_eq!(
        details::cell_pos(&cells, Side::Left, gap, 0),
        TextPos::new(0, 2)
    );
    // A cell after the gaps: the gap cells hold no byte of the left line (`c` is its byte 2).
    let c = cells
        .iter()
        .position(|x| x.left == Some('c'))
        .expect("the c cell");
    assert!(c > gap + 1, "two gap cells before it: {cells:?}");
    assert_eq!(
        details::cell_pos(&cells, Side::Left, c, 0),
        TextPos::new(0, 2)
    );
    assert!(details::align_type(g.m(), Side::Left, gap, "Q"));
    assert_eq!(g.text(Side::Left), "abQcd\n");
    // Lock Visible Whitespace is the panel's own flag, kept as a layout key.
    f.m().display_mut().details_whitespace = true;
    assert!(f.m().layout_keys().contains(&(
        "details_whitespace",
        hikasa_session::settings::state::LayoutValue::Bool(true)
    )));
}

#[test]
fn a11_13_the_ruler_is_cores() {
    let mut f = Fixture::new("\tab\tc\n", "\tab\tc\n");
    f.run(CommandId::ShowRuler);
    let tab = f.doc(Side::Left).options().tab_size;
    assert_eq!(
        details::ruler_marks(f.m(), Side::Left, 1, 30),
        ruler(tab, 1, 30)
    );
    // The caret's display column: after the tab, `a`, `b`, the second tab.
    f.place(Side::Left, 0, 4);
    let line = f.doc(Side::Left).line(LineIdx(0)).into_owned();
    assert_eq!(
        details::caret_column(f.m(), Side::Left),
        Some(hikasa_text::nav::column_of_tabs(&line, 4, tab).1)
    );
}

/// Open item: F1 opens the help topic of the panel holding the keyboard (Find and Replace, the details panel), else the
/// view's.
#[test]
fn w11b_review_f1_opens_the_panels_topic() {
    let mut f = Fixture::new("a\n", "b\n");
    let topic = |f: &mut Fixture| {
        f.run(CommandId::ContextHelp);
        let win = f.app.win();
        f.app.window(win).and_then(|w| w.help)
    };
    let view_topic = crate::commands::Scope::Text.help_topic();
    assert_eq!(topic(&mut f), Some(view_topic));
    f.run(CommandId::Find);
    f.m().set_keys_in(Some(crate::text::PanelKeys::Find));
    assert_eq!(topic(&mut f), Some(crate::text::find::FIND_TOPIC));
    f.run(CommandId::ShowTextDetails);
    f.m().set_keys_in(Some(crate::text::PanelKeys::Details));
    assert_eq!(topic(&mut f), Some(details::DETAILS_TOPIC));
    // A panel no longer shown: the view's topic.
    f.m().display_mut().details = crate::text::layout::Details::None;
    assert_eq!(topic(&mut f), Some(view_topic));
    f.m().set_keys_in(None);
    assert_eq!(topic(&mut f), Some(view_topic));
}
