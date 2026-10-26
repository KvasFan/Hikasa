//! Longest increasing subsequences: the weighted chain of Standard's stage A (Fenwick tree over `j`,
//! deterministic tie-breaking, §2.14.5) and the unweighted patience-sorting chain of Patience (§2.14.4).
//!
//! Both take candidates `(i, j)` sorted by strictly increasing `i` with pairwise distinct `j` (they come from keys
//! that are unique on both sides), given as the slice of their `j` values, and check cancellation every 65,536
//! candidates.

use hikasa_core::CancelToken;

use super::checkpoint;
use crate::error::DiffError;

/// "No predecessor".
const NONE: u32 = u32::MAX;

/// Maximum-score chain strictly increasing in `i` and `j`.
#[derive(Debug, Default)]
pub(crate) struct WeightedLis {
    /// Fenwick tree over `j + 1` of `(best chain score ending there, candidate index)`; `(0, 0)` = empty.
    tree: Vec<(u64, u32)>,
    /// Best chain score ending at each candidate.
    best: Vec<u64>,
    /// Predecessor of each candidate in its best chain.
    pred: Vec<u32>,
}

impl WeightedLis {
    /// Writes the indices (into `ys`) of the maximum-score chain to `out`, in increasing order. `ys` are the
    /// candidates' `j` values (`< span`, pairwise distinct, candidates sorted by `i`), `scores` their scores
    /// (≥ 1). Ties: higher total score; then the predecessor with the larger index (the nearer one); then the
    /// chain end with the smallest index (the smallest `i`). O(k log span).
    pub(crate) fn chain(
        &mut self,
        ys: &[u32],
        scores: &[u64],
        span: u32,
        out: &mut Vec<u32>,
        cancel: &CancelToken,
    ) -> Result<(), DiffError> {
        out.clear();
        if ys.is_empty() {
            return Ok(());
        }
        self.tree.clear();
        self.tree.resize(span as usize + 1, (0, 0));
        self.best.clear();
        self.pred.clear();
        for (u, (&y, &score)) in ys.iter().zip(scores).enumerate() {
            checkpoint(cancel, u)?;
            // Best chain over candidates with a smaller j (Fenwick positions 1..=y hold j' < y).
            let (before, at) = self.query(y as usize);
            let total = score + before;
            self.best.push(total);
            self.pred.push(if before > 0 { at } else { NONE });
            self.update(y as usize + 1, (total, u as u32));
        }
        let mut end = 0;
        for (u, &total) in self.best.iter().enumerate() {
            checkpoint(cancel, u)?;
            if total > self.best[end] {
                end = u;
            }
        }
        let mut u = end as u32;
        while u != NONE {
            out.push(u);
            u = self.pred[u as usize];
        }
        out.reverse();
        Ok(())
    }

    /// Maximum over Fenwick positions `1..=pos`.
    fn query(&self, mut pos: usize) -> (u64, u32) {
        let mut acc = (0, 0);
        while pos > 0 {
            acc = acc.max(self.tree[pos]);
            pos &= pos - 1;
        }
        acc
    }

    /// Raises position `pos` (and the nodes covering it) to `value`.
    fn update(&mut self, mut pos: usize, value: (u64, u32)) {
        while pos < self.tree.len() {
            if value > self.tree[pos] {
                self.tree[pos] = value;
            }
            pos += pos.isolate_lowest_one();
        }
    }
}

/// Longest chain by patience sorting: writes the indices (into `ys`) of the chain to `out`, in increasing order.
/// Each candidate goes onto the leftmost pile whose top has a larger `j` and links to the top of the pile to its
/// left; the chain is read back from the top of the last pile. O(k log k).
pub(crate) fn patience_chain(
    ys: &[u32],
    tails: &mut Vec<u32>,
    pred: &mut Vec<u32>,
    out: &mut Vec<u32>,
    cancel: &CancelToken,
) -> Result<(), DiffError> {
    tails.clear();
    pred.clear();
    out.clear();
    for (u, &y) in ys.iter().enumerate() {
        checkpoint(cancel, u)?;
        let pile = tails.partition_point(|&t| ys[t as usize] < y);
        pred.push(if pile > 0 { tails[pile - 1] } else { NONE });
        if pile == tails.len() {
            tails.push(u as u32);
        } else {
            tails[pile] = u as u32;
        }
    }
    let mut u = tails.last().copied().unwrap_or(NONE);
    while u != NONE {
        out.push(u);
        u = pred[u as usize];
    }
    out.reverse();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn weighted(ys: &[u32], scores: &[u64]) -> Vec<u32> {
        let mut lis = WeightedLis::default();
        let mut out = Vec::new();
        let span = ys.iter().max().map_or(0, |&y| y + 1);
        lis.chain(ys, scores, span, &mut out, &CancelToken::never())
            .unwrap();
        out
    }

    fn patience(ys: &[u32]) -> Vec<u32> {
        let (mut tails, mut pred, mut out) = (Vec::new(), Vec::new(), Vec::new());
        patience_chain(ys, &mut tails, &mut pred, &mut out, &CancelToken::never()).unwrap();
        out
    }

    #[test]
    fn weighted_chain_examples() {
        assert_eq!(weighted(&[], &[]), Vec::<u32>::new());
        assert_eq!(weighted(&[0, 1, 2], &[1, 1, 1]), [0, 1, 2]);
        // A weighted candidate (17) beats 16 crossing unweighted ones.
        let mut ys: Vec<u32> = (1..=16).collect();
        ys.push(0);
        let mut scores = vec![1; 16];
        scores.push(17);
        assert_eq!(weighted(&ys, &scores), [16]);
        // Equal totals: the chain end with the smallest i wins ([0] over [1]).
        assert_eq!(weighted(&[1, 0], &[1, 1]), [0]);
        // Equal predecessor totals: the nearer (larger index) predecessor wins.
        assert_eq!(weighted(&[0, 1, 2], &[1, 1, 5]), [0, 1, 2]);
        assert_eq!(weighted(&[1, 0, 2], &[1, 1, 1]), [1, 2]);
    }

    #[test]
    fn patience_chain_examples() {
        assert_eq!(patience(&[]), Vec::<u32>::new());
        assert_eq!(patience(&[3, 2, 1, 0]), [3]);
        assert_eq!(patience(&[0, 5, 1, 2, 6, 3]), [0, 2, 3, 5]);
    }

    /// O(k²) reference for the weighted chain with the documented tie-breaking.
    fn weighted_reference(ys: &[u32], scores: &[u64]) -> Vec<u32> {
        let k = ys.len();
        let mut best = vec![0u64; k];
        let mut pred = vec![NONE; k];
        for u in 0..k {
            let mut before = 0;
            for v in 0..u {
                if ys[v] < ys[u] && best[v] >= before && best[v] > 0 {
                    before = best[v];
                    pred[u] = v as u32;
                }
            }
            best[u] = scores[u] + before;
        }
        let Some(mut end) = (0..k).next() else {
            return Vec::new();
        };
        for u in 0..k {
            if best[u] > best[end] {
                end = u;
            }
        }
        let mut chain = Vec::new();
        let mut u = end as u32;
        while u != NONE {
            chain.push(u);
            u = pred[u as usize];
        }
        chain.reverse();
        chain
    }

    proptest! {
        #![proptest_config(crate::test_config::unit("tests/proptest-regressions/unit_lis.txt"))]

        #[test]
        fn weighted_chain_matches_the_quadratic_reference(
            perm in Just((0u32..40).collect::<Vec<u32>>()).prop_shuffle(),
            take in 0usize..40,
            scores in proptest::collection::vec(1u64..=17, 40),
        ) {
            let ys = &perm[..take];
            prop_assert_eq!(weighted(ys, &scores[..take]), weighted_reference(ys, &scores[..take]));
        }

        #[test]
        fn patience_chain_is_a_longest_increasing_subsequence(
            perm in Just((0u32..30).collect::<Vec<u32>>()).prop_shuffle(),
        ) {
            let chain = patience(&perm);
            for w in chain.windows(2) {
                prop_assert!(w[0] < w[1] && perm[w[0] as usize] < perm[w[1] as usize]);
            }
            let ones = vec![1u64; perm.len()];
            prop_assert_eq!(chain.len(), weighted_reference(&perm, &ones).len());
        }
    }
}
