//! [`summary_counts`] (M2 W7, D-198, T11): the counts of the summary layouts, in their order, as one list. The text
//! and HTML summaries and the XML `summary/count` elements are all made from it, so the three cannot disagree.

use hikasa_core::ContentKind;
use hikasa_folder::{Category, FolderComparison, FolderStats};
use hikasa_text::TextComparison;
use serde::Serialize;

use crate::request::ReportInput;

/// One count of a summary: `group` and `name` are stable kebab-case words (XML `count/@group`, `count/@name`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct SummaryCount {
    /// `rows`, `sections`, `important-lines`, `unimportant-lines`, `lines` (text); `files`, `folders`, `left`,
    /// `right`, `content` (folders).
    pub group: &'static str,
    /// The count within its group (`total`, `same`, `left-newer`, `bytes`, …).
    pub name: &'static str,
    /// The value.
    pub value: u64,
}

impl SummaryCount {
    const fn new(group: &'static str, name: &'static str, value: u64) -> SummaryCount {
        SummaryCount { group, name, value }
    }
}

/// The counts of the summary layout of `input`, in the layout's order. Text: rows by state and importance, sections,
/// the D-18 line counts of important and unimportant sections, the lines of each side (none of them depends on
/// `ignore_unimportant`). Folders: file and folder pairs per category under `ignore_unimportant`, files, folders and
/// bytes per side, content-test results.
pub fn summary_counts(input: ReportInput<'_>, ignore_unimportant: bool) -> Vec<SummaryCount> {
    match input {
        ReportInput::Text(cmp) => text_counts(cmp),
        ReportInput::Folder(cmp) => folder_counts(&cmp.stats(ignore_unimportant)),
    }
}

/// The text counts.
pub(crate) fn text_counts(cmp: &TextComparison) -> Vec<SummaryCount> {
    let s = cmp.stats();
    let mut out = vec![
        SummaryCount::new("rows", "total", s.rows),
        SummaryCount::new("rows", "same", s.same_rows),
        SummaryCount::new("rows", "changed", s.changed_rows),
        SummaryCount::new("rows", "left-only", s.left_only_rows),
        SummaryCount::new("rows", "right-only", s.right_only_rows),
        SummaryCount::new("rows", "important", s.important_rows),
        SummaryCount::new("rows", "unimportant", s.unimportant_rows),
        SummaryCount::new("sections", "total", s.sections),
        SummaryCount::new("sections", "important", s.important_sections),
        SummaryCount::new(
            "sections",
            "unimportant",
            s.sections.saturating_sub(s.important_sections),
        ),
    ];
    for (group, c) in [
        ("important-lines", &s.important),
        ("unimportant-lines", &s.unimportant),
    ] {
        out.push(SummaryCount::new(group, "added", c.added));
        out.push(SummaryCount::new(group, "deleted", c.deleted));
        out.push(SummaryCount::new(group, "changed", c.changed));
        out.push(SummaryCount::new(group, "original", c.original));
    }
    out.push(SummaryCount::new("lines", "left", s.left_lines));
    out.push(SummaryCount::new("lines", "right", s.right_lines));
    out
}

/// The kebab-case word of a category (XML `state` spelling).
pub(crate) const fn category_word(c: Category) -> &'static str {
    match c {
        Category::Same => "same",
        Category::LeftNewer => "left-newer",
        Category::RightNewer => "right-newer",
        Category::Different => "different",
        Category::LeftOrphan => "left-only",
        Category::RightOrphan => "right-only",
        Category::Unknown => "unknown",
        Category::Error => "error",
        Category::Filtered => "filtered",
    }
}

/// The kebab-case word of a content result.
pub(crate) const fn content_word(k: ContentKind) -> &'static str {
    match k {
        ContentKind::BinarySame => "binary-same",
        ContentKind::RulesSame => "rules-same",
        ContentKind::UnimportantOnly => "unimportant-only",
        ContentKind::ImportantDifferences => "important-differences",
        ContentKind::BinaryDifferent => "binary-different",
        ContentKind::Error => "error",
    }
}

/// The folder counts of `stats`.
pub(crate) fn folder_counts(stats: &FolderStats) -> Vec<SummaryCount> {
    let mut out = Vec::with_capacity(32);
    for c in Category::ALL {
        out.push(SummaryCount::new(
            "files",
            category_word(c),
            u64::from(stats.files.get(c)),
        ));
    }
    for c in Category::ALL {
        out.push(SummaryCount::new(
            "folders",
            category_word(c),
            u64::from(stats.folders.get(c)),
        ));
    }
    for (group, t) in [("left", &stats.left), ("right", &stats.right)] {
        out.push(SummaryCount::new(group, "files", t.files));
        out.push(SummaryCount::new(group, "folders", t.folders));
        out.push(SummaryCount::new(group, "bytes", t.bytes));
    }
    for k in ContentKind::ALL {
        out.push(SummaryCount::new(
            "content",
            content_word(k),
            u64::from(stats.content.get(k)),
        ));
    }
    out.push(SummaryCount::new(
        "content",
        "not-compared",
        u64::from(stats.content.not_compared),
    ));
    out
}

/// The value of `(group, name)` in `counts` (0 when absent).
pub(crate) fn value(counts: &[SummaryCount], group: &str, name: &str) -> u64 {
    counts
        .iter()
        .find(|c| c.group == group && c.name == name)
        .map_or(0, |c| c.value)
}

/// The folder comparison's counts under `ignore_unimportant`.
pub(crate) fn of_folder(cmp: &FolderComparison, ignore_unimportant: bool) -> Vec<SummaryCount> {
    folder_counts(&cmp.stats(ignore_unimportant))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_counts_cover_every_category_and_result() {
        let counts = folder_counts(&FolderStats::default());
        assert_eq!(counts.len(), 9 + 9 + 3 + 3 + 7);
        assert!(counts.iter().all(|c| c.value == 0));
        assert_eq!(counts[0], SummaryCount::new("files", "same", 0));
        assert_eq!(counts[30].name, "not-compared");
        let mut keys: Vec<(&str, &str)> = counts.iter().map(|c| (c.group, c.name)).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), counts.len(), "every count is named once");
        assert_eq!(value(&counts, "left", "bytes"), 0);
        assert_eq!(value(&counts, "nothing", "here"), 0);
    }
}
