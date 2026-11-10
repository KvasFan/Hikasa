//! One sheet pair (contract §5.2–§5.5): columns, rules, header, rows, cells, statistics.

use hikasa_core::CancelToken;
use rayon::prelude::*;

use crate::compare::cells::{compare_values, fast_same, mode, texts_header};
use crate::compare::columns::{SideView, align_columns, resolve_comparison};
use crate::compare::options::{ColumnRef, ColumnRule, SheetRules};
use crate::compare::result::{
    CellDiff, CellState, ColumnPair, DuplicateKeys, Importance, RowAlignmentUsed, RowKind, RowPair,
    SheetComparison, SheetState, SheetStats, SideRef,
};
use crate::compare::rows::{CHUNK, Col, RowRef, column_map, keyed, unkeyed, walk_stored};
use crate::error::{TableError, TableWarning, TableWarningCode};
use crate::sheet::ColumnKind;

/// The inputs of one pair.
pub(crate) struct PairInput<'a> {
    pub left: Option<(u32, SideView<'a>)>,
    pub right: Option<(u32, SideView<'a>)>,
    pub rules: &'a SheetRules,
    pub compare_formulas: bool,
}

struct Chunk {
    rows: Vec<RowPair>,
    diffs: Vec<CellDiff>,
    stats: SheetStats,
}

fn differs(rule: &ColumnRule) -> CellState {
    if rule.unimportant {
        CellState::Unimportant
    } else {
        CellState::Important
    }
}

/// Importance of an orphan row of `view`; `rules[c]` is the rule of sheet column `c` (its stored cells are
/// enough: a gap is blank).
fn orphan_importance(view: &SideView<'_>, r: u32, rules: &[Option<ColumnRule>]) -> Importance {
    let row = view.row(r);
    let mut imp = Importance::None;
    for (c, v) in row.stored() {
        let Some(Some(rule)) = rules.get(c as usize) else {
            continue;
        };
        if !v.is_blank() {
            if !rule.unimportant {
                return Importance::Important;
            }
            imp = Importance::Unimportant;
        }
    }
    // A blank orphan row is still a row the other side lacks: an unimportant difference.
    if imp == Importance::None {
        Importance::Unimportant
    } else {
        imp
    }
}

/// A compared pair: the comparison, its warnings, and the references its lenient rules left out (T-30) with the
/// error each would have been.
pub(crate) type PairOutput = (
    SheetComparison,
    Vec<TableWarning>,
    Vec<(String, TableError)>,
);

pub(crate) fn compare_pair(
    input: PairInput<'_>,
    cancel: &CancelToken,
) -> Result<PairOutput, TableError> {
    let rules = input.rules;
    match (&input.left, &input.right) {
        (Some((li, l)), Some((ri, r))) => {
            paired(*li, l, *ri, r, rules, input.compare_formulas, cancel)
        }
        (Some((i, v)), None) => Ok((orphan(*i, v, true, rules), Vec::new(), Vec::new())),
        (None, Some((i, v))) => Ok((orphan(*i, v, false, rules), Vec::new(), Vec::new())),
        (None, None) => Err(TableError::InvalidInput("an empty sheet pair".to_owned())),
    }
}

fn orphan(index: u32, v: &SideView<'_>, left: bool, rules: &SheetRules) -> SheetComparison {
    let rule = rules.default_rule;
    let cols: Vec<Option<ColumnRule>> = vec![Some(rule); v.width as usize];
    let mut stats = SheetStats::default();
    let rows: Vec<RowPair> = (0..v.rows)
        .map(|r| {
            let imp = orphan_importance(v, r, &cols);
            let important = imp == Importance::Important;
            if left {
                stats.left_rows += 1;
                if important {
                    stats.left_only_important += 1;
                } else {
                    stats.left_only_unimportant += 1;
                }
                RowPair::new(Some(v.data_row(r)), None, RowKind::LeftOnly, imp)
            } else {
                stats.right_rows += 1;
                if important {
                    stats.right_only_important += 1;
                } else {
                    stats.right_only_unimportant += 1;
                }
                RowPair::new(None, Some(v.data_row(r)), RowKind::RightOnly, imp)
            }
        })
        .collect();
    stats.rows = rows.len() as u64;
    let columns = (0..v.width)
        .map(|c| ColumnPair {
            left: left.then_some(c),
            right: (!left).then_some(c),
            name: v.name(c).into_owned(),
            rule,
            key: None,
            hidden: false,
            left_kind: if left { v.kind(c) } else { ColumnKind::Empty },
            right_kind: if left { ColumnKind::Empty } else { v.kind(c) },
        })
        .collect::<Vec<_>>();
    if left {
        stats.columns_left_only = columns.len() as u64;
    } else {
        stats.columns_right_only = columns.len() as u64;
    }
    let side = Some(SideRef {
        sheet: index,
        first: v.first,
    });
    SheetComparison {
        left: if left { side } else { None },
        right: if left { None } else { side },
        name: v.sheet.name().to_owned(),
        renamed: false,
        columns,
        rows,
        diffs: Vec::new(),
        header: Vec::new(),
        stats,
        state: if left {
            SheetState::LeftOnly
        } else {
            SheetState::RightOnly
        },
        alignment: RowAlignmentUsed::Orphan,
        duplicates: DuplicateKeys::default(),
    }
}

#[allow(clippy::too_many_arguments)]
fn paired(
    li: u32,
    l: &SideView<'_>,
    ri: u32,
    r: &SideView<'_>,
    rules: &SheetRules,
    compare_formulas: bool,
    cancel: &CancelToken,
) -> Result<PairOutput, TableError> {
    let pairs = align_columns(l, r, &rules.columns)?;
    // Effective rules. With lenient references (T-30) a reference that names no column of this sheet is left out
    // here and reported; `skipped` keeps its text for the check over every sheet.
    let mut skipped: Vec<(String, TableError)> = Vec::new();
    let resolve = |reference: &ColumnRef,
                   skipped: &mut Vec<(String, TableError)>|
     -> Result<Option<usize>, TableError> {
        match resolve_comparison(reference, &pairs, l, r) {
            Ok(c) => Ok(Some(c)),
            Err(e) if rules.lenient_references => {
                skipped.push((reference.to_string(), e));
                Ok(None)
            }
            Err(e) => Err(e),
        }
    };
    let mut col_rules: Vec<ColumnRule> = vec![rules.default_rule; pairs.len()];
    for (reference, patch) in &rules.column_rules {
        if let Some(c) = resolve(reference, &mut skipped)? {
            patch.apply(&mut col_rules[c]);
        }
    }
    let mut keys: Vec<usize> = Vec::new();
    let mut keys_complete = true;
    for reference in &rules.keys {
        let Some(c) = resolve(reference, &mut skipped)? else {
            keys_complete = false;
            continue;
        };
        if pairs[c].0.is_none() || pairs[c].1.is_none() {
            let e = TableError::InvalidInput(format!(
                "key column `{reference}` exists on one side only in sheet `{}`",
                l.sheet.name()
            ));
            if !rules.lenient_references {
                return Err(e);
            }
            keys_complete = false;
            skipped.push((reference.to_string(), e));
            continue;
        }
        if !keys.contains(&c) {
            keys.push(c);
        }
    }
    if !keys_complete {
        // A partial key would join rows by fewer columns than asked: the sheet is compared unkeyed.
        keys.clear();
    }
    let mut hidden = vec![false; pairs.len()];
    for reference in &rules.hidden {
        if let Some(c) = resolve(reference, &mut skipped)? {
            hidden[c] = true;
        }
    }
    let cols: Vec<Col> = pairs
        .iter()
        .zip(&col_rules)
        .map(|(&(a, b), rule)| Col {
            left: a,
            right: b,
            rule: *rule,
            mode: mode(
                rule,
                a.map_or(ColumnKind::Empty, |c| l.kind(c)),
                b.map_or(ColumnKind::Empty, |c| r.kind(c)),
            ),
        })
        .collect();
    let columns: Vec<ColumnPair> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| ColumnPair {
            left: c.left,
            right: c.right,
            name: match (c.left, c.right) {
                (Some(a), _) => l.name(a).into_owned(),
                (None, Some(b)) => r.name(b).into_owned(),
                (None, None) => String::new(),
            },
            rule: c.rule,
            key: keys.iter().position(|&k| k == i).map(|p| p.min(255) as u8),
            hidden: hidden[i],
            left_kind: c.left.map_or(ColumnKind::Empty, |x| l.kind(x)),
            right_kind: c.right.map_or(ColumnKind::Empty, |x| r.kind(x)),
        })
        .collect();
    let mut stats = SheetStats {
        left_rows: u64::from(l.rows),
        right_rows: u64::from(r.rows),
        columns_paired: cols
            .iter()
            .filter(|c| c.left.is_some() && c.right.is_some())
            .count() as u64,
        columns_left_only: cols.iter().filter(|c| c.right.is_none()).count() as u64,
        columns_right_only: cols.iter().filter(|c| c.left.is_none()).count() as u64,
        ..SheetStats::default()
    };
    // Header names.
    let mut header = Vec::new();
    if let (Some(lh), Some(rh)) = (&l.header, &r.header) {
        for c in &cols {
            let a = c
                .left
                .and_then(|x| lh.get(x as usize))
                .map_or("", String::as_str);
            let b = c
                .right
                .and_then(|x| rh.get(x as usize))
                .map_or("", String::as_str);
            let state = texts_header(a, b).unwrap_or_else(|| differs(&c.rule));
            if state != CellState::Same {
                stats.header_differences += 1;
            }
            header.push(state);
        }
    }
    // Rows.
    let mut warnings: Vec<TableWarning> = skipped
        .iter()
        .map(|(_, e)| {
            TableWarning::new(
                TableWarningCode::RuleNotApplied,
                format!("{e}; the rule is left out of this sheet"),
            )
            .in_sheet(l.sheet.name())
        })
        .collect();
    let mut duplicates = DuplicateKeys::default();
    let (pairing, alignment): (Vec<RowRef>, RowAlignmentUsed) = if keys.is_empty() {
        let (rows, w) = unkeyed(l, r, &cols, &rules.rows, cancel)?;
        warnings.extend(w.into_iter().map(|w| w.in_sheet(l.sheet.name())));
        (
            rows,
            RowAlignmentUsed::Engine {
                algorithm: rules.rows.algorithm,
                sorted: rules.rows.sort_rows,
            },
        )
    } else {
        let key_cols: Vec<Col> = keys.iter().map(|&k| cols[k]).collect();
        let (rows, d) = keyed(l, r, &key_cols, cancel)?;
        if d.left_rows > 0 || d.right_rows > 0 {
            warnings.push(
                TableWarning::new(
                    TableWarningCode::DuplicateKeys,
                    format!(
                        "sheet `{}`: {} left and {} right rows have duplicate keys (paired in order)",
                        l.sheet.name(),
                        d.left_rows,
                        d.right_rows
                    ),
                )
                .in_sheet(l.sheet.name()),
            );
        }
        stats.duplicate_left = d.left_rows;
        stats.duplicate_right = d.right_rows;
        duplicates = d;
        (
            rows,
            RowAlignmentUsed::Keyed {
                keys: keys.iter().map(|&k| k as u32).collect(),
            },
        )
    };
    cancel.check()?;
    // Cells, in parallel chunks.
    let side_rules = |width: u32, side: fn(&Col) -> Option<u32>| {
        let mut out: Vec<Option<ColumnRule>> = vec![None; width as usize];
        for c in &cols {
            if let Some(slot) = side(c).and_then(|x| out.get_mut(x as usize)) {
                *slot = Some(c.rule);
            }
        }
        out
    };
    let lcols = side_rules(l.width, |c| c.left);
    let rcols = side_rules(r.width, |c| c.right);
    let lmap = column_map(l.width, cols.iter().map(|c| c.left.unwrap_or(u32::MAX)));
    let rmap = column_map(r.width, cols.iter().map(|c| c.right.unwrap_or(u32::MAX)));
    let formulas = compare_formulas && (l.sheet.has_formulas() || r.sheet.has_formulas());
    let chunks: Vec<Chunk> = pairing
        .par_chunks(CHUNK)
        .enumerate()
        .map(|(ci, chunk)| {
            cancel.check()?;
            let base = (ci * CHUNK) as u32;
            let mut out = Chunk {
                rows: Vec::with_capacity(chunk.len()),
                diffs: Vec::new(),
                stats: SheetStats::default(),
            };
            let mut touched: Vec<u32> = Vec::new();
            for (k, &(lr, rr)) in chunk.iter().enumerate() {
                let index = base + k as u32;
                let row = match (lr, rr) {
                    (Some(a), Some(b)) => {
                        let (lrow, rrow) = (l.row(a), r.row(b));
                        let mut imp = Importance::None;
                        // Sparse rows: only the columns with a stored cell on a side can differ (F4).
                        let walk = walk_stored(&lrow, &rrow, cols.len());
                        if walk {
                            touched.clear();
                            touched.extend(
                                lrow.stored_cells()
                                    .filter_map(|(c, _)| lmap.get(c as usize).copied())
                                    .chain(
                                        rrow.stored_cells()
                                            .filter_map(|(c, _)| rmap.get(c as usize).copied()),
                                    )
                                    .filter(|&j| j != u32::MAX),
                            );
                            touched.sort_unstable();
                            touched.dedup();
                            let n = cols.len() as u64 - touched.len() as u64;
                            out.stats.cells_compared += n;
                            out.stats.cells_same += n;
                        }
                        let all = 0..cols.len() as u32;
                        let visit: &mut dyn Iterator<Item = u32> = if walk {
                            &mut touched.iter().copied()
                        } else {
                            &mut all.clone()
                        };
                        for c in visit {
                            let col = &cols[c as usize];
                            let state = match (col.left, col.right) {
                                (Some(lc), Some(rc)) => {
                                    let mut s = if fast_same(
                                        lrow.cell(lc),
                                        &lrow,
                                        lc,
                                        rrow.cell(rc),
                                        &rrow,
                                        rc,
                                    ) {
                                        CellState::Same
                                    } else {
                                        compare_values(
                                            lrow.get(lc),
                                            rrow.get(rc),
                                            col.mode,
                                            &col.rule,
                                            l.sheet.regional(),
                                            r.sheet.regional(),
                                        )
                                    };
                                    if formulas
                                        && s == CellState::Same
                                        && l.sheet.stored_formula(l.data_row(a) + l.first, lc)
                                            != r.sheet.stored_formula(r.data_row(b) + r.first, rc)
                                    {
                                        s = differs(&col.rule);
                                    }
                                    s
                                }
                                (Some(lc), None) => {
                                    if lrow.get(lc).is_blank() {
                                        CellState::Same
                                    } else {
                                        differs(&col.rule)
                                    }
                                }
                                (None, Some(rc)) => {
                                    if rrow.get(rc).is_blank() {
                                        CellState::Same
                                    } else {
                                        differs(&col.rule)
                                    }
                                }
                                (None, None) => CellState::Same,
                            };
                            out.stats.cells_compared += 1;
                            match state {
                                CellState::Same => out.stats.cells_same += 1,
                                CellState::Unimportant => out.stats.cells_unimportant += 1,
                                CellState::Important => out.stats.cells_important += 1,
                            }
                            if state != CellState::Same {
                                out.diffs.push(CellDiff {
                                    row: index,
                                    column: c,
                                    state,
                                });
                                imp = imp.max(Importance::of(state));
                            }
                        }
                        match imp {
                            Importance::None => out.stats.same_rows += 1,
                            Importance::Unimportant => out.stats.changed_unimportant += 1,
                            Importance::Important => out.stats.changed_important += 1,
                        }
                        let kind = if imp == Importance::None {
                            RowKind::Same
                        } else {
                            RowKind::Changed
                        };
                        RowPair::new(Some(l.data_row(a)), Some(r.data_row(b)), kind, imp)
                    }
                    (Some(a), None) => {
                        let imp = orphan_importance(l, a, &lcols);
                        if imp == Importance::Important {
                            out.stats.left_only_important += 1;
                        } else {
                            out.stats.left_only_unimportant += 1;
                        }
                        RowPair::new(Some(l.data_row(a)), None, RowKind::LeftOnly, imp)
                    }
                    (None, Some(b)) => {
                        let imp = orphan_importance(r, b, &rcols);
                        if imp == Importance::Important {
                            out.stats.right_only_important += 1;
                        } else {
                            out.stats.right_only_unimportant += 1;
                        }
                        RowPair::new(None, Some(r.data_row(b)), RowKind::RightOnly, imp)
                    }
                    (None, None) => RowPair::new(None, None, RowKind::Same, Importance::None),
                };
                out.rows.push(row);
            }
            Ok(out)
        })
        .collect::<Result<_, TableError>>()?;
    let mut rows = Vec::with_capacity(pairing.len());
    let mut diffs = Vec::new();
    for chunk in chunks {
        rows.extend(chunk.rows);
        diffs.extend(chunk.diffs);
        stats.add(&chunk.stats);
    }
    stats.rows = rows.len() as u64;
    let state = if stats.important_rows() > 0 || header.contains(&CellState::Important) {
        SheetState::Important
    } else if stats.unimportant_rows() > 0 || header.contains(&CellState::Unimportant) {
        SheetState::Unimportant
    } else {
        SheetState::Same
    };
    Ok((
        SheetComparison {
            left: Some(SideRef {
                sheet: li,
                first: l.first,
            }),
            right: Some(SideRef {
                sheet: ri,
                first: r.first,
            }),
            name: l.sheet.name().to_owned(),
            renamed: l.sheet.name() != r.sheet.name(),
            columns,
            rows,
            diffs,
            header,
            stats,
            state,
            alignment,
            duplicates,
        },
        warnings,
        skipped,
    ))
}
