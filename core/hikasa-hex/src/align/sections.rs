//! [`Sections`], [`Section`], [`SectionKind`], [`Thumbnail`] and the scan that finds differing bytes (§1.12.4,
//! §1.12.5, D-327, D-328).
//!
//! A section is a maximal run of non-same columns (different paired bytes and one-sided bytes) where runs
//! separated by fewer than [`SECTION_MERGE_GAP`] same columns merge. The scan finds the differing bytes of paired
//! segments 32 bytes at a time with `u64` XOR words and byte-exactly at the run edges; runs are merged inside a
//! chunk as they are found and across chunks by the [`Collector`], which also places the one-sided segments.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use super::store::{SegmentKind, SegmentStore};

/// Runs separated by fewer than this many same columns belong to one section (8).
pub const SECTION_MERGE_GAP: u64 = 8;

/// Buckets of the thumbnail (4,096).
pub const THUMBNAIL_BUCKETS: usize = 4096;

/// Default of [`super::HexAlignOptions::max_sections`] (1,048,576: 24 MiB of sections).
pub const DEFAULT_MAX_SECTIONS: u64 = 1 << 20;

/// What a section holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    /// Only paired bytes that differ.
    Changed,
    /// Only left-only bytes.
    LeftOnly,
    /// Only right-only bytes.
    RightOnly,
    /// Several of the above.
    Mixed,
}

impl SectionKind {
    /// Stable snake_case name.
    pub const fn as_str(self) -> &'static str {
        match self {
            SectionKind::Changed => "changed",
            SectionKind::LeftOnly => "left_only",
            SectionKind::RightOnly => "right_only",
            SectionKind::Mixed => "mixed",
        }
    }

    fn from_bits(bits: u8) -> SectionKind {
        match bits {
            CHANGED => SectionKind::Changed,
            LEFT_ONLY => SectionKind::LeftOnly,
            RIGHT_ONLY => SectionKind::RightOnly,
            _ => SectionKind::Mixed,
        }
    }

    fn bits(self) -> u64 {
        match self {
            SectionKind::Changed => 0,
            SectionKind::LeftOnly => 1,
            SectionKind::RightOnly => 2,
            SectionKind::Mixed => 3,
        }
    }

    fn from_packed(bits: u64) -> SectionKind {
        match bits {
            0 => SectionKind::Changed,
            1 => SectionKind::LeftOnly,
            2 => SectionKind::RightOnly,
            _ => SectionKind::Mixed,
        }
    }
}

/// Kind bits of a run.
pub(crate) const CHANGED: u8 = 1;
pub(crate) const LEFT_ONLY: u8 = 2;
pub(crate) const RIGHT_ONLY: u8 = 4;

/// One section.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Section {
    /// Aligned columns from its first to its last non-same column.
    pub columns: Range<u64>,
    /// Left bytes in those columns (empty for a right-only section).
    pub left: Range<u64>,
    /// Right bytes in those columns.
    pub right: Range<u64>,
    /// Kind.
    pub kind: SectionKind,
    /// Non-same columns in it.
    pub non_same: u64,
}

const COUNT_MASK: u64 = (1 << 62) - 1;

/// The sections of an alignment, at most `max_sections` of them stored (24 bytes each); `truncated` when more
/// exist (the statistics stay exact and `total` counts them all).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sections {
    /// `[first column, end column, kind << 62 | non-same columns]`.
    stored: Vec<[u64; 3]>,
    truncated: bool,
    total: u64,
}

impl Sections {
    /// Stored sections.
    pub fn len(&self) -> usize {
        self.stored.len()
    }

    /// True when no section is stored.
    pub fn is_empty(&self) -> bool {
        self.stored.is_empty()
    }

    /// True when more sections exist than are stored.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Every section, stored or not.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Aligned columns of stored section `index`.
    pub fn columns(&self, index: usize) -> Option<Range<u64>> {
        self.stored.get(index).map(|s| s[0]..s[1])
    }

    /// Stored section `index` with its byte ranges (from `store`).
    pub fn get(&self, index: usize, store: &SegmentStore) -> Option<Section> {
        let &[start, end, packed] = self.stored.get(index)?;
        let (l0, r0) = store.offsets_at(start);
        let (l1, r1) = store.offsets_at(end);
        Some(Section {
            columns: start..end,
            left: l0..l1,
            right: r0..r1,
            kind: SectionKind::from_packed(packed >> 62),
            non_same: packed & COUNT_MASK,
        })
    }

    /// The stored sections with their byte ranges.
    pub fn iter<'s>(&'s self, store: &'s SegmentStore) -> impl Iterator<Item = Section> + 's {
        (0..self.len()).filter_map(move |i| self.get(i, store))
    }

    /// Index of the first stored section that ends after column `col` (`len()` when none).
    pub fn first_ending_after(&self, col: u64) -> usize {
        self.stored.partition_point(|s| s[1] <= col)
    }

    /// The end column of the last stored section (0 when none).
    pub fn stored_end(&self) -> u64 {
        self.stored.last().map_or(0, |s| s[1])
    }

    /// Bytes held by memory (24 a stored section).
    pub fn heap_bytes(&self) -> usize {
        self.stored.capacity() * 24
    }
}

/// A run of non-same columns (`start..end`, `non_same` of them, `kinds` bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) start: u64,
    pub(crate) end: u64,
    pub(crate) non_same: u64,
    pub(crate) kinds: u8,
}

impl Run {
    /// Merges `next` (starting at or after `self.end`) into `self` when fewer than 8 same columns separate them.
    fn absorb(&mut self, next: &Run) -> bool {
        if next.start.saturating_sub(self.end) < SECTION_MERGE_GAP {
            self.end = self.end.max(next.end);
            self.non_same += next.non_same;
            self.kinds |= next.kinds;
            true
        } else {
            false
        }
    }
}

/// The geometry of the thumbnail: column → bucket.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Geometry {
    pub(crate) columns: u64,
}

impl Geometry {
    pub(crate) fn bucket(&self, col: u64) -> usize {
        if self.columns == 0 {
            return 0;
        }
        ((u128::from(col) * THUMBNAIL_BUCKETS as u128) / u128::from(self.columns)) as usize
    }

    /// First column of bucket `b` (`columns` for `b == THUMBNAIL_BUCKETS`).
    pub(crate) fn bucket_start(&self, b: usize) -> u64 {
        let num = b as u128 * u128::from(self.columns);
        num.div_ceil(THUMBNAIL_BUCKETS as u128) as u64
    }
}

/// What a scan found in one chunk: counts, merged runs, differing bytes per thumbnail bucket.
#[derive(Debug, Default)]
pub(crate) struct ChunkScan {
    pub(crate) same: u64,
    pub(crate) different: u64,
    pub(crate) runs: Vec<Run>,
    pub(crate) buckets: Vec<(u32, u64)>,
}

impl ChunkScan {
    /// Records one differing paired byte at column `col` (columns arrive in increasing order).
    fn differ(&mut self, col: u64, geo: &Geometry, bucket_end: &mut u64) {
        self.different += 1;
        if col >= *bucket_end || self.buckets.is_empty() {
            let b = geo.bucket(col);
            *bucket_end = geo.bucket_start(b + 1);
            self.buckets.push((b as u32, 0));
        }
        if let Some(last) = self.buckets.last_mut() {
            last.1 += 1;
        }
        let run = Run {
            start: col,
            end: col + 1,
            non_same: 1,
            kinds: CHANGED,
        };
        let merged = self.runs.last_mut().is_some_and(|last| last.absorb(&run));
        if !merged {
            self.runs.push(run);
        }
    }

    /// Scans two equally long slices whose first bytes sit at column `col0`.
    pub(crate) fn scan(&mut self, a: &[u8], b: &[u8], col0: u64, geo: &Geometry) {
        let n = a.len().min(b.len());
        let (a, b) = (&a[..n], &b[..n]);
        if a == b {
            self.same += n as u64;
            return;
        }
        let before = self.different;
        let mut bucket_end = 0u64;
        let mut i = 0;
        while i < n {
            let end = (i + 32).min(n);
            if end - i == 32 && words_equal(&a[i..end], &b[i..end]) {
                i = end;
                continue;
            }
            for j in i..end {
                if a[j] != b[j] {
                    self.differ(col0 + j as u64, geo, &mut bucket_end);
                }
            }
            i = end;
        }
        self.same += n as u64 - (self.different - before);
    }
}

/// True when 32 bytes are equal, compared as four `u64` XOR words.
fn words_equal(a: &[u8], b: &[u8]) -> bool {
    let mut x = 0u64;
    for k in 0..4 {
        let wa = u64::from_le_bytes(a[k * 8..k * 8 + 8].try_into().unwrap_or([0; 8]));
        let wb = u64::from_le_bytes(b[k * 8..k * 8 + 8].try_into().unwrap_or([0; 8]));
        x |= wa ^ wb;
    }
    x == 0
}

/// One bucket of the thumbnail (HX-004): column counts, saturating.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThumbBucket {
    /// Same paired columns.
    pub same: u32,
    /// Different paired columns.
    pub different: u32,
    /// Left-only columns.
    pub left_only: u32,
    /// Right-only columns.
    pub right_only: u32,
}

/// The thumbnail: [`THUMBNAIL_BUCKETS`] buckets over the aligned columns (column `c` is in bucket
/// `c · 4096 / columns`), so W13 draws the strip at any height.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thumbnail {
    /// Aligned columns.
    pub columns: u64,
    /// The buckets.
    pub buckets: Vec<ThumbBucket>,
}

/// Merges chunk scans in column order with the one-sided segments into sections, counts and the thumbnail.
pub(crate) struct Collector<'s> {
    store: &'s SegmentStore,
    /// Next segment to consider for one-sided runs, and its start column.
    next_seg: usize,
    next_col: u64,
    max_sections: u64,
    sections: Sections,
    current: Option<Run>,
    pub(crate) same: u64,
    pub(crate) different: u64,
    diff_buckets: Vec<u64>,
}

impl<'s> Collector<'s> {
    pub(crate) fn new(store: &'s SegmentStore, max_sections: u64) -> Collector<'s> {
        Collector {
            store,
            next_seg: 0,
            next_col: 0,
            max_sections: max_sections.max(1),
            sections: Sections::default(),
            current: None,
            same: 0,
            different: 0,
            diff_buckets: vec![0; THUMBNAIL_BUCKETS],
        }
    }

    pub(crate) fn geometry(&self) -> Geometry {
        Geometry {
            columns: self.store.columns(),
        }
    }

    /// Adds a chunk's scan (chunks in column order).
    pub(crate) fn add(&mut self, scan: ChunkScan) {
        self.same += scan.same;
        self.different += scan.different;
        for (b, n) in scan.buckets {
            if let Some(slot) = self.diff_buckets.get_mut(b as usize) {
                *slot += n;
            }
        }
        for run in scan.runs {
            self.one_sided_before(run.start);
            self.run(run);
        }
    }

    /// Places the one-sided segments that start before column `col`.
    fn one_sided_before(&mut self, col: u64) {
        while self.next_col < col {
            let Some(seg) = self.store.get(self.next_seg) else {
                return;
            };
            let start = self.next_col;
            self.next_seg += 1;
            self.next_col += seg.len;
            let kinds = match seg.kind {
                SegmentKind::Paired => continue,
                SegmentKind::LeftOnly => LEFT_ONLY,
                SegmentKind::RightOnly => RIGHT_ONLY,
            };
            self.run(Run {
                start,
                end: start + seg.len,
                non_same: seg.len,
                kinds,
            });
        }
    }

    fn run(&mut self, run: Run) {
        let merged = self
            .current
            .as_mut()
            .is_some_and(|current| current.absorb(&run));
        if !merged && let Some(done) = self.current.replace(run) {
            self.store_section(done);
        }
    }

    fn store_section(&mut self, run: Run) {
        self.sections.total += 1;
        if (self.sections.stored.len() as u64) < self.max_sections {
            let kind = SectionKind::from_bits(run.kinds);
            self.sections.stored.push([
                run.start,
                run.end,
                kind.bits() << 62 | (run.non_same & COUNT_MASK),
            ]);
        } else {
            self.sections.truncated = true;
        }
    }

    /// The sections, the exact counts and the thumbnail.
    pub(crate) fn finish(mut self) -> (Sections, u64, u64, Thumbnail) {
        self.one_sided_before(u64::MAX);
        if let Some(done) = self.current.take() {
            self.store_section(done);
        }
        let geo = self.geometry();
        let mut columns = vec![[0u64; 3]; THUMBNAIL_BUCKETS];
        let mut col = 0u64;
        for seg in self.store.iter() {
            let k = match seg.kind {
                SegmentKind::Paired => 0,
                SegmentKind::LeftOnly => 1,
                SegmentKind::RightOnly => 2,
            };
            let end = col + seg.len;
            let mut at = col;
            while at < end {
                let b = geo.bucket(at);
                let stop = geo.bucket_start(b + 1).clamp(at + 1, end);
                if let Some(slot) = columns.get_mut(b) {
                    slot[k] += stop - at;
                }
                at = stop;
            }
            col = end;
        }
        let sat = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
        let buckets = columns
            .iter()
            .zip(&self.diff_buckets)
            .map(|(c, &d)| ThumbBucket {
                same: sat(c[0].saturating_sub(d)),
                different: sat(d),
                left_only: sat(c[1]),
                right_only: sat(c[2]),
            })
            .collect();
        self.sections.stored.shrink_to_fit();
        let thumbnail = Thumbnail {
            columns: self.store.columns(),
            buckets,
        };
        (self.sections, self.same, self.different, thumbnail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::align::store::StoreBuilder;

    fn sections_of(a: &[u8], b: &[u8]) -> Vec<(u64, u64, u64, SectionKind)> {
        let mut builder = StoreBuilder::default();
        builder.push_none_style(a.len() as u64, b.len() as u64);
        let store = builder.finish();
        let mut c = Collector::new(&store, 100);
        let mut scan = ChunkScan::default();
        let n = a.len().min(b.len());
        scan.scan(&a[..n], &b[..n], 0, &c.geometry());
        c.add(scan);
        let (sections, ..) = c.finish();
        (0..sections.len())
            .map(|i| {
                let s = sections.get(i, &store).unwrap();
                (s.columns.start, s.columns.end, s.non_same, s.kind)
            })
            .collect()
    }

    #[test]
    fn runs_seven_apart_merge_eight_apart_do_not() {
        let a = vec![0u8; 64];
        let mut b = a.clone();
        b[3] = 1;
        b[11] = 1; // 7 same columns between (4..11)
        b[20] = 1; // 8 same columns between (12..20)
        assert_eq!(
            sections_of(&a, &b),
            [
                (3, 12, 2, SectionKind::Changed),
                (20, 21, 1, SectionKind::Changed)
            ]
        );
        // A longer right file: the tail merges with a run less than 8 columns before it.
        let mut c = a.clone();
        c[60] = 9;
        c.extend_from_slice(&[5; 4]);
        assert_eq!(sections_of(&a, &c), [(60, 68, 5, SectionKind::Mixed)]);
    }
}
