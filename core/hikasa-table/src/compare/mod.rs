//! The comparison (contract §5): [`compare_workbooks`], [`compare_specs`], [`TableComparator`] and the
//! navigation data of [`nav`].

mod cells;
mod columns;
pub mod nav;
pub mod options;
pub mod result;
mod rows;
mod sheet;

use std::sync::Arc;

use hikasa_core::{
    CancelToken, Cancelled, ContentComparator, ContentMethod, ContentOutcome, ContentResult,
    ContentSource, Ctl, ErrorReport, Side, Spec,
};
use hikasa_vfs::Vfs;
use rayon::prelude::*;

use crate::error::{TableError, TableWarning};
use crate::format::TableFormat;
use crate::load::{LoadOptions, load_bytes, read_spec, spec_name};
use crate::sheet::Workbook;

use columns::{pair_sheets, side_views};
use options::{SheetRules, TableCompareOptions};
use result::{SheetState, TableComparison, TableStats};
use sheet::{PairInput, compare_pair};

/// The rules of a sheet pair: the first matching `sheet_rules` entry (left name or position, then right).
fn rules_for<'a>(
    opts: &'a TableCompareOptions,
    left: Option<(usize, &str)>,
    right: Option<(usize, &str)>,
) -> &'a SheetRules {
    for (sel, rules) in &opts.sheet_rules {
        let hit = |side: Option<(usize, &str)>| {
            side.is_some_and(|(i, name)| match sel {
                options::SheetSelector::Position(n) => *n as usize == i + 1,
                options::SheetSelector::Name(n) => {
                    n == name || hikasa_core::case::eq_folded(n, name)
                }
            })
        };
        if hit(left) || hit(right) {
            return rules;
        }
    }
    &opts.default_sheet
}

/// Compares two loaded workbooks (contract §5.6, T-17): sheet pairs in parallel, results in pair order. A workbook
/// the comparison owns alone (no other `Arc` to it) is compacted first (F7, [`Workbook::compact`]): its text sheets
/// drop their packed cells, and the comparison, the reports and the grid split their rows again when they read them.
pub fn compare_workbooks(
    mut left: Arc<Workbook>,
    mut right: Arc<Workbook>,
    opts: &TableCompareOptions,
    cancel: &CancelToken,
) -> Result<TableComparison, TableError> {
    // Before the comparison, so its own allocations never sit on top of the packed cells (the peak).
    rayon::join(
        || Arc::get_mut(&mut left).map(Workbook::compact),
        || Arc::get_mut(&mut right).map(Workbook::compact),
    );
    let lnames: Vec<&str> = left.sheets.iter().map(|s| s.name()).collect();
    let rnames: Vec<&str> = right.sheets.iter().map(|s| s.name()).collect();
    let pairs = pair_sheets(&lnames, &rnames, &opts.sheets)?;
    let results: Vec<sheet::PairOutput> = pairs
        .par_iter()
        .map(|&(li, ri)| {
            cancel.check()?;
            let ls = li.map(|i| &left.sheets[i]);
            let rs = ri.map(|i| &right.sheets[i]);
            let rules = rules_for(opts, li.map(|i| (i, lnames[i])), ri.map(|i| (i, rnames[i])));
            let (lv, rv) = match (ls, rs) {
                (Some(a), Some(b)) => {
                    let (x, y) = side_views(a, b, rules);
                    (Some(x), Some(y))
                }
                (Some(a), None) => (Some(side_views(a, a, rules).0), None),
                (None, Some(b)) => (None, Some(side_views(b, b, rules).1)),
                (None, None) => (None, None),
            };
            compare_pair(
                PairInput {
                    left: li.zip(lv).map(|(i, v)| (i as u32, v)),
                    right: ri.zip(rv).map(|(i, v)| (i as u32, v)),
                    rules,
                    compare_formulas: opts.compare_formulas,
                },
                cancel,
            )
        })
        .collect::<Result<_, TableError>>()?;
    cancel.check()?;
    // T-30: a lenient reference (of the rules of every sheet, or inherited from them by per-sheet rules) that names a
    // column of no paired sheet it applies to is an error (a typo is never silently a rule that applies nowhere).
    for out in &results {
        for (reference, e) in &out.skipped {
            if !results.iter().any(|o| o.resolved.contains(reference)) {
                return Err(TableError::InvalidInput(e.to_string()));
            }
        }
    }
    let mut stats = TableStats::default();
    let mut warnings: Vec<TableWarning> = Vec::new();
    for (side, wb) in [(Side::Left, &left), (Side::Right, &right)] {
        warnings.extend(wb.warnings.iter().cloned().map(|mut w| {
            w.side = Some(side);
            w
        }));
    }
    let mut sheets = Vec::with_capacity(results.len());
    for sheet::PairOutput {
        comparison: s,
        warnings: w,
        ..
    } in results
    {
        stats.totals.add(s.stats());
        match s.state() {
            SheetState::LeftOnly => stats.sheets_left_only += 1,
            SheetState::RightOnly => stats.sheets_right_only += 1,
            _ => stats.sheets_paired += 1,
        }
        if s.state() != SheetState::Same {
            stats.sheets_differing += 1;
        }
        warnings.extend(w);
        sheets.push(s);
    }
    Ok(TableComparison {
        left,
        right,
        sheets,
        stats,
        warnings,
    })
}

/// What [`compare_specs`] loads with.
#[derive(Debug, Clone, Default)]
pub struct LoadRequest {
    /// The left side's format (`None`: the preset of its file name).
    pub left: Option<TableFormat>,
    /// The right side's format (`None`: the left's when given, else the preset of its file name).
    pub right: Option<TableFormat>,
    /// Load options.
    pub options: LoadOptions,
}

/// A comparison of two specs with the facts the verdict needs.
#[derive(Debug, Clone)]
pub struct SpecComparison {
    /// The comparison.
    pub comparison: TableComparison,
    /// Both inputs had identical bytes.
    pub bytes_identical: bool,
}

/// Loads both specs through the VFS in parallel and compares them (T-4, T-17).
pub fn compare_specs(
    vfs: &(dyn Vfs + Sync),
    left: &Spec,
    right: &Spec,
    request: &LoadRequest,
    opts: &TableCompareOptions,
    ctl: &Ctl<'_>,
) -> Result<SpecComparison, TableError> {
    let max = request.options.effective_max_bytes();
    let read =
        |spec: &Spec, side: Side| read_spec(vfs, spec, max, ctl).map_err(|e| e.with_side(side));
    let (lb, rb) = rayon::join(|| read(left, Side::Left), || read(right, Side::Right));
    let (lb, rb) = (lb?, rb?);
    let bytes_identical = lb == rb;
    let (ln, rn) = (spec_name(left), spec_name(right));
    let lf = request
        .left
        .clone()
        .unwrap_or_else(|| TableFormat::from_name(&ln));
    let rf = request
        .right
        .clone()
        .or_else(|| request.left.clone())
        .unwrap_or_else(|| TableFormat::from_name(&rn));
    let (lw, rw) = rayon::join(
        || {
            load_bytes(lb, &ln, &lf, &request.options, ctl.cancel)
                .map_err(|e| e.with_side(Side::Left))
        },
        || {
            load_bytes(rb, &rn, &rf, &request.options, ctl.cancel)
                .map_err(|e| e.with_side(Side::Right))
        },
    );
    let comparison = compare_workbooks(Arc::new(lw?), Arc::new(rw?), opts, ctl.cancel)?;
    Ok(SpecComparison {
        comparison,
        bytes_identical,
    })
}

/// Chooses the table format of a file name (the folder content test's format resolver).
pub type FormatResolver = dyn Fn(&str) -> Option<TableFormat> + Send + Sync;

/// The Rules content comparator of Table-kind formats (T-21).
pub struct TableComparator {
    /// Format of a file name (`None`: the preset of the name).
    pub formats: Arc<FormatResolver>,
    /// Comparison options.
    pub options: TableCompareOptions,
    /// Load options.
    pub load: LoadOptions,
    /// Largest file loaded (default 512 MiB).
    pub limit: u64,
}

impl TableComparator {
    /// A comparator with presets by name, default options and a 512 MiB limit.
    pub fn new() -> TableComparator {
        TableComparator {
            formats: Arc::new(|_: &str| None),
            options: TableCompareOptions::default(),
            load: LoadOptions::default(),
            limit: 512 << 20,
        }
    }

    fn run(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        cancel: &CancelToken,
    ) -> Result<Option<TableComparison>, TableError> {
        let read = |s: &dyn ContentSource| {
            s.read_all(self.limit, cancel).map_err(|e| match e {
                hikasa_core::ReadError::Cancelled => TableError::Cancelled,
                other => TableError::Corrupt {
                    name: s.display(),
                    message: other.to_string(),
                },
            })
        };
        let (lb, rb) = (read(left)?, read(right)?);
        if lb == rb {
            // Identical bytes are the same file whatever it holds, even one that does not load (F7).
            return Ok(None);
        }
        let format =
            |name: &str| (self.formats)(name).unwrap_or_else(|| TableFormat::from_name(name));
        let lw = load_bytes(lb, left.name(), &format(left.name()), &self.load, cancel)?;
        let rw = load_bytes(rb, right.name(), &format(right.name()), &self.load, cancel)?;
        Ok(Some(compare_workbooks(
            Arc::new(lw),
            Arc::new(rw),
            &self.options,
            cancel,
        )?))
    }
}

impl Default for TableComparator {
    fn default() -> TableComparator {
        TableComparator::new()
    }
}

impl ContentComparator for TableComparator {
    fn method(&self) -> ContentMethod {
        ContentMethod::Rules
    }

    fn size_limit(&self) -> Option<u64> {
        Some(self.limit)
    }

    fn compare(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        ctl: &Ctl<'_>,
    ) -> Result<ContentOutcome, Cancelled> {
        let result = match self.run(left, right, ctl.cancel) {
            Ok(None) => ContentResult::BinarySame,
            Ok(Some(cmp)) => match cmp.kind(false, false) {
                hikasa_core::ContentKind::BinarySame => ContentResult::BinarySame,
                hikasa_core::ContentKind::RulesSame => ContentResult::RulesSame,
                hikasa_core::ContentKind::UnimportantOnly => ContentResult::UnimportantOnly,
                _ => ContentResult::ImportantDifferences,
            },
            Err(TableError::Cancelled) => return Err(Cancelled),
            Err(e) => ContentResult::Error {
                error: ErrorReport::capture(&e, None),
            },
        };
        Ok(ContentOutcome {
            method: ContentMethod::Rules,
            result,
            warnings: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Cell;
    use crate::format::FirstLine;
    use crate::regional::Regional;
    use crate::sheet::{SheetBuilder, SourceKind};
    use proptest::prelude::*;

    /// A row: cells at increasing columns (`kind`, `column gap`).
    type Rows = Vec<Vec<(u8, u8)>>;

    fn build(rows: &Rows, sparse: bool) -> Arc<Workbook> {
        let mut b = SheetBuilder::new("S", String::new(), Regional::default());
        if sparse {
            b.set_sparse();
        }
        for row in rows {
            let mut col = 0u32;
            for &(kind, gap) in row {
                col += u32::from(gap);
                let cell = match kind % 6 {
                    0 => b.text_cell("x").unwrap(),
                    1 => b.text_cell("X").unwrap(),
                    2 => Cell::number(1.0),
                    3 => b.text_cell("1.0").unwrap(),
                    4 => b.text_cell(" ").unwrap(),
                    _ => Cell::bool(true),
                };
                b.push_at(col, cell).unwrap();
                col += 1;
            }
            b.end_row(None);
        }
        let sheet = b.finish(FirstLine::Data);
        Arc::new(Workbook::from_sheets(vec![sheet], SourceKind::Xlsx))
    }

    fn rows() -> impl Strategy<Value = Rows> {
        prop::collection::vec(
            prop::collection::vec((0u8..6, prop_oneof![Just(0u8), 5u8..40]), 0..4),
            0..12,
        )
    }

    fn outcome(c: &TableComparison) -> (Vec<RowPairSummary>, String) {
        let s = &c.sheets()[0];
        (
            s.rows()
                .iter()
                .map(|r| (r.left(), r.right(), r.kind(), r.importance()))
                .collect(),
            format!(
                "{:?} {:?} {:?}",
                s.all_differing_cells(),
                s.stats(),
                c.kind(false, false)
            ),
        )
    }

    #[test]
    fn p_sparse_builds_sparse_sheets() {
        let rows: Rows = vec![vec![(0, 0), (2, 30)], vec![(1, 20)]];
        let s = build(&rows, true);
        assert!(s.sheets()[0].is_sparse());
        assert_eq!(s.sheets()[0].cell(0, 31), crate::Value::Number(1.0));
        // Few gaps: the dense form is smaller and wins.
        let d = build(&vec![vec![(0, 0), (1, 1), (2, 0), (3, 0)]], true);
        assert!(!d.sheets()[0].is_sparse());
    }

    type RowPairSummary = (
        Option<u32>,
        Option<u32>,
        result::RowKind,
        result::Importance,
    );

    proptest! {
        #![proptest_config(ProptestConfig { cases: ProptestConfig::default().cases.max(256), ..ProptestConfig::default() })]

        /// P-SPARSE: a sparse sheet compares exactly like the same rows stored densely (F4), keyed and unkeyed,
        /// on either side.
        #[test]
        fn p_sparse(a in rows(), b in rows(), keyed in any::<bool>()) {
            let (sa, da, db) = (build(&a, true), build(&a, false), build(&b, false));
            prop_assert!(!da.sheets()[0].is_sparse());
            let mut opts = TableCompareOptions::default();
            if keyed {
                opts.default_sheet.keys = vec![options::ColumnRef::Position(1)];
            }
            let cancel = CancelToken::new();
            let run = |l: &Arc<Workbook>, r: &Arc<Workbook>| {
                compare_workbooks(Arc::clone(l), Arc::clone(r), &opts, &cancel).map(|c| outcome(&c)).map_err(|e| e.to_string())
            };
            prop_assert_eq!(run(&sa, &db), run(&da, &db));
            prop_assert_eq!(run(&db, &sa), run(&db, &da));
            prop_assert_eq!(run(&sa, &sa), run(&da, &da));
        }
    }
}
