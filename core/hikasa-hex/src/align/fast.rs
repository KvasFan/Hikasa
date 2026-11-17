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
use super::complete::{Pushes, align_gap, none_style};
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
    let refined: Vec<Option<Result<Pushes, HexError>>> = pieces
        .par_iter()
        .zip(plans.par_iter())
        .map(|(piece, plan)| match (piece, plan) {
            (Piece::Gap { l, r }, Plan::Refine) => Some(align_gap(
                left,
                l.clone(),
                right,
                r.clone(),
                window,
                ctl.cancel,
            )),
            _ => None,
        })
        .collect();
    let mut builder = StoreBuilder::default();
    for ((piece, plan), refined) in pieces.iter().zip(&plans).zip(refined) {
        match (piece, plan, refined) {
            (Piece::Paired(n), _, _) => builder.push(SegmentKind::Paired, *n),
            (_, _, Some(pushes)) => {
                for (kind, len) in pushes? {
                    builder.push(kind, len);
                }
            }
            (Piece::Gap { l, r }, _, None) => {
                let mut pushes = Pushes::new();
                none_style(&mut pushes, l.end - l.start, r.end - r.start);
                for (kind, len) in pushes {
                    builder.push(kind, len);
                }
            }
        }
    }
    let store = builder.finish();
    ctl.check()?;
    let (sections, same, different, thumbnail) = scan_paired(left, right, &store, opts, ctl)?;
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
    Ok(finish(
        mode, store, sections, same, different, thumbnail, degraded, opts,
    ))
}
