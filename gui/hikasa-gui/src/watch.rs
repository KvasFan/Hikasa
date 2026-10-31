//! The `hikasa-watch` thread (M2 W4a, D-117, D-118): it polls the settings files (`ConfigWatch` over every
//! settings file, `themes/` and `formats/`), the inputs of the open file views (`InputWatch`) and the session store
//! (D-124) every 2 s and at once when a window gains focus, and posts what changed to the UI thread. The UI sends it
//! [`WatchCmd`]s: `Poll` (focus), `Track`/`Untrack` (file views opening and closing), `Acknowledge` (after each of the
//! GUI's own writes, before the write's job returns, so the GUI's writes raise nothing), `MaterializeFormats` (the
//! first-run format files, written here so that no poll can run between the writes and their acknowledgement) and
//! `Stop`.
//!
//! Tests drive [`Watcher::poll_once`] directly (no thread, no timer).

use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use hikasa_session::settings::{ChangeKind, ConfigChange, ConfigWatch};
use hikasa_session::watch::InputWatch;
use hikasa_session::{FormatStore, Service};

/// The polling period.
pub const PERIOD: Duration = Duration::from_secs(2);

/// A command to the watch thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchCmd {
    /// Poll now (a window gained focus).
    Poll,
    /// Watch an input of a file view (its state is taken now, on the watch thread). Reference-counted: one
    /// `Track` per view and input; an input tracked already keeps its stamp.
    Track(String),
    /// One view stopped watching an input (it stays watched while another view tracks it).
    Untrack(String),
    /// The GUI wrote these files itself: take their stamps without reporting them.
    Acknowledge(Vec<PathBuf>),
    /// Writes the first-run format files (D-21) and acknowledges them, between two polls.
    MaterializeFormats,
    /// End the thread.
    Stop,
}

/// What a poll found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// Settings files changed.
    Config(Vec<ConfigChange>),
    /// Inputs of file views changed (spec, kind).
    Inputs(Vec<(String, ChangeKind)>),
    /// The session store changed, by this process or another one (D-124).
    Sessions,
}

/// The UI's end of the watch: commands go to the thread (or to a test's [`Watcher`]).
#[derive(Debug, Clone)]
pub struct WatchLink {
    tx: Sender<WatchCmd>,
}

impl WatchLink {
    /// Sends a command (ignored once the watcher is gone).
    pub fn send(&self, cmd: WatchCmd) {
        let _ = self.tx.send(cmd);
    }
}

/// The watcher: the settings watch, the input watch and the command queue.
pub struct Watcher {
    service: Service,
    config: ConfigWatch,
    inputs: InputWatch,
    /// [`Service::sessions_stamp`] at the last poll.
    sessions: u64,
    rx: Receiver<WatchCmd>,
}

impl Watcher {
    /// A watcher over the service's configuration (stamped now) and its link.
    pub fn new(service: Service) -> (Watcher, WatchLink) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let config = service.settings_store().watch();
        let sessions = service.sessions_stamp();
        (
            Watcher {
                service,
                config,
                inputs: InputWatch::new(),
                sessions,
                rx,
            },
            WatchLink { tx },
        )
    }

    /// Applies one command; false for `Stop`.
    fn apply(&mut self, cmd: WatchCmd) -> bool {
        match cmd {
            WatchCmd::Poll => {}
            WatchCmd::Track(spec) => {
                // A second view of an input counts once more and keeps the stamp (a pending change stays).
                if !self.inputs.retain(&spec) {
                    let now = self.service.input_stamp(&spec);
                    self.inputs.track(spec, now);
                }
            }
            WatchCmd::Untrack(spec) => self.inputs.untrack(&spec),
            WatchCmd::Acknowledge(paths) => {
                for path in paths {
                    self.config.acknowledge(&path);
                }
            }
            WatchCmd::MaterializeFormats => {
                match FormatStore::new(self.service.config()).materialize() {
                    Ok(written) => {
                        for path in written {
                            self.config.acknowledge(&path);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(target: "hikasa::gui", "cannot write the format files: {e}");
                    }
                }
            }
            WatchCmd::Stop => return false,
        }
        true
    }

    /// The watched inputs (tests).
    #[cfg(test)]
    pub fn tracked(&self) -> Vec<String> {
        self.inputs.specs().map(str::to_owned).collect()
    }

    /// Applies every queued command, then polls the settings and the inputs once. `None` after `Stop`.
    pub fn poll_once(&mut self) -> Option<Vec<WatchEvent>> {
        while let Ok(cmd) = self.rx.try_recv() {
            if !self.apply(cmd) {
                return None;
            }
        }
        let mut events = Vec::new();
        let config = self.config.poll();
        if !config.is_empty() {
            events.push(WatchEvent::Config(config));
        }
        let inputs = self.inputs.poll(&self.service);
        if !inputs.is_empty() {
            events.push(WatchEvent::Inputs(inputs));
        }
        let sessions = self.service.sessions_stamp();
        if sessions != self.sessions {
            self.sessions = sessions;
            events.push(WatchEvent::Sessions);
        }
        Some(events)
    }

    /// Runs the watcher on the thread `hikasa-watch`: a poll every [`PERIOD`] and at each `Poll`; `post` gets
    /// the events of each poll that found something.
    pub fn spawn(
        mut self,
        post: impl Fn(Vec<WatchEvent>) + Send + 'static,
    ) -> std::io::Result<JoinHandle<()>> {
        std::thread::Builder::new()
            .name("hikasa-watch".into())
            .spawn(move || {
                let mut due = std::time::Instant::now() + PERIOD;
                loop {
                    let wait = due.saturating_duration_since(std::time::Instant::now());
                    match self.rx.recv_timeout(wait) {
                        Ok(cmd) => {
                            let poll = cmd == WatchCmd::Poll;
                            if !self.apply(cmd) {
                                return;
                            }
                            if !poll {
                                continue;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                    due = std::time::Instant::now() + PERIOD;
                    match self.poll_once() {
                        None => return,
                        Some(events) if !events.is_empty() => post(events),
                        Some(_) => {}
                    }
                }
            })
    }
}
