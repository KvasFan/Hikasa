//! Patience (§2.14.4): unique-line anchors chained by patience sorting, recursion into the gaps between them,
//! and imara-diff's Myers (through `DenseRemap`) on ranges without unique lines.
//!
//! ```text
//! patience(range):
//!   strip prefix/suffix
//!   U = pairs (i, j) with a[i] = b[j], the key occurring exactly once in A and once in B, not trivial
//!   if U is empty: Myers on the range through imara_call, its gaps resolved per §2.15
//!   else: chain = longest increasing subsequence of U by j; the chain pairs are Equal; recurse into the gaps
//! ```
//!
//! The recursion is an explicit stack processed in document order, so the runs of every finished range are
//! final and are reported to the sink as soon as they are known.

use std::ops::Range;

use imara_diff::{Algorithm as ImaraAlgorithm, Token};

use super::lis::patience_chain;
use super::{Ctx, Task, checkpoint, lcs};
use crate::error::DiffError;
use crate::model::span;

/// Reusable buffers.
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

pub(crate) fn run(cx: &mut Ctx<'_, '_>) -> Result<(), DiffError> {
    let (n, m) = (cx.left.len() as u32, cx.right.len() as u32);
    let mut scratch = Scratch::default();
    let mut stack = vec![Task::Range(0..n, 0..m)];
    while let Some(task) = stack.pop() {
        match task {
            Task::Equal(len) => cx.out.equal(len),
            // Patience never produces `Residual` (only Standard's fallback does); a range is a range.
            Task::Range(a, b) | Task::Residual(a, b) => {
                process(cx, a, b, &mut stack, &mut scratch)?
            }
        }
        cx.flush_ready();
    }
    Ok(())
}

fn process(
    cx: &mut Ctx<'_, '_>,
    a: Range<u32>,
    b: Range<u32>,
    stack: &mut Vec<Task>,
    s: &mut Scratch,
) -> Result<(), DiffError> {
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
        return Ok(());
    }
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
        return lcs::range(cx, ImaraAlgorithm::Myers, a, b);
    }
    patience_chain(&s.u_y, &mut s.tails, &mut s.pred, &mut s.chain, cx.cancel)?;
    cx.tick(s.u_x.len() as u64)?;
    // The pieces in document order: gap, anchor, gap, anchor, …, gap; pushed in reverse.
    s.tasks.clear();
    let (mut x0, mut y0) = (0u32, 0u32);
    for &u in &s.chain {
        let (x, y) = (s.u_x[u as usize], s.u_y[u as usize]);
        if x > x0 || y > y0 {
            s.tasks.push(Task::Range(
                a.start + x0..a.start + x,
                b.start + y0..b.start + y,
            ));
        }
        s.tasks.push(Task::Equal(1));
        (x0, y0) = (x + 1, y + 1);
    }
    if a.start + x0 < a.end || b.start + y0 < b.end {
        s.tasks
            .push(Task::Range(a.start + x0..a.end, b.start + y0..b.end));
    }
    stack.extend(s.tasks.drain(..).rev());
    Ok(())
}
