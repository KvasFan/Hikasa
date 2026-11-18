//! Content-defined blocks of Fast and Complete (D-329 step 1): the [`GEAR`] table, [`cut_blocks`] and
//! [`block_hash`].
//!
//! Inside each 8 MiB visit chunk (the *segment*) a gear hash `h = (h << 1) + GEAR[byte]` cuts a block where
//! `h & mask == 0`, with a minimum of `avg / 4` (the bytes before it are not hashed), a maximum of `avg × 8`, the
//! mask `avg − 1` with normalized chunking (a mask of one more bit before `avg` bytes, one fewer after), and the
//! segment end always cuts. Cutting at segment ends makes the chunking parallel and deterministic: the blocks of
//! a file never depend on the thread count or on how it is read.

use std::ops::ControlFlow;

use hikasa_core::{Ctl, Phase};
use hikasa_vfs::{StreamOptions, visit_chunks};

use super::PhaseSink;
use crate::document::HexDocument;
use crate::error::HexError;

/// The seed of [`GEAR`]: `"HIKASA"`, 0, 1.
pub const GEAR_SEED: u64 = 0x48_49_4B_41_53_41_00_01;

/// SplitMix64's step.
const fn splitmix(state: u64) -> (u64, u64) {
    let state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (state, z ^ (z >> 31))
}

const fn gear() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut state = GEAR_SEED;
    let mut i = 0;
    while i < 256 {
        let (next, value) = splitmix(state);
        state = next;
        table[i] = value;
        i += 1;
    }
    table
}

/// The 256 gear constants, generated once by SplitMix64 from [`GEAR_SEED`] (pinned by a test).
pub static GEAR: [u64; 256] = gear();

/// One content-defined block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Block {
    /// Offset of its first byte.
    pub offset: u64,
    /// Length (≤ 8 MiB).
    pub len: u32,
    /// Its 64-bit hash (a hint: pairing decisions are re-checked against the bytes).
    pub hash: u64,
}

/// The length of the next block at the start of `bytes` for the average `avg` (a power of two).
fn next_cut(bytes: &[u8], avg: usize) -> usize {
    let n = bytes.len();
    let min = avg / 4;
    if n <= min {
        return n;
    }
    let mask = avg as u64 - 1;
    let (hard, easy) = ((mask << 1) | 1, mask >> 1);
    let normal = avg.min(n);
    let end = (avg * 8).min(n);
    let mut h = 0u64;
    let mut i = min;
    while i < normal {
        h = (h << 1).wrapping_add(GEAR[bytes[i] as usize]);
        if h & hard == 0 {
            return i + 1;
        }
        i += 1;
    }
    while i < end {
        h = (h << 1).wrapping_add(GEAR[bytes[i] as usize]);
        if h & easy == 0 {
            return i + 1;
        }
        i += 1;
    }
    end
}

/// The blocks of one segment (`bytes` at `offset`), hashed with `hash`.
pub fn cut_blocks(bytes: &[u8], offset: u64, avg: u64, hash: fn(&[u8]) -> u64) -> Vec<Block> {
    let avg = avg as usize;
    let mut blocks = Vec::with_capacity(bytes.len() / avg.max(1) + 1);
    let mut at = 0;
    while at < bytes.len() {
        let len = next_cut(&bytes[at..], avg);
        let block = &bytes[at..at + len];
        blocks.push(Block {
            offset: offset + at as u64,
            len: len as u32,
            hash: hash(block),
        });
        at += len;
    }
    blocks
}

/// The in-house block hash: a seeded multiply-xorshift over little-endian 8-byte words (the tail zero-padded),
/// then a final avalanche. No crate.
pub fn block_hash(bytes: &[u8]) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    const SEED: u64 = 0x48_49_4B_41_53_41_00_02;
    let mut h = SEED ^ (bytes.len() as u64).wrapping_mul(K);
    let (words, tail) = bytes.as_chunks::<8>();
    for word in words {
        h = (h ^ u64::from_le_bytes(*word)).wrapping_mul(K);
        h ^= h >> 29;
    }
    if !tail.is_empty() {
        let mut w = [0u8; 8];
        w[..tail.len()].copy_from_slice(tail);
        h = (h ^ u64::from_le_bytes(w)).wrapping_mul(K);
        h ^= h >> 29;
    }
    h ^= h >> 32;
    h = h.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^ (h >> 32)
}

/// The blocks of a whole document with the in-house hash: the visitor's chunks are the segments (the same blocks
/// mapped, streamed, in memory and with any thread count).
///
/// # Errors
///
/// A read error, [`HexError::Truncated`], cancellation.
pub fn document_blocks(
    doc: &HexDocument,
    avg: u64,
    stream: &StreamOptions,
    ctl: &Ctl<'_>,
) -> Result<Vec<Block>, HexError> {
    chunk_document(doc, avg, stream, block_hash, ctl)
}

/// The blocks of a whole document: the visitor's chunks are the segments.
pub(crate) fn chunk_document(
    doc: &HexDocument,
    avg: u64,
    stream: &StreamOptions,
    hash: fn(&[u8]) -> u64,
    ctl: &Ctl<'_>,
) -> Result<Vec<Block>, HexError> {
    let sink = PhaseSink::new(ctl, Phase::Hashing, doc.len());
    let inner = Ctl {
        cancel: ctl.cancel,
        progress: &sink,
    };
    let f = |offset: u64, bytes: &[u8]| {
        ControlFlow::<Vec<Block>, Vec<Block>>::Continue(cut_blocks(bytes, offset, avg, hash))
    };
    let visit = visit_chunks(doc.source(), 0..doc.len(), stream, &inner, &f)
        .map_err(|e| HexError::from_vfs(doc.display(), e))?;
    Ok(visit.values.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_respect_the_limits_and_cover_the_bytes() {
        let mut state = 7u64;
        let bytes: Vec<u8> = (0..200_000)
            .map(|_| {
                let (s, v) = splitmix(state);
                state = s;
                v as u8
            })
            .collect();
        let blocks = cut_blocks(&bytes, 100, 1024, block_hash);
        let mut at = 100;
        for (k, b) in blocks.iter().enumerate() {
            assert_eq!(b.offset, at);
            assert!(b.len <= 8 * 1024);
            if k + 1 < blocks.len() {
                assert!(b.len > 256, "{k}: {}", b.len);
            }
            at += u64::from(b.len);
        }
        assert_eq!(at, 100 + bytes.len() as u64);
        // Constant bytes: still covered, never longer than the maximum.
        let zeros = cut_blocks(&[0u8; 20_000], 0, 1024, block_hash);
        assert!(zeros.iter().all(|b| b.len <= 8192));
        assert_eq!(zeros.iter().map(|b| u64::from(b.len)).sum::<u64>(), 20_000);
    }

    #[test]
    fn the_hash_sees_every_byte_and_the_length() {
        assert_ne!(block_hash(b"abc"), block_hash(b"abd"));
        assert_ne!(block_hash(b"abc"), block_hash(b"abc\0"));
        assert_ne!(block_hash(b""), block_hash(b"\0"));
        assert_eq!(block_hash(b"0123456789"), block_hash(b"0123456789"));
    }
}
