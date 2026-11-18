//! The exact incremental recompare (normative, §1.10.11; D-269).
//!
//! ```text
//! step 1  dirty lines: the deltas merged into spans per side, re-keyed (old and new), the tokenizer re-converged;
//!         a span whose keys and weights are unchanged is dirty for the rows only (re-classified)
//! step 2  descend the segment's derivation from its root while the node's certificate holds:
//!           1 range     the dirty lines inside the stripped middle; the prefix and suffix keep their lengths
//!                       (the boundary pairs still differ); Auto resolves to the recorded engine
//!           2 candidates an AnchorsUnique node; no key of a dirty line is or becomes unique on both sides in the
//!                       middle (counts exact: a walk of the interned ids, stopped once every key is complete)
//!           3 window    per diagonal run of the node's unique pairs: membership before and after, exact
//!         a certified node keeps its chain; blocks near dirty lines are re-extended from their anchors; its
//!         changed gaps are recursed into (a recorded child of the same range) or re-run by the engine; the first
//!         node whose certificate fails is re-run whole by its engine on locally interned keys
//!           4 budgets   the totals (an upper bound: old totals plus every re-run's and re-extension's work) stay
//!                       within the budgets of the new line counts; else the full compare
//! step 3  the runs canonicalized; rows reused where the pairing of clean lines is the old one, else classified;
//!         sections, statistics and warnings re-derived; the derivation, ids (changed chunks only) and checkpoints updated
//! T6      dirty lines over 25 % of the lines or 200,000 lines, anchors, engines without nodes, a recorded
//!         budget refusal: the full compare (the same result)
//! ```
//!
//! Any doubt (a condition that cannot be shown cheaply) fails the certificate: a failed certificate only costs
//! time, never exactness.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use hikasa_core::cancel::{CancelToken, Cancelled};
use hikasa_core::progress::Ctl;
use hikasa_core::warning::{Warning, WarningCode};
use hikasa_diff::{
    Algorithm, AlignInput, AlignOptions, Closeness, NodeId, Run, RunKind, SegmentTrace, Stage,
    TokenId, TraceNode, TrivialSet, WorkTotals, align_traced, canonical_runs, intern_pair,
};
use hikasa_formats::LineState;

use super::dirty::{Keyed, Keyer, LineMap, Rekeyed, SideView};
use super::ids::{IdMap, IdView, LiveNode, LiveSegment, LiveTrace, TableDelta};
use super::recompare::{PathCounters, RecompareJob};
use super::{LiveKeys, splice};
use crate::classify::KeyShape;
use crate::compare::keys::KeyStore;
use crate::compare::rows::{self, Importance, RowState};
use crate::compare::score::{ProfileScorer, scorer_budget};
use crate::compare::{
    CHECKPOINT_LINES, TextComparison, WarningInputs, compose_warnings, truncation_warning,
};
use crate::document::LineIdx;
use crate::error::TextError;
use crate::rules::AlgorithmChoice;

/// T6: the incremental path is not taken beyond this share of dirty lines.
const FALLBACK_SHARE: f64 = 0.25;
/// T6: … or beyond this many dirty lines.
const FALLBACK_LINES: u64 = 200_000;
/// T6: the incremental work may always take this much, more only up to an eighth of the lines (`Inc::spend`).
const WORK_SHARE_LIMIT: u64 = 65_536;
/// T6: moved lines (between a middle and its ends) cost one unit of work per this many (a table lookup each,
/// against a re-run line's keying and alignment).
const MOVED_LINES_PER_UNIT: u64 = 16;
/// T6: lines re-run on the interned ids cost one unit per this many (no keying: the engine alone).
const RERUN_ID_LINES_PER_UNIT: u64 = 8;
/// The gap plan: a line re-run costs about this many moved lines (or chain pairs) of a child processed instead.
const RERUN_PER_MOVED_LINE: u64 = 6;
/// T6: ids walked by a count cost one unit per this many (one comparison each).
const WALK_STEPS_PER_UNIT: u64 = 128;

/// Why the incremental path gave up (the full compare runs).
enum Fail {
    Full,
    Cancelled,
}

impl From<Cancelled> for Fail {
    fn from(_: Cancelled) -> Fail {
        Fail::Cancelled
    }
}

/// The incremental path: `Ok(None)` when the full compare must run.
pub(crate) fn run(
    job: &RecompareJob,
    pool: Option<&rayon::ThreadPool>,
    ctl: &Ctl<'_>,
    path: &mut PathCounters,
) -> Result<Option<(TextComparison, LiveKeys)>, TextError> {
    let _ = pool;
    match attempt(job, ctl.cancel, path) {
        Ok(done) => Ok(done),
        Err(Fail::Full) => Ok(None),
        Err(Fail::Cancelled) => Err(TextError::Cancelled),
    }
}

/// The shift of every boundary of a side: old → new position (exact outside spans).
struct Sides<'a> {
    views: [SideView<'a>; 2],
    /// Alignment-dirty spans (keys or weights changed), trimmed of equal ends; old and new ranges.
    adirty: [Vec<(Range<u32>, Range<u32>)>; 2],
}

fn attempt(
    job: &RecompareJob,
    cancel: &CancelToken,
    path: &mut PathCounters,
) -> Result<Option<(TextComparison, LiveKeys)>, Fail> {
    let base: &TextComparison = &job.base;
    if !job.anchors.is_empty() || !base.anchors().is_empty() {
        return Ok(None);
    }
    let trace = &job.keys.trace;
    let [lseg] = trace.segments.as_slice() else {
        return Ok(None);
    };
    let seg = &lseg.seg;
    let Some(root) = seg.root else {
        return Ok(None);
    };
    if seg.pair || !matches!(seg.engine, Algorithm::Standard | Algorithm::Patience) {
        return Ok(None);
    }
    // Condition 4, recorded part: no budget refused anything in the last exact run.
    if seg.work.exhausted || lseg.nodes.iter().any(|n| n.node.refusals > 0) {
        path.budgets_failed += 1;
        return Ok(None);
    }
    let maps = [
        LineMap::from_deltas(
            job.log
                .iter()
                .filter(|n| n.side != hikasa_core::side::Side::Right)
                .map(|n| n.delta),
        ),
        LineMap::from_deltas(
            job.log
                .iter()
                .filter(|n| n.side == hikasa_core::side::Side::Right)
                .map(|n| n.delta),
        ),
    ];
    let (old_docs, new_docs) = ([&base.left, &base.right], [&job.left, &job.right]);
    let mut maps = maps;
    // FF-024 `or_line_one`: the key of line 0 depends on its being line 0, so an edit that reaches line 0 makes
    // the old and the new line 0 dirty.
    for k in 0..2 {
        if !base.shape_of(k).identity && base.format_k(k).grammar().has_line_one() {
            maps[k].touch_line_zero(old_docs[k].line_count(), new_docs[k].line_count());
        }
    }
    let total = u64::from(new_docs[0].line_count()) + u64::from(new_docs[1].line_count());
    let dirty = maps[0].dirty_lines() + maps[1].dirty_lines();
    if dirty > FALLBACK_LINES || dirty as f64 > FALLBACK_SHARE * total.max(1) as f64 {
        path.fallback += 1;
        return Ok(None);
    }
    // Step 1: re-keying, each side with its own format, rules and key path (§1.10.16).
    let shape = base.shape_of(0);
    let keyer_of = |k: usize| {
        let shape = base.shape_of(k);
        Keyer {
            set: base.set_of(k),
            shape,
            format: base.format_k(k),
            weights: seg.engine == Algorithm::Standard,
            multiline: !shape.identity && base.format_k(k).grammar().has_multiline(),
            cancel,
        }
    };
    let keyers = [keyer_of(0), keyer_of(1)];
    let mut rekeyed: [Vec<Rekeyed>; 2] = [
        keyers[0].rekey(&maps[0], old_docs[0], new_docs[0], &job.keys.checkpoints[0])?,
        keyers[1].rekey(&maps[1], old_docs[1], new_docs[1], &job.keys.checkpoints[1])?,
    ];
    let (idview, new_hash) = intern(job, &keyers, &mut rekeyed, &job.keys.table)?;
    let rekeyed = rekeyed;
    path.rekeyed += rekeyed
        .iter()
        .flatten()
        .map(|r| u64::from(r.new.end - r.new.start))
        .sum::<u64>();
    let weights = keyers[0].weights
        && (base.format_k(0).has_line_weights() || base.format_k(1).has_line_weights());
    let adirty = [trimmed(&rekeyed[0], weights), trimmed(&rekeyed[1], weights)];
    let news = |k: usize| adirty[k].iter().map(|(_, n)| n.clone()).collect::<Vec<_>>();
    let sides = Sides {
        views: [
            SideView::new(
                &rekeyed[0],
                old_docs[0],
                &job.keys.ids[0],
                &idview,
                &job.keys.checkpoints[0],
                &keyers[0],
                news(0),
            ),
            SideView::new(
                &rekeyed[1],
                old_docs[1],
                &job.keys.ids[1],
                &idview,
                &job.keys.checkpoints[1],
                &keyers[1],
                news(1),
            ),
        ],
        adirty,
    };
    let (n, m) = (new_docs[0].line_count(), new_docs[1].line_count());
    let rules = &job.rules.rules;
    let a = &rules.alignment;
    let opts = AlignOptions {
        algorithm: seg.engine,
        skew_tolerance: a.skew_tolerance,
        closeness: a.closeness.then_some(Closeness {
            threshold_permille: a.closeness_threshold_permille,
            ..Closeness::DEFAULT
        }),
        never_align_differences: a.never_align_differences,
        work_budget: None,
    };
    let auto = rules.alignment.algorithm == AlgorithmChoice::Auto
        && base.nominal_algorithm() == Algorithm::Histogram;
    let mut inc = Inc {
        base,
        old_nodes: &lseg.nodes,
        sides: &sides,
        opts,
        shape,
        cancel,
        out: Vec::with_capacity(base.runs.len() + 16),
        nodes: Vec::new(),
        bound: seg.work,
        budgets: (
            opts.work_budget_for(n as usize, m as usize),
            opts.closeness
                .map_or(0, |c| c.total_cells(n as usize, m as usize)),
            scorer_budget(u64::from(n), u64::from(m)),
        ),
        refusals: 0,
        path,
        added_outside: Vec::new(),
        mapped_chain: Vec::new(),
        weights,
        threshold: a.closeness_threshold_permille,
        total_lines: u64::from(n) + u64::from(m),
        work: std::cell::Cell::new(0),
    };
    let la = 0..n;
    let ra = 0..m;
    // Auto at the segment level: the engine of the new stripped middle (its prefix and suffix found from the
    // old runs outward, O(change)) must be the recorded one, else the full compare.
    if auto {
        let p = inc.new_prefix(&la, &ra)?;
        let s = inc.new_suffix(&la, &ra, p)?;
        let cells = u64::from(n - p - s) * u64::from(m - p - s);
        let engine = if cells <= AlignOptions::fallback_budget_for(n as usize, m as usize) {
            Algorithm::Histogram
        } else {
            Algorithm::Standard
        };
        if engine != seg.engine {
            inc.path.range_failed += 1;
            return Ok(None);
        }
    }
    let new_root = match inc.node(root, la, ra, hikasa_diff::NO_NODE) {
        Ok(id) => id,
        Err(Fail::Full) => {
            if inc.work.get() > WORK_SHARE_LIMIT.max(inc.total_lines / 8) {
                inc.path.fallback += 1;
            }
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    // Condition 4: the totals within the budgets of the new line counts.
    let (std_budget, cells_budget, scorer_cap) = inc.budgets;
    if inc.bound.standard > std_budget
        || inc.bound.closeness > cells_budget
        || inc.bound.scorer > scorer_cap
    {
        inc.path.budgets_failed += 1;
        return Ok(None);
    }
    inc.path.budgets_held += 1;
    // Step 3.
    let runs = canonical_runs(std::mem::take(&mut inc.out));
    let refusals = inc.refusals;
    let bound = inc.bound;
    let nodes = std::mem::take(&mut inc.nodes);
    let new_root_node = &nodes[new_root as usize].node;
    let new_seg = SegmentTrace {
        left: 0..n,
        right: 0..m,
        prefix: new_root_node.prefix,
        suffix: new_root_node.suffix,
        engine: seg.engine,
        pair: false,
        root: Some(new_root),
        nodes: Vec::new(),
        work: WorkTotals {
            standard: bound.standard,
            standard_budget: std_budget,
            closeness: bound.closeness,
            closeness_budget: cells_budget,
            scorer: bound.scorer,
            ..WorkTotals::default()
        },
    };
    let new_seg = LiveSegment {
        seg: new_seg,
        nodes,
    };
    let cmp = build_comparison(job, &sides, &rekeyed, runs, refusals, cancel)?;
    let keys = build_keys(job, &sides, &rekeyed, new_seg, &idview, new_hash)?;
    Ok(Some((cmp, keys)))
}

/// Interns the keys of the re-keyed spans (G8 c): the old lines keep the last exact result's ids (their counts go
/// down), each new key takes the id of an equal key — one this recompare already interned, one a removed line had,
/// or a live id with the same hash whose key a clean line holding it shows equal (found by one walk of the ids for
/// all such checks) — else a new id. Returns the table as this recompare sees it and the new ids' hashes.
fn intern<'t>(
    job: &RecompareJob,
    keyers: &[Keyer<'_>; 2],
    rekeyed: &mut [Vec<Rekeyed>; 2],
    table: &'t super::ids::IdTable,
) -> Result<(IdView<'t>, Vec<u64>), Fail> {
    let cancel = keyers[0].cancel;
    let mut view = IdView {
        table,
        new_trivial: Vec::new(),
        delta: IdMap::default(),
        delta_bits: Vec::new(),
    };
    let mut new_hash: Vec<u64> = Vec::new();
    // The removed lines: their ids, counts down, and their keys known.
    let mut known: HashMap<u32, Vec<u8>> = HashMap::new();
    for (side, spans) in rekeyed.iter_mut().enumerate() {
        for r in spans.iter_mut() {
            let mut ids = Vec::with_capacity((r.old.end - r.old.start) as usize);
            job.keys.ids[side].walk(r.old.clone(), |_, id| {
                ids.push(id);
                true
            });
            for (i, &id) in ids.iter().enumerate() {
                view.delta.entry(id).or_insert([0, 0])[side] -= 1;
                known
                    .entry(id)
                    .or_insert_with(|| r.old_keys.keys[i].clone());
            }
            r.old_keys.ids = ids;
        }
    }
    view.mark_delta();
    // The new keys: settled at once, or waiting for a check of a live id with the same hash.
    enum Slot {
        Id(u32),
        Wait(usize),
    }
    let mut local: HashMap<Vec<u8>, Slot> = HashMap::new();
    let mut waits: Vec<(Vec<u8>, u64, Vec<u32>)> = Vec::new();
    let mut slots: [Vec<Vec<Slot>>; 2] = [Vec::new(), Vec::new()];
    for side in 0..2 {
        for r in &rekeyed[side] {
            let mut row = Vec::with_capacity(r.new_keys.keys.len());
            for key in &r.new_keys.keys {
                let slot = match local.get(key) {
                    Some(Slot::Id(id)) => Slot::Id(*id),
                    Some(Slot::Wait(w)) => Slot::Wait(*w),
                    None => {
                        let h = crate::compare::fingerprint(key, cancel)?;
                        let cands = table.lookup(h);
                        let slot = if let Some(&c) = cands
                            .iter()
                            .find(|c| known.get(c).is_some_and(|k| k == key))
                        {
                            Slot::Id(c)
                        } else {
                            let live: Vec<u32> = cands
                                .into_iter()
                                // A removed line's id has its key known (and it differs); another one
                                // is checked when a line still holds it.
                                .filter(|&c| {
                                    !known.contains_key(&c)
                                        && view.counts(c).iter().any(|n| n.is_none_or(|n| n > 0))
                                })
                                .collect();
                            if live.is_empty() {
                                let id = table.len() + view.new_trivial.len() as u32;
                                view.new_trivial.push(keyers[0].shape.is_trivial(key));
                                new_hash.push(h);
                                Slot::Id(id)
                            } else {
                                waits.push((key.clone(), h, live));
                                Slot::Wait(waits.len() - 1)
                            }
                        };
                        local.insert(
                            key.clone(),
                            match &slot {
                                Slot::Id(id) => Slot::Id(*id),
                                Slot::Wait(w) => Slot::Wait(*w),
                            },
                        );
                        slot
                    }
                };
                row.push(slot);
            }
            slots[side].push(row);
        }
    }
    // One walk of the clean old lines finds a line of each live id to check, then each is re-keyed.
    let mut resolved: Vec<Option<u32>> = vec![None; waits.len()];
    if !waits.is_empty() {
        let wanted: std::collections::HashSet<u32> =
            waits.iter().flat_map(|w| w.2.iter().copied()).collect();
        let mut found: HashMap<u32, (usize, u32)> = HashMap::new();
        for (side, spans) in rekeyed.iter().enumerate() {
            let mut k = 0usize;
            let n = job.keys.ids[side].len();
            if found.len() == wanted.len() {
                break;
            }
            let mut targets =
                Targets::new(wanted.iter().copied().filter(|id| !found.contains_key(id)));
            let mut steps = 0u64;
            let mut hit = |line: u32, id: u32| -> Scan {
                while spans.get(k).is_some_and(|r| r.old.end <= line) {
                    k += 1;
                }
                if spans.get(k).is_some_and(|r| r.old.start <= line) {
                    return Scan::Go;
                }
                found.entry(id).or_insert((side, line));
                if found.len() < wanted.len() {
                    Scan::Complete
                } else {
                    Scan::Stop
                }
            };
            job.keys.ids[side].walk_slices(0..n, |first, slice| {
                let before = steps;
                steps += slice.len() as u64;
                if before / 65_536 != steps / 65_536 && cancel.is_cancelled() {
                    return false;
                }
                targets.scan(first, slice, &mut hit)
            });
            cancel.check().map_err(|_| Fail::Cancelled)?;
        }
        for (w, (key, h, cands)) in waits.iter().enumerate() {
            for c in cands {
                let Some(&(side, line)) = found.get(c) else {
                    continue;
                };
                let doc = if side == 0 {
                    &job.base.left
                } else {
                    &job.base.right
                };
                let keyer = &keyers[side];
                let state = keyer.old_state(doc, &job.keys.checkpoints[side], line)?;
                let mut keyed = Keyed::default();
                keyer.key_lines_pub(doc, line..line + 1, state, &mut keyed)?;
                if keyed.keys.first() == Some(key) {
                    resolved[w] = Some(*c);
                    break;
                }
            }
            if resolved[w].is_none() {
                let id = table.len() + view.new_trivial.len() as u32;
                view.new_trivial.push(keyers[0].shape.is_trivial(key));
                new_hash.push(*h);
                resolved[w] = Some(id);
            }
        }
    }
    // The new lines' ids, their counts up.
    for side in 0..2 {
        for (r, row) in rekeyed[side].iter_mut().zip(&slots[side]) {
            r.new_keys.ids = row
                .iter()
                .map(|slot| match slot {
                    Slot::Id(id) => *id,
                    Slot::Wait(w) => resolved[*w].unwrap_or(0),
                })
                .collect();
            for &id in &r.new_keys.ids {
                view.delta.entry(id).or_insert([0, 0])[side] += 1;
            }
        }
    }
    view.mark_delta();
    Ok((view, new_hash))
}

/// The alignment-dirty spans: every re-keyed span except those whose keys (and weights, when weights count) all
/// equal the old ones line by line. A span is taken whole: inside a span whose line count changed, which old
/// line an equal new line "is" is a choice the alignment makes, so no boundary inside it is fixed.
fn trimmed(spans: &[Rekeyed], weights: bool) -> Vec<(Range<u32>, Range<u32>)> {
    spans
        .iter()
        .filter(|s| {
            let (o, n) = (&s.old_keys, &s.new_keys);
            o.keys.len() != n.keys.len()
                || o.keys.iter().zip(&n.keys).any(|(a, b)| a != b)
                || (weights && o.weights != n.weights)
        })
        .map(|s| (s.old.clone(), s.new.clone()))
        .collect()
}

/// What a node's certificate gives: the new chain, the new prefix and suffix, and its unique pairs as diagonal
/// runs (new numbering).
struct Cert {
    chain: Vec<(u32, u32)>,
    prefix: u32,
    suffix: u32,
    diag: Vec<(u32, u32, u32)>,
}

/// The lines of one side whose key may count differently in a node's middle: alignment-dirty old lines (old
/// numbering) and new lines (new numbering) with their ids (trivial keys left out), and the clean lines that
/// moved out of the middle or into it (new numbering, two ranges each).
#[derive(Debug, Default)]
struct MiddleChanges {
    old: Vec<(u32, u32)>,
    new: Vec<(u32, u32)>,
    moved_out: [Range<u32>; 2],
    moved_in: [Range<u32>; 2],
}

/// A dirty key: its dirty counts per side (old lines, new lines) and the old lines that had it.
#[derive(Debug, Clone, Default)]
struct KeyInfo {
    dirty: [(i64, i64); 2],
    old_lines: [Vec<u32>; 2],
    /// The new lines of the middle known to hold it (dirty or moved in).
    new_lines: [Vec<u32>; 2],
}

/// The window of Standard's stage A (range-local coordinates): the start or the end diagonal within the skew.
fn window_holds(x: u32, y: u32, na: u32, nb: u32, skew: u32) -> bool {
    let start = (i64::from(x) - i64::from(y)).unsigned_abs();
    let end = (i64::from(na) - i64::from(x) - (i64::from(nb) - i64::from(y))).unsigned_abs();
    start <= u64::from(skew) || end <= u64::from(skew)
}

/// The incremental alignment of one segment.
struct Inc<'a, 'p> {
    base: &'a TextComparison,
    /// The last exact result's nodes (shared, moved lazily).
    old_nodes: &'a [LiveNode],
    sides: &'a Sides<'a>,
    opts: AlignOptions,
    shape: KeyShape,
    cancel: &'a CancelToken,
    /// The new runs in order (canonicalized at the end).
    out: Vec<Run>,
    /// The new derivation's nodes.
    nodes: Vec<LiveNode>,
    /// Upper bounds of the totals (old totals plus every re-run's and re-extension's work).
    bound: WorkTotals,
    /// (work, closeness cells, scorer units) budgets of the new line counts.
    budgets: (u64, u64, u64),
    refusals: u64,
    path: &'p mut PathCounters,
    /// The unique pairs the last certified node gained, in or outside the window (new coordinates).
    added_outside: Vec<(u32, u32)>,
    /// The last certified node's old chain in the new numbering (`None`: an anchor on a dirty line).
    mapped_chain: Vec<Option<(u32, u32)>>,
    weights: bool,
    threshold: u16,
    /// Lines of both new documents (the work guard).
    total_lines: u64,
    /// Work done so far (lines re-run, lines moved between a middle and its ends, ids walked): past an eighth
    /// of the lines the full compare is cheaper and runs instead (T6, the same result).
    work: std::cell::Cell<u64>,
}

impl Inc<'_, '_> {
    /// Counts `n` units of work; `Fail::Full` once the work passes an eighth of the lines (T6).
    fn spend(&self, n: u64) -> Result<(), Fail> {
        let w = self.work.get() + n;
        self.work.set(w);
        if w > WORK_SHARE_LIMIT.max(self.total_lines / 8) {
            return Err(Fail::Full);
        }
        Ok(())
    }

    /// True when an alignment-dirty span of `side` touches the old range `old` (a span without old lines touches
    /// it when it sits inside or at its ends).
    fn dirty_in(&self, side: usize, old: &Range<u32>) -> bool {
        self.sides.adirty[side].iter().any(|(o, _)| {
            if o.is_empty() {
                old.start <= o.start && o.start <= old.end
            } else {
                o.start < old.end && old.start < o.end
            }
        })
    }

    /// Equality of the keys of new left line `l` and new right line `r`: equal interned ids.
    fn equal(&self, l: u32, r: u32) -> Result<bool, Fail> {
        let [lv, rv] = &self.sides.views;
        Ok(lv.id(l) == rv.id(r))
    }

    /// The pairs from new lines `(l, r)` on known equal without comparing keys: both lines clean, their old lines
    /// on the diagonal of one old `Equal` run; up to the run's end or the next re-keyed line (0 when not known).
    fn known_equal_forward(&self, l: u32, r: u32) -> u32 {
        let [lv, rv] = &self.sides.views;
        let (Some(ox), Some(oy)) = (lv.old_of(l), rv.old_of(r)) else {
            return 0;
        };
        let runs = &self.base.runs;
        let k = runs.partition_point(|run| run.left.end <= ox);
        let Some(run) = runs.get(k) else { return 0 };
        if run.kind != RunKind::Equal
            || run.left.start > ox
            || i64::from(oy) - i64::from(ox)
                != i64::from(run.right.start) - i64::from(run.left.start)
        {
            return 0;
        }
        (run.left.end - ox)
            .min(lv.clean_run(l).max(1))
            .min(rv.clean_run(r).max(1))
    }

    /// The pairs ending just before new lines `(l, r)` known equal (as [`Inc::known_equal_forward`], backward).
    fn known_equal_backward(&self, l: u32, r: u32) -> u32 {
        if l == 0 || r == 0 {
            return 0;
        }
        let [lv, rv] = &self.sides.views;
        let (Some(ox), Some(oy)) = (lv.old_of(l - 1), rv.old_of(r - 1)) else {
            return 0;
        };
        let runs = &self.base.runs;
        let k = runs.partition_point(|run| run.left.end <= ox);
        let Some(run) = runs.get(k) else { return 0 };
        if run.kind != RunKind::Equal
            || run.left.start > ox
            || i64::from(oy) - i64::from(ox)
                != i64::from(run.right.start) - i64::from(run.left.start)
        {
            return 0;
        }
        (ox + 1 - run.left.start)
            .min(lv.clean_run_back(l).max(1))
            .min(rv.clean_run_back(r).max(1))
    }

    /// The common prefix of new ranges `a × b`, found as the engine does, but in steps of known-equal pairs (old
    /// `Equal` runs between re-keyed lines) and key comparisons elsewhere: O(change), not O(prefix).
    fn new_prefix(&self, a: &Range<u32>, b: &Range<u32>) -> Result<u32, Fail> {
        let limit = (a.end - a.start).min(b.end - b.start);
        let mut p = 0u32;
        while p < limit {
            let (l, r) = (a.start + p, b.start + p);
            let known = self.known_equal_forward(l, r);
            if known > 0 {
                p = (p + known).min(limit);
                continue;
            }
            if !self.equal(l, r)? {
                break;
            }
            p += 1;
        }
        Ok(p)
    }

    /// The common suffix of new ranges `a × b` without their first `prefix` pairs (as [`Inc::new_prefix`]).
    fn new_suffix(&self, a: &Range<u32>, b: &Range<u32>, prefix: u32) -> Result<u32, Fail> {
        let limit = (a.end - a.start).min(b.end - b.start) - prefix;
        let mut s = 0u32;
        while s < limit {
            let (l, r) = (a.end - s, b.end - s);
            let known = self.known_equal_backward(l, r);
            if known > 0 {
                s = (s + known).min(limit);
                continue;
            }
            if !self.equal(l - 1, r - 1)? {
                break;
            }
            s += 1;
        }
        Ok(s)
    }

    /// Emits the old runs between old path points `from` and `to`, moved to the new numbering (a clean region).
    fn emit_old(&mut self, from: (u32, u32), to: (u32, u32)) {
        let base = self.base;
        let (r0, r1) = (row_at(base, from), row_at(base, to));
        if r0 >= r1 {
            return;
        }
        let k0 = base.run_rows.partition_point(|&r| r <= r0) - 1;
        let [lv, rv] = &self.sides.views;
        let mut k = k0;
        while k < base.runs.len() && base.run_rows[k] < r1 {
            let run = &base.runs[k];
            let start = base.run_rows[k];
            let a = r0.max(start);
            let b = r1.min(start + run.rows());
            let part = cut(run, start, a..b);
            self.out.push(Run {
                kind: part.kind,
                left: lv.pos(part.left.start)
                    ..lv.pos(part.left.start) + (part.left.end - part.left.start),
                right: rv.pos(part.right.start)
                    ..rv.pos(part.right.start) + (part.right.end - part.right.start),
            });
            k += 1;
        }
    }

    /// Copies the recorded subtree `id` (a clean region) into the new derivation; returns its new id.
    fn copy_subtree(&mut self, id: NodeId, parent: NodeId) -> NodeId {
        // The node and its subtree are shared: only their offsets change (the region is clean, so every line of
        // it moves by the same shift).
        let old = &self.old_nodes[id as usize];
        let (l, r) = old.ranges();
        let [lv, rv] = &self.sides.views;
        let moved = LiveNode {
            node: old.node.clone(),
            dx: old.dx + i64::from(lv.pos(l.start)) - i64::from(l.start),
            dy: old.dy + i64::from(rv.pos(r.start)) - i64::from(r.start),
            children: Vec::new(),
        };
        let kids = old.children.clone();
        let new_id = self.push_live(moved, parent);
        for c in kids {
            self.copy_subtree(c, new_id);
        }
        new_id
    }

    fn push_node(&mut self, node: TraceNode, parent: NodeId) -> NodeId {
        let mut node = node;
        node.children = Vec::new();
        self.push_live(LiveNode::new(node), parent)
    }

    fn push_live(&mut self, node: LiveNode, parent: NodeId) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(node);
        if let Some(p) = self.nodes.get_mut(parent as usize) {
            p.children.push(id);
        }
        id
    }

    /// Processes recorded node `id` whose new ranges are `la × ra`: copies a clean node whose ranges map to the
    /// new ones, certifies and descends, or re-runs it. Returns the new node's id.
    fn node(
        &mut self,
        id: NodeId,
        la: Range<u32>,
        ra: Range<u32>,
        parent: NodeId,
    ) -> Result<NodeId, Fail> {
        self.cancel.check().map_err(|_| Fail::Cancelled)?;
        // The node in the last exact result's numbering, materialized once (its chain, blocks and runs moved).
        let nodes: &[LiveNode] = self.old_nodes;
        let old = nodes[id as usize].absolute();
        let [lv, rv] = &self.sides.views;
        let same_ranges = la == (lv.pos(old.left.start)..lv.pos(old.left.end))
            && ra == (rv.pos(old.right.start)..rv.pos(old.right.end));
        if same_ranges && !self.dirty_in(0, &old.left) && !self.dirty_in(1, &old.right) {
            self.emit_old(
                (old.left.start, old.right.start),
                (old.left.end, old.right.end),
            );
            return Ok(self.copy_subtree(id, parent));
        }
        let c = self.certify(&old, &la, &ra)?;
        match c {
            Some(cert) => self.descend(id, &old, la, ra, parent, cert),
            None => self.rerun(la, ra, parent),
        }
    }

    /// The certificate of node `id` on its new ranges `la × ra` (conditions 1–3). Condition 1 (edge deltas):
    /// the node's new common prefix and suffix are found from the old `Equal` runs outward (O(change)); the
    /// middle must stay non-empty on both sides; lines that move between the stripped ends and the middle count
    /// for condition 2 as removed (moved out) or inserted (moved in), like dirty lines.
    fn certify(
        &mut self,
        old: &TraceNode,
        la: &Range<u32>,
        ra: &Range<u32>,
    ) -> Result<Option<Cert>, Fail> {
        let (nl, nr) = (la.end - la.start, ra.end - ra.start);
        let p = self.new_prefix(la, ra)?;
        let s = self.new_suffix(la, ra, p)?;
        if p + s >= nl.min(nr) {
            self.path.range_failed += 1;
            return Ok(None);
        }
        self.path.range_held += 1;
        let nm = [la.start + p..la.end - s, ra.start + p..ra.end - s];
        let (oml, omr) = old.stripped();
        let om = [oml, omr];
        // 2. Candidates.
        let chain = if old.stage == Stage::AnchorsUnique {
            self.new_chain(old, &om, &nm)?
        } else {
            None
        };
        let Some(chain) = chain else {
            self.path.candidates_failed += 1;
            return Ok(None);
        };
        self.path.candidates_held += 1;
        // 3. Window (Standard): exact per diagonal run of the node's unique pairs (`TraceNode::diag`).
        let w = self.window(old, &om, &nm, chain)?;
        let Some((chain, diag)) = w else {
            self.path.window_failed += 1;
            return Ok(None);
        };
        self.path.window_held += 1;
        Ok(Some(Cert {
            chain,
            prefix: p,
            suffix: s,
            diag,
        }))
    }

    /// Condition 3 (the window, Standard) and the node's new diagonal runs. A unique pair's window membership is
    /// a function of its diagonal relative to the middle's start and of the middles' lengths, so each old run is
    /// cut into the pieces that stay clean and inside the new middle (dirty lines and moved ends cut it), and each
    /// piece's membership is decided before and after, exactly. When the chain holds every candidate, the pairs of
    /// a piece that leaves the window leave the chain and the unique pairs of a piece that enters it join (the set
    /// must still increase on both sides); otherwise any change fails. The new runs are the pieces plus a run of
    /// its own for each pair that became unique. `None`: the certificate fails.
    #[allow(clippy::type_complexity)]
    fn window(
        &mut self,
        old: &TraceNode,
        om: &[Range<u32>; 2],
        nm: &[Range<u32>; 2],
        mut chain: Vec<(u32, u32)>,
    ) -> Result<Option<(Vec<(u32, u32)>, Vec<(u32, u32, u32)>)>, Fail> {
        let skew = self.opts.skew_tolerance;
        let windowed = self.opts.algorithm == Algorithm::Standard && skew != u32::MAX;
        let all = old.chain.len() as u32 == old.candidates && old.candidates > 0;
        let [lv, rv] = &self.sides.views;
        let span = |r: &Range<u32>| r.end - r.start;
        let member = |d: i64, na: u32, nb: u32| -> bool {
            !windowed
                || d.unsigned_abs() <= u64::from(skew)
                || (i64::from(na) - i64::from(nb) + d).unsigned_abs() <= u64::from(skew)
        };
        let local_diag = |x: u32, y: u32, m: &[Range<u32>; 2]| -> i64 {
            (i64::from(y) - i64::from(m[1].start)) - (i64::from(x) - i64::from(m[0].start))
        };
        // The pieces: (x, y, len) in the new numbering, membership before and after.
        let mut pieces: Vec<((u32, u32, u32), bool, bool)> = Vec::with_capacity(old.diag.len() + 8);
        for &(x, y, len) in &old.diag {
            let m_old = member(local_diag(x, y, om), span(&om[0]), span(&om[1]));
            let mut k = 0u32;
            while k < len {
                let (xo, yo) = (x + k, y + k);
                let (Some(xn), Some(yn)) = (lv.new_of(xo), rv.new_of(yo)) else {
                    k += 1;
                    continue;
                };
                let run = (len - k)
                    .min(lv.clean_run(xn).max(1))
                    .min(rv.clean_run(yn).max(1));
                let lo = nm[0]
                    .start
                    .saturating_sub(xn)
                    .max(nm[1].start.saturating_sub(yn));
                let hi = run
                    .min(nm[0].end.saturating_sub(xn))
                    .min(nm[1].end.saturating_sub(yn));
                if lo < hi {
                    let m_new = member(local_diag(xn, yn, nm), span(&nm[0]), span(&nm[1]));
                    pieces.push(((xn + lo, yn + lo, hi - lo), m_old, m_new));
                }
                k += run;
            }
        }
        if pieces.iter().any(|&(_, a, b)| a != b) {
            if !all {
                return Ok(None);
            }
            let mut leaving: Vec<u32> = Vec::new();
            let mut lines: Vec<(u32, u32, u32)> = Vec::new();
            let mut direct: Vec<(u32, u32)> = Vec::new();
            for &((px, py, len), m_old, m_new) in &pieces {
                if m_old && !m_new {
                    let from = chain.partition_point(|c| c.0 < px);
                    for &(x, y) in chain[from..].iter().take_while(|c| c.0 < px + len) {
                        if i64::from(y) - i64::from(x) == i64::from(py) - i64::from(px) {
                            leaving.push(x);
                        }
                    }
                } else if !m_old && m_new {
                    let (li, ri) = (lv.ids_of(px..px + len), rv.ids_of(py..py + len));
                    for (i, (&id, &other)) in li.iter().zip(&ri).enumerate() {
                        let (x, y) = (px + i as u32, py + i as u32);
                        if id == other && !lv.table.trivial(id) {
                            // A key with one line on each side, both in the middle: unique there, no count.
                            if lv.table.counts(id) == [Some(1), Some(1)] {
                                direct.push((x, y));
                            } else {
                                lines.push((x, y, id));
                            }
                        }
                    }
                }
            }
            // The entering lines are pairs when their key is unique on both sides in the new middle.
            // Two entering pairs of one key put two of its lines in the middle on each side: neither is unique.
            // A key with one entering pair is counted.
            lines.sort_unstable_by_key(|l| l.2);
            let lines: Vec<(u32, u32, u32)> = lines
                .chunk_by(|a, b| a.2 == b.2)
                .filter(|g| g.len() == 1)
                .map(|g| g[0])
                .collect();
            if !lines.is_empty() {
                let ids: Vec<u32> = lines.iter().map(|l| l.2).collect();
                let caps = vec![[2i64, 2i64]; ids.len()];
                let known: Vec<[Vec<u32>; 2]> =
                    lines.iter().map(|&(x, y, _)| [vec![x], vec![y]]).collect();
                let (counts, _) = self.count_ids(&ids, &caps, nm, &known)?;
                for (k, &(x, y, _)) in lines.iter().enumerate() {
                    if counts[k] == [1, 1] {
                        direct.push((x, y));
                    }
                }
            }
            if !leaving.is_empty() {
                // The pieces ascend: so do their pairs that leave.
                leaving.sort_unstable();
                let mut k = 0usize;
                chain.retain(|&(x, _)| {
                    while k < leaving.len() && leaving[k] < x {
                        k += 1;
                    }
                    leaving.get(k) != Some(&x)
                });
            }
            direct.sort_unstable();
            chain = merge_pairs(chain, &direct);
            if chain
                .windows(2)
                .any(|w| w[0].0 >= w[1].0 || w[0].1 >= w[1].1)
            {
                return Ok(None);
            }
        }
        // The new runs: the pieces, and a run of its own for each pair that became unique (cutting a piece it
        // falls in); consecutive runs on one diagonal merge.
        let mut runs: Vec<(u32, u32, u32)> = pieces.into_iter().map(|(r, _, _)| r).collect();
        let mut added = self.added_outside.clone();
        added.sort_unstable();
        for &(x, y) in &added {
            let k = runs.partition_point(|r| r.0 + r.2 <= x);
            if let Some(&(rx, ry, rl)) = runs.get(k)
                && rx <= x
            {
                let mut cut = Vec::with_capacity(2);
                if x > rx {
                    cut.push((rx, ry, x - rx));
                }
                if x + 1 < rx + rl {
                    cut.push((x + 1, ry + (x + 1 - rx), rx + rl - x - 1));
                }
                runs.splice(k..k + 1, cut);
            }
            let k = runs.partition_point(|r| r.0 < x);
            runs.insert(k, (x, y, 1));
        }
        let mut merged: Vec<(u32, u32, u32)> = Vec::with_capacity(runs.len());
        for r in runs {
            match merged.last_mut() {
                Some(last)
                    if i64::from(last.1) - i64::from(last.0) == i64::from(r.1) - i64::from(r.0) =>
                {
                    last.2 = r.0 + r.2 - last.0;
                }
                _ => merged.push(r),
            }
        }
        Ok(Some((chain, merged)))
    }

    /// The changed lines of a node for condition 2, per side: the alignment-dirty old lines of its old middle
    /// `om` and new lines of its new middle `nm`, with their keys (trivial keys left out), and the clean lines
    /// that moved out of the middle or into it (new numbering; a clean line's old line is in the old middle
    /// exactly when its new line is in the old middle mapped). Every other line of the old middle is a clean line
    /// of the new middle and back.
    fn middle_changes(
        &self,
        side: usize,
        om: &Range<u32>,
        nm: &Range<u32>,
    ) -> Result<Option<MiddleChanges>, Fail> {
        let view = &self.sides.views[side];
        let mut out = MiddleChanges::default();
        for (o, n) in &self.sides.adirty[side] {
            let touches_old = if o.is_empty() {
                om.start <= o.start && o.start <= om.end
            } else {
                o.start < om.end && om.start < o.end
            };
            let touches_new = n.start < nm.end && nm.start < n.end;
            if !touches_old && !touches_new {
                continue;
            }
            let span = view.spans.iter().find(|r| {
                r.old.start <= o.start
                    && o.end <= r.old.end
                    && r.new.start <= n.start
                    && n.end <= r.new.end
            });
            let Some(span) = span else {
                return Ok(None);
            };
            for x in o.start.max(om.start)..o.end.min(om.end) {
                let id = span.old_keys.ids[(x - span.old.start) as usize];
                if !view.table.trivial(id) {
                    out.old.push((x, id));
                }
            }
            for x in n.start.max(nm.start)..n.end.min(nm.end) {
                let id = span.new_keys.ids[(x - span.new.start) as usize];
                if !view.table.trivial(id) {
                    out.new.push((x, id));
                }
            }
        }
        // Clean lines that moved: the new middle against the mapped old one (both intervals).
        let mapped = view.pos(om.start)..view.pos(om.end);
        out.moved_in = [
            nm.start..nm.end.min(mapped.start),
            nm.start.max(mapped.end)..nm.end,
        ];
        out.moved_out = [
            mapped.start..mapped.end.min(nm.start),
            mapped.start.max(nm.end)..mapped.end,
        ];
        Ok(Some(out))
    }

    /// Counts the keys `ids` in the new middles `middle` (per side, up to `caps`) by a walk of the middles'
    /// ids that stops once every key is complete (its cap reached, or every occurrence of the side found: the
    /// table's counts after this recompare say how many there are). Returns the counts and the lines found.
    #[allow(clippy::type_complexity)]
    fn count_ids(
        &self,
        ids: &[u32],
        caps: &[[i64; 2]],
        middle: &[Range<u32>; 2],
        known: &[[Vec<u32>; 2]],
    ) -> Result<(Vec<[i64; 2]>, Vec<[Vec<u32>; 2]>), Fail> {
        let mut index: IdMap<Vec<usize>> = IdMap::default();
        index.reserve(ids.len());
        for (k, &id) in ids.iter().enumerate() {
            index.entry(id).or_default().push(k);
        }
        let mut counts = vec![[0i64; 2]; ids.len()];
        let mut hits: Vec<[Vec<u32>; 2]> = vec![[Vec::new(), Vec::new()]; ids.len()];
        for side in 0..2 {
            let view = &self.sides.views[side];
            // The lines already known to hold a key (dirty or moved lines of the middle) count at once, and the
            // walk skips them for that entry only: `ids` may hold one id several times (the window's entering
            // pairs, one entry per pair), and a line known for one entry still counts for the others.
            let mut skip: std::collections::HashSet<(u32, usize)> = Default::default();
            for (k, kn) in known.iter().enumerate() {
                for &x in &kn[side] {
                    if counts[k][side] < caps[k][side] {
                        counts[k][side] += 1;
                        hits[k][side].push(x);
                    }
                    skip.insert((x, k));
                }
            }
            // How many each key can still find on this side (the whole document's count bounds the middle's,
            // so a key whose every occurrence is known needs no walk).
            let mut left: Vec<i64> = ids
                .iter()
                .zip(caps)
                .enumerate()
                .map(|(k, (&id, cap))| {
                    let total = view.table.counts(id)[side].map_or(i64::MAX, |c| c as i64);
                    let known = known.get(k).map_or(0, |kn| kn[side].len() as i64);
                    (cap[side].min(total) - known.min(counts[k][side])).max(0)
                })
                .collect();
            let mut open = left.iter().filter(|&&n| n > 0).count();
            if open == 0 {
                continue;
            }
            let mut targets = Targets::new(
                ids.iter()
                    .zip(&left)
                    .filter(|(_, n)| **n > 0)
                    .map(|(id, _)| *id),
            );
            let mut steps = 0u64;
            let mut cancelled = false;
            let mut hit = |x: u32, id: u32| -> Scan {
                let mut more = false;
                if let Some(ks) = index.get(&id) {
                    for &k in ks {
                        if left[k] > 0 && !skip.contains(&(x, k)) {
                            counts[k][side] += 1;
                            hits[k][side].push(x);
                            left[k] -= 1;
                            if left[k] == 0 {
                                open -= 1;
                            }
                        }
                        more |= left[k] > 0;
                    }
                }
                if open == 0 {
                    Scan::Stop
                } else if more {
                    Scan::Go
                } else {
                    Scan::Complete
                }
            };
            view.walk_id_slices(middle[side].clone(), |first, slice, _| {
                let before = steps;
                steps += slice.len() as u64;
                if before / 65_536 != steps / 65_536 && self.cancel.is_cancelled() {
                    cancelled = true;
                    return false;
                }
                targets.scan(first, slice, &mut hit)
            });
            self.spend(steps / WALK_STEPS_PER_UNIT)?;
            if cancelled {
                return Err(Fail::Cancelled);
            }
        }
        Ok((counts, hits))
    }

    /// Condition 2 and the node's new chain. The chain is unchanged when no key of a removed or inserted line
    /// (alignment-dirty, or moved out of or into the middle) is, or becomes, unique on both sides in the middle
    /// (counts exact: a walk of the interned ids). When the recorded chain holds every candidate, candidates may
    /// also go (their lines changed or left the middle, or their keys stopped being unique) and come (a key became
    /// unique in the window), as long as the new set is still increasing on both sides: the chain is then the new
    /// set (every candidate scores more than nothing). `None`: the certificate fails.
    ///
    /// Moved lines (G8 follow-up) need no count when the table's whole-document counts decide them: a line whose
    /// key has no other line on its side cannot make another pair unique or stop one being unique, so a line
    /// moving out only takes its own pair (if any) out of the middle, and the chain and the runs are clipped to
    /// the new middle; a key absent from the other side is unique nowhere; a key with one line on each side whose
    /// two lines both moved in is a new unique pair. Every other moved line is counted like a dirty line.
    fn new_chain(
        &mut self,
        old: &TraceNode,
        om: &[Range<u32>; 2],
        nm: &[Range<u32>; 2],
    ) -> Result<Option<Vec<(u32, u32)>>, Fail> {
        let middle = nm.clone();
        // What the window check and the new record read is this node's alone (an early return below keeps it
        // empty, never the previous node's).
        self.added_outside.clear();
        let all = old.chain.len() as u32 == old.candidates && old.candidates > 0;
        // The keys of the changed lines with their counts per side (old lines, new lines) and their lines: the
        // dirty lines first (a moved line of a dirty key is always counted).
        let mut keys: IdMap<KeyInfo> = IdMap::default();
        let mut moved: [MiddleChanges; 2] = Default::default();
        for side in 0..2 {
            let Some(changes) = self.middle_changes(side, &om[side], &nm[side])? else {
                return Ok(None);
            };
            for &(x, key) in &changes.old {
                let info = keys.entry(key).or_default();
                info.dirty[side].0 += 1;
                info.old_lines[side].push(x);
            }
            for &(x, key) in &changes.new {
                let info = keys.entry(key).or_default();
                info.dirty[side].1 += 1;
                info.new_lines[side].push(x);
            }
            moved[side] = changes;
        }
        let table = self.sides.views[0].table;
        // The dirty keys, one bit per id (a moved line of a dirty key is always counted).
        let mut dirty_ids: Vec<u64> = Vec::new();
        for &id in keys.keys() {
            let w = id as usize / 64;
            if dirty_ids.len() <= w {
                dirty_ids.resize(w + 1, 0);
            }
            dirty_ids[w] |= 1 << (id % 64);
        }
        let dirty_key = |id: u32| {
            dirty_ids
                .get(id as usize / 64)
                .is_some_and(|w| w & (1 << (id % 64)) != 0)
        };
        let mut solo: [Vec<(u32, u32)>; 2] = [Vec::new(), Vec::new()];
        let mut lines_moved = 0u64;
        for (side, changes) in moved.iter().enumerate() {
            let view = &self.sides.views[side];
            for r in changes.moved_out.iter().filter(|r| !r.is_empty()) {
                lines_moved += u64::from(r.end - r.start);
                view.walk_id_slices(r.clone(), |first, slice, old_first| {
                    for (i, &id) in slice.iter().enumerate() {
                        let x = first + i as u32;
                        if all && !dirty_key(id) {
                            let c = table.counts(id);
                            if c[side] == Some(1) || c[1 - side] == Some(0) {
                                continue;
                            }
                        }
                        if view.table.trivial(id) {
                            continue;
                        }
                        // A dirty line (alignment-dirty part of a span) is counted by its span.
                        let o = match old_first {
                            Some(o) => Some(o + i as u32),
                            None => view.old_of(x),
                        };
                        let Some(o) = o else {
                            continue;
                        };
                        let info = keys.entry(id).or_default();
                        info.dirty[side].0 += 1;
                        info.old_lines[side].push(o);
                    }
                    true
                });
            }
            for r in changes.moved_in.iter().filter(|r| !r.is_empty()) {
                lines_moved += u64::from(r.end - r.start);
                view.walk_id_slices(r.clone(), |first, slice, old_first| {
                    for (i, &id) in slice.iter().enumerate() {
                        let x = first + i as u32;
                        if old_first.is_none() && view.old_of(x).is_none() {
                            continue;
                        }
                        if all && !dirty_key(id) {
                            let c = table.counts(id);
                            if c[1 - side] == Some(0) {
                                continue;
                            }
                            if c == [Some(1), Some(1)] {
                                if !view.table.trivial(id) {
                                    solo[side].push((id, x));
                                }
                                continue;
                            }
                        }
                        if view.table.trivial(id) {
                            continue;
                        }
                        let info = keys.entry(id).or_default();
                        info.dirty[side].1 += 1;
                        info.new_lines[side].push(x);
                    }
                    true
                });
            }
        }
        self.spend(lines_moved / MOVED_LINES_PER_UNIT)?;
        // Keys with one line on each side, both moved in: a new unique pair; a solo line whose other line did not
        // move in is counted.
        let mut solo_pairs: Vec<(u32, u32)> = Vec::new();
        if !solo[0].is_empty() || !solo[1].is_empty() {
            let mut right: IdMap<u32> = IdMap::default();
            right.extend(solo[1].iter().copied());
            for &(id, x) in &solo[0] {
                match right.remove(&id) {
                    Some(y) => solo_pairs.push((x, y)),
                    None => {
                        let info = keys.entry(id).or_default();
                        info.dirty[0].1 += 1;
                        info.new_lines[0].push(x);
                    }
                }
            }
            for &(id, y) in &solo[1] {
                if right.contains_key(&id) {
                    let info = keys.entry(id).or_default();
                    info.dirty[1].1 += 1;
                    info.new_lines[1].push(y);
                }
            }
        }
        let [lv, rv] = &self.sides.views;
        // The old chain in the new numbering (both coordinates ascending: one walk of the spans each).
        self.mapped_chain.clear();
        self.mapped_chain.reserve(old.chain.len());
        {
            let (mut lm, mut rm) = (lv.ascending(), rv.ascending());
            self.mapped_chain.extend(
                old.chain
                    .iter()
                    .map(|&(x, y)| Some((lm.new_of(x)?, rm.new_of(y)?))),
            );
        }
        let (na, nb) = (
            middle[0].end - middle[0].start,
            middle[1].end - middle[1].start,
        );
        let in_window = |x: u32, y: u32, opts: &AlignOptions| -> bool {
            opts.algorithm != Algorithm::Standard
                || window_holds(
                    x - middle[0].start,
                    y - middle[1].start,
                    na,
                    nb,
                    opts.skew_tolerance,
                )
        };
        let mut removed: Vec<u32> = Vec::new();
        let mut added: Vec<(u32, u32)> = Vec::new();
        for &(x, y) in &solo_pairs {
            if in_window(x, y, &self.opts) {
                added.push((x, y));
            }
            self.added_outside.push((x, y));
        }
        if !keys.is_empty() {
            // Count every key in the new middle: a walk of the ids.
            let list: Vec<(u32, KeyInfo)> = keys.into_iter().collect();
            // Caps: a key's count matters exactly up to the value that decides `was` (its count before the edit
            // is 1: now 1 + new − old known lines) and `is` (1); known lines of the middle count first, so two more
            // decide both. A key is not counted at all when neither can hold: two known new lines on a side make
            // it non-unique now, two known old lines on a side made it non-unique before.
            let caps: Vec<[i64; 2]> = list
                .iter()
                .map(|(_, c)| [2 + c.dirty[0].1, 2 + c.dirty[1].1])
                .collect();
            let decides: Vec<bool> = list
                .iter()
                .map(|(_, c)| {
                    let is = c.dirty[0].1 <= 1 && c.dirty[1].1 <= 1;
                    let was = c.dirty[0].0 <= 1 && c.dirty[1].0 <= 1;
                    is || was
                })
                .collect();
            // A key missing from one whole side after the edit is unique nowhere: with an all-candidate chain it
            // only leaves the chain when one of its lines did (its candidate pair has a removed or moved-out line,
            // found in the chain by that line), so it is not counted. Every other key is counted by a walk.
            let missing: Vec<bool> = list
                .iter()
                .map(|(id, _)| all && table.counts(*id).contains(&Some(0)))
                .collect();
            let walk: Vec<bool> = missing
                .iter()
                .zip(&decides)
                .map(|(m, d)| !m && *d)
                .collect();
            let ids: Vec<u32> = list
                .iter()
                .zip(&walk)
                .filter(|(_, w)| **w)
                .map(|((id, _), _)| *id)
                .collect();
            let walk_caps: Vec<[i64; 2]> = caps
                .iter()
                .zip(&walk)
                .filter(|(_, w)| **w)
                .map(|(c, _)| *c)
                .collect();
            let walk_known: Vec<[Vec<u32>; 2]> = list
                .iter()
                .zip(&walk)
                .filter(|(_, w)| **w)
                .map(|((_, info), _)| info.new_lines.clone())
                .collect();
            let (walked_counts, walked_hits) =
                self.count_ids(&ids, &walk_caps, &middle, &walk_known)?;
            let mut counts = vec![[0i64; 2]; list.len()];
            let mut hits: Vec<[Vec<u32>; 2]> = vec![[Vec::new(), Vec::new()]; list.len()];
            let mut fast: Vec<usize> = Vec::new();
            let (mut wc, mut wh) = (walked_counts.into_iter(), walked_hits.into_iter());
            for (k, &w) in walk.iter().enumerate() {
                if w {
                    counts[k] = wc.next().unwrap_or([0, 0]);
                    hits[k] = wh.next().unwrap_or_default();
                } else if missing[k] {
                    fast.push(k);
                }
            }
            for &k in &fast {
                let info = &list[k].1;
                for &x in &info.old_lines[0] {
                    if old.chain.binary_search_by_key(&x, |c| c.0).is_ok() {
                        removed.push(x);
                    }
                }
                for &y in &info.old_lines[1] {
                    if let Ok(i) = old.chain.binary_search_by_key(&y, |c| c.1) {
                        removed.push(old.chain[i].0);
                    }
                }
            }
            for (k, (_, info)) in list.iter().enumerate() {
                if !walk[k] {
                    continue;
                }
                let new = counts[k];
                let before = [
                    new[0] - info.dirty[0].1 + info.dirty[0].0,
                    new[1] - info.dirty[1].1 + info.dirty[1].0,
                ];
                let was = before[0] == 1 && before[1] == 1;
                let is = new[0] == 1 && new[1] == 1;
                if !was && !is {
                    continue;
                }
                if !all {
                    return Ok(None);
                }
                if was {
                    // The candidate's old left line: the dirty old line with the key, else the clean hit.
                    let x = match info.old_lines[0].first() {
                        Some(&x) => Some(x),
                        None => hits[k][0]
                            .iter()
                            .find_map(|&h| lv.old_of(h).filter(|o| om[0].contains(o))),
                    };
                    let Some(x) = x else {
                        return Ok(None);
                    };
                    removed.push(x);
                }
                if is {
                    let (x, y) = (hits[k][0][0], hits[k][1][0]);
                    if in_window(x, y, &self.opts) {
                        added.push((x, y));
                    }
                    self.added_outside.push((x, y));
                }
            }
        }
        // The old chain moved, without the removed pairs and those that left the middle (a pair with a line
        // outside the new middle is no candidate of it: with an all-candidate chain it just leaves).
        removed.sort_unstable();
        let mut chain: Vec<(u32, u32)> = Vec::with_capacity(old.chain.len() + added.len());
        let mut r = 0usize;
        for (&(x, _), m) in old.chain.iter().zip(&self.mapped_chain) {
            // Both ascend: one pointer.
            while r < removed.len() && removed[r] < x {
                r += 1;
            }
            if removed.get(r) == Some(&x) {
                continue;
            }
            let Some((nx, ny)) = *m else {
                return Ok(None);
            };
            if !(middle[0].contains(&nx) && middle[1].contains(&ny)) {
                if !all {
                    return Ok(None);
                }
                continue;
            }
            chain.push((nx, ny));
        }
        if !added.is_empty() {
            added.sort_unstable();
            chain = merge_pairs(chain, &added);
            // Every added candidate also costs its examination.
            self.bound.standard += added.len() as u64;
        }
        if chain
            .windows(2)
            .any(|w| w[0].0 >= w[1].0 || w[0].1 >= w[1].1)
        {
            return Ok(None);
        }
        Ok(Some(chain))
    }

    /// A certified node: its chain kept, blocks re-extended near dirty lines, changed gaps recursed into or
    /// re-run.
    fn descend(
        &mut self,
        id: NodeId,
        old: &TraceNode,
        la: Range<u32>,
        ra: Range<u32>,
        parent: NodeId,
        cert: Cert,
    ) -> Result<NodeId, Fail> {
        let Cert {
            chain,
            prefix: p,
            suffix: s,
            diag,
        } = cert;
        let (ma, na) = (la.start + p, ra.start + p);
        let (mb, nb) = (la.end - s, ra.end - s);
        let blocks = if self.opts.algorithm == Algorithm::Standard {
            self.extend_blocks(old, &chain, (ma, na), (mb, nb))?
        } else {
            chain.iter().map(|&(x, y)| (x, y, 1)).collect()
        };
        // Growth of the node's own work: the remap of its middle (Standard charges its two lengths).
        if self.opts.algorithm == Algorithm::Standard {
            let (oml, omr) = old.stripped();
            let grow = (i64::from(mb - ma) + i64::from(nb - na))
                - (i64::from(oml.end - oml.start) + i64::from(omr.end - omr.start));
            self.bound.standard += grow.max(0) as u64;
        }
        let new_id = self.push_node(
            TraceNode {
                left: la.clone(),
                right: ra.clone(),
                candidates: if old.chain.len() as u32 == old.candidates {
                    chain.len() as u32
                } else {
                    old.candidates
                },
                chain,
                blocks: blocks.clone(),
                children: Vec::new(),
                prefix: p,
                suffix: s,
                diag,
                stage: old.stage,
                work: old.work,
                cells: old.cells,
                scorer: old.scorer,
                refusals: old.refusals,
            },
            parent,
        );
        self.out.push(equal_run(la.start, ra.start, p));
        // The gaps between the new blocks, in order.
        let (mut x0, mut y0) = (ma, na);
        let old_gaps = old_gaps(old);
        let kids = self.old_nodes[id as usize].children.clone();
        let mut new_gaps: Vec<(Range<u32>, Range<u32>)> = Vec::with_capacity(blocks.len() + 1);
        for &(x, y, len) in &blocks {
            new_gaps.push((x0..x, y0..y));
            (x0, y0) = (x + len, y + len);
        }
        new_gaps.push((x0..mb, y0..nb));
        let plan = self.gap_plan(&kids, &old_gaps, &new_gaps);
        for (k, ((a, b), how)) in new_gaps.into_iter().zip(plan).enumerate() {
            self.gap(how, a, b, new_id)?;
            if let Some(&(x, y, len)) = blocks.get(k) {
                self.out.push(equal_run(x, y, len));
            }
        }
        self.out.push(equal_run(mb, nb, s));
        Ok(new_id)
    }

    /// How each new gap of a certified node is processed (edge deltas, G8 follow-up). The old gaps are mapped to
    /// the new numbering; a new gap that is exactly one old gap mapped, without dirty lines, keeps its old runs
    /// and subtree. Otherwise each recorded child goes to the one new gap that overlaps its old gap most (lines
    /// of both sides; ties to the first), and each new gap processes the child with the largest overlap among
    /// those it was given on its new ranges (its prefix, suffix and middle found again, the lines that are new to
    /// its middle counted as inserted and those that left it as removed: §1.10.11 condition 1); a gap given no
    /// child is re-run. So a child is processed on one gap at most, whatever new blocks split or merge the old
    /// gaps, and every other part is re-run.
    fn gap_plan(
        &self,
        kids: &[NodeId],
        old_gaps: &[(Range<u32>, Range<u32>)],
        new_gaps: &[(Range<u32>, Range<u32>)],
    ) -> Vec<GapPlan> {
        let [lv, rv] = &self.sides.views;
        let mapped: Vec<(Range<u32>, Range<u32>)> = old_gaps
            .iter()
            .map(|(oa, ob)| {
                (
                    lv.pos(oa.start)..lv.pos(oa.end),
                    rv.pos(ob.start)..rv.pos(ob.end),
                )
            })
            .collect();
        let child_of: Vec<Option<NodeId>> = old_gaps
            .iter()
            .map(|(oa, ob)| {
                kids.iter().copied().find(|&c| {
                    let (l, r) = self.old_nodes[c as usize].ranges();
                    l == *oa && r == *ob
                })
            })
            .collect();
        let overlap = |x: &Range<u32>, y: &Range<u32>| -> u64 {
            u64::from(x.end.min(y.end).saturating_sub(x.start.max(y.start)))
        };
        // For each new gap, the old gaps it may meet: a contiguous stretch (both lists ascend on both sides).
        let mut stretches: Vec<Range<usize>> = Vec::with_capacity(new_gaps.len());
        let mut lo = 0usize;
        for (a, b) in new_gaps {
            while lo < mapped.len() && mapped[lo].0.end < a.start && mapped[lo].1.end < b.start {
                lo += 1;
            }
            let mut hi = lo;
            while hi < mapped.len() && (mapped[hi].0.start <= a.end || mapped[hi].1.start <= b.end)
            {
                hi += 1;
            }
            stretches.push(lo..hi);
        }
        // How many new gaps (not empty on both sides) meet each old gap.
        let mut met = vec![0u32; old_gaps.len()];
        for (j, (a, b)) in new_gaps.iter().enumerate() {
            if a.is_empty() && b.is_empty() {
                continue;
            }
            for i in stretches[j].clone() {
                if gap_meets(lv, rv, &old_gaps[i], a, b) {
                    met[i] += 1;
                }
            }
        }
        // Each child to the new gap that overlaps its old gap most.
        let mut best: Vec<Option<(usize, u64)>> = vec![None; old_gaps.len()];
        for (j, (a, b)) in new_gaps.iter().enumerate() {
            for i in stretches[j].clone() {
                if child_of[i].is_none() {
                    continue;
                }
                let o = overlap(&mapped[i].0, a) + overlap(&mapped[i].1, b);
                if o > 0 && best[i].is_none_or(|(_, bo)| o > bo) {
                    best[i] = Some((j, o));
                }
            }
        }
        new_gaps
            .iter()
            .enumerate()
            .map(|(j, (a, b))| {
                if a.is_empty() && b.is_empty() {
                    return GapPlan::Rerun;
                }
                let meets: Vec<usize> = stretches[j]
                    .clone()
                    .filter(|&i| gap_meets(lv, rv, &old_gaps[i], a, b))
                    .collect();
                // Exactly one old gap at the same place, met by no other new gap, without dirty lines: its runs and
                // subtree are kept.
                if let [i] = meets.as_slice() {
                    let (oa, ob) = &old_gaps[*i];
                    if met[*i] == 1
                        && mapped[*i] == (a.clone(), b.clone())
                        && !self.dirty_in(0, oa)
                        && !self.dirty_in(1, ob)
                    {
                        return GapPlan::Keep((oa.clone(), ob.clone()), child_of[*i]);
                    }
                }
                let chosen = meets
                    .iter()
                    .copied()
                    .filter(|&i| best[i].is_some_and(|(bj, _)| bj == j))
                    .max_by_key(|&i| (best[i].map_or(0, |(_, o)| o), std::cmp::Reverse(i)))
                    .and_then(|i| child_of[i].map(|c| (i, c)));
                // The child is processed when that costs less than re-running the gap: its moved lines and its
                // chain against the gap's lines (a moved line or a chain pair costs about a sixth of a line
                // re-run on the ids).
                let chosen = chosen.and_then(|(i, c)| {
                    let differ = |m: &Range<u32>, n: &Range<u32>| -> u64 {
                        u64::from(m.end - m.start) + u64::from(n.end - n.start) - 2 * overlap(m, n)
                    };
                    let moved = differ(&mapped[i].0, a) + differ(&mapped[i].1, b);
                    let chain = self.old_nodes[c as usize].node.chain.len() as u64;
                    let lines = u64::from(a.end - a.start) + u64::from(b.end - b.start);
                    (moved + 2 * chain <= RERUN_PER_MOVED_LINE * lines).then_some(c)
                });
                if let Some(c) = chosen {
                    return GapPlan::Child(c);
                }
                // A gap that was one of a child's own gaps, at the same place without dirty lines, keeps its old
                // runs and subtree: the engine processed exactly these lines as one range there.
                meets
                    .iter()
                    .filter_map(|&i| child_of[i])
                    .find_map(|c| self.child_gap(c, a, b))
                    .map_or(GapPlan::Rerun, |(old, kid)| GapPlan::Keep(old, kid))
            })
            .collect()
    }

    /// The gap of recorded node `c` (between its blocks inside its stripped middle) that maps exactly to new
    /// ranges `a × b` and holds no dirty line, with its recorded child: its old ranges.
    #[allow(clippy::type_complexity)]
    fn child_gap(
        &self,
        c: NodeId,
        a: &Range<u32>,
        b: &Range<u32>,
    ) -> Option<((Range<u32>, Range<u32>), Option<NodeId>)> {
        let live = &self.old_nodes[c as usize];
        let n = &live.node;
        if n.stage == Stage::Flat {
            return None;
        }
        let (mx, my) = (
            |x: u32| (i64::from(x) + live.dx) as u32,
            |y: u32| (i64::from(y) + live.dy) as u32,
        );
        let [lv, rv] = &self.sides.views;
        let (ml, mr) = n.stripped();
        // Gap g runs from the end of block g − 1 (the middle's start for g = 0) to the start of block g (the
        // middle's end for the last one); old numbering.
        let start = |g: usize| -> (u32, u32) {
            match g.checked_sub(1).and_then(|k| n.blocks.get(k)) {
                Some(&(x, y, len)) => (mx(x + len), my(y + len)),
                None => (mx(ml.start), my(mr.start)),
            }
        };
        let end = |g: usize| -> (u32, u32) {
            match n.blocks.get(g) {
                Some(&(x, y, _)) => (mx(x), my(y)),
                None => (mx(ml.end), my(mr.end)),
            }
        };
        let gaps = n.blocks.len() + 1;
        let (mut lo, mut hi) = (0usize, gaps);
        while lo < hi {
            let g = lo + (hi - lo) / 2;
            let (x, y) = start(g);
            if (lv.pos(x), rv.pos(y)) < (a.start, b.start) {
                lo = g + 1;
            } else {
                hi = g;
            }
        }
        let g = lo;
        if g >= gaps {
            return None;
        }
        let ((x0, y0), (x1, y1)) = (start(g), end(g));
        if (lv.pos(x0), rv.pos(y0), lv.pos(x1), rv.pos(y1)) != (a.start, b.start, a.end, b.end) {
            return None;
        }
        let old = (x0..x1, y0..y1);
        if x1 - x0 != a.end - a.start
            || y1 - y0 != b.end - b.start
            || self.dirty_in(0, &old.0)
            || self.dirty_in(1, &old.1)
        {
            return None;
        }
        let kid = live.children.iter().copied().find(|&k| {
            let (l, r) = self.old_nodes[k as usize].ranges();
            l == old.0 && r == old.1
        });
        Some((old, kid))
    }

    /// One gap of a certified node (new ranges `a × b`), as `gap_plan` decided: the old runs and subtree kept, a
    /// recorded child processed on the new ranges, or a re-run.
    fn gap(
        &mut self,
        how: GapPlan,
        a: Range<u32>,
        b: Range<u32>,
        parent: NodeId,
    ) -> Result<(), Fail> {
        if a.is_empty() && b.is_empty() {
            return Ok(());
        }
        match how {
            GapPlan::Keep((oa, ob), child) => {
                self.emit_old((oa.start, ob.start), (oa.end, ob.end));
                if let Some(c) = child {
                    self.copy_subtree(c, parent);
                }
                Ok(())
            }
            GapPlan::Child(c) => self.node(c, a, b, parent).map(|_| ()),
            GapPlan::Rerun => self.rerun(a, b, parent).map(|_| ()),
        }
    }

    /// Re-extends the blocks of a certified Standard node from its (new) chain, exactly as stage A extends
    /// them: backward to the previous block's end, forward to the next anchor. Pairs inside an old block on the
    /// same diagonal with both lines clean are known equal; other pairs are compared by key.
    fn extend_blocks(
        &mut self,
        old: &TraceNode,
        chain: &[(u32, u32)],
        start: (u32, u32),
        end: (u32, u32),
    ) -> Result<Vec<(u32, u32, u32)>, Fail> {
        let [lv, rv] = &self.sides.views;
        let in_old_block = |l: u32, r: u32| -> bool {
            let (Some(ox), Some(oy)) = (lv.old_of(l), rv.old_of(r)) else {
                return false;
            };
            let k = old.blocks.partition_point(|b| b.0 <= ox);
            k > 0 && {
                let (bx, by, bl) = old.blocks[k - 1];
                ox < bx + bl && i64::from(oy) - i64::from(ox) == i64::from(by) - i64::from(bx)
            }
        };
        // The old chain anchors of each old block (two pointers over the sorted chain and blocks).
        let mut block_anchors: Vec<Range<usize>> = Vec::with_capacity(old.blocks.len());
        let mut c0 = 0usize;
        for &(bx, _, bl) in &old.blocks {
            while c0 < old.chain.len() && old.chain[c0].0 < bx {
                c0 += 1;
            }
            let mut c1 = c0;
            while c1 < old.chain.len() && old.chain[c1].0 < bx + bl {
                c1 += 1;
            }
            block_anchors.push(c0..c1);
            c0 = c1;
        }
        // The old region of block k: from the previous block's end to the next block's first anchor (both sides).
        let (oml, omr) = old.stripped();
        let region = |k: usize| -> (Range<u32>, Range<u32>) {
            let (bx, by, bl) = old.blocks[k];
            let (px, py) = if k == 0 {
                (oml.start, omr.start)
            } else {
                let (x, y, l) = old.blocks[k - 1];
                (x + l, y + l)
            };
            // Up to the next anchor, which bounds the block's forward extension: the pair where the extension
            // stopped (possibly inside the next block's backward extension) is in the region.
            let (nx, ny) = block_anchors
                .get(k + 1)
                .and_then(|r| old.chain.get(r.start))
                .copied()
                .unwrap_or((oml.end, omr.end));
            let _ = (bx, by, bl);
            (px..nx, py..ny)
        };
        let mut blocks: Vec<(u32, u32, u32)> = Vec::with_capacity(old.blocks.len() + 2);
        let (mut px, mut py) = start;
        let mut steps = 0u64;
        let mut c = 0usize;
        while c < chain.len() {
            // A whole old block whose anchors, surroundings and start bound are unchanged is taken as it was.
            let reused = (|| -> Option<(usize, (u32, u32, u32))> {
                let ox = lv.old_of(chain[c].0)?;
                let k = old.blocks.partition_point(|b| b.0 <= ox).checked_sub(1)?;
                let anchors = &old.chain[block_anchors[k].clone()];
                if anchors.first()?.0 != ox || c + anchors.len() > chain.len() {
                    return None;
                }
                for (i, m) in self.mapped_chain[block_anchors[k].clone()]
                    .iter()
                    .enumerate()
                {
                    if (*m)? != chain[c + i] {
                        return None;
                    }
                }
                let (rl, rr) = region(k);
                if self.dirty_in(0, &rl) || self.dirty_in(1, &rr) {
                    return None;
                }
                // The backward bound: the previous block's end, or the middle's start (which edge deltas move).
                let prev_end = if k == 0 {
                    (lv.pos(oml.start), rv.pos(omr.start))
                } else {
                    let (x, y, l) = old.blocks[k - 1];
                    (lv.pos(x + l), rv.pos(y + l))
                };
                if prev_end != (px, py) {
                    return None;
                }
                // The next anchor bounds the forward extension: it must be the old next block's first anchor.
                let next_new = chain.get(c + anchors.len()).copied().unwrap_or(end);
                let next_old = old
                    .blocks
                    .get(k + 1)
                    .and_then(|_| old.chain.get(block_anchors[k].end))
                    .map(|&(x, y)| (lv.pos(x), rv.pos(y)))
                    .unwrap_or((lv.pos(oml.end), rv.pos(omr.end)));
                if next_new != next_old {
                    return None;
                }
                let (bx, by, bl) = old.blocks[k];
                Some((anchors.len(), (lv.pos(bx), rv.pos(by), bl)))
            })();
            if let Some((count, block)) = reused {
                match blocks.last_mut() {
                    Some(last) if last.0 + last.2 == block.0 && last.1 + last.2 == block.1 => {
                        last.2 += block.2
                    }
                    _ => blocks.push(block),
                }
                (px, py) = (block.0 + block.2, block.1 + block.2);
                c += count;
                continue;
            }
            let (x0, y0) = chain[c];
            let (nx, ny) = chain.get(c + 1).copied().unwrap_or(end);
            let mut back = 0u32;
            while x0 - back > px && y0 - back > py {
                let (l, r) = (x0 - back - 1, y0 - back - 1);
                if !(in_old_block(l, r) || self.equal(l, r)?) {
                    break;
                }
                back += 1;
            }
            let mut forward = 0u32;
            while x0 + 1 + forward < nx && y0 + 1 + forward < ny {
                let (l, r) = (x0 + 1 + forward, y0 + 1 + forward);
                if !(in_old_block(l, r) || self.equal(l, r)?) {
                    break;
                }
                forward += 1;
            }
            steps += u64::from(back) + u64::from(forward);
            let (x, y) = (x0 - back, y0 - back);
            let (xe, ye) = (x0 + 1 + forward, y0 + 1 + forward);
            match blocks.last_mut() {
                Some(last) if last.0 + last.2 == x && last.1 + last.2 == y => last.2 += xe - x,
                _ => blocks.push((x, y, xe - x)),
            }
            (px, py) = (xe, ye);
            c += 1;
        }
        // The extension steps re-counted (an upper bound of the node's new extension work).
        self.bound.standard += steps;
        Ok(blocks)
    }

    /// Re-runs the engine on new ranges `a × b` (keys interned locally; INV-ALIGN-8 makes this the full run's
    /// answer for the node), within what is left of the budgets.
    fn rerun(&mut self, a: Range<u32>, b: Range<u32>, parent: NodeId) -> Result<NodeId, Fail> {
        self.path.reruns += 1;
        let lines = u64::from(a.end - a.start) + u64::from(b.end - b.start);
        self.path.rerun_lines += lines;
        // Without line weights the engine runs on the interned ids (equal keys, equal ids): no line is keyed but
        // those closeness compares.
        let by_ids = !self.weights;
        self.spend(if by_ids {
            lines.div_ceil(RERUN_ID_LINES_PER_UNIT)
        } else {
            lines
        })?;
        let (std_budget, cells_budget, scorer_cap) = self.budgets;
        let left_std = std_budget
            .checked_sub(self.bound.standard)
            .ok_or(Fail::Full)?;
        let left_cells = cells_budget.saturating_sub(self.bound.closeness);
        let left_scorer = scorer_cap
            .checked_sub(self.bound.scorer)
            .ok_or(Fail::Full)?;
        let opts = AlignOptions {
            work_budget: Some(left_std),
            closeness: self.opts.closeness.map(|c| Closeness {
                max_cells_total: Some(left_cells),
                ..c
            }),
            ..self.opts
        };
        let (alignment, derivation) = if by_ids {
            self.align_ids(&a, &b, &opts, left_scorer)?
        } else {
            self.align_keys(&a, &b, &opts, left_scorer)?
        };
        self.bound.standard += derivation.work.standard;
        self.bound.closeness += derivation.work.closeness;
        self.bound.scorer += derivation.work.scorer;
        for r in alignment.runs() {
            self.out.push(Run {
                kind: r.kind,
                left: r.left.start + a.start..r.left.end + a.start,
                right: r.right.start + b.start..r.right.end + b.start,
            });
        }
        // The new subtree, moved to the segment's numbering.
        let seg = derivation.segments.into_iter().next();
        let mut root_id = hikasa_diff::NO_NODE;
        if let Some(seg) = seg {
            self.refusals += seg.nodes.iter().map(|n| n.refusals).sum::<u64>();
            let first = self.nodes.len() as NodeId;
            let (dx, dy) = (a.start, b.start);
            for (k, n) in seg.nodes.into_iter().enumerate() {
                let moved = TraceNode {
                    left: n.left.start + dx..n.left.end + dx,
                    right: n.right.start + dy..n.right.end + dy,
                    chain: n.chain.iter().map(|&(x, y)| (x + dx, y + dy)).collect(),
                    blocks: n
                        .blocks
                        .iter()
                        .map(|&(x, y, l)| (x + dx, y + dy, l))
                        .collect(),
                    diag: n
                        .diag
                        .iter()
                        .map(|&(x, y, l)| (x + dx, y + dy, l))
                        .collect(),
                    children: n.children.iter().map(|&c| c + first).collect(),
                    ..n
                };
                self.nodes.push(LiveNode::new(moved));
                if k == 0 {
                    root_id = first;
                }
            }
            if root_id != hikasa_diff::NO_NODE
                && let Some(p) = self.nodes.get_mut(parent as usize)
            {
                p.children.push(root_id);
            }
        }
        if root_id == hikasa_diff::NO_NODE {
            // A gap too small to record: its work belongs to the parent (the bound already counts it).
            return Ok(parent);
        }
        Ok(root_id)
    }
}

impl Inc<'_, '_> {
    /// The engine on new ranges `a × b` from keys rebuilt from the text (line weights count).
    fn align_keys(
        &self,
        a: &Range<u32>,
        b: &Range<u32>,
        opts: &AlignOptions,
        left_scorer: u64,
    ) -> Result<(hikasa_diff::Alignment, hikasa_diff::Derivation), Fail> {
        let [lv, rv] = &self.sides.views;
        let (lk, lw) = lv.keys_range(a.clone())?;
        let (rk, rw) = rv.keys_range(b.clone())?;
        let pair = intern_pair(lk.iter().map(Vec::as_slice), rk.iter().map(Vec::as_slice))
            .map_err(|_| Fail::Full)?;
        let mut trivial = TrivialSet::with_capacity(pair.distinct);
        for (ids, keys) in [(&pair.left, &lk), (&pair.right, &rk)] {
            for (id, key) in ids.iter().zip(keys) {
                if self.shape.is_trivial(key) {
                    trivial.insert(*id);
                }
            }
        }
        let (lw, rw) = if self.weights {
            (lw, rw)
        } else {
            (Vec::new(), Vec::new())
        };
        let input = AlignInput {
            left: &pair.left,
            right: &pair.right,
            left_weights: &lw,
            right_weights: &rw,
            trivial: &trivial,
            distinct: pair.distinct,
        };
        let ls = KeyStore::from_keys(lk.iter().map(Vec::as_slice));
        let rs = KeyStore::from_keys(rk.iter().map(Vec::as_slice));
        let scorer = ProfileScorer::with_budget(
            &ls,
            &rs,
            self.shape,
            self.threshold,
            self.cancel,
            left_scorer,
        );
        let done = self.align(&input, opts, &scorer)?;
        if scorer.exhausted() {
            return Err(Fail::Full);
        }
        Ok(done)
    }

    /// The engine on new ranges `a × b` from the interned ids (no line weights count), numbered densely in
    /// first-occurrence order, left then right, as `intern_pair` numbers keys: the same input. A first run with
    /// a scorer that only notes the lines closeness compares finds them (which gaps closeness pairs depends on
    /// the keys alone); those lines are keyed, and the second run scores them (any other line asked fails the
    /// path). Without a comparison asked, the first run is the result.
    fn align_ids(
        &self,
        a: &Range<u32>,
        b: &Range<u32>,
        opts: &AlignOptions,
        left_scorer: u64,
    ) -> Result<(hikasa_diff::Alignment, hikasa_diff::Derivation), Fail> {
        let [lv, rv] = &self.sides.views;
        let table = lv.table;
        let mut local: IdMap<u32> = IdMap::default();
        local.reserve(((a.end - a.start) + (b.end - b.start)) as usize / 2);
        let mut trivial = TrivialSet::default();
        let mut dense = |ids: Vec<u32>| -> Vec<TokenId> {
            ids.into_iter()
                .map(|g| {
                    let n = local.len() as u32;
                    let id = *local.entry(g).or_insert(n);
                    if id == n && table.trivial(g) {
                        trivial.insert(TokenId(n));
                    }
                    TokenId(id)
                })
                .collect()
        };
        let left = dense(lv.ids_of(a.clone()));
        let right = dense(rv.ids_of(b.clone()));
        let input = AlignInput {
            left: &left,
            right: &right,
            left_weights: &[],
            right_weights: &[],
            trivial: &trivial,
            distinct: local.len() as u32,
        };
        let asked = Asked::new(left.len(), right.len());
        let first = self.align(&input, opts, &asked)?;
        if !asked.any() {
            return Ok(first);
        }
        // The compared lines, keyed (the others stay empty: never read).
        let keys_of = |view: &SideView<'_>,
                       range: &Range<u32>,
                       bits: &[AtomicU64]|
         -> Result<KeyStore, Fail> {
            let n = range.end - range.start;
            let mut keys: Vec<Vec<u8>> = vec![Vec::new(); n as usize];
            let mut i = 0u32;
            while i < n {
                if !Asked::has(bits, i) {
                    i += 1;
                    continue;
                }
                let mut j = i + 1;
                while j < n && Asked::has(bits, j) {
                    j += 1;
                }
                let (k, _) = view.keys_range(range.start + i..range.start + j)?;
                for (slot, key) in keys[i as usize..j as usize].iter_mut().zip(k) {
                    *slot = key;
                }
                i = j;
            }
            Ok(KeyStore::from_keys(keys.iter().map(Vec::as_slice)))
        };
        let ls = keys_of(lv, a, &asked.left)?;
        let rs = keys_of(rv, b, &asked.right)?;
        let scorer = ProfileScorer::with_budget(
            &ls,
            &rs,
            self.shape,
            self.threshold,
            self.cancel,
            left_scorer,
        );
        let checked = Checked {
            inner: &scorer,
            asked: &asked,
            missed: AtomicBool::new(false),
        };
        let done = self.align(&input, opts, &checked)?;
        if scorer.exhausted() || checked.missed.load(Ordering::Relaxed) {
            return Err(Fail::Full);
        }
        Ok(done)
    }

    /// One engine run of a re-run node; a degraded or exhausted run fails the path.
    fn align(
        &self,
        input: &AlignInput<'_>,
        opts: &AlignOptions,
        scorer: &dyn hikasa_diff::PairScorer,
    ) -> Result<(hikasa_diff::Alignment, hikasa_diff::Derivation), Fail> {
        let (alignment, derivation) =
            align_traced(input, opts, &[], false, Some(scorer), None, self.cancel).map_err(
                |e| match e {
                    hikasa_diff::DiffError::Cancelled => Fail::Cancelled,
                    _ => Fail::Full,
                },
            )?;
        if derivation.work.exhausted
            || !alignment.diff_warnings().is_empty()
            || alignment
                .warnings()
                .iter()
                .any(|w| w.code == WarningCode::WorkBudgetExhausted)
        {
            return Err(Fail::Full);
        }
        Ok((alignment, derivation))
    }
}

/// The lines a closeness pass compares (one bit per range-local line and side); scores nothing.
struct Asked {
    left: Vec<AtomicU64>,
    right: Vec<AtomicU64>,
    any: AtomicBool,
}

impl Asked {
    fn new(left: usize, right: usize) -> Asked {
        let bits = |n: usize| (0..n.div_ceil(64)).map(|_| AtomicU64::new(0)).collect();
        Asked {
            left: bits(left),
            right: bits(right),
            any: AtomicBool::new(false),
        }
    }

    fn any(&self) -> bool {
        self.any.load(Ordering::Relaxed)
    }

    fn has(bits: &[AtomicU64], i: u32) -> bool {
        bits.get(i as usize / 64)
            .is_some_and(|w| w.load(Ordering::Relaxed) & (1 << (i % 64)) != 0)
    }

    fn note(bits: &[AtomicU64], i: u32) {
        if let Some(w) = bits.get(i as usize / 64) {
            w.fetch_or(1 << (i % 64), Ordering::Relaxed);
        }
    }
}

impl hikasa_diff::PairScorer for Asked {
    fn similarity(&self, l: u32, r: u32) -> u16 {
        Asked::note(&self.left, l);
        Asked::note(&self.right, r);
        self.any.store(true, Ordering::Relaxed);
        0
    }
}

/// The scorer of the second pass: the real one, on the lines the first pass noted only.
struct Checked<'s> {
    inner: &'s ProfileScorer<'s>,
    asked: &'s Asked,
    missed: AtomicBool,
}

impl hikasa_diff::PairScorer for Checked<'_> {
    fn similarity(&self, l: u32, r: u32) -> u16 {
        if !Asked::has(&self.asked.left, l) || !Asked::has(&self.asked.right, r) {
            self.missed.store(true, Ordering::Relaxed);
            return 0;
        }
        self.inner.similarity(l, r)
    }

    fn begin_gap(&self) {
        self.inner.begin_gap();
    }

    fn spent(&self) -> u64 {
        self.inner.spent()
    }
}

/// How a new gap of a certified node is processed (`Inc::gap_plan`).
enum GapPlan {
    /// The old gap at the same place without dirty lines (old ranges): its runs and recorded child are kept.
    Keep((Range<u32>, Range<u32>), Option<NodeId>),
    /// The recorded child processed on the new ranges (edge deltas).
    Child(NodeId),
    /// Re-run by the engine.
    Rerun,
}

/// True when new gap `a × b` meets old gap `og` mapped to the new numbering (half-open intervals meet when they
/// share a line; an empty one meets one that holds its point).
fn gap_meets(
    lv: &SideView<'_>,
    rv: &SideView<'_>,
    og: &(Range<u32>, Range<u32>),
    a: &Range<u32>,
    b: &Range<u32>,
) -> bool {
    let meets = |x: &Range<u32>, y: &Range<u32>| -> bool {
        if x.is_empty() || y.is_empty() {
            let (p, other) = if x.is_empty() {
                (x.start, y)
            } else {
                (y.start, x)
            };
            other.start <= p && p <= other.end
        } else {
            x.start < y.end && y.start < x.end
        }
    };
    let (oa, ob) = og;
    let (ma, mb) = (
        lv.pos(oa.start)..lv.pos(oa.end),
        rv.pos(ob.start)..rv.pos(ob.end),
    );
    meets(&ma, a) || meets(&mb, b)
}

/// A set of ids looked for by a walk (G8 follow-up): up to 32 ids are compared with blocks of 16 ids at once
/// (comparisons the compiler turns into vector instructions), more by one bit test per id.
struct Targets {
    groups: Vec<[u32; 8]>,
    bits: Vec<u64>,
}

impl Targets {
    /// Most ids compared block by block.
    const COMPARED: usize = 32;

    fn new(ids: impl IntoIterator<Item = u32>) -> Targets {
        let mut bits: Vec<u64> = Vec::new();
        let mut list: Vec<u32> = Vec::new();
        for id in ids {
            let w = id as usize / 64;
            if bits.len() <= w {
                bits.resize(w + 1, 0);
            }
            if bits[w] & (1 << (id % 64)) == 0 {
                bits[w] |= 1 << (id % 64);
                list.push(id);
            }
        }
        let groups = if list.len() <= Targets::COMPARED {
            // Padded with an id no line has (ids stay far below `u32::MAX`).
            list.chunks(8)
                .map(|c| {
                    let mut g = [u32::MAX; 8];
                    g[..c.len()].copy_from_slice(c);
                    g
                })
                .collect()
        } else {
            Vec::new()
        };
        Targets { groups, bits }
    }

    #[inline]
    fn contains(&self, id: u32) -> bool {
        self.bits
            .get(id as usize / 64)
            .is_some_and(|w| w & (1 << (id % 64)) != 0)
    }

    #[inline]
    fn block_has(block: &[u32; 16], group: &[u32; 8]) -> bool {
        let mut acc = 0u32;
        for &t in group {
            for &id in block {
                acc |= u32::from(id == t);
            }
        }
        acc != 0
    }

    /// Takes `id` out of the set.
    fn remove(&mut self, id: u32) {
        if let Some(w) = self.bits.get_mut(id as usize / 64) {
            *w &= !(1 << (id % 64));
        }
        for g in &mut self.groups {
            for t in g.iter_mut() {
                if *t == id {
                    *t = u32::MAX;
                }
            }
        }
    }

    /// Calls `hit(line, id)` for every id of `ids` (the ids of lines `first..`) in the set, in order; `hit` says
    /// whether to go on and whether the id is complete (it leaves the set). False when `hit` stopped the scan.
    fn scan(&mut self, first: u32, ids: &[u32], hit: &mut impl FnMut(u32, u32) -> Scan) -> bool {
        let mut at = first;
        let mut rest = ids;
        while !rest.is_empty() {
            let (block, tail) = rest.split_at(rest.len().min(16));
            let candidate = match <&[u32; 16]>::try_from(block) {
                Ok(b) if !self.groups.is_empty() => {
                    self.groups.iter().any(|g| Targets::block_has(b, g))
                }
                _ => true,
            };
            if candidate {
                for (i, &id) in block.iter().enumerate() {
                    if !self.contains(id) {
                        continue;
                    }
                    match hit(at + i as u32, id) {
                        Scan::Go => {}
                        Scan::Complete => self.remove(id),
                        Scan::Stop => return false,
                    }
                }
            }
            at += block.len() as u32;
            rest = tail;
        }
        true
    }
}

/// What a scan does after a hit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scan {
    /// Goes on.
    Go,
    /// Goes on without this id.
    Complete,
    /// Stops.
    Stop,
}

/// The union of two ascending lists of pairs, ascending, each pair once (a linear merge).
fn merge_pairs(a: Vec<(u32, u32)>, b: &[(u32, u32)]) -> Vec<(u32, u32)> {
    if b.is_empty() {
        return a;
    }
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() || j < b.len() {
        let next = match (a.get(i), b.get(j)) {
            (Some(&x), Some(&y)) if x <= y => {
                i += 1;
                if x == y {
                    j += 1;
                }
                x
            }
            (_, Some(&y)) => {
                j += 1;
                y
            }
            (Some(&x), None) => {
                i += 1;
                x
            }
            (None, None) => break,
        };
        out.push(next);
    }
    out
}

/// An Equal run of `len` pairs at `(x, y)`.
fn equal_run(x: u32, y: u32, len: u32) -> Run {
    Run {
        kind: RunKind::Equal,
        left: x..x + len,
        right: y..y + len,
    }
}

/// The old gaps of a node: between its blocks inside its stripped middle (old numbering).
fn old_gaps(node: &TraceNode) -> Vec<(Range<u32>, Range<u32>)> {
    let (ml, mr) = node.stripped();
    let mut out = Vec::with_capacity(node.blocks.len() + 1);
    let (mut x0, mut y0) = (ml.start, mr.start);
    for &(x, y, len) in &node.blocks {
        out.push((x0..x, y0..y));
        (x0, y0) = (x + len, y + len);
    }
    out.push((x0..ml.end, y0..mr.end));
    out
}

/// The row at old path point `(x, y)` (a row boundary of the old alignment).
fn row_at(cmp: &TextComparison, (x, y): (u32, u32)) -> u32 {
    let k = cmp.runs.partition_point(|r| {
        r.left.end < x
            || r.right.end < y
            || (r.left.end == x && r.right.end == y && !(r.left.is_empty() && r.right.is_empty()))
    });
    let Some(run) = cmp.runs.get(k) else {
        return cmp.row_count();
    };
    let offset = match run.kind {
        RunKind::Insert => y.saturating_sub(run.right.start),
        _ => x.saturating_sub(run.left.start),
    };
    cmp.run_rows[k] + offset.min(run.rows())
}

/// Rows `rows` (absolute, inside the run) of `run`, whose first row is `from_row`.
fn cut(run: &Run, from_row: u32, rows: Range<u32>) -> Run {
    let (j0, j1) = (rows.start - from_row, rows.end - from_row);
    match run.kind {
        RunKind::Equal | RunKind::Changed => Run {
            kind: run.kind,
            left: run.left.start + j0..run.left.start + j1,
            right: run.right.start + j0..run.right.start + j1,
        },
        RunKind::Delete => Run {
            kind: run.kind,
            left: run.left.start + j0..run.left.start + j1,
            right: run.right.start..run.right.start,
        },
        RunKind::Insert => Run {
            kind: run.kind,
            left: run.left.start..run.left.start,
            right: run.right.start + j0..run.right.start + j1,
        },
    }
}

/// Step 3: the comparison of the new runs, spliced into the last exact result (D-270): the runs that differ
/// from the old ones (mapped), and those holding re-keyed lines, form stretches; only their rows are built (the
/// old row's classification where it pairs the same clean lines, else classified) and replace the old rows of
/// each stretch, with sections, class runs and statistics changed by difference over the whole sections around
/// it (`splice::replace_window`); every other row chunk, section and run is the old one moved.
fn build_comparison(
    job: &RecompareJob,
    sides: &Sides<'_>,
    rekeyed: &[Vec<Rekeyed>; 2],
    runs: Vec<Run>,
    refusals: u64,
    cancel: &CancelToken,
) -> Result<TextComparison, Fail> {
    let base: &TextComparison = &job.base;
    let [lv, rv] = &sides.views;
    let (left, right) = (&job.left, &job.right);
    let orphans = base.rules.importance.orphans_always_important;
    let check = || cancel.check();
    // A run is unchanged when no re-keyed line is in it and its new ranges are its old ones moved.
    let touched = |side: usize, r: &Range<u32>| -> bool {
        let spans = &sides.views[side].spans;
        let k = spans.partition_point(|s| s.old.end < r.start);
        spans[k..]
            .iter()
            .take_while(|s| s.old.start <= r.end)
            .any(|s| {
                if s.old.is_empty() {
                    r.start < s.old.start && s.old.start < r.end
                } else {
                    s.old.start < r.end && r.start < s.old.end
                }
            })
    };
    let same = |o: &Run, n: &Run| -> bool {
        o.kind == n.kind
            && n.left == (lv.pos(o.left.start)..lv.pos(o.left.end))
            && n.right == (rv.pos(o.right.start)..rv.pos(o.right.end))
            && n.left.len() == o.left.len()
            && n.right.len() == o.right.len()
            && !touched(0, &o.left)
            && !touched(1, &o.right)
    };
    let old_runs = &base.runs;
    let at_old = |i: usize| -> u64 {
        old_runs.get(i).map_or(u64::MAX, |r| {
            u64::from(lv.pos(r.left.start)) + u64::from(rv.pos(r.right.start))
        })
    };
    let at_new = |j: usize| -> u64 {
        runs.get(j).map_or(u64::MAX, |r| {
            u64::from(r.left.start) + u64::from(r.right.start)
        })
    };
    let mut stretches: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < old_runs.len() || j < runs.len() {
        if i < old_runs.len() && j < runs.len() && same(&old_runs[i], &runs[j]) {
            i += 1;
            j += 1;
            continue;
        }
        let (i0, j0) = (i, j);
        loop {
            if i >= old_runs.len() && j >= runs.len() {
                break;
            }
            let (ao, an) = (at_old(i), at_new(j));
            if ao < an {
                i += 1;
            } else if an < ao {
                j += 1;
            } else if same(&old_runs[i], &runs[j]) {
                break;
            } else {
                i += 1;
                j += 1;
            }
        }
        stretches.push((i0..i, j0..j));
    }
    let row_of_run =
        |i: usize| -> u32 { base.run_rows.get(i).copied().unwrap_or(base.row_count()) };
    let compare_eol = base.set.compare_eol;
    let (mut lc, mut rc) = (left.cursor(), right.cursor());
    let mut cmp = base.clone();
    // The base's ignored lines moved through the edits (§1.10.17): the flags of every clean row stay right, and
    // the rebuilt rows take theirs from it; marks changed since (`set_ignored`) are applied at the end.
    for n in &job.log {
        cmp.ignored
            .side_mut(usize::from(n.side == hikasa_core::side::Side::Right))
            .move_through(n.delta);
    }
    let mut row_growth = 0i64;
    for (oi, nj) in stretches {
        cancel.check()?;
        let (r0, r1) = (row_of_run(oi.start), row_of_run(oi.end));
        let mut rows = rows::expand(
            &runs[nj.clone()],
            &|l: u32| lv.trivial(l),
            &|r: u32| rv.trivial(r),
            orphans,
            &check,
        )?;
        // Reuse or classify.
        for row in &mut rows {
            let ol = row
                .left()
                .filter(|&l| !lv.rekeyed(l))
                .and_then(|l| lv.old_of(l));
            let or = row
                .right()
                .filter(|&r| !rv.rekeyed(r))
                .and_then(|r| rv.old_of(r));
            let old_row = |side: hikasa_core::side::Side, line: u32| {
                base.row_for_line(side, LineIdx(line))
                    .and_then(|r| base.rows.get(r.0 as usize))
            };
            let old = match (row.left(), row.right(), ol, or) {
                (Some(_), Some(_), Some(ol), Some(or)) => {
                    old_row(hikasa_core::side::Side::Left, ol).filter(|o| o.right() == Some(or))
                }
                (Some(_), None, Some(ol), _) => {
                    old_row(hikasa_core::side::Side::Left, ol).filter(|o| o.right().is_none())
                }
                (None, Some(_), _, Some(or)) => {
                    old_row(hikasa_core::side::Side::Right, or).filter(|o| o.left().is_none())
                }
                _ => None,
            };
            if let Some(o) = old {
                row.set(o.state(), o.importance());
                continue;
            }
            let (Some(l), Some(r)) = (row.left(), row.right()) else {
                continue;
            };
            if base.identity_both() {
                continue;
            }
            let state = row.state();
            if state != RowState::Same && row.importance() != Importance::Important {
                continue;
            }
            let equal = rows::endings_equal(
                left.line_ending(LineIdx(l)),
                right.line_ending(LineIdx(r)),
                compare_eol,
            ) && rows::line_texts_equal(left, l, right, r, (&mut lc, &mut rc), &check)?;
            if (state == RowState::Same) != equal {
                row.set(RowState::Changed, Importance::Unimportant);
            }
        }
        if !cmp.ignored.is_empty() {
            for row in &mut rows {
                row.set_ignored(cmp.ignored.row_ignored(row.left(), row.right()));
            }
        }
        // The window: the stretch's rows (in the comparison as spliced so far) widened to whole sections.
        let (c0, c1) = (
            (i64::from(r0) + row_growth) as u32,
            (i64::from(r1) + row_growth) as u32,
        );
        let len = cmp.rows.len();
        let (mut w0, mut w1) = (c0, c1);
        if w0 > 0
            && let Some(id) = cmp.section_at(crate::compare::RowIdx(w0 - 1))
        {
            w0 = cmp.sections[id.0 as usize].rows.start;
        }
        if w1 < len
            && let Some(id) = cmp.section_at(crate::compare::RowIdx(w1))
        {
            w1 = cmp.sections[id.0 as usize].rows.end;
        }
        let old_window: Vec<rows::PackedRow> = cmp.rows.iter_range(w0..w1).collect();
        let (pre, post) = ((c0 - w0) as usize, (w1 - c1) as usize);
        // Line counts of the stretch, old and new, give the shift of every later row.
        let count = |rs: &[rows::PackedRow]| -> (i64, i64) {
            rs.iter().fold((0, 0), |(a, b), r| {
                (
                    a + i64::from(r.left().is_some()),
                    b + i64::from(r.right().is_some()),
                )
            })
        };
        let (ol, or) = count(&old_window[pre..old_window.len() - post]);
        let (nl, nr) = count(&rows);
        let shift = (nl - ol, nr - or);
        let move_row = |r: rows::PackedRow| {
            r.with_lines(
                r.left().map(|l| (i64::from(l) + shift.0) as u32),
                r.right().map(|x| (i64::from(x) + shift.1) as u32),
            )
        };
        let mut new_window: Vec<rows::PackedRow> = Vec::with_capacity(pre + rows.len() + post);
        new_window.extend_from_slice(&old_window[..pre]);
        new_window.extend_from_slice(&rows);
        new_window.extend(
            old_window[old_window.len() - post..]
                .iter()
                .copied()
                .map(move_row),
        );
        // The lines the window starts at.
        let first = runs[nj.clone()]
            .first()
            .map(|r| (r.left.start, r.right.start))
            .unwrap_or_else(|| {
                runs.get(nj.end)
                    .map_or((left.line_count(), right.line_count()), |r| {
                        (r.left.start, r.right.start)
                    })
            });
        let (pl, pr) = count(&old_window[..pre]);
        let lines = (
            (i64::from(first.0) - pl) as u32,
            (i64::from(first.1) - pr) as u32,
        );
        splice::replace_window(&mut cmp, w0..w1, &old_window, &new_window, lines, shift);
        row_growth += rows.len() as i64 - i64::from(r1 - r0);
    }
    // Warnings: the regex count adjusted by the re-keyed lines.
    let mut regex_errors = job.keys.regex_errors;
    for side in rekeyed {
        for r in side {
            regex_errors = (regex_errors + r.new_keys.errors).saturating_sub(r.old_keys.errors);
        }
    }
    let alignment_warnings: Vec<Warning> = if refusals > 0 {
        vec![Warning {
            code: WarningCode::ClosenessBudgetExhausted,
            message: "the closeness matching budget ran out; some differing lines were paired by position".to_owned(),
            spec: None,
        }]
    } else {
        Vec::new()
    };
    let doc_warnings = [left, right]
        .into_iter()
        .filter(|d| d.truncated())
        .map(|d| truncation_warning(d))
        .collect();
    let warnings = compose_warnings(&WarningInputs {
        left,
        right,
        doc_warnings,
        regex_errors,
        alignment: &alignment_warnings,
        scorer_exhausted: false,
        diff: &[],
        dropped: &[],
    });
    let mut run_rows = Vec::with_capacity(runs.len());
    let mut at = 0u32;
    for run in &runs {
        run_rows.push(at);
        at += run.rows();
    }
    cmp.left = left.clone();
    cmp.right = right.clone();
    cmp.bytes_identical = job.bytes_identical;
    cmp.runs = runs;
    cmp.run_rows = run_rows;
    cmp.warnings = warnings;
    cmp.start_states = [std::sync::OnceLock::new(), std::sync::OnceLock::new()];
    cmp.anchors = Vec::new();
    cmp.provisional = false;
    cmp.reignore(job.ignored.clone());
    Ok(cmp)
}

/// The live data of the new result: the ids spliced (only the chunks of the spans rebuilt), the table's
/// changes, the checkpoints spliced, the regex total, the derivation.
fn build_keys(
    job: &RecompareJob,
    sides: &Sides<'_>,
    rekeyed: &[Vec<Rekeyed>; 2],
    seg: LiveSegment,
    idview: &IdView<'_>,
    new_hash: Vec<u64>,
) -> Result<LiveKeys, Fail> {
    let mut checkpoints: [Vec<(u32, LineState)>; 2] = [Vec::new(), Vec::new()];
    let mut ids = [job.keys.ids[0].clone(), job.keys.ids[1].clone()];
    for side in 0..2 {
        let spans: Vec<(Range<u32>, &[u32])> = rekeyed[side]
            .iter()
            .map(|r| (r.old.clone(), r.new_keys.ids.as_slice()))
            .collect();
        ids[side] = job.keys.ids[side].spliced(&spans);
        let n = ids[side].len();
        let view = &sides.views[side];
        // Checkpoints: the old ones outside the spans moved, new ones inside the spans every 64 lines and at
        // their ends.
        let cps = &mut checkpoints[side];
        let mut spans = rekeyed[side].iter().peekable();
        let old_cps = &job.keys.checkpoints[side];
        let mut k = 0usize;
        loop {
            let next_span = spans.peek().map(|r| r.old.start);
            match (old_cps.get(k), next_span) {
                (Some(&(line, state)), Some(s)) if line < s => {
                    if let Some(x) = view.new_of(line) {
                        cps.push((x, state));
                    }
                    k += 1;
                }
                (_, Some(_)) => {
                    let Some(r) = spans.next() else { break };
                    for (i, &state) in r.new_keys.states.iter().enumerate() {
                        if (i as u32).is_multiple_of(CHECKPOINT_LINES) {
                            cps.push((r.new.start + i as u32, state));
                        }
                    }
                    if r.new.end < n {
                        cps.push((r.new.end, r.new_keys.end));
                    }
                    while old_cps.get(k).is_some_and(|c| c.0 <= r.old.end) {
                        k += 1;
                    }
                }
                (Some(&(line, state)), None) => {
                    if let Some(x) = view.new_of(line) {
                        cps.push((x, state));
                    }
                    k += 1;
                }
                (None, None) => break,
            }
        }
        cps.dedup_by_key(|c| c.0);
    }
    let mut regex_errors = job.keys.regex_errors;
    for side in rekeyed {
        for r in side {
            regex_errors = (regex_errors + r.new_keys.errors).saturating_sub(r.old_keys.errors);
        }
    }
    let [cl, cr] = checkpoints;
    let delta = TableDelta {
        new_ids: new_hash
            .into_iter()
            .zip(idview.new_trivial.iter().copied())
            .collect(),
        counts: idview.delta.clone(),
    };
    Ok(LiveKeys {
        trace: Arc::new(LiveTrace {
            work: seg.seg.work,
            segments: vec![seg],
        }),
        ids,
        table: job.keys.table.clone(),
        delta: Some(Arc::new(delta)),
        checkpoints: [Arc::new(cl), Arc::new(cr)],
        regex_errors,
    })
}
