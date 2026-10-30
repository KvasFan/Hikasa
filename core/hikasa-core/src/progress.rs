//! Progress reports, sinks and [`Ctl`] (ARCH-003, §5.4).
//!
//! Engines report through [`Ctl::progress`], which builds a [`Progress`] only when the sink's throttle is open
//! ([`ProgressSink::due`]), so hot loops allocate no `current` string between ticks. Progress never affects
//! results (T1).

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::cancel::{CancelToken, Cancelled};

/// What a job is doing (for status bars and stderr progress).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Phase {
    /// Reading input bytes.
    Reading,
    /// Decoding text.
    Decoding,
    /// Building line keys.
    Tokenizing,
    /// Aligning.
    Aligning,
    /// Building rows, sections and statistics.
    Classifying,
    /// Listing folders.
    Scanning,
    /// Running content tests.
    ComparingContent,
    /// Computing CRCs.
    Hashing,
    /// Rendering a report.
    Reporting,
    /// Writing files (VFS write operations, M2 W3; additive).
    Writing,
}

impl Phase {
    /// Number of phases (size of per-phase tables).
    const COUNT: usize = 10;

    /// Dense index of the phase.
    const fn index(self) -> usize {
        match self {
            Phase::Reading => 0,
            Phase::Decoding => 1,
            Phase::Tokenizing => 2,
            Phase::Aligning => 3,
            Phase::Classifying => 4,
            Phase::Scanning => 5,
            Phase::ComparingContent => 6,
            Phase::Hashing => 7,
            Phase::Reporting => 8,
            Phase::Writing => 9,
        }
    }

    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Phase::Reading => "reading",
            Phase::Decoding => "decoding",
            Phase::Tokenizing => "tokenizing",
            Phase::Aligning => "aligning",
            Phase::Classifying => "classifying",
            Phase::Scanning => "scanning",
            Phase::ComparingContent => "comparing_content",
            Phase::Hashing => "hashing",
            Phase::Reporting => "reporting",
            Phase::Writing => "writing",
        }
    }
}

/// Unit of `done`/`total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    /// Items (entries, rows, resolved ranges).
    Items,
    /// Bytes.
    Bytes,
    /// Lines.
    Lines,
    /// Folders.
    Folders,
}

/// A progress report. `total == None` means indeterminate. `done` never decreases within a phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    /// Phase.
    pub phase: Phase,
    /// Units done.
    pub done: u64,
    /// Units total, when known.
    pub total: Option<u64>,
    /// Unit.
    pub unit: Unit,
    /// Item being processed (e.g. a relative path).
    pub current: Option<String>,
}

/// Receives progress. Must be cheap and non-blocking; it is called from worker threads.
pub trait ProgressSink: Send + Sync {
    /// Receives one report.
    fn report(&self, progress: Progress);
    /// Whether a report for `phase` would be forwarded now (the throttle is open). Engines call it through
    /// `Ctl::progress` so that a report is built only when it will be used. Default: always.
    fn due(&self, _phase: Phase) -> bool {
        true
    }
}

impl<F: Fn(Progress) + Send + Sync> ProgressSink for F {
    /// Calls the closure.
    fn report(&self, progress: Progress) {
        self(progress);
    }
}

impl<S: ProgressSink + ?Sized> ProgressSink for Arc<S> {
    /// Forwards to the shared sink (so `ThrottledSink<Arc<dyn ProgressSink>>` works).
    fn report(&self, progress: Progress) {
        (**self).report(progress);
    }

    fn due(&self, phase: Phase) -> bool {
        (**self).due(phase)
    }
}

/// Discards everything (and is never `due`, so lazy reports are never built).
#[derive(Debug, Clone, Copy, Default)]
pub struct NullProgress;

impl ProgressSink for NullProgress {
    fn report(&self, _progress: Progress) {}

    fn due(&self, _phase: Phase) -> bool {
        false
    }
}

/// Throttle bookkeeping of [`ThrottledSink`]: when the last report of each phase was forwarded.
#[derive(Debug, Default)]
struct ThrottleState {
    last_at: [Option<Instant>; Phase::COUNT],
}

impl ThrottleState {
    /// The throttle of `phase` is open: the phase starts (nothing of it was forwarded yet) or `interval` has
    /// elapsed since its last forwarded report. Phases are throttled independently, so phases that report
    /// concurrently (folder compare: `scanning` and `comparing_content`) cannot open each other's throttle.
    fn open(&self, phase: Phase, now: Instant, interval: Duration) -> bool {
        match self.last_at.get(phase.index()).copied().flatten() {
            Some(last) => now.saturating_duration_since(last) >= interval,
            None => true,
        }
    }
}

/// Forwards at most one report per `interval` per phase. A phase change (the first report of a phase), or
/// `done == total`, is always forwarded. Uses `Instant::now()`, which affects only progress delivery, never
/// results (allowed by T1).
///
/// Each phase has its own throttle: when two phases interleave (folder compare reports `scanning` and
/// `comparing_content` from concurrent tasks), each is still limited to one report per `interval`, plus its
/// completions (§5.4: ≤ 20 Hz per phase with the service's 50 ms interval).
pub struct ThrottledSink<S: ProgressSink> {
    inner: S,
    interval: Duration,
    state: Mutex<ThrottleState>,
}

impl<S: ProgressSink> ThrottledSink<S> {
    /// Wraps `inner`.
    pub fn new(inner: S, interval: Duration) -> Self {
        ThrottledSink {
            inner,
            interval,
            state: Mutex::new(ThrottleState::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, ThrottleState> {
        // The state is a few timestamps; a panic elsewhere cannot leave it inconsistent.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Reads the monotonic clock for the delivery throttle.
// Architecture test check 2 (§6.1, tenet T1) allow-point: `ThrottledSink` may use `Instant::now`, which
// affects only *when* progress is delivered, never a result.
fn monotonic_now() -> Instant {
    Instant::now()
}

impl<S: ProgressSink> ProgressSink for ThrottledSink<S> {
    /// Forwards the first report of a phase, a report with `done == total`, and otherwise a report whose phase
    /// has not been forwarded for `interval`.
    fn report(&self, progress: Progress) {
        let now = monotonic_now();
        let forward = {
            let mut state = self.state();
            let complete = progress.total == Some(progress.done);
            let forward = complete || state.open(progress.phase, now, self.interval);
            if forward && let Some(slot) = state.last_at.get_mut(progress.phase.index()) {
                *slot = Some(now);
            }
            forward
        };
        if forward {
            self.inner.report(progress);
        }
    }

    /// True when nothing of `phase` was forwarded yet or `interval` has elapsed since the last forwarded report
    /// of `phase`.
    fn due(&self, phase: Phase) -> bool {
        let now = monotonic_now();
        self.state().open(phase, now, self.interval)
    }
}

impl<S: ProgressSink> fmt::Debug for ThrottledSink<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThrottledSink")
            .field("interval", &self.interval)
            .finish_non_exhaustive()
    }
}

/// Cancellation and progress for one long operation.
#[derive(Clone, Copy)]
pub struct Ctl<'a> {
    /// Cancellation token.
    pub cancel: &'a CancelToken,
    /// Progress sink.
    pub progress: &'a dyn ProgressSink,
}

static NEVER: CancelToken = CancelToken::never();
static NULL_PROGRESS: NullProgress = NullProgress;

impl Ctl<'static> {
    /// Never cancelled, discards progress (tests and synchronous helpers).
    pub fn detached() -> Ctl<'static> {
        Ctl {
            cancel: &NEVER,
            progress: &NULL_PROGRESS,
        }
    }
}

impl<'a> Ctl<'a> {
    /// `self.cancel.check()`.
    pub fn check(&self) -> Result<(), Cancelled> {
        self.cancel.check()
    }

    /// Lazy, throttled report: calls `make` (and forwards its result) only when `self.progress.due(phase)`.
    /// Hot loops use this, so no `Progress` (and no `current` string) is built between throttle ticks.
    pub fn progress(&self, phase: Phase, make: impl FnOnce() -> Progress) {
        if self.progress.due(phase) {
            self.progress.report(make());
        }
    }

    /// Unconditional report (phase starts and completion `done == total`).
    pub fn progress_now(&self, progress: Progress) {
        self.progress.report(progress);
    }
}

impl fmt::Debug for Ctl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ctl")
            .field("cancelled", &self.cancel.is_cancelled())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn report(phase: Phase, done: u64, total: Option<u64>) -> Progress {
        Progress {
            phase,
            done,
            total,
            unit: Unit::Items,
            current: None,
        }
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<Progress>>);

    impl ProgressSink for Recorder {
        fn report(&self, progress: Progress) {
            self.0.lock().unwrap().push(progress);
        }
    }

    fn seen(recorder: &Recorder) -> Vec<(Phase, u64)> {
        recorder
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|p| (p.phase, p.done))
            .collect()
    }

    #[test]
    fn throttle_rules() {
        let recorder = Arc::new(Recorder::default());
        let sink = ThrottledSink::new(Arc::clone(&recorder), Duration::from_secs(3600));
        assert!(sink.due(Phase::Reading));
        sink.report(report(Phase::Reading, 0, Some(10)));
        assert!(!sink.due(Phase::Reading), "interval not elapsed");
        sink.report(report(Phase::Reading, 5, Some(10)));
        sink.report(report(Phase::Reading, 10, Some(10)));
        assert!(sink.due(Phase::Aligning), "a phase change is always due");
        sink.report(report(Phase::Aligning, 1, None));
        sink.report(report(Phase::Aligning, 2, None));
        assert_eq!(
            seen(&recorder),
            [
                (Phase::Reading, 0),
                (Phase::Reading, 10),
                (Phase::Aligning, 1)
            ]
        );
    }

    #[test]
    fn interleaved_phases_are_throttled_per_phase() {
        let recorder = Arc::new(Recorder::default());
        let sink = ThrottledSink::new(Arc::clone(&recorder), Duration::from_secs(3600));
        let mut due = 0;
        for done in 0..1000u64 {
            let phase = if done % 2 == 0 {
                Phase::Scanning
            } else {
                Phase::ComparingContent
            };
            due += usize::from(sink.due(phase));
            sink.report(report(phase, done, None));
        }
        assert_eq!(due, 2, "each phase is due once per interval");
        assert_eq!(
            seen(&recorder),
            [(Phase::Scanning, 0), (Phase::ComparingContent, 1)]
        );
        sink.report(report(Phase::Scanning, 1000, Some(1000)));
        assert_eq!(
            seen(&recorder).last(),
            Some(&(Phase::Scanning, 1000)),
            "completion always passes"
        );
    }

    #[test]
    fn zero_interval_forwards_everything() {
        let count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&count);
        let sink = ThrottledSink::new(
            move |_p: Progress| {
                counter.fetch_add(1, Ordering::Relaxed);
            },
            Duration::ZERO,
        );
        for done in 0..5 {
            sink.report(report(Phase::Scanning, done, None));
        }
        assert_eq!(count.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn ctl_progress_is_lazy() {
        let built = AtomicUsize::new(0);
        let ctl = Ctl::detached();
        ctl.progress(Phase::Hashing, || {
            built.fetch_add(1, Ordering::Relaxed);
            report(Phase::Hashing, 0, None)
        });
        assert_eq!(
            built.load(Ordering::Relaxed),
            0,
            "NullProgress is never due"
        );
        let received = AtomicUsize::new(0);
        let sink = |_: Progress| {
            received.fetch_add(1, Ordering::Relaxed);
        };
        let cancel = CancelToken::new();
        let ctl = Ctl {
            cancel: &cancel,
            progress: &sink,
        };
        ctl.progress(Phase::Hashing, || {
            built.fetch_add(1, Ordering::Relaxed);
            report(Phase::Hashing, 0, None)
        });
        ctl.progress_now(report(Phase::Hashing, 1, Some(1)));
        assert_eq!(built.load(Ordering::Relaxed), 1);
        assert_eq!(received.load(Ordering::Relaxed), 2);
        assert_eq!(ctl.check(), Ok(()));
        cancel.cancel();
        assert_eq!(ctl.check(), Err(Cancelled));
        assert!(format!("{ctl:?}").contains("cancelled: true"));
    }

    #[test]
    fn phase_names_match_serde() {
        for phase in [
            Phase::Reading,
            Phase::Decoding,
            Phase::Tokenizing,
            Phase::Aligning,
            Phase::Classifying,
            Phase::Scanning,
            Phase::ComparingContent,
            Phase::Hashing,
            Phase::Reporting,
        ] {
            assert_eq!(
                serde_json::to_string(&phase).unwrap(),
                format!("\"{}\"", phase.as_str())
            );
            assert!(phase.index() < Phase::COUNT);
        }
    }
}
