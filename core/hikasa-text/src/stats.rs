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
    pub fn identities_hold(&self) -> bool {
        let lines_in_sections = |c: &ChangeCounts| c.added + c.deleted + c.changed + c.original;
        self.rows == self.same_rows + self.changed_rows + self.left_only_rows + self.right_only_rows
            && self.important_rows + self.unimportant_rows == self.rows - self.same_rows
            && self.important_sections <= self.sections
            && lines_in_sections(&self.important) + lines_in_sections(&self.unimportant)
                == 2 * self.changed_rows + self.left_only_rows + self.right_only_rows
            && self.left_lines == self.same_rows + self.changed_rows + self.left_only_rows
            && self.right_lines == self.same_rows + self.changed_rows + self.right_only_rows
    }
}
