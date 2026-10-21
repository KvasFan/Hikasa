//! The [`align`] dispatcher (§2.14–§2.15): input validation, the shared alignment context, prefix/suffix
//! stripping and gap pairing (positional, closeness DP, never-align).
//!
//! Every engine emits runs strictly in document order into one [`RunBuilder`], which canonicalises them; the
//! engines that work on sub-ranges (Standard, Patience) keep an explicit stack of pending ranges, so no input
//! can exhaust the call stack. Every loop over tokens checks cancellation at least every 65,536 tokens (§5.3),
//! including the linear scans (dense renumbering, common prefix and suffix, block extension).

mod fallback;
mod lcs;
mod lis;
mod patience;
mod standard;
mod unaligned;

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use hikasa_core::{CancelToken, Warning, WarningCode};
use imara_diff::Algorithm as ImaraAlgorithm;

use crate::closeness::{ClosenessDp, DpOutcome};
use crate::dense::DenseRemap;
use crate::error::DiffError;
use crate::input::{AlignInput, PairScorer, TrivialSet};
use crate::intern::{TokenId, check_sequence_len};
use crate::model::{Alignment, DiffWarning, DiffWarningCode, RunBuilder, RunSink, span};
use crate::options::{Algorithm, AlignOptions, Closeness};

/// imara-diff calls with more tokens than this on either side run on a helper thread (D-54 as amended in wave 2,
/// contract §1.3.3; the original value was 200,000).
///
/// imara-diff's worst case is roughly quadratic (random tokens over 50 symbols: 40,000 per side ≈ 0.4 s, 80,000
/// ≈ 1.7 s, so 200,000 ≈ 10 s), and an inline call cannot be cancelled, so 200,000 could not meet the ≤ 100 ms
/// cancellation latency of §5.3 / G4. At 4,096 tokens per side the slowest inline calls measured on the
/// development machine (release build; periodic sequences against their reversal and random sequences over 2–100
/// symbols) took 10–16 ms at moderate load; an independent measurement on the same machine found up to 40 ms
/// (best of three) and 60–80 ms under heavy load. The margin under 100 ms is therefore about 2.5×. A larger call
/// pays one thread spawn (tens of µs). Results never depend on the path (T6).
pub(crate) const HELPER_THRESHOLD: usize = 4096;

/// Elementary steps between two cancellation checks (§5.3).
const CHECK_INTERVAL: u64 = 65_536;

/// Tokens a linear scan (renumbering, common prefix/suffix, block extension) handles between two cancellation
/// checks (§5.3).
const SCAN_CHUNK: usize = 65_536;

/// Steps of a positional loop between two calls of [`Ctx::tick`].
const TICK_BATCH: u32 = 4096;

/// A cancellation check point inside an O(n) loop: checks `cancel` once every 65,536 iterations (§5.3).
#[inline]
pub(crate) fn checkpoint(cancel: &CancelToken, iteration: usize) -> Result<(), DiffError> {
    if iteration & 0xFFFF == 0xFFFF {
        cancel.check()?;
    }
    Ok(())
}

/// Length of the common prefix of `l` and `r`, checking `cancel` every 65,536 compared tokens.
pub(crate) fn common_run<T: PartialEq>(
    l: &[T],
    r: &[T],
    cancel: &CancelToken,
) -> Result<usize, DiffError> {
    let mut len = 0;
    for (lc, rc) in l.chunks(SCAN_CHUNK).zip(r.chunks(SCAN_CHUNK)) {
        let k = lc.iter().zip(rc).take_while(|(x, y)| x == y).count();
        len += k;
        // A short count is a mismatch or the end of the shorter side.
        if k < SCAN_CHUNK {
            break;
        }
        cancel.check()?;
    }
    Ok(len)
}

/// Length of the common suffix of `l` and `r`, checking `cancel` every 65,536 compared tokens.
pub(crate) fn common_run_back<T: PartialEq>(
    l: &[T],
    r: &[T],
    cancel: &CancelToken,
) -> Result<usize, DiffError> {
    let mut len = 0;
    for (lc, rc) in l.rchunks(SCAN_CHUNK).zip(r.rchunks(SCAN_CHUNK)) {
        let k = lc
            .iter()
            .rev()
            .zip(rc.iter().rev())
            .take_while(|(x, y)| x == y)
            .count();
        len += k;
        if k < SCAN_CHUNK {
            break;
        }
        cancel.check()?;
    }
    Ok(len)
}

/// Aligns two sequences (§2.14–2.15). Pure; cancellation is checked at least every 65 536 elementary steps.
/// An imara-diff call on at most 4,096 tokens per side runs inline (cancellation checked before and after);
/// a larger call runs on a helper thread that owns copies of the remapped tokens, while the caller polls
/// `cancel` every 10 ms (`Receiver::recv_timeout`) and returns `Err(Cancelled)` at once, discarding the
/// helper's result (D-54 as amended in wave 2: the threshold is 4,096 instead of 200,000 tokens, and while two
/// abandoned helpers are still running a new helper call first waits, cancellably, for one of them to end).
/// Results never depend on which path ran (T6).
///
/// Standard is bounded: O((n + m) log(n + m)) plus its budgets on every input (§2.14.5). Histogram, Myers and
/// Patience are exact and unbounded: their adversarial worst case is super-linear (roughly quadratic), and they
/// stay cancellable. Myers is imara-diff's heuristic O(ND) diff, not a guaranteed minimal LCS (TC-010).
///
/// ```
/// use hikasa_core::CancelToken;
/// use hikasa_diff::{AlignInput, AlignOptions, Algorithm, RunKind, TrivialSet, align, intern_pair};
///
/// let left: [&[u8]; 3] = [b"a", b"b", b"c"];
/// let right: [&[u8]; 3] = [b"a", b"x", b"c"];
/// let keys = intern_pair(left, right).unwrap();
/// let trivial = TrivialSet::with_capacity(keys.distinct);
/// let input = AlignInput {
///     left: &keys.left,
///     right: &keys.right,
///     left_weights: &[],
///     right_weights: &[],
///     trivial: &trivial,
///     distinct: keys.distinct,
/// };
/// // Without a scorer, closeness could only pair equal lines: pair the gap by position instead.
/// let opts = AlignOptions { closeness: None, ..AlignOptions::with_algorithm(Algorithm::Histogram) };
/// let alignment = align(&input, &opts, None, None, &CancelToken::never()).unwrap();
/// let kinds: Vec<RunKind> = alignment.runs().iter().map(|run| run.kind).collect();
/// assert_eq!(kinds, [RunKind::Equal, RunKind::Changed, RunKind::Equal]);
/// ```
pub fn align(
    input: &AlignInput<'_>,
    opts: &AlignOptions,
    scorer: Option<&dyn PairScorer>,
    sink: Option<&mut dyn RunSink>,
    cancel: &CancelToken,
) -> Result<Alignment, DiffError> {
    align_with_limits(input, opts, scorer, sink, cancel, Limits::PRODUCTION)
}

/// Internal limits that the tests of `testing` override; they change when or how much work is done, never the
/// meaning of a result.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// Tokens per side above which an imara-diff call runs on a helper thread (D-54).
    helper_threshold: usize,
    /// Standard's fallback budget in cells (`None` = `AlignOptions::fallback_budget_for(n, m)`, §2.14.5).
    fallback_budget: Option<u64>,
}

impl Limits {
    /// The limits of [`align`].
    const PRODUCTION: Limits = Limits {
        helper_threshold: HELPER_THRESHOLD,
        fallback_budget: None,
    };
}

/// [`align`] with explicit [`Limits`].
pub(crate) fn align_with_limits(
    input: &AlignInput<'_>,
    opts: &AlignOptions,
    scorer: Option<&dyn PairScorer>,
    sink: Option<&mut dyn RunSink>,
    cancel: &CancelToken,
    limits: Limits,
) -> Result<Alignment, DiffError> {
    check_sequence_len(input.left.len())?;
    check_sequence_len(input.right.len())?;
    check_weights(input.left_weights, input.left.len())?;
    check_weights(input.right_weights, input.right.len())?;
    cancel.check()?;
    let dense = Dense::of(input, cancel)?;
    let mut cx = Ctx::new(&dense, input, opts, scorer, sink, cancel, limits);
    match opts.algorithm {
        Algorithm::Unaligned => unaligned::run(&mut cx)?,
        Algorithm::Standard => standard::run(&mut cx)?,
        Algorithm::Myers => lcs::run(&mut cx, ImaraAlgorithm::Myers)?,
        Algorithm::Patience => patience::run(&mut cx)?,
        Algorithm::Histogram => lcs::run(&mut cx, ImaraAlgorithm::Histogram)?,
    }
    let alignment = cx.finish();
    debug_assert_eq!(alignment.check_invariants(input, opts), Ok(()));
    Ok(alignment)
}

/// `WeightsLength` unless `weights` is empty or as long as the sequence.
fn check_weights(weights: &[u8], len: usize) -> Result<(), DiffError> {
    if weights.is_empty() || weights.len() == len {
        Ok(())
    } else {
        Err(DiffError::WeightsLength {
            expected: len,
            got: weights.len(),
        })
    }
}

/// The token sequences with ids small enough to index tables of size O(n + m).
///
/// Inputs from an interner (`intern_pair`) already satisfy this, and are borrowed as they are. Ids far beyond
/// `4·(n+m)` (a caller bug, or ids that are hashes) are renumbered densely in first-occurrence order first, so
/// that no table is ever sized by a huge id (T9). The engines only compare ids for equality, so renumbering
/// never changes a result. Both the scan for the largest id and the renumbering check cancellation every
/// 65,536 tokens.
struct Dense<'a> {
    left: Cow<'a, [TokenId]>,
    right: Cow<'a, [TokenId]>,
    trivial: Cow<'a, TrivialSet>,
    /// Upper bound (exclusive) of every id.
    size: u32,
}

impl<'a> Dense<'a> {
    fn of(input: &AlignInput<'a>, cancel: &CancelToken) -> Result<Dense<'a>, DiffError> {
        let mut size = 0u64;
        for chunk in input
            .left
            .chunks(SCAN_CHUNK)
            .chain(input.right.chunks(SCAN_CHUNK))
        {
            cancel.check()?;
            let largest = chunk
                .iter()
                .map(|t| t.0)
                .max()
                .map_or(0, |id| u64::from(id) + 1);
            size = size.max(largest);
        }
        let tokens = (input.left.len() + input.right.len()) as u64;
        if size <= 4 * tokens + 4096 && size <= u64::from(u32::MAX) {
            return Ok(Dense {
                left: Cow::Borrowed(input.left),
                right: Cow::Borrowed(input.right),
                trivial: Cow::Borrowed(input.trivial),
                size: size as u32,
            });
        }
        let mut ids: HashMap<u32, TokenId> = HashMap::new();
        let mut trivial = TrivialSet::default();
        let left = renumber(input.left, input.trivial, &mut ids, &mut trivial, cancel)?;
        let right = renumber(input.right, input.trivial, &mut ids, &mut trivial, cancel)?;
        let size = ids.len() as u32;
        Ok(Dense {
            left: Cow::Owned(left),
            right: Cow::Owned(right),
            trivial: Cow::Owned(trivial),
            size,
        })
    }
}

/// Renumbers `sequence` densely in first-occurrence order (continuing `ids`), marking the new ids of trivial
/// tokens in `trivial`.
fn renumber(
    sequence: &[TokenId],
    input_trivial: &TrivialSet,
    ids: &mut HashMap<u32, TokenId>,
    trivial: &mut TrivialSet,
    cancel: &CancelToken,
) -> Result<Vec<TokenId>, DiffError> {
    let mut out = Vec::with_capacity(sequence.len());
    for chunk in sequence.chunks(SCAN_CHUNK) {
        cancel.check()?;
        for &t in chunk {
            let next = TokenId(ids.len() as u32);
            out.push(*ids.entry(t.0).or_insert_with(|| {
                if input_trivial.contains(t) {
                    trivial.insert(next);
                }
                next
            }));
        }
    }
    Ok(out)
}

/// A pending piece of work of the range-splitting engines (Standard, Patience).
#[derive(Debug, Clone)]
pub(crate) enum Task {
    /// A range still to align.
    Range(Range<u32>, Range<u32>),
    /// Equal pairs at the cursor (a block, or the common suffix of a range).
    Equal(u32),
    /// A gap left by the degraded split of Standard's fallback: Histogram if it fits the fallback budget, else
    /// gap pairing (§2.15); never split again. Only Standard produces it.
    Residual(Range<u32>, Range<u32>),
}

/// How a gap with both sides non-empty is resolved (§2.15.1); never-align is checked first.
#[derive(Debug, Clone, Copy)]
enum GapMode {
    /// Pairs `(k, k)`, then the rest as Delete/Insert.
    Positional,
    /// The closeness DP (Standard, Histogram and Patience with closeness on, D-17).
    Closeness(Closeness),
}

/// State shared by every engine during one alignment.
pub(crate) struct Ctx<'a, 's> {
    /// Left keys (dense ids).
    pub(crate) left: &'a [TokenId],
    /// Right keys (dense ids).
    pub(crate) right: &'a [TokenId],
    left_weights: &'a [u8],
    right_weights: &'a [u8],
    /// Trivial keys (dense ids).
    pub(crate) trivial: &'a TrivialSet,
    /// The options.
    pub(crate) opts: AlignOptions,
    scorer: Option<&'a dyn PairScorer>,
    /// Cancellation.
    pub(crate) cancel: &'a CancelToken,
    sink: Option<&'s mut (dyn RunSink + 's)>,
    /// The canonical runs being built.
    pub(crate) out: RunBuilder,
    warnings: Vec<Warning>,
    diff_warnings: Vec<DiffWarning>,
    steps: u64,
    next_check: u64,
    /// The alignment's one `DenseRemap` (D-16).
    pub(crate) remap: DenseRemap,
    /// Buffers of the imara-diff adapter.
    pub(crate) imara: lcs::ImaraState,
    helper_threshold: usize,
    /// Standard's fallback budget override (tests only).
    pub(crate) fallback_budget: Option<u64>,
    gap_mode: GapMode,
    dp: ClosenessDp,
    dp_pairs: Vec<(u32, u32)>,
    cells_used: u64,
    cells_cap: u64,
}

impl<'a, 's> Ctx<'a, 's> {
    fn new(
        dense: &'a Dense<'_>,
        input: &AlignInput<'a>,
        opts: &AlignOptions,
        scorer: Option<&'a dyn PairScorer>,
        sink: Option<&'s mut (dyn RunSink + 's)>,
        cancel: &'a CancelToken,
        limits: Limits,
    ) -> Ctx<'a, 's> {
        let (n, m) = (dense.left.len(), dense.right.len());
        let gap_mode = match (opts.algorithm, opts.closeness) {
            (Algorithm::Standard | Algorithm::Histogram | Algorithm::Patience, Some(c)) => {
                GapMode::Closeness(c)
            }
            _ => GapMode::Positional,
        };
        let cells_cap = opts.closeness.map_or(0, |c| c.total_cells(n, m));
        let mut cx = Ctx {
            left: &dense.left,
            right: &dense.right,
            left_weights: input.left_weights,
            right_weights: input.right_weights,
            trivial: &dense.trivial,
            opts: *opts,
            scorer,
            cancel,
            sink,
            out: RunBuilder::default(),
            warnings: Vec::new(),
            diff_warnings: Vec::new(),
            steps: 0,
            next_check: CHECK_INTERVAL,
            remap: DenseRemap::new(dense.size),
            imara: lcs::ImaraState::default(),
            helper_threshold: limits.helper_threshold,
            fallback_budget: limits.fallback_budget,
            gap_mode,
            dp: ClosenessDp::default(),
            dp_pairs: Vec::new(),
            cells_used: 0,
            cells_cap,
        };
        if opts.algorithm == Algorithm::Myers && opts.closeness.is_some() {
            cx.warn(WarningCode::ClosenessIgnored, || {
                "closeness matching does not apply to the Myers algorithm; differing lines are paired by \
                 position"
                    .to_owned()
            });
        }
        cx
    }

    /// The result (sends the remaining runs to the sink).
    fn finish(self) -> Alignment {
        let left_len = self.left.len() as u32;
        let right_len = self.right.len() as u32;
        debug_assert_eq!(self.out.cursor(), (left_len, right_len));
        let runs = self.out.finish(self.sink);
        Alignment::new(
            runs,
            left_len,
            right_len,
            self.opts.algorithm,
            self.warnings,
            self.diff_warnings,
        )
    }

    /// Adds a warning unless one with the same code exists (each code at most once per alignment).
    pub(crate) fn warn(&mut self, code: WarningCode, message: impl FnOnce() -> String) {
        if !self.warnings.iter().any(|w| w.code == code) {
            self.warnings.push(Warning {
                code,
                message: message(),
                spec: None,
            });
        }
    }

    /// Adds a diff-local warning unless one with the same code exists (each code at most once per alignment).
    pub(crate) fn diff_warn(&mut self, code: DiffWarningCode, message: impl FnOnce() -> String) {
        if !self.diff_warnings.iter().any(|w| w.code == code) {
            self.diff_warnings.push(DiffWarning {
                code,
                message: message(),
            });
        }
    }

    /// Counts `steps` elementary steps; checks cancellation every 65,536 of them.
    pub(crate) fn tick(&mut self, steps: u64) -> Result<(), DiffError> {
        self.steps = self.steps.saturating_add(steps);
        if self.steps >= self.next_check {
            self.next_check = self.steps.saturating_add(CHECK_INTERVAL);
            self.cancel.check()?;
        }
        Ok(())
    }

    /// Sends the runs that are final to the sink.
    pub(crate) fn flush_ready(&mut self) {
        self.out.flush_ready(self.sink.as_deref_mut());
    }

    /// Sends every run so far to the sink (see [`RunBuilder::flush_final`]: only when none of them can change).
    pub(crate) fn flush_final(&mut self) {
        self.out.flush_final(self.sink.as_deref_mut());
    }

    /// True when either side has line weights.
    pub(crate) fn has_weights(&self) -> bool {
        !self.left_weights.is_empty() || !self.right_weights.is_empty()
    }

    /// Weight (0..=4) of left line `i`.
    #[inline]
    pub(crate) fn left_weight(&self, i: u32) -> u64 {
        self.left_weights
            .get(i as usize)
            .map_or(0, |&w| u64::from(w.min(4)))
    }

    /// Weight (0..=4) of right line `j`.
    #[inline]
    pub(crate) fn right_weight(&self, j: u32) -> u64 {
        self.right_weights
            .get(j as usize)
            .map_or(0, |&w| u64::from(w.min(4)))
    }

    /// Length of the common prefix of the two ranges (cancellation checked every 65,536 tokens).
    pub(crate) fn common_prefix(&self, a: &Range<u32>, b: &Range<u32>) -> Result<u32, DiffError> {
        let l = &self.left[a.start as usize..a.end as usize];
        let r = &self.right[b.start as usize..b.end as usize];
        Ok(common_run(l, r, self.cancel)? as u32)
    }

    /// Length of the common suffix of the two ranges (cancellation checked every 65,536 tokens).
    pub(crate) fn common_suffix(&self, a: &Range<u32>, b: &Range<u32>) -> Result<u32, DiffError> {
        let l = &self.left[a.start as usize..a.end as usize];
        let r = &self.right[b.start as usize..b.end as usize];
        Ok(common_run_back(l, r, self.cancel)? as u32)
    }

    /// Resolves a gap `(A, B)` per §2.15.1: one side empty → Delete/Insert; never-align → Delete(A), Insert(B);
    /// closeness mode → the DP (positional, plus warning `closeness_budget_exhausted`, when the gap exceeds the
    /// per-gap or total cell budget); otherwise positional. Every gap counts as a step, so cancellation is
    /// checked at least every 65,536 gaps however small they are.
    pub(crate) fn gap(&mut self, a: Range<u32>, b: Range<u32>) -> Result<(), DiffError> {
        self.tick(1)?;
        let (p, q) = (span(&a), span(&b));
        if p == 0 || q == 0 || self.opts.never_align_differences {
            self.out.delete(p);
            self.out.insert(q);
            return Ok(());
        }
        if let GapMode::Closeness(c) = self.gap_mode {
            self.cancel.check()?;
            let cells = u64::from(p) * u64::from(q);
            if cells <= c.max_cells_per_gap
                && self.cells_used.saturating_add(cells) <= self.cells_cap
                && self.closeness(c, &a, &b)?
            {
                self.cells_used += cells;
                return Ok(());
            }
            self.warn(WarningCode::ClosenessBudgetExhausted, || {
                "the closeness matching budget ran out; some differing lines were paired by position"
                    .to_owned()
            });
        }
        self.positional(a, b)
    }

    /// Positional pairing: `(k, k)` for `k < min(p, q)` (Equal if the keys are equal, else Changed), then the
    /// rest as Delete/Insert.
    pub(crate) fn positional(&mut self, a: Range<u32>, b: Range<u32>) -> Result<(), DiffError> {
        let (p, q) = (span(&a), span(&b));
        let common = p.min(q);
        for k in 0..common {
            if self.left[(a.start + k) as usize] == self.right[(b.start + k) as usize] {
                self.out.equal(1);
            } else {
                self.out.changed(1);
            }
            if k % TICK_BATCH == TICK_BATCH - 1 {
                self.tick(u64::from(TICK_BATCH))?;
            }
        }
        self.out.delete(p - common);
        self.out.insert(q - common);
        self.tick(u64::from(common % TICK_BATCH))
    }

    /// The closeness DP on one gap; false (nothing emitted) when its table could not be allocated.
    fn closeness(
        &mut self,
        c: Closeness,
        a: &Range<u32>,
        b: &Range<u32>,
    ) -> Result<bool, DiffError> {
        let (left, right, trivial, scorer) = (self.left, self.right, self.trivial, self.scorer);
        let (a0, b0) = (a.start, b.start);
        let sim = |i: u32, j: u32| -> u16 {
            let (x, y) = (left[(a0 + i) as usize], right[(b0 + j) as usize]);
            if x == y {
                1000
            } else if trivial.contains(x) || trivial.contains(y) {
                0
            } else {
                scorer.map_or(0, |s| s.similarity(a0 + i, b0 + j).min(1000))
            }
        };
        let (p, q) = (span(a), span(b));
        let outcome = self.dp.pair(
            p,
            q,
            c.threshold_permille,
            sim,
            self.cancel,
            &mut self.dp_pairs,
        )?;
        if outcome == DpOutcome::NoMemory {
            return Ok(false);
        }
        let pairs = std::mem::take(&mut self.dp_pairs);
        let (mut pi, mut pj) = (0u32, 0u32);
        for &(i, j) in &pairs {
            self.out.delete(i - pi);
            self.out.insert(j - pj);
            if left[(a0 + i) as usize] == right[(b0 + j) as usize] {
                self.out.equal(1);
            } else {
                self.out.changed(1);
            }
            pi = i + 1;
            pj = j + 1;
        }
        self.out.delete(p - pi);
        self.out.insert(q - pj);
        self.dp_pairs = pairs;
        Ok(true)
    }
}

/// Helpers for tests of this crate's internals that must run outside `src/` (timing tests may not read the
/// clock inside `src/`, §6.1 check 2). **Not a stable API.**
#[doc(hidden)]
pub mod testing {
    use super::*;

    /// [`align`] with the helper-thread threshold of D-54 set to `helper_threshold` tokens per side instead of
    /// [`HELPER_THRESHOLD_TOKENS`] (the "test-only option" of §6.2.3).
    pub fn align_with_helper_threshold(
        input: &AlignInput<'_>,
        opts: &AlignOptions,
        scorer: Option<&dyn PairScorer>,
        sink: Option<&mut dyn RunSink>,
        cancel: &CancelToken,
        helper_threshold: usize,
    ) -> Result<Alignment, DiffError> {
        let limits = Limits {
            helper_threshold,
            ..Limits::PRODUCTION
        };
        align_with_limits(input, opts, scorer, sink, cancel, limits)
    }

    /// The production helper-thread threshold (4,096 tokens per side).
    pub const HELPER_THRESHOLD_TOKENS: usize = HELPER_THRESHOLD;

    /// [`align`] with Standard's fallback budget set to `fallback_budget` cells instead of
    /// [`AlignOptions::fallback_budget_for`], so that small inputs reach the degraded split of §2.14.5.
    pub fn align_with_fallback_budget(
        input: &AlignInput<'_>,
        opts: &AlignOptions,
        scorer: Option<&dyn PairScorer>,
        sink: Option<&mut dyn RunSink>,
        cancel: &CancelToken,
        fallback_budget: u64,
    ) -> Result<Alignment, DiffError> {
        let limits = Limits {
            fallback_budget: Some(fallback_budget),
            ..Limits::PRODUCTION
        };
        align_with_limits(input, opts, scorer, sink, cancel, limits)
    }

    /// Number of abandoned helper threads (of cancelled calls) whose imara-diff computation is still running,
    /// process-wide (D-54: at most 2 before a new helper call waits).
    pub fn abandoned_helpers() -> usize {
        lcs::abandoned_helpers()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(raw: impl IntoIterator<Item = u32>) -> Vec<TokenId> {
        raw.into_iter().map(TokenId).collect()
    }

    /// The chunked scans give the naive lengths, at and around chunk boundaries.
    #[test]
    fn common_runs_match_the_naive_scans() {
        let never = CancelToken::never();
        let n = 2 * SCAN_CHUNK + 3;
        let base = ids((0..n as u32).map(|i| i % 7));
        for cut in [0, 1, SCAN_CHUNK - 1, SCAN_CHUNK, SCAN_CHUNK + 1, n - 1] {
            let mut other = base.clone();
            other[cut] = TokenId(99);
            assert_eq!(common_run(&base, &other, &never), Ok(cut));
            assert_eq!(common_run_back(&base, &other, &never), Ok(n - 1 - cut));
        }
        assert_eq!(common_run(&base, &base, &never), Ok(n));
        assert_eq!(common_run_back(&base, &base, &never), Ok(n));
        // Different lengths: the shorter side ends the run.
        assert_eq!(
            common_run(&base[..SCAN_CHUNK + 5], &base, &never),
            Ok(SCAN_CHUNK + 5)
        );
        assert_eq!(
            common_run_back(&base[n - SCAN_CHUNK - 5..], &base, &never),
            Ok(SCAN_CHUNK + 5)
        );
        assert_eq!(common_run::<TokenId>(&[], &base, &never), Ok(0));
    }

    /// Regression (uncancellable linear scans): the common prefix/suffix scans and the dense renumbering of huge
    /// inputs check cancellation every 65,536 tokens instead of running to the end.
    #[test]
    fn linear_scans_observe_cancellation() {
        let cancel = CancelToken::new();
        cancel.cancel();
        let long = ids((0..3 * SCAN_CHUNK as u32).map(|i| i % 5));
        assert_eq!(common_run(&long, &long, &cancel), Err(DiffError::Cancelled));
        assert_eq!(
            common_run_back(&long, &long, &cancel),
            Err(DiffError::Cancelled)
        );
        // A run shorter than a chunk needs no check.
        let mut other = long.clone();
        other[10] = TokenId(9);
        assert_eq!(common_run(&long, &other, &cancel), Ok(10));
        // Sparse ids force the renumbering; both the scan and the renumbering stop.
        let sparse = ids((0..3 * SCAN_CHUNK as u32).map(|i| u32::MAX - i));
        let trivial = TrivialSet::default();
        let input = AlignInput {
            left: &sparse,
            right: &sparse,
            left_weights: &[],
            right_weights: &[],
            trivial: &trivial,
            distinct: 0,
        };
        assert!(matches!(
            Dense::of(&input, &cancel),
            Err(DiffError::Cancelled)
        ));
        let dense = Dense::of(&input, &CancelToken::never()).unwrap();
        assert_eq!(dense.size, 3 * SCAN_CHUNK as u32);
        assert!(matches!(dense.left, Cow::Owned(_)));
        assert_eq!(dense.left[1], TokenId(1));
    }
}
