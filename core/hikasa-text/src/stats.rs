//! Text statistics (§2.17.3, D-18; RPT text "Statistics" and "Summary"): [`TextStats`] and [`ChangeCounts`].

use serde::{Deserialize, Serialize};

/// Text statistics (RPT text "Statistics" + "Summary").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextStats {
    /// Rows.
    pub rows: u64,
    /// Same rows.
    pub same_rows: u64,
    /// Changed rows (both sides present).
    pub changed_rows: u64,
    /// Left-only rows.
    pub left_only_rows: u64,
    /// Right-only rows.
    pub right_only_rows: u64,
    /// Rows with importance Important.
    pub important_rows: u64,
    /// Rows with importance Unimportant.
    pub unimportant_rows: u64,
    /// Section-based counts for important sections (D-18).
    pub important: ChangeCounts,
    /// Section-based counts for unimportant sections.
    pub unimportant: ChangeCounts,
    /// Sections.
    pub sections: u64,
    /// Important sections.
    pub important_sections: u64,
    /// Left document lines.
    pub left_lines: u64,
    /// Right document lines.
    pub right_lines: u64,
    /// Ignored rows (TC-024, D-282; §1.10.17): rows whose left or right line is marked ignored, whatever their
    /// state. They count as same: an ignored non-Same row is in neither `important_rows` nor
    /// `unimportant_rows`, and a section whose rows are all ignored is in neither `sections` nor the D-18
    /// counts; the row-state counts (`same_rows` … `right_only_rows`) still describe the alignment. Additive
    /// (W10b); serialized only when not 0, so documents without ignored lines are unchanged.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ignored_rows: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// Added/deleted/changed/original line counts (D-18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChangeCounts {
    /// Right lines of `Add` sections.
    pub added: u64,
    /// Left lines of `Delete` sections.
    pub deleted: u64,
    /// Right lines of `Change` sections.
    pub changed: u64,
    /// Left lines of `Change` sections.
    pub original: u64,
}

impl TextStats {
    /// Checks the statistics identities (P-TC-5): the row classes partition the rows, importance classes
    /// partition the non-Same rows, important sections ≤ sections, and the D-18 counts cover exactly the
    /// non-Same rows' lines.
    ///
    /// With ignored rows (§1.10.17) the importance classes and the D-18 counts cover the non-Same rows that are
    /// not ignored, so those two identities become bounds: at most `ignored_rows` non-Same rows are outside the
    /// importance classes.
    pub fn identities_hold(&self) -> bool {
        let lines_in_sections = |c: &ChangeCounts| c.added + c.deleted + c.changed + c.original;
        let differing = self.rows.checked_sub(self.same_rows);
        let classified = self.important_rows + self.unimportant_rows;
        let section_lines =
            lines_in_sections(&self.important) + lines_in_sections(&self.unimportant);
        let all_lines = 2 * self.changed_rows + self.left_only_rows + self.right_only_rows;
        let shared = self.rows
            == self.same_rows + self.changed_rows + self.left_only_rows + self.right_only_rows
            && self.important_sections <= self.sections
            && self.left_lines == self.same_rows + self.changed_rows + self.left_only_rows
            && self.right_lines == self.same_rows + self.changed_rows + self.right_only_rows;
        let Some(differing) = differing else {
            return false;
        };
        shared
            && if self.ignored_rows == 0 {
                classified == differing && section_lines == all_lines
            } else {
                classified <= differing
                    && differing - classified <= self.ignored_rows
                    && self.ignored_rows <= self.rows
                    && section_lines <= all_lines
            }
    }
}

impl TextStats {
    /// Adds (`sign` 1) or removes (`sign` -1) the row and section counts of `other` (a provisional splice
    /// replaces a window's counts, §1.10.10); `rows`, `left_lines` and `right_lines` are left alone.
    pub(crate) fn merge_counts(&mut self, other: &TextStats, sign: i64) {
        let f = |a: &mut u64, b: u64| *a = (*a as i64 + sign * b as i64).max(0) as u64;
        f(&mut self.same_rows, other.same_rows);
        f(&mut self.changed_rows, other.changed_rows);
        f(&mut self.left_only_rows, other.left_only_rows);
        f(&mut self.right_only_rows, other.right_only_rows);
        f(&mut self.important_rows, other.important_rows);
        f(&mut self.unimportant_rows, other.unimportant_rows);
        f(&mut self.ignored_rows, other.ignored_rows);
        f(&mut self.sections, other.sections);
        f(&mut self.important_sections, other.important_sections);
        for (a, b) in [
            (&mut self.important, other.important),
            (&mut self.unimportant, other.unimportant),
        ] {
            f(&mut a.added, b.added);
            f(&mut a.deleted, b.deleted);
            f(&mut a.changed, b.changed);
            f(&mut a.original, b.original);
        }
    }
}
