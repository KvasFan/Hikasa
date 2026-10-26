//! Text display filters and context (TC-040, TC-035, RPT-010; §2.18): [`TextDisplayFilter`], [`TextToggles`],
//! [`ViewOptions`], [`FilteredView`] and [`ViewItem`].
//!
//! ```text
//! cat(row, iu) = Same          if state == Same || (iu && importance == Unimportant)
//!              = LeftOrphans   if state == LeftOnly
//!              = RightOrphans  if state == RightOnly
//!              = Differences   otherwise (Changed)
//! visible(row) = All → true; None → false; Same → cat == Same; Differences → cat ≠ Same;
//!                Context{N} → some row d with cat(d) ≠ Same and |row − d| ≤ N; Toggles{bits} → bits ∋ cat
//! visible'(row) = visible(row) || row ∈ ⋃ expanded
//! items: every visible' row is a Row item; every maximal run of hidden rows is one Gap item
//! ```
//!
//! The view is built from the comparison's class runs (maximal runs of equal non-Same rows), so building it
//! costs O(runs + expanded ranges), not O(rows); `get` is a binary search over the segments.

use std::ops::Range;

use hikasa_core::error::InvariantViolation;
use serde::{Deserialize, Serialize};

use crate::compare::rows::{Importance, RowIdx, RowState};
use crate::compare::sections::ClassRun;

bitflags::bitflags! {
    /// Atomic text display categories (toggle mode).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct TextToggles: u8 {
        /// Same rows (and unimportant ones under ignore-unimportant).
        const SAME = 1;
        /// Left orphans.
        const LEFT_ORPHANS = 2;
        /// Right orphans.
        const RIGHT_ORPHANS = 4;
        /// Changed rows.
        const DIFFERENCES = 8;
    }
}

/// Display filter (TC-040).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TextDisplayFilter {
    /// Show All.
    All,
    /// Show Differences.
    Differences,
    /// Show Same.
    Same,
    /// Show None.
    None,
    /// Show Context (differences ± `lines`).
    Context {
        /// Context lines around every difference.
        lines: u32,
    },
    /// Toggle mode (`TextToggles` bits).
    Toggles {
        /// [`TextToggles`] bits; unknown bits are ignored.
        bits: u8,
    },
}

impl TextDisplayFilter {
    /// The atomic categories the filter can show, in full or in part (TC-040: the check state of the toggle
    /// commands, and the set toggle mode starts from). Additive API (wave 6b, §1.5.7).
    ///
    /// | Filter | Toggles |
    /// |---|---|
    /// | All | every category |
    /// | Differences, `Context { lines: 0 }` | left orphans, right orphans, differences |
    /// | `Context { lines: 1.. }` | every category (the context rows are Same rows) |
    /// | Same | Same |
    /// | None | none |
    /// | `Toggles { bits }` | `bits` (unknown bits dropped) |
    pub fn toggles(self) -> TextToggles {
        let diffs =
            TextToggles::LEFT_ORPHANS | TextToggles::RIGHT_ORPHANS | TextToggles::DIFFERENCES;
        match self {
            TextDisplayFilter::All => TextToggles::all(),
            TextDisplayFilter::Differences | TextDisplayFilter::Context { lines: 0 } => diffs,
            TextDisplayFilter::Context { .. } => TextToggles::all(),
            TextDisplayFilter::Same => TextToggles::SAME,
            TextDisplayFilter::None => TextToggles::empty(),
            TextDisplayFilter::Toggles { bits } => TextToggles::from_bits_truncate(bits),
        }
    }

    /// The filter after the user flips the categories `bits` in toggle mode (TC-040): `Toggles` with
    /// [`toggles`](Self::toggles) `^ bits`. Additive API (wave 6b, §1.5.7).
    ///
    /// Flipping a category never hides a row of another category that the filter showed: from Show Context,
    /// whose context rows are Same rows, switching a difference category off keeps every Same row, and only
    /// switching Same off removes the context rows.
    pub fn toggled(self, bits: TextToggles) -> TextDisplayFilter {
        TextDisplayFilter::Toggles {
            bits: (self.toggles() ^ bits).bits(),
        }
    }
}

/// View options.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ViewOptions {
    /// Filter.
    pub filter: TextDisplayFilter,
    /// TC-024: unimportant counts as same for display, navigation, counts and reports.
    pub ignore_unimportant: bool,
    /// Gap bars the user expanded (TC-035), as row ranges: rows inside them are visible whatever the filter.
    /// Sorted or not, overlapping or not (normalised on use). Default empty; reports leave it empty.
    #[serde(default)]
    pub expanded: Vec<Range<u32>>,
}

impl Default for ViewOptions {
    /// Show All, ignore-unimportant off, nothing expanded.
    fn default() -> ViewOptions {
        ViewOptions {
            filter: TextDisplayFilter::All,
            ignore_unimportant: false,
            expanded: Vec::new(),
        }
    }
}

/// A visible item: a row, or a gap of hidden rows ("N FILTERED LINES" / "N lines hidden").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewItem {
    /// A visible row.
    Row {
        /// The row.
        row: RowIdx,
    },
    /// Hidden rows `[start, end)`.
    Gap {
        /// The hidden rows.
        rows: Range<u32>,
    },
}

/// One segment: a maximal run of visible rows, or one gap.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    rows: Range<u32>,
    visible: bool,
    /// Visible position of the segment's first item.
    pos: u32,
}

/// Random-access filtered view: O(log k) `get` over k segments (INV-VIEW-1..3; < 200 ns: gate G5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredView {
    segments: Vec<Segment>,
    len: u32,
    hidden: u64,
    rows: u32,
}

/// The display category of a class run (`None` = Same).
fn run_category(run: &ClassRun, ignore_unimportant: bool) -> Option<TextToggles> {
    if ignore_unimportant && run.importance == Importance::Unimportant {
        return None;
    }
    Some(match run.state {
        RowState::Same => return None,
        RowState::LeftOnly => TextToggles::LEFT_ORPHANS,
        RowState::RightOnly => TextToggles::RIGHT_ORPHANS,
        RowState::Changed => TextToggles::DIFFERENCES,
    })
}

/// Sorts and merges overlapping or touching ranges (empty ones dropped), clamped to `0..rows`.
fn normalise(mut ranges: Vec<Range<u32>>, rows: u32) -> Vec<Range<u32>> {
    ranges.retain_mut(|r| {
        r.end = r.end.min(rows);
        r.start < r.end
    });
    ranges.sort_unstable_by_key(|r| (r.start, r.end));
    let mut out: Vec<Range<u32>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

/// The complement of sorted, disjoint `ranges` in `0..rows`.
fn complement(ranges: &[Range<u32>], rows: u32) -> Vec<Range<u32>> {
    let mut out = Vec::with_capacity(ranges.len() + 1);
    let mut at = 0u32;
    for r in ranges {
        if r.start > at {
            out.push(at..r.start);
        }
        at = at.max(r.end);
    }
    if at < rows {
        out.push(at..rows);
    }
    out
}

impl FilteredView {
    /// Builds the view of `rows` rows whose non-Same rows are `runs` (§2.18).
    pub(crate) fn build(rows: u32, runs: &[ClassRun], opts: &ViewOptions) -> FilteredView {
        let iu = opts.ignore_unimportant;
        let of = |cats: TextToggles| -> Vec<Range<u32>> {
            runs.iter()
                .filter(|r| run_category(r, iu).is_some_and(|c| cats.contains(c)))
                .map(|r| r.start..r.end)
                .collect()
        };
        let diff_cats =
            TextToggles::LEFT_ORPHANS | TextToggles::RIGHT_ORPHANS | TextToggles::DIFFERENCES;
        let mut visible: Vec<Range<u32>> = match opts.filter {
            TextDisplayFilter::All => std::iter::once(0..rows).collect(),
            TextDisplayFilter::None => Vec::new(),
            TextDisplayFilter::Differences => of(diff_cats),
            TextDisplayFilter::Same => complement(&normalise(of(diff_cats), rows), rows),
            TextDisplayFilter::Context { lines } => of(diff_cats)
                .into_iter()
                .map(|r| r.start.saturating_sub(lines)..r.end.saturating_add(lines).min(rows))
                .collect(),
            TextDisplayFilter::Toggles { bits } => {
                let bits = TextToggles::from_bits_truncate(bits);
                let mut v = of(bits & diff_cats);
                if bits.contains(TextToggles::SAME) {
                    v.extend(complement(&normalise(of(diff_cats), rows), rows));
                }
                v
            }
        };
        visible.extend(opts.expanded.iter().cloned());
        let visible = normalise(visible, rows);
        let mut segments = Vec::with_capacity(visible.len() * 2 + 1);
        let mut pos = 0u32;
        let mut at = 0u32;
        let mut hidden = 0u64;
        for r in visible {
            if r.start > at {
                segments.push(Segment {
                    rows: at..r.start,
                    visible: false,
                    pos,
                });
                hidden += u64::from(r.start - at);
                pos += 1;
            }
            segments.push(Segment {
                rows: r.clone(),
                visible: true,
                pos,
            });
            pos += r.end - r.start;
            at = r.end;
        }
        if at < rows {
            segments.push(Segment {
                rows: at..rows,
                visible: false,
                pos,
            });
            hidden += u64::from(rows - at);
            pos += 1;
        }
        FilteredView {
            segments,
            len: pos,
            hidden,
            rows,
        }
    }

    /// Number of items (rows + gaps).
    pub fn len(&self) -> u32 {
        self.len
    }

    /// True when no items.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Item at visible position `pos`.
    #[inline]
    pub fn get(&self, pos: u32) -> Option<ViewItem> {
        if pos >= self.len {
            return None;
        }
        let k = self
            .segments
            .partition_point(|s| s.pos <= pos)
            .checked_sub(1)?;
        let seg = self.segments.get(k)?;
        Some(if seg.visible {
            ViewItem::Row {
                row: RowIdx(seg.rows.start + (pos - seg.pos)),
            }
        } else {
            ViewItem::Gap {
                rows: seg.rows.clone(),
            }
        })
    }

    /// All items in order.
    pub fn iter(&self) -> impl Iterator<Item = ViewItem> + '_ {
        self.segments.iter().flat_map(|seg| {
            let (visible, rows) = (seg.visible, seg.rows.clone());
            let gap = (!visible).then(|| ViewItem::Gap { rows: rows.clone() });
            let row_items = rows
                .clone()
                .filter(move |_| visible)
                .map(|r| ViewItem::Row { row: RowIdx(r) });
            gap.into_iter().chain(row_items)
        })
    }

    /// Visible position of `row`: `Ok(pos)` when visible, `Err(pos)` of the gap item that hides it. A row
    /// beyond the last row gives `Err(len())`.
    pub fn position_of(&self, row: RowIdx) -> Result<u32, u32> {
        let Some(k) = self
            .segments
            .partition_point(|s| s.rows.start <= row.0)
            .checked_sub(1)
        else {
            return Err(self.len);
        };
        match self.segments.get(k) {
            Some(seg) if row.0 < seg.rows.end => {
                if seg.visible {
                    Ok(seg.pos + (row.0 - seg.rows.start))
                } else {
                    Err(seg.pos)
                }
            }
            _ => Err(self.len),
        }
    }

    /// Number of hidden rows.
    pub fn hidden_rows(&self) -> u64 {
        self.hidden
    }

    /// Visible row ranges in order (the maximal runs of visible rows; report hunks). Additive (wave 3).
    pub fn visible_runs(&self) -> impl Iterator<Item = Range<u32>> + '_ {
        self.segments
            .iter()
            .filter(|s| s.visible)
            .map(|s| s.rows.clone())
    }

    /// Checks INV-VIEW-1 (the items cover every row in order without overlap) and INV-VIEW-2 (no two gaps are
    /// adjacent). Additive (T4: every result model can re-check its invariants).
    pub fn check_invariants(&self) -> Result<(), InvariantViolation> {
        let mut at = 0u32;
        let mut pos = 0u32;
        let mut previous_gap = false;
        for seg in &self.segments {
            if seg.rows.start != at || seg.rows.start >= seg.rows.end || seg.pos != pos {
                return Err(InvariantViolation {
                    id: "INV-VIEW-1",
                    detail: format!("segment {:?} at row {at}, position {pos}", seg),
                });
            }
            if !seg.visible && previous_gap {
                return Err(InvariantViolation {
                    id: "INV-VIEW-2",
                    detail: format!("adjacent gaps before row {}", seg.rows.start),
                });
            }
            previous_gap = !seg.visible;
            pos += if seg.visible {
                seg.rows.end - seg.rows.start
            } else {
                1
            };
            at = seg.rows.end;
        }
        if at != self.rows || pos != self.len {
            return Err(InvariantViolation {
                id: "INV-VIEW-1",
                detail: format!(
                    "items end at row {at} of {}, {pos} of {} items",
                    self.rows, self.len
                ),
            });
        }
        Ok(())
    }
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

    /// Rows: 0..3 same, 3 changed (unimportant), 4..6 same, 6 left-only (important), 7 right-only
    /// (important), 8..12 same.
    fn runs() -> Vec<ClassRun> {
        vec![
            run(3, 4, RowState::Changed, Importance::Unimportant),
            run(6, 7, RowState::LeftOnly, Importance::Important),
            run(7, 8, RowState::RightOnly, Importance::Important),
        ]
    }

    fn visible(filter: TextDisplayFilter, iu: bool, expanded: Vec<Range<u32>>) -> Vec<u32> {
        let view = FilteredView::build(
            12,
            &runs(),
            &ViewOptions {
                filter,
                ignore_unimportant: iu,
                expanded,
            },
        );
        view.check_invariants().unwrap();
        view.iter()
            .filter_map(|i| match i {
                ViewItem::Row { row } => Some(row.0),
                ViewItem::Gap { .. } => None,
            })
            .collect()
    }

    #[test]
    fn filter_table() {
        use TextDisplayFilter as F;
        assert_eq!(visible(F::All, false, vec![]), (0..12).collect::<Vec<_>>());
        assert!(visible(F::None, false, vec![]).is_empty());
        assert_eq!(visible(F::Differences, false, vec![]), [3, 6, 7]);
        assert_eq!(visible(F::Differences, true, vec![]), [6, 7]);
        assert_eq!(
            visible(F::Same, false, vec![]),
            [0, 1, 2, 4, 5, 8, 9, 10, 11]
        );
        assert_eq!(
            visible(F::Same, true, vec![]),
            [0, 1, 2, 3, 4, 5, 8, 9, 10, 11]
        );
        assert_eq!(
            visible(F::Context { lines: 1 }, false, vec![]),
            [2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(visible(F::Context { lines: 1 }, true, vec![]), [5, 6, 7, 8]);
        assert_eq!(visible(F::Context { lines: 0 }, false, vec![]), [3, 6, 7]);
        let left = TextToggles::LEFT_ORPHANS.bits();
        assert_eq!(visible(F::Toggles { bits: left }, false, vec![]), [6]);
        let same_right = (TextToggles::SAME | TextToggles::RIGHT_ORPHANS).bits();
        assert_eq!(
            visible(F::Toggles { bits: same_right }, false, vec![]),
            [0, 1, 2, 4, 5, 7, 8, 9, 10, 11]
        );
        let diffs = TextToggles::DIFFERENCES.bits() | 0xF0;
        assert_eq!(visible(F::Toggles { bits: diffs }, false, vec![]), [3]);
        assert!(visible(F::Toggles { bits: diffs }, true, vec![]).is_empty());
        // Expanded gap bars, unsorted and overlapping, clamped.
        assert_eq!(
            visible(F::Differences, false, vec![10..20, 0..2, 1..2, 5..5]),
            [0, 1, 3, 6, 7, 10, 11]
        );
    }

    #[test]
    fn toggles_of_every_filter() {
        use TextDisplayFilter as F;
        use TextToggles as T;
        let diffs = T::LEFT_ORPHANS | T::RIGHT_ORPHANS | T::DIFFERENCES;
        assert_eq!(F::All.toggles(), T::all());
        assert_eq!(F::Differences.toggles(), diffs);
        assert_eq!(F::Context { lines: 0 }.toggles(), diffs);
        assert_eq!(F::Context { lines: 3 }.toggles(), T::all());
        assert_eq!(F::Same.toggles(), T::SAME);
        assert_eq!(F::None.toggles(), T::empty());
        assert_eq!(
            F::Toggles { bits: 0xF5 }.toggles(),
            T::SAME | T::RIGHT_ORPHANS
        );
        assert_eq!(
            F::Differences.toggled(T::SAME),
            F::Toggles {
                bits: T::all().bits()
            }
        );
        assert_eq!(
            F::Context { lines: 2 }.toggled(T::SAME),
            F::Toggles { bits: diffs.bits() }
        );
    }

    /// Flipping one category in toggle mode never hides a visible row of another category (the context rows
    /// of Show Context survive every flip but Same), and a flipped category is wholly shown or wholly hidden.
    #[test]
    fn toggling_keeps_the_rows_of_other_categories() {
        use TextDisplayFilter as F;
        let cat = |row: u32, iu: bool| -> TextToggles {
            runs()
                .iter()
                .find(|r| r.start <= row && row < r.end)
                .and_then(|r| run_category(r, iu))
                .unwrap_or(TextToggles::SAME)
        };
        let filters = [
            F::All,
            F::Differences,
            F::Same,
            F::None,
            F::Context { lines: 0 },
            F::Context { lines: 1 },
            F::Context { lines: 5 },
            F::Toggles { bits: 0b1010 },
        ];
        for filter in filters {
            for iu in [false, true] {
                let before = visible(filter, iu, vec![]);
                for bit in TextToggles::all().iter() {
                    let after = visible(filter.toggled(bit), iu, vec![]);
                    for &row in &before {
                        if cat(row, iu) != bit {
                            assert!(
                                after.contains(&row),
                                "{filter:?} flip {bit:?} iu={iu}: row {row} lost"
                            );
                        }
                    }
                    let on = filter.toggled(bit).toggles().contains(bit);
                    for row in 0..12 {
                        if cat(row, iu) == bit {
                            assert_eq!(after.contains(&row), on, "{filter:?} flip {bit:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn items_positions_and_gaps() {
        let view = FilteredView::build(
            12,
            &runs(),
            &ViewOptions {
                filter: TextDisplayFilter::Differences,
                ignore_unimportant: false,
                expanded: vec![],
            },
        );
        let items: Vec<ViewItem> = view.iter().collect();
        assert_eq!(
            items,
            [
                ViewItem::Gap { rows: 0..3 },
                ViewItem::Row { row: RowIdx(3) },
                ViewItem::Gap { rows: 4..6 },
                ViewItem::Row { row: RowIdx(6) },
                ViewItem::Row { row: RowIdx(7) },
                ViewItem::Gap { rows: 8..12 },
            ]
        );
        assert_eq!(view.len(), 6);
        for (pos, item) in items.iter().enumerate() {
            assert_eq!(view.get(pos as u32).as_ref(), Some(item));
        }
        assert_eq!(view.get(6), None);
        assert_eq!(view.position_of(RowIdx(3)), Ok(1));
        assert_eq!(view.position_of(RowIdx(5)), Err(2));
        assert_eq!(view.position_of(RowIdx(7)), Ok(4));
        assert_eq!(view.position_of(RowIdx(11)), Err(5));
        assert_eq!(view.position_of(RowIdx(12)), Err(6));
        assert_eq!(view.hidden_rows(), 9);
        assert_eq!(view.visible_runs().collect::<Vec<_>>(), [3..4, 6..8]);
        let empty = FilteredView::build(0, &[], &ViewOptions::default());
        assert!(empty.is_empty());
        assert!(empty.check_invariants().is_ok());
    }

    #[test]
    fn serde_shapes() {
        assert_eq!(
            serde_json::to_string(&TextDisplayFilter::Context { lines: 3 }).unwrap(),
            r#"{"mode":"context","lines":3}"#
        );
        assert_eq!(
            serde_json::to_string(&TextDisplayFilter::All).unwrap(),
            r#"{"mode":"all"}"#
        );
        assert_eq!(
            serde_json::to_string(&ViewItem::Gap { rows: 0..3 }).unwrap(),
            r#"{"type":"gap","rows":{"start":0,"end":3}}"#
        );
        let opts: ViewOptions = serde_json::from_str(
            r#"{"filter":{"mode":"toggles","bits":9},"ignore_unimportant":true}"#,
        )
        .unwrap();
        assert!(opts.expanded.is_empty());
    }
}
