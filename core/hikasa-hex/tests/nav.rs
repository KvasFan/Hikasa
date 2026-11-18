//! A12.8 (D-333): navigation, Go To, address text — `nav::step` for every step and both sides, at the first and
//! last section and past a truncated list (the scan on demand); `parse_address` for every spelling; address
//! widths.

mod common;

use common::{doc, opts};
use hikasa_core::{Ctl, Side};
use hikasa_hex::nav::{
    AddressQuery, Step, address_text, parse_address, scan_next_difference, step,
};
use hikasa_hex::{AlignMode, HexAlignOptions, HexLayout, HexView, Radix, align};
use hikasa_vfs::StreamOptions;

/// Left: 100 random bytes. Right: changes at 10 and 50..53, 4 bytes inserted at 80, a 2-byte tail.
fn pair() -> (Vec<u8>, Vec<u8>) {
    let a = common::XorShift(21).bytes(100);
    let mut b = a.clone();
    b[10] ^= 1;
    for x in &mut b[50..53] {
        *x ^= 2;
    }
    b.splice(80..80, [7u8, 7, 7, 7]);
    b.extend_from_slice(&[9, 9]);
    (a, b)
}

#[test]
fn steps() {
    let (a, b) = pair();
    let (da, db) = (doc("a", &a), doc("b", &b));
    let al = align(&da, &db, &opts(AlignMode::Complete, 1), &Ctl::detached()).unwrap();
    let starts: Vec<u64> = al
        .sections
        .iter(&al.segments)
        .map(|s| s.columns.start)
        .collect();
    assert_eq!(starts, [10, 50, 80, 104]);
    let at = |from, s| step(&al, from, s);
    // Next and previous difference, at the first and the last section.
    assert_eq!(at(0, Step::NextDifference), Some(10));
    assert_eq!(at(10, Step::NextDifference), Some(50));
    assert_eq!(at(51, Step::NextDifference), Some(80));
    assert_eq!(at(104, Step::NextDifference), None);
    assert_eq!(at(9, Step::PrevDifference), None);
    assert_eq!(at(10, Step::PrevDifference), None);
    assert_eq!(at(30, Step::PrevDifference), Some(10));
    // From inside a section: the one before it.
    assert_eq!(at(52, Step::PrevDifference), Some(10));
    assert_eq!(at(60, Step::PrevDifference), Some(50));
    assert_eq!(at(105, Step::PrevDifference), Some(80));
    // Existing bytes of each side: the right-only columns 80..84 and 104..106 are the left's gaps.
    assert_eq!(at(79, Step::NextExisting(Side::Left)), Some(84));
    assert_eq!(at(79, Step::NextExisting(Side::Right)), Some(80));
    assert_eq!(at(84, Step::PrevExisting(Side::Left)), Some(79));
    assert_eq!(at(84, Step::PrevExisting(Side::Right)), Some(83));
    assert_eq!(at(103, Step::NextExisting(Side::Left)), None);
    assert_eq!(at(0, Step::PrevExisting(Side::Left)), None);
    // From a column past the end: the last column holding a byte of that side.
    assert_eq!(at(500, Step::PrevExisting(Side::Left)), Some(103));
    assert_eq!(at(u64::MAX, Step::PrevExisting(Side::Right)), Some(105));
    assert_eq!(at(77, Step::First), Some(0));
    assert_eq!(at(0, Step::Last), Some(105));
}

#[test]
fn past_a_truncated_list_the_next_difference_is_scanned() {
    let (a, b) = pair();
    let (da, db) = (doc("a", &a), doc("b", &b));
    for mode in [AlignMode::None, AlignMode::Complete] {
        let o = HexAlignOptions {
            max_sections: 1,
            ..opts(mode, 1)
        };
        let al = align(&da, &db, &o, &Ctl::detached()).unwrap();
        assert!(al.sections.truncated());
        assert_eq!(step(&al, 10, Step::NextDifference), None);
        let s = StreamOptions::default();
        let ctl = Ctl::detached();
        let scan = |from| scan_next_difference(&da, &db, &al, from, &s, &ctl).unwrap();
        assert_eq!(scan(10), Some(50), "{mode}");
        assert_eq!(scan(52), Some(80), "{mode}");
        if mode == AlignMode::Complete {
            assert_eq!(scan(84), Some(104));
            assert_eq!(scan(105), None);
        }
    }
}

#[test]
fn go_to_spellings_and_address_texts() {
    use AddressQuery::*;
    for (text, radix, q) in [
        ("0x1F", Radix::Decimal, Absolute(31)),
        ("1Fh", Radix::Decimal, Absolute(31)),
        ("$1F", Radix::Decimal, Absolute(31)),
        ("#31", Radix::Hex, Absolute(31)),
        ("31d", Radix::Hex, Absolute(31)),
        ("31", Radix::Hex, Absolute(49)),
        ("31", Radix::Decimal, Absolute(31)),
        ("+0x10", Radix::Decimal, Relative(16)),
        ("-10", Radix::Decimal, Relative(-10)),
        ("end", Radix::Hex, End),
        ("0xFFFF_FFFF_FFFF_FFFF", Radix::Hex, Absolute(u64::MAX)),
    ] {
        assert_eq!(parse_address(text, radix), Ok(q), "{text}");
    }
    for bad in [
        "0x1_0000_0000_0000_0000",
        "99999999999999999999999",
        "zz",
        "-",
        "0x",
        "1 2",
    ] {
        assert!(parse_address(bad, Radix::Decimal).is_err(), "{bad}");
    }
    assert_eq!(Relative(-5).resolve(100, 200), Some((95, None)));
    let (clamped, warning) = Absolute(500).resolve(0, 200).unwrap();
    assert_eq!(clamped, 199);
    assert!(warning.unwrap().contains("past the end"));
    // Address widths: 8 hex digits at least, more for larger files.
    let (a, b) = pair();
    let (da, db) = (doc("a", &a), doc("b", &b));
    let al = align(&da, &db, &opts(AlignMode::None, 1), &Ctl::detached()).unwrap();
    let view = HexView::new(&da, &db, &al, HexLayout::default());
    assert_eq!(view.address_width(), 8);
    let dec = HexView::new(
        &da,
        &db,
        &al,
        HexLayout {
            address: Radix::Decimal,
            ..HexLayout::default()
        },
    );
    assert_eq!(dec.address_width(), 3);
    assert_eq!(
        address_text(Side::Left, 0x1F40, Radix::Hex, 8),
        "0x0000_1F40"
    );
    assert_eq!(
        address_text(Side::Left, 1 << 40, Radix::Hex, 8),
        "0x100_0000_0000"
    );
}

/// W13a: `column_at_offset` inverts `offsets_at` for every byte of each side; `step_store` equals `step` for the
/// existing-byte steps; the provisional scan equals the None alignment's scan; a filter cache reused by a second view
/// gives the same rows.
#[test]
fn w13a_store_helpers_and_the_provisional_scan() {
    use hikasa_hex::nav::{
        Step, scan_next_difference, scan_next_difference_provisional, step, step_store,
    };
    let a = common::XorShift(11).bytes(30_000);
    let b = common::edited(&a, &common::random_edits(a.len(), 8, 200, 5), 7);
    let (da, db) = (common::doc("a", &a), common::doc("b", &b));
    let ctl = hikasa_core::Ctl::detached();
    for mode in [
        hikasa_hex::AlignMode::None,
        hikasa_hex::AlignMode::Fast,
        hikasa_hex::AlignMode::Complete,
    ] {
        let al = hikasa_hex::align(&da, &db, &common::opts(mode, 1), &ctl).unwrap();
        let store = &al.segments;
        for (side, len) in [
            (hikasa_core::Side::Left, a.len()),
            (hikasa_core::Side::Right, b.len()),
        ] {
            for offset in (0..len as u64).step_by(97) {
                let col = store.column_at_offset(side, offset).unwrap();
                let (l, r) = store.offsets_at(col);
                assert_eq!(
                    if side == hikasa_core::Side::Left {
                        l
                    } else {
                        r
                    },
                    offset
                );
            }
            assert_eq!(store.column_at_offset(side, len as u64), None);
        }
        for col in (0..store.columns()).step_by(131) {
            for s in [
                Step::NextExisting(hikasa_core::Side::Left),
                Step::PrevExisting(hikasa_core::Side::Right),
                Step::First,
                Step::Last,
            ] {
                assert_eq!(step_store(store, col, s), step(&al, col, s));
            }
            assert_eq!(step_store(store, col, Step::NextDifference), None);
        }
        let layout = hikasa_hex::HexLayout::default();
        let view = hikasa_hex::HexView::new(&da, &db, &al, layout)
            .with_filter(hikasa_hex::DisplayFilter::Differences, 2);
        let n = view.row_count();
        let reused = hikasa_hex::HexView::new(&da, &db, &al, layout)
            .with_filter(hikasa_hex::DisplayFilter::Differences, 2)
            .reusing(&view.filter_cache());
        assert_eq!(reused.row_count(), n);
        assert_eq!(reused.rows(0, 50).unwrap(), view.rows(0, 50).unwrap());
    }
    let none = hikasa_hex::align(
        &da,
        &db,
        &common::opts(hikasa_hex::AlignMode::None, 1),
        &ctl,
    )
    .unwrap();
    let stream = common::stream(false, 1);
    for from in [0u64, 100, 5_000, 29_000] {
        assert_eq!(
            scan_next_difference_provisional(&da, &db, from, &stream, &ctl).unwrap(),
            scan_next_difference(&da, &db, &none, from, &stream, &ctl).unwrap()
        );
    }
}

/// The naive run after (before) the one holding `from` over column states (0 same).
fn naive_run(states: &[u8], from: u64, forward: bool) -> Option<std::ops::Range<u64>> {
    let n = states.len() as u64;
    let differs = |c: u64| states[c as usize] != 0;
    if n == 0 {
        return None;
    }
    if forward {
        let mut c = from;
        while c < n && differs(c) {
            c += 1;
        }
        while c < n && !differs(c) {
            c += 1;
        }
        if c >= n {
            return None;
        }
        let start = c;
        while c < n && differs(c) {
            c += 1;
        }
        Some(start..c)
    } else {
        let mut c = i128::from(from.min(n - 1));
        while c >= 0 && differs(c as u64) {
            c -= 1;
        }
        while c >= 0 && !differs(c as u64) {
            c -= 1;
        }
        if c < 0 {
            return None;
        }
        let end = c as u64 + 1;
        while c >= 0 && differs(c as u64) {
            c -= 1;
        }
        Some((c + 1) as u64..end)
    }
}

/// W13a review (D-358): `scan_difference_run` gives the differing run after (before) the caret's run, for every mode
/// and None's provisional layout, equal to a naive walk over the column states.
#[test]
fn w13a_review_difference_runs_equal_a_naive_walk() {
    use hikasa_hex::nav::scan_difference_run;
    let a = common::XorShift(31).bytes(20_000);
    let mut b = common::edited(&a, &common::random_edits(a.len(), 10, 300, 9), 3);
    // A long changed run, a run at the start and one at the end.
    for x in &mut b[5_000..7_500] {
        *x ^= 0xFF;
    }
    b[0] ^= 1;
    let last = b.len() - 1;
    b[last] ^= 1;
    let (da, db) = (doc("a", &a), doc("b", &b));
    let ctl = Ctl::detached();
    for mode in [AlignMode::None, AlignMode::Fast, AlignMode::Complete] {
        let al = align(&da, &db, &opts(mode, 1), &ctl).unwrap();
        let states = common::naive_states(&a, &b, &al.segments);
        let columns = al.segments.columns();
        let mut froms: Vec<u64> = (0..columns).step_by(173).collect();
        froms.extend([0, 1, 5_000, 6_000, 7_499, 7_500, columns - 1, columns + 5]);
        for &from in &froms {
            for forward in [true, false] {
                // Past the last column nothing follows; backwards the last column is the start.
                let want = if forward && from >= columns {
                    None
                } else {
                    naive_run(&states, from.min(columns - 1), forward)
                };
                assert_eq!(
                    scan_difference_run(&da, &db, Some(&al), from, forward, &ctl).unwrap(),
                    want,
                    "{mode} from {from} forward {forward}"
                );
            }
        }
        if mode == AlignMode::None {
            for &from in &froms {
                for forward in [true, false] {
                    assert_eq!(
                        scan_difference_run(&da, &db, None, from, forward, &ctl).unwrap(),
                        scan_difference_run(&da, &db, Some(&al), from, forward, &ctl).unwrap(),
                        "provisional from {from}"
                    );
                }
            }
        }
    }
    // Empty files: no run.
    let e = doc("e", &[]);
    assert_eq!(
        scan_difference_run(&e, &e, None, 0, true, &ctl).unwrap(),
        None
    );
    assert_eq!(
        scan_difference_run(&e, &e, None, 0, false, &ctl).unwrap(),
        None
    );
}
