//! Row pairing (contract §5.4, T-8): the keyed join and the unkeyed alignment through `hikasa-diff`.

use std::hash::{DefaultHasher, Hasher};

use hikasa_core::CancelToken;
use hikasa_diff::{AlignInput, AlignOptions, Closeness, Interner, PairScorer, TrivialSet, align};
use rayon::prelude::*;

use crate::compare::cells::{Interp, Mode, compare_values, encode, fast_same, interpret};
use crate::compare::columns::SideView;
use crate::compare::options::{ColumnRule, RowAlignment};
use crate::compare::result::{CellState, DuplicateKeys};
use crate::error::{TableError, TableWarning, TableWarningCode};

/// Rows per parallel chunk and per cancellation check.
pub(crate) const CHUNK: usize = 65_536;

/// One comparison column as the row code needs it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Col {
    pub left: Option<u32>,
    pub right: Option<u32>,
    pub rule: ColumnRule,
    pub mode: Mode,
}

/// A pairing: left and right data rows.
pub(crate) type RowRef = (Option<u32>, Option<u32>);

/// Appends the encoding of `view`'s row `r` over `cols` (one side) to `out`; returns true when every value
/// was empty.
fn encode_row(view: &SideView<'_>, r: u32, cols: &[(u32, Col)], out: &mut Vec<u8>) -> bool {
    let row = view.row(r);
    let regional = view.sheet.regional();
    let mut blank = true;
    for (c, col) in cols {
        let i = interpret(row.get(*c), col.mode, regional, col.rule.exact_numbers);
        if !matches!(i, Interp::Empty) {
            blank = false;
        }
        encode(&i, &col.rule, out);
    }
    blank
}

/// Keys of every row of a side: an arena and offsets (`offsets[r]..offsets[r + 1]`).
struct Keys {
    arena: Vec<u8>,
    offsets: Vec<usize>,
}

impl Keys {
    fn get(&self, r: u32) -> &[u8] {
        &self.arena[self.offsets[r as usize]..self.offsets[r as usize + 1]]
    }
}

fn keys_of(
    view: &SideView<'_>,
    cols: &[(u32, Col)],
    cancel: &CancelToken,
) -> Result<Keys, TableError> {
    let chunks: Vec<(Vec<u8>, Vec<usize>)> = (0..view.rows)
        .collect::<Vec<u32>>()
        .par_chunks(CHUNK)
        .map(|chunk| {
            cancel.check()?;
            let mut arena = Vec::with_capacity(chunk.len() * 12);
            let mut ends = Vec::with_capacity(chunk.len());
            for &r in chunk {
                encode_row(view, r, cols, &mut arena);
                ends.push(arena.len());
            }
            Ok((arena, ends))
        })
        .collect::<Result<_, TableError>>()?;
    let total: usize = chunks.iter().map(|(a, _)| a.len()).sum();
    let mut arena = Vec::with_capacity(total);
    let mut offsets = Vec::with_capacity(view.rows as usize + 1);
    offsets.push(0);
    for (a, ends) in chunks {
        let base = arena.len();
        arena.extend_from_slice(&a);
        offsets.extend(ends.into_iter().map(|e| base + e));
    }
    Ok(Keys { arena, offsets })
}

fn sorted_order(keys: &Keys, n: u32) -> Vec<u32> {
    let mut idx: Vec<u32> = (0..n).collect();
    idx.par_sort_by(|a, b| keys.get(*a).cmp(keys.get(*b)));
    idx
}

/// The keyed join (T-8): key order, duplicates in order of appearance.
pub(crate) fn keyed(
    l: &SideView<'_>,
    r: &SideView<'_>,
    key_cols: &[Col],
    cancel: &CancelToken,
) -> Result<(Vec<RowRef>, DuplicateKeys), TableError> {
    let lcols: Vec<(u32, Col)> = key_cols
        .iter()
        .filter_map(|c| c.left.map(|x| (x, *c)))
        .collect();
    let rcols: Vec<(u32, Col)> = key_cols
        .iter()
        .filter_map(|c| c.right.map(|x| (x, *c)))
        .collect();
    let (lk, rk) = rayon::join(|| keys_of(l, &lcols, cancel), || keys_of(r, &rcols, cancel));
    let (lk, rk) = (lk?, rk?);
    cancel.check()?;
    let (lo, ro) = rayon::join(|| sorted_order(&lk, l.rows), || sorted_order(&rk, r.rows));
    cancel.check()?;
    let mut out = Vec::with_capacity(lo.len().max(ro.len()));
    let mut dups = DuplicateKeys::default();
    let example =
        |view: &SideView<'_>, cols: &[(u32, Col)], rows: &[u32]| -> (Vec<String>, Vec<u64>) {
            let row = view.row(rows[0]);
            (
                cols.iter()
                    .map(|(c, _)| row.get(*c).display().into_owned())
                    .collect(),
                rows.iter()
                    .take(10)
                    .map(|&x| view.sheet.stored_label(x + view.first))
                    .collect(),
            )
        };
    let (mut i, mut j) = (0usize, 0usize);
    while i < lo.len() || j < ro.len() {
        if (i + j) % CHUNK == 0 {
            cancel.check()?;
        }
        let ord = match (lo.get(i), ro.get(j)) {
            (Some(a), Some(b)) => lk.get(*a).cmp(rk.get(*b)),
            (Some(_), None) => std::cmp::Ordering::Less,
            _ => std::cmp::Ordering::Greater,
        };
        // The group of equal keys on each side.
        let key: &[u8] = match ord {
            std::cmp::Ordering::Greater => rk.get(ro[j]),
            _ => lk.get(lo[i]),
        };
        let gl = if ord != std::cmp::Ordering::Greater {
            lo[i..].iter().take_while(|&&x| lk.get(x) == key).count()
        } else {
            0
        };
        let gr = if ord != std::cmp::Ordering::Less {
            ro[j..].iter().take_while(|&&x| rk.get(x) == key).count()
        } else {
            0
        };
        if gl > 1 {
            dups.left_rows += gl as u64;
            if dups.left_examples.len() < 10 {
                dups.left_examples.push(example(l, &lcols, &lo[i..i + gl]));
            }
        }
        if gr > 1 {
            dups.right_rows += gr as u64;
            if dups.right_examples.len() < 10 {
                dups.right_examples.push(example(r, &rcols, &ro[j..j + gr]));
            }
        }
        for k in 0..gl.max(gr) {
            out.push(((k < gl).then(|| lo[i + k]), (k < gr).then(|| ro[j + k])));
        }
        i += gl;
        j += gr;
    }
    Ok((out, dups))
}

/// Similarity of two rows per mille: the share of paired columns whose cells are not importantly different.
struct RowScorer<'a> {
    l: &'a SideView<'a>,
    r: &'a SideView<'a>,
    cols: &'a [Col],
    lperm: Option<&'a [u32]>,
    rperm: Option<&'a [u32]>,
}

impl PairScorer for RowScorer<'_> {
    fn similarity(&self, li: u32, ri: u32) -> u16 {
        let li = self.lperm.map_or(li, |p| p[li as usize]);
        let ri = self.rperm.map_or(ri, |p| p[ri as usize]);
        let (lrow, rrow) = (self.l.row(li), self.r.row(ri));
        let mut total = 0u32;
        let mut alike = 0u32;
        for col in self.cols {
            let (Some(lc), Some(rc)) = (col.left, col.right) else {
                continue;
            };
            total += 1;
            let same = fast_same(lrow.cell(lc), &lrow, lc, rrow.cell(rc), &rrow, rc)
                || compare_values(
                    lrow.get(lc),
                    rrow.get(rc),
                    col.mode,
                    &col.rule,
                    self.l.sheet.regional(),
                    self.r.sheet.regional(),
                ) != CellState::Important;
            if same {
                alike += 1;
            }
        }
        if total == 0 {
            return 1000;
        }
        (alike * 1000 / total) as u16
    }
}

fn row_hashes(
    view: &SideView<'_>,
    cols: &[(u32, Col)],
    cancel: &CancelToken,
) -> Result<Vec<(u64, bool)>, TableError> {
    let rows: Vec<u32> = (0..view.rows).collect();
    let chunks: Vec<Vec<(u64, bool)>> = rows
        .par_chunks(CHUNK)
        .map(|chunk| {
            cancel.check()?;
            let mut buf = Vec::with_capacity(64);
            Ok(chunk
                .iter()
                .map(|&r| {
                    buf.clear();
                    let blank = encode_row(view, r, cols, &mut buf);
                    let mut h = DefaultHasher::new();
                    h.write(&buf);
                    (h.finish(), blank)
                })
                .collect())
        })
        .collect::<Result<_, TableError>>()?;
    Ok(chunks.into_iter().flatten().collect())
}

fn warn_code(code: &str) -> Option<TableWarningCode> {
    Some(match code {
        "alignment_degraded" => TableWarningCode::AlignmentDegraded,
        "work_budget_exhausted" => TableWarningCode::WorkBudgetExhausted,
        "closeness_budget_exhausted" => TableWarningCode::ClosenessBudgetExhausted,
        "closeness_ignored" => TableWarningCode::ClosenessIgnored,
        _ => return None,
    })
}

/// Unkeyed alignment through `hikasa_diff::align` (§5.4).
pub(crate) fn unkeyed(
    l: &SideView<'_>,
    r: &SideView<'_>,
    cols: &[Col],
    opts: &RowAlignment,
    cancel: &CancelToken,
) -> Result<(Vec<RowRef>, Vec<TableWarning>), TableError> {
    let paired: Vec<Col> = cols
        .iter()
        .filter(|c| c.left.is_some() && c.right.is_some())
        .copied()
        .collect();
    let token_cols: Vec<Col> = paired
        .iter()
        .filter(|c| !c.rule.unimportant)
        .copied()
        .collect();
    let lcols: Vec<(u32, Col)> = token_cols
        .iter()
        .filter_map(|c| c.left.map(|x| (x, *c)))
        .collect();
    let rcols: Vec<(u32, Col)> = token_cols
        .iter()
        .filter_map(|c| c.right.map(|x| (x, *c)))
        .collect();
    // Optional sort by the full typed rows.
    let (lperm, rperm) = if opts.sort_rows {
        let lall: Vec<(u32, Col)> = paired
            .iter()
            .filter_map(|c| c.left.map(|x| (x, *c)))
            .collect();
        let rall: Vec<(u32, Col)> = paired
            .iter()
            .filter_map(|c| c.right.map(|x| (x, *c)))
            .collect();
        let (lk, rk) = rayon::join(|| keys_of(l, &lall, cancel), || keys_of(r, &rall, cancel));
        let (lk, rk) = (lk?, rk?);
        (
            Some(sorted_order(&lk, l.rows)),
            Some(sorted_order(&rk, r.rows)),
        )
    } else {
        (None, None)
    };
    let (lh, rh) = rayon::join(
        || row_hashes(l, &lcols, cancel),
        || row_hashes(r, &rcols, cancel),
    );
    let (lh, rh) = (lh?, rh?);
    cancel.check()?;
    let order = |perm: &Option<Vec<u32>>, n: usize| -> Vec<usize> {
        match perm {
            Some(p) => p.iter().map(|&x| x as usize).collect(),
            None => (0..n).collect(),
        }
    };
    let (lord, rord) = (order(&lperm, lh.len()), order(&rperm, rh.len()));
    let mut interner: Interner<u64> = Interner::with_capacity(lh.len().max(rh.len()).min(1 << 20));
    let mut trivial_hashes = Vec::new();
    let mut tokens =
        |hashes: &[(u64, bool)], ord: &[usize]| -> Result<Vec<hikasa_diff::TokenId>, TableError> {
            ord.iter()
                .map(|&i| {
                    let (h, blank) = hashes[i];
                    let id = interner
                        .intern(h)
                        .map_err(|e| TableError::Align(e.to_string()))?;
                    if blank {
                        trivial_hashes.push(id);
                    }
                    Ok(id)
                })
                .collect()
        };
    let ltok = tokens(&lh, &lord)?;
    let rtok = tokens(&rh, &rord)?;
    let distinct = interner.len();
    let mut trivial = TrivialSet::with_capacity(distinct);
    for id in trivial_hashes {
        trivial.insert(id);
    }
    let input = AlignInput {
        left: &ltok,
        right: &rtok,
        left_weights: &[],
        right_weights: &[],
        trivial: &trivial,
        distinct,
    };
    let align_opts = AlignOptions {
        algorithm: opts.algorithm,
        skew_tolerance: opts.skew,
        closeness: opts.closeness.map(|t| Closeness {
            threshold_permille: t.min(1000),
            ..Closeness::DEFAULT
        }),
        never_align_differences: opts.never_align,
        work_budget: None,
    };
    let scorer = RowScorer {
        l,
        r,
        cols: &paired,
        lperm: lperm.as_deref(),
        rperm: rperm.as_deref(),
    };
    let alignment =
        align(&input, &align_opts, Some(&scorer), None, cancel).map_err(|e| match e {
            hikasa_diff::DiffError::Cancelled => TableError::Cancelled,
            other => TableError::Align(other.to_string()),
        })?;
    let mut warnings = Vec::new();
    for w in alignment.warnings() {
        if let Some(code) = warn_code(w.code.as_str()) {
            warnings.push(TableWarning::new(code, w.message.clone()));
        }
    }
    for w in alignment.diff_warnings() {
        if let Some(code) = warn_code(w.code.as_str()) {
            warnings.push(TableWarning::new(code, w.message.clone()));
        }
    }
    let map = |perm: &Option<Vec<u32>>, i: u32| perm.as_ref().map_or(i, |p| p[i as usize]);
    let rows = alignment
        .rows()
        .map(|row| {
            (
                row.left.map(|i| map(&lperm, i)),
                row.right.map(|i| map(&rperm, i)),
            )
        })
        .collect();
    Ok((rows, warnings))
}
