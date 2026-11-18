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

static UNIQUE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A block hash under which no two blocks are equal: the whole pair is one gap.
fn unique(_: &[u8]) -> u64 {
    UNIQUE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn non_same(al: &HexAlignment) -> (u64, u64, u64) {
    (
        al.stats.differing_bytes,
        al.stats.left_only_bytes,
        al.stats.right_only_bytes,
    )
}

/// D-329 step 3: a gap whose sides are both at most 64 KiB is aligned byte-level, a larger one None-style. A
/// 65,536-byte side with a byte deleted in the middle is one left-only byte; one byte more and the gap is laid
/// out None-style (its one-sided byte at the start: the bytes before the deletion differ).
#[test]
fn a_gap_of_64_kib_is_refined_and_a_larger_one_is_not() {
    use hikasa_hex::align::testing::align_blocks_with_hash;
    let ctl = Ctl::detached();
    for len in [64 << 10, (64 << 10) + 1] {
        let a = XorShift(31).bytes(len);
        let mut b = a.clone();
        b.remove(30_000);
        let al = align_blocks_with_hash(
            &doc("a", &a),
            &doc("b", &b),
            &opts(AlignMode::Fast, 1),
            false,
            unique,
            &ctl,
        )
        .unwrap();
        check_alignment(&a, &b, &al).unwrap();
        let (differing, left_only, right_only) = non_same(&al);
        if len <= HexAlignOptions::FAST_REFINE_MAX as usize {
            assert_eq!((differing, left_only, right_only), (0, 1, 0), "{len}");
        } else {
            assert_eq!((left_only, right_only), (1, 0), "{len}");
            assert!(differing > 20_000, "{len}: {differing}");
        }
    }
}

/// Ties keep the block layout: "AB" against "BA" is one left-only and one right-only byte (Myers' layout), as
/// the None layout's two differing bytes are not fewer non-same columns (§1.12.33 W12a note 14).
#[test]
fn a_tie_with_the_none_layout_keeps_the_block_layout() {
    for mode in [AlignMode::Fast, AlignMode::Complete] {
        let al = align(
            &doc("a", b"AB"),
            &doc("b", b"BA"),
            &opts(mode, 1),
            &Ctl::detached(),
        )
        .unwrap();
        assert_eq!(non_same(&al), (0, 1, 1), "{mode}");
    }
}

/// A large gap whose one-sided rest costs the same at its start and at its end keeps it at the end (None-style):
/// 100 KiB of zeros against 32 KiB of ones.
#[test]
fn a_large_gap_tie_puts_the_rest_at_the_end() {
    let (a, b) = (vec![0u8; 100 << 10], vec![1u8; 32 << 10]);
    let al = fast(&a, &b);
    assert_eq!(
        segments(&al),
        format!(
            "  paired 0 0 {}\n  left_only {} 32768 {}\n",
            32 << 10,
            32 << 10,
            68 << 10
        )
    );
}

/// The skew (D-329 step 2: `max(1, skew / block)` blocks) bounds where equal blocks are paired: 256 KiB shared
/// between a 64 KiB prefix and a 64 KiB suffix (64 blocks from both diagonals) pairs under the default skew, not
/// under a skew of 4 KiB (4 blocks).
#[test]
fn the_skew_bounds_paired_blocks() {
    let common = XorShift(8).bytes(256 << 10);
    let mut a = XorShift(9).bytes(64 << 10);
    a.extend_from_slice(&common);
    a.extend_from_slice(&XorShift(10).bytes(64 << 10));
    let run = |skew: u64| {
        let o = HexAlignOptions {
            block: 1 << 10,
            skew,
            ..opts(AlignMode::Fast, 1)
        };
        let al = align(&doc("a", &a), &doc("b", &common), &o, &Ctl::detached()).unwrap();
        check_alignment(&a, &common, &al).unwrap();
        non_same(&al)
    };
    assert_eq!(run(HexAlignOptions::DEFAULT_SKEW), (0, 128 << 10, 0));
    let (differing, _, _) = run(4 << 10);
    assert!(differing > 100_000, "{differing}");
}

/// The 8-byte-gram probe (§1.12.33 W12a note 15): a window whose shorter side shares grams at 1/10 of its
/// positions is related and goes to Myers. The right side is the left one shifted by a byte, with its first 400
/// bytes clean and every 4th byte after them replaced (no shared gram there); Myers pairs about 3/4 of it.
#[test]
fn a_window_sharing_a_tenth_of_its_grams_is_related() {
    let n = 4096;
    let a = XorShift(44).bytes(n);
    let mut b: Vec<u8> = a[1..].to_vec();
    b.push(0);
    for i in (400..n).filter(|i| i % 4 == 0) {
        b[i] ^= 0x5A;
    }
    for mode in [AlignMode::Fast, AlignMode::Complete] {
        let al = align(
            &doc("a", &a),
            &doc("b", &b),
            &opts(mode, 1),
            &Ctl::detached(),
        )
        .unwrap();
        check_alignment(&a, &b, &al).unwrap();
        assert!(
            al.stats.same_bytes > (n / 2) as u64,
            "{mode}: {:?}",
            al.stats
        );
    }
}
