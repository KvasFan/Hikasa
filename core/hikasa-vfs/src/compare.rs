//! Binary comparison and CRC of content sources (§2.20; FC-025, VFS-020, ARCH-081): [`BinaryOutcome`],
//! [`crc32_source`], [`compare_sources`], [`CrcComparator`], [`BinaryComparator`].
//!
//! ```text
//! compare_sources(a, b, opts):
//!   1. both local and same_file::is_same_file(pa, pb)       → Same          (same object)
//!   2. both lengths known, different, and both > 4096 bytes  → Different{first_difference: None}
//!      (a reported size ≤ 4096 is not trusted: procfs reports 0 and sysfs 4096; such files are read)
//!   3. policy Auto, both local, length ≥ min_len:
//!         mapped comparison (8 MiB chunks on opts.threads threads, cancel checked per chunk, lowest
//!         differing offset wins); afterwards both files are re-stat'ed: an unchanged (length, mtime) pair
//!         keeps the result, a change discards it
//!   4. streaming: fill buf_a fully, fill buf_b with the same length; compare slices
//! crc32_source(s): known_crc32 → mmap (policy; chunk CRCs combined in order; same re-stat rule)
//!                  → streaming crc32fast::Hasher
//! ```
//!
//! A mapped attempt that fails for any reason other than cancellation falls back to streaming, which either
//! succeeds or reports the failure with the source's spec. Results never depend on `opts.threads` (T2).

use hikasa_core::{
    Cancelled, Classify, ContentComparator, ContentMethod, ContentOutcome, ContentResult,
    ContentSource, Ctl, ErrorKind, ErrorReport, io_error_kind,
};
use serde::{Deserialize, Serialize};

use crate::error::{VfsError, VfsOp};
use crate::mmap;
use crate::stream::{StreamOptions, compare_stream, crc32_stream};

/// Outcome of a binary comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BinaryOutcome {
    /// Identical bytes.
    Same,
    /// Different; `first_difference` is the first differing offset, or `None` when decided by length alone.
    Different {
        /// First differing byte offset (the shorter length when one input is a prefix of the other).
        first_difference: Option<u64>,
    },
}

impl BinaryOutcome {
    /// True for [`BinaryOutcome::Same`].
    pub fn is_same(self) -> bool {
        matches!(self, BinaryOutcome::Same)
    }
}

/// CRC of a source: `known_crc32()` first, then mmap (policy permitting), then streaming.
pub fn crc32_source(
    src: &dyn ContentSource,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
) -> Result<u32, VfsError> {
    crc32_source_with(src, opts, ctl, None)
}

/// [`crc32_source`] with an optional mmap re-stat test hook (runs between the mapped CRC and the re-stat).
pub(crate) fn crc32_source_with(
    src: &dyn ContentSource,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
    before_restat: Option<&dyn Fn()>,
) -> Result<u32, VfsError> {
    if let Some(crc) = src.known_crc32() {
        return Ok(crc);
    }
    ctl.check()?;
    // The policy first: a file too small to be mapped never asks for its local path (a `stat` on LocalVfs).
    if let Some(len) = src.len()
        && opts.mmap.maps(len)
        && let Some(path) = src.local_path()
    {
        let (chunk, threads) = (StreamOptions::MAPPED_CHUNK, opts.effective_threads());
        let mapped = match before_restat {
            None => mmap::crc32_file_mapped(path, chunk, threads, ctl),
            Some(hook) => mmap::crc32_file_mapped_with(path, chunk, threads, ctl, hook),
        };
        match mapped {
            Ok(Some(crc)) => return Ok(crc),
            Ok(None) => {}
            Err(e) if io_error_kind(&e) == ErrorKind::Cancelled => return Err(VfsError::Cancelled),
            // Streaming reports the failure with the source's spec (or succeeds).
            Err(_) => {}
        }
    }
    let display = src.display();
    let mut reader = src
        .open()
        .map_err(|e| VfsError::from_io(display.as_str(), VfsOp::Open, e))?;
    crc32_stream(&mut reader, opts.effective_buffer_size(), src.len(), ctl)
        .map_err(|fail| fail.into_vfs([display.as_str(), display.as_str()]))
}

/// Binary comparison of two sources (algorithm §2.20): same-object shortcut, length shortcut, mmap, stream.
pub fn compare_sources(
    a: &dyn ContentSource,
    b: &dyn ContentSource,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
) -> Result<BinaryOutcome, VfsError> {
    compare_sources_with(a, b, opts, ctl, None)
}

/// [`compare_sources`] with an optional mmap re-stat test hook (runs between the mapped comparison and the
/// re-stat).
pub(crate) fn compare_sources_with(
    a: &dyn ContentSource,
    b: &dyn ContentSource,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
    before_restat: Option<&dyn Fn()>,
) -> Result<BinaryOutcome, VfsError> {
    ctl.check()?;
    let local = a.local_path().zip(b.local_path());
    // 1. The same object on both sides.
    if let Some((pa, pb)) = local
        && same_file::is_same_file(pa, pb).unwrap_or(false)
    {
        return Ok(BinaryOutcome::Same);
    }
    // 2. Known, different lengths (reported sizes ≤ 4096 bytes are not trusted: pseudo-files).
    if StreamOptions::lengths_differ(a.len(), b.len()) {
        return Ok(BinaryOutcome::Different {
            first_difference: None,
        });
    }
    // 3. Memory-mapped comparison of large local regular files, kept only when a re-stat shows no change.
    if let (Some((pa, pb)), Some(len)) = (local, a.len())
        && opts.mmap.maps(len)
    {
        let (chunk, threads) = (StreamOptions::MAPPED_CHUNK, opts.effective_threads());
        let mapped = match before_restat {
            None => mmap::compare_files_mapped(pa, pb, chunk, threads, ctl),
            Some(hook) => mmap::compare_files_mapped_with(pa, pb, chunk, threads, ctl, hook),
        };
        match mapped {
            Ok(Some(outcome)) => return Ok(outcome),
            Ok(None) => {}
            Err(e) if io_error_kind(&e) == ErrorKind::Cancelled => return Err(VfsError::Cancelled),
            // Streaming reports the failure with the source's spec (or succeeds).
            Err(_) => {}
        }
    }
    // 4. Streaming, the reference implementation.
    let (da, db) = (a.display(), b.display());
    let mut ra = a
        .open()
        .map_err(|e| VfsError::from_io(da.as_str(), VfsOp::Open, e))?;
    let mut rb = b
        .open()
        .map_err(|e| VfsError::from_io(db.as_str(), VfsOp::Open, e))?;
    let hint = a.len().or(b.len());
    compare_stream(&mut ra, &mut rb, opts.effective_buffer_size(), hint, ctl)
        .map_err(|fail| fail.into_vfs([da.as_str(), db.as_str()]))
}

/// `ContentResult::Error` for `error`. The report names the spec the error carries (the failed source), else
/// `src`.
fn error_result(error: &VfsError, src: &dyn ContentSource) -> ContentResult {
    let mut report = ErrorReport::capture(error, None);
    report.spec = Some(error.spec().map_or_else(|| src.display(), str::to_owned));
    ContentResult::Error { error: report }
}

/// `ContentComparator` for the CRC method.
#[derive(Debug, Clone, Copy)]
pub struct CrcComparator {
    /// Streaming parameters.
    pub opts: StreamOptions,
}

/// `ContentComparator` for the binary method.
#[derive(Debug, Clone, Copy)]
pub struct BinaryComparator {
    /// Streaming parameters.
    pub opts: StreamOptions,
}

impl ContentComparator for CrcComparator {
    fn method(&self) -> ContentMethod {
        ContentMethod::Crc
    }

    /// Lengths known, different and both above 4096 bytes → `BinaryDifferent` (smaller reported sizes are not
    /// trusted, §2.20 step 2); else `crc(a) == crc(b)` ? `BinarySame` : `BinaryDifferent`. Failures become
    /// `ContentResult::Error`; cancellation is `Err(Cancelled)`.
    fn compare(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        ctl: &Ctl<'_>,
    ) -> Result<ContentOutcome, Cancelled> {
        ctl.check()?;
        let result = if StreamOptions::lengths_differ(left.len(), right.len()) {
            ContentResult::BinaryDifferent
        } else {
            crc_result(left, right, &self.opts, ctl)?
        };
        Ok(ContentOutcome {
            method: ContentMethod::Crc,
            result,
            warnings: Vec::new(),
        })
    }
}

fn crc_result(
    left: &dyn ContentSource,
    right: &dyn ContentSource,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
) -> Result<ContentResult, Cancelled> {
    let crc = |src: &dyn ContentSource| match crc32_source(src, opts, ctl) {
        Ok(crc) => Ok(Ok(crc)),
        Err(e) if e.kind() == ErrorKind::Cancelled => Err(Cancelled),
        Err(e) => Ok(Err(error_result(&e, src))),
    };
    let l = match crc(left)? {
        Ok(l) => l,
        Err(result) => return Ok(result),
    };
    let r = match crc(right)? {
        Ok(r) => r,
        Err(result) => return Ok(result),
    };
    Ok(if l == r {
        ContentResult::BinarySame
    } else {
        ContentResult::BinaryDifferent
    })
}

impl ContentComparator for BinaryComparator {
    fn method(&self) -> ContentMethod {
        ContentMethod::Binary
    }

    /// `compare_sources` → `Same` ? `BinarySame` : `BinaryDifferent`. Failures become
    /// `ContentResult::Error` (naming the failed source); cancellation is `Err(Cancelled)`.
    fn compare(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        ctl: &Ctl<'_>,
    ) -> Result<ContentOutcome, Cancelled> {
        let result = match compare_sources(left, right, &self.opts, ctl) {
            Ok(BinaryOutcome::Same) => ContentResult::BinarySame,
            Ok(BinaryOutcome::Different { .. }) => ContentResult::BinaryDifferent,
            Err(e) if e.kind() == ErrorKind::Cancelled => return Err(Cancelled),
            Err(e) => error_result(&e, left),
        };
        Ok(ContentOutcome {
            method: ContentMethod::Binary,
            result,
            warnings: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::MmapPolicy;
    use hikasa_core::{CancelToken, ReadError};
    use std::fs;
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};

    /// A local file as a content source (the length is taken once, like an `Entry`).
    struct LocalFile {
        path: PathBuf,
        len: u64,
    }

    impl LocalFile {
        fn new(path: &Path) -> LocalFile {
            LocalFile {
                path: path.to_path_buf(),
                len: fs::metadata(path).unwrap().len(),
            }
        }
    }

    impl ContentSource for LocalFile {
        fn name(&self) -> &str {
            "f"
        }
        fn display(&self) -> String {
            self.path.display().to_string()
        }
        fn len(&self) -> Option<u64> {
            Some(self.len)
        }
        fn local_path(&self) -> Option<&Path> {
            Some(&self.path)
        }
        fn open(&self) -> std::io::Result<Box<dyn Read + Send + '_>> {
            Ok(Box::new(fs::File::open(&self.path)?))
        }
        fn read_all(&self, _limit: u64, _cancel: &CancelToken) -> Result<Vec<u8>, ReadError> {
            Ok(fs::read(&self.path)?)
        }
    }

    fn mapped_always() -> StreamOptions {
        StreamOptions {
            buffer_size: 4096,
            mmap: MmapPolicy::Auto { min_len: 0 },
            threads: 1,
        }
    }

    /// The mmap re-stat test (§6.2.2): a hook changes the file length between the mapped comparison and the
    /// re-stat, so the mapped result (`Same`) is discarded and the result comes from streaming.
    #[test]
    fn restat_change_makes_the_result_come_from_streaming() {
        let dir = tempfile::tempdir().unwrap();
        let pa = dir.path().join("a");
        let pb = dir.path().join("b");
        fs::write(&pa, vec![9u8; 50_000]).unwrap();
        fs::write(&pb, vec![9u8; 50_000]).unwrap();
        let (a, b) = (LocalFile::new(&pa), LocalFile::new(&pb));
        let ctl = Ctl::detached();
        // Without a change the mapped result stands.
        assert_eq!(
            compare_sources_with(&a, &b, &mapped_always(), &ctl, None).unwrap(),
            BinaryOutcome::Same
        );
        let grow = || {
            let mut f = fs::OpenOptions::new().append(true).open(&pa).unwrap();
            f.write_all(b"tail").unwrap();
        };
        assert_eq!(
            compare_sources_with(&a, &b, &mapped_always(), &ctl, Some(&grow)).unwrap(),
            BinaryOutcome::Different {
                first_difference: Some(50_000)
            },
            "the streaming path saw the appended bytes"
        );
        // Same length, new content and a new mtime: streaming finds the changed byte.
        let rewrite = || {
            let mut data = vec![9u8; 50_000];
            data[1234] = 0;
            fs::write(&pb, data).unwrap();
            let later = filetime::FileTime::from_unix_time(2_000_000_000, 0);
            filetime::set_file_mtime(&pb, later).unwrap();
        };
        let c = LocalFile::new(&pb);
        let d_path = dir.path().join("d");
        fs::write(&d_path, vec![9u8; 50_000]).unwrap();
        let d = LocalFile::new(&d_path);
        assert_eq!(
            compare_sources_with(&d, &c, &mapped_always(), &ctl, Some(&rewrite)).unwrap(),
            BinaryOutcome::Different {
                first_difference: Some(1234)
            }
        );
        // The CRC path follows the same rule.
        let e_path = dir.path().join("e");
        fs::write(&e_path, vec![1u8; 10_000]).unwrap();
        let e = LocalFile::new(&e_path);
        let append = || {
            let mut f = fs::OpenOptions::new().append(true).open(&e_path).unwrap();
            f.write_all(b"x").unwrap();
        };
        let mut expected = vec![1u8; 10_000];
        expected.push(b'x');
        assert_eq!(
            crc32_source_with(&e, &mapped_always(), &ctl, Some(&append)).unwrap(),
            crc32fast::hash(&expected)
        );
    }
}
