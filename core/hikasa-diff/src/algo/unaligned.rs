//! Unaligned (§2.14.2): for `k < min(n, m)` pair `(k, k)`, Equal if the keys are equal, else Changed (or
//! Delete + Insert under never-align); the rest becomes Delete or Insert. Skew, weights and closeness are
//! ignored, and there is no prefix/suffix stripping. Runs are reported to the sink every 4,096 pairs.

use super::Ctx;
use crate::error::DiffError;

/// Pairs between two calls of [`Ctx::tick`].
const BATCH: u32 = 4096;

pub(crate) fn run(cx: &mut Ctx<'_, '_>) -> Result<(), DiffError> {
    let (n, m) = (cx.left.len() as u32, cx.right.len() as u32);
    let common = n.min(m);
    let never = cx.opts.never_align_differences;
    for k in 0..common {
        if cx.left[k as usize] == cx.right[k as usize] {
            cx.out.equal(1);
        } else if never {
            cx.out.delete(1);
            cx.out.insert(1);
        } else {
            cx.out.changed(1);
        }
        if k % BATCH == BATCH - 1 {
            cx.tick(u64::from(BATCH))?;
            cx.flush_ready();
        }
    }
    cx.out.delete(n - common);
    cx.out.insert(m - common);
    Ok(())
}
