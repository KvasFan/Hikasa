//! The tar block walker (D-139; §1.5.5): one forward pass over a plain or decoded stream.
//!
//! The `tar` crate's `Archive` and `Entries` are not used to read, because `Entries` buffers extension headers
//! without a limit and borrows the archive; only `tar::Header` reads the fields of a block. Extension bodies stop at
//! 1 MiB, each member's bytes pass through `crc32fast` and are dropped, and the walk never seeks.

use std::io::{self, Read};

use hikasa_core::FileTime;
use tar::Header;

use super::codec::{BUFFER, Failure, read_full};
use super::time::{pax_time, tar_time};
use crate::error::VfsError;

/// A tar block.
const BLOCK: u64 = 512;
/// The longest `x`, `L` or `K` body.
const EXTENSION_MAX: u64 = 1 << 20;

/// What a tar record is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TarKind {
    /// A regular file.
    File,
    /// A folder.
    Dir,
    /// A symbolic link.
    Symlink,
    /// A hard link.
    HardLink,
    /// A device, a FIFO or an unknown type.
    Other,
    /// A GNU sparse file: listed, never read.
    Sparse,
}

/// One member, complete in the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TarItem {
    /// The raw name (header, GNU long name or PAX `path`).
    pub(crate) name: Vec<u8>,
    /// The raw link target.
    pub(crate) link: Option<Vec<u8>>,
    pub(crate) kind: TarKind,
    /// Offset of the first block, extension headers included.
    pub(crate) first_block: u64,
    /// Offset of the data.
    pub(crate) data: u64,
    /// Bytes of data in the stream.
    pub(crate) data_len: u64,
    /// The size to show: the data, or the expanded size of a sparse file.
    pub(crate) size: u64,
    /// Offset of the end of the padded data.
    pub(crate) end: u64,
    pub(crate) mode: u32,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) mtime: Option<FileTime>,
    /// CRC32 of the data (regular files only).
    pub(crate) crc32: Option<u32>,
}

/// Where and why a stream stopped being readable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Damage {
    pub(crate) offset: u64,
    pub(crate) reason: String,
}

/// How a walk ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Walked {
    /// Bytes read from the stream.
    pub(crate) stream_len: u64,
    /// `None` when the stream was sound to its end.
    pub(crate) damage: Option<Damage>,
    /// Where the blocks of members end: the first zero block, or the end of the stream.
    pub(crate) members_end: u64,
    /// Bytes other than zeros follow the first zero block. No reader sees them, and a rebuild keeps them.
    pub(crate) tail: bool,
}

/// The extension records that apply to the next member.
#[derive(Debug, Default)]
struct Pending {
    first: Option<u64>,
    long_name: Option<Vec<u8>>,
    long_link: Option<Vec<u8>>,
    pax_path: Option<Vec<u8>>,
    pax_link: Option<Vec<u8>>,
    pax_size: Option<u64>,
    pax_mtime: Option<FileTime>,
    sparse: bool,
    sparse_name: Option<Vec<u8>>,
    sparse_size: Option<u64>,
}

/// Calls `f` for each well-formed record of a PAX body (`<length> <key>=<value>\n`). `tar::PaxExtensions` is not used
/// because it splits on newlines, which a `path` value may hold, and a wrong `size` here would make two readers
/// disagree. A malformed record ends the body.
pub(crate) fn pax_records(mut body: &[u8], f: &mut dyn FnMut(&[u8], &[u8])) {
    while !body.is_empty() {
        let Some(space) = body.iter().position(|&b| b == b' ') else {
            return;
        };
        let length = body
            .get(..space)
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| digits.parse::<usize>().ok());
        let Some(length) = length.filter(|&l| l > space + 1 && l <= body.len()) else {
            return;
        };
        let record = body.get(space + 1..length - 1).unwrap_or_default();
        if body.get(length - 1) != Some(&b'\n') {
            return;
        }
        let Some(equals) = record.iter().position(|&b| b == b'=') else {
            return;
        };
        f(
            record.get(..equals).unwrap_or_default(),
            record.get(equals + 1..).unwrap_or_default(),
        );
        body = body.get(length..).unwrap_or_default();
    }
}

impl Pending {
    fn apply_pax(&mut self, body: &[u8]) {
        pax_records(body, &mut |key, value| {
            let text = || std::str::from_utf8(value).ok();
            let number = || text().and_then(|t| t.parse::<u64>().ok());
            match key {
                b"path" => self.pax_path = Some(value.to_vec()),
                b"linkpath" => self.pax_link = Some(value.to_vec()),
                b"size" => self.pax_size = number().or(self.pax_size),
                b"mtime" => self.pax_mtime = text().and_then(pax_time).or(self.pax_mtime),
                b"GNU.sparse.name" => self.sparse_name = Some(value.to_vec()),
                b"GNU.sparse.realsize" | b"GNU.sparse.size" => {
                    self.sparse = true;
                    self.sparse_size = number().or(self.sparse_size);
                }
                key if key.starts_with(b"GNU.sparse.") => self.sparse = true,
                _ => {}
            }
        });
    }
}

/// The checksum of a header block: the sum of its bytes with the checksum field read as spaces. Old tools summed
/// signed bytes, so both forms are accepted.
fn checksum_ok(block: &[u8; BLOCK as usize], header: &Header) -> bool {
    let Ok(stored) = header.cksum() else {
        return false;
    };
    let field = 148..156;
    let (mut unsigned, mut signed) = (0u32, 0i32);
    for (at, &byte) in block.iter().enumerate() {
        let byte = if field.contains(&at) { b' ' } else { byte };
        unsigned += u32::from(byte);
        signed += i32::from(byte.cast_signed());
    }
    stored == unsigned || i64::from(stored) == i64::from(signed)
}

/// Drops the NULs at the end of a GNU long name. One inside stays, so rule 1 refuses the name: what follows it must not
/// be lost to a reader that stops at the first NUL.
fn trim_body(mut body: Vec<u8>) -> Vec<u8> {
    let end = body
        .iter()
        .rposition(|&b| b != 0)
        .map_or(0, |last| last + 1);
    body.truncate(end);
    body
}

/// Why a walk stopped early.
enum Stop {
    /// Bad content at this offset.
    Damage(u64, String),
    /// An error that fails the whole pass.
    Fatal(VfsError),
}

/// Streaming state.
struct Walker<'a> {
    stream: &'a mut dyn Read,
    classify: &'a dyn Fn(io::Error) -> Failure,
    pos: u64,
    buf: Vec<u8>,
    members_end: u64,
    tail: bool,
}

impl Walker<'_> {
    fn stop(&self, error: io::Error) -> Stop {
        match (self.classify)(error) {
            Failure::Fatal(vfs) => Stop::Fatal(vfs),
            Failure::Damage(reason) => Stop::Damage(self.pos, reason),
        }
    }

    /// A block, or the count of bytes the stream had left.
    fn block(&mut self, block: &mut [u8; BLOCK as usize]) -> Result<usize, Stop> {
        match read_full(self.stream, block) {
            Ok(n) => {
                self.pos += n as u64;
                Ok(n)
            }
            Err(error) => Err(self.stop(error)),
        }
    }

    /// Reads `len` bytes, passing them to `hasher` when one is given. Short data is damage.
    fn data(&mut self, len: u64, mut hasher: Option<&mut crc32fast::Hasher>) -> Result<(), Stop> {
        let mut left = len;
        while left > 0 {
            let want = usize::try_from(left).unwrap_or(BUFFER).min(BUFFER);
            let chunk = self.buf.get_mut(..want).unwrap_or_default();
            let n = match self.stream.read(chunk) {
                Ok(0) => return Err(Stop::Damage(self.pos, "truncated member".into())),
                Ok(n) => n,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(self.stop(error)),
            };
            if let Some(hasher) = hasher.as_deref_mut() {
                hasher.update(chunk.get(..n).unwrap_or_default());
            }
            self.pos += n as u64;
            left -= n as u64;
        }
        Ok(())
    }

    /// The padding after `len` bytes of data.
    fn padding(&mut self, len: u64) -> Result<(), Stop> {
        let rest = len % BLOCK;
        if rest == 0 {
            Ok(())
        } else {
            self.data(BLOCK - rest, None)
        }
    }

    /// An extension body, at most [`EXTENSION_MAX`] bytes.
    fn body(&mut self, len: u64) -> Result<Vec<u8>, Stop> {
        if len > EXTENSION_MAX {
            return Err(Stop::Damage(self.pos, "extension header too long".into()));
        }
        let mut body = vec![0u8; usize::try_from(len).unwrap_or(0)];
        match read_full(self.stream, &mut body) {
            Ok(n) if n as u64 == len => self.pos += n as u64,
            Ok(n) => {
                self.pos += n as u64;
                return Err(Stop::Damage(self.pos, "truncated extension header".into()));
            }
            Err(error) => return Err(self.stop(error)),
        }
        self.padding(len)?;
        Ok(body)
    }

    /// The extension blocks of a GNU sparse member: while the last flag of a block is set another one follows.
    fn sparse_extensions(&mut self, header: &Header) -> Result<(), Stop> {
        const IS_EXTENDED: usize = 504;
        let mut more = header.as_gnu().is_some_and(tar::GnuHeader::is_extended);
        while more {
            let mut block = [0u8; BLOCK as usize];
            if self.block(&mut block)? < block.len() {
                return Err(Stop::Damage(self.pos, "truncated sparse header".into()));
            }
            more = block.get(IS_EXTENDED).is_some_and(|&b| b != 0);
        }
        Ok(())
    }

    /// Reads to the end of the stream, so a decoder's checksum is seen.
    fn drain(&mut self) -> Result<(), Stop> {
        loop {
            let chunk = self.buf.as_mut_slice();
            match self.stream.read(chunk) {
                Ok(0) => return Ok(()),
                Ok(n) => {
                    self.pos += n as u64;
                    self.tail |= chunk.get(..n).unwrap_or_default().iter().any(|&b| b != 0);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(self.stop(error)),
            }
        }
    }

    fn walk(&mut self, visit: &mut dyn FnMut(TarItem) -> Result<(), VfsError>) -> Result<(), Stop> {
        let mut pending = Pending::default();
        loop {
            let at = self.pos;
            let mut block = [0u8; BLOCK as usize];
            self.members_end = at;
            match self.block(&mut block)? {
                0 => return Ok(()),
                512 => {}
                _ => return Err(Stop::Damage(at, "short block".into())),
            }
            if block.iter().all(|&b| b == 0) {
                return self.drain();
            }
            let header = Header::from_byte_slice(&block);
            if !checksum_ok(&block, header) {
                return Err(Stop::Damage(at, "bad header checksum".into()));
            }
            pending.first.get_or_insert(at);
            let declared = header
                .entry_size()
                .map_err(|_| Stop::Damage(at, "bad size field".into()))?;
            let type_byte = block.get(156).copied().unwrap_or(0);
            match type_byte {
                b'x' | b'X' => {
                    let body = self.body(declared)?;
                    pending.apply_pax(&body);
                    continue;
                }
                b'L' => {
                    pending.long_name = Some(trim_body(self.body(declared)?));
                    continue;
                }
                b'K' => {
                    pending.long_link = Some(trim_body(self.body(declared)?));
                    continue;
                }
                b'g' | b'V' => {
                    self.data(declared, None)?;
                    self.padding(declared)?;
                    // Not part of the next member: a rebuild that drops that member keeps these blocks.
                    if pending.first == Some(at) {
                        pending.first = None;
                    }
                    continue;
                }
                _ => {}
            }
            let extensions = std::mem::take(&mut pending);
            let item = self.member(&block, type_byte, declared, extensions)?;
            let padded = item.data_len;
            visit(item).map_err(Stop::Fatal)?;
            self.padding(padded)?;
        }
    }

    /// One member: its header block has been read; reads its data (and sparse blocks).
    fn member(
        &mut self,
        block: &[u8; BLOCK as usize],
        type_byte: u8,
        declared: u64,
        pending: Pending,
    ) -> Result<TarItem, Stop> {
        let header = Header::from_byte_slice(block);
        let first_block = pending.first.unwrap_or(self.pos - BLOCK);
        // A hard link has no data of its own, whatever its size field says, unless PAX states it: reading "its data"
        // would hide the next member (as GNU tar reads it).
        let header_size = if type_byte == b'1' { 0 } else { declared };
        let data_len = pending.pax_size.unwrap_or(header_size);
        let name = pending
            .sparse_name
            .or(pending.pax_path)
            .or(pending.long_name)
            .unwrap_or_else(|| header.path_bytes().into_owned());
        let mut kind = match type_byte {
            0 | b'0' | b'7' => TarKind::File,
            b'5' | b'D' => TarKind::Dir,
            b'2' => TarKind::Symlink,
            b'1' => TarKind::HardLink,
            b'S' => TarKind::Sparse,
            _ => TarKind::Other,
        };
        if kind == TarKind::File && name.last() == Some(&b'/') {
            kind = TarKind::Dir;
        }
        if kind == TarKind::File && pending.sparse {
            kind = TarKind::Sparse;
        }
        let link = matches!(kind, TarKind::Symlink | TarKind::HardLink).then(|| {
            pending
                .pax_link
                .or(pending.long_link)
                .or_else(|| header.link_name_bytes().map(|link| link.into_owned()))
                .unwrap_or_default()
        });
        let mut size = data_len;
        if type_byte == b'S' {
            self.sparse_extensions(header)?;
            if let Some(real) = header.as_gnu().and_then(|gnu| gnu.real_size().ok()) {
                size = real;
            }
        }
        size = pending.sparse_size.unwrap_or(size);
        let data = self.pos;
        let mut hasher = (kind == TarKind::File).then(crc32fast::Hasher::new);
        self.data(data_len, hasher.as_mut())?;
        Ok(TarItem {
            name,
            link,
            kind,
            first_block,
            data,
            data_len,
            size,
            end: self.pos + (BLOCK - data_len % BLOCK) % BLOCK,
            mode: header.mode().unwrap_or(0),
            uid: narrow(header.uid()),
            gid: narrow(header.gid()),
            mtime: pending
                .pax_mtime
                .or_else(|| header.mtime().ok().and_then(tar_time)),
            crc32: hasher.map(crc32fast::Hasher::finalize),
        })
    }
}

/// An owner id as the `u32` of `UnixMeta`; `0` when it does not fit.
fn narrow(id: io::Result<u64>) -> u32 {
    id.ok().and_then(|id| u32::try_from(id).ok()).unwrap_or(0)
}

/// Walks a tar stream: `visit` gets each complete member. Bad content ends the walk with [`Walked::damage`]; an error
/// of `visit`, and any error that `classify` calls fatal, fails it.
pub(crate) fn walk(
    stream: &mut dyn Read,
    classify: &dyn Fn(io::Error) -> Failure,
    visit: &mut dyn FnMut(TarItem) -> Result<(), VfsError>,
) -> Result<Walked, VfsError> {
    let mut walker = Walker {
        stream,
        classify,
        pos: 0,
        buf: vec![0u8; BUFFER],
        members_end: 0,
        tail: false,
    };
    let damage = match walker.walk(visit) {
        Ok(()) => None,
        Err(Stop::Damage(offset, reason)) => Some(Damage { offset, reason }),
        Err(Stop::Fatal(error)) => return Err(error),
    };
    Ok(Walked {
        stream_len: walker.pos,
        damage,
        members_end: walker.members_end,
        tail: walker.tail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pax_bodies() {
        let mut seen: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        pax_records(b"12 path=a\nb\n12 size=7\n", &mut |k, v| {
            seen.push((k.to_vec(), v.to_vec()));
        });
        assert_eq!(seen.len(), 1, "the second record has a wrong length");
        assert_eq!(seen[0].1, b"a\nb");
        for bad in [&b"x"[..], b"5 ", b"99 path=x\n", b"0 =\n", b"6 ab=c\n"] {
            pax_records(bad, &mut |_, _| panic!("{bad:?}"));
        }
    }

    #[test]
    fn long_names_lose_their_nuls() {
        assert_eq!(trim_body(b"abc\0\0".to_vec()), b"abc");
        assert_eq!(trim_body(b"abc".to_vec()), b"abc");
        assert_eq!(
            trim_body(b"a\0b\0".to_vec()),
            b"a\0b",
            "rule 1 sees the NUL"
        );
        assert_eq!(trim_body(b"\0\0".to_vec()), b"");
    }
}
