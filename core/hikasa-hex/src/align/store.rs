//! [`Segment`], [`SegmentKind`] and the compact [`SegmentStore`] (§1.12.4, D-327).

use serde::{Deserialize, Serialize};

/// What a segment holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    /// Byte `left + i` is paired with byte `right + i` (same or different, decided by the bytes).
    Paired,
    /// Bytes of the left file only (a gap on the right).
    LeftOnly,
    /// Bytes of the right file only (a gap on the left).
    RightOnly,
}

impl SegmentKind {
    /// Stable snake_case name.
    pub const fn as_str(self) -> &'static str {
        match self {
            SegmentKind::Paired => "paired",
            SegmentKind::LeftOnly => "left_only",
            SegmentKind::RightOnly => "right_only",
        }
    }

    const fn bits(self) -> u64 {
        match self {
            SegmentKind::Paired => 0,
            SegmentKind::LeftOnly => 1,
            SegmentKind::RightOnly => 2,
        }
    }

    const fn from_bits(bits: u64) -> SegmentKind {
        match bits {
            1 => SegmentKind::LeftOnly,
            2 => SegmentKind::RightOnly,
            _ => SegmentKind::Paired,
        }
    }
}

/// One aligned segment: `len` columns starting at left offset `left` and right offset `right` (offsets of its
/// first byte; a one-sided segment's other offset is where the gap sits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Segment {
    /// Kind.
    pub kind: SegmentKind,
    /// Left offset of its first byte.
    pub left: u64,
    /// Right offset of its first byte.
    pub right: u64,
    /// Length in bytes (= aligned columns).
    pub len: u64,
}

impl Segment {
    /// Left bytes it holds.
    pub const fn left_len(&self) -> u64 {
        match self.kind {
            SegmentKind::Paired | SegmentKind::LeftOnly => self.len,
            SegmentKind::RightOnly => 0,
        }
    }

    /// Right bytes it holds.
    pub const fn right_len(&self) -> u64 {
        match self.kind {
            SegmentKind::Paired | SegmentKind::RightOnly => self.len,
            SegmentKind::LeftOnly => 0,
        }
    }

    /// End of its left bytes.
    pub const fn left_end(&self) -> u64 {
        self.left + self.left_len()
    }

    /// End of its right bytes.
    pub const fn right_end(&self) -> u64 {
        self.right + self.right_len()
    }
}

/// Segments between two prefix-sum checkpoints.
const CHECKPOINT: usize = 1024;
/// The kind's bits in a packed length.
const KIND_SHIFT: u32 = 62;
const LEN_MASK: u64 = (1 << KIND_SHIFT) - 1;

/// The segments of an alignment, compactly: 24 bytes a segment, the aligned column of every 1,024th segment for
/// `O(log n)` lookups. The segments cover both files exactly once, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SegmentStore {
    /// `[left, right, kind << 62 | len]`.
    packed: Vec<[u64; 3]>,
    /// Start column of segment `k · CHECKPOINT`.
    checkpoints: Vec<u64>,
    columns: u64,
    left_len: u64,
    right_len: u64,
}

impl SegmentStore {
    /// Number of segments.
    pub fn len(&self) -> usize {
        self.packed.len()
    }

    /// True when both files are empty.
    pub fn is_empty(&self) -> bool {
        self.packed.is_empty()
    }

    /// Segment `index`.
    pub fn get(&self, index: usize) -> Option<Segment> {
        self.packed
            .get(index)
            .map(|&[left, right, packed]| Segment {
                kind: SegmentKind::from_bits(packed >> KIND_SHIFT),
                left,
                right,
                len: packed & LEN_MASK,
            })
    }

    /// The segments in order.
    pub fn iter(&self) -> impl Iterator<Item = Segment> + '_ {
        (0..self.len()).filter_map(|i| self.get(i))
    }

    /// Aligned columns (the sum of the segment lengths).
    pub fn columns(&self) -> u64 {
        self.columns
    }

    /// Left file length.
    pub fn left_len(&self) -> u64 {
        self.left_len
    }

    /// Right file length.
    pub fn right_len(&self) -> u64 {
        self.right_len
    }

    /// Start column of segment `index` (`columns()` for `index == len()`).
    pub fn column_of(&self, index: usize) -> u64 {
        if index >= self.len() {
            return self.columns;
        }
        let checkpoint = index / CHECKPOINT;
        let mut col = self
            .checkpoints
            .get(checkpoint)
            .copied()
            .unwrap_or_default();
        for seg in &self.packed[checkpoint * CHECKPOINT..index] {
            col += seg[2] & LEN_MASK;
        }
        col
    }

    /// The segment holding aligned column `col` and its start column (`None` past the end).
    pub fn at_column(&self, col: u64) -> Option<(usize, u64)> {
        if col >= self.columns {
            return None;
        }
        // The last checkpoint at or before `col`.
        let checkpoint = self
            .checkpoints
            .partition_point(|&c| c <= col)
            .saturating_sub(1);
        let mut start = self
            .checkpoints
            .get(checkpoint)
            .copied()
            .unwrap_or_default();
        let first = checkpoint * CHECKPOINT;
        for (i, seg) in self.packed[..self.len()].iter().enumerate().skip(first) {
            let len = seg[2] & LEN_MASK;
            if col < start + len {
                return Some((i, start));
            }
            start += len;
        }
        None
    }

    /// The first segment whose left bytes end after `offset` (the segment holding left byte `offset`); `len()`
    /// when none. Right-only segments before it are skipped.
    pub fn first_at_left(&self, offset: u64) -> usize {
        self.packed.partition_point(|&[left, _, packed]| {
            let held = match SegmentKind::from_bits(packed >> KIND_SHIFT) {
                SegmentKind::RightOnly => 0,
                _ => packed & LEN_MASK,
            };
            left + held <= offset
        })
    }

    /// The segment holding byte `offset` of `side`'s file (`None` past its end).
    pub fn at_offset(&self, side: hikasa_core::Side, offset: u64) -> Option<usize> {
        let left_side = side != hikasa_core::Side::Right;
        let index = self.packed.partition_point(|&[left, right, packed]| {
            let kind = SegmentKind::from_bits(packed >> KIND_SHIFT);
            let len = packed & LEN_MASK;
            let (start, held) = if left_side {
                (
                    left,
                    if kind == SegmentKind::RightOnly {
                        0
                    } else {
                        len
                    },
                )
            } else {
                (
                    right,
                    if kind == SegmentKind::LeftOnly {
                        0
                    } else {
                        len
                    },
                )
            };
            start + held <= offset
        });
        (index..self.len()).find(|&i| {
            self.get(i).is_some_and(|s| {
                let (start, held) = if left_side {
                    (s.left, s.left_len())
                } else {
                    (s.right, s.right_len())
                };
                held > 0 && start <= offset && offset < start + held
            })
        })
    }

    /// The aligned column of byte `offset` of `side`'s file (`None` past its end): the inverse of
    /// [`offsets_at`](Self::offsets_at) for a column holding a byte of that side (W13a: the caret keeps its byte when
    /// an alignment lands).
    pub fn column_at_offset(&self, side: hikasa_core::Side, offset: u64) -> Option<u64> {
        let i = self.at_offset(side, offset)?;
        let seg = self.get(i)?;
        let start = if side == hikasa_core::Side::Right {
            seg.right
        } else {
            seg.left
        };
        Some(self.column_of(i) + (offset - start))
    }

    /// The left and right offsets at aligned column `col` (for `col == columns()`, the file lengths): a column
    /// without a byte of a side gives the offset where that side's gap sits.
    pub fn offsets_at(&self, col: u64) -> (u64, u64) {
        match self.at_column(col) {
            None => (self.left_len, self.right_len),
            Some((i, start)) => {
                let s = self.get(i).unwrap_or(Segment {
                    kind: SegmentKind::Paired,
                    left: 0,
                    right: 0,
                    len: 0,
                });
                let within = col - start;
                (
                    s.left + if s.left_len() > 0 { within } else { 0 },
                    s.right + if s.right_len() > 0 { within } else { 0 },
                )
            }
        }
    }

    /// Bytes held by memory (24 a segment plus the checkpoints).
    pub fn heap_bytes(&self) -> usize {
        self.packed.capacity() * 24 + self.checkpoints.capacity() * 8
    }

    /// Checks that the segments cover both files exactly once, in order, with no empty segment.
    pub fn check(&self) -> Result<(), String> {
        let (mut l, mut r, mut col) = (0u64, 0u64, 0u64);
        for (i, s) in self.iter().enumerate() {
            if s.len == 0 {
                return Err(format!("segment {i} is empty"));
            }
            if s.left != l || s.right != r {
                return Err(format!(
                    "segment {i} starts at ({}, {}), expected ({l}, {r})",
                    s.left, s.right
                ));
            }
            if self.column_of(i) != col {
                return Err(format!(
                    "segment {i}: column {} != {col}",
                    self.column_of(i)
                ));
            }
            l += s.left_len();
            r += s.right_len();
            col += s.len;
        }
        if (l, r, col) != (self.left_len, self.right_len, self.columns) {
            return Err(format!(
                "covers ({l}, {r}) in {col} columns, expected ({}, {}) in {}",
                self.left_len, self.right_len, self.columns
            ));
        }
        Ok(())
    }
}

/// Builds a [`SegmentStore`] from pushes in order; adjacent pushes of the same kind merge.
#[derive(Debug, Default)]
pub(crate) struct StoreBuilder {
    store: SegmentStore,
}

impl StoreBuilder {
    /// Appends `len` bytes of `kind` at the current offsets.
    pub(crate) fn push(&mut self, kind: SegmentKind, len: u64) {
        if len == 0 {
            return;
        }
        let s = &mut self.store;
        if let Some(last) = s.packed.last_mut()
            && SegmentKind::from_bits(last[2] >> KIND_SHIFT) == kind
        {
            last[2] += len;
        } else {
            if s.packed.len().is_multiple_of(CHECKPOINT) {
                s.checkpoints.push(s.columns);
            }
            s.packed
                .push([s.left_len, s.right_len, kind.bits() << KIND_SHIFT | len]);
        }
        s.columns += len;
        match kind {
            SegmentKind::Paired => {
                s.left_len += len;
                s.right_len += len;
            }
            SegmentKind::LeftOnly => s.left_len += len,
            SegmentKind::RightOnly => s.right_len += len,
        }
    }

    #[cfg(test)]
    /// Appends a gap laid out None-style (D-329 step 3): paired over the shorter side, the rest one-sided at
    /// the gap's end.
    pub(crate) fn push_none_style(&mut self, left: u64, right: u64) {
        self.push(SegmentKind::Paired, left.min(right));
        if left > right {
            self.push(SegmentKind::LeftOnly, left - right);
        } else {
            self.push(SegmentKind::RightOnly, right - left);
        }
    }

    pub(crate) fn finish(mut self) -> SegmentStore {
        self.store.packed.shrink_to_fit();
        self.store.checkpoints.shrink_to_fit();
        self.store
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushes_merge_and_cover() {
        let mut b = StoreBuilder::default();
        b.push(SegmentKind::Paired, 3);
        b.push(SegmentKind::Paired, 2);
        b.push(SegmentKind::RightOnly, 4);
        b.push(SegmentKind::LeftOnly, 1);
        b.push_none_style(5, 2);
        let s = b.finish();
        assert_eq!(s.check(), Ok(()));
        let segs: Vec<_> = s.iter().collect();
        assert_eq!(segs.len(), 5);
        assert_eq!(
            segs[0],
            Segment {
                kind: SegmentKind::Paired,
                left: 0,
                right: 0,
                len: 5
            }
        );
        assert_eq!(
            segs[1],
            Segment {
                kind: SegmentKind::RightOnly,
                left: 5,
                right: 5,
                len: 4
            }
        );
        assert_eq!(
            segs[2],
            Segment {
                kind: SegmentKind::LeftOnly,
                left: 5,
                right: 9,
                len: 1
            }
        );
        assert_eq!(
            segs[3],
            Segment {
                kind: SegmentKind::Paired,
                left: 6,
                right: 9,
                len: 2
            }
        );
        assert_eq!(
            segs[4],
            Segment {
                kind: SegmentKind::LeftOnly,
                left: 8,
                right: 11,
                len: 3
            }
        );
        assert_eq!((s.left_len(), s.right_len(), s.columns()), (11, 11, 15));
        assert_eq!(s.at_column(5), Some((1, 5)));
        assert_eq!(s.at_column(14), Some((4, 12)));
        assert_eq!(s.at_column(15), None);
        assert_eq!(s.offsets_at(7), (5, 7));
        assert_eq!(s.offsets_at(9), (5, 9));
        assert_eq!(s.offsets_at(15), (11, 11));
        assert_eq!(s.first_at_left(5), 2);
        assert_eq!(s.first_at_left(6), 3);
        assert_eq!(s.at_offset(hikasa_core::Side::Right, 6), Some(1));
        assert_eq!(s.at_offset(hikasa_core::Side::Left, 5), Some(2));
    }

    #[test]
    fn checkpoints_every_1024_segments() {
        let mut b = StoreBuilder::default();
        for i in 0..5000u64 {
            let kind = if i % 2 == 0 {
                SegmentKind::Paired
            } else {
                SegmentKind::LeftOnly
            };
            b.push(kind, i % 7 + 1);
        }
        let s = b.finish();
        assert_eq!(s.check(), Ok(()));
        let mut col = 0;
        for (i, seg) in s.iter().enumerate() {
            assert_eq!(s.at_column(col), Some((i, col)));
            assert_eq!(s.at_column(col + seg.len - 1), Some((i, col)));
            col += seg.len;
        }
    }
}
