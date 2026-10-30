//! The jobs of the views: [`Service::spawn`] with a wake-up the UI can rely on.
//!
//! `Service::spawn` publishes a job's result only after the job's body has returned, so a body that wakes the UI
//! as its last step races the publication: the UI thread may poll `try_result` before the result is there and
//! then wait for the next low-rate poll. A [`Job`] wakes the UI after its body returned (also when it panicked)
//! and marks itself *settling* until the result is published; the window keeps repainting at once while any job
//! settles (a matter of microseconds), so a finished job shows in the next frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hikasa_core::{JobKind, Phase, Progress};
use hikasa_session::{ApiError, JobControl, JobHandle, Service};

/// The UI's wake-up (posts to the mailbox).
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// A running job of a view.
pub struct Job<T> {
    handle: JobHandle<T>,
    ended: Arc<AtomicBool>,
    /// The last progress report seen (the job indicator, D-115).
    last: parking_lot::Mutex<Option<Progress>>,
}

/// Marks the body's end and wakes the UI when dropped: after the body returned or while it unwinds.
struct EndGuard {
    ended: Arc<AtomicBool>,
    wake: Wake,
}

impl Drop for EndGuard {
    fn drop(&mut self) {
        self.ended.store(true, Ordering::Release);
        (self.wake)();
    }
}

impl<T: Send + 'static> Job<T> {
    /// Starts `body` on a job thread; `wake` is called once the body has returned.
    pub fn spawn(
        service: &Service,
        kind: JobKind,
        title: String,
        wake: &Wake,
        body: impl FnOnce(&Service, &JobControl) -> Result<T, ApiError> + Send + 'static,
    ) -> Job<T> {
        let ended = Arc::new(AtomicBool::new(false));
        let guard = EndGuard {
            ended: Arc::clone(&ended),
            wake: Arc::clone(wake),
        };
        let handle = service.spawn(kind, title, move |svc, job| {
            let _guard = guard;
            body(svc, job)
        });
        Job {
            handle,
            ended,
            last: parking_lot::Mutex::new(None),
        }
    }
}

impl<T> Job<T> {
    /// The result, once (`JobHandle::try_result`).
    pub fn try_result(&self) -> Option<Result<T, ApiError>> {
        self.handle.try_result()
    }

    /// Requests cancellation.
    pub fn cancel(&self) {
        self.handle.cancel();
    }

    /// The job's id.
    pub fn id(&self) -> hikasa_core::JobId {
        self.handle.id()
    }

    /// The job's title.
    pub fn title(&self) -> &str {
        self.handle.title()
    }

    /// The last phase reported and the fraction done when the total is known (drains the progress channel).
    pub fn progress(&self) -> (Option<Phase>, Option<f32>) {
        let mut last = self.last.lock();
        if let Some(p) = self.handle.progress().try_iter().last() {
            *last = Some(p);
        }
        match &*last {
            Some(p) => (
                Some(p.phase),
                p.total
                    .filter(|t| *t > 0)
                    .map(|t| (p.done as f64 / t as f64).clamp(0.0, 1.0) as f32),
            ),
            None => (None, None),
        }
    }

    /// True while the body has returned (and woken the UI) but the result is not published yet: the UI must
    /// look again at once instead of waiting for its low-rate poll.
    pub fn settling(&self) -> bool {
        self.ended.load(Ordering::Acquire) && !self.handle.is_finished()
    }
}
