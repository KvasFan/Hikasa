//! Statistics (FC-064; §2.6.5): [`FolderStats`] and [`ContentCounts`], read in O(1) from the root counters.

use hikasa_core::ContentKind;
use serde::{Deserialize, Serialize};

use crate::counts::{CategoryCounts, SideTotals};

/// Content result counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContentCounts {
    /// `BinarySame` results.
    pub binary_same: u32,
    /// `RulesSame` results.
    pub rules_same: u32,
    /// `UnimportantOnly` results.
    pub unimportant_only: u32,
    /// `ImportantDifferences` results.
    pub important_differences: u32,
    /// `BinaryDifferent` results.
    pub binary_different: u32,
    /// `Error` results.
    pub error: u32,
    /// Content tests skipped because the quick tests said "same" (flag `CONTENT_SKIPPED`, FC-026).
    pub not_compared: u32,
}

impl ContentCounts {
    /// Count of one result kind. Additive API.
    pub fn get(&self, kind: ContentKind) -> u32 {
        match kind {
            ContentKind::BinarySame => self.binary_same,
            ContentKind::RulesSame => self.rules_same,
            ContentKind::UnimportantOnly => self.unimportant_only,
            ContentKind::ImportantDifferences => self.important_differences,
            ContentKind::BinaryDifferent => self.binary_different,
            ContentKind::Error => self.error,
        }
    }
}

/// Folder statistics (FC-064).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FolderStats {
    /// File-like pairs by category.
    pub files: CategoryCounts,
    /// Folder pairs by category (the root excluded).
    pub folders: CategoryCounts,
    /// Left totals.
    pub left: SideTotals,
    /// Right totals.
    pub right: SideTotals,
    /// Content test results.
    pub content: ContentCounts,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_counts_get() {
        let c = ContentCounts {
            binary_same: 1,
            rules_same: 2,
            unimportant_only: 3,
            important_differences: 4,
            binary_different: 5,
            error: 6,
            not_compared: 7,
        };
        let got: Vec<u32> = ContentKind::ALL.iter().map(|&k| c.get(k)).collect();
        assert_eq!(got, [1, 2, 3, 4, 5, 6]);
    }
}
