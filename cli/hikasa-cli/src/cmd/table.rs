//! `hikasa table compare <left> <right>` (Table Compare phase 2, contract §6.5): `Service::table_compare`, then the
//! human output (a line per sheet pair, the differing rows, the summary line), a report through REPORT
//! ([`super::report::run_table`]: summary, side-by-side, interleaved, xml, json; text or HTML) or
//! `hikasa.table_compare.v1` with `--json`. Every word is parsed by the service (T11, check 7). `--gui` sends the words to
//! the window's Table Compare view (phase 3, T-52: checked here first, `hikasa.gui_open.v1`). Exit codes 1, 2, 12, 13,
//! 100 (T-16); with `--gui` those of `gui open` (0, or the view's code with `--wait`; 100, 102, 103).

use hikasa_core::Pair;
use hikasa_report::ReportKind;
use hikasa_session::ApiError;
use hikasa_session::api::dto::{GUI_OPEN_SCHEMA, TABLE_COMPARE_SCHEMA};
use hikasa_session::api::{
    ApiOp, GuiOpenRequest, ReportTarget, SheetRuleWords, TableCompareRequest, TableFormatWords,
    TableRuleWords,
};

use super::report::{Shape, asks_report, flags, run_table};
use super::{Ctx, gui, tri};
use crate::args::{ReportArgs, TableCompareArgs, TableSideParseArgs};

/// Runs `table compare`.
pub fn run(ctx: &mut Ctx<'_>, args: &TableCompareArgs) -> u8 {
    if args.gui {
        // The window is the output (Table Compare phase 3, T-52): a report, `--cells` or `--max-rows` has nowhere to go.
        if asks_report(&args.report.report_args())
            || args.cells.is_some()
            || args.max_rows.is_some()
        {
            let refused = ApiError::InvalidArgument(
                "the report flags, --cells and --max-rows do not apply with --gui (the window shows the comparison; \
                 File > Report writes a report from it)"
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
        let open = GuiOpenRequest::for_table(&request, args.wait);
        return gui::open_view(ctx, &open, args.no_launch, &[]);
    }
    let mut report = args.report.report_args();
    if let Err(e) = output_flags(ctx, args, &mut report) {
        return ctx.fail(TABLE_COMPARE_SCHEMA, &e, ApiOp::TableCompare, &[]);
    }
    let request = request_of(ctx, args);
    run_table(ctx, &report, request, Shape::Compare, args.max_rows)
}

/// The output flags against the output (T-46, phase-2 review F10): `--cells` is JSON's (`--json`, or `--report json`
/// where it becomes the report word `cells-*`); `--max-rows` the human output's; `--width` side-by-side and
/// interleaved text's; `--line-numbers`/`--no-line-numbers` side-by-side, interleaved and xml's; `--title` every
/// layout's but json; `--display` every output's but the summary. A flag the output does not use is refused
/// (`invalid_input`) before anything is read.
fn output_flags(
    ctx: &Ctx<'_>,
    args: &TableCompareArgs,
    report: &mut ReportArgs,
) -> Result<(), ApiError> {
    let refuse = |flag: &str, applies: &str| {
        Err(ApiError::InvalidArgument(format!(
            "{flag} means nothing with this output (it is for {applies})"
        )))
    };
    let target = super::report::target(report);
    let asked = asks_report(report);
    // The layout as the report request reads it (a refused word is reported by the comparison).
    let layout = if asked {
        match ctx.service.report_request(
            ReportKind::Table,
            "side-by-side",
            &flags(report, false),
            &target,
        ) {
            Ok(req) => Some(req.layout_word()),
            Err(_) => return Ok(()),
        }
    } else {
        None
    };
    let json_document = ctx.out.json() && (layout.is_none() || target == ReportTarget::Stdout);
    if let Some(cells) = &args.cells {
        match layout {
            Some("json") => report.options.push(format!("cells-{}", cells.trim())),
            _ if json_document => {}
            _ => return refuse("--cells", "JSON (--json, or --report json)"),
        }
    }
    if args.max_rows.is_some() && (asked || ctx.out.json()) {
        return refuse("--max-rows", "the human output (no --json, no report)");
    }
    let Some(layout) = layout else {
        return Ok(());
    };
    let html = report.html || report.html_mono;
    if report.width.is_some() && (!matches!(layout, "side-by-side" | "interleaved") || html) {
        return refuse("--width", "side-by-side and interleaved text reports");
    }
    if (report.line_numbers || report.no_line_numbers)
        && !matches!(layout, "side-by-side" | "interleaved" | "xml")
    {
        return refuse(
            "--line-numbers and --no-line-numbers",
            "side-by-side, interleaved and xml reports",
        );
    }
    if report.title.is_some() && layout == "json" {
        return refuse(
            "--title",
            "summary, side-by-side, interleaved and xml reports",
        );
    }
    if report.display.is_some() && layout == "summary" {
        return refuse("--display", "the rows of a report (the summary shows none)");
    }
    Ok(())
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
        side_parse: side_words(&args.side_parse),
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

/// The per-side parser words of the flags (T-43).
fn side_words(a: &TableSideParseArgs) -> Pair<TableFormatWords> {
    Pair::new(
        TableFormatWords {
            kind: a.left_kind.clone(),
            delimiters: a.left_delimiter.clone(),
            qualifier: a.left_qualifier.clone(),
            consecutive: a.left_consecutive,
            trim: a.left_trim,
            widths: a.left_widths.clone(),
            first_line: a.left_first_line.clone(),
            regional: a.left_regional.clone(),
            decimal: a.left_decimal.clone(),
            thousands: a.left_thousands.clone(),
            date_order: a.left_date_order.clone(),
            date_sep: a.left_date_sep.clone(),
            encoding: a.left_encoding.clone(),
            ctrl_z: a.left_ctrl_z,
        },
        TableFormatWords {
            kind: a.right_kind.clone(),
            delimiters: a.right_delimiter.clone(),
            qualifier: a.right_qualifier.clone(),
            consecutive: a.right_consecutive,
            trim: a.right_trim,
            widths: a.right_widths.clone(),
            first_line: a.right_first_line.clone(),
            regional: a.right_regional.clone(),
            decimal: a.right_decimal.clone(),
            thousands: a.right_thousands.clone(),
            date_order: a.right_date_order.clone(),
            date_sep: a.right_date_sep.clone(),
            encoding: a.right_encoding.clone(),
            ctrl_z: a.right_ctrl_z,
        },
    )
}
