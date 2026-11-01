//! Zip records as bytes (M2 W5b, D-149, §1.5.19): what a rebuild does to a zip.
//!
//! A kept member is copied record by record: its local header, its data and whatever follows them, and its central
//! header, with only the offset changed. The crate's `raw_copy_file` builds new headers from what it parsed, which loses
//! what this build does not understand (the encryption flag of a member it cannot decode, extra fields it does not
//! know, a comment in a code page), and it refuses two records of one name, which a refused member may have. A member
//! that is renamed or touched has those fields edited in the same bytes, so its data is never decoded.
//!
//! New members come from `ZipWriter`, into a zip of their own, and are appended the same way. [`ZipOut`] then writes
//! the central directory and the end records: `ZIP64` only when the figures need it.

use std::io::{self, Read, SeekFrom, Write};

use hikasa_core::{Ctl, Phase, Progress, Unit};

use super::codec::{BUFFER, STEP, corrupt};
use super::name::{NameRules, check_name};
use crate::error::{VfsError, VfsOp};
use crate::vfs::ReadSeek;

const LOCAL_SIGNATURE: [u8; 4] = *b"PK\x03\x04";
const CENTRAL_SIGNATURE: [u8; 4] = *b"PK\x01\x02";
const LOCAL_FIXED: usize = 30;
const CENTRAL_FIXED: usize = 46;
const END_FIXED: usize = 22;
const ZIP64_END_FIXED: usize = 56;
/// A field that says "see the zip64 extra field".
const FULL_32: u32 = u32::MAX;
const FULL_16: u16 = u16::MAX;
const ZIP64_EXTRA: u16 = 0x0001;
const NTFS_EXTRA: u16 = 0x000a;
const TIMESTAMP_EXTRA: u16 = 0x5455;
const UNICODE_PATH_EXTRA: u16 = 0x7075;
/// The UTF-8 flag.
const FLAG_UTF8: u16 = 1 << 11;
/// The "made by" host of a Unix mode.
const HOST_UNIX: u8 = 3;
/// The version a zip64 record needs.
const VERSION_ZIP64: u16 = 45;

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    bytes
        .get(at..at + 2)
        .and_then(|b| b.first_chunk())
        .map_or(0, |b| u16::from_le_bytes(*b))
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    bytes
        .get(at..at + 4)
        .and_then(|b| b.first_chunk())
        .map_or(0, |b| u32::from_le_bytes(*b))
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    bytes
        .get(at..at + 8)
        .and_then(|b| b.first_chunk())
        .map_or(0, |b| u64::from_le_bytes(*b))
}

fn put_u16(bytes: &mut [u8], at: usize, value: u16) {
    if let Some(field) = bytes.get_mut(at..at + 2) {
        field.copy_from_slice(&value.to_le_bytes());
    }
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    if let Some(field) = bytes.get_mut(at..at + 4) {
        field.copy_from_slice(&value.to_le_bytes());
    }
}

/// The fields of an extra block: `(id, whole field)`, in order. Bytes that are no field stay in one last pseudo field
/// of id `None`, so an edit keeps them.
fn extra_fields(block: &[u8]) -> Vec<(Option<u16>, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(head) = block.get(at..at + 4) {
        let size = usize::from(u16_at(head, 2));
        match block.get(at..at + 4 + size) {
            Some(field) => {
                out.push((Some(u16_at(head, 0)), field));
                at += 4 + size;
            }
            None => break,
        }
    }
    if let Some(rest) = block.get(at..).filter(|rest| !rest.is_empty()) {
        out.push((None, rest));
    }
    out
}

/// A member's time, as the fields that hold it (§1.5.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NewTime {
    /// The DOS date and time; `None` keeps them: an encrypted member's check byte may be its DOS time.
    pub(crate) dos: Option<(u16, u16)>,
    /// The value of a 0x5455 field; `None` writes none.
    pub(crate) unix: Option<u32>,
    /// The value an NTFS field gets, when there is one; `None` drops it.
    pub(crate) ntfs: Option<u64>,
}

/// What changes in a kept member's headers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Patch {
    /// The new name (a folder's with its `/`).
    pub(crate) name: Option<Vec<u8>>,
    pub(crate) time: Option<NewTime>,
    /// The external attributes, and the "made by" host when it changes.
    pub(crate) attrs: Option<(u32, Option<u8>)>,
}

impl Patch {
    fn is_empty(&self) -> bool {
        *self == Patch::default()
    }

    /// `extra` with this patch applied. A new name drops the fields that repeat the old one.
    fn extra(&self, extra: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(extra.len() + 9);
        let mut stamped = false;
        for (id, field) in extra_fields(extra) {
            match (id, self.time) {
                (Some(UNICODE_PATH_EXTRA), _) if self.name.is_some() => {}
                (Some(TIMESTAMP_EXTRA), Some(time)) => {
                    if let Some(seconds) = time.unix {
                        out.extend_from_slice(&timestamp_field(field, seconds));
                        stamped = true;
                    }
                }
                (Some(NTFS_EXTRA), Some(time)) => {
                    if let Some(ticks) = time.ntfs {
                        out.extend_from_slice(&ntfs_field(field, ticks));
                    }
                }
                _ => out.extend_from_slice(field),
            }
        }
        if let Some(seconds) = self.time.and_then(|time| time.unix).filter(|_| !stamped) {
            out.extend_from_slice(&TIMESTAMP_EXTRA.to_le_bytes());
            out.extend_from_slice(&5u16.to_le_bytes());
            out.push(1);
            out.extend_from_slice(&seconds.to_le_bytes());
        }
        out
    }

    /// The fixed fields shared by a local and a central header; `base` is where the flags stand in `head`, and the
    /// DOS time and date follow two bytes after the method.
    fn fixed(&self, head: &mut [u8], base: usize) {
        // The flag also says how a comment reads, so it is only ever turned on; a name that keeps raw bytes of another
        // code page stays without it.
        let text = |name: &Vec<u8>| !name.is_ascii() && std::str::from_utf8(name).is_ok();
        if self.name.as_ref().is_some_and(text) {
            put_u16(head, base, u16_at(head, base) | FLAG_UTF8);
        }
        if let Some((date, time)) = self.time.and_then(|time| time.dos) {
            put_u16(head, base + 4, time);
            put_u16(head, base + 6, date);
        }
    }

    /// A local header (`local` = fixed part, name, extra) with this patch.
    fn local(&self, local: &[u8]) -> Result<Vec<u8>, ()> {
        let name_len = usize::from(u16_at(local, 26));
        let extra_len = usize::from(u16_at(local, 28));
        let old_name = local.get(LOCAL_FIXED..LOCAL_FIXED + name_len).ok_or(())?;
        let extra = local
            .get(LOCAL_FIXED + name_len..LOCAL_FIXED + name_len + extra_len)
            .ok_or(())?;
        let name = self.name.as_deref().unwrap_or(old_name);
        let extra = self.extra(extra);
        let mut out = local.get(..LOCAL_FIXED).ok_or(())?.to_vec();
        self.fixed(&mut out, 6);
        put_u16(&mut out, 26, u16::try_from(name.len()).map_err(drop)?);
        put_u16(&mut out, 28, u16::try_from(extra.len()).map_err(drop)?);
        out.extend_from_slice(name);
        out.extend_from_slice(&extra);
        Ok(out)
    }
}

/// A 0x5455 field with a new modification time, which comes first when the flags say there is one. The access and
/// change times of a local field stay; the central form has a flag for them and no value.
fn timestamp_field(field: &[u8], seconds: u32) -> Vec<u8> {
    let mut out = field.to_vec();
    match out.get_mut(4) {
        Some(flags) if *flags & 1 != 0 => {
            if let Some(slot) = out.get_mut(5..9) {
                slot.copy_from_slice(&seconds.to_le_bytes());
            }
        }
        Some(flags) => {
            *flags |= 1;
            out.splice(5..5, seconds.to_le_bytes());
            put_u16(
                &mut out,
                2,
                u16::try_from(field.len() - 4 + 4).unwrap_or(FULL_16),
            );
        }
        None => {
            out = Vec::with_capacity(9);
            out.extend_from_slice(&TIMESTAMP_EXTRA.to_le_bytes());
            out.extend_from_slice(&5u16.to_le_bytes());
            out.push(1);
            out.extend_from_slice(&seconds.to_le_bytes());
        }
    }
    out
}

/// An NTFS field with a new modification time; the creation and access times stay.
fn ntfs_field(field: &[u8], ticks: u64) -> Vec<u8> {
    let mut out = field.to_vec();
    let mut at = 8;
    while let Some(head) = field.get(at..at + 4) {
        let size = usize::from(u16_at(head, 2));
        if u16_at(head, 0) == 1 && size >= 24 {
            if let Some(slot) = out.get_mut(at + 4..at + 12) {
                slot.copy_from_slice(&ticks.to_le_bytes());
            }
            break;
        }
        at += 4 + size;
    }
    out
}

/// One central header, as it stands in the directory.
#[derive(Debug, Clone)]
pub(crate) struct Central {
    /// Where it stood.
    pub(crate) at: u64,
    raw: Vec<u8>,
}

impl Central {
    fn name_len(&self) -> usize {
        usize::from(u16_at(&self.raw, 28))
    }

    fn extra_len(&self) -> usize {
        usize::from(u16_at(&self.raw, 30))
    }

    /// The raw name.
    pub(crate) fn name(&self) -> &[u8] {
        let end = CENTRAL_FIXED + self.name_len();
        self.raw.get(CENTRAL_FIXED..end).unwrap_or_default()
    }

    fn extra(&self) -> &[u8] {
        let start = CENTRAL_FIXED + self.name_len();
        self.raw
            .get(start..start + self.extra_len())
            .unwrap_or_default()
    }

    fn comment(&self) -> &[u8] {
        let start = CENTRAL_FIXED + self.name_len() + self.extra_len();
        self.raw.get(start..).unwrap_or_default()
    }

    /// The external attributes.
    pub(crate) fn external(&self) -> u32 {
        u32_at(&self.raw, 38)
    }

    /// What the zip64 field holds: `(uncompressed, compressed, offset)`, each only where its header field says so.
    fn zip64(&self) -> (Option<u64>, Option<u64>, Option<u64>) {
        let mut values = (None, None, None);
        for (id, field) in extra_fields(self.extra()) {
            if id != Some(ZIP64_EXTRA) {
                continue;
            }
            let body = field.get(4..).unwrap_or_default();
            let mut at = 0;
            let mut take = |wanted: bool| {
                wanted.then(|| {
                    let value = body.get(at..at + 8).map(|_| u64_at(body, at));
                    at += 8;
                    value
                })?
            };
            values.0 = take(u32_at(&self.raw, 24) == FULL_32);
            values.1 = take(u32_at(&self.raw, 20) == FULL_32);
            values.2 = take(u32_at(&self.raw, 42) == FULL_32);
        }
        values
    }

    /// Bytes of data.
    fn compressed(&self) -> u64 {
        self.zip64()
            .1
            .unwrap_or_else(|| u64::from(u32_at(&self.raw, 20)))
    }

    /// Bytes once decoded.
    fn uncompressed(&self) -> u64 {
        self.zip64()
            .0
            .unwrap_or_else(|| u64::from(u32_at(&self.raw, 24)))
    }

    /// Where its local header stands.
    pub(crate) fn local_at(&self) -> u64 {
        self.zip64()
            .2
            .unwrap_or_else(|| u64::from(u32_at(&self.raw, 42)))
    }

    /// The header with `patch` applied, at the local offset `local_at`.
    fn rewritten(&self, patch: &Patch, local_at: u64) -> Result<Vec<u8>, ()> {
        let mut head = self.raw.get(..CENTRAL_FIXED).ok_or(())?.to_vec();
        patch.fixed(&mut head, 8);
        if let Some((external, host)) = patch.attrs {
            put_u32(&mut head, 38, external);
            if let (Some(host), Some(byte)) = (host, head.get_mut(5)) {
                *byte = host;
            }
        }
        let name = patch.name.as_deref().unwrap_or(self.name());
        let mut extra = patch.extra(self.extra());
        if u32_at(&self.raw, 42) == FULL_32 {
            place_offset(
                &mut extra,
                local_at,
                u32_at(&self.raw, 24),
                u32_at(&self.raw, 20),
            );
        } else if let Some(offset) = u32::try_from(local_at).ok().filter(|&at| at != FULL_32) {
            put_u32(&mut head, 42, offset);
        } else {
            grow_for_offset(&mut head, &mut extra, local_at);
        }
        put_u16(&mut head, 28, u16::try_from(name.len()).map_err(drop)?);
        put_u16(&mut head, 30, u16::try_from(extra.len()).map_err(drop)?);
        let mut out = head;
        out.extend_from_slice(name);
        out.extend_from_slice(&extra);
        out.extend_from_slice(self.comment());
        Ok(out)
    }
}

/// Writes `local_at` over the offset a zip64 field already holds.
fn place_offset(extra: &mut [u8], local_at: u64, uncompressed: u32, compressed: u32) {
    let mut at = 0;
    while at + 4 <= extra.len() {
        let size = usize::from(u16_at(extra, at + 2));
        if u16_at(extra, at) == ZIP64_EXTRA {
            let skip =
                8 * (usize::from(uncompressed == FULL_32) + usize::from(compressed == FULL_32));
            if let Some(slot) = extra.get_mut(at + 4 + skip..at + 12 + skip) {
                slot.copy_from_slice(&local_at.to_le_bytes());
            }
            return;
        }
        at += 4 + size;
    }
}

/// Moves an offset past 4 GiB to a zip64 field: the header says so, the field gains it, and the version follows.
fn grow_for_offset(head: &mut [u8], extra: &mut Vec<u8>, local_at: u64) {
    put_u32(head, 42, FULL_32);
    let (uncompressed, compressed) = (u32_at(head, 24), u32_at(head, 20));
    let needed = u16_at(head, 6).max(VERSION_ZIP64);
    put_u16(head, 6, needed);
    let mut at = 0;
    while at + 4 <= extra.len() {
        let size = usize::from(u16_at(extra, at + 2));
        if u16_at(extra, at) == ZIP64_EXTRA {
            let skip =
                8 * (usize::from(uncompressed == FULL_32) + usize::from(compressed == FULL_32));
            extra.splice(at + 4 + skip..at + 4 + skip, local_at.to_le_bytes());
            put_u16(extra, at + 2, u16::try_from(size + 8).unwrap_or(FULL_16));
            return;
        }
        at += 4 + size;
    }
    extra.extend_from_slice(&ZIP64_EXTRA.to_le_bytes());
    extra.extend_from_slice(&8u16.to_le_bytes());
    extra.extend_from_slice(&local_at.to_le_bytes());
}

/// Every header of the directory at `start` of `reader`, duplicates included (the crate keeps one per name), up to a
/// signature that is no header. At most `bytes_max` bytes of them.
pub(crate) fn read_central(
    reader: &mut dyn ReadSeek,
    start: u64,
    bytes_max: u64,
    spec: &str,
) -> Result<Vec<Central>, VfsError> {
    let read_err = |e: io::Error| VfsError::from_io(spec, VfsOp::Read, e);
    reader.seek(SeekFrom::Start(start)).map_err(read_err)?;
    let mut buffered = io::BufReader::with_capacity(1 << 16, reader);
    let mut out = Vec::new();
    let (mut at, mut total) = (start, 0u64);
    loop {
        let mut raw = vec![0u8; CENTRAL_FIXED];
        match buffered.read_exact(&mut raw) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(read_err(error)),
        }
        if !raw.starts_with(&CENTRAL_SIGNATURE) {
            break;
        }
        let rest = usize::from(u16_at(&raw, 28))
            + usize::from(u16_at(&raw, 30))
            + usize::from(u16_at(&raw, 32));
        raw.resize(CENTRAL_FIXED + rest, 0);
        buffered
            .read_exact(raw.get_mut(CENTRAL_FIXED..).unwrap_or_default())
            .map_err(read_err)?;
        total += raw.len() as u64;
        if total > bytes_max {
            return Err(corrupt(spec, "a central directory past the limits"));
        }
        let length = raw.len() as u64;
        out.push(Central { at, raw });
        at += length;
    }
    Ok(out)
}

/// The extensible data of the zip64 end record that follows the directory at `end`, if there is one. The crate cuts it
/// short (it subtracts the whole record from a size that leaves the first 12 bytes out), so it is read here.
pub(crate) fn zip64_data(
    reader: &mut dyn ReadSeek,
    end: u64,
    bytes_max: u64,
    spec: &str,
) -> Result<Option<Vec<u8>>, VfsError> {
    let read_err = |e: io::Error| VfsError::from_io(spec, VfsOp::Read, e);
    reader.seek(SeekFrom::Start(end)).map_err(read_err)?;
    let mut head = [0u8; ZIP64_END_FIXED];
    match reader.read_exact(&mut head) {
        Ok(()) if head.starts_with(b"PK\x06\x06") => {}
        Ok(()) => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(read_err(error)),
    }
    let length = u64_at(&head, 4).saturating_sub((ZIP64_END_FIXED - 12) as u64);
    if length == 0 {
        return Ok(None);
    }
    if length > bytes_max {
        return Err(corrupt(spec, "a zip64 record past the limits"));
    }
    let mut data = vec![0u8; usize::try_from(length).unwrap_or(0)];
    reader.read_exact(&mut data).map_err(read_err)?;
    Ok(Some(data))
}

/// True when a digital signature record (`PK\x05\x05`) follows the directory at `end`. It signs what a rebuild
/// changes, so it cannot be carried over.
pub(crate) fn directory_signature(
    reader: &mut dyn ReadSeek,
    end: u64,
    spec: &str,
) -> Result<bool, VfsError> {
    let read_err = |e: io::Error| VfsError::from_io(spec, VfsOp::Read, e);
    reader.seek(SeekFrom::Start(end)).map_err(read_err)?;
    let mut head = [0u8; 4];
    match reader.read_exact(&mut head) {
        Ok(()) => Ok(head == *b"PK\x05\x05"),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(read_err(error)),
    }
}

/// Where the end records that follow the directory at `end` stop. `None` when they are not what a zip has there (a
/// signature block, say): then nothing past them is known to be a tail.
pub(crate) fn records_end(
    reader: &mut dyn ReadSeek,
    mut end: u64,
    spec: &str,
) -> Result<Option<u64>, VfsError> {
    let read_err = |e: io::Error| VfsError::from_io(spec, VfsOp::Read, e);
    let mut head = [0u8; END_FIXED];
    for _ in 0..4 {
        reader.seek(SeekFrom::Start(end)).map_err(read_err)?;
        match reader.read_exact(&mut head) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(read_err(error)),
        }
        if head.starts_with(b"PK\x05\x05") {
            end = end
                .saturating_add(6)
                .saturating_add(u64::from(u16_at(&head, 4)));
        } else if head.starts_with(b"PK\x06\x06") {
            end = end.saturating_add(12).saturating_add(u64_at(&head, 4));
        } else if head.starts_with(b"PK\x06\x07") {
            end += 20;
        } else if head.starts_with(b"PK\x05\x06") {
            return Ok(Some(end + END_FIXED as u64 + u64::from(u16_at(&head, 20))));
        } else {
            return Ok(None);
        }
    }
    Ok(None)
}

/// Where the directory `headers` ends.
pub(crate) fn directory_end(headers: &[Central], start: u64) -> u64 {
    headers
        .last()
        .map_or(start, |last| last.at + last.raw.len() as u64)
}

/// The start of each local record of `headers`, sorted: what ends the record before it.
pub(crate) fn record_starts(headers: &[Central]) -> Vec<u64> {
    let mut starts: Vec<u64> = headers.iter().map(Central::local_at).collect();
    starts.sort_unstable();
    starts.dedup();
    starts
}

/// The output of a rebuild: records as they come, then the directory and the end records.
pub(crate) struct ZipOut<'a> {
    out: &'a mut dyn Write,
    ctl: Ctl<'a>,
    spec: &'a str,
    /// Bytes written; bound by `output_max`, so overlapping records cannot multiply.
    pos: u64,
    output_max: u64,
    central: Vec<u8>,
    entries: u64,
    /// Bytes of the names of the records.
    names: u64,
    declared: u64,
    next_check: u64,
    buffer: Vec<u8>,
}

impl<'a> ZipOut<'a> {
    /// An empty archive on `out`, of at most `output_max` bytes.
    pub(crate) fn new(
        out: &'a mut dyn Write,
        ctl: Ctl<'a>,
        spec: &'a str,
        output_max: u64,
    ) -> ZipOut<'a> {
        ZipOut {
            out,
            ctl,
            spec,
            pos: 0,
            output_max,
            central: Vec::new(),
            entries: 0,
            names: 0,
            declared: 0,
            next_check: STEP,
            buffer: vec![0u8; BUFFER],
        }
    }

    /// Records so far.
    pub(crate) fn entries(&self) -> u64 {
        self.entries
    }

    /// Bytes of the names of the records so far.
    pub(crate) fn names(&self) -> u64 {
        self.names
    }

    /// The sum of the decoded sizes of the records.
    pub(crate) fn declared(&self) -> u64 {
        self.declared
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), VfsError> {
        let end = self.pos.saturating_add(bytes.len() as u64);
        if end > self.output_max {
            return Err(VfsError::ArchiveLimit {
                spec: self.spec.to_owned(),
                limit: "stream",
                value: self.output_max,
            });
        }
        self.out
            .write_all(bytes)
            .map_err(|e| VfsError::from_io(self.spec, VfsOp::Write, e))?;
        self.pos = end;
        if self.pos >= self.next_check {
            self.next_check = self.pos + STEP;
            self.ctl.check()?;
            let done = self.pos;
            let spec = self.spec;
            self.ctl.progress(Phase::Writing, || Progress {
                phase: Phase::Writing,
                done,
                total: None,
                unit: Unit::Bytes,
                current: Some(spec.to_owned()),
            });
        }
        Ok(())
    }

    fn copy(&mut self, input: &mut dyn ReadSeek, start: u64, len: u64) -> Result<(), VfsError> {
        let spec = self.spec;
        let read_err = |e: io::Error| VfsError::from_io(spec, VfsOp::Read, e);
        input.seek(SeekFrom::Start(start)).map_err(read_err)?;
        let mut buffer = std::mem::take(&mut self.buffer);
        let mut left = len;
        let mut result = Ok(());
        while left > 0 && result.is_ok() {
            let want = usize::try_from(left).unwrap_or(BUFFER).min(buffer.len());
            let chunk = buffer.get_mut(..want).unwrap_or_default();
            result = match input.read(chunk) {
                Ok(0) => Err(corrupt(spec, "a member ends past the file")),
                Ok(n) => {
                    left -= n as u64;
                    self.write(chunk.get(..n).unwrap_or_default())
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(()),
                Err(error) => Err(read_err(error)),
            };
        }
        self.buffer = buffer;
        result
    }

    /// Carries one record of `input` over: its local header, data and what follows up to the next record (a data
    /// descriptor, whatever its form), then its central header. `starts` comes from [`record_starts`] and `limit` is
    /// where the records of `input` end.
    pub(crate) fn record(
        &mut self,
        input: &mut dyn ReadSeek,
        header: &Central,
        patch: &Patch,
        starts: &[u64],
        limit: u64,
    ) -> Result<(), VfsError> {
        let spec = self.spec;
        let bad = || corrupt(spec, "a member's local header is damaged");
        let read_err = |e: io::Error| VfsError::from_io(spec, VfsOp::Read, e);
        let start = header.local_at();
        input.seek(SeekFrom::Start(start)).map_err(read_err)?;
        let mut local = vec![0u8; LOCAL_FIXED];
        input.read_exact(&mut local).map_err(|e| match e.kind() {
            io::ErrorKind::UnexpectedEof => bad(),
            _ => read_err(e),
        })?;
        if !local.starts_with(&LOCAL_SIGNATURE) {
            return Err(bad());
        }
        let rest = usize::from(u16_at(&local, 26)) + usize::from(u16_at(&local, 28));
        local.resize(LOCAL_FIXED + rest, 0);
        input
            .read_exact(local.get_mut(LOCAL_FIXED..).unwrap_or_default())
            .map_err(|e| match e.kind() {
                io::ErrorKind::UnexpectedEof => bad(),
                _ => read_err(e),
            })?;
        let data = start + local.len() as u64;
        let data_end = data.saturating_add(header.compressed());
        if data_end > limit {
            return Err(bad());
        }
        let after = starts.partition_point(|&at| at < data_end);
        let end = starts.get(after).copied().unwrap_or(limit).min(limit);
        let new_at = self.pos;
        if patch.is_empty() {
            self.write(&local)?;
        } else {
            let patched = patch.local(&local).map_err(|()| too_long(spec))?;
            self.write(&patched)?;
        }
        self.copy(input, data, end - data)?;
        let central = header
            .rewritten(patch, new_at)
            .map_err(|()| too_long(spec))?;
        self.central.extend_from_slice(&central);
        self.entries += 1;
        self.names += patch.name.as_ref().map_or(header.name().len(), Vec::len) as u64;
        self.declared = self.declared.saturating_add(header.uncompressed());
        Ok(())
    }

    /// Carries bytes over as they are: what followed the end of the old archive.
    pub(crate) fn tail(
        &mut self,
        input: &mut dyn ReadSeek,
        start: u64,
        len: u64,
    ) -> Result<(), VfsError> {
        self.copy(input, start, len)
    }

    /// The directory and the end records; `comment` is the archive's, `zip64_data` the extensible data of its zip64
    /// record.
    pub(crate) fn finish(
        &mut self,
        comment: &[u8],
        zip64_data: Option<&[u8]>,
    ) -> Result<u64, VfsError> {
        let start = self.pos;
        let central = std::mem::take(&mut self.central);
        self.write(&central)?;
        let size = central.len() as u64;
        drop(central);
        let wide = self.entries >= u64::from(FULL_16)
            || size >= u64::from(FULL_32)
            || start >= u64::from(FULL_32)
            || zip64_data.is_some();
        if wide {
            let at = self.pos;
            let data = zip64_data.unwrap_or_default();
            let mut record = Vec::with_capacity(ZIP64_END_FIXED + data.len() + 20);
            record.extend_from_slice(b"PK\x06\x06");
            record.extend_from_slice(&((ZIP64_END_FIXED - 12 + data.len()) as u64).to_le_bytes());
            record.extend_from_slice(&VERSION_ZIP64.to_le_bytes());
            record.extend_from_slice(&VERSION_ZIP64.to_le_bytes());
            record.extend_from_slice(&[0u8; 8]);
            record.extend_from_slice(&self.entries.to_le_bytes());
            record.extend_from_slice(&self.entries.to_le_bytes());
            record.extend_from_slice(&size.to_le_bytes());
            record.extend_from_slice(&start.to_le_bytes());
            record.extend_from_slice(data);
            record.extend_from_slice(b"PK\x06\x07");
            record.extend_from_slice(&[0u8; 4]);
            record.extend_from_slice(&at.to_le_bytes());
            record.extend_from_slice(&1u32.to_le_bytes());
            self.write(&record)?;
        }
        let entries = u16::try_from(self.entries).unwrap_or(FULL_16);
        let mut end = Vec::with_capacity(END_FIXED + comment.len());
        end.extend_from_slice(b"PK\x05\x06");
        end.extend_from_slice(&[0u8; 4]);
        end.extend_from_slice(&entries.to_le_bytes());
        end.extend_from_slice(&entries.to_le_bytes());
        end.extend_from_slice(&u32::try_from(size).unwrap_or(FULL_32).to_le_bytes());
        end.extend_from_slice(&u32::try_from(start).unwrap_or(FULL_32).to_le_bytes());
        let length = u16::try_from(comment.len()).map_err(|_| too_long(self.spec))?;
        end.extend_from_slice(&length.to_le_bytes());
        end.extend_from_slice(comment);
        self.write(&end)?;
        Ok(self.pos)
    }
}

/// The name and extra fields of a header no longer fit its 16-bit lengths.
fn too_long(spec: &str) -> VfsError {
    VfsError::ArchiveLimit {
        spec: spec.to_owned(),
        limit: "paths",
        value: u64::from(FULL_16),
    }
}

/// The raw name of a record that moves from the path `from` to `to`, a folder's with its `/`: the UTF-8 text of `to`.
/// A name that is not UTF-8 is another code page, whose reading here is only a reading, so what the move leaves alone
/// keeps its bytes around the new ASCII text. New text that is not ASCII cannot share a name with such bytes: then the
/// whole name is text.
pub(crate) fn moved_name(raw: &[u8], from: &str, to: &str, folder: bool) -> Vec<u8> {
    let mut name = spliced_name(raw, from, to).unwrap_or_else(|| to.as_bytes().to_vec());
    if folder {
        name.push(b'/');
    }
    name
}

fn spliced_name(raw: &[u8], from: &str, to: &str) -> Option<Vec<u8>> {
    let clean = check_name(raw, NameRules::Zip).ok()?;
    if std::str::from_utf8(&clean).is_ok() {
        return None;
    }
    let bytes: Vec<&[u8]> = clean.split(|&b| b == b'/').collect();
    let old: Vec<&str> = from.split('/').collect();
    let new: Vec<&str> = to.split('/').collect();
    if bytes.len() != old.len() {
        return None;
    }
    let shorter = old.len().min(new.len());
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let tail = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(shorter - head)
        .take_while(|(a, b)| a == b)
        .count();
    let middle = new.get(head..new.len() - tail)?;
    if !middle.iter().all(|part| part.is_ascii()) {
        return None;
    }
    let parts = bytes.get(..head)?.iter().copied();
    let parts = parts.chain(middle.iter().map(|part| part.as_bytes()));
    let parts = parts.chain(bytes.get(bytes.len() - tail..)?.iter().copied());
    Some(parts.collect::<Vec<_>>().join(&b'/'))
}

/// The attributes of a kept member, for [`Patch::attrs`]: a Unix mode makes `made by` Unix; DOS bits stay where they are.
pub(crate) fn unix_external(old: u32, type_bits: u32, mode: u32) -> (u32, Option<u8>) {
    (
        ((type_bits | (mode & 0o7777)) << 16) | (old & 0xffff),
        Some(HOST_UNIX),
    )
}

/// See [`unix_external`]: the DOS byte of `old` replaced by `bits`.
pub(crate) fn dos_external(old: u32, bits: u8) -> (u32, Option<u8>) {
    ((old & !0xff) | u32::from(bits), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A zip with a data descriptor, an unknown extra field and a comment on a member, built by hand.
    fn crafted() -> Vec<u8> {
        let mut out = Vec::new();
        let name = b"a.txt";
        let extra = [0x34, 0x12, 2, 0, 9, 9];
        let data = b"hello";
        let crc = crc32fast::hash(data);
        out.extend_from_slice(&LOCAL_SIGNATURE);
        for field in [20u16, 1 << 3, 0, 0x6000, 0x5cef] {
            out.extend_from_slice(&field.to_le_bytes());
        }
        out.extend_from_slice(&[0u8; 12]);
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&extra);
        out.extend_from_slice(data);
        out.extend_from_slice(b"PK\x07\x08");
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&5u32.to_le_bytes());
        out.extend_from_slice(&5u32.to_le_bytes());
        let start = out.len() as u32;
        out.extend_from_slice(&CENTRAL_SIGNATURE);
        for field in [0x031eu16, 20, 1 << 3, 0, 0x6000, 0x5cef] {
            out.extend_from_slice(&field.to_le_bytes());
        }
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&5u32.to_le_bytes());
        out.extend_from_slice(&5u32.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&0o100_644u32.wrapping_shl(16).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&extra);
        out.extend_from_slice(b"\xe9!");
        let size = out.len() as u32 - start;
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn central_of(bytes: &[u8]) -> (Vec<Central>, u64) {
        let start = u64::from(u32_at(bytes, bytes.len() - 6));
        let mut reader: Box<dyn ReadSeek> = Box::new(Cursor::new(bytes.to_vec()));
        (
            read_central(&mut *reader, start, 1 << 20, "t").unwrap(),
            start,
        )
    }

    fn carry(bytes: &[u8], patch: &Patch) -> Vec<u8> {
        let (headers, start) = central_of(bytes);
        let starts = record_starts(&headers);
        let mut reader: Box<dyn ReadSeek> = Box::new(Cursor::new(bytes.to_vec()));
        let mut out = Vec::new();
        let mut zip = ZipOut::new(&mut out, Ctl::detached(), "t", u64::MAX);
        zip.record(&mut *reader, &headers[0], patch, &starts, start)
            .unwrap();
        zip.finish(b"", None).unwrap();
        out
    }

    #[test]
    fn a_kept_record_is_the_same_bytes() {
        let bytes = crafted();
        assert_eq!(carry(&bytes, &Patch::default()), bytes);
    }

    /// A rename edits both headers and nothing else: the data, the descriptor, the unknown field, the comment stay.
    #[test]
    fn a_renamed_record_keeps_everything_but_its_name() {
        let bytes = crafted();
        let patch = Patch {
            name: Some(b"b.txt".to_vec()),
            ..Patch::default()
        };
        let out = carry(&bytes, &patch);
        assert_eq!(out.len(), bytes.len());
        let same: Vec<u8> = out
            .iter()
            .map(|&b| if b == b'b' { b'a' } else { b })
            .collect();
        assert_eq!(same, bytes);
        let mut archive = zip::ZipArchive::new(Cursor::new(out)).unwrap();
        let mut text = String::new();
        archive
            .by_name("b.txt")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "hello");
    }

    #[test]
    fn a_touched_record_gets_its_fields() {
        let patch = Patch {
            time: Some(NewTime {
                dos: Some((0x1111, 0x2222)),
                unix: Some(7),
                ntfs: None,
            }),
            ..Patch::default()
        };
        let out = carry(&crafted(), &patch);
        let (headers, _) = central_of(&out);
        let central = &headers[0];
        assert_eq!(u16_at(&central.raw, 14), 0x1111);
        assert_eq!(u16_at(&central.raw, 12), 0x2222);
        let ids: Vec<_> = extra_fields(central.extra()).iter().map(|f| f.0).collect();
        assert_eq!(ids, [Some(0x1234), Some(TIMESTAMP_EXTRA)]);
        assert_eq!(central.comment(), b"\xe9!");
    }

    /// Past 4 GiB an offset moves to a zip64 field, in the order the format wants.
    #[test]
    fn a_far_offset_takes_a_zip64_field() {
        let (headers, _) = central_of(&crafted());
        let far = 5u64 << 32;
        let raw = headers[0].rewritten(&Patch::default(), far).unwrap();
        let moved = Central { at: 0, raw };
        assert_eq!(moved.local_at(), far);
        assert_eq!(u16_at(&moved.raw, 6), VERSION_ZIP64);
        assert_eq!(moved.compressed(), 5, "the sizes stay in the header");
        let again = moved.rewritten(&Patch::default(), far + 1).unwrap();
        assert_eq!(Central { at: 0, raw: again }.local_at(), far + 1);
    }

    /// A name in another code page keeps the bytes a move leaves alone, around ASCII; anything else is whole text.
    #[test]
    fn a_moved_name_keeps_raw_bytes_where_it_can() {
        let raw = b"d\\\x93\xfa/\xe9.txt";
        let (from, folder) = ("d/ôú/Θ.txt", false);
        assert_eq!(
            moved_name(raw, from, "new/ôú/Θ.txt", folder),
            b"new/\x93\xfa/\xe9.txt"
        );
        assert_eq!(
            moved_name(raw, from, "d/ôú/b.txt", folder),
            b"d/\x93\xfa/b.txt"
        );
        assert_eq!(
            moved_name(raw, from, "ôú/Θ.txt", folder),
            b"\x93\xfa/\xe9.txt"
        );
        assert_eq!(
            moved_name(raw, from, "é/ôú/Θ.txt", folder),
            "é/ôú/Θ.txt".as_bytes(),
            "new text that is not ASCII takes the whole name"
        );
        assert_eq!(moved_name(b"a/b", "a/b", "c/b", true), b"c/b/");
    }

    /// Only a name that is text gets the UTF-8 flag.
    #[test]
    fn raw_bytes_get_no_utf8_flag() {
        let patch = |name: &[u8]| Patch {
            name: Some(name.to_vec()),
            ..Patch::default()
        };
        let flags = |patch: Patch| {
            let mut head = vec![0u8; 8];
            patch.fixed(&mut head, 0);
            u16_at(&head, 0)
        };
        assert_eq!(flags(patch("é".as_bytes())), FLAG_UTF8);
        assert_eq!(flags(patch(b"e")), 0);
        assert_eq!(flags(patch(b"\x93\xfa")), 0);
    }

    /// A new time keeps its place and what stands after it; a field without one gets it where the flags say.
    #[test]
    fn a_timestamp_field_changes_in_place() {
        let mut long = vec![0x55, 0x54, 13, 0, 7];
        long.extend_from_slice(&[1; 12]);
        let out = timestamp_field(&long, 2);
        assert_eq!(out.get(5..9), Some(&2u32.to_le_bytes()[..]));
        assert_eq!(out.get(9..), Some(&[1u8; 8][..]));
        let bare = [0x55, 0x54, 5, 0, 2, 9, 9, 9, 9];
        let out = timestamp_field(&bare, 2);
        assert_eq!(u16_at(&out, 2), 9);
        assert_eq!(out.get(4), Some(&3));
        assert_eq!(out.get(5..9), Some(&2u32.to_le_bytes()[..]));
        assert_eq!(out.get(9..13), Some(&[9u8; 4][..]));
        assert_eq!(timestamp_field(&[0x55, 0x54, 0, 0], 2).len(), 9);
    }

    #[test]
    fn a_new_name_drops_the_field_that_repeats_the_old_one() {
        let extra = [0x75, 0x70, 3, 0, 1, 2, 3, 0x34, 0x12, 0, 0, 0xff];
        let patch = Patch {
            name: Some(b"x".to_vec()),
            ..Patch::default()
        };
        assert_eq!(
            patch.extra(&extra),
            [0x34, 0x12, 0, 0, 0xff],
            "and the stray byte stays"
        );
    }

    #[test]
    fn end_records_widen_only_when_they_must() {
        let mut out = Vec::new();
        ZipOut::new(&mut out, Ctl::detached(), "t", u64::MAX)
            .finish(b"note", None)
            .unwrap();
        assert_eq!(out.len(), END_FIXED + 4);
        let mut out = Vec::new();
        ZipOut::new(&mut out, Ctl::detached(), "t", u64::MAX)
            .finish(b"", Some(b"data"))
            .unwrap();
        assert_eq!(out.len(), ZIP64_END_FIXED + 4 + 20 + END_FIXED);
        assert_eq!(
            zip::ZipArchive::new(Cursor::new(out.clone()))
                .unwrap()
                .len(),
            0
        );
        let mut reader: Box<dyn ReadSeek> = Box::new(Cursor::new(out));
        let data = zip64_data(&mut *reader, 0, 16, "t").unwrap();
        assert_eq!(data.as_deref(), Some(&b"data"[..]));
    }
}
