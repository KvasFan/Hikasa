//! The thumbnail (TC-032): [`ThumbCell`] buckets built from the class runs in O(runs + buckets).

use crate::compare::rows::{Importance, RowState};
use crate::compare::sections::ClassRun;

bitflags::bitflags! {
    /// One thumbnail bucket (TC-032): which row classes occur among the bucket's rows.
    ///
    /// A Same row sets `SAME`; a Changed row sets `IMPORTANT` or `UNIMPORTANT`; an orphan sets `LEFT_ONLY` or
    /// `RIGHT_ONLY` plus its importance flag. With ignore-unimportant, unimportant rows count as `SAME`. A
    /// bucket that covers no row (more buckets than rows) is empty.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct ThumbCell: u8 {
        /// Same rows (and unimportant ones under ignore-unimportant).
        const SAME = 1;
        /// Rows with only unimportant differences.
        const UNIMPORTANT = 2;
        /// Rows with important differences.
        const IMPORTANT = 4;
        /// Left orphans.
        const LEFT_ONLY = 8;
        /// Right orphans.
        const RIGHT_ONLY = 16;
    }
}

/// The cell flags of a class run.
fn run_flags(run: &ClassRun, ignore_unimportant: bool) -> ThumbCell {
    if run.importance == Importance::Unimportant && ignore_unimportant {
        return ThumbCell::SAME;
    }
    let mut flags = match run.importance {
        Importance::Important => ThumbCell::IMPORTANT,
        Importance::Unimportant => ThumbCell::UNIMPORTANT,
        Importance::None => ThumbCell::SAME,
    };
    match run.state {
        RowState::LeftOnly => flags |= ThumbCell::LEFT_ONLY,
        RowState::RightOnly => flags |= ThumbCell::RIGHT_ONLY,
        RowState::Same | RowState::Changed => {}
    }
    flags
}

/// `buckets` cells covering `rows` rows evenly (bucket `b` covers rows `[b·rows/buckets, (b+1)·rows/buckets)`),
/// from the class runs (the non-Same rows); rows outside every run are Same.
pub(crate) fn thumbnail(
    rows: u32,
    runs: &[ClassRun],
    buckets: u32,
    ignore_unimportant: bool,
) -> Vec<ThumbCell> {
    let mut out = Vec::with_capacity(buckets as usize);
    let mut k = 0usize;
    for b in 0..u64::from(buckets) {
        let start = (b * u64::from(rows) / u64::from(buckets)) as u32;
        let end = ((b + 1) * u64::from(rows) / u64::from(buckets)) as u32;
        let mut cell = ThumbCell::empty();
        if start < end {
            while k < runs.len() && runs[k].end <= start {
                k += 1;
            }
            let mut covered = 0u32;
            let mut j = k;
            while j < runs.len() && runs[j].start < end {
                let run = &runs[j];
                covered += run.end.min(end) - run.start.max(start);
                cell |= run_flags(run, ignore_unimportant);
                j += 1;
            }
            if covered < end - start {
                cell |= ThumbCell::SAME;
            }
        }
        out.push(cell);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(start: u32, end: u32, state: RowState, importance: Importance) -> ClassRun {
        ClassRun {
            start,
            end,
            state,
            importance,
        }
    }

    /// The per-row definition, for comparison.
    fn brute(rows: u32, runs: &[ClassRun], buckets: u32, iu: bool) -> Vec<ThumbCell> {
        (0..buckets)
            .map(|b| {
                let s = (u64::from(b) * u64::from(rows) / u64::from(buckets)) as u32;
                let e = ((u64::from(b) + 1) * u64::from(rows) / u64::from(buckets)) as u32;
                let mut cell = ThumbCell::empty();
                for row in s..e {
                    cell |= runs
                        .iter()
                        .find(|r| r.start <= row && row < r.end)
                        .map_or(ThumbCell::SAME, |r| run_flags(r, iu));
                }
                cell
            })
            .collect()
    }

    #[test]
    fn fewer_and_more_buckets_than_rows() {
        let runs = [
            run(2, 4, RowState::Changed, Importance::Unimportant),
            run(4, 5, RowState::LeftOnly, Importance::Important),
            run(9, 10, RowState::RightOnly, Importance::Unimportant),
        ];
        for buckets in [1, 2, 3, 5, 10, 17, 40] {
            for iu in [false, true] {
                assert_eq!(
                    thumbnail(10, &runs, buckets, iu),
                    brute(10, &runs, buckets, iu),
                    "{buckets} {iu}"
                );
            }
        }
        let cells = thumbnail(10, &runs, 2, false);
        assert_eq!(
            cells[0],
            ThumbCell::SAME | ThumbCell::UNIMPORTANT | ThumbCell::IMPORTANT | ThumbCell::LEFT_ONLY
        );
        assert_eq!(
            cells[1],
            ThumbCell::SAME | ThumbCell::UNIMPORTANT | ThumbCell::RIGHT_ONLY
        );
        let cells = thumbnail(10, &runs, 2, true);
        assert_eq!(
            cells[0],
            ThumbCell::SAME | ThumbCell::IMPORTANT | ThumbCell::LEFT_ONLY
        );
        assert_eq!(cells[1], ThumbCell::SAME);
        // More buckets than rows: some buckets are empty.
        let cells = thumbnail(2, &[], 5, false);
        assert_eq!(cells.len(), 5);
        assert_eq!(cells.iter().filter(|c| c.is_empty()).count(), 3);
        assert!(thumbnail(0, &[], 3, false).iter().all(|c| c.is_empty()));
        assert!(thumbnail(5, &runs, 0, false).is_empty());
    }
}
