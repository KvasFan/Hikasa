//! Fast (D-329) and Complete (D-330): content-defined blocks aligned with `hikasa_diff`, then the gaps.
//!
//! 1. Each side is chunked into content-defined blocks ([`super::cdc`]).
//! 2. The block sequences are interned on 16-byte keys (hash, length) and aligned with `Algorithm::Standard`,
//!    `never_align_differences = false`, no closeness, a skew of `max(1, skew / block)` blocks and the automatic
//!    work budget. Equal tokens become `Paired` segments; the rest are gaps.
//! 3. Fast: a gap whose sides are both at most 64 KiB is aligned byte-level as Complete aligns a window; a larger
//!    gap is laid out None-style. Complete: every gap byte-level, within `budget_mib`, which is charged before any
//!    gap runs, in gap order, each gap its own size (left + right bytes); a gap whose charge would exceed what
//!    remains is laid out as Fast lays out a large gap and the result is `degraded`. Gaps are aligned in
//!    parallel and committed in order (the same gaps degrade with any thread count).
//! 4. The statistics pass over the paired segments ([`super::scan_paired`]).

use std::ops::Range;

use hikasa_core::Ctl;
use hikasa_diff::{Algorithm, AlignInput, AlignOptions, RunKind, TrivialSet, intern_pair};
use rayon::prelude::*;

use super::cdc::{Block, chunk_document};
use super::complete::{Pushes, align_gap, large_gap, none_style, paired_cost};
use super::store::{SegmentKind, StoreBuilder};
use super::{AlignMode, HexAlignOptions, HexAlignment, finish, scan_paired};
use crate::document::HexDocument;
use crate::error::HexError;

/// A piece of the block outline.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    /// Paired bytes (equal block tokens).
    Paired(u64),
    /// A gap between paired runs.
    Gap { l: Range<u64>, r: Range<u64> },
}

/// What happens to a gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    /// One side is empty: one-sided.
    OneSided,
    /// Byte-level.
    Refine,
    /// None-style (a large gap of Fast, a degraded gap of Complete).
    NoneStyle,
}

/// The 16-byte interning key of a block: its hash and its length.
fn key(block: &Block) -> [u8; 16] {
    let mut k = [0u8; 16];
    k[..8].copy_from_slice(&block.hash.to_le_bytes());
    k[8..].copy_from_slice(&u64::from(block.len).to_le_bytes());
    k
}

/// Steps 1–2: the outline of paired runs and gaps.
fn outline(
    bl: &[Block],
    br: &[Block],
    left_len: u64,
    right_len: u64,
    skew_blocks: u64,
    ctl: &Ctl<'_>,
) -> Result<Vec<Piece>, HexError> {
    let kl: Vec<[u8; 16]> = bl.iter().map(key).collect();
    let kr: Vec<[u8; 16]> = br.iter().map(key).collect();
    let pair = intern_pair(kl.iter().map(|k| &k[..]), kr.iter().map(|k| &k[..]))?;
    drop((kl, kr));
    let trivial = TrivialSet::with_capacity(pair.distinct);
    let input = AlignInput {
        left: &pair.left,
        right: &pair.right,
        left_weights: &[],
        right_weights: &[],
        trivial: &trivial,
        distinct: pair.distinct,
    };
    let opts = AlignOptions {
        algorithm: Algorithm::Standard,
        skew_tolerance: u32::try_from(skew_blocks.max(1)).unwrap_or(u32::MAX),
        closeness: None,
        never_align_differences: false,
        work_budget: None,
    };
    let alignment = hikasa_diff::align(&input, &opts, None, None, ctl.cancel)?;
    let mut pieces = Vec::new();
    let (mut lpos, mut rpos) = (0u64, 0u64);
    let gap = |pieces: &mut Vec<Piece>, l: Range<u64>, r: Range<u64>| {
        if !l.is_empty() || !r.is_empty() {
            pieces.push(Piece::Gap { l, r });
        }
    };
    for run in alignment.runs() {
        if run.kind != RunKind::Equal || run.left.is_empty() {
            continue;
        }
        let (Some(first_l), Some(last_l), Some(first_r)) = (
            bl.get(run.left.start as usize),
            bl.get(run.left.end as usize - 1),
            br.get(run.right.start as usize),
        ) else {
            continue;
        };
        let len = last_l.offset + u64::from(last_l.len) - first_l.offset;
        gap(&mut pieces, lpos..first_l.offset, rpos..first_r.offset);
        match pieces.last_mut() {
            Some(Piece::Paired(n)) => *n += len,
            _ => pieces.push(Piece::Paired(len)),
        }
        lpos = first_l.offset + len;
        rpos = first_r.offset + len;
    }
    gap(&mut pieces, lpos..left_len, rpos..right_len);
    Ok(pieces)
}

/// Non-same columns of the None layout: differing paired bytes (through the pair visitor) plus the tail.
fn none_cost(
    left: &HexDocument,
    right: &HexDocument,
    opts: &HexAlignOptions,
    ctl: &Ctl<'_>,
) -> Result<u64, HexError> {
    let paired = left.len().min(right.len());
    let f = |_: u64, a: &[u8], b: &[u8]| {
        let n = if a == b {
            0
        } else {
            a.iter().zip(b).filter(|(x, y)| x != y).count() as u64
        };
        std::ops::ControlFlow::<u64, u64>::Continue(n)
    };
    let visit = hikasa_vfs::visit_chunk_pairs(
        left.source(),
        right.source(),
        0..paired,
        &opts.stream,
        ctl,
        &f,
    )
    .map_err(|e| HexError::from_vfs(left.display(), e))?;
    Ok(visit.values.iter().sum::<u64>() + left.len().abs_diff(right.len()))
}

/// The orientation key of a side: its length, then a fold of its block hashes and lengths.
fn order_key(blocks: &[Block], len: u64) -> (u64, u64) {
    let fold = blocks.iter().fold(0x48_49_4B_41u64, |h, b| {
        (h ^ b.hash ^ u64::from(b.len))
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .rotate_left(23)
    });
    (len, fold)
}

/// Fast (`complete = false`) or Complete with the block hash `hash`.
pub(crate) fn align_blocks(
    left: &HexDocument,
    right: &HexDocument,
    opts: &HexAlignOptions,
    complete: bool,
    hash: fn(&[u8]) -> u64,
    ctl: &Ctl<'_>,
) -> Result<HexAlignment, HexError> {
    let avg = opts.effective_block();
    let bl = chunk_document(left, avg, &opts.stream, hash, ctl)?;
    let br = chunk_document(right, avg, &opts.stream, hash, ctl)?;
    ctl.check()?;
    // Align in a canonical orientation (the smaller order key first) and mirror the result, so swapping the
    // inputs mirrors the alignment exactly whatever the engines' tie-breaking.
    let swapped = order_key(&br, right.len()) < order_key(&bl, left.len());
    let (left, right, bl, br) = if swapped {
        (right, left, br, bl)
    } else {
        (left, right, bl, br)
    };
    let pieces = outline(&bl, &br, left.len(), right.len(), opts.skew / avg, ctl)?;
    drop((bl, br));
    // Plans, the budget charged in gap order before anything runs.
    let mut budget = opts.budget_mib.saturating_mul(1 << 20);
    let mut degraded: Option<(u64, u64, u64)> = None; // (first left, first right, count)
    let plans: Vec<Plan> = pieces
        .iter()
        .map(|piece| match piece {
            Piece::Paired(_) => Plan::OneSided,
            Piece::Gap { l, r } if l.is_empty() || r.is_empty() => Plan::OneSided,
            Piece::Gap { l, r } if !complete => {
                let small = HexAlignOptions::FAST_REFINE_MAX;
                if l.end - l.start <= small && r.end - r.start <= small {
                    Plan::Refine
                } else {
                    Plan::NoneStyle
                }
            }
            Piece::Gap { l, r } => {
                let charge = (l.end - l.start) + (r.end - r.start);
                if charge <= budget {
                    budget -= charge;
                    Plan::Refine
                } else {
                    let d = degraded.get_or_insert((l.start, r.start, 0));
                    d.2 += 1;
                    Plan::NoneStyle
                }
            }
        })
        .collect();
    let window = opts.effective_window();
    let small = HexAlignOptions::FAST_REFINE_MAX;
    let refined: Vec<Option<Result<Pushes, HexError>>> = pieces
        .par_iter()
        .zip(plans.par_iter())
        .map(|(piece, plan)| match (piece, plan) {
            (Piece::Gap { l, r }, Plan::Refine) => {
                let large = l.end - l.start > small || r.end - r.start > small;
                Some(
                    align_gap(left, l.clone(), right, r.clone(), window, ctl.cancel).and_then(
                        |(walked, cost)| {
                            if !large {
                                return Ok(walked);
                            }
                            // Complete on a gap Fast lays out as a large gap: the better of the two layouts,
                            // so Complete never shows more non-same columns than Fast.
                            let (laid, laid_cost) =
                                large_gap(left, l.clone(), right, r.clone(), small, ctl.cancel)?;
                            let laid_cost = match laid_cost {
                                Some(c) => c,
                                None => {
                                    let (ll, rl) = (l.end - l.start, r.end - r.start);
                                    paired_cost(
                                        left,
                                        l.start,
                                        right,
                                        r.start,
                                        ll.min(rl),
                                        ctl.cancel,
                                    )? + ll.abs_diff(rl)
                                }
                            };
                            Ok(if laid_cost < cost { laid } else { walked })
                        },
                    ),
                )
            }
            (Piece::Gap { l, r }, Plan::NoneStyle) => Some(
                large_gap(left, l.clone(), right, r.clone(), small, ctl.cancel).map(|(p, _)| p),
            ),
            _ => None,
        })
        .collect();
    let mut builder = StoreBuilder::default();
    let mut push = |kind: SegmentKind, len: u64| {
        let kind = match (swapped, kind) {
            (true, SegmentKind::LeftOnly) => SegmentKind::RightOnly,
            (true, SegmentKind::RightOnly) => SegmentKind::LeftOnly,
            (_, kind) => kind,
        };
        builder.push(kind, len);
    };
    for ((piece, plan), refined) in pieces.iter().zip(&plans).zip(refined) {
        match (piece, plan, refined) {
            (Piece::Paired(n), _, _) => push(SegmentKind::Paired, *n),
            (_, _, Some(pushes)) => {
                for (kind, len) in pushes? {
                    push(kind, len);
                }
            }
            (Piece::Gap { l, r }, _, None) => {
                let mut pushes = Pushes::new();
                none_style(&mut pushes, l.end - l.start, r.end - r.start);
                for (kind, len) in pushes {
                    push(kind, len);
                }
            }
        }
    }
    let store = builder.finish();
    ctl.check()?;
    let (left, right) = if swapped {
        (right, left)
    } else {
        (left, right)
    };
    let (sections, same, different, thumbnail) = scan_paired(left, right, &store, opts, ctl)?;
    let degraded = degraded.map(|(l, r, n)| if swapped { (r, l, n) } else { (l, r, n) });
    let degraded = degraded.map(|(l, r, n)| {
        format!(
            "Complete alignment ran out of its budget of {} MiB: {n} gap(s) from left offset {l} and right offset \
             {r} on were not aligned byte by byte",
            opts.budget_mib
        )
    });
    let mode = if complete {
        AlignMode::Complete
    } else {
        AlignMode::Fast
    };
    let result = finish(
        mode, store, sections, same, different, thumbnail, degraded, opts,
    );
    // The None layout is a candidate too: when pairing byte n with byte n leaves fewer non-same columns (a few
    // replaced bytes that spurious block matches or Myers' fragments would shift), it is the result.
    let mine =
        result.stats.differing_bytes + result.stats.left_only_bytes + result.stats.right_only_bytes;
    if mine > left.len().abs_diff(right.len()) {
        let none = none_cost(left, right, opts, ctl)?;
        if none < mine {
            let mut al = super::align_none(left, right, opts, ctl)?;
            al.mode = mode;
            return Ok(al);
        }
    }
    Ok(result)
}
