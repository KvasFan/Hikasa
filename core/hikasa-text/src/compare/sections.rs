//! Sections (§2.17.2): [`SectionId`], [`Section`], [`SectionKind`]; statistics (§2.17.3) and the class runs
//! that the thumbnail and the display filters use, all derived from the rows in one pass.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::compare::rows::{Importance, PackedRow, RowState};
use crate::stats::{ChangeCounts, TextStats};

/// Section id: ordinal in row order (stable for identical inputs; ARCH-050).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SectionId(pub u32);

/// Shape of a section (normal-diff a/d/c).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    /// Only RightOnly rows.
    Add,
    /// Only LeftOnly rows.
    Delete,
    /// Anything else.
    Change,
}

impl SectionKind {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            SectionKind::Add => "add",
            SectionKind::Delete => "delete",
            SectionKind::Change => "change",
        }
    }
}

/// A maximal run of non-Same rows (or a single row for `lines_independent` formats).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Section {
    /// Id.
    pub id: SectionId,
    /// Rows `[start, end)`.
    pub rows: Range<u32>,
    /// Left lines covered `[start, end)` (empty for Add; start = insertion point).
    pub left_lines: Range<u32>,
    /// Right lines covered (empty for Delete).
    pub right_lines: Range<u32>,
    /// Kind.
    pub kind: SectionKind,
    /// Important if any row is important, else Unimportant.
    pub importance: Importance,
}

/// A maximal run of non-Same rows with equal state and importance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClassRun {
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) state: RowState,
    pub(crate) importance: Importance,
}

/// Everything derived from the rows.
#[derive(Debug, Clone, Default)]
pub(crate) struct Derived {
    pub(crate) sections: Vec<Section>,
    pub(crate) class_runs: Vec<ClassRun>,
    pub(crate) stats: TextStats,
}

/// A section being built.
struct Open {
    start: u32,
    left_start: u32,
    right_start: u32,
    left_rows: u32,
    right_rows: u32,
    only_left: bool,
    only_right: bool,
    important: bool,
}

/// Sections, class runs and statistics of `rows` in one pass (§2.17.2, §2.17.3).
pub(crate) fn derive(
    rows: &[PackedRow],
    left_lines: u32,
    right_lines: u32,
    lines_independent: bool,
) -> Derived {
    let mut d = Derived::default();
    let mut open: Option<Open> = None;
    let (mut left_seen, mut right_seen) = (0u32, 0u32);
    let s = &mut d.stats;
    s.rows = rows.len() as u64;
    s.left_lines = u64::from(left_lines);
    s.right_lines = u64::from(right_lines);
    for (k, row) in rows.iter().enumerate() {
        let k = k as u32;
        let state = row.state();
        let importance = row.importance();
        match state {
            RowState::Same => s.same_rows += 1,
            RowState::Changed => s.changed_rows += 1,
            RowState::LeftOnly => s.left_only_rows += 1,
            RowState::RightOnly => s.right_only_rows += 1,
        }
        match importance {
            Importance::Important => s.important_rows += 1,
            Importance::Unimportant => s.unimportant_rows += 1,
            Importance::None => {}
        }
        if state == RowState::Same {
            if let Some(o) = open.take() {
                close(&mut d.sections, o, k);
            }
        } else {
            // Class runs.
            match d.class_runs.last_mut() {
                Some(run) if run.end == k && run.state == state && run.importance == importance => {
                    run.end = k + 1;
                }
                _ => d.class_runs.push(ClassRun {
                    start: k,
                    end: k + 1,
                    state,
                    importance,
                }),
            }
            if lines_independent && let Some(o) = open.take() {
                close(&mut d.sections, o, k);
            }
            let o = open.get_or_insert(Open {
                start: k,
                left_start: left_seen,
                right_start: right_seen,
                left_rows: 0,
                right_rows: 0,
                only_left: true,
                only_right: true,
                important: false,
            });
            let has_left = row.left().is_some();
            let has_right = row.right().is_some();
            o.left_rows += u32::from(has_left);
            o.right_rows += u32::from(has_right);
            o.only_left &= state == RowState::LeftOnly;
            o.only_right &= state == RowState::RightOnly;
            o.important |= importance == Importance::Important;
        }
        left_seen += u32::from(row.left().is_some());
        right_seen += u32::from(row.right().is_some());
    }
    if let Some(o) = open.take() {
        close(&mut d.sections, o, rows.len() as u32);
    }
    for section in &d.sections {
        let class = if section.importance == Importance::Important {
            s.important_sections += 1;
            &mut s.important
        } else {
            &mut s.unimportant
        };
        add_counts(class, section);
    }
    s.sections = d.sections.len() as u64;
    d
}

fn close(sections: &mut Vec<Section>, o: Open, end: u32) {
    let kind = if o.only_right {
        SectionKind::Add
    } else if o.only_left {
        SectionKind::Delete
    } else {
        SectionKind::Change
    };
    sections.push(Section {
        id: SectionId(sections.len() as u32),
        rows: o.start..end,
        left_lines: o.left_start..o.left_start + o.left_rows,
        right_lines: o.right_start..o.right_start + o.right_rows,
        kind,
        importance: if o.important {
            Importance::Important
        } else {
            Importance::Unimportant
        },
    });
}

/// D-18: Add → added += R; Delete → deleted += L; Change → original += L, changed += R.
fn add_counts(c: &mut ChangeCounts, s: &Section) {
    let l = u64::from(s.left_lines.end - s.left_lines.start);
    let r = u64::from(s.right_lines.end - s.right_lines.start);
    match s.kind {
        SectionKind::Add => c.added += r,
        SectionKind::Delete => c.deleted += l,
        SectionKind::Change => {
            c.original += l;
            c.changed += r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(l: Option<u32>, r: Option<u32>, s: RowState, i: Importance) -> PackedRow {
        PackedRow::new(l, r, s, i)
    }

    #[test]
    fn sections_kinds_lines_and_statistics() {
        use Importance::{Important, Unimportant};
        use RowState::*;
        let no = Importance::None;
        let rows = [
            row(Some(0), Some(0), Same, no),
            row(Some(1), None, LeftOnly, Important),
            row(Some(2), None, LeftOnly, Unimportant),
            row(Some(3), Some(1), Same, no),
            row(None, Some(2), RightOnly, Unimportant),
            row(Some(4), Some(3), Same, no),
            row(Some(5), Some(4), Changed, Unimportant),
            row(None, Some(5), RightOnly, Important),
        ];
        let d = derive(&rows, 6, 6, false);
        assert_eq!(
            d.sections,
            [
                Section {
                    id: SectionId(0),
                    rows: 1..3,
                    left_lines: 1..3,
                    right_lines: 1..1,
                    kind: SectionKind::Delete,
                    importance: Important,
                },
                Section {
                    id: SectionId(1),
                    rows: 4..5,
                    left_lines: 4..4,
                    right_lines: 2..3,
                    kind: SectionKind::Add,
                    importance: Unimportant,
                },
                Section {
                    id: SectionId(2),
                    rows: 6..8,
                    left_lines: 5..6,
                    right_lines: 4..6,
                    kind: SectionKind::Change,
                    importance: Important,
                },
            ]
        );
        let s = d.stats;
        assert_eq!(
            (
                s.rows,
                s.same_rows,
                s.changed_rows,
                s.left_only_rows,
                s.right_only_rows
            ),
            (8, 3, 1, 2, 2)
        );
        assert_eq!((s.important_rows, s.unimportant_rows), (2, 3));
        assert_eq!((s.sections, s.important_sections), (3, 2));
        assert_eq!(
            s.important,
            ChangeCounts {
                added: 0,
                deleted: 2,
                changed: 2,
                original: 1
            }
        );
        assert_eq!(
            s.unimportant,
            ChangeCounts {
                added: 1,
                deleted: 0,
                changed: 0,
                original: 0
            }
        );
        assert!(s.identities_hold());
        assert_eq!(d.class_runs.len(), 5);
        // lines_independent: every non-Same row is its own section.
        let d = derive(&rows, 6, 6, true);
        assert_eq!(d.sections.len(), 5);
        assert_eq!(d.sections[1].rows, 2..3);
        assert_eq!(d.sections[1].left_lines, 2..3);
        assert!(d.stats.identities_hold());
    }

    #[test]
    fn empty_and_all_same() {
        let d = derive(&[], 0, 0, false);
        assert!(d.sections.is_empty());
        assert!(d.stats.identities_hold());
        let rows = [PackedRow::new(
            Some(0),
            Some(0),
            RowState::Same,
            Importance::None,
        )];
        let d = derive(&rows, 1, 1, false);
        assert!(d.sections.is_empty() && d.class_runs.is_empty());
    }
}
