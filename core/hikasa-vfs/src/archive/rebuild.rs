//! The rebuild of an archive (M2 W5b, D-149; §1.5.10): the bytes of a new container, from an index and its overlay.
//!
//! What a rebuild did not change it keeps as bytes: raw zip records (`zip_write`), raw tar blocks, by one forward pass
//! over the decoded stream. Whatever lies between tar records (global headers, volume labels) goes along, because the
//! pass copies everything that no dropped record owns. Refused and encrypted members, sparse blocks and links are
//! never decoded. A shadowed duplicate shares the fate of the record that won its path (rule 3): it stays while that
//! record stays, follows it to a new path, and goes when it is removed or replaced, with a warning.
//!
//! This module only writes to the `out` it is given: `batch` decides where that is and commits it.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::io::{self, Cursor, Read, Write};
use std::sync::Arc;

use flate2::Compression;
use hikasa_core::{Ctl, FileTime, Phase, Progress, Spec, Unit, WarningCode};
use tar::{EntryType, Header};
use zip::ZipWriter;
use zip::write::FullFileOptions;
use zip::{CompressionMethod, DateTime};

use super::batch::{LevelView, Notes};
use super::cache::{CacheFile, CacheWriter};
use super::codec::{self, Codec, Guard, Input, InputSize, Pass, STEP, corrupt, read_failure};
use super::format::ArchiveFormat;
use super::index::{DataRef, MemberKind, NO_RECORD, Node, RecordState, Shadow};
use super::name::{Refusal, escape_name};
use super::overlay::{Item, Origin};
use super::tar_walk::{entry_size, pax_records};
use super::time::{DosFields, ntfs_ticks, unix_field};
use super::zip_read::directory_limit;
use super::zip_write::{
    Central, NewTime, Patch, ZipOut, directory_end, directory_signature, dos_external, moved_name,
    read_central, record_starts, records_end, unix_external, zip64_data,
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
    /// A shadowed record that follows the one that won its path, from the path they shared.
    Follow {
        from: &'a str,
        to: &'a str,
        folder: bool,
    },
    /// A hard link that lost what it names: a regular member at its own place, with the content of `Plan::staged`.
    Materialize(usize),
}

/// What [`Rebuild::plan`] decides.
struct Planned<'a> {
    fates: Vec<Fate<'a>>,
    new: Vec<New<'a>>,
    /// The content of each [`Fate::Materialize`].
    staged: Vec<Staged<'a>>,
    /// Shadowed records that went with their winner.
    dropped: u64,
}

/// A hard link written as a regular member.
struct Staged<'a> {
    path: &'a str,
    item: &'a Item,
    file: Arc<CacheFile>,
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
    /// Extension blocks that still apply.
    blocks: Vec<u8>,
    /// Its header: owner, type, device numbers and link name.
    header: Header,
    /// Bytes of data that follow the blocks: a PAX `size` before the header's.
    data_len: u64,
}

/// What of a member's blocks still holds.
#[derive(Clone, Copy)]
struct StillTrue {
    /// The time: a PAX `mtime`.
    time: bool,
    /// The name: a GNU long name, a PAX `path`, and the bytes in the header. Raw, so a name stays even if it does not
    /// decode.
    name: bool,
    /// That it is a link with no data of its own: a long link name, the PAX `linkpath` and `size`.
    link: bool,
}

/// Reads the blocks before a member's data (`raw`).
fn old_headers(raw: &[u8], still: StillTrue, spec: &str) -> Result<OldHeaders, VfsError> {
    let bad = || corrupt(spec, "a member's headers changed since they were read");
    let mut blocks = Vec::new();
    let mut pax_size = None;
    let mut at = 0;
    loop {
        let block = raw.get(at..at + BLOCK).ok_or_else(bad)?;
        let header = Header::from_byte_slice(block);
        let size = entry_size(header).map_err(|_| bad())?;
        let size = usize::try_from(size).map_err(|_| bad())?;
        let next = at + BLOCK + padded(size);
        match block.get(156).copied().unwrap_or(0) {
            b'x' | b'X' => {
                let body = raw.get(at + BLOCK..at + BLOCK + size).ok_or_else(bad)?;
                let mut kept = Vec::with_capacity(body.len());
                pax_records(body, &mut |key, value| {
                    if key == b"size" {
                        pax_size = std::str::from_utf8(value).ok().and_then(|t| t.parse().ok());
                    }
                    let gone = (matches!(key, b"path" | b"GNU.sparse.name") && !still.name)
                        || (key == b"mtime" && !still.time)
                        || (matches!(key, b"linkpath" | b"size") && !still.link);
                    if !gone {
                        pax_record(&mut kept, key, value);
                    }
                });
                if kept == body {
                    blocks.extend_from_slice(raw.get(at..next).ok_or_else(bad)?);
                } else if !kept.is_empty() {
                    let extension = pax_blocks(&kept).map_err(|e| corrupt(spec, e.to_string()))?;
                    blocks.extend_from_slice(&extension);
                }
            }
            b'L' if !still.name => {}
            b'K' if !still.link => {}
            b'L' | b'K' | b'g' | b'V' => {
                blocks.extend_from_slice(raw.get(at..next).ok_or_else(bad)?);
            }
            kind => {
                if at + BLOCK != raw.len() {
                    return Err(bad());
                }
                // A hard link has no data of its own, whatever its size field says, unless PAX states it.
                let own = if kind == b'1' { 0 } else { size as u64 };
                return Ok(OldHeaders {
                    blocks,
                    header: header.clone(),
                    data_len: pax_size.unwrap_or(own),
                });
            }
        }
        at = next;
    }
}

/// Clears the name fields of a header that gets a new name: the crate writes over what it needs and leaves the rest,
/// and a `prefix` left behind in a ustar header would join the new name.
fn clear_names(header: &mut Header) {
    header.as_old_mut().name.fill(0);
    if let Some(ustar) = header.as_ustar_mut() {
        ustar.prefix.fill(0);
    }
}

/// What a rewrite of one tar record changes.
struct Change<'a> {
    /// The path it had, and the one it gets.
    from: &'a str,
    to: &'a str,
    folder: bool,
    /// A new time.
    time: Option<Option<FileTime>>,
    mode: Option<u32>,
    /// New content; `None` keeps its own.
    body: Option<&'a Arc<CacheFile>>,
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
    /// What the rebuild has to say once it is in place.
    pub(super) notes: &'a Notes,
}

impl Rebuild<'_> {
    fn text(&self) -> String {
        self.spec.canonical()
    }

    fn node(&self, at: u32) -> Option<&Node> {
        self.view.index.tree.nodes.get(at as usize)
    }

    /// `ArchiveLimit` unless what is written can be read again: rules 11 and 13 bind the new archive too. The records
    /// of a shadow count: the next index reads them. A zip's hidden records are counted by [`Rebuild::check_records`].
    fn check_counts(&self, planned: &Planned<'_>, text: &str) -> Result<(), VfsError> {
        let limits = self.config.limits;
        let tree = &self.view.index.tree;
        let kept = |shadow: &&Shadow| {
            !matches!(
                planned.fates.get(shadow.record as usize),
                Some(Fate::Drop) | None
            )
        };
        let (shadows, shadow_text) = tree
            .shadows
            .iter()
            .filter(kept)
            .fold((0u64, 0u64), |(count, text), shadow| {
                (count + 1, text + u64::from(shadow.path_len))
            });
        let entries = self.view.overlay.len() as u64 + tree.refused_total + shadows;
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
        if (names + refused) as u64 + shadow_text > limits.paths_max {
            return Err(limit(text, "paths", limits.paths_max));
        }
        Ok(())
    }

    /// The records of a finished zip against rule 13: a central directory is what the next index counts, hidden
    /// records and all.
    fn check_records(&self, zip: &ZipOut<'_>, text: &str) -> Result<(), VfsError> {
        let limits = self.config.limits;
        if zip.entries() > limits.entries_max {
            return Err(limit(text, "entries", limits.entries_max));
        }
        if zip.names() > limits.paths_max {
            return Err(limit(text, "paths", limits.paths_max));
        }
        Ok(())
    }

    /// The ratio of rule 11 of what was written: past it no reader would decode the new archive. Only what was added
    /// can be refused for that: without new content the rebuild is a remove or a rename, which must not fail because
    /// the incompressible part went. It says so, since reads now need a higher limit.
    fn check_ratio(
        &self,
        grew: bool,
        decoded: u64,
        written: u64,
        text: &str,
    ) -> Result<(), VfsError> {
        let limits = self.config.limits;
        let max = u64::from(limits.ratio_max);
        if max > 0 && decoded > limits.ratio_floor && decoded / written.max(1) > max {
            if grew {
                return Err(limit(text, "ratio", max));
            }
            self.notes.push(
                self.spec,
                WarningCode::CapabilityMissing,
                format!("the archive now passes `archives.limits.ratio_max` ({max})"),
            );
        }
        Ok(())
    }

    /// What became of the winner `winner` is what becomes of a record it shadows.
    fn follow<'a>(&'a self, fates: &[Fate<'a>], winner: u32) -> Fate<'a> {
        match fates.get(winner as usize).copied() {
            Some(Fate::Drop) => Fate::Drop,
            Some(Fate::Follow { from, to, folder }) => Fate::Follow { from, to, folder },
            Some(Fate::Rewrite(to, item)) => match self.kept_path(item) {
                Some(from) if from != to => Fate::Follow {
                    from,
                    to,
                    folder: item.kind == MemberKind::Dir,
                },
                _ => Fate::Keep,
            },
            Some(Fate::Keep | Fate::Materialize(_)) | None => Fate::Keep,
        }
    }

    /// The path an item had in the archive.
    fn kept_path(&self, item: &Item) -> Option<&str> {
        match item.origin {
            Origin::Kept { node, .. } => self.node(node).map(Node::path),
            _ => None,
        }
    }

    /// What becomes of each record, and the members written after the kept ones, sorted by path. Shadows are decided
    /// before hard links: a link to a kept shadow stays a link, and a winner that is a link written as a file does not
    /// take its shadow along.
    fn plan(&self) -> Result<Planned<'_>, VfsError> {
        let tree = &self.view.index.tree;
        let mut fates: Vec<Fate<'_>> = vec![Fate::Keep; tree.records.len()];
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
        let mut dropped = 0;
        for shadow in &tree.shadows {
            let fate = self.follow(&fates, shadow.winner);
            dropped += u64::from(matches!(fate, Fate::Drop));
            set_fate(&mut fates, shadow.record, fate);
        }
        let mut staged = Vec::new();
        if !tree.hard_links.is_empty() {
            self.keep_hard_links(&mut fates, &mut staged, &mut dropped)?;
        }
        new.sort_by(|a, b| a.path.cmp(b.path));
        self.note_freed();
        Ok(Planned {
            fates,
            new,
            staged,
            dropped,
        })
    }

    /// A refused member stays in the archive, and one refused only for a conflict with a path that is gone reads as a
    /// member again: the operation that freed the path has to say so (§1.5.10).
    fn note_freed(&self) {
        let tree = &self.view.index.tree;
        let conflicts = || {
            tree.refused
                .iter()
                .filter(|row| row.reason == Refusal::Conflict)
        };
        if conflicts().next().is_none() {
            return;
        }
        let overlay = &self.view.overlay;
        let free: HashSet<String> = tree
            .nodes
            .iter()
            .filter(|node| overlay.get(node.path()).is_none())
            .map(|node| escape_name(node.path().as_bytes()))
            .collect();
        let back = conflicts().filter(|row| free.contains(&row.name)).count();
        if back > 0 {
            self.notes.push(
                self.spec,
                WarningCode::ArchiveMemberRefused,
                format!(
                    "{back} refused members stay in the archive and read at the path that came free"
                ),
            );
        }
    }

    /// A hard link that loses the member it names becomes a regular member with its content, at its own place in the
    /// archive, so a link that names it stays one. A link that moves does too: its record names the member, not
    /// itself. A shadowed link has no member to become, since a file would win, so it goes with what it names.
    fn keep_hard_links<'a>(
        &'a self,
        fates: &mut [Fate<'a>],
        staged: &mut Vec<Staged<'a>>,
        dropped: &mut u64,
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
            let target_stays = match fates.get(target as usize).copied() {
                Some(Fate::Keep) => true,
                Some(Fate::Rewrite(to, _)) => nodes.get(&target).is_some_and(|n| n.path() == to),
                Some(Fate::Materialize(at)) => staged
                    .get(at)
                    .zip(nodes.get(&target))
                    .is_some_and(|(file, node)| file.path == node.path()),
                Some(Fate::Drop | Fate::Follow { .. }) | None => false,
            };
            let (Some(&(path, item)), Some(&node)) = (owners.get(&record), nodes.get(&record))
            else {
                let shadow = tree
                    .records
                    .get(record as usize)
                    .is_some_and(|r| r.state == RecordState::Shadowed);
                let kept = !matches!(fates.get(record as usize), Some(Fate::Drop) | None);
                if shadow && kept && !target_stays {
                    set_fate(fates, record, Fate::Drop);
                    *dropped += 1;
                }
                continue;
            };
            let stays = matches!(fates.get(record as usize), Some(Fate::Keep));
            if !(stays && target_stays) {
                let file = self.stage_member(node, path)?;
                set_fate(fates, record, Fate::Materialize(staged.len()));
                staged.push(Staged { path, item, file });
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
        let planned = self.plan()?;
        self.check_counts(&planned, &text)?;
        let dropped = if self.view.index.format == ArchiveFormat::Zip {
            self.zip(out, &planned, &text)?
        } else {
            self.tar(out, &planned, &text)?;
            planned.dropped
        };
        if dropped > 0 {
            self.notes.push(
                self.spec,
                WarningCode::ArchiveDuplicateMember,
                format!("{dropped} shadowed members went with the member that replaced them"),
            );
        }
        Ok(())
    }

    // ---- zip ------------------------------------------------------------------------------------------------

    /// The zip, and how many shadowed records went.
    fn zip(&self, out: &mut dyn Write, planned: &Planned<'_>, text: &str) -> Result<u64, VfsError> {
        let Planned { fates, new, .. } = planned;
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
        if directory_signature(&mut *input, end, text)? {
            self.notes.push(
                self.spec,
                WarningCode::CapabilityMissing,
                "the signature of the central directory no longer applies and went".to_owned(),
            );
        }
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
        let mut dropped = planned.dropped;
        let mut seen = 0usize;
        for header in &headers {
            let fate = match by_offset.get(&header.at) {
                Some(&record) => {
                    seen += 1;
                    fates.get(record).copied().unwrap_or(Fate::Keep)
                }
                // The crate keeps one record per name, so the others have no number: what won that name says.
                None => match index.tree.hidden_winner(header.at) {
                    Some(winner) => self.follow(fates, winner),
                    None => Fate::Keep,
                },
            };
            match fate {
                Fate::Drop if by_offset.contains_key(&header.at) => {}
                Fate::Drop => dropped += 1,
                Fate::Keep | Fate::Materialize(_) => {
                    zip.record(&mut *input, header, &Patch::default(), &starts, start)?;
                }
                Fate::Rewrite(path, item) => {
                    let patch = self.patch(header, path, item)?;
                    zip.record(&mut *input, header, &patch, &starts, start)?;
                }
                Fate::Follow { from, to, folder } => {
                    let patch = Patch {
                        name: Some(moved_name(header.name(), from, to, folder)),
                        ..Patch::default()
                    };
                    zip.record(&mut *input, header, &patch, &starts, start)?;
                }
            }
        }
        if seen != by_offset.len() {
            return Err(corrupt(text, "a central directory that changed"));
        }
        if !new.is_empty() {
            self.append_new(&mut zip, new, text)?;
        }
        self.check_records(&zip, text)?;
        let declared = zip.declared();
        let mut written = zip.finish(data.comment(), extension.as_deref())?;
        if let Some((tail, len)) = tail {
            zip.tail(&mut *input, tail, len)?;
            written += len;
        }
        let grew = new.iter().any(|member| member.file.is_some());
        self.check_ratio(grew, declared, written, text)?;
        Ok(dropped)
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
            let folder = item.kind == MemberKind::Dir;
            patch.name = Some(moved_name(header.name(), node.path(), path, folder));
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

    fn tar(&self, out: &mut dyn Write, planned: &Planned<'_>, text: &str) -> Result<(), VfsError> {
        let Planned {
            fates, new, staged, ..
        } = planned;
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
            let change = match *fate {
                Fate::Keep | Fate::Drop => {
                    let keep = matches!(fate, Fate::Keep);
                    let len = end - first_block;
                    Copy {
                        buffer: &mut buffer,
                        fail: &fail,
                        text,
                    }
                    .run(&mut guard, &mut builder, len, keep)?;
                    pos = end;
                    continue;
                }
                Fate::Rewrite(to, item) => {
                    let node = self.kept_node(item, text)?;
                    self.change_of(node, to, item, None)
                }
                Fate::Materialize(at) => {
                    let Some(file) = staged.get(at) else {
                        return Err(corrupt(text, "inconsistent overlay"));
                    };
                    let node = self.kept_node(file.item, text)?;
                    self.change_of(node, file.path, file.item, Some(&file.file))
                }
                Fate::Follow { from, to, folder } => Change {
                    from,
                    to,
                    folder,
                    time: None,
                    mode: None,
                    body: None,
                },
            };
            let mut raw = Vec::new();
            let mut sink = |bytes: &[u8]| {
                raw.extend_from_slice(bytes);
                Ok(())
            };
            codec::pump(&mut guard, data - first_block, &mut sink).map_err(fail)?;
            let still = StillTrue {
                time: change.time.is_none(),
                name: change.to == change.from,
                link: change.body.is_none(),
            };
            let mut old = old_headers(&raw, still, text)?;
            builder.get_mut().write_all(&old.blocks).map_err(fail)?;
            if let Some(time) = change.time {
                old.header.set_mtime(tar_seconds(time));
            }
            if let Some(mode) = change.mode {
                old.header.set_mode(mode);
            }
            let header = &mut old.header;
            let rest = end - data;
            let left = match change.body {
                Some(file) => {
                    header.set_entry_type(EntryType::Regular);
                    header.as_old_mut().linkname.fill(0);
                    header.set_size(file.len());
                    let body = Exact {
                        inner: file.reader()?,
                        left: file.len(),
                    };
                    append_as(&mut builder, header, &change, body).map_err(fail)?;
                    rest
                }
                None => {
                    let body = Exact {
                        inner: &mut guard,
                        left: old.data_len,
                    };
                    append_as(&mut builder, header, &change, body).map_err(fail)?;
                    rest.saturating_sub(old.data_len)
                }
            };
            Copy {
                buffer: &mut buffer,
                fail: &fail,
                text,
            }
            .run(&mut guard, &mut builder, left, false)?;
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
            let grew = new.iter().any(|member| member.file.is_some());
            self.check_ratio(grew, decoded, written.get(), text)?;
        }
        Ok(())
    }

    /// What a kept member's new state changes in its record.
    fn change_of<'a>(
        &self,
        node: &'a Node,
        to: &'a str,
        item: &'a Item,
        body: Option<&'a Arc<CacheFile>>,
    ) -> Change<'a> {
        Change {
            from: node.path(),
            to,
            folder: item.kind == MemberKind::Dir,
            time: (item.mtime != node.mtime).then_some(item.mtime),
            mode: item.attrs.unix().map(|meta| meta.mode),
            body,
        }
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

/// Writes one rewritten member: under its old name, whose blocks went ahead of it and whose fields stand in `header`,
/// or under a new one.
fn append_as<W: Write>(
    builder: &mut tar::Builder<W>,
    header: &mut Header,
    change: &Change<'_>,
    body: impl Read,
) -> io::Result<()> {
    if change.to == change.from {
        header.set_cksum();
        return builder.append(header, body);
    }
    clear_names(header);
    let name = if change.folder {
        format!("{}/", change.to)
    } else {
        change.to.to_owned()
    };
    builder.append_data(header, name, body)
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
