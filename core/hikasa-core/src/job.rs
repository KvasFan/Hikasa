//! Job identifiers and kinds (ARCH-003, ARCH-060).

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Identifier of a job; unique within one `JobIdAllocator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub u64);

impl fmt::Display for JobId {
    /// Writes the number.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Kind of job (status bar text, log grouping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum JobKind {
    /// `quick`.
    QuickCompare,
    /// `text compare`.
    TextCompare,
    /// `folder compare`.
    FolderCompare,
    /// Report rendering.
    Report,
    /// `format tokens`.
    FormatTokens,
    /// `vfs ls`.
    VfsList,
    /// `vfs cat` / `vfs stat`.
    VfsRead,
}

impl JobKind {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            JobKind::QuickCompare => "quick_compare",
            JobKind::TextCompare => "text_compare",
            JobKind::FolderCompare => "folder_compare",
            JobKind::Report => "report",
            JobKind::FormatTokens => "format_tokens",
            JobKind::VfsList => "vfs_list",
            JobKind::VfsRead => "vfs_read",
        }
    }
}

/// Allocates increasing ids starting at 1. Owned by a service instance (never global, T7).
#[derive(Debug, Default)]
pub struct JobIdAllocator(AtomicU64);

impl JobIdAllocator {
    /// New allocator.
    pub const fn new() -> Self {
        JobIdAllocator(AtomicU64::new(0))
    }

    /// Next id (Relaxed ordering; uniqueness only).
    pub fn next(&self) -> JobId {
        JobId(self.0.fetch_add(1, Ordering::Relaxed).wrapping_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_start_at_one_and_increase() {
        let allocator = JobIdAllocator::new();
        assert_eq!(allocator.next(), JobId(1));
        assert_eq!(allocator.next(), JobId(2));
        assert_eq!(JobIdAllocator::default().next(), JobId(1));
        assert_eq!(JobId(7).to_string(), "7");
        assert_eq!(serde_json::to_string(&JobId(7)).unwrap(), "7");
    }

    #[test]
    fn ids_are_unique_across_threads() {
        let allocator = JobIdAllocator::new();
        let mut ids: Vec<u64> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| scope.spawn(|| (0..1000).map(|_| allocator.next().0).collect::<Vec<_>>()))
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect()
        });
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 4000);
        assert_eq!(ids.first(), Some(&1));
        assert_eq!(ids.last(), Some(&4000));
    }

    #[test]
    fn kind_names_match_serde() {
        for kind in [
            JobKind::QuickCompare,
            JobKind::TextCompare,
            JobKind::FolderCompare,
            JobKind::Report,
            JobKind::FormatTokens,
            JobKind::VfsList,
            JobKind::VfsRead,
        ] {
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{}\"", kind.as_str())
            );
        }
    }
}
