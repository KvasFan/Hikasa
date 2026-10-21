//! The result model: [`RunKind`], [`Run`], [`RowRef`], [`Alignment`] and the progressive [`RunSink`].
//!
//! An alignment is a list of runs in document order. The stable id of a run is its index (ARCH-050); ids are
//! deterministic for identical inputs and options. Runs are canonical (§2.14.1): adjacent runs of the same kind
//! are merged, inside a gap `Delete` precedes `Insert`, and no run is empty (INV-ALIGN-4/5).

use std::ops::Range;

use hikasa_core::{InvariantViolation, Warning};
use serde::{Deserialize, Serialize};

use crate::input::AlignInput;
use crate::options::{Algorithm, AlignOptions};

/// Run kinds of an alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    /// Pairs with equal tokens (one row per pair).
    Equal,
    /// Pairs with different tokens (one row per pair; 1:1, equal lengths).
    Changed,
    /// Left-only tokens.
    Delete,
    /// Right-only tokens.
    Insert,
}

impl RunKind {
    /// Stable snake_case name, identical to the serde form (`"equal"`, …).
    pub const fn as_str(self) -> &'static str {
        match self {
            RunKind::Equal => "equal",
            RunKind::Changed => "changed",
            RunKind::Delete => "delete",
            RunKind::Insert => "insert",
        }
    }
}

/// A run: `left` and `right` are half-open index ranges.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Run {
    /// Kind.
    pub kind: RunKind,
    /// Left range.
    pub left: Range<u32>,
    /// Right range.
    pub right: Range<u32>,
}

impl Run {
    /// Number of rows of the run: `max(|left|, |right|)`.
    pub fn rows(&self) -> u32 {
        span(&self.left).max(span(&self.right))
    }
}

/// One row of an alignment expanded from its runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowRef {
    /// Kind of the run it came from.
    pub kind: RunKind,
    /// Left index (None for `Insert`).
    pub left: Option<u32>,
    /// Right index (None for `Delete`).
    pub right: Option<u32>,
}

/// Codes of the warnings that `hikasa_core::WarningCode` does not have yet (wave 2, contract §1.3.3). The
/// snake_case names are the stable codes: hikasa-text maps each one to the `hikasa_core::Warning` of the same name
/// once hikasa-core adds that variant (an upstream change of hikasa-core).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DiffWarningCode {
    /// Standard alignment ran out of its work budget **and** of its fallback budget: at least one remaining
    /// range was aligned by unique-line anchors and gap pairing instead of Histogram (D-16, §2.14.5).
    AlignmentDegraded,
}

impl DiffWarningCode {
    /// Stable snake_case name, identical to the serde form (`"alignment_degraded"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            DiffWarningCode::AlignmentDegraded => "alignment_degraded",
        }
    }
}

impl std::fmt::Display for DiffWarningCode {
    /// Writes [`DiffWarningCode::as_str`].
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A warning with a [`DiffWarningCode`]: the shape of `hikasa_core::Warning` without its spec (an alignment
/// concerns no spec).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DiffWarning {
    /// Stable code.
    pub code: DiffWarningCode,
    /// Message.
    pub message: String,
}

/// Receives runs as soon as they are final, in document order (progressive output, ARCH-004).
pub trait RunSink {
    /// Called with consecutive, non-overlapping runs.
    fn runs_ready(&mut self, runs: &[Run]);
}

/// The result of an alignment. Immutable; invariants INV-ALIGN-1..7 (Appendix B).
/// The stable id of a run is its index; ids are deterministic for identical inputs and options (ARCH-050).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alignment {
    /// Canonical runs in document order.
    runs: Vec<Run>,
    /// Left sequence length.
    left_len: u32,
    /// Right sequence length.
    right_len: u32,
    /// The requested algorithm.
    used: Algorithm,
    /// Warnings, in order of first occurrence (each code at most once).
    warnings: Vec<Warning>,
    /// Warnings whose codes hikasa-core does not have yet (each code at most once).
    diff_warnings: Vec<DiffWarning>,
}

impl Alignment {
    /// Assembles a result (crate-internal: alignments are only produced by `align`).
    pub(crate) fn new(
        runs: Vec<Run>,
        left_len: u32,
        right_len: u32,
        used: Algorithm,
        warnings: Vec<Warning>,
        diff_warnings: Vec<DiffWarning>,
    ) -> Alignment {
        Alignment {
            runs,
            left_len,
            right_len,
            used,
            warnings,
            diff_warnings,
        }
    }

    /// Runs in document order.
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// Left sequence length.
    pub fn left_len(&self) -> u32 {
        self.left_len
    }

    /// Right sequence length.
    pub fn right_len(&self) -> u32 {
        self.right_len
    }

    /// Rows = Σ max(|left|, |right|) over runs (Delete/Insert contribute their own length).
    pub fn row_count(&self) -> u64 {
        self.runs.iter().map(|run| u64::from(run.rows())).sum()
    }

    /// Expands runs into rows, in order.
    pub fn rows(&self) -> impl Iterator<Item = RowRef> + '_ {
        self.runs.iter().flat_map(|run| {
            let kind = run.kind;
            let (left, right) = (run.left.start, run.right.start);
            (0..run.rows()).map(move |k| RowRef {
                kind,
                left: (kind != RunKind::Insert).then_some(left + k),
                right: (kind != RunKind::Delete).then_some(right + k),
            })
        })
    }

    /// Algorithm requested. A Standard alignment whose budget ran out still reports `Standard`; the fallback
    /// shows up as warning `work_budget_exhausted`.
    pub fn algorithm(&self) -> Algorithm {
        self.used
    }

    /// Warnings.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// Warnings whose codes `hikasa_core::WarningCode` does not have yet (`alignment_degraded`); the caller maps
    /// them to core warnings (contract §1.3.3).
    pub fn diff_warnings(&self) -> &[DiffWarning] {
        &self.diff_warnings
    }

    /// Checks INV-ALIGN-1..7 against the inputs.
    pub fn check_invariants(
        &self,
        input: &AlignInput<'_>,
        opts: &AlignOptions,
    ) -> Result<(), InvariantViolation> {
        let (left, right) = (input.left, input.right);
        if self.left_len as usize != left.len() || self.right_len as usize != right.len() {
            return Err(violation(
                "INV-ALIGN-1",
                format!(
                    "lengths {}/{} do not match the input lengths {}/{}",
                    self.left_len,
                    self.right_len,
                    left.len(),
                    right.len()
                ),
            ));
        }
        let (mut li, mut ri) = (0u32, 0u32);
        let mut previous: Option<RunKind> = None;
        for (id, run) in self.runs.iter().enumerate() {
            if run.left.start != li
                || run.right.start != ri
                || run.left.end < run.left.start
                || run.right.end < run.right.start
                || run.left.end > self.left_len
                || run.right.end > self.right_len
            {
                return Err(violation(
                    "INV-ALIGN-1",
                    format!("run {id} {run:?} does not continue at ({li}, {ri})"),
                ));
            }
            let (ll, rl) = (span(&run.left), span(&run.right));
            let pairs = || {
                left[run.left.start as usize..run.left.end as usize]
                    .iter()
                    .zip(&right[run.right.start as usize..run.right.end as usize])
            };
            match run.kind {
                RunKind::Equal => {
                    if ll != rl || ll == 0 {
                        return Err(violation(
                            if ll == 0 {
                                "INV-ALIGN-4"
                            } else {
                                "INV-ALIGN-2"
                            },
                            format!("equal run {id} {run:?} has lengths {ll}/{rl}"),
                        ));
                    }
                    if let Some(k) = pairs().position(|(a, b)| a != b) {
                        return Err(violation(
                            "INV-ALIGN-2",
                            format!("equal run {id} pairs different keys at offset {k}"),
                        ));
                    }
                }
                RunKind::Changed => {
                    if ll != rl || ll == 0 {
                        return Err(violation(
                            if ll == 0 {
                                "INV-ALIGN-4"
                            } else {
                                "INV-ALIGN-3"
                            },
                            format!("changed run {id} {run:?} has lengths {ll}/{rl}"),
                        ));
                    }
                    if let Some(k) = pairs().position(|(a, b)| a == b) {
                        return Err(violation(
                            "INV-ALIGN-3",
                            format!("changed run {id} pairs equal keys at offset {k}"),
                        ));
                    }
                    if opts.never_align_differences {
                        return Err(violation(
                            "INV-ALIGN-7",
                            format!("changed run {id} under never-align"),
                        ));
                    }
                }
                RunKind::Delete => {
                    if rl != 0 || ll == 0 {
                        return Err(violation(
                            "INV-ALIGN-4",
                            format!("delete run {id} {run:?} must have left only"),
                        ));
                    }
                }
                RunKind::Insert => {
                    if ll != 0 || rl == 0 {
                        return Err(violation(
                            "INV-ALIGN-4",
                            format!("insert run {id} {run:?} must have right only"),
                        ));
                    }
                }
            }
            if previous == Some(run.kind)
                || (previous == Some(RunKind::Insert) && run.kind == RunKind::Delete)
            {
                return Err(violation(
                    "INV-ALIGN-5",
                    format!("run {id} {:?} follows a {:?} run", run.kind, previous),
                ));
            }
            if opts.algorithm == Algorithm::Unaligned
                && matches!(run.kind, RunKind::Equal | RunKind::Changed)
                && run.left.start != run.right.start
            {
                return Err(violation(
                    "INV-ALIGN-7",
                    format!("unaligned run {id} {run:?} pairs off the diagonal"),
                ));
            }
            previous = Some(run.kind);
            li = run.left.end;
            ri = run.right.end;
        }
        if li != self.left_len || ri != self.right_len {
            return Err(violation(
                "INV-ALIGN-1",
                format!(
                    "runs end at ({li}, {ri}) instead of ({}, {})",
                    self.left_len, self.right_len
                ),
            ));
        }
        if left == right {
            let one_equal = match self.runs.as_slice() {
                [] => left.is_empty(),
                [run] => run.kind == RunKind::Equal,
                _ => false,
            };
            if !one_equal {
                return Err(violation(
                    "INV-ALIGN-6",
                    format!(
                        "identical inputs gave {} runs instead of one equal run",
                        self.runs.len()
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Reconstructs the right sequence from left + inserted tokens (property "apply(diff, left) = right"):
    /// Equal runs copy `left`, Changed and Insert runs copy `right`, Delete runs copy nothing. Ranges outside the
    /// given slices are skipped (the function is total).
    pub fn apply<T: Clone>(&self, left: &[T], right: &[T]) -> Vec<T> {
        let mut out = Vec::with_capacity(right.len());
        for run in &self.runs {
            let source = match run.kind {
                RunKind::Equal => left.get(run.left.start as usize..run.left.end as usize),
                RunKind::Changed | RunKind::Insert => {
                    right.get(run.right.start as usize..run.right.end as usize)
                }
                RunKind::Delete => None,
            };
            if let Some(items) = source {
                out.extend_from_slice(items);
            }
        }
        out
    }
}

/// Length of a half-open range (0 when reversed).
#[inline]
pub(crate) fn span(range: &Range<u32>) -> u32 {
    range.end.saturating_sub(range.start)
}

fn violation(id: &'static str, detail: String) -> InvariantViolation {
    InvariantViolation { id, detail }
}

/// Accumulates canonical runs in document order from an implicit cursor and forwards final runs to a sink.
///
/// Canonicalisation (§2.14.1): a run of the same kind as the last one is merged into it; a `Delete` that follows
/// an `Insert` is moved in front of it (and merged into a preceding `Delete`), because both belong to the same
/// gap; empty runs are dropped. A push therefore changes at most the last two runs, so everything before them
/// is final and may be sent to the [`RunSink`].
#[derive(Debug, Default)]
pub(crate) struct RunBuilder {
    /// Runs so far.
    runs: Vec<Run>,
    /// Left cursor.
    li: u32,
    /// Right cursor.
    ri: u32,
    /// Runs already sent to the sink.
    sent: usize,
}

impl RunBuilder {
    /// `len` equal pairs at the cursor.
    #[inline]
    pub(crate) fn equal(&mut self, len: u32) {
        self.push(RunKind::Equal, len, len);
    }

    /// `len` changed pairs at the cursor.
    #[inline]
    pub(crate) fn changed(&mut self, len: u32) {
        self.push(RunKind::Changed, len, len);
    }

    /// `len` left-only tokens at the cursor.
    #[inline]
    pub(crate) fn delete(&mut self, len: u32) {
        self.push(RunKind::Delete, len, 0);
    }

    /// `len` right-only tokens at the cursor.
    #[inline]
    pub(crate) fn insert(&mut self, len: u32) {
        self.push(RunKind::Insert, 0, len);
    }

    /// The cursor `(left, right)`.
    pub(crate) fn cursor(&self) -> (u32, u32) {
        (self.li, self.ri)
    }

    fn push(&mut self, kind: RunKind, l: u32, r: u32) {
        if l == 0 && r == 0 {
            return;
        }
        let (li, ri) = (self.li, self.ri);
        self.li += l;
        self.ri += r;
        if kind == RunKind::Delete
            && let Some(insert) = self.runs.pop_if(|last| last.kind == RunKind::Insert)
        {
            // Delete after Insert: both are unmatched tokens of one gap. Put the deletion first.
            let at = insert.right.start;
            match self.runs.last_mut() {
                Some(last) if last.kind == RunKind::Delete => last.left.end = li + l,
                _ => self.runs.push(Run {
                    kind: RunKind::Delete,
                    left: li..li + l,
                    right: at..at,
                }),
            }
            self.runs.push(Run {
                kind: RunKind::Insert,
                left: li + l..li + l,
                right: insert.right,
            });
            return;
        }
        match self.runs.last_mut() {
            Some(last) if last.kind == kind => {
                last.left.end = li + l;
                last.right.end = ri + r;
            }
            _ => self.runs.push(Run {
                kind,
                left: li..li + l,
                right: ri..ri + r,
            }),
        }
    }

    /// Sends every run that can no longer change (all but the last two) to `sink`.
    pub(crate) fn flush_ready(&mut self, sink: Option<&mut (dyn RunSink + '_)>) {
        let ready = self.runs.len().saturating_sub(2);
        self.send(ready, sink);
    }

    /// Sends every run, the last two included, to `sink`. Only for a caller that knows the next push can neither
    /// merge into nor reorder them (after a maximal common prefix the next run is never `Equal`).
    pub(crate) fn flush_final(&mut self, sink: Option<&mut (dyn RunSink + '_)>) {
        let all = self.runs.len();
        self.send(all, sink);
    }

    /// Sends the remaining runs to `sink` and returns all runs.
    pub(crate) fn finish(mut self, sink: Option<&mut (dyn RunSink + '_)>) -> Vec<Run> {
        let all = self.runs.len();
        self.send(all, sink);
        self.runs
    }

    fn send(&mut self, upto: usize, sink: Option<&mut (dyn RunSink + '_)>) {
        if upto > self.sent {
            if let Some(sink) = sink {
                sink.runs_ready(&self.runs[self.sent..upto]);
            }
            self.sent = upto;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::TrivialSet;
    use crate::intern::TokenId;

    fn run(kind: RunKind, left: Range<u32>, right: Range<u32>) -> Run {
        Run { kind, left, right }
    }

    #[test]
    fn the_builder_canonicalises() {
        let mut b = RunBuilder::default();
        b.equal(0);
        b.equal(2);
        b.equal(1);
        b.insert(1);
        b.delete(2);
        b.insert(1);
        b.delete(1);
        b.changed(1);
        b.changed(2);
        b.delete(1);
        assert_eq!(b.cursor(), (10, 8));
        let runs = b.finish(None);
        assert_eq!(
            runs,
            [
                run(RunKind::Equal, 0..3, 0..3),
                run(RunKind::Delete, 3..6, 3..3),
                run(RunKind::Insert, 6..6, 3..5),
                run(RunKind::Changed, 6..9, 5..8),
                run(RunKind::Delete, 9..10, 8..8),
            ]
        );
    }

    #[derive(Default)]
    struct Collect(Vec<Run>, usize);

    impl RunSink for Collect {
        fn runs_ready(&mut self, runs: &[Run]) {
            assert!(!runs.is_empty());
            self.0.extend_from_slice(runs);
            self.1 += 1;
        }
    }

    #[test]
    fn diff_warning_codes_match_serde() {
        let code = DiffWarningCode::AlignmentDegraded;
        assert_eq!(
            serde_json::to_string(&code).unwrap(),
            format!("\"{}\"", code.as_str())
        );
        assert_eq!(code.to_string(), "alignment_degraded");
    }

    #[test]
    fn flush_final_sends_the_last_runs_too() {
        let mut b = RunBuilder::default();
        let mut sink = Collect::default();
        b.equal(3);
        b.flush_ready(Some(&mut sink));
        assert_eq!(sink.1, 0, "a lone run may still change");
        b.flush_final(Some(&mut sink));
        assert_eq!(sink.0, [run(RunKind::Equal, 0..3, 0..3)]);
        b.changed(1);
        let runs = b.finish(Some(&mut sink));
        assert_eq!(sink.0, runs);
        assert_eq!(sink.1, 2);
    }

    #[test]
    fn flushing_never_sends_a_run_that_can_still_change() {
        let mut b = RunBuilder::default();
        let mut sink = Collect::default();
        b.equal(1);
        b.delete(1);
        b.insert(1);
        b.flush_ready(Some(&mut sink));
        b.delete(1); // merges into the unsent Delete
        b.flush_ready(Some(&mut sink));
        b.equal(1);
        b.flush_ready(Some(&mut sink));
        let runs = b.finish(Some(&mut sink));
        assert_eq!(sink.0, runs);
        assert_eq!(runs.len(), 4);
    }

    #[test]
    fn rows_apply_and_counts() {
        let left = [TokenId(0), TokenId(1), TokenId(2)];
        let right = [TokenId(0), TokenId(5), TokenId(6)];
        let a = Alignment::new(
            vec![
                run(RunKind::Equal, 0..1, 0..1),
                run(RunKind::Changed, 1..2, 1..2),
                run(RunKind::Delete, 2..3, 2..2),
                run(RunKind::Insert, 3..3, 2..3),
            ],
            3,
            3,
            Algorithm::Myers,
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(a.row_count(), 4);
        assert_eq!(a.algorithm(), Algorithm::Myers);
        assert_eq!(a.left_len(), 3);
        assert_eq!(a.right_len(), 3);
        assert!(a.warnings().is_empty());
        let rows: Vec<RowRef> = a.rows().collect();
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows[2],
            RowRef {
                kind: RunKind::Delete,
                left: Some(2),
                right: None
            }
        );
        assert_eq!(
            rows[3],
            RowRef {
                kind: RunKind::Insert,
                left: None,
                right: Some(2)
            }
        );
        assert_eq!(a.apply(&left, &right), right);
        let trivial = TrivialSet::default();
        let input = AlignInput {
            left: &left,
            right: &right,
            left_weights: &[],
            right_weights: &[],
            trivial: &trivial,
            distinct: 7,
        };
        let opts = AlignOptions::with_algorithm(Algorithm::Myers);
        assert_eq!(a.check_invariants(&input, &opts), Ok(()));
        // The never-align variant forbids the Changed run.
        let never = AlignOptions {
            never_align_differences: true,
            ..opts
        };
        assert_eq!(
            a.check_invariants(&input, &never).map_err(|v| v.id),
            Err("INV-ALIGN-7")
        );
    }

    #[test]
    fn check_invariants_reports_each_violation() {
        let keys = [TokenId(0), TokenId(1)];
        let trivial = TrivialSet::default();
        let input = AlignInput {
            left: &keys,
            right: &keys,
            left_weights: &[],
            right_weights: &[],
            trivial: &trivial,
            distinct: 2,
        };
        let opts = AlignOptions::with_algorithm(Algorithm::Unaligned);
        let check = |runs: Vec<Run>, l: u32, r: u32| {
            Alignment::new(runs, l, r, Algorithm::Unaligned, Vec::new(), Vec::new())
                .check_invariants(&input, &opts)
                .map_err(|v| v.id)
        };
        assert_eq!(check(vec![run(RunKind::Equal, 0..2, 0..2)], 2, 2), Ok(()));
        assert_eq!(check(vec![], 2, 2), Err("INV-ALIGN-1"));
        assert_eq!(check(vec![], 1, 2), Err("INV-ALIGN-1"));
        assert_eq!(
            check(vec![run(RunKind::Equal, 0..2, 0..1)], 2, 2),
            Err("INV-ALIGN-2")
        );
        assert_eq!(
            check(
                vec![
                    run(RunKind::Equal, 0..1, 0..1),
                    run(RunKind::Equal, 1..2, 1..2)
                ],
                2,
                2
            ),
            Err("INV-ALIGN-5")
        );
        assert_eq!(
            check(
                vec![
                    run(RunKind::Equal, 0..1, 0..1),
                    run(RunKind::Changed, 1..2, 1..2)
                ],
                2,
                2
            ),
            Err("INV-ALIGN-3")
        );
        assert_eq!(
            check(
                vec![
                    run(RunKind::Insert, 0..0, 0..2),
                    run(RunKind::Delete, 0..2, 2..2)
                ],
                2,
                2
            ),
            Err("INV-ALIGN-5")
        );
        assert_eq!(
            check(
                vec![
                    run(RunKind::Delete, 0..2, 0..0),
                    run(RunKind::Insert, 2..2, 0..2)
                ],
                2,
                2
            ),
            Err("INV-ALIGN-6")
        );
        assert_eq!(
            check(
                vec![
                    run(RunKind::Delete, 0..2, 0..1),
                    run(RunKind::Insert, 2..2, 1..2)
                ],
                2,
                2
            ),
            Err("INV-ALIGN-4")
        );
        assert_eq!(
            check(vec![run(RunKind::Equal, 0..3, 0..3)], 2, 2),
            Err("INV-ALIGN-1")
        );
        let shifted = [TokenId(9), TokenId(0)];
        let input = AlignInput {
            left: &keys,
            right: &shifted,
            ..input
        };
        let a = Alignment::new(
            vec![
                run(RunKind::Insert, 0..0, 0..1),
                run(RunKind::Equal, 0..1, 1..2),
                run(RunKind::Delete, 1..2, 2..2),
            ],
            2,
            2,
            Algorithm::Unaligned,
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(
            a.check_invariants(&input, &opts).map_err(|v| v.id),
            Err("INV-ALIGN-7")
        );
    }
}
