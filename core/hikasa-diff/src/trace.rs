//! The derivation of an alignment (D-267, contract §1.10.9): [`align_traced`] returns the runs of
//! [`crate::align`] plus a record of why they look as they do — per anchor segment, every node of the
//! range-splitting engines (Standard, Patience) with its range, stripped prefix and suffix, stage, anchor chain,
//! extended blocks, children, the budget work it spent and its window slack — and the budget totals. The exact
//! incremental recompare of hikasa-text (§1.10.11) certifies nodes against this record and re-runs only the
//! first node whose certificate fails.
//!
//! Recording never changes a result: `align` and `align_traced` share one routine and give the same runs on
//! every input (A10.8). A range under [`RECORD_MIN`] lines on both sides is part of its parent's record.

use std::ops::Range;

use hikasa_core::CancelToken;
use serde::{Deserialize, Serialize};

use crate::anchors::{Anchor, run_segments};
use crate::error::DiffError;
use crate::input::{AlignInput, PairScorer};
use crate::model::{Alignment, RunSink};
use crate::options::{Algorithm, AlignOptions};

/// Index of a node in [`SegmentTrace::nodes`].
pub type NodeId = u32;

/// No node (the owner of a segment's root).
pub const NO_NODE: NodeId = u32::MAX;

/// Ranges with fewer lines than this on both sides are not recorded (re-running the parent's gap is cheaper
/// than storing it).
pub const RECORD_MIN: u32 = 64;

/// How a node split its range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Unique-on-both-sides anchors chained (Standard stage A, Patience, the fallback's unique split).
    AnchorsUnique,
    /// One best block (Standard stage B or C).
    Block,
    /// No split: the range is a gap, one side is empty, or an imara-diff engine aligned it.
    Flat,
}

/// One recorded node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceNode {
    /// The node's left range (after its parent's decisions, before stripping).
    pub left: Range<u32>,
    /// The node's right range.
    pub right: Range<u32>,
    /// Its common prefix.
    pub prefix: u32,
    /// Its common suffix.
    pub suffix: u32,
    /// How it split its range.
    pub stage: Stage,
    /// The anchor pairs of an `AnchorsUnique` node, in order (segment coordinates).
    pub chain: Vec<(u32, u32)>,
    /// The extended blocks `(x, y, len)` (segment coordinates).
    pub blocks: Vec<(u32, u32, u32)>,
    /// Its recorded children (gaps of at least [`RECORD_MIN`] lines a side), in order.
    pub children: Vec<NodeId>,
    /// Standard work units it spent, its unrecorded descendants included.
    pub work: u64,
    /// Closeness cells it spent, its unrecorded descendants included.
    pub cells: u64,
    /// Scorer units it spent, its unrecorded descendants included.
    pub scorer: u64,
    /// Gaps it (or an unrecorded descendant) paired by position because a closeness budget refused them.
    pub refusals: u64,
    /// §1.10.11 condition 3: how far the line counts may change before a unique pair's window membership can
    /// change (`u32::MAX`: no window).
    pub slack: u32,
}

impl TraceNode {
    fn new(left: Range<u32>, right: Range<u32>) -> TraceNode {
        TraceNode {
            left,
            right,
            prefix: 0,
            suffix: 0,
            stage: Stage::Flat,
            chain: Vec::new(),
            blocks: Vec::new(),
            children: Vec::new(),
            work: 0,
            cells: 0,
            scorer: 0,
            refusals: 0,
            slack: u32::MAX,
        }
    }

    /// The stripped range (without the common prefix and suffix).
    pub fn stripped(&self) -> (Range<u32>, Range<u32>) {
        (
            self.left.start + self.prefix..self.left.end - self.suffix,
            self.right.start + self.prefix..self.right.end - self.suffix,
        )
    }
}

/// The budget totals of an alignment (or of one segment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WorkTotals {
    /// Standard work units spent.
    pub standard: u64,
    /// Standard work budget.
    pub standard_budget: u64,
    /// Fallback cells spent.
    pub fallback: u64,
    /// Fallback budget.
    pub fallback_budget: u64,
    /// Closeness cells spent.
    pub closeness: u64,
    /// Closeness cell budget.
    pub closeness_budget: u64,
    /// Scorer units spent (`PairScorer::spent`).
    pub scorer: u64,
    /// True when a budget whose decisions depend on the order of the work refused something (the work budget
    /// ran out, the fallback degraded, the total closeness budget refused a gap).
    pub exhausted: bool,
}

impl WorkTotals {
    fn add(&mut self, other: &WorkTotals) {
        self.standard += other.standard;
        self.standard_budget += other.standard_budget;
        self.fallback += other.fallback;
        self.fallback_budget += other.fallback_budget;
        self.closeness += other.closeness;
        self.closeness_budget += other.closeness_budget;
        self.scorer += other.scorer;
        self.exhausted |= other.exhausted;
    }
}

/// The record of one anchor segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentTrace {
    /// Left lines of the segment.
    pub left: Range<u32>,
    /// Right lines of the segment.
    pub right: Range<u32>,
    /// The segment's common prefix.
    pub prefix: u32,
    /// The segment's common suffix (of what the prefix leaves).
    pub suffix: u32,
    /// The engine that aligned it (*Auto* resolved).
    pub engine: Algorithm,
    /// True for the segment of an anchored pair (`Anchor::Pair`).
    pub pair: bool,
    /// The root node (`None` for engines without nodes: Unaligned, Myers, Histogram, and 1:1 pairs).
    pub root: Option<NodeId>,
    /// Every recorded node; node coordinates are relative to the segment's start.
    pub nodes: Vec<TraceNode>,
    /// The segment's budget totals.
    pub work: WorkTotals,
}

/// The record of an alignment.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Derivation {
    /// One per anchor segment, in order.
    pub segments: Vec<SegmentTrace>,
    /// The totals of every segment.
    pub work: WorkTotals,
}

impl Derivation {
    /// Builds the totals from the segments.
    pub(crate) fn from_segments(segments: Vec<SegmentTrace>) -> Derivation {
        let mut work = WorkTotals::default();
        for s in &segments {
            work.add(&s.work);
        }
        Derivation { segments, work }
    }
}

/// The recorder of one engine run.
#[derive(Debug, Default)]
pub(crate) struct Recorder {
    pub(crate) nodes: Vec<TraceNode>,
    pub(crate) root: Option<NodeId>,
    pub(crate) totals: WorkTotals,
}

impl Recorder {
    /// Opens a node for `left × right` under `owner` (the parent's id, or [`NO_NODE`] for the root).
    pub(crate) fn open(&mut self, owner: NodeId, left: Range<u32>, right: Range<u32>) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(TraceNode::new(left, right));
        if let Some(parent) = self.nodes.get_mut(owner as usize) {
            parent.children.push(id);
        }
        if self.root.is_none() {
            self.root = Some(id);
        }
        id
    }

    /// Adds work to `node` (no-op for [`NO_NODE`]).
    pub(crate) fn charge(
        &mut self,
        node: NodeId,
        work: u64,
        cells: u64,
        scorer: u64,
        refusals: u64,
    ) {
        if let Some(n) = self.nodes.get_mut(node as usize) {
            n.work += work;
            n.cells += cells;
            n.scorer += scorer;
            n.refusals += refusals;
        }
    }

    /// The node `id`.
    pub(crate) fn node(&mut self, id: NodeId) -> Option<&mut TraceNode> {
        self.nodes.get_mut(id as usize)
    }
}

/// Whether a range is recorded as a node of its own.
#[inline]
pub(crate) fn recorded(owner: NodeId, a: &Range<u32>, b: &Range<u32>) -> bool {
    owner == NO_NODE || a.end - a.start >= RECORD_MIN || b.end - b.start >= RECORD_MIN
}

/// [`crate::align`] with anchors (§1.10.8; empty = none) and *Auto* (`auto`: each segment takes Histogram when
/// the cost of its stripped middle fits the fallback budget, else Standard, as hikasa-text's D-36), returning the
/// derivation too. The runs equal [`crate::align_anchored`]'s, and without anchors and `auto` they equal
/// [`crate::align`]'s, bit for bit.
pub fn align_traced(
    input: &AlignInput<'_>,
    opts: &AlignOptions,
    anchors: &[Anchor],
    auto: bool,
    scorer: Option<&dyn PairScorer>,
    sink: Option<&mut dyn RunSink>,
    cancel: &CancelToken,
) -> Result<(Alignment, Derivation), DiffError> {
    let (alignment, derivation, _) =
        run_segments(input, opts, anchors, auto, scorer, sink, cancel, true)?;
    Ok((alignment, derivation.unwrap_or_default()))
}
