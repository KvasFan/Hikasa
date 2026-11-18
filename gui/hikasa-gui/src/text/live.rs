//! Recompare scheduling of a live view (M2 W11a, contract §1.11.3, D-294): one recompare job at a time, started
//! [`DEBOUNCE_S`] after the last edit, or at once when the previous job landed with edits waiting (`Replayed`). Frame
//! times are explicit (`now`, seconds of the window's clock), so the debounce is tested without sleeping.

use hikasa_core::JobId;
use hikasa_text::Installed;

/// The debounce after the last edit (D-268's open default, confirmed by D-294).
pub const DEBOUNCE_S: f64 = 0.100;
/// A running recompare shows *Recomparing…* in the status bar after this long.
pub const SHOW_AFTER_S: f64 = 0.250;

/// The recompare state of a view.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recompare {
    /// Frame time (s) of the last edit not yet in a job; `None`: nothing to do.
    pub pending_since: Option<f64>,
    /// The running job and the live generation it was made for.
    pub running: Option<(JobId, u64)>,
    /// When the running job started (the status's *Recomparing…* after 250 ms).
    pub started_at: Option<f64>,
    /// Jobs started so far (tests: "at most one at a time").
    pub started: u64,
}

impl Recompare {
    /// An edit at `now`: the next job waits [`DEBOUNCE_S`] from here (edits during a job only move this).
    pub fn on_edit(&mut self, now: f64) {
        self.pending_since = Some(now);
    }

    /// True when a job is to start this frame: none runs, and the last edit is [`DEBOUNCE_S`] old.
    pub fn due(&mut self, now: f64) -> bool {
        self.running.is_none()
            && self
                .pending_since
                .is_some_and(|since| now - since >= DEBOUNCE_S - 1e-9)
    }

    /// The job `id` for live generation `generation` starts at `now`.
    pub fn start(&mut self, id: JobId, generation: u64, now: f64) {
        self.running = Some((id, generation));
        self.started_at = Some(now);
        self.pending_since = None;
        self.started += 1;
    }

    /// The running job landed and was installed at `now`: `Replayed` means newer edits wait, and they waited for the
    /// running job already, so the next one is due at once.
    pub fn landed(&mut self, now: f64, installed: Installed) {
        self.running = None;
        self.started_at = None;
        if let Installed::Replayed { .. } = installed {
            self.pending_since = Some(self.pending_since.map_or(now, |s| s.min(now)) - DEBOUNCE_S);
        }
    }

    /// The running job failed or was cancelled: the view stays provisional until the next edit retries.
    pub fn failed(&mut self) {
        self.running = None;
        self.started_at = None;
    }

    /// True while a job has run for longer than [`SHOW_AFTER_S`].
    pub fn showing(&self, now: f64) -> bool {
        self.started_at
            .is_some_and(|at| now - at >= SHOW_AFTER_S - 1e-9)
    }

    /// Nothing waits and nothing runs.
    pub fn idle(&self) -> bool {
        self.pending_since.is_none() && self.running.is_none()
    }

    /// When a job becomes due (the window asks for a repaint then), if one waits.
    pub fn due_at(&self) -> Option<f64> {
        if self.running.is_some() {
            return None;
        }
        self.pending_since.map(|s| s + DEBOUNCE_S)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_debounce_waits_and_one_job_runs_at_a_time() {
        let mut r = Recompare::default();
        assert!(!r.due(0.0));
        r.on_edit(1.0);
        assert!(!r.due(1.05));
        assert!(r.due(1.1));
        r.start(JobId(1), 3, 1.1);
        r.on_edit(1.12);
        assert!(!r.due(1.5), "none while one runs");
        assert!(!r.showing(1.3));
        assert!(r.showing(1.36));
        r.landed(1.6, Installed::Replayed { pending: 2 });
        assert!(r.due(1.6), "the next at once after a replay");
        r.start(JobId(2), 4, 1.6);
        r.landed(1.7, Installed::Current);
        assert!(r.idle());
        assert_eq!(r.started, 2);
    }
}
