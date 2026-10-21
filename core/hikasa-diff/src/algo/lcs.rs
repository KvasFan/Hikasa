//! The imara-diff adapter (§2.14.3): Myers and Histogram on a range, through [`DenseRemap`], with the helper
//! thread of D-54 (amended in wave 2) for calls on more than 4,096 tokens per side (see `HELPER_THRESHOLD`).
//!
//! ```text
//! imara_call(algorithm, A, B):                          // every imara-diff call in the crate goes through this
//!   k = dense.remap(A, B, &mut before, &mut after)       // DenseRemap: local alphabet 0..k (D-16)
//!   if |A| ≤ 4096 && |B| ≤ 4096: check; compute_with; postprocess; check
//!   else: wait (polling `cancel` every 10 ms) while 2 abandoned helpers still run;
//!         a helper thread owns copies of the tokens and runs compute_with + postprocess, sending the hunks
//!         through a channel; the caller polls `cancel` every 10 ms (recv_timeout) → Err(Cancelled) at once,
//!         and the helper counts as abandoned until its computation ends
//!   return hunks (local indices = indices into A and B)
//! ```
//!
//! imara-diff's Myers is a heuristic O(ND) diff: its preprocessing prunes tokens that occur often in the other
//! side when their neighbours are unmatched, so it is not guaranteed to find a minimal LCS (TC-010). Myers and
//! Histogram have no budget: their adversarial worst case is roughly quadratic (and cancellable).
//!
//! [`DenseRemap`]: crate::dense::DenseRemap

use std::ops::Range;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use hikasa_core::CancelToken;
use imara_diff::{Algorithm as ImaraAlgorithm, Diff, Hunk, NoSliderHeuristic, Token};

use super::Ctx;
use crate::error::DiffError;
use crate::model::span;

/// How often the caller polls its `CancelToken` while a helper thread runs (D-54).
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Most abandoned helpers (helpers of cancelled calls whose imara-diff computation is still running, because
/// imara-diff cannot be interrupted) before a new helper call waits for one of them to end (D-54, amended in
/// wave 2). The wait polls the caller's `CancelToken` every 10 ms, so cancellation latency is unchanged; the
/// number of abandoned helpers stays ≤ 2 + the number of `align` calls running concurrently.
const MAX_ABANDONED_HELPERS: usize = 2;

/// Abandoned helpers still running, process-wide. The crate's only mutable static (an accepted deviation from
/// T7, contract §1.3.3): it decides only *when* a helper call starts, never what any call returns (T6).
static ABANDONED: Mutex<usize> = Mutex::new(0);

/// Signalled whenever an abandoned helper ends.
static ABANDONED_ENDED: Condvar = Condvar::new();

/// Helper states; the transitions that change the count of abandoned helpers happen under `ABANDONED`'s lock.
const RUNNING: u8 = 0;
const ABANDONED_STATE: u8 = 1;
const ENDED: u8 = 2;

/// The count of abandoned helpers (a poisoned lock still holds a valid count: every critical section only
/// increments or decrements it).
fn abandoned() -> MutexGuard<'static, usize> {
    ABANDONED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Number of abandoned helpers still running (for the tests of `testing`).
pub(crate) fn abandoned_helpers() -> usize {
    *abandoned()
}

/// Waits (polling `cancel` every 10 ms) while `MAX_ABANDONED_HELPERS` abandoned helpers are still running.
fn wait_for_helper_slot(cancel: &CancelToken) -> Result<(), DiffError> {
    let mut count = abandoned();
    while *count >= MAX_ABANDONED_HELPERS {
        cancel.check()?;
        count = match ABANDONED_ENDED.wait_timeout(count, POLL_INTERVAL) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        };
    }
    Ok(())
}

/// Marks a running helper as abandoned (its caller was cancelled).
fn abandon(state: &AtomicU8) {
    let mut count = abandoned();
    if state
        .compare_exchange(
            RUNNING,
            ABANDONED_STATE,
            Ordering::Relaxed,
            Ordering::Relaxed,
        )
        .is_ok()
    {
        *count += 1;
    }
}

/// Marks the end of a helper's job when dropped (also when imara-diff panics).
struct HelperEnd(Arc<AtomicU8>);

impl Drop for HelperEnd {
    fn drop(&mut self) {
        let mut count = abandoned();
        if self.0.swap(ENDED, Ordering::Relaxed) == ABANDONED_STATE {
            *count = count.saturating_sub(1);
            ABANDONED_ENDED.notify_all();
        }
    }
}

/// Reusable buffers of the adapter (one per alignment).
#[derive(Debug, Default)]
pub(crate) struct ImaraState {
    /// Reused across inline calls (its flag vectors keep their capacity).
    diff: Diff,
    /// Remapped left tokens of the current call.
    before: Vec<Token>,
    /// Remapped right tokens of the current call.
    after: Vec<Token>,
    /// Hunks of the last call.
    hunks: Vec<Hunk>,
}

/// Myers or Histogram on the whole input: the common prefix (reported to the sink at once, before the imara-diff
/// call: after a maximal common prefix the next run is never `Equal`, so the prefix run is final), the middle
/// through [`range`], the common suffix.
pub(crate) fn run(cx: &mut Ctx<'_, '_>, algorithm: ImaraAlgorithm) -> Result<(), DiffError> {
    let (n, m) = (cx.left.len() as u32, cx.right.len() as u32);
    let prefix = cx.common_prefix(&(0..n), &(0..m))?;
    cx.out.equal(prefix);
    cx.flush_final();
    range(cx, algorithm, prefix..n, prefix..m)
}

/// Aligns one range with imara-diff: strips the common prefix and suffix (as Equal), remaps the middle densely,
/// runs the call, and turns the hunks into Equal runs and gaps; every gap is resolved per §2.15.
pub(crate) fn range(
    cx: &mut Ctx<'_, '_>,
    algorithm: ImaraAlgorithm,
    a: Range<u32>,
    b: Range<u32>,
) -> Result<(), DiffError> {
    let prefix = cx.common_prefix(&a, &b)?;
    cx.out.equal(prefix);
    let (a, b) = (a.start + prefix..a.end, b.start + prefix..b.end);
    let suffix = cx.common_suffix(&a, &b)?;
    let (a, b) = (a.start..a.end - suffix, b.start..b.end - suffix);
    if a.is_empty() || b.is_empty() {
        cx.gap(a, b)?;
    } else {
        let (left, right) = (cx.left, cx.right);
        let k = cx.remap.remap_cancellable(
            &left[a.start as usize..a.end as usize],
            &right[b.start as usize..b.end as usize],
            &mut cx.imara.before,
            &mut cx.imara.after,
            cx.cancel,
        )?;
        call(cx, algorithm, k)?;
        let hunks = std::mem::take(&mut cx.imara.hunks);
        let (mut pa, mut pb) = (0u32, 0u32);
        for hunk in &hunks {
            debug_assert_eq!(hunk.before.start - pa, hunk.after.start - pb);
            cx.out.equal(hunk.before.start - pa);
            cx.gap(
                a.start + hunk.before.start..a.start + hunk.before.end,
                b.start + hunk.after.start..b.start + hunk.after.end,
            )?;
            pa = hunk.before.end;
            pb = hunk.after.end;
            cx.flush_ready();
        }
        debug_assert_eq!(span(&a) - pa, span(&b) - pb);
        cx.out.equal(span(&a) - pa);
        cx.imara.hunks = hunks;
    }
    cx.out.equal(suffix);
    Ok(())
}

/// One imara-diff call on `cx.imara.before` / `cx.imara.after` (dense tokens `< k`); the hunks end up in
/// `cx.imara.hunks`. Inline for at most `helper_threshold` tokens per side, else on a helper thread (D-54).
fn call(cx: &mut Ctx<'_, '_>, algorithm: ImaraAlgorithm, k: u32) -> Result<(), DiffError> {
    let state = &mut cx.imara;
    debug_assert!(
        state.before.iter().chain(&state.after).all(|t| t.0 < k),
        "INV-DENSE-1: imara-diff tokens must be remapped below k = {k}"
    );
    cx.cancel.check()?;
    if (state.before.len() > cx.helper_threshold || state.after.len() > cx.helper_threshold)
        && let Some(hunks) = on_helper(cx.cancel, algorithm, &state.before, &state.after, k)?
    {
        state.hunks = hunks;
        return Ok(());
    }
    state
        .diff
        .compute_with(algorithm, &state.before, &state.after, k);
    state
        .diff
        .postprocess_with(&state.before, &state.after, NoSliderHeuristic);
    state.hunks.clear();
    state.hunks.extend(state.diff.hunks());
    cx.cancel.check()?;
    Ok(())
}

/// Runs one call on a helper thread that owns copies of the tokens; polls `cancel` every 10 ms and returns
/// `Err(Cancelled)` without waiting for the helper (its result is dropped; the helper is counted as abandoned
/// until its computation ends). Before starting, waits (cancellably) while `MAX_ABANDONED_HELPERS` abandoned
/// helpers are still running. `Ok(None)` when no thread could be started (the caller then runs the call inline,
/// with the same result). A panic of the helper is resumed on the calling thread (§5.6: it reaches the service
/// boundary like any engine panic).
fn on_helper(
    cancel: &CancelToken,
    algorithm: ImaraAlgorithm,
    before: &[Token],
    after: &[Token],
    k: u32,
) -> Result<Option<Vec<Hunk>>, DiffError> {
    wait_for_helper_slot(cancel)?;
    let (before, after) = (before.to_vec(), after.to_vec());
    let (tx, rx) = mpsc::sync_channel::<Vec<Hunk>>(1);
    let state = Arc::new(AtomicU8::new(RUNNING));
    let end = HelperEnd(Arc::clone(&state));
    let job = move || {
        let _end = end;
        let mut diff = Diff::default();
        diff.compute_with(algorithm, &before, &after, k);
        diff.postprocess_with(&before, &after, NoSliderHeuristic);
        // The receiver is gone when the caller was cancelled; the result is then discarded.
        let _ = tx.send(diff.hunks().collect());
    };
    let Ok(handle) = thread::Builder::new()
        .name("hikasa-imara".to_owned())
        .spawn(job)
    else {
        return Ok(None);
    };
    loop {
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(hunks) => {
                // The helper has sent its result and is about to exit.
                let _ = handle.join();
                cancel.check()?;
                return Ok(Some(hunks));
            }
            Err(RecvTimeoutError::Timeout) => {
                if cancel.is_cancelled() {
                    // Abandon the helper: dropping the handle detaches it, dropping `rx` discards its result.
                    abandon(&state);
                    return Err(DiffError::Cancelled);
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return match handle.join() {
                    Err(payload) => std::panic::resume_unwind(payload),
                    Ok(()) => Ok(None),
                };
            }
        }
    }
}
