//! The exact incremental recompare (normative, §1.10.11; D-269).
//!
//! ```text
//! step 1  dirty lines: the deltas merged into spans per side, re-keyed (old and new), the tokenizer re-converged;
//!         a span whose keys and weights are unchanged is dirty for the rows only (re-classified)
//! step 2  descend the segment's derivation from its root while the node's certificate holds:
//!           1 range     the dirty lines inside the stripped middle; the prefix and suffix keep their lengths
//!                       (the boundary pairs still differ); Auto resolves to the recorded engine
//!           2 candidates an AnchorsUnique node; no key of a dirty line is or becomes unique on both sides in the
//!                       middle (counts exact: fingerprint scan, keys verified, at most 4,096 verifications)
//!           3 window    per diagonal run of the node's unique pairs: membership before and after, exact
//!         a certified node keeps its chain; blocks near dirty lines are re-extended from their anchors; its
//!         changed gaps are recursed into (a recorded child of the same range) or re-run by the engine; the first
//!         node whose certificate fails is re-run whole by its engine on locally interned keys
//!           4 budgets   the totals (an upper bound: old totals plus every re-run's and re-extension's work) stay
//!                       within the budgets of the new line counts; else the full compare
//! step 3  the runs canonicalized; rows reused where the pairing of clean lines is the old one, else classified;
//!         sections, statistics and warnings re-derived; the derivation, fingerprints and checkpoints updated
//! T6      dirty lines over 25 % of the lines or 200,000 lines, anchors, engines without nodes, a recorded
//!         budget refusal: the full compare (the same result)
//! ```
//!
//! Any doubt (a condition that cannot be shown cheaply) fails the certificate: a failed certificate only costs
//! time, never exactness.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use hikasa_core::cancel::{CancelToken, Cancelled};
use hikasa_core::progress::Ctl;
use hikasa_core::warning::{Warning, WarningCode};
use hikasa_diff::{
    Algorithm, AlignInput, AlignOptions, Closeness, Derivation, NodeId, Run, RunKind, SegmentTrace,
    Stage, TraceNode, TrivialSet, WorkTotals, align_traced, canonical_runs, intern_pair,
};
use hikasa_formats::LineState;

use super::LiveKeys;
use super::fingerprint::{Keyer, LineMap, Rekeyed, SideView};
use super::recompare::{PathCounters, RecompareJob};
use crate::classify::KeyShape;
use crate::compare::keys::KeyStore;
use crate::compare::rows::{self, Importance, RowState};
use crate::compare::score::{ProfileScorer, scorer_budget};
use crate::compare::{
    CHECKPOINT_LINES, TextComparison, WarningInputs, compose_warnings, sections, store,
    truncation_warning,
};
use crate::document::LineIdx;
use crate::error::TextError;
use crate::rules::AlgorithmChoice;

/// T6: the incremental path is not taken beyond this share of dirty lines.
const FALLBACK_SHARE: f64 = 0.25;
/// T6: … or beyond this many dirty lines.
const FALLBACK_LINES: u64 = 200_000;
/// Key verifications one node's candidate count may take before its certificate fails.
const VERIFY_LIMIT: u32 = 4_096;

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
    let [seg] = trace.segments.as_slice() else {
        return Ok(None);
    };
    let Some(root) = seg.root else {
        return Ok(None);
    };
    if seg.pair || !matches!(seg.engine, Algorithm::Standard | Algorithm::Patience) {
        return Ok(None);
    }
    // Condition 4, recorded part: no budget refused anything in the last exact run.
    if seg.work.exhausted || seg.nodes.iter().any(|n| n.refusals > 0) {
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
    if !base.identity_path && base.format.grammar().has_line_one() {
        for k in 0..2 {
            maps[k].touch_line_zero(old_docs[k].line_count(), new_docs[k].line_count());
        }
    }
    let total = u64::from(new_docs[0].line_count()) + u64::from(new_docs[1].line_count());
    let dirty = maps[0].dirty_lines() + maps[1].dirty_lines();
    if dirty > FALLBACK_LINES || dirty as f64 > FALLBACK_SHARE * total.max(1) as f64 {
        path.fallback += 1;
        return Ok(None);
    }
    // Step 1: re-keying.
    let shape = KeyShape {
        identity: base.identity_path,
        compare_eol: base.set.compare_eol,
    };
    let keyer = Keyer {
        set: &base.set,
        shape,
        format: &base.format,
        weights: seg.engine == Algorithm::Standard,
        multiline: !shape.identity && base.format.grammar().has_multiline(),
        cancel,
    };
    let rekeyed: [Vec<Rekeyed>; 2] = [
        keyer.rekey(&maps[0], old_docs[0], new_docs[0], &job.keys.checkpoints[0])?,
        keyer.rekey(&maps[1], old_docs[1], new_docs[1], &job.keys.checkpoints[1])?,
    ];
    path.rekeyed += rekeyed
        .iter()
        .flatten()
        .map(|r| u64::from(r.new.end - r.new.start))
        .sum::<u64>();
    let weights = keyer.weights && base.format.has_line_weights();
    let adirty = [trimmed(&rekeyed[0], weights), trimmed(&rekeyed[1], weights)];
    let news = |k: usize| adirty[k].iter().map(|(_, n)| n.clone()).collect::<Vec<_>>();
    let sides = Sides {
        views: [
            SideView::new(
                &rekeyed[0],
                old_docs[0],
                &job.keys.fingerprints[0],
                &job.keys.checkpoints[0],
                &keyer,
                news(0),
            ),
            SideView::new(
                &rekeyed[1],
                old_docs[1],
                &job.keys.fingerprints[1],
                &job.keys.checkpoints[1],
                &keyer,
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
        && rules.algorithm_for(&base.format) == Algorithm::Histogram;
    let mut inc = Inc {
        base,
        seg,
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
    let new_root = inc.node(root, la, ra, hikasa_diff::NO_NODE)?;
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
    let new_root_node = &nodes[new_root as usize];
    let new_seg = SegmentTrace {
        left: 0..n,
        right: 0..m,
        prefix: new_root_node.prefix,
        suffix: new_root_node.suffix,
        engine: seg.engine,
        pair: false,
        root: Some(new_root),
        nodes,
        work: WorkTotals {
            standard: bound.standard,
            standard_budget: std_budget,
            closeness: bound.closeness,
            closeness_budget: cells_budget,
            scorer: bound.scorer,
            ..WorkTotals::default()
        },
    };
    let cmp = build_comparison(job, &sides, &rekeyed, runs, refusals, cancel)?;
    let keys = build_keys(job, &sides, &rekeyed, new_seg)?;
    Ok(Some((cmp, keys)))
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

/// The lines of one side whose key may count differently in a node's middle: old lines (old numbering) and new
/// lines (new numbering), with their keys (trivial keys left out).
#[derive(Debug, Default)]
struct MiddleChanges {
    old: Vec<(u32, Vec<u8>)>,
    new: Vec<(u32, Vec<u8>)>,
}

/// A dirty key: its dirty counts per side (old lines, new lines) and the old lines that had it.
#[derive(Debug, Clone, Default)]
struct KeyInfo {
    dirty: [(i64, i64); 2],
    old_lines: [Vec<u32>; 2],
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
    seg: &'a SegmentTrace,
    sides: &'a Sides<'a>,
    opts: AlignOptions,
    shape: KeyShape,
    cancel: &'a CancelToken,
    /// The new runs in order (canonicalized at the end).
    out: Vec<Run>,
    /// The new derivation's nodes.
    nodes: Vec<TraceNode>,
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
}

impl Inc<'_, '_> {
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

    /// Equality of the keys of new left line `l` and new right line `r` (fingerprints first).
    fn equal(&self, l: u32, r: u32) -> Result<bool, Fail> {
        let [lv, rv] = &self.sides.views;
        if lv.fp(l) != rv.fp(r) {
            return Ok(false);
        }
        let (a, _) = lv.key(l)?;
        let (b, _) = rv.key(r)?;
        Ok(a == b)
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
        let old = &self.seg.nodes[id as usize];
        let [lv, rv] = &self.sides.views;
        let ml = |x: u32| lv.pos(x);
        let mr = |x: u32| rv.pos(x);
        let node = TraceNode {
            left: ml(old.left.start)..ml(old.left.end),
            right: mr(old.right.start)..mr(old.right.end),
            chain: old.chain.iter().map(|&(x, y)| (ml(x), mr(y))).collect(),
            blocks: old
                .blocks
                .iter()
                .map(|&(x, y, l)| (ml(x), mr(y), l))
                .collect(),
            diag: old
                .diag
                .iter()
                .map(|&(x, y, l)| (ml(x), mr(y), l))
                .collect(),
            children: Vec::new(),
            ..old.clone()
        };
        let new_id = self.push_node(node, parent);
        for &c in &old.children.clone() {
            self.copy_subtree(c, new_id);
        }
        new_id
    }

    fn push_node(&mut self, node: TraceNode, parent: NodeId) -> NodeId {
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
        let old = &self.seg.nodes[id as usize];
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
        match self.certify(id, &la, &ra)? {
            Some(cert) => self.descend(id, la, ra, parent, cert),
            None => self.rerun(la, ra, parent),
        }
    }

    /// The certificate of node `id` on its new ranges `la × ra` (conditions 1–3). Condition 1 (edge deltas):
    /// the node's new common prefix and suffix are found from the old `Equal` runs outward (O(change)); the
    /// middle must stay non-empty on both sides; lines that move between the stripped ends and the middle count
    /// for condition 2 as removed (moved out) or inserted (moved in), like dirty lines.
    fn certify(
        &mut self,
        id: NodeId,
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
        let (oml, omr) = self.seg.nodes[id as usize].stripped();
        let om = [oml, omr];
        let old = &self.seg.nodes[id as usize];
        // 2. Candidates.
        let chain = if old.stage == Stage::AnchorsUnique {
            self.new_chain(id, &om, &nm)?
        } else {
            None
        };
        let Some(chain) = chain else {
            self.path.candidates_failed += 1;
            return Ok(None);
        };
        self.path.candidates_held += 1;
        // 3. Window (Standard): exact per diagonal run of the node's unique pairs (`TraceNode::diag`).
        let Some((chain, diag)) = self.window(id, &om, &nm, chain)? else {
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
        id: NodeId,
        om: &[Range<u32>; 2],
        nm: &[Range<u32>; 2],
        mut chain: Vec<(u32, u32)>,
    ) -> Result<Option<(Vec<(u32, u32)>, Vec<(u32, u32, u32)>)>, Fail> {
        let old = &self.seg.nodes[id as usize];
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
            let mut leaving: std::collections::HashSet<u32> = Default::default();
            let mut lines: Vec<(u32, u32)> = Vec::new();
            for &((px, py, len), m_old, m_new) in &pieces {
                if m_old && !m_new {
                    let from = chain.partition_point(|c| c.0 < px);
                    for &(x, y) in chain[from..].iter().take_while(|c| c.0 < px + len) {
                        if i64::from(y) - i64::from(x) == i64::from(py) - i64::from(px) {
                            leaving.insert(x);
                        }
                    }
                } else if !m_old && m_new {
                    for i in 0..len {
                        let (x, y) = (px + i, py + i);
                        let f = lv.fp(x);
                        if f != 0 && f == rv.fp(y) {
                            lines.push((x, y));
                        }
                    }
                }
            }
            // The entering lines are pairs when their keys are equal and unique on both sides in the new middle.
            let mut keys: Vec<Vec<u8>> = Vec::with_capacity(lines.len());
            let mut pairs: Vec<(u32, u32)> = Vec::with_capacity(lines.len());
            for &(x, y) in &lines {
                let (a, _) = lv.key(x)?;
                let (b, _) = rv.key(y)?;
                if a == b {
                    keys.push(a.into_owned());
                    pairs.push((x, y));
                }
            }
            if !keys.is_empty() {
                let refs: Vec<&[u8]> = keys.iter().map(Vec::as_slice).collect();
                let caps = vec![[2i64, 2i64]; keys.len()];
                let Some((counts, _)) = self.count_keys(&refs, &caps, nm)? else {
                    return Ok(None);
                };
                for (k, &pair) in pairs.iter().enumerate() {
                    if counts[k] == [1, 1] {
                        chain.push(pair);
                    }
                }
            }
            chain.retain(|(x, _)| !leaving.contains(x));
            chain.sort_unstable();
            chain.dedup();
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

    /// The dirty lines of a node for condition 2, per side: the old lines of its old middle `om` whose key may
    /// count differently now (alignment-dirty, or moved out of the middle) and the new lines of its new middle
    /// `nm` likewise (alignment-dirty, or moved in), with their keys. Every other line of the old middle is a
    /// clean line of the new middle and back.
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
                let i = (x - span.old.start) as usize;
                if span.old_keys.fps[i] != 0 {
                    out.old.push((x, span.old_keys.keys[i].clone()));
                }
            }
            for x in n.start.max(nm.start)..n.end.min(nm.end) {
                let i = (x - span.new.start) as usize;
                if span.new_keys.fps[i] != 0 {
                    out.new.push((x, span.new_keys.keys[i].clone()));
                }
            }
        }
        // Clean lines that moved: the new middle against the mapped old one (both intervals; a clean line's
        // old line is in the old middle exactly when its new line is in the mapped interval).
        let mapped = view.pos(om.start)..view.pos(om.end);
        let moved_in = [
            nm.start..nm.end.min(mapped.start),
            nm.start.max(mapped.end)..nm.end,
        ];
        let moved_out = [
            mapped.start..mapped.end.min(nm.start),
            mapped.start.max(nm.end)..mapped.end,
        ];
        for r in moved_in {
            for x in r {
                if view.old_of(x).is_some() && view.fp(x) != 0 {
                    let (key, _) = view.key(x)?;
                    out.new.push((x, key.into_owned()));
                }
            }
        }
        for r in moved_out {
            for x in r {
                if let Some(o) = view.old_of(x)
                    && view.fp(x) != 0
                {
                    let (key, _) = view.key(x)?;
                    out.old.push((o, key.into_owned()));
                }
            }
        }
        Ok(Some(out))
    }

    /// Counts `keys` in the new middles `middle` (per side, up to `caps`): a fingerprint walk of the middles and
    /// the hits verified by their keys (at most `VERIFY_LIMIT` verifications, else `None`). Returns the counts
    /// and the lines found.
    #[allow(clippy::type_complexity)]
    fn count_keys(
        &self,
        keys: &[&[u8]],
        caps: &[[i64; 2]],
        middle: &[Range<u32>; 2],
    ) -> Result<Option<(Vec<[i64; 2]>, Vec<[Vec<u32>; 2]>)>, Fail> {
        let mut fps: HashMap<u16, Vec<usize>> = HashMap::new();
        let mut bitmap = vec![0u64; 1024];
        for (k, key) in keys.iter().enumerate() {
            let fp = crate::compare::key_fingerprint(key, self.shape, self.cancel)?;
            fps.entry(fp).or_default().push(k);
            bitmap[usize::from(fp) / 64] |= 1 << (fp % 64);
        }
        let mut counts = vec![[0i64; 2]; keys.len()];
        let mut hits: Vec<[Vec<u32>; 2]> = vec![[Vec::new(), Vec::new()]; keys.len()];
        let mut verified = 0u32;
        for side in 0..2 {
            let view = &self.sides.views[side];
            let mut candidates: Vec<u32> = Vec::new();
            view.for_each_fp(middle[side].clone(), |x, fp| {
                if bitmap[usize::from(fp) / 64] & (1 << (fp % 64)) != 0 {
                    candidates.push(x);
                }
            });
            self.cancel.check().map_err(|_| Fail::Cancelled)?;
            for x in candidates {
                let Some(ks) = fps.get(&view.fp(x)) else {
                    continue;
                };
                if ks.iter().all(|&k| counts[k][side] >= caps[k][side]) {
                    continue;
                }
                verified += 1;
                if verified > VERIFY_LIMIT {
                    return Ok(None);
                }
                let (key, _) = view.key(x)?;
                for &k in ks {
                    if *key == *keys[k] && counts[k][side] < caps[k][side] {
                        counts[k][side] += 1;
                        hits[k][side].push(x);
                    }
                }
            }
        }
        Ok(Some((counts, hits)))
    }

    /// Condition 2 and the node's new chain. The chain is unchanged when no key of a removed or inserted dirty
    /// line is, or becomes, unique on both sides in the middle (counts exact: fingerprint scan, hits verified,
    /// at most 4,096 verifications). When the recorded chain holds every candidate, candidates may also go (their
    /// lines changed, or their keys stopped being unique) and come (a key became unique in the window), as long as
    /// the new set is still increasing on both sides: the chain is then the new set (every candidate scores
    /// more than nothing). `None`: the certificate fails.
    fn new_chain(
        &mut self,
        id: NodeId,
        om: &[Range<u32>; 2],
        nm: &[Range<u32>; 2],
    ) -> Result<Option<Vec<(u32, u32)>>, Fail> {
        let old = &self.seg.nodes[id as usize];
        let middle = nm.clone();
        // What the window check and the new record read is this node's alone (an early return below keeps it
        // empty, never the previous node's).
        self.added_outside.clear();
        // The keys of the dirty lines (old and new, moved lines included) with their dirty counts per side and the
        // old lines that had them.
        let mut keys: HashMap<Vec<u8>, KeyInfo> = HashMap::new();
        for side in 0..2 {
            let Some(changes) = self.middle_changes(side, &om[side], &nm[side])? else {
                return Ok(None);
            };
            for (x, key) in changes.old {
                let info = keys.entry(key).or_default();
                info.dirty[side].0 += 1;
                info.old_lines[side].push(x);
            }
            for (_, key) in changes.new {
                keys.entry(key).or_default().dirty[side].1 += 1;
            }
        }
        let [lv, rv] = &self.sides.views;
        // The old chain in the new numbering (both coordinates ascending: one walk each).
        let xs = lv.new_of_sorted(old.chain.iter().map(|c| c.0));
        let ys = rv.new_of_sorted(old.chain.iter().map(|c| c.1));
        self.mapped_chain = xs
            .into_iter()
            .zip(ys)
            .map(|(x, y)| Some((x?, y?)))
            .collect();
        let mapped_chain = self.mapped_chain.clone();
        let mapped = |chain: &[(u32, u32)],
                      drop: &std::collections::HashSet<u32>|
         -> Option<Vec<(u32, u32)>> {
            chain
                .iter()
                .zip(&mapped_chain)
                .filter(|((x, _), _)| !drop.contains(x))
                .map(|(_, m)| *m)
                .collect()
        };
        if keys.is_empty() {
            return Ok(mapped(&old.chain, &Default::default()));
        }
        // Count every key in the new middles: fingerprint walk, verification of the hits.
        let list: Vec<(Vec<u8>, KeyInfo)> = keys.into_iter().collect();
        let caps: Vec<[i64; 2]> = list
            .iter()
            .map(|(_, c)| {
                [
                    3 + c.dirty[0].0.max(c.dirty[0].1),
                    3 + c.dirty[1].0.max(c.dirty[1].1),
                ]
            })
            .collect();
        let key_refs: Vec<&[u8]> = list.iter().map(|(k, _)| k.as_slice()).collect();
        let Some((counts, hits)) = self.count_keys(&key_refs, &caps, &middle)? else {
            return Ok(None);
        };
        let all = old.chain.len() as u32 == old.candidates && old.candidates > 0;
        let (na, nb) = (
            middle[0].end - middle[0].start,
            middle[1].end - middle[1].start,
        );
        let mut removed: std::collections::HashSet<u32> = Default::default();
        let mut gone_lines: Vec<u32> = Vec::new();
        let mut added: Vec<(u32, u32)> = Vec::new();
        for (k, (_, info)) in list.iter().enumerate() {
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
            if was && !is {
                // The pair stops being unique: drop it from the recorded pairs.
                if let Some(&x) = info.old_lines[0].first() {
                    gone_lines.push(x);
                } else if let Some(x) = hits[k][0]
                    .iter()
                    .find_map(|&h| lv.old_of(h).filter(|o| om[0].contains(o)))
                {
                    gone_lines.push(x);
                }
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
                removed.insert(x);
            }
            if is {
                let (x, y) = (hits[k][0][0], hits[k][1][0]);
                let in_window = self.opts.algorithm != Algorithm::Standard
                    || window_holds(
                        x - middle[0].start,
                        y - middle[1].start,
                        na,
                        nb,
                        self.opts.skew_tolerance,
                    );
                if in_window {
                    added.push((x, y));
                }
                self.added_outside.push((x, y));
            }
        }
        let _ = gone_lines;
        let Some(mut chain) = mapped(&old.chain, &removed) else {
            return Ok(None);
        };
        if !added.is_empty() {
            chain.extend(added.iter().copied());
            chain.sort_unstable();
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
        la: Range<u32>,
        ra: Range<u32>,
        parent: NodeId,
        cert: Cert,
    ) -> Result<NodeId, Fail> {
        let old = self.seg.nodes[id as usize].clone();
        let Cert {
            chain,
            prefix: p,
            suffix: s,
            diag,
        } = cert;
        let (ma, na) = (la.start + p, ra.start + p);
        let (mb, nb) = (la.end - s, ra.end - s);
        let blocks = if self.opts.algorithm == Algorithm::Standard {
            self.extend_blocks(&old, &chain, (ma, na), (mb, nb))?
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
                chain: chain.clone(),
                blocks: blocks.clone(),
                children: Vec::new(),
                prefix: p,
                suffix: s,
                diag,
                ..old.clone()
            },
            parent,
        );
        self.out.push(equal_run(la.start, ra.start, p));
        // The gaps between the new blocks, in order.
        let (mut x0, mut y0) = (ma, na);
        let old_gaps = old_gaps(&old);
        for &(x, y, len) in &blocks {
            self.gap(&old, &old_gaps, x0..x, y0..y, new_id)?;
            self.out.push(equal_run(x, y, len));
            (x0, y0) = (x + len, y + len);
        }
        self.gap(&old, &old_gaps, x0..mb, y0..nb, new_id)?;
        self.out.push(equal_run(mb, nb, s));
        Ok(new_id)
    }

    /// One gap of a certified node (new ranges `a × b`), mapped to the old gaps it overlaps (edge deltas): the
    /// old runs when it is an old gap without dirty lines at the same place; the recorded child of the one old
    /// gap it overlaps, processed on the new ranges (its prefix and suffix adjusted by `node`); else a re-run.
    fn gap(
        &mut self,
        old: &TraceNode,
        old_gaps: &[(Range<u32>, Range<u32>)],
        a: Range<u32>,
        b: Range<u32>,
        parent: NodeId,
    ) -> Result<(), Fail> {
        if a.is_empty() && b.is_empty() {
            return Ok(());
        }
        let [lv, rv] = &self.sides.views;
        // Half-open intervals meet when they share a line; an empty one meets one that holds its point.
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
        let overlapping: Vec<&(Range<u32>, Range<u32>)> = old_gaps
            .iter()
            .filter(|(oa, ob)| {
                let (ma, mb) = (
                    lv.pos(oa.start)..lv.pos(oa.end),
                    rv.pos(ob.start)..rv.pos(ob.end),
                );
                meets(&ma, &a) || meets(&mb, &b)
            })
            .collect();
        if let [(oa, ob)] = overlapping.as_slice() {
            let (oa, ob) = (oa.clone(), ob.clone());
            let child = old.children.iter().copied().find(|&c| {
                let n = &self.seg.nodes[c as usize];
                n.left == oa && n.right == ob
            });
            let same = lv.pos(oa.start) == a.start
                && lv.pos(oa.end) == a.end
                && rv.pos(ob.start) == b.start
                && rv.pos(ob.end) == b.end;
            if same && !self.dirty_in(0, &oa) && !self.dirty_in(1, &ob) {
                self.emit_old((oa.start, ob.start), (oa.end, ob.end));
                if let Some(c) = child {
                    self.copy_subtree(c, parent);
                }
                return Ok(());
            }
            if let Some(c) = child {
                self.node(c, a, b, parent)?;
                return Ok(());
            }
        }
        self.rerun(a, b, parent).map(|_| ())
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
        let (alignment, derivation) =
            align_traced(&input, &opts, &[], false, Some(&scorer), None, self.cancel).map_err(
                |e| match e {
                    hikasa_diff::DiffError::Cancelled => Fail::Cancelled,
                    _ => Fail::Full,
                },
            )?;
        if scorer.exhausted()
            || derivation.work.exhausted
            || !alignment.diff_warnings().is_empty()
            || alignment
                .warnings()
                .iter()
                .any(|w| w.code == WarningCode::WorkBudgetExhausted)
        {
            return Err(Fail::Full);
        }
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
                self.nodes.push(moved);
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

/// Step 3: the comparison of the new runs: rows (the old row's classification where it pairs the same clean
/// lines), sections, statistics, warnings.
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
    let mut rows = rows::expand(
        &runs,
        &|l: u32| lv.fp(l) == 0,
        &|r: u32| rv.fp(r) == 0,
        orphans,
        &check,
    )?;
    // Reuse or classify.
    let old_rows = base.rows.to_vec();
    let mut by_left: Vec<u32> = vec![u32::MAX; base.left.line_count() as usize];
    let mut by_right: Vec<u32> = vec![u32::MAX; base.right.line_count() as usize];
    for (k, r) in old_rows.iter().enumerate() {
        if let Some(l) = r.left() {
            by_left[l as usize] = k as u32;
        }
        if let Some(x) = r.right() {
            by_right[x as usize] = k as u32;
        }
    }
    let compare_eol = base.set.compare_eol;
    let (mut lc, mut rc) = (left.cursor(), right.cursor());
    for (k, row) in rows.iter_mut().enumerate() {
        if k % 65_536 == 0 {
            cancel.check()?;
        }
        let ol = row
            .left()
            .filter(|&l| !lv.rekeyed(l))
            .and_then(|l| lv.old_of(l));
        let or = row
            .right()
            .filter(|&r| !rv.rekeyed(r))
            .and_then(|r| rv.old_of(r));
        let old = match (row.left(), row.right(), ol, or) {
            (Some(_), Some(_), Some(ol), Some(or)) => {
                let k = by_left[ol as usize];
                old_rows
                    .get(k as usize)
                    .filter(|o| o.right() == Some(or))
                    .copied()
            }
            (Some(_), None, Some(ol), _) => old_rows
                .get(by_left[ol as usize] as usize)
                .filter(|o| o.right().is_none())
                .copied(),
            (None, Some(_), _, Some(or)) => old_rows
                .get(by_right[or as usize] as usize)
                .filter(|o| o.left().is_none())
                .copied(),
            _ => None,
        };
        if let Some(o) = old {
            row.set(o.state(), o.importance());
            continue;
        }
        let (Some(l), Some(r)) = (row.left(), row.right()) else {
            continue;
        };
        if base.identity_path {
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
    let derived = sections::derive(
        &rows,
        left.line_count(),
        right.line_count(),
        base.format.text().lines_independent,
    );
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
    Ok(TextComparison {
        left: left.clone(),
        right: right.clone(),
        format: base.format.clone(),
        rules: base.rules.clone(),
        set: base.set.clone(),
        algorithm: base.algorithm,
        identity_path: base.identity_path,
        bytes_identical: job.bytes_identical,
        runs,
        run_rows,
        rows: store::RowStore::from_vec(rows),
        sections: derived.sections,
        class_runs: derived.class_runs,
        stats: derived.stats,
        warnings,
        start_states: [std::sync::OnceLock::new(), std::sync::OnceLock::new()],
        anchors: Vec::new(),
        provisional: false,
    })
}

/// The live data of the new result: fingerprints and checkpoints spliced, the regex total, the derivation.
fn build_keys(
    job: &RecompareJob,
    sides: &Sides<'_>,
    rekeyed: &[Vec<Rekeyed>; 2],
    seg: SegmentTrace,
) -> Result<LiveKeys, Fail> {
    let mut fingerprints = [Vec::new(), Vec::new()];
    let mut checkpoints: [Vec<(u32, LineState)>; 2] = [Vec::new(), Vec::new()];
    for side in 0..2 {
        let old_fps = &job.keys.fingerprints[side];
        let view = &sides.views[side];
        let n = if side == 0 {
            job.left.line_count()
        } else {
            job.right.line_count()
        };
        let fp = &mut fingerprints[side];
        fp.reserve_exact(n as usize);
        let mut old_at = 0usize;
        for r in &rekeyed[side] {
            fp.extend_from_slice(&old_fps[old_at..r.old.start as usize]);
            fp.extend_from_slice(&r.new_keys.fps);
            old_at = r.old.end as usize;
        }
        fp.extend_from_slice(&old_fps[old_at.min(old_fps.len())..]);
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
    let [fl, fr] = fingerprints;
    let [cl, cr] = checkpoints;
    Ok(LiveKeys {
        trace: Arc::new(Derivation {
            work: seg.work,
            segments: vec![seg],
        }),
        fingerprints: [Arc::new(fl), Arc::new(fr)],
        checkpoints: [Arc::new(cl), Arc::new(cr)],
        regex_errors,
    })
}
