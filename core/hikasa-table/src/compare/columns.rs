//! Side views (header harmonization), sheet pairing (T-10), column alignment (T-11) and column references
//! (T-12).

use std::borrow::Cow;

use crate::compare::options::{ColumnAlignment, ColumnRef, SheetAlignment, SheetRules};
use crate::error::TableError;
use crate::header::kinds_from;
use crate::sheet::{ColumnKind, RowView, Sheet, parse_column_letters};

/// One side of a sheet pair as compared.
pub(crate) struct SideView<'a> {
    pub sheet: &'a Sheet,
    /// 1 when the first stored row is the header.
    pub first: u32,
    pub header: Option<Vec<String>>,
    pub kinds: Vec<ColumnKind>,
    pub rows: u32,
    pub width: u32,
    /// With ignore-hidden-rows: the data row of each compared row (F13).
    rows_map: Option<Vec<u32>>,
    /// With ignore-hidden-columns: the columns compared.
    visible: Option<Vec<u32>>,
}

impl<'a> SideView<'a> {
    fn new(sheet: &'a Sheet, header: bool, rules: &SheetRules) -> SideView<'a> {
        let width = sheet.column_count();
        let own = sheet.header().is_some();
        let first = u32::from(header);
        let (names, kinds) = if header == own {
            (
                sheet.header().map(<[String]>::to_vec),
                (0..width).map(|c| sheet.column_kind(c)).collect(),
            )
        } else {
            let names = header.then(|| {
                (0..width)
                    .map(|c| sheet.stored_row(0).get(c).display().trim().to_owned())
                    .collect()
            });
            (names, kinds_from(sheet, first, width))
        };
        let all = sheet.stored_count().saturating_sub(first);
        let rows_map: Option<Vec<u32>> =
            (rules.ignore_hidden_rows && !sheet.info().hidden_rows.is_empty()).then(|| {
                (0..all)
                    .filter(|&r| !sheet.stored_hidden(r + first))
                    .collect()
            });
        let visible = (rules.ignore_hidden_columns && !sheet.info().hidden_columns.is_empty())
            .then(|| (0..width).filter(|&c| !sheet.column_hidden(c)).collect());
        SideView {
            sheet,
            first,
            header: names,
            kinds,
            rows: rows_map.as_ref().map_or(all, |m| m.len() as u32),
            width,
            rows_map,
            visible,
        }
    }

    /// The data row of compared row `r`.
    pub fn data_row(&self, r: u32) -> u32 {
        self.rows_map.as_ref().map_or(r, |m| m[r as usize])
    }

    /// Compared row `r`.
    pub fn row(&self, r: u32) -> RowView<'a> {
        self.sheet.stored_row(self.data_row(r) + self.first)
    }

    /// 1-based label of compared row `r`.
    pub fn label(&self, r: u32) -> u64 {
        self.sheet.stored_label(self.data_row(r) + self.first)
    }

    /// The columns compared (all but hidden ones with ignore-hidden-columns).
    pub fn columns(&self) -> Vec<u32> {
        self.visible
            .clone()
            .unwrap_or_else(|| (0..self.width).collect())
    }

    /// Name of column `c`: the header name, else its letters.
    pub fn name(&self, c: u32) -> Cow<'_, str> {
        match self.header.as_ref().and_then(|h| h.get(c as usize)) {
            Some(n) if !n.is_empty() => Cow::Borrowed(n),
            _ => Cow::Owned(self.sheet.column_label(c)),
        }
    }

    /// Kind of column `c`.
    pub fn kind(&self, c: u32) -> ColumnKind {
        self.kinds
            .get(c as usize)
            .copied()
            .unwrap_or(ColumnKind::Empty)
    }
}

/// The two side views of a pair, with the header choice made the same on both sides: when only one side
/// detected a header, the other gets one too if its first row could be that header (it shares a name with it, or
/// it is all names, blanks and years; its votes are not against it), else neither has one.
pub(crate) fn side_views<'a>(
    l: &'a Sheet,
    r: &'a Sheet,
    rules: &SheetRules,
) -> (SideView<'a>, SideView<'a>) {
    let (lh, rh) = (l.header().is_some(), r.header().is_some());
    let both = if lh == rh {
        lh
    } else {
        let (detected, other) = if lh { (l, r) } else { (r, l) };
        crate::header::could_be_header(other, detected.header().unwrap_or(&[]))
    };
    (SideView::new(l, both, rules), SideView::new(r, both, rules))
}

/// One side of a pairing: an index, or none.
pub(crate) type Paired = (Option<usize>, Option<usize>);

/// Column pairs of one sheet pair.
pub(crate) type ColumnPairs = Vec<(Option<u32>, Option<u32>)>;

fn match_names(
    left: &[&str],
    right: &[&str],
    fold: bool,
    l_used: &mut [bool],
    r_used: &mut [bool],
    out: &mut [Option<usize>],
) {
    for (i, ln) in left.iter().enumerate() {
        if l_used[i] {
            continue;
        }
        let found = right.iter().enumerate().position(|(j, rn)| {
            !r_used[j]
                && if fold {
                    hikasa_core::case::eq_folded(ln.trim(), rn.trim())
                } else {
                    ln == rn
                }
        });
        if let Some(j) = found {
            l_used[i] = true;
            r_used[j] = true;
            out[i] = Some(j);
        }
    }
}

/// Pairs by name (exact, then folded), optionally the rest by position; the output follows the left order
/// with right-only items after the left item they follow on the right.
fn pair_by_name(left: &[&str], right: &[&str], position_rest: bool) -> Vec<Paired> {
    let mut l_used = vec![false; left.len()];
    let mut r_used = vec![false; right.len()];
    let mut to = vec![None; left.len()];
    match_names(left, right, false, &mut l_used, &mut r_used, &mut to);
    match_names(left, right, true, &mut l_used, &mut r_used, &mut to);
    if position_rest {
        let ls: Vec<usize> = (0..left.len()).filter(|&i| !l_used[i]).collect();
        let rs: Vec<usize> = (0..right.len()).filter(|&j| !r_used[j]).collect();
        for (&i, &j) in ls.iter().zip(&rs) {
            l_used[i] = true;
            r_used[j] = true;
            to[i] = Some(j);
        }
    }
    interleave(&to, right.len())
}

/// Builds the output order from `to[i]` (left `i` paired with right `to[i]`): left order, each right-only
/// item placed after the left item paired with the closest preceding right item.
fn interleave(to: &[Option<usize>], right_len: usize) -> Vec<Paired> {
    let mut from = vec![None; right_len];
    for (i, j) in to.iter().enumerate() {
        if let Some(j) = j {
            from[*j] = Some(i);
        }
    }
    // Anchor of each right-only item: the left index of the closest preceding paired right item.
    let mut anchored: Vec<Vec<usize>> = vec![Vec::new(); to.len() + 1];
    let mut anchor = 0usize; // 0 = before every left item; i + 1 = after left item i
    for (j, f) in from.iter().enumerate() {
        match f {
            Some(i) => anchor = i + 1,
            None => anchored[anchor].push(j),
        }
    }
    let mut out = Vec::with_capacity(to.len() + right_len);
    out.extend(anchored[0].iter().map(|&j| (None, Some(j))));
    for (i, j) in to.iter().enumerate() {
        out.push((Some(i), *j));
        out.extend(anchored[i + 1].iter().map(|&j| (None, Some(j))));
    }
    out
}

fn mirror(pairs: Vec<Paired>) -> Vec<Paired> {
    pairs.into_iter().map(|(a, b)| (b, a)).collect()
}

/// Sheet pairs (T-10).
pub(crate) fn pair_sheets(
    left: &[&str],
    right: &[&str],
    how: &SheetAlignment,
) -> Result<Vec<Paired>, TableError> {
    Ok(match how {
        SheetAlignment::ByName => pair_by_name(left, right, true),
        SheetAlignment::LeftName => pair_by_name(left, right, false),
        SheetAlignment::RightName => mirror(pair_by_name(right, left, false)),
        SheetAlignment::Position => by_position(left.len(), right.len()),
        SheetAlignment::Custom(pairs) => {
            let mut out = Vec::with_capacity(pairs.len());
            for p in pairs {
                let resolve = |sel: &Option<crate::compare::options::SheetSelector>,
                               names: &[&str],
                               side: &str|
                 -> Result<Option<usize>, TableError> {
                    match sel {
                        None => Ok(None),
                        Some(sel) => {
                            sel.resolve(names.iter().copied()).map(Some).ok_or_else(|| {
                                TableError::InvalidInput(format!(
                                    "no {side} sheet {sel:?} (sheets: {})",
                                    names.join(", ")
                                ))
                            })
                        }
                    }
                };
                out.push((
                    resolve(&p.left, left, "left")?,
                    resolve(&p.right, right, "right")?,
                ));
            }
            out
        }
    })
}

fn by_position(l: usize, r: usize) -> Vec<Paired> {
    (0..l.max(r))
        .map(|i| ((i < l).then_some(i), (i < r).then_some(i)))
        .collect()
}

/// Resolves a reference against one side's columns (`None` when it names nothing).
pub(crate) fn resolve_side(r: &ColumnRef, view: &SideView<'_>) -> Option<u32> {
    let names: Vec<Cow<'_, str>> = (0..view.width).map(|c| view.name(c)).collect();
    let by_name = |n: &str| -> Option<u32> {
        names
            .iter()
            .position(|x| x == n)
            .or_else(|| {
                names
                    .iter()
                    .position(|x| hikasa_core::case::eq_folded(x.trim(), n.trim()))
            })
            .map(|i| i as u32)
    };
    match r {
        ColumnRef::Position(n) => (*n <= view.width).then(|| n - 1),
        ColumnRef::Name(n) => by_name(n),
        ColumnRef::Auto(n) => {
            by_name(n).or_else(|| parse_column_letters(n).filter(|&c| c < view.width))
        }
    }
}

/// Column pairs (T-11).
pub(crate) fn align_columns(
    l: &SideView<'_>,
    r: &SideView<'_>,
    how: &ColumnAlignment,
) -> Result<ColumnPairs, TableError> {
    // Pairing runs over the compared columns of each side (hidden ones left out with ignore-hidden-columns) and
    // maps back to sheet columns.
    let (lv, rv) = (l.columns(), r.columns());
    let names = |v: &SideView<'_>, cols: &[u32]| -> Vec<String> {
        cols.iter().map(|&c| v.name(c).into_owned()).collect()
    };
    let to_u32 = |p: Vec<Paired>| -> Vec<(Option<u32>, Option<u32>)> {
        p.into_iter()
            .map(|(a, b)| (a.map(|x| lv[x]), b.map(|x| rv[x])))
            .collect()
    };
    let named = |left_order: bool| {
        let (ln, rn) = (names(l, &lv), names(r, &rv));
        let lr: Vec<&str> = ln.iter().map(String::as_str).collect();
        let rr: Vec<&str> = rn.iter().map(String::as_str).collect();
        if left_order {
            pair_by_name(&lr, &rr, false)
        } else {
            mirror(pair_by_name(&rr, &lr, false))
        }
    };
    Ok(match how {
        ColumnAlignment::Auto => {
            if l.header.is_some() && r.header.is_some() {
                to_u32(named(true))
            } else {
                to_u32(by_position(lv.len(), rv.len()))
            }
        }
        ColumnAlignment::Number => to_u32(by_position(lv.len(), rv.len())),
        ColumnAlignment::LeftName => to_u32(named(true)),
        ColumnAlignment::RightName => to_u32(named(false)),
        ColumnAlignment::Custom(pairs) => {
            let mut l_used = vec![false; l.width as usize];
            let mut r_used = vec![false; r.width as usize];
            let mut out = Vec::new();
            for (a, b) in pairs {
                let find = |x: &Option<ColumnRef>,
                            v: &SideView<'_>,
                            used: &mut [bool],
                            side: &str| {
                    match x {
                        None => Ok(None),
                        Some(x) => {
                            let c = resolve_side(x, v).ok_or_else(|| {
                                TableError::InvalidInput(format!(
                                    "column `{x}` names no {side} column of sheet `{}` (columns: {})",
                                    v.sheet.name(),
                                    (0..v.width).map(|c| v.name(c).into_owned()).collect::<Vec<_>>().join(", ")
                                ))
                            })?;
                            if used[c as usize] {
                                return Err(TableError::InvalidInput(format!(
                                    "{side} column `{x}` is paired twice"
                                )));
                            }
                            used[c as usize] = true;
                            Ok(Some(c))
                        }
                    }
                };
                let lc = find(a, l, &mut l_used, "left")?;
                let rc = find(b, r, &mut r_used, "right")?;
                out.push((lc, rc));
            }
            let ls: Vec<u32> = lv
                .iter()
                .copied()
                .filter(|&c| !l_used[c as usize])
                .collect();
            let rs: Vec<u32> = rv
                .iter()
                .copied()
                .filter(|&c| !r_used[c as usize])
                .collect();
            for i in 0..ls.len().max(rs.len()) {
                out.push((ls.get(i).copied(), rs.get(i).copied()));
            }
            out
        }
    })
}

/// Resolves a reference against the comparison columns: a position or letters name a comparison column, a
/// name matches the left names, then the right names (exact, then folded).
pub(crate) fn resolve_comparison(
    r: &ColumnRef,
    cols: &[(Option<u32>, Option<u32>)],
    l: &SideView<'_>,
    rv: &SideView<'_>,
) -> Result<usize, TableError> {
    let left_names: Vec<Option<Cow<'_, str>>> =
        cols.iter().map(|(a, _)| a.map(|c| l.name(c))).collect();
    let right_names: Vec<Option<Cow<'_, str>>> =
        cols.iter().map(|(_, b)| b.map(|c| rv.name(c))).collect();
    let by_name = |n: &str| -> Option<usize> {
        let exact =
            |names: &[Option<Cow<'_, str>>]| names.iter().position(|x| x.as_deref() == Some(n));
        let folded = |names: &[Option<Cow<'_, str>>]| {
            names.iter().position(|x| {
                x.as_deref()
                    .is_some_and(|x| hikasa_core::case::eq_folded(x.trim(), n.trim()))
            })
        };
        exact(&left_names)
            .or_else(|| exact(&right_names))
            .or_else(|| folded(&left_names))
            .or_else(|| folded(&right_names))
    };
    let found = match r {
        ColumnRef::Position(n) => ((*n as usize) <= cols.len()).then(|| *n as usize - 1),
        ColumnRef::Name(n) => by_name(n),
        ColumnRef::Auto(n) => by_name(n).or_else(|| {
            parse_column_letters(n)
                .map(|c| c as usize)
                .filter(|&c| c < cols.len())
        }),
    };
    found.ok_or_else(|| {
        let list: Vec<String> = cols
            .iter()
            .map(|(a, b)| match (a, b) {
                (Some(a), _) => l.name(*a).into_owned(),
                (None, Some(b)) => rv.name(*b).into_owned(),
                (None, None) => String::new(),
            })
            .collect();
        TableError::InvalidInput(format!(
            "column `{r}` names no column of sheet `{}` (columns: {})",
            l.sheet.name(),
            list.join(", ")
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheets_pair_by_name_then_position() {
        let p = pair_sheets(
            &["A", "b", "X"],
            &["B", "a", "Y", "Z"],
            &SheetAlignment::ByName,
        )
        .unwrap();
        assert_eq!(
            p,
            vec![
                (Some(0), Some(1)),
                (Some(1), Some(0)),
                (Some(2), Some(2)),
                (None, Some(3))
            ]
        );
        let p = pair_sheets(&["A", "X"], &["Q", "A"], &SheetAlignment::LeftName).unwrap();
        assert_eq!(
            p,
            vec![(None, Some(0)), (Some(0), Some(1)), (Some(1), None)]
        );
        let p = pair_sheets(&["A", "X"], &["Q", "A"], &SheetAlignment::RightName).unwrap();
        assert_eq!(
            p,
            vec![(None, Some(0)), (Some(0), Some(1)), (Some(1), None)]
        );
        let p = pair_sheets(&["A"], &["B", "C"], &SheetAlignment::Position).unwrap();
        assert_eq!(p, vec![(Some(0), Some(0)), (None, Some(1))]);
    }

    #[test]
    fn interleave_places_right_only_items_after_their_neighbour() {
        // left: a b c; right: x a y c → x before a, y after a.
        let to = vec![Some(1), None, Some(3)];
        assert_eq!(
            interleave(&to, 4),
            vec![
                (None, Some(0)),
                (Some(0), Some(1)),
                (None, Some(2)),
                (Some(1), None),
                (Some(2), Some(3))
            ]
        );
    }
}
