//! Decoders and the guards every decoded byte passes through (D-143, D-145; §1.5.7 rules 9–15).
//!
//! No decoder is trusted: [`Guard`] counts its output and stops at the limits, [`Counted`] counts what it consumed
//! and keeps a base failure apart from corrupt content, and every pass runs inside [`no_panic`].

use std::io::{self, BufReader, Read};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use hikasa_core::{Cancelled, Ctl, Phase, Progress, Unit};

use super::ArchiveLimits;
use super::format::ArchiveFormat;
use crate::error::{VfsError, VfsOp};

/// Limits, cancellation and progress are looked at once per this many decoded bytes (G4).
pub(crate) const STEP: u64 = 1 << 20;
/// Size of a decoding buffer.
pub(crate) const BUFFER: usize = 128 << 10;
/// xz: the decoder memory limit of rule 15, 1 GiB in KiB.
const XZ_MEM_LIMIT_KIB: u32 = 1 << 20;
/// zstd: the window limit of rule 15, 1 GiB.
const ZSTD_WINDOW_LOG_MAX: u32 = 30;

/// What one pass over an archive runs under.
pub(crate) struct Pass<'a> {
    /// Canonical text of the archive or member, for errors.
    pub(crate) spec: &'a str,
    pub(crate) limits: ArchiveLimits,
    pub(crate) ctl: Ctl<'a>,
    /// The zone of DOS times, from the injected clock (T1).
    pub(crate) zone: jiff::tz::TimeZone,
}

/// A stream compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Codec {
    Gz,
    Bz2,
    Xz,
    Zst,
}

impl Codec {
    /// The compression of a stream format; `None` for zip and plain tar.
    pub(crate) const fn of(format: ArchiveFormat) -> Option<Codec> {
        match format {
            ArchiveFormat::Zip | ArchiveFormat::Tar => None,
            ArchiveFormat::TarGz | ArchiveFormat::Gz => Some(Codec::Gz),
            ArchiveFormat::TarBz2 | ArchiveFormat::Bz2 => Some(Codec::Bz2),
            ArchiveFormat::TarXz | ArchiveFormat::Xz => Some(Codec::Xz),
            ArchiveFormat::TarZst | ArchiveFormat::Zst => Some(Codec::Zst),
        }
    }
}

/// A decoder of concatenated streams, read to the end (§1.5.3), over `input`.
pub(crate) fn decoder<'a>(
    codec: Codec,
    input: Box<dyn Read + Send + 'a>,
) -> io::Result<Box<dyn Read + Send + 'a>> {
    Ok(match codec {
        Codec::Gz => Box::new(flate2::bufread::MultiGzDecoder::new(
            BufReader::with_capacity(BUFFER, input),
        )),
        Codec::Bz2 => Box::new(bzip2::read::MultiBzDecoder::new(input)),
        Codec::Xz => Box::new(lzma_rust2::XzReader::new_mem_limit(
            BufReader::with_capacity(BUFFER, input),
            true,
            XZ_MEM_LIMIT_KIB,
        )),
        Codec::Zst => {
            let mut decoder = zstd::stream::read::Decoder::new(input)?;
            decoder.window_log_max(ZSTD_WINDOW_LOG_MAX)?;
            Box::new(decoder)
        }
    })
}

/// What a [`Counted`] saw of its input.
#[derive(Debug, Default)]
pub(crate) struct Input {
    consumed: AtomicU64,
    error: Mutex<Option<io::Error>>,
}

impl Input {
    /// A base failure, taken out once.
    fn take_error(&self) -> Option<io::Error> {
        self.error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// Counts the compressed bytes consumed, and keeps a failure of the reader for [`classify`]: decoders rewrap it, and
/// it must not look like corrupt content.
pub(crate) struct Counted<R> {
    inner: R,
    input: Arc<Input>,
}

impl<R> Counted<R> {
    /// Wraps `inner`.
    pub(crate) fn new(inner: R, input: Arc<Input>) -> Counted<R> {
        Counted { inner, input }
    }
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.inner.read(buf) {
            Ok(n) => {
                self.input.consumed.fetch_add(n as u64, Ordering::Relaxed);
                Ok(n)
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
            Err(error) => {
                let copy = io::Error::new(error.kind(), error.to_string());
                *self
                    .input
                    .error
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(error);
                Err(copy)
            }
        }
    }
}

/// Where a [`Guard`] learns how much input was consumed.
pub(crate) enum InputSize {
    /// Live, from a [`Counted`].
    Live(Arc<Input>),
    /// The size of the compressed data (a zip member, whose reader stops there).
    Fixed(u64),
}

impl InputSize {
    fn get(&self) -> u64 {
        match self {
            InputSize::Live(input) => input.consumed.load(Ordering::Relaxed),
            InputSize::Fixed(size) => *size,
        }
    }
}

/// A decoded stream under the limits: a pass over more than `stream_max`, overlapping zip members that pass it together
/// (rule 12), a ratio past `ratio_max` once over `ratio_floor`, and cancellation.
pub(crate) struct Guard<'a, R> {
    inner: R,
    limits: ArchiveLimits,
    ctl: Ctl<'a>,
    spec: &'a str,
    input: InputSize,
    /// Decoded bytes of the members of this archive that overlap another.
    archive_total: Option<&'a AtomicU64>,
    output: u64,
    checked: u64,
}

impl<'a, R> Guard<'a, R> {
    /// Guards `inner`, a pass over the archive `spec`.
    pub(crate) fn new(
        inner: R,
        limits: ArchiveLimits,
        ctl: Ctl<'a>,
        spec: &'a str,
        input: InputSize,
    ) -> Guard<'a, R> {
        Guard {
            inner,
            limits,
            ctl,
            spec,
            input,
            archive_total: None,
            output: 0,
            checked: 0,
        }
    }

    /// Also counts into a total of the archive.
    pub(crate) fn with_total(self, total: &'a AtomicU64) -> Guard<'a, R> {
        Guard {
            archive_total: Some(total),
            ..self
        }
    }

    /// Bytes decoded so far.
    pub(crate) fn output(&self) -> u64 {
        self.output
    }

    fn limit(&self, limit: &'static str, value: u64) -> io::Error {
        io::Error::from(VfsError::ArchiveLimit {
            spec: self.spec.to_owned(),
            limit,
            value,
        })
    }

    /// The checks of one step.
    fn step(&mut self) -> io::Result<()> {
        let fresh = self.output - self.checked;
        self.checked = self.output;
        if self.ctl.cancel.is_cancelled() {
            return Err(io::Error::other(Cancelled));
        }
        if let Some(total) = self.archive_total {
            let all = total
                .fetch_add(fresh, Ordering::Relaxed)
                .saturating_add(fresh);
            if all > self.limits.stream_max {
                return Err(self.limit("stream", self.limits.stream_max));
            }
        }
        let max = u64::from(self.limits.ratio_max);
        if max > 0
            && self.output > self.limits.ratio_floor
            && self.output / self.input.get().max(1) > max
        {
            return Err(self.limit("ratio", max));
        }
        let done = self.output;
        self.ctl.progress(Phase::Scanning, || Progress {
            phase: Phase::Scanning,
            done,
            total: None,
            unit: Unit::Bytes,
            current: None,
        });
        Ok(())
    }
}

impl<R: Read> Read for Guard<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.output += n as u64;
        if self.output > self.limits.stream_max {
            return Err(self.limit("stream", self.limits.stream_max));
        }
        if n == 0 || self.output - self.checked >= STEP {
            self.step()?;
        }
        Ok(n)
    }
}

/// What a failed read means.
pub(crate) enum Failure {
    /// A real error: the base failed, a limit was reached, or the call was cancelled.
    Fatal(VfsError),
    /// The content is bad.
    Damage(String),
}

/// Sorts a read error of a decoder chain. `input` holds a base failure, if there was one.
pub(crate) fn classify(error: io::Error, input: Option<&Input>, spec: &str) -> Failure {
    if let Some(base) = input.and_then(Input::take_error) {
        return Failure::Fatal(VfsError::from_io(spec, VfsOp::Read, base));
    }
    match error.downcast::<VfsError>() {
        Ok(vfs) => Failure::Fatal(vfs),
        Err(error) => {
            if error.get_ref().is_some_and(|inner| inner.is::<Cancelled>()) {
                Failure::Fatal(VfsError::Cancelled)
            } else {
                Failure::Damage(error.to_string())
            }
        }
    }
}

/// [`classify`], with bad content as `ArchiveCorrupt`.
pub(crate) fn read_failure(error: io::Error, input: Option<&Input>, spec: &str) -> VfsError {
    match classify(error, input, spec) {
        Failure::Fatal(vfs) => vfs,
        Failure::Damage(reason) => corrupt(spec, reason),
    }
}

/// `ArchiveCorrupt`.
pub(crate) fn corrupt(spec: &str, reason: impl Into<String>) -> VfsError {
    VfsError::ArchiveCorrupt {
        spec: spec.to_owned(),
        reason: reason.into(),
    }
}

/// Runs `pass`; a panic becomes `ArchiveCorrupt { reason: "decoder failed" }` (rule 9).
pub(crate) fn no_panic<T>(
    spec: &str,
    pass: impl FnOnce() -> Result<T, VfsError>,
) -> Result<T, VfsError> {
    catch_unwind(AssertUnwindSafe(pass)).unwrap_or_else(|_| Err(corrupt(spec, "decoder failed")))
}

/// Fills `buf` as far as the stream goes; the count is short only at the end of the stream.
pub(crate) fn read_full(reader: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while let Some(rest) = buf.get_mut(filled..).filter(|rest| !rest.is_empty()) {
        match reader.read(rest) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

/// Copies at most `limit` bytes of `reader` to `sink`; returns how many it copied.
pub(crate) fn pump(
    reader: &mut dyn Read,
    limit: u64,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), VfsError>,
) -> io::Result<u64> {
    let mut buf = vec![0u8; BUFFER];
    let mut copied = 0u64;
    while copied < limit {
        let want = usize::try_from(limit - copied)
            .unwrap_or(BUFFER)
            .min(BUFFER);
        let n = read_full(reader, buf.get_mut(..want).unwrap_or_default())?;
        if n == 0 {
            break;
        }
        sink(buf.get(..n).unwrap_or_default()).map_err(io::Error::from)?;
        copied += n as u64;
    }
    Ok(copied)
}
