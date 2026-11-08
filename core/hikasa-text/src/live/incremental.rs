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
//!           3 window    |Δ left| + |Δ right| below the node's slack
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
        added_slack: u32::MAX,
        weights,
        threshold: a.closeness_threshold_permille,
    };
    let root_node = &seg.nodes[root as usize];
    let la = 0..n;
    let ra = 0..m;
    // Auto at the segment level: the recorded engine on the new sizes.
    if auto {
        let p = root_node.prefix;
        let s = root_node.suffix;
        let cells = u64::from(n.saturating_sub(p + s)) * u64::from(m.saturating_sub(p + s));
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

/// A dirty key: its dirty counts per side (old lines, new lines) and the old lines that had it.
#[derive(Debug, Clone, Default)]
struct KeyInfo {
    dirty: [(i64, i64); 2],
    old_lines: [Vec<u32>; 2],
}

/// The window slack of a unique pair (as hikasa-diff records it, §1.10.11 condition 3).
fn pair_slack(x: u32, y: u32, na: u32, nb: u32, skew: u32) -> u32 {
    if skew == u32::MAX {
        return u32::MAX;
    }
    let s = i64::from(skew);
    let ds = (i64::from(x) - i64::from(y)).abs();
    let de = (i64::from(na) - i64::from(x) - (i64::from(nb) - i64::from(y))).abs();
    let slack = if ds <= s || de <= s {
        let margin = |d: i64| if d <= s { s - d + 1 } else { 0 };
        margin(ds).max(margin(de))
    } else {
        (ds - s).min(de - s)
    };
    slack.clamp(0, i64::from(u32::MAX - 1)) as u32
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
    /// The smallest window slack of the unique pairs the last certified node gained.
    added_slack: u32,
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

    /// The alignment-dirty spans of `side` touching the old range `old`.
    fn spans_in(&self, side: usize, old: &Range<u32>) -> Vec<(Range<u32>, Range<u32>)> {
        self.sides.adirty[side]
            .iter()
            .filter(|(o, _)| {
                if o.is_empty() {
                    old.start <= o.start && o.start <= old.end
                } else {
                    o.start < old.end && old.start < o.end
                }
            })
            .cloned()
            .collect()
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

    /// Processes recorded node `id` whose new ranges are `la × ra`: copies a clean node, certifies and descends,
    /// or re-runs it. Returns the new node's id.
    fn node(
        &mut self,
        id: NodeId,
        la: Range<u32>,
        ra: Range<u32>,
        parent: NodeId,
    ) -> Result<NodeId, Fail> {
        self.cancel.check().map_err(|_| Fail::Cancelled)?;
        let old = &self.seg.nodes[id as usize];
        if !self.dirty_in(0, &old.left) && !self.dirty_in(1, &old.right) {
            self.emit_old(
                (old.left.start, old.right.start),
                (old.left.end, old.right.end),
            );
            return Ok(self.copy_subtree(id, parent));
        }
        match self.certify(id, &la, &ra)? {
            Some(chain) => self.descend(id, la, ra, parent, chain),
            None => self.rerun(la, ra, parent),
        }
    }

    /// The certificate of node `id` (conditions 1–3).
    fn certify(
        &mut self,
        id: NodeId,
        la: &Range<u32>,
        ra: &Range<u32>,
    ) -> Result<Option<Vec<(u32, u32)>>, Fail> {
        let old = &self.seg.nodes[id as usize];
        let (p, s) = (old.prefix, old.suffix);
        let (nl, nr) = (la.end - la.start, ra.end - ra.start);
        // 1. Range: no dirty line in the prefix or suffix, the boundary pairs still differ, the middle non-empty.
        let [lv, rv] = &self.sides.views;
        let clean = |v: &SideView<'_>, lines: Range<u32>, side: usize, me: &Self| -> bool {
            let _ = v;
            !me.sides.adirty[side].iter().any(|(_, n)| {
                let n = if n.is_empty() {
                    n.start.saturating_sub(1)..n.start + 1
                } else {
                    n.clone()
                };
                n.start < lines.end && lines.start < n.end
            })
        };
        let range_ok = p + s < nl.min(nr)
            && clean(lv, la.start..la.start + p, 0, self)
            && clean(rv, ra.start..ra.start + p, 1, self)
            && clean(lv, la.end - s..la.end, 0, self)
            && clean(rv, ra.end - s..ra.end, 1, self)
            && !self.equal(la.start + p, ra.start + p)?
            && !self.equal(la.end - s - 1, ra.end - s - 1)?;
        if !range_ok {
            self.path.range_failed += 1;
            return Ok(None);
        }
        self.path.range_held += 1;
        // 2. Candidates.
        let chain = if old.stage == Stage::AnchorsUnique {
            self.new_chain(id, la, ra)?
        } else {
            None
        };
        let Some(chain) = chain else {
            self.path.candidates_failed += 1;
            return Ok(None);
        };
        self.path.candidates_held += 1;
        // 3. Window: the line counts change at several places, so a unique pair's displacement from the window's
        // start diagonal changes by at most the largest cumulative shift before it, and from the end diagonal
        // by the largest shift after it; both sides add up.
        let (a_l, b_l) = self.shift_extremes(0, &old.left);
        let (a_r, b_r) = self.shift_extremes(1, &old.right);
        if (a_l + a_r).max(b_l + b_r) >= u64::from(old.slack) {
            self.path.window_failed += 1;
            return Ok(None);
        }
        self.path.window_held += 1;
        Ok(Some(chain))
    }

    /// The largest |cumulative line-count change| before any line of the old range `old` on `side` (A), and the
    /// largest |change after it| (B), over the alignment-dirty spans inside the range.
    fn shift_extremes(&self, side: usize, old: &Range<u32>) -> (u64, u64) {
        let deltas: Vec<i64> = self
            .spans_in(side, old)
            .iter()
            .map(|(o, n)| i64::from(n.end - n.start) - i64::from(o.end - o.start))
            .collect();
        let total: i64 = deltas.iter().sum();
        let (mut cum, mut a, mut b) = (0i64, 0u64, total.unsigned_abs());
        for d in deltas {
            cum += d;
            a = a.max(cum.unsigned_abs());
            b = b.max((total - cum).unsigned_abs());
        }
        (a, b)
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
        la: &Range<u32>,
        ra: &Range<u32>,
    ) -> Result<Option<Vec<(u32, u32)>>, Fail> {
        let old = &self.seg.nodes[id as usize];
        let (p, s) = (old.prefix, old.suffix);
        let middle = [la.start + p..la.end - s, ra.start + p..ra.end - s];
        let old_middle = [
            old.left.start + p..old.left.end - s,
            old.right.start + p..old.right.end - s,
        ];
        // The keys of the dirty lines (old and new) with their dirty counts per side and the old lines that
        // had them.
        let mut keys: HashMap<Vec<u8>, KeyInfo> = HashMap::new();
        for (side, old_range) in old_middle.iter().enumerate() {
            for (o, n) in self.spans_in(side, old_range) {
                let span = self.sides.views[side].spans.iter().find(|r| {
                    r.old.start <= o.start
                        && o.end <= r.old.end
                        && r.new.start <= n.start
                        && n.end <= r.new.end
                });
                let Some(span) = span else {
                    return Ok(None);
                };
                for x in o.clone() {
                    let i = (x - span.old.start) as usize;
                    if span.old_keys.fps[i] != 0 {
                        let info = keys.entry(span.old_keys.keys[i].clone()).or_default();
                        info.dirty[side].0 += 1;
                        info.old_lines[side].push(x);
                    }
                }
                for x in n.clone() {
                    let i = (x - span.new.start) as usize;
                    if span.new_keys.fps[i] != 0 {
                        keys.entry(span.new_keys.keys[i].clone()).or_default().dirty[side].1 += 1;
                    }
                }
            }
        }
        let [lv, rv] = &self.sides.views;
        let mapped = |chain: &[(u32, u32)],
                      drop: &std::collections::HashSet<u32>|
         -> Option<Vec<(u32, u32)>> {
            chain
                .iter()
                .filter(|(x, _)| !drop.contains(x))
                .map(|&(x, y)| Some((lv.new_of(x)?, rv.new_of(y)?)))
                .collect()
        };
        if keys.is_empty() {
            return Ok(mapped(&old.chain, &Default::default()));
        }
        // Count every key in the new middles: fingerprint walk, verification of the hits.
        let list: Vec<(Vec<u8>, KeyInfo)> = keys.into_iter().collect();
        let mut fps: HashMap<u16, Vec<usize>> = HashMap::new();
        let mut bitmap = vec![0u64; 1024];
        for (k, (key, _)) in list.iter().enumerate() {
            let fp = crate::compare::key_fingerprint(key, self.shape, self.cancel)?;
            fps.entry(fp).or_default().push(k);
            bitmap[usize::from(fp) / 64] |= 1 << (fp % 64);
        }
        let mut counts = vec![[0i64; 2]; list.len()];
        let mut hits: Vec<[Vec<u32>; 2]> = vec![[Vec::new(), Vec::new()]; list.len()];
        let mut verified = 0u32;
        for side in 0..2 {
            let view = &self.sides.views[side];
            let cap: Vec<i64> = list
                .iter()
                .map(|(_, c)| 3 + c.dirty[side].0.max(c.dirty[side].1))
                .collect();
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
                if ks.iter().all(|&k| counts[k][side] >= cap[k]) {
                    continue;
                }
                verified += 1;
                if verified > VERIFY_LIMIT {
                    return Ok(None);
                }
                let (key, _) = view.key(x)?;
                for &k in ks {
                    if *key == *list[k].0 && counts[k][side] < cap[k] {
                        counts[k][side] += 1;
                        hits[k][side].push(x);
                    }
                }
            }
        }
        let all = old.chain.len() as u32 == old.candidates && old.candidates > 0;
        self.added_slack = u32::MAX;
        let (na, nb) = (
            middle[0].end - middle[0].start,
            middle[1].end - middle[1].start,
        );
        let mut removed: std::collections::HashSet<u32> = Default::default();
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
            if was {
                // The candidate's old left line: the dirty old line with the key, else the clean hit.
                let x = match info.old_lines[0].first() {
                    Some(&x) => Some(x),
                    None => hits[k][0].iter().find_map(|&h| lv.old_of(h)),
                };
                let Some(x) = x else {
                    return Ok(None);
                };
                removed.insert(x);
            }
            if is {
                let (x, y) = (hits[k][0][0], hits[k][1][0]);
                if self.opts.algorithm == Algorithm::Standard {
                    self.added_slack = self.added_slack.min(pair_slack(
                        x - middle[0].start,
                        y - middle[1].start,
                        na,
                        nb,
                        self.opts.skew_tolerance,
                    ));
                }
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
            }
        }
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
        chain: Vec<(u32, u32)>,
    ) -> Result<NodeId, Fail> {
        let old = self.seg.nodes[id as usize].clone();
        let (p, s) = (old.prefix, old.suffix);
        let (ma, na) = (la.start + p, ra.start + p);
        let (mb, nb) = (la.end - s, ra.end - s);
        let blocks = if self.opts.algorithm == Algorithm::Standard {
            self.extend_blocks(&old, &chain, (ma, na), (mb, nb))?
        } else {
            chain.iter().map(|&(x, y)| (x, y, 1)).collect()
        };
        // Growth of the node's own work: the remap of its middle (Standard charges its two lengths).
        if self.opts.algorithm == Algorithm::Standard {
            let grow = (i64::from(mb - ma) + i64::from(nb - na))
                - (i64::from(old.left.end - old.left.start - p - s)
                    + i64::from(old.right.end - old.right.start - p - s));
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
                slack: {
                    // Every pair's margin shrank by at most the shifts this edit made.
                    let (a_l, b_l) = self.shift_extremes(0, &old.left);
                    let (a_r, b_r) = self.shift_extremes(1, &old.right);
                    let used = (a_l + a_r + b_l + b_r).min(u64::from(u32::MAX)) as u32;
                    old.slack.saturating_sub(used).min(self.added_slack)
                },
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

    /// One gap of a certified node (new ranges): the old runs when it is an old gap without dirty lines, the
    /// recorded child when it has the child's range, else a re-run.
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
        // The old gap with the same new range.
        let same = old_gaps.iter().find(|(oa, ob)| {
            lv.pos(oa.start) == a.start
                && lv.pos(oa.end) == a.end
                && rv.pos(ob.start) == b.start
                && rv.pos(ob.end) == b.end
        });
        if let Some((oa, ob)) = same.cloned() {
            let child = old.children.iter().copied().find(|&c| {
                let n = &self.seg.nodes[c as usize];
                n.left == oa && n.right == ob
            });
            if !self.dirty_in(0, &oa) && !self.dirty_in(1, &ob) {
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
        let mut blocks: Vec<(u32, u32, u32)> = Vec::with_capacity(old.blocks.len() + 2);
        let (mut px, mut py) = start;
        let mut steps = 0u64;
        for c in 0..chain.len() {
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
