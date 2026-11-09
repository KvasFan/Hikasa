//! The provisional splice (normative, §1.10.10): one `LineDelta` of one side applied to a comparison's rows,
//! runs, sections, class runs and statistics without aligning anything, so the rows stay valid for the live
//! documents at every moment.
//!
//! ```text
//! delta (at, removed, inserted) on side s, k = min(removed, inserted), d = inserted − removed:
//!   rows of s-lines [at, at + k)            keep their pairing, pending
//!   s-lines [at + k, at + inserted)         new pending orphan rows of s, after the row of line at + removed − 1
//!                                           (before the row of line at when removed = 0; at the end without one)
//!   rows of s-lines [at + inserted, at + removed)  lose their s side (a row left with no line is removed)
//!   every later s-line                      shifted by d
//! ```
//!
//! The work is done on a window of rows that holds the touched rows and is widened to whole runs and whole
//! sections, so runs, sections, class runs and the statistics are re-derived over the window only.

use std::ops::Range;

use hikasa_core::side::Side;
use hikasa_diff::{Run, RunKind};

use crate::compare::TextComparison;
use crate::compare::rows::{Importance, PackedRow, RowState};
use crate::compare::sections::{self, ClassRun, Section, SectionId};
use crate::edit::LineDelta;

/// A section of more rows than this takes an edit inside it by difference (`replace_inside`).
const LARGE_SECTION: u32 = 4096;

/// Sorted, disjoint row ranges that are pending.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Pending(pub(crate) Vec<Range<u32>>);

impl Pending {
    /// True when `row` is pending.
    pub(crate) fn contains(&self, row: u32) -> bool {
        let k = self.0.partition_point(|r| r.end <= row);
        self.0.get(k).is_some_and(|r| r.start <= row)
    }

    /// Replaces the window `old` (old numbering) with the window's new pending rows `flags` (starting at
    /// `old.start`), shifting later ranges by the window's growth.
    fn splice(&mut self, old: Range<u32>, flags: &[bool]) {
        let growth = flags.len() as i64 - i64::from(old.end - old.start);
        let mut out: Vec<Range<u32>> = Vec::with_capacity(self.0.len() + 2);
        let push = |r: Range<u32>, out: &mut Vec<Range<u32>>| {
            if r.is_empty() {
                return;
            }
            match out.last_mut() {
                Some(last) if last.end >= r.start => last.end = last.end.max(r.end),
                _ => out.push(r),
            }
        };
        for r in &self.0 {
            if r.end <= old.start {
                push(r.clone(), &mut out);
            } else if r.start < old.start {
                push(r.start..old.start, &mut out);
            }
        }
        let mut k = 0;
        while k < flags.len() {
            if flags[k] {
                let s = k;
                while k < flags.len() && flags[k] {
                    k += 1;
                }
                push(old.start + s as u32..old.start + k as u32, &mut out);
            } else {
                k += 1;
            }
        }
        for r in &self.0 {
            let (s, e) = (r.start.max(old.end), r.end);
            if s < e {
                let shift = |x: u32| (i64::from(x) + growth) as u32;
                push(shift(s)..shift(e), &mut out);
            }
        }
        self.0 = out;
    }

    /// Pending rows in total.
    pub(crate) fn count(&self) -> u32 {
        self.0.iter().map(|r| r.end - r.start).sum()
    }
}

/// One row of the window with the kind of its run and its pending flag.
#[derive(Debug, Clone, Copy)]
struct Item {
    row: PackedRow,
    kind: RunKind,
    pending: bool,
}

/// The row holding `line` of `side`, from the runs.
fn row_of(cmp: &TextComparison, side: Side, line: u32) -> Option<u32> {
    cmp.row_for_line(side, crate::document::LineIdx(line))
        .map(|r| r.0)
}

/// Index of the run holding row `row`.
fn run_at(cmp: &TextComparison, row: u32) -> usize {
    cmp.run_rows
        .partition_point(|&r| r <= row)
        .saturating_sub(1)
}

/// Applies one delta of `side` to `cmp` (see the module documentation).
pub(crate) fn splice(cmp: &mut TextComparison, side: Side, d: LineDelta, pending: &mut Pending) {
    if d.removed == 0 && d.inserted == 0 {
        return;
    }
    let is_left = side != Side::Right;
    let len = cmp.row_count();
    let side_line = |r: PackedRow| if is_left { r.left() } else { r.right() };
    // The touched rows.
    let (t0, t1) = if d.removed > 0 {
        let a = row_of(cmp, side, d.at).unwrap_or(len);
        let b = row_of(cmp, side, d.at + d.removed - 1).map_or(len, |r| r + 1);
        (a, b.max(a))
    } else {
        match row_of(cmp, side, d.at) {
            Some(r) => (r, r + 1),
            None => (len, len),
        }
    };
    // Widen to whole sections (a section boundary is a Same row or an end), so the window's sections and
    // statistics can be re-derived; runs are cut at the window's edges instead. Inside one very large section
    // (two unrelated files are one section) the window stays the touched rows and the section changes by
    // difference (its rows by class), so a keystroke costs O(touched rows + runs + sections), not O(section).
    let inside = (|| {
        if cmp.format.text().lines_independent || t0 == 0 || t1 >= len || t0 >= t1 {
            return None;
        }
        let a = cmp.section_at(crate::compare::RowIdx(t0 - 1))?;
        let b = cmp.section_at(crate::compare::RowIdx(t1))?;
        let rows = &cmp.sections[a.0 as usize].rows;
        (a == b && rows.end - rows.start > LARGE_SECTION).then_some(a.0 as usize)
    })();
    let (mut w0, mut w1) = (t0, t1);
    while inside.is_none() {
        let (a, b) = (w0, w1);
        if w0 > 0
            && let Some(id) = cmp.section_at(crate::compare::RowIdx(w0 - 1))
        {
            w0 = cmp.sections[id.0 as usize].rows.start;
        }
        if w1 < len
            && let Some(id) = cmp.section_at(crate::compare::RowIdx(w1))
        {
            w1 = cmp.sections[id.0 as usize].rows.end;
        }
        if (w0, w1) == (a, b) {
            break;
        }
    }
    // The runs the window cuts: [k0, k1) with the part of run k0 before the window and of run k1 − 1 after it.
    let k0 = if w0 < len {
        run_at(cmp, w0)
    } else {
        cmp.runs.len()
    };
    let k1 = if w1 > w0 { run_at(cmp, w1 - 1) + 1 } else { k0 };
    let cut = |run: &Run, from_row: u32, rows: Range<u32>| -> Run {
        // Rows `rows` (relative to the run's first row `from_row`) of `run`.
        let (j0, j1) = (rows.start - from_row, rows.end - from_row);
        let pairs = matches!(run.kind, RunKind::Equal | RunKind::Changed);
        let left = run.kind != RunKind::Insert;
        let right = run.kind != RunKind::Delete;
        Run {
            kind: run.kind,
            left: if left || pairs {
                run.left.start + j0..run.left.start + j1
            } else {
                run.left.start..run.left.start
            },
            right: if right || pairs {
                run.right.start + j0..run.right.start + j1
            } else {
                run.right.start..run.right.start
            },
        }
    };
    let head: Option<Run> = (k0 < cmp.runs.len() && w0 > cmp.run_rows[k0])
        .then(|| cut(&cmp.runs[k0], cmp.run_rows[k0], cmp.run_rows[k0]..w0));
    let tail_run: Option<Run> = (k1 > 0 && k1 > k0 && {
        let end = cmp.run_rows[k1 - 1] + cmp.runs[k1 - 1].rows();
        w1 < end
    })
    .then(|| {
        let start = cmp.run_rows[k1 - 1];
        cut(
            &cmp.runs[k1 - 1],
            start,
            w1..start + cmp.runs[k1 - 1].rows(),
        )
    });
    // Line cursors at the window start.
    let (lc, rc) = match &head {
        Some(h) => (h.left.end, h.right.end),
        None if k0 < cmp.runs.len() => (cmp.runs[k0].left.start, cmp.runs[k0].right.start),
        None => (
            cmp.runs.last().map_or(0, |r| r.left.end),
            cmp.runs.last().map_or(0, |r| r.right.end),
        ),
    };
    let old: Vec<Item> = (w0..w1)
        .map(|r| Item {
            row: cmp
                .rows
                .get(r as usize)
                .unwrap_or_else(|| PackedRow::new(None, None, RowState::Same, Importance::None)),
            kind: cmp.runs[run_at(cmp, r)].kind,
            pending: pending.contains(r),
        })
        .collect();
    // The transformation.
    let k = d.removed.min(d.inserted);
    let shift = i64::from(d.inserted) - i64::from(d.removed);
    let orphan = |line: u32| Item {
        row: if is_left {
            PackedRow::new(Some(line), None, RowState::LeftOnly, Importance::Important)
        } else {
            PackedRow::new(None, Some(line), RowState::RightOnly, Importance::Important)
        },
        kind: if is_left {
            RunKind::Delete
        } else {
            RunKind::Insert
        },
        pending: true,
    };
    let new_orphans = |out: &mut Vec<Item>| {
        for line in d.at + k..d.at + d.inserted {
            out.push(orphan(line));
        }
    };
    let mut new: Vec<Item> =
        Vec::with_capacity(old.len() + (d.inserted as usize).saturating_sub(d.removed as usize));
    let mut inserted = d.inserted <= d.removed;
    for item in &old {
        let Some(l) = side_line(item.row) else {
            new.push(*item);
            continue;
        };
        if l < d.at {
            new.push(*item);
        } else if l < d.at + k {
            new.push(Item {
                pending: true,
                ..*item
            });
            if !inserted && d.removed > 0 && l == d.at + d.removed - 1 {
                new_orphans(&mut new);
                inserted = true;
            }
        } else if l < d.at + d.removed {
            // The surplus lines lose their side.
            let other = if is_left {
                item.row.right()
            } else {
                item.row.left()
            };
            if let Some(o) = other {
                new.push(Item {
                    row: if is_left {
                        PackedRow::new(None, Some(o), RowState::RightOnly, Importance::Important)
                    } else {
                        PackedRow::new(Some(o), None, RowState::LeftOnly, Importance::Important)
                    },
                    kind: if is_left {
                        RunKind::Insert
                    } else {
                        RunKind::Delete
                    },
                    pending: true,
                });
            }
        } else {
            if !inserted && d.removed == 0 && l == d.at {
                new_orphans(&mut new);
                inserted = true;
            }
            let moved = (i64::from(l) + shift) as u32;
            let row = if is_left {
                item.row.with_lines(Some(moved), item.row.right())
            } else {
                item.row.with_lines(item.row.left(), Some(moved))
            };
            new.push(Item { row, ..*item });
        }
    }
    if !inserted {
        new_orphans(&mut new);
    }
    // Runs of the window.
    let mut runs: Vec<Run> = Vec::new();
    let (mut l, mut r) = (lc, rc);
    for item in &new {
        let (dl, dr) = (
            u32::from(item.row.left().is_some()),
            u32::from(item.row.right().is_some()),
        );
        match runs.last_mut() {
            Some(last) if last.kind == item.kind => {
                last.left.end += dl;
                last.right.end += dr;
            }
            _ => runs.push(Run {
                kind: item.kind,
                left: l..l + dl,
                right: r..r + dr,
            }),
        }
        l += dl;
        r += dr;
    }
    let old_rows: Vec<PackedRow> = old.iter().map(|i| i.row).collect();
    let new_rows: Vec<PackedRow> = new.iter().map(|i| i.row).collect();
    let (dl_shift, dr_shift) = if is_left { (shift, 0) } else { (0, shift) };
    // Runs: the untouched head of the first cut run, the window's runs, the rest of the last cut run, the later
    // runs shifted; same kinds meeting at a cut are merged.
    let moved_run = |run: &Run| Run {
        kind: run.kind,
        left: (i64::from(run.left.start) + dl_shift) as u32
            ..(i64::from(run.left.end) + dl_shift) as u32,
        right: (i64::from(run.right.start) + dr_shift) as u32
            ..(i64::from(run.right.end) + dr_shift) as u32,
    };
    let later: Vec<Run> = tail_run
        .iter()
        .chain(cmp.runs[k1..].iter())
        .map(moved_run)
        .collect();
    cmp.runs.truncate(k0);
    let push_run = |runs: &mut Vec<Run>, run: Run| match runs.last_mut() {
        Some(last)
            if last.kind == run.kind
                && last.left.end == run.left.start
                && last.right.end == run.right.start =>
        {
            last.left.end = run.left.end;
            last.right.end = run.right.end;
        }
        _ => runs.push(run),
    };
    for run in head.into_iter().chain(runs).chain(later) {
        push_run(&mut cmp.runs, run);
    }
    cmp.run_rows.clear();
    let mut at = 0u32;
    for run in &cmp.runs {
        cmp.run_rows.push(at);
        at += run.rows();
    }
    match inside {
        Some(section) => replace_inside(
            cmp,
            w0..w1,
            &old_rows,
            &new_rows,
            (dl_shift, dr_shift),
            section,
        ),
        None => replace_window(
            cmp,
            w0..w1,
            &old_rows,
            &new_rows,
            (lc, rc),
            (dl_shift, dr_shift),
        ),
    }
    // Pending rows.
    let flags: Vec<bool> = new.iter().map(|i| i.pending).collect();
    pending.splice(w0..w1, &flags);
}

/// Replaces the rows of window `w` (whole sections: a Same row or an end on each side) by `new_rows`, and the
/// window's sections, class runs and statistics by those of the new rows (by difference: the old window's are
/// taken out, the new one's put in); `lines` are the left and right lines the window starts at, `shift` the
/// change of line numbers of every later row. The runs are the caller's.
pub(crate) fn replace_window(
    cmp: &mut TextComparison,
    w: Range<u32>,
    old_rows: &[PackedRow],
    new_rows: &[PackedRow],
    lines: (u32, u32),
    shift: (i64, i64),
) {
    let (w0, w1) = (w.start, w.end);
    let (lc, rc) = lines;
    let (dl_shift, dr_shift) = shift;
    let lines_independent = cmp.format.text().lines_independent;
    let before = sections::derive(old_rows, 0, 0, lines_independent);
    let after = sections::derive(new_rows, 0, 0, lines_independent);
    let growth = new_rows.len() as i64 - old_rows.len() as i64;
    cmp.rows.splice(w0..w1, new_rows, (dl_shift, dr_shift));
    // Sections.
    let sa = cmp.sections.partition_point(|s| s.rows.end <= w0);
    let sb = cmp.sections.partition_point(|s| s.rows.start < w1);
    let moved_section = |s: &Section| Section {
        rows: (i64::from(s.rows.start) + growth) as u32..(i64::from(s.rows.end) + growth) as u32,
        left_lines: (i64::from(s.left_lines.start) + dl_shift) as u32
            ..(i64::from(s.left_lines.end) + dl_shift) as u32,
        right_lines: (i64::from(s.right_lines.start) + dr_shift) as u32
            ..(i64::from(s.right_lines.end) + dr_shift) as u32,
        ..s.clone()
    };
    let window_sections = after.sections.iter().map(|s| Section {
        rows: s.rows.start + w0..s.rows.end + w0,
        left_lines: s.left_lines.start + lc..s.left_lines.end + lc,
        right_lines: s.right_lines.start + rc..s.right_lines.end + rc,
        ..s.clone()
    });
    let renumber_from = sa;
    let tail: Vec<Section> = cmp.sections[sb..].iter().map(moved_section).collect();
    cmp.sections.truncate(sa);
    cmp.sections.extend(window_sections);
    cmp.sections.extend(tail);
    cmp.section_counts
        .splice(sa..sb, after.counts.iter().copied());
    for (k, s) in cmp.sections.iter_mut().enumerate().skip(renumber_from) {
        s.id = SectionId(k as u32);
    }
    // Class runs.
    let ca = cmp.class_runs.partition_point(|c| c.end <= w0);
    let cb = cmp.class_runs.partition_point(|c| c.start < w1);
    let tail: Vec<ClassRun> = cmp.class_runs[cb..]
        .iter()
        .map(|c| ClassRun {
            start: (i64::from(c.start) + growth) as u32,
            end: (i64::from(c.end) + growth) as u32,
            ..*c
        })
        .collect();
    cmp.class_runs.truncate(ca);
    cmp.class_runs
        .extend(after.class_runs.iter().map(|c| ClassRun {
            start: c.start + w0,
            end: c.end + w0,
            ..*c
        }));
    cmp.class_runs.extend(tail);
    // Statistics.
    cmp.stats.merge_counts(&before.stats, -1);
    cmp.stats.merge_counts(&after.stats, 1);
    cmp.stats.rows = u64::from(cmp.rows.len());
    cmp.stats.left_lines = (cmp.stats.left_lines as i64 + dl_shift) as u64;
    cmp.stats.right_lines = (cmp.stats.right_lines as i64 + dr_shift) as u64;
}

/// Replaces rows `w` (strictly inside section `section`, whose rows all stay non-Same: pending rows keep their
/// states and new or orphaned rows are LeftOnly or RightOnly) by `new_rows`: the section's rows by class, kind,
/// importance and line ranges change by difference, later sections move, the class runs of the window are
/// rebuilt and joined to their neighbours, the statistics change by difference.
fn replace_inside(
    cmp: &mut TextComparison,
    w: Range<u32>,
    old_rows: &[PackedRow],
    new_rows: &[PackedRow],
    shift: (i64, i64),
    section: usize,
) {
    let (w0, w1) = (w.start, w.end);
    let (dl, dr) = shift;
    let growth = new_rows.len() as i64 - old_rows.len() as i64;
    let before = sections::derive(old_rows, 0, 0, false);
    let after = sections::derive(new_rows, 0, 0, false);
    cmp.rows.splice(w0..w1, new_rows, shift);
    // Statistics: the rows by class, then the section's own share.
    let rows_only = |s: &crate::stats::TextStats| crate::stats::TextStats {
        sections: 0,
        important_sections: 0,
        important: Default::default(),
        unimportant: Default::default(),
        ..*s
    };
    cmp.stats.merge_counts(&rows_only(&before.stats), -1);
    cmp.stats.merge_counts(&rows_only(&after.stats), 1);
    cmp.stats
        .merge_counts(&sections::section_stats(&cmp.sections[section]), -1);
    let counts = &mut cmp.section_counts[section];
    counts.add_rows(old_rows, -1);
    counts.add_rows(new_rows, 1);
    let counts = *counts;
    let moved =
        |r: &Range<u32>, d: i64| (i64::from(r.start) + d) as u32..(i64::from(r.end) + d) as u32;
    {
        let s = &mut cmp.sections[section];
        s.rows.end = (i64::from(s.rows.end) + growth) as u32;
        s.left_lines.end = (i64::from(s.left_lines.end) + dl) as u32;
        s.right_lines.end = (i64::from(s.right_lines.end) + dr) as u32;
        s.kind = counts.kind();
        s.importance = if counts.important > 0 {
            Importance::Important
        } else {
            Importance::Unimportant
        };
    }
    cmp.stats
        .merge_counts(&sections::section_stats(&cmp.sections[section]), 1);
    for s in &mut cmp.sections[section + 1..] {
        s.rows = moved(&s.rows, growth);
        s.left_lines = moved(&s.left_lines, dl);
        s.right_lines = moved(&s.right_lines, dr);
    }
    // Class runs: the window's rebuilt, cut neighbours kept, equal neighbours joined.
    let ca = cmp.class_runs.partition_point(|c| c.end <= w0);
    let cb = cmp.class_runs.partition_point(|c| c.start < w1);
    let mut mid: Vec<ClassRun> = Vec::new();
    if let Some(c) = cmp.class_runs.get(ca)
        && c.start < w0
    {
        mid.push(ClassRun { end: w0, ..*c });
    }
    mid.extend(after.class_runs.iter().map(|c| ClassRun {
        start: c.start + w0,
        end: c.end + w0,
        ..*c
    }));
    if cb > 0
        && let Some(c) = cmp.class_runs.get(cb - 1)
        && c.end > w1
    {
        let new_w1 = (i64::from(w1) + growth) as u32;
        mid.push(ClassRun {
            start: new_w1,
            end: (i64::from(c.end) + growth) as u32,
            ..*c
        });
    }
    let tail: Vec<ClassRun> = cmp.class_runs[cb..]
        .iter()
        .map(|c| ClassRun {
            start: (i64::from(c.start) + growth) as u32,
            end: (i64::from(c.end) + growth) as u32,
            ..*c
        })
        .collect();
    cmp.class_runs.truncate(ca);
    for c in mid.into_iter().chain(tail) {
        match cmp.class_runs.last_mut() {
            Some(last)
                if last.end == c.start
                    && last.state == c.state
                    && last.importance == c.importance =>
            {
                last.end = c.end;
            }
            _ => cmp.class_runs.push(c),
        }
    }
    cmp.stats.rows = u64::from(cmp.rows.len());
    cmp.stats.left_lines = (cmp.stats.left_lines as i64 + dl) as u64;
    cmp.stats.right_lines = (cmp.stats.right_lines as i64 + dr) as u64;
}
