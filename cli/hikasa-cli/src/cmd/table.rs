//! `hikasa table compare <left> <right>` (Table Compare phase 2, contract §6.5): `Service::table_compare`, then the
//! human output (a line per sheet pair, the differing rows, the summary line), a report through REPORT
//! ([`super::report::run_table`]: summary, side-by-side, interleaved, xml, json; text or HTML) or
//! `hikasa.table_compare.v1` with `--json`. Every word is parsed by the service (T11, check 7). `--gui` is refused
//! until the Table Compare view exists (phase 3). Exit codes 1, 2, 12, 13, 100 (T-16).

use hikasa_core::Pair;
use hikasa_session::ApiError;
use hikasa_session::api::dto::GUI_OPEN_SCHEMA;
use hikasa_session::api::{
    ApiOp, SheetRuleWords, TableCompareRequest, TableFormatWords, TableRuleWords,
};

use super::report::{Shape, run_table};
use super::{Ctx, tri};
use crate::args::TableCompareArgs;

/// Runs `table compare`.
pub fn run(ctx: &mut Ctx<'_>, args: &TableCompareArgs) -> u8 {
    if args.gui {
        let refused = ApiError::Unsupported(
            "Table Compare has no GUI view in this version (it arrives with the table grid): compare without --gui, \
             or write an HTML report with --report side-by-side --html --output-to browser"
                .to_owned(),
        );
        return ctx.fail(
            GUI_OPEN_SCHEMA,
            &refused,
            ApiOp::GuiOpen { wait: args.wait },
            &[],
        );
    }
    let request = request_of(ctx, args);
    run_table(ctx, &args.report.report_args(), request, Shape::Compare)
}

/// The request of the flags (the words as typed; the service parses them).
pub fn request_of(ctx: &Ctx<'_>, args: &TableCompareArgs) -> TableCompareRequest {
    let p = &args.parse;
    let r = &args.rules;
    TableCompareRequest {
        left: args.left.clone(),
        right: args.right.clone(),
        format: args.format.clone(),
        formats: Pair::new(args.left_format.clone(), args.right_format.clone()),
        parse: TableFormatWords {
            kind: p.kind.clone(),
            delimiters: p.delimiter.clone(),
            qualifier: p.qualifier.clone(),
            consecutive: p.consecutive,
            trim: p.trim,
            widths: p.widths.clone(),
            first_line: p.first_line.clone(),
            decimal: p.decimal.clone(),
            thousands: p.thousands.clone(),
            date_order: p.date_order.clone(),
            date_sep: p.date_sep.clone(),
            regional: p.regional.clone(),
            encoding: p.encoding.clone(),
            ctrl_z: p.ctrl_z,
        },
        rules: TableRuleWords {
            sheets: r.sheets.clone(),
            sheet_pairs: r.sheet.clone(),
            formulas: r.formulas,
            rules: SheetRuleWords {
                columns: r.columns.clone(),
                align_columns: r.align_column.clone(),
                keys: r.key.clone(),
                unimportant: r.unimportant.clone(),
                text: r.text.clone(),
                number: r.number.clone(),
                ignore_case: r.ignore_case.clone(),
                ignore_whitespace: r.ignore_whitespace.clone(),
                tolerance: r.tolerance.clone(),
                date_tolerance: r.date_tolerance.clone(),
                exact_numbers: r.exact_numbers.clone(),
                hide: r.hide.clone(),
                algorithm: r.algorithm.clone(),
                skew: r.skew,
                closeness: tri(r.closeness, r.no_closeness),
                closeness_threshold: r.closeness_threshold,
                never_align: r.never_align,
                sort_rows: r.sort_rows,
                ignore_hidden_rows: r.ignore_hidden_rows,
                ignore_hidden_columns: r.ignore_hidden_columns,
            },
            sheet: Vec::new(),
        },
        ignore_unimportant: args.ignore_unimportant,
        display: args.report.display.clone(),
        cells: args.cells.clone(),
        format_overrides: Default::default(),
        verbose: ctx
            .matches
            .try_get_one::<bool>("verbose")
            .ok()
            .flatten()
            .copied()
            .unwrap_or(false),
    }
}
