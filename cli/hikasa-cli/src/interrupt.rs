//! Ctrl-C (contract M2 D-159; plan §5.3: the handler lives in the adapter, core only sees a [`CancelToken`]).
//!
//! The first job of a command arms the handler, so a command without a job (`format list`, `gui open --wait`, …)
//! keeps the OS's behaviour: Ctrl-C ends it at once. With a job, the first Ctrl-C cancels it through the one token
//! every job of this process shares: a rebuild aborts and removes its temporary and lock files, a `vfs cp` stops
//! between items, a comparison stops at its next check, and the command fails or reports through its usual path,
//! exit 100 (`ApiError::exit_code`; `VfsOpDto::ok`). The second Ctrl-C does not wait: the process exits 100 where it
//! stands, as a kill would, and the next rebuild of that archive removes what is left (D-158).
//!
//! The crate runs the closure on a thread of its own, not in the signal handler, so it may lock and write.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};

use hikasa_core::{CancelToken, ExitCode};

use crate::progress::ProgressLine;

/// Shown on a terminal after the first Ctrl-C.
const HINT: &str = "Cancelling; press Ctrl-C again to exit at once.";

/// What a press of Ctrl-C does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Press {
    /// The first: cancel the job.
    Cancel,
    /// Any later one: exit now.
    Exit,
}

/// The state the handler's thread shares with the command.
struct State {
    /// The token of every job of this process.
    token: CancelToken,
    pressed: AtomicBool,
    progress: ProgressLine,
}

impl State {
    fn press(&self) -> Press {
        if self.pressed.swap(true, Ordering::AcqRel) {
            Press::Exit
        } else {
            Press::Cancel
        }
    }
}

/// Ctrl-C for this process.
pub struct Interrupt {
    state: Arc<State>,
    armed: Once,
}

impl Interrupt {
    /// Not armed yet: no handler is installed until the first [`Interrupt::token`].
    pub fn new(progress: ProgressLine) -> Interrupt {
        Interrupt {
            state: Arc::new(State {
                token: CancelToken::new(),
                pressed: AtomicBool::new(false),
                progress,
            }),
            armed: Once::new(),
        }
    }

    /// The token for a job; arms the handler the first time. If the handler cannot be set, Ctrl-C ends the process
    /// as it does without one, which loses the cleanup and nothing else.
    pub fn token(&self) -> CancelToken {
        self.armed.call_once(|| {
            let state = Arc::clone(&self.state);
            let handler = move || match state.press() {
                Press::Cancel => {
                    state.token.cancel();
                    state.progress.note(HINT);
                }
                Press::Exit => std::process::exit(i32::from(ExitCode::UnknownError.code())),
            };
            if let Err(e) = ctrlc::set_handler(handler) {
                tracing::warn!(target: "hikasa::cli", "cannot set the Ctrl-C handler: {e}");
            }
        });
        self.state.token.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_press_cancels_and_any_later_one_exits() {
        let interrupt = Interrupt::new(ProgressLine::Off);
        assert_eq!(interrupt.state.press(), Press::Cancel);
        assert_eq!(interrupt.state.press(), Press::Exit);
        assert_eq!(interrupt.state.press(), Press::Exit);
    }
}
