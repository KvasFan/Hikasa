//! A12.3 (D-327, D-328): None alignment and sections.
//!
//! - goldens of equal files, a one-byte change, runs 7 and 8 equal bytes apart, a longer right file, empty files;
//! - P-NONE: sections and statistics equal a naive byte loop's for random pairs with planted runs across chunk
//!   edges, with the same result mapped, streamed and from memory with 1, 4 and 16 threads;
//! - `max_sections` 4: `sections_truncated` and exact statistics; the thumbnail's buckets sum to the statistics.

mod common;

use common::{XorShift, check_alignment, doc, dump, golden, naive_sections, naive_states, opts};
use hikasa_core::Ctl;
use hikasa_hex::{AlignMode, HexAlignOptions, HexWarningCode, align_none};
use hikasa_vfs::VISIT_CHUNK;
use proptest::prelude::*;

const CHUNK: usize = VISIT_CHUNK as usize;

fn none(a: &[u8], b: &[u8]) -> hikasa_hex::HexAlignment {
    align_none(
        &doc("a", a),
        &doc("b", b),
        &opts(AlignMode::None, 1),
        &Ctl::detached(),
    )
    .unwrap()
}

#[test]
fn goldens() {
    let base: Vec<u8> = (0..64u8).collect();
    let mut one = base.clone();
    one[10] = 0xFF;
    let mut runs = base.clone();
    runs[3] = 0xEE; // 7 same columns (4..11) before the next run: merged
    runs[11] = 0xEE;
    runs[20] = 0xEE; // 8 same columns (12..20): a new section
    let mut longer = base.clone();
    longer.extend_from_slice(b"tail");
    let mut cases = String::new();
    for (name, a, b) in [
        ("equal", &base, &base),
        ("one_byte", &base, &one),
        ("runs_7_and_8_apart", &base, &runs),
        ("longer_right", &base, &longer),
        ("empty", &Vec::new(), &Vec::new()),
        ("empty_left", &Vec::new(), &base[..5].to_vec()),
    ] {
        let al = none(a, b);
        check_alignment(a, b, &al).unwrap();
        cases.push_str(&format!("== {name}\n{}", dump(&al)));
    }
    golden("align_none", &cases);
}

/// A pair with planted differing runs, some across the chunk edges, and a tail.
fn pair() -> impl Strategy<Value = (Vec<u8>, Vec<u8>)> {
    (
        prop_oneof![12 => 0usize..20_000, 1 => (CHUNK - 600)..(CHUNK + 600), 1 => (2 * CHUNK - 10)..(2 * CHUNK + 40)],
        any::<u64>(),
        prop::collection::vec((any::<u64>(), 1usize..70, any::<bool>()), 0..12),
        0usize..40,
        any::<bool>(),
    )
        .prop_map(|(len, seed, runs, tail, left_longer)| {
            let a = XorShift(seed | 1).bytes(len);
            let mut b = a.clone();
            let mut rng = XorShift(seed ^ 0x55);
            for (at, n, edge) in runs {
                if len == 0 {
                    break;
                }
                // Near a chunk edge (± 40 bytes) or anywhere.
                let at = if edge && len > CHUNK {
                    (CHUNK - 40 + (at % 80) as usize).min(len - 1)
                } else {
                    (at % len as u64) as usize
                };
                for x in b.iter_mut().skip(at).take(n) {
                    *x ^= 1 + (rng.next() % 255) as u8;
                }
            }
            let extra = XorShift(seed ^ 0xAA).bytes(tail);
            if left_longer {
                let mut a = a;
                a.extend_from_slice(&extra);
                (a, b)
            } else {
                b.extend_from_slice(&extra);
                (a, b)
            }
        })
}

proptest! {
    #![proptest_config(common::config_with("tests/proptest-regressions/align_none.txt", 32))]

    /// P-NONE.
    #[test]
    fn p_none((a, b) in pair()) {
        let reference = none(&a, &b);
        prop_assert_eq!(check_alignment(&a, &b, &reference), Ok(()));
        let mut failures = Vec::new();
        common::every_reading(&a, &b, &mut |label, l, r, stream| {
            let o = HexAlignOptions { stream, ..opts(AlignMode::None, 1) };
            let al = align_none(l, r, &o, &Ctl::detached()).unwrap();
            if al != reference {
                failures.push(format!("{label}/{}", stream.threads));
            }
        });
        prop_assert!(failures.is_empty(), "{:?}", failures);
    }
}

#[test]
fn truncated_sections_keep_exact_statistics() {
    let a = vec![0u8; 1000];
    let mut b = a.clone();
    for k in 0..20 {
        b[k * 40] = 1;
    }
    b.extend_from_slice(&[7; 5]);
    let o = HexAlignOptions {
        max_sections: 4,
        ..opts(AlignMode::None, 1)
    };
    let al = align_none(&doc("a", &a), &doc("b", &b), &o, &Ctl::detached()).unwrap();
    assert!(al.sections.truncated());
    assert_eq!(al.sections.len(), 4);
    assert_eq!(al.stats.sections, 21);
    assert_eq!(al.stats.differing_bytes, 20);
    assert_eq!(al.stats.right_only_bytes, 5);
    assert_eq!(al.stats.same_bytes, 980);
    assert!(
        al.warnings
            .iter()
            .any(|w| w.code == HexWarningCode::SectionsTruncated)
    );
    let states = naive_states(&a, &b, &al.segments);
    assert_eq!(&common::sections_of(&al)[..], &naive_sections(&states)[..4]);
    let sum: u64 = al
        .thumbnail
        .buckets
        .iter()
        .map(|t| u64::from(t.same + t.different + t.left_only + t.right_only))
        .sum();
    assert_eq!(sum, al.segments.columns());
}
