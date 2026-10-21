//! Job control (§1.8.3, §5.1, §5.6): [`JobControl`], [`JobHandle`], [`run_guarded`].
//!
//! The CLI calls service methods directly with a [`JobControl`] from [`crate::api::Service::job`]; the GUI starts
//! every long operation with [`crate::api::Service::spawn`], which runs it on a thread named
//! `hikasa-job-<id>`, inside the service pool, behind the panic boundary, with a lossy progress channel.

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use hikasa_core::{CancelToken, Ctl, JobId, JobKind, Progress, ProgressSink};

use crate::api::ApiError;

/// Capacity of a job's progress channel (§5.2): reports are dropped while it is full.
pub const PROGRESS_CAPACITY: usize = 64;

/// Minimum interval between two forwarded reports of one phase (≤ 20 Hz per phase, §5.4).
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// Cancellation + progress of one service call. The GUI keeps the `CancelToken` to stop the job.
#[derive(Clone)]
pub struct JobControl {
    /// Id.
    pub id: JobId,
    /// Kind.
    pub kind: JobKind,
    /// Cancellation.
    pub cancel: CancelToken,
    /// Progress sink (wrapped by the service in a `ThrottledSink`, ≤ 20 Hz per phase).
    pub progress: Arc<dyn ProgressSink>,
}

impl JobControl {
    /// Borrowed control for engine calls.
    pub fn ctl(&self) -> Ctl<'_> {
        Ctl {
            cancel: &self.cancel,
            progress: &*self.progress,
        }
    }
}

impl fmt::Debug for JobControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobControl")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("cancelled", &self.cancel.is_cancelled())
            .finish_non_exhaustive()
    }
}

/// A job started with `Service::spawn` (the GUI's job API; the CLI calls service methods directly).
pub struct JobHandle<T> {
    id: JobId,
    kind: JobKind,
    title: String,
    cancel: CancelToken,
    progress: Receiver<Progress>,
    result: Receiver<Result<T, ApiError>>,
}

impl<T> JobHandle<T> {
    /// Assembles a handle (the service creates the channels and the thread).
    pub(crate) fn new(
        id: JobId,
        kind: JobKind,
        title: String,
        cancel: CancelToken,
        progress: Receiver<Progress>,
        result: Receiver<Result<T, ApiError>>,
    ) -> JobHandle<T> {
        JobHandle {
            id,
            kind,
            title,
            cancel,
            progress,
            result,
        }
    }

    /// Id.
    pub fn id(&self) -> JobId {
        self.id
    }

    /// Kind.
    pub fn kind(&self) -> JobKind {
        self.kind
    }

    /// Title given to `spawn`.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Requests cancellation.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Lossy progress stream: a bounded channel (capacity 64) whose sender drops reports when it is full, so a
    /// slow GUI never blocks a job.
    pub fn progress(&self) -> &Receiver<Progress> {
        &self.progress
    }

    /// The result, if the job has finished (non-blocking; `Some` at most once).
    pub fn try_result(&self) -> Option<Result<T, ApiError>> {
        self.result.try_recv().ok()
    }

    /// Blocks until the job finishes.
    pub fn wait(self) -> Result<T, ApiError> {
        self.result.recv().unwrap_or_else(|_| {
            Err(ApiError::Internal(format!(
                "job {} ended without a result (or its result was already taken)",
                self.id
            )))
        })
    }
}

impl<T> fmt::Debug for JobHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobHandle")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

/// Runs `f` inside `std::panic::catch_unwind`; a panic becomes `ApiError::Internal("panic: …")` (§5.6).
pub fn run_guarded<T>(f: impl FnOnce() -> Result<T, ApiError>) -> Result<T, ApiError> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => {
            let message = panic_message(payload.as_ref());
            tracing::error!(target: "hikasa", "caught a panic: {message}");
            Err(ApiError::Internal(format!("panic: {message}")))
        }
    }
}

/// The text of a panic payload (`&str` or `String`; anything else is described generically).
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "a panic without a message".to_owned()
    }
}

/// The progress sink of a spawned job: `try_send` into the bounded channel, dropping reports while it is full.
pub(crate) struct LossySink(pub(crate) Sender<Progress>);

impl ProgressSink for LossySink {
    fn report(&self, progress: Progress) {
        let _ = self.0.try_send(progress);
    }
}
