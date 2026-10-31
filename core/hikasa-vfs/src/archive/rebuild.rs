//! The rebuild of an archive (M2 W5b, D-149; §1.5.10): the bytes of a new container, from an index and its overlay.
//!
//! What a rebuild did not change it keeps as bytes: raw zip records (`zip_write`), raw tar blocks, by one forward pass
//! over the decoded stream. Whatever lies between tar records (global headers, volume labels) goes along, because the
//! pass copies everything that no dropped record owns. Refused and encrypted members, sparse blocks and links are
//! never decoded. Shadowed duplicates go, with a warning.
//!
//! This module only writes to the `out` it is given: `batch` decides where that is and commits it.

use std::cell::Cell;
use std::collections::HashMap;
use std::io::{self, Cursor, Read, Write};
use std::sync::Arc;

use flate2::Compression;
use hikasa_core::{Ctl, FileTime, Phase, Progress, Spec, Unit, WarningCode};
use tar::{EntryType, Header};
use zip::ZipWriter;
use zip::write::FullFileOptions;
use zip::{CompressionMethod, DateTime};

use super::batch::LevelView;
use super::cache::{CacheFile, CacheWriter};
use super::codec::{self, Codec, Guard, Input, InputSize, Pass, STEP, corrupt, read_failure};
use super::format::ArchiveFormat;
use super::index::{DataRef, MemberKind, NO_RECORD, Node, RecordState};
use super::name::{NameRules, check_name};
use super::overlay::{Item, Origin};
use super::tar_walk::pax_records;
use super::time::{DosFields, ntfs_ticks, unix_field};
use super::zip_read::directory_limit;
use super::zip_write::{
    Central, NewTime, Patch, ZipOut, directory_end, dos_external, member_name, read_central,
    record_starts, records_end, unix_external, zip64_data,
};
use super::{ArchiveConfig, Core, open_stream};
use crate::entry::Attrs;
use crate::error::{VfsError, VfsOp};
use crate::vfs::ReadSeek;

/// Levels of a rebuild (§1.5.10): the formats' usual defaults, not options.
const DEFLATE_LEVEL: i64 = 6;
const GZIP_LEVEL: u32 = 6;
const BZIP2_LEVEL: u32 = 9;
const XZ_PRESET: u32 = 6;
const ZSTD_LEVEL: i32 = 3;
/// A tar block.
const BLOCK: usize = 512;
/// Tar's two zero blocks.
const EMPTY_TAR: [u8; 2 * BLOCK] = [0; 2 * BLOCK];
/// `large_file` from here on.
const LARGE_FILE: u64 = 0xffff_ffff;
/// The 0x5455 field.
const TIMESTAMP_EXTRA: u16 = 0x5455;
/// Unix file type bits.
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;
const S_IFLNK: u32 = 0o120_000;

/// What becomes of one record.
#[derive(Clone, Copy)]
enum Fate<'a> {
    Keep,
    Drop,
    /// Same data, under a new path or with new metadata.
    Rewrite(&'a str, &'a Item),
}

/// A member written after the kept ones.
struct New<'a> {
    path: &'a str,
    item: &'a Item,
    /// Its content, when it has one.
    file: Option<Arc<CacheFile>>,
}

/// The output stream of a rebuild: counts bytes, looks at `ctl` each 1 MiB, and tells its own failures from those of
/// the reading side.
struct Output<'a> {
    inner: &'a mut dyn Write,
    written: u64,
    next_check: u64,
    ctl: Ctl<'a>,
    spec: &'a str,
    failed: &'a Cell<bool>,
    total: &'a Cell<u64>,
}

impl Write for Output<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self
            .inner
            .write(buf)
            .inspect_err(|_| self.failed.set(true))?;
        self.written += n as u64;
        self.total.set(self.written);
        if self.written >= self.next_check {
            self.next_check = self.written + STEP;
            self.ctl.cancel.check().map_err(io::Error::other)?;
            let (done, spec) = (self.written, self.spec);
            self.ctl.progress(Phase::Writing, || Progress {
                phase: Phase::Writing,
                done,
                total: None,
                unit: Unit::Bytes,
                current: Some(spec.to_owned()),
            });
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush().inspect_err(|_| self.failed.set(true))
    }
}

/// The compression of a tar stream.
enum Encoder<W: Write> {
    Plain(W),
    Gz(flate2::write::GzEncoder<W>),
    Bz2(bzip2::write::BzEncoder<W>),
    Xz(lzma_rust2::XzWriter<W>),
    Zst(zstd::stream::write::Encoder<'static, W>),
}

impl<W: Write> Encoder<W> {
    /// §1.5.10: gzip without time or name, so rebuilds are repeatable; xz with CRC64, zstd with its checksum.
    fn new(format: ArchiveFormat, out: W) -> io::Result<Encoder<W>> {
        Ok(match Codec::of(format) {
            None => Encoder::Plain(out),
            Some(Codec::Gz) => Encoder::Gz(
                flate2::GzBuilder::new()
                    .mtime(0)
                    .operating_system(255)
                    .write(out, Compression::new(GZIP_LEVEL)),
            ),
            Some(Codec::Bz2) => Encoder::Bz2(bzip2::write::BzEncoder::new(
                out,
                bzip2::Compression::new(BZIP2_LEVEL),
            )),
            Some(Codec::Xz) => Encoder::Xz(lzma_rust2::XzWriter::new(
                out,
                lzma_rust2::XzOptions::with_preset(XZ_PRESET),
            )?),
            Some(Codec::Zst) => {
                let mut encoder = zstd::stream::write::Encoder::new(out, ZSTD_LEVEL)?;
                encoder.include_checksum(true)?;
                Encoder::Zst(encoder)
            }
        })
    }

    fn finish(self) -> io::Result<W> {
        match self {
            Encoder::Plain(out) => Ok(out),
            Encoder::Gz(encoder) => encoder.finish(),
            Encoder::Bz2(encoder) => encoder.finish(),
            Encoder::Xz(encoder) => encoder.finish(),
            Encoder::Zst(encoder) => encoder.finish(),
        }
    }
}

impl<W: Write> Write for Encoder<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Encoder::Plain(out) => out.write(buf),
            Encoder::Gz(out) => out.write(buf),
            Encoder::Bz2(out) => out.write(buf),
            Encoder::Xz(out) => out.write(buf),
            Encoder::Zst(out) => out.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Encoder::Plain(out) => out.flush(),
            Encoder::Gz(out) => out.flush(),
            Encoder::Bz2(out) => out.flush(),
            Encoder::Xz(out) => out.flush(),
            Encoder::Zst(out) => out.flush(),
        }
    }
}

/// Counts the decoded bytes of a new tar stream: a stream past `stream_max` is one no reader would open (rule 12).
struct Stream<W> {
    inner: W,
    written: u64,
    max: u64,
    spec: String,
}

impl<W: Write> Write for Stream<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.written += n as u64;
        if self.written > self.max {
            return Err(io::Error::from(VfsError::ArchiveLimit {
                spec: self.spec.clone(),
                limit: "stream",
                value: self.max,
            }));
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// `len` bytes of `inner`, or an error: a short read must not leave a member with a header that promises more.
struct Exact<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Read for Exact<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Ok(0);
        }
        let want = usize::try_from(self.left)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let n = self.inner.read(buf.get_mut(..want).unwrap_or_default())?;
        if n == 0 && want > 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.left -= n as u64;
        Ok(n)
    }
}

/// `seconds` as a tar header holds them: whole, and never before the epoch.
fn tar_seconds(time: Option<FileTime>) -> u64 {
    time.map_or(0, |t| u64::try_from(t.unix_seconds()).unwrap_or(0))
}

/// The padded length of a tar body.
fn padded(len: usize) -> usize {
    len.div_ceil(BLOCK) * BLOCK
}

/// One PAX record.
fn pax_record(out: &mut Vec<u8>, key: &[u8], value: &[u8]) {
    // The length counts its own digits.
    let body = key.len() + value.len() + 3;
    let mut digits = body.to_string().len();
    while (body + digits).to_string().len() != digits {
        digits = (body + digits).to_string().len();
    }
    out.extend_from_slice((body + digits).to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(key);
    out.push(b'=');
    out.extend_from_slice(value);
    out.push(b'\n');
}

/// The blocks of a PAX header with `body`.
fn pax_blocks(body: &[u8]) -> io::Result<Vec<u8>> {
    let mut header = Header::new_ustar();
    header.set_path("PaxHeaders/member")?;
    header.set_entry_type(EntryType::XHeader);
    header.set_mode(0o644);
    header.set_size(body.len() as u64);
    header.set_mtime(0);
    header.set_cksum();
    let mut out = header.as_bytes().to_vec();
    out.extend_from_slice(body);
    out.resize(out.len().next_multiple_of(BLOCK), 0);
    Ok(out)
}

/// What a rewritten tar member keeps of its old blocks.
struct OldHeaders {
    /// Extension blocks that still apply: a long link name, the PAX records that are not about the name.
    blocks: Vec<u8>,
    /// Its header: owner, type, device numbers and link name.
    header: Header,
}

/// Reads the blocks before a member's data (`raw`). `keep_time`: a PAX `mtime` is still true.
fn old_headers(raw: &[u8], keep_time: bool, spec: &str) -> Result<OldHeaders, VfsError> {
    let bad = || corrupt(spec, "a member's headers changed since they were read");
    let mut blocks = Vec::new();
    let mut at = 0;
    loop {
        let block = raw.get(at..at + BLOCK).ok_or_else(bad)?;
        let header = Header::from_byte_slice(block);
        let size = header.entry_size().map_err(|_| bad())?;
        let size = usize::try_from(size).map_err(|_| bad())?;
        let next = at + BLOCK + padded(size);
        match block.get(156).copied().unwrap_or(0) {
            b'x' | b'X' => {
                let body = raw.get(at + BLOCK..at + BLOCK + size).ok_or_else(bad)?;
                let mut kept = Vec::with_capacity(body.len());
                pax_records(body, &mut |key, value| {
                    let gone = matches!(key, b"path" | b"GNU.sparse.name")
                        || (key == b"mtime" && !keep_time);
                    if !gone {
                        pax_record(&mut kept, key, value);
                    }
                });
                if !kept.is_empty() {
                    let extension = pax_blocks(&kept).map_err(|e| corrupt(spec, e.to_string()))?;
                    blocks.extend_from_slice(&extension);
                }
            }
            b'L' => {}
            b'K' | b'g' | b'V' => {
                blocks.extend_from_slice(raw.get(at..next).ok_or_else(bad)?);
            }
            _ => {
                if at + BLOCK != raw.len() {
                    return Err(bad());
                }
                return Ok(OldHeaders {
                    blocks,
                    header: header.clone(),
                });
            }
        }
        at = next;
    }
}

/// The bytes of an empty archive (§1.5.10).
pub(super) fn empty_archive(format: ArchiveFormat) -> io::Result<Vec<u8>> {
    if format == ArchiveFormat::Zip {
        let zip = ZipWriter::new(Cursor::new(Vec::new()));
        return Ok(zip.finish().map_err(io::Error::other)?.into_inner());
    }
    let mut encoder = Encoder::new(format, Vec::new())?;
    encoder.write_all(&EMPTY_TAR)?;
    encoder.finish()
}

/// One rebuild.
pub(super) struct Rebuild<'a> {
    pub(super) core: &'a Core,
    pub(super) spec: &'a Spec,
    pub(super) view: &'a LevelView,
    pub(super) config: &'a ArchiveConfig,
    pub(super) ctl: Ctl<'a>,
}

impl Rebuild<'_> {
    fn text(&self) -> String {
        self.spec.canonical()
    }

    fn node(&self, at: u32) -> Option<&Node> {
        self.view.index.tree.nodes.get(at as usize)
    }

    /// `ArchiveLimit` unless what is written can be read again: rules 11 and 13 bind the new archive too.
    fn check_counts(&self, text: &str) -> Result<(), VfsError> {
        let limits = self.config.limits;
        let tree = &self.view.index.tree;
        let entries = self.view.overlay.len() as u64 + tree.refused_total;
        if entries > limits.entries_max {
            return Err(limit(text, "entries", limits.entries_max));
        }
        let names: usize = self
            .view
            .overlay
            .items()
            .map(|(path, item)| path.len() + item.link_target.as_ref().map_or(0, |t| t.len()))
            .sum();
        // Rows stop at 1,000: past them a hostile archive had to pass this limit to be read at all.
        let refused: usize = tree.refused.iter().map(|row| row.name.len()).sum();
        if (names + refused) as u64 > limits.paths_max {
            return Err(limit(text, "paths", limits.paths_max));
        }
        Ok(())
    }

    /// The ratio of rule 11 of what was written: past it no reader would decode the new archive.
    fn check_ratio(&self, decoded: u64, written: u64, text: &str) -> Result<(), VfsError> {
        let limits = self.config.limits;
        let max = u64::from(limits.ratio_max);
        if max > 0 && decoded > limits.ratio_floor && decoded / written.max(1) > max {
            return Err(limit(text, "ratio", max));
        }
        Ok(())
    }

    /// What becomes of each record, and the members written after the kept ones, sorted by path.
    fn plan(&self) -> Result<(Vec<Fate<'_>>, Vec<New<'_>>), VfsError> {
        let tree = &self.view.index.tree;
        let mut fates: Vec<Fate<'_>> = tree
            .records
            .iter()
            .map(|record| match record.state {
                RecordState::Shadowed => Fate::Drop,
                RecordState::Live | RecordState::Refused => Fate::Keep,
            })
            .collect();
        let shadowed = tree
            .records
            .iter()
            .filter(|record| record.state == RecordState::Shadowed)
            .count();
        if shadowed > 0 {
            self.core.warn(
                WarningCode::ArchiveDuplicateMember,
                format!("{shadowed} members that another of the same name shadowed were dropped"),
                self.spec,
            );
        }
        for node in tree.nodes.iter().filter(|node| node.record != NO_RECORD) {
            set_fate(&mut fates, node.record, Fate::Drop);
        }
        let mut new: Vec<New<'_>> = Vec::new();
        for (path, item) in self.view.overlay.items() {
            match &item.origin {
                Origin::Kept {
                    node,
                    moved,
                    touched,
                } => {
                    let fate = if *moved || *touched {
                        Fate::Rewrite(path, item)
                    } else {
                        Fate::Keep
                    };
                    let record = self.node(*node).map_or(NO_RECORD, |node| node.record);
                    set_fate(&mut fates, record, fate);
                }
                Origin::Implicit if self.view.overlay.has_children(path) => {}
                Origin::Implicit | Origin::Fresh => new.push(New {
                    path,
                    item,
                    file: None,
                }),
                Origin::Staged(file) => new.push(New {
                    path,
                    item,
                    file: Some(Arc::clone(file)),
                }),
            }
        }
        if !tree.hard_links.is_empty() {
            self.keep_hard_links(&mut fates, &mut new)?;
        }
        new.sort_by(|a, b| a.path.cmp(b.path));
        Ok((fates, new))
    }

    /// A hard link that loses the member it names becomes a regular member with its content, in archive order, so a
    /// chain of links is decided from its first. A link that moves does too: its record names the member, not itself.
    fn keep_hard_links<'a>(
        &'a self,
        fates: &mut [Fate<'a>],
        new: &mut Vec<New<'a>>,
    ) -> Result<(), VfsError> {
        let tree = &self.view.index.tree;
        let nodes: HashMap<u32, &Node> = tree.nodes.iter().map(|n| (n.record, n)).collect();
        let mut owners: HashMap<u32, (&str, &Item)> = HashMap::new();
        for (path, item) in self.view.overlay.items() {
            if let Origin::Kept { node, .. } = item.origin
                && let Some(node) = self.node(node)
            {
                owners.insert(node.record, (path, item));
            }
        }
        for &(record, target) in &tree.hard_links {
            let (Some(&(path, item)), Some(&node)) = (owners.get(&record), nodes.get(&record))
            else {
                continue;
            };
            let target_stays = match fates.get(target as usize).copied() {
                Some(Fate::Keep) => true,
                Some(Fate::Rewrite(to, _)) => nodes.get(&target).is_some_and(|n| n.path() == to),
                Some(Fate::Drop) | None => false,
            };
            let stays = matches!(fates.get(record as usize), Some(Fate::Keep));
            if !(stays && target_stays) {
                set_fate(fates, record, Fate::Drop);
                let file = self.stage_member(node, path)?;
                new.push(New {
                    path,
                    item,
                    file: Some(file),
                });
            }
        }
        Ok(())
    }

    /// Stages the content of `node`, read from the archive as it stands.
    fn stage_member(&self, node: &Node, path: &str) -> Result<Arc<CacheFile>, VfsError> {
        let text = self.text();
        let member = Spec::archive_member(self.spec.clone(), path);
        let mut reader = self.core.read_node(
            &member,
            self.spec,
            &self.view.slot,
            &self.view.index,
            node,
            self.config,
            self.ctl,
        )?;
        let mut stage: CacheWriter =
            self.core
                .cache
                .stage(&self.config.cache_root, self.config.limits.cache_max, &text)?;
        let mut buffer = vec![0u8; codec::BUFFER];
        loop {
            self.ctl.check()?;
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => stage.write(buffer.get(..n).unwrap_or_default())?,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(VfsError::from_io(text.as_str(), VfsOp::Read, error)),
            }
        }
        stage.finish_staged()
    }

    /// Writes the new archive to `out`. Every reader of the container is closed when this returns: Windows cannot
    /// rename over a file this process holds without sharing.
    pub(super) fn run(&self, out: &mut dyn Write) -> Result<(), VfsError> {
        let text = self.text();
        self.check_counts(&text)?;
        let (fates, new) = self.plan()?;
        if self.view.index.format == ArchiveFormat::Zip {
            self.zip(out, &fates, &new, &text)
        } else {
            self.tar(out, &fates, &new, &text)
        }
    }

    // ---- zip ------------------------------------------------------------------------------------------------

    fn zip(
        &self,
        out: &mut dyn Write,
        fates: &[Fate<'_>],
        new: &[New<'_>],
        text: &str,
    ) -> Result<(), VfsError> {
        let index = &self.view.index;
        let Some(data) = &index.zip else {
            return Err(corrupt(text, "inconsistent index"));
        };
        if data.prefix() != 0 {
            return Err(VfsError::Unsupported {
                spec: text.to_owned(),
                what: "rewriting a zip with a prefix",
            });
        }
        let limits = self.config.limits;
        let mut input = self.core.open_container(self.spec, self.config, self.ctl)?;
        let start = data.directory_start();
        let headers = read_central(&mut *input, start, directory_limit(&limits), text)?;
        let by_offset: HashMap<u64, usize> = data.directory_offsets().collect();
        let starts = record_starts(&headers);
        // A prefix the offsets count in, which `ZipArchive::offset` does not see: what lies before the first record
        // belongs to no member, and a stub that runs the archive would go.
        if starts.first().copied().unwrap_or(start) != 0 {
            return Err(VfsError::Unsupported {
                spec: text.to_owned(),
                what: "rewriting a zip with a prefix",
            });
        }
        let end = directory_end(&headers, start);
        let extension = zip64_data(&mut *input, end, directory_limit(&limits), text)?;
        let tail = records_end(&mut *input, end, text)?
            .filter(|&tail| tail < data.container_len())
            .map(|tail| (tail, data.container_len() - tail));
        // Records are bytes, not decoded output, so a sound zip of any size can be rewritten. The limit is there for
        // records that overlap: each would be written whole.
        let staged: u64 = new.iter().map(|member| member.item.size).sum();
        let output_max = data
            .container_len()
            .saturating_add(staged)
            .saturating_add(limits.stream_max);
        let mut zip = ZipOut::new(out, self.ctl, text, output_max);
        let mut hidden = 0u64;
        let mut seen = 0usize;
        for header in &headers {
            let fate = match by_offset.get(&header.at) {
                Some(&record) => {
                    seen += 1;
                    fates.get(record).copied().unwrap_or(Fate::Keep)
                }
                // The crate keeps one record per name, so the others have no number. A refused name stays; any
                // other is a shadowed duplicate.
                None if check_name(header.name(), NameRules::Zip).is_err() => Fate::Keep,
                None => {
                    hidden += 1;
                    Fate::Drop
                }
            };
            match fate {
                Fate::Drop => {}
                Fate::Keep => zip.record(&mut *input, header, &Patch::default(), &starts, start)?,
                Fate::Rewrite(path, item) => {
                    let patch = self.patch(header, path, item)?;
                    zip.record(&mut *input, header, &patch, &starts, start)?;
                }
            }
        }
        if seen != by_offset.len() {
            return Err(corrupt(text, "a central directory that changed"));
        }
        if hidden > 0 {
            self.core.warn(
                WarningCode::ArchiveDuplicateMember,
                format!("{hidden} members that another of the same name shadowed were dropped"),
                self.spec,
            );
        }
        if !new.is_empty() {
            self.append_new(&mut zip, new, text)?;
        }
        let declared = zip.declared();
        let mut written = zip.finish(data.comment(), extension.as_deref())?;
        if let Some((tail, len)) = tail {
            zip.tail(&mut *input, tail, len)?;
            written += len;
        }
        self.check_ratio(declared, written, text)
    }

    /// What a touched or renamed zip member changes in its headers. Its data is never decoded, so this also works on
    /// a member this build cannot read; an encrypted one keeps its DOS time, which may be its password check.
    fn patch(&self, header: &Central, path: &str, item: &Item) -> Result<Patch, VfsError> {
        let Origin::Kept { node, .. } = item.origin else {
            return Err(corrupt(path, "inconsistent index"));
        };
        let node = self
            .node(node)
            .ok_or_else(|| corrupt(path, "inconsistent index"))?;
        let mut patch = Patch::default();
        if path != node.path() {
            patch.name = Some(member_name(path, item.kind == MemberKind::Dir));
        }
        if item.mtime != node.mtime
            && let Some(mtime) = item.mtime
        {
            let zone = (self.config.clock)().time_zone().clone();
            let dos = DosFields::of(mtime, &zone);
            patch.time = Some(NewTime {
                dos: (!node.encrypted).then(|| (dos.date(), dos.time())),
                unix: unix_field(mtime),
                ntfs: ntfs_ticks(mtime),
            });
        }
        if item.attrs != node.attrs {
            let old = header.external();
            patch.attrs = match item.attrs {
                Attrs::Unix { meta } => Some(unix_external(old, type_bits(item.kind), meta.mode)),
                Attrs::Dos { attrs } => Some(dos_external(
                    old,
                    u8::try_from(attrs.bits() & 0xff).unwrap_or(0),
                )),
                Attrs::None => None,
            };
        }
        Ok(patch)
    }

    /// New members: `ZipWriter` writes them to a zip of their own, in the cache, and its records are carried over like
    /// the kept ones. It needs a seekable file, and the kept records need no `ZipWriter`.
    fn append_new(
        &self,
        zip: &mut ZipOut<'_>,
        new: &[New<'_>],
        text: &str,
    ) -> Result<(), VfsError> {
        // The members are in the cache already: this file is their second copy, and must not fail for that.
        let staged: u64 = new.iter().map(|member| member.item.size).sum();
        let room = self.config.limits.cache_max.saturating_add(staged);
        let stage = self.core.cache.stage(&self.config.cache_root, room, text)?;
        let now = (self.config.clock)();
        let zone = now.time_zone().clone();
        let now = FileTime::from_unix(now.timestamp().as_second(), 0);
        let mut writer = ZipWriter::new(stage);
        let fail = |e: zip::result::ZipError| match e {
            zip::result::ZipError::Io(error) => VfsError::from_io(text, VfsOp::Write, error),
            other => corrupt(text, other.to_string()),
        };
        let mut buffer = vec![0u8; codec::BUFFER];
        for member in new {
            self.ctl.check()?;
            let options = file_options(member.item, now, &zone).map_err(fail)?;
            let name = member.path;
            match member.item.kind {
                MemberKind::Dir => writer.add_directory(name, options).map_err(fail)?,
                MemberKind::Symlink => {
                    let target = member.item.link_target.as_deref().unwrap_or_default();
                    writer.add_symlink(name, target, options).map_err(fail)?;
                }
                _ => {
                    let large = member.item.size >= LARGE_FILE;
                    writer
                        .start_file(name, options.large_file(large))
                        .map_err(fail)?;
                    let Some(file) = &member.file else {
                        return Err(corrupt(text, "inconsistent overlay"));
                    };
                    let mut reader = file.reader()?;
                    loop {
                        self.ctl.check()?;
                        let n = match reader.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                            Err(e) => return Err(VfsError::from_io(text, VfsOp::Read, e)),
                        };
                        writer
                            .write_all(buffer.get(..n).unwrap_or_default())
                            .map_err(|e| VfsError::from_io(text, VfsOp::Write, e))?;
                    }
                }
            }
        }
        let stage = writer.finish().map_err(fail)?;
        let staged = stage.finish_staged()?;
        let mut reader = staged.reader()?;
        let archive = zip::ZipArchive::new(&mut reader).map_err(fail)?;
        let start = archive.central_directory_start();
        drop(archive);
        let mut input: Box<dyn ReadSeek> = Box::new(reader);
        let headers = read_central(&mut *input, start, staged.len(), text)?;
        let starts = record_starts(&headers);
        for header in &headers {
            zip.record(&mut *input, header, &Patch::default(), &starts, start)?;
        }
        Ok(())
    }

    // ---- tar ------------------------------------------------------------------------------------------------

    fn tar(
        &self,
        out: &mut dyn Write,
        fates: &[Fate<'_>],
        new: &[New<'_>],
        text: &str,
    ) -> Result<(), VfsError> {
        let index = &self.view.index;
        let format = index.format;
        let codec = Codec::of(format);
        let limits = self.config.limits;
        let reader = self.core.open_container(self.spec, self.config, self.ctl)?;
        let pass = Pass {
            spec: text,
            limits,
            ctl: self.ctl,
            zone: jiff::tz::TimeZone::UTC,
        };
        let input = Arc::new(Input::default());
        let write_failed = Cell::new(false);
        let written = Cell::new(0);
        let fail = |error: io::Error| {
            if write_failed.get() {
                VfsError::from_io(text, VfsOp::Write, error)
            } else {
                read_failure(error, Some(&input), text)
            }
        };
        let stream = open_stream(reader, codec, &input, &pass)?;
        // As for an index: a plain tar decodes nothing, so a backup of any size can be rewritten (rule 12).
        let mut read_limits = limits;
        if codec.is_none() {
            read_limits.stream_max = u64::MAX;
        }
        let mut guard = Guard::new(
            stream,
            read_limits,
            self.ctl,
            text,
            InputSize::Live(Arc::clone(&input)),
        );
        let output = Output {
            inner: out,
            written: 0,
            next_check: STEP,
            ctl: self.ctl,
            spec: text,
            failed: &write_failed,
            total: &written,
        };
        let encoder = Encoder::new(format, output).map_err(fail)?;
        let mut builder = tar::Builder::new(Stream {
            inner: encoder,
            written: 0,
            max: if codec.is_none() {
                u64::MAX
            } else {
                limits.stream_max
            },
            spec: text.to_owned(),
        });
        let mut pos = 0u64;
        let mut buffer = vec![0u8; codec::BUFFER];
        for (record, fate) in index.tree.records.iter().zip(fates) {
            let DataRef::Tar {
                first_block,
                data,
                end,
            } = record.data
            else {
                continue;
            };
            let gap = first_block.saturating_sub(pos);
            let copy = Copy {
                buffer: &mut buffer,
                fail: &fail,
                text,
            };
            copy.run(&mut guard, &mut builder, gap, true)?;
            match *fate {
                Fate::Keep | Fate::Drop => {
                    let keep = matches!(fate, Fate::Keep);
                    let len = end - first_block;
                    Copy {
                        buffer: &mut buffer,
                        fail: &fail,
                        text,
                    }
                    .run(&mut guard, &mut builder, len, keep)?;
                }
                Fate::Rewrite(path, item) => {
                    let mut raw = Vec::new();
                    let head = data - first_block;
                    let mut sink = |bytes: &[u8]| {
                        raw.extend_from_slice(bytes);
                        Ok(())
                    };
                    codec::pump(&mut guard, head, &mut sink).map_err(fail)?;
                    let node = self.kept_node(item, text)?;
                    let time = item.mtime != node.mtime;
                    let mut old = old_headers(&raw, !time, text)?;
                    builder.get_mut().write_all(&old.blocks).map_err(fail)?;
                    if time {
                        old.header.set_mtime(tar_seconds(item.mtime));
                    }
                    if let Attrs::Unix { meta } = item.attrs {
                        old.header.set_mode(meta.mode);
                    }
                    old.header.set_size(node.size);
                    let name = if item.kind == MemberKind::Dir {
                        format!("{path}/")
                    } else {
                        path.to_owned()
                    };
                    let body = Exact {
                        inner: &mut guard,
                        left: node.size,
                    };
                    builder
                        .append_data(&mut old.header, name, body)
                        .map_err(fail)?;
                    let padding = (end - data).saturating_sub(node.size);
                    Copy {
                        buffer: &mut buffer,
                        fail: &fail,
                        text,
                    }
                    .run(&mut guard, &mut builder, padding, false)?;
                }
            }
            pos = end;
        }
        let rest = index.tree.members_end.saturating_sub(pos);
        Copy {
            buffer: &mut buffer,
            fail: &fail,
            text,
        }
        .run(&mut guard, &mut builder, rest, true)?;
        for member in new {
            self.ctl.check()?;
            self.append_tar(&mut builder, member, &fail, text)?;
        }
        let mut stream_out = builder.into_inner().map_err(fail)?;
        if index.tree.tail {
            let mut sink = |bytes: &[u8]| stream_out.write_all(bytes).map_err(fail);
            codec::pump(&mut guard, u64::MAX, &mut sink).map_err(fail)?;
        } else {
            codec::pump(&mut guard, u64::MAX, &mut |_| Ok(())).map_err(fail)?;
        }
        drop(guard);
        let decoded = stream_out.written;
        stream_out.inner.finish().map_err(fail)?;
        if codec.is_some() {
            self.check_ratio(decoded, written.get(), text)?;
        }
        Ok(())
    }

    fn kept_node(&self, item: &Item, text: &str) -> Result<&Node, VfsError> {
        match item.origin {
            Origin::Kept { node, .. } => self.node(node),
            _ => None,
        }
        .ok_or_else(|| corrupt(text, "inconsistent overlay"))
    }

    /// One new member (§1.5.10): GNU headers, uid and gid 0, whole seconds.
    fn append_tar<W: Write>(
        &self,
        builder: &mut tar::Builder<W>,
        member: &New<'_>,
        fail: &dyn Fn(io::Error) -> VfsError,
        text: &str,
    ) -> Result<(), VfsError> {
        let item = member.item;
        let mut header = Header::new_gnu();
        let folder = item.kind == MemberKind::Dir;
        let mode = if folder { 0o755 } else { 0o644 };
        header.set_mode(item.attrs.unix().map_or(mode, |meta| meta.mode));
        header.set_uid(0);
        header.set_gid(0);
        let now = (self.config.clock)().timestamp().as_second();
        let now = FileTime::from_unix(now, 0);
        header.set_mtime(tar_seconds(item.mtime.or(now)));
        header.set_size(0);
        let path = member.path;
        match item.kind {
            MemberKind::Dir => {
                header.set_entry_type(EntryType::Directory);
                builder
                    .append_data(&mut header, format!("{path}/"), io::empty())
                    .map_err(fail)
            }
            MemberKind::Symlink => {
                header.set_entry_type(EntryType::Symlink);
                let target = item.link_target.as_deref().unwrap_or_default();
                builder.append_link(&mut header, path, target).map_err(fail)
            }
            _ => {
                let Some(file) = &member.file else {
                    return Err(corrupt(text, "inconsistent overlay"));
                };
                header.set_entry_type(EntryType::Regular);
                header.set_size(file.len());
                let body = Exact {
                    inner: file.reader()?,
                    left: file.len(),
                };
                builder.append_data(&mut header, path, body).map_err(fail)
            }
        }
    }
}

fn set_fate<'a>(fates: &mut [Fate<'a>], record: u32, fate: Fate<'a>) {
    if let Some(slot) = fates.get_mut(record as usize) {
        *slot = fate;
    }
}

/// One run of blocks of the stream.
struct Copy<'a> {
    buffer: &'a mut [u8],
    fail: &'a dyn Fn(io::Error) -> VfsError,
    text: &'a str,
}

impl Copy<'_> {
    /// Passes `len` bytes of the stream on to the output, or over them.
    fn run<R: Read, W: Write>(
        self,
        guard: &mut Guard<'_, R>,
        builder: &mut tar::Builder<W>,
        len: u64,
        keep: bool,
    ) -> Result<(), VfsError> {
        let fail = self.fail;
        let mut sink = |bytes: &[u8]| {
            if keep {
                builder.get_mut().write_all(bytes).map_err(fail)?;
            }
            Ok(())
        };
        let copied = codec::pump_with(self.buffer, guard, len, &mut sink).map_err(fail)?;
        if copied != len {
            return Err(corrupt(self.text, "size mismatch"));
        }
        Ok(())
    }
}

fn limit(spec: &str, limit: &'static str, value: u64) -> VfsError {
    VfsError::ArchiveLimit {
        spec: spec.to_owned(),
        limit,
        value,
    }
}

/// The Unix type bits of a member.
fn type_bits(kind: MemberKind) -> u32 {
    match kind {
        MemberKind::Dir => S_IFDIR,
        MemberKind::Symlink => S_IFLNK,
        _ => S_IFREG,
    }
}

/// The options of a new zip member (§1.5.8, §1.5.10): DOS time in the injected zone plus a 5-byte 0x5455 field, unless
/// the time does not fit it. A folder and a link take the crate's way of saying what they are.
fn file_options(
    item: &Item,
    now: Option<FileTime>,
    zone: &jiff::tz::TimeZone,
) -> Result<FullFileOptions<'static, 'static>, zip::result::ZipError> {
    let mut options = FullFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(DEFLATE_LEVEL));
    if let Some(mtime) = item.mtime.or(now) {
        let dos = DosFields::of(mtime, zone);
        let time = DateTime::from_date_and_time(
            dos.year, dos.month, dos.day, dos.hour, dos.minute, dos.second,
        );
        if let Ok(time) = time {
            options = options.last_modified_time(time);
        }
        if let Some(seconds) = unix_field(mtime) {
            let mut field = vec![1u8];
            field.extend_from_slice(&seconds.to_le_bytes());
            options.add_extra_field(TIMESTAMP_EXTRA, field, false)?;
        }
    }
    Ok(match item.attrs {
        Attrs::Unix { meta } => options.unix_permissions(meta.mode),
        Attrs::Dos { attrs } if item.kind == MemberKind::File => options
            .system(zip::System::Dos)
            .external_attributes(u32::from(u8::try_from(attrs.bits() & 0xff).unwrap_or(0))),
        Attrs::Dos { .. } | Attrs::None => options,
    })
}
