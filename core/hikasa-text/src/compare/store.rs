//! The chunked row store of a comparison (D-270, contract §1.10.10): rows in `Arc` chunks of at most
//! [`CHUNK_MAX`] packed rows, each chunk's line numbers relative to its own `(left, right)` base, plus prefix sums
//! of row counts. A shift of every later line is an O(chunks) change of bases, a splice rebuilds only the chunks
//! it touches, and a clone (a new comparison) shares every other chunk. `get` stays O(1) with a division while
//! every chunk but the last is full, else a binary search over the prefix sums (gate G5).

use std::ops::Range;
use std::sync::Arc;

use super::rows::PackedRow;

/// Rows a chunk holds at most (and is built with).
pub(crate) const CHUNK_MAX: usize = 1024;
/// A rebuilt range below this many rows takes a neighbour (unless it is the only chunk).
const CHUNK_MIN: usize = 256;

/// The row store (see the module documentation).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RowStore {
    chunks: Vec<Arc<[PackedRow]>>,
    /// Per chunk: the left and right line its relative lines start from.
    bases: Vec<(u32, u32)>,
    /// Prefix sums of the chunks' row counts (`chunks + 1` entries); empty while every chunk but the last has
    /// `CHUNK_MAX` rows and the last at most that many.
    first_row: Vec<u32>,
    len: u32,
}

/// `row` with its lines made relative to `base` (absent lines stay absent).
#[inline]
fn relative(row: PackedRow, base: (u32, u32)) -> PackedRow {
    row.with_lines(
        row.left().map(|l| l - base.0),
        row.right().map(|r| r - base.1),
    )
}

/// `row` with its relative lines made absolute.
#[inline]
fn absolute(row: PackedRow, base: (u32, u32)) -> PackedRow {
    row.with_lines(
        row.left().map(|l| l + base.0),
        row.right().map(|r| r + base.1),
    )
}

/// One chunk of `rows` (absolute lines): its relative rows and its base (the first line of each side).
fn chunk_of(rows: &[PackedRow]) -> (Arc<[PackedRow]>, (u32, u32)) {
    let base = (
        rows.iter().find_map(|r| r.left()).unwrap_or(0),
        rows.iter().find_map(|r| r.right()).unwrap_or(0),
    );
    (rows.iter().map(|&r| relative(r, base)).collect(), base)
}

impl RowStore {
    /// The store of `rows` (absolute lines), in chunks of `CHUNK_MAX`.
    pub(crate) fn from_vec(rows: Vec<PackedRow>) -> RowStore {
        let mut store = RowStore {
            len: rows.len() as u32,
            ..RowStore::default()
        };
        store.chunks.reserve_exact(rows.len().div_ceil(CHUNK_MAX));
        store.bases.reserve_exact(rows.len().div_ceil(CHUNK_MAX));
        for part in rows.chunks(CHUNK_MAX) {
            let (chunk, base) = chunk_of(part);
            store.chunks.push(chunk);
            store.bases.push(base);
        }
        store
    }

    /// Rows.
    #[inline]
    pub(crate) fn len(&self) -> u32 {
        self.len
    }

    /// Chunk and offset of row `idx`.
    #[inline]
    fn locate(&self, idx: u32) -> Option<(usize, usize)> {
        if idx >= self.len {
            return None;
        }
        if self.first_row.is_empty() {
            let i = idx as usize;
            return Some((i / CHUNK_MAX, i % CHUNK_MAX));
        }
        let c = self.first_row.partition_point(|&f| f <= idx) - 1;
        Some((c, (idx - self.first_row[c]) as usize))
    }

    /// Row `idx` (absolute lines).
    #[inline]
    pub(crate) fn get(&self, idx: usize) -> Option<PackedRow> {
        let (c, i) = self.locate(u32::try_from(idx).ok()?)?;
        Some(absolute(self.chunks[c][i], self.bases[c]))
    }

    /// Rows `range` (clamped), in order.
    pub(crate) fn iter_range(&self, range: Range<u32>) -> impl Iterator<Item = PackedRow> + '_ {
        let range = range.start.min(self.len)..range.end.min(self.len);
        let first = self.locate(range.start);
        let mut c = first.map_or(self.chunks.len(), |(c, _)| c);
        let mut i = first.map_or(0, |(_, i)| i);
        let mut left = (range.end - range.start) as usize;
        std::iter::from_fn(move || {
            if left == 0 {
                return None;
            }
            while c < self.chunks.len() && i >= self.chunks[c].len() {
                c += 1;
                i = 0;
            }
            let row = absolute(*self.chunks.get(c)?.get(i)?, self.bases[c]);
            i += 1;
            left -= 1;
            Some(row)
        })
    }

    /// Every row, in order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = PackedRow> + '_ {
        self.iter_range(0..self.len)
    }

    /// Every row as a vector (checks and recounts).
    pub(crate) fn to_vec(&self) -> Vec<PackedRow> {
        self.iter().collect()
    }

    /// Replaces rows `rows` with `new` (absolute lines), and shifts the lines of every row after them by
    /// `shift` (left, right). Only the chunks holding the replaced rows (and a neighbour when they become small)
    /// are rebuilt; later chunks change their bases. O(new + touched chunks + chunks).
    pub(crate) fn splice(&mut self, rows: Range<u32>, new: &[PackedRow], shift: (i64, i64)) {
        debug_assert!(rows.start <= rows.end && rows.end <= self.len);
        let n = self.chunks.len();
        let (mut lo, mut hi) = if n == 0 {
            (0, 0)
        } else {
            let first = self.locate(rows.start).map_or(n - 1, |(c, _)| c);
            let last = if rows.end > rows.start {
                self.locate(rows.end - 1).map_or(first, |(c, _)| c)
            } else {
                first
            };
            (first, last + 1)
        };
        let count = |s: &RowStore, c: usize| s.chunks[c].len();
        let mut total: usize = (lo..hi).map(|c| count(self, c)).sum::<usize>() + new.len()
            - (rows.end - rows.start) as usize;
        while total < CHUNK_MIN && (lo > 0 || hi < n) {
            if hi < n {
                total += count(self, hi);
                hi += 1;
            } else {
                lo -= 1;
                total += count(self, lo);
            }
        }
        let start_row = self.chunk_start(lo);
        let end_row = if hi < n {
            self.chunk_start(hi)
        } else {
            self.len
        };
        let mut all: Vec<PackedRow> = Vec::with_capacity(total);
        all.extend(self.iter_range(start_row..rows.start));
        all.extend_from_slice(new);
        let move_row = |r: PackedRow| -> PackedRow {
            r.with_lines(
                r.left().map(|l| (i64::from(l) + shift.0) as u32),
                r.right().map(|x| (i64::from(x) + shift.1) as u32),
            )
        };
        all.extend(self.iter_range(rows.end..end_row).map(move_row));
        let pieces = if all.is_empty() {
            0
        } else {
            all.len().div_ceil(CHUNK_MAX)
        };
        let mut chunks = Vec::with_capacity(pieces);
        let mut bases = Vec::with_capacity(pieces);
        // Full chunks first, so a store spliced near its end stays uniform where it can.
        for part in all.chunks(CHUNK_MAX) {
            let (chunk, base) = chunk_of(part);
            chunks.push(chunk);
            bases.push(base);
        }
        for base in &mut self.bases[hi..] {
            base.0 = (i64::from(base.0) + shift.0).max(0) as u32;
            base.1 = (i64::from(base.1) + shift.1).max(0) as u32;
        }
        self.chunks.splice(lo..hi, chunks);
        self.bases.splice(lo..hi, bases);
        self.len = self.len + new.len() as u32 - (rows.end - rows.start);
        let n = self.chunks.len();
        let uniform = n == 0
            || ((0..n - 1).all(|c| self.chunks[c].len() == CHUNK_MAX)
                && self.chunks[n - 1].len() <= CHUNK_MAX);
        self.first_row.clear();
        if !uniform {
            self.first_row.reserve_exact(n + 1);
            let mut at = 0u32;
            self.first_row.push(0);
            for c in &self.chunks {
                at += c.len() as u32;
                self.first_row.push(at);
            }
        }
    }

    /// First row of chunk `c`.
    fn chunk_start(&self, c: usize) -> u32 {
        if c >= self.chunks.len() {
            self.len
        } else if self.first_row.is_empty() {
            (c * CHUNK_MAX) as u32
        } else {
            self.first_row[c]
        }
    }

    /// Checks the chunk invariants (A10.11): at most `CHUNK_MAX` rows a chunk, no empty chunk, the prefix sums,
    /// `uniform` exactly when it may be. Rows without a line on a side keep no base on that side.
    pub(crate) fn check(&self) -> Result<(), String> {
        let n = self.chunks.len();
        if self.bases.len() != n {
            return Err("bases out of step".into());
        }
        let mut total = 0usize;
        for (c, chunk) in self.chunks.iter().enumerate() {
            if chunk.is_empty() || chunk.len() > CHUNK_MAX {
                return Err(format!("chunk {c} has {} rows", chunk.len()));
            }
            total += chunk.len();
        }
        if total != self.len as usize {
            return Err(format!("{total} rows in chunks, len {}", self.len));
        }
        let uniform = n == 0
            || ((0..n - 1).all(|c| self.chunks[c].len() == CHUNK_MAX)
                && self.chunks[n - 1].len() <= CHUNK_MAX);
        if uniform != self.first_row.is_empty() {
            return Err("uniform flag".into());
        }
        if !uniform {
            let mut at = 0u32;
            for c in 0..n {
                if self.first_row[c] != at {
                    return Err(format!("first_row[{c}]"));
                }
                at += self.chunks[c].len() as u32;
            }
            if self.first_row[n] != at {
                return Err("first_row end".into());
            }
        }
        Ok(())
    }

    /// Chunk count and the identity of each chunk (sharing tests).
    pub(crate) fn layout(&self) -> Vec<(usize, usize)> {
        self.chunks
            .iter()
            .map(|c| (Arc::as_ptr(c).cast::<()>() as usize, c.len()))
            .collect()
    }
}
