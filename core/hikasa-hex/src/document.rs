//! The hex document (HX-001, HX-002; D-326): [`HexDocument`], [`HexLoad`], [`HexLoadOptions`], [`HexFileInfo`]
//! and the conversion hook [`HexConversion`] (HX-003, D-336).
//!
//! Every read goes through the VFS (T1): `Vfs::stat`, then `open_read`. A local file of at most
//! [`HexLoadOptions::unlocked_max`] bytes is read whole into memory and its handle closed at once, so it stays free
//! for other programs (HX-002's *loaded without file locks*); a larger one keeps one read handle for positional
//! reads. Archive members and every other back-end read through the temp cache file the VFS hands out
//! (`TempCache`). Pages of 64 KiB are read into a fixed LRU cache (256 pages); no mapping outlives a call — the
//! bulk passes read through the chunk visitor of `hikasa-vfs` (D-325), which maps local files one chunk at a time.

use std::any::Any;
use std::fmt;
use std::io::{self, Read, SeekFrom};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hikasa_core::{CancelToken, ContentSource, Ctl, FileTime, ReadError, Spec};
use hikasa_vfs::{EntryKind, FollowLinks, ReadSeek, StreamOptions, Vfs, VfsError, VfsOp};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{Changed, HexError};

/// How a document reads its bytes (HX-001, HX-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HexLoad {
    /// Whole in memory: a local file of at most `unlocked_max` bytes (its handle closed at once), standard input,
    /// the clipboard, test bytes.
    Memory,
    /// A local file larger than `unlocked_max`: one read handle kept, positional reads.
    File,
    /// An archive member, another back-end or a converted output: read from the temp cache file.
    TempCache,
}

impl HexLoad {
    /// Stable snake_case name.
    pub const fn as_str(self) -> &'static str {
        match self {
            HexLoad::Memory => "memory",
            HexLoad::File => "file",
            HexLoad::TempCache => "temp_cache",
        }
    }
}

/// Options of [`HexDocument::open`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexLoadOptions {
    /// Local files up to this size are read whole and their handle closed (HX-002; default 16 MiB).
    pub unlocked_max: u64,
    /// Page size of the cache (default 64 KiB; at least 1).
    pub page: u32,
    /// Pages kept (default 256 = 16 MiB per document; at least 1).
    pub cache_pages: u32,
}

impl HexLoadOptions {
    /// Default `unlocked_max` (16 MiB).
    pub const DEFAULT_UNLOCKED_MAX: u64 = 16 << 20;
    /// Default page (64 KiB).
    pub const DEFAULT_PAGE: u32 = 64 << 10;
    /// Default pages (256).
    pub const DEFAULT_CACHE_PAGES: u32 = 256;
}

impl Default for HexLoadOptions {
    fn default() -> HexLoadOptions {
        HexLoadOptions {
            unlocked_max: HexLoadOptions::DEFAULT_UNLOCKED_MAX,
            page: HexLoadOptions::DEFAULT_PAGE,
            cache_pages: HexLoadOptions::DEFAULT_CACHE_PAGES,
        }
    }
}

/// The File Info of one side (HX-004).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HexFileInfo {
    /// Canonical spec.
    pub spec: String,
    /// File name.
    pub name: String,
    /// Length in bytes (of the converted output when converted).
    pub len: u64,
    /// Modification time of the input, when known.
    pub mtime: Option<FileTime>,
    /// How the bytes are read.
    pub load: HexLoad,
    /// A Hex format's load conversion produced the bytes (*Converted*).
    pub converted: bool,
}

/// A Hex format's load conversion (HX-003, D-336), run by the Service with its `ProcessRunner`.
pub trait HexConversion: Send + Sync {
    /// Converts the input `spec` and returns the converted output: a file in the temp cache that `vfs` reads.
    ///
    /// # Errors
    ///
    /// [`HexError::Convert`] (or another error naming `spec`); [`HexError::Cancelled`].
    fn convert(&self, vfs: &dyn Vfs, spec: &Spec, ctl: &Ctl<'_>) -> Result<Converted, HexError>;
}

/// The output of a [`HexConversion`].
pub struct Converted {
    /// Where the converted bytes are (a local temp cache file, read through the VFS).
    pub spec: Spec,
    /// Kept as long as the document lives (a guard that removes the temp file when dropped).
    pub keep: Option<Box<dyn Any + Send + Sync>>,
}

impl fmt::Debug for Converted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Converted")
            .field("spec", &self.spec)
            .finish_non_exhaustive()
    }
}

/// (length, modification time) of an input when it was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    mtime: Option<FileTime>,
}

/// Where the bytes are.
enum Content {
    /// Whole in memory.
    Memory(Arc<[u8]>),
    /// A kept read handle and the page cache.
    Paged {
        handle: Mutex<Box<dyn ReadSeek>>,
        cache: Mutex<PageCache>,
    },
}

/// The LRU page cache: at most `capacity` pages; the least recently used one is evicted.
struct PageCache {
    capacity: usize,
    /// `(page number, bytes, last use)`.
    slots: Vec<(u64, Arc<[u8]>, u64)>,
    tick: u64,
}

impl PageCache {
    fn get(&mut self, page: u64) -> Option<Arc<[u8]>> {
        self.tick += 1;
        let tick = self.tick;
        self.slots
            .iter_mut()
            .find(|(p, _, _)| *p == page)
            .map(|slot| {
                slot.2 = tick;
                Arc::clone(&slot.1)
            })
    }

    fn contains(&self, page: u64) -> bool {
        self.slots.iter().any(|(p, _, _)| *p == page)
    }

    fn insert(&mut self, page: u64, bytes: Arc<[u8]>) {
        self.tick += 1;
        if let Some(slot) = self.slots.iter_mut().find(|(p, _, _)| *p == page) {
            slot.2 = self.tick;
            return;
        }
        if self.slots.len() < self.capacity {
            self.slots.push((page, bytes, self.tick));
            return;
        }
        if let Some(oldest) = self.slots.iter_mut().min_by_key(|(_, _, used)| *used) {
            *oldest = (page, bytes, self.tick);
        }
    }
}

/// A byte document: a file, an archive member, a converted output or bytes in memory, of any `u64` length,
/// read lazily (D-326). Shared between jobs and the view (`Sync`).
pub struct HexDocument {
    spec: Option<Spec>,
    display: String,
    name: String,
    len: u64,
    load: HexLoad,
    converted: bool,
    page: u64,
    stamp: Option<Stamp>,
    content: Content,
    /// The local path of a local regular file (the visitor maps it).
    local: Option<PathBuf>,
    /// Kept for the document's life (a converted output's guard).
    _keep: Option<Box<dyn Any + Send + Sync>>,
}

impl fmt::Debug for HexDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HexDocument")
            .field("spec", &self.display)
            .field("len", &self.len)
            .field("load", &self.load)
            .field("converted", &self.converted)
            .finish_non_exhaustive()
    }
}

/// Cancellation granularity of whole reads (1 MiB).
const READ_STEP: usize = 1 << 20;

impl HexDocument {
    /// Opens `spec` through `vfs` (stat, then open; D-326).
    ///
    /// # Errors
    ///
    /// The VFS error of the stat or the open (naming `spec`; a folder is `is_a_directory`); a short read of a
    /// file loaded in memory is [`HexError::Truncated`]; an input whose reported size is at most 4096 bytes but
    /// that holds more than `max(unlocked_max, 4096)` bytes is `too_large`; cancellation.
    pub fn open(
        vfs: &dyn Vfs,
        spec: &Spec,
        opts: &HexLoadOptions,
        ctl: &Ctl<'_>,
    ) -> Result<HexDocument, HexError> {
        Self::open_inner(vfs, spec, None, opts, ctl)
    }

    /// Opens `spec` through `vfs` and its Hex format's load conversion (D-336): the document reads the converted
    /// output (`TempCache`, *Converted*), and [`check_unchanged`](Self::check_unchanged) watches the input.
    ///
    /// # Errors
    ///
    /// As [`open`](Self::open), and the conversion's error.
    pub fn open_converted(
        vfs: &dyn Vfs,
        spec: &Spec,
        conversion: &dyn HexConversion,
        opts: &HexLoadOptions,
        ctl: &Ctl<'_>,
    ) -> Result<HexDocument, HexError> {
        Self::open_inner(vfs, spec, Some(conversion), opts, ctl)
    }

    fn open_inner(
        vfs: &dyn Vfs,
        spec: &Spec,
        conversion: Option<&dyn HexConversion>,
        opts: &HexLoadOptions,
        ctl: &Ctl<'_>,
    ) -> Result<HexDocument, HexError> {
        ctl.check()?;
        let display = spec.canonical();
        let entry = vfs
            .stat(spec, FollowLinks::Yes)
            .map_err(|e| HexError::from_vfs(&display, e))?;
        if entry.kind == EntryKind::Dir {
            return Err(HexError::from_vfs(
                &display,
                VfsError::IsADirectory {
                    spec: display.clone(),
                },
            ));
        }
        let stamp = Stamp {
            len: entry.size,
            mtime: entry.mtime,
        };
        let name = entry.name.as_str().to_owned();
        let (read_spec, keep, converted) = match conversion {
            Some(conversion) => {
                let out = conversion.convert(vfs, spec, ctl)?;
                (out.spec, out.keep, true)
            }
            None => (spec.clone(), None, false),
        };
        let read_display = read_spec.canonical();
        let read_entry = if converted {
            vfs.stat(&read_spec, FollowLinks::Yes)
                .map_err(|e| HexError::from_vfs(&display, e))?
        } else {
            entry
        };
        let mut len = read_entry.size;
        // A reported size of at most 4096 bytes is not trusted (procfs reports 0, sysfs 4096; §2.20 step 2, the
        // rule of `compare_sources`): such an input is read to its end, at most `bound` bytes, and its length is
        // the bytes read (HX-009).
        let unreliable = !converted && len <= StreamOptions::UNRELIABLE_SIZE_MAX;
        let bound = opts.unlocked_max.max(StreamOptions::UNRELIABLE_SIZE_MAX);
        let load = match spec {
            _ if converted => HexLoad::TempCache,
            Spec::Stdin | Spec::Clipboard => HexLoad::Memory,
            Spec::Local(_) if len <= opts.unlocked_max => HexLoad::Memory,
            Spec::Local(_) => HexLoad::File,
            _ => HexLoad::TempCache,
        };
        let mut handle = vfs
            .open_read_with(&read_spec, ctl.cancel)
            .map_err(|e| HexError::from_vfs(&display, e))?;
        let whole_error = |e: Whole| match e {
            Whole::Short => HexError::Truncated {
                spec: display.clone(),
            },
            Whole::TooLarge => HexError::from_vfs(
                &display,
                VfsError::TooLarge {
                    spec: display.clone(),
                    limit: bound,
                },
            ),
            Whole::Cancelled => HexError::Cancelled,
            Whole::Io(e) => HexError::from_vfs(
                &display,
                VfsError::from_io(read_display.as_str(), VfsOp::Read, e),
            ),
        };
        let content = if load == HexLoad::Memory {
            // Read whole, then the handle is dropped here: nothing keeps the file open.
            let bytes = if unreliable {
                read_to_end(handle.as_mut(), bound, ctl)
            } else {
                read_whole(handle.as_mut(), len, ctl)
            }
            .map_err(whole_error)?;
            len = bytes.len() as u64;
            Content::Memory(bytes.into())
        } else {
            if unreliable {
                // Paged reads seek to every page: the handle's position after measuring does not matter.
                len = measure(handle.as_mut(), bound, ctl).map_err(whole_error)?;
            }
            Content::Paged {
                handle: Mutex::new(handle),
                cache: Mutex::new(PageCache {
                    capacity: opts.cache_pages.max(1) as usize,
                    slots: Vec::new(),
                    tick: 0,
                }),
            }
        };
        // (Not mapped when its reported size is not trusted: the visitor's mapped path checks the reported size.)
        let local = if load == HexLoad::File && !unreliable {
            vfs.local_path(&read_spec)
        } else {
            None
        };
        Ok(HexDocument {
            spec: Some(spec.clone()),
            display,
            name,
            len,
            load,
            converted,
            page: u64::from(opts.page.max(1)),
            stamp: Some(stamp),
            content,
            local,
            _keep: keep,
        })
    }

    /// A document over bytes in memory (standard input, the clipboard, tests): `Memory`, never changes.
    pub fn from_bytes(name: &str, bytes: Arc<[u8]>) -> HexDocument {
        HexDocument {
            spec: None,
            display: name.to_owned(),
            name: name.to_owned(),
            len: bytes.len() as u64,
            load: HexLoad::Memory,
            converted: false,
            page: u64::from(HexLoadOptions::DEFAULT_PAGE),
            stamp: None,
            content: Content::Memory(bytes),
            local: None,
            _keep: None,
        }
    }

    /// Length in bytes.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// True for a 0-byte document.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The spec it was opened from (`None` for [`from_bytes`](Self::from_bytes)).
    pub fn spec(&self) -> Option<&Spec> {
        self.spec.as_ref()
    }

    /// The canonical spec (or the name given to [`from_bytes`](Self::from_bytes)).
    pub fn display(&self) -> &str {
        &self.display
    }

    /// How the bytes are read.
    pub fn load(&self) -> HexLoad {
        self.load
    }

    /// The bytes when the document holds them in memory.
    pub fn memory(&self) -> Option<&Arc<[u8]>> {
        match &self.content {
            Content::Memory(bytes) => Some(bytes),
            Content::Paged { .. } => None,
        }
    }

    /// Reads up to `buf.len()` bytes at `offset` through the page cache; returns the number read (less only at
    /// the end of the document).
    ///
    /// # Errors
    ///
    /// [`HexError::Truncated`] when the input is shorter than when it was opened; the read error.
    pub fn read(&self, offset: u64, buf: &mut [u8]) -> Result<usize, HexError> {
        let end = offset
            .saturating_add(buf.len() as u64)
            .min(self.len)
            .max(offset);
        let want = (end - offset) as usize;
        // At or past the end (and for an empty `buf`): nothing to read.
        let Some(buf) = buf.get_mut(..want).filter(|_| want > 0) else {
            return Ok(0);
        };
        match &self.content {
            Content::Memory(bytes) => {
                let start = offset as usize;
                buf.copy_from_slice(&bytes[start..start + want]);
            }
            Content::Paged { cache, .. } => {
                let mut at = offset;
                let mut filled = 0;
                while at < end {
                    let page = at / self.page;
                    let cached = cache.lock().get(page);
                    let bytes = match cached {
                        Some(bytes) => bytes,
                        None => {
                            // Read outside the index lock; another reader may insert the same page meanwhile.
                            let bytes = self.read_page(page)?;
                            cache.lock().insert(page, Arc::clone(&bytes));
                            bytes
                        }
                    };
                    let within = (at - page * self.page) as usize;
                    let take = (bytes.len().saturating_sub(within)).min((end - at) as usize);
                    if take == 0 {
                        return Err(self.truncated());
                    }
                    buf[filled..filled + take].copy_from_slice(&bytes[within..within + take]);
                    filled += take;
                    at += take as u64;
                }
            }
        }
        Ok(want)
    }

    /// True when the bytes of `range` can be read without I/O (in memory, or every page cached): W13 shows a
    /// placeholder otherwise.
    pub fn is_cached(&self, range: Range<u64>) -> bool {
        match &self.content {
            Content::Memory(_) => true,
            Content::Paged { cache, .. } => {
                let end = range.end.min(self.len);
                if range.start >= end {
                    return true;
                }
                let cache = cache.lock();
                (range.start / self.page..=(end - 1) / self.page).all(|page| cache.contains(page))
            }
        }
    }

    /// The document as a content source for the chunk visitor (mapped when it is a local file under the mmap
    /// policy, in place when in memory, else streamed through the kept handle).
    pub fn source(&self) -> &dyn ContentSource {
        self
    }

    /// The File Info data (HX-004).
    pub fn info(&self) -> HexFileInfo {
        HexFileInfo {
            spec: self.display.clone(),
            name: self.name.clone(),
            len: self.len,
            mtime: self.stamp.and_then(|s| s.mtime),
            load: self.load,
            converted: self.converted,
        }
    }

    /// Re-stats the input through `vfs` (the Service calls it after every job on the document): `Err(Changed)`
    /// when its length or modification time differs from when it was opened, or it cannot be stat'ed any more.
    /// A document made by [`from_bytes`](Self::from_bytes) never changes.
    ///
    /// # Errors
    ///
    /// [`Changed`] naming the input.
    pub fn check_unchanged(&self, vfs: &dyn Vfs) -> Result<(), Changed> {
        let (Some(spec), Some(stamp)) = (&self.spec, self.stamp) else {
            return Ok(());
        };
        if matches!(spec, Spec::Stdin | Spec::Clipboard) {
            return Ok(());
        }
        let now = vfs.stat(spec, FollowLinks::Yes).ok().map(|e| Stamp {
            len: e.size,
            mtime: e.mtime,
        });
        if now == Some(stamp) {
            Ok(())
        } else {
            Err(Changed {
                spec: self.display.clone(),
            })
        }
    }

    /// Reads exactly `buf.len()` bytes at `offset` without the page cache (the bulk passes: the statistics pass's
    /// right side, Complete's windows).
    pub(crate) fn read_bulk(&self, offset: u64, buf: &mut [u8]) -> Result<(), HexError> {
        let end = offset.saturating_add(buf.len() as u64);
        if end > self.len {
            return Err(self.truncated());
        }
        match &self.content {
            Content::Memory(bytes) => {
                let start = offset as usize;
                buf.copy_from_slice(&bytes[start..start + buf.len()]);
                Ok(())
            }
            Content::Paged { handle, .. } => {
                let mut handle = handle.lock();
                handle
                    .seek(SeekFrom::Start(offset))
                    .map_err(|e| self.io_error(e))?;
                read_full(handle.as_mut(), buf).map_err(|e| self.io_error(e))?;
                Ok(())
            }
        }
    }

    /// One page from the kept handle.
    fn read_page(&self, page: u64) -> Result<Arc<[u8]>, HexError> {
        let start = page * self.page;
        let len = self.page.min(self.len.saturating_sub(start)) as usize;
        let mut bytes = vec![0u8; len];
        self.read_bulk(start, &mut bytes)?;
        Ok(bytes.into())
    }

    fn truncated(&self) -> HexError {
        HexError::Truncated {
            spec: self.display.clone(),
        }
    }

    fn io_error(&self, e: io::Error) -> HexError {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            return self.truncated();
        }
        HexError::from_vfs(
            &self.display,
            VfsError::from_io(self.display.as_str(), VfsOp::Read, e),
        )
    }
}

/// `read_exact` that retries `Interrupted` (and reports a short input as `UnexpectedEof`).
fn read_full(r: &mut dyn ReadSeek, buf: &mut [u8]) -> io::Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Why a whole read failed.
enum Whole {
    Short,
    TooLarge,
    Cancelled,
    Io(io::Error),
}

/// Reads exactly `len` bytes (more are ignored: the stamp check reports a growth), checking cancellation per
/// 1 MiB.
fn read_whole(r: &mut dyn ReadSeek, len: u64, ctl: &Ctl<'_>) -> Result<Vec<u8>, Whole> {
    let len = usize::try_from(len).map_err(|_| Whole::Short)?;
    let mut bytes = vec![0u8; len];
    let mut filled = 0;
    while filled < len {
        ctl.check().map_err(|_| Whole::Cancelled)?;
        let step = (len - filled).min(READ_STEP);
        match r.read(&mut bytes[filled..filled + step]) {
            Ok(0) => return Err(Whole::Short),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(Whole::Io(e)),
        }
    }
    Ok(bytes)
}

/// Reads to the end of `r` (an input whose reported size is not trusted), at most `bound` bytes, checking
/// cancellation per 1 MiB; more than `bound` bytes is [`Whole::TooLarge`].
fn read_to_end(r: &mut dyn ReadSeek, bound: u64, ctl: &Ctl<'_>) -> Result<Vec<u8>, Whole> {
    let mut bytes = Vec::new();
    let mut filled = 0;
    loop {
        ctl.check().map_err(|_| Whole::Cancelled)?;
        if filled as u64 > bound {
            return Err(Whole::TooLarge);
        }
        // One byte past the bound tells a source of exactly `bound` bytes from a longer one.
        let rest = bound.saturating_add(1) - filled as u64;
        let step = usize::try_from(rest).map_or(READ_STEP, |rest| rest.min(READ_STEP));
        bytes.resize(filled + step, 0);
        match r.read(&mut bytes[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(Whole::Io(e)),
        }
    }
    bytes.truncate(filled);
    Ok(bytes)
}

/// The length of `r` read to its end (an input whose reported size is not trusted), at most `bound` bytes;
/// more is [`Whole::TooLarge`].
fn measure(r: &mut dyn ReadSeek, bound: u64, ctl: &Ctl<'_>) -> Result<u64, Whole> {
    let mut buf = vec![0u8; 64 << 10];
    let mut filled = 0u64;
    loop {
        ctl.check().map_err(|_| Whole::Cancelled)?;
        if filled > bound {
            return Err(Whole::TooLarge);
        }
        match r.read(&mut buf) {
            Ok(0) => return Ok(filled),
            Ok(n) => filled += n as u64,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(Whole::Io(e)),
        }
    }
}

/// A sequential reader over the kept handle (the visitor's streaming path): positional reads from its own
/// position.
struct PositionalReader<'d> {
    doc: &'d HexDocument,
    at: u64,
}

impl Read for PositionalReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let want = (self.doc.len.saturating_sub(self.at)).min(buf.len() as u64) as usize;
        if want == 0 {
            return Ok(0);
        }
        let buf = &mut buf[..want];
        match self.doc.read_bulk(self.at, buf) {
            Ok(()) => {
                self.at += want as u64;
                Ok(want)
            }
            // The visitor names the source: a short read ends the stream.
            Err(HexError::Truncated { .. }) => Ok(0),
            Err(e) => Err(io::Error::other(e)),
        }
    }
}

impl ContentSource for HexDocument {
    fn name(&self) -> &str {
        &self.name
    }

    fn display(&self) -> String {
        self.display.clone()
    }

    fn len(&self) -> Option<u64> {
        Some(self.len)
    }

    fn local_path(&self) -> Option<&Path> {
        self.local.as_deref()
    }

    fn in_memory(&self) -> Option<&[u8]> {
        self.memory().map(|bytes| &bytes[..])
    }

    fn open(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        match &self.content {
            Content::Memory(bytes) => Ok(Box::new(io::Cursor::new(&bytes[..]))),
            Content::Paged { .. } => Ok(Box::new(PositionalReader { doc: self, at: 0 })),
        }
    }

    fn read_all(&self, limit: u64, cancel: &CancelToken) -> Result<Vec<u8>, ReadError> {
        cancel.check()?;
        if self.len > limit {
            return Err(ReadError::TooLarge { limit });
        }
        let mut bytes = vec![0u8; self.len as usize];
        for (k, piece) in bytes.chunks_mut(READ_STEP).enumerate() {
            cancel.check()?;
            self.read_bulk((k * READ_STEP) as u64, piece)
                .map_err(|e| ReadError::Io(io::Error::other(e)))?;
        }
        Ok(bytes)
    }
}
