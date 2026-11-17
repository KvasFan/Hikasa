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
    #![proptest_config(common::config_with("tests/proptest-regressions/align_complete.txt", 96))]

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
    // One gap of 60,000 bytes (blocks of 1 MiB average: the whole file is one block) with an insertion every
    // 3,000 bytes and a changed byte every 1,500, aligned in windows of 4 KiB that commit up to W/2.
    let a = XorShift(3).bytes(60_000);
    let mut b = a.clone();
    for k in (0..18).rev() {
        b.splice(4_000 + k * 3000..4_000 + k * 3000, [0xAA ^ k as u8, 0x55]);
    }
    for at in (2_000..58_000).step_by(1500) {
        b[at] ^= 1;
    }
    // Insertions of 1,500 random bytes, which some windows cut: committing past W/2 would keep pairings decided
    // without the bytes after the window.
    for (k, at) in [52_000usize, 43_100, 31_700, 19_300, 7_900]
        .into_iter()
        .enumerate()
    {
        b.splice(at..at, XorShift(100 + k as u64).bytes(1500));
    }
    let o = HexAlignOptions {
        block: 1 << 20,
        window: 4096,
        ..opts(AlignMode::Complete, 1)
    };
    let walked = run(AlignMode::Complete, o, &a, &b);
    assert_eq!(walked.stats.right_only_bytes, 36 + 5 * 1500);
    assert_eq!(walked.stats.left_only_bytes, 0);
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
    // Three regions of 400 KiB where every 1,000th byte changed (no block matches: a gap of about 800 KiB each) and
    // one byte is inserted at the start: byte-level alignment finds the insertion, a degraded gap pairs the shifted
    // bytes. 1 MiB of budget fits the first gap only.
    let a = XorShift(5).bytes(4 << 20);
    let mut b = a.clone();
    let regions = [500_000usize, 1_700_000, 2_900_000];
    for &start in &regions {
        for at in (start..start + 400_000).step_by(1000) {
            b[at] ^= 0x20;
        }
    }
    for &start in regions.iter().rev() {
        b.insert(start, 0x77);
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
    // The budget is charged in gap order: the first region is aligned byte by byte (its changed bytes only), the
    // second and third are paired shifted (almost every byte differs).
    let in_region = |k: usize| {
        let r = regions[k] as u64..regions[k] as u64 + 400_000;
        one.sections
            .iter(&one.segments)
            .filter(|s| r.contains(&s.left.start))
            .map(|s| s.non_same)
            .sum::<u64>()
    };
    assert!(in_region(0) < 1_000, "{}", in_region(0));
    assert!(in_region(1) > 300_000 && in_region(2) > 300_000);
    // With the default budget nothing degrades.
    let full = run(AlignMode::Complete, opts(AlignMode::Complete, 4), &a, &b);
    assert!(!full.degraded && full.warnings.is_empty());
}
