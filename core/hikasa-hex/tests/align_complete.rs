//! A12.5 (D-330): Complete alignment.
//!
//! - a 1-byte insertion inside a large gap that Fast lays out as changed bytes is a 1-byte `RightOnly` under
//!   Complete;
//! - on random pairs of at most 4 KiB with up to 8 edits, Complete's non-same columns ≤ Fast's ≤ None's, and the
//!   byte-level Myers' edit cost equals the LCS cost (dynamic programming) on inputs of at most 256 bytes;
//! - a gap larger than the window walks with the `W/2` rule (golden); an unrelated gap is laid out None-style;
//! - a budget of 1 MiB degrades the same gaps with 1 and 16 threads and warns `alignment_degraded`.

mod common;

use common::{Edit, XorShift, check_alignment, doc, dump, edited, golden, opts};
use hikasa_core::Ctl;
use hikasa_hex::align::testing::myers_cost;
use hikasa_hex::{AlignMode, HexAlignOptions, HexAlignment, HexWarningCode, SegmentKind, align};
use proptest::prelude::*;

fn run(mode: AlignMode, o: HexAlignOptions, a: &[u8], b: &[u8]) -> HexAlignment {
    let al = align(
        &doc("a", a),
        &doc("b", b),
        &HexAlignOptions { mode, ..o },
        &Ctl::detached(),
    )
    .unwrap();
    check_alignment(a, b, &al).unwrap();
    al
}

fn non_same(al: &HexAlignment) -> u64 {
    let s = al.stats;
    s.differing_bytes + s.left_only_bytes + s.right_only_bytes
}

#[test]
fn an_insertion_in_a_large_gap() {
    // 100 KiB where every 2 KiB a byte is replaced (no block matches, a gap above 64 KiB), with one inserted byte
    // in its middle.
    let a = XorShift(1).bytes(400_000);
    let mut b = a.clone();
    for at in (150_000..250_000).step_by(2048) {
        b[at] ^= 0x40;
    }
    let b = edited(
        &b,
        &[Edit::Insert {
            at: 200_001,
            len: 1,
        }],
        9,
    );
    let o = opts(AlignMode::Fast, 4);
    let fast = run(AlignMode::Fast, o, &a, &b);
    let complete = run(AlignMode::Complete, o, &a, &b);
    assert!(fast.stats.differing_bytes > 10_000, "{:?}", fast.stats);
    assert_eq!(complete.stats.right_only_bytes, 1, "{:?}", complete.stats);
    assert_eq!(complete.stats.left_only_bytes, 0);
    assert_eq!(complete.stats.differing_bytes, 49);
}

/// The LCS edit cost `|a| + |b| − 2·LCS` by dynamic programming.
fn lcs_cost(a: &[u8], b: &[u8]) -> u64 {
    let mut prev = vec![0u32; b.len() + 1];
    for &x in a {
        let mut cur = vec![0u32; b.len() + 1];
        for (j, &y) in b.iter().enumerate() {
            cur[j + 1] = if x == y {
                prev[j] + 1
            } else {
                prev[j + 1].max(cur[j])
            };
        }
        prev = cur;
    }
    (a.len() + b.len()) as u64 - 2 * u64::from(prev[b.len()])
}

fn small_pair(max: usize) -> impl Strategy<Value = (Vec<u8>, Vec<u8>, bool)> {
    (0..=max, any::<u64>(), 0usize..=8, any::<bool>()).prop_map(move |(len, seed, n, text)| {
        let mut a = XorShift(seed | 1).bytes(len);
        if text {
            for x in &mut a {
                *x = b'a' + *x % 3;
            }
        }
        let b = edited(
            &a,
            &common::random_edits(len.max(1), n, (len / 8).max(1), seed ^ 7),
            seed,
        );
        (a, b, text)
    })
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/align_complete.txt"))]

    /// Complete ≤ Fast ≤ None in non-same columns.
    #[test]
    fn complete_is_at_most_fast_at_most_none((a, b, _) in small_pair(4096)) {
        let o = HexAlignOptions { block: 1024, window: 4096, ..opts(AlignMode::Fast, 1) };
        let none = non_same(&run(AlignMode::None, o, &a, &b));
        let fast = non_same(&run(AlignMode::Fast, o, &a, &b));
        let complete = non_same(&run(AlignMode::Complete, o, &a, &b));
        prop_assert!(complete <= fast && fast <= none, "{} {} {}", complete, fast, none);
    }

    /// The byte-level Myers' edit cost is the LCS cost on inputs of at most 256 bytes over the whole byte range;
    /// over a 3-letter alphabet imara-diff's preprocessing (it prunes tokens frequent on the other side) makes it a
    /// heuristic: never below the LCS cost, at most half again above it.
    #[test]
    fn myers_cost_is_the_lcs_cost((a, b, text) in small_pair(256)) {
        let (got, best) = (myers_cost(&a, &b).unwrap(), lcs_cost(&a, &b));
        if text {
            prop_assert!(best <= got && got <= best + best / 2, "{} vs {}", got, best);
        } else {
            prop_assert_eq!(got, best);
        }
    }
}

#[test]
fn a_large_gap_walks_and_an_unrelated_one_is_none_style() {
    // A gap of about 20 KiB with an insertion every 3 KiB, aligned in windows of 4 KiB.
    let a = XorShift(3).bytes(60_000);
    let mut b = a.clone();
    for k in (0..6).rev() {
        b.insert(20_000 + k * 3000, 0xAA ^ k as u8);
    }
    for at in (20_000..40_000).step_by(1500) {
        b[at] ^= 1;
    }
    let o = HexAlignOptions {
        block: 1024,
        window: 4096,
        ..opts(AlignMode::Complete, 1)
    };
    let walked = run(AlignMode::Complete, o, &a, &b);
    assert_eq!(walked.stats.right_only_bytes, 6);
    // Unrelated files: one gap, laid out None-style.
    let c = XorShift(4).bytes(70_000);
    let unrelated = run(AlignMode::Complete, o, &a, &c);
    // None-style: 60,000 paired bytes and 10,000 right-only ones at one end of the gap.
    let mut segs: Vec<_> = unrelated.segments.iter().map(|s| (s.kind, s.len)).collect();
    segs.sort_by_key(|s| s.1);
    assert_eq!(
        segs,
        [
            (SegmentKind::RightOnly, 10_000),
            (SegmentKind::Paired, 60_000)
        ]
    );
    golden(
        "align_complete_walk",
        &format!("{}== unrelated\n{}", dump(&walked), dump(&unrelated)),
    );
}

#[test]
fn the_budget_degrades_the_same_gaps_with_any_thread_count() {
    // Three regions of 400 KiB replaced by unrelated bytes, each a gap of about 800 KiB: the first fits in 1 MiB.
    let a = XorShift(5).bytes(4 << 20);
    let mut b = a.clone();
    for start in [500_000usize, 1_700_000, 2_900_000] {
        let fresh = XorShift(start as u64).bytes(400_000);
        b[start..start + 400_000].copy_from_slice(&fresh);
    }
    let o = |threads| HexAlignOptions {
        budget_mib: 1,
        ..opts(AlignMode::Complete, threads)
    };
    let one = run(AlignMode::Complete, o(1), &a, &b);
    let many = run(AlignMode::Complete, o(16), &a, &b);
    assert_eq!(one, many);
    assert!(one.degraded);
    let w: Vec<_> = one
        .warnings
        .iter()
        .filter(|w| w.code == HexWarningCode::AlignmentDegraded)
        .collect();
    assert_eq!(w.len(), 1);
    assert!(w[0].message.contains("2 gap(s)"), "{}", w[0].message);
    // With the default budget nothing degrades.
    let full = run(AlignMode::Complete, opts(AlignMode::Complete, 4), &a, &b);
    assert!(!full.degraded && full.warnings.is_empty());
}
