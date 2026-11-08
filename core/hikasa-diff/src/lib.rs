//! Pure token-sequence alignment (ARCH-002) for Hikasa.
//!
//! The crate does no I/O and knows nothing about text or grammars: it aligns two sequences of [`TokenId`]
//! with optional line weights and a set of "trivial" tokens, and it computes inline (within-line) differences
//! between two strings and a bigram similarity measure.
//!
//! | Item | Content |
//! |---|---|
//! | [`intern_pair`], [`Interner`], [`TokenId`] | dense, deterministic token ids (INV-INTERN-1) |
//! | [`align`] | the engines Unaligned, Standard, Myers, Patience and Histogram (TC-010…TC-012; §2.14) with gap pairing, closeness matching and never-align (TC-011; §2.15) |
//! | [`Alignment`], [`Run`], [`RunKind`], [`RowRef`], [`RunSink`] | the canonical run model with stable run ids and progressive output (ARCH-004) |
//! | [`inline_diff`], [`split_ranges_by_class`] | inline differences (TC-014; §2.16) |
//! | [`BigramProfile`], [`bigram_similarity`] | the closeness similarity measure (§2.15.3) |
//! | [`Anchor`], [`align_anchored`], [`normalize_anchors`] | Align With and Isolate (TC-013; M2 §1.10.8) |
//! | [`align_traced`], [`Derivation`] | the alignment with its derivation (M2 §1.10.9) |
//!
//! Every function is a deterministic function of its arguments (T1). Heuristic cut-offs count operations,
//! never time (T6): the Standard work and fallback budgets and the closeness cell budgets. Every imara-diff call
//! goes through a dense per-call remapping of the token ids (D-16), and calls on more than 4,096 tokens per side
//! run on a short-lived helper thread so that cancellation returns within 100 ms; at most two helpers of
//! cancelled calls keep running before new helper calls wait for them (D-54). Warnings whose codes
//! `hikasa_core::WarningCode` lacks are returned as [`DiffWarning`]s (`alignment_degraded`).
//!
//! Specification: `docs/design/m1-contracts.md` §1.3 (§1.3.3: wave-2 additions and deviations), §2.14–§2.16 and
//! Appendix B (INV-INTERN-1,
//! INV-DENSE-1, INV-ALIGN-1..7, INV-CLOSE-1..3, INV-INLINE-1..4).
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub(crate) mod algo;
pub mod anchors;
pub mod classify;
mod closeness;
mod dense;
pub mod error;
pub mod inline;
pub mod input;
pub mod intern;
pub mod model;
pub mod options;
pub mod similarity;
pub mod trace;

pub use algo::align;
#[doc(hidden)]
pub use algo::testing;
pub use anchors::{Anchor, AnchorSide, align_anchored, auto_algorithm, normalize_anchors};
pub use classify::split_ranges_by_class;
pub use error::DiffError;
pub use inline::{InlineDiff, InlineOptions, inline_diff};
pub use input::{AlignInput, PairScorer, TrivialSet};
pub use intern::{InternedPair, Interner, MAX_SEQUENCE_LEN, TokenId, intern_pair};
pub use model::{
    Alignment, DiffWarning, DiffWarningCode, RowRef, Run, RunKind, RunSink, canonical_runs,
};
pub use options::{Algorithm, AlignOptions, Closeness};
pub use similarity::{BigramProfile, bigram_similarity};
pub use trace::{
    Derivation, NO_NODE, NodeId, RECORD_MIN, SegmentTrace, Stage, TraceNode, WINDOW_BAND,
    WorkTotals, align_traced,
};

#[cfg(test)]
mod test_config {
    //! Proptest configuration of the unit properties in `src/` (P-ALIGN-9, the closeness DP, the LIS chains).
    use proptest::test_runner::{Config, FileFailurePersistence};

    /// Fewest cases of a unit property: these properties are cheap, so they run more cases than the 256 of the
    /// integration suites.
    pub(crate) const MIN_CASES: u32 = 512;

    /// `max(512, PROPTEST_CASES)` cases (`Config::default()` reads `PROPTEST_CASES`, §6.1: 256 in CI, 100,000
    /// nightly); failing seeds are persisted in `regressions` (relative to the package).
    pub(crate) fn unit(regressions: &'static str) -> Config {
        let default = Config::default();
        Config {
            cases: default.cases.max(MIN_CASES),
            failure_persistence: Some(Box::new(FileFailurePersistence::Direct(regressions))),
            ..default
        }
    }

    /// Regression (unit properties ignored `PROPTEST_CASES`): the nightly case count reaches the unit properties.
    #[test]
    fn unit_properties_honour_proptest_cases() {
        let config = unit("tests/proptest-regressions/unused.txt");
        assert!(config.cases >= Config::default().cases);
        assert!(config.cases >= MIN_CASES);
    }
}
