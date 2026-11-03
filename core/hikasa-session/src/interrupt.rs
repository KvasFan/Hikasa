//! Ctrl-C for the adapters (contract M2 D-159; plan §5.3): [`CtrlC`], shared by `hikasa` and `hikasa-gui @script
//! --silent` (M2 W8b, review F8). Adapter-only, like `capture_env` (architecture check 2): only `cli/` and `gui/` name
//! [`CtrlC`], and this file is the only one in the workspace that names the `ctrlc` crate. The engines only see the
//! [`CancelToken`].
//!
//! The first job of a command arms the handler ([`CtrlC::token`]), so a command without a job keeps the OS's behaviour:
//! Ctrl-C ends it at once. With a job, the first Ctrl-C cancels it through the one token every job of the process
//! shares (a rebuild aborts and cleans up, a copy stops between items, a script's current call stops and the rest is
//! `not_run`), and then calls the adapter's `first` (a hint on the terminal). Any later Ctrl-C calls the adapter's
//! `again`, which ends the process (exit 100) where it stands, as a kill would.
//!
//! A process started with SIGINT ignored (`cmd &` without job control, `nohup`-style wrappers) stays that way: the
//! handler is only set over the default action.
//!
//! The crate runs the closure on a thread of its own, not in the signal handler, so it may lock and write. Each step
//! leaves a debug line (target `hikasa::interrupt`) in the `--log` file, which is how the tests see that a press
//! arrived, without timing it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};

use hikasa_core::CancelToken;

/// What a press of Ctrl-C does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// The first: cancel the job.
    Cancel,
    /// Any later one: exit now.
    Exit,
}

/// What an adapter does on a press, besides the cancel.
type OnPress = Box<dyn Fn() + Send + Sync>;

/// The state the handler's thread shares with the command.
struct State {
    /// The token of every job of this process.
    token: CancelToken,
    pressed: AtomicBool,
    first: OnPress,
    again: OnPress,
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

/// Ctrl-C for this process (one per process: the crate takes one handler).
pub struct CtrlC {
    state: Arc<State>,
    armed: Once,
}

impl std::fmt::Debug for CtrlC {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CtrlC")
            .field("armed", &self.armed.is_completed())
            .field("pressed", &self.state.pressed.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl CtrlC {
    /// Not armed yet: no handler is installed until the first [`CtrlC::token`]. `first` runs after the first press
    /// cancelled the token; `again` on every later press (it should end the process with exit 100).
    pub fn new(
        first: impl Fn() + Send + Sync + 'static,
        again: impl Fn() + Send + Sync + 'static,
    ) -> CtrlC {
        CtrlC {
            state: Arc::new(State {
                token: CancelToken::new(),
                pressed: AtomicBool::new(false),
                first: Box::new(first),
                again: Box::new(again),
            }),
            armed: Once::new(),
        }
    }

    /// The token for a job; arms the handler the first time. If the handler cannot be set, Ctrl-C does what it did
    /// without one: it ends the process, which loses the cleanup and nothing else, or nothing where it is ignored.
    pub fn token(&self) -> CancelToken {
        self.armed.call_once(|| {
            let state = Arc::clone(&self.state);
            let handler = move || handle(&state, state.press());
            match ctrlc::try_set_handler(handler) {
                Ok(()) => tracing::debug!(target: "hikasa::interrupt", "Ctrl-C handler set"),
                // Inherited as ignored (or taken): the action in place stays.
                Err(ctrlc::Error::MultipleHandlers) => {
                    tracing::debug!(target: "hikasa::interrupt", "Ctrl-C keeps the action it inherited");
                }
                Err(e) => {
                    tracing::warn!(target: "hikasa::interrupt", "cannot set the Ctrl-C handler: {e}")
                }
            }
        });
        self.state.token.clone()
    }

    /// What a press does now, as the handler does it (tests: no signal is needed).
    pub fn press(&self) -> Press {
        let press = self.state.press();
        handle(&self.state, press);
        press
    }
}

fn handle(state: &State, press: Press) {
    match press {
        Press::Cancel => {
            tracing::debug!(target: "hikasa::interrupt", "Ctrl-C: cancelling");
            state.token.cancel();
            (state.first)();
        }
        Press::Exit => {
            tracing::debug!(target: "hikasa::interrupt", "Ctrl-C again: exiting");
            (state.again)();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn the_first_press_cancels_and_any_later_one_calls_again() {
        let firsts = Arc::new(AtomicU32::new(0));
        let agains = Arc::new(AtomicU32::new(0));
        let (f, a) = (Arc::clone(&firsts), Arc::clone(&agains));
        let ctrl_c = CtrlC::new(
            move || {
                f.fetch_add(1, Ordering::SeqCst);
            },
            move || {
                a.fetch_add(1, Ordering::SeqCst);
            },
        );
        let token = ctrl_c.state.token.clone();
        assert_eq!(ctrl_c.press(), Press::Cancel);
        assert!(token.is_cancelled());
        assert_eq!(ctrl_c.press(), Press::Exit);
        assert_eq!(ctrl_c.press(), Press::Exit);
        assert_eq!(firsts.load(Ordering::SeqCst), 1);
        assert_eq!(agains.load(Ordering::SeqCst), 2);
    }
}
