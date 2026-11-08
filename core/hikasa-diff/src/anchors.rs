//! Alignment anchors (TC-013; D-266, contract §1.10.8): *Align With* pairs line ranges into a segment of their
//! own, *Isolate* makes lines align with nothing.
//!
//! ```text
//! normalize: pairs sorted by left.start; one that is empty, leaves the lines, or overlaps / crosses an earlier
//!            kept pair is dropped (anchor_dropped); isolated ranges clipped to the lines
//! align:     gap segment, pair segment, gap segment, …: each an alignment of its own (prefix and suffix
//!            stripped, Auto decided, budgets sized from the segment); a 1:1 pair is one row
//! isolated:  a fresh key that matches nothing, and no part in gap pairing: always Delete / Insert
//! ```
//!
//! Runs never cross a pair's boundaries (INV-ALIGN-9), so two runs of the same kind may meet at a boundary;
//! inside a segment the runs are canonical. Without anchors the result is [`crate::align`]'s, bit for bit.

use std::ops::Range;

use hikasa_core::CancelToken;
use serde::{Deserialize, Serialize};

use crate::algo::{Extras, Limits, align_core, common_run, common_run_back};
use crate::error::DiffError;
use crate::input::{AlignInput, PairScorer};
use crate::intern::TokenId;
use crate::model::{Alignment, DiffWarning, DiffWarningCode, Run, RunKind, RunSink, span};
use crate::options::{Algorithm, AlignOptions};
use crate::trace::{Derivation, SegmentTrace, WorkTotals};

/// The side of an isolated range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorSide {
    /// The left document.
    Left,
    /// The right document.
    Right,
}

/// An alignment anchor (0-based, half-open line ranges).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Anchor {
    /// Align With: left lines `left` and right lines `right` form one segment of their own (a pair of single
    /// lines is one row). Both ranges non-empty.
    Pair {
        /// Left lines.
        left: Range<u32>,
        /// Right lines.
        right: Range<u32>,
    },
    /// Isolate: these lines of `side` align with nothing.
    Isolate {
        /// The side.
        side: AnchorSide,
        /// The lines.
        lines: Range<u32>,
    },
}

/// 1-based text of a range (`10` or `10-14`).
fn lines_text(r: &Range<u32>) -> String {
    if r.end <= r.start {
        format!("{} (empty)", r.start + 1)
    } else if r.end == r.start + 1 {
        format!("{}", r.start + 1)
    } else {
        format!("{}-{}", r.start + 1, r.end)
    }
}

impl std::fmt::Display for Anchor {
    /// `align left 10-14 with right 12-13`, `isolate left 5-6` (1-based lines).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Anchor::Pair { left, right } => {
                write!(
                    f,
                    "align left {} with right {}",
                    lines_text(left),
                    lines_text(right)
                )
            }
            Anchor::Isolate { side, lines } => {
                let side = match side {
                    AnchorSide::Left => "left",
                    AnchorSide::Right => "right",
                };
                write!(f, "isolate {side} {}", lines_text(lines))
            }
        }
    }
}

/// The kept anchors for sequences of `n` and `m` lines, and a warning `anchor_dropped` for every dropped one:
/// pairs sorted by `left.start`, each pair that is empty, leaves the lines, or overlaps or crosses an earlier kept
/// pair dropped; isolated ranges clipped to the lines (dropped when nothing is left), sorted by side and start.
/// The result lists the kept pairs, then the isolated ranges.
pub fn normalize_anchors(anchors: &[Anchor], n: u32, m: u32) -> (Vec<Anchor>, Vec<DiffWarning>) {
    let mut warnings = Vec::new();
    let drop = |a: &Anchor, why: &str, warnings: &mut Vec<DiffWarning>| {
        warnings.push(DiffWarning {
            code: DiffWarningCode::AnchorDropped,
            message: format!("anchor `{a}` dropped: {why}"),
        });
    };
    let mut pairs: Vec<(Range<u32>, Range<u32>)> = Vec::new();
    let mut isolated: Vec<(AnchorSide, Range<u32>)> = Vec::new();
    for a in anchors {
        match a {
            Anchor::Pair { left, right } => {
                if left.start >= left.end || right.start >= right.end {
                    drop(a, "a range is empty", &mut warnings);
                } else if left.end > n || right.end > m {
                    drop(a, "it leaves the lines", &mut warnings);
                } else {
                    pairs.push((left.clone(), right.clone()));
                }
            }
            Anchor::Isolate { side, lines } => {
                let len = match side {
                    AnchorSide::Left => n,
                    AnchorSide::Right => m,
                };
                let clipped = lines.start.min(len)..lines.end.min(len);
                if clipped.start >= clipped.end {
                    drop(a, "it leaves the lines", &mut warnings);
                } else {
                    isolated.push((*side, clipped));
                }
            }
        }
    }
    pairs.sort_by_key(|(l, r)| (l.start, r.start));
    let mut kept: Vec<Anchor> = Vec::new();
    let (mut le, mut re) = (0u32, 0u32);
    for (left, right) in pairs {
        if left.start >= le && right.start >= re {
            (le, re) = (left.end, right.end);
            kept.push(Anchor::Pair { left, right });
        } else {
            drop(
                &Anchor::Pair { left, right },
                "it overlaps or crosses an earlier anchor",
                &mut warnings,
            );
        }
    }
    isolated.sort_by_key(|(side, r)| (*side, r.start, r.end));
    kept.extend(
        isolated
            .into_iter()
            .map(|(side, lines)| Anchor::Isolate { side, lines }),
    );
    (kept, warnings)
}

/// The engine *Auto* picks for a segment (hikasa-text's D-36 as amended in wave 3): Histogram while the estimated
/// cost of the stripped middle, `|A|·|B|` cells, fits Standard's fallback budget, else Standard.
pub fn auto_algorithm(
    left: &[TokenId],
    right: &[TokenId],
    cancel: &CancelToken,
) -> Result<Algorithm, DiffError> {
    let prefix = common_run(left, right, cancel)?;
    let suffix = common_run_back(&left[prefix..], &right[prefix..], cancel)?;
    let (a, b) = (left.len() - prefix - suffix, right.len() - prefix - suffix);
    let cells = (a as u64).saturating_mul(b as u64);
    Ok(
        if cells <= AlignOptions::fallback_budget_for(left.len(), right.len()) {
            Algorithm::Histogram
        } else {
            Algorithm::Standard
        },
    )
}

/// [`crate::align`] with anchors (§1.10.8) and optional *Auto* per segment (see [`crate::align_traced`]).
pub fn align_anchored(
    input: &AlignInput<'_>,
    opts: &AlignOptions,
    anchors: &[Anchor],
    auto: bool,
    scorer: Option<&dyn PairScorer>,
    sink: Option<&mut dyn RunSink>,
    cancel: &CancelToken,
) -> Result<Alignment, DiffError> {
    run_segments(input, opts, anchors, auto, scorer, sink, cancel, false).map(|(a, _, _)| a)
}

/// A scorer of one segment: indices shifted to the whole sequences.
struct OffsetScorer<'s> {
    inner: &'s dyn PairScorer,
    left: u32,
    right: u32,
}

impl PairScorer for OffsetScorer<'_> {
    fn similarity(&self, l: u32, r: u32) -> u16 {
        self.inner.similarity(self.left + l, self.right + r)
    }

    fn begin_gap(&self) {
        self.inner.begin_gap();
    }

    fn spent(&self) -> u64 {
        self.inner.spent()
    }
}

/// A sink of one segment: runs shifted to the whole sequences.
struct OffsetSink<'s, 'k> {
    inner: &'s mut (dyn RunSink + 'k),
    left: u32,
    right: u32,
    buf: Vec<Run>,
}

impl RunSink for OffsetSink<'_, '_> {
    fn runs_ready(&mut self, runs: &[Run]) {
        self.buf.clear();
        self.buf.extend(runs.iter().map(|r| Run {
            kind: r.kind,
            left: r.left.start + self.left..r.left.end + self.left,
            right: r.right.start + self.right..r.right.end + self.right,
        }));
        self.inner.runs_ready(&self.buf);
    }
}

/// One segment of an anchored alignment.
struct Segment {
    left: Range<u32>,
    right: Range<u32>,
    pair: bool,
}

/// The segments of the kept anchors over `n × m` lines.
fn segments(kept: &[Anchor], n: u32, m: u32) -> Vec<Segment> {
    let mut out = Vec::new();
    let (mut l, mut r) = (0u32, 0u32);
    for a in kept {
        if let Anchor::Pair { left, right } = a {
            out.push(Segment {
                left: l..left.start,
                right: r..right.start,
                pair: false,
            });
            out.push(Segment {
                left: left.clone(),
                right: right.clone(),
                pair: true,
            });
            (l, r) = (left.end, right.end);
        }
    }
    out.push(Segment {
        left: l..n,
        right: r..m,
        pair: false,
    });
    out
}

/// The anchored (and optionally traced) alignment: normalizes `anchors`, aligns every segment, concatenates.
/// Returns the alignment, the derivation when `record`, and the normalization warnings.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_segments(
    input: &AlignInput<'_>,
    opts: &AlignOptions,
    anchors: &[Anchor],
    auto: bool,
    scorer: Option<&dyn PairScorer>,
    mut sink: Option<&mut dyn RunSink>,
    cancel: &CancelToken,
    record: bool,
) -> Result<(Alignment, Option<Derivation>, Vec<DiffWarning>), DiffError> {
    let (n, m) = (input.left.len() as u32, input.right.len() as u32);
    let (kept, dropped) = normalize_anchors(anchors, n, m);
    if kept.is_empty() && !auto && !record {
        let a = crate::align(input, opts, scorer, sink, cancel)?;
        return Ok((a, None, dropped));
    }
    // Isolated lines: one flag per line, and keys that match nothing.
    let mut iso: [Vec<bool>; 2] = [Vec::new(), Vec::new()];
    for a in &kept {
        if let Anchor::Isolate { side, lines } = a {
            let (k, len) = match side {
                AnchorSide::Left => (0, n),
                AnchorSide::Right => (1, m),
            };
            if iso[k].is_empty() {
                iso[k] = vec![false; len as usize];
            }
            iso[k][lines.start as usize..lines.end as usize].fill(true);
        }
    }
    let fresh_base = input
        .left
        .iter()
        .chain(input.right)
        .map(|t| t.0)
        .max()
        .map_or(0, |x| x.saturating_add(1))
        .max(input.distinct);
    let replace = |keys: &[TokenId], flags: &[bool], offset: u32| -> Option<Vec<TokenId>> {
        if flags.is_empty() {
            return None;
        }
        Some(
            keys.iter()
                .zip(flags)
                .enumerate()
                .map(|(i, (&t, &f))| {
                    if f {
                        TokenId(fresh_base.saturating_add(offset + i as u32))
                    } else {
                        t
                    }
                })
                .collect(),
        )
    };
    let left_keys = replace(input.left, &iso[0], 0);
    let right_keys = replace(input.right, &iso[1], n);
    let left: &[TokenId] = left_keys.as_deref().unwrap_or(input.left);
    let right: &[TokenId] = right_keys.as_deref().unwrap_or(input.right);

    let mut runs: Vec<Run> = Vec::new();
    let mut warnings: Vec<hikasa_core::Warning> = Vec::new();
    let mut diff_warnings: Vec<DiffWarning> = Vec::new();
    let mut traces: Vec<SegmentTrace> = Vec::new();
    let mut any_standard = false;
    for seg in segments(&kept, n, m) {
        if seg.left.is_empty() && seg.right.is_empty() {
            continue;
        }
        cancel.check()?;
        let (ls, rs) = (
            seg.left.start as usize..seg.left.end as usize,
            seg.right.start as usize..seg.right.end as usize,
        );
        let flags = |k: usize, r: Range<usize>| -> &[bool] {
            if iso[k].is_empty() { &[] } else { &iso[k][r] }
        };
        let (iso_l, iso_r) = (flags(0, ls.clone()), flags(1, rs.clone()));
        if seg.pair && span(&seg.left) == 1 && span(&seg.right) == 1 {
            let (i, j) = (seg.left.start, seg.right.start);
            let isolated = iso_l.first() == Some(&true) || iso_r.first() == Some(&true);
            let kind = if isolated {
                None
            } else if left[i as usize] == right[j as usize] {
                Some(RunKind::Equal)
            } else if opts.never_align_differences {
                None
            } else {
                Some(RunKind::Changed)
            };
            let new: Vec<Run> = match kind {
                Some(kind) => vec![Run {
                    kind,
                    left: i..i + 1,
                    right: j..j + 1,
                }],
                None => vec![
                    Run {
                        kind: RunKind::Delete,
                        left: i..i + 1,
                        right: j..j,
                    },
                    Run {
                        kind: RunKind::Insert,
                        left: i + 1..i + 1,
                        right: j..j + 1,
                    },
                ],
            };
            if let Some(s) = sink.as_deref_mut() {
                s.runs_ready(&new);
            }
            runs.extend(new);
            if record {
                traces.push(SegmentTrace {
                    left: seg.left.clone(),
                    right: seg.right.clone(),
                    prefix: u32::from(kind == Some(RunKind::Equal)),
                    suffix: 0,
                    engine: opts.algorithm,
                    pair: true,
                    root: None,
                    nodes: Vec::new(),
                    work: WorkTotals::default(),
                });
            }
            continue;
        }
        let lw = if input.left_weights.is_empty() {
            &[][..]
        } else {
            &input.left_weights[ls.clone()]
        };
        let rw = if input.right_weights.is_empty() {
            &[][..]
        } else {
            &input.right_weights[rs.clone()]
        };
        let sub = AlignInput {
            left: &left[ls],
            right: &right[rs],
            left_weights: lw,
            right_weights: rw,
            trivial: input.trivial,
            distinct: input.distinct,
        };
        let engine = if auto {
            auto_algorithm(sub.left, sub.right, cancel)?
        } else {
            opts.algorithm
        };
        any_standard |= engine == Algorithm::Standard;
        let sub_opts = AlignOptions {
            algorithm: engine,
            ..*opts
        };
        let offset_scorer = scorer.map(|inner| OffsetScorer {
            inner,
            left: seg.left.start,
            right: seg.right.start,
        });
        let mut offset_sink = sink.as_deref_mut().map(|inner| OffsetSink {
            inner,
            left: seg.left.start,
            right: seg.right.start,
            buf: Vec::new(),
        });
        let (a, rec) = align_core(
            &sub,
            &sub_opts,
            offset_scorer.as_ref().map(|s| s as &dyn PairScorer),
            offset_sink.as_mut().map(|s| s as &mut dyn RunSink),
            cancel,
            Limits::PRODUCTION,
            Extras {
                isolated: [iso_l, iso_r],
                record,
            },
        )?;
        runs.extend(a.runs().iter().map(|r| Run {
            kind: r.kind,
            left: r.left.start + seg.left.start..r.left.end + seg.left.start,
            right: r.right.start + seg.right.start..r.right.end + seg.right.start,
        }));
        for w in a.warnings() {
            if !warnings.iter().any(|x| x.code == w.code) {
                warnings.push(w.clone());
            }
        }
        for w in a.diff_warnings() {
            if !diff_warnings.iter().any(|x| x.code == w.code) {
                diff_warnings.push(w.clone());
            }
        }
        if let Some(rec) = rec {
            let prefix = common_run(sub.left, sub.right, cancel)? as u32;
            let suffix = common_run_back(
                &sub.left[prefix as usize..],
                &sub.right[prefix as usize..],
                cancel,
            )? as u32;
            traces.push(SegmentTrace {
                left: seg.left.clone(),
                right: seg.right.clone(),
                prefix,
                suffix,
                engine,
                pair: seg.pair,
                root: rec.root,
                nodes: rec.nodes,
                work: rec.totals,
            });
        }
    }
    let reported = if !auto {
        opts.algorithm
    } else if any_standard {
        Algorithm::Standard
    } else {
        Algorithm::Histogram
    };
    let alignment = Alignment::new(runs, n, m, reported, warnings, diff_warnings);
    let derivation = record.then(|| Derivation::from_segments(traces));
    Ok((alignment, derivation, dropped))
}

impl Alignment {
    /// Checks INV-ALIGN-1…4 and 7 over the whole alignment, INV-ALIGN-5 inside each segment of `anchors`
    /// (normalized here), and INV-ALIGN-9: every kept pair's ranges are exactly one segment of the runs, and every
    /// isolated line is in a Delete or Insert run.
    pub fn check_invariants_anchored(
        &self,
        input: &AlignInput<'_>,
        opts: &AlignOptions,
        anchors: &[Anchor],
    ) -> Result<(), String> {
        let (n, m) = (input.left.len() as u32, input.right.len() as u32);
        let (kept, _) = normalize_anchors(anchors, n, m);
        let (mut li, mut ri) = (0u32, 0u32);
        for (k, run) in self.runs().iter().enumerate() {
            if run.left.start != li || run.right.start != ri {
                return Err(format!(
                    "INV-ALIGN-1: run {k} {run:?} does not continue at ({li}, {ri})"
                ));
            }
            let (ll, rl) = (span(&run.left), span(&run.right));
            let ok = match run.kind {
                RunKind::Equal | RunKind::Changed => ll == rl && ll > 0,
                RunKind::Delete => rl == 0 && ll > 0,
                RunKind::Insert => ll == 0 && rl > 0,
            };
            if !ok {
                return Err(format!("INV-ALIGN-2/3/4: run {k} {run:?}"));
            }
            if run.kind == RunKind::Changed && opts.never_align_differences {
                return Err(format!("INV-ALIGN-7: run {k} is Changed under never-align"));
            }
            for (i, j) in run.left.clone().zip(run.right.clone()) {
                let equal = input.left[i as usize] == input.right[j as usize];
                if run.kind == RunKind::Equal && !equal {
                    return Err(format!(
                        "INV-ALIGN-2: run {k} pairs different keys at ({i}, {j})"
                    ));
                }
            }
            (li, ri) = (run.left.end, run.right.end);
        }
        if (li, ri) != (n, m) {
            return Err(format!(
                "INV-ALIGN-1: runs end at ({li}, {ri}), not ({n}, {m})"
            ));
        }
        // Segment boundaries: every pair's starts and ends are run boundaries.
        let starts: std::collections::HashSet<(u32, u32)> = self
            .runs()
            .iter()
            .map(|r| (r.left.start, r.right.start))
            .collect();
        let ends: std::collections::HashSet<(u32, u32)> = self
            .runs()
            .iter()
            .map(|r| (r.left.end, r.right.end))
            .collect();
        let mut boundaries = Vec::new();
        for a in &kept {
            match a {
                Anchor::Pair { left, right } => {
                    if !starts.contains(&(left.start, right.start))
                        || !ends.contains(&(left.end, right.end))
                    {
                        return Err(format!(
                            "INV-ALIGN-9: a run crosses the boundaries of `{a}`"
                        ));
                    }
                    boundaries.push((left.start, right.start));
                    boundaries.push((left.end, right.end));
                }
                Anchor::Isolate { side, lines } => {
                    for run in self.runs() {
                        let r = match side {
                            AnchorSide::Left => &run.left,
                            AnchorSide::Right => &run.right,
                        };
                        let overlaps = r.start < lines.end && lines.start < r.end;
                        if overlaps && !matches!(run.kind, RunKind::Delete | RunKind::Insert) {
                            return Err(format!(
                                "INV-ALIGN-9: an isolated line of `{a}` is in {run:?}"
                            ));
                        }
                    }
                }
            }
        }
        // INV-ALIGN-5 inside segments.
        for (k, w) in self.runs().windows(2).enumerate() {
            let at_boundary = boundaries.contains(&(w[1].left.start, w[1].right.start));
            if at_boundary {
                continue;
            }
            if w[0].kind == w[1].kind {
                return Err(format!(
                    "INV-ALIGN-5: runs {k} and {} have the same kind",
                    k + 1
                ));
            }
            if w[0].kind == RunKind::Insert && w[1].kind == RunKind::Delete {
                return Err(format!("INV-ALIGN-5: Insert before Delete at run {k}"));
            }
        }
        Ok(())
    }
}
