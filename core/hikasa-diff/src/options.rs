//! Alignment options: [`Algorithm`], [`AlignOptions`], [`Closeness`] and the Standard work budget.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Alignment engine (TC-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Algorithm {
    /// Pairs line `k` with line `k` (§2.14.2).
    Unaligned,
    /// Staged anchors in a two-diagonal skew window, with line weights, a work budget and a bounded fallback
    /// (§2.14.5): O(n log n) plus its budgets on every input.
    Standard,
    /// imara-diff's Myers (§2.14.3), a heuristic O(ND) diff that is not guaranteed to find a minimal LCS; gaps
    /// are paired by position. Exact in the sense of "no budget": unbounded time on adversarial input.
    Myers,
    /// Unique-line anchors and LIS, Myers in anchor-less ranges (§2.14.4). No budget.
    Patience,
    /// imara-diff's Histogram (§2.14.3). No budget: roughly quadratic on adversarial input (cancellable).
    Histogram,
}

impl Algorithm {
    /// Every algorithm, in declaration order.
    pub const ALL: [Algorithm; 5] = [
        Algorithm::Unaligned,
        Algorithm::Standard,
        Algorithm::Myers,
        Algorithm::Patience,
        Algorithm::Histogram,
    ];

    /// Stable snake_case name, identical to the serde form (`"standard"`, …).
    pub const fn as_str(self) -> &'static str {
        match self {
            Algorithm::Unaligned => "unaligned",
            Algorithm::Standard => "standard",
            Algorithm::Myers => "myers",
            Algorithm::Patience => "patience",
            Algorithm::Histogram => "histogram",
        }
    }
}

impl fmt::Display for Algorithm {
    /// Writes [`Algorithm::as_str`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Closeness matching parameters (TC-011; §2.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Closeness {
    /// Minimum similarity (per mille, 0..=1000) for two lines to pair. Default 500.
    pub threshold_permille: u16,
    /// Per-gap DP cell cap (p·q); larger gaps pair by position. Default 4_000_000.
    pub max_cells_per_gap: u64,
    /// Total cell budget for one alignment (`None` = 32·(n+m) + 4_000_000).
    pub max_cells_total: Option<u64>,
}

impl Closeness {
    /// The defaults: threshold 500 ‰, 4,000,000 cells per gap, automatic total budget.
    pub const DEFAULT: Closeness = Closeness {
        threshold_permille: 500,
        max_cells_per_gap: 4_000_000,
        max_cells_total: None,
    };

    /// The total cell budget for sequences of lengths `n` and `m`: `max_cells_total`, or 32·(n+m) + 4,000,000.
    pub fn total_cells(&self, n: usize, m: usize) -> u64 {
        self.max_cells_total.unwrap_or_else(|| {
            (n as u64)
                .saturating_add(m as u64)
                .saturating_mul(32)
                .saturating_add(4_000_000)
        })
    }
}

impl Default for Closeness {
    /// [`Closeness::DEFAULT`].
    fn default() -> Closeness {
        Closeness::DEFAULT
    }
}

/// Alignment options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AlignOptions {
    /// Engine.
    pub algorithm: Algorithm,
    /// Skew tolerance in lines (Standard; TC-011). Default 300. `u32::MAX` = unlimited.
    pub skew_tolerance: u32,
    /// Closeness, applied to the gaps of Standard, Histogram and Patience (D-17). Unaligned and Myers pair by
    /// position; Myers adds warning `closeness_ignored` when this is `Some`.
    pub closeness: Option<Closeness>,
    /// Never pair differing tokens: gaps become Delete + Insert (TC-011).
    pub never_align_differences: bool,
    /// Standard work budget in elementary operations (`None` = 64·(n+m) + 1_000_000; §2.14.5).
    pub work_budget: Option<u64>,
}

impl AlignOptions {
    /// The defaults of D-04 for an explicit algorithm: skew 300, closeness on (500 ‰), never-align off,
    /// automatic work budget.
    pub const fn with_algorithm(algorithm: Algorithm) -> AlignOptions {
        AlignOptions {
            algorithm,
            skew_tolerance: 300,
            closeness: Some(Closeness::DEFAULT),
            never_align_differences: false,
            work_budget: None,
        }
    }

    /// The Standard work budget for sequences of lengths `n` and `m`: `work_budget`, or 64·(n+m) + 1,000,000.
    pub fn work_budget_for(&self, n: usize, m: usize) -> u64 {
        self.work_budget.unwrap_or_else(|| {
            (n as u64)
                .saturating_add(m as u64)
                .saturating_mul(64)
                .saturating_add(1_000_000)
        })
    }

    /// Cells per token of Standard's fallback budget.
    const FALLBACK_CELLS_PER_TOKEN: u64 = 128;
    /// Fixed part of Standard's fallback budget.
    const FALLBACK_CELLS_BASE: u64 = 32_000_000;

    /// The fallback budget of Standard alignment for sequences of lengths `n` and `m`: 128·(n+m) + 32,000,000
    /// cells (§2.14.5, D-16). After the work budget runs out, a remaining range `(A, B)` is aligned with Histogram
    /// only while its estimated cost `|A|·|B|` (after stripping its common prefix and suffix) fits what is left of
    /// this budget; otherwise it is split at unique lines and its gaps are paired (warning
    /// `alignment_degraded`). It counts operations, never time (T6), and does not depend on the options. An
    /// imara-diff Histogram call cost at most about 1.6 ns per cell of `|A|·|B|` in measurements on the
    /// development machine (every input shape tried), so the Histogram part of the fallback stays within about
    /// 0.05 s + 0.2 µs per token.
    pub fn fallback_budget_for(n: usize, m: usize) -> u64 {
        (n as u64)
            .saturating_add(m as u64)
            .saturating_mul(Self::FALLBACK_CELLS_PER_TOKEN)
            .saturating_add(Self::FALLBACK_CELLS_BASE)
    }
}

impl Default for AlignOptions {
    /// `AlignOptions::with_algorithm(Algorithm::Standard)`.
    fn default() -> AlignOptions {
        AlignOptions::with_algorithm(Algorithm::Standard)
    }
}

/// The deterministic work budget of Standard alignment (T6: operations are counted, never time). Every
/// examined candidate pair, extension step and hash insertion costs 1 (§2.14.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkBudget {
    /// Operations allowed.
    limit: u64,
    /// Operations spent so far.
    spent: u64,
}

impl WorkBudget {
    /// A fresh budget of `limit` operations.
    pub(crate) fn new(limit: u64) -> WorkBudget {
        WorkBudget { limit, spent: 0 }
    }

    /// Spends `cost` operations; false (and nothing spent) when that would exceed the limit.
    pub(crate) fn charge(&mut self, cost: u64) -> bool {
        match self.spent.checked_add(cost) {
            Some(total) if total <= self.limit => {
                self.spent = total;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_d04() {
        let opts = AlignOptions::default();
        assert_eq!(opts.algorithm, Algorithm::Standard);
        assert_eq!(opts.skew_tolerance, 300);
        assert_eq!(opts.closeness, Some(Closeness::default()));
        assert!(!opts.never_align_differences);
        assert_eq!(opts.work_budget_for(10, 20), 64 * 30 + 1_000_000);
        assert_eq!(
            AlignOptions {
                work_budget: Some(7),
                ..opts
            }
            .work_budget_for(10, 20),
            7
        );
        assert_eq!(
            AlignOptions::fallback_budget_for(10, 20),
            128 * 30 + 32_000_000
        );
        assert_eq!(
            AlignOptions::fallback_budget_for(usize::MAX, usize::MAX),
            u64::MAX
        );
        let close = Closeness::default();
        assert_eq!(close.threshold_permille, 500);
        assert_eq!(close.max_cells_per_gap, 4_000_000);
        assert_eq!(close.total_cells(1, 2), 32 * 3 + 4_000_000);
        assert_eq!(
            Closeness {
                max_cells_total: Some(5),
                ..close
            }
            .total_cells(1, 2),
            5
        );
    }

    #[test]
    fn algorithm_names_match_serde() {
        for algorithm in Algorithm::ALL {
            assert_eq!(
                serde_json::to_string(&algorithm).unwrap(),
                format!("\"{}\"", algorithm.as_str())
            );
            assert_eq!(algorithm.to_string(), algorithm.as_str());
        }
    }

    #[test]
    fn the_budget_refuses_overdrafts() {
        let mut budget = WorkBudget::new(5);
        assert!(budget.charge(3));
        assert!(!budget.charge(3));
        assert!(budget.charge(2));
        assert!(!budget.charge(1));
        assert!(budget.charge(0));
        let mut zero = WorkBudget::new(0);
        assert!(zero.charge(0));
        assert!(!zero.charge(1));
        let mut huge = WorkBudget::new(u64::MAX);
        assert!(huge.charge(u64::MAX));
        assert!(!huge.charge(1));
    }
}
