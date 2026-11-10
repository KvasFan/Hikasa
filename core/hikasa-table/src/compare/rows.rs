//! Row pairing (contract §5.4, T-8): the keyed join and the unkeyed alignment through `hikasa-diff`.

use std::hash::{DefaultHasher, Hasher};

use hikasa_core::CancelToken;
use hikasa_diff::{
    Algorithm, AlignInput, AlignOptions, Closeness, Interner, PairScorer, RunKind, TokenId,
    TrivialSet, align,
};
use rayon::prelude::*;

use crate::compare::cells::{Interp, Mode, compare_values, encode, fast_same, interpret};
use crate::compare::columns::SideView;
use crate::compare::options::{ColumnRule, RowAlignment};
use crate::compare::result::{CellState, DuplicateKeys};
use crate::error::{TableError, TableWarning, TableWarningCode};
use crate::sheet::RowView;

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
    /// The paired columns.
    cols: &'a [Col],
    /// Sheet column → index in `cols`, per side.
    lmap: Vec<u32>,
    rmap: Vec<u32>,
    lperm: Option<&'a [u32]>,
    rperm: Option<&'a [u32]>,
}

impl RowScorer<'_> {
    fn important(&self, col: &Col, lrow: &RowView<'_>, rrow: &RowView<'_>) -> bool {
        let (Some(lc), Some(rc)) = (col.left, col.right) else {
            return false;
        };
        !fast_same(lrow.cell(lc), lrow, lc, rrow.cell(rc), rrow, rc)
            && compare_values(
                lrow.get(lc),
                rrow.get(rc),
                col.mode,
                &col.rule,
                self.l.sheet.regional(),
                self.r.sheet.regional(),
            ) == CellState::Important
    }
}

impl PairScorer for RowScorer<'_> {
    fn similarity(&self, li: u32, ri: u32) -> u16 {
        let li = self.lperm.map_or(li, |p| p[li as usize]);
        let ri = self.rperm.map_or(ri, |p| p[ri as usize]);
        let (lrow, rrow) = (self.l.row(li), self.r.row(ri));
        let total = self.cols.len() as u32;
        if total == 0 {
            return 1000;
        }
        let important = if walk_stored(&lrow, &rrow, self.cols.len()) {
            // Columns without a stored cell on either side are empty on both: alike.
            let mut touched: Vec<u32> = lrow
                .stored_cells()
                .filter_map(|(c, _)| self.lmap.get(c as usize).copied())
                .chain(
                    rrow.stored_cells()
                        .filter_map(|(c, _)| self.rmap.get(c as usize).copied()),
                )
                .filter(|&j| j != u32::MAX)
                .collect();
            touched.sort_unstable();
            touched.dedup();
            touched
                .iter()
                .filter(|&&j| self.important(&self.cols[j as usize], &lrow, &rrow))
                .count() as u32
        } else {
            self.cols
                .iter()
                .filter(|col| self.important(col, &lrow, &rrow))
                .count() as u32
        };
        ((total - important) * 1000 / total) as u16
    }
}

/// `map[c]`: the index in `cols` of sheet column `c`, or `u32::MAX`.
pub(crate) fn column_map(width: u32, cols: impl Iterator<Item = u32>) -> Vec<u32> {
    let mut map = vec![u32::MAX; width as usize];
    for (i, c) in cols.enumerate() {
        if let Some(slot) = map.get_mut(c as usize) {
            *slot = i as u32;
        }
    }
    map
}

/// True when a row pair is better walked through its stored cells than column by column (a sparse row with few
/// cells against many columns, F4).
pub(crate) fn walk_stored(a: &RowView<'_>, b: &RowView<'_>, columns: usize) -> bool {
    (a.is_sparse() || b.is_sparse())
        && (a.stored_len() as usize + b.stored_len() as usize) * 2 < columns
}

/// The token bytes of a row (§5.4): for each non-empty value in column order, the count of empty columns before
/// it and its encoding. The same bytes whether the row is dense or sparse; `true` when every value is empty.
fn token_bytes(
    view: &SideView<'_>,
    r: u32,
    cols: &[(u32, Col)],
    map: &[u32],
    out: &mut Vec<u8>,
    scratch: &mut Vec<(u32, u32)>,
) -> bool {
    let row = view.row(r);
    let regional = view.sheet.regional();
    let mut next = 0u32;
    let mut blank = true;
    let mut emit = |j: u32, c: u32, out: &mut Vec<u8>| {
        let col = &cols[j as usize].1;
        let i = interpret(row.get(c), col.mode, regional, col.rule.exact_numbers);
        if matches!(i, Interp::Empty) {
            return;
        }
        let mut gap = j - next;
        while gap >= 0x80 {
            out.push((gap as u8) | 0x80);
            gap >>= 7;
        }
        out.push(gap as u8);
        encode(&i, &col.rule, out);
        next = j + 1;
        blank = false;
    };
    if row.is_sparse() && (row.stored_len() as usize) * 2 < cols.len() {
        scratch.clear();
        for (c, _) in row.stored_cells() {
            if let Some(&j) = map.get(c as usize).filter(|&&j| j != u32::MAX) {
                scratch.push((j, c));
            }
        }
        scratch.sort_unstable();
        for &(j, c) in scratch.iter() {
            emit(j, c, out);
        }
    } else {
        for (j, (c, _)) in cols.iter().enumerate() {
            emit(j as u32, *c, out);
        }
    }
    blank
}

fn row_hashes(
    view: &SideView<'_>,
    cols: &[(u32, Col)],
    cancel: &CancelToken,
) -> Result<Vec<(u64, bool)>, TableError> {
    let rows: Vec<u32> = (0..view.rows).collect();
    let map = column_map(view.width, cols.iter().map(|(c, _)| *c));
    let chunks: Vec<Vec<(u64, bool)>> = rows
        .par_chunks(CHUNK)
        .map(|chunk| {
            cancel.check()?;
            let mut buf = Vec::with_capacity(64);
            let mut scratch = Vec::new();
            Ok(chunk
                .iter()
                .map(|&r| {
                    buf.clear();
                    let blank = token_bytes(view, r, cols, &map, &mut buf, &mut scratch);
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

/// Edits a Myers pass may make in one degraded range (its trace is O(edits²)).
const MAX_EDITS: i64 = 2048;

/// Myers' greedy O((N+M)·D) alignment of `a` and `b` (review F9) within `MAX_EDITS` edits and `budget`
/// operations: the matched blocks `(x, y, len)` in order, `None` past a limit.
fn bounded_myers(
    a: &[TokenId],
    b: &[TokenId],
    budget: &mut u64,
    cancel: &CancelToken,
) -> Result<Option<Vec<(u32, u32, u32)>>, TableError> {
    let (n, m) = (a.len() as i64, b.len() as i64);
    let max_d = (n + m).min(MAX_EDITS);
    let off = max_d + 1;
    let mut v = vec![0i64; (2 * off + 1) as usize];
    let mut trace: Vec<Vec<u32>> = Vec::new();
    let at = |k: i64| (k + off) as usize;
    for d in 0..=max_d {
        if d % 256 == 0 {
            cancel.check()?;
        }
        let mut spent = 0u64;
        let mut end = None;
        for k in (-d..=d).step_by(2) {
            let down = k == -d || (k != d && v[at(k - 1)] < v[at(k + 1)]);
            let mut x = if down { v[at(k + 1)] } else { v[at(k - 1)] + 1 };
            let mut y = x - k;
            while x < n && y < m && y >= 0 && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
                spent += 1;
            }
            v[at(k)] = x;
            spent += 1;
            if x >= n && y >= m {
                end = Some((x, y));
                break;
            }
        }
        if spent > *budget {
            *budget = 0;
            return Ok(None);
        }
        *budget -= spent;
        if let Some((mut x, mut y)) = end {
            // Back from the end: each edit's snake is a matched block (kept within the sequences).
            let mut blocks = Vec::new();
            for dd in (1..=d).rev() {
                let prev = &trace[(dd - 1) as usize];
                let get = |kk: i64| i64::from(prev[(kk + dd - 1) as usize]);
                let k = x - y;
                let down = k == -dd || (k != dd && get(k - 1) < get(k + 1));
                let pk = if down { k + 1 } else { k - 1 };
                let px = get(pk);
                let start = if down { px } else { px + 1 };
                let (sx, sy) = (start, start - k);
                let (ex, ey) = (x.min(n), y.min(m));
                let len = (ex - sx).min(ey - sy);
                if len > 0 && sx >= 0 && sy >= 0 {
                    blocks.push((sx as u32, sy as u32, len as u32));
                }
                x = px;
                y = px - pk;
            }
            let len = x.min(n).min(y.min(m));
            if len > 0 {
                blocks.push((0, 0, len as u32));
            }
            blocks.reverse();
            return Ok(Some(blocks));
        }
        trace.push(
            v[at(-d)..=at(d)]
                .iter()
                .map(|&x| x.clamp(0, i64::from(u32::MAX)) as u32)
                .collect(),
        );
    }
    Ok(None)
}

/// A scorer of a sub-range: indices shifted to the whole sequences.
struct Shifted<'s> {
    inner: &'s dyn PairScorer,
    left: u32,
    right: u32,
}

impl PairScorer for Shifted<'_> {
    fn similarity(&self, l: u32, r: u32) -> u16 {
        self.inner.similarity(self.left + l, self.right + r)
    }
}

/// The inputs of a re-alignment.
struct Realign<'a> {
    left: &'a [TokenId],
    right: &'a [TokenId],
    trivial: &'a TrivialSet,
    distinct: u32,
    opts: &'a AlignOptions,
    scorer: &'a dyn PairScorer,
}

impl Realign<'_> {
    /// Aligns left `l` and right `r` with the original options and appends the rows.
    fn gap(
        &self,
        l: std::ops::Range<u32>,
        r: std::ops::Range<u32>,
        out: &mut Vec<RowRef>,
        cancel: &CancelToken,
    ) -> Result<(), TableError> {
        if l.is_empty() || r.is_empty() {
            out.extend(l.map(|x| (Some(x), None)));
            out.extend(r.map(|y| (None, Some(y))));
            return Ok(());
        }
        let input = AlignInput {
            left: &self.left[l.start as usize..l.end as usize],
            right: &self.right[r.start as usize..r.end as usize],
            left_weights: &[],
            right_weights: &[],
            trivial: self.trivial,
            distinct: self.distinct,
        };
        let scorer = Shifted {
            inner: self.scorer,
            left: l.start,
            right: r.start,
        };
        let a = align(&input, self.opts, Some(&scorer), None, cancel).map_err(|e| match e {
            hikasa_diff::DiffError::Cancelled => TableError::Cancelled,
            other => TableError::Align(other.to_string()),
        })?;
        out.extend(a.rows().map(|row| {
            (
                row.left.map(|x| x + l.start),
                row.right.map(|y| y + r.start),
            )
        }));
        Ok(())
    }

    /// Standard's degraded ranges again (review F9): every range of non-equal runs goes through a bounded Myers
    /// pass, whose matched blocks pair as equal rows and whose gaps align with the original options; a range
    /// Myers cannot do within its budget keeps Standard's rows.
    fn run(
        &self,
        alignment: &hikasa_diff::Alignment,
        cancel: &CancelToken,
    ) -> Result<Vec<RowRef>, TableError> {
        let runs = alignment.runs();
        let mut out: Vec<RowRef> = Vec::with_capacity(alignment.row_count() as usize);
        let mut budget = AlignOptions::fallback_budget_for(self.left.len(), self.right.len());
        let mut i = 0;
        while i < runs.len() {
            if runs[i].kind == RunKind::Equal {
                out.extend(
                    runs[i]
                        .left
                        .clone()
                        .zip(runs[i].right.clone())
                        .map(|(x, y)| (Some(x), Some(y))),
                );
                i += 1;
                continue;
            }
            let mut j = i;
            while j < runs.len() && runs[j].kind != RunKind::Equal {
                j += 1;
            }
            let (l0, l1) = (runs[i].left.start, runs[j - 1].left.end);
            let (r0, r1) = (runs[i].right.start, runs[j - 1].right.end);
            let blocks = if budget > 0 && l1 > l0 && r1 > r0 {
                bounded_myers(
                    &self.left[l0 as usize..l1 as usize],
                    &self.right[r0 as usize..r1 as usize],
                    &mut budget,
                    cancel,
                )?
            } else {
                None
            };
            match blocks.filter(|b| !b.is_empty()) {
                Some(blocks) => {
                    let (mut x, mut y) = (l0, r0);
                    for (bx, by, len) in blocks {
                        let (bx, by) = (bx + l0, by + r0);
                        self.gap(x..bx, y..by, &mut out, cancel)?;
                        out.extend((0..len).map(|t| (Some(bx + t), Some(by + t))));
                        (x, y) = (bx + len, by + len);
                    }
                    self.gap(x..l1, y..r1, &mut out, cancel)?;
                }
                None => {
                    for run in &runs[i..j] {
                        out.extend(run_rows(run));
                    }
                }
            }
            i = j;
        }
        Ok(out)
    }
}

/// The rows of one run.
fn run_rows(run: &hikasa_diff::Run) -> Vec<RowRef> {
    let (l, r) = (run.left.clone(), run.right.clone());
    match run.kind {
        RunKind::Equal | RunKind::Changed => l.zip(r).map(|(x, y)| (Some(x), Some(y))).collect(),
        RunKind::Delete => l.map(|x| (Some(x), None)).collect(),
        RunKind::Insert => r.map(|y| (None, Some(y))).collect(),
    }
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
        lmap: column_map(l.width, paired.iter().filter_map(|c| c.left)),
        rmap: column_map(r.width, paired.iter().filter_map(|c| c.right)),
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
    let degraded = opts.algorithm == Algorithm::Standard
        && warnings
            .iter()
            .any(|w| w.code == TableWarningCode::AlignmentDegraded);
    let rows: Vec<RowRef> = if degraded {
        Realign {
            left: &ltok,
            right: &rtok,
            trivial: &trivial,
            distinct,
            opts: &align_opts,
            scorer: &scorer,
        }
        .run(&alignment, cancel)?
    } else {
        alignment.rows().map(|row| (row.left, row.right)).collect()
    };
    let map = |perm: &Option<Vec<u32>>, i: u32| perm.as_ref().map_or(i, |p| p[i as usize]);
    let rows = rows
        .into_iter()
        .map(|(l, r)| (l.map(|i| map(&lperm, i)), r.map(|i| map(&rperm, i))))
        .collect();
    Ok((rows, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn lcs(a: &[TokenId], b: &[TokenId]) -> u32 {
        let mut dp = vec![vec![0u32; b.len() + 1]; a.len() + 1];
        for i in (0..a.len()).rev() {
            for j in (0..b.len()).rev() {
                dp[i][j] = if a[i] == b[j] {
                    dp[i + 1][j + 1] + 1
                } else {
                    dp[i + 1][j].max(dp[i][j + 1])
                };
            }
        }
        dp[0][0]
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: ProptestConfig::default().cases.max(512), ..ProptestConfig::default() })]

        /// Myers' blocks are valid matched pairs, in order, as many as a longest common subsequence.
        #[test]
        fn p_myers_blocks_are_a_longest_common_subsequence(
            a in prop::collection::vec(0u32..4, 0..40),
            b in prop::collection::vec(0u32..4, 0..40),
        ) {
            let (a, b): (Vec<TokenId>, Vec<TokenId>) = (a.into_iter().map(TokenId).collect(), b.into_iter().map(TokenId).collect());
            let mut budget = u64::MAX;
            let blocks = bounded_myers(&a, &b, &mut budget, &CancelToken::new()).unwrap().unwrap();
            let (mut x, mut y) = (0u32, 0u32);
            let mut matched = 0;
            for (bx, by, len) in blocks {
                prop_assert!(bx >= x && by >= y && len > 0);
                for t in 0..len {
                    prop_assert_eq!(a[(bx + t) as usize], b[(by + t) as usize]);
                }
                (x, y) = (bx + len, by + len);
                matched += len;
            }
            prop_assert!(x as usize <= a.len() && y as usize <= b.len());
            prop_assert_eq!(matched, lcs(&a, &b));
        }
    }

    #[test]
    fn myers_gives_up_past_its_budget() {
        let a: Vec<TokenId> = (0..1000).map(TokenId).collect();
        let b: Vec<TokenId> = (1000..2000).map(TokenId).collect();
        let mut budget = 10_000;
        assert_eq!(
            bounded_myers(&a, &b, &mut budget, &CancelToken::new()).unwrap(),
            None
        );
        assert_eq!(budget, 0);
        let shifted: Vec<TokenId> = (1..1001).map(TokenId).collect();
        let mut budget = 100_000;
        assert_eq!(
            bounded_myers(&a, &shifted, &mut budget, &CancelToken::new()).unwrap(),
            Some(vec![(1, 0, 999)])
        );
    }
}
