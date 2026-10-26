//! Output (contract §1.9.1): the stdout writer with the broken-pipe policy, the colour decision, JSON envelopes,
//! warnings and error lines (D-35, D-40, CLI invariants 2, 3 and 5).
//!
//! - stdout carries the result only: the JSON document (`--json`) or the human output. A broken pipe stops all
//!   further writing and never changes the exit code; the command keeps the code it computed (D-35).
//! - stderr carries errors (always: one line per failed command), warnings and progress (not with `--quiet`).
//!   Names and messages go through `hikasa_report::escape_controls`, so no raw control character reaches the
//!   terminal.

use std::io::{self, BufWriter, IsTerminal, StdoutLock, Write};

use hikasa_core::Warning;
use hikasa_report::escape_controls;
use hikasa_session::ApiError;
use hikasa_session::api::dto::JsonEnvelope;
use serde::Serialize;

/// The colour decision of §1.9.1: ANSI colours only on a terminal, without `--no-color` and with `NO_COLOR`
/// unset (no-color.org).
pub fn colour_enabled(is_terminal: bool, no_color_flag: bool, env_no_color: bool) -> bool {
    is_terminal && !no_color_flag && !env_no_color
}

/// The process's stdout with the broken-pipe policy (buffered; flushed by [`Output::finish`]). The first failed
/// write is final: every later write and flush fails at once (a broken pipe as a broken pipe), so one failure is
/// reported once — by the command that saw it, or by [`Output::finish`] when only the last flush fails.
pub struct Stdout {
    inner: BufWriter<StdoutLock<'static>>,
    broken: bool,
    failed: bool,
    wrote: bool,
}

impl Stdout {
    fn new() -> Stdout {
        Stdout {
            inner: BufWriter::with_capacity(64 * 1024, io::stdout().lock()),
            broken: false,
            failed: false,
            wrote: false,
        }
    }

    /// Notes a failed write: it stops every later write (a broken pipe stays a broken pipe).
    fn note(&mut self, error: &io::Error) {
        self.failed = true;
        if error.kind() == io::ErrorKind::BrokenPipe {
            self.broken = true;
        }
    }

    /// The error of a write after the first failure.
    fn refused(&self) -> io::Error {
        if self.broken {
            io::ErrorKind::BrokenPipe.into()
        } else {
            io::Error::other("standard output failed earlier")
        }
    }
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.failed {
            return Err(self.refused());
        }
        match self.inner.write(buf) {
            Ok(n) => {
                self.wrote |= n > 0;
                Ok(n)
            }
            Err(e) => {
                self.note(&e);
                Err(e)
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.failed {
            return Err(self.refused());
        }
        self.inner.flush().inspect_err(|e| self.note(e))
    }
}

/// What a write to stdout came to.
#[derive(Debug)]
pub enum WriteError {
    /// The reader went away: stop writing, keep the exit code (D-35).
    BrokenPipe,
    /// Any other failure (a full disk behind a redirection, …): the command fails with exit 100.
    Other(ApiError),
}

impl WriteError {
    /// Classifies an error of the service or of a write.
    pub fn from_api(error: ApiError) -> WriteError {
        if error.is_broken_pipe() {
            WriteError::BrokenPipe
        } else {
            WriteError::Other(error)
        }
    }

    /// Classifies an I/O error of a direct stdout write.
    pub fn from_io(error: io::Error) -> WriteError {
        if error.kind() == io::ErrorKind::BrokenPipe {
            WriteError::BrokenPipe
        } else {
            WriteError::Other(stdout_error(error))
        }
    }
}

/// A failure writing to stdout as an `ApiError` (exit 100).
pub fn stdout_error(error: io::Error) -> ApiError {
    ApiError::Vfs(hikasa_vfs::VfsError::Io {
        spec: "<stdout>".into(),
        op: hikasa_vfs::VfsOp::Write,
        source: error,
    })
}

/// stdout and stderr of one invocation, with the output mode.
pub struct Output {
    stdout: Stdout,
    json: bool,
    quiet: bool,
    colour: bool,
}

impl Output {
    /// The output of this process: `json` and `quiet` from the merged global flags; ANSI colours on stdout per
    /// [`colour_enabled`].
    pub fn new(json: bool, quiet: bool, no_color_flag: bool, env_no_color: bool) -> Output {
        Output {
            stdout: Stdout::new(),
            json,
            quiet,
            colour: colour_enabled(io::stdout().is_terminal(), no_color_flag, env_no_color),
        }
    }

    /// `--json` was given.
    pub fn json(&self) -> bool {
        self.json
    }

    /// ANSI colours are allowed on stdout.
    pub fn colour(&self) -> bool {
        self.colour
    }

    /// True once anything reached stdout (a late failure then prints no second JSON document).
    pub fn wrote_stdout(&self) -> bool {
        self.stdout.wrote
    }

    /// The stdout writer (human output, raw `vfs cat` bytes, reports).
    pub fn stdout(&mut self) -> &mut Stdout {
        &mut self.stdout
    }

    /// Writes one JSON document and its final newline (§4.1 rule 1).
    pub fn envelope<T: Serialize>(&mut self, envelope: &JsonEnvelope<T>) -> Result<(), WriteError> {
        envelope
            .write_to(&mut self.stdout)
            .map_err(WriteError::from_io)
    }

    /// Writes human lines (each followed by `\n`).
    pub fn lines<I, S>(&mut self, lines: I) -> Result<(), WriteError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for line in lines {
            self.stdout
                .write_all(line.as_ref().as_bytes())
                .and_then(|()| self.stdout.write_all(b"\n"))
                .map_err(WriteError::from_io)?;
        }
        Ok(())
    }

    /// Writes raw bytes to stdout.
    pub fn bytes(&mut self, bytes: &[u8]) -> Result<(), WriteError> {
        self.stdout.write_all(bytes).map_err(WriteError::from_io)
    }

    /// Flushes stdout. A broken pipe is not an error here (D-35), and neither is a failure the command already
    /// met and reported (one error line per failure).
    pub fn finish(&mut self) -> Result<(), ApiError> {
        if self.stdout.failed {
            return Ok(());
        }
        match self.stdout.flush() {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            Err(e) => Err(stdout_error(e)),
        }
    }

    /// Prints warnings to stderr in human mode, unless `--quiet` (with `--json` they are in the envelope).
    pub fn warnings(&self, warnings: &[Warning]) {
        if self.json || self.quiet {
            return;
        }
        for warning in warnings {
            stderr_line(&format!("warning: {}", warning.message));
        }
    }

    /// Prints the one stderr line of a failed command (also with `--quiet` and `--json`).
    pub fn error(&self, error: &ApiError) {
        stderr_line(&format!("error: {error}"));
    }
}

/// Writes `hikasa: <escaped message>` to stderr (failures to write to stderr are ignored).
pub fn stderr_line(message: &str) {
    let mut err = io::stderr().lock();
    let _ = writeln!(err, "hikasa: {}", escape_controls(message));
    let _ = err.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_need_a_terminal_and_no_opt_out() {
        assert!(colour_enabled(true, false, false));
        assert!(!colour_enabled(false, false, false));
        assert!(!colour_enabled(true, true, false));
        assert!(!colour_enabled(true, false, true));
        assert!(!colour_enabled(false, true, true));
    }

    #[test]
    fn write_errors_are_classified() {
        assert!(matches!(
            WriteError::from_io(io::ErrorKind::BrokenPipe.into()),
            WriteError::BrokenPipe
        ));
        assert!(matches!(
            WriteError::from_io(io::Error::other("disk full")),
            WriteError::Other(_)
        ));
        let pipe = stdout_error(io::ErrorKind::BrokenPipe.into());
        assert!(pipe.is_broken_pipe());
        assert!(matches!(WriteError::from_api(pipe), WriteError::BrokenPipe));
    }
}
