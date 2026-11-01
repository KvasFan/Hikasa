//! Work that must never delay the window or the IPC endpoint: the first-run format files (D-21).
//!
//! `FormatStore::materialize` writes about twenty files with an atomic write each (file and folder flushes).
//! On a busy or slow disk that took seconds before the first frame and could make a CLI-launched GUI miss the
//! launch budget of `gui open --wait`. The GUI therefore binds its endpoint and opens the window first; the
//! window starts this deferred work on a background thread after its first frame ([`Deferred::start`] returns at
//! once). Comparisons never wait for it: the built-in formats are always loaded from the binary, and the files
//! only make them editable.
//!
//! With a watch thread the files are written there, so that they raise no "changed on disk" notice (D-117).

use std::thread::JoinHandle;

use hikasa_session::{FormatStore, Service};

use crate::watch::{WatchCmd, WatchLink};

/// A one-shot task started off the UI thread.
pub struct Deferred {
    task: Option<Box<dyn FnOnce() + Send>>,
}

impl Deferred {
    /// A deferred `task`.
    pub fn new(task: impl FnOnce() + Send + 'static) -> Deferred {
        Deferred {
            task: Some(Box::new(task)),
        }
    }

    /// The first-run format files of `service`'s configuration (D-21), by the watch thread when `watch` is given; a
    /// failure is logged as a warning.
    pub fn materialize_formats(service: &Service, watch: Option<WatchLink>) -> Deferred {
        let service = service.clone();
        Deferred::new(move || {
            if let Some(link) = watch {
                link.send(WatchCmd::MaterializeFormats);
            } else if let Err(e) = FormatStore::new(service.config()).materialize() {
                tracing::warn!(target: "hikasa::gui", "cannot write the format files: {e}");
            }
        })
    }

    /// Starts the task on its own thread and returns at once; later calls do nothing. `None` when the task already
    /// started or the thread could not be created (then the task is dropped with a warning: the files are written
    /// on a later start).
    pub fn start(&mut self) -> Option<JoinHandle<()>> {
        let task = self.task.take()?;
        match std::thread::Builder::new()
            .name("hikasa-startup".into())
            .spawn(task)
        {
            Ok(handle) => Some(handle),
            Err(e) => {
                tracing::warn!(target: "hikasa::gui", "cannot start the first-run work: {e}");
                None
            }
        }
    }

    /// True until [`Deferred::start`] took the task.
    pub fn pending(&self) -> bool {
        self.task.is_some()
    }
}
