//! [`VfsSource`]: a file reachable through a [`Vfs`], as a [`ContentSource`].

use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use hikasa_core::{CancelToken, ContentSource, ErrorKind, ReadError, Spec, io_error_kind};

use crate::entry::{Entry, EntryKind};
use crate::stream::{FillError, StreamOptions, fill};
use crate::vfs::Vfs;

/// Cancellation granularity of `read_all` (1 MiB).
const READ_STEP: usize = 1 << 20;

/// A file reachable through a `Vfs`, as a `ContentSource`.
pub struct VfsSource<'a> {
    vfs: &'a dyn Vfs,
    spec: Spec,
    entry: Entry,
    /// [`Vfs::local_path`], asked on first use (a `LocalVfs` answers with a `stat`).
    local: OnceLock<Option<PathBuf>>,
    /// The token of [`ContentSource::open`] (M2 W5); `None` = never cancelled.
    cancel: Option<&'a CancelToken>,
}

impl<'a> VfsSource<'a> {
    /// Builds a source (`entry` must describe `spec`). Makes no back-end call: the local path that enables the
    /// mapped fast path ([`Vfs::local_path`]) is asked when [`ContentSource::local_path`] is first called.
    pub fn new(vfs: &'a dyn Vfs, spec: Spec, entry: Entry) -> VfsSource<'a> {
        VfsSource {
            vfs,
            spec,
            entry,
            local: OnceLock::new(),
            cancel: None,
        }
    }

    /// Lets `open` be cancelled while an archive member is decoded (G4, D-139); the default is never cancelled.
    /// `read_all` passes its own token.
    pub fn with_cancel(self, cancel: &'a CancelToken) -> VfsSource<'a> {
        VfsSource {
            cancel: Some(cancel),
            ..self
        }
    }

    /// The spec.
    pub fn spec(&self) -> &Spec {
        &self.spec
    }

    /// The entry describing the spec.
    pub fn entry(&self) -> &Entry {
        &self.entry
    }
}

impl fmt::Debug for VfsSource<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VfsSource")
            .field("spec", &self.spec)
            .field("entry", &self.entry)
            .field("local", &self.local)
            .finish_non_exhaustive()
    }
}

impl ContentSource for VfsSource<'_> {
    /// The entry name.
    fn name(&self) -> &str {
        self.entry.name.as_str()
    }

    /// The canonical spec.
    fn display(&self) -> String {
        self.spec.canonical()
    }

    /// The entry size, for files (including followed symlinks to files).
    fn len(&self) -> Option<u64> {
        (self.entry.kind == EntryKind::File).then_some(self.entry.size)
    }

    /// The entry's CRC32 (back-end provided or computed earlier).
    fn known_crc32(&self) -> Option<u32> {
        self.entry.crc32
    }

    /// [`Vfs::local_path`] for files (asked once, on the first call); `None` for other kinds.
    fn local_path(&self) -> Option<&Path> {
        self.local
            .get_or_init(|| match self.entry.kind {
                EntryKind::File => self.vfs.local_path(&self.spec),
                EntryKind::Dir | EntryKind::Symlink | EntryKind::Other => None,
            })
            .as_deref()
    }

    /// [`Vfs::open_read_with`]; a [`crate::VfsError`] is wrapped so that `VfsError::from_io` recovers it.
    fn open(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        let never = CancelToken::never();
        let cancel = self.cancel.unwrap_or(&never);
        let reader = self.vfs.open_read_with(&self.spec, cancel)?;
        Ok(Box::new(reader))
    }

    /// Exact-size read through `open_read`, with a cancellation check per 1 MiB and between partial reads.
    /// Content longer than `limit` (known in advance or found while reading) is `ReadError::TooLarge`; a
    /// reported size of at most 4096 bytes is not trusted for that (procfs and sysfs,
    /// [`StreamOptions::UNRELIABLE_SIZE_MAX`]). Cancellation — the token, or an error that means
    /// cancellation (a `VfsError::Cancelled` from `open_read`, a reader error that wraps `Cancelled`) — is
    /// `ReadError::Cancelled`.
    fn read_all(&self, limit: u64, cancel: &CancelToken) -> Result<Vec<u8>, ReadError> {
        cancel.check()?;
        let expected = self.len();
        if expected.is_some_and(|len| len > limit && len > StreamOptions::UNRELIABLE_SIZE_MAX) {
            return Err(ReadError::TooLarge { limit });
        }
        let mut reader = self
            .vfs
            .open_read_with(&self.spec, cancel)
            .map_err(|e| read_error(io::Error::from(e)))?;
        read_limited(&mut reader, expected, limit, cancel)
    }
}

/// `ReadError::Cancelled` for an I/O error that means cancellation (`io_error_kind` = `cancelled`), else
/// `ReadError::Io`.
fn read_error(error: io::Error) -> ReadError {
    if io_error_kind(&error) == ErrorKind::Cancelled {
        ReadError::Cancelled
    } else {
        ReadError::Io(error)
    }
}

impl From<FillError> for ReadError {
    fn from(fail: FillError) -> ReadError {
        match fail {
            FillError::Cancelled => ReadError::Cancelled,
            FillError::Io(error) => read_error(error),
        }
    }
}

/// Reads everything `r` yields into a buffer of exactly the content's size (when `expected` is right), with a
/// cancellation check per 1 MiB and between partial reads. More than `limit` bytes → `TooLarge`.
pub(crate) fn read_limited(
    r: &mut dyn Read,
    expected: Option<u64>,
    limit: u64,
    cancel: &CancelToken,
) -> Result<Vec<u8>, ReadError> {
    let mut target = expected
        .and_then(|len| usize::try_from(len).ok())
        .unwrap_or(0);
    let mut buf: Vec<u8> = Vec::new();
    buf.try_reserve_exact(target)
        .map_err(|_| ReadError::Io(io::Error::from(io::ErrorKind::OutOfMemory)))?;
    loop {
        cancel.check()?;
        let at_end = if buf.len() < target {
            let old = buf.len();
            let want = READ_STEP.min(target - old);
            buf.resize(old + want, 0);
            let n = fill(r, buf.get_mut(old..).unwrap_or_default(), cancel)?;
            buf.truncate(old + n);
            n < want
        } else {
            // At the expected size (or the size is unknown): probe for more.
            let mut probe = [0u8; 8192];
            let n = fill(r, &mut probe, cancel)?;
            buf.extend_from_slice(probe.get(..n).unwrap_or_default());
            target = buf.len().saturating_add(READ_STEP);
            n < probe.len()
        };
        if buf.len() as u64 > limit {
            return Err(ReadError::TooLarge { limit });
        }
        if at_end {
            break;
        }
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn exact_sizes_and_limits() {
        let never = CancelToken::never();
        let data: Vec<u8> = (0..3_000_000u32).map(|i| i as u8).collect();
        let got =
            read_limited(&mut Cursor::new(&data), Some(3_000_000), 3_000_000, &never).unwrap();
        assert_eq!(got, data);
        assert_eq!(got.capacity(), data.len(), "exact-size buffer");
        // Unknown size.
        let got = read_limited(&mut Cursor::new(&data), None, u64::MAX, &never).unwrap();
        assert_eq!(got, data);
        // Wrong hints in both directions.
        let got = read_limited(&mut Cursor::new(&data), Some(10), u64::MAX, &never).unwrap();
        assert_eq!(got, data);
        let got = read_limited(&mut Cursor::new(&data), Some(5_000_000), u64::MAX, &never).unwrap();
        assert_eq!(got, data);
        // Growth beyond the limit.
        assert!(matches!(
            read_limited(&mut Cursor::new(&data), Some(10), 2_999_999, &never),
            Err(ReadError::TooLarge { limit: 2_999_999 })
        ));
        assert!(matches!(
            read_limited(&mut Cursor::new(&data), None, 0, &never),
            Err(ReadError::TooLarge { limit: 0 })
        ));
        assert!(
            read_limited(&mut Cursor::new(Vec::<u8>::new()), None, 0, &never)
                .unwrap()
                .is_empty()
        );
    }

    /// A reader that returns `step` bytes per call and cancels `token` on its third call.
    struct SlowCancelling<'a> {
        calls: usize,
        token: &'a CancelToken,
    }

    impl Read for SlowCancelling<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.calls += 1;
            if self.calls == 3 {
                self.token.cancel();
            }
            let n = buf.len().min(64 << 10);
            buf[..n].fill(1);
            Ok(n)
        }
    }

    /// Regression (slow readers): the token is checked between the partial reads of one 1 MiB step.
    #[test]
    fn cancellation_between_partial_reads() {
        for expected in [Some(100 << 20), None] {
            let token = CancelToken::new();
            let mut slow = SlowCancelling {
                calls: 0,
                token: &token,
            };
            let r = read_limited(&mut slow, expected, u64::MAX, &token);
            assert!(matches!(r, Err(ReadError::Cancelled)), "{expected:?}");
            assert_eq!(slow.calls, 3, "no read after cancel()");
        }
    }

    /// Regression: a reader error that means cancellation is `ReadError::Cancelled`.
    #[test]
    fn cancelled_reader_errors_are_cancelled() {
        struct Fails;
        impl Read for Fails {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other(hikasa_core::Cancelled))
            }
        }
        let r = read_limited(&mut Fails, Some(10), 10, &CancelToken::never());
        assert!(matches!(r, Err(ReadError::Cancelled)), "{r:?}");
        let vfs_cancelled = io::Error::from(crate::VfsError::Cancelled);
        assert!(matches!(read_error(vfs_cancelled), ReadError::Cancelled));
        assert!(matches!(
            read_error(io::Error::other("boom")),
            ReadError::Io(_)
        ));
    }

    #[test]
    fn cancellation_is_checked() {
        let token = CancelToken::new();
        token.cancel();
        assert!(matches!(
            read_limited(&mut Cursor::new(vec![1u8; 10]), Some(10), 10, &token),
            Err(ReadError::Cancelled)
        ));
    }
}
