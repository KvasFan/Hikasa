//! Changes of the inputs of open file views (M2 W4a, D-118; OPT-015): [`InputStamp`], [`InputState`],
//! [`InputWatch`].
//!
//! An input is watched by its length and modification time from the VFS `stat`, never by reading its content
//! (inputs may be gigabytes). Standard input, `clipboard://` and other blobs are not watched
//! ([`InputState::Unwatched`]). The watch holds no timer and no thread: the GUI's watch thread polls it every
//! 2 s and when a window gains focus (D-117).

use crate::api::Service;
use crate::settings::ChangeKind;

/// The identity of an input file at one moment: its length and modification time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputStamp {
    /// Length in bytes.
    pub len: u64,
    /// Modification time in nanoseconds since the Unix epoch (none when the file system has none).
    pub modified_ns: Option<i128>,
}

/// What `Service::input_stamp` found for a spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputState {
    /// Not watched (a blob: `-`, `clipboard://`; a spec that does not parse).
    Unwatched,
    /// The input does not exist (removed or renamed).
    Missing,
    /// The input exists.
    Present(InputStamp),
}

/// One watched input: its last state and how many open views track it.
#[derive(Debug, Clone)]
struct Watched {
    spec: String,
    state: InputState,
    refs: usize,
}

/// The inputs of the open file views, by spec, reference-counted: two views of one input track it twice and it
/// stays watched until both untracked it.
#[derive(Debug, Clone, Default)]
pub struct InputWatch {
    inputs: Vec<Watched>,
}

impl InputWatch {
    /// An empty watch.
    pub fn new() -> InputWatch {
        InputWatch::default()
    }

    /// True when `spec` is watched.
    pub fn is_tracked(&self, spec: &str) -> bool {
        self.inputs.iter().any(|w| w.spec == spec)
    }

    /// One more view tracks `spec`, already watched: its count grows, its stamp stays (a change not polled
    /// yet is still reported). False when `spec` is not watched (track it with [`InputWatch::track`]).
    pub fn retain(&mut self, spec: &str) -> bool {
        match self.inputs.iter_mut().find(|w| w.spec == spec) {
            Some(w) => {
                w.refs += 1;
                true
            }
            None => false,
        }
    }

    /// One view tracks `spec` from its state `now`. A spec already watched only counts one more view and keeps
    /// its stamp ([`InputWatch::retain`]). An `Unwatched` spec is not tracked.
    pub fn track(&mut self, spec: String, now: InputState) {
        if self.retain(&spec) || now == InputState::Unwatched {
            return;
        }
        self.inputs.push(Watched {
            spec,
            state: now,
            refs: 1,
        });
    }

    /// One view stops tracking `spec`; the input stays watched while another view tracks it.
    pub fn untrack(&mut self, spec: &str) {
        if let Some(i) = self.inputs.iter().position(|w| w.spec == spec) {
            self.inputs[i].refs -= 1;
            if self.inputs[i].refs == 0 {
                self.inputs.remove(i);
            }
        }
    }

    /// The watched specs, in tracking order.
    pub fn specs(&self) -> impl Iterator<Item = &str> {
        self.inputs.iter().map(|w| w.spec.as_str())
    }

    /// Stats every watched input and reports the changes since the last poll (`Created`: it exists again after
    /// it was missing; `Changed`: length or time differ; `Removed`: it is gone), in tracking order.
    pub fn poll(&mut self, svc: &Service) -> Vec<(String, ChangeKind)> {
        let mut changes = Vec::new();
        for Watched { spec, state, .. } in &mut self.inputs {
            let now = svc.input_stamp(spec);
            let kind = match (*state, now) {
                (a, b) if a == b => None,
                (_, InputState::Unwatched) => None,
                (InputState::Missing | InputState::Unwatched, InputState::Present(_)) => {
                    Some(ChangeKind::Created)
                }
                (_, InputState::Missing) => Some(ChangeKind::Removed),
                (InputState::Present(_), InputState::Present(_)) => Some(ChangeKind::Changed),
            };
            if now != InputState::Unwatched {
                *state = now;
            }
            if let Some(kind) = kind {
                changes.push((spec.clone(), kind));
            }
        }
        changes
    }
}
