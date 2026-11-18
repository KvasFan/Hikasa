//! A13.4 (W13a, D-357): `copy::selection_text` — the bytes of one side in a range of aligned columns (gap columns
//! contribute nothing) as hex pairs or characters, equal to a naive formatting of the file's bytes; the limit.

mod common;

use common::{XorShift, doc, edited, opts};
use hikasa_core::{Ctl, Side};
use hikasa_hex::copy::{CopyError, CopyForm, selection_range, selection_text};
use hikasa_hex::{AlignMode, CharSet, HexLayout, align};
use proptest::prelude::*;

fn naive_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn naive_chars(bytes: &[u8], set: CharSet) -> String {
    let layout = HexLayout {
        chars: set,
        ..HexLayout::default()
    };
    bytes
        .iter()
        .map(|&b| layout.char_of(b).unwrap_or('.'))
        .collect()
}

#[test]
fn copy_of_known_bytes() {
    let a = doc("a", b"Hi\x00\xde\xad\xbe\xef\xe9");
    let b = doc("b", b"Hi\xde\xad\xbe\xef");
    let al = align(&a, &b, &opts(AlignMode::Complete, 1), &Ctl::detached()).unwrap();
    let all = 0..al.segments.columns();
    assert_eq!(
        selection_text(
            &a,
            &al.segments,
            all.clone(),
            Side::Left,
            CopyForm::Hex,
            100
        )
        .unwrap(),
        "48 69 00 de ad be ef e9"
    );
    assert_eq!(
        selection_text(
            &b,
            &al.segments,
            all.clone(),
            Side::Right,
            CopyForm::Hex,
            100
        )
        .unwrap(),
        "48 69 de ad be ef"
    );
    assert_eq!(
        selection_text(
            &a,
            &al.segments,
            all.clone(),
            Side::Left,
            CopyForm::Chars(CharSet::Latin1),
            100
        )
        .unwrap(),
        "Hi.Þ\u{ad}¾ïé"
    );
    assert_eq!(
        selection_text(
            &a,
            &al.segments,
            all.clone(),
            Side::Left,
            CopyForm::Chars(CharSet::None),
            100
        )
        .unwrap(),
        "Hi......",
        "no character column: Copy as Text copies ASCII"
    );
    assert!(matches!(
        selection_text(&a, &al.segments, all, Side::Left, CopyForm::Hex, 7),
        Err(CopyError::TooLarge { bytes: 8, limit: 7 })
    ));
    // An empty range and a range past the end.
    assert_eq!(
        selection_text(&a, &al.segments, 3..3, Side::Left, CopyForm::Hex, 1).unwrap(),
        ""
    );
    assert_eq!(selection_range(&al.segments, 0..1_000, Side::Left), 0..8);
}

proptest! {
    #![proptest_config(common::config_with("tests/proptest-regressions/copy.txt", 64))]

    /// A13.4: for random pairs, modes and column ranges, the copy of each side equals the naive formatting of the
    /// side's bytes in those columns (found by walking the segments), in both forms.
    #[test]
    fn p_copy_equals_naive((len, seed, n) in (0usize..6_000, any::<u64>(), 0usize..5), cuts in (any::<u64>(), any::<u64>())) {
        let a = XorShift(seed | 1).bytes(len);
        let b = edited(&a, &common::random_edits(len.max(1), n, 200, seed ^ 7), seed);
        let (da, db) = (doc("a", &a), doc("b", &b));
        for mode in [AlignMode::None, AlignMode::Fast, AlignMode::Complete] {
            let al = align(&da, &db, &opts(mode, 1), &Ctl::detached()).unwrap();
            let cols = al.segments.columns();
            let (x, y) = (cuts.0 % (cols + 1), cuts.1 % (cols + 1));
            let range = x.min(y)..x.max(y);
            // The naive bytes of each side in the columns.
            let (mut nl, mut nr) = (Vec::new(), Vec::new());
            let mut col = 0u64;
            for s in al.segments.iter() {
                for i in 0..s.len {
                    if range.contains(&col) {
                        if s.left_len() > 0 { nl.push(a[(s.left + i) as usize]); }
                        if s.right_len() > 0 { nr.push(b[(s.right + i) as usize]); }
                    }
                    col += 1;
                }
            }
            for (side, d, naive) in [(Side::Left, &da, &nl), (Side::Right, &db, &nr)] {
                prop_assert_eq!(
                    selection_text(d, &al.segments, range.clone(), side, CopyForm::Hex, u64::MAX).unwrap(),
                    naive_hex(naive)
                );
                prop_assert_eq!(
                    selection_text(d, &al.segments, range.clone(), side, CopyForm::Chars(CharSet::Ascii), u64::MAX).unwrap(),
                    naive_chars(naive, CharSet::Ascii)
                );
            }
        }
    }
}
