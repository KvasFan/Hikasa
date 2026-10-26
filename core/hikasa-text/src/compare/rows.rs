//! Rows (§2.17.1): [`RowIdx`], [`Row`], [`RowState`], [`Importance`] and the packed row storage (12 bytes per
//! row, O(1) random access for virtualised views, gate G5).

use std::ops::Range;

use hikasa_core::cancel::Cancelled;
use hikasa_diff::{Run, RunKind};
use serde::{Deserialize, Serialize};

use crate::document::{LineCursor, LineEnding, LineIdx, TextDocument};
use crate::util::{PhaseProgress, par_chunks_mut};

/// Row index (0-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RowIdx(pub u32);

/// Row state (TC-037 vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowState {
    /// Both lines present and identical after rules.
    Same,
    /// Both present, different (importance says whether it matters).
    Changed,
    /// Left orphan.
    LeftOnly,
    /// Right orphan.
    RightOnly,
}

impl RowState {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            RowState::Same => "same",
            RowState::Changed => "changed",
            RowState::LeftOnly => "left_only",
            RowState::RightOnly => "right_only",
        }
    }
}

/// Importance of a row. INV-ROW-1: `state == Same` ⇔ `importance == None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Importance {
    /// No difference (Same rows).
    None,
    /// Only unimportant differences.
    Unimportant,
    /// At least one important difference.
    Important,
}

impl Importance {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Importance::None => "none",
            Importance::Unimportant => "unimportant",
            Importance::Important => "important",
        }
    }
}

/// One alignment row (O(1) random access).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Row {
    /// Left line (None for RightOnly).
    pub left: Option<LineIdx>,
    /// Right line (None for LeftOnly).
    pub right: Option<LineIdx>,
    /// State.
    pub state: RowState,
    /// Importance.
    pub importance: Importance,
}

/// "No line" in a packed row.
const NO_LINE: u32 = u32::MAX;

/// A row in 12 bytes: the two line indices (`u32::MAX` = none) and the state/importance code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub(crate) struct PackedRow {
    left: u32,
    right: u32,
    meta: u32,
}

const fn state_code(state: RowState) -> u32 {
    match state {
        RowState::Same => 0,
        RowState::Changed => 1,
        RowState::LeftOnly => 2,
        RowState::RightOnly => 3,
    }
}

const fn importance_code(importance: Importance) -> u32 {
    match importance {
        Importance::None => 0,
        Importance::Unimportant => 1,
        Importance::Important => 2,
    }
}

impl PackedRow {
    #[inline]
    pub(crate) fn new(
        left: Option<u32>,
        right: Option<u32>,
        state: RowState,
        importance: Importance,
    ) -> PackedRow {
        PackedRow {
            left: left.unwrap_or(NO_LINE),
            right: right.unwrap_or(NO_LINE),
            meta: state_code(state) | (importance_code(importance) << 2),
        }
    }

    #[inline]
    pub(crate) fn left(self) -> Option<u32> {
        (self.left != NO_LINE).then_some(self.left)
    }

    #[inline]
    pub(crate) fn right(self) -> Option<u32> {
        (self.right != NO_LINE).then_some(self.right)
    }

    #[inline]
    pub(crate) fn state(self) -> RowState {
        match self.meta & 3 {
            0 => RowState::Same,
            1 => RowState::Changed,
            2 => RowState::LeftOnly,
            _ => RowState::RightOnly,
        }
    }

    #[inline]
    pub(crate) fn importance(self) -> Importance {
        match (self.meta >> 2) & 3 {
            0 => Importance::None,
            1 => Importance::Unimportant,
            _ => Importance::Important,
        }
    }

    #[inline]
    pub(crate) fn set(&mut self, state: RowState, importance: Importance) {
        self.meta = state_code(state) | (importance_code(importance) << 2);
    }

    #[inline]
    pub(crate) fn unpack(self) -> Row {
        Row {
            left: self.left().map(LineIdx),
            right: self.right().map(LineIdx),
            state: self.state(),
            importance: self.importance(),
        }
    }
}

/// Rows between two cancellation checks and progress reports (§5.3: every 4096 rows built).
pub(crate) const ROW_CHUNK: usize = 4096;

/// Expands `runs` into rows (§2.17.1). Equal pairs start as `Same` (the raw-text check runs afterwards);
/// orphans are Important when `orphans_important` or their line is not trivial.
pub(crate) fn expand(
    runs: &[Run],
    left_trivial: &dyn Fn(u32) -> bool,
    right_trivial: &dyn Fn(u32) -> bool,
    orphans_important: bool,
    check: &dyn Fn() -> Result<(), Cancelled>,
) -> Result<Vec<PackedRow>, Cancelled> {
    let total: usize = runs.iter().map(|r| r.rows() as usize).sum();
    let mut rows = Vec::with_capacity(total);
    let orphan = |trivial: bool| {
        if orphans_important || !trivial {
            Importance::Important
        } else {
            Importance::Unimportant
        }
    };
    for run in runs {
        for k in 0..run.rows() {
            if rows.len() % ROW_CHUNK == 0 {
                check()?;
            }
            let (l, r) = (run.left.start + k, run.right.start + k);
            rows.push(match run.kind {
                RunKind::Equal => {
                    PackedRow::new(Some(l), Some(r), RowState::Same, Importance::None)
                }
                RunKind::Changed => {
                    PackedRow::new(Some(l), Some(r), RowState::Changed, Importance::Important)
                }
                RunKind::Delete => {
                    PackedRow::new(Some(l), None, RowState::LeftOnly, orphan(left_trivial(l)))
                }
                RunKind::Insert => {
                    PackedRow::new(None, Some(r), RowState::RightOnly, orphan(right_trivial(r)))
                }
            });
        }
    }
    Ok(rows)
}

/// Lines at least this long are compared piece by piece from the ropes (no copy; cancellation every 1 MiB).
pub(crate) const LONG_LINE_BYTES: usize = 64 * 1024;

/// Bytes compared between two cancellation checks (§5.3).
const COMPARE_STEP: usize = 1 << 20;

/// Equal byte strings, compared in 1 MiB steps with `check` between them.
pub(crate) fn bytes_equal(
    a: &[u8],
    b: &[u8],
    check: &dyn Fn() -> Result<(), Cancelled>,
) -> Result<bool, Cancelled> {
    if a.len() != b.len() {
        return Ok(false);
    }
    if a.len() <= COMPARE_STEP {
        return Ok(a == b);
    }
    for (x, y) in a.chunks(COMPARE_STEP).zip(b.chunks(COMPARE_STEP)) {
        check()?;
        if x != y {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The texts of left line `l` and right line `r` are equal: through the cursors for ordinary lines, piece by
/// piece from the ropes for long ones (never copying a multi-megabyte line in one step).
pub(crate) fn line_texts_equal(
    left: &TextDocument,
    l: u32,
    right: &TextDocument,
    r: u32,
    cursors: (&mut LineCursor<'_>, &mut LineCursor<'_>),
    check: &dyn Fn() -> Result<(), Cancelled>,
) -> Result<bool, Cancelled> {
    let len = left.line_len(l);
    if len != right.line_len(r) {
        return Ok(false);
    }
    if len < LONG_LINE_BYTES {
        let (lc, rc) = cursors;
        return Ok(lc.line(l) == rc.line(r));
    }
    let (mut a, mut b) = (left.line_chunks(l), right.line_chunks(r));
    let (mut x, mut y): (&[u8], &[u8]) = (&[], &[]);
    let mut since = 0usize;
    loop {
        if x.is_empty() {
            match a.next() {
                Some(c) => x = c.as_bytes(),
                None => return Ok(y.is_empty() && b.next().is_none()),
            }
            continue;
        }
        if y.is_empty() {
            match b.next() {
                Some(c) => y = c.as_bytes(),
                None => return Ok(false),
            }
            continue;
        }
        let n = x.len().min(y.len());
        if x[..n] != y[..n] {
            return Ok(false);
        }
        x = &x[n..];
        y = &y[n..];
        since += n;
        if since >= COMPARE_STEP {
            since = 0;
            check()?;
        }
    }
}

/// Raw equality of two lines for row classification (§2.17.1): equal text, both or neither ended by an
/// artificial break of the characters-per-line limit (TC-003), and equal endings when endings are compared
/// (TC-022).
#[inline]
pub(crate) fn raw_equal(
    left: &str,
    right: &str,
    left_eol: LineEnding,
    right_eol: LineEnding,
    compare_eol: bool,
) -> bool {
    left == right && endings_equal(left_eol, right_eol, compare_eol)
}

/// The ending part of [`raw_equal`].
#[inline]
pub(crate) fn endings_equal(
    left_eol: LineEnding,
    right_eol: LineEnding,
    compare_eol: bool,
) -> bool {
    (left_eol == LineEnding::Artificial) == (right_eol == LineEnding::Artificial)
        && (!compare_eol || left_eol == right_eol)
}

/// The raw-text checks of the two-sided rows (§2.17.1 as amended in wave 3): a `Same` row (Equal pair) whose
/// lines are not raw-equal becomes `Changed`/`Unimportant`; a `Changed`/`Important` row (Changed pair) whose
/// lines are raw-equal becomes `Changed`/`Unimportant`, because byte-identical lines never show as important
/// (their keys can differ only through the tokenizer's start state). Runs on `pool` in chunks of
/// [`ROW_CHUNK`] rows, each walking both ropes with sequential cursors.
pub(crate) fn check_raw_equality(
    rows: &mut [PackedRow],
    left: &TextDocument,
    right: &TextDocument,
    compare_eol: bool,
    pool: Option<&rayon::ThreadPool>,
    check: &(dyn Fn() -> Result<(), Cancelled> + Sync),
    progress: &PhaseProgress<'_>,
) -> Result<(), Cancelled> {
    par_chunks_mut(pool, rows, ROW_CHUNK, |_, chunk| {
        check()?;
        let mut lc = left.cursor();
        let mut rc = right.cursor();
        for row in chunk.iter_mut() {
            let (Some(l), Some(r)) = (row.left(), row.right()) else {
                continue;
            };
            let state = row.state();
            if state != RowState::Same && row.importance() != Importance::Important {
                continue;
            }
            let equal = endings_equal(
                left.line_ending(LineIdx(l)),
                right.line_ending(LineIdx(r)),
                compare_eol,
            ) && line_texts_equal(left, l, right, r, (&mut lc, &mut rc), check)?;
            if (state == RowState::Same) != equal {
                row.set(RowState::Changed, Importance::Unimportant);
            }
        }
        progress.add(chunk.len() as u64);
        Ok(())
    })
}

/// Clamps `range` to `0..len`.
pub(crate) fn clamp(range: Range<u32>, len: u32) -> Range<u32> {
    let end = range.end.min(len);
    range.start.min(end)..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_round_trips() {
        assert_eq!(std::mem::size_of::<PackedRow>(), 12);
        for state in [
            RowState::Same,
            RowState::Changed,
            RowState::LeftOnly,
            RowState::RightOnly,
        ] {
            for importance in [
                Importance::None,
                Importance::Unimportant,
                Importance::Important,
            ] {
                let p = PackedRow::new(Some(3), None, state, importance);
                let row = p.unpack();
                assert_eq!(row.state, state);
                assert_eq!(row.importance, importance);
                assert_eq!(row.left, Some(LineIdx(3)));
                assert_eq!(row.right, None);
            }
        }
        assert_eq!(clamp(5..9, 7), 5..7);
        assert_eq!(clamp(9..12, 7), 7..7);
    }

    #[test]
    fn names_match_serde() {
        for s in [
            RowState::Same,
            RowState::Changed,
            RowState::LeftOnly,
            RowState::RightOnly,
        ] {
            assert_eq!(
                serde_json::to_string(&s).unwrap(),
                format!("\"{}\"", s.as_str())
            );
        }
        for i in [
            Importance::None,
            Importance::Unimportant,
            Importance::Important,
        ] {
            assert_eq!(
                serde_json::to_string(&i).unwrap(),
                format!("\"{}\"", i.as_str())
            );
        }
    }

    /// Long lines are compared piecewise from ropes whose chunk boundaries differ; the result equals plain
    /// equality, and the check runs between pieces.
    #[test]
    fn long_lines_compare_piecewise() {
        let long = "é1".repeat(LONG_LINE_BYTES);
        let left = TextDocument::from_text(&format!("x\n{long}\n"));
        let right = TextDocument::from_text(&format!("yyyyyyy\n{long}\n"));
        // Same length, last char different.
        let mut other = long.clone();
        other.pop();
        other.push('z');
        let changed = TextDocument::from_text(&format!("{other}\n"));
        let ok = || Ok(());
        let eq = |a: &TextDocument, l: u32, b: &TextDocument, r: u32| {
            let (mut lc, mut rc) = (a.cursor(), b.cursor());
            line_texts_equal(a, l, b, r, (&mut lc, &mut rc), &ok).unwrap()
        };
        assert!(eq(&left, 1, &right, 1));
        assert!(!eq(&left, 1, &changed, 0));
        assert!(!eq(&left, 0, &right, 0));
        assert!(!eq(&left, 1, &right, 0));
        let calls = std::cell::Cell::new(0);
        let counting = || {
            calls.set(calls.get() + 1);
            Ok(())
        };
        let (mut lc, mut rc) = (left.cursor(), right.cursor());
        let big = "q".repeat(3 << 20);
        let (a, b) = (TextDocument::from_text(&big), TextDocument::from_text(&big));
        assert!(line_texts_equal(&a, 0, &b, 0, (&mut lc, &mut rc), &counting).unwrap());
        assert!(calls.get() >= 2);
        assert!(bytes_equal(big.as_bytes(), big.as_bytes(), &counting).unwrap());
        assert!(!bytes_equal(b"ab", b"ac", &ok).unwrap());
    }

    #[test]
    fn expansion_from_hand_built_runs() {
        let runs = [
            Run {
                kind: RunKind::Equal,
                left: 0..2,
                right: 0..2,
            },
            Run {
                kind: RunKind::Delete,
                left: 2..4,
                right: 2..2,
            },
            Run {
                kind: RunKind::Insert,
                left: 4..4,
                right: 2..3,
            },
            Run {
                kind: RunKind::Changed,
                left: 4..5,
                right: 3..4,
            },
        ];
        let trivial = |l: u32| l == 3;
        let rows = expand(&runs, &trivial, &|_| true, false, &|| Ok(())).unwrap();
        let got: Vec<(Option<u32>, Option<u32>, RowState, Importance)> = rows
            .iter()
            .map(|r| (r.left(), r.right(), r.state(), r.importance()))
            .collect();
        assert_eq!(
            got,
            [
                (Some(0), Some(0), RowState::Same, Importance::None),
                (Some(1), Some(1), RowState::Same, Importance::None),
                (Some(2), None, RowState::LeftOnly, Importance::Important),
                (Some(3), None, RowState::LeftOnly, Importance::Unimportant),
                (None, Some(2), RowState::RightOnly, Importance::Unimportant),
                (Some(4), Some(3), RowState::Changed, Importance::Important),
            ]
        );
        let rows = expand(&runs, &trivial, &|_| true, true, &|| Ok(())).unwrap();
        assert!(
            rows.iter()
                .filter(|r| matches!(r.state(), RowState::LeftOnly | RowState::RightOnly))
                .all(|r| r.importance() == Importance::Important)
        );
    }
}
