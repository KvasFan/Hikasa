//! Standard alignment (§2.14.5, D-16): recursive best-anchor search with skew tolerance and line weights, under
//! a deterministic work budget.
//!
//! ```text
//! window(i, j) ⇔ |(i − a0) − (j − b0)| ≤ S  ∨  |(a1 − i) − (b1 − j)| ≤ S
//!
//! process(range):                                   // explicit stack, ranges in document order
//!   1. Equal(common prefix); remember the common suffix (emitted after the range)
//!   2. one side empty → Delete/Insert
//!   3. stage A: unique, non-trivial pairs in the window, score 1 + 4·max(w_a, w_b); maximum-score chain
//!      (weighted LIS); each chain pair extended backward and forward while equal, without crossing a neighbour
//!   4. no block → stage B: best maximal diagonal block starting at a non-trivial match in the window,
//!      score Σ over its non-trivial lines; best by (score ↓, min(|start disp|, |end disp|) ↑, i ↑, j ↑)
//!   5. no block → stage C: as B over trivial keys, score = block length
//!   6. no block → the range is a gap (§2.15)
//!   7. otherwise gap, block, gap, …, block, gap (gaps are processed recursively)
//! budget: candidates examined, extension steps and hash insertions cost 1 each; when it runs out (warning
//!   work_budget_exhausted), the current and every pending range go to the bounded fallback (`fallback.rs`):
//!   Histogram while the fallback budget lasts, else unique-line anchors and gap pairing (warning
//!   alignment_degraded)
//! ```
//!
//! Stage B and C skip a candidate `(i, j)` when `(i − 1, j − 1)` is itself a candidate of the same stage (a match
//! of the same triviality class), so that each maximal block is scored once, from its first eligible line.
//! Deviation from the literal §2.14.5 text ("skip if `(i − 1, j − 1)` is a match"): with that rule a block whose
//! first line is a matching *trivial* line (a blank line before repeated code) would never be scored by stage B,
//! because stage B starts only at non-trivial lines, and a worse block could win; e.g. left `["", x, y, e1]`,
//! right `[x, y, "", x, y, e2]` would align 2 lines instead of 3. In stage C both rules coincide (any match in
//! the window there is trivial, or stage B would have found a block).

use std::ops::Range;

use hikasa_core::WarningCode;
use imara_diff::Token;

use super::fallback::Fallback;
use super::lis::WeightedLis;
use super::{Ctx, SCAN_CHUNK, Task, checkpoint, common_run, common_run_back};
use crate::error::DiffError;
use crate::model::span;
use crate::options::{AlignOptions, WorkBudget};

/// Weight multiplier of the anchor score (§2.14.5).
const WEIGHT_FACTOR: u64 = 4;

/// Operations accumulated before they are charged to the budget inside the stage B/C scan.
const CHARGE_BATCH: u64 = 4096;

/// `prev_b` value of right position 0 (no predecessor). Local ids are below 2³¹, so it matches no key.
const NO_PREDECESSOR: u32 = u32::MAX;

/// Left-predecessor value that matches no `prev_b` entry (not even `NO_PREDECESSOR`).
const NO_MATCH: u32 = u32::MAX - 1;

/// Why the processing of a range stopped early.
enum Interrupt {
    /// The work budget ran out.
    Exhausted,
    /// Cancellation.
    Error(DiffError),
}

impl From<DiffError> for Interrupt {
    fn from(e: DiffError) -> Interrupt {
        Interrupt::Error(e)
    }
}

/// A range pair `(left, right)` still to align.
type RangePair = (Range<u32>, Range<u32>);

/// A diagonal block of equal keys, in range-local coordinates.
#[derive(Debug, Clone, Copy)]
struct Block {
    x: u32,
    y: u32,
    len: u32,
}

/// Reusable buffers (sized by the largest range seen; never by the number of distinct keys).
#[derive(Debug, Default)]
struct Scratch {
    /// Range-local dense ids of the left and right keys.
    la: Vec<Token>,
    lb: Vec<Token>,
    /// Occurrences per local id, and the position of a key's (last) occurrence on the right.
    cnt_a: Vec<u32>,
    cnt_b: Vec<u32>,
    pos_b: Vec<u32>,
    /// Stage A candidates (sorted by x).
    u_x: Vec<u32>,
    u_y: Vec<u32>,
    u_score: Vec<u64>,
    lis: WeightedLis,
    chain: Vec<u32>,
    blocks: Vec<Block>,
    /// Right positions grouped by local id (counting sort): `b_list[b_start[t]..b_start[t + 1]]`, ascending.
    b_start: Vec<u32>,
    b_list: Vec<u32>,
    /// Local id of the right predecessor of every right position (`NO_PREDECESSOR` at position 0).
    prev_b: Vec<u32>,
    /// Stage B/C: whether each left line of the range belongs to the stage's class.
    in_stage: Vec<bool>,
    /// Stage B/C: the block starts among the candidates of one interval.
    starts: Vec<u32>,
    tasks: Vec<Task>,
}

/// One Standard alignment.
struct Standard {
    budget: WorkBudget,
    s: Scratch,
}

pub(crate) fn run(cx: &mut Ctx<'_, '_>) -> Result<(), DiffError> {
    let (n, m) = (cx.left.len() as u32, cx.right.len() as u32);
    let limit = cx.opts.work_budget_for(n as usize, m as usize);
    let mut standard = Standard {
        budget: WorkBudget::new(limit),
        s: Scratch::default(),
    };
    let fallback_budget = cx
        .fallback_budget
        .unwrap_or_else(|| AlignOptions::fallback_budget_for(n as usize, m as usize));
    let mut fallback = Fallback::new(fallback_budget);
    let mut exhausted = false;
    let mut stack = vec![Task::Range(0..n, 0..m)];
    while let Some(task) = stack.pop() {
        match task {
            Task::Equal(len) => cx.out.equal(len),
            Task::Range(a, b) if exhausted => fallback.range(cx, a, b, &mut stack)?,
            Task::Range(a, b) => {
                if let Some((a, b)) = standard.process(cx, a, b, &mut stack)? {
                    exhausted = true;
                    cx.warn(WarningCode::WorkBudgetExhausted, || {
                        format!(
                            "the Standard alignment work budget of {limit} operations ran out; the remaining \
                             ranges were aligned with Histogram within the fallback budget"
                        )
                    });
                    fallback.range(cx, a, b, &mut stack)?;
                }
            }
            Task::Residual(a, b) => fallback.residual(cx, a, b)?,
        }
        cx.flush_ready();
    }
    Ok(())
}

/// Spends `cost` operations of `budget` (and counts them as steps for cancellation).
fn charge(budget: &mut WorkBudget, cx: &mut Ctx<'_, '_>, cost: u64) -> Result<(), Interrupt> {
    if !budget.charge(cost) {
        return Err(Interrupt::Exhausted);
    }
    cx.tick(cost)?;
    Ok(())
}

/// The window of §2.14.5 in range-local coordinates (`na`, `nb` are the range lengths).
#[inline]
fn in_window(x: u32, y: u32, na: u32, nb: u32, skew: u32) -> bool {
    let start = (i64::from(x) - i64::from(y)).unsigned_abs();
    let end = (i64::from(na - x) - i64::from(nb - y)).unsigned_abs();
    start <= u64::from(skew) || end <= u64::from(skew)
}

/// min(|start displacement|, |end displacement|) of a block starting at `(x, y)`.
#[inline]
fn displacement(x: u32, y: u32, na: u32, nb: u32) -> u64 {
    let start = (i64::from(x) - i64::from(y)).unsigned_abs();
    let end = (i64::from(na - x) - i64::from(nb - y)).unsigned_abs();
    start.min(end)
}

impl Standard {
    /// Processes one range (steps 1–7). `Ok(Some(core))` when the budget ran out: `core` (the range without its
    /// common prefix and suffix) is still unaligned and nothing of it was emitted.
    fn process(
        &mut self,
        cx: &mut Ctx<'_, '_>,
        a: Range<u32>,
        b: Range<u32>,
        stack: &mut Vec<Task>,
    ) -> Result<Option<RangePair>, DiffError> {
        let prefix = cx.common_prefix(&a, &b)?;
        cx.out.equal(prefix);
        let (a, b) = (a.start + prefix..a.end, b.start + prefix..b.end);
        let suffix = cx.common_suffix(&a, &b)?;
        if suffix > 0 {
            stack.push(Task::Equal(suffix));
        }
        let (a, b) = (a.start..a.end - suffix, b.start..b.end - suffix);
        if a.is_empty() || b.is_empty() {
            cx.out.delete(span(&a));
            cx.out.insert(span(&b));
            return Ok(None);
        }
        match self.find_blocks(cx, &a, &b) {
            Ok(true) => {
                self.push_tasks(&a, &b, stack);
                Ok(None)
            }
            Ok(false) => {
                cx.gap(a, b)?;
                Ok(None)
            }
            Err(Interrupt::Exhausted) => Ok(Some((a, b))),
            Err(Interrupt::Error(e)) => Err(e),
        }
    }

    /// Stages A, B and C on a stripped range with both sides non-empty; true when `s.blocks` holds blocks.
    fn find_blocks(
        &mut self,
        cx: &mut Ctx<'_, '_>,
        a: &Range<u32>,
        b: &Range<u32>,
    ) -> Result<bool, Interrupt> {
        let (left, right) = (cx.left, cx.right);
        let ta = &left[a.start as usize..a.end as usize];
        let tb = &right[b.start as usize..b.end as usize];
        // Hash insertions: every token of the range is remapped once.
        charge(&mut self.budget, cx, (ta.len() + tb.len()) as u64)?;
        let k = cx
            .remap
            .remap_cancellable(ta, tb, &mut self.s.la, &mut self.s.lb, cx.cancel)?;
        if self.stage_a(cx, a, b, k)? {
            return Ok(true);
        }
        self.index_right(cx, k)?;
        for trivial_stage in [false, true] {
            if let Some(block) = self.best_block(cx, a, b, trivial_stage)? {
                self.s.blocks.clear();
                self.s.blocks.push(block);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Stage A: the weighted chain of unique anchors in the window, extended to blocks.
    fn stage_a(
        &mut self,
        cx: &mut Ctx<'_, '_>,
        a: &Range<u32>,
        b: &Range<u32>,
        k: u32,
    ) -> Result<bool, Interrupt> {
        let s = &mut self.s;
        let (na, nb) = (span(a), span(b));
        let ku = k as usize;
        s.cnt_a.clear();
        s.cnt_a.resize(ku, 0);
        s.cnt_b.clear();
        s.cnt_b.resize(ku, 0);
        s.pos_b.clear();
        s.pos_b.resize(ku, 0);
        for (x, t) in s.la.iter().enumerate() {
            checkpoint(cx.cancel, x)?;
            s.cnt_a[t.0 as usize] += 1;
        }
        for (y, t) in s.lb.iter().enumerate() {
            checkpoint(cx.cancel, y)?;
            s.cnt_b[t.0 as usize] += 1;
            s.pos_b[t.0 as usize] = y as u32;
        }
        s.u_x.clear();
        s.u_y.clear();
        s.u_score.clear();
        let skew = cx.opts.skew_tolerance;
        let mut examined = 0u64;
        for (x, t) in s.la.iter().enumerate() {
            checkpoint(cx.cancel, x)?;
            let t = t.0 as usize;
            if s.cnt_a[t] != 1 || s.cnt_b[t] != 1 {
                continue;
            }
            let (x, i) = (x as u32, a.start + x as u32);
            if cx.trivial.contains(cx.left[i as usize]) {
                continue;
            }
            examined += 1;
            let y = s.pos_b[t];
            if !in_window(x, y, na, nb, skew) {
                continue;
            }
            let weight = cx.left_weight(i).max(cx.right_weight(b.start + y));
            s.u_x.push(x);
            s.u_y.push(y);
            s.u_score.push(1 + WEIGHT_FACTOR * weight);
        }
        charge(&mut self.budget, cx, examined)?;
        if s.u_x.is_empty() {
            return Ok(false);
        }
        s.lis
            .chain(&s.u_y, &s.u_score, nb, &mut s.chain, cx.cancel)?;
        let (left, right) = (cx.left, cx.right);
        let ta = &left[a.start as usize..a.end as usize];
        let tb = &right[b.start as usize..b.end as usize];
        s.blocks.clear();
        let (mut px, mut py) = (0u32, 0u32);
        for c in 0..s.chain.len() {
            let u = s.chain[c] as usize;
            let (x0, y0) = (s.u_x[u], s.u_y[u]);
            let (nx, ny) = match s.chain.get(c + 1) {
                Some(&v) => (s.u_x[v as usize], s.u_y[v as usize]),
                None => (na, nb),
            };
            // Extend backward and forward while equal, without crossing the previous block or the next anchor.
            let back = common_run_back(
                &ta[px as usize..x0 as usize],
                &tb[py as usize..y0 as usize],
                cx.cancel,
            )? as u32;
            let forward = common_run(
                &ta[x0 as usize + 1..nx as usize],
                &tb[y0 as usize + 1..ny as usize],
                cx.cancel,
            )? as u32;
            let (x, y) = (x0 - back, y0 - back);
            let (xe, ye) = (x0 + 1 + forward, y0 + 1 + forward);
            charge(
                &mut self.budget,
                cx,
                u64::from(x0 - x) + u64::from(xe - x0 - 1),
            )?;
            match s.blocks.last_mut() {
                Some(last) if last.x + last.len == x && last.y + last.len == y => {
                    last.len += xe - x;
                }
                _ => s.blocks.push(Block { x, y, len: xe - x }),
            }
            (px, py) = (xe, ye);
        }
        Ok(true)
    }

    /// Groups the right positions of the range by local id (counting sort, ascending positions).
    fn index_right(&mut self, cx: &mut Ctx<'_, '_>, k: u32) -> Result<(), Interrupt> {
        let s = &mut self.s;
        let ku = k as usize;
        s.b_start.clear();
        s.b_start.resize(ku + 1, 0);
        for (y, t) in s.lb.iter().enumerate() {
            checkpoint(cx.cancel, y)?;
            s.b_start[t.0 as usize + 1] += 1;
        }
        for t in 0..ku {
            checkpoint(cx.cancel, t)?;
            s.b_start[t + 1] += s.b_start[t];
        }
        // `cnt_b` becomes the write cursor of each group.
        s.cnt_b.clear();
        s.cnt_b.extend_from_slice(&s.b_start[..ku]);
        s.b_list.clear();
        s.b_list.resize(s.lb.len(), 0);
        for (y, t) in s.lb.iter().enumerate() {
            checkpoint(cx.cancel, y)?;
            let cursor = &mut s.cnt_b[t.0 as usize];
            s.b_list[*cursor as usize] = y as u32;
            *cursor += 1;
        }
        s.prev_b.clear();
        s.prev_b.push(NO_PREDECESSOR);
        for chunk in s.lb[..s.lb.len().saturating_sub(1)].chunks(SCAN_CHUNK) {
            cx.cancel.check().map_err(DiffError::from)?;
            s.prev_b.extend(chunk.iter().map(|t| t.0));
        }
        charge(&mut self.budget, cx, s.lb.len() as u64)
    }

    /// Stage B (`trivial_stage == false`) or C (`true`): the best maximal block in the window that starts at a
    /// match of the stage's class.
    fn best_block(
        &mut self,
        cx: &mut Ctx<'_, '_>,
        a: &Range<u32>,
        b: &Range<u32>,
        trivial_stage: bool,
    ) -> Result<Option<Block>, Interrupt> {
        let (na, nb) = (span(a), span(b));
        let skew = i64::from(cx.opts.skew_tolerance);
        let shift = i64::from(nb) - i64::from(na);
        let weighted = cx.has_weights();
        let s = &mut self.s;
        // Range-local keys (equal local ids ⇔ equal keys) and the stage class of every left line.
        s.in_stage.clear();
        for chunk in cx.left[a.start as usize..a.end as usize].chunks(SCAN_CHUNK) {
            cx.cancel.check().map_err(DiffError::from)?;
            s.in_stage.extend(
                chunk
                    .iter()
                    .map(|&key| cx.trivial.contains(key) == trivial_stage),
            );
        }
        let (la, lb, in_stage) = (&s.la, &s.lb, &s.in_stage);
        let mut best: Option<(Block, u64, u64)> = None;
        let mut pending = 0u64;
        for x in 0..na {
            let xu = x as usize;
            checkpoint(cx.cancel, xu)?;
            if !in_stage[xu] {
                continue;
            }
            let t = la[xu].0 as usize;
            let list = &s.b_list[s.b_start[t] as usize..s.b_start[t + 1] as usize];
            if list.is_empty() {
                continue;
            }
            // (x−1, y−1) is a candidate of this stage, so (x, y) is not a block start, iff the left predecessor
            // is in the stage and the right predecessor (`prev_b[y]`) has the same key.
            let predecessor = if x > 0 && in_stage[xu - 1] {
                la[xu - 1].0
            } else {
                NO_MATCH
            };
            // The window as two y-intervals (start diagonal, end diagonal), clipped and merged when they meet.
            let xi = i64::from(x);
            let clip = |(lo, hi): (i64, i64)| (lo.max(0), hi.min(i64::from(nb) - 1));
            let (mut first, mut second) = (
                clip((xi - skew, xi + skew)),
                clip((xi + shift - skew, xi + shift + skew)),
            );
            if second.0 < first.0 {
                std::mem::swap(&mut first, &mut second);
            }
            if second.0 <= first.1.saturating_add(1) {
                first.1 = first.1.max(second.1);
                second = (1, 0);
            }
            for (lo, hi) in [first, second] {
                if lo > hi {
                    continue;
                }
                let from = list.partition_point(|&y| i64::from(y) < lo);
                let to = list.partition_point(|&y| i64::from(y) <= hi);
                let candidates = &list[from..to];
                pending += candidates.len() as u64;
                // Branch-free compaction of the block starts (the predecessor test is data-dependent and would
                // be mispredicted half of the time on repetitive input).
                s.starts.clear();
                s.starts.resize(candidates.len(), 0);
                let mut count = 0;
                for &y in candidates {
                    s.starts[count] = y;
                    count += usize::from(s.prev_b[y as usize] != predecessor);
                }
                for &y in &s.starts[..count] {
                    let yu = y as usize;
                    let len = common_run(&la[xu..], &lb[yu..], cx.cancel)?;
                    let score = if trivial_stage {
                        len as u64
                    } else if weighted {
                        (0..len)
                            .filter(|&d| in_stage[xu + d])
                            .map(|d| {
                                let w = cx
                                    .left_weight(a.start + x + d as u32)
                                    .max(cx.right_weight(b.start + y + d as u32));
                                1 + WEIGHT_FACTOR * w
                            })
                            .sum()
                    } else {
                        in_stage[xu..xu + len].iter().filter(|&&v| v).count() as u64
                    };
                    pending += len as u64;
                    let disp = displacement(x, y, na, nb);
                    let better = match best {
                        None => true,
                        Some((_, best_score, best_disp)) => {
                            score > best_score || (score == best_score && disp < best_disp)
                        }
                    };
                    if better {
                        best = Some((
                            Block {
                                x,
                                y,
                                len: len as u32,
                            },
                            score,
                            disp,
                        ));
                    }
                    if pending >= CHARGE_BATCH {
                        charge(&mut self.budget, cx, pending)?;
                        pending = 0;
                    }
                }
            }
        }
        charge(&mut self.budget, cx, pending)?;
        Ok(best.map(|(block, _, _)| block))
    }

    /// Pushes gap, block, gap, …, block, gap (document order) onto the stack, last piece first.
    fn push_tasks(&mut self, a: &Range<u32>, b: &Range<u32>, stack: &mut Vec<Task>) {
        let s = &mut self.s;
        s.tasks.clear();
        let (mut x0, mut y0) = (0u32, 0u32);
        for block in &s.blocks {
            if block.x > x0 || block.y > y0 {
                s.tasks.push(Task::Range(
                    a.start + x0..a.start + block.x,
                    b.start + y0..b.start + block.y,
                ));
            }
            s.tasks.push(Task::Equal(block.len));
            (x0, y0) = (block.x + block.len, block.y + block.len);
        }
        if a.start + x0 < a.end || b.start + y0 < b.end {
            s.tasks
                .push(Task::Range(a.start + x0..a.end, b.start + y0..b.end));
        }
        stack.extend(s.tasks.drain(..).rev());
    }
}
