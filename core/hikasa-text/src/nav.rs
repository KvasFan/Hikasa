//! Navigation data (TC-050, TC-051; D-286; contract §1.10.20): [`step`] per [`BrowseMode`], [`existing_line`],
//! Go To ([`parse_goto`], [`goto`]) and the status bar's [`column_of`]. Pure: the views call it directly.
//!
//! | Mode | Target |
//! |---|---|
//! | `DiffSection` | the next section that is not same under the view's filter, skipping ignored sections and, with *Ignore Unimportant Differences*, unimportant ones |
//! | `Difference` | the next inline difference range of the caret's row after the caret, then of the following non-same rows (a one-sided row is its whole line) |
//! | `Edit` | the next run of `Edited` or `SavedEdit` lines of the focus pane |
//! | `Find` | `find_in_comparison_at`, or `Finder::next` without a comparison |

use std::ops::Range;

use hikasa_core::{Pair, Side};
use serde::{Deserialize, Serialize};

use crate::compare::{Direction, RowIdx, RowState, TextComparison};
use crate::document::LineIdx;
use crate::edit::{EditableDocument, TextPos};
use crate::find::{FindSides, Finder, find_in_comparison_at};
use crate::view::FilteredView;

/// A direction.
pub type Dir = Direction;

/// TC-050's browse selector (`Replacement` and `Taken` are P1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowseMode {
    /// Next/previous difference section (Ctrl+N/P).
    DiffSection,
    /// Next/previous inline difference (Shift+Ctrl+N/P).
    Difference,
    /// Next/previous edited lines (EditNext/Prev).
    Edit,
    /// Next/previous hit of the find panel's query.
    Find,
}

/// What a step looks at.
#[derive(Debug, Clone, Copy)]
pub struct NavContext<'a> {
    /// The comparison (`None` in a Text Edit view).
    pub cmp: Option<&'a TextComparison>,
    /// The panes' documents (a Text Edit view: the left one).
    pub docs: Pair<Option<&'a EditableDocument>>,
    /// The display filter: rows it hides are never targets.
    pub view: Option<&'a FilteredView>,
    /// *Ignore Unimportant Differences*.
    pub ignore_unimportant: bool,
    /// The find panel's query (`Find`).
    pub finder: Option<&'a Finder>,
    /// The sides the find panel searches.
    pub sides: FindSides,
}

/// Where a step starts: the caret's row (with a comparison), side and position in that side's document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NavPos {
    /// The caret's row (`None` without a comparison: it is found from the line when needed).
    pub row: Option<RowIdx>,
    /// The focus side.
    pub side: Side,
    /// The caret.
    pub pos: TextPos,
}

/// Where a step goes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NavTarget {
    /// The row (`None` without a comparison).
    pub row: Option<RowIdx>,
    /// The side the range is on.
    pub side: Side,
    /// The range to select (empty: a place, such as a section with no line on this side).
    pub range: Range<TextPos>,
    /// The step went past an end once (OPT-007's *wrap around*).
    pub wrapped: bool,
}

/// One step (§1.10.20's table); `None` when there is nothing in that direction (the view shows OPT-007's message or
/// beeps). With `wrap` the step continues once from the other end and the target is `wrapped`.
pub fn step(
    mode: BrowseMode,
    ctx: &NavContext<'_>,
    from: NavPos,
    dir: Dir,
    wrap: bool,
) -> Option<NavTarget> {
    let side = if from.side == Side::Right {
        Side::Right
    } else {
        Side::Left
    };
    let from = NavPos { side, ..from };
    match mode {
        BrowseMode::DiffSection => section_step(ctx, from, dir, wrap),
        BrowseMode::Difference => difference_step(ctx, from, dir, wrap),
        BrowseMode::Edit => edit_step(ctx, from, dir, wrap),
        BrowseMode::Find => find_step(ctx, from, dir, wrap),
    }
}

fn doc_of<'a>(ctx: &NavContext<'a>, side: Side) -> Option<&'a EditableDocument> {
    match side {
        Side::Right => ctx.docs.right,
        _ => ctx.docs.left,
    }
}

fn line_of(row: &crate::compare::Row, side: Side) -> Option<LineIdx> {
    match side {
        Side::Right => row.right,
        _ => row.left,
    }
}

/// The caret's row: `from.row`, else the row of its line.
fn row_of(cmp: &TextComparison, from: &NavPos) -> RowIdx {
    from.row
        .or_else(|| cmp.row_for_line(from.side, from.pos.line))
        .unwrap_or(RowIdx(cmp.row_count()))
}

/// True when the view shows `row` (no view: every row).
fn shown(ctx: &NavContext<'_>, row: RowIdx) -> bool {
    ctx.view.is_none_or(|v| v.position_of(row).is_ok())
}

/// The lines `lines` of `side` as a range of positions (empty at the insertion point when there is none).
fn lines_range(ctx: &NavContext<'_>, side: Side, lines: Range<u32>) -> Range<TextPos> {
    let start = TextPos::new(lines.start, 0);
    if lines.is_empty() {
        return start..start;
    }
    let last = lines.end - 1;
    let len = doc_of(ctx, side).map_or(0, |d| d.line_len(LineIdx(last)));
    start..TextPos::new(last, len)
}

fn section_step(ctx: &NavContext<'_>, from: NavPos, dir: Dir, wrap: bool) -> Option<NavTarget> {
    let cmp = ctx.cmp?;
    let row = row_of(cmp, &from);
    let pick = |start: RowIdx, inclusive: bool| -> Option<NavTarget> {
        let mut at = start;
        let mut first = true;
        loop {
            let id = if first && inclusive {
                cmp.next_section_from(at, dir, ctx.ignore_unimportant)
            } else {
                cmp.next_section(at, dir, ctx.ignore_unimportant)
            }?;
            first = false;
            let section = cmp.sections().get(id.0 as usize)?;
            if (section.rows.start..section.rows.end).any(|r| shown(ctx, RowIdx(r))) {
                let lines = match from.side {
                    Side::Right => section.right_lines.clone(),
                    _ => section.left_lines.clone(),
                };
                return Some(NavTarget {
                    row: Some(RowIdx(section.rows.start)),
                    side: from.side,
                    range: lines_range(ctx, from.side, lines),
                    wrapped: false,
                });
            }
            at = RowIdx(section.rows.start);
        }
    };
    if let Some(t) = pick(row, false) {
        return Some(t);
    }
    if !wrap {
        return None;
    }
    let restart = match dir {
        Direction::Forward => pick(RowIdx(0), true),
        Direction::Backward => pick(RowIdx(cmp.row_count()), false),
    };
    restart.map(|t| NavTarget { wrapped: true, ..t })
}

/// The difference ranges of `row` on `side`, in order: the inline ranges (only the important ones with *Ignore
/// Unimportant*), or the whole line of a one-sided row.
fn row_ranges(
    ctx: &NavContext<'_>,
    cmp: &TextComparison,
    row: RowIdx,
    side: Side,
) -> Vec<Range<u32>> {
    let Some(r) = cmp.row(row) else {
        return Vec::new();
    };
    if r.state == RowState::Same || r.ignored || !shown(ctx, row) {
        return Vec::new();
    }
    if ctx.ignore_unimportant && r.importance == crate::compare::Importance::Unimportant {
        return Vec::new();
    }
    let Some(line) = line_of(&r, side) else {
        return Vec::new();
    };
    let len = doc_of(ctx, side).map_or_else(
        || cmp.document(side).line_len(line.0) as u32,
        |d| d.line_len(line),
    );
    let other = if side == Side::Right {
        Side::Left
    } else {
        Side::Right
    };
    if line_of(&r, other).is_none() {
        return std::iter::once(0..len).collect();
    }
    let Some(inline) = cmp.inline(row) else {
        return Vec::new();
    };
    let ranges = if side == Side::Right {
        inline.right
    } else {
        inline.left
    };
    ranges
        .into_iter()
        .filter(|x| !ctx.ignore_unimportant || x.important)
        .map(|x| x.range.start..x.range.end.min(len))
        .collect()
}

fn difference_step(ctx: &NavContext<'_>, from: NavPos, dir: Dir, wrap: bool) -> Option<NavTarget> {
    let cmp = ctx.cmp?;
    let side = from.side;
    let rows = cmp.row_count();
    let target = |row: RowIdx, r: Range<u32>, wrapped: bool| -> Option<NavTarget> {
        let line = line_of(&cmp.row(row)?, side)?;
        Some(NavTarget {
            row: Some(row),
            side,
            range: TextPos {
                line,
                byte: r.start,
            }..TextPos { line, byte: r.end },
            wrapped,
        })
    };
    // Rows after (before) `start`, walked section by section so long same stretches cost nothing.
    let scan = |start: RowIdx, inclusive: bool, wrapped: bool| -> Option<NavTarget> {
        let mut at = start;
        let mut first = true;
        loop {
            let id = if first && inclusive {
                cmp.next_section_from(at, dir, false)
            } else {
                cmp.next_section(at, dir, false)
            };
            first = false;
            let section = cmp.sections().get(id?.0 as usize)?.clone();
            let rows_in: Vec<u32> = match dir {
                Direction::Forward => {
                    (section
                        .rows
                        .start
                        .max(if inclusive { start.0 } else { start.0 + 1 })
                        ..section.rows.end)
                        .collect()
                }
                Direction::Backward => (section.rows.start..section.rows.end.min(start.0))
                    .rev()
                    .collect(),
            };
            for r in rows_in {
                let ranges = row_ranges(ctx, cmp, RowIdx(r), side);
                let pick = match dir {
                    Direction::Forward => ranges.into_iter().next(),
                    Direction::Backward => ranges.into_iter().next_back(),
                };
                if let Some(x) = pick {
                    return target(RowIdx(r), x, wrapped);
                }
            }
            at = RowIdx(section.rows.start);
        }
    };
    let row = row_of(cmp, &from);
    // The caret's own row first: the ranges after (before) the caret.
    if row.0 < rows {
        let ranges = row_ranges(ctx, cmp, row, side);
        let b = from.pos.byte;
        let pick = match dir {
            Direction::Forward => ranges.into_iter().find(|x| x.start >= b),
            Direction::Backward => ranges.into_iter().rev().find(|x| x.start < b),
        };
        if let Some(x) = pick {
            return target(row, x, false);
        }
    }
    // The rest of the caret's section, then the next sections.
    let found = match dir {
        Direction::Forward => {
            let mut found = None;
            if let Some(id) = cmp.section_at(row) {
                let section = cmp.sections().get(id.0 as usize)?;
                for r in row.0 + 1..section.rows.end {
                    if let Some(x) = row_ranges(ctx, cmp, RowIdx(r), side).into_iter().next() {
                        found = target(RowIdx(r), x, false);
                        break;
                    }
                }
            }
            found.or_else(|| scan(row, false, false))
        }
        Direction::Backward => {
            let mut found = None;
            if let Some(id) = cmp.section_at(row) {
                let section = cmp.sections().get(id.0 as usize)?;
                for r in (section.rows.start..row.0).rev() {
                    if let Some(x) = row_ranges(ctx, cmp, RowIdx(r), side)
                        .into_iter()
                        .next_back()
                    {
                        found = target(RowIdx(r), x, false);
                        break;
                    }
                }
            }
            found.or_else(|| {
                let start = cmp
                    .section_at(row)
                    .and_then(|id| cmp.sections().get(id.0 as usize))
                    .map_or(row, |s| RowIdx(s.rows.start));
                scan(start, false, false)
            })
        }
    };
    if found.is_some() || !wrap {
        return found;
    }
    match dir {
        Direction::Forward => scan(RowIdx(0), true, true),
        Direction::Backward => scan(RowIdx(rows), false, true),
    }
}

fn edit_step(ctx: &NavContext<'_>, from: NavPos, dir: Dir, wrap: bool) -> Option<NavTarget> {
    let side = from.side;
    let doc = doc_of(ctx, side)?;
    let make = |lines: Range<u32>, wrapped: bool| NavTarget {
        row: ctx
            .cmp
            .and_then(|c| c.row_for_line(side, LineIdx(lines.start))),
        side,
        range: lines_range(ctx, side, lines),
        wrapped,
    };
    if let Some(lines) = doc.edited_ranges(from.pos.line, dir) {
        return Some(make(lines, false));
    }
    if !wrap {
        return None;
    }
    let n = doc.line_count();
    let again = match dir {
        // From "before the first line": a run starting at line 0 counts.
        Direction::Forward => {
            if n > 0 && doc.mark(LineIdx(0)) != crate::edit::LineMark::Original {
                let mut end = 1;
                while end < n && doc.mark(LineIdx(end)) != crate::edit::LineMark::Original {
                    end += 1;
                }
                Some(0..end)
            } else {
                doc.edited_ranges(LineIdx(0), dir)
            }
        }
        Direction::Backward => doc.edited_ranges(LineIdx(n), dir),
    }?;
    Some(make(again, true))
}

fn find_step(ctx: &NavContext<'_>, from: NavPos, dir: Dir, wrap: bool) -> Option<NavTarget> {
    let finder = ctx.finder?;
    match (ctx.cmp, ctx.docs.left, ctx.docs.right) {
        (Some(cmp), Some(left), Some(right)) => {
            let row = row_of(cmp, &from);
            let (side, hit) = find_in_comparison_at(
                cmp,
                Pair::new(left, right),
                finder,
                (row, from.side, from.pos.byte),
                ctx.sides,
                dir,
                wrap,
            )
            .ok()??;
            Some(NavTarget {
                row: cmp.row_for_line(side, hit.range.start.line),
                side,
                range: hit.range,
                wrapped: hit.wrapped,
            })
        }
        _ => {
            let doc = doc_of(ctx, from.side)?;
            let hit = finder.next(doc, from.pos, dir, wrap).ok()??;
            Some(NavTarget {
                row: ctx
                    .cmp
                    .and_then(|c| c.row_for_line(from.side, hit.range.start.line)),
                side: from.side,
                range: hit.range,
                wrapped: hit.wrapped,
            })
        }
    }
}

/// The next (previous) row after `from` where `side` has a line, skipping gaps (Alt+Down/Up).
pub fn existing_line(cmp: &TextComparison, from: RowIdx, side: Side, dir: Dir) -> Option<RowIdx> {
    let lines = cmp.document(side).line_count();
    // The first line of `side` whose row is after `from` (forward), or the last before it (backward).
    let bound = crate::find::line_bound(cmp, side, lines, from.0 + 1);
    match dir {
        Direction::Forward => (bound < lines).then(|| cmp.row_for_line(side, LineIdx(bound)))?,
        Direction::Backward => {
            let before = crate::find::line_bound(cmp, side, lines, from.0);
            before
                .checked_sub(1)
                .and_then(|l| cmp.row_for_line(side, LineIdx(l)))
        }
    }
}

/// Parses Go To's text: `120`, `120:7`, `:7` (the current line's column 7: line 0 stands for the current line).
/// Numbers are 1-based; spaces around them are allowed.
pub fn parse_goto(text: &str) -> Result<(u32, Option<u32>), String> {
    let text = text.trim();
    let number = |s: &str, what: &str| -> Result<u32, String> {
        let s = s.trim();
        let n: u32 = s
            .parse()
            .map_err(|_| format!("`{s}` is not a {what} number"))?;
        if n == 0 {
            return Err(format!("{what} numbers start at 1"));
        }
        Ok(n)
    };
    if text.is_empty() {
        return Err("a line number, `line:column` or `:column` is needed".to_owned());
    }
    match text.split_once(':') {
        None => Ok((number(text, "line")?, None)),
        Some((line, column)) => {
            let line = if line.trim().is_empty() {
                0
            } else {
                number(line, "line")?
            };
            Ok((line, Some(number(column, "column")?)))
        }
    }
}

/// The position of physical line `line` (1-based, as the gutter numbers them, TC-003) and character column `column`
/// (1-based): a line past the last goes to the last, a column past the line's end to its end.
pub fn goto(doc: &EditableDocument, line: u32, column: Option<u32>) -> TextPos {
    let d = doc.document();
    let n = d.line_count();
    if n == 0 {
        return TextPos::ZERO;
    }
    let want = line.max(1);
    // The first table line whose physical number is `want` (physical numbers grow with table lines).
    let (mut lo, mut hi) = (0u32, n);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if d.physical_line(LineIdx(mid)) < want {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let mut first = lo.min(n - 1);
    if lo >= n {
        // Past the last line: the last physical line's first piece.
        let last = d.physical_line(LineIdx(n - 1));
        while first > 0 && d.physical_line(LineIdx(first - 1)) == last {
            first -= 1;
        }
    }
    let Some(column) = column else {
        return TextPos::new(first, 0);
    };
    // Walk the pieces of the physical line by characters.
    let physical = d.physical_line(LineIdx(first));
    let mut left = column.max(1) - 1;
    let mut l = first;
    loop {
        let text = doc.line(LineIdx(l));
        let chars = text.chars().count() as u32;
        let next_same = l + 1 < n
            && d.line_ending(LineIdx(l)) == crate::document::LineEnding::Artificial
            && d.physical_line(LineIdx(l + 1)) == physical;
        if left < chars || !next_same {
            let byte = text
                .char_indices()
                .nth(left as usize)
                .map_or(text.len(), |(k, _)| k);
            return TextPos::new(l, byte as u32);
        }
        left -= chars;
        l += 1;
    }
}

/// The status bar's column of byte `byte` in `line`: the character column and the display column (tabs to the next
/// multiple of 4, wide characters 2, combining marks 0), both 1-based. [`column_of_tabs`] takes the pane's tab size.
pub fn column_of(line: &str, byte: u32) -> (u32, u32) {
    column_of_tabs(line, byte, 4)
}

/// [`column_of`] with the pane's tab size.
pub fn column_of_tabs(line: &str, byte: u32, tab_size: u8) -> (u32, u32) {
    let mut b = (byte as usize).min(line.len());
    while !line.is_char_boundary(b) {
        b -= 1;
    }
    let chars = line[..b].chars().count() as u32;
    let display = crate::edit::words::column(line, b, tab_size);
    (chars + 1, display + 1)
}
