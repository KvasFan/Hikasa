//! The comparison engine (§2.13–§2.17): [`compare`] (keys → intern → align → rows → sections → statistics),
//! the immutable result [`TextComparison`] with O(1) random access by row (gate G5), the progressive
//! [`CompareObserver`] for GUIs (ARCH-004) and its [`PartialRows`].
//!
//! ```text
//! compare(input):
//!   tokenizing  identity path (D-53, rules.is_identity(format) && !force_tokenize): raw lines into per-chunk
//!               arenas, no tokenisation; general path: tokenize_all (parallel speculative chunks) writing the
//!               keys of §2.13.2
//!   aligning    intern_pair (left then right) → TrivialSet of the trivial keys → algorithm per D-36 (Auto:
//!               Histogram for a grammar when |A|·|B| of the stripped middle fits the fallback budget, else
//!               Standard) → line weights when Standard → diff::align with the bounded sample scorer
//!               (score.rs); the RunSink forwards confirmed runs to the observer (partial) and to the progress
//!   classifying rows from the runs (§2.17.1; Equal pairs whose raw text differs and Changed pairs whose raw
//!               text is identical become Changed/Unimportant), sections, statistics, class runs, warnings
//! ```

mod check;
mod inline;
pub(crate) mod keys;
mod nav;
pub(crate) mod rows;
mod score;
pub(crate) mod sections;
mod thumb;

use std::fmt;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

use hikasa_core::cancel::{CancelToken, Cancelled};
use hikasa_core::content::ContentResult;
use hikasa_core::progress::{Ctl, Phase, Unit};
use hikasa_core::side::Side;
use hikasa_core::warning::{Warning, WarningCode};
use hikasa_diff::{
    Algorithm, AlignInput, AlignOptions, Closeness, DiffWarningCode, InlineDiff, InlineOptions,
    Run, RunSink, TokenId, TrivialSet, align, inline_diff, intern_pair,
};
use hikasa_formats::{CompiledFormat, ElementId, LineState, Pattern, RegexLimits, Tokenizer};

use crate::classify::{CharClass, ClassifiedSpan, KeyShape, RuleSet, Span, compile_patterns};
use crate::document::{LineIdx, TextDocument};
use crate::error::TextError;
use crate::rules::{AlgorithmChoice, TextRules};
use crate::stats::TextStats;
use crate::util::{PhaseProgress, ReleaseOnCancel, release_later};
use crate::view::{FilteredView, ViewOptions};

pub use inline::{ClassifiedInline, InlineRange};
pub use nav::Direction;
pub use rows::{Importance, Row, RowIdx, RowState};
pub use sections::{Section, SectionId, SectionKind};
pub use thumb::ThumbCell;

pub(crate) use keys::SideKeys;
use rows::PackedRow;
use sections::ClassRun;

/// Inputs of a comparison. One effective format applies to both sides (D-15).
#[derive(Debug, Clone)]
pub struct CompareInput {
    /// Left document.
    pub left: Arc<TextDocument>,
    /// Right document.
    pub right: Arc<TextDocument>,
    /// Effective format.
    pub format: Arc<CompiledFormat>,
    /// Rules.
    pub rules: TextRules,
    /// True when the raw source bytes were identical (enables the `BinarySame` verdict).
    pub bytes_identical: bool,
    /// Disables the identity fast path (property tests and benches compare both paths; D-53). Default false.
    pub force_tokenize: bool,
}

/// Progressive observer (ARCH-004; the GUI's first screen, ARCH-GUI-003). Called on the thread that runs
/// `compare` (and `Service::text_compare`); implementations must return quickly.
pub trait CompareObserver: Send {
    /// Both documents are loaded (called by `Service::text_compare` before `compare` starts), so a GUI can
    /// render unaligned panes at once.
    fn loaded(&mut self, _left: &Arc<TextDocument>, _right: &Arc<TextDocument>) {}
    /// A phase started (`Tokenizing`, `Aligning`, `Classifying`).
    fn phase(&mut self, _phase: Phase) {}
    /// A confirmed prefix of the alignment (forwarded from `hikasa_diff::RunSink`): `rows.runs` extends the runs
    /// of the previous call; every row covering left lines `< left_done` and right lines `< right_done` is final
    /// (not yet classified). Standard alignment reports progressively; other engines report the common prefix
    /// once and the rest at the end.
    fn partial(&mut self, _rows: &PartialRows<'_>) {}
}

/// Partial result passed to `CompareObserver::partial`.
#[derive(Debug, Clone, Copy)]
pub struct PartialRows<'a> {
    /// New runs, in document order.
    pub runs: &'a [Run],
    /// Left lines whose rows are final.
    pub left_done: u32,
    /// Right lines whose rows are final.
    pub right_done: u32,
}

/// Compares two documents (§2.13–§2.17). Pure apart from `pool` usage, and deterministic for any pool (T2).
/// The observer never changes the result.
///
/// The unimportant-text patterns are compiled with the default [`RegexLimits`]; a session whose registry uses
/// other limits calls [`compare_with_limits`] with them, as [`crate::RulesComparator`] and
/// [`crate::quick_compare_bytes_with`] do, so that every path applies the same limits (TC-016). An invalid
/// pattern is `TextError::Rules`. Cancellation is checked between the phases, every 8,192 lines and every
/// 64 KiB inside a line while keys are built, every 65,536 alignment steps, every 4,096 closeness cells and
/// every 4,096 rows (§5.3), and returns `Err(TextError::Cancelled)`.
pub fn compare(
    input: CompareInput,
    pool: Option<&rayon::ThreadPool>,
    ctl: &Ctl<'_>,
    observer: Option<&mut dyn CompareObserver>,
) -> Result<TextComparison, TextError> {
    compare_with_limits(input, &RegexLimits::default(), pool, ctl, observer)
}

/// [`compare`] with the regex limits of the session's format registry for the unimportant-text patterns.
/// Additive (wave 3).
pub fn compare_with_limits(
    input: CompareInput,
    limits: &RegexLimits,
    pool: Option<&rayon::ThreadPool>,
    ctl: &Ctl<'_>,
    observer: Option<&mut dyn CompareObserver>,
) -> Result<TextComparison, TextError> {
    let patterns = compile_patterns(&input.rules, limits)?;
    let mut observer = observer;
    let prepared = Prepared::new(input, patterns, pool, ctl, observer.as_deref_mut())?;
    prepared.finish(pool, ctl, observer)
}

/// The state after the keys are built (shared by [`compare`] and the quick compare of §2.19, which inspects the
/// keys before deciding whether to align).
pub(crate) struct Prepared {
    pub(crate) left: Arc<TextDocument>,
    pub(crate) right: Arc<TextDocument>,
    pub(crate) format: Arc<CompiledFormat>,
    pub(crate) rules: TextRules,
    pub(crate) set: RuleSet,
    pub(crate) shape: KeyShape,
    pub(crate) keys: [SideKeys; 2],
    pub(crate) bytes_identical: bool,
    /// Warnings of the documents (D-43 cuts of `TextDocument::from_text`).
    pub(crate) doc_warnings: Vec<Warning>,
}

impl Prepared {
    /// Builds both sides' keys (the `tokenizing` phase).
    pub(crate) fn new<'o>(
        input: CompareInput,
        patterns: Arc<[Pattern]>,
        pool: Option<&rayon::ThreadPool>,
        ctl: &Ctl<'_>,
        observer: Option<&mut (dyn CompareObserver + 'o)>,
    ) -> Result<Prepared, TextError> {
        ctl.check()?;
        let CompareInput {
            left,
            right,
            format,
            rules,
            bytes_identical,
            force_tokenize,
        } = input;
        let set = RuleSet::new(&rules, &format, patterns);
        let shape = KeyShape {
            identity: set.all_important && !force_tokenize,
            compare_eol: set.compare_eol,
        };
        if let Some(o) = observer {
            o.phase(Phase::Tokenizing);
        }
        let total = u64::from(left.line_count()) + u64::from(right.line_count());
        let progress = PhaseProgress::start(ctl, Phase::Tokenizing, Unit::Lines, total);
        let job = keys::KeyJob {
            format: &format,
            set: &set,
            shape,
            pool,
            cancel: ctl.cancel,
            progress: &progress,
        };
        let left_keys = SideKeys::build(&left, &job)?;
        let right_keys = match SideKeys::build(&right, &job) {
            Ok(keys) => keys,
            Err(e) => {
                release_later(left_keys);
                return Err(e.into());
            }
        };
        // A key cut short by a cancellation inside a long line is discarded here (off this thread: the arenas
        // are about as large as the text).
        if ctl.cancel.is_cancelled() {
            release_later((left_keys, right_keys));
            return Err(TextError::Cancelled);
        }
        progress.finish();
        let doc_warnings = [&left, &right]
            .into_iter()
            .filter(|d| d.truncated())
            .map(|d| truncation_warning(d))
            .collect();
        Ok(Prepared {
            left,
            right,
            format,
            rules,
            set,
            shape,
            keys: [left_keys, right_keys],
            bytes_identical,
            doc_warnings,
        })
    }

    /// The key sequences are equal (same length, pairwise equal keys; long keys compared in 1 MiB steps).
    pub(crate) fn keys_equal(&self, cancel: &CancelToken) -> Result<bool, Cancelled> {
        let (l, r) = (&self.keys[0].store, &self.keys[1].store);
        if l.len() != r.len() {
            return Ok(false);
        }
        let check = || cancel.check();
        for line in 0..l.len() {
            if line % 65_536 == 0 {
                cancel.check()?;
            }
            if !rows::bytes_equal(l.get(line), r.get(line), &check)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Every line pair `(k, k)` is raw-equal ([`rows::raw_equal`]: text, artificial break, and endings when they
    /// are compared). Requires equal line counts.
    pub(crate) fn raw_equal_pairwise(&self, cancel: &CancelToken) -> Result<bool, Cancelled> {
        if self.left.line_count() != self.right.line_count() {
            return Ok(false);
        }
        let (mut lc, mut rc) = (self.left.cursor(), self.right.cursor());
        let check = || cancel.check();
        for line in 0..self.left.line_count() {
            if line % 65_536 == 0 {
                cancel.check()?;
            }
            let equal = rows::endings_equal(
                self.left.line_ending(LineIdx(line)),
                self.right.line_ending(LineIdx(line)),
                self.shape.compare_eol,
            ) && rows::line_texts_equal(
                &self.left,
                line,
                &self.right,
                line,
                (&mut lc, &mut rc),
                &check,
            )?;
            if !equal {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// True when raw-identical lines always have equal keys (both sides' keys are context-free, see
    /// `SideKeys::context_free`): then no Changed pair is raw-identical, so every Changed row is Important.
    pub(crate) fn keys_context_free(&self, cancel: &CancelToken) -> Result<bool, Cancelled> {
        Ok(
            self.keys[0].context_free(&self.left, &self.format, &self.set, cancel)?
                && self.keys[1].context_free(&self.right, &self.format, &self.set, cancel)?,
        )
    }

    /// Total regex runtime-limit hits of both sides.
    pub(crate) fn regex_errors(&self) -> u64 {
        self.keys[0].regex_errors + self.keys[1].regex_errors
    }

    /// Interns, aligns and classifies (the `aligning` and `classifying` phases).
    pub(crate) fn finish<'o>(
        self,
        pool: Option<&rayon::ThreadPool>,
        ctl: &Ctl<'_>,
        mut observer: Option<&mut (dyn CompareObserver + 'o)>,
    ) -> Result<TextComparison, TextError> {
        ctl.check()?;
        let Prepared {
            left,
            right,
            format,
            rules,
            set,
            shape,
            keys: [left_keys, right_keys],
            bytes_identical,
            doc_warnings,
        } = self;
        let mut regex_errors = left_keys.regex_errors + right_keys.regex_errors;
        // The key arenas are about as large as the text: a cancellation frees them off this thread.
        let mut left_keys = ReleaseOnCancel::new(left_keys, ctl.cancel);
        let mut right_keys = ReleaseOnCancel::new(right_keys, ctl.cancel);

        // Aligning: intern (left then right), trivial set, algorithm (D-36), weights, align.
        if let Some(o) = observer.as_deref_mut() {
            o.phase(Phase::Aligning);
        }
        let n = u64::from(left.line_count()) + u64::from(right.line_count());
        let progress = PhaseProgress::start(ctl, Phase::Aligning, Unit::Items, n);
        // intern_pair has no cancellation token: the key iterators end early when the token fires (checked
        // every 65,536 keys), and the result is then discarded (§5.3, G4). Long keys are interned through short
        // surrogates of their exact equivalence classes, so no single key is hashed in one long call.
        let surrogates = long_key_surrogates(
            [&left_keys.value.store, &right_keys.value.store],
            ctl.cancel,
        )?;
        let pair = intern_pair(
            Cancellable::new(
                interned_keys(&left_keys.value.store, &surrogates[0]),
                ctl.cancel,
            ),
            Cancellable::new(
                interned_keys(&right_keys.value.store, &surrogates[1]),
                ctl.cancel,
            ),
        )?;
        drop(surrogates);
        ctl.check()?;
        let mut trivial = TrivialSet::with_capacity(pair.distinct);
        for (ids, keys) in [
            (&pair.left, &left_keys.value),
            (&pair.right, &right_keys.value),
        ] {
            for (line, &id) in ids.iter().enumerate() {
                if line % 65_536 == 0 {
                    ctl.check()?;
                }
                if shape.is_trivial(keys.store.get(line as u32)) {
                    trivial.insert(id);
                }
            }
        }
        let algorithm = resolve_algorithm(&rules, &format, &pair.left, &pair.right, ctl.cancel)?;
        let (left_weights, right_weights) = if algorithm == Algorithm::Standard {
            let (lw, le) = keys::line_weights(&left, &format, pool, ctl.cancel)?;
            let (rw, re) = keys::line_weights(&right, &format, pool, ctl.cancel)?;
            regex_errors += le + re;
            (lw, rw)
        } else {
            (Vec::new(), Vec::new())
        };
        let a = &rules.alignment;
        let opts = AlignOptions {
            algorithm,
            skew_tolerance: a.skew_tolerance,
            closeness: a.closeness.then_some(Closeness {
                threshold_permille: a.closeness_threshold_permille,
                ..Closeness::DEFAULT
            }),
            never_align_differences: a.never_align_differences,
            work_budget: None,
        };
        let input = AlignInput {
            left: &pair.left,
            right: &pair.right,
            left_weights: &left_weights,
            right_weights: &right_weights,
            trivial: &trivial,
            distinct: pair.distinct,
        };
        let scorer = score::ProfileScorer::new(
            &left_keys.value.store,
            &right_keys.value.store,
            shape,
            a.closeness_threshold_permille,
            ctl.cancel,
        );
        let mut sink = ObserverSink {
            observer: observer.as_deref_mut(),
            progress: &progress,
        };
        let alignment = align(&input, &opts, Some(&scorer), Some(&mut sink), ctl.cancel)?;
        let scorer_exhausted = scorer.exhausted();
        drop(scorer);
        drop(left_weights);
        drop(right_weights);
        progress.finish();

        // Classifying: rows, raw-text checks of the pairs, sections, statistics.
        if let Some(o) = observer {
            o.phase(Phase::Classifying);
        }
        let row_total = alignment.row_count();
        let progress = PhaseProgress::start(ctl, Phase::Classifying, Unit::Lines, row_total);
        let left_trivial = |l: u32| {
            pair.left
                .get(l as usize)
                .is_some_and(|&id| trivial.contains(id))
        };
        let right_trivial = |r: u32| {
            pair.right
                .get(r as usize)
                .is_some_and(|&id| trivial.contains(id))
        };
        let check = || ctl.check();
        let mut rows = rows::expand(
            alignment.runs(),
            &left_trivial,
            &right_trivial,
            rules.importance.orphans_always_important,
            &check,
        )?;
        drop(pair);
        drop(trivial);
        let (left_states, right_states) = (
            std::mem::take(&mut left_keys.value.start_states),
            std::mem::take(&mut right_keys.value.start_states),
        );
        drop(std::mem::take(&mut left_keys.value.store));
        drop(std::mem::take(&mut right_keys.value.store));
        if shape.identity {
            // Identity keys are the raw lines plus their end markers: equal keys are raw-equal lines and
            // different keys are not raw-identical, so no row changes (§2.17.1).
            progress.add(rows.len() as u64);
        } else {
            rows::check_raw_equality(
                &mut rows,
                &left,
                &right,
                shape.compare_eol,
                pool,
                &check,
                &progress,
            )?;
        }
        let derived = sections::derive(
            &rows,
            left.line_count(),
            right.line_count(),
            format.text().lines_independent,
        );
        progress.finish();

        let mut warnings = Vec::new();
        for doc in [&left, &right] {
            let spec = doc.origin().spec.as_ref().map(|s| s.canonical());
            if doc.binary().is_binary {
                warnings.push(Warning {
                    code: WarningCode::BinaryContent,
                    message: format!(
                        "{}: binary content ({}); the verdict is a binary comparison (D-12)",
                        doc.origin().display(),
                        binary_reason(doc)
                    ),
                    spec: spec.clone(),
                });
            }
            if doc.malformed() > 0 {
                warnings.push(Warning {
                    code: WarningCode::LossyDecode,
                    message: format!(
                        "{}: {} malformed byte sequence(s) replaced with U+FFFD while decoding as {}",
                        doc.origin().display(),
                        doc.malformed(),
                        doc.encoding().encoding
                    ),
                    spec,
                });
            }
        }
        warnings.extend(doc_warnings);
        if regex_errors > 0 {
            warnings.push(Warning {
                code: WarningCode::RegexRuntimeError,
                message: format!(
                    "{regex_errors} regular-expression match attempt(s) hit the backtracking limit and counted as no match"
                ),
                spec: None,
            });
        }
        warnings.extend(alignment.warnings().iter().cloned());
        if scorer_exhausted
            && !warnings
                .iter()
                .any(|w| w.code == WarningCode::ClosenessBudgetExhausted)
        {
            warnings.push(Warning {
                code: WarningCode::ClosenessBudgetExhausted,
                message: format!(
                    "the closeness scorer's work budget ({} units) ran out; the remaining differing lines were \
                     left unpaired",
                    score::scorer_budget(
                        u64::from(left.line_count()),
                        u64::from(right.line_count())
                    )
                ),
                spec: None,
            });
        }
        for w in alignment.diff_warnings() {
            warnings.push(diff_warning(w.code, &w.message));
        }

        let runs = alignment.runs().to_vec();
        let mut run_rows = Vec::with_capacity(runs.len());
        let mut at = 0u32;
        for run in &runs {
            run_rows.push(at);
            at += run.rows();
        }
        let cmp = TextComparison {
            start_states: [state_cell(left_states), state_cell(right_states)],
            left,
            right,
            format,
            rules,
            set,
            algorithm,
            identity_path: shape.identity,
            bytes_identical,
            runs,
            run_rows,
            rows,
            sections: derived.sections,
            class_runs: derived.class_runs,
            stats: derived.stats,
            warnings,
        };
        debug_assert_eq!(cmp.check_invariants(), Ok(()));
        Ok(cmp)
    }
}

/// The keys of one side as interned: the surrogate of a long key, else the key itself.
fn interned_keys<'a>(
    store: &'a keys::KeyStore,
    surrogates: &'a std::collections::BTreeMap<u32, Vec<u8>>,
) -> impl ExactSizeIterator<Item = &'a [u8]> + 'a {
    (0..store.len()).map(move |line| {
        surrogates
            .get(&line)
            .map_or_else(|| store.get(line), Vec::as_slice)
    })
}

/// Keys of at least this many bytes are interned through surrogates (see [`long_key_surrogates`]).
const LONG_KEY_BYTES: usize = 64 * 1024;

/// For every key of at least [`LONG_KEY_BYTES`] bytes on either side, a short surrogate that is equal for two such
/// keys exactly when the keys are equal: `0xFC varint(class)`, the classes numbered in first-occurrence order (left
/// then right), so the interned ids stay in first-occurrence order (INV-INTERN-1). No other key can start with 0xFC
/// (text bytes are below 0xF8; a key starts with text or with the end markers 0xFD/0xFE), and every long key is
/// longer than any surrogate. Fingerprints and exact comparisons run in 1 MiB steps with cancellation checks, so
/// interning never hashes a multi-megabyte key in one uninterruptible call (§5.3).
fn long_key_surrogates(
    stores: [&keys::KeyStore; 2],
    cancel: &CancelToken,
) -> Result<[std::collections::BTreeMap<u32, Vec<u8>>; 2], Cancelled> {
    let check = || cancel.check();
    let mut out = [
        std::collections::BTreeMap::new(),
        std::collections::BTreeMap::new(),
    ];
    // (length, fingerprint) → classes; a class is represented by its first key.
    let mut buckets: std::collections::HashMap<(usize, u64), Vec<u32>> =
        std::collections::HashMap::new();
    let mut representatives: Vec<(usize, u32)> = Vec::new();
    for (side, store) in stores.into_iter().enumerate() {
        for line in 0..store.len() {
            if line % 65_536 == 0 {
                cancel.check()?;
            }
            let key = store.get(line);
            if key.len() < LONG_KEY_BYTES {
                continue;
            }
            let print = fingerprint(key, cancel)?;
            let candidates = buckets.entry((key.len(), print)).or_default();
            let mut class = None;
            for &c in candidates.iter() {
                let (rs, rl) = representatives[c as usize];
                if rows::bytes_equal(stores[rs].get(rl), key, &check)? {
                    class = Some(c);
                    break;
                }
            }
            let class = match class {
                Some(c) => c,
                None => {
                    let c = representatives.len() as u32;
                    representatives.push((side, line));
                    candidates.push(c);
                    c
                }
            };
            let mut surrogate = vec![0xFC];
            let mut v = class;
            loop {
                let byte = (v & 0x7F) as u8;
                v >>= 7;
                if v == 0 {
                    surrogate.push(byte);
                    break;
                }
                surrogate.push(byte | 0x80);
            }
            out[side].insert(line, surrogate);
        }
    }
    Ok(out)
}

/// A deterministic 64-bit fingerprint of `key` (word-wise multiply-xor), computed in 1 MiB steps with cancellation
/// checks. Only groups candidates: equality is always decided by an exact comparison.
fn fingerprint(key: &[u8], cancel: &CancelToken) -> Result<u64, Cancelled> {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for (k, block) in key.chunks(1 << 20).enumerate() {
        if k > 0 {
            cancel.check()?;
        }
        let (words, rest) = block.as_chunks::<8>();
        for w in words {
            h = (h ^ u64::from_le_bytes(*w))
                .wrapping_mul(0x0000_0100_0000_01B3)
                .rotate_left(23);
        }
        for &b in rest {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    Ok(h)
}

/// The engine of D-36 as amended in wave 3: an explicit choice as given; `Auto` is Standard without a grammar,
/// and for source code Histogram only while the estimated cost of its stripped middle, `|A|·|B|` cells, fits
/// the fallback budget of hikasa-diff (`AlignOptions::fallback_budget_for(n, m)`, §1.3.3 item 3), else the
/// bounded Standard. Deterministic: only lengths decide.
fn resolve_algorithm(
    rules: &TextRules,
    format: &CompiledFormat,
    left: &[TokenId],
    right: &[TokenId],
    cancel: &CancelToken,
) -> Result<Algorithm, Cancelled> {
    let nominal = rules.algorithm_for(format);
    if rules.alignment.algorithm != AlgorithmChoice::Auto || nominal != Algorithm::Histogram {
        return Ok(nominal);
    }
    let (a, b) = stripped_middle(left, right, cancel)?;
    let cells = (a as u64).saturating_mul(b as u64);
    Ok(
        if cells <= AlignOptions::fallback_budget_for(left.len(), right.len()) {
            Algorithm::Histogram
        } else {
            Algorithm::Standard
        },
    )
}

/// Lengths of the two sequences after stripping their common prefix and suffix (cancellation checked every
/// 65,536 tokens).
fn stripped_middle(
    left: &[TokenId],
    right: &[TokenId],
    cancel: &CancelToken,
) -> Result<(usize, usize), Cancelled> {
    let mut prefix = 0usize;
    for (x, y) in left.iter().zip(right) {
        if prefix.is_multiple_of(65_536) {
            cancel.check()?;
        }
        if x != y {
            break;
        }
        prefix += 1;
    }
    let (l, r) = (&left[prefix..], &right[prefix..]);
    let mut suffix = 0usize;
    for (x, y) in l.iter().rev().zip(r.iter().rev()) {
        if suffix.is_multiple_of(65_536) {
            cancel.check()?;
        }
        if x != y {
            break;
        }
        suffix += 1;
    }
    Ok((l.len() - suffix, r.len() - suffix))
}

/// The core warning of a diff warning (§1.3.3: hikasa-text maps every entry). `DiffWarningCode` is
/// non-exhaustive: a code added later that has no core counterpart yet is reported as `alignment_degraded`
/// (the generic "the alignment is not the full result of the chosen algorithm" warning) with its own code
/// named in the message, never dropped.
fn diff_warning(code: DiffWarningCode, message: &str) -> Warning {
    if code == DiffWarningCode::AlignmentDegraded {
        Warning {
            code: WarningCode::AlignmentDegraded,
            message: message.to_owned(),
            spec: None,
        }
    } else {
        Warning {
            code: WarningCode::AlignmentDegraded,
            message: format!("{code}: {message}"),
            spec: None,
        }
    }
}

/// The warning of a document cut at the D-43 limits by [`TextDocument::from_text`].
fn truncation_warning(doc: &TextDocument) -> Warning {
    Warning {
        code: WarningCode::LossyDecode,
        message: format!(
            "{}: text beyond the document limits (D-43: 4 GiB − 1 bytes, {} lines) was cut; the comparison \
             covers the first {} bytes only",
            doc.origin().display(),
            crate::MAX_LINES,
            doc.rope().len_bytes()
        ),
        spec: None,
    }
}

/// An iterator that ends early once `cancel` fires (checked every 65,536 items). Its size hint keeps claiming
/// the full remaining length, so a consumer that pre-sizes from it (`intern_pair`) is not misled; the result of
/// an iteration that ended early is discarded by the caller, which checks the token afterwards.
struct Cancellable<'c, I> {
    inner: I,
    cancel: &'c CancelToken,
    seen: usize,
}

impl<'c, I: ExactSizeIterator> Cancellable<'c, I> {
    fn new(inner: I, cancel: &'c CancelToken) -> Self {
        Cancellable {
            inner,
            cancel,
            seen: 0,
        }
    }
}

impl<I: ExactSizeIterator> Iterator for Cancellable<'_, I> {
    type Item = I::Item;

    #[inline]
    fn next(&mut self) -> Option<I::Item> {
        if self.seen & 0xFFFF == 0xFFFF && self.cancel.is_cancelled() {
            return None;
        }
        self.seen += 1;
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

/// The start-state cache of one side (pre-filled when the general path computed the states).
fn state_cell(states: Vec<LineState>) -> OnceLock<Vec<LineState>> {
    let cell = OnceLock::new();
    if !states.is_empty() {
        let _ = cell.set(states);
    }
    cell
}

/// The rule that classified a document binary (warning text).
fn binary_reason(doc: &TextDocument) -> String {
    match doc.binary().reason {
        hikasa_formats::BinaryReason::Magic { kind } => format!("{kind:?} signature"),
        hikasa_formats::BinaryReason::NulByte { offset } => format!("NUL byte at offset {offset}"),
        hikasa_formats::BinaryReason::ControlRatio { permille } => {
            format!("{permille}‰ control bytes")
        }
        other => format!("{other:?}"),
    }
}

/// Forwards confirmed runs to the observer and the `aligning` progress (§5.4: done = left + right lines
/// confirmed).
struct ObserverSink<'s, 'o, 'p> {
    observer: Option<&'s mut (dyn CompareObserver + 'o)>,
    progress: &'p PhaseProgress<'p>,
}

impl RunSink for ObserverSink<'_, '_, '_> {
    fn runs_ready(&mut self, runs: &[Run]) {
        let Some(last) = runs.last() else {
            return;
        };
        let (left_done, right_done) = (last.left.end, last.right.end);
        if let Some(o) = self.observer.as_deref_mut() {
            o.partial(&PartialRows {
                runs,
                left_done,
                right_done,
            });
        }
        self.progress
            .set(u64::from(left_done) + u64::from(right_done));
    }
}

/// Immutable comparison result. `Send + Sync`; share via `Arc`. INV-TC-1..8 (Appendix B).
pub struct TextComparison {
    pub(crate) left: Arc<TextDocument>,
    pub(crate) right: Arc<TextDocument>,
    pub(crate) format: Arc<CompiledFormat>,
    pub(crate) rules: TextRules,
    pub(crate) set: RuleSet,
    pub(crate) algorithm: Algorithm,
    pub(crate) identity_path: bool,
    pub(crate) bytes_identical: bool,
    pub(crate) runs: Vec<Run>,
    /// First row of every run.
    pub(crate) run_rows: Vec<u32>,
    pub(crate) rows: Vec<PackedRow>,
    pub(crate) sections: Vec<Section>,
    pub(crate) class_runs: Vec<ClassRun>,
    pub(crate) stats: TextStats,
    pub(crate) warnings: Vec<Warning>,
    /// Tokenizer start states per side (general path: filled at construction for multi-line grammars;
    /// identity path: computed by `prepare_spans`).
    pub(crate) start_states: [OnceLock<Vec<LineState>>; 2],
}

impl fmt::Debug for TextComparison {
    /// A summary (never the rows or the text).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextComparison")
            .field("left", &self.left)
            .field("right", &self.right)
            .field("format", &self.format.name())
            .field("algorithm", &self.algorithm)
            .field("identity_path", &self.identity_path)
            .field("bytes_identical", &self.bytes_identical)
            .field("runs", &self.runs.len())
            .field("stats", &self.stats)
            .field("warnings", &self.warnings)
            .finish_non_exhaustive()
    }
}

impl TextComparison {
    /// Number of rows.
    pub fn row_count(&self) -> u32 {
        self.rows.len() as u32
    }

    /// Row `idx` (O(1), < 200 ns: gate G5). Panics never: out-of-range returns None.
    #[inline]
    pub fn row(&self, idx: RowIdx) -> Option<Row> {
        self.rows.get(idx.0 as usize).map(|r| r.unpack())
    }

    /// Rows in a range (clamped).
    pub fn rows(&self, range: Range<u32>) -> impl Iterator<Item = Row> + '_ {
        let range = rows::clamp(range, self.row_count());
        self.rows[range.start as usize..range.end as usize]
            .iter()
            .map(|r| r.unpack())
    }

    /// Document of `side` (Left or Right; `Center`/`Output` return the left document in M1).
    pub fn document(&self, side: Side) -> &Arc<TextDocument> {
        match side {
            Side::Right => &self.right,
            Side::Left | Side::Center | Side::Output => &self.left,
        }
    }

    /// Effective format.
    pub fn format(&self) -> &Arc<CompiledFormat> {
        &self.format
    }

    /// Rules used.
    pub fn rules(&self) -> &TextRules {
        &self.rules
    }

    /// Algorithm actually used (after resolving Auto).
    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }

    /// The alignment runs in document order (the stable run ids of ARCH-050 are their indices). Additive
    /// (wave 3): reports and golden files serialise them.
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// True when the raw source bytes were identical. Additive (wave 3).
    pub fn bytes_identical(&self) -> bool {
        self.bytes_identical
    }

    /// True when the identity fast path built the keys (D-53). Additive (wave 3; benches and tests).
    pub fn used_identity_path(&self) -> bool {
        self.identity_path
    }

    /// Classified inline differences for a two-sided non-Same row (lazy, pure; None otherwise; p99 < 50 µs:
    /// gate G5).
    ///
    /// On a `Changed`/`Unimportant` row every range is unimportant (its keys are equal, so no important char
    /// differs; TC-014). Lines longer than 40,000 bytes (so over the 10,000-char inline limit) get their
    /// whole-line ranges from a byte-wise prefix/suffix scan, and their classification looks at the first
    /// [`SPAN_LIMIT_BYTES`] of each line only (the rest counts as important), so one call stays bounded on
    /// lines of any length.
    pub fn inline(&self, idx: RowIdx) -> Option<ClassifiedInline> {
        let row = self.row(idx)?;
        if row.state == RowState::Same {
            return None;
        }
        let (l, r) = (row.left?, row.right?);
        let a = self.left.line(l);
        let b = self.right.line(r);
        let diff = inline_ranges(&a, &b);
        if row.importance == Importance::Unimportant {
            return Some(inline::all_unimportant(&a, &b, &diff));
        }
        if self.set.all_important {
            return Some(inline::classify(&a, &b, &diff, None, None));
        }
        let ls = self.line_spans(Side::Left, l, &a);
        let rs = self.line_spans(Side::Right, r, &b);
        Some(inline::classify(&a, &b, &diff, Some(&ls), Some(&rs)))
    }

    /// Computes the per-line tokenizer start states of `side` when the identity fast path skipped
    /// tokenisation and the grammar has multi-line items (one sequential pass, cached). GUIs call it on a worker
    /// before the first `spans` call; `spans` calls it implicitly otherwise.
    pub fn prepare_spans(&self, side: Side, cancel: &CancelToken) -> Result<(), Cancelled> {
        let doc = self.document(side);
        let cell = self.state_cell(side);
        if !self.format.grammar().has_multiline() || doc.line_count() == 0 || cell.get().is_some() {
            return Ok(());
        }
        let grammar = self.format.grammar();
        let mut tokenizer = Tokenizer::new(grammar);
        let mut tokens = Vec::new();
        let mut states = Vec::with_capacity(doc.line_count() as usize);
        let mut state = LineState::Normal;
        let mut cursor = doc.cursor();
        for line in 0..doc.line_count() {
            if line % keys::CHUNK_LINES == 0 {
                cancel.check()?;
            }
            states.push(state);
            let text = cursor.line(line);
            state = tokenizer
                .tokenize_line_cancellable(text, line, state, &mut tokens, cancel)?
                .end_state;
        }
        // Another thread may have filled the cell meanwhile; both computed the same states.
        let _ = cell.set(states);
        Ok(())
    }

    /// Spans of a line (re-tokenises from the stored start state; O(line length) once prepared).
    ///
    /// Bounded on very long lines: only the first [`SPAN_LIMIT_BYTES`] of a line are tokenised and classified
    /// (a token cut by that limit is classified as the tokenizer sees the shortened text); the rest of the line
    /// is one span of `Everything Else`, class `Other`, important.
    pub fn spans(&self, side: Side, line: LineIdx) -> Vec<ClassifiedSpan> {
        let doc = self.document(side);
        if line.0 >= doc.line_count() {
            return Vec::new();
        }
        let text = doc.line(line);
        self.line_spans(side, line, &text)
            .into_iter()
            .map(|s| ClassifiedSpan {
                range: s.start..s.end,
                element: s.element,
                class: s.class,
                important: s.important,
            })
            .collect()
    }

    /// The start-state cell of `side`.
    fn state_cell(&self, side: Side) -> &OnceLock<Vec<LineState>> {
        match side {
            Side::Right => &self.start_states[1],
            Side::Left | Side::Center | Side::Output => &self.start_states[0],
        }
    }

    /// Start state of `line` of `side` (computing the states first when needed).
    fn start_state(&self, side: Side, line: u32) -> LineState {
        if !self.format.grammar().has_multiline() {
            return LineState::Normal;
        }
        if self.state_cell(side).get().is_none() {
            // Never cancelled: the result is cached for later calls.
            let _ = self.prepare_spans(side, &CancelToken::never());
        }
        self.state_cell(side)
            .get()
            .and_then(|states| states.get(line as usize).copied())
            .unwrap_or(LineState::Normal)
    }

    /// Classified spans of `line` (whose text is `text`) of `side`, bounded as [`TextComparison::spans`] says.
    pub(crate) fn line_spans(&self, side: Side, line: LineIdx, text: &str) -> Vec<Span> {
        let state = self.start_state(side, line.0);
        let mut cut = text.len().min(SPAN_LIMIT_BYTES);
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        let head = &text[..cut];
        let mut tokens = Vec::new();
        Tokenizer::new(self.format.grammar()).tokenize_line(head, line.0, state, &mut tokens);
        let mut errors = 0;
        let mut spans = self
            .set
            .classify(head, &tokens, &CancelToken::never(), &mut errors);
        if cut < text.len() {
            spans.push(Span {
                start: cut as u32,
                end: text.len() as u32,
                element: ElementId::EVERYTHING_ELSE,
                class: CharClass::Other,
                important: true,
                case_sensitive: true,
            });
        }
        spans
    }

    /// Sections in row order.
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// Thumbnail (TC-032): `buckets` cells covering the rows evenly (bucket `b` covers rows
    /// `[b·rows/buckets, (b+1)·rows/buckets)`), built from the class runs (maximal runs of non-Same rows of one
    /// state and importance) in O(runs + buckets). With `ignore_unimportant`, unimportant rows count as SAME
    /// (§2.18 `cat`). A bucket that covers no row (more buckets than rows) is empty.
    pub fn thumbnail(&self, buckets: u32, ignore_unimportant: bool) -> Vec<ThumbCell> {
        thumb::thumbnail(
            self.row_count(),
            &self.class_runs,
            buckets,
            ignore_unimportant,
        )
    }

    /// Statistics (§2.17.3).
    pub fn stats(&self) -> &TextStats {
        &self.stats
    }

    /// Verdict (§2.17.5); `ignore_unimportant` maps UnimportantOnly to RulesSame.
    pub fn verdict(&self, ignore_unimportant: bool) -> ContentResult {
        verdict_of(
            self.bytes_identical,
            self.left.binary().is_binary || self.right.binary().is_binary,
            self.stats.important_rows,
            self.stats.unimportant_rows,
            ignore_unimportant,
        )
    }

    /// Filtered view (§2.18).
    pub fn view(&self, opts: &ViewOptions) -> FilteredView {
        FilteredView::build(self.row_count(), &self.class_runs, opts)
    }

    /// Warnings of the comparison: binary content, lossy decode (also a document cut by
    /// `TextDocument::from_text`), regex runtime limits, budgets (work, closeness, alignment degraded) and
    /// closeness ignored. `format_mismatch` is not among them: one effective format is an input of `compare`, so
    /// the caller that chooses it (`Service::text_compare`, [`crate::RulesComparator`]) emits that warning (D-15).
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }
}

/// Bytes of a line that [`TextComparison::spans`] and the inline classification tokenise and classify; the rest
/// of a longer line is one important span.
pub const SPAN_LIMIT_BYTES: usize = 256 * 1024;

/// `hikasa_diff::inline_diff` with the default options, except that a line longer than 4 × 10,000 bytes (so
/// certainly over the 10,000-char limit, where `inline_diff` reports the whole differing middle) gets those
/// whole-line ranges from a block-wise byte scan: the same result, without per-char iteration over megabytes.
fn inline_ranges(a: &str, b: &str) -> InlineDiff {
    let opts = InlineOptions::DEFAULT;
    let long = opts.max_line_chars as usize * 4;
    if a.len() <= long && b.len() <= long {
        return inline_diff(a, b, &opts);
    }
    if a == b {
        return InlineDiff::default();
    }
    let (x, y) = (a.as_bytes(), b.as_bytes());
    // The common char prefix: the common byte prefix, backed off to a char boundary (equal bytes before it, so
    // the boundary is one on both sides).
    let mut prefix = common_prefix(x, y);
    while !a.is_char_boundary(prefix) {
        prefix -= 1;
    }
    // The common char suffix of the remainders: the common byte suffix, moved forward to a char boundary.
    let suffix_bytes = common_suffix(&x[prefix..], &y[prefix..]);
    let mut start = a.len() - suffix_bytes;
    while !a.is_char_boundary(start) {
        start += 1;
    }
    let suffix = a.len() - start;
    let mut out = InlineDiff {
        whole_line: true,
        ..InlineDiff::default()
    };
    if prefix < a.len() - suffix {
        out.left.push(prefix as u32..(a.len() - suffix) as u32);
    }
    if prefix < b.len() - suffix {
        out.right.push(prefix as u32..(b.len() - suffix) as u32);
    }
    out
}

/// Length of the common prefix of two byte strings (compared in blocks, then per byte).
fn common_prefix(x: &[u8], y: &[u8]) -> usize {
    const BLOCK: usize = 4096;
    let n = x.len().min(y.len());
    let mut at = 0;
    while at + BLOCK <= n && x[at..at + BLOCK] == y[at..at + BLOCK] {
        at += BLOCK;
    }
    while at < n && x[at] == y[at] {
        at += 1;
    }
    at
}

/// Length of the common suffix of two byte strings (compared in blocks, then per byte).
fn common_suffix(x: &[u8], y: &[u8]) -> usize {
    const BLOCK: usize = 4096;
    let n = x.len().min(y.len());
    let (xl, yl) = (x.len(), y.len());
    let mut at = 0;
    while at + BLOCK <= n && x[xl - at - BLOCK..xl - at] == y[yl - at - BLOCK..yl - at] {
        at += BLOCK;
    }
    while at < n && x[xl - at - 1] == y[yl - at - 1] {
        at += 1;
    }
    at
}

/// The verdict rules of §2.17.5 (shared with the quick compare).
pub(crate) fn verdict_of(
    bytes_identical: bool,
    binary: bool,
    important_rows: u64,
    unimportant_rows: u64,
    ignore_unimportant: bool,
) -> ContentResult {
    if bytes_identical {
        ContentResult::BinarySame
    } else if binary {
        ContentResult::BinaryDifferent
    } else if important_rows == 0 && unimportant_rows == 0 {
        ContentResult::RulesSame
    } else if important_rows > 0 {
        ContentResult::ImportantDifferences
    } else if ignore_unimportant {
        ContentResult::RulesSame
    } else {
        ContentResult::UnimportantOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_are_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<TextComparison>();
        check::<FilteredView>();
    }

    fn input(left: TextDocument, right: TextDocument) -> CompareInput {
        CompareInput {
            left: Arc::new(left),
            right: Arc::new(right),
            format: Arc::new(CompiledFormat::everything_else()),
            rules: TextRules::default(),
            bytes_identical: false,
            force_tokenize: false,
        }
    }

    /// Conformance finding: a document cut by `from_text` is reported (`lossy_decode`), never silent.
    #[test]
    fn a_truncated_document_is_reported() {
        let cmp = compare(
            input(
                TextDocument::from_text("a\n").mark_truncated(),
                TextDocument::from_text("a\n"),
            ),
            None,
            &Ctl::detached(),
            None,
        )
        .unwrap();
        let w: Vec<&Warning> = cmp
            .warnings()
            .iter()
            .filter(|w| w.code == WarningCode::LossyDecode)
            .collect();
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].message.contains("D-43"), "{}", w[0].message);
        let plain = compare(
            input(
                TextDocument::from_text("a\n"),
                TextDocument::from_text("b\n"),
            ),
            None,
            &Ctl::detached(),
            None,
        )
        .unwrap();
        assert!(plain.warnings().is_empty());
    }

    /// The direct whole-line scan equals `inline_diff` on long lines of every shape.
    #[test]
    fn inline_ranges_equal_inline_diff_on_long_lines() {
        let mut seed = 99u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let pieces = ["a", "é", "€", "😀", "e\u{301}", " "];
        for _ in 0..40 {
            let n = 10_001 + (next() % 3_000) as usize;
            let base: String = (0..n).map(|_| pieces[(next() % 6) as usize]).collect();
            let mut other: Vec<char> = base.chars().collect();
            for _ in 0..(next() % 4) {
                let at = (next() as usize) % other.len();
                match next() % 3 {
                    0 => other[at] = 'Z',
                    1 => other.insert(at, '€'),
                    _ => {
                        other.remove(at);
                    }
                }
            }
            let other: String = other.into_iter().collect();
            let (a, b) = if next() % 2 == 0 {
                (base.repeat(4), other.repeat(4))
            } else {
                (base.clone(), other)
            };
            let reference = inline_diff(&a, &b, &InlineOptions::DEFAULT);
            assert_eq!(inline_ranges(&a, &b), reference);
        }
        assert_eq!(inline_ranges("x", "x"), InlineDiff::default());
    }

    /// Long keys are interned through surrogates: equal long keys (on either side) share one surrogate, different
    /// ones never do, and the classes follow first occurrence; the comparison treats them exactly like keys.
    #[test]
    fn long_keys_are_interned_through_exact_surrogates() {
        let a = "a".repeat(LONG_KEY_BYTES + 5);
        let b = "b".repeat(LONG_KEY_BYTES + 5);
        let a2 = format!("{}x", &a[..a.len() - 1]);
        let left = format!("{a}\nshort\n{b}\n");
        let right = format!("{b}\n{a2}\n{a}\nshort\n");
        let cmp = compare(
            CompareInput {
                rules: {
                    let mut r = TextRules::default();
                    r.alignment.algorithm = crate::AlgorithmChoice::Myers;
                    r.alignment.closeness = false;
                    r
                },
                ..input(
                    TextDocument::from_text(&left),
                    TextDocument::from_text(&right),
                )
            },
            None,
            &Ctl::detached(),
            None,
        )
        .unwrap();
        // The common subsequence [a, short] (or [b]) is found exactly as with the plain keys.
        let same: Vec<(u32, u32)> = cmp
            .rows(0..cmp.row_count())
            .filter(|r| r.state == RowState::Same)
            .map(|r| (r.left.unwrap().0, r.right.unwrap().0))
            .collect();
        assert_eq!(same, [(0, 2), (1, 3)]);
        // Surrogates directly: classes in first-occurrence order, equal exactly for equal keys.
        let never = CancelToken::never();
        let doc = |t: &str| Arc::new(TextDocument::from_text(t));
        let rules = TextRules::default();
        let format = CompiledFormat::everything_else();
        let set = RuleSet::new(&rules, &format, Arc::from(Vec::new()));
        let shape = KeyShape {
            identity: true,
            compare_eol: false,
        };
        let progress_ctl = Ctl::detached();
        let progress = PhaseProgress::start(&progress_ctl, Phase::Tokenizing, Unit::Lines, 0);
        let job = keys::KeyJob {
            format: &format,
            set: &set,
            shape,
            pool: None,
            cancel: &never,
            progress: &progress,
        };
        let (l, r) = (doc(&left), doc(&right));
        let lk = SideKeys::build(&l, &job).unwrap();
        let rk = SideKeys::build(&r, &job).unwrap();
        let s = long_key_surrogates([&lk.store, &rk.store], &never).unwrap();
        assert_eq!(s[0].get(&0), Some(&vec![0xFC, 0]));
        assert_eq!(s[0].get(&1), None);
        assert_eq!(s[0].get(&2), Some(&vec![0xFC, 1]));
        assert_eq!(s[1].get(&0), Some(&vec![0xFC, 1]));
        assert_eq!(s[1].get(&1), Some(&vec![0xFC, 2]));
        assert_eq!(s[1].get(&2), Some(&vec![0xFC, 0]));
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(long_key_surrogates([&lk.store, &rk.store], &cancel).is_err());
    }

    /// Unknown diff warning codes are never dropped.
    #[test]
    fn diff_warnings_are_mapped() {
        let w = diff_warning(DiffWarningCode::AlignmentDegraded, "m");
        assert_eq!(
            (w.code, w.message.as_str()),
            (WarningCode::AlignmentDegraded, "m")
        );
    }

    #[test]
    fn verdict_rules() {
        use ContentResult::*;
        assert_eq!(verdict_of(true, true, 5, 5, false), BinarySame);
        assert_eq!(verdict_of(false, true, 0, 0, false), BinaryDifferent);
        assert_eq!(verdict_of(false, false, 0, 0, false), RulesSame);
        assert_eq!(verdict_of(false, false, 1, 3, true), ImportantDifferences);
        assert_eq!(verdict_of(false, false, 0, 3, false), UnimportantOnly);
        assert_eq!(verdict_of(false, false, 0, 3, true), RulesSame);
    }
}
