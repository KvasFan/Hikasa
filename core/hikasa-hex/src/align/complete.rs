//! The byte-level window of Complete, also Fast's refinement of small gaps (D-330, normative).
//!
//! A gap `L[a, b) × R[c, d)` with both sides at most `W` bytes is aligned with `hikasa_diff`'s Myers on byte
//! tokens; equal and position-paired changed bytes become `Paired` runs, the rest one-sided. A larger gap is
//! walked: align `L[a, min(b, a+W)) × R[c, min(d, c+W))`, commit the operations up to the last equal run that ends
//! within the first `W/2` bytes of both windows, advance past it, repeat; without such a run, commit
//! `min(W/2, remaining)` bytes paired and advance. A window whose equal bytes are fewer than 1/8 of its shorter
//! side is *unrelated*: the rest of the gap is laid out None-style.

use std::ops::Range;

use hikasa_core::CancelToken;
use hikasa_diff::{Algorithm, AlignInput, AlignOptions, Run, RunKind, TokenId, TrivialSet};

use super::store::SegmentKind;
use crate::document::HexDocument;
use crate::error::HexError;

/// The pushes of an aligned gap, in order.
pub(crate) type Pushes = Vec<(SegmentKind, u64)>;

/// Myers on two byte windows.
fn myers(a: &[u8], b: &[u8], cancel: &CancelToken) -> Result<Vec<Run>, HexError> {
    let left: Vec<TokenId> = a.iter().map(|&x| TokenId(u32::from(x))).collect();
    let right: Vec<TokenId> = b.iter().map(|&x| TokenId(u32::from(x))).collect();
    let trivial = TrivialSet::with_capacity(256);
    let input = AlignInput {
        left: &left,
        right: &right,
        left_weights: &[],
        right_weights: &[],
        trivial: &trivial,
        distinct: 256,
    };
    let opts = AlignOptions {
        closeness: None,
        ..AlignOptions::with_algorithm(Algorithm::Myers)
    };
    let alignment = hikasa_diff::align(&input, &opts, None, None, cancel)?;
    Ok(alignment.runs().to_vec())
}

fn span(r: &Range<u32>) -> u64 {
    u64::from(r.end - r.start)
}

/// Appends runs as pushes.
fn commit(out: &mut Pushes, runs: &[Run]) {
    for run in runs {
        match run.kind {
            RunKind::Equal | RunKind::Changed => out.push((SegmentKind::Paired, span(&run.left))),
            RunKind::Delete => out.push((SegmentKind::LeftOnly, span(&run.left))),
            RunKind::Insert => out.push((SegmentKind::RightOnly, span(&run.right))),
        }
    }
}

/// Deleted plus inserted bytes of Myers' alignment of `a` and `b` (a changed run counts both ways).
pub(crate) fn myers_cost(a: &[u8], b: &[u8]) -> Result<u64, HexError> {
    let runs = myers(a, b, &CancelToken::never())?;
    let equal: u64 = runs
        .iter()
        .filter(|run| run.kind == RunKind::Equal)
        .map(|run| span(&run.left))
        .sum();
    Ok(a.len() as u64 + b.len() as u64 - 2 * equal)
}

/// Non-same columns of Myers' runs over `a` and `b`.
fn non_same_runs(runs: &[Run], a: &[u8], b: &[u8]) -> u64 {
    runs.iter()
        .map(|run| match run.kind {
            RunKind::Equal => 0,
            RunKind::Changed => {
                let (l, r) = (run.left.start as usize, run.right.start as usize);
                let n = span(&run.left) as usize;
                a[l..l + n]
                    .iter()
                    .zip(&b[r..r + n])
                    .filter(|(x, y)| x != y)
                    .count() as u64
            }
            RunKind::Delete => span(&run.left),
            RunKind::Insert => span(&run.right),
        })
        .sum()
}

/// Non-same columns of the None-style layout of `a` and `b`.
fn non_same_none(a: &[u8], b: &[u8]) -> u64 {
    let paired = a.iter().zip(b).filter(|(x, y)| x != y).count();
    (paired + a.len().abs_diff(b.len())) as u64
}

/// The layout of a gap too large to align byte-level (Fast's large gaps, Complete's degraded ones): None-style,
/// with the one-sided rest at the gap's end — or at its start when the shorter side is at most `probe` bytes and
/// that pairing has fewer differing bytes (an insertion before matching bytes, which the blocks cut off).
/// Returns the pushes and, when the bytes were compared, their non-same columns.
pub(crate) fn large_gap(
    left: &HexDocument,
    l: Range<u64>,
    right: &HexDocument,
    r: Range<u64>,
    probe: u64,
    cancel: &CancelToken,
) -> Result<(Pushes, Option<u64>), HexError> {
    let (ll, rl) = (l.end - l.start, r.end - r.start);
    let mut out = Pushes::new();
    let short = ll.min(rl);
    if short == 0 || short > probe || ll == rl {
        none_style(&mut out, ll, rl);
        return Ok((out, (short == 0).then_some(ll + rl)));
    }
    cancel.check()?;
    let (s_doc, s_at, long_doc, long) = if ll < rl {
        (left, l.start, right, r)
    } else {
        (right, r.start, left, l)
    };
    let mut s = vec![0u8; short as usize];
    let mut head = vec![0u8; short as usize];
    let mut tail = vec![0u8; short as usize];
    s_doc.read_bulk(s_at, &mut s)?;
    long_doc.read_bulk(long.start, &mut head)?;
    long_doc.read_bulk(long.end - short, &mut tail)?;
    let at_end = non_same_none(&s, &head);
    let at_start = non_same_none(&s, &tail);
    let rest = ll.abs_diff(rl);
    if at_start < at_end {
        out.push((
            if ll > rl {
                SegmentKind::LeftOnly
            } else {
                SegmentKind::RightOnly
            },
            rest,
        ));
        out.push((SegmentKind::Paired, short));
        Ok((out, Some(at_start + rest)))
    } else {
        none_style(&mut out, ll, rl);
        Ok((out, Some(at_end + ll.abs_diff(rl))))
    }
}

/// None-style: paired over the shorter side, the rest one-sided at the end.
pub(crate) fn none_style(out: &mut Pushes, left: u64, right: u64) {
    out.push((SegmentKind::Paired, left.min(right)));
    if left > right {
        out.push((SegmentKind::LeftOnly, left - right));
    } else if right > left {
        out.push((SegmentKind::RightOnly, right - left));
    }
}

/// Fewer shared 8-byte grams than this fraction of a window's shorter side make it unrelated without Myers.
const GRAM: usize = 8;
const GRAM_SHARE_DIVISOR: usize = 16;
/// Windows whose shorter side is below this run Myers without the gram probe.
const PROBE_MIN: usize = 256;

/// True when fewer than 1/16 of the positions of the shorter window start an 8-byte gram that also occurs in the
/// other window: Myers would find only coincidental bytes there (about 11 % of the shorter side on random bytes,
/// under the 1/8 rule) at its worst-case cost.
fn grams_unrelated(a: &[u8], b: &[u8]) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if short.len() < PROBE_MIN {
        return false;
    }
    let gram = |w: &[u8]| {
        let mut x = [0u8; 8];
        x.copy_from_slice(w);
        u64::from_le_bytes(x)
    };
    let set: std::collections::HashSet<u64> = long.windows(GRAM).map(gram).collect();
    let shared = short
        .windows(GRAM)
        .filter(|w| set.contains(&gram(w)))
        .count();
    shared * GRAM_SHARE_DIVISOR < short.len()
}

/// Non-same columns of `n` bytes paired from `l0` and `r0` (read in 1 MiB steps).
pub(crate) fn paired_cost(
    left: &HexDocument,
    l0: u64,
    right: &HexDocument,
    r0: u64,
    n: u64,
    cancel: &CancelToken,
) -> Result<u64, HexError> {
    const STEP: u64 = 1 << 20;
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let mut cost = 0;
    let mut done = 0;
    while done < n {
        cancel.check()?;
        let k = STEP.min(n - done);
        a.resize(k as usize, 0);
        b.resize(k as usize, 0);
        left.read_bulk(l0 + done, &mut a)?;
        right.read_bulk(r0 + done, &mut b)?;
        cost += a.iter().zip(&b).filter(|(x, y)| x != y).count() as u64;
        done += k;
    }
    Ok(cost)
}

/// Aligns the gap `l × r` byte-level with windows of `window` bytes (the walk above). Returns the pushes and
/// their non-same columns.
pub(crate) fn align_gap(
    left: &HexDocument,
    l: Range<u64>,
    right: &HexDocument,
    r: Range<u64>,
    window: u64,
    cancel: &CancelToken,
) -> Result<(Pushes, u64), HexError> {
    let half = window / 2;
    let mut out = Pushes::new();
    let mut cost = 0u64;
    let (mut a, mut c) = (l.start, r.start);
    let (mut la, mut lb) = (Vec::new(), Vec::new());
    loop {
        cancel.check()?;
        let (rest_l, rest_r) = (l.end - a, r.end - c);
        if rest_l == 0 || rest_r == 0 {
            none_style(&mut out, rest_l, rest_r);
            cost += rest_l + rest_r;
            break;
        }
        let (lw, rw) = (rest_l.min(window), rest_r.min(window));
        la.resize(lw as usize, 0);
        lb.resize(rw as usize, 0);
        left.read_bulk(a, &mut la)?;
        right.read_bulk(c, &mut lb)?;
        let unrelated = |out: &mut Pushes| -> Result<u64, HexError> {
            // Unrelated bytes: Myers' worst case for nothing; the rest of the gap None-style.
            none_style(out, rest_l, rest_r);
            Ok(paired_cost(left, a, right, c, rest_l.min(rest_r), cancel)?
                + rest_l.abs_diff(rest_r))
        };
        if grams_unrelated(&la, &lb) {
            cost += unrelated(&mut out)?;
            break;
        }
        let runs = myers(&la, &lb, cancel)?;
        let equal: u64 = runs
            .iter()
            .filter(|run| run.kind == RunKind::Equal)
            .map(|run| span(&run.left))
            .sum();
        if equal * 8 < lw.min(rw) {
            cost += unrelated(&mut out)?;
            break;
        }
        if lw == rest_l && rw == rest_r {
            // The window holds the whole rest: Myers' layout, unless the None-style layout has fewer non-same
            // columns (random replaced bytes stay one changed run instead of a fragmented LCS).
            let (mine, none) = (non_same_runs(&runs, &la, &lb), non_same_none(&la, &lb));
            if none < mine {
                none_style(&mut out, rest_l, rest_r);
                cost += none;
            } else {
                commit(&mut out, &runs);
                cost += mine;
            }
            break;
        }
        let cut = runs.iter().rposition(|run| {
            run.kind == RunKind::Equal
                && u64::from(run.left.end) <= half
                && u64::from(run.right.end) <= half
        });
        match cut {
            Some(k) => {
                commit(&mut out, &runs[..=k]);
                cost += non_same_runs(&runs[..=k], &la, &lb);
                a += u64::from(runs[k].left.end);
                c += u64::from(runs[k].right.end);
            }
            None => {
                let k = half.min(rest_l).min(rest_r);
                out.push((SegmentKind::Paired, k));
                cost += non_same_none(&la[..k as usize], &lb[..k as usize]);
                a += k;
                c += k;
            }
        }
    }
    Ok((out, cost))
}
