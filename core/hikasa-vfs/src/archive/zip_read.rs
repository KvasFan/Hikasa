//! Zip: the index from the crate's central directory plus an in-house read of the time fields (D-139), without
//! touching local headers, and member decoding.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use hikasa_core::{CancelToken, Cancelled, FileTime};
use jiff::tz::TimeZone;
use zip::ZipArchive;
use zip::result::ZipError;

use super::codec::{self, Guard, InputSize, Pass, corrupt, read_failure};
use super::index::{DataRef, MemberKind, NameBudget, RawMember, Tree, build_tree};
use super::name::{NameRules, check_name, escape_name, reduce_text};
use super::shared::{ReaderLease, SharedReader};
use super::time::{dos_time, ntfs_time};
use super::zip_write::read_central;
use crate::entry::{Attrs, DosAttrs, UnixMeta};
use crate::error::VfsError;
use crate::vfs::ReadSeek;

/// The AES method id: encrypted, whether or not an AES extra field is there.
const METHOD_AES: u16 = 99;
/// The longest link target read at index time.
pub(crate) const LINK_TARGET_MAX: u64 = 4096;
/// Unix file type bits.
const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;
const S_IFLNK: u32 = 0o120_000;
/// The UTF-8 flag of a central header.
const FLAG_UTF8: u16 = 1 << 11;
/// The central directory read at index time may take this much per entry on top of `paths_max`: a fixed header and
/// the extra fields of real archives.
const DIRECTORY_ENTRY_MAX: u64 = 128;
/// The central directory header signature.
const CENTRAL_SIGNATURE: [u8; 4] = *b"PK\x01\x02";
/// Fixed part of a central directory header.
const CENTRAL_HEADER: usize = 46;

/// What one central directory header says about time, and its flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CentralFields {
    /// Offset of the header in the container (`central_header_start`).
    pub at: u64,
    /// General purpose flags.
    pub flags: u16,
    /// Compression method.
    pub method: u16,
    /// DOS date.
    pub dos_date: u16,
    /// DOS time.
    pub dos_time: u16,
    /// NTFS field (0x000a): modification and creation time, 100 ns.
    pub ntfs: Option<(u64, u64)>,
    /// Extended timestamp (0x5455), central form: modification time in Unix seconds.
    pub unix: Option<u32>,
}

impl CentralFields {
    /// §1.5.8: NTFS, then 0x5455, then the DOS time in `zone`.
    pub fn times(&self, zone: &TimeZone) -> (Option<FileTime>, Option<FileTime>) {
        if let Some((modified, created)) = self.ntfs
            && let Some(mtime) = ntfs_time(modified)
        {
            return (Some(mtime), ntfs_time(created));
        }
        if let Some(seconds) = self.unix {
            return (FileTime::from_unix(i64::from(seconds), 0), None);
        }
        (dos_time(self.dos_date, self.dos_time, zone), None)
    }
}

/// The little-endian `u16` at `at`.
fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(*bytes.get(at..at + 2)?.first_chunk()?))
}

/// The little-endian `u32` at `at`.
fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(*bytes.get(at..at + 4)?.first_chunk()?))
}

/// The little-endian `u64` at `at`.
fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(*bytes.get(at..at + 8)?.first_chunk()?))
}

/// Reads the time fields out of an extra-field block: bounds-checked, a malformed field is ignored.
fn parse_extra(extra: &[u8], fields: &mut CentralFields) {
    let mut rest = extra;
    while let (Some(id), Some(size)) = (u16_at(rest, 0), u16_at(rest, 2)) {
        let size = usize::from(size);
        let Some(body) = rest.get(4..4 + size) else {
            return;
        };
        match id {
            0x000a => fields.ntfs = parse_ntfs(body).or(fields.ntfs),
            0x5455 => {
                let flags = body.first().copied().unwrap_or(0);
                if flags & 1 != 0 && size >= 5 {
                    fields.unix = u32_at(body, 1).or(fields.unix);
                }
            }
            _ => {}
        }
        rest = rest.get(4 + size..).unwrap_or_default();
    }
}

/// The modification and creation time of an NTFS field: 4 reserved bytes, then tagged blocks.
fn parse_ntfs(body: &[u8]) -> Option<(u64, u64)> {
    let mut rest = body.get(4..)?;
    while let (Some(tag), Some(size)) = (u16_at(rest, 0), u16_at(rest, 2)) {
        let size = usize::from(size);
        let block = rest.get(4..4 + size)?;
        if tag == 1 && size >= 24 {
            return Some((u64_at(block, 0)?, u64_at(block, 16)?));
        }
        rest = rest.get(4 + size..)?;
    }
    None
}

/// The time fields of a central directory read from `reader`, which stands at offset `start`; at most `max` headers.
/// A header that is not one ends the walk. Pure and bounds-checked: any bytes give a result.
pub fn central_times(reader: impl Read, start: u64, max: usize) -> Vec<CentralFields> {
    let mut reader = BufReader::with_capacity(1 << 16, reader);
    let mut out = Vec::new();
    let mut at = start;
    let mut extra = Vec::new();
    while out.len() < max {
        let mut header = [0u8; CENTRAL_HEADER];
        if reader.read_exact(&mut header).is_err() || !header.starts_with(&CENTRAL_SIGNATURE) {
            break;
        }
        let field = |offset| u16_at(&header, offset).unwrap_or(0);
        let (name, extra_len, comment) = (field(28), field(30), field(32));
        let mut fields = CentralFields {
            at,
            flags: field(8),
            method: field(10),
            dos_time: field(12),
            dos_date: field(14),
            ntfs: None,
            unix: None,
        };
        let skip = |reader: &mut BufReader<_>, bytes: u16| {
            std::io::copy(
                &mut reader.by_ref().take(u64::from(bytes)),
                &mut std::io::sink(),
            )
            .is_ok_and(|n| n == u64::from(bytes))
        };
        if !skip(&mut reader, name) {
            break;
        }
        extra.clear();
        extra.resize(usize::from(extra_len), 0);
        if reader.read_exact(&mut extra).is_err() {
            break;
        }
        parse_extra(&extra, &mut fields);
        if !skip(&mut reader, comment) {
            break;
        }
        out.push(fields);
        at += CENTRAL_HEADER as u64 + u64::from(name) + u64::from(extra_len) + u64::from(comment);
    }
    out
}

/// The largest entry count any end record in the tail of the container states, before the crate reads a directory
/// that may be as large as the file (rule 13: memory stops at the limit).
fn stated_entries(reader: &mut dyn ReadSeek, len: u64) -> u64 {
    const TAIL: u64 = (1 << 16) + 22 + 20;
    let start = len.saturating_sub(TAIL);
    if reader.seek(SeekFrom::Start(start)).is_err() {
        return 0;
    }
    let mut tail = Vec::new();
    if (&mut *reader).take(TAIL).read_to_end(&mut tail).is_err() {
        return 0;
    }
    let mut most = 0u64;
    for at in (0..tail.len().saturating_sub(21)).rev() {
        if tail.get(at..at + 4) != Some(b"PK\x05\x06") {
            continue;
        }
        most = most.max(u64::from(u16_at(&tail, at + 10).unwrap_or(0)));
        let Some(locator) = at.checked_sub(20) else {
            continue;
        };
        if tail.get(locator..locator + 4) != Some(b"PK\x06\x07") {
            continue;
        }
        let record = u64_at(&tail, locator + 8).unwrap_or(u64::MAX);
        let mut fixed = [0u8; 56];
        if reader.seek(SeekFrom::Start(record)).is_ok()
            && reader.read_exact(&mut fixed).is_ok()
            && fixed.starts_with(b"PK\x06\x06")
        {
            most = most.max(u64_at(&fixed, 32).unwrap_or(0));
        }
    }
    most
}

/// `ZipError` as a `VfsError`. `reader` tells a base failure from bad content.
fn zip_error(spec: &str, reader: &SharedReader, error: ZipError) -> VfsError {
    match error {
        ZipError::Io(error) if reader.failed() => read_failure(error, None, spec),
        ZipError::Io(error) => match error.downcast::<VfsError>() {
            Ok(vfs) => vfs,
            Err(error) => corrupt(spec, error.to_string()),
        },
        ZipError::InvalidArchive(reason) => corrupt(spec, reason.into_owned()),
        ZipError::UnsupportedArchive(reason) if reason == ZipError::PASSWORD_REQUIRED => {
            VfsError::ArchiveEncrypted {
                spec: spec.to_owned(),
            }
        }
        ZipError::UnsupportedArchive(reason) => corrupt(spec, reason),
        ZipError::CompressionMethodNotSupported(_) => VfsError::Unsupported {
            spec: spec.to_owned(),
            what: "this zip compression method",
        },
        other => corrupt(spec, other.to_string()),
    }
}

/// The zip-specific part of an index.
pub(crate) struct ZipData {
    archive: ZipArchive<SharedReader>,
    reader: SharedReader,
    /// Sum of the declared sizes.
    declared_total: u64,
    container_len: u64,
    /// The data ranges read so far, which never overlap: start → (end, member).
    read_ranges: Mutex<BTreeMap<u64, (u64, usize)>>,
}

impl std::fmt::Debug for ZipData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZipData")
            .field("entries", &self.archive.len())
            .field("declared_total", &self.declared_total)
            .finish_non_exhaustive()
    }
}

/// The kind and attributes of an entry.
fn kind_and_attrs(unix_mode: Option<u32>, external: u32, is_dir: bool) -> (MemberKind, Attrs) {
    let Some(mode) = unix_mode else {
        let attrs = Attrs::Dos {
            attrs: DosAttrs::from_bits_truncate(external & 0xff),
        };
        let kind = if is_dir {
            MemberKind::Dir
        } else {
            MemberKind::File
        };
        return (kind, attrs);
    };
    let kind = match mode & S_IFMT {
        _ if is_dir => MemberKind::Dir,
        S_IFDIR => MemberKind::Dir,
        S_IFLNK => MemberKind::Symlink,
        0 | S_IFREG => MemberKind::File,
        _ => MemberKind::Other,
    };
    let attrs = Attrs::Unix {
        meta: UnixMeta {
            mode: mode & 0o7777,
            uid: 0,
            gid: 0,
        },
    };
    (kind, attrs)
}

/// Text of a clean name (§1.5.7 rule 1): valid UTF-8 as it is; with the UTF-8 flag otherwise lossy; else as the crate
/// decodes it (CP437).
fn member_text(raw: &[u8], utf8_flag: bool, crate_text: Option<String>) -> String {
    let text = match std::str::from_utf8(raw) {
        Ok(text) => text.to_owned(),
        Err(_) if utf8_flag => String::from_utf8_lossy(raw).into_owned(),
        Err(_) => crate_text.unwrap_or_else(|| String::from_utf8_lossy(raw).into_owned()),
    };
    reduce_text(&text.replace('\\', "/"))
}

/// The reader of an index pass. The crate reads a central directory as long as the entry count says, whatever size
/// the end record states, and keeps it in memory, so this reader is what bounds it (rule 13): past `left` bytes it
/// answers `ArchiveLimit`, and it looks at `cancel` on every read.
struct IndexReader {
    inner: Box<dyn ReadSeek>,
    left: Arc<AtomicU64>,
    cancel: CancelToken,
    spec: String,
    limit: u64,
}

impl Read for IndexReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(io::Error::other(Cancelled));
        }
        let n = self.inner.read(buf)?;
        // Calls come one at a time through `SharedReader`.
        let Some(left) = self.left.load(Ordering::Relaxed).checked_sub(n as u64) else {
            return Err(io::Error::from(VfsError::ArchiveLimit {
                spec: self.spec.clone(),
                limit: "paths",
                value: self.limit,
            }));
        };
        self.left.store(left, Ordering::Relaxed);
        Ok(n)
    }
}

impl Seek for IndexReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.inner.seek(to)
    }
}

/// Builds the tree of a zip.
pub(crate) fn index_zip(
    mut reader: Box<dyn ReadSeek>,
    len: u64,
    pass: &Pass<'_>,
) -> Result<(Tree, ZipData), VfsError> {
    let spec = pass.spec;
    let entries_max = pass.limits.entries_max;
    if stated_entries(&mut *reader, len) > entries_max {
        return Err(entries_limit(spec, entries_max));
    }
    let directory_max = directory_limit(&pass.limits);
    let left = Arc::new(AtomicU64::new(directory_max));
    let reader: Box<dyn ReadSeek> = Box::new(IndexReader {
        inner: reader,
        left: Arc::clone(&left),
        cancel: pass.ctl.cancel.clone(),
        spec: spec.to_owned(),
        limit: pass.limits.paths_max,
    });
    let (shared, lease) = SharedReader::new(reader, len);
    let opened = ZipArchive::new(shared.clone());
    // The crate tries other end records after a failed read, so a stop can come back as "not a zip".
    pass.ctl.check()?;
    let mut archive = opened.map_err(|e| zip_error(spec, &shared, e))?;
    left.store(u64::MAX, Ordering::Relaxed);
    if archive.len() as u64 > entries_max {
        return Err(entries_limit(spec, entries_max));
    }
    pass.ctl.check()?;
    let start = archive.central_directory_start();
    let fields = {
        let mut directory = shared.clone();
        directory.seek(SeekFrom::Start(start)).ok();
        let bound = usize::try_from(entries_max).unwrap_or(usize::MAX);
        central_times(
            directory.take(len.saturating_sub(start)),
            start,
            bound.saturating_add(1),
        )
    };
    // The crate's numbers are not the directory's order: it keeps one record per raw name, the last, at the place of
    // the first. A record's number here is its place among the kept ones in the directory, which is what rule 3 asks
    // about and what `directory_offsets` gives a rebuild; the crate's own number stays in `DataRef::Zip`.
    let order = directory_order(&archive).map_err(|e| zip_error(spec, &shared, e))?;
    let mut members = Vec::with_capacity(archive.len());
    let mut budget = NameBudget::new(pass);
    let mut declared_total = 0u64;
    for (step, &(_, index)) in order.iter().enumerate() {
        if step % 1024 == 0 {
            pass.ctl.check()?;
        }
        let entry = archive
            .by_index_data(index)
            .map_err(|e| zip_error(spec, &shared, e))?;
        let raw_name = entry.name_raw();
        let central = central_of(&fields, entry.central_header_start());
        let flags = central.map_or(0, |f| f.flags);
        // Without the flag the crate decodes CP437, which has a character for every byte.
        let lossy = flags & FLAG_UTF8 != 0 && std::str::from_utf8(raw_name).is_err();
        let path = check_name(raw_name, NameRules::Zip)
            .map(|clean| {
                member_text(
                    &clean,
                    flags & FLAG_UTF8 != 0,
                    entry.name().ok().map(|text| text.into_owned()),
                )
            })
            .map_err(|reason| (escape_name(raw_name), reason));
        let is_dir = raw_name.last().is_some_and(|&b| b == b'/' || b == b'\\');
        let (kind, attrs) = kind_and_attrs(entry.unix_mode(), entry.external_attributes(), is_dir);
        let encrypted = entry.encrypted() || central.is_some_and(|f| f.method == METHOD_AES);
        let (mtime, ctime) = central.map_or((None, None), |f| f.times(&pass.zone));
        declared_total = declared_total.saturating_add(entry.size());
        let member = RawMember {
            path,
            kind,
            lossy,
            encrypted,
            size: entry.size(),
            mtime,
            ctime,
            attrs,
            crc32: (!encrypted && kind != MemberKind::Dir).then(|| entry.crc32()),
            link_target: None,
            hard_target: None,
            data: DataRef::Zip { index },
        };
        budget.member(&member)?;
        members.push(member);
    }
    for (&(_, index), member) in order.iter().zip(members.iter_mut()) {
        if member.kind == MemberKind::Symlink && !member.encrypted && member.size <= LINK_TARGET_MAX
        {
            pass.ctl.check()?;
            member.link_target = read_link(&mut archive, index);
            budget.member_link(member)?;
        }
    }
    let duplicates = archive.has_duplicate_names();
    let mut tree = build_tree(members, budget, pass)?;
    if fields.len() > archive.len() {
        hidden_records(&archive, &shared, start, directory_max, &mut tree, spec);
    }
    if duplicates && tree.duplicates == 0 {
        tree.duplicates = 1;
    }
    drop(lease);
    let data = ZipData {
        archive,
        reader: shared,
        declared_total,
        container_len: len,
        read_ranges: Mutex::default(),
    };
    Ok((tree, data))
}

/// Finds the records the crate keeps no number for: it holds one per raw name, the last, and a central directory that
/// states more is read once more here, so an archive without duplicates pays nothing. A refused name is no shadow: a
/// rebuild keeps those whatever else has that name. A directory that cannot be walked again lists as it did, and the
/// rebuild, which walks it too, gives the error.
fn hidden_records(
    archive: &ZipArchive<SharedReader>,
    shared: &SharedReader,
    start: u64,
    bytes_max: u64,
    tree: &mut Tree,
    spec: &str,
) {
    let Ok(order) = directory_order(archive) else {
        return;
    };
    let mut numbers: HashMap<u64, u32> = HashMap::with_capacity(archive.len());
    let mut winners: HashMap<Box<[u8]>, u32> = HashMap::with_capacity(archive.len());
    for (record, &(start, at)) in order.iter().enumerate() {
        let Ok(entry) = archive.by_index_data(at) else {
            return;
        };
        let number = u32::try_from(record).unwrap_or(u32::MAX);
        numbers.insert(start, number);
        winners.insert(entry.name_raw().into(), number);
    }
    // The record that holds a raw name can be shadowed itself, by another spelling: the count goes to the path's winner.
    let above: HashMap<u32, u32> = tree.shadows.iter().map(|s| (s.record, s.winner)).collect();
    let mut directory: Box<dyn ReadSeek> = Box::new(shared.clone());
    let Ok(headers) = read_central(&mut *directory, start, bytes_max, spec) else {
        return;
    };
    for header in headers.iter().filter(|h| !numbers.contains_key(&h.at)) {
        if check_name(header.name(), NameRules::Zip).is_err() {
            continue;
        }
        if let Some(&winner) = winners.get(header.name()) {
            let winner = above.get(&winner).copied().unwrap_or(winner);
            tree.hidden.push((header.at, winner));
            *tree.shadowed.entry(winner).or_insert(0) += 1;
            tree.duplicates += 1;
        }
    }
    tree.hidden.sort_unstable();
}

/// The entries the crate kept, as `(offset of the central header, the crate's number)`, in the directory's order: the
/// place in this list is the record's number in the index.
fn directory_order(archive: &ZipArchive<SharedReader>) -> Result<Vec<(u64, usize)>, ZipError> {
    let mut order = Vec::with_capacity(archive.len());
    for at in 0..archive.len() {
        order.push((archive.by_index_data(at)?.central_header_start(), at));
    }
    order.sort_unstable();
    Ok(order)
}

/// The most bytes of a central directory an index reads (rule 13).
pub(crate) fn directory_limit(limits: &super::ArchiveLimits) -> u64 {
    limits
        .paths_max
        .saturating_add(DIRECTORY_ENTRY_MAX.saturating_mul(limits.entries_max))
}

/// The fields of the central header at `at`.
fn central_of(fields: &[CentralFields], at: u64) -> Option<&CentralFields> {
    fields
        .binary_search_by_key(&at, |f| f.at)
        .ok()
        .and_then(|found| fields.get(found))
}

fn entries_limit(spec: &str, value: u64) -> VfsError {
    VfsError::ArchiveLimit {
        spec: spec.to_owned(),
        limit: "entries",
        value,
    }
}

/// The content of a link entry; `None` when it cannot be read.
fn read_link(archive: &mut ZipArchive<SharedReader>, index: usize) -> Option<Box<str>> {
    let mut file = archive.by_index(index).ok()?;
    let mut target = Vec::new();
    (&mut file)
        .take(LINK_TARGET_MAX + 1)
        .read_to_end(&mut target)
        .ok()?;
    (target.len() as u64 <= LINK_TARGET_MAX).then(|| {
        String::from_utf8_lossy(&target)
            .into_owned()
            .into_boxed_str()
    })
}

impl ZipData {
    /// Bytes in front of the first record (a self-extractor): such a zip is not rewritten (§1.5.10).
    pub(crate) fn prefix(&self) -> u64 {
        self.archive.offset()
    }

    /// Where the central directory starts.
    pub(crate) fn directory_start(&self) -> u64 {
        self.archive.central_directory_start()
    }

    /// Each entry the crate kept, as `(offset of its central header, its record in the index)`.
    pub(crate) fn directory_offsets(&self) -> impl Iterator<Item = (u64, usize)> {
        let order = directory_order(&self.archive).unwrap_or_default();
        order
            .into_iter()
            .enumerate()
            .map(|(record, (start, _))| (start, record))
    }

    /// Bytes of the file.
    pub(crate) fn container_len(&self) -> u64 {
        self.container_len
    }

    /// The archive's comment, as bytes.
    pub(crate) fn comment(&self) -> &[u8] {
        self.archive.comment()
    }

    /// Holds the container open for one call.
    pub(crate) fn lease<E>(
        &self,
        open: impl FnOnce() -> Result<Box<dyn ReadSeek>, E>,
    ) -> Result<ReaderLease, E> {
        self.reader.clear_failed();
        self.reader.lease(open)
    }

    /// The ratio of rule 11, over the whole archive, before decoding.
    pub(crate) fn check_ratio(&self, pass: &Pass<'_>) -> Result<(), VfsError> {
        let limits = pass.limits;
        let max = u64::from(limits.ratio_max);
        if max > 0
            && self.declared_total > limits.ratio_floor
            && self.declared_total / self.container_len.max(1) > max
        {
            return Err(VfsError::ArchiveLimit {
                spec: pass.spec.to_owned(),
                limit: "ratio",
                value: max,
            });
        }
        Ok(())
    }

    /// Notes that the member `index` reads `start..end`; true when that overlaps the data of another member. A real
    /// local header is needed to know the range, so this runs at the first read, not at the index.
    fn overlaps(&self, index: usize, start: u64, end: u64) -> bool {
        if start == end {
            return false;
        }
        let mut ranges = self
            .read_ranges
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((&before, &(before_end, member))) = ranges.range(..end).next_back() {
            if member == index && before == start {
                return false;
            }
            if before_end > start {
                return true;
            }
        }
        ranges.insert(start, (end, index));
        false
    }

    /// Decodes the member `index`, of `declared` bytes and `compressed` bytes of data, to `sink`, through the
    /// guards. A member whose data overlaps another's counts in `total` (rule 12): a sound archive has none, however
    /// often it is read. Reads at most `declared + 1` bytes (rule 10), and a CRC failure of the crate is
    /// `CRC mismatch`. Needs a lease.
    pub(crate) fn decode(
        &self,
        index: usize,
        declared: u64,
        pass: &Pass<'_>,
        total: &std::sync::atomic::AtomicU64,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), VfsError>,
    ) -> Result<(), VfsError> {
        let spec = pass.spec;
        let mut archive = self.archive.clone();
        let compressed = archive
            .by_index_data(index)
            .map_err(|e| zip_error(spec, &self.reader, e))?
            .compressed_size();
        let file = archive
            .by_index(index)
            .map_err(|e| zip_error(spec, &self.reader, e))?;
        let start = file.data_start().unwrap_or_else(|| file.header_start());
        let overlapping = self.overlaps(index, start, start.saturating_add(compressed));
        let mut guard = Guard::new(
            file,
            pass.limits,
            pass.ctl,
            spec,
            InputSize::Fixed(compressed),
        );
        if overlapping {
            guard = guard.with_total(total);
        }
        let copied = codec::pump(&mut guard, declared.saturating_add(1), sink)
            .map_err(|error| self.read_error(spec, error))?;
        if copied != declared {
            return Err(corrupt(spec, "size mismatch"));
        }
        Ok(())
    }
}

impl ZipData {
    /// A read error of a member: the crate reports CRC and size failures and unknown methods through `io::Error`.
    fn read_error(&self, spec: &str, error: std::io::Error) -> VfsError {
        if self.reader.failed() {
            return read_failure(error, None, spec);
        }
        let error = match error.downcast::<ZipError>() {
            Ok(zip) => return zip_error(spec, &self.reader, zip),
            Err(error) => error,
        };
        match error.to_string().as_str() {
            "Invalid checksum" => corrupt(spec, "CRC mismatch"),
            "File is larger than its declared uncompressed size" => corrupt(spec, "size mismatch"),
            _ => read_failure(error, None, spec),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A central header with the given extra block.
    fn header(extra: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; CENTRAL_HEADER];
        out[..4].copy_from_slice(&CENTRAL_SIGNATURE);
        out[12..14].copy_from_slice(&0x6000u16.to_le_bytes());
        out[14..16].copy_from_slice(&0x5cefu16.to_le_bytes());
        out[28..30].copy_from_slice(&1u16.to_le_bytes());
        out[30..32].copy_from_slice(&(extra.len() as u16).to_le_bytes());
        out.push(b'n');
        out.extend_from_slice(extra);
        out
    }

    #[test]
    fn fields_are_found_and_malformed_ones_ignored() {
        let mut ut = vec![0x55, 0x54, 5, 0, 1];
        ut.extend_from_slice(&1_792_415_520u32.to_le_bytes());
        let mut bytes = header(&ut);
        let second = bytes.len() as u64;
        bytes.extend(header(&[0x55, 0x54, 3, 0, 1, 2, 3]));
        bytes.extend(header(&[0x55, 0x54, 9, 0, 1]));
        bytes.extend(b"PK\x05\x06");
        let got = central_times(Cursor::new(bytes), 0, 10);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].unix, Some(1_792_415_520));
        assert_eq!(got[1].unix, None, "length 3");
        assert_eq!(got[1].at, second);
        assert_eq!(got[2].unix, None, "field past the block");
    }

    #[test]
    fn ntfs_blocks() {
        let mut body = vec![0u8; 4];
        body.extend_from_slice(&[9, 0, 2, 0, 0, 0]);
        body.extend_from_slice(&[1, 0, 24, 0]);
        body.extend_from_slice(&7u64.to_le_bytes());
        body.extend_from_slice(&8u64.to_le_bytes());
        body.extend_from_slice(&9u64.to_le_bytes());
        assert_eq!(parse_ntfs(&body), Some((7, 9)));
        assert_eq!(parse_ntfs(&body[..20]), None);
        assert_eq!(parse_ntfs(&[]), None);
    }

    #[test]
    fn unix_types() {
        let kind = |mode, dir| kind_and_attrs(Some(mode), 0, dir).0;
        assert_eq!(kind(0o120_777, false), MemberKind::Symlink);
        assert_eq!(kind(0o100_644, false), MemberKind::File);
        assert_eq!(kind(0o644, false), MemberKind::File);
        assert_eq!(kind(0o020_644, false), MemberKind::Other);
        assert_eq!(kind(0o040_755, true), MemberKind::Dir);
        assert_eq!(
            kind_and_attrs(None, 0x21, false).1.dos().unwrap().bits(),
            0x21
        );
        let meta = kind_and_attrs(Some(0o100_640), 0, false).1.unix().unwrap();
        assert_eq!(meta.mode, 0o640);
    }

    #[test]
    fn text_of_names() {
        assert_eq!(member_text("a\\b".as_bytes(), false, None), "a/b");
        assert_eq!(member_text(b"\xff", true, Some("x".into())), "\u{FFFD}");
        assert_eq!(member_text(b"\xff", false, Some("x".into())), "x");
    }
}
