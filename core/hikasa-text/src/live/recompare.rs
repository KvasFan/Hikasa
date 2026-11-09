//! The recompare job (§1.10.11): the exact comparison of two snapshots, incrementally from the last exact result
//! when its certificates hold, else by a full compare. Which path ran never changes the result (T6).

use std::sync::Arc;

use hikasa_core::progress::Ctl;
use hikasa_diff::Anchor;

use super::{LiveKeys, LiveRules, Noted, full_live};
use crate::compare::{CompareInput, TextComparison};
use crate::document::TextDocument;
use crate::error::TextError;

/// Which paths and certificate conditions a recompare went through (A10.9's path counters).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PathCounters {
    /// Full compares (anchors changed, no derivation, a failed segment root).
    pub full: u64,
    /// T6 fallbacks (dirty lines over 25 % of the lines or 200,000 lines).
    pub fallback: u64,
    /// Results computed incrementally.
    pub incremental: u64,
    /// Condition 1 (range, prefix, suffix, Auto) held / failed.
    pub range_held: u64,
    /// Condition 1 failed.
    pub range_failed: u64,
    /// Condition 2 (candidates) held.
    pub candidates_held: u64,
    /// Condition 2 failed.
    pub candidates_failed: u64,
    /// Condition 3 (window slack) held.
    pub window_held: u64,
    /// Condition 3 failed.
    pub window_failed: u64,
    /// Condition 4 (budgets) held.
    pub budgets_held: u64,
    /// Condition 4 failed.
    pub budgets_failed: u64,
    /// Nodes re-run by their engine.
    pub reruns: u64,
    /// Lines (both sides) of the ranges re-run by the engine.
    pub rerun_lines: u64,
    /// Lines whose keys were rebuilt.
    pub rekeyed: u64,
}

impl PathCounters {
    /// Adds `other`.
    pub fn add(&mut self, other: &PathCounters) {
        self.full += other.full;
        self.fallback += other.fallback;
        self.incremental += other.incremental;
        self.range_held += other.range_held;
        self.range_failed += other.range_failed;
        self.candidates_held += other.candidates_held;
        self.candidates_failed += other.candidates_failed;
        self.window_held += other.window_held;
        self.window_failed += other.window_failed;
        self.budgets_held += other.budgets_held;
        self.budgets_failed += other.budgets_failed;
        self.reruns += other.reruns;
        self.rerun_lines += other.rerun_lines;
        self.rekeyed += other.rekeyed;
    }
}

/// An owned recompare (`Send`): the snapshots and the state since the last exact result.
#[derive(Debug)]
pub struct RecompareJob {
    pub(crate) left: Arc<TextDocument>,
    pub(crate) right: Arc<TextDocument>,
    pub(crate) base: Arc<TextComparison>,
    pub(crate) keys: LiveKeys,
    pub(crate) log: Vec<Noted>,
    pub(crate) anchors: Vec<Anchor>,
    pub(crate) full: bool,
    pub(crate) rules: Arc<LiveRules>,
    pub(crate) generation: u64,
    pub(crate) bytes_identical: bool,
}

/// A recompare's result.
#[derive(Debug)]
pub struct Recompared {
    pub(crate) cmp: TextComparison,
    pub(crate) keys: LiveKeys,
    pub(crate) anchors: Vec<Anchor>,
    /// The live generation the result is exact for.
    pub(crate) generation: u64,
    /// The paths taken.
    pub path: PathCounters,
}

impl Recompared {
    /// The comparison (exact for [`Recompared::generation`]).
    pub fn comparison(&self) -> &TextComparison {
        &self.cmp
    }

    /// The live generation the result is exact for.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl RecompareJob {
    /// The live generation the job's snapshots are of.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Runs the job (§1.10.11). A cancelled job returns `Err(Cancelled)` and changes nothing.
    pub fn run(
        self,
        pool: Option<&rayon::ThreadPool>,
        ctl: &Ctl<'_>,
    ) -> Result<Recompared, TextError> {
        let mut path = PathCounters::default();
        if !self.full
            && let Some(done) = super::incremental::run(&self, pool, ctl, &mut path)?
        {
            path.incremental += 1;
            let (cmp, keys) = done;
            return Ok(Recompared {
                cmp,
                keys,
                anchors: self.anchors,
                generation: self.generation,
                path,
            });
        }
        path.full += 1;
        self.full_run(pool, ctl, path)
    }

    /// The full compare of the snapshots.
    pub(crate) fn full_run(
        self,
        pool: Option<&rayon::ThreadPool>,
        ctl: &Ctl<'_>,
        path: PathCounters,
    ) -> Result<Recompared, TextError> {
        let input = CompareInput {
            left: self.left.clone(),
            right: self.right.clone(),
            format: self.rules.format.clone(),
            rules: self.rules.rules.clone(),
            bytes_identical: self.bytes_identical,
            force_tokenize: self.rules.force_tokenize,
            anchors: self.anchors.clone(),
        };
        let (cmp, keys) = full_live(input, &self.rules, pool, ctl, None)?;
        Ok(Recompared {
            cmp,
            keys,
            anchors: self.anchors,
            generation: self.generation,
            path,
        })
    }
}
