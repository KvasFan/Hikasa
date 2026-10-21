//! Closeness matching (§2.15.2): the monotone, maximum-similarity pairing of the lines of one gap.
//!
//! ```text
//! M[0][*] = M[*][0] = 0
//! M[i][j] = max( M[i−1][j], M[i][j−1], allowed(i−1, j−1) ? M[i−1][j−1] + sim(i−1, j−1) : −∞ )
//! traceback from (p, q): the diagonal when it attains the max and is allowed; else up; else left
//! ```
//!
//! Memory: one direction byte per cell (≤ `max_cells_per_gap` bytes per gap) plus two vectors of totals along
//! the **shorter** side of the gap (16 B × (min(p, q) + 1)). The table is filled with the longer side in the outer
//! loop; `M` and the direction of every cell do not depend on the iteration order, so the pairs (and their
//! tie-breaking) are the same as with a row-major fill. The direction table is reserved with `try_reserve`, so an
//! absurd cell budget fails softly (the caller pairs the gap by position) instead of aborting the process (T9).
//! Cancellation is checked every 4,096 cells, however wide the gap (§5.3).

use hikasa_core::CancelToken;

use crate::error::DiffError;

/// Traceback directions.
const DIAG: u8 = 0;
const UP: u8 = 1;
const LEFT: u8 = 2;

/// Cells between two cancellation checks. Lower than the 65,536 steps of §5.3 because every cell calls the
/// caller's scorer, whose cost is unknown.
const CHECK_CELLS: u32 = 4096;

/// Reusable buffers of the DP (one instance per alignment).
#[derive(Debug, Default)]
pub(crate) struct ClosenessDp {
    /// Direction per cell, `outer × inner` (outer = the longer side of the gap).
    dir: Vec<u8>,
    /// `M` along the shorter side at the previous outer index.
    prev: Vec<u64>,
    /// `M` along the shorter side at the current outer index.
    cur: Vec<u64>,
}

/// Outcome of [`ClosenessDp::pair`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DpOutcome {
    /// The pairs were written.
    Paired,
    /// The direction table could not be allocated; nothing was written.
    NoMemory,
}

impl ClosenessDp {
    /// Pairs a `p × q` gap: writes the chosen pairs `(i, j)` (gap-local, strictly increasing in both
    /// coordinates) to `out`. `sim(i, j)` must return the similarity (0..=1000) of gap lines `i` and `j`; a pair
    /// is allowed when `sim ≥ threshold`. Checks `cancel` every 4,096 cells.
    pub(crate) fn pair(
        &mut self,
        p: u32,
        q: u32,
        threshold: u16,
        mut sim: impl FnMut(u32, u32) -> u16,
        cancel: &CancelToken,
        out: &mut Vec<(u32, u32)>,
    ) -> Result<DpOutcome, DiffError> {
        out.clear();
        let (pu, qu) = (p as usize, q as usize);
        let Some(cells) = pu.checked_mul(qu) else {
            return Ok(DpOutcome::NoMemory);
        };
        self.dir.clear();
        if self.dir.try_reserve_exact(cells).is_err() {
            return Ok(DpOutcome::NoMemory);
        }
        // The totals run along the shorter side: q ≥ p → outer = j (right lines), inner = i (left lines).
        let transposed = q > p;
        if transposed {
            self.fill::<true>(q, p, threshold, |o, n| sim(n, o), cancel)?;
        } else {
            self.fill::<false>(p, q, threshold, &mut sim, cancel)?;
        }
        let cell = |i: usize, j: usize| {
            if transposed {
                (j - 1) * pu + (i - 1)
            } else {
                (i - 1) * qu + (j - 1)
            }
        };
        let (mut i, mut j) = (pu, qu);
        while i > 0 && j > 0 {
            match self.dir[cell(i, j)] {
                DIAG => {
                    out.push(((i - 1) as u32, (j - 1) as u32));
                    i -= 1;
                    j -= 1;
                }
                UP => i -= 1,
                _ => j -= 1,
            }
        }
        out.reverse();
        Ok(DpOutcome::Paired)
    }

    /// Fills `dir` for `outer × inner` cells. `T == false`: outer = left line `i`, inner = right line `j`;
    /// `T == true`: outer = `j`, inner = `i`. `sim(o, n)` takes (outer, inner) indices. The directions are always
    /// those of the row-major definition: "up" = `M[i−1][j]` (left line unpaired), "left" = `M[i][j−1]`.
    fn fill<const T: bool>(
        &mut self,
        outer: u32,
        inner: u32,
        threshold: u16,
        mut sim: impl FnMut(u32, u32) -> u16,
        cancel: &CancelToken,
    ) -> Result<(), DiffError> {
        let width = inner as usize + 1;
        self.prev.clear();
        self.prev.resize(width, 0);
        self.cur.clear();
        self.cur.resize(width, 0);
        let mut until_check = CHECK_CELLS;
        for o in 0..outer {
            self.cur[0] = 0;
            for n in 0..inner {
                let nu = n as usize;
                // Same outer index, previous inner index; previous outer index, same inner index.
                let (same_outer, same_inner) = (self.cur[nu], self.prev[nu + 1]);
                let (up, left) = if T {
                    (same_outer, same_inner)
                } else {
                    (same_inner, same_outer)
                };
                let s = sim(o, n);
                let allowed = s >= threshold;
                let diag = self.prev[nu] + u64::from(s);
                let mut best = up.max(left);
                if allowed {
                    best = best.max(diag);
                }
                let d = if allowed && diag == best {
                    DIAG
                } else if up == best {
                    UP
                } else {
                    LEFT
                };
                self.cur[nu + 1] = best;
                self.dir.push(d);
                until_check -= 1;
                if until_check == 0 {
                    until_check = CHECK_CELLS;
                    cancel.check()?;
                }
            }
            std::mem::swap(&mut self.prev, &mut self.cur);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn run(p: u32, q: u32, threshold: u16, table: &[Vec<u16>]) -> Vec<(u32, u32)> {
        let mut dp = ClosenessDp::default();
        let mut out = Vec::new();
        let outcome = dp
            .pair(
                p,
                q,
                threshold,
                |i, j| table[i as usize][j as usize],
                &CancelToken::never(),
                &mut out,
            )
            .unwrap();
        assert_eq!(outcome, DpOutcome::Paired);
        out
    }

    /// Best total over all monotone pairings with every pair allowed (exhaustive search).
    fn brute_force(i: usize, j: usize, threshold: u16, table: &[Vec<u16>]) -> u64 {
        if i == table.len() || j == table.first().map_or(0, Vec::len) {
            return 0;
        }
        let skip_left = brute_force(i + 1, j, threshold, table);
        let skip_right = brute_force(i, j + 1, threshold, table);
        let mut best = skip_left.max(skip_right);
        if table[i][j] >= threshold {
            best = best.max(u64::from(table[i][j]) + brute_force(i + 1, j + 1, threshold, table));
        }
        best
    }

    #[test]
    fn pairs_the_most_similar_lines() {
        let table = vec![vec![900, 100], vec![100, 600], vec![0, 700]];
        assert_eq!(run(3, 2, 500, &table), [(0, 0), (2, 1)]);
        assert_eq!(run(3, 2, 950, &table), []);
        assert_eq!(run(0, 4, 0, &[]), []);
    }

    #[test]
    fn ties_prefer_the_diagonal_then_up() {
        // Every cell scores the same: the traceback takes the diagonal from the bottom-right corner.
        let table = vec![vec![600; 3]; 3];
        assert_eq!(run(3, 3, 500, &table), [(0, 0), (1, 1), (2, 2)]);
        let table = vec![vec![600; 2]; 3];
        // (0,0)+(1,1), (0,0)+(2,1) and (1,0)+(2,1) tie; the traceback at (3,2) takes the diagonal (2,1), then at
        // (2,1) the diagonal (1,0).
        assert_eq!(run(3, 2, 500, &table), [(1, 0), (2, 1)]);
    }

    #[test]
    fn an_absurd_gap_fails_softly() {
        let mut dp = ClosenessDp::default();
        let mut out = vec![(9, 9)];
        let outcome = dp
            .pair(
                u32::MAX,
                u32::MAX,
                0,
                |_, _| 0,
                &CancelToken::never(),
                &mut out,
            )
            .unwrap();
        assert_eq!(outcome, DpOutcome::NoMemory);
        assert!(out.is_empty());
    }

    /// The row-major fill of the original definition (outer loop over the left lines), kept as the reference for
    /// the tie-breaking of the transposed fill.
    fn row_major(p: u32, q: u32, threshold: u16, table: &[Vec<u16>]) -> Vec<(u32, u32)> {
        let (pu, qu) = (p as usize, q as usize);
        let mut m = vec![vec![0u64; qu + 1]; pu + 1];
        let mut dir = vec![vec![LEFT; qu]; pu];
        for i in 0..pu {
            for j in 0..qu {
                let (up, left) = (m[i][j + 1], m[i + 1][j]);
                let s = table[i][j];
                let allowed = s >= threshold;
                let diag = m[i][j] + u64::from(s);
                let mut best = up.max(left);
                if allowed {
                    best = best.max(diag);
                }
                dir[i][j] = if allowed && diag == best {
                    DIAG
                } else if up == best {
                    UP
                } else {
                    LEFT
                };
                m[i + 1][j + 1] = best;
            }
        }
        let (mut i, mut j) = (pu, qu);
        let mut out = Vec::new();
        while i > 0 && j > 0 {
            match dir[i - 1][j - 1] {
                DIAG => {
                    out.push(((i - 1) as u32, (j - 1) as u32));
                    i -= 1;
                    j -= 1;
                }
                UP => i -= 1,
                _ => j -= 1,
            }
        }
        out.reverse();
        out
    }

    #[test]
    fn wide_and_tall_gaps_keep_the_row_major_tie_breaking() {
        // Every cell ties: the traceback must take the same path whichever side is the outer loop.
        for (p, q) in [(2, 5), (5, 2), (1, 4), (4, 1), (3, 3)] {
            let table = vec![vec![600u16; q as usize]; p as usize];
            assert_eq!(
                run(p, q, 500, &table),
                row_major(p, q, 500, &table),
                "{p} × {q}"
            );
        }
    }

    /// Regression (closeness cancellation granularity): a 1 × N gap is one row of N cells; cancellation is still
    /// observed within 4,096 cells, not at the end of the row.
    #[test]
    fn a_wide_gap_checks_cancellation_every_4096_cells() {
        for (p, q) in [(1u32, 200_000u32), (200_000, 1), (2, 100_000)] {
            let mut dp = ClosenessDp::default();
            let cancel = CancelToken::new();
            let mut calls = 0u32;
            let mut out = Vec::new();
            let result = dp.pair(
                p,
                q,
                500,
                |_, _| {
                    calls += 1;
                    if calls == 10 {
                        cancel.cancel();
                    }
                    0
                },
                &cancel,
                &mut out,
            );
            assert_eq!(result, Err(DiffError::Cancelled));
            assert!(calls <= 10 + CHECK_CELLS, "{p} × {q}: {calls} cells scored");
        }
    }

    /// Regression (closeness DP memory, §5.7): the totals run along the shorter side, so a 1 × N gap allocates
    /// N direction bytes and two totals vectors of 2 entries (a few slots of capacity), not 2 × (N + 1) totals.
    #[test]
    fn the_totals_run_along_the_shorter_side() {
        for (p, q) in [(1u32, 1_000_000u32), (1_000_000, 1)] {
            let mut dp = ClosenessDp::default();
            let mut out = Vec::new();
            let outcome = dp
                .pair(p, q, 500, |_, _| 0, &CancelToken::never(), &mut out)
                .unwrap();
            assert_eq!(outcome, DpOutcome::Paired);
            assert!(dp.prev.capacity() <= 8 && dp.cur.capacity() <= 8);
            assert_eq!(dp.dir.len(), 1_000_000);
        }
    }

    #[test]
    fn cancellation_is_observed() {
        let mut dp = ClosenessDp::default();
        let cancel = CancelToken::new();
        cancel.cancel();
        let mut out = Vec::new();
        assert_eq!(
            dp.pair(300, 300, 0, |_, _| 0, &cancel, &mut out),
            Err(DiffError::Cancelled)
        );
    }

    proptest! {
        #![proptest_config(crate::test_config::unit("tests/proptest-regressions/unit_closeness.txt"))]

        /// INV-CLOSE-1..3 on the DP itself: monotone pairs, each above the threshold, maximal total
        /// (exhaustive check for p, q ≤ 6).
        #[test]
        fn dp_is_monotone_allowed_and_optimal(
            p in 0u32..=6,
            q in 0u32..=6,
            threshold in 0u16..=1000,
            seed in proptest::collection::vec(0u16..=1000, 36),
        ) {
            let table: Vec<Vec<u16>> = (0..p as usize)
                .map(|i| (0..q as usize).map(|j| seed[i * 6 + j]).collect())
                .collect();
            let pairs = run(p, q, threshold, &table);
            for w in pairs.windows(2) {
                prop_assert!(w[0].0 < w[1].0 && w[0].1 < w[1].1, "INV-CLOSE-1");
            }
            let mut total = 0u64;
            for &(i, j) in &pairs {
                let s = table[i as usize][j as usize];
                prop_assert!(s >= threshold, "INV-CLOSE-2");
                total += u64::from(s);
            }
            let best = if p == 0 { 0 } else { brute_force(0, 0, threshold, &table) };
            prop_assert_eq!(total, best, "INV-CLOSE-3");
        }

        /// The transposed fill (the longer side outer) gives exactly the pairs of the row-major definition,
        /// ties included (similarities from a tiny value set, so ties are frequent).
        #[test]
        fn the_fill_order_never_changes_the_pairs(
            p in 0u32..=9,
            q in 0u32..=9,
            threshold in prop_oneof![Just(0u16), Just(500), Just(1000)],
            seed in proptest::collection::vec(prop_oneof![Just(0u16), Just(500), Just(1000)], 81),
        ) {
            let table: Vec<Vec<u16>> = (0..p as usize)
                .map(|i| (0..q as usize).map(|j| seed[i * 9 + j]).collect())
                .collect();
            prop_assert_eq!(run(p, q, threshold, &table), row_major(p, q, threshold, &table));
        }
    }
}
