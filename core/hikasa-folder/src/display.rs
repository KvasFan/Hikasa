//! Folder display filters (FC-055, FC-056, FC-057; algorithm §2.7; D-19, D-44), visible rows, and colour classes
//! (FC-031; §2.6.4).

use hikasa_core::Side;
use serde::{Deserialize, Serialize};

use crate::error::FolderError;
use crate::model::store::{Node, Store, bits, internal};
use crate::model::{FolderComparison, NodeId, ScanStatus};
use crate::resolve::side_index;
use crate::state::Category;

bitflags::bitflags! {
    /// Atomic folder display categories (FC-055 toggle mode).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct FolderToggles: u8 {
        /// Same items.
        const SAME = 1;
        /// Left-only items.
        const LEFT_ORPHANS = 2;
        /// Right-only items.
        const RIGHT_ORPHANS = 4;
        /// Left newer items.
        const LEFT_NEWER = 8;
        /// Right newer items.
        const RIGHT_NEWER = 16;
        /// Different, unknown and error items (D-19).
        const DIFFERENCES = 32;
    }
}

impl FolderToggles {
    /// The atom of a display category (§2.7): Same → SAME, LeftOrphan → LEFT_ORPHANS, RightOrphan →
    /// RIGHT_ORPHANS, LeftNewer → LEFT_NEWER, RightNewer → RIGHT_NEWER, Different/Unknown/Error → DIFFERENCES
    /// (D-19). `Filtered` has no atom (governed by Suppress Filters only, D-44). Additive API.
    pub const fn atom(category: Category) -> Option<FolderToggles> {
        match category {
            Category::Same => Some(FolderToggles::SAME),
            Category::LeftOrphan => Some(FolderToggles::LEFT_ORPHANS),
            Category::RightOrphan => Some(FolderToggles::RIGHT_ORPHANS),
            Category::LeftNewer => Some(FolderToggles::LEFT_NEWER),
            Category::RightNewer => Some(FolderToggles::RIGHT_NEWER),
            Category::Different | Category::Unknown | Category::Error => {
                Some(FolderToggles::DIFFERENCES)
            }
            Category::Filtered => None,
        }
    }

    /// `toggles ∋ atom(c)` (false for `Filtered`). Additive API.
    pub fn accepts(self, category: Category) -> bool {
        FolderToggles::atom(category).is_some_and(|atom| self.contains(atom))
    }
}

/// Display filter presets (FC-055) plus toggle mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderDisplayFilter {
    /// Show All.
    All,
    /// Show Differences.
    Differences,
    /// Show Same.
    Same,
    /// Show No Orphans.
    NoOrphans,
    /// Show Differences but No Orphans.
    DifferencesNoOrphans,
    /// Show Orphans.
    Orphans,
    /// Show Left Newer.
    LeftNewer,
    /// Show Right Newer.
    RightNewer,
    /// Show Left Newer and Left Orphans.
    LeftNewerAndLeftOrphans,
    /// Show Right Newer and Right Orphans.
    RightNewerAndRightOrphans,
    /// Show Left Orphans.
    LeftOrphans,
    /// Show Right Orphans.
    RightOrphans,
    /// Show None.
    None,
    /// Toggle mode with explicit `FolderToggles` bits.
    Toggles {
        /// The `FolderToggles` bits (unknown bits are ignored).
        bits: u8,
    },
}

impl FolderDisplayFilter {
    /// Every preset (toggle mode excluded), in declaration order. Additive API.
    pub const PRESETS: [FolderDisplayFilter; 13] = [
        FolderDisplayFilter::All,
        FolderDisplayFilter::Differences,
        FolderDisplayFilter::Same,
        FolderDisplayFilter::NoOrphans,
        FolderDisplayFilter::DifferencesNoOrphans,
        FolderDisplayFilter::Orphans,
        FolderDisplayFilter::LeftNewer,
        FolderDisplayFilter::RightNewer,
        FolderDisplayFilter::LeftNewerAndLeftOrphans,
        FolderDisplayFilter::RightNewerAndRightOrphans,
        FolderDisplayFilter::LeftOrphans,
        FolderDisplayFilter::RightOrphans,
        FolderDisplayFilter::None,
    ];

    /// Equivalent toggle set (table in §2.7).
    pub fn toggles(self) -> FolderToggles {
        use FolderToggles as T;
        match self {
            FolderDisplayFilter::All => T::all(),
            FolderDisplayFilter::Differences => {
                T::LEFT_ORPHANS | T::RIGHT_ORPHANS | T::LEFT_NEWER | T::RIGHT_NEWER | T::DIFFERENCES
            }
            FolderDisplayFilter::Same => T::SAME,
            FolderDisplayFilter::NoOrphans => {
                T::SAME | T::LEFT_NEWER | T::RIGHT_NEWER | T::DIFFERENCES
            }
            FolderDisplayFilter::DifferencesNoOrphans => {
                T::LEFT_NEWER | T::RIGHT_NEWER | T::DIFFERENCES
            }
            FolderDisplayFilter::Orphans => T::LEFT_ORPHANS | T::RIGHT_ORPHANS,
            FolderDisplayFilter::LeftNewer => T::LEFT_NEWER,
            FolderDisplayFilter::RightNewer => T::RIGHT_NEWER,
            FolderDisplayFilter::LeftNewerAndLeftOrphans => T::LEFT_ORPHANS | T::LEFT_NEWER,
            FolderDisplayFilter::RightNewerAndRightOrphans => T::RIGHT_ORPHANS | T::RIGHT_NEWER,
            FolderDisplayFilter::LeftOrphans => T::LEFT_ORPHANS,
            FolderDisplayFilter::RightOrphans => T::RIGHT_ORPHANS,
            FolderDisplayFilter::None => T::empty(),
            FolderDisplayFilter::Toggles { bits } => T::from_bits_truncate(bits),
        }
    }

    /// Parses CLI values (`differences-no-orphans`) and script names (`display-mismatches-no-orphans`).
    ///
    /// Accepted (ASCII case-insensitive, `_` equals `-`, an optional `display-` prefix): `all`, `differences` (or
    /// `mismatches`), `same` (or `matches`), `no-orphans`, `differences-no-orphans` (or `mismatches-no-orphans`),
    /// `orphans`, `left-newer`, `right-newer`, `left-newer-orphans` (or `left-newer-and-left-orphans`),
    /// `right-newer-orphans` (or `right-newer-and-right-orphans`), `left-orphans`, `right-orphans`, `none`.
    pub fn parse(s: &str) -> Result<FolderDisplayFilter, FolderError> {
        let normalized = s.trim().to_ascii_lowercase().replace('_', "-");
        let word = normalized
            .strip_prefix("display-")
            .unwrap_or(normalized.as_str());
        Ok(match word {
            "all" => FolderDisplayFilter::All,
            "differences" | "mismatches" => FolderDisplayFilter::Differences,
            "same" | "matches" => FolderDisplayFilter::Same,
            "no-orphans" => FolderDisplayFilter::NoOrphans,
            "differences-no-orphans" | "mismatches-no-orphans" => {
                FolderDisplayFilter::DifferencesNoOrphans
            }
            "orphans" => FolderDisplayFilter::Orphans,
            "left-newer" => FolderDisplayFilter::LeftNewer,
            "right-newer" => FolderDisplayFilter::RightNewer,
            "left-newer-orphans" | "left-newer-and-left-orphans" => {
                FolderDisplayFilter::LeftNewerAndLeftOrphans
            }
            "right-newer-orphans" | "right-newer-and-right-orphans" => {
                FolderDisplayFilter::RightNewerAndRightOrphans
            }
            "left-orphans" => FolderDisplayFilter::LeftOrphans,
            "right-orphans" => FolderDisplayFilter::RightOrphans,
            "none" => FolderDisplayFilter::None,
            _ => {
                return Err(FolderError::InvalidRules(format!(
                    "unknown display filter `{s}`"
                )));
            }
        })
    }
}

/// Folder display mode (FC-056).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderMode {
    /// Always Show Folders.
    AlwaysShowFolders,
    /// Compare Files and Folder Structure.
    #[default]
    CompareStructure,
    /// Only Compare Files.
    OnlyFiles,
}

impl FolderMode {
    /// Every mode in declaration order. Additive API.
    pub const ALL: [FolderMode; 3] = [
        FolderMode::AlwaysShowFolders,
        FolderMode::CompareStructure,
        FolderMode::OnlyFiles,
    ];
}

/// Display options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DisplayOptions {
    /// Filter.
    pub filter: FolderDisplayFilter,
    /// Folder mode.
    pub folders: FolderMode,
    /// FC-057 Ignore Unimportant Differences.
    pub ignore_unimportant: bool,
    /// FC-057 Suppress Filters: reveals items hidden by file filters, in teal (D-44).
    pub suppress_filters: bool,
}

/// A visible row of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VisibleRow {
    /// Node.
    pub node: NodeId,
    /// Depth (root children = 0).
    pub depth: u32,
}

/// Visible rows in pre-order (random access for a virtualised tree).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VisibleTree {
    rows: Vec<VisibleRow>,
}

impl VisibleTree {
    /// Rows.
    pub fn rows(&self) -> &[VisibleRow] {
        &self.rows
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no row is visible.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// `TreeRead::visible_rows` collected (CLI and reports pass `|_| true`).
pub fn visible_tree(
    cmp: &FolderComparison,
    opts: &DisplayOptions,
    expanded: &dyn Fn(NodeId) -> bool,
) -> VisibleTree {
    let mut rows = Vec::new();
    cmp.read().visible_rows(opts, expanded, &mut rows);
    VisibleTree { rows }
}

/// Colour class of a node on one side (FC-031 mapping, §2.6.4; actual colours are theme data).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColourClass {
    /// Same (black).
    Same,
    /// Newer or different, or a folder that contains newer/different items (red).
    NewerOrDifferent,
    /// Orphan, or a folder that contains orphans (purple).
    Orphan,
    /// Older or unknown, or a folder that contains older items (grey).
    OlderOrUnknown,
    /// Hidden by a filter, shown by Suppress Filters (teal).
    Filtered,
    /// Error.
    Error,
    /// No entry on this side.
    Absent,
}

/// Whether a node is visible under `opts` (§2.7), O(1).
pub(crate) fn is_visible(store: &Store, node: &Node, opts: &DisplayOptions) -> bool {
    let iu = opts.ignore_unimportant;
    let toggles = opts.filter.toggles();
    let category = store.category_of(node, iu);
    if category == Category::Filtered {
        return opts.suppress_filters;
    }
    if !node.is_folder() {
        return toggles.accepts(category);
    }
    let agg = store.aggregates(node);
    let a = agg.get(iu);
    let below = |counts: &crate::counts::CategoryCounts| {
        Category::ALL
            .iter()
            .any(|&c| toggles.accepts(c) && counts.get(c) > 0)
            || (opts.suppress_filters && counts.filtered > 0)
    };
    match opts.folders {
        FolderMode::AlwaysShowFolders => true,
        FolderMode::CompareStructure => toggles.accepts(category) || below(&a.both()),
        FolderMode::OnlyFiles => below(&a.files),
    }
}

/// Appends the visible rows in pre-order (§2.7): a node is listed iff it is visible, its parent is listed (the
/// root counts as listed) and its parent is expanded (the root always is).
pub(crate) fn visible_rows_into(
    store: &Store,
    opts: &DisplayOptions,
    expanded: &dyn Fn(NodeId) -> bool,
    out: &mut Vec<VisibleRow>,
) {
    let Some(root) = store.node(NodeId::ROOT) else {
        return;
    };
    let mut stack: Vec<(u32, u32)> = root.children().rev().map(|c| (c, 0)).collect();
    while let Some((id, depth)) = stack.pop() {
        let Some(node) = store.node(NodeId(id)) else {
            continue;
        };
        if !is_visible(store, node, opts) {
            continue;
        }
        out.push(VisibleRow {
            node: NodeId(id),
            depth,
        });
        if node.is_folder() && node.child_count > 0 && expanded(NodeId(id)) {
            stack.extend(node.children().rev().map(|c| (c, depth + 1)));
        }
    }
}

/// The reference definition of §2.7 used by the invariant check INV-FOLDER-9: visibility from the explicit
/// "has a visible descendant" definition instead of the counters, evaluated bottom-up (children have larger ids
/// than their folder, so one pass in decreasing id order is linear), then listed top-down.
pub(crate) fn reference_rows(store: &Store, opts: &DisplayOptions) -> Vec<VisibleRow> {
    let len = store.len() as usize;
    let toggles = opts.filter.toggles();
    let iu = opts.ignore_unimportant;
    // visible[id], and whether a visible file lies below a folder.
    let mut visible = vec![false; len];
    let mut file_below = vec![false; len];
    for id in (0..len).rev() {
        let Some(node) = store.node(NodeId(id as u32)) else {
            continue;
        };
        let any_child = |flags: &[bool]| {
            node.children()
                .any(|c| flags.get(c as usize).copied().unwrap_or(false))
        };
        let category = store.category_of(node, iu);
        let self_visible = if category == Category::Filtered {
            opts.suppress_filters
        } else if !node.is_folder() {
            toggles.accepts(category)
        } else {
            match opts.folders {
                FolderMode::AlwaysShowFolders => true,
                FolderMode::CompareStructure => toggles.accepts(category) || any_child(&visible),
                FolderMode::OnlyFiles => any_child(&file_below),
            }
        };
        let files = if node.is_folder() {
            any_child(&file_below)
        } else {
            self_visible
        };
        if let Some(slot) = visible.get_mut(id) {
            *slot = self_visible;
        }
        if let Some(slot) = file_below.get_mut(id) {
            *slot = files;
        }
    }
    let mut out = Vec::new();
    let mut stack: Vec<(u32, u32)> = store
        .node(NodeId::ROOT)
        .map(|root| root.children().rev().map(|c| (c, 0)).collect())
        .unwrap_or_default();
    while let Some((id, depth)) = stack.pop() {
        if !visible.get(id as usize).copied().unwrap_or(false) {
            continue;
        }
        out.push(VisibleRow {
            node: NodeId(id),
            depth,
        });
        if let Some(node) = store.node(NodeId(id)) {
            stack.extend(node.children().rev().map(|c| (c, depth + 1)));
        }
    }
    out
}

/// Colour class of `node` on `side` (§2.6.4 table), O(1).
pub(crate) fn colour_of(store: &Store, node: &Node, side: Side, iu: bool) -> ColourClass {
    if !matches!(side, Side::Left | Side::Right) {
        return ColourClass::Absent;
    }
    let rec = &node.sides[side_index(side)];
    if !rec.present() {
        return ColourClass::Absent;
    }
    if rec.has(bits::FILTERED) {
        return ColourClass::Filtered;
    }
    let category = store.category_of(node, iu);
    if !node.is_folder() {
        return match category {
            Category::Same => ColourClass::Same,
            Category::Different => ColourClass::NewerOrDifferent,
            Category::Error => ColourClass::Error,
            Category::LeftNewer => {
                if side == Side::Left {
                    ColourClass::NewerOrDifferent
                } else {
                    ColourClass::OlderOrUnknown
                }
            }
            Category::RightNewer => {
                if side == Side::Right {
                    ColourClass::NewerOrDifferent
                } else {
                    ColourClass::OlderOrUnknown
                }
            }
            Category::LeftOrphan | Category::RightOrphan => ColourClass::Orphan,
            Category::Unknown => ColourClass::OlderOrUnknown,
            Category::Filtered => ColourClass::Filtered,
        };
    }
    // The same order as `own_category`: a folder that could not be listed is an error, one-sided too.
    if node.scan() == ScanStatus::Failed {
        return ColourClass::Error;
    }
    if node.internal & internal::PARTIAL_UNKNOWN != 0 {
        return ColourClass::OlderOrUnknown;
    }
    let present = node.sides.iter().filter(|s| s.present()).count();
    if present == 1 {
        return ColourClass::Orphan;
    }
    let c = store.aggregates(node).get(iu).both();
    let (this_newer, other_newer) = if side == Side::Left {
        (c.left_newer, c.right_newer)
    } else {
        (c.right_newer, c.left_newer)
    };
    if c.different > 0 || this_newer > 0 {
        ColourClass::NewerOrDifferent
    } else if c.left_orphan + c.right_orphan > 0 {
        ColourClass::Orphan
    } else if other_newer > 0 || c.unknown > 0 || node.scan() == ScanStatus::NotScanned {
        ColourClass::OlderOrUnknown
    } else if c.error > 0 {
        ColourClass::Error
    } else if c.filtered > 0 && c.total() == u64::from(c.filtered) {
        ColourClass::Filtered
    } else {
        ColourClass::Same
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §2.7 preset table (13 rows).
    #[test]
    fn preset_table() {
        use FolderDisplayFilter as F;
        // Columns: SAME, LEFT_ORPHANS, RIGHT_ORPHANS, LEFT_NEWER, RIGHT_NEWER, DIFFERENCES.
        let rows: [(F, [bool; 6]); 13] = [
            (F::All, [true, true, true, true, true, true]),
            (F::Differences, [false, true, true, true, true, true]),
            (F::Same, [true, false, false, false, false, false]),
            (F::NoOrphans, [true, false, false, true, true, true]),
            (
                F::DifferencesNoOrphans,
                [false, false, false, true, true, true],
            ),
            (F::Orphans, [false, true, true, false, false, false]),
            (F::LeftNewer, [false, false, false, true, false, false]),
            (F::RightNewer, [false, false, false, false, true, false]),
            (
                F::LeftNewerAndLeftOrphans,
                [false, true, false, true, false, false],
            ),
            (
                F::RightNewerAndRightOrphans,
                [false, false, true, false, true, false],
            ),
            (F::LeftOrphans, [false, true, false, false, false, false]),
            (F::RightOrphans, [false, false, true, false, false, false]),
            (F::None, [false; 6]),
        ];
        let atoms = [
            FolderToggles::SAME,
            FolderToggles::LEFT_ORPHANS,
            FolderToggles::RIGHT_ORPHANS,
            FolderToggles::LEFT_NEWER,
            FolderToggles::RIGHT_NEWER,
            FolderToggles::DIFFERENCES,
        ];
        for (preset, expected) in rows {
            let t = preset.toggles();
            for (atom, want) in atoms.iter().zip(expected) {
                assert_eq!(t.contains(*atom), want, "{preset:?} {atom:?}");
            }
            let bits = FolderDisplayFilter::Toggles { bits: t.bits() };
            assert_eq!(bits.toggles(), t);
        }
        assert_eq!(FolderDisplayFilter::PRESETS.len(), 13);
    }

    #[test]
    fn atoms_and_acceptance() {
        assert_eq!(
            FolderToggles::atom(Category::Unknown),
            Some(FolderToggles::DIFFERENCES)
        );
        assert_eq!(
            FolderToggles::atom(Category::Error),
            Some(FolderToggles::DIFFERENCES)
        );
        assert_eq!(FolderToggles::atom(Category::Filtered), None);
        assert!(!FolderToggles::all().accepts(Category::Filtered));
        assert!(FolderToggles::DIFFERENCES.accepts(Category::Error));
        assert!(!FolderToggles::SAME.accepts(Category::Different));
    }

    #[test]
    fn parsing() {
        use FolderDisplayFilter as F;
        let cases = [
            ("all", F::All),
            ("differences", F::Differences),
            ("display-mismatches", F::Differences),
            ("same", F::Same),
            ("display-matches", F::Same),
            ("no-orphans", F::NoOrphans),
            ("differences-no-orphans", F::DifferencesNoOrphans),
            ("display-mismatches-no-orphans", F::DifferencesNoOrphans),
            ("orphans", F::Orphans),
            ("left-newer", F::LeftNewer),
            ("right-newer", F::RightNewer),
            ("left-newer-orphans", F::LeftNewerAndLeftOrphans),
            ("display-right-newer-orphans", F::RightNewerAndRightOrphans),
            ("left-orphans", F::LeftOrphans),
            ("Display-Right-Orphans", F::RightOrphans),
            ("none", F::None),
            ("differences_no_orphans", F::DifferencesNoOrphans),
            ("left_newer_and_left_orphans", F::LeftNewerAndLeftOrphans),
        ];
        for (text, want) in cases {
            assert_eq!(F::parse(text).unwrap(), want, "{text}");
        }
        assert!(matches!(
            F::parse("context"),
            Err(FolderError::InvalidRules(_))
        ));
        assert!(matches!(F::parse(""), Err(FolderError::InvalidRules(_))));
        assert_eq!(
            serde_json::to_string(&F::DifferencesNoOrphans).unwrap(),
            "\"differences_no_orphans\""
        );
        assert_eq!(
            serde_json::to_string(&F::Toggles { bits: 5 }).unwrap(),
            r#"{"toggles":{"bits":5}}"#
        );
        assert_eq!(FolderMode::default(), FolderMode::CompareStructure);
        assert_eq!(
            serde_json::to_string(&FolderMode::OnlyFiles).unwrap(),
            "\"only_files\""
        );
    }
}
