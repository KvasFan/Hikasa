//! A12.4 (D-329): Fast alignment, and P-HEXALIGN for every mode.
//!
//! - `GEAR` pinned (the CRC of the table); chunk cuts identical across threads and ways of reading (and pinned);
//! - after an insertion of 1 byte, 1 KiB and 1 MiB at the start, the middle and the end, the blocks
//!   resynchronize within one block (golden segment lists); a moved block, a deletion, a replacement;
//! - P-HEXALIGN for every mode: segments cover each file once in order, statistics and sections equal a naive
//!   walk over the segments, `compare(a, a)` is one `Paired` segment, swapping sides swaps `LeftOnly` and
//!   `RightOnly` and the statistics; the same result with 1, 4 and 16 threads.

mod common;

use common::{Edit, XorShift, check_alignment, doc, dump, edited, golden, opts};
use hikasa_core::Ctl;
use hikasa_hex::align::cdc::{GEAR, document_blocks};
use hikasa_hex::{AlignMode, HexAlignOptions, HexAlignment, SegmentKind, align};
use proptest::prelude::*;

#[test]
fn gear_is_pinned() {
    let bytes: Vec<u8> = GEAR.iter().flat_map(|g| g.to_le_bytes()).collect();
    assert_eq!(hikasa_vfs::crc32_bytes(&bytes), 0x921F_7AD9, "GEAR changed");
}

/// The block cuts of a 20 MiB file (three visit chunks) are the same for every way of reading and thread count,
/// and pinned by their CRC.
#[test]
fn chunk_cuts_are_identical_and_pinned() {
    let data = XorShift(42).bytes(20 << 20);
    let mut reference: Option<Vec<(u64, u32, u64)>> = None;
    common::every_reading(&data, &data, &mut |label, l, _, stream| {
        let blocks: Vec<(u64, u32, u64)> = document_blocks(l, 8192, &stream, &Ctl::detached())
            .unwrap()
            .into_iter()
            .map(|b| (b.offset, b.len, b.hash))
            .collect();
        match &reference {
            None => reference = Some(blocks),
            Some(r) => assert!(r == &blocks, "{label} {}", stream.threads),
        }
    });
    let blocks = reference.unwrap();
    // Every visit chunk edge (8 MiB) is a cut.
    for edge in [8u64 << 20, 16 << 20] {
        assert!(blocks.iter().any(|b| b.0 == edge), "no cut at {edge}");
    }
    let bytes: Vec<u8> = blocks
        .iter()
        .flat_map(|b| {
            [
                b.0.to_le_bytes(),
                u64::from(b.1).to_le_bytes(),
                b.2.to_le_bytes(),
            ]
            .concat()
        })
        .collect();
    assert_eq!(
        (blocks.len(), hikasa_vfs::crc32_bytes(&bytes)),
        (2138, 0x0A71_0509),
        "the cuts or the block hash changed"
    );
}

fn fast(a: &[u8], b: &[u8]) -> HexAlignment {
    align(
        &doc("a", a),
        &doc("b", b),
        &opts(AlignMode::Fast, 4),
        &Ctl::detached(),
    )
    .unwrap()
}

/// The segment list of a Fast alignment, compactly.
fn segments(al: &HexAlignment) -> String {
    let mut out = String::new();
    for s in al.segments.iter() {
        out.push_str(&format!(
            "  {} {} {} {}\n",
            s.kind.as_str(),
            s.left,
            s.right,
            s.len
        ));
    }
    out
}

#[test]
fn insertions_resynchronize_within_one_block() {
    let base = XorShift(7).bytes(4 << 20);
    let mut out = String::new();
    for (where_, at) in [("start", 0usize), ("middle", 2 << 20), ("end", 4 << 20)] {
        for len in [1usize, 1 << 10, 1 << 20] {
            let b = edited(&base, &[Edit::Insert { at, len }], 99);
            let al = fast(&base, &b);
            check_alignment(&base, &b, &al).unwrap();
            // Everything but the inserted bytes and at most one block around them is paired and same.
            assert!(
                al.stats.same_bytes >= base.len() as u64 - 64 * 1024,
                "{where_} {len}: {}",
                al.stats.same_bytes
            );
            out.push_str(&format!("== insert {len} at {where_}\n{}", segments(&al)));
        }
    }
    // A moved block, a deletion, a replacement.
    let mut moved = base.clone();
    let block: Vec<u8> = moved.drain(1_000_000..1_100_000).collect();
    moved.splice(3_000_000..3_000_000, block);
    let deleted = edited(
        &base,
        &[Edit::Delete {
            at: 1_234_567,
            len: 5000,
        }],
        1,
    );
    let replaced = edited(
        &base,
        &[Edit::Replace {
            at: 2_345_678,
            len: 300,
        }],
        1,
    );
    for (name, b) in [
        ("moved", &moved),
        ("deleted", &deleted),
        ("replaced", &replaced),
    ] {
        let al = fast(&base, b);
        check_alignment(&base, b, &al).unwrap();
        out.push_str(&format!("== {name}\n{}", segments(&al)));
    }
    golden("align_fast_edits", &out);
}

/// A pair: a base and an edited copy (up to 8 edits of up to 3,000 bytes), or unrelated bytes.
fn pair() -> impl Strategy<Value = (Vec<u8>, Vec<u8>)> {
    (
        prop_oneof![12 => 0usize..40_000, 1 => 60_000usize..120_000],
        any::<u64>(),
        0usize..8,
        prop_oneof![8 => Just(false), 1 => Just(true)],
        any::<bool>(),
    )
        .prop_map(|(len, seed, edits, unrelated, low_entropy)| {
            let mut a = XorShift(seed | 1).bytes(len);
            if low_entropy && len <= 40_000 {
                // Text-like bytes: few distinct values, so blocks and byte runs repeat.
                for x in &mut a {
                    *x = b'a' + *x % 4;
                }
            }
            let b = if unrelated {
                XorShift(seed ^ 0xFFFF).bytes(len / 2 + 17)
            } else {
                edited(
                    &a,
                    &common::random_edits(len.max(1), edits, 3000, seed ^ 3),
                    seed,
                )
            };
            (a, b)
        })
}

/// `al` with the sides swapped: (left-only, right-only) swapped, and the statistics.
fn swapped_stats(al: &HexAlignment) -> (u64, u64, u64, u64) {
    let s = al.stats;
    (
        s.same_bytes,
        s.differing_bytes,
        s.right_only_bytes,
        s.left_only_bytes,
    )
}

proptest! {
    #![proptest_config(common::config_with("tests/proptest-regressions/align_fast.txt", 6))]

    /// P-HEXALIGN for every mode.
    #[test]
    fn p_hexalign((a, b) in pair()) {
        for mode in [AlignMode::None, AlignMode::Fast, AlignMode::Complete] {
            let o = |threads| HexAlignOptions { window: 4096, ..opts(mode, threads) };
            let al = align(&doc("a", &a), &doc("b", &b), &o(1), &Ctl::detached()).unwrap();
            prop_assert_eq!(check_alignment(&a, &b, &al), Ok(()), "{}", mode);
            for threads in [4, 16] {
                let other = align(&doc("a", &a), &doc("b", &b), &o(threads), &Ctl::detached()).unwrap();
                prop_assert!(other == al, "{} threads {}", mode, threads);
            }
            // Swapping sides swaps the one-sided kinds and the statistics.
            let back = align(&doc("b", &b), &doc("a", &a), &o(1), &Ctl::detached()).unwrap();
            prop_assert_eq!(check_alignment(&b, &a, &back), Ok(()));
            prop_assert_eq!(
                (back.stats.same_bytes, back.stats.differing_bytes, back.stats.left_only_bytes, back.stats.right_only_bytes),
                swapped_stats(&al),
                "{} swapped", mode
            );
            // A file against itself: one paired segment.
            let same = align(&doc("a", &a), &doc("a", &a), &o(4), &Ctl::detached()).unwrap();
            let segs: Vec<_> = same.segments.iter().collect();
            prop_assert!(a.is_empty() || (segs.len() == 1 && segs[0].kind == SegmentKind::Paired), "{}", mode);
            prop_assert!(same.stats.identical());
        }
    }
}

#[test]
fn a_dump_of_a_small_edit() {
    let a = XorShift(5).bytes(100_000);
    let b = edited(
        &a,
        &[
            Edit::Insert { at: 50_000, len: 3 },
            Edit::Replace { at: 10, len: 2 },
        ],
        2,
    );
    golden("align_fast_small", &dump(&fast(&a, &b)));
}
