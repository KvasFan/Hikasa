//! A13.4 (M2 W13a, D-357): caret, selection, Copy, Current Byte Address — every row of §1.13.5's key table in both
//! panes and parts, on gap columns and under filters; the pointer rules (click, Shift+click, drag, double-click, a
//! right-click outside the selection, a click on an address); P-HEXCARET: random key and click scripts keep `caret`
//! and `anchor` inside the columns and the caret's byte equal to `offsets_at(caret)`; Select All; Copy and Copy as Text
//! across gaps and pages and the 16 MiB limit equal `selection_text` (whose own property is in core); the primary
//! selection; Current Byte Address. Mutations: Copy with the other side's bytes at gap columns; Up under a filter into
//! a hidden row.

use std::sync::Arc;

use hikasa_core::Side;
use hikasa_hex::DisplayFilter;
use hikasa_hex::copy::{CopyForm, selection_text};
use hikasa_session::clipboard::FixedClipboard;
use proptest::prelude::*;

use crate::commands::CommandId;
use crate::hex::pane::{CaretKey, Hit, Part};
use crate::tests::{TestApp, World};
use crate::tests_hex_open::{bytes, hex_model, hex_world, open_pair, with_insert};

/// A hex view with a fixed clipboard: left 4,000 bytes; right with 10 bytes inserted at 1,000 and one change.
fn fixture() -> (
    World,
    TestApp,
    hikasa_session::ipc::ViewId,
    Arc<FixedClipboard>,
) {
    let world = hex_world();
    let mut app = world.app(None);
    let clip = Arc::new(FixedClipboard::text("clip"));
    app.0.set_script_clipboard(Arc::clone(&clip) as _);
    let a = bytes(41, 4000);
    let b = with_insert(&a, 1000, 10, 3000);
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    m.frame(20, 0.0);
    (world, app, view, clip)
}

fn last_text(clip: &FixedClipboard) -> Option<String> {
    clip.payloads().last().map(|p| p.text.clone())
}

/// A13.4: the key table.
#[test]
fn a13_4_the_key_table() {
    let (_world, mut app, view, _) = fixture();
    let m = hex_model(&mut app, view);
    let columns = m.columns();
    let caret = |m: &crate::hex::HexViewModel| m.pane(m.focus()).caret;
    m.set_caret(40, false);
    m.key(CaretKey::Right, false);
    assert_eq!(caret(m), 41);
    m.key(CaretKey::Left, false);
    assert_eq!(caret(m), 40);
    m.key(CaretKey::Down, false);
    assert_eq!(caret(m), 56);
    m.key(CaretKey::Up, false);
    assert_eq!(caret(m), 40);
    m.key(CaretKey::Home, false);
    assert_eq!(caret(m), 32);
    m.key(CaretKey::End, false);
    assert_eq!(caret(m), 47);
    m.key(CaretKey::Last, false);
    assert_eq!(caret(m), columns - 1);
    m.key(CaretKey::Right, false);
    assert_eq!(caret(m), columns - 1, "stays on the last column");
    m.key(CaretKey::First, false);
    assert_eq!(caret(m), 0);
    m.key(CaretKey::Left, false);
    assert_eq!(caret(m), 0);
    m.key(CaretKey::PageDown, false);
    assert_eq!(
        caret(m),
        20 * 16,
        "a screen of rows, the position in the row kept"
    );
    m.key(CaretKey::PageUp, false);
    assert_eq!(caret(m), 0);
    // Shift extends from the anchor; both panes keep the column.
    m.set_caret(100, false);
    m.key(CaretKey::Right, true);
    m.key(CaretKey::Right, true);
    assert_eq!(m.pane(Side::Left).selection(), 100..103);
    assert_eq!(m.pane(Side::Right).caret, 102);
    // Tab switches the part; Esc collapses the selection.
    m.switch_part();
    assert_eq!(m.pane(Side::Left).part, Part::Chars);
    m.switch_part();
    assert_eq!(m.pane(Side::Left).part, Part::Hex);
    m.escape();
    assert!(!m.pane(Side::Left).has_selection());
    // The focus side moves with GoRight; the column is kept.
    m.focus_side(Side::Right);
    assert_eq!(m.pane(Side::Right).caret, 102);
}

/// A13.4: Up and Down under a filter step over hidden rows to the same position of the shown row. Mutation: Up into
/// a hidden row.
#[test]
fn a13_4_up_and_down_under_a_filter() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(42, 16 * 64);
    let mut b = a.clone();
    // Differences in rows 3 and 40 only.
    b[3 * 16 + 5] ^= 1;
    b[40 * 16 + 9] ^= 1;
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::Differences);
    assert_eq!(m.row_count(), 2);
    m.set_caret(40 * 16 + 2, false);
    m.key(CaretKey::Up, false);
    assert_eq!(
        m.pane(Side::Left).caret,
        3 * 16 + 2,
        "the previous shown row"
    );
    m.key(CaretKey::Down, false);
    assert_eq!(m.pane(Side::Left).caret, 40 * 16 + 2);
    m.key(CaretKey::Down, false);
    assert_eq!(m.pane(Side::Left).caret, 40 * 16 + 2, "no row below");
}

/// A13.4: the pointer rules.
#[test]
fn a13_4_pointer_rules() {
    let (_world, mut app, view, _) = fixture();
    let m = hex_model(&mut app, view);
    let at = |row: u64, i: u32, part: Part| Hit {
        row,
        cell: Some((part, i)),
    };
    m.click(Side::Right, &at(2, 5, Part::Chars), false);
    assert_eq!(m.focus(), Side::Right);
    assert_eq!(m.pane(Side::Right).caret, 37);
    assert_eq!(m.pane(Side::Right).part, Part::Chars);
    m.click(Side::Right, &at(3, 1, Part::Hex), true);
    assert_eq!(m.selection(), 37..50, "Shift+click extends");
    // Press and drag selects.
    m.click(Side::Left, &at(0, 0, Part::Hex), false);
    m.drag(&at(1, 3, Part::Hex));
    m.release();
    assert_eq!(m.selection(), 0..20);
    // A right-click inside the selection keeps it; outside moves the caret.
    m.context_click(Side::Left, &at(0, 4, Part::Hex));
    assert_eq!(m.selection(), 0..20);
    m.context_click(Side::Left, &at(5, 4, Part::Hex));
    assert_eq!(m.selection(), 84..85);
    // A double-click selects the group under the pointer (group 4).
    let mut layout = m.display().layout;
    layout.group = 4;
    m.set_layout(layout, true, crate::hex::PaneLayout::SideBySide, 0);
    m.double_click(Side::Left, &at(1, 6, Part::Hex));
    assert_eq!(m.selection(), 20..24);
    // A click on an address selects the row.
    m.click(Side::Left, &Hit { row: 4, cell: None }, false);
    assert_eq!(m.selection(), 64..80);
}

/// A13.4: Select All, Copy and Copy as Text across the gap equal `selection_text` of the focus side; the primary
/// selection gets the same text when it settles; Current Byte Address copies `address_text`. Mutation: Copy with the
/// other side's bytes at gap columns.
#[test]
fn a13_4_copy_and_the_address() {
    let (world, mut app, view, clip) = fixture();
    let m = hex_model(&mut app, view);
    let src = m.source().expect("src").clone();
    let store = src.view();
    let gap = store
        .segments()
        .iter()
        .find(|s| s.left_len() == 0)
        .expect("a gap");
    let gap_col = store
        .segments()
        .column_at_offset(Side::Right, gap.right)
        .expect("col");
    // A selection across the 10 gap columns of the left side.
    m.set_caret(gap_col - 4, false);
    m.set_caret(gap_col + 13, true);
    let columns = m.selection();
    app.execute(CommandId::Copy);
    world.settle(&mut app);
    let docs = &src.opened.documents;
    let want = selection_text(
        &docs.left,
        store.segments(),
        columns.clone(),
        Side::Left,
        CopyForm::Hex,
        u64::MAX,
    )
    .expect("text");
    assert_eq!(last_text(&clip), Some(want.clone()));
    assert_eq!(
        want.split(' ').count(),
        18 - 10,
        "the gap contributes nothing"
    );
    // Copy as Text: the characters.
    app.execute(CommandId::CopyAsText);
    world.settle(&mut app);
    let chars = selection_text(
        &docs.left,
        store.segments(),
        columns.clone(),
        Side::Left,
        CopyForm::Chars(hikasa_hex::CharSet::Ascii),
        u64::MAX,
    )
    .expect("text");
    assert_eq!(last_text(&clip), Some(chars));
    // The right side's copy of the same columns holds its 10 inserted bytes.
    hex_model(&mut app, view).focus_side(Side::Right);
    app.execute(CommandId::Copy);
    world.settle(&mut app);
    assert_eq!(last_text(&clip).expect("copied").split(' ').count(), 18);
    // The primary selection settles 300 ms after a Shift+move.
    let m = hex_model(&mut app, view);
    m.now = 10.0;
    m.key(CaretKey::Right, true);
    m.frame(20, 10.1);
    assert_eq!(m.primary_written, 0);
    m.frame(20, 10.5);
    assert_eq!(m.primary_written, 1);
    world.settle(&mut app);
    assert!(!clip.primaries().is_empty());
    // Select All: every column.
    app.execute(CommandId::SelectAll);
    let m = hex_model(&mut app, view);
    assert_eq!(m.selection(), 0..m.columns());
    // Current Byte Address of the caret's byte on the focus side.
    m.set_caret(gap_col + 12, false);
    let byte = m.byte_at(Side::Right, gap_col + 12).expect("byte").0;
    app.execute(CommandId::CurrentAddress);
    world.settle(&mut app);
    let width = hex_model(&mut app, view).address_width();
    assert_eq!(
        last_text(&clip),
        Some(hikasa_hex::nav::address_text(
            Side::Right,
            byte,
            hikasa_hex::Radix::Hex,
            width
        ))
    );
}

/// A13.4: Copy of more than 16 MiB is refused with the core's message (the selection's bytes are read in a job when
/// they are not cached).
#[test]
fn a13_4_copy_takes_at_most_16_mib() {
    let world = hex_world();
    let mut app = world.app(None);
    let clip = Arc::new(FixedClipboard::text("clip"));
    app.0.set_script_clipboard(Arc::clone(&clip) as _);
    let a = vec![7u8; (16 << 20) + 5];
    let view = open_pair(&world, &mut app, &a, &a);
    app.execute(CommandId::SelectAll);
    app.execute(CommandId::Copy);
    world.settle(&mut app);
    let message = app
        .tab(view)
        .and_then(|t| t.message.clone())
        .expect("message");
    assert!(message.text.contains("at most 16 MiB"), "{}", message.text);
    assert_eq!(last_text(&clip), None);
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(16),
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// P-HEXCARET.
    #[test]
    fn p_hexcaret(seed in any::<u64>(), script in prop::collection::vec((0u8..14, any::<bool>(), 0u64..400, 0u32..16), 1..40)) {
        let world = hex_world();
        let mut app = world.app(None);
        let a = bytes(seed, 3000 + (seed % 900) as usize);
        let b = with_insert(&a, (seed % 2000) as usize, (seed % 40) as usize + 1, 2500);
        let view = open_pair(&world, &mut app, &a, &b);
        let m = hex_model(&mut app, view);
        m.frame(17, 0.0);
        let keys = [
            CaretKey::Left, CaretKey::Right, CaretKey::Up, CaretKey::Down, CaretKey::Home, CaretKey::End,
            CaretKey::First, CaretKey::Last, CaretKey::PageUp, CaretKey::PageDown,
        ];
        for (k, shift, row, i) in script {
            match k {
                0..=9 => m.key(keys[k as usize], shift),
                10 => m.click(if shift { Side::Right } else { Side::Left }, &Hit { row, cell: Some((Part::Hex, i)) }, false),
                11 => m.set_filter([DisplayFilter::All, DisplayFilter::Differences, DisplayFilter::Same][(row % 3) as usize]),
                12 => m.focus_side(if shift { Side::Right } else { Side::Left }),
                _ => m.select_all(),
            }
            let columns = m.columns();
            // Both panes stand on one column (the rows are aligned), the anchor too.
            prop_assert_eq!(m.pane(Side::Left).caret, m.pane(Side::Right).caret);
            prop_assert_eq!(m.pane(Side::Left).anchor, m.pane(Side::Right).anchor);
            for side in [Side::Left, Side::Right] {
                let p = *m.pane(side);
                prop_assert!(p.caret < columns && p.anchor < columns);
                let src = m.source().expect("src");
                let (l, r) = src.view().segments().offsets_at(p.caret);
                let want = if side == Side::Right { r } else { l };
                prop_assert_eq!(m.byte_at(side, p.caret).expect("byte").0, want);
            }
        }
    }
}
