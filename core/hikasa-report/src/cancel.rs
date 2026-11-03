//! [`CancelWriter`] (M2 W7, D-202): cancellation reaches the pure renderers through their writer. The renderers stay
//! without a `Ctl`; the writer looks at the token every MiB written and then fails with an error that
//! [`ReportError::is_cancelled`](crate::ReportError::is_cancelled) recognises, which the Service maps to `Cancelled`.

use std::io::Write;

use hikasa_core::CancelToken;

/// How often the token is looked at: every MiB written.
pub const CANCEL_CHECK_BYTES: u64 = 1 << 20;

/// The message of the error a cancelled [`CancelWriter`] returns.
pub(crate) const CANCELLED: &str = "the report was cancelled";

/// A writer that stops at the first MiB boundary after its token was cancelled: at most one MiB is written after the
/// cancel. Bytes written so far are counted ([`CancelWriter::written`]).
pub struct CancelWriter<'a, W: Write> {
    inner: W,
    cancel: &'a CancelToken,
    written: u64,
    next_check: u64,
}

impl<'a, W: Write> CancelWriter<'a, W> {
    /// A writer into `inner` that looks at `cancel` every [`CANCEL_CHECK_BYTES`].
    pub fn new(inner: W, cancel: &'a CancelToken) -> CancelWriter<'a, W> {
        CancelWriter {
            inner,
            cancel,
            written: 0,
            next_check: 0,
        }
    }

    /// The bytes written into the inner writer.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// The inner writer.
    pub fn into_inner(self) -> W {
        self.inner
    }
}

/// The error of a cancelled writer.
pub(crate) fn cancelled() -> std::io::Error {
    std::io::Error::other(CancelledError)
}

/// The payload of the error of a cancelled writer.
#[derive(Debug)]
pub(crate) struct CancelledError;

impl std::fmt::Display for CancelledError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(CANCELLED)
    }
}

impl std::error::Error for CancelledError {}

impl<W: Write> Write for CancelWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.written >= self.next_check {
            if self.cancel.is_cancelled() {
                return Err(cancelled());
            }
            self.next_check = self.written - self.written % CANCEL_CHECK_BYTES + CANCEL_CHECK_BYTES;
        }
        let room = usize::try_from(self.next_check - self.written).unwrap_or(usize::MAX);
        let n = self.inner.write(&buf[..buf.len().min(room)])?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_until_the_next_boundary_after_a_cancel() {
        let token = CancelToken::new();
        let mut w = CancelWriter::new(Vec::new(), &token);
        w.write_all(&[1u8; 1000]).unwrap();
        token.cancel();
        let chunk = vec![0u8; 4096];
        let err = loop {
            if let Err(e) = w.write_all(&chunk) {
                break e;
            }
        };
        assert_eq!(err.to_string(), CANCELLED);
        assert_eq!(
            w.written(),
            CANCEL_CHECK_BYTES,
            "up to the MiB boundary, never past it"
        );
    }
}
