//! Ignored lines (TC-024, D-282; contract §1.10.17): [`IgnoredLines`], one [`RangeSet`] of 0-based lines per side.
//!
//! ```text
//! a row is ignored  ⇔  its left line ∈ left  ∨  its right line ∈ right
//! alignment         unchanged (the marks are applied after it, on a full run and a recompare alike)
//! display, navigation, statistics, verdict, exit code, reports: an ignored row counts as same; its own class
//!                   `Ignored` (TextStats::ignored_rows, Row::ignored, ThumbCell::IGNORED)
//! edits             every range moves like the anchors (`move_through`); a range that loses every line goes
//! ```

use std::ops::Range;

use hikasa_core::cancel::Cancelled;
use hikasa_core::side::Side;
use hikasa_diff::RunKind;
use serde::{Deserialize, Serialize};

use crate::compare::TextComparison;
use crate::compare::rows::{PackedRow, ROW_CHUNK, RowIdx};
use crate::compare::sections;
use crate::compare::store::RowStore;
use crate::document::{LineIdx, TextDocument};
use crate::edit::LineDelta;

/// Sorted, disjoint, non-touching, non-empty ranges of 0-based lines (merged on construction). Serialized as a
/// list of `[start, end)` pairs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "Vec<Range<u32>>", into = "Vec<Range<u32>>")]
pub struct RangeSet(Vec<Range<u32>>);

impl From<Vec<Range<u32>>> for RangeSet {
    fn from(ranges: Vec<Range<u32>>) -> RangeSet {
        RangeSet::from_ranges(ranges)
    }
}

impl From<RangeSet> for Vec<Range<u32>> {
    fn from(set: RangeSet) -> Vec<Range<u32>> {
        set.0
    }
}

impl FromIterator<Range<u32>> for RangeSet {
    fn from_iter<I: IntoIterator<Item = Range<u32>>>(iter: I) -> RangeSet {
        RangeSet::from_ranges(iter.into_iter().collect())
    }
}

impl RangeSet {
    /// The empty set.
    pub const fn new() -> RangeSet {
        RangeSet(Vec::new())
    }

    /// The set of `ranges` in any order, overlapping or touching ones merged, empty ones dropped.
    pub fn from_ranges(mut ranges: Vec<Range<u32>>) -> RangeSet {
        ranges.retain(|r| r.start < r.end);
        ranges.sort_unstable_by_key(|r| (r.start, r.end));
        let mut out: Vec<Range<u32>> = Vec::with_capacity(ranges.len());
        for r in ranges {
            match out.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => out.push(r),
            }
        }
        RangeSet(out)
    }

    /// The ranges in order.
    pub fn ranges(&self) -> &[Range<u32>] {
        &self.0
    }

    /// True when no line is in the set.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Lines in the set.
    pub fn line_count(&self) -> u64 {
        self.0.iter().map(|r| u64::from(r.end - r.start)).sum()
    }

    /// True when `line` is in the set (binary search).
    #[inline]
    pub fn contains(&self, line: u32) -> bool {
        let k = self.0.partition_point(|r| r.end <= line);
        self.0.get(k).is_some_and(|r| r.start <= line)
    }

    /// True when every line of `lines` (non-empty) is in the set.
    pub fn contains_range(&self, lines: &Range<u32>) -> bool {
        if lines.start >= lines.end {
            return false;
        }
        let k = self.0.partition_point(|r| r.end <= lines.start);
        self.0
            .get(k)
            .is_some_and(|r| r.start <= lines.start && lines.end <= r.end)
    }

    /// The union with `lines`.
    pub fn insert(&mut self, lines: Range<u32>) {
        let mut v = std::mem::take(&mut self.0);
        v.push(lines);
        *self = RangeSet::from_ranges(v);
    }

    /// The set without `lines`.
    pub fn remove(&mut self, lines: Range<u32>) {
        if lines.start >= lines.end {
            return;
        }
        let mut out = Vec::with_capacity(self.0.len() + 1);
        for r in self.0.drain(..) {
            if r.end <= lines.start || r.start >= lines.end {
                out.push(r);
                continue;
            }
            if r.start < lines.start {
                out.push(r.start..lines.start);
            }
            if lines.end < r.end {
                out.push(lines.end..r.end);
            }
        }
        self.0 = out;
    }

    /// The *Ignored* command on `lines` (TC-024): marking again removes the marks, so when every line of
    /// `lines` is in the set they are removed, else they are added.
    pub fn toggle(&mut self, lines: Range<u32>) {
        if self.contains_range(&lines) {
            self.remove(lines);
        } else {
            self.insert(lines);
        }
    }

    /// The set without the lines at and beyond `lines` (a document of `lines` lines).
    pub fn clipped(&self, lines: u32) -> RangeSet {
        self.0
            .iter()
            .map(|r| r.start.min(lines)..r.end.min(lines))
            .collect()
    }

    /// The lines that are in exactly one of `self` and `other`, as ranges in order.
    pub fn symmetric_difference(&self, other: &RangeSet) -> Vec<Range<u32>> {
        let mut points: Vec<(u32, i32)> = Vec::with_capacity(2 * (self.0.len() + other.0.len()));
        for r in self.0.iter().chain(&other.0) {
            points.push((r.start, 1));
            points.push((r.end, -1));
        }
        points.sort_unstable();
        let mut out: Vec<Range<u32>> = Vec::new();
        let mut depth = 0i32;
        let mut k = 0;
        while k < points.len() {
            let at = points[k].0;
            let before = depth;
            while k < points.len() && points[k].0 == at {
                depth += points[k].1;
                k += 1;
            }
            // Inside the difference exactly while one set covers the line.
            match (before == 1, depth == 1) {
                (false, true) => out.push(at..at),
                (true, false) => {
                    if let Some(last) = out.last_mut() {
                        last.end = at;
                    }
                }
                _ => {}
            }
        }
        out.retain(|r| r.start < r.end);
        RangeSet::from_ranges(out).0
    }

    /// The set after `delta` (§1.10.17: the marks follow edits as the anchors do, W10b note 6): a range's start
    /// and end move as line boundaries — a boundary before the change stays, one after it shifts by
    /// `inserted − removed`, one inside it keeps its offset up to `inserted`; lines inserted exactly at a
    /// range's start or end stay outside it, and the new lines of a change that replaced a range's last lines
    /// join it. A range that loses every line disappears.
    pub fn move_through(&mut self, d: LineDelta) {
        if d.removed == 0 && d.inserted == 0 {
            return;
        }
        let end_of_change = d.at + d.removed;
        let shift = |x: u32| (i64::from(x) + i64::from(d.inserted) - i64::from(d.removed)) as u32;
        let start = |x: u32| -> u32 {
            if x < d.at {
                x
            } else if x >= end_of_change {
                shift(x)
            } else {
                d.at + (x - d.at).min(d.inserted)
            }
        };
        let end = |x: u32| -> u32 {
            if x <= d.at {
                x
            } else if x >= end_of_change {
                shift(x)
            } else {
                d.at + (x - d.at).min(d.inserted)
            }
        };
        let moved: Vec<Range<u32>> = self
            .0
            .iter()
            .map(|r| {
                let s = start(r.start);
                s..end(r.end).max(s)
            })
            .collect();
        *self = RangeSet::from_ranges(moved);
    }
}

/// Ignored lines of a comparison (TC-024, D-282; §1.10.17): 0-based line ranges per side, merged and sorted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IgnoredLines {
    /// Left lines.
    pub left: RangeSet,
    /// Right lines.
    pub right: RangeSet,
}

impl IgnoredLines {
    /// No line ignored.
    pub fn is_empty(&self) -> bool {
        self.left.is_empty() && self.right.is_empty()
    }

    /// The lines of side `k` (0 = left, 1 = right).
    pub(crate) fn side(&self, k: usize) -> &RangeSet {
        if k == 0 { &self.left } else { &self.right }
    }

    /// The lines of side `k`, mutable.
    pub(crate) fn side_mut(&mut self, k: usize) -> &mut RangeSet {
        if k == 0 {
            &mut self.left
        } else {
            &mut self.right
        }
    }

    /// True when a row with these lines is ignored.
    #[inline]
    pub fn row_ignored(&self, left: Option<u32>, right: Option<u32>) -> bool {
        left.is_some_and(|l| self.left.contains(l)) || right.is_some_and(|r| self.right.contains(r))
    }

    /// Both sides clipped to documents of `lines` lines.
    pub fn clipped(&self, left_lines: u32, right_lines: u32) -> IgnoredLines {
        IgnoredLines {
            left: self.left.clipped(left_lines),
            right: self.right.clipped(right_lines),
        }
    }
}

/// The check of one side's ignored lines (§1.10.17, the session's `ignored_check_left`/`ignored_check_right`):
/// the CRC-32 (IEEE) of the marked lines' text, each line followed by `\n`, in line order (terminators and the
/// BOM are not part of it, so a change of line endings or encoding keeps the marks). `None` when a marked line
/// is beyond the document (it cannot match a saved check); `Some` of the empty input's CRC (0) without marks.
pub fn ignored_check(doc: &TextDocument, lines: &RangeSet) -> Option<u32> {
    if lines
        .ranges()
        .last()
        .is_some_and(|r| r.end > doc.line_count())
    {
        return None;
    }
    /// The marked lines as one byte stream, a line at a time.
    struct Lines<'d> {
        cursor: crate::document::LineCursor<'d>,
        lines: Box<dyn Iterator<Item = u32> + 'd>,
        buffer: Vec<u8>,
        at: usize,
    }
    impl std::io::Read for Lines<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            while self.at == self.buffer.len() {
                let Some(line) = self.lines.next() else {
                    return Ok(0);
                };
                self.buffer.clear();
                self.buffer
                    .extend_from_slice(self.cursor.line(line).as_bytes());
                self.buffer.push(b'\n');
                self.at = 0;
            }
            let n = out.len().min(self.buffer.len() - self.at);
            out[..n].copy_from_slice(&self.buffer[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
    }
    let mut reader = Lines {
        cursor: doc.cursor(),
        lines: Box::new(lines.ranges().iter().flat_map(Clone::clone)),
        buffer: Vec::new(),
        at: 0,
    };
    hikasa_vfs::crc32_reader(
        &mut reader,
        &hikasa_vfs::StreamOptions::default(),
        &hikasa_core::progress::Ctl::detached(),
    )
    .ok()
}

/// Sets every row's ignored flag from `ignored` (cancellation checked every 4,096 rows).
pub(crate) fn mark_rows(
    rows: &mut [PackedRow],
    ignored: &IgnoredLines,
    check: &dyn Fn() -> Result<(), Cancelled>,
) -> Result<(), Cancelled> {
    if ignored.is_empty() {
        return Ok(());
    }
    for (k, row) in rows.iter_mut().enumerate() {
        if k % ROW_CHUNK == 0 {
            check()?;
        }
        row.set_ignored(ignored.row_ignored(row.left(), row.right()));
    }
    Ok(())
}

/// Changed rows above this share of all rows re-derive the whole comparison instead of windows.
const WHOLE_SHARE: u32 = 8;
/// Windows closer than this many rows are handled as one.
const JOIN_GAP: u32 = 4096;

impl TextComparison {
    /// The first left and right line at or after row `row` (the lines before it).
    fn lines_before(&self, row: u32) -> (u32, u32) {
        if row >= self.row_count() {
            return (self.left.line_count(), self.right.line_count());
        }
        let k = self
            .run_rows
            .partition_point(|&r| r <= row)
            .saturating_sub(1);
        let (run, first) = (&self.runs[k], self.run_rows[k]);
        let j = row - first;
        let paired = matches!(run.kind, RunKind::Equal | RunKind::Changed);
        (
            if paired || run.kind == RunKind::Delete {
                run.left.start + j
            } else {
                run.left.start
            },
            if paired || run.kind == RunKind::Insert {
                run.right.start + j
            } else {
                run.right.start
            },
        )
    }

    /// Replaces the ignored lines (clipped to the documents) and re-derives what depends on them — the rows'
    /// flags, the sections' importance and counts, the class runs, the statistics — over the rows whose flag
    /// changes only, widened to whole sections (all rows when many change). Alignment and states stay.
    pub(crate) fn reignore(&mut self, new: IgnoredLines) {
        let new = new.clipped(self.left.line_count(), self.right.line_count());
        if new == self.ignored {
            return;
        }
        // The rows of the lines whose mark changes.
        let mut changed: Vec<Range<u32>> = Vec::new();
        for (k, side) in [(0usize, Side::Left), (1, Side::Right)] {
            for lines in self.ignored.side(k).symmetric_difference(new.side(k)) {
                for line in lines {
                    if let Some(r) = self.row_for_line(side, LineIdx(line)) {
                        changed.push(r.0..r.0 + 1);
                    }
                }
            }
        }
        self.ignored = new;
        let changed = RangeSet::from_ranges(changed);
        let total: u32 = changed.ranges().iter().map(|r| r.end - r.start).sum();
        if total.saturating_mul(WHOLE_SHARE) >= self.row_count() {
            let mut rows = self.rows.to_vec();
            for row in &mut rows {
                row.set_ignored(self.ignored.row_ignored(row.left(), row.right()));
            }
            let d = sections::derive(
                &rows,
                self.left.line_count(),
                self.right.line_count(),
                self.lines_independent(),
            );
            self.rows = RowStore::from_vec(rows);
            self.sections = d.sections;
            self.section_counts = d.counts;
            self.class_runs = d.class_runs;
            self.stats = d.stats;
            return;
        }
        // Windows: whole sections around the changed rows, near ones joined.
        let mut windows: Vec<Range<u32>> = Vec::new();
        for r in changed.ranges() {
            let start = self
                .section_at(RowIdx(r.start))
                .map_or(r.start, |s| self.sections[s.0 as usize].rows.start);
            let end = self
                .section_at(RowIdx(r.end - 1))
                .map_or(r.end, |s| self.sections[s.0 as usize].rows.end);
            match windows.last_mut() {
                Some(w) if start <= w.end.saturating_add(JOIN_GAP) => w.end = w.end.max(end),
                _ => windows.push(start..end),
            }
        }
        for w in windows {
            let old: Vec<PackedRow> = self.rows.iter_range(w.clone()).collect();
            let mut new_rows = old.clone();
            for row in &mut new_rows {
                row.set_ignored(self.ignored.row_ignored(row.left(), row.right()));
            }
            let lines = self.lines_before(w.start);
            crate::live::splice::replace_window(self, w, &old, &new_rows, lines, (0, 0));
        }
    }
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // one-range line sets are meant as such
mod tests {
    use super::*;

    fn set(r: &[Range<u32>]) -> RangeSet {
        RangeSet::from_ranges(r.to_vec())
    }

    #[test]
    fn ranges_merge_and_answer_membership() {
        let s = set(&[5..7, 1..2, 2..3, 6..9, 4..4]);
        assert_eq!(s.ranges(), &[1..3, 5..9]);
        assert!(s.contains(1) && s.contains(2) && !s.contains(3) && s.contains(8));
        assert!(!s.contains(9) && !s.contains(0));
        assert_eq!(s.line_count(), 6);
        assert!(s.contains_range(&(5..9)) && !s.contains_range(&(2..6)));
    }

    #[test]
    fn toggle_adds_then_removes() {
        let mut s = set(&[2..6]);
        s.toggle(3..5);
        assert_eq!(s.ranges(), &[2..3, 5..6]);
        s.toggle(2..6);
        assert_eq!(s.ranges(), &[2..6]);
        s.toggle(2..6);
        assert!(s.is_empty());
    }

    #[test]
    fn symmetric_difference_of_sets() {
        let a = set(&[0..4, 10..12]);
        let b = set(&[2..6, 10..12, 20..21]);
        assert_eq!(a.symmetric_difference(&b), vec![0..2, 4..6, 20..21]);
        assert!(a.symmetric_difference(&a).is_empty());
    }

    #[test]
    fn ranges_move_with_edits() {
        let d = |at, removed, inserted| LineDelta {
            at,
            removed,
            inserted,
        };
        let moved = |r: &[Range<u32>], delta| {
            let mut s = set(r);
            s.move_through(delta);
            s.ranges().to_vec()
        };
        // Enter inside the range: both halves stay ignored.
        assert_eq!(moved(&[3..8], d(5, 1, 2)), vec![3..9]);
        // Lines inserted at a range's start or end stay outside it.
        assert_eq!(moved(&[3..8], d(3, 0, 2)), vec![5..10]);
        assert_eq!(moved(&[3..8], d(8, 0, 2)), vec![3..8]);
        // The last line replaced by two: both join.
        assert_eq!(moved(&[3..8], d(7, 1, 2)), vec![3..9]);
        // A removal of part of the range; of all of it.
        assert_eq!(moved(&[3..8], d(3, 2, 0)), vec![3..6]);
        assert_eq!(moved(&[3..8], d(2, 7, 1)), Vec::<Range<u32>>::new());
        // Lines before and after.
        assert_eq!(moved(&[3..8], d(0, 1, 3)), vec![5..10]);
        assert_eq!(moved(&[3..8], d(9, 3, 0)), vec![3..8]);
        // Two lines merged into one across the range's start.
        assert_eq!(moved(&[3..8], d(2, 2, 1)), vec![3..7]);
    }
}
