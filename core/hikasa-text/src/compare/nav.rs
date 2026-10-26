//! Navigation (TC-050): [`Direction`], next/previous section, the section of a row, and the row of a line
//! (Go To line). Every lookup is a binary search.

use hikasa_core::side::Side;
use hikasa_diff::Run;

use crate::compare::TextComparison;
use crate::compare::rows::{Importance, RowIdx};
use crate::compare::sections::SectionId;
use crate::document::LineIdx;

/// Search direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Towards the end.
    Forward,
    /// Towards the start.
    Backward,
}

impl TextComparison {
    /// Section containing a row.
    pub fn section_at(&self, idx: RowIdx) -> Option<SectionId> {
        let k = self
            .sections
            .partition_point(|s| s.rows.start <= idx.0)
            .checked_sub(1)?;
        let section = self.sections.get(k)?;
        (idx.0 < section.rows.end).then_some(section.id)
    }

    /// Next/previous section from `from` (exclusive), skipping Unimportant ones when `ignore_unimportant`
    /// (TC-050 Next Difference Section).
    ///
    /// Sections are found by their first row: forward gives the first section that starts after `from`;
    /// backward gives the last section that starts before `from` (so from inside a section, backward goes to
    /// that section's start).
    pub fn next_section(
        &self,
        from: RowIdx,
        dir: Direction,
        ignore_unimportant: bool,
    ) -> Option<SectionId> {
        self.find_section(from, dir, ignore_unimportant, false)
    }

    /// Like [`next_section`](Self::next_section), but going forward a section that *starts at* `from` counts
    /// as the next one: navigation from a position that is not a cursor, such as the first row on screen
    /// (TC-050). Backward is the same as `next_section`. Additive API (wave 6b, §1.5.7).
    pub fn next_section_from(
        &self,
        from: RowIdx,
        dir: Direction,
        ignore_unimportant: bool,
    ) -> Option<SectionId> {
        self.find_section(from, dir, ignore_unimportant, true)
    }

    fn find_section(
        &self,
        from: RowIdx,
        dir: Direction,
        ignore_unimportant: bool,
        inclusive: bool,
    ) -> Option<SectionId> {
        let wanted = |k: &usize| {
            !ignore_unimportant || self.sections[*k].importance == Importance::Important
        };
        let k = match dir {
            Direction::Forward => {
                let first = self.sections.partition_point(|s| {
                    s.rows.start < from.0 || (!inclusive && s.rows.start == from.0)
                });
                (first..self.sections.len()).find(wanted)
            }
            Direction::Backward => {
                let end = self.sections.partition_point(|s| s.rows.start < from.0);
                (0..end).rev().find(wanted)
            }
        }?;
        self.sections.get(k).map(|s| s.id)
    }

    /// Row showing `line` of `side` (Go To line). `Center` and `Output` use the left document (M1).
    pub fn row_for_line(&self, side: Side, line: LineIdx) -> Option<RowIdx> {
        let range = |run: &Run| match side {
            Side::Right => run.right.clone(),
            Side::Left | Side::Center | Side::Output => run.left.clone(),
        };
        let k = self.runs.partition_point(|run| range(run).end <= line.0);
        let run = self.runs.get(k)?;
        let r = range(run);
        if r.start <= line.0 && line.0 < r.end {
            Some(RowIdx(self.run_rows.get(k)? + (line.0 - r.start)))
        } else {
            None
        }
    }
}
