//! Navigation data (TB-022): next and previous difference rows, cells and sheets. Wrap-free: the caller
//! wraps per OPT-007.

use crate::compare::result::{SheetComparison, TableComparison};

/// Direction of a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Towards the end.
    Next,
    /// Towards the start.
    Prev,
}

/// The next (or previous) differing row strictly after (before) `from` (`None` = from the edge).
pub fn next_difference_row(
    sheet: &SheetComparison,
    from: Option<u32>,
    dir: Dir,
    ignore_unimportant: bool,
) -> Option<u32> {
    let rows = sheet.rows();
    let differs = |i: usize| rows[i].differs(ignore_unimportant);
    match dir {
        Dir::Next => {
            let start = from.map_or(0, |f| f as usize + 1);
            (start..rows.len()).find(|&i| differs(i)).map(|i| i as u32)
        }
        Dir::Prev => {
            let end = from.map_or(rows.len(), |f| (f as usize).min(rows.len()));
            (0..end).rev().find(|&i| differs(i)).map(|i| i as u32)
        }
    }
}

/// The next (or previous) differing cell `(row, column)` after (before) `from`, in row-major order; orphan
/// rows count by their first column.
pub fn next_difference_cell(
    sheet: &SheetComparison,
    from: Option<(u32, u32)>,
    dir: Dir,
    ignore_unimportant: bool,
) -> Option<(u32, u32)> {
    use crate::compare::result::{CellState, RowKind};
    let rows = sheet.rows();
    let cells_of = |i: u32| -> Vec<u32> {
        let r = &rows[i as usize];
        match r.kind() {
            RowKind::Same => Vec::new(),
            RowKind::Changed => sheet
                .differing_cells(i)
                .iter()
                .filter(|d| !(ignore_unimportant && d.state == CellState::Unimportant))
                .map(|d| d.column)
                .collect(),
            _ if r.differs(ignore_unimportant) => vec![0],
            _ => Vec::new(),
        }
    };
    match dir {
        Dir::Next => {
            let (start_row, after_col) = match from {
                None => (0u32, None),
                Some((r, c)) => (r, Some(c)),
            };
            for i in start_row..rows.len() as u32 {
                let cols = cells_of(i);
                let pick = cols
                    .into_iter()
                    .find(|&c| i != start_row || after_col.is_none_or(|a| c > a));
                if let Some(c) = pick {
                    return Some((i, c));
                }
            }
            None
        }
        Dir::Prev => {
            let (start_row, before_col) = match from {
                None => (rows.len() as u32, None),
                Some((r, c)) => (r + 1, Some((r, c))),
            };
            for i in (0..start_row.min(rows.len() as u32)).rev() {
                let cols = cells_of(i);
                let pick = cols
                    .into_iter()
                    .rev()
                    .find(|&c| before_col.is_none_or(|(r, b)| i != r || c < b));
                if let Some(c) = pick {
                    return Some((i, c));
                }
            }
            None
        }
    }
}

/// The next (or previous) sheet pair whose state is not same.
pub fn next_difference_sheet(
    cmp: &TableComparison,
    from: Option<usize>,
    dir: Dir,
    ignore_unimportant: bool,
) -> Option<usize> {
    use crate::compare::result::SheetState;
    let sheets = cmp.sheets();
    let differs = |i: usize| match sheets[i].state() {
        SheetState::Same => false,
        SheetState::Unimportant => !ignore_unimportant,
        _ => true,
    };
    match dir {
        Dir::Next => (from.map_or(0, |f| f + 1)..sheets.len()).find(|&i| differs(i)),
        Dir::Prev => (0..from.unwrap_or(sheets.len()).min(sheets.len()))
            .rev()
            .find(|&i| differs(i)),
    }
}
