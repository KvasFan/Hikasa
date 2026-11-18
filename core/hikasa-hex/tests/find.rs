//! A12.9 (D-334): find.
//!
//! - P-HEXFIND: `all` equals a naive search for random documents and patterns (literal, with `??`, overlapping
//!   hits), with hits planted across page and chunk edges, mapped, streamed and from memory with 1, 4 and 16
//!   threads;
//! - text in each encoding, ASCII case folding on and off; `next` forward and backward, with and without wrap;
//!   refused patterns; limits and `limit_reached`; cancellation.

mod common;

use common::{XorShift, doc};
use hikasa_core::{Classify, Ctl, ErrorKind};
use hikasa_hex::find::parse_hex_bytes;
use hikasa_hex::{Direction, HexError, HexFinder, HexPattern, TextEncoding};
use hikasa_vfs::{StreamOptions, VISIT_CHUNK};
use proptest::prelude::*;

const CHUNK: usize = VISIT_CHUNK as usize;
const PAGE: usize = 64 << 10;

fn naive(hay: &[u8], pattern: &[Option<u8>]) -> Vec<u64> {
    if hay.len() < pattern.len() {
        return Vec::new();
    }
    (0..=hay.len() - pattern.len())
        .filter(|&s| {
            pattern
                .iter()
                .zip(&hay[s..])
                .all(|(p, &b)| p.is_none_or(|p| p == b))
        })
        .map(|s| s as u64)
        .collect()
}

fn bytes_pattern(p: &[Option<u8>]) -> HexFinder {
    HexFinder::new(&HexPattern::Bytes { bytes: p.to_vec() }).unwrap()
}

/// A document with a pattern planted at page and chunk edges (and anywhere), and a pattern with wildcards.
fn case() -> impl Strategy<Value = (Vec<u8>, Vec<Option<u8>>)> {
    (
        prop_oneof![6 => 0usize..300_000, 1 => (CHUNK - 100)..(CHUNK + 5000)],
        any::<u64>(),
        prop::collection::vec(
            prop_oneof![3 => any::<u8>().prop_map(Some), 1 => Just(None)],
            1..12,
        ),
        prop::collection::vec((any::<usize>(), any::<bool>(), 0usize..3), 0..10),
        any::<bool>(),
    )
        .prop_filter("not all wildcards", |(_, _, p, _, _)| {
            p.iter().any(Option::is_some)
        })
        .prop_map(|(len, seed, pattern, plants, small_alphabet)| {
            let mut hay = XorShift(seed | 1).bytes(len);
            if small_alphabet {
                // Few distinct bytes: many (and overlapping) hits.
                for x in &mut hay {
                    *x %= 2;
                }
            }
            let concrete: Vec<u8> = pattern.iter().map(|b| b.unwrap_or(0x5A)).collect();
            for (at, edge, back) in plants {
                if hay.len() < concrete.len() {
                    break;
                }
                let max = hay.len() - concrete.len();
                let at = if edge {
                    // Across a page or chunk edge.
                    let unit = if hay.len() > CHUNK && at % 2 == 0 {
                        CHUNK
                    } else {
                        PAGE
                    };
                    let edge_at = (at / 2 % (hay.len() / unit).max(1) + 1) * unit;
                    edge_at.saturating_sub(back + 1).min(max)
                } else {
                    at % (max + 1)
                };
                hay[at..at + concrete.len()].copy_from_slice(&concrete);
            }
            let pattern = if small_alphabet {
                pattern.iter().map(|b| b.map(|x| x % 2)).collect()
            } else {
                pattern
            };
            (hay, pattern)
        })
}

proptest! {
    #![proptest_config(common::config_with("tests/proptest-regressions/find.txt", 32))]

    /// P-HEXFIND.
    #[test]
    fn p_hexfind((hay, pattern) in case()) {
        let expected = naive(&hay, &pattern);
        let finder = bytes_pattern(&pattern);
        let mut failures = Vec::new();
        common::every_reading(&hay, &[], &mut |label, d, _, stream| {
            let (hits, reached) = finder.all(d, u64::MAX - 1, &stream, &Ctl::detached()).unwrap();
            if hits != expected || reached {
                failures.push(format!("{label}/{}: {} hits, expected {}", stream.threads, hits.len(), expected.len()));
            }
            // The first hit forward from 0, and the last one backward from the end.
            let first = finder.next(d, 0, Direction::Forward, false, &stream, &Ctl::detached()).unwrap();
            let last = finder.next(d, hay.len() as u64, Direction::Backward, false, &stream, &Ctl::detached()).unwrap();
            let plen = pattern.len() as u64;
            if first != expected.first().map(|&s| s..s + plen) || last != expected.last().map(|&s| s..s + plen) {
                failures.push(format!("{label}/{}: next", stream.threads));
            }
        });
        prop_assert!(failures.is_empty(), "{:?}", failures);
    }
}

/// A hit at every position across the first chunk edge, mapped, streamed and in memory: found exactly once.
#[test]
fn hits_across_the_chunk_edge() {
    let pattern: Vec<Option<u8>> = vec![Some(9), Some(8), None, Some(7), Some(6), Some(5)];
    let concrete = [9u8, 8, 0, 7, 6, 5];
    let f = bytes_pattern(&pattern);
    for back in 1..pattern.len() {
        let mut hay = vec![0u8; CHUNK + 64];
        let at = CHUNK - back;
        hay[at..at + 6].copy_from_slice(&concrete);
        common::every_reading(&hay, &[], &mut |label, d, _, stream| {
            if stream.threads == 16 {
                return;
            }
            let (hits, _) = f.all(d, 10, &stream, &Ctl::detached()).unwrap();
            assert_eq!(hits, [at as u64], "{label} {} back {back}", stream.threads);
        });
    }
}

/// Hits across the edges of the windows of the visit (2 × threads chunks), not only of its chunks: `all`, and
/// `next` forward from every hit + 1 and backward from every hit.
#[test]
fn hits_across_window_edges() {
    let pat = [0xA5u8, 0x5A, 0xC3, 0x3C, 0x99, 0x66, 0x11];
    let f = bytes_pattern(&pat.map(Some));
    let ctl = Ctl::detached();
    for threads in [1usize, 2] {
        let window = 2 * CHUNK * threads;
        let mut hay = XorShift(5).bytes(2 * window + 1000);
        // One hit an edge (planted hits must not overlap): 1 byte before the window edges (its last 6 bytes in
        // the next window), 3 and 6 bytes before chunk edges, and 2 bytes before a window edge again.
        for (edge, back) in [
            (CHUNK, 3),
            (window, 1),
            (window + CHUNK, 6),
            (2 * window, 2),
        ] {
            let at = edge - back;
            hay[at..at + pat.len()].copy_from_slice(&pat);
        }
        let planted = naive(&hay, &pat.map(Some));
        assert_eq!(planted.len(), 4);
        let d = doc("d", &hay);
        let s = StreamOptions {
            threads,
            ..StreamOptions::default()
        };
        let (all, _) = f.all(&d, u64::MAX - 1, &s, &ctl).unwrap();
        assert_eq!(all, planted, "threads {threads}");
        for (k, &h) in planted.iter().enumerate() {
            let next = f
                .next(&d, h + 1, Direction::Forward, false, &s, &ctl)
                .unwrap();
            assert_eq!(
                next.map(|r| r.start),
                planted.get(k + 1).copied(),
                "from {h}"
            );
            let prev = f.next(&d, h, Direction::Backward, false, &s, &ctl).unwrap();
            assert_eq!(
                prev.map(|r| r.start),
                k.checked_sub(1).map(|j| planted[j]),
                "back from {h}"
            );
        }
    }
}

#[test]
fn text_in_every_encoding_and_case_folding() {
    let s = StreamOptions::default();
    let ctl = Ctl::detached();
    let find = |hay: &[u8], text: &str, encoding, match_case| {
        let f = HexFinder::new(&HexPattern::Text {
            text: text.into(),
            encoding,
            match_case,
        })
        .unwrap();
        f.all(&doc("d", hay), 100, &s, &ctl).unwrap().0
    };
    let utf8 = "xx Héllo HÉLLO hello".as_bytes();
    assert_eq!(find(utf8, "héllo", TextEncoding::Utf8, false), [3]);
    assert_eq!(find(utf8, "hello", TextEncoding::Utf8, false), [17]);
    assert_eq!(
        find(utf8, "HELLO", TextEncoding::Utf8, true),
        Vec::<u64>::new()
    );
    let latin1: Vec<u8> = b"..Caf\xE9 caf\xE9".to_vec();
    assert_eq!(find(&latin1, "café", TextEncoding::Latin1, false), [2, 7]);
    assert_eq!(find(&latin1, "café", TextEncoding::Latin1, true), [7]);
    let le: Vec<u8> = "aAb".encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert_eq!(find(&le, "ab", TextEncoding::Utf16Le, false), [2]);
    assert_eq!(
        find(&le, "ab", TextEncoding::Utf16Le, true),
        Vec::<u64>::new()
    );
    let be: Vec<u8> = "zAb".encode_utf16().flat_map(u16::to_be_bytes).collect();
    assert_eq!(find(&be, "aB", TextEncoding::Utf16Be, false), [2]);
    // Only the low byte of an ASCII code unit folds: U+0141 (Ł: 01 41) and U+0161 (š: 01 61) hold the bytes of
    // `A` and `a` but are other letters.
    for (encoding, be) in [
        (TextEncoding::Utf16Be, true),
        (TextEncoding::Utf16Le, false),
    ] {
        let hay: Vec<u8> = "šŁ"
            .encode_utf16()
            .flat_map(|u| if be { u.to_be_bytes() } else { u.to_le_bytes() })
            .collect();
        assert_eq!(find(&hay, "Ł", encoding, false), [2], "{encoding:?}");
        assert_eq!(find(&hay, "š", encoding, false), [0], "{encoding:?}");
    }
    // Overlapping hits.
    assert_eq!(find(b"aaa", "aa", TextEncoding::Utf8, true), [0, 1]);
}

#[test]
fn next_with_and_without_wrap() {
    let hay = b"ab..ab..ab";
    let f = HexFinder::new(&HexPattern::Bytes {
        bytes: parse_hex_bytes("61 62").unwrap(),
    })
    .unwrap();
    let d = doc("d", hay);
    let s = StreamOptions::default();
    let ctl = Ctl::detached();
    let next = |from, dir, wrap| {
        f.next(&d, from, dir, wrap, &s, &ctl)
            .unwrap()
            .map(|r| r.start)
    };
    assert_eq!(next(0, Direction::Forward, false), Some(0));
    assert_eq!(next(1, Direction::Forward, false), Some(4));
    assert_eq!(next(9, Direction::Forward, false), None);
    assert_eq!(next(9, Direction::Forward, true), Some(0));
    assert_eq!(next(8, Direction::Backward, false), Some(4));
    assert_eq!(next(9, Direction::Backward, false), Some(8));
    assert_eq!(next(0, Direction::Backward, false), None);
    assert_eq!(next(0, Direction::Backward, true), Some(8));
    assert_eq!(next(3, Direction::Backward, true), Some(0));
}

#[test]
fn limits_and_refusals() {
    let hay = vec![7u8; 1000];
    let f = HexFinder::new(&HexPattern::Bytes {
        bytes: vec![Some(7), None],
    })
    .unwrap();
    let s = StreamOptions::default();
    let (hits, reached) = f.all(&doc("d", &hay), 10, &s, &Ctl::detached()).unwrap();
    assert_eq!((hits.len(), reached), (10, true));
    let (hits, reached) = f.all(&doc("d", &hay), 999, &s, &Ctl::detached()).unwrap();
    assert_eq!((hits.len(), reached), (999, false));
    for bad in [vec![], vec![None], vec![Some(1); 4097]] {
        let e = HexFinder::new(&HexPattern::Bytes { bytes: bad }).unwrap_err();
        assert_eq!(HexError::from(e).kind(), ErrorKind::InvalidInput);
    }
    assert!(parse_hex_bytes("12 3").is_err());
}

#[test]
fn cancellation() {
    let hay = XorShift(1).bytes(3 * CHUNK);
    let f = HexFinder::new(&HexPattern::Bytes {
        bytes: vec![Some(1), Some(2), Some(3), Some(4)],
    })
    .unwrap();
    let token = common::cancelled();
    let ctl = hikasa_core::Ctl {
        cancel: &token,
        progress: &hikasa_core::NullProgress,
    };
    let err = f
        .all(&doc("d", &hay), 10, &StreamOptions::default(), &ctl)
        .unwrap_err();
    assert!(err.is_cancelled(), "{err:?}");
}
