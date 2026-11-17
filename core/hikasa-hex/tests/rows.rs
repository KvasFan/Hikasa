//! A12.7 (D-332): rows, layout, filters.
//!
//! - rows for bytes per line 1, 7, 16, 256, groups 1–8, ASCII / Latin-1 / no characters, hex and decimal
//!   addresses, gaps on either side, a row with no left byte (goldens);
//! - `differences` and `same` with context 0 and 3 and their `hidden_before` counts;
//! - P-HEXROWS: `row_of(side, offset)` and the row's address round trip for random offsets in every mode;
//!   provisional rows equal the None alignment's rows.

mod common;

use std::fmt::Write as _;

use common::{XorShift, doc, edited, golden, opts};
use hikasa_core::{Ctl, Side};
use hikasa_hex::nav::address_text;
use hikasa_hex::{
    AlignMode, ByteState, CharSet, DisplayFilter, HexLayout, HexRow, HexView, Radix, RowState,
    align,
};
use proptest::prelude::*;

/// A text rendering of rows: addresses, the hex digits of each side spaced by the group, the characters.
fn render(view: &HexView<'_>, rows: &[HexRow]) -> String {
    let layout = *view.layout();
    let width = view.address_width();
    let mut out = String::new();
    for row in rows {
        let addr = |a: Option<u64>, side| {
            a.map_or_else(
                || "-".repeat(4),
                |a| address_text(side, a, layout.address, width),
            )
        };
        if row.cells.is_empty() {
            let _ = writeln!(out, "[marker] {} rows past the sections", row.hidden_before);
            continue;
        }
        if row.hidden_before > 0 {
            let _ = writeln!(out, "[{} rows hidden]", row.hidden_before);
        }
        let side = |pick: fn(&hikasa_hex::HexCell) -> Option<u8>| {
            let mut s = String::new();
            for (i, c) in row.cells.iter().enumerate() {
                if i > 0 && i % usize::from(layout.group) == 0 {
                    s.push(' ');
                }
                match pick(c) {
                    Some(b) => {
                        let _ = write!(s, "{b:02X}");
                    }
                    None => s.push_str(".."),
                }
            }
            s
        };
        let chars: String = row
            .cells
            .iter()
            .map(|c| c.left.and_then(|b| layout.char_of(b)).unwrap_or(' '))
            .collect();
        let states: String = row
            .cells
            .iter()
            .map(|c| match c.state {
                ByteState::Same => '=',
                ByteState::Different => '*',
                ByteState::LeftOnly => '<',
                ByteState::RightOnly => '>',
            })
            .collect();
        let _ = writeln!(
            out,
            "{:>4} {} {} | {} | {} | {chars:?} {states} {:?}",
            row.index,
            addr(row.left_address, Side::Left),
            addr(row.right_address, Side::Right),
            side(|c| c.left),
            side(|c| c.right),
            row.state
        );
    }
    out
}

/// A pair with gaps on both sides: text with an insertion and a deletion.
fn texts() -> (Vec<u8>, Vec<u8>) {
    let a: Vec<u8> = b"The quick brown fox jumps over the lazy dog. \xE9t\xE9 \x00\x01\x7F\xA0\xFF end of the first file."
        .to_vec();
    let mut b = a.clone();
    b.splice(10..10, b"VERY ".iter().copied());
    b.drain(40..46);
    b[60] = b'#';
    (a, b)
}

#[test]
fn layouts() {
    let (a, b) = texts();
    let (da, db) = (doc("a", &a), doc("b", &b));
    let al = align(&da, &db, &opts(AlignMode::Complete, 1), &Ctl::detached()).unwrap();
    let mut out = String::new();
    for (bpl, group, chars, address) in [
        (16, 1, CharSet::Ascii, Radix::Hex),
        (7, 1, CharSet::Latin1, Radix::Decimal),
        (16, 4, CharSet::None, Radix::Hex),
        (1, 1, CharSet::Ascii, Radix::Hex),
        (256, 8, CharSet::Latin1, Radix::Hex),
        (16, 2, CharSet::Ascii, Radix::Decimal),
    ] {
        let layout = HexLayout {
            bytes_per_line: bpl,
            group,
            chars,
            address,
        };
        let view = HexView::new(&da, &db, &al, layout);
        let rows = view.rows(0, u32::MAX).unwrap();
        assert_eq!(rows.len() as u64, view.row_count());
        let _ = writeln!(out, "== {bpl} bytes, group {group}, {chars}, {address}");
        if bpl == 1 {
            // Only the rows around the first gap.
            out.push_str(&render(&view, &rows[8..16]));
        } else {
            out.push_str(&render(&view, &rows));
        }
    }
    // A row with no left byte: an insertion of a whole row.
    let c: Vec<u8> = [&a[..16], &[0x55; 16][..], &a[16..]].concat();
    let dc = doc("c", &c);
    let al = align(&da, &dc, &opts(AlignMode::Complete, 1), &Ctl::detached()).unwrap();
    let view = HexView::new(&da, &dc, &al, HexLayout::default());
    let rows = view.rows(0, 3).unwrap();
    assert_eq!(rows[1].left_address, None);
    assert_eq!(rows[1].right_address, Some(16));
    assert_eq!(rows[1].state, RowState::Orphan);
    assert_eq!(rows[2].left_address, Some(16));
    let _ = writeln!(out, "== a row with no left byte");
    out.push_str(&render(&view, &rows));
    golden("rows_layouts", &out);
}

#[test]
fn filters_and_hidden_rows() {
    // 64 rows of 16 bytes; differences in rows 5, 6 and 40, and an orphan tail.
    let a = XorShift(2).bytes(1024);
    let mut b = a.clone();
    b[5 * 16 + 3] ^= 1;
    b[6 * 16 + 15] ^= 1;
    b[40 * 16] ^= 1;
    b.extend_from_slice(&[9; 20]);
    let (da, db) = (doc("a", &a), doc("b", &b));
    let al = align(&da, &db, &opts(AlignMode::None, 1), &Ctl::detached()).unwrap();
    let mut out = String::new();
    for (filter, context) in [
        (DisplayFilter::Differences, 0),
        (DisplayFilter::Differences, 3),
        (DisplayFilter::Same, 0),
        (DisplayFilter::Same, 3),
    ] {
        let view = HexView::new(&da, &db, &al, HexLayout::default()).with_filter(filter, context);
        let rows = view.rows(0, u32::MAX).unwrap();
        let summary: Vec<(u64, u64)> = rows.iter().map(|r| (r.index, r.hidden_before)).collect();
        let _ = writeln!(
            out,
            "== {filter} context {context}: {} rows\n{summary:?}",
            view.row_count()
        );
        // Every shown row is found again from its first byte.
        for (k, row) in rows.iter().enumerate() {
            if let Some(at) = row.left_address {
                assert_eq!(
                    view.row_of(Side::Left, at),
                    Some(k as u64),
                    "{filter} {context}"
                );
            }
        }
    }
    golden("rows_filters", &out);
    // The filtered rows of `differences` with context 0 are exactly the rows that hold a non-same cell.
    let view = HexView::new(&da, &db, &al, HexLayout::default())
        .with_filter(DisplayFilter::Differences, 0);
    let shown: Vec<u64> = view
        .rows(0, u32::MAX)
        .unwrap()
        .iter()
        .map(|r| r.index)
        .collect();
    let all = HexView::new(&da, &db, &al, HexLayout::default())
        .rows(0, u32::MAX)
        .unwrap();
    let differing: Vec<u64> = all
        .iter()
        .filter(|r| r.state != RowState::Same)
        .map(|r| r.index)
        .collect();
    assert_eq!(shown, differing);
}

#[test]
fn rows_past_truncated_sections_end_with_a_marker() {
    let a = vec![0u8; 400];
    let mut b = a.clone();
    for k in 0..20 {
        b[k * 20] = 1;
    }
    let (da, db) = (doc("a", &a), doc("b", &b));
    let o = hikasa_hex::HexAlignOptions {
        max_sections: 3,
        ..opts(AlignMode::None, 1)
    };
    let al = align(&da, &db, &o, &Ctl::detached()).unwrap();
    let view = HexView::new(&da, &db, &al, HexLayout::default())
        .with_filter(DisplayFilter::Differences, 0);
    let rows = view.rows(0, u32::MAX).unwrap();
    let last = rows.last().unwrap();
    assert!(last.cells.is_empty());
    assert_eq!(view.row_count(), rows.len() as u64);
    assert_eq!(last.index, 3);
    assert_eq!(last.hidden_before, 25 - 3);
}

fn pair() -> impl Strategy<Value = (Vec<u8>, Vec<u8>)> {
    (0usize..20_000, any::<u64>(), 0usize..6).prop_map(|(len, seed, n)| {
        let a = XorShift(seed | 1).bytes(len);
        let b = edited(
            &a,
            &common::random_edits(len.max(1), n, 500, seed ^ 5),
            seed,
        );
        (a, b)
    })
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/rows.txt"))]

    /// P-HEXROWS.
    #[test]
    fn p_hexrows((a, b) in pair(), bpl in 1u16..=64, offsets in prop::collection::vec(any::<u64>(), 8)) {
        let (da, db) = (doc("a", &a), doc("b", &b));
        let layout = HexLayout { bytes_per_line: bpl, ..HexLayout::default() };
        for mode in [AlignMode::None, AlignMode::Fast, AlignMode::Complete] {
            let al = align(&da, &db, &opts(mode, 1), &Ctl::detached()).unwrap();
            let view = HexView::new(&da, &db, &al, layout);
            for (side, bytes) in [(Side::Left, &a), (Side::Right, &b)] {
                if bytes.is_empty() {
                    prop_assert_eq!(view.row_of(side, 0), None);
                    continue;
                }
                for &o in &offsets {
                    let offset = o % bytes.len() as u64;
                    let r = view.row_of(side, offset).unwrap();
                    let row = &view.rows(r, 1).unwrap()[0];
                    // The row holds the byte: its address plus the side's cells before it.
                    let address = if side == Side::Left { row.left_address } else { row.right_address };
                    let held: Vec<u8> = row.cells.iter().filter_map(|c| if side == Side::Left { c.left } else { c.right }).collect();
                    let first = address.unwrap();
                    prop_assert!(first <= offset && offset < first + held.len() as u64, "{} {:?} {}", mode, side, offset);
                    prop_assert_eq!(held[(offset - first) as usize], bytes[offset as usize]);
                }
            }
            if mode == AlignMode::None {
                let provisional = HexView::provisional(&da, &db, layout);
                prop_assert_eq!(provisional.row_count(), view.row_count());
                prop_assert_eq!(provisional.rows(0, u32::MAX).unwrap(), view.rows(0, u32::MAX).unwrap());
            }
        }
    }
}
