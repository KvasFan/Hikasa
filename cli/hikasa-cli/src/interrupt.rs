//! Ctrl-C (contract M2 D-159; plan §5.3): the handler is the session's adapter-only [`CtrlC`] (shared with
//! `hikasa-gui @script --silent`, M2 W8b); core only sees a [`CancelToken`].
//!
//! The first job of a command arms the handler, so a command without a job (`format list`, `gui open --wait`, …)
//! keeps the OS's behaviour: Ctrl-C ends it at once. With a job, the first Ctrl-C cancels it through the one token
//! every job of this process shares: a rebuild aborts and removes its temporary and lock files, a `vfs cp` stops
//! between items, a comparison stops at its next check, and the command fails or reports through its usual path,
//! exit 100 (`ApiError::exit_code`; `VfsOpDto::ok`). The second Ctrl-C does not wait: the process exits 100 where it
//! stands, as a kill would, and the next rebuild of that archive removes what is left (D-158).
//!
//! A process started with SIGINT ignored (`cmd &` in a shell without job control, `nohup`-style wrappers) stays that
//! way: Ctrl-C of the terminal is not meant for it, so the handler is only set over the default action. Each step
//! leaves a debug line in the `--log` file, which is how the tests see that a press arrived, without timing it.

use hikasa_core::{CancelToken, ExitCode};
use hikasa_session::interrupt::CtrlC;

use crate::progress::ProgressLine;

/// Shown on a terminal after the first Ctrl-C.
const HINT: &str = "Cancelling; press Ctrl-C again to exit at once.";

/// Ctrl-C for this process.
pub struct Interrupt(CtrlC);

impl Interrupt {
    /// Not armed yet: no handler is installed until the first [`Interrupt::token`].
    pub fn new(progress: ProgressLine) -> Interrupt {
        Interrupt(CtrlC::new(
            move || progress.note(HINT),
            || std::process::exit(i32::from(ExitCode::UnknownError.code())),
        ))
    }

    /// The token for a job; arms the handler the first time.
    pub fn token(&self) -> CancelToken {
        self.0.token()
    }
}
