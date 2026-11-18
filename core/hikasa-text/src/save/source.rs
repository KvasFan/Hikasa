//! [`SaveSource`]: the content of a save as a read-only `Vfs` with one file per item (§1.10.13), so that
//! `hikasa_vfs::ops` streams it like any source. Reads only through `&dyn Vfs` (§1.10.25 check 2).

use std::collections::BTreeMap;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

use hikasa_core::{CancelToken, Spec};
use hikasa_vfs::{
    AtomicWrite, Capabilities, Entry, EntryKind, FileName, FollowLinks, ListOptions, Listing,
    ReadSeek, Vfs, VfsError, VfsOp, WriteOptions, crc32_update,
};

use super::content::{Piece, SCAN_STEP, SaveContent};
use super::convert::ConvertPieces;
use super::{ItemContent, SavePlan};
use crate::edit::{EditableDocument, SpliceSource};

/// The reason of an item whose splice source changed between the plan and its execution.
const CHANGED: &str = "changed since the plan was made";

/// What one item streams, or why it cannot.
#[derive(Debug, Clone)]
enum Slot {
    Ready(ItemContent),
    Refused(&'static str),
}

/// The content of a save plan as a read-only back-end: `open_read(target)` streams the item's pieces in order — the
/// BOM, encoded runs, and the ranges of the splice source, which is read **once, sequentially, from its first byte to
/// its last** in steps of at most 1 MiB (the bytes between the ranges too), its CRC-32 and length compared with the
/// stamp at the end of the read: a mismatch fails the read, so the item fails before its commit and the target stays.
/// Memory: the pieces (the encoded text) and no buffer of its own (the caller's is used to step over bytes). The CRC of
/// the bytes each item streamed is kept for the item's new stamp.
pub struct SaveSource<'a> {
    vfs: &'a dyn Vfs,
    items: BTreeMap<String, Slot>,
    crcs: Arc<Mutex<BTreeMap<String, u32>>>,
}

impl std::fmt::Debug for SaveSource<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveSource")
            .field("items", &self.items.len())
            .finish()
    }
}

impl<'a> SaveSource<'a> {
    /// The content of `plan`, reading splice sources through `vfs`. `docs` are the plan's documents in the order of
    /// `plan_save`'s input (empty: not checked): an item whose document changed since the plan was made (another
    /// generation) is refused, and so is every item of a plan read back from its serialized form (its content is not
    /// serialized).
    pub fn new(vfs: &'a dyn Vfs, plan: &SavePlan, docs: &[&EditableDocument]) -> SaveSource<'a> {
        let check = docs.len() == plan.items.len();
        let mut items = BTreeMap::new();
        for (i, item) in plan.items.iter().enumerate() {
            let slot = match plan.contents.get(i).cloned().flatten() {
                Some(_)
                    if check
                        && docs
                            .get(i)
                            .is_some_and(|d| d.generation() != item.generation) =>
                {
                    Slot::Refused("the document changed since the plan was made")
                }
                Some(content) => Slot::Ready(content),
                None => Slot::Refused("the content of the save is not available: plan it again"),
            };
            items.insert(item.target.canonical(), slot);
        }
        SaveSource {
            vfs,
            items,
            crcs: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// The CRC-32 of the bytes streamed for `target` by a read that went to its end.
    pub fn written_crc(&self, target: &Spec) -> Option<u32> {
        self.crcs
            .lock()
            .ok()
            .and_then(|crcs| crcs.get(&target.canonical()).copied())
    }

    fn slot(&self, spec: &Spec) -> Result<&ItemContent, VfsError> {
        match self.items.get(&spec.canonical()) {
            Some(Slot::Ready(content)) => Ok(content),
            Some(Slot::Refused(reason)) => Err(VfsError::InvalidTarget {
                spec: spec.canonical(),
                reason,
            }),
            None => Err(VfsError::NotFound {
                spec: spec.canonical(),
            }),
        }
    }
}

impl Vfs for SaveSource<'_> {
    fn capabilities(&self, _at: &Spec) -> Result<Capabilities, VfsError> {
        Ok(Capabilities::empty())
    }

    fn stat(&self, spec: &Spec, _follow: FollowLinks) -> Result<Entry, VfsError> {
        let content = self.slot(spec)?;
        let name = FileName::new(&spec.file_name().unwrap_or_else(|| "content".to_owned()))
            .unwrap_or_else(|_| FileName::from_os(std::ffi::OsStr::new("content")));
        let mut entry = Entry::new(name, EntryKind::File);
        entry.size = content.content.bytes;
        Ok(entry)
    }

    fn list(
        &self,
        dir: &Spec,
        _opts: &ListOptions<'_>,
        _cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        Err(VfsError::NotADirectory {
            spec: dir.canonical(),
        })
    }

    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        let content = self.slot(spec)?.clone();
        // The splice source is opened now: its stamp is checked at the end of the read.
        let reader = match &content.source {
            Some(source) => Some(self.vfs.open_read(&source.spec)?),
            None => None,
        };
        Ok(Box::new(PieceReader {
            spec: spec.canonical(),
            pieces: content.content,
            source: content.source,
            at: 0,
            within: 0,
            made: None,
            sub: None,
            streamed: 0,
            reader,
            read: 0,
            crc_in: 0,
            crc_out: 0,
            ended: false,
            crcs: Arc::clone(&self.crcs),
        }))
    }

    fn open_write(
        &self,
        spec: &Spec,
        _opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        Err(VfsError::OperationUnsupported {
            spec: spec.canonical(),
            op: VfsOp::Write,
        })
    }

    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        Err(VfsError::NotFound {
            spec: format!("{}/{}", dir.canonical(), name.as_str()),
        })
    }
}

/// The stream of one item: its pieces, the splice source read once.
struct PieceReader {
    spec: String,
    pieces: Arc<SaveContent>,
    source: Option<SpliceSource>,
    /// The piece being streamed.
    at: usize,
    /// Bytes of it already streamed.
    within: u64,
    /// The pieces a `Converted` piece is making, and the one of them being streamed.
    made: Option<ConvertPieces>,
    sub: Option<Piece>,
    /// Bytes streamed.
    streamed: u64,
    /// The splice source.
    reader: Option<Box<dyn ReadSeek>>,
    /// Bytes of the splice source read so far, and their CRC.
    read: u64,
    crc_in: u32,
    /// The CRC of the bytes streamed.
    crc_out: u32,
    ended: bool,
    crcs: Arc<Mutex<BTreeMap<String, u32>>>,
}

/// A read error naming the item.
fn changed(spec: &str) -> io::Error {
    io::Error::other(VfsError::InvalidTarget {
        spec: spec.to_owned(),
        reason: CHANGED,
    })
}

/// "Come back": a step of skipped bytes was read; the caller checks its cancel token and reads again.
fn step() -> io::Error {
    io::Error::from(io::ErrorKind::Interrupted)
}

impl PieceReader {
    /// The splice source's reader.
    fn source_reader(&mut self) -> io::Result<&mut Box<dyn ReadSeek>> {
        self.reader.as_mut().ok_or_else(|| changed(&self.spec))
    }

    /// Reads at most `buf.len()` (and 1 MiB) bytes of the splice source into `buf`, CRC-ing them.
    fn read_source(&mut self, buf: &mut [u8], max: u64) -> io::Result<usize> {
        let want = (buf.len() as u64).min(max).min(SCAN_STEP as u64) as usize;
        let Some(room) = buf.get_mut(..want) else {
            return Ok(0);
        };
        let n = loop {
            match self.source_reader()?.read(room) {
                Ok(n) => break n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        };
        self.crc_in = crc32_update(self.crc_in, room.get(..n).unwrap_or_default());
        self.read += n as u64;
        Ok(n)
    }

    /// The end of the stream: the rest of the splice source read and checked against its stamp, the written CRC
    /// kept. `Ok(true)` when done, `Err(Interrupted)` after a step of the rest.
    fn finish(&mut self, buf: &mut [u8]) -> io::Result<()> {
        if let Some(source) = self.source.clone()
            && self.reader.is_some()
        {
            if self.read < source.stamp.size.saturating_add(1) {
                let n = self.read_source(buf, u64::MAX)?;
                if n > 0 {
                    return Err(step());
                }
            }
            if self.read != source.stamp.size || self.crc_in != source.stamp.crc32 {
                return Err(changed(&self.spec));
            }
        }
        self.ended = true;
        if let Ok(mut crcs) = self.crcs.lock() {
            crcs.insert(self.spec.clone(), self.crc_out);
        }
        Ok(())
    }
}

impl PieceReader {
    /// The piece being streamed (a piece a `Converted` one made, in its place); `None` after the last.
    fn current(&mut self) -> Option<Piece> {
        loop {
            if let Some(piece) = &self.sub {
                return Some(piece.clone());
            }
            match self.pieces.pieces.get(self.at)? {
                Piece::Converted(stream) => {
                    let made = self.made.get_or_insert_with(|| stream.pieces());
                    match made.next() {
                        Some(piece) => self.sub = Some(piece),
                        None => {
                            self.made = None;
                            self.at += 1;
                        }
                    }
                }
                other => return Some(other.clone()),
            }
        }
    }

    /// The piece being streamed is done.
    fn advance(&mut self) {
        self.within = 0;
        if self.sub.take().is_none() {
            self.at += 1;
        }
    }
}

impl Read for PieceReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.ended {
                return Ok(0);
            }
            let Some(piece) = self.current() else {
                self.finish(buf)?;
                continue;
            };
            let n = match &piece {
                Piece::Bom => copy_from(self.pieces.encoding.bom(), self.within, buf),
                Piece::Encoded(bytes) => copy_from(bytes, self.within, buf),
                Piece::Source { offset, len } => {
                    let from = offset + self.within;
                    if self.read < from {
                        // The bytes before the range: read and CRC-ed, not streamed, one step a call.
                        let gap = from - self.read;
                        let n = self.read_source(buf, gap)?;
                        if n == 0 {
                            return Err(changed(&self.spec));
                        }
                        return Err(step());
                    }
                    if self.read > from {
                        return Err(changed(&self.spec));
                    }
                    let n = self.read_source(buf, len - self.within)?;
                    if n == 0 {
                        return Err(changed(&self.spec));
                    }
                    n
                }
                Piece::Converted(_) => 0,
            };
            let total = match &piece {
                Piece::Bom => self.pieces.encoding.bom().len() as u64,
                Piece::Encoded(bytes) => bytes.len() as u64,
                Piece::Source { len, .. } => *len,
                Piece::Converted(_) => 0,
            };
            if n == 0 && self.within >= total {
                self.advance();
                continue;
            }
            self.within += n as u64;
            if self.within >= total {
                self.advance();
            }
            self.streamed += n as u64;
            self.crc_out = crc32_update(self.crc_out, buf.get(..n).unwrap_or_default());
            return Ok(n);
        }
    }
}

/// Copies `bytes[from..]` into `buf`, as much as fits.
fn copy_from(bytes: &[u8], from: u64, buf: &mut [u8]) -> usize {
    let rest = bytes.get(from as usize..).unwrap_or_default();
    let n = rest.len().min(buf.len());
    if let (Some(to), Some(from)) = (buf.get_mut(..n), rest.get(..n)) {
        to.copy_from_slice(from);
    }
    n
}

impl Seek for PieceReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        // A stream: only where it stands can be asked.
        let streamed = self.streamed;
        match to {
            SeekFrom::Current(0) => Ok(streamed),
            SeekFrom::Start(at) if at == streamed => Ok(streamed),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the content of a save is a stream",
            )),
        }
    }
}
