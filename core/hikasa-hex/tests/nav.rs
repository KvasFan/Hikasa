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
