//! Standard's bounded fallback (§2.14.5; D-16 as amended in wave 2): what happens to the ranges that are still
//! pending when the Standard work budget runs out.
//!
//! ```text
//! range(A, B):                                  // every range pending when the work budget ran out
//!   Equal(common prefix); remember the common suffix (emitted after the range)
//!   cells = |A|·|B|                             // estimated cost of a Histogram call (0 if a side is empty)
//!   cells ≤ remaining fallback budget → remaining −= cells; Histogram through imara_call (§2.14.3)
//!   else → warning alignment_degraded; split(A, B)
//! split(A, B):                                  // O((|A| + |B|) log(|A| + |B|)), one level, no recursion
//!   U = pairs (x, y) whose key occurs exactly once in A and once in B and is not trivial
//!   U empty → the range is a gap (§2.15)
//!   chain = longest increasing subsequence of U by y (patience sorting)
//!   each chain pair extended backward and forward while equal, without crossing a neighbour → blocks
//!   gap, block, …, block, gap; every gap is a residual
//! residual(G):                                  // never split again
//!   strip; |G_A|·|G_B| ≤ remaining → remaining −= cells; Histogram; else the gap is resolved per §2.15
//!   (never-align → Delete/Insert; closeness DP within its own cell budgets; else positional)
//! ```
//!
//! Every cost is counted in operations, never in time (T6), so the result is a deterministic function of the
//! input: the ranges are visited in document order and the budget is spent in that order. The fallback budget
//! (`AlignOptions::fallback_budget_for`, 128·(n+m) + 32,000,000 cells) bounds the total Histogram work; the
//! split is O(n log n); the gap pairing is positional (O(n)) or bounded by the closeness cell budgets. Standard
//! therefore runs in O((n+m) log(n+m)) plus bounded terms on every input. Explicitly chosen Histogram, Myers and
//! Patience stay exact and unbounded (their adversarial worst case is super-linear; they remain cancellable).

use std::ops::Range;

use imara_diff::{Algorithm as ImaraAlgorithm, Token};

use super::lis::patience_chain;
use super::{Ctx, Task, checkpoint, common_run, common_run_back, lcs};
use crate::error::DiffError;
use crate::model::{DiffWarningCode, span};

/// Reusable buffers of the degraded split.
#[derive(Debug, Default)]
struct Scratch {
    la: Vec<Token>,
    lb: Vec<Token>,
    cnt_a: Vec<u32>,
    cnt_b: Vec<u32>,
    pos_b: Vec<u32>,
    u_x: Vec<u32>,
    u_y: Vec<u32>,
    tails: Vec<u32>,
    pred: Vec<u32>,
    chain: Vec<u32>,
    tasks: Vec<Task>,
}

/// The fallback of one Standard alignment.
pub(crate) struct Fallback {
    /// The fallback budget (cells).
    limit: u64,
    /// Cells still available for Histogram calls.
    remaining: u64,
    s: Scratch,
}

impl Fallback {
    /// A fallback with `limit` cells.
    pub(crate) fn new(limit: u64) -> Fallback {
        Fallback {
            limit,
            remaining: limit,
            s: Scratch::default(),
        }
    }

    /// Aligns a range that was pending when the work budget ran out: Histogram while it fits the fallback
    /// budget, else the degraded split (whose pieces are pushed onto `stack`).
    pub(crate) fn range(
        &mut self,
        cx: &mut Ctx<'_, '_>,
        a: Range<u32>,
        b: Range<u32>,
        stack: &mut Vec<Task>,
    ) -> Result<(), DiffError> {
        let (a, b, suffix) = strip(cx, a, b)?;
        if self.spend(&a, &b) {
            lcs::range(cx, ImaraAlgorithm::Histogram, a, b)?;
            cx.out.equal(suffix);
            return Ok(());
        }
        let limit = self.limit;
        cx.diff_warn(DiffWarningCode::AlignmentDegraded, || {
            format!(
                "the Standard alignment work budget and its fallback budget of {limit} cells ran out; some \
                 ranges were aligned at lines that are unique on both sides, and their gaps were paired by \
                 position or similarity"
            )
        });
        if suffix > 0 {
            stack.push(Task::Equal(suffix));
        }
        self.split(cx, a, b, stack)
    }

    /// Aligns a gap of the degraded split: Histogram while it fits the fallback budget, else gap pairing.
    pub(crate) fn residual(
        &mut self,
        cx: &mut Ctx<'_, '_>,
        a: Range<u32>,
        b: Range<u32>,
    ) -> Result<(), DiffError> {
        let (a, b, suffix) = strip(cx, a, b)?;
        if self.spend(&a, &b) {
            lcs::range(cx, ImaraAlgorithm::Histogram, a, b)?;
        } else {
            cx.gap(a, b)?;
        }
        cx.out.equal(suffix);
        Ok(())
    }

    /// Spends the estimated cost `|A|·|B|` of a Histogram call on the stripped range; false (nothing spent)
    /// when it does not fit.
    fn spend(&mut self, a: &Range<u32>, b: &Range<u32>) -> bool {
        let cells = u64::from(span(a)) * u64::from(span(b));
        if cells <= self.remaining {
            self.remaining -= cells;
            true
        } else {
            false
        }
    }

    /// The degraded split of a stripped range with both sides non-empty: unique-line anchors chained by
    /// patience sorting, extended to blocks; the pieces (residual gaps and Equal blocks) are pushed onto
    /// `stack` in document order.
    fn split(
        &mut self,
        cx: &mut Ctx<'_, '_>,
        a: Range<u32>,
        b: Range<u32>,
        stack: &mut Vec<Task>,
    ) -> Result<(), DiffError> {
        let s = &mut self.s;
        let (left, right) = (cx.left, cx.right);
        let ta = &left[a.start as usize..a.end as usize];
        let tb = &right[b.start as usize..b.end as usize];
        cx.tick((ta.len() + tb.len()) as u64)?;
        let k = cx
            .remap
            .remap_cancellable(ta, tb, &mut s.la, &mut s.lb, cx.cancel)? as usize;
        s.cnt_a.clear();
        s.cnt_a.resize(k, 0);
        s.cnt_b.clear();
        s.cnt_b.resize(k, 0);
        s.pos_b.clear();
        s.pos_b.resize(k, 0);
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
        for (x, t) in s.la.iter().enumerate() {
            checkpoint(cx.cancel, x)?;
            let t = t.0 as usize;
            if s.cnt_a[t] == 1 && s.cnt_b[t] == 1 && !cx.trivial.contains(ta[x]) {
                s.u_x.push(x as u32);
                s.u_y.push(s.pos_b[t]);
            }
        }
        if s.u_x.is_empty() {
            return cx.gap(a, b);
        }
        patience_chain(&s.u_y, &mut s.tails, &mut s.pred, &mut s.chain, cx.cancel)?;
        cx.tick(s.u_x.len() as u64)?;
        let (na, nb) = (span(&a), span(&b));
        s.tasks.clear();
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
            if x > px || y > py {
                s.tasks.push(Task::Residual(
                    a.start + px..a.start + x,
                    b.start + py..b.start + y,
                ));
            }
            let len = back + 1 + forward;
            s.tasks.push(Task::Equal(len));
            (px, py) = (x + len, y + len);
        }
        if px < na || py < nb {
            s.tasks
                .push(Task::Residual(a.start + px..a.end, b.start + py..b.end));
        }
        stack.extend(s.tasks.drain(..).rev());
        Ok(())
    }
}

/// Emits the common prefix of `(a, b)` and returns the rest without its common suffix, and the suffix length.
fn strip(
    cx: &mut Ctx<'_, '_>,
    a: Range<u32>,
    b: Range<u32>,
) -> Result<(Range<u32>, Range<u32>, u32), DiffError> {
    let prefix = cx.common_prefix(&a, &b)?;
    cx.out.equal(prefix);
    let (a, b) = (a.start + prefix..a.end, b.start + prefix..b.end);
    let suffix = cx.common_suffix(&a, &b)?;
    Ok((a.start..a.end - suffix, b.start..b.end - suffix, suffix))
}
