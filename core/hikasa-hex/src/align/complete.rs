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

/// None-style: paired over the shorter side, the rest one-sided at the end.
pub(crate) fn none_style(out: &mut Pushes, left: u64, right: u64) {
    out.push((SegmentKind::Paired, left.min(right)));
    if left > right {
        out.push((SegmentKind::LeftOnly, left - right));
    } else if right > left {
        out.push((SegmentKind::RightOnly, right - left));
    }
}

/// Aligns the gap `l × r` byte-level with windows of `window` bytes (the walk above).
pub(crate) fn align_gap(
    left: &HexDocument,
    l: Range<u64>,
    right: &HexDocument,
    r: Range<u64>,
    window: u64,
    cancel: &CancelToken,
) -> Result<Pushes, HexError> {
    let half = window / 2;
    let mut out = Pushes::new();
    let (mut a, mut c) = (l.start, r.start);
    let (mut la, mut lb) = (Vec::new(), Vec::new());
    loop {
        cancel.check()?;
        let (rest_l, rest_r) = (l.end - a, r.end - c);
        if rest_l == 0 || rest_r == 0 {
            none_style(&mut out, rest_l, rest_r);
            break;
        }
        let (lw, rw) = (rest_l.min(window), rest_r.min(window));
        la.resize(lw as usize, 0);
        lb.resize(rw as usize, 0);
        left.read_bulk(a, &mut la)?;
        right.read_bulk(c, &mut lb)?;
        let runs = myers(&la, &lb, cancel)?;
        let equal: u64 = runs
            .iter()
            .filter(|run| run.kind == RunKind::Equal)
            .map(|run| span(&run.left))
            .sum();
        if equal * 8 < lw.min(rw) {
            // Unrelated bytes: Myers' worst case for nothing.
            none_style(&mut out, rest_l, rest_r);
            break;
        }
        if lw == rest_l && rw == rest_r {
            commit(&mut out, &runs);
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
                a += u64::from(runs[k].left.end);
                c += u64::from(runs[k].right.end);
            }
            None => {
                let k = half.min(rest_l).min(rest_r);
                out.push((SegmentKind::Paired, k));
                a += k;
                c += k;
            }
        }
    }
    Ok(out)
}
