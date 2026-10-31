//! Readers over a shared or partial source: [`SharedReader`], which lets threads decode one zip in parallel without
//! keeping its file open between calls, and [`RangeReader`], a window of a seekable stream.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::vfs::ReadSeek;

/// The state behind every clone of a [`SharedReader`].
struct Slot {
    /// `None` while no call uses the archive.
    reader: Option<Box<dyn ReadSeek>>,
    /// Calls that hold a [`ReaderLease`].
    users: usize,
    /// Where the reader stands.
    at: u64,
}

/// What the clones share.
struct Shared {
    slot: Mutex<Slot>,
    /// The base failed (as opposed to a decoder that refused the bytes).
    failed: AtomicBool,
    len: u64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A clone-able `Read + Seek` with its own position over one reader, so `ZipArchive` clones decode in parallel.
///
/// The reader is held only while a [`ReaderLease`] lives: an index keeps no file open between calls, so the
/// container can be removed or replaced on every platform (§1.5.19).
#[derive(Clone)]
pub(crate) struct SharedReader {
    shared: Arc<Shared>,
    pos: u64,
}

impl SharedReader {
    /// A shared reader of `len` bytes over `reader`, leased until the returned [`ReaderLease`] is dropped.
    pub(crate) fn new(reader: Box<dyn ReadSeek>, len: u64) -> (SharedReader, ReaderLease) {
        let shared = Arc::new(Shared {
            slot: Mutex::new(Slot {
                reader: Some(reader),
                users: 1,
                at: u64::MAX,
            }),
            failed: AtomicBool::new(false),
            len,
        });
        let lease = ReaderLease {
            shared: Arc::clone(&shared),
        };
        (SharedReader { shared, pos: 0 }, lease)
    }

    /// Holds the reader for one call, opening it with `open` when no other call holds it.
    pub(crate) fn lease<E>(
        &self,
        open: impl FnOnce() -> Result<Box<dyn ReadSeek>, E>,
    ) -> Result<ReaderLease, E> {
        let mut slot = self.shared.lock();
        if slot.reader.is_none() {
            slot.reader = Some(open()?);
            slot.at = u64::MAX;
        }
        slot.users += 1;
        Ok(ReaderLease {
            shared: Arc::clone(&self.shared),
        })
    }

    /// True when a read of the underlying source failed (a base error, not corrupt content).
    pub(crate) fn failed(&self) -> bool {
        self.shared.failed.load(Ordering::Relaxed)
    }

    /// Forgets an earlier failure.
    pub(crate) fn clear_failed(&self) {
        self.shared.failed.store(false, Ordering::Relaxed);
    }
}

/// Keeps the reader of a [`SharedReader`] open; the last lease closes it.
pub(crate) struct ReaderLease {
    shared: Arc<Shared>,
}

impl Drop for ReaderLease {
    fn drop(&mut self) {
        let mut slot = self.shared.lock();
        slot.users = slot.users.saturating_sub(1);
        if slot.users == 0 {
            slot.reader = None;
        }
    }
}

impl Read for SharedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut slot = self.shared.lock();
        let slot = &mut *slot;
        let Some(reader) = slot.reader.as_mut() else {
            return Err(io::Error::other("archive reader not open"));
        };
        let result = (|| {
            if slot.at != self.pos {
                reader.seek(SeekFrom::Start(self.pos))?;
                slot.at = self.pos;
            }
            reader.read(buf)
        })();
        match result {
            Ok(n) => {
                self.pos += n as u64;
                slot.at = self.pos;
                Ok(n)
            }
            Err(error) => {
                slot.at = u64::MAX;
                self.shared.failed.store(true, Ordering::Relaxed);
                Err(error)
            }
        }
    }
}

impl Seek for SharedReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.pos = seek_position(to, self.pos, self.shared.len)?;
        Ok(self.pos)
    }
}

/// The target of `to` from `pos` in a stream of `len` bytes.
fn seek_position(to: SeekFrom, pos: u64, len: u64) -> io::Result<u64> {
    let target = match to {
        SeekFrom::Start(at) => Some(at),
        SeekFrom::Current(delta) => pos.checked_add_signed(delta),
        SeekFrom::End(delta) => len.checked_add_signed(delta),
    };
    target.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "seek before the start of the stream",
        )
    })
}

/// `len` bytes of `inner` from `start`, as a stream of its own.
pub(crate) struct RangeReader<R> {
    inner: R,
    start: u64,
    len: u64,
    pos: u64,
    /// `inner` stands at `start + pos`.
    placed: bool,
}

impl<R: Read + Seek> RangeReader<R> {
    /// The window `start..start + len` of `inner`.
    pub(crate) fn new(inner: R, start: u64, len: u64) -> RangeReader<R> {
        RangeReader {
            inner,
            start,
            len,
            pos: 0,
            placed: false,
        }
    }
}

impl<R: Read + Seek> Read for RangeReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.len.saturating_sub(self.pos);
        if left == 0 || buf.is_empty() {
            return Ok(0);
        }
        if !self.placed {
            self.inner
                .seek(SeekFrom::Start(self.start.saturating_add(self.pos)))?;
            self.placed = true;
        }
        let want = usize::try_from(left).unwrap_or(usize::MAX).min(buf.len());
        let n = self.inner.read(buf.get_mut(..want).unwrap_or_default())?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for RangeReader<R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = seek_position(to, self.pos, self.len)?;
        if target != self.pos {
            self.pos = target;
            self.placed = false;
        }
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn boxed(data: &[u8]) -> Box<dyn ReadSeek> {
        Box::new(Cursor::new(data.to_vec()))
    }

    #[test]
    fn clones_keep_their_own_position() {
        let (mut a, lease) = SharedReader::new(boxed(b"abcdef"), 6);
        let mut b = a.clone();
        let mut buf = [0u8; 2];
        a.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ab");
        b.seek(SeekFrom::End(-2)).unwrap();
        b.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ef");
        a.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"cd");
        drop(lease);
        assert!(a.read(&mut buf).is_err(), "the last lease closes the file");
        let lease = a.lease(|| Ok::<_, ()>(boxed(b"abcdef"))).unwrap();
        a.seek(SeekFrom::Start(1)).unwrap();
        a.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"bc");
        drop(lease);
    }

    #[test]
    fn range_reader_is_a_stream() {
        let mut r = RangeReader::new(Cursor::new(b"0123456789".to_vec()), 2, 5);
        let mut all = String::new();
        r.read_to_string(&mut all).unwrap();
        assert_eq!(all, "23456");
        assert_eq!(r.seek(SeekFrom::End(-1)).unwrap(), 4);
        all.clear();
        r.read_to_string(&mut all).unwrap();
        assert_eq!(all, "6");
        assert!(r.seek(SeekFrom::Current(-6)).is_err());
        assert_eq!(r.seek(SeekFrom::Start(100)).unwrap(), 100);
        assert_eq!(r.read(&mut [0u8; 4]).unwrap(), 0);
    }
}
