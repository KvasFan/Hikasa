//! CRC on demand (M2 W9a, FC-040; D-237): while the CRC column shows, the view asks for the CRCs of the file rows it
//! renders once the list has not scrolled for [`SETTLE`] (the frame time decides; no timer thread), at most [`BATCH`]
//! rows a job; a scroll cancels the job of the previous window. Toolkit-free: this is the schedule, the view model runs
//! the `folder_contents` job.

use std::time::{Duration, Instant};

/// How long the list stays still before the view asks (D-237).
pub const SETTLE: Duration = Duration::from_millis(200);

/// The most rows one job asks for (D-237).
pub const BATCH: usize = 256;

/// The rendered window: first row and row count.
pub type Window = (u32, u32);

/// What a frame means for the CRC job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrcStep {
    /// The window moved: the running job is cancelled, the wait starts again.
    Moved,
    /// Still, but not for long enough: ask again after this.
    Wait(Duration),
    /// Still for [`SETTLE`] and not yet answered for these rows: time to ask.
    Due,
    /// Answered for this window and these rows.
    Done,
}

/// The schedule of one view's CRC requests.
#[derive(Debug, Clone, Default)]
pub struct CrcDemand {
    window: Option<Window>,
    still_since: Option<Instant>,
    /// The window and rows revision the last job (or an empty ask) answered.
    answered: Option<(Window, u64)>,
}

impl CrcDemand {
    /// A frame showing `window` of the rows of `revision` at frame time `now`.
    pub fn frame(&mut self, window: Window, revision: u64, now: Instant) -> CrcStep {
        if self.window != Some(window) {
            self.window = Some(window);
            self.still_since = Some(now);
            self.answered = None;
            return CrcStep::Moved;
        }
        let since = *self.still_since.get_or_insert(now);
        let still = now.saturating_duration_since(since);
        if still < SETTLE {
            return CrcStep::Wait(SETTLE - still);
        }
        if self.answered == Some((window, revision)) {
            CrcStep::Done
        } else {
            CrcStep::Due
        }
    }

    /// The rows of `window` and `revision` are answered (a job ended, or none of them lacks a CRC).
    pub fn answered(&mut self, window: Window, revision: u64) {
        self.answered = Some((window, revision));
    }

    /// The column was hidden: everything starts again when it shows.
    pub fn reset(&mut self) {
        *self = CrcDemand::default();
    }
}
