//! Per-folder counters (D-55; §2.6.4): [`CategoryCounts`], [`Aggregate`], [`Aggregates`], [`SideTotals`], and the
//! private deltas the model applies to every ancestor of a change.

use serde::{Deserialize, Serialize};

use crate::state::Category;

/// Per-category counts (u32: one model holds at most `u32::MAX - 1` nodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CategoryCounts {
    /// Same.
    pub same: u32,
    /// Left newer.
    pub left_newer: u32,
    /// Right newer.
    pub right_newer: u32,
    /// Different.
    pub different: u32,
    /// Left only.
    pub left_orphan: u32,
    /// Right only.
    pub right_orphan: u32,
    /// Unknown.
    pub unknown: u32,
    /// Error.
    pub error: u32,
    /// Filtered.
    pub filtered: u32,
}

impl CategoryCounts {
    /// Count of one category.
    pub fn get(&self, c: Category) -> u32 {
        match c {
            Category::Same => self.same,
            Category::LeftNewer => self.left_newer,
            Category::RightNewer => self.right_newer,
            Category::Different => self.different,
            Category::LeftOrphan => self.left_orphan,
            Category::RightOrphan => self.right_orphan,
            Category::Unknown => self.unknown,
            Category::Error => self.error,
            Category::Filtered => self.filtered,
        }
    }

    /// Mutable count of one category.
    fn get_mut(&mut self, c: Category) -> &mut u32 {
        match c {
            Category::Same => &mut self.same,
            Category::LeftNewer => &mut self.left_newer,
            Category::RightNewer => &mut self.right_newer,
            Category::Different => &mut self.different,
            Category::LeftOrphan => &mut self.left_orphan,
            Category::RightOrphan => &mut self.right_orphan,
            Category::Unknown => &mut self.unknown,
            Category::Error => &mut self.error,
            Category::Filtered => &mut self.filtered,
        }
    }

    /// left_newer + right_newer + different + left_orphan + right_orphan.
    pub fn differences(&self) -> u64 {
        u64::from(self.left_newer)
            + u64::from(self.right_newer)
            + u64::from(self.different)
            + u64::from(self.left_orphan)
            + u64::from(self.right_orphan)
    }

    /// Sum of all counters.
    pub fn total(&self) -> u64 {
        Category::ALL.iter().map(|&c| u64::from(self.get(c))).sum()
    }

    /// Field-wise sum (saturating).
    pub fn plus(&self, other: &CategoryCounts) -> CategoryCounts {
        let mut out = *self;
        for c in Category::ALL {
            let slot = out.get_mut(c);
            *slot = slot.saturating_add(other.get(c));
        }
        out
    }

    /// Adds one to category `c` (saturating).
    pub(crate) fn add_one(&mut self, c: Category) {
        let slot = self.get_mut(c);
        *slot = slot.saturating_add(1);
    }

    /// Adds a signed delta per category (clamped to the `u32` range; a correct model never clamps, and
    /// INV-FOLDER-5 would report it).
    fn apply(&mut self, delta: &[i64; 9]) {
        for c in Category::ALL {
            let d = delta[c.index()];
            if d != 0 {
                let slot = self.get_mut(c);
                *slot = (i64::from(*slot) + d).clamp(0, i64::from(u32::MAX)) as u32;
            }
        }
    }
}

/// Counts of a folder's descendants (D-55): file-like nodes in `files`, folder nodes (by their own category,
/// §2.6.4) in `folders`. The folder itself is not counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Aggregate {
    /// File-like descendants (files, symlinks, others) by category.
    pub files: CategoryCounts,
    /// Folder descendants by their own category.
    pub folders: CategoryCounts,
}

impl Aggregate {
    /// `files + folders`, field-wise (the colour and `CompareStructure` visibility input).
    pub fn both(&self) -> CategoryCounts {
        self.files.plus(&self.folders)
    }
}

/// Aggregates for both values of ignore-unimportant (FC-057 toggles without recomputation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Aggregates {
    /// With ignore-unimportant off.
    pub all: Aggregate,
    /// With ignore-unimportant on.
    pub ignoring_unimportant: Aggregate,
}

impl Aggregates {
    /// `all` or `ignoring_unimportant`.
    pub fn get(&self, ignore_unimportant: bool) -> &Aggregate {
        if ignore_unimportant {
            &self.ignoring_unimportant
        } else {
            &self.all
        }
    }

    /// Mutable `all` or `ignoring_unimportant`.
    pub(crate) fn get_mut(&mut self, ignore_unimportant: bool) -> &mut Aggregate {
        if ignore_unimportant {
            &mut self.ignoring_unimportant
        } else {
            &mut self.all
        }
    }

    /// Applies a delta to both aggregates.
    pub(crate) fn apply(&mut self, delta: &AggDelta) {
        for (iu, d) in [(false, &delta.all), (true, &delta.ignoring)] {
            let a = self.get_mut(iu);
            a.files.apply(&d[0]);
            a.folders.apply(&d[1]);
        }
    }
}

/// Totals of present, non-filtered entries of one side in a subtree (FC-040 folder size, FC-064).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SideTotals {
    /// File-like entries (files, symlinks, others).
    pub files: u64,
    /// Folders.
    pub folders: u64,
    /// Bytes of regular files.
    pub bytes: u64,
}

impl SideTotals {
    /// Field-wise saturating sum.
    pub(crate) fn add(&mut self, other: &SideTotals) {
        self.files = self.files.saturating_add(other.files);
        self.folders = self.folders.saturating_add(other.folders);
        self.bytes = self.bytes.saturating_add(other.bytes);
    }

    /// True when every field is zero.
    pub(crate) fn is_zero(&self) -> bool {
        self.files == 0 && self.folders == 0 && self.bytes == 0
    }
}

/// A change of the counters of every ancestor: `[files, folders]` deltas per category, for both ignore-unimportant
/// values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct AggDelta {
    /// ignore-unimportant off.
    pub(crate) all: [[i64; 9]; 2],
    /// ignore-unimportant on.
    pub(crate) ignoring: [[i64; 9]; 2],
}

/// Slot of a contribution: a file-like node or a folder node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    /// `files`.
    Files,
    /// `folders`.
    Folders,
}

impl AggDelta {
    /// Adds `n` to category `c` of `slot` for one ignore-unimportant value.
    pub(crate) fn add(&mut self, iu: bool, slot: Slot, c: Category, n: i64) {
        let table = if iu {
            &mut self.ignoring
        } else {
            &mut self.all
        };
        let row = match slot {
            Slot::Files => 0,
            Slot::Folders => 1,
        };
        table[row][c.index()] += n;
    }

    /// Records a change of a contribution from `old` to `new` categories (per ignore-unimportant value).
    pub(crate) fn change(&mut self, slot: Slot, old: [Category; 2], new: [Category; 2]) {
        for (i, iu) in [false, true].into_iter().enumerate() {
            if old[i] != new[i] {
                self.add(iu, slot, old[i], -1);
                self.add(iu, slot, new[i], 1);
            }
        }
    }

    /// True when nothing changes.
    pub(crate) fn is_zero(&self) -> bool {
        self.all
            .iter()
            .chain(self.ignoring.iter())
            .all(|row| row.iter().all(|&v| v == 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_helpers() {
        let mut c = CategoryCounts::default();
        for cat in Category::ALL {
            c.add_one(cat);
        }
        c.add_one(Category::Different);
        assert_eq!(c.total(), 10);
        assert_eq!(c.differences(), 6);
        assert_eq!(c.get(Category::Different), 2);
        assert_eq!(c.plus(&c).get(Category::Error), 2);
        let mut d = AggDelta::default();
        assert!(d.is_zero());
        d.change(
            Slot::Files,
            [Category::Different, Category::Different],
            [Category::Different, Category::Same],
        );
        assert!(!d.is_zero());
        let mut a = Aggregates::default();
        a.ignoring_unimportant.files.different = 1;
        a.apply(&d);
        assert_eq!(a.ignoring_unimportant.files.same, 1);
        assert_eq!(a.ignoring_unimportant.files.different, 0);
        assert_eq!(a.get(false).files.total(), 0);
        let mut t = SideTotals::default();
        assert!(t.is_zero());
        t.add(&SideTotals {
            files: 1,
            folders: 2,
            bytes: 3,
        });
        assert!(!t.is_zero());
        assert_eq!(a.get(true).both().same, 1);
    }
}
