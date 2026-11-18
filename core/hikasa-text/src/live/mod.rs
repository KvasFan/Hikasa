//! Live comparison (TC-061; D-268, D-270, contract §1.10.10–§1.10.11): a comparison whose documents are edited.
//!
//! ```text
//! note_edit (UI thread, ≤ 1 ms)   provisional splice of every change's LineDelta: current() stays valid for the
//!                                 live documents; the touched rows are pending; stats and verdict provisional
//! recompare_job (after the        snapshots of both documents (O(blocks)) and the state since the last exact
//!   adapter's debounce)           result; RecompareJob::run computes the exact comparison off the UI thread
//! install                         the result in place when no edit came meanwhile (Current), else the edits
//!                                 since its snapshot replayed as provisional splices on top (Replayed)
//! ```
//!
//! Whatever path a recompare takes, the installed comparison equals a fresh [`crate::compare_with_limits`] of
//! the snapshots with the same rules and anchors (P-EDIT-2).

mod dirty;
mod ids;
mod incremental;
mod recompare;
pub(crate) mod splice;

use std::sync::Arc;

use hikasa_core::progress::Ctl;
use hikasa_core::side::{Pair, Side};
use hikasa_core::warning::{Warning, WarningCode};
use hikasa_diff::{Anchor, AnchorSide, Derivation};
use hikasa_formats::{CompiledFormat, LineState, RegexLimits};

use crate::classify::compile_patterns;
use crate::compare::{
    ClassifiedInline, CompareInput, CompareObserver, IgnoredLines, Prepared, RowIdx, TextComparison,
};
use crate::edit::{EditOutcome, EditableDocument, LineDelta};
use crate::error::TextError;
use crate::rules::TextRules;

pub use recompare::{PathCounters, RecompareJob, Recompared};

/// What a live comparison keeps of the last exact run besides the comparison (§1.10.11): the derivation, the
/// interned id of every line's key per side (chunked, shared with the jobs) with the ids' table, and the
/// tokenizer's start state every 64 lines per side (multi-line grammars only).
#[derive(Debug, Clone, Default)]
pub struct LiveKeys {
    pub(crate) trace: Arc<ids::LiveTrace>,
    pub(crate) ids: [ids::IdLines; 2],
    pub(crate) table: Arc<ids::IdTable>,
    /// A recompare's table changes, applied by `settle` (at `install`, in place when no job shares the table).
    pub(crate) delta: Option<Arc<ids::TableDelta>>,
    pub(crate) checkpoints: [Arc<Vec<(u32, LineState)>>; 2],
    pub(crate) regex_errors: u64,
}

impl LiveKeys {
    /// The derivation of the last exact result, every node in place. **Not a stable API** (tests and
    /// measurements).
    #[doc(hidden)]
    pub fn derivation(&self) -> Derivation {
        self.trace.to_derivation()
    }

    /// Bytes held (memory accounting): ids, their table, checkpoints, the derivation's nodes, chains and runs.
    pub fn heap_bytes(&self) -> usize {
        let fp: usize =
            self.ids.iter().map(ids::IdLines::heap_bytes).sum::<usize>() + self.table.heap_bytes();
        let cp: usize = self
            .checkpoints
            .iter()
            .map(|c| std::mem::size_of::<LineState>() * c.capacity())
            .sum();
        let trace = self.trace.heap_bytes();
        fp + cp + trace
    }

    /// Applies a recompare's table changes (in place unless a running job still shares the table).
    pub(crate) fn settle(&mut self) {
        if let Some(delta) = self.delta.take() {
            Arc::make_mut(&mut self.table).apply(&delta);
        }
    }
}

/// The rules of a live comparison (everything of a [`CompareInput`] but the documents and anchors).
#[derive(Debug, Clone)]
pub(crate) struct LiveRules {
    pub(crate) format: Arc<CompiledFormat>,
    /// The right side's own format (D-281).
    pub(crate) right_format: Option<Arc<CompiledFormat>>,
    pub(crate) rules: TextRules,
    pub(crate) force_tokenize: bool,
    pub(crate) limits: RegexLimits,
}

/// One change noted since the last exact result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Noted {
    /// The live generation after the change.
    pub(crate) generation: u64,
    pub(crate) side: Side,
    pub(crate) delta: LineDelta,
}

/// The outcome of [`LiveComparison::install`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// The exact result is current.
    Current,
    /// Edits came after the job's snapshots: they were replayed as provisional splices on top of the exact
    /// result; `pending` rows are pending (a result older than the installed one is dropped, nothing changes).
    Replayed {
        /// Pending rows after the replay.
        pending: u32,
    },
}

/// A comparison of two edited documents (see the module documentation).
#[derive(Debug)]
pub struct LiveComparison {
    current: Arc<TextComparison>,
    /// The last exact result and what it keeps.
    base: Arc<TextComparison>,
    keys: LiveKeys,
    base_generation: u64,
    base_anchors: Vec<Anchor>,
    /// The anchors in the live numbering (moved by edits).
    anchors: Vec<Anchor>,
    rules: Arc<LiveRules>,
    pending: splice::Pending,
    log: Vec<Noted>,
    generation: u64,
    /// Warnings of edits (anchors that lost every line), until taken.
    warnings: Vec<Warning>,
    /// The anchors changed since the last exact result (the next recompare runs in full).
    anchors_changed: bool,
}

/// Compares two documents for live editing: [`crate::compare_with_limits`] that also keeps the derivation, the
/// interned ids and the checkpoints (§1.10.11).
pub fn compare_live(
    input: CompareInput,
    limits: &RegexLimits,
    pool: Option<&rayon::ThreadPool>,
    ctl: &Ctl<'_>,
    observer: Option<&mut dyn CompareObserver>,
) -> Result<LiveComparison, TextError> {
    let rules = Arc::new(LiveRules {
        format: input.format.clone(),
        right_format: input.right_format.clone(),
        rules: input.rules.clone(),
        force_tokenize: input.force_tokenize,
        limits: *limits,
    });
    let (cmp, keys) = full_live(input, &rules, pool, ctl, observer)?;
    Ok(LiveComparison::new(cmp, keys, rules))
}

/// A full compare that keeps the live data.
pub(crate) fn full_live<'o>(
    input: CompareInput,
    rules: &LiveRules,
    pool: Option<&rayon::ThreadPool>,
    ctl: &Ctl<'_>,
    observer: Option<&mut (dyn CompareObserver + 'o)>,
) -> Result<(TextComparison, LiveKeys), TextError> {
    let patterns = compile_patterns(&input.rules, &rules.limits)?;
    let mut observer = observer;
    let prepared = Prepared::new(input, patterns, pool, ctl, observer.as_deref_mut())?;
    let (cmp, live) = prepared.finish_with(pool, ctl, observer, true)?;
    let live = live.unwrap_or_else(|| crate::compare::LiveData {
        trace: Derivation::default(),
        ids: [Vec::new(), Vec::new()],
        hash: Vec::new(),
        trivial: Vec::new(),
        checkpoints: [Vec::new(), Vec::new()],
        regex_errors: 0,
    });
    let [il, ir] = live.ids;
    let (table, [il, ir]) = ids::IdTable::build(&il, &ir, live.hash, &live.trivial);
    let numbered = |states: Vec<LineState>| -> Vec<(u32, LineState)> {
        states
            .into_iter()
            .enumerate()
            .map(|(k, s)| (k as u32 * crate::compare::CHECKPOINT_LINES, s))
            .collect()
    };
    let [cl, cr] = live.checkpoints.map(numbered);
    Ok((
        cmp,
        LiveKeys {
            trace: Arc::new(ids::LiveTrace::from_derivation(live.trace)),
            ids: [ids::IdLines::from_vec(&il), ids::IdLines::from_vec(&ir)],
            table: Arc::new(table),
            delta: None,
            checkpoints: [Arc::new(cl), Arc::new(cr)],
            regex_errors: live.regex_errors,
        },
    ))
}

impl LiveComparison {
    fn new(cmp: TextComparison, keys: LiveKeys, rules: Arc<LiveRules>) -> LiveComparison {
        let cmp = Arc::new(cmp);
        let anchors = cmp.anchors().to_vec();
        LiveComparison {
            current: cmp.clone(),
            base: cmp,
            keys,
            base_generation: 0,
            base_anchors: anchors.clone(),
            anchors,
            rules,
            pending: splice::Pending::default(),
            log: Vec::new(),
            generation: 0,
            warnings: Vec::new(),
            anchors_changed: false,
        }
    }

    /// The rows valid for the live documents at every moment.
    pub fn current(&self) -> &Arc<TextComparison> {
        &self.current
    }

    /// True for a row the provisional splice made or touched since the last exact result.
    pub fn is_pending(&self, row: RowIdx) -> bool {
        self.pending.contains(row.0)
    }

    /// Pending rows.
    pub fn pending_rows(&self) -> u32 {
        self.pending.count()
    }

    /// +1 per noted edit.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The anchors in the live numbering.
    pub fn anchors(&self) -> &[Anchor] {
        &self.anchors
    }

    /// Warnings of the edits since the last call (an anchor whose lines were all removed is dropped, warning
    /// `anchor_dropped`): they are not part of the comparison, which equals a fresh compare.
    pub fn take_warnings(&mut self) -> Vec<Warning> {
        std::mem::take(&mut self.warnings)
    }

    /// Notes an applied edit, undo or redo of `side` (on the UI thread, at once): every change of `out` is
    /// spliced provisionally, the anchors move like bookmarks, and `current()` takes a snapshot of `doc`.
    pub fn note_edit(&mut self, side: Side, doc: &EditableDocument, out: &EditOutcome) {
        let cmp = Arc::make_mut(&mut self.current);
        for &delta in &out.deltas {
            if delta.is_empty() {
                continue;
            }
            splice::splice(cmp, side, delta, &mut self.pending);
            self.generation += 1;
            self.log.push(Noted {
                generation: self.generation,
                side,
                delta,
            });
            move_anchors(&mut self.anchors, side, delta, &mut self.warnings);
        }
        if out.deltas.iter().all(LineDelta::is_empty) {
            // A property change (encoding) still makes a new generation: the snapshot differs.
            self.generation += 1;
        }
        let snapshot = doc.snapshot();
        match side {
            Side::Right => cmp.right = snapshot,
            _ => cmp.left = snapshot,
        }
        // The tokenizer start states follow the edit (exact; W11a): a key frame draws syntax colours without
        // tokenizing the document again. Several changes of one step: the states are prepared again.
        let k = usize::from(side == Side::Right);
        // The exact comparison of the last install shares them until this first edit after it; its start states are
        // never read again (an install takes the live ones), so they go when nothing else holds it: the splice then
        // copies none (G8 follow-up item 8).
        if let Some(base) = Arc::get_mut(&mut self.base) {
            base.start_states[k] = std::sync::OnceLock::new();
        }
        match out.deltas.iter().filter(|d| !d.is_empty()).count() {
            0 => {}
            1 => {
                if let Some(&d) = out.deltas.iter().find(|d| !d.is_empty()) {
                    cmp.splice_start_states(side, d);
                }
            }
            _ => cmp.start_states[k] = std::sync::OnceLock::new(),
        }
        cmp.anchors = self.anchors.clone();
        cmp.bytes_identical = false;
        cmp.provisional = true;
    }

    /// The ignored lines in the live numbering (§1.10.17): they follow every edit like the anchors.
    pub fn ignored(&self) -> &IgnoredLines {
        self.current.ignored()
    }

    /// Replaces the ignored lines (the *Ignored* command, TC-024; §1.10.17): the rows whose flag changes, with
    /// their sections, class runs and statistics, are re-derived at once (no recompare is needed: alignment does
    /// not change); the next recompare applies them too.
    pub fn set_ignored(&mut self, ignored: IgnoredLines) {
        self.generation += 1;
        let cmp = Arc::make_mut(&mut self.current);
        cmp.reignore(ignored);
    }

    /// Replaces the anchors (Align With, Isolate; §1.10.8). The next recompare runs in full.
    pub fn set_anchors(&mut self, anchors: Vec<Anchor>) {
        self.anchors = anchors;
        self.anchors_changed = true;
        self.generation += 1;
        let cmp = Arc::make_mut(&mut self.current);
        cmp.anchors = self.anchors.clone();
        cmp.provisional = true;
    }

    /// The job that recomputes the exact comparison of the documents as they stand (owned, `Send`).
    pub fn recompare_job(&self, docs: Pair<&EditableDocument>) -> RecompareJob {
        RecompareJob {
            left: docs.left.snapshot(),
            right: docs.right.snapshot(),
            base: self.base.clone(),
            keys: self.keys.clone(),
            log: self.log.clone(),
            anchors: self.anchors.clone(),
            ignored: self.current.ignored().clone(),
            full: self.anchors_changed,
            rules: self.rules.clone(),
            generation: self.generation,
            bytes_identical: self.base.bytes_identical()
                && self.log.is_empty()
                && !self.anchors_changed,
        }
    }

    /// Puts a recompare's result in place: `Current` when no edit came after its snapshots, else the newer edits
    /// are replayed on top as provisional splices. A result older than the installed one changes nothing.
    pub fn install(&mut self, done: Recompared, docs: Pair<&EditableDocument>) -> Installed {
        if done.generation < self.base_generation {
            return Installed::Replayed {
                pending: self.pending.count(),
            };
        }
        let live_ignored = self.current.ignored().clone();
        let base = Arc::new(done.cmp);
        self.base = base.clone();
        self.keys = done.keys;
        self.keys.settle();
        self.base_generation = done.generation;
        self.base_anchors = done.anchors.clone();
        self.log.retain(|n| n.generation > done.generation);
        self.pending = splice::Pending::default();
        if done.generation == self.generation {
            self.anchors_changed = false;
            // The live comparison's start states are those of the same documents (W11a).
            let mut exact = Arc::try_unwrap(base).unwrap_or_else(|shared| (*shared).clone());
            exact.take_start_states(Arc::make_mut(&mut self.current));
            let exact = Arc::new(exact);
            self.base = exact.clone();
            self.current = exact;
            return Installed::Current;
        }
        // Replay the newer edits on a copy of the exact result.
        let mut cmp = (*base).clone();
        for n in &self.log {
            splice::splice(&mut cmp, n.side, n.delta, &mut self.pending);
        }
        cmp.left = docs.left.snapshot();
        cmp.right = docs.right.snapshot();
        cmp.start_states = [std::sync::OnceLock::new(), std::sync::OnceLock::new()];
        // The live comparison's start states are those of the live documents (W11a).
        cmp.take_start_states(Arc::make_mut(&mut self.current));
        cmp.anchors = self.anchors.clone();
        cmp.bytes_identical = false;
        cmp.provisional = true;
        // Marks changed after the job's snapshot (`set_ignored`) are applied on top.
        cmp.reignore(live_ignored);
        self.current = Arc::new(cmp);
        Installed::Replayed {
            pending: self.pending.count(),
        }
    }

    /// The inline differences of `row` of `current()`: the comparison's own for a row that is not pending, else
    /// computed from the live lines of a paired row, whatever its provisional state (a `Same` row whose line was
    /// just edited differs already): every difference important until the recompare classifies it; `None` when
    /// the live lines are equal.
    pub fn inline(&self, row: RowIdx, docs: Pair<&EditableDocument>) -> Option<ClassifiedInline> {
        if !self.is_pending(row) {
            return self.current.inline(row);
        }
        let r = self.current.row(row)?;
        let (l, x) = (r.left?, r.right?);
        let (a, b) = (docs.left.line(l), docs.right.line(x));
        if a == b {
            return None;
        }
        Some(crate::compare::pending_inline(&a, &b))
    }

    /// [`TextComparison::check_invariants`] of `current()`, except INV-TC-8 (importance follows the rules) on
    /// pending rows: the provisional splice keeps a pending row's state and importance from before the edit
    /// until the recompare classifies it (§1.10.10), so a `Same` row whose line was edited is not raw-equal yet.
    /// Every other invariant (rows, runs, sections, statistics, verdicts) holds at every moment. **Not a stable
    /// API** (tests).
    #[doc(hidden)]
    pub fn check_invariants(&self) -> Result<(), hikasa_core::error::InvariantViolation> {
        self.current
            .check_invariants_except(&|row| self.pending.contains(row))
    }

    /// The live data (memory accounting).
    pub fn keys(&self) -> &LiveKeys {
        &self.keys
    }
}

/// Moves the anchors of `side` through `delta` like bookmarks; a range that loses every line is dropped with
/// warning `anchor_dropped`.
fn move_anchors(anchors: &mut Vec<Anchor>, side: Side, d: LineDelta, warnings: &mut Vec<Warning>) {
    let start = |x: u32| -> u32 {
        if x < d.at {
            x
        } else if x >= d.at + d.removed {
            x + d.inserted - d.removed
        } else {
            d.at + (x - d.at).min(d.inserted)
        }
    };
    let map = |r: &std::ops::Range<u32>| start(r.start)..start(r.end).max(start(r.start));
    let is_left = side != Side::Right;
    anchors.retain_mut(|a| {
        let range = match a {
            Anchor::Pair { left, right } => {
                if is_left {
                    left
                } else {
                    right
                }
            }
            Anchor::Isolate { side: s, lines } => {
                if (*s == AnchorSide::Left) != is_left {
                    return true;
                }
                lines
            }
        };
        let moved = map(range);
        if moved.is_empty() {
            warnings.push(Warning {
                code: WarningCode::AnchorDropped,
                message: format!("anchor `{a}` dropped: its lines were removed"),
                spec: None,
            });
            return false;
        }
        *range = moved;
        true
    });
}
