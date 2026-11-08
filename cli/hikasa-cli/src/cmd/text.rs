//! `hikasa text compare <left> <right>` (§3.2): `Service::text_compare`, then the report through REPORT
//! ([`super::report::run_text`]; the annotated unified diff by default) or `hikasa.text_compare.v1`
//! (`TextCompareResponse::json`, display `mismatches` by default). `--output`/`--output-to` writes the report to its
//! target and prints nothing (with `--json` the envelope still goes to stdout). `--gui` sends the request to the GUI
//! instead (D-50).

use hikasa_core::Pair;
use hikasa_session::ApiError;
use hikasa_session::api::dto::{GUI_OPEN_SCHEMA, TEXT_COMPARE_SCHEMA};
use hikasa_session::api::{ApiOp, GuiOpenRequest, TextCompareRequest, ViewWords, parse_algorithm};
use std::ffi::OsString;

use super::report::{Shape, run_text};
use super::{Ctx, gui, importance_patch, tri};
use crate::args::{TextCompareArgs, TextRuleArgs};

/// Runs `text compare`.
pub fn run(ctx: &mut Ctx<'_>, args: &TextCompareArgs) -> u8 {
    let request = match request_of(ctx, &args.rule, args.left.clone(), args.right.clone()) {
        Ok(request) => request,
        Err(e) => return fail(ctx, args, &e),
    };
    if args.gui {
        let words = ViewWords {
            display: args.report.display.clone(),
            context_lines: args.report.context,
            ..ViewWords::default()
        };
        let open = GuiOpenRequest::for_text(&request, &words, args.wait);
        return gui::open_view(ctx, &open, args.no_launch, &[]);
    }
    run_text(ctx, &args.report, request, "unified", Shape::Compare)
}

/// A failure before the comparison: the GUI's document and operation with `--gui`.
fn fail(ctx: &mut Ctx<'_>, args: &TextCompareArgs, err: &ApiError) -> u8 {
    let (schema, op) = if args.gui {
        (GUI_OPEN_SCHEMA, ApiOp::GuiOpen { wait: args.wait })
    } else {
        (TEXT_COMPARE_SCHEMA, ApiOp::TextCompare)
    };
    ctx.fail(schema, err, op, &[])
}

/// The request of the rule flags (§1.9.2 mapping): `--encoding` fills both sides, `--left-encoding` and
/// `--right-encoding` replace one; the rules patch carries the alignment and importance flags. One owner for
/// `text compare` and `report text`.
pub fn request_of(
    ctx: &Ctx<'_>,
    rule: &TextRuleArgs,
    left: OsString,
    right: OsString,
) -> Result<TextCompareRequest, ApiError> {
    let mut rules_patch = importance_patch(&rule.importance, ctx.matches)?;
    rules_patch.algorithm = rule.algorithm.as_deref().map(parse_algorithm).transpose()?;
    rules_patch.skew_tolerance = rule.skew;
    rules_patch.closeness = tri(rule.closeness, rule.no_closeness);
    rules_patch.closeness_threshold_permille = rule.closeness_threshold;
    rules_patch.never_align_differences = rule.never_align.then_some(true);
    rules_patch.compare_line_endings = rule.compare_line_endings.then_some(true);
    rules_patch.orphans_always_important = rule.orphans_unimportant.then_some(false);
    Ok(TextCompareRequest {
        left,
        right,
        format: rule.format.clone(),
        encodings: Pair::new(
            rule.left_encoding.clone().or_else(|| rule.encoding.clone()),
            rule.right_encoding
                .clone()
                .or_else(|| rule.encoding.clone()),
        ),
        rules_file: rule.rules_file.clone(),
        rules_patch,
        ignore_unimportant: rule.ignore_unimportant,
        base_rules: None,
        format_overrides: Default::default(),
    })
}
