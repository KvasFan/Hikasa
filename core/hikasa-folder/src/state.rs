//! Comparison states and display categories (FC-030; algorithms §2.6.2 and §2.6.3; D-48).

use hikasa_core::ContentKind;
use serde::{Deserialize, Serialize};

use crate::quick::DiffFlags;
use crate::time_cmp::TimeRelation;

/// Comparison state (FC-030). Canonical (computed with ignore_unimportant = false).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareState {
    /// Not compared (yet): pending, cancelled, or below a folder whose listing failed on one side.
    Unknown,
    /// No difference.
    Same,
    /// The left file is newer (timestamps differ).
    LeftNewer,
    /// The right file is newer.
    RightNewer,
    /// Different (size, content or metadata), or a folder that contains differences.
    Different,
    /// Present on the left only.
    LeftOrphan,
    /// Present on the right only.
    RightOrphan,
    /// A listing, stat or content test failed.
    Error,
    /// Every present side is hidden by a file filter (FC-054).
    Filtered,
}

impl CompareState {
    /// Every state in declaration order.
    pub const ALL: [CompareState; 9] = [
        CompareState::Unknown,
        CompareState::Same,
        CompareState::LeftNewer,
        CompareState::RightNewer,
        CompareState::Different,
        CompareState::LeftOrphan,
        CompareState::RightOrphan,
        CompareState::Error,
        CompareState::Filtered,
    ];

    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            CompareState::Unknown => "unknown",
            CompareState::Same => "same",
            CompareState::LeftNewer => "left_newer",
            CompareState::RightNewer => "right_newer",
            CompareState::Different => "different",
            CompareState::LeftOrphan => "left_orphan",
            CompareState::RightOrphan => "right_orphan",
            CompareState::Error => "error",
            CompareState::Filtered => "filtered",
        }
    }

    /// The same-named category.
    pub const fn category(self) -> Category {
        match self {
            CompareState::Unknown => Category::Unknown,
            CompareState::Same => Category::Same,
            CompareState::LeftNewer => Category::LeftNewer,
            CompareState::RightNewer => Category::RightNewer,
            CompareState::Different => Category::Different,
            CompareState::LeftOrphan => Category::LeftOrphan,
            CompareState::RightOrphan => Category::RightOrphan,
            CompareState::Error => Category::Error,
            CompareState::Filtered => Category::Filtered,
        }
    }

    /// Left↔Right (orphans and newer swap).
    pub const fn mirrored(self) -> CompareState {
        match self {
            CompareState::LeftNewer => CompareState::RightNewer,
            CompareState::RightNewer => CompareState::LeftNewer,
            CompareState::LeftOrphan => CompareState::RightOrphan,
            CompareState::RightOrphan => CompareState::LeftOrphan,
            CompareState::Unknown
            | CompareState::Same
            | CompareState::Different
            | CompareState::Error
            | CompareState::Filtered => self,
        }
    }
}

/// Display category (state adjusted for ignore-unimportant; §2.6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Same.
    Same,
    /// Left newer.
    LeftNewer,
    /// Right newer.
    RightNewer,
    /// Different.
    Different,
    /// Left only.
    LeftOrphan,
    /// Right only.
    RightOrphan,
    /// Unknown.
    Unknown,
    /// Error.
    Error,
    /// Filtered.
    Filtered,
}

impl Category {
    /// Every category in declaration order (the order of JSON `aggregate` lists, §4.3.3).
    pub const ALL: [Category; 9] = [
        Category::Same,
        Category::LeftNewer,
        Category::RightNewer,
        Category::Different,
        Category::LeftOrphan,
        Category::RightOrphan,
        Category::Unknown,
        Category::Error,
        Category::Filtered,
    ];

    /// Dense index (declaration order).
    pub const fn index(self) -> usize {
        match self {
            Category::Same => 0,
            Category::LeftNewer => 1,
            Category::RightNewer => 2,
            Category::Different => 3,
            Category::LeftOrphan => 4,
            Category::RightOrphan => 5,
            Category::Unknown => 6,
            Category::Error => 7,
            Category::Filtered => 8,
        }
    }

    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Category::Same => "same",
            Category::LeftNewer => "left_newer",
            Category::RightNewer => "right_newer",
            Category::Different => "different",
            Category::LeftOrphan => "left_orphan",
            Category::RightOrphan => "right_orphan",
            Category::Unknown => "unknown",
            Category::Error => "error",
            Category::Filtered => "filtered",
        }
    }

    /// True for the categories counted by `CategoryCounts::differences`: left/right newer, different, left/right
    /// orphan.
    pub const fn is_difference(self) -> bool {
        match self {
            Category::LeftNewer
            | Category::RightNewer
            | Category::Different
            | Category::LeftOrphan
            | Category::RightOrphan => true,
            Category::Same | Category::Unknown | Category::Error | Category::Filtered => false,
        }
    }

    /// The same-named state.
    pub const fn state(self) -> CompareState {
        match self {
            Category::Same => CompareState::Same,
            Category::LeftNewer => CompareState::LeftNewer,
            Category::RightNewer => CompareState::RightNewer,
            Category::Different => CompareState::Different,
            Category::LeftOrphan => CompareState::LeftOrphan,
            Category::RightOrphan => CompareState::RightOrphan,
            Category::Unknown => CompareState::Unknown,
            Category::Error => CompareState::Error,
            Category::Filtered => CompareState::Filtered,
        }
    }
}

/// The quick-test differences a content match may cancel (D-48).
pub const OVERRIDABLE: DiffFlags = DiffFlags::SIZE.union(DiffFlags::TIME);

/// `base(d, t)` of §2.6.2: no difference → Same; TIME among the differences → the newer side; else Different.
pub fn base_state(diffs: DiffFlags, time: Option<TimeRelation>) -> CompareState {
    if diffs.is_empty() {
        CompareState::Same
    } else if diffs.contains(DiffFlags::TIME) {
        newer(time)
    } else {
        CompareState::Different
    }
}

/// `newer(t)`: the newer side; Different when the relation is unknown or `Same` (defensive: TIME implies a
/// newer side).
fn newer(time: Option<TimeRelation>) -> CompareState {
    match time {
        Some(TimeRelation::LeftNewer) => CompareState::LeftNewer,
        Some(TimeRelation::RightNewer) => CompareState::RightNewer,
        Some(TimeRelation::Same) | None => CompareState::Different,
    }
}

/// State of a pair of present, stat'ed, same-kind entries from its quick tests and content result (§2.6.2).
/// `content` is `None` when no content test ran. Returns the state and the stored diff flags (which keep every
/// flag found, plus `CONTENT` when the content differs).
pub fn pair_state(
    diffs: DiffFlags,
    time: Option<TimeRelation>,
    content: Option<ContentKind>,
    override_quick: bool,
) -> (CompareState, DiffFlags) {
    match content {
        None => (base_state(diffs, time), diffs),
        Some(ContentKind::Error) => (CompareState::Error, diffs),
        Some(ContentKind::BinarySame | ContentKind::RulesSame) => {
            let state = if override_quick {
                base_state(diffs.difference(OVERRIDABLE), None)
            } else {
                base_state(diffs, time)
            };
            (state, diffs)
        }
        Some(
            ContentKind::UnimportantOnly
            | ContentKind::ImportantDifferences
            | ContentKind::BinaryDifferent,
        ) => {
            let diffs = diffs | DiffFlags::CONTENT;
            let state = if diffs.contains(DiffFlags::TIME) {
                newer(time)
            } else {
                CompareState::Different
            };
            (state, diffs)
        }
    }
}

/// `state_if_rules_same(n)` of §2.6.3: the state the pair would have if its content were rules-same.
pub fn state_if_rules_same(
    diffs: DiffFlags,
    time: Option<TimeRelation>,
    override_quick: bool,
) -> CompareState {
    let without_content = diffs.difference(DiffFlags::CONTENT);
    if override_quick {
        base_state(without_content.difference(OVERRIDABLE), None)
    } else {
        base_state(without_content, time)
    }
}

/// Display category of a file-like node (§2.6.3): Filtered stays Filtered; with ignore-unimportant an
/// `UnimportantOnly` content result counts as rules-same; otherwise the state's category.
pub fn file_category(
    state: CompareState,
    diffs: DiffFlags,
    time: Option<TimeRelation>,
    content: Option<ContentKind>,
    override_quick: bool,
    ignore_unimportant: bool,
) -> Category {
    if state == CompareState::Filtered {
        return Category::Filtered;
    }
    if ignore_unimportant
        && content == Some(ContentKind::UnimportantOnly)
        && state != CompareState::Error
        && state != CompareState::Unknown
    {
        return state_if_rules_same(diffs, time, override_quick).category();
    }
    state.category()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ContentKind as K;
    use TimeRelation as T;

    const D: DiffFlags = DiffFlags::empty();

    /// §2.6.2 table, rows 1–15 (state, plus the ignore-unimportant category of rows 9, 14, 15).
    #[test]
    fn state_table() {
        type Row = (DiffFlags, Option<T>, Option<K>, bool, CompareState);
        let rows: [Row; 15] = [
            (D, Some(T::Same), None, true, CompareState::Same),
            (
                DiffFlags::TIME,
                Some(T::LeftNewer),
                None,
                true,
                CompareState::LeftNewer,
            ),
            (
                DiffFlags::SIZE,
                Some(T::Same),
                None,
                true,
                CompareState::Different,
            ),
            (
                DiffFlags::TIME,
                Some(T::LeftNewer),
                Some(K::BinarySame),
                true,
                CompareState::Same,
            ),
            (
                DiffFlags::TIME,
                Some(T::LeftNewer),
                Some(K::BinarySame),
                false,
                CompareState::LeftNewer,
            ),
            (
                D,
                Some(T::Same),
                Some(K::BinaryDifferent),
                true,
                CompareState::Different,
            ),
            (
                DiffFlags::TIME,
                Some(T::RightNewer),
                Some(K::ImportantDifferences),
                true,
                CompareState::RightNewer,
            ),
            (
                DiffFlags::SIZE,
                Some(T::Same),
                Some(K::RulesSame),
                true,
                CompareState::Same,
            ),
            (
                D,
                Some(T::Same),
                Some(K::UnimportantOnly),
                true,
                CompareState::Different,
            ),
            (
                DiffFlags::SIZE | DiffFlags::TIME,
                Some(T::LeftNewer),
                Some(K::Error),
                true,
                CompareState::Error,
            ),
            (
                DiffFlags::TIME | DiffFlags::ATTRIBUTES,
                Some(T::LeftNewer),
                Some(K::BinarySame),
                true,
                CompareState::Different,
            ),
            (
                DiffFlags::NAME_CASE,
                None,
                Some(K::RulesSame),
                true,
                CompareState::Different,
            ),
            (
                DiffFlags::SIZE | DiffFlags::PERMISSIONS,
                None,
                Some(K::RulesSame),
                true,
                CompareState::Different,
            ),
            (
                DiffFlags::TIME,
                Some(T::LeftNewer),
                Some(K::UnimportantOnly),
                true,
                CompareState::LeftNewer,
            ),
            (
                DiffFlags::TIME,
                Some(T::LeftNewer),
                Some(K::UnimportantOnly),
                false,
                CompareState::LeftNewer,
            ),
        ];
        for (i, (diffs, time, content, ovr, want)) in rows.iter().enumerate() {
            let (state, stored) = pair_state(*diffs, *time, *content, *ovr);
            assert_eq!(state, *want, "row {}", i + 1);
            assert!(stored.contains(*diffs), "row {}: diffs kept", i + 1);
            let differs = matches!(
                content,
                Some(K::UnimportantOnly | K::ImportantDifferences | K::BinaryDifferent)
            );
            assert_eq!(
                stored.contains(DiffFlags::CONTENT),
                differs,
                "row {}",
                i + 1
            );
        }
        // Row 4 keeps TIME for display.
        assert_eq!(
            pair_state(
                DiffFlags::TIME,
                Some(T::LeftNewer),
                Some(K::BinarySame),
                true
            )
            .1,
            DiffFlags::TIME
        );
        // Categories under ignore-unimportant (rows 9, 14, 15).
        let cat = |diffs, time, ovr| {
            let (state, stored) = pair_state(diffs, time, Some(K::UnimportantOnly), ovr);
            (
                file_category(state, stored, time, Some(K::UnimportantOnly), ovr, false),
                file_category(state, stored, time, Some(K::UnimportantOnly), ovr, true),
            )
        };
        assert_eq!(
            cat(D, Some(T::Same), true),
            (Category::Different, Category::Same)
        );
        assert_eq!(
            cat(DiffFlags::TIME, Some(T::LeftNewer), true),
            (Category::LeftNewer, Category::Same)
        );
        assert_eq!(
            cat(DiffFlags::TIME, Some(T::LeftNewer), false),
            (Category::LeftNewer, Category::LeftNewer)
        );
    }

    /// The category function (§2.6.3) for every state.
    #[test]
    fn category_table() {
        for state in CompareState::ALL {
            for iu in [false, true] {
                let cat = file_category(state, D, None, None, true, iu);
                assert_eq!(cat, state.category(), "{state:?} iu={iu}");
                assert_eq!(cat.state(), state);
            }
        }
        // Filtered stays Filtered even with an unimportant-only result.
        assert_eq!(
            file_category(
                CompareState::Filtered,
                D,
                None,
                Some(K::UnimportantOnly),
                true,
                true
            ),
            Category::Filtered
        );
        // SIZE + UnimportantOnly: override on → Same, off → Different.
        let (state, stored) = pair_state(DiffFlags::SIZE, None, Some(K::UnimportantOnly), true);
        assert_eq!(state, CompareState::Different);
        assert_eq!(
            file_category(state, stored, None, Some(K::UnimportantOnly), true, true),
            Category::Same
        );
        let (state, stored) = pair_state(DiffFlags::SIZE, None, Some(K::UnimportantOnly), false);
        assert_eq!(
            file_category(state, stored, None, Some(K::UnimportantOnly), false, true),
            Category::Different
        );
        // ATTRIBUTES + UnimportantOnly stays Different (D-48).
        let (state, stored) =
            pair_state(DiffFlags::ATTRIBUTES, None, Some(K::UnimportantOnly), true);
        assert_eq!(
            file_category(state, stored, None, Some(K::UnimportantOnly), true, true),
            Category::Different
        );
        for c in Category::ALL {
            assert_eq!(Category::ALL[c.index()], c);
            assert_eq!(c.state().category(), c);
            assert_eq!(c.as_str(), c.state().as_str());
        }
        assert_eq!(
            CompareState::LeftOrphan.mirrored(),
            CompareState::RightOrphan
        );
        assert_eq!(CompareState::Same.mirrored(), CompareState::Same);
    }
}
